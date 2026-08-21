//! Operational CLI for one lightweight Captain Client.

use super::node::support::{
    pairing_retry_delay, proxy_mode, resolve_proxy_password, safe_error, PAIRING_POLL_INTERVAL,
};
use crate::{
    captain_version, cli_captain_home, client_profiles, open_in_browser, ui, ClientCommands,
};
use captain_node::{
    ClientLocalConfig, ClientLocalConfigStore, ClientPairingClient, ClientPairingProgress,
    ClientPairingStore, ClientProfileEntry, ClientProfileRegistry, NodeNetworkConfig,
};
use serde_json::json;
use std::{future::Future, path::PathBuf};

pub(crate) const CLIENT_STATE_DIR: &str = "state";

pub(crate) fn cmd_client(command: ClientCommands) {
    let result = match command {
        ClientCommands::Pair(args) => block_on(pair_client(PairRequest {
            hub: args.hub,
            profile: args.profile,
            label: args.label,
            name: args.name,
            ca_bundle: args.ca_bundle,
            proxy: args.proxy,
            proxy_username: args.proxy_username,
            proxy_password_secret: args.proxy_password_secret,
            no_proxy: args.no_proxy,
            no_browser: args.no_browser,
        })),
        ClientCommands::List { json } => client_list(json),
        ClientCommands::Use { profile } => use_client_profile(&profile),
        ClientCommands::Status { profile, json } => client_status(profile.as_deref(), json),
        ClientCommands::Reset { yes } => reset_client(yes),
    };
    if let Err(error) = result {
        ui::error(&error);
        std::process::exit(1);
    }
}

struct PairRequest {
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

async fn pair_client(request: PairRequest) -> Result<(), String> {
    let home = cli_captain_home();
    let requested_label = request
        .label
        .as_deref()
        .map(validated_profile_label)
        .transpose()?;
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
    let registry = client_profiles::open_registry().map_err(safe_error)?;
    let profile = match request.profile.as_deref() {
        Some(selector) => resolve_profile(&registry, selector)?,
        None => matching_hub_profile(&registry, &config.network.hub_url)?.unwrap_or(
            registry
                .create_profile(current_time_ms()?)
                .map_err(safe_error)?,
        ),
    };
    let profile_root = registry.profile_root(&profile.id).map_err(safe_error)?;
    let store = ClientLocalConfigStore::open(profile_root).map_err(safe_error)?;
    if let Some(existing) = store.load().map_err(safe_error)? {
        if normalized_hub(&existing.network.hub_url) != normalized_hub(&config.network.hub_url) {
            return Err(
                "The selected Client profile belongs to a different Captain; reset it explicitly or create a new profile"
                    .to_string(),
            );
        }
    }
    let label = requested_label
        .or_else(|| profile.label.clone())
        .unwrap_or_else(|| default_profile_label(&profile.id));
    registry
        .set_label(&profile.id, &label)
        .map_err(safe_error)?;
    store.save(&config).map_err(safe_error)?;
    registry.set_active(&profile.id).map_err(safe_error)?;
    let state_root = store.root().join(CLIENT_STATE_DIR);
    let mut pairing_store =
        ClientPairingStore::open(&state_root, &profile.id).map_err(safe_error)?;
    if matches!(
        pairing_store.status().map_err(safe_error)?,
        Some(ClientPairingProgress::Denied { .. } | ClientPairingProgress::Expired { .. })
    ) {
        pairing_store.reset().map_err(safe_error)?;
        pairing_store = ClientPairingStore::open(&state_root, &profile.id).map_err(safe_error)?;
    }
    let pairing = ClientPairingClient::new(http, pairing_store);
    let progress = pairing
        .start_or_resume(&config.pairing_profile(&captain_version()))
        .await
        .map_err(safe_error)?;
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

fn client_list(json_output: bool) -> Result<(), String> {
    let registry = client_profiles::open_registry().map_err(safe_error)?;
    let profiles = registry
        .list()
        .map_err(safe_error)?
        .into_iter()
        .map(|profile| profile_payload(&registry, &profile))
        .collect::<Result<Vec<_>, _>>()?;
    if json_output {
        println!(
            "{}",
            serde_json::to_string_pretty(&json!({ "profiles": profiles }))
                .map_err(|_| "The Client profile list could not be serialized".to_string())?
        );
        return Ok(());
    }
    ui::section("Captain profiles");
    if profiles.is_empty() {
        ui::hint("No Captain is paired. Run `captain client pair --hub https://...`.");
        return Ok(());
    }
    for profile in profiles {
        let marker = if profile["active"].as_bool() == Some(true) {
            "active"
        } else {
            "available"
        };
        let name = profile["display_name"].as_str().unwrap_or("Unconfigured");
        let id = profile["id"].as_str().unwrap_or("unknown");
        let state = profile["state"].as_str().unwrap_or("unknown");
        ui::kv(name, &format!("{marker} · {state} · {id}"));
    }
    Ok(())
}

fn use_client_profile(selector: &str) -> Result<(), String> {
    let registry = client_profiles::open_registry().map_err(safe_error)?;
    let profile = resolve_profile(&registry, selector)?;
    if profile_config(&registry, &profile)?.is_none() {
        return Err(
            "That Client profile is not configured; pair it before selecting it".to_string(),
        );
    }
    registry.set_active(&profile.id).map_err(safe_error)?;
    let name = profile_display_name(&profile);
    ui::success(&format!("Future Client sessions will use {name}."));
    Ok(())
}

fn client_status(selector: Option<&str>, json_output: bool) -> Result<(), String> {
    if !client_profiles::client_state_present() {
        return render_status(
            json_output,
            json!({"configured": false, "state": "unconfigured"}),
        );
    }
    let registry = client_profiles::open_registry().map_err(safe_error)?;
    let profile = match selector {
        Some(selector) => resolve_profile(&registry, selector)?,
        None => registry
            .active_profile()
            .map_err(safe_error)?
            .ok_or_else(|| "No active Client profile is selected".to_string())?,
    };
    render_status(json_output, profile_payload(&registry, &profile)?)
}

fn reset_client(confirmed: bool) -> Result<(), String> {
    if !confirmed {
        return Err("Client reset requires explicit confirmation with `--yes`".to_string());
    }
    let registry = client_profiles::open_registry().map_err(safe_error)?;
    let profile = registry
        .active_profile()
        .map_err(safe_error)?
        .ok_or_else(|| "No active Client profile is selected".to_string())?;
    let config_store =
        ClientLocalConfigStore::open(registry.profile_root(&profile.id).map_err(safe_error)?)
            .map_err(safe_error)?;
    let state_root = config_store.root().join(CLIENT_STATE_DIR);
    if state_root.exists() {
        ClientPairingStore::open(&state_root, &profile.id)
            .map_err(safe_error)?
            .reset()
            .map_err(safe_error)?;
    }
    config_store.remove_config().map_err(safe_error)?;
    registry.clear_active(&profile.id).map_err(safe_error)?;
    ui::success("Local Client identity and Hub configuration were reset.");
    Ok(())
}

fn status_payload(
    profile: &ClientProfileEntry,
    config: &ClientLocalConfig,
    state: &str,
    device_id: Option<&str>,
) -> serde_json::Value {
    json!({
        "id": profile.id,
        "active": profile.active,
        "configured": true,
        "state": state,
        "device_id": device_id,
        "display_name": profile_display_name(profile),
        "platform": config.platform,
        "network": {
            "proxy": super::node::support::proxy_name(&config.network.proxy),
            "enterprise_ca_configured": config.network.enterprise_ca_bundle.is_some(),
        },
        "execution_capable": false,
    })
}

fn profile_payload(
    registry: &ClientProfileRegistry,
    profile: &ClientProfileEntry,
) -> Result<serde_json::Value, String> {
    let Some(config) = profile_config(registry, profile)? else {
        return Ok(json!({
            "id": profile.id,
            "active": profile.active,
            "configured": false,
            "state": "unconfigured",
            "display_name": profile_display_name(profile),
            "execution_capable": false,
        }));
    };
    let root = registry.profile_root(&profile.id).map_err(safe_error)?;
    let state_root = root.join(CLIENT_STATE_DIR);
    let (state, device_id) = if state_root.exists() {
        let store = ClientPairingStore::open(state_root, &profile.id).map_err(safe_error)?;
        let progress = store.status().map_err(safe_error)?;
        (
            pairing_state_name(progress.as_ref()).to_string(),
            paired_device_id(progress.as_ref()),
        )
    } else {
        ("unpaired".to_string(), None)
    };
    Ok(status_payload(
        profile,
        &config,
        &state,
        device_id.as_deref(),
    ))
}

fn profile_config(
    registry: &ClientProfileRegistry,
    profile: &ClientProfileEntry,
) -> Result<Option<ClientLocalConfig>, String> {
    let root = registry.profile_root(&profile.id).map_err(safe_error)?;
    ClientLocalConfigStore::open(root)
        .map_err(safe_error)?
        .load()
        .map_err(safe_error)
}

fn resolve_profile(
    registry: &ClientProfileRegistry,
    selector: &str,
) -> Result<ClientProfileEntry, String> {
    let selector = selector.trim();
    if selector.is_empty() || selector.chars().any(char::is_control) {
        return Err("The Client profile selector is invalid".to_string());
    }
    let mut matches = Vec::new();
    for profile in registry.list().map_err(safe_error)? {
        let id_match = profile.id == selector
            || (selector.len() >= 8 && profile.id.starts_with(&selector.to_ascii_lowercase()));
        let name_match = profile_display_name(&profile).eq_ignore_ascii_case(selector);
        if id_match || name_match {
            matches.push(profile);
        }
    }
    match matches.len() {
        1 => Ok(matches.remove(0)),
        0 => Err("No Client profile matches that selector".to_string()),
        _ => Err("The Client profile selector is ambiguous; use the full UUID".to_string()),
    }
}

fn matching_hub_profile(
    registry: &ClientProfileRegistry,
    hub_url: &str,
) -> Result<Option<ClientProfileEntry>, String> {
    let mut matches = Vec::new();
    for profile in registry.list().map_err(safe_error)? {
        if profile_config(registry, &profile)?.is_some_and(|config| {
            normalized_hub(&config.network.hub_url) == normalized_hub(hub_url)
        }) {
            matches.push(profile);
        }
    }
    match matches.len() {
        0 => Ok(None),
        1 => Ok(matches.pop()),
        _ => Err(
            "Several local Client profiles target the same Captain; select one explicitly"
                .to_string(),
        ),
    }
}

fn normalized_hub(hub_url: &str) -> &str {
    hub_url.trim_end_matches('/')
}

fn profile_display_name(profile: &ClientProfileEntry) -> String {
    profile
        .label
        .clone()
        .unwrap_or_else(|| default_profile_label(&profile.id))
}

fn default_profile_label(profile_id: &str) -> String {
    format!("Captain {}", profile_id.get(..8).unwrap_or("unknown"))
}

fn validated_profile_label(label: &str) -> Result<String, String> {
    let label = label.trim();
    if label.is_empty() || label.len() > 80 || label.chars().any(char::is_control) {
        return Err("The local Captain profile label is invalid".to_string());
    }
    Ok(label.to_string())
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

fn current_time_ms() -> Result<i64, String> {
    let elapsed = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_err(|_| "The system clock is unavailable".to_string())?;
    i64::try_from(elapsed.as_millis()).map_err(|_| "The system clock is unavailable".to_string())
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

    fn configured_profile(
        registry: &ClientProfileRegistry,
        created_at_ms: i64,
        label: &str,
        hub_url: &str,
    ) -> ClientProfileEntry {
        let profile = registry.create_profile(created_at_ms).unwrap();
        let root = registry.profile_root(&profile.id).unwrap();
        let config = ClientLocalConfig::new(
            "Test Client",
            "test-platform",
            NodeNetworkConfig::new(hub_url),
        )
        .unwrap();
        ClientLocalConfigStore::open(root)
            .unwrap()
            .save(&config)
            .unwrap();
        registry.set_label(&profile.id, label).unwrap();
        registry
            .list()
            .unwrap()
            .into_iter()
            .find(|entry| entry.id == profile.id)
            .unwrap()
    }

    #[test]
    fn status_never_exposes_the_hub_origin_or_execution_authority() {
        let config = ClientLocalConfig::new(
            "Office Client",
            "test-platform",
            NodeNetworkConfig::new("https://private-hub.example"),
        )
        .unwrap();
        let profile = ClientProfileEntry {
            id: "f72d3f5f-c980-4cef-a083-0494ea9efb90".to_string(),
            created_at_ms: 1,
            active: true,
            label: Some("Production Captain".to_string()),
        };
        let rendered = status_payload(&profile, &config, "paired", Some("client-1")).to_string();
        assert!(!rendered.contains("private-hub"));
        assert!(!rendered.contains("Office Client"));
        assert!(rendered.contains("Production Captain"));
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

    #[test]
    fn profile_selection_is_explicit_and_origin_bound() {
        let dir = tempfile::tempdir().unwrap();
        let registry = ClientProfileRegistry::open(dir.path().join("console")).unwrap();
        let office = configured_profile(&registry, 10, "Office Captain", "https://office.example");
        let personal = configured_profile(
            &registry,
            20,
            "Personal Captain",
            "https://personal.example",
        );

        assert_eq!(
            resolve_profile(&registry, "office captain").unwrap().id,
            office.id
        );
        assert_eq!(
            resolve_profile(&registry, &personal.id[..8]).unwrap().id,
            personal.id
        );
        assert_eq!(
            matching_hub_profile(&registry, "https://office.example/")
                .unwrap()
                .unwrap()
                .id,
            office.id
        );
        assert!(matching_hub_profile(&registry, "https://other.example")
            .unwrap()
            .is_none());
    }

    #[test]
    fn ambiguous_display_names_never_select_an_authority() {
        let dir = tempfile::tempdir().unwrap();
        let registry = ClientProfileRegistry::open(dir.path().join("console")).unwrap();
        configured_profile(&registry, 10, "Captain", "https://one.example");
        configured_profile(&registry, 20, "Captain", "https://two.example");

        let error = resolve_profile(&registry, "captain").unwrap_err();
        assert!(error.contains("ambiguous"));
        assert!(!error.contains("one.example"));
        assert!(!error.contains("two.example"));
    }
}
