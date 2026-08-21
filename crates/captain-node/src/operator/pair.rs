use super::support::{
    node_root, pairing_retry_delay, proxy_mode, safe_error, NodeEventSink, NodeOperatorEvent,
    NodeProxyPasswordResolver, NODE_STATE_DIR, PAIRING_POLL_INTERVAL,
};
use crate::{
    NodeLocalConfig, NodeLocalConfigStore, NodeLocalWorkspace, NodeNetworkConfig,
    NodePairingClient, NodePairingProgress, NodePairingStore,
};
use std::path::PathBuf;

pub struct NodePairRequest {
    pub home: PathBuf,
    pub captain_version: String,
    pub hub: String,
    pub workspace: PathBuf,
    pub workspace_id: String,
    pub name: Option<String>,
    pub label: Option<String>,
    pub allow_mutation: bool,
    pub ca_bundle: Option<PathBuf>,
    pub proxy: Option<String>,
    pub proxy_username: Option<String>,
    pub proxy_password_secret: Option<String>,
    pub no_proxy: bool,
}

pub async fn pair_node(
    request: NodePairRequest,
    secrets: &dyn NodeProxyPasswordResolver,
    events: &dyn NodeEventSink,
) -> Result<(), String> {
    let store = NodeLocalConfigStore::open(node_root(&request.home)).map_err(safe_error)?;
    let workspace = std::fs::canonicalize(&request.workspace)
        .map_err(|_| "The selected local Node workspace is unavailable".to_string())?;
    if !workspace.is_dir() {
        return Err("The selected local Node workspace is not a directory".to_string());
    }

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
    let proxy_password = secrets.resolve(&network)?;
    let http = network
        .build_client(proxy_password.as_ref())
        .map_err(safe_error)?;
    let config = NodeLocalConfig::new(
        request.name.unwrap_or_else(default_node_name),
        local_platform(),
        network,
        vec![NodeLocalWorkspace {
            workspace_id: request.workspace_id,
            label: request
                .label
                .unwrap_or_else(|| default_workspace_label(&workspace)),
            root: workspace,
            read_only: !request.allow_mutation,
        }],
        request.allow_mutation,
    )
    .map_err(safe_error)?;

    let state_root = store.root().join(NODE_STATE_DIR);
    let mut pairing_store = NodePairingStore::open(&state_root).map_err(safe_error)?;
    if matches!(
        pairing_store.status().map_err(safe_error)?,
        Some(NodePairingProgress::Denied { .. } | NodePairingProgress::Expired { .. })
    ) {
        pairing_store.reset().map_err(safe_error)?;
        pairing_store = NodePairingStore::open(&state_root).map_err(safe_error)?;
    }
    if matches!(
        pairing_store.status().map_err(safe_error)?,
        Some(NodePairingProgress::Paired { .. })
    ) {
        config
            .execution_policy(pairing_store.approved_grants().map_err(safe_error)?)
            .map_err(safe_error)?;
    }

    let pairing = NodePairingClient::new(http, pairing_store);
    let progress = pairing
        .start_or_resume(&config.pairing_profile(&request.captain_version))
        .await
        .map_err(safe_error)?;
    store.save(&config).map_err(safe_error)?;
    wait_for_pairing(&pairing, &config, progress, events).await
}

async fn wait_for_pairing(
    pairing: &NodePairingClient,
    config: &NodeLocalConfig,
    mut progress: NodePairingProgress,
    events: &dyn NodeEventSink,
) -> Result<(), String> {
    let mut approval_shown = false;
    loop {
        match progress {
            NodePairingProgress::AwaitingApproval {
                ref display_code,
                ref approval_path,
                ..
            } => {
                if !approval_shown {
                    let approval = approval_url(&config.network.hub_url, approval_path)?;
                    events.emit(NodeOperatorEvent::Pairing {
                        display_code: display_code.clone(),
                        approval_url: approval,
                    });
                    approval_shown = true;
                }
            }
            NodePairingProgress::Paired { ref device_id, .. } => {
                events.emit(NodeOperatorEvent::Paired {
                    device_id: device_id.clone(),
                });
                return Ok(());
            }
            NodePairingProgress::Denied { .. } => {
                return Err("The Hub denied this Node pairing request".to_string());
            }
            NodePairingProgress::Expired { .. } => {
                return Err(
                    "The Node pairing request expired; run the pair command again".to_string(),
                );
            }
            NodePairingProgress::ReadyToClaim => {
                return Err("The local Node pairing state did not advance safely".to_string());
            }
        }

        tokio::select! {
            _ = tokio::signal::ctrl_c() => {
                events.emit(NodeOperatorEvent::PairingResumable);
                return Ok(());
            }
            _ = tokio::time::sleep(PAIRING_POLL_INTERVAL) => {}
        }
        let Some(next) = poll_until_available(pairing, events).await? else {
            return Ok(());
        };
        progress = next;
    }
}

async fn poll_until_available(
    pairing: &NodePairingClient,
    events: &dyn NodeEventSink,
) -> Result<Option<NodePairingProgress>, String> {
    loop {
        match pairing.poll().await {
            Ok(progress) => return Ok(Some(progress)),
            Err(error) if pairing_retry_delay(&error).is_some() => {
                let delay = pairing_retry_delay(&error).unwrap_or(PAIRING_POLL_INTERVAL);
                tracing::warn!(error_class = %error, "Node pairing poll will retry");
                tokio::select! {
                    _ = tokio::signal::ctrl_c() => {
                        events.emit(NodeOperatorEvent::PairingResumable);
                        return Ok(None);
                    }
                    _ = tokio::time::sleep(delay) => {}
                }
            }
            Err(error) => return Err(safe_error(error)),
        }
    }
}

fn approval_url(hub_url: &str, approval_path: &str) -> Result<String, String> {
    if !approval_path.starts_with('/') || approval_path.starts_with("//") {
        return Err("The Hub returned an invalid Node approval path".to_string());
    }
    let mut origin =
        url::Url::parse(hub_url).map_err(|_| "The configured Hub URL is invalid".to_string())?;
    origin.set_path("/");
    origin.set_query(None);
    origin.set_fragment(None);
    let approval = origin
        .join(approval_path.trim_start_matches('/'))
        .map_err(|_| "The Hub returned an invalid Node approval path".to_string())?;
    if approval.origin() != origin.origin() {
        return Err("The Hub returned an invalid Node approval origin".to_string());
    }
    Ok(approval.to_string())
}

fn default_node_name() -> String {
    ["COMPUTERNAME", "HOSTNAME"]
        .into_iter()
        .find_map(|key| std::env::var(key).ok())
        .filter(|value| !value.trim().is_empty() && !value.chars().any(char::is_control))
        .unwrap_or_else(|| "Captain Node".to_string())
}

fn default_workspace_label(workspace: &std::path::Path) -> String {
    workspace
        .file_name()
        .and_then(|name| name.to_str())
        .filter(|name| !name.is_empty() && !name.chars().any(char::is_control))
        .unwrap_or("Workspace")
        .to_string()
}

fn local_platform() -> String {
    format!("{}-{}", std::env::consts::OS, std::env::consts::ARCH)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::NodeProxyMode;

    #[test]
    fn approval_url_never_changes_the_configured_hub_origin() {
        assert_eq!(
            approval_url("https://hub.example.com", "/devices/pair?code=1234").unwrap(),
            "https://hub.example.com/devices/pair?code=1234"
        );
        assert!(approval_url("https://hub.example.com", "//evil.example/pair").is_err());
        assert!(approval_url("https://hub.example.com", "https://evil.example/pair").is_err());
    }

    #[test]
    fn proxy_configuration_is_explicit_and_fail_closed() {
        assert!(matches!(
            proxy_mode(None, None, None, false).unwrap(),
            NodeProxyMode::Environment
        ));
        assert!(matches!(
            proxy_mode(None, None, None, true).unwrap(),
            NodeProxyMode::Disabled
        ));
        assert!(proxy_mode(
            Some("https://proxy.example".to_string()),
            Some("operator".to_string()),
            None,
            false,
        )
        .is_err());
    }
}
