//! Captain CLI — command-line interface for the Captain Agent OS.
//!
//! When a daemon is running (`captain start`), the CLI talks to it over HTTP.
//! Otherwise, commands boot an in-process kernel (single-shot mode).

mod agent_api_sheet;
mod bundled_agents;
mod cli_args;
mod cli_args_config;
mod cli_args_ops;
mod cli_args_project;
mod cli_root;
mod cli_runtime;
mod cli_support;
mod commands;
mod daemon_api;
mod dotenv;
mod i18n;
mod launcher;
mod mcp;
pub mod progress;
mod snapshot;
pub mod table;
mod templates;
#[cfg(test)]
mod tests;
mod tui;
mod ui;
pub mod workspace_config;

use clap::Parser;
pub(crate) use cli_args::{
    AutonomyCommands, EmbeddingsCommands, LogTarget, ScaffoldKind, SessionExportFormat,
    SessionsCommands, SnapshotCommands, VaultCommands, VoiceCommands,
};
pub(crate) use cli_args_config::{
    ChannelCommands, ChannelDeadLetterCommands, ChannelInboundCommands, ConfigCommands,
    HandCommands, IntegrationCommands, KnownHostsCommands, SkillCommands, SshCommands,
};
pub(crate) use cli_args_ops::{
    AgentCommands, ApprovalsCommands, AuthCommands, CronCommands, DevicesCommands, GatewayCommands,
    LoginCommands, MemoryCommands, ModelsCommands, ProcessCommands, SecurityCommands,
    ServiceCommands, ServiceManagerArg, SystemCommands, TriggerCommands, WebhooksCommands,
    WorkflowCommands,
};
pub(crate) use cli_args_project::{
    ProjectCommands, ProjectTaskCommands, ProjectTaskStatusArg, ProjectToolDecisionArg,
};
pub(crate) use cli_root::{Cli, Commands};
pub(crate) use cli_runtime::{
    boot_kernel, boot_kernel_error, captain_version, cli_captain_home, init_tracing_file,
    init_tracing_stderr, install_ctrlc_handler, maybe_print_version_and_exit,
};
pub(crate) use cli_support::{
    captain_home, command_version, copy_dir_recursive, find_captain_cli_on_path, open_in_browser,
    path_eq_best_effort, prompt_input, prompt_secret, restrict_dir_permissions,
    restrict_file_permissions, test_api_key, truncate_display,
};
pub(crate) use commands::daemon::start_daemon_background;
pub(crate) use commands::init::{
    check_ollama_available, cmd_init, codex_auth_available, detect_best_provider, provider_list,
    write_default_model_config,
};
pub(crate) use daemon_api::{
    daemon_auth_headers, daemon_client, daemon_json, find_daemon, require_daemon,
};

fn main() {
    // Load ~/.captain/.env into process environment (system env takes priority).
    dotenv::load_dotenv();
    maybe_print_version_and_exit();

    let cli = Cli::parse();

    // Determine if this invocation launches a ratatui TUI.
    // TUI modes must NOT install the Ctrl+C handler (it calls process::exit
    // which bypasses ratatui::restore and leaves the terminal in raw mode).
    // TUI modes also need file-based tracing (stderr output corrupts the TUI).
    let is_launcher = cli.command.is_none() && std::io::IsTerminal::is_terminal(&std::io::stdout());
    let is_tui_mode = is_launcher
        || matches!(cli.command, Some(Commands::Tui))
        || matches!(cli.command, Some(Commands::Chat { .. }))
        || matches!(
            cli.command,
            Some(Commands::Agent(AgentCommands::Chat { .. }))
        );

    if is_tui_mode {
        init_tracing_file();
    } else {
        // CLI subcommands: install Ctrl+C handler for clean interrupt of
        // blocking read_line calls, and trace to stderr.
        install_ctrlc_handler();
        init_tracing_stderr();
    }

    commands::dispatch::dispatch_cli(cli);
}
