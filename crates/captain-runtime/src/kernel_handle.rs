//! Trait abstraction for kernel operations needed by the agent runtime.
//!
//! This trait allows `captain-runtime` to call back into the kernel for
//! inter-agent operations (spawn, send, list, kill) without creating
//! a circular dependency. The kernel implements this trait and passes
//! it into the agent loop.

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use crate::memory_retractions::{load_retractions, MemoryRetraction, MEMORY_RETRACTIONS_KEY};

pub const SKILL_PROPOSAL_APPROVAL_VERIFICATION: &str = "schema_diff_tests_human";

pub fn skill_proposal_approval_decider(decided_by: &str) -> String {
    format!(
        "{}:{}",
        decided_by.trim(),
        SKILL_PROPOSAL_APPROVAL_VERIFICATION
    )
}

pub fn skill_proposal_decider_has_external_validation(decided_by: Option<&str>) -> bool {
    let Some(raw) = decided_by.map(str::trim).filter(|value| !value.is_empty()) else {
        return false;
    };
    let Some((label, verification)) = raw.split_once(':') else {
        return false;
    };
    !label.trim().is_empty() && verification.trim() == SKILL_PROPOSAL_APPROVAL_VERIFICATION
}

pub fn skill_proposal_decider_public_label(decided_by: Option<&str>) -> Option<String> {
    decided_by
        .map(str::trim)
        .and_then(|value| {
            value
                .split_once(':')
                .map(|(label, _)| label)
                .or(Some(value))
        })
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToString::to_string)
}

#[cfg(test)]
mod skill_proposal_approval_tests {
    use super::{
        skill_proposal_approval_decider, skill_proposal_decider_has_external_validation,
        skill_proposal_decider_public_label,
    };

    #[test]
    fn approval_decider_marks_external_validation() {
        let decided_by = skill_proposal_approval_decider("web");

        assert_eq!(decided_by, "web:schema_diff_tests_human");
        assert!(skill_proposal_decider_has_external_validation(Some(
            &decided_by
        )));
    }

    #[test]
    fn public_label_strips_external_validation_marker() {
        assert_eq!(
            skill_proposal_decider_public_label(Some("channel:schema_diff_tests_human")),
            Some("channel".to_string())
        );
        assert!(!skill_proposal_decider_has_external_validation(Some(
            "channel"
        )));
    }
}

/// Agent info returned by list and discovery operations.
#[derive(Debug, Clone)]
pub struct AgentInfo {
    pub id: String,
    pub name: String,
    pub state: String,
    pub model_provider: String,
    pub model_name: String,
    pub description: String,
    pub tags: Vec<String>,
    pub tools: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CapSpecForgeAction {
    List,
    Inspect,
    Validate,
    Propose,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CapSpecForgeScope {
    Effective,
    All,
    Global,
    Project,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CapSpecForgeRequest {
    pub action: CapSpecForgeAction,
    #[serde(default)]
    pub scope: Option<CapSpecForgeScope>,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub source: Option<String>,
    #[serde(default)]
    pub include_source: bool,
}

/// Handle to kernel operations, passed into the agent loop so agents
/// can interact with each other via tools.
#[allow(clippy::too_many_arguments)]
#[async_trait]
pub trait KernelHandle: Send + Sync {
    /// Spawn a new agent from a TOML manifest string.
    /// `parent_id` is the UUID string of the spawning agent (for lineage tracking).
    /// Returns (agent_id, agent_name) on success.
    async fn spawn_agent(
        &self,
        manifest_toml: &str,
        parent_id: Option<&str>,
    ) -> Result<(String, String), String>;

    /// Provision the dedicated external API surface for a newly spawned agent.
    ///
    /// Implementations should rotate the ingress bearer token by default and,
    /// when a callback URL is supplied, configure signed egress callbacks.
    async fn provision_spawned_agent_api(
        &self,
        _agent_id: &str,
        _request: captain_types::agent_api::AgentApiSpawnProvisionRequest,
    ) -> Result<captain_types::agent_api::AgentApiSpawnProvisionReport, String> {
        Err("agent API provisioning is not available on this kernel".to_string())
    }

    /// Send a message to another agent and get the response.
    async fn send_to_agent(&self, agent_id: &str, message: &str) -> Result<String, String>;

    /// List all running agents.
    fn list_agents(&self) -> Vec<AgentInfo>;

    /// Extra workspace roots a given agent is allowed to roam, on top of
    /// its primary workspace. Returns `Vec::new()` for ordinary agents;
    /// `CaptainKernel` returns `~/.captain/` + any user-declared paths
    /// for the principal `captain` agent. Used by `tool_runner` to widen
    /// the file sandbox when Captain reaches into its own home.
    fn additional_workspace_roots(
        &self,
        _caller_agent_id: Option<&str>,
    ) -> Vec<std::path::PathBuf> {
        Vec::new()
    }

    /// Hard blocklist that overrides every allowed root. `~/.ssh/`,
    /// `~/.gnupg/` and credential stores such as `~/.captain/secrets.env`
    /// live here so even Captain must use the typed/audited tools instead
    /// of raw file_read / file_write.
    fn blocked_workspace_paths(&self) -> Vec<std::path::PathBuf> {
        Vec::new()
    }

    /// Hard per-agent tool blocklist. This is enforced again at dispatch so
    /// hidden or composed calls cannot bypass catalog visibility.
    fn tool_is_blocked_for_agent(&self, _caller_agent_id: Option<&str>, _tool_name: &str) -> bool {
        false
    }

    /// Return the durable CapSpec executor after refreshing the workspace
    /// scope. Stub kernels expose no native capability runtime.
    fn capspec_executor_for_workspace(
        &self,
        _workspace: Option<&std::path::Path>,
    ) -> Result<Option<std::sync::Arc<captain_capspec::CapabilityExecutor>>, String> {
        Ok(None)
    }

    /// Return active CapSpec tool definitions visible in this workspace.
    fn capspec_tool_definitions(
        &self,
        _workspace: Option<&std::path::Path>,
    ) -> Result<Vec<captain_types::tool::ToolDefinition>, String> {
        Ok(Vec::new())
    }

    /// Validate, inspect, list or propose a native CapSpec. Deliberately no
    /// approval action is available through this agent-facing boundary.
    fn capspec_forge(
        &self,
        _request: &CapSpecForgeRequest,
        _workspace: Option<&std::path::Path>,
        _caller_agent_id: Option<&str>,
    ) -> Result<serde_json::Value, String> {
        Err("Captain Forge is not available on this kernel".to_string())
    }

    /// Persist a new authorized workspace path for the principal agent.
    /// Backs the `workspace_add` tool. Implementations canonicalize and
    /// write the value into config.toml's `[workspace] extra_paths`.
    /// Default returns an error so the tool fails clean on stub kernels.
    fn add_workspace_path(&self, _path: &std::path::Path) -> Result<(), String> {
        Err("add_workspace_path not implemented on this kernel".to_string())
    }

    /// Kill an agent by ID.
    fn kill_agent(&self, agent_id: &str) -> Result<(), String>;

    /// Create a Manager agent with its own domain, budget, and orchestration tools.
    /// Returns (manager_agent_id, manager_name).
    async fn create_manager(
        &self,
        name: &str,
        domain: &str,
        model: Option<&str>,
        budget_tokens: u64,
    ) -> Result<(String, String), String> {
        let _ = (name, domain, model, budget_tokens);
        Err("Not implemented".into())
    }

    /// List all active Manager agents with their fleet info.
    fn list_managers(&self) -> Vec<serde_json::Value> {
        vec![]
    }

    /// Close a Manager and all its workers.
    async fn close_manager(&self, manager_id: &str) -> Result<u32, String> {
        let _ = manager_id;
        Err("Not implemented".into())
    }

    /// Persist the current mission string for a manager (survives daemon reboot).
    fn set_manager_mission(&self, manager_id: &str, mission: Option<&str>) -> Result<(), String> {
        let _ = (manager_id, mission);
        Err("Not implemented".into())
    }

    /// Configure auto-scaling for a manager's fleet.
    fn configure_autoscale(
        &self,
        manager_id: &str,
        config: captain_types::agent::AutoScaleConfig,
    ) -> Result<(), String> {
        let _ = (manager_id, config);
        Err("Not implemented".into())
    }

    /// Compute current load metrics for a fleet.
    fn fleet_metrics(&self, manager_id: &str) -> Result<serde_json::Value, String> {
        let _ = manager_id;
        Err("Not implemented".into())
    }

    /// Check if an agent has exceeded its token quota. Called per iteration.
    fn check_agent_quota(&self, agent_id: &str) -> Result<(), String> {
        let _ = agent_id;
        Ok(())
    }

    /// Get detailed status for an agent (tokens, cost, errors, last activity).
    fn agent_status_info(&self, agent_id: &str) -> Result<serde_json::Value, String> {
        let _ = agent_id;
        Err("Not implemented".into())
    }

    /// Get recent events for an agent from the EventBus history.
    async fn agent_events(
        &self,
        agent_id: &str,
        limit: usize,
    ) -> Result<Vec<serde_json::Value>, String> {
        let _ = (agent_id, limit);
        Ok(vec![])
    }

    /// Full capability + budget report for an agent (declared and effective
    /// tools/network/shell/memory scopes, plus hourly/daily/monthly cost and
    /// token budget usage) — the same data `captain agent caps` shows, but
    /// callable directly by an agent instead of shelling out to the CLI.
    fn agent_capability_report(&self, agent_id: &str) -> Result<serde_json::Value, String> {
        let _ = agent_id;
        Err("Not implemented".into())
    }

    /// Summarize which tools an agent's own current session actually
    /// executed, from the persisted session event log (same substrate as
    /// `captain replay`). Lets an agent verify its own claims instead of
    /// asserting a capability was exercised without evidence.
    fn session_tool_call_summary(
        &self,
        agent_id: &str,
        limit: usize,
    ) -> Result<serde_json::Value, String> {
        let _ = (agent_id, limit);
        Err("Not implemented".into())
    }

    /// Inject a system-level correction message into an agent's session.
    async fn inject_system_message(&self, agent_id: &str, message: &str) -> Result<(), String> {
        let _ = (agent_id, message);
        Err("Not implemented".into())
    }

    /// Whether `agent_id` currently has an LLM turn in flight. Used to skip
    /// a redundant wake-up message when the agent is already actively
    /// working (e.g. it will notice a finished detached tool_run on its own
    /// next turn) rather than interrupting it. Defaults to `false` (assume
    /// idle) so callers without a concrete kernel handle still get woken up.
    fn agent_is_busy(&self, agent_id: &str) -> bool {
        let _ = agent_id;
        false
    }

    /// Publish a typed payload on the kernel's event bus, best-effort. Lets
    /// runtime code (e.g. detached tool_run completion) surface activity to
    /// TUI/SSE/agent-API-webhook subscribers without depending on the
    /// concrete kernel type. Distinct from `publish_event` (custom
    /// string-keyed events for proactive triggers) — this one carries a
    /// structured `EventPayload` variant. Defaults to a no-op.
    async fn publish_typed_event(&self, payload: captain_types::event::EventPayload) {
        let _ = payload;
    }

    /// Delegate a task to an agent with a token budget. Returns task ID.
    async fn delegate_task(
        &self,
        agent_id: &str,
        task: &str,
        max_tokens: u64,
    ) -> Result<String, String> {
        let _ = (agent_id, task, max_tokens);
        Err("Not implemented".into())
    }

    /// Which memory backend is configured (graph or mempalace).
    fn memory_backend(&self) -> captain_types::config::MemoryBackend {
        captain_types::config::MemoryBackend::default()
    }

    /// Store a value in shared memory (cross-agent accessible).
    fn memory_store(&self, key: &str, value: serde_json::Value) -> Result<(), String>;

    /// Recall a value from shared memory.
    fn memory_recall(&self, key: &str) -> Result<Option<serde_json::Value>, String>;

    /// Store system metadata without adding a semantic memory row.
    /// Implementations should override this when they have a KV store.
    fn memory_kv_store(&self, key: &str, value: serde_json::Value) -> Result<(), String> {
        self.memory_store(key, value)
    }

    /// Recall system metadata without semantic fallback.
    fn memory_kv_recall(&self, key: &str) -> Result<Option<serde_json::Value>, String> {
        self.memory_recall(key)
    }

    /// Active memory retractions used to filter stale archived context
    /// before prompt injection. Checkpoints remain stored; they simply stop
    /// being treated as active truth when a retraction matches.
    fn memory_retractions(&self) -> Vec<MemoryRetraction> {
        load_retractions(self.memory_kv_recall(MEMORY_RETRACTIONS_KEY).ok().flatten())
    }

    /// Apply active retractions to mutable prompt sources such as canonical
    /// summaries. Historical archives/checkpoints should remain intact.
    fn memory_sanitize_active_context(
        &self,
        retractions: &[MemoryRetraction],
    ) -> Result<serde_json::Value, String> {
        let _ = retractions;
        Ok(serde_json::json!({
            "status": "noop",
            "reason": "memory_sanitize_active_context not implemented on this kernel"
        }))
    }

    /// Return the shared SQLite connection used by `memory_writes`
    /// (v3.12a write-through buffer). Returning `None` means the caller
    /// must fall back to direct MCP calls without local persistence.
    fn memory_writes_conn(&self) -> Option<std::sync::Arc<std::sync::Mutex<rusqlite::Connection>>> {
        None
    }

    /// List pending learning review items (v3.12g approval mode).
    /// Returns a JSON array suitable for an LLM tool response.
    fn learning_review_list(&self, limit: usize) -> Result<serde_json::Value, String> {
        let _ = limit;
        Err("learning_review_list not implemented on this kernel".into())
    }

    /// Approve or deny a pending learning review item (v3.12g).
    /// Approval triggers a write_through via the MemoryCommitter.
    async fn learning_review_decide(
        &self,
        review_id: &str,
        approve: bool,
        decided_by: Option<&str>,
    ) -> Result<serde_json::Value, String> {
        let _ = (review_id, approve, decided_by);
        Err("learning_review_decide not implemented on this kernel".into())
    }

    /// List pending skill proposals (v3.13c).
    fn skill_proposal_list(&self, limit: usize) -> Result<serde_json::Value, String> {
        let _ = limit;
        Err("skill_proposal_list not implemented on this kernel".into())
    }

    /// Approve or deny a pending skill proposal (v3.13d). On approve
    /// the SkillWriter drops a generated `.md` under the configured
    /// `[skills] generated_dir`; the resulting path is returned and
    /// recorded via `mark_written`.
    async fn skill_proposal_decide(
        &self,
        proposal_id: &str,
        approve: bool,
        decided_by: Option<&str>,
    ) -> Result<serde_json::Value, String> {
        let _ = (proposal_id, approve, decided_by);
        Err("skill_proposal_decide not implemented on this kernel".into())
    }

    /// Find agents by query (matches on name substring, tag, or tool name; case-insensitive).
    fn find_agents(&self, query: &str) -> Vec<AgentInfo>;

    /// Post a task to the shared task queue. Returns the task ID.
    async fn task_post(
        &self,
        title: &str,
        description: &str,
        assigned_to: Option<&str>,
        created_by: Option<&str>,
    ) -> Result<String, String>;

    /// Claim the next available task (optionally filtered by assignee). Returns task JSON or None.
    async fn task_claim(&self, agent_id: &str) -> Result<Option<serde_json::Value>, String>;

    /// Mark a task as completed with a result string.
    async fn task_complete(&self, task_id: &str, result: &str) -> Result<(), String>;

    /// List tasks, optionally filtered by status.
    async fn task_list(&self, status: Option<&str>) -> Result<Vec<serde_json::Value>, String> {
        let _ = status;
        Err("Tasks not available".into())
    }

    /// Publish a custom event that can trigger proactive agents.
    async fn publish_event(
        &self,
        event_type: &str,
        payload: serde_json::Value,
    ) -> Result<(), String> {
        let _ = (event_type, payload);
        Err("Event bus not available".into())
    }

    /// Add an entity to the knowledge graph.
    async fn knowledge_add_entity(
        &self,
        entity: captain_types::memory::Entity,
    ) -> Result<String, String> {
        let _ = entity;
        Err("Knowledge graph not available".into())
    }

    /// Add a relation to the knowledge graph.
    async fn knowledge_add_relation(
        &self,
        relation: captain_types::memory::Relation,
    ) -> Result<String, String> {
        let _ = relation;
        Err("Knowledge graph not available".into())
    }

    /// Query the knowledge graph with a pattern.
    async fn knowledge_query(
        &self,
        pattern: captain_types::memory::GraphPattern,
    ) -> Result<Vec<captain_types::memory::GraphMatch>, String> {
        let _ = pattern;
        Err("Knowledge graph not available".into())
    }

    /// Create a cron job for the calling agent.
    async fn cron_create(
        &self,
        agent_id: &str,
        job_json: serde_json::Value,
    ) -> Result<String, String> {
        let _ = (agent_id, job_json);
        Err("Cron scheduler not available".to_string())
    }

    /// List cron jobs for the calling agent.
    async fn cron_list(&self, agent_id: &str) -> Result<Vec<serde_json::Value>, String> {
        let _ = agent_id;
        Err("Cron scheduler not available".to_string())
    }

    /// Update a cron job owned by the calling agent.
    async fn cron_update(
        &self,
        agent_id: &str,
        job_json: serde_json::Value,
    ) -> Result<String, String> {
        let _ = (agent_id, job_json);
        Err("Cron scheduler not available".to_string())
    }

    /// Cancel a cron job by ID.
    async fn cron_cancel(&self, job_id: &str) -> Result<(), String> {
        let _ = job_id;
        Err("Cron scheduler not available".to_string())
    }

    // --- File-change triggers (filesystem watchers) -------------------

    /// Register a file-change trigger and arm its watcher.
    ///
    /// `input` is the JSON payload accepted by the REST handler:
    /// `{ "paths": [...], "events": [...], "recursive": bool, "prompt_template": "...", "debounce_ms": u64, "enabled": bool }`.
    /// Returns the new trigger ID as a string on success.
    async fn file_trigger_register(
        &self,
        agent_id: &str,
        input: serde_json::Value,
    ) -> Result<String, String> {
        let _ = (agent_id, input);
        Err("File-change triggers not available".to_string())
    }

    /// List file-change triggers, optionally filtered by agent.
    async fn file_trigger_list(
        &self,
        agent_id: Option<&str>,
    ) -> Result<Vec<serde_json::Value>, String> {
        let _ = agent_id;
        Err("File-change triggers not available".to_string())
    }

    /// Enable or disable a file-change trigger by ID.
    async fn file_trigger_set_enabled(
        &self,
        trigger_id: &str,
        enabled: bool,
    ) -> Result<bool, String> {
        let _ = (trigger_id, enabled);
        Err("File-change triggers not available".to_string())
    }

    /// Remove a file-change trigger by ID.
    async fn file_trigger_remove(&self, trigger_id: &str) -> Result<bool, String> {
        let _ = trigger_id;
        Err("File-change triggers not available".to_string())
    }

    // --- Cross-session todos (v3.12g) ---------------------------------

    /// Insert a todo. Returns the JSON for the new row.
    fn todo_create(&self, title: &str, body: &str) -> Result<serde_json::Value, String> {
        let _ = (title, body);
        Err("Todo store not available".into())
    }

    /// List todos by filter (`open` | `done` | `all`). `limit` is clamped
    /// inside the substrate; pass `None` for the default page size.
    fn todo_list(&self, filter: &str, limit: Option<u32>) -> Result<serde_json::Value, String> {
        let _ = (filter, limit);
        Err("Todo store not available".into())
    }

    /// Mark a todo done. Returns `Ok(None)` if the id is unknown.
    fn todo_complete(&self, id: &str) -> Result<Option<serde_json::Value>, String> {
        let _ = id;
        Err("Todo store not available".into())
    }

    /// Reopen a previously-completed todo.
    fn todo_reopen(&self, id: &str) -> Result<Option<serde_json::Value>, String> {
        let _ = id;
        Err("Todo store not available".into())
    }

    /// Delete a todo. Returns `true` when a row was removed.
    fn todo_delete(&self, id: &str) -> Result<bool, String> {
        let _ = id;
        Err("Todo store not available".into())
    }

    // --- Project / tasks / milestones / checkpoints (v3.11) -----------

    fn project_create(
        &self,
        name: &str,
        slug: &str,
        goal: &str,
        deadline: Option<i64>,
    ) -> Result<serde_json::Value, String> {
        let _ = (name, slug, goal, deadline);
        Err("Project store not available".into())
    }

    fn project_list(&self, include_archived: bool) -> Result<serde_json::Value, String> {
        let _ = include_archived;
        Err("Project store not available".into())
    }

    fn project_find_by_slug(&self, slug: &str) -> Result<Option<serde_json::Value>, String> {
        let _ = slug;
        Err("Project store not available".into())
    }

    fn project_archive(&self, id: &str) -> Result<Option<serde_json::Value>, String> {
        let _ = id;
        Err("Project store not available".into())
    }

    fn project_delete(&self, id: &str) -> Result<bool, String> {
        let _ = id;
        Err("Project store not available".into())
    }

    fn project_task_create(
        &self,
        project_id: &str,
        title: &str,
        description: &str,
        parent_id: Option<&str>,
    ) -> Result<serde_json::Value, String> {
        let _ = (project_id, title, description, parent_id);
        Err("Project task store not available".into())
    }

    fn project_task_list(&self, project_id: &str) -> Result<serde_json::Value, String> {
        let _ = project_id;
        Err("Project task store not available".into())
    }

    fn project_task_update_status(
        &self,
        id: &str,
        status: &str,
    ) -> Result<Option<serde_json::Value>, String> {
        let _ = (id, status);
        Err("Project task store not available".into())
    }

    fn milestone_create(
        &self,
        project_id: &str,
        name: &str,
        due_date: Option<i64>,
    ) -> Result<serde_json::Value, String> {
        let _ = (project_id, name, due_date);
        Err("Milestone store not available".into())
    }

    fn milestone_list(&self, project_id: &str) -> Result<serde_json::Value, String> {
        let _ = project_id;
        Err("Milestone store not available".into())
    }

    fn milestone_complete(&self, id: &str) -> Result<Option<serde_json::Value>, String> {
        let _ = id;
        Err("Milestone store not available".into())
    }

    fn milestone_progress(&self, project_id: &str) -> Result<serde_json::Value, String> {
        let _ = project_id;
        Err("Milestone store not available".into())
    }

    fn checkpoint_save(
        &self,
        project_id: &str,
        summary: &str,
        state: serde_json::Value,
        session_id: Option<&str>,
    ) -> Result<serde_json::Value, String> {
        let _ = (project_id, summary, state, session_id);
        Err("Checkpoint store not available".into())
    }

    fn project_resume(&self, slug: &str) -> Result<serde_json::Value, String> {
        let _ = slug;
        Err("Checkpoint store not available".into())
    }

    fn active_project_set(&self, agent_id: &str, slug: Option<&str>) -> Result<(), String> {
        let _ = (agent_id, slug);
        Ok(())
    }

    fn active_project_get(&self, agent_id: &str) -> Option<String> {
        let _ = agent_id;
        None
    }

    /// Check if a tool requires approval based on current policy.
    fn requires_approval(&self, tool_name: &str) -> bool {
        let _ = tool_name;
        false
    }

    /// Request approval for a tool execution. Blocks until approved/denied/timed out.
    /// Returns `Ok(true)` if approved, `Ok(false)` if denied or timed out.
    async fn request_approval(
        &self,
        agent_id: &str,
        tool_name: &str,
        action_summary: &str,
    ) -> Result<bool, String> {
        let _ = (agent_id, tool_name, action_summary);
        Ok(true) // Default: auto-approve
    }

    /// List available Hands and their activation status.
    async fn hand_list(&self) -> Result<Vec<serde_json::Value>, String> {
        Err("Hands system not available".to_string())
    }

    /// Install a Hand from TOML content.
    async fn hand_install(
        &self,
        toml_content: &str,
        skill_content: &str,
    ) -> Result<serde_json::Value, String> {
        let _ = (toml_content, skill_content);
        Err("Hands system not available".to_string())
    }

    /// Activate a Hand — spawns a specialized autonomous agent.
    async fn hand_activate(
        &self,
        hand_id: &str,
        config: std::collections::HashMap<String, serde_json::Value>,
    ) -> Result<serde_json::Value, String> {
        let _ = (hand_id, config);
        Err("Hands system not available".to_string())
    }

    /// Check the status and dashboard metrics of an active Hand.
    async fn hand_status(&self, hand_id: &str) -> Result<serde_json::Value, String> {
        let _ = hand_id;
        Err("Hands system not available".to_string())
    }

    /// Deactivate a running Hand and stop its agent.
    async fn hand_deactivate(&self, instance_id: &str) -> Result<(), String> {
        let _ = instance_id;
        Err("Hands system not available".to_string())
    }

    /// List discovered external A2A agents as (name, url) pairs.
    fn list_a2a_agents(&self) -> Vec<(String, String)> {
        vec![]
    }

    /// Get the URL of a discovered external A2A agent by name.
    fn get_a2a_agent_url(&self, name: &str) -> Option<String> {
        let _ = name;
        None
    }

    /// Get the default recipient for a channel (e.g. default_chat_id for Telegram).
    async fn get_channel_default_recipient(&self, channel: &str) -> Option<String> {
        let _ = channel;
        None
    }

    /// Return a summary of configured channels for injection into agent system prompts.
    async fn get_channels_context(&self) -> Option<String> {
        None
    }

    /// Send a message to a user on a named channel adapter (e.g., "email", "telegram").
    /// When `thread_id` is provided, the message is sent as a thread reply.
    /// Returns a confirmation string on success.
    async fn send_channel_message(
        &self,
        channel: &str,
        recipient: &str,
        message: &str,
        thread_id: Option<&str>,
    ) -> Result<String, String> {
        self.send_channel_message_from(channel, recipient, message, thread_id, None)
            .await
    }

    /// Send a channel message with caller agent context (for auto topic routing).
    async fn send_channel_message_from(
        &self,
        channel: &str,
        recipient: &str,
        message: &str,
        thread_id: Option<&str>,
        caller_agent_name: Option<&str>,
    ) -> Result<String, String> {
        let _ = (channel, recipient, message, thread_id, caller_agent_name);
        Err("Channel send not available".to_string())
    }

    /// Get the Telegram topic ID for an agent (from persisted store or config).
    fn get_telegram_topic(&self, agent_name: &str) -> Option<String> {
        let _ = agent_name;
        None
    }

    /// Persist a Telegram topic ID association for an agent/hand.
    fn set_telegram_topic(&self, agent_name: &str, topic_id: &str) {
        let _ = (agent_name, topic_id);
    }

    /// Send a rich message with metadata (buttons, thread_id, etc.).
    async fn send_channel_rich(
        &self,
        channel: &str,
        recipient: &str,
        message: &str,
        metadata: &std::collections::HashMap<String, serde_json::Value>,
    ) -> Result<String, String> {
        let _ = (channel, recipient, message, metadata);
        Err("Rich channel send not available".to_string())
    }

    /// Send media content (image/file) to a user on a named channel adapter.
    /// `media_type` is "image" or "file", `media_url` is the URL, `caption` is optional text.
    /// When `thread_id` is provided, the media is sent as a thread reply.
    async fn send_channel_media(
        &self,
        channel: &str,
        recipient: &str,
        media_type: &str,
        media_url: &str,
        caption: Option<&str>,
        filename: Option<&str>,
        thread_id: Option<&str>,
    ) -> Result<String, String> {
        let _ = (
            channel, recipient, media_type, media_url, caption, filename, thread_id,
        );
        Err("Channel media send not available".to_string())
    }

    /// Send a local file (raw bytes) to a user on a named channel adapter.
    /// Used by the `channel_send` tool when `file_path` is provided.
    /// When `thread_id` is provided, the file is sent as a thread reply.
    async fn send_channel_file_data(
        &self,
        channel: &str,
        recipient: &str,
        data: Vec<u8>,
        filename: &str,
        mime_type: &str,
        thread_id: Option<&str>,
    ) -> Result<String, String> {
        let _ = (channel, recipient, data, filename, mime_type, thread_id);
        Err("Channel file data send not available".to_string())
    }

    /// Send a local image (raw bytes) to a user on a named channel adapter (v3.8c).
    /// Used by the `channel_send` tool when `file_path` is an image — routed
    /// to the channel's native photo-upload endpoint so it arrives inline.
    async fn send_channel_image_data(
        &self,
        channel: &str,
        recipient: &str,
        data: Vec<u8>,
        mime_type: &str,
        caption: Option<&str>,
        thread_id: Option<&str>,
    ) -> Result<String, String> {
        let _ = (channel, recipient, data, mime_type, caption, thread_id);
        Err("Channel image data send not available".to_string())
    }

    /// Spawn an agent with capability inheritance enforcement.
    /// `parent_caps` are the parent's granted capabilities. The kernel MUST verify
    /// that every capability in the child manifest is covered by `parent_caps`.
    async fn spawn_agent_checked(
        &self,
        manifest_toml: &str,
        parent_id: Option<&str>,
        parent_caps: &[captain_types::capability::Capability],
    ) -> Result<(String, String), String> {
        // Default: delegate to spawn_agent (no enforcement)
        // The kernel MUST override this with real enforcement
        let _ = parent_caps;
        self.spawn_agent(manifest_toml, parent_id).await
    }

    /// Consume queued emergent thoughts from the neural heartbeat.
    /// Called at the start of agent interactions to inject graph-consciousness context.
    fn consume_thoughts(&self, max: usize) -> Vec<serde_json::Value> {
        let _ = max;
        vec![]
    }

    /// Recall past reflections (tool success/failure history) for an agent.
    fn recall_reflections(&self, agent_name: &str, limit: usize) -> String {
        let _ = (agent_name, limit);
        String::new()
    }

    /// Update user state from message content and return prompt context.
    fn update_user_state(&self, content: &str) -> String {
        let _ = content;
        String::new()
    }

    /// Get system mood prompt context.
    fn mood_prompt(&self) -> String {
        String::new()
    }

    /// Compact runtime health/goals/anomaly summary for prompt injection.
    fn operational_awareness_prompt(&self, agent_name: &str) -> String {
        let _ = agent_name;
        String::new()
    }

    /// Get anticipated actions from temporal patterns.
    fn temporal_prompt(&self) -> String {
        String::new()
    }

    /// M.4: Shared knowledge from the graph (user info, prefs, people, habits).
    fn shared_knowledge_prompt(&self) -> String {
        String::new()
    }

    /// Read a config value by dotted path (e.g. "channels.telegram.default_chat_id").
    /// Return the full default config template with header. v3.14-config.c.
    /// Used by the agent to discover every configurable field without
    /// reading Rust source. Safe to call anytime — pure function.
    fn config_schema(&self) -> Result<String, String> {
        captain_types::config_template::render_default_toml_with_header()
            .map_err(|e| format!("render config template: {e}"))
    }

    fn config_read(&self, path: &str) -> Result<Option<String>, String> {
        let _ = path;
        Err("Config not available".into())
    }

    /// Write a config value by dotted path. Persists to config.toml.
    async fn config_write(&self, path: &str, value: &str) -> Result<(), String> {
        let _ = (path, value);
        Err("Config not available".into())
    }

    /// Update the calling agent's own configuration (model, routing, fallbacks, etc.).
    /// `agent_id` is the UUID of the agent calling this method.
    async fn update_self_config(
        &self,
        agent_id: &str,
        config_json: &str,
    ) -> Result<String, String> {
        let _ = (agent_id, config_json);
        Err("Self-configure not available".into())
    }

    /// Preflight a safe model/provider switch for the calling agent.
    fn model_switch_plan(
        &self,
        agent_id: &str,
        model: &str,
        provider: Option<&str>,
    ) -> Result<serde_json::Value, String> {
        let _ = (agent_id, model, provider);
        Err("Model switch preflight not available".into())
    }

    /// Apply a safe model/provider switch with an explicit session strategy.
    fn model_switch_apply(
        &self,
        agent_id: &str,
        model: &str,
        provider: Option<&str>,
        session_strategy: &str,
    ) -> Result<serde_json::Value, String> {
        let _ = (agent_id, model, provider, session_strategy);
        Err("Model switch apply not available".into())
    }

    /// Read a secret from the centralized secrets.env.
    fn secret_read(&self, key: &str) -> Result<Option<String>, String> {
        let _ = key;
        Err("Secrets not available".into())
    }

    /// Write a secret to the centralized secrets.env.
    fn secret_write(&self, key: &str, value: &str) -> Result<(), String> {
        let _ = (key, value);
        Err("Secrets not available".into())
    }

    /// Get the home directory path.
    fn home_dir(&self) -> Option<std::path::PathBuf> {
        None
    }

    /// R.3.2 — broadcast `SystemEvent::IntegrationConfigured { name }`
    /// after `config_setup` (or `captain integration setup`) succeeds, so
    /// channel managers / TTS engines can hot-reload the affected adapter
    /// without a daemon restart. Default impl is a no-op for environments
    /// that do not have an event bus wired (tests, embedded uses).
    fn publish_integration_configured(&self, name: &str) {
        let _ = name;
    }

    /// Search bundled MCP integration templates and installed state. This is
    /// the typed alternative to shelling out to `captain integrations`.
    async fn mcp_catalog_search(
        &self,
        query: Option<&str>,
        limit: usize,
    ) -> Result<serde_json::Value, String> {
        let _ = (query, limit);
        Err("MCP integration catalog not available".into())
    }

    /// Install a bundled MCP integration template, storing provided
    /// credentials through the extension vault/resolver and optionally
    /// hot-reloading the running MCP pool.
    async fn mcp_integration_install(
        &self,
        id: &str,
        credentials: serde_json::Value,
        reload: bool,
    ) -> Result<serde_json::Value, String> {
        let _ = (id, credentials, reload);
        Err("MCP integration install not available".into())
    }

    /// Report configured, connected, and tool-level MCP state from Captain's
    /// current runtime perspective.
    async fn mcp_status(&self) -> Result<serde_json::Value, String> {
        Err("MCP status not available".into())
    }

    /// R.2.1 — goal-driven autopilot. JSON in/out so the runtime stays
    /// decoupled from the kernel-owned `Goal` type. Default impl returns
    /// an explicit error so embedded/test environments can detect the
    /// missing wiring.
    fn goal_create(&self, _goal_json: &str) -> Result<String, String> {
        Err("Autopilot goals not available".into())
    }

    /// Return all goals as a JSON array string.
    fn goal_list(&self) -> Result<String, String> {
        Err("Autopilot goals not available".into())
    }

    /// Flip a goal to Paused. Returns `true` if the goal exists.
    fn goal_pause(&self, _id: &str) -> Result<bool, String> {
        Err("Autopilot goals not available".into())
    }

    /// Flip a paused goal back to Active.
    fn goal_resume(&self, _id: &str) -> Result<bool, String> {
        Err("Autopilot goals not available".into())
    }

    /// Return a single goal's full JSON (status + last checks + counters).
    fn goal_status(&self, _id: &str) -> Result<String, String> {
        Err("Autopilot goals not available".into())
    }

    /// Remove a goal entirely.
    fn goal_delete(&self, _id: &str) -> Result<bool, String> {
        Err("Autopilot goals not available".into())
    }

    /// Record a check outcome and return the live `consecutive_fails`
    /// counter so the goal loop can decide whether to escalate. Output
    /// is auto-truncated by the store.
    fn goal_record_check(
        &self,
        _id: &str,
        _ok: bool,
        _output: &str,
        _latency_ms: u64,
    ) -> Result<u32, String> {
        Err("Autopilot goals not available".into())
    }

    /// Atomically flip a goal to Escalated and stamp `escalated_at`.
    fn goal_mark_escalated(&self, _id: &str) -> Result<bool, String> {
        Err("Autopilot goals not available".into())
    }

    /// R.1.1 — UUID generated at boot. Used by peer_discovery to filter
    /// our own mDNS broadcasts and by API server to advertise ourselves.
    fn instance_id(&self) -> String {
        String::new()
    }

    /// R.1.1 — true when an external A2A agent with this name is already
    /// in the store. Used by peer_discovery to dedupe re-broadcasts.
    fn has_external_agent(&self, _name: &str) -> bool {
        false
    }

    /// R.1.1 — JSON list of every external A2A agent (name + card).
    fn list_external_agents(&self) -> Result<String, String> {
        Err("A2A external agents not available".into())
    }

    /// Commit-A — broadcast a `ChatStreamEvent::MemoryStored` on the bus
    /// so each surface (TUI launcher, SSE clients, channel adapters)
    /// can render a `🧠` notice in the right canal. Used by the
    /// Captain-native `memory_save` tool which writes synchronously and
    /// needs to surface the action immediately, bypassing the async
    /// reflection pipeline. Default impl is a no-op for embedded uses.
    #[allow(clippy::too_many_arguments)]
    fn publish_memory_stored(
        &self,
        _subject: &str,
        _predicate: &str,
        _object: &str,
        _source: &str,
        _wing: Option<&str>,
        _room: Option<&str>,
        _channel: Option<&str>,
        _category: Option<&str>,
    ) {
    }

    /// Broadcast a `ChatStreamEvent::SkillRefinementQueued` when Captain
    /// proactively detects that an existing skill should improve. Default is
    /// no-op for embedded tests and kernels without a channel bus.
    #[allow(clippy::too_many_arguments)]
    fn publish_skill_refinement_queued(
        &self,
        _refinement_id: &str,
        _skill: &str,
        _finding: &str,
        _suggested_change: &str,
        _risk: &str,
        _source: &str,
        _channel: Option<&str>,
    ) {
    }

    /// R.2.2 — sliding 1h LLM rate limiter for the reflection job.
    /// Returns true if a slot was reserved, false if the goal has
    /// burned its `max_llm_calls_per_hour` budget.
    fn goal_try_consume_llm_quota(&self, _id: &str) -> bool {
        false
    }

    /// R.2.2 — list all suggestions (any status) for a goal as JSON.
    fn goal_list_suggestions(&self, _id: &str) -> Result<String, String> {
        Err("Autopilot goals not available".into())
    }

    /// R.2.2 — append a Pending suggestion produced by the reflection
    /// job. Caller passes already-serialized JSON to keep the runtime
    /// crate decoupled from the kernel `Suggestion` type.
    fn goal_add_suggestion_raw(&self, _id: &str, _suggestion_json: &str) -> Result<(), String> {
        Err("Autopilot goals not available".into())
    }

    /// R.2.2 — apply a Pending suggestion (mutates the goal +
    /// re-validates). Returns true if the suggestion existed and was
    /// applied.
    fn goal_apply_suggestion(&self, _id: &str, _suggestion_id: &str) -> Result<bool, String> {
        Err("Autopilot goals not available".into())
    }

    /// R.2.2 — reject a Pending suggestion (no goal mutation).
    fn goal_reject_suggestion(&self, _id: &str, _suggestion_id: &str) -> Result<bool, String> {
        Err("Autopilot goals not available".into())
    }

    /// W1: Record a tool action for temporal pattern detection.
    fn record_temporal_action(&self, _action: &str) {}

    /// W3: Return curiosity items formatted as a system prompt snippet.
    fn curiosity_prompt(&self) -> String {
        String::new()
    }

    /// D.8: Return recent narration summaries as a system prompt snippet.
    fn narration_prompt(&self) -> String {
        String::new()
    }
}
