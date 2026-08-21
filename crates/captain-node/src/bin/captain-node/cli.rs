use clap::{Args, Parser, Subcommand};
use std::path::PathBuf;

#[derive(Debug, Parser)]
#[command(
    name = "captain-node",
    about = "Outbound-only lightweight execution node for Captain",
    disable_version_flag = true,
    arg_required_else_help = true
)]
pub(crate) struct Cli {
    /// Captain state directory. Defaults to CAPTAIN_HOME or ~/.captain.
    #[arg(long, global = true, value_name = "DIR")]
    pub(crate) home: Option<PathBuf>,
    /// Print the canonical Captain build version.
    #[arg(long, global = true)]
    pub(crate) version: bool,
    #[command(subcommand)]
    pub(crate) command: Option<Command>,
}

#[derive(Debug, Subcommand)]
pub(crate) enum Command {
    /// Pair this machine with a Captain Full authority over outbound HTTPS.
    Pair(PairArgs),
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
    /// Manage authenticated proxy passwords in the native secret store.
    ProxySecret {
        #[command(subcommand)]
        command: ProxySecretCommand,
    },
    /// Install or control the native Captain Node service.
    Service {
        #[command(subcommand)]
        command: ServiceCommand,
    },
    /// Internal entry point used only by the native service manager.
    #[command(hide = true)]
    ServiceRuntime,
}

#[derive(Debug, Args)]
pub(crate) struct PairArgs {
    /// HTTPS origin of the Captain Full authority.
    #[arg(long, value_name = "HTTPS_URL")]
    pub(crate) hub: String,
    /// Local workspace directory exposed under a logical identifier.
    #[arg(long, value_name = "DIR")]
    pub(crate) workspace: PathBuf,
    /// Logical workspace identifier visible to the Captain Full authority.
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
    pub(crate) ca_bundle: Option<PathBuf>,
    /// Explicit HTTP(S) proxy URL; credentials must not be embedded.
    #[arg(long, value_name = "PROXY_URL", conflicts_with = "no_proxy")]
    pub(crate) proxy: Option<String>,
    /// Username for an explicit authenticated proxy.
    #[arg(long, requires = "proxy")]
    pub(crate) proxy_username: Option<String>,
    /// Native secret-store name containing the explicit proxy password.
    #[arg(long, requires_all = ["proxy", "proxy_username"])]
    pub(crate) proxy_password_secret: Option<String>,
    /// Ignore proxy environment variables.
    #[arg(long, conflicts_with = "proxy")]
    pub(crate) no_proxy: bool,
    /// Print the approval URL without opening a browser.
    #[arg(long)]
    pub(crate) no_browser: bool,
}

#[derive(Debug, Subcommand)]
pub(crate) enum ProxySecretCommand {
    /// Prompt for and store an authenticated proxy password.
    Set { name: String },
    /// Delete a stored proxy password.
    Delete {
        name: String,
        /// Confirm deletion.
        #[arg(long)]
        yes: bool,
    },
}

#[derive(Debug, Subcommand)]
pub(crate) enum ServiceCommand {
    /// Install and start the current binary as the native user service.
    Install {
        /// Replace an existing managed definition.
        #[arg(long)]
        force: bool,
        /// Windows account used by the service; defaults to the current account.
        #[arg(long, value_name = "DOMAIN\\USER")]
        account: Option<String>,
    },
    /// Start an installed native service.
    Start,
    /// Stop the native service without deleting its definition.
    Stop,
    /// Show native service manager state.
    Status {
        /// Output as JSON for scripting.
        #[arg(long)]
        json: bool,
    },
    /// Stop and remove the native service definition.
    Uninstall {
        /// Confirm service removal.
        #[arg(long)]
        yes: bool,
    },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pair_defaults_to_read_only_and_environment_proxy() {
        let cli = Cli::try_parse_from([
            "captain-node",
            "pair",
            "--hub",
            "https://hub.example",
            "--workspace",
            ".",
        ])
        .unwrap();
        let Some(Command::Pair(pair)) = cli.command else {
            panic!("pair command expected");
        };
        assert!(!pair.allow_mutation);
        assert!(!pair.no_proxy);
        assert_eq!(pair.workspace_id, "workspace-main");
    }

    #[test]
    fn proxy_credentials_require_a_complete_explicit_proxy() {
        let error = Cli::try_parse_from([
            "captain-node",
            "pair",
            "--hub",
            "https://hub.example",
            "--workspace",
            ".",
            "--proxy-username",
            "operator",
        ])
        .unwrap_err();
        assert_eq!(
            error.kind(),
            clap::error::ErrorKind::MissingRequiredArgument
        );
    }

    #[test]
    fn passwords_can_never_be_supplied_on_the_command_line() {
        let proxy = Cli::try_parse_from([
            "captain-node",
            "proxy-secret",
            "set",
            "corp",
            "--password",
            "leaked",
        ]);
        let service =
            Cli::try_parse_from(["captain-node", "service", "install", "--password", "leaked"]);
        assert!(proxy.is_err());
        assert!(service.is_err());
    }

    #[test]
    fn version_is_valid_without_a_subcommand() {
        let cli = Cli::try_parse_from(["captain-node", "--version"]).unwrap();
        assert!(cli.version);
        assert!(cli.command.is_none());
    }
}
