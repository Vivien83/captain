use std::path::PathBuf;

use clap::{Args, Subcommand, ValueEnum};

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub(crate) enum EmailProviderArg {
    /// Connect a Google Gmail account through OAuth.
    Gmail,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub(crate) enum GmailAccessArg {
    /// Send messages only; mailbox reads are not permitted.
    Send,
    /// Read messages without sending or changing labels.
    Read,
    /// Read, send and modify labels.
    Assistant,
}

impl GmailAccessArg {
    pub(crate) fn profile(self) -> captain_types::email::GmailAccessProfile {
        match self {
            Self::Send => captain_types::email::GmailAccessProfile::Send,
            Self::Read => captain_types::email::GmailAccessProfile::Read,
            Self::Assistant => captain_types::email::GmailAccessProfile::Assistant,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub(crate) enum GmailDeliveryStatusArg {
    Pending,
    Delivering,
    RetryWait,
    Delivered,
    Dead,
    Uncertain,
}

impl GmailDeliveryStatusArg {
    pub(crate) fn status(self) -> captain_memory::gmail_automation::GmailAutomationOutboxStatus {
        use captain_memory::gmail_automation::GmailAutomationOutboxStatus as Status;
        match self {
            Self::Pending => Status::Pending,
            Self::Delivering => Status::Delivering,
            Self::RetryWait => Status::RetryWait,
            Self::Delivered => Status::Delivered,
            Self::Dead => Status::Dead,
            Self::Uncertain => Status::Uncertain,
        }
    }
}

// Clap constructs one command variant at startup; boxing individual arguments
// would complicate command handling without reducing steady-state memory.
#[allow(clippy::large_enum_variant)]
#[derive(Subcommand)]
pub(crate) enum GmailRuleCommands {
    /// Create a deterministic mailbox rule for an agent.
    Add {
        /// Stable rule ID. Derived from the name when omitted.
        #[arg(long)]
        id: Option<String>,
        /// Connected Gmail account alias. Uses the default when omitted.
        #[arg(long)]
        account: Option<String>,
        /// Human-readable rule name.
        #[arg(long)]
        name: String,
        /// Match when the sender contains this text.
        #[arg(long)]
        from_contains: Option<String>,
        /// Match when a To or Cc recipient contains this text.
        #[arg(long)]
        recipient_contains: Option<String>,
        /// Match when the subject contains this text.
        #[arg(long)]
        subject_contains: Option<String>,
        /// Gmail label required on every matching message. Repeatable.
        #[arg(long = "all-label")]
        all_label_ids: Vec<String>,
        /// At least one of these Gmail labels must match. Repeatable.
        #[arg(long = "any-label")]
        any_label_ids: Vec<String>,
        /// Agent ID, ID prefix or exact persisted name.
        #[arg(long, default_value = "captain")]
        agent: String,
        /// Trusted operator instruction executed for each matching email.
        #[arg(long)]
        instruction: String,
        /// Include bounded plain-text email body data in the agent turn.
        #[arg(long)]
        include_body: bool,
        /// Maximum plain-text body bytes exposed to the agent.
        #[arg(long, default_value_t = 32 * 1024)]
        max_body_bytes: usize,
        /// Maximum automatic delivery attempts before dead letter.
        #[arg(long, default_value_t = 3)]
        max_delivery_attempts: u8,
        /// Maximum matches accepted per rolling hour.
        #[arg(long, default_value_t = 20)]
        max_fires_per_hour: u16,
        /// Create the rule disabled.
        #[arg(long)]
        disabled: bool,
        /// Output the created rule as JSON.
        #[arg(long)]
        json: bool,
    },
    /// List durable Gmail automation rules.
    List {
        /// Filter by connected Gmail account alias.
        #[arg(long)]
        account: Option<String>,
        /// Output as JSON for scripting.
        #[arg(long)]
        json: bool,
    },
    /// Inspect one durable Gmail automation rule.
    Show {
        /// Stable rule ID.
        id: String,
        /// Output as JSON for scripting.
        #[arg(long)]
        json: bool,
    },
    /// Enable one rule using compare-and-swap persistence.
    Enable {
        /// Stable rule ID.
        id: String,
        /// Output as JSON for scripting.
        #[arg(long)]
        json: bool,
    },
    /// Disable one rule without deleting its audit history.
    Disable {
        /// Stable rule ID.
        id: String,
        /// Output as JSON for scripting.
        #[arg(long)]
        json: bool,
    },
    /// Delete an unused rule. Rules with audit history must be disabled.
    Remove {
        /// Stable rule ID.
        id: String,
        /// Confirm deletion without an interactive prompt.
        #[arg(long)]
        yes: bool,
        /// Output as JSON for scripting.
        #[arg(long)]
        json: bool,
    },
}

#[derive(Subcommand)]
pub(crate) enum GmailDeliveryCommands {
    /// List recent Gmail automation deliveries without exposing message data.
    List {
        /// Filter by durable delivery state.
        #[arg(long, value_enum)]
        status: Option<GmailDeliveryStatusArg>,
        /// Maximum records to return (1 to 1000).
        #[arg(long, default_value_t = 50)]
        limit: usize,
        /// Output as JSON for scripting.
        #[arg(long)]
        json: bool,
    },
    /// Inspect one delivery and its recovery metadata.
    Show {
        /// Durable outbox ID.
        id: String,
        /// Output as JSON for scripting.
        #[arg(long)]
        json: bool,
    },
    /// Explicitly retry a reviewed dead or uncertain delivery.
    Requeue {
        /// Durable outbox ID.
        id: String,
        /// Accept duplicate-execution risk for an uncertain delivery.
        #[arg(long)]
        yes: bool,
        /// Output as JSON for scripting.
        #[arg(long)]
        json: bool,
    },
}

#[derive(Subcommand)]
pub(crate) enum EmailCommands {
    /// Connect or reconnect a Gmail account through Google OAuth.
    Connect {
        /// Email provider. Gmail is currently the native provider.
        #[arg(value_enum, default_value_t = EmailProviderArg::Gmail)]
        provider: EmailProviderArg,
        /// Stable name used by Captain, for example personal or work.
        #[arg(long)]
        alias: Option<String>,
        /// Least-privilege Gmail access profile.
        #[arg(long, value_enum, default_value_t = GmailAccessArg::Assistant)]
        access: GmailAccessArg,
        /// Override the bundled OAuth identity with a Google Desktop app JSON.
        #[arg(long, value_name = "PATH")]
        client_json: Option<PathBuf>,
        /// Suggest one Google account on the consent screen.
        #[arg(long)]
        login_hint: Option<String>,
        /// Make this account the default after connection.
        #[arg(long = "default")]
        make_default: bool,
        /// Print the authorization URL without opening a browser.
        #[arg(long)]
        no_browser: bool,
        /// Fixed loopback port, useful with SSH local port forwarding.
        #[arg(long, value_parser = clap::value_parser!(u16).range(1..))]
        callback_port: Option<u16>,
        /// Output the connected account as JSON.
        #[arg(long)]
        json: bool,
    },
    /// List connected email accounts without exposing credentials.
    Accounts {
        /// Output as JSON for scripting.
        #[arg(long)]
        json: bool,
    },
    /// Show durable local readiness for one or all accounts.
    Status {
        /// Account alias. Uses all accounts when omitted.
        alias: Option<String>,
        /// Output as JSON for scripting.
        #[arg(long)]
        json: bool,
    },
    /// Verify one account live and refresh its access token when needed.
    Test {
        /// Account alias. Uses the default account when omitted.
        alias: Option<String>,
        /// Output as JSON for scripting.
        #[arg(long)]
        json: bool,
    },
    /// Select the default account used when no alias is specified.
    Default {
        /// Connected account alias.
        alias: String,
    },
    /// Remove a local account and optionally revoke its Google grant.
    Disconnect {
        /// Connected account alias.
        alias: String,
        /// Revoke the Google grant before deleting local state.
        #[arg(long)]
        revoke: bool,
        /// Confirm grant revocation without an interactive prompt.
        #[arg(long)]
        yes: bool,
        /// Output the result as JSON.
        #[arg(long)]
        json: bool,
    },
    /// Manage deterministic Gmail-to-agent automation rules.
    Rules {
        #[command(subcommand)]
        command: GmailRuleCommands,
    },
    /// Inspect and recover durable Gmail automation deliveries.
    Deliveries {
        #[command(subcommand)]
        command: GmailDeliveryCommands,
    },
}

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
    /// Set an agent property (model or reasoning).
    Set {
        /// Agent ID (UUID).
        agent_id: String,
        /// Field to set (model or reasoning).
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
    /// Verify audit trail integrity (versioned SHA-256 hash chain).
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
    /// List pending pairing requests awaiting an operator decision.
    Pending {
        /// Output as JSON for scripting.
        #[arg(long)]
        json: bool,
    },
    /// Review and approve the one-time code displayed by a device.
    Approve {
        /// One-time display code shown on the device.
        code: String,
        /// Grant mutation authority when the device requested it.
        #[arg(long)]
        allow_mutation: bool,
    },
    /// Deny a pending request by its request ID.
    Deny {
        /// Pairing request ID shown by `captain devices pending`.
        request_id: String,
    },
    /// Remove a paired device.
    Remove {
        /// Device ID.
        id: String,
    },
}

#[derive(Subcommand)]
pub(crate) enum NodeCommands {
    /// Pair this machine with a Hub over outbound HTTPS.
    Pair(Box<NodePairArgs>),
    /// Run the outbound Node worker in the foreground.
    Run,
    /// Show local pairing and durable rail state without exposing local paths.
    Status {
        /// Output as JSON for scripting.
        #[arg(long)]
        json: bool,
    },
    /// Remove local Node credentials and durable rail state.
    Reset {
        /// Confirm the local reset.
        #[arg(long)]
        yes: bool,
    },
}

#[derive(Subcommand)]
pub(crate) enum ClientCommands {
    /// Pair this terminal or desktop interface with a Captain Hub.
    Pair(Box<ClientPairArgs>),
    /// List independent Captain profiles without exposing their origins.
    List {
        /// Output as JSON for scripting.
        #[arg(long)]
        json: bool,
    },
    /// Select the Captain profile used by future Client processes.
    Use {
        /// Full profile UUID, unique UUID prefix, or unique display name.
        profile: String,
    },
    /// Show the local lightweight Client state without exposing the Hub URL.
    Status {
        /// Inspect a profile other than the active one.
        #[arg(long)]
        profile: Option<String>,
        /// Output as JSON for scripting.
        #[arg(long)]
        json: bool,
    },
    /// Remove the local Client identity and Hub configuration.
    Reset {
        /// Confirm the local reset.
        #[arg(long)]
        yes: bool,
    },
}

#[derive(Args)]
pub(crate) struct ClientPairArgs {
    /// HTTPS origin of the Captain Hub.
    #[arg(long, value_name = "HTTPS_URL")]
    pub(crate) hub: String,
    /// Pair into an existing local profile instead of selecting by Hub origin.
    #[arg(long)]
    pub(crate) profile: Option<String>,
    /// Local label for this Captain authority (never sent to the Hub).
    #[arg(long)]
    pub(crate) label: Option<String>,
    /// Human-readable Client name shown in Appareils.
    #[arg(long)]
    pub(crate) name: Option<String>,
    /// PEM bundle for an enterprise certificate authority.
    #[arg(long, value_name = "PEM_FILE")]
    pub(crate) ca_bundle: Option<std::path::PathBuf>,
    /// Explicit HTTP(S) proxy URL; credentials must not be embedded.
    #[arg(long, value_name = "PROXY_URL", conflicts_with = "no_proxy")]
    pub(crate) proxy: Option<String>,
    /// Username for an explicit authenticated proxy.
    #[arg(long, requires = "proxy")]
    pub(crate) proxy_username: Option<String>,
    /// Captain secret name containing the explicit proxy password.
    #[arg(long, requires_all = ["proxy", "proxy_username"])]
    pub(crate) proxy_password_secret: Option<String>,
    /// Ignore proxy environment variables.
    #[arg(long, conflicts_with = "proxy")]
    pub(crate) no_proxy: bool,
    /// Print the approval URL without opening a browser.
    #[arg(long)]
    pub(crate) no_browser: bool,
}

#[derive(Args)]
pub(crate) struct NodePairArgs {
    /// HTTPS origin of the Captain Hub.
    #[arg(long, value_name = "HTTPS_URL")]
    pub(crate) hub: String,
    /// Local workspace directory exposed under a logical identifier.
    #[arg(long, value_name = "DIR")]
    pub(crate) workspace: std::path::PathBuf,
    /// Logical workspace identifier visible to the Hub.
    #[arg(long, default_value = "workspace-main")]
    pub(crate) workspace_id: String,
    /// Human-readable Node name.
    #[arg(long)]
    pub(crate) name: Option<String>,
    /// Human-readable workspace label.
    #[arg(long)]
    pub(crate) label: Option<String>,
    /// Request mutation authority for this workspace (read-only by default).
    #[arg(long)]
    pub(crate) allow_mutation: bool,
    /// PEM bundle for an enterprise certificate authority.
    #[arg(long, value_name = "PEM_FILE")]
    pub(crate) ca_bundle: Option<std::path::PathBuf>,
    /// Explicit HTTP(S) proxy URL; credentials must not be embedded.
    #[arg(long, value_name = "PROXY_URL", conflicts_with = "no_proxy")]
    pub(crate) proxy: Option<String>,
    /// Username for an explicit authenticated proxy.
    #[arg(long, requires = "proxy")]
    pub(crate) proxy_username: Option<String>,
    /// Captain secret name containing the explicit proxy password.
    #[arg(long, requires_all = ["proxy", "proxy_username"])]
    pub(crate) proxy_password_secret: Option<String>,
    /// Ignore proxy environment variables.
    #[arg(long, conflicts_with = "proxy")]
    pub(crate) no_proxy: bool,
    /// Print the approval URL without opening a browser.
    #[arg(long)]
    pub(crate) no_browser: bool,
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
