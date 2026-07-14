//! Agent and fleet orchestration handlers.

use crate::core_tools::SUBAGENT_DEFAULT_TOOLS;
use crate::kernel_handle::KernelHandle;
use crate::tools::{require_kernel, AGENT_CALL_DEPTH, MAX_AGENT_CALL_DEPTH};
use captain_types::agent::AgentManifest;
use captain_types::agent_api::{AgentApiSpawnProvisionReport, AgentApiSpawnProvisionRequest};
use captain_types::tool_compat::normalize_tool_name;
use std::collections::HashSet;
use std::sync::Arc;

pub(crate) async fn tool_agent_send(
    input: &serde_json::Value,
    kernel: Option<&Arc<dyn KernelHandle>>,
) -> Result<String, String> {
    let kh = require_kernel(kernel)?;
    let agent_id = input["agent_id"]
        .as_str()
        .ok_or("Missing 'agent_id' parameter")?;
    let message = input["message"]
        .as_str()
        .ok_or("Missing 'message' parameter")?;

    let current_depth = AGENT_CALL_DEPTH.try_with(|d| d.get()).unwrap_or(0);
    if current_depth >= MAX_AGENT_CALL_DEPTH {
        return Err(format!(
            "Inter-agent call depth exceeded (max {}). \
             A->B->C chain is too deep. Use the task queue instead.",
            MAX_AGENT_CALL_DEPTH
        ));
    }

    AGENT_CALL_DEPTH
        .scope(std::cell::Cell::new(current_depth + 1), async {
            kh.send_to_agent(agent_id, message).await
        })
        .await
}

pub(crate) async fn tool_agent_spawn(
    input: &serde_json::Value,
    kernel: Option<&Arc<dyn KernelHandle>>,
    parent_id: Option<&str>,
    parent_allowed_tools: Option<&[String]>,
) -> Result<String, String> {
    let kh = require_kernel(kernel)?;
    let manifest_toml = input["manifest_toml"]
        .as_str()
        .ok_or("Missing 'manifest_toml' parameter")?;
    let provision_request = parse_agent_api_spawn_provision(input)?;
    validate_child_agent_tool_scope(manifest_toml, parent_allowed_tools)?;
    let (id, name) = kh.spawn_agent(manifest_toml, parent_id).await?;
    let api_report = kh
        .provision_spawned_agent_api(&id, provision_request)
        .await
        .ok();
    Ok(format_agent_spawn_success(&id, &name, api_report.as_ref()))
}

fn parse_agent_api_spawn_provision(
    input: &serde_json::Value,
) -> Result<AgentApiSpawnProvisionRequest, String> {
    match input.get("agent_api") {
        Some(value) if !value.is_null() => serde_json::from_value(value.clone())
            .map_err(|err| format!("Invalid 'agent_api' provisioning object: {err}")),
        _ => Ok(AgentApiSpawnProvisionRequest::default()),
    }
}

fn format_agent_spawn_success(
    id: &str,
    name: &str,
    api_report: Option<&AgentApiSpawnProvisionReport>,
) -> String {
    let mut output = format!("Agent spawned successfully.\n  ID: {id}\n  Name: {name}");
    let Some(report) = api_report else {
        output.push_str(&format!(
            "\n\nAgent API protocol:\n  Status: provisioning_unavailable\n  Next: inspect with captain agent api {id} or GET /api/agents/{id}/api"
        ));
        return output;
    };

    output.push_str(&format!(
        "\n\nAgent API protocol ({})\n  Status: {}\n  Manifest: {}\n  Events: {}",
        report.protocol, report.status, report.manifest_url, report.audit_events_url
    ));
    output.push_str(&format!(
        "\n\nIngress\n  Status: {}\n  URL: {}\n  Auth: {}\n  Token env: {}\n  Rotate: {}",
        report.ingress.status,
        report.ingress.ingress_url,
        report.ingress.auth_scheme,
        report.ingress.token_env,
        report.ingress.token_rotate_url
    ));
    if let Some(token) = report.ingress.token.as_deref() {
        output.push_str(&format!("\n  Token returned once: {token}"));
    }
    if let Some(warning) = report.ingress.warning.as_deref() {
        output.push_str(&format!("\n  Warning: {warning}"));
    }
    output.push_str(&format!(
        "\n\nEgress\n  Status: {}\n  Configure: {}\n  Test: {}\n  Queue: {}\n  Retry: {}",
        report.egress.status,
        report.egress.configure_url,
        report.egress.test_url,
        report.egress.queue_status_url,
        report.egress.retry_url_template
    ));
    if let Some(callback_secret) = report.egress.callback_secret.as_deref() {
        output.push_str(&format!(
            "\n  Callback secret returned once: {callback_secret}"
        ));
    }
    if let Some(issue) = report.egress.issue.as_deref() {
        output.push_str(&format!("\n  Issue: {issue}"));
    }
    if !report.operator_actions.is_empty() {
        output.push_str("\n\nOperator actions");
        for action in &report.operator_actions {
            output.push_str(&format!("\n  - {action}"));
        }
    }
    output
}

fn effective_child_tool_policy(manifest: &AgentManifest) -> Option<Vec<String>> {
    let mut tools = explicit_child_tool_policy(manifest)?;
    add_subagent_default_tools(&mut tools);
    Some(tools)
}

fn explicit_child_tool_policy(manifest: &AgentManifest) -> Option<Vec<String>> {
    if !manifest.tool_allowlist.is_empty() {
        if manifest.tool_allowlist.iter().any(|t| t == "*") {
            return None;
        }
        return Some(normalized_tool_list(&manifest.tool_allowlist));
    }

    if !manifest.capabilities.tools.is_empty() {
        if manifest.capabilities.tools.iter().any(|t| t == "*") {
            return None;
        }
        return Some(normalized_tool_list(&manifest.capabilities.tools));
    }

    None
}

fn normalized_tool_list(tools: &[String]) -> Vec<String> {
    let mut out = Vec::new();
    for tool in tools {
        push_normalized_tool(&mut out, tool);
    }
    out
}

fn push_normalized_tool(tools: &mut Vec<String>, tool: &str) {
    let normalized = normalize_tool_name(tool);
    if !tools
        .iter()
        .any(|existing| normalize_tool_name(existing) == normalized)
    {
        tools.push(normalized.to_string());
    }
}

fn add_subagent_default_tools(tools: &mut Vec<String>) {
    for tool in SUBAGENT_DEFAULT_TOOLS {
        push_normalized_tool(tools, tool);
    }
}

pub(crate) fn validate_child_agent_tool_scope(
    manifest_toml: &str,
    parent_allowed_tools: Option<&[String]>,
) -> Result<(), String> {
    let child_manifest: AgentManifest = toml::from_str(manifest_toml)
        .map_err(|e| captain_types::agent::format_agent_manifest_parse_error(&e, manifest_toml))?;
    let child_tools = effective_child_tool_policy(&child_manifest).ok_or_else(|| {
        "Denied agent_spawn: child manifest must declare an explicit non-wildcard \
         tool_allowlist or capabilities.tools. Sub-agents cannot rely on a profile-only \
         or unrestricted tool surface."
            .to_string()
    })?;

    let Some(parent_allowed_tools) = parent_allowed_tools else {
        return Ok(());
    };
    if parent_allowed_tools.iter().any(|t| t == "*") {
        return Ok(());
    }

    let mut parent_tools: HashSet<String> = parent_allowed_tools
        .iter()
        .map(|t| normalize_tool_name(t).to_string())
        .collect();
    for tool in SUBAGENT_DEFAULT_TOOLS {
        parent_tools.insert(normalize_tool_name(tool).to_string());
    }
    let denied: Vec<String> = child_tools
        .iter()
        .map(|t| normalize_tool_name(t).to_string())
        .filter(|t| !parent_tools.contains(t))
        .collect();

    if denied.is_empty() {
        Ok(())
    } else {
        Err(format!(
            "Denied agent_spawn: child requests tools outside parent scope: {}",
            denied.join(", ")
        ))
    }
}

pub(crate) fn tool_agent_list(kernel: Option<&Arc<dyn KernelHandle>>) -> Result<String, String> {
    let kh = require_kernel(kernel)?;
    let agents = kh.list_agents();
    if agents.is_empty() {
        return Ok("No agents currently running.".to_string());
    }
    let mut output = format!("Running agents ({}):\n", agents.len());
    for a in &agents {
        output.push_str(&format!(
            "  - {} (id: {}, state: {}, model: {}:{})\n",
            a.name, a.id, a.state, a.model_provider, a.model_name
        ));
    }
    Ok(output)
}

pub(crate) fn tool_agent_kill(
    input: &serde_json::Value,
    kernel: Option<&Arc<dyn KernelHandle>>,
) -> Result<String, String> {
    let kh = require_kernel(kernel)?;
    let agent_id = input["agent_id"]
        .as_str()
        .ok_or("Missing 'agent_id' parameter")?;
    kh.kill_agent(agent_id)?;
    Ok(format!("Agent {agent_id} killed successfully."))
}

pub(crate) fn tool_agent_status(
    input: &serde_json::Value,
    kernel: Option<&Arc<dyn KernelHandle>>,
) -> Result<String, String> {
    let kh = require_kernel(kernel)?;
    let agent_id = input["agent_id"].as_str().ok_or("Missing 'agent_id'")?;
    let status = kh.agent_status_info(agent_id)?;
    Ok(serde_json::to_string_pretty(&status).unwrap_or_else(|_| status.to_string()))
}

pub(crate) fn tool_agent_caps(
    input: &serde_json::Value,
    kernel: Option<&Arc<dyn KernelHandle>>,
) -> Result<String, String> {
    let kh = require_kernel(kernel)?;
    let agent_id = input["agent_id"].as_str().ok_or("Missing 'agent_id'")?;
    let report = kh.agent_capability_report(agent_id)?;
    Ok(serde_json::to_string_pretty(&report).unwrap_or_else(|_| report.to_string()))
}

pub(crate) async fn tool_agent_watch(
    input: &serde_json::Value,
    kernel: Option<&Arc<dyn KernelHandle>>,
) -> Result<String, String> {
    let kh = require_kernel(kernel)?;
    let agent_id = input["agent_id"].as_str().ok_or("Missing 'agent_id'")?;
    let limit = input["limit"].as_u64().unwrap_or(10) as usize;
    let events = kh.agent_events(agent_id, limit).await?;
    if events.is_empty() {
        return Ok(format!("No recent events for agent {agent_id}."));
    }
    Ok(serde_json::to_string_pretty(&events).unwrap_or_else(|_| format!("{events:?}")))
}

pub(crate) async fn tool_agent_delegate(
    input: &serde_json::Value,
    kernel: Option<&Arc<dyn KernelHandle>>,
) -> Result<String, String> {
    let kh = require_kernel(kernel)?;
    let agent_id = input["agent_id"].as_str().ok_or("Missing 'agent_id'")?;
    let task = input["task"].as_str().ok_or("Missing 'task'")?;
    let max_tokens = input["max_tokens"].as_u64().unwrap_or(5000);
    let current_depth = AGENT_CALL_DEPTH.try_with(|d| d.get()).unwrap_or(0);
    if current_depth >= MAX_AGENT_CALL_DEPTH {
        return Err(format!(
            "Inter-agent delegation depth exceeded (max {}). \
             A->B->C chain is too deep. Use task_post/task_claim instead.",
            MAX_AGENT_CALL_DEPTH
        ));
    }

    AGENT_CALL_DEPTH
        .scope(std::cell::Cell::new(current_depth + 1), async {
            kh.delegate_task(agent_id, task, max_tokens).await
        })
        .await
}

pub(crate) async fn tool_agent_correct(
    input: &serde_json::Value,
    kernel: Option<&Arc<dyn KernelHandle>>,
) -> Result<String, String> {
    let kh = require_kernel(kernel)?;
    let agent_id = input["agent_id"].as_str().ok_or("Missing 'agent_id'")?;
    let message = input["message"].as_str().ok_or("Missing 'message'")?;
    kh.inject_system_message(agent_id, message).await?;
    Ok(format!("Correction sent to agent {agent_id}."))
}

pub(crate) async fn tool_fleet_create_manager(
    input: &serde_json::Value,
    kernel: Option<&Arc<dyn KernelHandle>>,
) -> Result<String, String> {
    let kh = require_kernel(kernel)?;
    let name = input["name"].as_str().ok_or("Missing 'name'")?;
    let domain = input["domain"].as_str().ok_or("Missing 'domain'")?;
    let model = input["model"].as_str();
    let budget = input["budget_tokens"].as_u64().unwrap_or(10000);
    let (id, spawned_name) = kh.create_manager(name, domain, model, budget).await?;
    Ok(format!(
        "Manager '{spawned_name}' created (id: {id}, budget: {budget} tokens/h)."
    ))
}

pub(crate) fn tool_fleet_list_managers(
    kernel: Option<&Arc<dyn KernelHandle>>,
) -> Result<String, String> {
    let kh = require_kernel(kernel)?;
    let managers = kh.list_managers();
    if managers.is_empty() {
        return Ok("No active managers.".to_string());
    }
    Ok(serde_json::to_string_pretty(&managers).unwrap_or_else(|_| format!("{managers:?}")))
}

pub(crate) async fn tool_fleet_close_manager(
    input: &serde_json::Value,
    kernel: Option<&Arc<dyn KernelHandle>>,
) -> Result<String, String> {
    let kh = require_kernel(kernel)?;
    let manager_id = input["manager_id"].as_str().ok_or("Missing 'manager_id'")?;
    let killed = kh.close_manager(manager_id).await?;
    Ok(format!("Manager closed. {killed} agent(s) terminated."))
}

pub(crate) fn tool_fleet_set_mission(
    input: &serde_json::Value,
    kernel: Option<&Arc<dyn KernelHandle>>,
) -> Result<String, String> {
    let kh = require_kernel(kernel)?;
    let manager_id = input["manager_id"].as_str().ok_or("Missing 'manager_id'")?;
    let mission = input["mission"]
        .as_str()
        .map(str::trim)
        .filter(|s| !s.is_empty());
    kh.set_manager_mission(manager_id, mission)?;
    Ok(match mission {
        Some(m) => format!("Mission set: {m}"),
        None => "Mission cleared.".to_string(),
    })
}

pub(crate) fn tool_fleet_configure_autoscale(
    input: &serde_json::Value,
    kernel: Option<&Arc<dyn KernelHandle>>,
) -> Result<String, String> {
    let kh = require_kernel(kernel)?;
    let manager_id = input["manager_id"].as_str().ok_or("Missing 'manager_id'")?;
    let cfg = captain_types::agent::AutoScaleConfig {
        enabled: input["enabled"].as_bool().unwrap_or(true),
        min_workers: input["min_workers"].as_u64().unwrap_or(0) as u32,
        max_workers: input["max_workers"].as_u64().unwrap_or(3) as u32,
        spawn_threshold: input["spawn_threshold"].as_u64().unwrap_or(2) as u32,
        kill_threshold: input["kill_threshold"].as_u64().unwrap_or(0) as u32,
        cooldown_secs: input["cooldown_secs"].as_u64().unwrap_or(60),
        worker_template: input["worker_template"].as_str().map(String::from),
    };
    kh.configure_autoscale(manager_id, cfg.clone())?;
    Ok(format!(
        "Autoscale configured: min={} max={} spawn>={} kill<={} cooldown={}s",
        cfg.min_workers,
        cfg.max_workers,
        cfg.spawn_threshold,
        cfg.kill_threshold,
        cfg.cooldown_secs
    ))
}

pub(crate) fn tool_fleet_metrics(
    input: &serde_json::Value,
    kernel: Option<&Arc<dyn KernelHandle>>,
) -> Result<String, String> {
    let kh = require_kernel(kernel)?;
    let manager_id = input["manager_id"].as_str().ok_or("Missing 'manager_id'")?;
    let metrics = kh.fleet_metrics(manager_id)?;
    Ok(serde_json::to_string_pretty(&metrics).unwrap_or_else(|_| format!("{metrics:?}")))
}

#[cfg(test)]
mod tests {
    use super::*;
    use captain_types::{
        agent::AgentId,
        agent_api::{pending_egress_report, ready_ingress_report, AgentApiSpawnProvisionReport},
    };

    #[test]
    fn agent_spawn_success_includes_api_protocol_and_actions() {
        let agent_id: AgentId = "99999999-9999-9999-9999-999999999999".parse().unwrap();
        let report = AgentApiSpawnProvisionReport::new(
            &agent_id,
            ready_ingress_report(&agent_id, "cap_at_test-token-value".to_string()),
            pending_egress_report(&agent_id),
            Vec::new(),
        );

        let output =
            format_agent_spawn_success(&agent_id.to_string(), "veille-tech", Some(&report));

        assert!(output.contains("Agent spawned successfully."));
        assert!(output.contains("ID: 99999999-9999-9999-9999-999999999999"));
        assert!(output.contains("Name: veille-tech"));
        assert!(output.contains("Agent API protocol (agent-as-service.v1)"));
        assert!(output.contains("Status: ingress_ready"));
        assert!(output.contains("/hooks/agents/99999999-9999-9999-9999-999999999999/ingress"));
        assert!(output.contains("Token returned once: cap_at_test-token-value"));
        assert!(output
            .contains("/api/agents/99999999-9999-9999-9999-999999999999/api/egress/configure"));
        assert!(output.contains("cannot infer the external callback URL"));
        assert!(output.contains("Operator actions"));
    }

    #[test]
    fn agent_api_spawn_provision_defaults_to_ingress_token() {
        let parsed = parse_agent_api_spawn_provision(&serde_json::json!({})).unwrap();

        assert!(parsed.provision_ingress_token);
        assert!(parsed.generate_callback_secret);
        assert!(parsed.egress_callback_url.is_none());
    }
}
