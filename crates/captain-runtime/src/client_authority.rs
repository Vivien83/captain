//! Fail-closed authority inherited by work started from a paired Client.

use captain_types::tool::ToolDefinition;

pub const PAIRED_CLIENT_ORIGIN: &str = "paired-client";
pub const PAIRED_CLIENT_AUTHORITY_VERSION: u16 = 1;

const ALLOWED_TOOLS: &[&str] = &[
    "artifact_inspect",
    "artifact_list",
    "ask_user",
    "capability_search",
    "captain_docs",
    "checkpoint_save",
    "knowledge_add_entity",
    "knowledge_add_relation",
    "knowledge_query",
    "memory_context_batch",
    "memory_forget",
    "memory_recall",
    "memory_save",
    "memory_store",
    "milestone_complete",
    "milestone_create",
    "milestone_list",
    "milestone_progress",
    "project_archive",
    "project_create",
    "project_delete",
    "project_get",
    "project_list",
    "project_resume",
    "project_task_create",
    "project_task_list",
    "project_task_update",
    "session_recall",
    "session_tool_call_summary",
    "system_time",
    "tool_run_cancel",
    "tool_search",
    "web_citation_audit",
    "web_fetch",
    "web_research_batch",
    "web_search",
];

pub fn paired_client_origin(surface: &str) -> String {
    let surface = surface
        .trim()
        .chars()
        .filter(|character| character.is_ascii_alphanumeric() || *character == '-')
        .take(32)
        .collect::<String>();
    if surface.is_empty() {
        PAIRED_CLIENT_ORIGIN.to_string()
    } else {
        format!("{PAIRED_CLIENT_ORIGIN}:{surface}")
    }
}

pub fn is_paired_client_origin(origin: Option<&str>) -> bool {
    origin.is_some_and(|origin| {
        origin == PAIRED_CLIENT_ORIGIN
            || origin
                .strip_prefix(PAIRED_CLIENT_ORIGIN)
                .is_some_and(|suffix| suffix.starts_with(':'))
    })
}

pub fn paired_client_tool_name_is_allowed(tool_name: &str) -> bool {
    ALLOWED_TOOLS.contains(&tool_name)
}

pub fn paired_client_tool_name_is_allowed_for_route(tool_name: &str, node_target: bool) -> bool {
    paired_client_tool_name_is_allowed(tool_name)
        || (node_target && crate::node_tool_runtime::local_node_tool_family(tool_name).is_some())
}

pub fn paired_client_tool_is_allowed(tool_name: &str, input: &serde_json::Value) -> bool {
    let node_target = matches!(
        crate::execution_routing::current_turn_execution_context().map(|context| context.target),
        Some(crate::execution_routing::ResolvedExecutionTarget::Node { .. })
    );
    paired_client_tool_name_is_allowed_for_route(tool_name, node_target)
        && match tool_name {
            "web_fetch" => web_fetch_is_read_only(input),
            _ => true,
        }
}

pub fn paired_client_agent_module_is_allowed(module: &str) -> bool {
    matches!(module, "builtin:chat" | "llm")
}

pub fn paired_client_daemon_command_is_allowed(command: &str) -> bool {
    matches!(
        command
            .trim_start_matches('/')
            .to_ascii_lowercase()
            .as_str(),
        "status" | "health" | "version"
    )
}

pub fn filter_tool_definitions_for_origin(
    tools: Vec<ToolDefinition>,
    origin: Option<&str>,
) -> Vec<ToolDefinition> {
    let node_target = matches!(
        crate::execution_routing::current_turn_execution_context().map(|context| context.target),
        Some(crate::execution_routing::ResolvedExecutionTarget::Node { .. })
    );
    filter_tool_definitions_for_origin_and_route(tools, origin, node_target)
}

pub fn filter_tool_definitions_for_origin_and_route(
    tools: Vec<ToolDefinition>,
    origin: Option<&str>,
    node_target: bool,
) -> Vec<ToolDefinition> {
    if !is_paired_client_origin(origin) {
        return tools;
    }
    tools
        .into_iter()
        .filter(|tool| paired_client_tool_name_is_allowed_for_route(&tool.name, node_target))
        .map(restrict_visible_schema)
        .collect()
}

fn web_fetch_is_read_only(input: &serde_json::Value) -> bool {
    let method = input
        .get("method")
        .and_then(serde_json::Value::as_str)
        .unwrap_or("GET");
    if !method.eq_ignore_ascii_case("GET") {
        return false;
    }
    if input
        .get("body")
        .is_some_and(|body| !body.is_null() && body.as_str().is_none_or(|body| !body.is_empty()))
    {
        return false;
    }
    !input
        .get("headers")
        .and_then(serde_json::Value::as_object)
        .is_some_and(|headers| {
            headers.keys().any(|name| {
                matches!(
                    name.to_ascii_lowercase().as_str(),
                    "authorization" | "cookie" | "proxy-authorization" | "x-api-key"
                )
            })
        })
}

fn restrict_visible_schema(mut tool: ToolDefinition) -> ToolDefinition {
    if tool.name == "web_fetch" {
        tool.description.push_str(
            " Paired Client authority limits this tool to GET without credential headers or a request body.",
        );
        if let Some(method) = tool
            .input_schema
            .pointer_mut("/properties/method")
            .and_then(serde_json::Value::as_object_mut)
        {
            method.insert("enum".to_string(), serde_json::json!(["GET"]));
            method.insert("default".to_string(), serde_json::json!("GET"));
        }
        if let Some(properties) = tool
            .input_schema
            .get_mut("properties")
            .and_then(serde_json::Value::as_object_mut)
        {
            properties.remove("body");
        }
    }
    tool
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tool(name: &str) -> ToolDefinition {
        ToolDefinition {
            name: name.to_string(),
            description: String::new(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "method": {"type": "string", "enum": ["GET", "POST"]},
                    "body": {"type": "string"}
                }
            }),
        }
    }

    #[test]
    fn origin_marker_is_exact_and_surface_bounded() {
        assert!(is_paired_client_origin(Some("paired-client")));
        assert!(is_paired_client_origin(Some("paired-client:project")));
        assert!(!is_paired_client_origin(Some("paired-client-operator")));
        assert!(!is_paired_client_origin(Some("web")));
        assert_eq!(
            paired_client_origin("Project / Local"),
            "paired-client:ProjectLocal"
        );
    }

    #[test]
    fn future_and_administrative_tools_fail_closed() {
        for tool in [
            "config_read",
            "secret_read",
            "shell_exec",
            "file_read",
            "agent_spawn",
            "model_switch_apply",
            "skill_execute",
            "system_update",
            "future_tool",
        ] {
            assert!(!paired_client_tool_is_allowed(tool, &serde_json::json!({})));
        }
        for tool in [
            "memory_save",
            "project_task_update",
            "session_recall",
            "tool_run_cancel",
            "web_search",
        ] {
            assert!(paired_client_tool_is_allowed(tool, &serde_json::json!({})));
        }
    }

    #[test]
    fn only_llm_modules_and_read_only_daemon_commands_are_allowed() {
        assert!(paired_client_agent_module_is_allowed("builtin:chat"));
        assert!(paired_client_agent_module_is_allowed("llm"));
        assert!(!paired_client_agent_module_is_allowed("wasm:agent.wasm"));
        assert!(!paired_client_agent_module_is_allowed("python:agent.py"));
        assert!(!paired_client_agent_module_is_allowed("future:module"));

        for command in ["status", "/health", "VERSION"] {
            assert!(paired_client_daemon_command_is_allowed(command));
        }
        for command in ["config", "reload", "restart", "shutdown", "future"] {
            assert!(!paired_client_daemon_command_is_allowed(command));
        }
    }

    #[test]
    fn web_fetch_is_read_only_and_rejects_credentials() {
        assert!(paired_client_tool_is_allowed(
            "web_fetch",
            &serde_json::json!({"url": "https://example.com", "method": "GET"})
        ));
        assert!(!paired_client_tool_is_allowed(
            "web_fetch",
            &serde_json::json!({"url": "https://example.com", "method": "POST"})
        ));
        assert!(!paired_client_tool_is_allowed(
            "web_fetch",
            &serde_json::json!({
                "url": "https://example.com",
                "headers": {"Authorization": "Bearer hidden"}
            })
        ));
    }

    #[test]
    fn visible_catalog_matches_dispatch_and_narrows_web_fetch_schema() {
        let tools = filter_tool_definitions_for_origin(
            vec![tool("web_fetch"), tool("config_read"), tool("future_tool")],
            Some("paired-client:chat"),
        );
        assert_eq!(tools.len(), 1);
        assert_eq!(tools[0].name, "web_fetch");
        assert_eq!(
            tools[0].input_schema["properties"]["method"]["enum"],
            serde_json::json!(["GET"])
        );
        assert!(tools[0].input_schema["properties"].get("body").is_none());
    }

    #[test]
    fn node_local_tools_are_visible_only_for_an_explicit_node_route() {
        let source = vec![tool("file_read"), tool("shell_exec"), tool("config_read")];
        assert!(filter_tool_definitions_for_origin_and_route(
            source.clone(),
            Some("paired-client:chat"),
            false,
        )
        .is_empty());

        let names =
            filter_tool_definitions_for_origin_and_route(source, Some("paired-client:chat"), true)
                .into_iter()
                .map(|tool| tool.name)
                .collect::<Vec<_>>();
        assert_eq!(names, vec!["file_read", "shell_exec"]);
    }

    #[tokio::test]
    async fn dispatch_authority_never_grants_node_tools_on_the_hub() {
        let hub_allowed = crate::execution_routing::with_turn_execution_context(
            crate::execution_routing::TurnExecutionContext::hub("session-1"),
            async { paired_client_tool_is_allowed("file_read", &serde_json::json!({})) },
        )
        .await;
        let node_allowed = crate::execution_routing::with_turn_execution_context(
            crate::execution_routing::TurnExecutionContext::node(
                "session-1",
                "node-1",
                "workspace-1",
            ),
            async { paired_client_tool_is_allowed("file_read", &serde_json::json!({})) },
        )
        .await;

        assert!(!hub_allowed);
        assert!(node_allowed);
        assert!(!paired_client_tool_name_is_allowed_for_route(
            "config_read",
            true
        ));
    }
}
