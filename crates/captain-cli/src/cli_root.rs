use std::path::PathBuf;

use clap::{Parser, Subcommand};

use crate::{
    AgentCommands, ApprovalsCommands, AuthCommands, AutonomyCommands, ChannelCommands,
    ConfigCommands, CronCommands, DevicesCommands, EmbeddingsCommands, GatewayCommands,
    HandCommands, IntegrationCommands, LogTarget, LoginCommands, MemoryCommands, ModelsCommands,
    ProcessCommands, ProjectCommands, ScaffoldKind, SecurityCommands, ServiceCommands,
    SessionsCommands, SkillCommands, SnapshotCommands, SshCommands, SystemCommands,
    TriggerCommands, VaultCommands, VoiceCommands, WebhooksCommands, WorkflowCommands,
};

const AFTER_HELP: &str = "\
\x1b[1mHint:\x1b[0m Commands suffixed with [*] have subcommands. Run `<command> --help` for details.

\x1b[1;36mExamples:\x1b[0m
  captain init                 Initialize config and data directories
  captain start                Start the kernel daemon
  captain tui                  Launch the interactive terminal UI
  captain terminal             Open the browser terminal
  captain chat                 Quick chat with the default agent
  captain agent new coder      Spawn a new agent from a template
  captain models list          Browse available LLM models
  captain add github           Install the GitHub integration
  captain doctor               Run diagnostic health checks
  captain channel setup        Interactive channel setup wizard
  captain cron list            List scheduled jobs
  captain uninstall            Completely remove Captain from your system

\x1b[1;36mQuick Start:\x1b[0m
  1. captain init              Set up config + API key
  2. captain start             Launch the daemon
  3. captain chat              Start chatting!

\x1b[1;36mMore:\x1b[0m
  Web terminal:  http://127.0.0.1:50051/terminal (when daemon is running)";

/// Captain — the open-source Agent Operating System.
#[derive(Parser)]
#[command(
    name = "captain",
    version,
    about = "Captain \u{2014} Open-source Agent Operating System",
    long_about = "Captain \u{2014} Open-source Agent Operating System\n\n\
                  Deploy, manage, and orchestrate AI agents from your terminal.\n\
                  Six operational hubs \u{00b7} durable work \u{00b7} observable autonomy.",
    after_help = AFTER_HELP,
)]
pub(crate) struct Cli {
    /// Path to config file.
    #[arg(long, global = true)]
    pub(crate) config: Option<PathBuf>,

    #[command(subcommand)]
    pub(crate) command: Option<Commands>,
}

#[derive(Subcommand)]
pub(crate) enum Commands {
    /// Initialize Captain (create ~/.captain/ and default config).
    Init {
        /// Quick mode: no prompts, just write config + .env (for CI/scripts).
        #[arg(long)]
        quick: bool,
    },
    /// Start the Captain kernel daemon (API server + kernel).
    Start {
        /// Auto-approve all tool calls (no confirmation prompts).
        #[arg(long)]
        yolo: bool,
    },
    /// Stop the running daemon.
    Stop,
    /// Manage agents (new, list, chat, kill, spawn) [*].
    #[command(subcommand)]
    Agent(AgentCommands),
    /// Manage workflows (list, create, run) [*].
    #[command(subcommand)]
    Workflow(WorkflowCommands),
    /// Manage project runtime operator actions [*].
    #[command(subcommand, alias = "projects")]
    Project(ProjectCommands),
    /// Manage event triggers (list, create, delete) [*].
    #[command(subcommand)]
    Trigger(TriggerCommands),
    /// Manage skills (install, list, search, create, remove) [*].
    #[command(subcommand)]
    Skill(SkillCommands),
    /// Manage channel integrations (setup, test, enable, disable) [*].
    #[command(subcommand, alias = "channels")]
    Channel(ChannelCommands),
    /// Manage hands (list, activate, deactivate, info) [*].
    #[command(subcommand, hide = true)]
    Hand(HandCommands),
    /// Manage SSH credentials in the vault (Q.6) — add, list, test, remove, use.
    #[command(subcommand)]
    Ssh(SshCommands),
    /// Auto-install native channel/TTS integrations (R.3.2) — interactive `setup` flow.
    #[command(subcommand)]
    Integration(IntegrationCommands),
    /// Manage native no-API-key STT/TTS assets.
    #[command(subcommand)]
    Voice(VoiceCommands),
    /// Manage native local embedding runtime assets.
    #[command(subcommand)]
    Embeddings(EmbeddingsCommands),
    /// Show or edit configuration (show, edit, get, set, keys) [*].
    #[command(subcommand)]
    Config(ConfigCommands),
    /// Quick chat with the default agent.
    Chat {
        /// Optional agent name or ID to chat with.
        agent: Option<String>,
        /// Use plain line-based chat for SSH/native terminal scrollback.
        #[arg(long)]
        plain: bool,
    },
    /// Show kernel status.
    Status {
        /// Output as JSON for scripting.
        #[arg(long)]
        json: bool,
        /// Show full operational context (paths, auth, channels, media).
        #[arg(long, short = 'v')]
        verbose: bool,
    },
    /// Inspect and stop managed background processes [*].
    #[command(subcommand)]
    Process(ProcessCommands),
    /// Run diagnostic health checks.
    Doctor {
        /// Output as JSON for scripting.
        #[arg(long)]
        json: bool,
        /// Attempt to auto-fix issues (create missing dirs/config).
        #[arg(long)]
        repair: bool,
        /// Include daemon operational inventory in addition to health checks.
        #[arg(long)]
        full: bool,
        /// Scan source/docs/local state for legacy public branding.
        #[arg(long)]
        brand_audit: bool,
    },
    /// Update Captain to the latest release (download, verify, swap, restart).
    Update {
        /// Only check whether a newer version is available.
        #[arg(long)]
        check: bool,
        /// Skip the confirmation prompt.
        #[arg(long)]
        yes: bool,
        /// Install a specific version instead of the latest.
        #[arg(long)]
        version: Option<String>,
    },
    /// Open the web terminal in the default browser.
    Terminal,
    /// Generate shell completion scripts.
    Completion {
        /// Shell to generate completions for.
        #[arg(value_enum)]
        shell: clap_complete::Shell,
    },
    /// Start MCP (Model Context Protocol) server over stdio.
    Mcp,
    /// Add an integration (one-click MCP server setup).
    Add {
        /// Integration name (e.g., "github", "slack", "notion").
        name: String,
        /// API key or token to store in the vault.
        #[arg(long)]
        key: Option<String>,
    },
    /// Remove an installed integration.
    Remove {
        /// Integration name.
        name: String,
    },
    /// List or search integrations.
    Integrations {
        /// Search query (optional — lists all if omitted).
        query: Option<String>,
        /// Render the registry as INTEGRATIONS.md instead of a table.
        #[arg(long)]
        doc: bool,
        /// Optional output file when --doc is set; default is stdout.
        #[arg(long)]
        out: Option<PathBuf>,
    },
    /// Manage the credential vault (init, set, list, remove) [*].
    #[command(subcommand)]
    Vault(VaultCommands),
    /// Scaffold a new skill or integration template.
    New {
        /// What to scaffold.
        #[arg(value_enum)]
        kind: ScaffoldKind,
    },
    /// Launch the interactive terminal UI.
    Tui,
    /// Sign into a provider via OAuth (no API key needed).
    #[command(subcommand)]
    Login(LoginCommands),
    /// Provider/model authentication status and diagnostics.
    #[command(subcommand)]
    Auth(AuthCommands),
    /// Browse models, aliases, and providers [*].
    #[command(subcommand, alias = "model")]
    Models(ModelsCommands),
    /// Daemon control (start, stop, status) [*].
    #[command(subcommand)]
    Gateway(GatewayCommands),
    /// Manage the installed daemon service (launchd/systemd/tmux fallback).
    #[command(subcommand)]
    Service(ServiceCommands),
    /// Manage execution approvals (list, approve, reject) [*].
    #[command(subcommand)]
    Approvals(ApprovalsCommands),
    /// Manage scheduled jobs (list, create, delete, enable, disable) [*].
    #[command(subcommand)]
    Cron(CronCommands),
    /// Inspect autonomous activity (jobs, triggers, workflows, approvals) [*].
    #[command(subcommand)]
    Autonomy(AutonomyCommands),
    /// Manage conversation sessions.
    Sessions {
        #[command(subcommand)]
        command: Option<SessionsCommands>,
        /// Optional agent name or ID to filter by when no subcommand is used.
        #[arg(long)]
        agent: Option<String>,
        /// Output as JSON for scripting.
        #[arg(long)]
        json: bool,
    },
    /// Replay a bounded, operator-safe session timeline.
    Replay {
        /// Session ID to replay.
        session_id: String,
        /// Maximum number of timeline events to show.
        #[arg(long, default_value = "80")]
        events: usize,
        /// Output as JSON for scripting.
        #[arg(long)]
        json: bool,
    },
    /// Inspect Captain logs (daemon file or structured runtime events).
    Logs {
        /// Log source to inspect.
        #[arg(value_enum, default_value_t = LogTarget::Daemon)]
        target: LogTarget,
        /// Number of lines to show.
        #[arg(long, default_value = "50")]
        lines: usize,
        /// Follow log output in real time.
        #[arg(long, short)]
        follow: bool,
        /// Only show entries since a duration or UTC timestamp.
        #[arg(long)]
        since: Option<String>,
        /// Agent name/id filter for structured event logs.
        #[arg(long)]
        agent: Option<String>,
        /// Channel name filter for daemon/channel logs.
        #[arg(long)]
        channel: Option<String>,
        /// Output structured event logs as JSON.
        #[arg(long)]
        json: bool,
    },
    /// Quick daemon health check.
    Health {
        /// Output as JSON for scripting.
        #[arg(long)]
        json: bool,
    },
    /// Security tools and audit trail [*].
    #[command(subcommand)]
    Security(SecurityCommands),
    /// Search and manage agent memory (KV store) [*].
    #[command(subcommand)]
    Memory(MemoryCommands),
    /// Device pairing and token management [*].
    #[command(subcommand)]
    Devices(DevicesCommands),
    /// Generate device pairing QR code.
    Qr,
    /// Webhook helpers and trigger management [*].
    #[command(subcommand)]
    Webhooks(WebhooksCommands),
    /// Interactive onboarding wizard.
    Onboard {
        /// Quick non-interactive mode.
        #[arg(long)]
        quick: bool,
    },
    /// One-shot setup wizard: install (Docker recommandé) → provider → canal → launch.
    Setup {
        /// Optional profile shorthand: core, vps, desktop, full-media.
        #[arg(value_name = "PROFILE")]
        target: Option<String>,
        /// Skip the wizard and run a quick non-interactive init from detected env vars.
        #[arg(long)]
        quick: bool,
        /// Run setup without prompts from env vars and optional answers file.
        #[arg(long)]
        non_interactive: bool,
        /// Installation profile metadata: core, vps, desktop, full-media.
        #[arg(long)]
        profile: Option<String>,
        /// Confirm non-interactive setup choices.
        #[arg(long, alias = "confirm")]
        yes: bool,
        /// Read credentials and provider hints from environment.
        #[arg(long)]
        from_env: bool,
        /// TOML answers file for non-interactive setup.
        #[arg(long)]
        answers: Option<PathBuf>,
    },
    /// Interactive setup wizard for credentials and channels.
    Configure,
    /// Send a one-shot message to an agent.
    Message {
        /// Agent name or ID.
        agent: String,
        /// Message text.
        text: String,
        /// Output as JSON for scripting.
        #[arg(long)]
        json: bool,
    },
    /// System info and version [*].
    #[command(subcommand)]
    System(SystemCommands),
    /// Manage local state snapshots for backup and restore.
    #[command(subcommand)]
    Snapshot(SnapshotCommands),
    /// Reset local config and state.
    Reset {
        /// Skip confirmation prompt.
        #[arg(long, alias = "yes")]
        confirm: bool,
        /// Factory reset: stop daemon, snapshot first, then recreate a clean Captain home.
        #[arg(long)]
        factory: bool,
        /// Do not create a pre-reset snapshot.
        #[arg(long)]
        no_snapshot: bool,
        /// Keep secrets files after reset (.env, secrets.env, vault.enc).
        #[arg(long)]
        preserve_secrets: bool,
        /// Keep existing snapshots after reset.
        #[arg(long)]
        preserve_snapshots: bool,
    },
    /// Completely uninstall Captain from your system.
    Uninstall {
        /// Skip confirmation prompt (also --yes).
        #[arg(long, alias = "yes")]
        confirm: bool,
        /// Keep config files (config.toml, .env, secrets.env).
        #[arg(long)]
        keep_config: bool,
    },
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::CommandFactory;

    #[test]
    fn public_help_uses_stable_core_contract_and_hides_hands() {
        let help = Cli::command().render_long_help().to_string();

        assert!(help.contains("Six operational hubs"));
        assert!(help.contains("durable work"));
        assert!(!help.contains("60 skills"));
        assert!(!help.contains("50+ models"));
        assert!(!help.contains("\n  hand "));
    }

    #[test]
    fn frozen_hand_command_remains_available_by_exact_name() {
        let cli = Cli::try_parse_from(["captain", "hand", "list"]).unwrap();

        assert!(matches!(
            cli.command,
            Some(Commands::Hand(HandCommands::List))
        ));
    }
}
