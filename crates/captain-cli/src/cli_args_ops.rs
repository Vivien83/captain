use std::path::PathBuf;

use clap::{Subcommand, ValueEnum};

#[derive(Subcommand)]
pub(crate) enum AgentCommands {
    /// Spawn a new agent from a template (interactive or by name).
    New {
        /// Template name (e.g., "coder", "assistant"). Interactive picker if omitted.
        template: Option<String>,
    },
    /// Spawn a new agent from a manifest file.
    Spawn {
        /// Path to the agent manifest TOML file.
        manifest: PathBuf,
    },
    /// List all running agents.
    List {
        /// Output as JSON for scripting.
        #[arg(long)]
        json: bool,
    },
    /// Show one agent's effective capabilities and live budget.
    Caps {
        /// Agent ID, ID prefix, or exact name.
        agent_id: String,
        /// Output as JSON for scripting.
        #[arg(long)]
        json: bool,
    },
    /// Show or prepare one agent's external API surface.
    Api {
        /// Agent ID, ID prefix, or exact name.
        agent_id: String,
        /// Output as JSON for scripting.
        #[arg(long)]
        json: bool,
        /// Print the full external integration manifest.
        #[arg(long)]
        manifest: bool,
        /// Rotate/generate the ingress bearer token and print it once.
        #[arg(long)]
        rotate_token: bool,
    },
    /// Interactive chat with an agent.
    Chat {
        /// Agent ID (UUID).
        agent_id: String,
    },
    /// Kill an agent.
    Kill {
        /// Agent ID (UUID).
        agent_id: String,
    },
    /// Set an agent property (e.g., model).
    Set {
        /// Agent ID (UUID).
        agent_id: String,
        /// Field to set (model).
        field: String,
        /// New value.
        value: String,
    },
}

#[derive(Subcommand)]
pub(crate) enum WorkflowCommands {
    /// List all registered workflows.
    List,
    /// Create a workflow from a JSON file.
    Create {
        /// Path to a JSON file describing the workflow.
        file: PathBuf,
    },
    /// Get a workflow by ID.
    Get {
        /// Workflow ID (UUID).
        workflow_id: String,
    },
    /// Update a workflow from a JSON file.
    Update {
        /// Workflow ID (UUID).
        workflow_id: String,
        /// Path to a JSON file with the updated workflow definition.
        file: PathBuf,
    },
    /// Delete a workflow by ID.
    Delete {
        /// Workflow ID (UUID).
        workflow_id: String,
    },
    /// Run a workflow by ID.
    Run {
        /// Workflow ID (UUID).
        workflow_id: String,
        /// Input text for the workflow.
        input: String,
    },
}

#[derive(Subcommand)]
pub(crate) enum TriggerCommands {
    /// List all triggers (optionally filtered by agent).
    List {
        /// Optional agent ID to filter by.
        #[arg(long)]
        agent_id: Option<String>,
    },
    /// Create a trigger for an agent.
    Create {
        /// Agent ID (UUID) that owns the trigger.
        agent_id: String,
        /// Trigger pattern as JSON.
        pattern_json: String,
        /// Prompt template (use {{event}} placeholder).
        #[arg(long, default_value = "Event: {{event}}")]
        prompt: String,
        /// Maximum number of times to fire (0 = unlimited).
        #[arg(long, default_value = "0")]
        max_fires: u64,
    },
    /// Delete a trigger by ID.
    Delete {
        /// Trigger ID (UUID).
        trigger_id: String,
    },
}

#[derive(Subcommand)]
pub(crate) enum ModelsCommands {
    /// Show the active default provider/model and fallbacks.
    Current {
        /// Output as JSON for scripting.
        #[arg(long)]
        json: bool,
    },
    /// List available models (optionally filter by provider).
    List {
        /// Filter by provider name.
        #[arg(long)]
        provider: Option<String>,
        /// Output as JSON for scripting.
        #[arg(long)]
        json: bool,
    },
    /// Show model aliases (shorthand names).
    Aliases {
        /// Output as JSON for scripting.
        #[arg(long)]
        json: bool,
    },
    /// List known LLM providers and their auth status.
    Providers {
        /// Output as JSON for scripting.
        #[arg(long)]
        json: bool,
    },
    /// Set the default model for the daemon.
    Set {
        /// Model ID or alias. Interactive picker if omitted.
        model: Option<String>,
    },
    /// Test provider connectivity using the daemon's provider test endpoint.
    Test {
        /// Provider to test. Defaults to the current provider.
        provider: Option<String>,
        /// Output as JSON for scripting.
        #[arg(long)]
        json: bool,
    },
}

#[derive(Subcommand)]
pub(crate) enum AuthCommands {
    /// Show provider credentials, OAuth readiness and active model.
    Status {
        /// Output as JSON for scripting.
        #[arg(long)]
        json: bool,
    },
    /// Diagnose provider credentials and optionally test the active provider.
    Doctor {
        /// Output as JSON for scripting.
        #[arg(long)]
        json: bool,
        /// Run a live provider test for the current provider.
        #[arg(long)]
        test: bool,
    },
    /// Login to a provider.
    Login {
        /// Provider id, for example codex, anthropic, openai, mistral.
        provider: String,
    },
}

#[derive(Subcommand)]
pub(crate) enum GatewayCommands {
    /// Start the kernel daemon.
    Start,
    /// Stop the running daemon.
    Stop,
    /// Show daemon status.
    Status {
        /// Output as JSON for scripting.
        #[arg(long)]
        json: bool,
        /// Show full operational context (paths, auth, channels, media).
        #[arg(long, short = 'v')]
        verbose: bool,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub(crate) enum ServiceManagerArg {
    /// Pick the installed platform service, then tmux/background fallback.
    Auto,
    /// macOS LaunchAgent (`~/Library/LaunchAgents/ai.captain.daemon.plist`).
    Launchd,
    /// Linux systemd service (`captain.service`).
    Systemd,
    /// Detached tmux session fallback (`captain-daemon`).
    Tmux,
}

#[derive(Subcommand)]
pub(crate) enum ServiceCommands {
    /// Install a native service definition for this user/platform.
    Install {
        /// Service manager to install.
        #[arg(long, value_enum, default_value_t = ServiceManagerArg::Auto)]
        manager: ServiceManagerArg,
        /// Overwrite an existing service definition.
        #[arg(long)]
        force: bool,
        /// Show the file that would be written without writing it.
        #[arg(long)]
        dry_run: bool,
        /// Start the service after installing it.
        #[arg(long)]
        start: bool,
    },
    /// Start Captain through the installed service manager or fallback.
    Start {
        /// Service manager to use.
        #[arg(long, value_enum, default_value_t = ServiceManagerArg::Auto)]
        manager: ServiceManagerArg,
    },
    /// Stop Captain through the installed service manager or fallback.
    Stop {
        /// Service manager to use.
        #[arg(long, value_enum, default_value_t = ServiceManagerArg::Auto)]
        manager: ServiceManagerArg,
    },
    /// Restart Captain through the installed service manager or fallback.
    Restart {
        /// Service manager to use.
        #[arg(long, value_enum, default_value_t = ServiceManagerArg::Auto)]
        manager: ServiceManagerArg,
    },
    /// Show service manager, daemon, and fallback status.
    Status {
        /// Output as JSON for scripting.
        #[arg(long)]
        json: bool,
    },
    /// Show service logs.
    Logs {
        /// Number of lines to show.
        #[arg(long, default_value = "80")]
        lines: usize,
        /// Follow logs.
        #[arg(long, short)]
        follow: bool,
    },
}

#[derive(Subcommand)]
pub(crate) enum ProcessCommands {
    /// List managed background processes from the daemon status snapshot.
    List {
        /// Output as JSON for scripting.
        #[arg(long)]
        json: bool,
    },
    /// Stop a managed background process intentionally.
    Kill {
        /// Process ID, for example proc_1.
        process_id: String,
    },
}

#[derive(Subcommand)]
pub(crate) enum ApprovalsCommands {
    /// List pending approvals.
    List {
        /// Output as JSON for scripting.
        #[arg(long)]
        json: bool,
    },
    /// Approve a pending request.
    Approve {
        /// Approval ID.
        id: String,
    },
    /// Reject a pending request.
    Reject {
        /// Approval ID.
        id: String,
    },
}

#[derive(Subcommand)]
pub(crate) enum CronCommands {
    /// List scheduled jobs.
    List {
        /// Output as JSON for scripting.
        #[arg(long)]
        json: bool,
    },
    /// Create a new scheduled job.
    Create {
        /// Agent name or ID to run.
        agent: String,
        /// Cron expression (e.g. "0 */6 * * *").
        spec: String,
        /// Prompt to send when the job fires.
        prompt: String,
        /// Optional job name (auto-generated if omitted).
        #[arg(long)]
        name: Option<String>,
    },
    /// Delete a scheduled job.
    Delete {
        /// Job ID.
        id: String,
    },
    /// Enable a disabled job.
    Enable {
        /// Job ID.
        id: String,
    },
    /// Disable a job without deleting it.
    Disable {
        /// Job ID.
        id: String,
    },
}

#[derive(Subcommand)]
pub(crate) enum SecurityCommands {
    /// Show security status summary.
    Status {
        /// Output as JSON for scripting.
        #[arg(long)]
        json: bool,
    },
    /// Show recent audit trail entries.
    Audit {
        /// Maximum number of entries to show.
        #[arg(long, default_value = "20")]
        limit: usize,
        /// Output as JSON for scripting.
        #[arg(long)]
        json: bool,
    },
    /// Verify audit trail integrity (Merkle chain).
    Verify,
}

#[derive(Subcommand)]
pub(crate) enum MemoryCommands {
    /// Show managed MemPalace runtime and palace readiness.
    Status {
        /// Output as JSON for scripting.
        #[arg(long)]
        json: bool,
    },
    /// Install or repair Captain's managed MemPalace runtime.
    Install {
        /// Return success even when provisioning is incomplete.
        #[arg(long)]
        best_effort: bool,
        /// Reinstall pinned runtime components.
        #[arg(long)]
        force: bool,
    },
    /// Diagnose the managed MemPalace runtime.
    Doctor {
        /// Output as JSON for scripting.
        #[arg(long)]
        json: bool,
    },
    /// Internal stdio bridge used by Captain's bundled MCP configuration.
    #[command(hide = true)]
    McpServe,
    /// List KV pairs for an agent.
    List {
        /// Agent name or ID.
        agent: String,
        /// Output as JSON for scripting.
        #[arg(long)]
        json: bool,
    },
    /// Get a specific KV value.
    Get {
        /// Agent name or ID.
        agent: String,
        /// Key name.
        key: String,
        /// Output as JSON for scripting.
        #[arg(long)]
        json: bool,
    },
    /// Set a KV value.
    Set {
        /// Agent name or ID.
        agent: String,
        /// Key name.
        key: String,
        /// Value to store.
        value: String,
    },
    /// Delete a KV pair.
    Delete {
        /// Agent name or ID.
        agent: String,
        /// Key name.
        key: String,
    },
}

#[derive(Subcommand)]
pub(crate) enum DevicesCommands {
    /// List paired devices.
    List {
        /// Output as JSON for scripting.
        #[arg(long)]
        json: bool,
    },
    /// Start a new device pairing flow.
    Pair,
    /// Remove a paired device.
    Remove {
        /// Device ID.
        id: String,
    },
}

#[derive(Subcommand)]
pub(crate) enum WebhooksCommands {
    /// List configured webhooks.
    List {
        /// Output as JSON for scripting.
        #[arg(long)]
        json: bool,
    },
    /// Create a new webhook trigger.
    Create {
        /// Agent name or ID.
        agent: String,
        /// Webhook callback URL.
        url: String,
    },
    /// Delete a webhook.
    Delete {
        /// Webhook ID.
        id: String,
    },
    /// Send a test payload to a webhook.
    Test {
        /// Webhook ID.
        id: String,
    },
}

#[derive(Subcommand)]
pub(crate) enum SystemCommands {
    /// Show detailed system info.
    Info {
        /// Output as JSON for scripting.
        #[arg(long)]
        json: bool,
    },
    /// Show version information.
    Version {
        /// Output as JSON for scripting.
        #[arg(long)]
        json: bool,
    },
}

#[derive(Subcommand)]
pub(crate) enum LoginCommands {
    /// ChatGPT (Codex) device-code flow.
    Codex {
        /// Prompt to pick a model and save it as default after login.
        #[arg(long)]
        with_model: bool,
    },
}
