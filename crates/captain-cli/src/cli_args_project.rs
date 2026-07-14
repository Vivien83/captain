use clap::{Subcommand, ValueEnum};

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub(crate) enum ProjectToolDecisionArg {
    /// Approve the requested tools for the current phase.
    Approve,
    /// Deny the requested tools and keep the decision in runtime context.
    Deny,
}

impl ProjectToolDecisionArg {
    pub(crate) fn as_api_str(self) -> &'static str {
        match self {
            Self::Approve => "approve",
            Self::Deny => "deny",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub(crate) enum ProjectTaskStatusArg {
    Todo,
    Doing,
    Blocked,
    Review,
    Done,
    Cancelled,
}

impl ProjectTaskStatusArg {
    pub(crate) fn as_api_str(self) -> &'static str {
        match self {
            Self::Todo => "todo",
            Self::Doing => "doing",
            Self::Blocked => "blocked",
            Self::Review => "review",
            Self::Done => "done",
            Self::Cancelled => "cancelled",
        }
    }
}

#[derive(Subcommand)]
pub(crate) enum ProjectTaskCommands {
    /// List durable project tasks.
    List {
        /// Project id or slug.
        project_id: String,
        /// Filter by task status.
        #[arg(long, value_enum)]
        status: Option<ProjectTaskStatusArg>,
        /// Maximum number of tasks to show.
        #[arg(long, default_value_t = 20)]
        limit: usize,
        /// Output an operator-safe JSON task list.
        #[arg(long)]
        json: bool,
    },
    /// Create a durable project task.
    Create {
        /// Project id or slug.
        project_id: String,
        /// Task title.
        #[arg(long)]
        title: String,
        /// Task description stored durably but not echoed by default.
        #[arg(long)]
        description: Option<String>,
        /// Optional parent task id.
        #[arg(long)]
        parent_id: Option<String>,
        /// Task priority.
        #[arg(long)]
        priority: Option<i32>,
        /// Optional deadline as unix milliseconds.
        #[arg(long)]
        deadline: Option<i64>,
        /// Output an operator-safe JSON task.
        #[arg(long)]
        json: bool,
    },
    /// Update a durable project task.
    Update {
        /// Task id.
        task_id: String,
        /// New task status.
        #[arg(long, value_enum)]
        status: Option<ProjectTaskStatusArg>,
        /// New task title.
        #[arg(long)]
        title: Option<String>,
        /// New task description stored durably but not echoed by default.
        #[arg(long)]
        description: Option<String>,
        /// New parent task id.
        #[arg(long)]
        parent_id: Option<String>,
        /// Clear the parent task link.
        #[arg(long)]
        clear_parent: bool,
        /// New task priority.
        #[arg(long)]
        priority: Option<i32>,
        /// Output an operator-safe JSON task.
        #[arg(long)]
        json: bool,
    },
    /// Delete a durable project task.
    Delete {
        /// Task id.
        task_id: String,
        /// Confirm deletion.
        #[arg(long)]
        yes: bool,
        /// Output an operator-safe JSON result.
        #[arg(long)]
        json: bool,
    },
}

#[derive(Subcommand)]
pub(crate) enum ProjectMilestoneCommands {
    /// List durable project milestones.
    List {
        /// Project id or slug.
        project_id: String,
        /// Maximum number of milestones to show.
        #[arg(long, default_value_t = 20)]
        limit: usize,
        /// Output an operator-safe JSON milestone list.
        #[arg(long)]
        json: bool,
    },
    /// Create a durable project milestone.
    Create {
        /// Project id or slug.
        project_id: String,
        /// Milestone name.
        #[arg(long)]
        name: String,
        /// Optional due date as unix milliseconds.
        #[arg(long)]
        due_date: Option<i64>,
        /// Deliverable text stored durably but not echoed by default.
        #[arg(long = "deliverable")]
        deliverables: Vec<String>,
        /// Output an operator-safe JSON milestone.
        #[arg(long)]
        json: bool,
    },
    /// Mark a durable project milestone completed.
    Complete {
        /// Milestone id.
        milestone_id: String,
        /// Output an operator-safe JSON milestone.
        #[arg(long)]
        json: bool,
    },
    /// Show milestone progress for a project.
    Progress {
        /// Project id or slug.
        project_id: String,
        /// Output an operator-safe JSON progress summary.
        #[arg(long)]
        json: bool,
    },
}

#[derive(Subcommand)]
pub(crate) enum ProjectGoalCommands {
    /// List durable goals scoped to a project.
    List {
        /// Project id or slug.
        project_id: String,
        /// Maximum number of goals to show.
        #[arg(long, default_value_t = 20)]
        limit: usize,
        /// Output an operator-safe JSON goal list.
        #[arg(long)]
        json: bool,
    },
    /// Create a durable project goal.
    Create {
        /// Project id or slug.
        project_id: String,
        /// Optional goal id.
        #[arg(long)]
        id: Option<String>,
        /// Goal name.
        #[arg(long)]
        name: Option<String>,
        /// Goal description stored durably but not echoed by default.
        #[arg(long)]
        description: Option<String>,
        /// Shell check command. Stored durably but never echoed by default.
        #[arg(long)]
        check_command: String,
        /// Optional recovery command. Stored durably but never echoed by default.
        #[arg(long)]
        recovery_command: Option<String>,
        /// Check interval in seconds.
        #[arg(long)]
        interval_secs: Option<u64>,
        /// Consecutive failures before escalation.
        #[arg(long)]
        escalation_threshold: Option<u32>,
        /// Hourly LLM reflection budget for this goal.
        #[arg(long)]
        max_llm_calls_per_hour: Option<u32>,
        /// Output an operator-safe JSON goal.
        #[arg(long)]
        json: bool,
    },
    /// Update a durable project goal.
    Update {
        /// Project id or slug.
        project_id: String,
        /// Goal id.
        goal_id: String,
        /// Goal name.
        #[arg(long)]
        name: Option<String>,
        /// Goal description stored durably but not echoed by default.
        #[arg(long)]
        description: Option<String>,
        /// Shell check command. Stored durably but never echoed by default.
        #[arg(long)]
        check_command: Option<String>,
        /// Recovery command. Empty string clears it.
        #[arg(long)]
        recovery_command: Option<String>,
        /// Check interval in seconds.
        #[arg(long)]
        interval_secs: Option<u64>,
        /// Consecutive failures before escalation.
        #[arg(long)]
        escalation_threshold: Option<u32>,
        /// Hourly LLM reflection budget for this goal.
        #[arg(long)]
        max_llm_calls_per_hour: Option<u32>,
        /// Output an operator-safe JSON goal.
        #[arg(long)]
        json: bool,
    },
    /// Pause a project goal.
    Pause {
        /// Project id or slug.
        project_id: String,
        /// Goal id.
        goal_id: String,
        /// Output an operator-safe JSON goal.
        #[arg(long)]
        json: bool,
    },
    /// Resume a project goal.
    Resume {
        /// Project id or slug.
        project_id: String,
        /// Goal id.
        goal_id: String,
        /// Output an operator-safe JSON goal.
        #[arg(long)]
        json: bool,
    },
    /// Delete a project goal.
    Delete {
        /// Project id or slug.
        project_id: String,
        /// Goal id.
        goal_id: String,
        /// Confirm deletion.
        #[arg(long)]
        yes: bool,
        /// Output an operator-safe JSON result.
        #[arg(long)]
        json: bool,
    },
}

#[derive(Subcommand)]
pub(crate) enum ProjectCommands {
    /// List projects with compact runtime state.
    List {
        /// Include archived projects.
        #[arg(long)]
        include_archived: bool,
        /// Show only projects that likely need operator attention.
        #[arg(long)]
        attention: bool,
        /// Maximum number of projects to show.
        #[arg(long, default_value_t = 20)]
        limit: usize,
        /// Output an operator-safe JSON list for scripting.
        #[arg(long)]
        json: bool,
    },
    /// Show one project's runtime operator status.
    Status {
        /// Project id or slug.
        project_id: String,
        /// Output as JSON for scripting.
        #[arg(long)]
        json: bool,
    },
    /// Show project runtime workers/sub-agents.
    Workers {
        /// Project id or slug.
        project_id: String,
        /// Optional lifecycle phase filter.
        #[arg(long)]
        phase: Option<String>,
        /// Maximum number of workers to show.
        #[arg(long, default_value_t = 20)]
        limit: usize,
        /// Output an operator-safe JSON worker list.
        #[arg(long)]
        json: bool,
    },
    /// Show project runtime user questions.
    Questions {
        /// Project id or slug.
        project_id: String,
        /// Optional lifecycle phase filter.
        #[arg(long)]
        phase: Option<String>,
        /// Include answered and closed questions.
        #[arg(long)]
        all: bool,
        /// Maximum number of questions to show.
        #[arg(long, default_value_t = 20)]
        limit: usize,
        /// Output an operator-safe JSON question list.
        #[arg(long)]
        json: bool,
    },
    /// Show a bounded project runtime replay.
    Replay {
        /// Project id or slug.
        project_id: String,
        /// Maximum number of replay events to show.
        #[arg(long, default_value_t = 20)]
        events: usize,
        /// Maximum number of worker summaries to show.
        #[arg(long, default_value_t = 8)]
        workers: usize,
        /// Output an operator-safe JSON replay capsule.
        #[arg(long)]
        json: bool,
    },
    /// Show durable resume context without starting the runtime.
    Context {
        /// Project id or slug.
        project_id: String,
        /// Maximum number of tasks and goals to show.
        #[arg(long, default_value_t = 8)]
        limit: usize,
        /// Output an operator-safe JSON context for scripting.
        #[arg(long)]
        json: bool,
    },
    /// Operate durable project tasks.
    Task {
        #[command(subcommand)]
        command: ProjectTaskCommands,
    },
    /// Operate durable project milestones.
    Milestone {
        #[command(subcommand)]
        command: ProjectMilestoneCommands,
    },
    /// Operate durable project goals.
    Goal {
        #[command(subcommand)]
        command: ProjectGoalCommands,
    },
    /// Show recent project runtime timeline events.
    Timeline {
        /// Project id or slug.
        project_id: String,
        /// Maximum number of recent events to show.
        #[arg(long, default_value_t = 12)]
        limit: usize,
        /// Keep polling and print new runtime events as they arrive.
        #[arg(long)]
        follow: bool,
        /// Output an operator-safe JSON timeline for scripting.
        #[arg(long)]
        json: bool,
    },
    /// Show recent durable project checkpoints.
    Checkpoints {
        /// Project id or slug.
        project_id: String,
        /// Maximum number of recent checkpoints to show.
        #[arg(long, default_value_t = 8)]
        limit: usize,
        /// Output an operator-safe JSON checkpoint list for scripting.
        #[arg(long)]
        json: bool,
    },
    /// Archive a durable project without deleting history.
    Archive {
        /// Project id or slug.
        project_id: String,
        /// Output an operator-safe JSON result for scripting.
        #[arg(long)]
        json: bool,
    },
    /// Reactivate an archived durable project without starting its runtime.
    Unarchive {
        /// Project id or slug.
        project_id: String,
        /// Output an operator-safe JSON result for scripting.
        #[arg(long)]
        json: bool,
    },
    /// Start a project runtime.
    Start {
        /// Project id or slug.
        project_id: String,
        /// Output as JSON for scripting.
        #[arg(long)]
        json: bool,
    },
    /// Resume a paused, stale, or resume-ready project runtime.
    Resume {
        /// Project id or slug.
        project_id: String,
        /// Output as JSON for scripting.
        #[arg(long)]
        json: bool,
    },
    /// Pause a running project runtime.
    Pause {
        /// Project id or slug.
        project_id: String,
        /// Output as JSON for scripting.
        #[arg(long)]
        json: bool,
    },
    /// Pause and switch a project runtime back to manual control.
    Takeover {
        /// Project id or slug.
        project_id: String,
        /// Output as JSON for scripting.
        #[arg(long)]
        json: bool,
    },
    /// Answer a pending project question.
    Answer {
        /// Project id or slug.
        project_id: String,
        /// Pending ask id shown by `captain status --verbose`.
        #[arg(long)]
        ask_id: String,
        /// Answer text to record or deliver.
        #[arg(long)]
        answer: String,
        /// Output as JSON for scripting.
        #[arg(long)]
        json: bool,
    },
    /// Approve or deny a pending project tool request.
    ToolRequest {
        /// Project id or slug.
        project_id: String,
        /// Operator decision.
        #[arg(value_enum)]
        decision: ProjectToolDecisionArg,
        /// Runtime phase, for example build or verify. Inferred if omitted.
        #[arg(long)]
        phase: Option<String>,
        /// Approved tools. Repeat for multiple values. Inferred from the pending request if omitted.
        #[arg(long = "tool")]
        tools: Vec<String>,
        /// Operator reason stored with the decision.
        #[arg(long)]
        reason: Option<String>,
        /// Output as JSON for scripting.
        #[arg(long)]
        json: bool,
    },
}
