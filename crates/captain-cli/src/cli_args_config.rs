use std::path::PathBuf;

use clap::Subcommand;

#[derive(Subcommand)]
pub(crate) enum SkillCommands {
    /// Install a skill from Captain Marketplace or a local directory.
    Install {
        /// Skill name, local path, or git URL.
        source: String,
    },
    /// List installed skills.
    List,
    /// Remove an installed skill.
    Remove {
        /// Skill name.
        name: String,
    },
    /// Search installed and bundled skills by query or family.
    Search {
        /// Search query.
        query: String,
    },
    /// Create a new skill scaffold.
    Create,
    /// Generate a SKILLS.md reference from installed skills (auto-doc).
    Doc {
        /// Optional output file. Default: stdout.
        #[arg(long)]
        out: Option<PathBuf>,
    },
}

#[derive(Subcommand)]
pub(crate) enum ChannelCommands {
    /// List configured channels and their status.
    List,
    /// Interactive setup wizard for a channel.
    Setup {
        /// Channel name (telegram, discord, slack, whatsapp, etc.). Shows picker if omitted.
        channel: Option<String>,
    },
    /// Test a channel by sending a test message.
    Test {
        /// Channel name.
        channel: String,
    },
    /// Enable a channel.
    Enable {
        /// Channel name.
        channel: String,
    },
    /// Disable a channel without removing its configuration.
    Disable {
        /// Channel name.
        channel: String,
    },
    /// Manage inbound channel queue operations.
    #[command(alias = "inbound-queue")]
    Inbound {
        #[command(subcommand)]
        command: ChannelInboundCommands,
    },
}

#[derive(Subcommand)]
pub(crate) enum ChannelInboundCommands {
    /// Manage inbound dead letters.
    #[command(name = "dead-letters", alias = "deadletters")]
    DeadLetters {
        #[command(subcommand)]
        command: ChannelDeadLetterCommands,
    },
}

#[derive(Subcommand)]
pub(crate) enum ChannelDeadLetterCommands {
    /// Clear handled inbound dead letters without printing message content.
    Clear {
        /// Optional active channel filter, for example telegram.
        #[arg(long)]
        channel: Option<String>,
    },
}

#[derive(Subcommand)]
pub(crate) enum HandCommands {
    /// List all available hands.
    List,
    /// Show currently active hand instances.
    Active,
    /// Install a hand from a local directory containing HAND.toml.
    Install {
        /// Path to the hand directory (must contain HAND.toml).
        path: String,
    },
    /// Activate a hand by ID.
    Activate {
        /// Hand ID (e.g. "clip", "lead", "researcher").
        id: String,
    },
    /// Deactivate an active hand instance.
    Deactivate {
        /// Hand ID.
        id: String,
    },
    /// Show detailed info about a hand.
    Info {
        /// Hand ID.
        id: String,
    },
    /// Check dependency status for a hand.
    CheckDeps {
        /// Hand ID.
        id: String,
    },
    /// Install missing dependencies for a hand.
    InstallDeps {
        /// Hand ID.
        id: String,
    },
    /// Pause a running hand instance.
    Pause {
        /// Instance ID (from `hand active`).
        id: String,
    },
    /// Resume a paused hand instance.
    Resume {
        /// Instance ID (from `hand active`).
        id: String,
    },
}

#[derive(Subcommand)]
pub(crate) enum SshCommands {
    /// Add a new SSH credential to the vault (interactive).
    Add {
        /// Friendly alias (e.g. "prod-server").
        name: String,
    },
    /// List all stored SSH credentials.
    List,
    /// Test connectivity to a host (TCP only — full SSH handshake in Q.7).
    Test {
        /// Alias to test.
        name: String,
    },
    /// Remove an SSH credential from the vault.
    Remove {
        /// Alias to remove.
        name: String,
    },
    /// Set an alias as the default for the ssh_* tools.
    Use {
        /// Alias to mark as default.
        name: String,
    },
    /// Manage the persistent known_hosts store (Q.7c.b).
    #[command(subcommand, name = "known-hosts")]
    KnownHosts(KnownHostsCommands),
}

#[derive(Subcommand)]
pub(crate) enum IntegrationCommands {
    /// Interactive setup of a native channel or TTS integration
    /// (telegram, tts_elevenlabs). Backs up config.toml, vaults the
    /// secrets, patches the TOML in place, and notifies the daemon to
    /// hot-reload the affected adapter.
    Setup {
        /// Integration name (telegram | tts_elevenlabs).
        name: String,
        /// Skip the live remote test (default: also runs a getMe / voices ping).
        #[arg(long)]
        no_test: bool,
    },
    /// List native integrations supported by `captain integration setup`.
    List,
}

#[derive(Subcommand)]
pub(crate) enum KnownHostsCommands {
    /// Show the current known_hosts file content (~/.captain/known_hosts).
    List,
    /// Clear all stored host keys (backup is kept). Next connect re-learns.
    Clear,
    /// Show or set the verification mode.
    /// Run without argument to show; pass `strict|tofu_learn|insecure` to set.
    Mode {
        /// `strict` (refuse unknown), `tofu_learn` (default), or `insecure`.
        mode: Option<String>,
    },
}

#[derive(Subcommand)]
pub(crate) enum ConfigCommands {
    /// Show the current configuration.
    Show,
    /// Open the configuration file in your editor.
    Edit,
    /// Get a config value by dotted key path (e.g. "default_model.provider").
    Get {
        /// Dotted key path (e.g. "default_model.provider", "api_listen").
        key: String,
    },
    /// Set a config value (warning: strips TOML comments).
    Set {
        /// Dotted key path.
        key: String,
        /// New value.
        value: String,
    },
    /// Remove a config key (warning: strips TOML comments).
    Unset {
        /// Dotted key path to remove (e.g. "api.cors_origin").
        key: String,
    },
    /// Save an API key to ~/.captain/.env (prompts interactively).
    SetKey {
        /// Provider name (groq, anthropic, openai, gemini, deepseek, etc.).
        provider: String,
    },
    /// Remove an API key from ~/.captain/.env.
    DeleteKey {
        /// Provider name.
        provider: String,
    },
    /// Test provider connectivity with the stored API key.
    TestKey {
        /// Provider name.
        provider: String,
    },
    /// Initialise ~/.captain/config.toml with every configurable field set to its default.
    InitFull {
        /// Overwrite without prompting (a backup is still taken).
        #[arg(long)]
        force: bool,
    },
    /// Compare the current config.toml against the full schema.
    Doctor,
    /// Dump the full default config.toml template to stdout.
    Schema,
    /// Append any missing top-level sections to config.toml.
    Reconcile,
    /// Print the workspace `.captain.toml` discovered from the current directory.
    Workspace,
}
