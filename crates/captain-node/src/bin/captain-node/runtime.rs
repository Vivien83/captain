use crate::cli::{Cli, Command, PairArgs, ProxySecretCommand, ServiceCommand};
use crate::render::{render_node_status, render_service_status, TerminalEvents};
use captain_node::operator::{node_status, pair_node, reset_node, run_node, NodePairRequest};
use captain_node::{
    node_shutdown_channel, NativeNodeProxySecrets, NativeNodeServiceController, NodeShutdown,
};
use captain_types::config::{ExecPolicy, ExecutionProfile};
use std::path::{Path, PathBuf};
use zeroize::Zeroizing;

pub(crate) async fn execute(cli: Cli) -> Result<(), String> {
    if cli.version {
        println!("captain-node {}", captain_types::version::captain_version());
        return Ok(());
    }
    let command = cli
        .command
        .ok_or_else(|| "A captain-node command is required".to_string())?;
    let home = resolve_home(cli.home)?;
    match command {
        Command::Pair(args) => pair(&home, args).await,
        Command::Run => run_foreground(require_home(&home)?).await,
        Command::Status { json } => render_node_status(json, &node_status(&home)?),
        Command::Reset { yes } => {
            let had_state = home.join("node").join("state").exists();
            reset_node(&home, yes)?;
            if had_state {
                println!("Local Node credentials and durable rail state were reset.");
            } else {
                println!("No local Node credential state exists.");
            }
            Ok(())
        }
        Command::ProxySecret { command } => proxy_secret(command),
        Command::Service { command } => service(&home, command),
        Command::ServiceRuntime => service_runtime(home).await,
    }
}

async fn pair(home: &Path, args: PairArgs) -> Result<(), String> {
    let home = prepare_home(home)?;
    let secrets = NativeNodeProxySecrets::default();
    let events = TerminalEvents::interactive(args.no_browser);
    pair_node(
        NodePairRequest {
            home,
            captain_version: captain_types::version::captain_version(),
            hub: args.hub,
            workspace: args.workspace,
            workspace_id: args.workspace_id,
            name: args.name,
            label: args.label,
            allow_mutation: args.allow_mutation,
            ca_bundle: args.ca_bundle,
            proxy: args.proxy,
            proxy_username: args.proxy_username,
            proxy_password_secret: args.proxy_password_secret,
            no_proxy: args.no_proxy,
        },
        &secrets,
        &events,
    )
    .await
}

async fn run_foreground(home: PathBuf) -> Result<(), String> {
    let (shutdown_handle, shutdown) = node_shutdown_channel();
    let signal_task = tokio::spawn(async move {
        wait_for_shutdown_signal().await;
        shutdown_handle.cancel();
    });
    let result = run_node_runtime(&home, shutdown, TerminalEvents::interactive(true)).await;
    signal_task.abort();
    let _ = signal_task.await;
    result
}

pub(crate) async fn run_node_service(home: &Path, shutdown: NodeShutdown) -> Result<(), String> {
    run_node_runtime(home, shutdown, TerminalEvents::service()).await
}

async fn run_node_runtime(
    home: &Path,
    shutdown: NodeShutdown,
    events: TerminalEvents,
) -> Result<(), String> {
    let secrets = NativeNodeProxySecrets::default();
    run_node(
        home,
        &captain_types::version::captain_version(),
        node_exec_policy(),
        &secrets,
        &events,
        shutdown,
    )
    .await
}

fn node_exec_policy() -> ExecPolicy {
    ExecPolicy {
        profile: ExecutionProfile::RemoteOperator,
        ..ExecPolicy::default()
    }
}

async fn service_runtime(home: PathBuf) -> Result<(), String> {
    let home = require_home(&home)?;
    #[cfg(target_os = "windows")]
    {
        return crate::windows_service::dispatch(home);
    }
    #[cfg(not(target_os = "windows"))]
    {
        let (shutdown_handle, shutdown) = node_shutdown_channel();
        let signal_task = tokio::spawn(async move {
            wait_for_shutdown_signal().await;
            shutdown_handle.cancel();
        });
        let result = run_node_service(&home, shutdown).await;
        signal_task.abort();
        let _ = signal_task.await;
        result
    }
}

async fn wait_for_shutdown_signal() {
    #[cfg(unix)]
    {
        use tokio::signal::unix::{signal, SignalKind};
        let sigint = signal(SignalKind::interrupt());
        let sigterm = signal(SignalKind::terminate());
        match (sigint, sigterm) {
            (Ok(mut sigint), Ok(mut sigterm)) => {
                tokio::select! {
                    _ = sigint.recv() => {}
                    _ = sigterm.recv() => {}
                }
            }
            _ => {
                let _ = tokio::signal::ctrl_c().await;
            }
        }
    }
    #[cfg(not(unix))]
    {
        let _ = tokio::signal::ctrl_c().await;
    }
}

fn proxy_secret(command: ProxySecretCommand) -> Result<(), String> {
    let secrets = NativeNodeProxySecrets::default();
    match command {
        ProxySecretCommand::Set { name } => {
            let password = Zeroizing::new(
                rpassword::prompt_password("Proxy password: ")
                    .map_err(|_| "The proxy password could not be read securely".to_string())?,
            );
            let confirmation = Zeroizing::new(
                rpassword::prompt_password("Confirm proxy password: ")
                    .map_err(|_| "The proxy password could not be read securely".to_string())?,
            );
            if password.as_bytes() != confirmation.as_bytes() {
                return Err("The proxy password confirmation does not match".to_string());
            }
            secrets.set(&name, password.as_str()).map_err(safe_error)?;
            println!("Proxy secret `{name}` stored in the native secret manager.");
        }
        ProxySecretCommand::Delete { name, yes } => {
            secrets.delete(&name, yes).map_err(safe_error)?;
            println!("Proxy secret `{name}` deleted from the native secret manager.");
        }
    }
    Ok(())
}

fn service(home: &Path, command: ServiceCommand) -> Result<(), String> {
    let install = matches!(command, ServiceCommand::Install { .. });
    let home = if install {
        prepare_home(home)?
    } else {
        require_home(home)?
    };
    let binary = std::env::current_exe()
        .and_then(std::fs::canonicalize)
        .map_err(|_| "The current captain-node binary path is unavailable".to_string())?;
    let controller = NativeNodeServiceController::detect(binary, home).map_err(safe_error)?;
    match command {
        ServiceCommand::Install { force, account } => {
            let status = install_service(&controller, force, account)?;
            render_service_status(false, &status)
        }
        ServiceCommand::Start => {
            render_service_status(false, &controller.start().map_err(safe_error)?)
        }
        ServiceCommand::Stop => {
            render_service_status(false, &controller.stop().map_err(safe_error)?)
        }
        ServiceCommand::Status { json } => {
            render_service_status(json, &controller.status().map_err(safe_error)?)
        }
        ServiceCommand::Uninstall { yes } => {
            render_service_status(false, &controller.uninstall(yes).map_err(safe_error)?)
        }
    }
}

#[cfg(not(target_os = "windows"))]
fn install_service(
    controller: &NativeNodeServiceController,
    force: bool,
    account: Option<String>,
) -> Result<captain_node::NodeNativeServiceStatus, String> {
    if account.is_some() {
        return Err("--account is only valid for a Windows service".to_string());
    }
    controller.install(force).map_err(safe_error)
}

#[cfg(target_os = "windows")]
fn install_service(
    controller: &NativeNodeServiceController,
    force: bool,
    account: Option<String>,
) -> Result<captain_node::NodeNativeServiceStatus, String> {
    let account = account.unwrap_or_else(default_windows_account);
    println!("Windows service account: {account}");
    let password = Zeroizing::new(
        rpassword::prompt_password(format!("Password for {account}: "))
            .map_err(|_| "The Windows account password could not be read securely".to_string())?,
    );
    controller
        .install_windows_user(force, &account, password.as_str())
        .map_err(safe_error)
}

#[cfg(target_os = "windows")]
fn default_windows_account() -> String {
    let username = std::env::var("USERNAME").unwrap_or_default();
    let domain = std::env::var("USERDOMAIN").unwrap_or_else(|_| ".".to_string());
    if username.trim().is_empty() {
        String::new()
    } else {
        format!(r"{domain}\{username}")
    }
}

fn resolve_home(explicit: Option<PathBuf>) -> Result<PathBuf, String> {
    let candidate = explicit
        .or_else(|| std::env::var_os("CAPTAIN_HOME").map(PathBuf::from))
        .or_else(|| dirs::home_dir().map(|home| home.join(".captain")))
        .ok_or_else(|| "Captain home is unavailable; pass --home explicitly".to_string())?;
    let absolute = if candidate.is_absolute() {
        candidate
    } else {
        std::env::current_dir()
            .map_err(|_| "The current directory is unavailable".to_string())?
            .join(candidate)
    };
    if absolute
        .to_str()
        .is_none_or(|value| value.is_empty() || value.chars().any(char::is_control))
    {
        return Err("Captain home path is invalid".to_string());
    }
    Ok(absolute)
}

fn prepare_home(home: &Path) -> Result<PathBuf, String> {
    captain_types::durable_fs::create_dir_all(home)
        .map_err(|_| "Captain home could not be created durably".to_string())?;
    require_home(home)
}

fn require_home(home: &Path) -> Result<PathBuf, String> {
    let metadata = std::fs::symlink_metadata(home)
        .map_err(|_| "Captain home is unavailable; pair this Node first".to_string())?;
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err("Captain home must be a private real directory".to_string());
    }
    restrict_home_permissions(home)?;
    std::fs::canonicalize(home).map_err(|_| "Captain home path is unavailable".to_string())
}

#[cfg(unix)]
fn restrict_home_permissions(home: &Path) -> Result<(), String> {
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(home, std::fs::Permissions::from_mode(0o700))
        .map_err(|_| "Captain home permissions could not be restricted".to_string())
}

#[cfg(not(unix))]
fn restrict_home_permissions(_home: &Path) -> Result<(), String> {
    Ok(())
}

fn safe_error(error: impl std::fmt::Display) -> String {
    error.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn home_resolution_does_not_create_state() {
        let temp = tempfile::tempdir().unwrap();
        let home = temp.path().join("absent");
        let resolved = resolve_home(Some(home.clone())).unwrap();
        assert_eq!(resolved, home);
        assert!(!resolved.exists());
    }

    #[test]
    fn prepared_home_is_real_and_private() {
        let temp = tempfile::tempdir().unwrap();
        let home = prepare_home(&temp.path().join("captain-home")).unwrap();
        assert!(home.is_dir());
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                std::fs::metadata(&home).unwrap().permissions().mode() & 0o777,
                0o700
            );
        }
    }

    #[cfg(unix)]
    #[test]
    fn symlink_home_is_rejected() {
        use std::os::unix::fs::symlink;
        let temp = tempfile::tempdir().unwrap();
        let real = temp.path().join("real");
        std::fs::create_dir(&real).unwrap();
        let linked = temp.path().join("linked");
        symlink(&real, &linked).unwrap();
        assert!(require_home(&linked).is_err());
    }

    #[test]
    fn node_runtime_always_uses_remote_operator_policy() {
        let policy = node_exec_policy();
        assert_eq!(policy.profile, ExecutionProfile::RemoteOperator);
        assert_eq!(
            policy.mode,
            captain_types::config::ExecSecurityMode::Allowlist
        );
    }
}
