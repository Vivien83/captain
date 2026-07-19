//! Background I/O for the native CapSpec TUI operator surface.

use super::event::{AppEvent, BackendRef};
use super::screens::native_capabilities::{
    NativeCapabilityInfo, NativeRevision, NativeRunDecision, NativeRunInfo, NativeRunNode,
    NativeScope,
};
use captain_runtime::kernel_handle::CapSpecForgeScope;
use serde_json::{json, Value};
use std::path::Path;
use std::sync::mpsc;
use std::time::Duration;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum NativeMutation {
    Decide {
        name: String,
        scope: String,
        expected_hash: String,
        approve: bool,
    },
    Rollback {
        name: String,
        scope: String,
        target_hash: String,
    },
    Disable {
        name: String,
        scope: String,
    },
    ResolveRun {
        run_id: String,
        node_id: String,
        tool_use_id: String,
        attempt: u32,
        decision: NativeRunDecision,
    },
}

pub fn spawn_fetch(
    backend: BackendRef,
    scope: NativeScope,
    workspace: Option<String>,
    tx: mpsc::Sender<AppEvent>,
) {
    std::thread::spawn(move || {
        let result = match backend {
            BackendRef::Daemon(base_url) => fetch_daemon(&base_url, scope, workspace.as_deref()),
            BackendRef::InProcess(kernel) => {
                let workspace_path = workspace.as_deref().map(Path::new);
                let capabilities = kernel
                    .capspec_management_list(forge_scope(scope), workspace_path)
                    .map_err(|error| format!("Native capabilities: {error}"));
                let runs = kernel
                    .capspec_management_runs(25)
                    .map_err(|error| format!("Native runs: {error}"));
                capabilities.and_then(|body| {
                    runs.map(|run_body| (parse_capabilities(&body), parse_runs(&run_body)))
                })
            }
        };
        match result {
            Ok((capabilities, runs)) => {
                let _ = tx.send(AppEvent::NativeCapabilitiesLoaded { capabilities, runs });
            }
            Err(error) => {
                let _ = tx.send(AppEvent::FetchError(error));
            }
        }
    });
}

pub fn spawn_inspect(
    backend: BackendRef,
    name: String,
    scope: String,
    workspace: Option<String>,
    include_source: bool,
    tx: mpsc::Sender<AppEvent>,
) {
    std::thread::spawn(move || {
        let result = match backend {
            BackendRef::Daemon(base_url) => {
                let client = daemon_client();
                let mut request = client
                    .get(format!("{base_url}/api/capabilities/native/{name}"))
                    .query(&[("scope", scope.as_str())])
                    .query(&[("include_source", include_source)]);
                if let Some(workspace) = workspace.as_deref() {
                    request = request.query(&[("workspace", workspace)]);
                }
                response_json(request.send(), "inspect native capability")
            }
            BackendRef::InProcess(kernel) => mutation_scope(&scope).and_then(|scope| {
                kernel
                    .capspec_management_inspect(
                        &name,
                        scope,
                        workspace.as_deref().map(Path::new),
                        include_source,
                    )
                    .map_err(|error| format!("Inspect native capability: {error}"))
            }),
        }
        .map(|body| parse_capability(&body));
        match result {
            Ok(capability) => {
                let _ = tx.send(AppEvent::NativeCapabilityInspected(capability));
            }
            Err(error) => {
                let _ = tx.send(AppEvent::FetchError(error));
            }
        }
    });
}

pub fn spawn_mutation(
    backend: BackendRef,
    mutation: NativeMutation,
    workspace: Option<String>,
    tx: mpsc::Sender<AppEvent>,
) {
    std::thread::spawn(move || {
        let label = mutation_label(&mutation);
        let result = match backend {
            BackendRef::Daemon(base_url) => {
                mutate_daemon(&base_url, &mutation, workspace.as_deref())
            }
            BackendRef::InProcess(kernel) => {
                mutate_in_process(kernel, &mutation, workspace.as_deref().map(Path::new))
            }
        };
        match result {
            Ok(_) => {
                let _ = tx.send(AppEvent::NativeCapabilityChanged(label));
            }
            Err(error) => {
                let _ = tx.send(AppEvent::FetchError(error));
            }
        }
    });
}

fn fetch_daemon(
    base_url: &str,
    scope: NativeScope,
    workspace: Option<&str>,
) -> Result<(Vec<NativeCapabilityInfo>, Vec<NativeRunInfo>), String> {
    let client = daemon_client();
    let mut request = client
        .get(format!("{base_url}/api/capabilities/native"))
        .query(&[("scope", scope.query())]);
    if let Some(workspace) = workspace {
        request = request.query(&[("workspace", workspace)]);
    }
    let capabilities = response_json(request.send(), "load native capabilities")?;
    let runs = response_json(
        client
            .get(format!("{base_url}/api/capabilities/native/runs"))
            .query(&[("limit", 25usize)])
            .send(),
        "load native runs",
    )?;
    Ok((parse_capabilities(&capabilities), parse_runs(&runs)))
}

fn mutate_daemon(
    base_url: &str,
    mutation: &NativeMutation,
    workspace: Option<&str>,
) -> Result<Value, String> {
    let client = daemon_client();
    let request = match mutation {
        NativeMutation::Decide {
            name,
            scope,
            expected_hash,
            approve,
        } => client
            .post(format!(
                "{base_url}/api/capabilities/native/{name}/decision"
            ))
            .json(&json!({
                "decision": if *approve { "approve" } else { "reject" },
                "expected_hash": expected_hash,
                "scope": scope,
                "workspace": workspace,
            })),
        NativeMutation::Rollback {
            name,
            scope,
            target_hash,
        } => client
            .post(format!(
                "{base_url}/api/capabilities/native/{name}/rollback"
            ))
            .json(&json!({
                "target_hash": target_hash,
                "scope": scope,
                "workspace": workspace,
            })),
        NativeMutation::Disable { name, scope } => {
            let mut request = client
                .delete(format!("{base_url}/api/capabilities/native/{name}"))
                .query(&[("scope", scope.as_str())]);
            if let Some(workspace) = workspace {
                request = request.query(&[("workspace", workspace)]);
            }
            request
        }
        NativeMutation::ResolveRun {
            run_id,
            node_id,
            tool_use_id,
            attempt,
            decision,
        } => client
            .post(format!(
                "{base_url}/api/capabilities/native/runs/{run_id}/decision"
            ))
            .json(&run_decision_body(
                node_id,
                tool_use_id,
                *attempt,
                *decision,
            )),
    };
    response_json(request.send(), "mutate native capability")
}

fn mutate_in_process(
    kernel: std::sync::Arc<captain_kernel::CaptainKernel>,
    mutation: &NativeMutation,
    workspace: Option<&Path>,
) -> Result<Value, String> {
    match mutation {
        NativeMutation::Decide {
            name,
            scope,
            expected_hash,
            approve,
        } => kernel.capspec_management_decide(
            name,
            mutation_scope(scope)?,
            workspace,
            expected_hash,
            *approve,
            "tui",
        ),
        NativeMutation::Rollback {
            name,
            scope,
            target_hash,
        } => kernel.capspec_management_rollback(
            name,
            mutation_scope(scope)?,
            workspace,
            target_hash,
            "tui",
        ),
        NativeMutation::Disable { name, scope } => {
            kernel.capspec_management_disable(name, mutation_scope(scope)?, workspace, "tui")
        }
        NativeMutation::ResolveRun {
            run_id,
            node_id,
            tool_use_id,
            attempt,
            decision,
        } => resolve_run_in_process(kernel, run_id, node_id, tool_use_id, *attempt, *decision),
    }
}

fn forge_scope(scope: NativeScope) -> CapSpecForgeScope {
    match scope {
        NativeScope::Effective => CapSpecForgeScope::Effective,
        NativeScope::Global => CapSpecForgeScope::Global,
        NativeScope::Project => CapSpecForgeScope::Project,
    }
}

fn mutation_scope(scope: &str) -> Result<CapSpecForgeScope, String> {
    match scope {
        "global" => Ok(CapSpecForgeScope::Global),
        "project" => Ok(CapSpecForgeScope::Project),
        other => Err(format!(
            "Native mutation requires global or project scope, got '{other}'"
        )),
    }
}

fn mutation_label(mutation: &NativeMutation) -> String {
    match mutation {
        NativeMutation::Decide { name, approve, .. } => {
            format!("{name}: {}", if *approve { "approved" } else { "rejected" })
        }
        NativeMutation::Rollback {
            name, target_hash, ..
        } => format!("{name}: restored {}", short_hash(target_hash)),
        NativeMutation::Disable { name, .. } => format!("{name}: disabled with history"),
        NativeMutation::ResolveRun {
            run_id, decision, ..
        } => format!(
            "run {}: {} accepted",
            short_hash(run_id),
            run_decision_name(*decision)
        ),
    }
}

fn daemon_client() -> reqwest::blocking::Client {
    reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(10))
        .default_headers(crate::daemon_auth_headers())
        .build()
        .unwrap_or_else(|_| reqwest::blocking::Client::new())
}

fn response_json(
    response: Result<reqwest::blocking::Response, reqwest::Error>,
    action: &str,
) -> Result<Value, String> {
    let response = response.map_err(|error| format!("Failed to {action}: {error}"))?;
    let status = response.status();
    let body = response
        .json::<Value>()
        .map_err(|error| format!("Failed to decode {action}: {error}"))?;
    if status.is_success() {
        Ok(body)
    } else {
        Err(body["error"]
            .as_str()
            .map(str::to_string)
            .unwrap_or_else(|| format!("Failed to {action}: HTTP {status}")))
    }
}

fn parse_capabilities(body: &Value) -> Vec<NativeCapabilityInfo> {
    body["capabilities"]
        .as_array()
        .map(|items| items.iter().map(parse_capability).collect())
        .unwrap_or_default()
}

fn parse_runs(body: &Value) -> Vec<NativeRunInfo> {
    body["runs"]
        .as_array()
        .map(|items| {
            items
                .iter()
                .map(|item| NativeRunInfo {
                    run_id: text(item, "run_id"),
                    capability_name: text(item, "capability_name"),
                    source_hash: text(item, "source_hash"),
                    status: text(item, "status"),
                    origin: text(item, "origin"),
                    nodes: item["nodes"]
                        .as_array()
                        .map(|nodes| {
                            nodes
                                .iter()
                                .map(|node| NativeRunNode {
                                    step_id: text(node, "step_id"),
                                    tool_name: text(node, "tool_name"),
                                    status: text(node, "status"),
                                    attempts: node["attempts"].as_u64().unwrap_or(0) as u32,
                                    tool_use_id: optional_text(node, "tool_use_id"),
                                })
                                .collect()
                        })
                        .unwrap_or_default(),
                })
                .collect()
        })
        .unwrap_or_default()
}

fn parse_capability(item: &Value) -> NativeCapabilityInfo {
    NativeCapabilityInfo {
        name: text(item, "name"),
        tool_name: text(item, "tool_name"),
        description: text(item, "description"),
        version: text(item, "version"),
        status: text(item, "status"),
        scope: text(item, "scope"),
        ready: item["ready"].as_bool().unwrap_or(false),
        human_action_required: item["human_action_required"].as_bool().unwrap_or(false),
        active_hash: optional_text(item, "active_hash"),
        pending_hash: optional_text(item, "pending_hash"),
        selected_hash: optional_text(item, "selected_hash"),
        permission_fingerprint: text(item, "permission_fingerprint"),
        tools: item["permissions"]["tools"]
            .as_array()
            .map(|tools| {
                tools
                    .iter()
                    .filter_map(Value::as_str)
                    .map(str::to_string)
                    .collect()
            })
            .unwrap_or_default(),
        revisions: item["revisions"]
            .as_array()
            .map(|revisions| revisions.iter().map(parse_revision).collect())
            .unwrap_or_default(),
        source: optional_text(item, "source"),
        last_error: optional_text(item, "last_error"),
    }
}

fn parse_revision(item: &Value) -> NativeRevision {
    NativeRevision {
        source_hash: text(item, "source_hash"),
        version: text(item, "version"),
        approved_by: optional_text(item, "approved_by"),
        rejected_by: optional_text(item, "rejected_by"),
    }
}

fn text(value: &Value, key: &str) -> String {
    value[key].as_str().unwrap_or_default().to_string()
}

fn optional_text(value: &Value, key: &str) -> Option<String> {
    value[key].as_str().map(str::to_string)
}

fn short_hash(hash: &str) -> String {
    hash.chars().take(12).collect()
}

fn run_decision_body(
    node_id: &str,
    tool_use_id: &str,
    attempt: u32,
    decision: NativeRunDecision,
) -> Value {
    let mut body = json!({
        "node_id": node_id,
        "expected_tool_use_id": tool_use_id,
        "expected_attempt": attempt,
        "decision": run_decision_name(decision),
    });
    match decision {
        NativeRunDecision::ConfirmSucceeded => body["output"] = Value::Null,
        NativeRunDecision::Retry => {}
        NativeRunDecision::MarkFailed => {
            body["reason"] = json!("operator marked the uncertain side effect failed from TUI")
        }
    }
    body
}

fn run_decision_name(decision: NativeRunDecision) -> &'static str {
    match decision {
        NativeRunDecision::ConfirmSucceeded => "confirm_succeeded",
        NativeRunDecision::Retry => "retry",
        NativeRunDecision::MarkFailed => "mark_failed",
    }
}

fn resolve_run_in_process(
    kernel: std::sync::Arc<captain_kernel::CaptainKernel>,
    run_id: &str,
    node_id: &str,
    tool_use_id: &str,
    attempt: u32,
    decision: NativeRunDecision,
) -> Result<Value, String> {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|error| format!("start native run decision runtime: {error}"))?;
    runtime.block_on(async {
        let resolution = match decision {
            NativeRunDecision::ConfirmSucceeded => {
                captain_capspec::UncertainResolution::ConfirmSucceeded {
                    output: Value::Null,
                }
            }
            NativeRunDecision::Retry => captain_capspec::UncertainResolution::Retry,
            NativeRunDecision::MarkFailed => captain_capspec::UncertainResolution::MarkFailed {
                reason: "operator marked the uncertain side effect failed from TUI".to_string(),
            },
        };
        let response = kernel
            .capspec_management_resolve_run(
                run_id,
                node_id,
                captain_capspec::UncertainNodeExpectation {
                    tool_use_id: tool_use_id.to_string(),
                    attempt,
                },
                resolution,
                "tui",
            )
            .await?;
        if response["resume_scheduled"].as_bool().unwrap_or(false) {
            loop {
                let run = kernel.capspec_management_run(run_id)?;
                let status = run["status"].as_str().unwrap_or("unknown");
                if !matches!(status, "pending" | "interrupted" | "running") {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(25)).await;
            }
        }
        Ok(response)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_public_projection_without_runtime_payloads() {
        let body = json!({"capabilities": [{
            "name": "reader",
            "tool_name": "cap_reader",
            "status": "pending_approval",
            "scope": "global",
            "pending_hash": "full-hash",
            "human_action_required": true,
            "permissions": {"tools": ["file_read"]},
            "revisions": [{"source_hash": "full-hash", "version": "1"}],
        }]});
        let parsed = parse_capabilities(&body);
        assert_eq!(parsed[0].pending_hash.as_deref(), Some("full-hash"));
        assert_eq!(parsed[0].tools, ["file_read"]);
        assert_eq!(parsed[0].revisions[0].source_hash, "full-hash");
    }

    #[test]
    fn mutation_scope_never_accepts_effective_authority() {
        assert_eq!(mutation_scope("global").unwrap(), CapSpecForgeScope::Global);
        assert_eq!(
            mutation_scope("project").unwrap(),
            CapSpecForgeScope::Project
        );
        assert!(mutation_scope("effective").is_err());
    }

    #[test]
    fn mutation_labels_are_public_safe_and_bounded() {
        let label = mutation_label(&NativeMutation::Rollback {
            name: "reader".into(),
            scope: "global".into(),
            target_hash: "1234567890abcdef".into(),
        });
        assert_eq!(label, "reader: restored 1234567890ab");
    }

    #[test]
    fn run_projection_preserves_exact_uncertain_identity() {
        let body = json!({"runs": [{
            "run_id": "run-full",
            "capability_name": "writer",
            "source_hash": "source-full",
            "status": "waiting_decision",
            "origin": "telegram",
            "nodes": [{
                "step_id": "write",
                "tool_name": "file_write",
                "status": "uncertain",
                "attempts": 3,
                "tool_use_id": "capspec:run-full:write:3"
            }]
        }]});
        let runs = parse_runs(&body);
        assert_eq!(runs[0].source_hash, "source-full");
        assert_eq!(runs[0].origin, "telegram");
        assert_eq!(runs[0].nodes[0].attempts, 3);
        assert_eq!(
            runs[0].nodes[0].tool_use_id.as_deref(),
            Some("capspec:run-full:write:3")
        );
    }

    #[test]
    fn run_decision_payloads_are_strict_and_do_not_forge_an_actor() {
        let retry = run_decision_body(
            "write",
            "capspec:run-full:write:3",
            3,
            NativeRunDecision::Retry,
        );
        assert_eq!(retry["decision"], "retry");
        assert_eq!(retry["expected_attempt"], 3);
        assert!(retry.get("actor").is_none());
        assert!(retry.get("output").is_none());

        let confirmed = run_decision_body(
            "write",
            "capspec:run-full:write:3",
            3,
            NativeRunDecision::ConfirmSucceeded,
        );
        assert!(confirmed.get("output").is_some());
        assert!(confirmed["output"].is_null());
    }
}
