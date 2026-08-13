//! Operational CLI for one lightweight Captain Client.

use super::node::support::{
    pairing_retry_delay, proxy_mode, resolve_proxy_password, safe_error, PAIRING_POLL_INTERVAL,
};
use crate::{captain_version, cli_captain_home, open_in_browser, ui, ClientCommands};
use captain_node::{
    ClientLocalConfig, ClientLocalConfigStore, ClientPairingClient, ClientPairingProgress,
    ClientPairingStore, NodeNetworkConfig,
};
use serde_json::json;
use std::{future::Future, path::PathBuf};

pub(crate) const CLIENT_STATE_DIR: &str = "state";

pub(crate) fn cmd_client(command: ClientCommands) {
    let result = match command {
        ClientCommands::Pair(args) => block_on(pair_client(PairRequest {
            hub: args.hub,
            name: args.name,
            ca_bundle: args.ca_bundle,
            proxy: args.proxy,
            proxy_username: args.proxy_username,
            proxy_password_secret: args.proxy_password_secret,
            no_proxy: args.no_proxy,
            no_browser: args.no_browser,
        })),
        ClientCommands::Status { json } => client_status(json),
        ClientCommands::Reset { yes } => reset_client(yes),
    };
    if let Err(error) = result {
        ui::error(&error);
        std::process::exit(1);
    }
}

struct PairRequest {
    hub: String,
    name: Option<String>,
    ca_bundle: Option<PathBuf>,
    proxy: Option<String>,
    proxy_username: Option<String>,
    proxy_password_secret: Option<String>,
    no_proxy: bool,
    no_browser: bool,
}

async fn pair_client(request: PairRequest) -> Result<(), String> {
    let home = cli_captain_home();
    let store = ClientLocalConfigStore::open(client_root()).map_err(safe_error)?;
    let network = NodeNetworkConfig {
        hub_url: request.hub,
        proxy: proxy_mode(
            request.proxy,
            request.proxy_username,
            request.proxy_password_secret,
            request.no_proxy,
        )?,
        enterprise_ca_bundle: request.ca_bundle,
        ..NodeNetworkConfig::new("")
    };
    let proxy_password = resolve_proxy_password(&network, &home)?;
    let http = network
        .build_client(proxy_password.as_ref())
        .map_err(safe_error)?;
    let config = ClientLocalConfig::new(
        request.name.unwrap_or_else(default_client_name),
        local_platform(),
        network,
    )
    .map_err(safe_error)?;
    let state_root = store.root().join(CLIENT_STATE_DIR);
    let mut pairing_store = ClientPairingStore::open(&state_root).map_err(safe_error)?;
    if matches!(
        pairing_store.status().map_err(safe_error)?,
        Some(ClientPairingProgress::Denied { .. } | ClientPairingProgress::Expired { .. })
    ) {
        pairing_store.reset().map_err(safe_error)?;
        pairing_store = ClientPairingStore::open(&state_root).map_err(safe_error)?;
    }
    let pairing = ClientPairingClient::new(http, pairing_store);
    let progress = pairing
        .start_or_resume(&config.pairing_profile(&captain_version()))
        .await
        .map_err(safe_error)?;
    store.save(&config).map_err(safe_error)?;
    wait_for_pairing(&pairing, &config, progress, request.no_browser).await
}

async fn wait_for_pairing(
    pairing: &ClientPairingClient,
    config: &ClientLocalConfig,
    mut progress: ClientPairingProgress,
    no_browser: bool,
) -> Result<(), String> {
    let mut approval_shown = false;
    loop {
        match progress {
            ClientPairingProgress::AwaitingApproval {
                ref display_code,
                ref approval_path,
                ..
            } => {
                if !approval_shown {
                    let approval = approval_url(&config.network.hub_url, approval_path)?;
                    ui::section("Client pairing");
                    ui::kv("Code", display_code);
                    ui::kv("Approve", &approval);
                    if !no_browser && !open_in_browser(&approval) {
                        ui::hint("Open the approval URL from a browser signed into the Hub.");
                    }
                    approval_shown = true;
                }
            }
            ClientPairingProgress::Paired { ref device_id, .. } => {
                ui::success("This interface is paired as a lightweight Captain Client.");
                ui::kv("Device", device_id);
                ui::next_steps(&["Run `captain chat` or `captain tui` on this machine."]);
                return Ok(());
            }
            ClientPairingProgress::Denied { .. } => {
                return Err("The Hub denied this Client pairing request".to_string());
            }
            ClientPairingProgress::Expired { .. } => {
                return Err(
                    "The Client pairing request expired; run the pair command again".to_string(),
                );
            }
            ClientPairingProgress::ReadyToClaim => {
                return Err("The local Client pairing state did not advance safely".to_string());
            }
        }

        tokio::select! {
            _ = tokio::signal::ctrl_c() => {
                ui::hint("Pairing remains durable; rerun the same command to resume.");
                return Ok(());
            }
            _ = tokio::time::sleep(PAIRING_POLL_INTERVAL) => {}
        }
        progress = poll_until_available(pairing).await?;
    }
}

async fn poll_until_available(
    pairing: &ClientPairingClient,
) -> Result<ClientPairingProgress, String> {
    loop {
        match pairing.poll().await {
            Ok(progress) => return Ok(progress),
            Err(error) if pairing_retry_delay(&error).is_some() => {
                let delay = pairing_retry_delay(&error).unwrap_or(PAIRING_POLL_INTERVAL);
                tracing::warn!(error_class = %error, "Client pairing poll will retry");
                tokio::time::sleep(delay).await;
            }
            Err(error) => return Err(safe_error(error)),
        }
    }
}

fn client_status(json_output: bool) -> Result<(), String> {
    let root = client_root();
    if !root.join("config.toml").exists() {
        return render_status(
            json_output,
            json!({"configured": false, "state": "unconfigured"}),
        );
    }
    let config_store = ClientLocalConfigStore::open(&root).map_err(safe_error)?;
    let config = config_store
        .load()
        .map_err(safe_error)?
        .ok_or_else(|| "The local Client configuration is unavailable".to_string())?;
    let state_root = config_store.root().join(CLIENT_STATE_DIR);
    let (state, device_id) = if state_root.exists() {
        let store = ClientPairingStore::open(state_root).map_err(safe_error)?;
        let progress = store.status().map_err(safe_error)?;
        (
            pairing_state_name(progress.as_ref()).to_string(),
            paired_device_id(progress.as_ref()),
        )
    } else {
        ("unpaired".to_string(), None)
    };
    render_status(
        json_output,
        status_payload(&config, &state, device_id.as_deref()),
    )
}

fn reset_client(confirmed: bool) -> Result<(), String> {
    if !confirmed {
        return Err("Client reset requires explicit confirmation with `--yes`".to_string());
    }
    let config_store = ClientLocalConfigStore::open(client_root()).map_err(safe_error)?;
    let state_root = config_store.root().join(CLIENT_STATE_DIR);
    if state_root.exists() {
        ClientPairingStore::open(&state_root)
            .map_err(safe_error)?
            .reset()
            .map_err(safe_error)?;
    }
    config_store.remove_config().map_err(safe_error)?;
    ui::success("Local Client identity and Hub configuration were reset.");
    Ok(())
}

fn status_payload(
    config: &ClientLocalConfig,
    state: &str,
    device_id: Option<&str>,
) -> serde_json::Value {
    json!({
        "configured": true,
        "state": state,
        "device_id": device_id,
        "display_name": config.display_name,
        "platform": config.platform,
        "network": {
            "proxy": super::node::support::proxy_name(&config.network.proxy),
            "enterprise_ca_configured": config.network.enterprise_ca_bundle.is_some(),
        },
        "execution_capable": false,
    })
}

fn render_status(json_output: bool, payload: serde_json::Value) -> Result<(), String> {
    if json_output {
        println!(
            "{}",
            serde_json::to_string_pretty(&payload)
                .map_err(|_| "The Client status could not be serialized".to_string())?
        );
        return Ok(());
    }
    ui::section("Lightweight Client");
    ui::kv("State", payload["state"].as_str().unwrap_or("unknown"));
    if let Some(name) = payload["display_name"].as_str() {
        ui::kv("Name", name);
    }
    ui::kv("Local execution", "disabled");
    Ok(())
}

fn pairing_state_name(progress: Option<&ClientPairingProgress>) -> &'static str {
    match progress {
        None => "unpaired",
        Some(ClientPairingProgress::ReadyToClaim) => "ready_to_pair",
        Some(ClientPairingProgress::AwaitingApproval { .. }) => "awaiting_approval",
        Some(ClientPairingProgress::Paired { .. }) => "paired",
        Some(ClientPairingProgress::Denied { .. }) => "denied",
        Some(ClientPairingProgress::Expired { .. }) => "expired",
    }
}

fn paired_device_id(progress: Option<&ClientPairingProgress>) -> Option<String> {
    match progress {
        Some(ClientPairingProgress::Paired { device_id, .. }) => Some(device_id.clone()),
        _ => None,
    }
}

fn approval_url(hub_url: &str, approval_path: &str) -> Result<String, String> {
    if !approval_path.starts_with('/') || approval_path.starts_with("//") {
        return Err("The Hub returned an invalid Client approval path".to_string());
    }
    let mut origin =
        url::Url::parse(hub_url).map_err(|_| "The configured Hub URL is invalid".to_string())?;
    origin.set_path("/");
    origin.set_query(None);
    origin.set_fragment(None);
    let approval = origin
        .join(approval_path.trim_start_matches('/'))
        .map_err(|_| "The Hub returned an invalid Client approval path".to_string())?;
    if approval.origin() != origin.origin() {
        return Err("The Hub returned an invalid Client approval origin".to_string());
    }
    Ok(approval.to_string())
}

pub(crate) fn client_root() -> PathBuf {
    cli_captain_home().join("client")
}

fn default_client_name() -> String {
    ["COMPUTERNAME", "HOSTNAME"]
        .into_iter()
        .find_map(|key| std::env::var(key).ok())
        .filter(|value| !value.trim().is_empty() && !value.chars().any(char::is_control))
        .map(|value| format!("{value} Client"))
        .unwrap_or_else(|| "Captain Client".to_string())
}

fn local_platform() -> String {
    format!("{}-{}", std::env::consts::OS, std::env::consts::ARCH)
}

fn block_on<F>(future: F) -> Result<(), String>
where
    F: Future<Output = Result<(), String>>,
{
    tokio::runtime::Runtime::new()
        .map_err(|_| "The lightweight Client async runtime could not start".to_string())?
        .block_on(future)
}

#[cfg(test)]
mod tests {
    use super::*;
    use captain_node::NodeProxyMode;

    #[test]
    fn status_never_exposes_the_hub_origin_or_execution_authority() {
        let config = ClientLocalConfig::new(
            "Office Client",
            "test-platform",
            NodeNetworkConfig::new("https://private-hub.example"),
        )
        .unwrap();
        let rendered = status_payload(&config, "paired", Some("client-1")).to_string();
        assert!(!rendered.contains("private-hub"));
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(&rendered).unwrap()["execution_capable"],
            false
        );
    }

    #[test]
    fn approval_origin_and_proxy_options_fail_closed() {
        assert_eq!(
            approval_url("https://hub.example.com", "/devices/pair?code=1234").unwrap(),
            "https://hub.example.com/devices/pair?code=1234"
        );
        assert!(approval_url("https://hub.example.com", "//evil.example/pair").is_err());
        assert!(matches!(
            proxy_mode(None, None, None, false).unwrap(),
            NodeProxyMode::Environment
        ));
    }

    #[test]
    fn pairing_retry_only_accepts_transient_failures() {
        assert!(
            pairing_retry_delay(&captain_node::ClientPairingError::NetworkUnavailable).is_some()
        );
        assert!(
            pairing_retry_delay(&captain_node::ClientPairingError::InvalidDeviceCredential)
                .is_none()
        );
    }
}
