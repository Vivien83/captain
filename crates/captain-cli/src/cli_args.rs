use std::path::PathBuf;

use clap::{Subcommand, ValueEnum};

#[derive(Subcommand)]
pub(crate) enum VaultCommands {
    /// Initialize the credential vault.
    Init,
    /// Store a credential in the vault.
    Set {
        /// Credential key (env var name).
        key: String,
    },
    /// List all keys in the vault (values are hidden).
    List,
    /// Remove a credential from the vault.
    Remove {
        /// Credential key.
        key: String,
    },
}

#[derive(Subcommand)]
pub(crate) enum SnapshotCommands {
    /// Create a compressed snapshot of ~/.captain.
    Create {
        /// Human-readable reason stored in the sidecar metadata.
        #[arg(long)]
        reason: Option<String>,
        /// Output as JSON for scripting.
        #[arg(long)]
        json: bool,
    },
    /// List local snapshots.
    List {
        /// Output as JSON for scripting.
        #[arg(long)]
        json: bool,
    },
    /// Restore a snapshot id or archive path.
    Restore {
        /// Snapshot id, filename, or archive path.
        id: String,
        /// Skip confirmation prompt.
        #[arg(long, alias = "yes")]
        confirm: bool,
    },
    /// Prune old snapshots, keeping the newest N.
    Prune {
        /// Number of newest snapshots to keep.
        #[arg(long, default_value_t = 10)]
        keep: usize,
        /// Show what would be deleted without deleting.
        #[arg(long)]
        dry_run: bool,
        /// Required for actual deletion.
        #[arg(long, alias = "yes")]
        confirm: bool,
    },
}

#[derive(Subcommand)]
pub(crate) enum VoiceCommands {
    /// Show native STT/TTS readiness.
    Status {
        /// Output as JSON for scripting.
        #[arg(long)]
        json: bool,
    },
    /// Install or repair native STT/TTS assets without API keys.
    Install {
        /// Continue even if optional premium assets fail.
        #[arg(long)]
        best_effort: bool,
        /// Re-download existing assets.
        #[arg(long)]
        force: bool,
    },
    /// Run a local voice self-test.
    Test {
        /// Output as JSON for scripting.
        #[arg(long)]
        json: bool,
    },
    /// Diagnose native voice dependencies and config.
    Doctor {
        /// Output as JSON for scripting.
        #[arg(long)]
        json: bool,
    },
    /// Remove downloaded native voice models and runtime files.
    Uninstall {
        /// Skip confirmation.
        #[arg(long, alias = "yes")]
        confirm: bool,
    },
}

#[derive(Subcommand)]
pub(crate) enum EmbeddingsCommands {
    /// Show native local embedding runtime readiness.
    Status {
        /// Output as JSON for scripting.
        #[arg(long)]
        json: bool,
    },
    /// Install or repair the native ONNX Runtime used by local embeddings.
    Install {
        /// Continue even if the runtime download fails.
        #[arg(long)]
        best_effort: bool,
        /// Re-download existing runtime assets.
        #[arg(long)]
        force: bool,
    },
    /// Diagnose native embedding runtime dependencies.
    Doctor {
        /// Output as JSON for scripting.
        #[arg(long)]
        json: bool,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub(crate) enum LogTarget {
    /// The daemon process log file (`~/.captain/captain.log`).
    Daemon,
    /// The TUI log file (`~/.captain/tui.log`).
    Tui,
    /// Structured runtime events from `sessions_events`.
    Events,
    /// Structured tool start/end/result events.
    Tools,
    /// Structured events for one agent/session.
    Agent,
    /// Channel-related daemon lines and structured events.
    Channel,
    /// Error/warning lines plus structured failed tool events.
    Errors,
    /// Daemon lines plus structured runtime events.
    All,
}

#[derive(Subcommand)]
pub(crate) enum SessionsCommands {
    /// List persisted sessions.
    List {
        /// Optional agent name or ID to filter by.
        #[arg(long)]
        agent: Option<String>,
        /// Output as JSON for scripting.
        #[arg(long)]
        json: bool,
    },
    /// Show the active session for an agent.
    Current {
        /// Agent name or ID. Defaults to the primary `captain` agent.
        agent: Option<String>,
        /// Output as JSON for scripting.
        #[arg(long)]
        json: bool,
    },
    /// Switch an agent to an existing session.
    Resume {
        /// Session UUID to resume.
        session_id: String,
        /// Agent name or ID. Defaults to the session owner when available.
        #[arg(long)]
        agent: Option<String>,
        /// Output as JSON for scripting.
        #[arg(long)]
        json: bool,
    },
    /// Resume the newest persisted session for an agent.
    Continue {
        /// Agent name or ID. Defaults to the primary `captain` agent.
        #[arg(long)]
        agent: Option<String>,
        /// Output as JSON for scripting.
        #[arg(long)]
        json: bool,
    },
    /// Search sessions by metadata and recent message text.
    Search {
        /// Case-insensitive query.
        query: String,
        /// Optional agent name or ID to filter by.
        #[arg(long)]
        agent: Option<String>,
        /// Maximum results to show.
        #[arg(long, default_value_t = 20)]
        limit: usize,
        /// Output as JSON for scripting.
        #[arg(long)]
        json: bool,
    },
    /// Export a session as JSON or Markdown.
    Export {
        /// Session UUID to export.
        session_id: String,
        /// Optional output file. Defaults to stdout.
        #[arg(long)]
        out: Option<PathBuf>,
        /// Export format.
        #[arg(long, value_enum, default_value_t = SessionExportFormat::Json)]
        format: SessionExportFormat,
    },
    /// Delete old sessions with explicit safeguards.
    Prune {
        /// Agent name or ID to prune. Defaults to all agents.
        #[arg(long)]
        agent: Option<String>,
        /// Keep at least the newest N sessions after filtering.
        #[arg(long)]
        keep: Option<usize>,
        /// Delete only sessions older than a duration or UTC timestamp.
        #[arg(long)]
        older_than: Option<String>,
        /// Show what would be deleted without deleting.
        #[arg(long)]
        dry_run: bool,
        /// Required for actual deletion.
        #[arg(long)]
        yes: bool,
    },
}

#[derive(Subcommand)]
pub(crate) enum AutonomyCommands {
    /// Show autonomous runtime state and recent actions.
    Status {
        /// Output as JSON for scripting.
        #[arg(long)]
        json: bool,
        /// Number of recent actions/errors to show.
        #[arg(long, default_value_t = 8)]
        lines: usize,
        /// Only consider recent structured events since a duration or UTC timestamp.
        #[arg(long)]
        since: Option<String>,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub(crate) enum SessionExportFormat {
    Json,
    Markdown,
}

#[derive(Clone, ValueEnum)]
pub(crate) enum ScaffoldKind {
    Skill,
    Integration,
}
