use captain_node::operator::{NodeEventSink, NodeOperatorEvent};
use captain_node::{NodeNativeServiceState, NodeNativeServiceStatus};
use std::process::{Command, Stdio};

#[derive(Clone, Copy)]
pub(crate) enum EventMode {
    Interactive { no_browser: bool },
    Service,
}

pub(crate) struct TerminalEvents {
    mode: EventMode,
}

impl TerminalEvents {
    pub(crate) fn interactive(no_browser: bool) -> Self {
        Self {
            mode: EventMode::Interactive { no_browser },
        }
    }

    pub(crate) fn service() -> Self {
        Self {
            mode: EventMode::Service,
        }
    }
}

impl NodeEventSink for TerminalEvents {
    fn emit(&self, event: NodeOperatorEvent) {
        if matches!(self.mode, EventMode::Service) {
            emit_service_event(event);
            return;
        }
        match event {
            NodeOperatorEvent::Pairing {
                display_code,
                approval_url,
            } => {
                println!("Node pairing");
                println!("  Code:    {display_code}");
                println!("  Approve: {approval_url}");
                let EventMode::Interactive { no_browser } = self.mode else {
                    return;
                };
                if !no_browser && !open_in_browser(&approval_url) {
                    println!("  Open the approval URL in a browser signed into Captain Full.");
                }
            }
            NodeOperatorEvent::PairingResumable => {
                println!("Pairing remains durable; rerun the same command to resume.");
            }
            NodeOperatorEvent::Paired { device_id } => {
                println!("Paired as an outbound-only Captain Node.");
                println!("  Device: {device_id}");
                println!("  Next:   captain-node run");
            }
            NodeOperatorEvent::Connected {
                transport,
                allow_mutation,
            } => {
                println!("Captain Node connected.");
                println!("  Transport: {transport}");
                println!(
                    "  Authority: {}",
                    if allow_mutation {
                        "approved read/write"
                    } else {
                        "read-only"
                    }
                );
                println!("Press Ctrl+C to stop.");
            }
            NodeOperatorEvent::Stopped => println!("Captain Node stopped."),
        }
    }
}

fn emit_service_event(event: NodeOperatorEvent) {
    match event {
        NodeOperatorEvent::Connected {
            transport,
            allow_mutation,
        } => tracing::info!(transport, allow_mutation, "Captain Node service connected"),
        NodeOperatorEvent::Stopped => tracing::info!("Captain Node service stopped"),
        NodeOperatorEvent::Pairing { .. }
        | NodeOperatorEvent::PairingResumable
        | NodeOperatorEvent::Paired { .. } => {
            tracing::warn!("Captain Node service requires completed interactive pairing")
        }
    }
}

pub(crate) fn render_node_status(
    json_output: bool,
    payload: &serde_json::Value,
) -> Result<(), String> {
    if json_output {
        println!(
            "{}",
            serde_json::to_string_pretty(payload)
                .map_err(|_| "The Node status could not be serialized".to_string())?
        );
        return Ok(());
    }
    println!("Captain Node");
    println!(
        "  State:               {}",
        payload["state"].as_str().unwrap_or("unknown")
    );
    if let Some(name) = payload["display_name"].as_str() {
        println!("  Name:                {name}");
    }
    if let Some(authority) = payload["requested_authority"].as_str() {
        println!("  Requested authority: {authority}");
    }
    if let Some(authority) = payload["effective_authority"].as_str() {
        println!("  Effective authority: {authority}");
    }
    if let Some(workspaces) = payload["workspaces"].as_array() {
        println!("  Workspaces:           {}", workspaces.len());
    }
    if let Some(rail) = payload["rail"].as_object() {
        println!(
            "  Pending rail:         {} out / {} in",
            rail["pending_outbound"].as_u64().unwrap_or(0),
            rail["pending_inbound"].as_u64().unwrap_or(0)
        );
    }
    if let Some(runtime) = payload["runtime"].as_object() {
        if let Some(transport) = runtime["transport"].as_str() {
            println!("  Transport:            {transport}");
        }
        if let Some(error) = runtime["last_error_code"].as_str() {
            println!("  Last runtime issue:   {error}");
        }
    }
    Ok(())
}

pub(crate) fn render_service_status(
    json_output: bool,
    status: &NodeNativeServiceStatus,
) -> Result<(), String> {
    if json_output {
        println!(
            "{}",
            serde_json::to_string_pretty(status)
                .map_err(|_| "The service status could not be serialized".to_string())?
        );
        return Ok(());
    }
    println!("Captain Node service");
    println!("  Manager: {}", status.manager);
    println!("  State:   {}", service_state_name(status.state));
    Ok(())
}

fn service_state_name(state: NodeNativeServiceState) -> &'static str {
    match state {
        NodeNativeServiceState::NotInstalled => "not installed",
        NodeNativeServiceState::Stopped => "stopped",
        NodeNativeServiceState::Running => "running",
    }
}

fn open_in_browser(url: &str) -> bool {
    #[cfg(target_os = "macos")]
    let mut command = Command::new("open");
    #[cfg(target_os = "linux")]
    let mut command = Command::new("xdg-open");
    #[cfg(target_os = "windows")]
    let mut command = {
        let mut command = Command::new("rundll32.exe");
        command.arg("url.dll,FileProtocolHandler");
        command
    };
    #[cfg(not(any(target_os = "macos", target_os = "linux", target_os = "windows")))]
    return false;

    command
        .arg(url)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .is_ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn service_state_labels_are_stable() {
        assert_eq!(
            service_state_name(NodeNativeServiceState::NotInstalled),
            "not installed"
        );
        assert_eq!(
            service_state_name(NodeNativeServiceState::Stopped),
            "stopped"
        );
        assert_eq!(
            service_state_name(NodeNativeServiceState::Running),
            "running"
        );
    }
}
