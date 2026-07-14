//! Workflow engine — multi-step agent pipeline execution.
//!
//! A workflow defines a sequence of steps where each step routes
//! a task to a specific agent. Steps can:
//! - Pass their output as input to the next step
//! - Run in sequence (pipeline) or in parallel (fan-out)
//! - Conditionally skip based on previous output
//! - Loop until a condition is met
//! - Store outputs in named variables for later reference
//!
//! Workflows are defined as Rust structs or loaded from JSON.

use captain_types::agent::AgentId;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;
use tracing::{debug, info, warn};
use uuid::Uuid;

/// Unique identifier for a workflow definition.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct WorkflowId(pub Uuid);

impl WorkflowId {
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }
}

impl Default for WorkflowId {
    fn default() -> Self {
        Self::new()
    }
}

impl std::fmt::Display for WorkflowId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// Unique identifier for a running workflow instance.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct WorkflowRunId(pub Uuid);

impl WorkflowRunId {
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }
}

impl Default for WorkflowRunId {
    fn default() -> Self {
        Self::new()
    }
}

impl std::fmt::Display for WorkflowRunId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// A workflow definition — a named sequence of steps.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Workflow {
    /// Unique identifier.
    pub id: WorkflowId,
    /// Human-readable name.
    pub name: String,
    /// Description of what this workflow does.
    pub description: String,
    /// The steps in execution order (legacy sequential mode).
    pub steps: Vec<WorkflowStep>,
    /// Graph-based workflow (v2). When present, the DAG executor is used.
    #[serde(default)]
    pub graph: Option<WorkflowGraph>,
    /// Created at.
    pub created_at: DateTime<Utc>,
}

/// A single step in a workflow.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkflowStep {
    /// Step name for logging/display.
    pub name: String,
    /// Which agent to route this step to.
    pub agent: StepAgent,
    /// The prompt template. Use `{{input}}` for previous output, `{{var_name}}` for variables.
    pub prompt_template: String,
    /// Execution mode for this step.
    pub mode: StepMode,
    /// Maximum time for this step in seconds (default: 120).
    #[serde(default = "default_timeout")]
    pub timeout_secs: u64,
    /// Error handling mode for this step (default: Fail).
    #[serde(default)]
    pub error_mode: ErrorMode,
    /// Optional variable name to store this step's output in.
    #[serde(default)]
    pub output_var: Option<String>,
}

fn default_timeout() -> u64 {
    120
}

/// Owned context passed to step senders that need operational metadata.
#[derive(Debug, Clone)]
pub struct WorkflowStepContext {
    pub step_name: String,
    pub timeout_secs: u64,
}

impl WorkflowStepContext {
    fn from_step(step: &WorkflowStep) -> Self {
        Self {
            step_name: step.name.clone(),
            timeout_secs: step.timeout_secs,
        }
    }
}

/// How workflow step timeouts are enforced.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WorkflowStepTimeoutPolicy {
    WallClock,
    CallerManaged,
}

struct WorkflowRunProgress {
    current_input: String,
    all_outputs: Vec<String>,
    variables: HashMap<String, String>,
}

impl WorkflowRunProgress {
    fn new(input: String) -> Self {
        Self {
            current_input: input,
            all_outputs: Vec::new(),
            variables: HashMap::new(),
        }
    }
}

/// How to identify the agent for a step.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum StepAgent {
    /// Reference an agent by UUID.
    ById { id: String },
    /// Reference an agent by name (first match).
    ByName { name: String },
}

/// Execution mode for a workflow step.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StepMode {
    /// Execute sequentially — this step runs after the previous completes.
    #[default]
    Sequential,
    /// Fan-out — this step runs in parallel with subsequent FanOut steps until Collect.
    FanOut,
    /// Collect results from all preceding fan-out steps.
    Collect,
    /// Conditional — skip this step if previous output doesn't contain `condition` (case-insensitive).
    Conditional { condition: String },
    /// Loop — repeat this step until output contains `until` or `max_iterations` reached.
    Loop { max_iterations: u32, until: String },
}

/// Error handling mode for a workflow step.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ErrorMode {
    /// Abort the workflow on error (default).
    #[default]
    Fail,
    /// Skip this step on error and continue.
    Skip,
    /// Retry the step up to N times before failing.
    Retry { max_retries: u32 },
}

// ═══════════════════════════════════════════════════════════════════════════════
// Graph-based workflow model (v2)
// ═══════════════════════════════════════════════════════════════════════════════

/// A workflow graph — DAG of nodes connected by edges.
/// When present on a Workflow, the graph-based executor is used instead of steps.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkflowGraph {
    pub nodes: Vec<WorkflowNode>,
    pub edges: Vec<WorkflowEdge>,
}

/// A node in the workflow graph.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkflowNode {
    /// Unique node ID (matches ReactFlow node id).
    pub id: String,
    /// Node type determines execution behavior.
    pub node_type: NodeType,
    /// Human-readable label.
    pub label: String,
    /// Node-specific configuration.
    pub config: NodeConfig,
    /// Position in the visual editor (for ReactFlow).
    #[serde(default)]
    pub position: NodePosition,
    /// Error handling mode for this node.
    #[serde(default)]
    pub error_mode: ErrorMode,
    /// Timeout in seconds (0 = use default 120s).
    #[serde(default)]
    pub timeout_secs: u64,
}

/// Visual position for the node editor.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct NodePosition {
    pub x: f64,
    pub y: f64,
}

/// The type of a workflow node.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NodeType {
    /// Entry point — triggers workflow execution.
    Trigger,
    /// Deterministic action — no LLM involved.
    Action,
    /// LLM agent — can make decisions, use tools.
    Agent,
    /// Control flow — IF, Switch, Loop, Merge.
    Logic,
    /// Calls another workflow as a sub-workflow.
    SubWorkflow,
    /// Pauses for human input/approval.
    Human,
}

/// Node-specific configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum NodeConfig {
    /// Trigger configuration.
    #[serde(rename = "trigger")]
    Trigger {
        /// Trigger kind: manual, cron, webhook, event.
        trigger_type: TriggerType,
        /// Cron expression (for cron triggers).
        #[serde(default)]
        cron_expr: String,
        /// Webhook path (for webhook triggers).
        #[serde(default)]
        webhook_path: String,
    },
    /// Action configuration (deterministic, no LLM).
    #[serde(rename = "action")]
    Action {
        /// Action kind: http, shell, transform, file.
        action_type: ActionType,
        /// Configuration specific to the action type.
        #[serde(default)]
        params: serde_json::Value,
    },
    /// Agent configuration (LLM-powered).
    #[serde(rename = "agent")]
    Agent {
        /// Agent to use (by name or ID).
        agent: StepAgent,
        /// Prompt template. Use {{input}} for previous node output.
        prompt_template: String,
        /// Optional variable name to store output.
        #[serde(default)]
        output_var: Option<String>,
    },
    /// Logic node configuration.
    #[serde(rename = "logic")]
    Logic {
        /// Logic kind: if_then, switch, loop, merge.
        logic_type: LogicType,
        /// Condition expression (for if/switch).
        #[serde(default)]
        condition: String,
        /// Max iterations (for loops).
        #[serde(default)]
        max_iterations: u32,
    },
    /// Sub-workflow configuration.
    #[serde(rename = "sub_workflow")]
    SubWorkflow {
        /// ID of the workflow to call.
        workflow_id: String,
    },
    /// Human approval/input configuration.
    #[serde(rename = "human")]
    Human {
        /// Question to ask the human.
        question: String,
        /// Optional choices.
        #[serde(default)]
        options: Vec<String>,
        /// Channel to send the question on (web, telegram, etc.).
        #[serde(default)]
        channel: String,
    },
}

/// Trigger types.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TriggerType {
    Manual,
    Cron,
    Webhook,
    Event,
}

/// Action types (deterministic, no LLM).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ActionType {
    Http,
    Shell,
    Transform,
    File,
    SetVariable,
}

/// Logic node types.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LogicType {
    IfThen,
    Switch,
    Loop,
    Merge,
}

/// An edge connecting two nodes in the workflow graph.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkflowEdge {
    /// Unique edge ID.
    pub id: String,
    /// Source node ID.
    pub source: String,
    /// Target node ID.
    pub target: String,
    /// Edge type (normal or conditional).
    #[serde(default)]
    pub edge_type: EdgeType,
    /// Condition label (for conditional edges, e.g., "true", "false", "error").
    #[serde(default)]
    pub label: String,
}

/// Edge types.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EdgeType {
    /// Normal edge — always taken.
    #[default]
    Normal,
    /// Conditional edge — taken only if the condition matches.
    Conditional,
    /// Error edge — taken only on node failure.
    Error,
}

/// Data passed between nodes.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NodeOutput {
    /// Structured JSON data.
    pub json: serde_json::Value,
    /// Optional binary data (base64 encoded).
    #[serde(default)]
    pub binary: Option<String>,
    /// Node that produced this output.
    pub source_node: String,
}

// ═══════════════════════════════════════════════════════════════════════════════
// Workflow run state
// ═══════════════════════════════════════════════════════════════════════════════

/// The current state of a workflow run.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkflowRunState {
    Pending,
    Running,
    Completed,
    Failed,
}

/// A running workflow instance.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkflowRun {
    /// Run instance ID.
    pub id: WorkflowRunId,
    /// The workflow being run.
    pub workflow_id: WorkflowId,
    /// Workflow name (copied for quick access).
    pub workflow_name: String,
    /// Initial input to the workflow.
    pub input: String,
    /// Current state.
    pub state: WorkflowRunState,
    /// Results from each completed step.
    pub step_results: Vec<StepResult>,
    /// Final output (set when workflow completes).
    pub output: Option<String>,
    /// Error message if failed.
    pub error: Option<String>,
    /// Started at.
    pub started_at: DateTime<Utc>,
    /// Completed at.
    pub completed_at: Option<DateTime<Utc>>,
}

/// Result from a single workflow step.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StepResult {
    /// Step name.
    pub step_name: String,
    /// Agent that executed this step.
    pub agent_id: String,
    /// Agent name.
    pub agent_name: String,
    /// Output from this step.
    pub output: String,
    /// Token usage.
    pub input_tokens: u64,
    pub output_tokens: u64,
    /// Duration in milliseconds.
    pub duration_ms: u64,
}

/// The workflow engine — manages definitions and executes pipeline runs.
pub struct WorkflowEngine {
    /// Registered workflow definitions.
    workflows: Arc<RwLock<HashMap<WorkflowId, Workflow>>>,
    /// Active and completed workflow runs.
    runs: Arc<RwLock<HashMap<WorkflowRunId, WorkflowRun>>>,
}

impl WorkflowEngine {
    /// Create a new workflow engine.
    pub fn new() -> Self {
        Self {
            workflows: Arc::new(RwLock::new(HashMap::new())),
            runs: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    /// Register a new workflow definition.
    pub async fn register(&self, workflow: Workflow) -> WorkflowId {
        let id = workflow.id;
        self.workflows.write().await.insert(id, workflow);
        info!(workflow_id = %id, "Workflow registered");
        id
    }

    /// List all registered workflows.
    pub async fn list_workflows(&self) -> Vec<Workflow> {
        self.workflows.read().await.values().cloned().collect()
    }

    /// Get a specific workflow by ID.
    pub async fn get_workflow(&self, id: WorkflowId) -> Option<Workflow> {
        self.workflows.read().await.get(&id).cloned()
    }

    /// Remove a workflow definition.
    pub async fn remove_workflow(&self, id: WorkflowId) -> bool {
        self.workflows.write().await.remove(&id).is_some()
    }

    /// Update an existing workflow definition.
    ///
    /// Preserves the original `id` and `created_at`. Replaces `name`,
    /// `description`, and `steps`. Returns `true` if the workflow was
    /// found and updated.
    pub async fn update_workflow(&self, id: WorkflowId, updated: Workflow) -> bool {
        let mut workflows = self.workflows.write().await;
        if let Some(existing) = workflows.get_mut(&id) {
            existing.name = updated.name;
            existing.description = updated.description;
            existing.steps = updated.steps;
            info!(workflow_id = %id, "Workflow updated");
            true
        } else {
            false
        }
    }

    /// Maximum number of retained workflow runs. Oldest completed/failed
    /// runs are evicted when this limit is exceeded.
    const MAX_RETAINED_RUNS: usize = 200;

    /// Start a workflow run. Returns the run ID and a handle to check progress.
    ///
    /// The actual execution is driven externally by calling `execute_run()`
    /// with the kernel handle, since the workflow engine doesn't own the kernel.
    pub async fn create_run(
        &self,
        workflow_id: WorkflowId,
        input: String,
    ) -> Option<WorkflowRunId> {
        let workflow = self.workflows.read().await.get(&workflow_id)?.clone();
        let run_id = WorkflowRunId::new();

        let run = WorkflowRun {
            id: run_id,
            workflow_id,
            workflow_name: workflow.name,
            input,
            state: WorkflowRunState::Pending,
            step_results: Vec::new(),
            output: None,
            error: None,
            started_at: Utc::now(),
            completed_at: None,
        };

        let mut runs = self.runs.write().await;
        runs.insert(run_id, run);

        // Evict oldest completed/failed runs when we exceed the cap
        if runs.len() > Self::MAX_RETAINED_RUNS {
            let mut evictable: Vec<(WorkflowRunId, DateTime<Utc>)> = runs
                .iter()
                .filter(|(_, r)| {
                    matches!(
                        r.state,
                        WorkflowRunState::Completed | WorkflowRunState::Failed
                    )
                })
                .map(|(id, r)| (*id, r.started_at))
                .collect();

            // Sort oldest first
            evictable.sort_by_key(|(_, t)| *t);

            let to_remove = runs.len() - Self::MAX_RETAINED_RUNS;
            for (id, _) in evictable.into_iter().take(to_remove) {
                runs.remove(&id);
                debug!(run_id = %id, "Evicted old workflow run");
            }
        }

        Some(run_id)
    }

    /// Get the current state of a workflow run.
    pub async fn get_run(&self, run_id: WorkflowRunId) -> Option<WorkflowRun> {
        self.runs.read().await.get(&run_id).cloned()
    }

    /// List all workflow runs (optionally filtered by state).
    pub async fn list_runs(&self, state_filter: Option<&str>) -> Vec<WorkflowRun> {
        self.runs
            .read()
            .await
            .values()
            .filter(|r| {
                state_filter
                    .map(|f| match f {
                        "pending" => matches!(r.state, WorkflowRunState::Pending),
                        "running" => matches!(r.state, WorkflowRunState::Running),
                        "completed" => matches!(r.state, WorkflowRunState::Completed),
                        "failed" => matches!(r.state, WorkflowRunState::Failed),
                        _ => true,
                    })
                    .unwrap_or(true)
            })
            .cloned()
            .collect()
    }

    /// Replace `{{var_name}}` references in a template with stored variable values.
    fn expand_variables(template: &str, input: &str, vars: &HashMap<String, String>) -> String {
        let mut result = template.replace("{{input}}", input);
        for (key, value) in vars {
            result = result.replace(&format!("{{{{{key}}}}}"), value);
        }
        result
    }

    /// Execute a single step with error mode handling. Returns (output, input_tokens, output_tokens).
    async fn execute_step_attempt<F, Fut>(
        step: &WorkflowStep,
        agent_id: AgentId,
        prompt: String,
        send_message: &F,
        timeout_policy: WorkflowStepTimeoutPolicy,
    ) -> Result<(String, u64, u64), String>
    where
        F: Fn(AgentId, String, WorkflowStepContext) -> Fut,
        Fut: std::future::Future<Output = Result<(String, u64, u64), String>>,
    {
        let timeout_dur = std::time::Duration::from_secs(step.timeout_secs);
        let context = WorkflowStepContext::from_step(step);

        match timeout_policy {
            WorkflowStepTimeoutPolicy::WallClock => {
                tokio::time::timeout(timeout_dur, send_message(agent_id, prompt, context))
                    .await
                    .map_err(|_| format!("timed out after {}s", step.timeout_secs))?
            }
            WorkflowStepTimeoutPolicy::CallerManaged => {
                send_message(agent_id, prompt, context).await
            }
        }
    }

    /// Execute a single step with error mode handling. Returns (output, input_tokens, output_tokens).
    async fn execute_step_with_error_mode<F, Fut>(
        step: &WorkflowStep,
        agent_id: AgentId,
        prompt: String,
        send_message: &F,
        timeout_policy: WorkflowStepTimeoutPolicy,
    ) -> Result<Option<(String, u64, u64)>, String>
    where
        F: Fn(AgentId, String, WorkflowStepContext) -> Fut,
        Fut: std::future::Future<Output = Result<(String, u64, u64), String>>,
    {
        match &step.error_mode {
            ErrorMode::Fail => {
                let result = Self::execute_step_attempt(
                    step,
                    agent_id,
                    prompt,
                    send_message,
                    timeout_policy,
                )
                .await
                .map_err(|e| {
                    if e.starts_with("timed out after ") {
                        format!("Step '{}' {e}", step.name)
                    } else {
                        format!("Step '{}' failed: {}", step.name, e)
                    }
                })?;
                Ok(Some(result))
            }
            ErrorMode::Skip => {
                match Self::execute_step_attempt(
                    step,
                    agent_id,
                    prompt,
                    send_message,
                    timeout_policy,
                )
                .await
                {
                    Ok(result) => Ok(Some(result)),
                    Err(e) if e.starts_with("timed out after ") => {
                        warn!(
                            "Step '{}' timed out (skipping) after {}s",
                            step.name, step.timeout_secs
                        );
                        Ok(None)
                    }
                    Err(e) => {
                        warn!("Step '{}' failed (skipping): {e}", step.name);
                        Ok(None)
                    }
                }
            }
            ErrorMode::Retry { max_retries } => {
                let mut last_err = String::new();
                for attempt in 0..=*max_retries {
                    match Self::execute_step_attempt(
                        step,
                        agent_id,
                        prompt.clone(),
                        send_message,
                        timeout_policy,
                    )
                    .await
                    {
                        Ok(result) => return Ok(Some(result)),
                        Err(e) if e.starts_with("timed out after ") => {
                            last_err = e;
                            if attempt < *max_retries {
                                warn!(
                                    "Step '{}' attempt {} timed out, retrying",
                                    step.name,
                                    attempt + 1
                                );
                            }
                        }
                        Err(e) => {
                            last_err = e.to_string();
                            if attempt < *max_retries {
                                warn!(
                                    "Step '{}' attempt {} failed: {e}, retrying",
                                    step.name,
                                    attempt + 1
                                );
                            }
                        }
                    }
                }
                Err(format!(
                    "Step '{}' failed after {} retries: {last_err}",
                    step.name, max_retries
                ))
            }
        }
    }

    /// Execute a workflow run step-by-step.
    ///
    /// This method takes a closure that sends messages to agents,
    /// so the workflow engine remains decoupled from the kernel.
    pub async fn execute_run<F, Fut>(
        &self,
        run_id: WorkflowRunId,
        agent_resolver: impl Fn(&StepAgent) -> Option<(AgentId, String)>,
        send_message: F,
    ) -> Result<String, String>
    where
        F: Fn(AgentId, String) -> Fut,
        Fut: std::future::Future<Output = Result<(String, u64, u64), String>>,
    {
        self.execute_run_with_step_timeout_policy(
            run_id,
            agent_resolver,
            |agent_id, prompt, _context| send_message(agent_id, prompt),
            WorkflowStepTimeoutPolicy::WallClock,
        )
        .await
    }

    /// Execute a workflow run with caller-selected step timeout semantics.
    pub async fn execute_run_with_step_timeout_policy<F, Fut>(
        &self,
        run_id: WorkflowRunId,
        agent_resolver: impl Fn(&StepAgent) -> Option<(AgentId, String)>,
        send_message: F,
        timeout_policy: WorkflowStepTimeoutPolicy,
    ) -> Result<String, String>
    where
        F: Fn(AgentId, String, WorkflowStepContext) -> Fut,
        Fut: std::future::Future<Output = Result<(String, u64, u64), String>>,
    {
        let (workflow, input) = self.prepare_workflow_run(run_id).await?;

        Self::log_workflow_start(run_id, &workflow);

        let mut progress = WorkflowRunProgress::new(input);
        let mut i = 0;

        while i < workflow.steps.len() {
            let step = &workflow.steps[i];
            debug!(step = i + 1, name = %step.name, "Executing workflow step");

            match &step.mode {
                StepMode::Sequential => {
                    self.execute_linear_workflow_step(
                        run_id,
                        i,
                        step,
                        &agent_resolver,
                        &send_message,
                        timeout_policy,
                        &mut progress,
                        true,
                    )
                    .await?;
                }
                StepMode::FanOut => {
                    i = self
                        .execute_fan_out_workflow_steps(
                            run_id,
                            i,
                            &workflow.steps,
                            &agent_resolver,
                            &send_message,
                            timeout_policy,
                            &mut progress,
                        )
                        .await?;
                    continue;
                }
                StepMode::Collect => Self::collect_workflow_outputs(step, &mut progress),
                StepMode::Conditional { condition } => {
                    if !Self::workflow_condition_matches(&progress.current_input, condition) {
                        info!(
                            step = i + 1,
                            name = %step.name,
                            condition,
                            "Conditional step skipped (condition not met)"
                        );
                        i += 1;
                        continue;
                    }

                    self.execute_linear_workflow_step(
                        run_id,
                        i,
                        step,
                        &agent_resolver,
                        &send_message,
                        timeout_policy,
                        &mut progress,
                        false,
                    )
                    .await?;
                }
                StepMode::Loop {
                    max_iterations,
                    until,
                } => {
                    self.execute_loop_workflow_step(
                        run_id,
                        i,
                        step,
                        *max_iterations,
                        until,
                        &agent_resolver,
                        &send_message,
                        timeout_policy,
                        &mut progress,
                    )
                    .await?;
                }
            }

            i += 1;
        }

        Ok(self.finish_workflow_run(run_id, &progress).await)
    }

    async fn prepare_workflow_run(
        &self,
        run_id: WorkflowRunId,
    ) -> Result<(Workflow, String), String> {
        let mut runs = self.runs.write().await;
        let run = runs.get_mut(&run_id).ok_or("Workflow run not found")?;
        run.state = WorkflowRunState::Running;

        let workflow = self
            .workflows
            .read()
            .await
            .get(&run.workflow_id)
            .ok_or("Workflow definition not found")?
            .clone();

        Ok((workflow, run.input.clone()))
    }

    fn log_workflow_start(run_id: WorkflowRunId, workflow: &Workflow) {
        info!(
            run_id = %run_id,
            workflow = %workflow.name,
            steps = workflow.steps.len(),
            "Starting workflow execution"
        );
    }

    async fn finish_workflow_run(
        &self,
        run_id: WorkflowRunId,
        progress: &WorkflowRunProgress,
    ) -> String {
        let final_output = progress.current_input.clone();
        self.complete_workflow_run(run_id, final_output.clone())
            .await;
        info!(run_id = %run_id, "Workflow completed successfully");
        final_output
    }

    async fn fail_workflow_run(&self, run_id: WorkflowRunId, error: &str) {
        if let Some(run) = self.runs.write().await.get_mut(&run_id) {
            run.state = WorkflowRunState::Failed;
            run.error = Some(error.to_string());
            run.completed_at = Some(Utc::now());
        }
    }

    async fn complete_workflow_run(&self, run_id: WorkflowRunId, output: String) {
        if let Some(run) = self.runs.write().await.get_mut(&run_id) {
            run.state = WorkflowRunState::Completed;
            run.output = Some(output);
            run.completed_at = Some(Utc::now());
        }
    }

    async fn record_workflow_step_result(&self, run_id: WorkflowRunId, result: StepResult) {
        if let Some(run) = self.runs.write().await.get_mut(&run_id) {
            run.step_results.push(result);
        }
    }

    fn workflow_step_result(
        step_name: String,
        agent_id: AgentId,
        agent_name: String,
        output: String,
        input_tokens: u64,
        output_tokens: u64,
        duration_ms: u64,
    ) -> StepResult {
        StepResult {
            step_name,
            agent_id: agent_id.to_string(),
            agent_name,
            output,
            input_tokens,
            output_tokens,
            duration_ms,
        }
    }

    fn apply_workflow_step_output(
        step: &WorkflowStep,
        output: String,
        progress: &mut WorkflowRunProgress,
    ) {
        if let Some(ref var) = step.output_var {
            progress.variables.insert(var.clone(), output.clone());
        }
        progress.all_outputs.push(output.clone());
        progress.current_input = output;
    }

    #[allow(clippy::too_many_arguments)]
    async fn execute_linear_workflow_step<F, Fut>(
        &self,
        run_id: WorkflowRunId,
        step_index: usize,
        step: &WorkflowStep,
        agent_resolver: &impl Fn(&StepAgent) -> Option<(AgentId, String)>,
        send_message: &F,
        timeout_policy: WorkflowStepTimeoutPolicy,
        progress: &mut WorkflowRunProgress,
        log_skip: bool,
    ) -> Result<(), String>
    where
        F: Fn(AgentId, String, WorkflowStepContext) -> Fut,
        Fut: std::future::Future<Output = Result<(String, u64, u64), String>>,
    {
        let (agent_id, agent_name) = agent_resolver(&step.agent)
            .ok_or_else(|| format!("Agent not found for step '{}'", step.name))?;
        let prompt = Self::expand_variables(
            &step.prompt_template,
            &progress.current_input,
            &progress.variables,
        );

        let start = std::time::Instant::now();
        let result = Self::execute_step_with_error_mode(
            step,
            agent_id,
            prompt,
            send_message,
            timeout_policy,
        )
        .await;
        let duration_ms = start.elapsed().as_millis() as u64;

        match result {
            Ok(Some((output, input_tokens, output_tokens))) => {
                self.record_workflow_step_result(
                    run_id,
                    Self::workflow_step_result(
                        step.name.clone(),
                        agent_id,
                        agent_name,
                        output.clone(),
                        input_tokens,
                        output_tokens,
                        duration_ms,
                    ),
                )
                .await;
                Self::apply_workflow_step_output(step, output, progress);
                info!(step = step_index + 1, name = %step.name, duration_ms, "Step completed");
                Ok(())
            }
            Ok(None) => {
                if log_skip {
                    info!(step = step_index + 1, name = %step.name, "Step skipped");
                }
                Ok(())
            }
            Err(error) => {
                self.fail_workflow_run(run_id, &error).await;
                Err(error)
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    async fn execute_fan_out_workflow_steps<F, Fut>(
        &self,
        run_id: WorkflowRunId,
        start_index: usize,
        steps: &[WorkflowStep],
        agent_resolver: &impl Fn(&StepAgent) -> Option<(AgentId, String)>,
        send_message: &F,
        timeout_policy: WorkflowStepTimeoutPolicy,
        progress: &mut WorkflowRunProgress,
    ) -> Result<usize, String>
    where
        F: Fn(AgentId, String, WorkflowStepContext) -> Fut,
        Fut: std::future::Future<Output = Result<(String, u64, u64), String>>,
    {
        let mut fan_out_steps = vec![(start_index, &steps[start_index])];
        let mut next_index = start_index + 1;
        while next_index < steps.len() && matches!(steps[next_index].mode, StepMode::FanOut) {
            fan_out_steps.push((next_index, &steps[next_index]));
            next_index += 1;
        }

        let mut futures = Vec::new();
        let mut step_infos = Vec::new();
        for (idx, fan_step) in &fan_out_steps {
            let (agent_id, agent_name) = agent_resolver(&fan_step.agent)
                .ok_or_else(|| format!("Agent not found for step '{}'", fan_step.name))?;
            let prompt = Self::expand_variables(
                &fan_step.prompt_template,
                &progress.current_input,
                &progress.variables,
            );
            step_infos.push((*idx, fan_step.name.clone(), agent_id, agent_name));
            futures.push(Self::execute_step_attempt(
                fan_step,
                agent_id,
                prompt,
                send_message,
                timeout_policy,
            ));
        }

        let start = std::time::Instant::now();
        let results = futures::future::join_all(futures).await;
        let duration_ms = start.elapsed().as_millis() as u64;

        for (index, result) in results.into_iter().enumerate() {
            let (_, step_name, agent_id, agent_name) = &step_infos[index];
            let fan_step = fan_out_steps[index].1;
            match result {
                Ok((output, input_tokens, output_tokens)) => {
                    self.record_workflow_step_result(
                        run_id,
                        Self::workflow_step_result(
                            step_name.clone(),
                            *agent_id,
                            agent_name.clone(),
                            output.clone(),
                            input_tokens,
                            output_tokens,
                            duration_ms,
                        ),
                    )
                    .await;
                    Self::apply_workflow_step_output(fan_step, output, progress);
                }
                Err(error) => {
                    let error_msg = Self::fan_out_error_message(step_name, &error);
                    warn!(%error_msg);
                    self.fail_workflow_run(run_id, &error_msg).await;
                    return Err(error_msg);
                }
            }
        }

        info!(
            count = fan_out_steps.len(),
            duration_ms, "FanOut steps completed"
        );
        Ok(next_index)
    }

    fn fan_out_error_message(step_name: &str, error: &str) -> String {
        if error.starts_with("timed out after ") {
            format!("FanOut step '{}' {error}", step_name)
        } else {
            format!("FanOut step '{}' failed: {}", step_name, error)
        }
    }

    fn collect_workflow_outputs(step: &WorkflowStep, progress: &mut WorkflowRunProgress) {
        progress.current_input = progress.all_outputs.join("\n\n---\n\n");
        progress.all_outputs.clear();
        progress.all_outputs.push(progress.current_input.clone());
        if let Some(ref var) = step.output_var {
            progress
                .variables
                .insert(var.clone(), progress.current_input.clone());
        }
    }

    fn workflow_condition_matches(input: &str, condition: &str) -> bool {
        input.to_lowercase().contains(&condition.to_lowercase())
    }

    #[allow(clippy::too_many_arguments)]
    async fn execute_loop_workflow_step<F, Fut>(
        &self,
        run_id: WorkflowRunId,
        step_index: usize,
        step: &WorkflowStep,
        max_iterations: u32,
        until: &str,
        agent_resolver: &impl Fn(&StepAgent) -> Option<(AgentId, String)>,
        send_message: &F,
        timeout_policy: WorkflowStepTimeoutPolicy,
        progress: &mut WorkflowRunProgress,
    ) -> Result<(), String>
    where
        F: Fn(AgentId, String, WorkflowStepContext) -> Fut,
        Fut: std::future::Future<Output = Result<(String, u64, u64), String>>,
    {
        let (agent_id, agent_name) = agent_resolver(&step.agent)
            .ok_or_else(|| format!("Agent not found for step '{}'", step.name))?;
        let until_lower = until.to_lowercase();

        for loop_iter in 0..max_iterations {
            let prompt = Self::expand_variables(
                &step.prompt_template,
                &progress.current_input,
                &progress.variables,
            );
            let start = std::time::Instant::now();
            let result = Self::execute_step_with_error_mode(
                step,
                agent_id,
                prompt,
                send_message,
                timeout_policy,
            )
            .await;
            let duration_ms = start.elapsed().as_millis() as u64;

            match result {
                Ok(Some((output, input_tokens, output_tokens))) => {
                    self.record_workflow_step_result(
                        run_id,
                        Self::workflow_step_result(
                            format!("{} (iter {})", step.name, loop_iter + 1),
                            agent_id,
                            agent_name.clone(),
                            output.clone(),
                            input_tokens,
                            output_tokens,
                            duration_ms,
                        ),
                    )
                    .await;
                    progress.current_input = output.clone();

                    if output.to_lowercase().contains(&until_lower) {
                        info!(
                            step = step_index + 1,
                            name = %step.name,
                            iterations = loop_iter + 1,
                            "Loop terminated (until condition met)"
                        );
                        break;
                    }

                    if loop_iter + 1 == max_iterations {
                        info!(step = step_index + 1, name = %step.name, "Loop terminated (max iterations reached)");
                    }
                }
                Ok(None) => break,
                Err(error) => {
                    self.fail_workflow_run(run_id, &error).await;
                    return Err(error);
                }
            }
        }

        if let Some(ref var) = step.output_var {
            progress
                .variables
                .insert(var.clone(), progress.current_input.clone());
        }
        progress.all_outputs.push(progress.current_input.clone());
        Ok(())
    }

    // ═══════════════════════════════════════════════════════════════════════
    // Graph-based execution engine (v2)
    // ═══════════════════════════════════════════════════════════════════════

    /// Execute a workflow using the graph-based DAG engine.
    /// Nodes are executed in topological order following edges.
    /// Agent nodes call the LLM, Action nodes are deterministic.
    pub async fn execute_graph<F, Fut>(
        &self,
        run_id: WorkflowRunId,
        graph: &WorkflowGraph,
        input: &str,
        agent_resolver: impl Fn(&StepAgent) -> Option<(AgentId, String)>,
        send_message: F,
    ) -> Result<String, String>
    where
        F: Fn(AgentId, String) -> Fut,
        Fut: std::future::Future<Output = Result<(String, u64, u64), String>>,
    {
        let mut node_outputs: HashMap<String, NodeOutput> = HashMap::new();
        let mut variables: HashMap<String, String> = HashMap::new();
        let trigger_input = Self::graph_trigger_output("trigger".to_string(), input);
        let order = Self::topological_sort(graph)?;

        info!(
            run_id = %run_id,
            nodes = order.len(),
            "Starting graph workflow execution"
        );

        for node_id in &order {
            let node = Self::graph_node(graph, node_id)?;
            let (incoming, node_input) =
                Self::graph_inputs(graph, node_id, &node_outputs, &trigger_input);

            debug!(node = %node.label, node_type = ?node.node_type, "Executing graph node");

            match self
                .execute_graph_node(
                    run_id,
                    node,
                    input,
                    node_input,
                    &incoming,
                    &mut variables,
                    &agent_resolver,
                    &send_message,
                )
                .await
            {
                Ok(output) => {
                    node_outputs.insert(node.id.clone(), output);
                }
                Err(e) => {
                    let output = Self::graph_error_output(node, e)?;
                    node_outputs.insert(node.id.clone(), output);
                }
            }
        }

        let final_output = Self::graph_final_output(graph, &node_outputs);
        if let Some(r) = self.runs.write().await.get_mut(&run_id) {
            r.state = WorkflowRunState::Completed;
            r.output = Some(final_output.clone());
            r.completed_at = Some(Utc::now());
        }

        info!(run_id = %run_id, "Graph workflow completed");
        Ok(final_output)
    }

    fn graph_trigger_output(source_node: String, input: &str) -> NodeOutput {
        NodeOutput {
            json: serde_json::json!({ "input": input }),
            binary: None,
            source_node,
        }
    }

    fn graph_node<'a>(graph: &'a WorkflowGraph, node_id: &str) -> Result<&'a WorkflowNode, String> {
        graph
            .nodes
            .iter()
            .find(|node| node.id == node_id)
            .ok_or_else(|| format!("Node '{}' not found in graph", node_id))
    }

    fn graph_inputs<'a>(
        graph: &'a WorkflowGraph,
        node_id: &str,
        node_outputs: &'a HashMap<String, NodeOutput>,
        trigger_input: &'a NodeOutput,
    ) -> (Vec<&'a NodeOutput>, &'a NodeOutput) {
        let incoming: Vec<&NodeOutput> = graph
            .edges
            .iter()
            .filter(|edge| edge.target == node_id)
            .filter_map(|edge| node_outputs.get(&edge.source))
            .collect();
        let node_input = incoming.first().copied().unwrap_or(trigger_input);
        (incoming, node_input)
    }

    #[allow(clippy::too_many_arguments)]
    async fn execute_graph_node<F, Fut>(
        &self,
        run_id: WorkflowRunId,
        node: &WorkflowNode,
        input: &str,
        node_input: &NodeOutput,
        incoming: &[&NodeOutput],
        variables: &mut HashMap<String, String>,
        agent_resolver: &impl Fn(&StepAgent) -> Option<(AgentId, String)>,
        send_message: &F,
    ) -> Result<NodeOutput, String>
    where
        F: Fn(AgentId, String) -> Fut,
        Fut: std::future::Future<Output = Result<(String, u64, u64), String>>,
    {
        match &node.config {
            NodeConfig::Trigger { .. } => Ok(Self::graph_trigger_output(node.id.clone(), input)),
            NodeConfig::Agent {
                agent,
                prompt_template,
                output_var,
            } => {
                self.execute_graph_agent_node(
                    run_id,
                    node,
                    agent,
                    prompt_template,
                    output_var,
                    node_input,
                    variables,
                    agent_resolver,
                    send_message,
                )
                .await
            }
            NodeConfig::Action {
                action_type,
                params,
            } => {
                Self::execute_graph_action_node(node, action_type, params, node_input, variables)
                    .await
            }
            NodeConfig::Logic {
                logic_type,
                condition,
                ..
            } => Ok(Self::execute_graph_logic_node(
                node, logic_type, condition, incoming, node_input,
            )),
            NodeConfig::Human {
                question, options, ..
            } => {
                warn!(question = %question, "Human node reached — auto-passing (not yet interactive)");
                Ok(NodeOutput {
                    json: serde_json::json!({ "output": format!("[Human input needed: {}]", question), "options": options }),
                    binary: None,
                    source_node: node.id.clone(),
                })
            }
            NodeConfig::SubWorkflow { workflow_id } => {
                warn!(workflow_id = %workflow_id, "SubWorkflow node not yet implemented");
                Ok(NodeOutput {
                    json: node_input.json.clone(),
                    binary: None,
                    source_node: node.id.clone(),
                })
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    async fn execute_graph_agent_node<F, Fut>(
        &self,
        run_id: WorkflowRunId,
        node: &WorkflowNode,
        agent: &StepAgent,
        prompt_template: &str,
        output_var: &Option<String>,
        node_input: &NodeOutput,
        variables: &mut HashMap<String, String>,
        agent_resolver: &impl Fn(&StepAgent) -> Option<(AgentId, String)>,
        send_message: &F,
    ) -> Result<NodeOutput, String>
    where
        F: Fn(AgentId, String) -> Fut,
        Fut: std::future::Future<Output = Result<(String, u64, u64), String>>,
    {
        let (agent_id, agent_name) = agent_resolver(agent)
            .ok_or_else(|| format!("Agent not found for node '{}'", node.label))?;
        let input_text = node_input
            .json
            .get("input")
            .or_else(|| node_input.json.get("output"))
            .and_then(|value| value.as_str())
            .unwrap_or("");
        let prompt = Self::expand_variables(prompt_template, input_text, variables);

        let start = std::time::Instant::now();
        let (output, input_tokens, output_tokens) = send_message(agent_id, prompt)
            .await
            .map_err(|e| format!("Agent node '{}' failed: {}", node.label, e))?;
        let duration_ms = start.elapsed().as_millis() as u64;

        if let Some(r) = self.runs.write().await.get_mut(&run_id) {
            r.step_results.push(StepResult {
                step_name: node.label.clone(),
                agent_id: agent_id.to_string(),
                agent_name,
                output: output.clone(),
                input_tokens,
                output_tokens,
                duration_ms,
            });
        }

        if let Some(var) = output_var {
            variables.insert(var.clone(), output.clone());
        }

        Ok(NodeOutput {
            json: serde_json::json!({ "output": output }),
            binary: None,
            source_node: node.id.clone(),
        })
    }

    async fn execute_graph_action_node(
        node: &WorkflowNode,
        action_type: &ActionType,
        params: &serde_json::Value,
        node_input: &NodeOutput,
        variables: &mut HashMap<String, String>,
    ) -> Result<NodeOutput, String> {
        let output = match action_type {
            ActionType::Http => Self::execute_graph_http_action(params).await?,
            ActionType::Shell => Self::execute_graph_shell_action(params).await?,
            ActionType::Transform => serde_json::to_string(&node_input.json).unwrap_or_default(),
            ActionType::SetVariable => {
                Self::execute_graph_set_variable_action(params, node_input, variables)
            }
            ActionType::File => Self::execute_graph_file_action(params)?,
        };

        Ok(NodeOutput {
            json: serde_json::json!({ "output": output }),
            binary: None,
            source_node: node.id.clone(),
        })
    }

    async fn execute_graph_http_action(params: &serde_json::Value) -> Result<String, String> {
        let url = params
            .get("url")
            .and_then(|value| value.as_str())
            .unwrap_or("");
        let method = params
            .get("method")
            .and_then(|value| value.as_str())
            .unwrap_or("GET");
        let body = params
            .get("body")
            .and_then(|value| value.as_str())
            .unwrap_or("");

        let client = reqwest::Client::new();
        let req = match method.to_uppercase().as_str() {
            "POST" => client.post(url).body(body.to_string()),
            "PUT" => client.put(url).body(body.to_string()),
            _ => client.get(url),
        };
        let resp = req.send().await.map_err(|e| format!("HTTP error: {e}"))?;
        resp.text()
            .await
            .map_err(|e| format!("HTTP read error: {e}"))
    }

    async fn execute_graph_shell_action(params: &serde_json::Value) -> Result<String, String> {
        let cmd = params
            .get("command")
            .and_then(|value| value.as_str())
            .unwrap_or("");
        let output = tokio::process::Command::new("sh")
            .arg("-c")
            .arg(cmd)
            .output()
            .await
            .map_err(|e| format!("Shell error: {e}"))?;
        Ok(String::from_utf8_lossy(&output.stdout).to_string())
    }

    fn execute_graph_set_variable_action(
        params: &serde_json::Value,
        node_input: &NodeOutput,
        variables: &mut HashMap<String, String>,
    ) -> String {
        let var_name = params
            .get("name")
            .and_then(|value| value.as_str())
            .unwrap_or("var");
        let var_value = params
            .get("value")
            .and_then(|value| value.as_str())
            .unwrap_or_else(|| {
                node_input
                    .json
                    .get("output")
                    .and_then(|value| value.as_str())
                    .unwrap_or("")
            });
        variables.insert(var_name.to_string(), var_value.to_string());
        var_value.to_string()
    }

    fn execute_graph_file_action(params: &serde_json::Value) -> Result<String, String> {
        let path = params
            .get("path")
            .and_then(|value| value.as_str())
            .unwrap_or("");
        let action = params
            .get("action")
            .and_then(|value| value.as_str())
            .unwrap_or("read");
        match action {
            "read" => std::fs::read_to_string(path).map_err(|e| format!("File read error: {e}")),
            "write" => {
                let content = params
                    .get("content")
                    .and_then(|value| value.as_str())
                    .unwrap_or("");
                std::fs::write(path, content).map_err(|e| format!("File write error: {e}"))?;
                Ok(format!("Written to {path}"))
            }
            _ => Err(format!("Unknown file action: {action}")),
        }
    }

    fn execute_graph_logic_node(
        node: &WorkflowNode,
        logic_type: &LogicType,
        condition: &str,
        incoming: &[&NodeOutput],
        node_input: &NodeOutput,
    ) -> NodeOutput {
        let json = match logic_type {
            LogicType::IfThen => {
                let input_text = node_input
                    .json
                    .get("output")
                    .and_then(|value| value.as_str())
                    .unwrap_or("");
                let result = input_text
                    .to_lowercase()
                    .contains(&condition.to_lowercase());
                serde_json::json!({ "result": result, "output": input_text })
            }
            LogicType::Merge => {
                let merged: Vec<serde_json::Value> =
                    incoming.iter().map(|input| input.json.clone()).collect();
                serde_json::json!({ "merged": merged })
            }
            _ => node_input.json.clone(),
        };

        NodeOutput {
            json,
            binary: None,
            source_node: node.id.clone(),
        }
    }

    fn graph_error_output(node: &WorkflowNode, error: String) -> Result<NodeOutput, String> {
        match node.error_mode {
            ErrorMode::Skip => {
                warn!(node = %node.label, error = %error, "Node failed, skipping");
                Ok(NodeOutput {
                    json: serde_json::json!({ "error": error, "skipped": true }),
                    binary: None,
                    source_node: node.id.clone(),
                })
            }
            ErrorMode::Retry { max_retries } => {
                warn!(node = %node.label, error = %error, max_retries, "Node failed, retry not implemented for graph (falling through)");
                Err(format!("Node '{}' failed: {}", node.label, error))
            }
            ErrorMode::Fail => Err(format!("Node '{}' failed: {}", node.label, error)),
        }
    }

    fn graph_final_output(
        graph: &WorkflowGraph,
        node_outputs: &HashMap<String, NodeOutput>,
    ) -> String {
        graph
            .nodes
            .iter()
            .filter(|node| !graph.edges.iter().any(|edge| edge.source == node.id))
            .filter_map(|node| node_outputs.get(node.id.as_str()))
            .map(|output| {
                output
                    .json
                    .get("output")
                    .and_then(|value| value.as_str())
                    .unwrap_or("")
                    .to_string()
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    /// Topological sort of the graph nodes (Kahn's algorithm).
    fn topological_sort(graph: &WorkflowGraph) -> Result<Vec<String>, String> {
        let mut in_degree: HashMap<String, usize> = HashMap::new();
        let mut adj: HashMap<String, Vec<String>> = HashMap::new();

        for node in &graph.nodes {
            in_degree.entry(node.id.clone()).or_insert(0);
            adj.entry(node.id.clone()).or_default();
        }

        for edge in &graph.edges {
            *in_degree.entry(edge.target.clone()).or_insert(0) += 1;
            adj.entry(edge.source.clone())
                .or_default()
                .push(edge.target.clone());
        }

        let mut queue: Vec<String> = in_degree
            .iter()
            .filter(|(_, &deg)| deg == 0)
            .map(|(id, _)| id.clone())
            .collect();
        queue.sort(); // Deterministic order

        let mut result = Vec::new();
        while let Some(node) = queue.pop() {
            result.push(node.clone());
            if let Some(neighbors) = adj.get(&node) {
                for next in neighbors {
                    if let Some(deg) = in_degree.get_mut(next) {
                        *deg -= 1;
                        if *deg == 0 {
                            queue.push(next.clone());
                        }
                    }
                }
            }
        }

        if result.len() != graph.nodes.len() {
            return Err("Cycle detected in workflow graph".to_string());
        }

        Ok(result)
    }
}

impl Default for WorkflowEngine {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_workflow() -> Workflow {
        Workflow {
            id: WorkflowId::new(),
            name: "test-pipeline".to_string(),
            description: "A test pipeline".to_string(),
            steps: vec![
                WorkflowStep {
                    name: "analyze".to_string(),
                    agent: StepAgent::ByName {
                        name: "analyst".to_string(),
                    },
                    prompt_template: "Analyze this: {{input}}".to_string(),
                    mode: StepMode::Sequential,
                    timeout_secs: 30,
                    error_mode: ErrorMode::Fail,
                    output_var: None,
                },
                WorkflowStep {
                    name: "summarize".to_string(),
                    agent: StepAgent::ByName {
                        name: "writer".to_string(),
                    },
                    prompt_template: "Summarize this analysis: {{input}}".to_string(),
                    mode: StepMode::Sequential,
                    timeout_secs: 30,
                    error_mode: ErrorMode::Fail,
                    output_var: None,
                },
            ],
            graph: None,
            created_at: Utc::now(),
        }
    }

    fn mock_resolver(agent: &StepAgent) -> Option<(AgentId, String)> {
        let _ = agent;
        Some((AgentId::new(), "mock-agent".to_string()))
    }

    #[tokio::test]
    async fn test_register_workflow() {
        let engine = WorkflowEngine::new();
        let wf = test_workflow();
        let id = engine.register(wf.clone()).await;
        assert_eq!(id, wf.id);

        let retrieved = engine.get_workflow(id).await;
        assert!(retrieved.is_some());
        assert_eq!(retrieved.unwrap().name, "test-pipeline");
    }

    #[tokio::test]
    async fn test_create_run() {
        let engine = WorkflowEngine::new();
        let wf = test_workflow();
        let wf_id = engine.register(wf).await;

        let run_id = engine.create_run(wf_id, "test input".to_string()).await;
        assert!(run_id.is_some());

        let run = engine.get_run(run_id.unwrap()).await.unwrap();
        assert_eq!(run.input, "test input");
        assert!(matches!(run.state, WorkflowRunState::Pending));
    }

    #[tokio::test]
    async fn test_list_workflows() {
        let engine = WorkflowEngine::new();
        let wf = test_workflow();
        engine.register(wf).await;

        let list = engine.list_workflows().await;
        assert_eq!(list.len(), 1);
    }

    #[tokio::test]
    async fn test_remove_workflow() {
        let engine = WorkflowEngine::new();
        let wf = test_workflow();
        let id = engine.register(wf).await;

        assert!(engine.remove_workflow(id).await);
        assert!(engine.get_workflow(id).await.is_none());
    }

    #[tokio::test]
    async fn test_execute_pipeline() {
        let engine = WorkflowEngine::new();
        let wf = test_workflow();
        let wf_id = engine.register(wf).await;
        let run_id = engine
            .create_run(wf_id, "raw data".to_string())
            .await
            .unwrap();

        let sender = |_id: AgentId, msg: String| async move {
            Ok((format!("Processed: {msg}"), 100u64, 50u64))
        };

        let result = engine.execute_run(run_id, mock_resolver, sender).await;
        assert!(result.is_ok());

        let output = result.unwrap();
        assert!(output.contains("Processed:"));

        let run = engine.get_run(run_id).await.unwrap();
        assert!(matches!(run.state, WorkflowRunState::Completed));
        assert_eq!(run.step_results.len(), 2);
        assert!(run.output.is_some());
    }

    #[tokio::test]
    async fn test_conditional_skip() {
        let engine = WorkflowEngine::new();
        let wf = Workflow {
            id: WorkflowId::new(),
            name: "conditional-test".to_string(),
            description: "".to_string(),
            steps: vec![
                WorkflowStep {
                    name: "first".to_string(),
                    agent: StepAgent::ByName {
                        name: "a".to_string(),
                    },
                    prompt_template: "{{input}}".to_string(),
                    mode: StepMode::Sequential,
                    timeout_secs: 10,
                    error_mode: ErrorMode::Fail,
                    output_var: None,
                },
                WorkflowStep {
                    name: "only-if-error".to_string(),
                    agent: StepAgent::ByName {
                        name: "a".to_string(),
                    },
                    prompt_template: "Fix: {{input}}".to_string(),
                    mode: StepMode::Conditional {
                        condition: "ERROR".to_string(),
                    },
                    timeout_secs: 10,
                    error_mode: ErrorMode::Fail,
                    output_var: None,
                },
            ],
            graph: None,
            created_at: Utc::now(),
        };
        let wf_id = engine.register(wf).await;
        let run_id = engine
            .create_run(wf_id, "all good".to_string())
            .await
            .unwrap();

        let sender =
            |_id: AgentId, msg: String| async move { Ok((format!("OK: {msg}"), 10u64, 5u64)) };

        let result = engine.execute_run(run_id, mock_resolver, sender).await;
        assert!(result.is_ok());

        let run = engine.get_run(run_id).await.unwrap();
        // Only 1 step executed (conditional was skipped)
        assert_eq!(run.step_results.len(), 1);
    }

    #[tokio::test]
    async fn test_conditional_executes() {
        let engine = WorkflowEngine::new();
        let wf = Workflow {
            id: WorkflowId::new(),
            name: "conditional-test".to_string(),
            description: "".to_string(),
            steps: vec![
                WorkflowStep {
                    name: "first".to_string(),
                    agent: StepAgent::ByName {
                        name: "a".to_string(),
                    },
                    prompt_template: "{{input}}".to_string(),
                    mode: StepMode::Sequential,
                    timeout_secs: 10,
                    error_mode: ErrorMode::Fail,
                    output_var: None,
                },
                WorkflowStep {
                    name: "only-if-error".to_string(),
                    agent: StepAgent::ByName {
                        name: "a".to_string(),
                    },
                    prompt_template: "Fix: {{input}}".to_string(),
                    mode: StepMode::Conditional {
                        condition: "ERROR".to_string(),
                    },
                    timeout_secs: 10,
                    error_mode: ErrorMode::Fail,
                    output_var: None,
                },
            ],
            graph: None,
            created_at: Utc::now(),
        };
        let wf_id = engine.register(wf).await;
        let run_id = engine.create_run(wf_id, "data".to_string()).await.unwrap();

        // This sender returns output containing "ERROR"
        let sender = |_id: AgentId, _msg: String| async move {
            Ok(("Found an ERROR in the data".to_string(), 10u64, 5u64))
        };

        let result = engine.execute_run(run_id, mock_resolver, sender).await;
        assert!(result.is_ok());

        let run = engine.get_run(run_id).await.unwrap();
        // Both steps executed
        assert_eq!(run.step_results.len(), 2);
    }

    #[tokio::test]
    async fn test_loop_until_condition() {
        let engine = WorkflowEngine::new();
        let wf = Workflow {
            id: WorkflowId::new(),
            name: "loop-test".to_string(),
            description: "".to_string(),
            steps: vec![WorkflowStep {
                name: "refine".to_string(),
                agent: StepAgent::ByName {
                    name: "a".to_string(),
                },
                prompt_template: "Refine: {{input}}".to_string(),
                mode: StepMode::Loop {
                    max_iterations: 5,
                    until: "DONE".to_string(),
                },
                timeout_secs: 10,
                error_mode: ErrorMode::Fail,
                output_var: None,
            }],
            graph: None,
            created_at: Utc::now(),
        };
        let wf_id = engine.register(wf).await;
        let run_id = engine.create_run(wf_id, "draft".to_string()).await.unwrap();

        let call_count = Arc::new(std::sync::atomic::AtomicU32::new(0));
        let cc = call_count.clone();
        let sender = move |_id: AgentId, _msg: String| {
            let cc = cc.clone();
            async move {
                let n = cc.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                if n >= 2 {
                    Ok(("Result: DONE".to_string(), 10u64, 5u64))
                } else {
                    Ok(("Still working...".to_string(), 10u64, 5u64))
                }
            }
        };

        let result = engine.execute_run(run_id, mock_resolver, sender).await;
        assert!(result.is_ok());
        assert!(result.unwrap().contains("DONE"));
        assert_eq!(call_count.load(std::sync::atomic::Ordering::SeqCst), 3);
    }

    #[tokio::test]
    async fn test_loop_max_iterations() {
        let engine = WorkflowEngine::new();
        let wf = Workflow {
            id: WorkflowId::new(),
            name: "loop-max-test".to_string(),
            description: "".to_string(),
            steps: vec![WorkflowStep {
                name: "refine".to_string(),
                agent: StepAgent::ByName {
                    name: "a".to_string(),
                },
                prompt_template: "{{input}}".to_string(),
                mode: StepMode::Loop {
                    max_iterations: 3,
                    until: "NEVER_MATCH".to_string(),
                },
                timeout_secs: 10,
                error_mode: ErrorMode::Fail,
                output_var: None,
            }],
            graph: None,
            created_at: Utc::now(),
        };
        let wf_id = engine.register(wf).await;
        let run_id = engine.create_run(wf_id, "data".to_string()).await.unwrap();

        let sender = |_id: AgentId, _msg: String| async move {
            Ok(("iteration output".to_string(), 10u64, 5u64))
        };

        let result = engine.execute_run(run_id, mock_resolver, sender).await;
        assert!(result.is_ok());

        let run = engine.get_run(run_id).await.unwrap();
        assert_eq!(run.step_results.len(), 3); // max_iterations
    }

    #[tokio::test]
    async fn test_error_mode_skip() {
        let engine = WorkflowEngine::new();
        let wf = Workflow {
            id: WorkflowId::new(),
            name: "skip-test".to_string(),
            description: "".to_string(),
            steps: vec![
                WorkflowStep {
                    name: "will-fail".to_string(),
                    agent: StepAgent::ByName {
                        name: "a".to_string(),
                    },
                    prompt_template: "{{input}}".to_string(),
                    mode: StepMode::Sequential,
                    timeout_secs: 10,
                    error_mode: ErrorMode::Skip,
                    output_var: None,
                },
                WorkflowStep {
                    name: "succeeds".to_string(),
                    agent: StepAgent::ByName {
                        name: "a".to_string(),
                    },
                    prompt_template: "{{input}}".to_string(),
                    mode: StepMode::Sequential,
                    timeout_secs: 10,
                    error_mode: ErrorMode::Fail,
                    output_var: None,
                },
            ],
            graph: None,
            created_at: Utc::now(),
        };
        let wf_id = engine.register(wf).await;
        let run_id = engine.create_run(wf_id, "data".to_string()).await.unwrap();

        let call_count = Arc::new(std::sync::atomic::AtomicU32::new(0));
        let cc = call_count.clone();
        let sender = move |_id: AgentId, _msg: String| {
            let cc = cc.clone();
            async move {
                let n = cc.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                if n == 0 {
                    Err("simulated error".to_string())
                } else {
                    Ok(("success".to_string(), 10u64, 5u64))
                }
            }
        };

        let result = engine.execute_run(run_id, mock_resolver, sender).await;
        assert!(result.is_ok());

        let run = engine.get_run(run_id).await.unwrap();
        // Only 1 step result (the first was skipped due to error)
        assert_eq!(run.step_results.len(), 1);
        assert!(matches!(run.state, WorkflowRunState::Completed));
    }

    #[tokio::test]
    async fn test_error_mode_retry() {
        let engine = WorkflowEngine::new();
        let wf = Workflow {
            id: WorkflowId::new(),
            name: "retry-test".to_string(),
            description: "".to_string(),
            steps: vec![WorkflowStep {
                name: "flaky".to_string(),
                agent: StepAgent::ByName {
                    name: "a".to_string(),
                },
                prompt_template: "{{input}}".to_string(),
                mode: StepMode::Sequential,
                timeout_secs: 10,
                error_mode: ErrorMode::Retry { max_retries: 2 },
                output_var: None,
            }],
            graph: None,
            created_at: Utc::now(),
        };
        let wf_id = engine.register(wf).await;
        let run_id = engine.create_run(wf_id, "data".to_string()).await.unwrap();

        let call_count = Arc::new(std::sync::atomic::AtomicU32::new(0));
        let cc = call_count.clone();
        let sender = move |_id: AgentId, _msg: String| {
            let cc = cc.clone();
            async move {
                let n = cc.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                if n < 2 {
                    Err("transient error".to_string())
                } else {
                    Ok(("finally worked".to_string(), 10u64, 5u64))
                }
            }
        };

        let result = engine.execute_run(run_id, mock_resolver, sender).await;
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), "finally worked");
        assert_eq!(call_count.load(std::sync::atomic::Ordering::SeqCst), 3);
    }

    #[tokio::test]
    async fn test_output_variables() {
        let engine = WorkflowEngine::new();
        let wf = Workflow {
            id: WorkflowId::new(),
            name: "vars-test".to_string(),
            description: "".to_string(),
            steps: vec![
                WorkflowStep {
                    name: "produce".to_string(),
                    agent: StepAgent::ByName {
                        name: "a".to_string(),
                    },
                    prompt_template: "{{input}}".to_string(),
                    mode: StepMode::Sequential,
                    timeout_secs: 10,
                    error_mode: ErrorMode::Fail,
                    output_var: Some("first_result".to_string()),
                },
                WorkflowStep {
                    name: "transform".to_string(),
                    agent: StepAgent::ByName {
                        name: "a".to_string(),
                    },
                    prompt_template: "{{input}}".to_string(),
                    mode: StepMode::Sequential,
                    timeout_secs: 10,
                    error_mode: ErrorMode::Fail,
                    output_var: Some("second_result".to_string()),
                },
                WorkflowStep {
                    name: "combine".to_string(),
                    agent: StepAgent::ByName {
                        name: "a".to_string(),
                    },
                    prompt_template: "First: {{first_result}} | Second: {{second_result}}"
                        .to_string(),
                    mode: StepMode::Sequential,
                    timeout_secs: 10,
                    error_mode: ErrorMode::Fail,
                    output_var: None,
                },
            ],
            graph: None,
            created_at: Utc::now(),
        };
        let wf_id = engine.register(wf).await;
        let run_id = engine.create_run(wf_id, "start".to_string()).await.unwrap();

        let call_count = Arc::new(std::sync::atomic::AtomicU32::new(0));
        let cc = call_count.clone();
        let sender = move |_id: AgentId, msg: String| {
            let cc = cc.clone();
            async move {
                let n = cc.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                match n {
                    0 => Ok(("alpha".to_string(), 10u64, 5u64)),
                    1 => Ok(("beta".to_string(), 10u64, 5u64)),
                    _ => Ok((format!("Combined: {msg}"), 10u64, 5u64)),
                }
            }
        };

        let result = engine.execute_run(run_id, mock_resolver, sender).await;
        assert!(result.is_ok());
        let output = result.unwrap();
        // The third step receives "First: alpha | Second: beta" as its prompt
        assert!(output.contains("First: alpha"));
        assert!(output.contains("Second: beta"));
    }

    #[tokio::test]
    async fn test_fan_out_parallel() {
        let engine = WorkflowEngine::new();
        let wf = Workflow {
            id: WorkflowId::new(),
            name: "fanout-test".to_string(),
            description: "".to_string(),
            steps: vec![
                WorkflowStep {
                    name: "task-a".to_string(),
                    agent: StepAgent::ByName {
                        name: "a".to_string(),
                    },
                    prompt_template: "Task A: {{input}}".to_string(),
                    mode: StepMode::FanOut,
                    timeout_secs: 10,
                    error_mode: ErrorMode::Fail,
                    output_var: None,
                },
                WorkflowStep {
                    name: "task-b".to_string(),
                    agent: StepAgent::ByName {
                        name: "b".to_string(),
                    },
                    prompt_template: "Task B: {{input}}".to_string(),
                    mode: StepMode::FanOut,
                    timeout_secs: 10,
                    error_mode: ErrorMode::Fail,
                    output_var: None,
                },
                WorkflowStep {
                    name: "collect".to_string(),
                    agent: StepAgent::ByName {
                        name: "c".to_string(),
                    },
                    prompt_template: "unused".to_string(),
                    mode: StepMode::Collect,
                    timeout_secs: 10,
                    error_mode: ErrorMode::Fail,
                    output_var: Some("joined".to_string()),
                },
                WorkflowStep {
                    name: "summarize".to_string(),
                    agent: StepAgent::ByName {
                        name: "c".to_string(),
                    },
                    prompt_template: "Summary: {{joined}}".to_string(),
                    mode: StepMode::Sequential,
                    timeout_secs: 10,
                    error_mode: ErrorMode::Fail,
                    output_var: None,
                },
            ],
            graph: None,
            created_at: Utc::now(),
        };
        let wf_id = engine.register(wf).await;
        let run_id = engine.create_run(wf_id, "data".to_string()).await.unwrap();

        let sender =
            |_id: AgentId, msg: String| async move { Ok((format!("Done: {msg}"), 10u64, 5u64)) };

        let result = engine.execute_run(run_id, mock_resolver, sender).await;
        assert!(result.is_ok());

        let output = result.unwrap();
        // Collect step joins all outputs and exposes them to the following step.
        assert!(output.contains("Done: Summary:"));
        assert!(output.contains("Done: Task A"));
        assert!(output.contains("Done: Task B"));
        assert!(output.contains("---"));

        let run = engine.get_run(run_id).await.unwrap();
        assert_eq!(run.step_results.len(), 3);
    }

    #[tokio::test]
    async fn test_graph_execution_preserves_agent_and_action_variables() {
        let engine = WorkflowEngine::new();
        let wf = Workflow {
            id: WorkflowId::new(),
            name: "graph-test".to_string(),
            description: "".to_string(),
            steps: vec![],
            graph: None,
            created_at: Utc::now(),
        };
        let wf_id = engine.register(wf).await;
        let run_id = engine
            .create_run(wf_id, "graph input".to_string())
            .await
            .unwrap();

        let node = |id: &str, label: &str, node_type: NodeType, config: NodeConfig| WorkflowNode {
            id: id.to_string(),
            label: label.to_string(),
            node_type,
            config,
            position: NodePosition::default(),
            error_mode: ErrorMode::Fail,
            timeout_secs: 0,
        };
        let edge = |source: &str, target: &str| WorkflowEdge {
            id: format!("{source}-{target}"),
            source: source.to_string(),
            target: target.to_string(),
            edge_type: EdgeType::Normal,
            label: String::new(),
        };

        let graph = WorkflowGraph {
            nodes: vec![
                node(
                    "trigger",
                    "Trigger",
                    NodeType::Trigger,
                    NodeConfig::Trigger {
                        trigger_type: TriggerType::Manual,
                        cron_expr: String::new(),
                        webhook_path: String::new(),
                    },
                ),
                node(
                    "agent-1",
                    "First agent",
                    NodeType::Agent,
                    NodeConfig::Agent {
                        agent: StepAgent::ByName { name: "a".into() },
                        prompt_template: "First {{input}}".to_string(),
                        output_var: Some("first".to_string()),
                    },
                ),
                node(
                    "set-var",
                    "Set variable",
                    NodeType::Action,
                    NodeConfig::Action {
                        action_type: ActionType::SetVariable,
                        params: serde_json::json!({
                            "name": "saved",
                            "value": "from-action"
                        }),
                    },
                ),
                node(
                    "agent-2",
                    "Final agent",
                    NodeType::Agent,
                    NodeConfig::Agent {
                        agent: StepAgent::ByName { name: "a".into() },
                        prompt_template: "Final {{first}} {{saved}} {{input}}".to_string(),
                        output_var: None,
                    },
                ),
            ],
            edges: vec![
                edge("trigger", "agent-1"),
                edge("agent-1", "set-var"),
                edge("set-var", "agent-2"),
            ],
        };

        let prompts = Arc::new(std::sync::Mutex::new(Vec::new()));
        let observed = prompts.clone();
        let sender = move |_id: AgentId, msg: String| {
            let observed = observed.clone();
            async move {
                observed.lock().unwrap().push(msg.clone());
                if msg.starts_with("First ") {
                    Ok(("alpha".to_string(), 10u64, 5u64))
                } else {
                    Ok((format!("final:{msg}"), 10u64, 5u64))
                }
            }
        };

        let output = engine
            .execute_graph(run_id, &graph, "graph input", mock_resolver, sender)
            .await
            .unwrap();

        assert_eq!(output, "final:Final alpha from-action from-action");
        let prompts = prompts.lock().unwrap();
        assert_eq!(prompts[0], "First graph input");
        assert_eq!(prompts[1], "Final alpha from-action from-action");

        let run = engine.get_run(run_id).await.unwrap();
        assert!(matches!(run.state, WorkflowRunState::Completed));
        assert_eq!(run.step_results.len(), 2);
    }

    #[tokio::test]
    async fn test_expand_variables() {
        let mut vars = HashMap::new();
        vars.insert("name".to_string(), "Alice".to_string());
        vars.insert("task".to_string(), "code review".to_string());

        let result = WorkflowEngine::expand_variables(
            "Hello {{name}}, please do {{task}} on {{input}}",
            "main.rs",
            &vars,
        );
        assert_eq!(result, "Hello Alice, please do code review on main.rs");
    }

    #[tokio::test]
    async fn test_error_mode_serialization() {
        let fail_json = serde_json::to_string(&ErrorMode::Fail).unwrap();
        assert_eq!(fail_json, "\"fail\"");

        let skip_json = serde_json::to_string(&ErrorMode::Skip).unwrap();
        assert_eq!(skip_json, "\"skip\"");

        let retry_json = serde_json::to_string(&ErrorMode::Retry { max_retries: 3 }).unwrap();
        let retry: ErrorMode = serde_json::from_str(&retry_json).unwrap();
        assert!(matches!(retry, ErrorMode::Retry { max_retries: 3 }));
    }

    #[tokio::test]
    async fn test_step_mode_conditional_serialization() {
        let mode = StepMode::Conditional {
            condition: "error".to_string(),
        };
        let json = serde_json::to_string(&mode).unwrap();
        let parsed: StepMode = serde_json::from_str(&json).unwrap();
        assert!(matches!(parsed, StepMode::Conditional { condition } if condition == "error"));
    }

    #[tokio::test]
    async fn test_step_mode_loop_serialization() {
        let mode = StepMode::Loop {
            max_iterations: 5,
            until: "done".to_string(),
        };
        let json = serde_json::to_string(&mode).unwrap();
        let parsed: StepMode = serde_json::from_str(&json).unwrap();
        assert!(matches!(parsed, StepMode::Loop { max_iterations: 5, until } if until == "done"));
    }
}
