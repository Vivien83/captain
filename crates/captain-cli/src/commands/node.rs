//! Operational CLI adapter for the lightweight outbound-only Captain Node.

pub(crate) mod support;

use crate::{captain_version, cli_captain_home, open_in_browser, ui, NodeCommands};
use captain_node::node_shutdown_channel;
use captain_node::operator::{
    node_status, pair_node, reset_node, run_node, NodeEventSink, NodeOperatorEvent, NodePairRequest,
};
use std::{future::Future, path::PathBuf};
use support::{load_kernel_config, CliNodeProxyResolver};

pub(crate) fn cmd_node(config_path: Option<PathBuf>, command: NodeCommands) {
    let home = cli_captain_home();
    let result = match command {
        NodeCommands::Pair(args) => {
            let resolver = CliNodeProxyResolver::new(home.clone());
            let events = CliNodeEvents {
                no_browser: args.no_browser,
            };
            block_on(pair_node(
                NodePairRequest {
                    home,
                    captain_version: captain_version(),
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
                &resolver,
                &events,
            ))
        }
        NodeCommands::Run => load_kernel_config(config_path, &home).and_then(|config| {
            let resolver = CliNodeProxyResolver::new(home.clone());
            let events = CliNodeEvents { no_browser: true };
            let (shutdown_handle, shutdown) = node_shutdown_channel();
            block_on(async {
                let signal = tokio::spawn(async move {
                    if tokio::signal::ctrl_c().await.is_ok() {
                        shutdown_handle.cancel();
                    }
                });
                let result = run_node(
                    &home,
                    &captain_version(),
                    config.exec_policy,
                    &resolver,
                    &events,
                    shutdown,
                )
                .await;
                signal.abort();
                result
            })
        }),
        NodeCommands::Status { json } => {
            node_status(&home).and_then(|payload| render_status(json, &payload))
        }
        NodeCommands::Reset { yes } => {
            let had_state = home.join("node").join("state").exists();
            reset_node(&home, yes).map(|()| {
                if had_state {
                    ui::success("Local Node credentials and durable rail state were reset.");
                } else {
                    ui::success("No local Node credential state exists.");
                }
            })
        }
    };

    if let Err(error) = result {
        ui::error(&error);
        std::process::exit(1);
    }
}

struct CliNodeEvents {
    no_browser: bool,
}

impl NodeEventSink for CliNodeEvents {
    fn emit(&self, event: NodeOperatorEvent) {
        match event {
            NodeOperatorEvent::Pairing {
                display_code,
                approval_url,
            } => {
                ui::section("Node pairing");
                ui::kv("Code", &display_code);
                ui::kv("Approve", &approval_url);
                if !self.no_browser && !open_in_browser(&approval_url) {
                    ui::hint("Open the approval URL from a browser signed into the Hub.");
                }
            }
            NodeOperatorEvent::PairingResumable => {
                ui::hint("Pairing remains durable; rerun the same command to resume.");
            }
            NodeOperatorEvent::Paired { device_id } => {
                ui::success("This machine is paired as an outbound-only Captain Node.");
                ui::kv("Device", &device_id);
                ui::next_steps(&["Run `captain node run` on this machine."]);
            }
            NodeOperatorEvent::Connected {
                transport,
                allow_mutation,
            } => {
                ui::success("Local Node connected to the Captain Hub.");
                ui::kv("Transport", &transport);
                ui::kv(
                    "Authority",
                    if allow_mutation {
                        "approved read/write"
                    } else {
                        "read-only"
                    },
                );
                ui::hint("Press Ctrl+C to stop the local Node worker.");
            }
            NodeOperatorEvent::Stopped => ui::success("Local Node stopped."),
        }
    }
}

fn render_status(json_output: bool, payload: &serde_json::Value) -> Result<(), String> {
    if json_output {
        println!(
            "{}",
            serde_json::to_string_pretty(payload)
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

fn block_on<F>(future: F) -> Result<(), String>
where
    F: Future<Output = Result<(), String>>,
{
    tokio::runtime::Runtime::new()
        .map_err(|_| "The local Node async runtime could not start".to_string())?
        .block_on(future)
}
