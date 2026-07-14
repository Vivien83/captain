use crate::project_runtime_tool_status::approved_tools_for_phase;
use captain_types::agent::ToolProfile;

pub(crate) fn runtime_worker_authorized_tools(profile: &ToolProfile) -> Vec<String> {
    let mut tools = profile.tools();
    for tool in captain_runtime::core_tools::SUBAGENT_DEFAULT_TOOLS {
        if !tools.iter().any(|existing| existing == tool) {
            tools.push((*tool).to_string());
        }
    }
    tools
}

pub(crate) fn runtime_worker_authorized_tools_for_runtime(
    profile: &ToolProfile,
    phase: &str,
    runtime: &serde_json::Value,
) -> Vec<String> {
    let mut tools = runtime_worker_authorized_tools(profile);
    for tool in approved_tools_for_phase(runtime, phase) {
        if !tools.iter().any(|existing| existing == &tool) {
            tools.push(tool);
        }
    }
    tools
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn worker_tools_include_profile_and_discovery_defaults() {
        let tools = runtime_worker_authorized_tools(&ToolProfile::Coding);

        assert!(tools.contains(&"file_read".to_string()));
        assert!(tools.contains(&"shell_exec".to_string()));
        assert!(tools.contains(&"capability_search".to_string()));
        assert!(tools.contains(&"tool_search".to_string()));
        assert!(tools.contains(&"captain_docs".to_string()));
    }

    #[test]
    fn worker_tools_include_only_approved_phase_request_tools() {
        let runtime = serde_json::json!({
            "worker_results": {
                "build": {
                    "tool_request": {
                        "status": "approved",
                        "tools": ["extra_tool"]
                    }
                },
                "verify": {
                    "tool_request": {
                        "status": "approved",
                        "tools": ["verify_only"]
                    }
                },
                "execute": {
                    "tool_request": {
                        "status": "denied",
                        "tools": ["denied_tool"]
                    }
                }
            }
        });

        let tools =
            runtime_worker_authorized_tools_for_runtime(&ToolProfile::Coding, "build", &runtime);
        assert!(tools.contains(&"extra_tool".to_string()));
        assert!(!tools.contains(&"verify_only".to_string()));
        assert!(!tools.contains(&"denied_tool".to_string()));
    }

    #[test]
    fn worker_tools_do_not_duplicate_defaults() {
        let tools = runtime_worker_authorized_tools_for_runtime(
            &ToolProfile::Coding,
            "build",
            &serde_json::json!({}),
        );
        assert_eq!(
            tools
                .iter()
                .filter(|tool| tool.as_str() == "shell_exec")
                .count(),
            1
        );
    }
}
