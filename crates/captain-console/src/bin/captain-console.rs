use captain_console::{
    run_tui, ClientPairingProgress, ConsoleManager, ConsoleManagerError, ConsolePairingError,
    ConsolePairingOptions, ConsolePairingSession, ConsoleTuiError, PAIRING_POLL_INTERVAL,
};
use captain_node::{
    NativeNodeProxySecrets, NodeNetworkConfig, NodeProxyMode, NodeProxySecretError,
};
use clap::{Parser, Subcommand};
use std::{path::PathBuf, process::Stdio};

#[derive(Parser)]
#[command(
    name = "captain-console",
    disable_version_flag = true,
    about = "Lightweight Captain Console"
)]
struct Cli {
    /// Print the canonical Captain build version.
    #[arg(short = 'V', long, global = true)]
    version: bool,
    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Subcommand)]
enum Command {
    /// Pair this lightweight Console with a Captain Full authority.
    Pair {
        #[arg(long, value_name = "HTTPS_URL")]
        hub: String,
        /// Existing profile UUID, unique prefix, or local label.
        #[arg(long)]
        profile: Option<String>,
        /// Local label for this Captain authority.
        #[arg(long)]
        label: Option<String>,
        /// Device name shown to the Captain Full administrator.
        #[arg(long)]
        name: Option<String>,
        /// PEM bundle for an enterprise certificate authority.
        #[arg(long)]
        ca_bundle: Option<PathBuf>,
        /// Explicit HTTPS proxy URL. Environment proxy is used by default.
        #[arg(long, conflicts_with = "no_proxy")]
        proxy: Option<String>,
        #[arg(long, requires = "proxy")]
        proxy_username: Option<String>,
        /// Native secret name created by `captain-console proxy-secret set`.
        #[arg(long, requires = "proxy")]
        proxy_password_secret: Option<String>,
        /// Disable environment and explicit proxies.
        #[arg(long, conflicts_with = "proxy")]
        no_proxy: bool,
        /// Print the approval URL without opening the system browser.
        #[arg(long)]
        no_browser: bool,
    },
    /// Open the active Captain Web surface through a private loopback gateway.
    Open {
        /// Profile UUID, unique UUID prefix, or local label.
        #[arg(long)]
        profile: Option<String>,
        /// Print the one-time local URL instead of opening the system browser.
        #[arg(long)]
        no_browser: bool,
    },
    /// Open the lightweight terminal Console for chat and shared sessions.
    Tui {
        /// Profile UUID, unique UUID prefix, or local label.
        #[arg(long)]
        profile: Option<String>,
    },
    /// List configured Captain authorities without exposing their origins.
    List {
        #[arg(long)]
        json: bool,
        /// Read only the local non-secret inventory without probing authorities.
        #[arg(long)]
        local: bool,
    },
    /// Select the Captain used by future Console processes.
    Use { profile: String },
    /// Change a local Captain label without notifying the remote authority.
    Rename { profile: String, label: String },
    /// Manage passwords in the native Console/Node proxy secret store.
    ProxySecret {
        #[command(subcommand)]
        command: ProxySecretCommand,
    },
}

#[derive(Subcommand)]
enum ProxySecretCommand {
    /// Read a proxy password without echo and store it under a local name.
    Set { name: String },
    /// Delete a named proxy password after explicit confirmation.
    Delete {
        name: String,
        #[arg(long)]
        yes: bool,
    },
}

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "captain_console=info".into()),
        )
        .init();
    if let Err(error) = run(Cli::parse()).await {
        eprintln!("Error: {error}");
        std::process::exit(1);
    }
}

async fn run(cli: Cli) -> Result<(), ConsoleCliError> {
    if cli.version {
        println!(
            "captain-console {}",
            captain_types::version::captain_version()
        );
        return Ok(());
    }
    let command = cli.command.unwrap_or(Command::Open {
        profile: None,
        no_browser: false,
    });
    match command {
        Command::Pair {
            hub,
            profile,
            label,
            name,
            ca_bundle,
            proxy,
            proxy_username,
            proxy_password_secret,
            no_proxy,
            no_browser,
        } => {
            pair(PairArgs {
                hub,
                profile,
                label,
                name,
                ca_bundle,
                proxy,
                proxy_username,
                proxy_password_secret,
                no_proxy,
                no_browser,
            })
            .await
        }
        Command::Tui { profile } => run_tui(profile.as_deref()).await.map_err(Into::into),
        Command::ProxySecret { command } => manage_proxy_secret(command),
        command => run_manager(command).await,
    }
}

async fn run_manager(command: Command) -> Result<(), ConsoleCliError> {
    let mut manager = ConsoleManager::open_default()?;
    match command {
        Command::Pair { .. } => unreachable!("pair command handled before opening the manager"),
        Command::Tui { .. } => unreachable!("tui command handled before opening the manager"),
        Command::ProxySecret { .. } => {
            unreachable!("proxy secret command handled before opening the manager")
        }
        Command::List { json, local } => {
            let profiles = if local {
                manager.local_inventory()?
            } else {
                manager.live_inventory().await?
            };
            if json {
                println!(
                    "{}",
                    serde_json::to_string_pretty(&serde_json::json!({ "profiles": profiles }))
                        .map_err(|_| ConsoleManagerError::SerializationUnavailable)?
                );
            } else if profiles.is_empty() {
                println!("No Captain profile is configured.");
            } else {
                for profile in profiles {
                    let marker = if profile.profile.active {
                        "active"
                    } else {
                        "available"
                    };
                    let model = match (&profile.provider, &profile.model) {
                        (Some(provider), Some(model)) => format!("{provider}/{model}"),
                        _ => "model unavailable".to_string(),
                    };
                    let workload = match (profile.session_count, profile.active_project_count) {
                        (Some(sessions), Some(projects)) => {
                            format!("{sessions} sessions · {projects} projects")
                        }
                        _ => "workload unavailable".to_string(),
                    };
                    let version = profile.version.as_deref().unwrap_or("version unavailable");
                    let health = match (&profile.health, profile.alert_count) {
                        (Some(health), Some(alerts)) => {
                            format!("health {health} · {alerts} alerts")
                        }
                        _ => "health unavailable".to_string(),
                    };
                    let quotas = if profile.quotas.is_empty() {
                        "quota unavailable".to_string()
                    } else {
                        profile
                            .quotas
                            .iter()
                            .map(|quota| {
                                let remaining = quota
                                    .remaining_percent
                                    .map(|percent| format!("{percent:.0}% left"))
                                    .unwrap_or_else(|| "remaining unknown".to_string());
                                let reset = quota
                                    .resets_at
                                    .as_deref()
                                    .map(|at| format!(" · reset {at}"))
                                    .unwrap_or_default();
                                format!("{} {} {remaining}{reset}", quota.name, quota.window)
                            })
                            .collect::<Vec<_>>()
                            .join(" · ")
                    };
                    println!(
                        "{}  {}  {}  {}  {}  {}",
                        profile.profile.label,
                        marker,
                        profile.availability.as_str(),
                        version,
                        model,
                        profile.profile.id
                    );
                    println!("    {workload} · {health} · {quotas}");
                }
            }
        }
        Command::Use { profile } => {
            let selected = manager.activate(&profile)?;
            println!("Active Captain: {}", selected.label);
        }
        Command::Rename { profile, label } => {
            let renamed = manager.rename(&profile, &label)?;
            println!("Captain profile renamed to {}.", renamed.label);
        }
        Command::Open {
            profile,
            no_browser,
        } => {
            let mut launch = match profile {
                Some(profile) => manager.launch(&profile)?,
                None => manager.launch_active()?,
            };
            let bootstrap_url = launch.take_bootstrap_url()?;
            if no_browser || !open_in_browser(&bootstrap_url) {
                println!("Open once: {bootstrap_url}");
            }
            println!(
                "Captain Console is connected to {} on 127.0.0.1:{}.",
                launch.profile.label, launch.port
            );
            println!("Press Ctrl+C to close this local Console gateway.");
            let _ = tokio::signal::ctrl_c().await;
        }
    }
    Ok(())
}

fn manage_proxy_secret(command: ProxySecretCommand) -> Result<(), ConsoleCliError> {
    let store = NativeNodeProxySecrets::default();
    match command {
        ProxySecretCommand::Set { name } => {
            let password = zeroize::Zeroizing::new(
                rpassword::prompt_password("Proxy password: ")
                    .map_err(|_| ConsoleCliError::ProxySecretInputUnavailable)?,
            );
            store.set(&name, password.as_str())?;
            println!("Native proxy secret stored: {name}");
        }
        ProxySecretCommand::Delete { name, yes } => {
            store.delete(&name, yes)?;
            println!("Native proxy secret deleted: {name}");
        }
    }
    Ok(())
}

struct PairArgs {
    hub: String,
    profile: Option<String>,
    label: Option<String>,
    name: Option<String>,
    ca_bundle: Option<PathBuf>,
    proxy: Option<String>,
    proxy_username: Option<String>,
    proxy_password_secret: Option<String>,
    no_proxy: bool,
    no_browser: bool,
}

async fn pair(args: PairArgs) -> Result<(), ConsoleCliError> {
    let network = NodeNetworkConfig {
        hub_url: args.hub,
        proxy: proxy_mode(
            args.proxy,
            args.proxy_username,
            args.proxy_password_secret,
            args.no_proxy,
        )?,
        enterprise_ca_bundle: args.ca_bundle,
        ..NodeNetworkConfig::new("")
    };
    let options = ConsolePairingOptions {
        home: console_home()?,
        profile_selector: args.profile,
        label: args.label,
        client_name: args.name.unwrap_or_else(default_client_name),
        platform: format!("{}-{}", std::env::consts::OS, std::env::consts::ARCH),
        network,
        captain_version: env!("CARGO_PKG_VERSION").to_string(),
    };
    let (session, mut progress) = ConsolePairingSession::start(options).await?;
    let mut approval_shown = false;
    loop {
        match &progress {
            ClientPairingProgress::AwaitingApproval { display_code, .. } => {
                if !approval_shown {
                    let approval = session
                        .approval_url(&progress)?
                        .ok_or(ConsoleCliError::PairingStateUnavailable)?;
                    println!("Pairing code: {display_code}");
                    println!("Approve: {approval}");
                    if !args.no_browser {
                        let _ = open_in_browser(&approval);
                    }
                    approval_shown = true;
                }
            }
            ClientPairingProgress::Paired { device_id, .. } => {
                println!("Paired Captain: {}", session.profile().label);
                println!("Device: {device_id}");
                return Ok(());
            }
            ClientPairingProgress::Denied { .. } => return Err(ConsoleCliError::PairingDenied),
            ClientPairingProgress::Expired { .. } => return Err(ConsoleCliError::PairingExpired),
            ClientPairingProgress::ReadyToClaim => {
                return Err(ConsoleCliError::PairingStateUnavailable)
            }
        }
        tokio::select! {
            _ = tokio::signal::ctrl_c() => {
                println!("Pairing remains durable; rerun the same command to resume.");
                return Ok(());
            }
            _ = tokio::time::sleep(PAIRING_POLL_INTERVAL) => {}
        }
        progress = poll_until_available(&session).await?;
    }
}

async fn poll_until_available(
    session: &ConsolePairingSession,
) -> Result<ClientPairingProgress, ConsolePairingError> {
    loop {
        match session.poll().await {
            Ok(progress) => return Ok(progress),
            Err(error) if error.retry_delay().is_some() => {
                tokio::time::sleep(error.retry_delay().unwrap_or(PAIRING_POLL_INTERVAL)).await;
            }
            Err(error) => return Err(error),
        }
    }
}

fn proxy_mode(
    proxy: Option<String>,
    username: Option<String>,
    password_secret: Option<String>,
    no_proxy: bool,
) -> Result<NodeProxyMode, ConsoleCliError> {
    if no_proxy {
        if username.is_some() || password_secret.is_some() {
            return Err(ConsoleCliError::InvalidProxyConfiguration);
        }
        return Ok(NodeProxyMode::Disabled);
    }
    match proxy {
        Some(url) if username.is_some() == password_secret.is_some() => {
            Ok(NodeProxyMode::Explicit {
                url,
                username,
                password_secret,
            })
        }
        Some(_) => Err(ConsoleCliError::InvalidProxyConfiguration),
        None if username.is_none() && password_secret.is_none() => Ok(NodeProxyMode::Environment),
        None => Err(ConsoleCliError::InvalidProxyConfiguration),
    }
}

fn console_home() -> Result<PathBuf, ConsoleCliError> {
    std::env::var_os("CAPTAIN_HOME")
        .map(PathBuf::from)
        .or_else(|| dirs::home_dir().map(|home| home.join(".captain")))
        .ok_or(ConsoleCliError::HomeUnavailable)
}

fn default_client_name() -> String {
    ["COMPUTERNAME", "HOSTNAME"]
        .into_iter()
        .find_map(|key| std::env::var(key).ok())
        .filter(|value| !value.trim().is_empty() && !value.chars().any(char::is_control))
        .map(|value| format!("{value} Console"))
        .unwrap_or_else(|| "Captain Console".to_string())
}

#[derive(Debug, thiserror::Error)]
enum ConsoleCliError {
    #[error(transparent)]
    Manager(#[from] ConsoleManagerError),
    #[error(transparent)]
    Pairing(#[from] ConsolePairingError),
    #[error(transparent)]
    Tui(#[from] ConsoleTuiError),
    #[error(transparent)]
    ProxySecret(#[from] NodeProxySecretError),
    #[error("the proxy configuration is incomplete")]
    InvalidProxyConfiguration,
    #[error("the proxy password could not be read from this terminal")]
    ProxySecretInputUnavailable,
    #[error("the Captain Console home directory is unavailable")]
    HomeUnavailable,
    #[error("the Hub denied this Console pairing request")]
    PairingDenied,
    #[error("the Console pairing request expired")]
    PairingExpired,
    #[error("the Console pairing state did not advance safely")]
    PairingStateUnavailable,
}

fn open_in_browser(url: &str) -> bool {
    #[cfg(target_os = "windows")]
    {
        // guarded-exec-audit: fixed-command (OS browser launcher, no agent shell)
        std::process::Command::new("cmd")
            .args(["/C", "start", "", url])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .is_ok()
    }
    #[cfg(target_os = "macos")]
    {
        // guarded-exec-audit: fixed-command (OS browser launcher, no agent shell)
        std::process::Command::new("open")
            .arg(url)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .is_ok()
    }
    #[cfg(target_os = "linux")]
    {
        // guarded-exec-audit: fixed-command (OS browser launcher, no agent shell)
        std::process::Command::new("xdg-open")
            .arg(url)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .is_ok()
    }
}
