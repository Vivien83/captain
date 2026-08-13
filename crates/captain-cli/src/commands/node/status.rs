use super::support::{node_root, proxy_name, safe_error, NODE_STATE_DIR};
use crate::ui;
use captain_node::{
    NodeLocalConfig, NodeLocalConfigStore, NodePairingError, NodePairingProgress, NodePairingStore,
    NodeRailSnapshot, NodeRailStore, NodeRuntimeStatus, NodeRuntimeStatusStore,
};
use serde_json::json;

pub(super) fn node_status(json_output: bool) -> Result<(), String> {
    let node_root = node_root();
    if !node_root.join("config.toml").exists() {
        return render_status(
            json_output,
            json!({
                "configured": false,
                "state": "unconfigured",
                "runtime_active": false,
            }),
        );
    }
    let config_store = NodeLocalConfigStore::open(&node_root).map_err(safe_error)?;
    let config = config_store
        .load()
        .map_err(safe_error)?
        .ok_or_else(|| "The local Node configuration is unavailable".to_string())?;
    let state_root = config_store.root().join(NODE_STATE_DIR);
    let (state, device_id, rail, runtime_active, effective_allow_mutation, runtime) =
        if !state_root.exists() {
            ("unpaired".to_string(), None, None, false, None, None)
        } else {
            match NodePairingStore::open(&state_root) {
                Err(NodePairingError::NodeAlreadyRunning) => {
                    let runtime = NodeRuntimeStatusStore::open(config_store.root())
                        .and_then(|store| store.load())
                        .map_err(safe_error)?;
                    (
                        runtime
                            .as_ref()
                            .map(|status| status.state().to_string())
                            .unwrap_or_else(|| "running".to_string()),
                        runtime
                            .as_ref()
                            .and_then(|status| status.device_id().map(ToString::to_string)),
                        runtime
                            .as_ref()
                            .and_then(|status| status.rail_snapshot().cloned()),
                        true,
                        runtime.as_ref().and_then(NodeRuntimeStatus::allow_mutation),
                        runtime,
                    )
                }
                Err(error) => return Err(safe_error(error)),
                Ok(pairing_store) => {
                    let progress = pairing_store.status().map_err(safe_error)?;
                    let state = pairing_state_name(progress.as_ref()).to_string();
                    let device_id = paired_device_id(progress.as_ref());
                    let (rail, effective_allow_mutation) =
                        if matches!(progress, Some(NodePairingProgress::Paired { .. })) {
                            let grants = pairing_store.approved_grants().map_err(safe_error)?;
                            (
                                Some(
                                    NodeRailStore::open(&pairing_store)
                                        .map_err(safe_error)?
                                        .snapshot()
                                        .map_err(safe_error)?,
                                ),
                                Some(grants.allow_mutation),
                            )
                        } else {
                            (None, None)
                        };
                    (
                        state,
                        device_id,
                        rail,
                        false,
                        effective_allow_mutation,
                        None,
                    )
                }
            }
        };
    render_status(
        json_output,
        status_payload(
            &config,
            &state,
            device_id.as_deref(),
            rail.as_ref(),
            runtime_active,
            effective_allow_mutation,
            runtime.as_ref(),
        ),
    )
}

pub(super) fn reset_node(confirmed: bool) -> Result<(), String> {
    if !confirmed {
        return Err("Node reset requires explicit confirmation with `--yes`".to_string());
    }
    let state_root = node_root().join(NODE_STATE_DIR);
    if !state_root.exists() {
        ui::success("No local Node credential state exists.");
        return Ok(());
    }
    NodePairingStore::open(state_root)
        .map_err(safe_error)?
        .reset()
        .map_err(safe_error)?;
    ui::success("Local Node credentials and durable rail state were reset.");
    Ok(())
}

fn status_payload(
    config: &NodeLocalConfig,
    state: &str,
    device_id: Option<&str>,
    rail: Option<&NodeRailSnapshot>,
    runtime_active: bool,
    effective_allow_mutation: Option<bool>,
    runtime: Option<&NodeRuntimeStatus>,
) -> serde_json::Value {
    json!({
        "configured": true,
        "state": state,
        "runtime_active": runtime_active,
        "device_id": device_id,
        "display_name": config.display_name,
        "platform": config.platform,
        "requested_authority": authority_name(config.allow_mutation),
        "effective_authority": effective_allow_mutation.map(authority_name),
        "network": {
            "proxy": proxy_name(&config.network.proxy),
            "enterprise_ca_configured": config.network.enterprise_ca_bundle.is_some(),
        },
        "workspaces": config.workspaces.iter().map(|workspace| json!({
            "workspace_id": workspace.workspace_id,
            "label": workspace.label,
            "read_only": workspace.read_only,
        })).collect::<Vec<_>>(),
        "rail": rail.map(|snapshot| json!({
            "pending_outbound": snapshot.pending_outbound,
            "pending_inbound": snapshot.pending_inbound,
            "last_node_sequence": snapshot.last_node_sequence,
            "acknowledged_node_sequence": snapshot.acknowledged_node_sequence,
            "last_hub_sequence": snapshot.last_hub_sequence,
            "confirmed_hub_ack_sequence": snapshot.confirmed_hub_ack_sequence,
        })),
        "runtime": runtime.map(|status| json!({
            "updated_at_ms": status.updated_at_ms(),
            "transport": status.transport(),
            "capability_state": status.capability_state(),
            "fallback_count": status.fallback_count(),
            "last_error_code": status.last_error_code(),
        })),
    })
}

fn authority_name(allow_mutation: bool) -> &'static str {
    if allow_mutation {
        "read_write"
    } else {
        "read_only"
    }
}

fn render_status(json_output: bool, payload: serde_json::Value) -> Result<(), String> {
    if json_output {
        println!(
            "{}",
            serde_json::to_string_pretty(&payload)
                .map_err(|_| "The Node status could not be serialized".to_string())?
        );
        return Ok(());
    }
    ui::section("Local Node");
    ui::kv("State", payload["state"].as_str().unwrap_or("unknown"));
    if let Some(name) = payload["display_name"].as_str() {
        ui::kv("Name", name);
    }
    if let Some(authority) = payload["requested_authority"].as_str() {
        ui::kv("Requested authority", authority);
    }
    if let Some(authority) = payload["effective_authority"].as_str() {
        ui::kv("Effective authority", authority);
    }
    if let Some(workspaces) = payload["workspaces"].as_array() {
        ui::kv("Workspaces", &workspaces.len().to_string());
    }
    if let Some(rail) = payload["rail"].as_object() {
        ui::kv(
            "Pending",
            &format!(
                "{} out / {} in",
                rail["pending_outbound"].as_u64().unwrap_or(0),
                rail["pending_inbound"].as_u64().unwrap_or(0)
            ),
        );
    }
    if let Some(runtime) = payload["runtime"].as_object() {
        if let Some(transport) = runtime["transport"].as_str() {
            ui::kv("Transport", transport);
        }
        if let Some(error) = runtime["last_error_code"].as_str() {
            ui::kv("Last runtime issue", error);
        }
    }
    Ok(())
}

fn pairing_state_name(progress: Option<&NodePairingProgress>) -> &'static str {
    match progress {
        None => "unpaired",
        Some(NodePairingProgress::ReadyToClaim) => "ready_to_pair",
        Some(NodePairingProgress::AwaitingApproval { .. }) => "awaiting_approval",
        Some(NodePairingProgress::Paired { .. }) => "paired",
        Some(NodePairingProgress::Denied { .. }) => "denied",
        Some(NodePairingProgress::Expired { .. }) => "expired",
    }
}

fn paired_device_id(progress: Option<&NodePairingProgress>) -> Option<String> {
    match progress {
        Some(NodePairingProgress::Paired { device_id, .. }) => Some(device_id.clone()),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use captain_node::{NodeBootstrapCapabilityState, NodeLocalWorkspace, NodeNetworkConfig};
    use captain_wire::NodeTransport;

    #[test]
    fn status_payload_never_contains_hub_or_workspace_paths() {
        let temp = tempfile::tempdir().unwrap();
        let config = NodeLocalConfig::new(
            "Office Node",
            "test-platform",
            NodeNetworkConfig::new("https://private-hub.example"),
            vec![NodeLocalWorkspace {
                workspace_id: "project-main".to_string(),
                label: "Main".to_string(),
                root: temp.path().to_path_buf(),
                read_only: true,
            }],
            false,
        )
        .unwrap();
        let rendered = status_payload(
            &config,
            "paired",
            Some("device-1"),
            None,
            false,
            Some(false),
            None,
        )
        .to_string();
        assert!(!rendered.contains("private-hub"));
        assert!(!rendered.contains(temp.path().to_string_lossy().as_ref()));
        assert!(rendered.contains("project-main"));
        assert!(rendered.contains("effective_authority"));
        assert!(!rendered.contains("read_write_requested"));
    }

    #[test]
    fn active_status_distinguishes_requested_effective_and_degraded_runtime_state() {
        let temp = tempfile::tempdir().unwrap();
        let config = NodeLocalConfig::new(
            "Office Node",
            "test-platform",
            NodeNetworkConfig::new("https://hub.example"),
            vec![NodeLocalWorkspace {
                workspace_id: "project-main".to_string(),
                label: "Main".to_string(),
                root: temp.path().to_path_buf(),
                read_only: false,
            }],
            true,
        )
        .unwrap();
        let rail = NodeRailSnapshot {
            device_id: "device-1".to_string(),
            connection_id: "connection-1".to_string(),
            last_node_sequence: 1,
            acknowledged_node_sequence: 1,
            last_hub_sequence: 2,
            confirmed_hub_ack_sequence: 2,
            pending_outbound: 0,
            pending_inbound: 0,
        };
        let runtime = NodeRuntimeStatus::connected(
            123,
            NodeTransport::LongPoll,
            NodeBootstrapCapabilityState::Current,
            false,
            rail.clone(),
            2,
            Some("transport_retry"),
        )
        .unwrap();
        let payload = status_payload(
            &config,
            "degraded",
            Some("device-1"),
            Some(&rail),
            true,
            runtime.allow_mutation(),
            Some(&runtime),
        );
        assert_eq!(payload["requested_authority"], "read_write");
        assert_eq!(payload["effective_authority"], "read_only");
        assert_eq!(payload["runtime"]["transport"], "long_poll");
        assert_eq!(payload["runtime"]["last_error_code"], "transport_retry");
        assert!(!payload.to_string().contains("hub.example"));
        assert!(!payload
            .to_string()
            .contains(temp.path().to_string_lossy().as_ref()));
    }
}
