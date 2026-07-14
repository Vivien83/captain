use captain_runtime::core_tools::{is_core_tool, CORE_TOOLS};
use captain_types::agent::ToolProfile;
use captain_types::tool::ToolDefinition;

/// TS.2 — Pick the visible builtin tool subset for an agent.
///
/// Declared tools are exact, curated profiles get their listed tools plus CORE
/// discovery, and unrestricted agents see only CORE tools. Non-CORE builtins
/// remain discoverable through tool search instead of being eagerly injected.
pub fn filter_builtins_for_agent(
    all_builtins: Vec<ToolDefinition>,
    declared_tools: &[String],
    profile: &Option<ToolProfile>,
) -> Vec<ToolDefinition> {
    let tools_unrestricted = declared_tools.is_empty() || declared_tools.iter().any(|t| t == "*");

    if !tools_unrestricted {
        return all_builtins
            .into_iter()
            .filter(|t| declared_tools.iter().any(|d| d == &t.name))
            .collect();
    }

    match profile {
        Some(p) if *p != ToolProfile::Full && *p != ToolProfile::Custom => {
            let mut allowed = p.tools();
            for core in CORE_TOOLS {
                if !allowed.iter().any(|a| a == core) {
                    allowed.push((*core).to_string());
                }
            }
            all_builtins
                .into_iter()
                .filter(|t| allowed.iter().any(|a| a == "*" || a == &t.name))
                .collect()
        }
        _ => all_builtins
            .into_iter()
            .filter(|t| is_core_tool(&t.name))
            .collect(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn td(name: &str) -> ToolDefinition {
        ToolDefinition {
            name: name.to_string(),
            description: format!("desc for {name}"),
            input_schema: serde_json::json!({}),
        }
    }

    fn fixture_builtins() -> Vec<ToolDefinition> {
        vec![
            td("capability_search"),
            td("tool_search"),
            td("captain_docs"),
            td("ask_user"),
            td("file_read"),
            td("shell_exec"),
            td("browser_navigate"),
            td("text_to_speech"),
        ]
    }

    #[test]
    fn ts2_unrestricted_no_profile_keeps_only_core() {
        let result = filter_builtins_for_agent(fixture_builtins(), &[], &None);
        let names: Vec<&str> = result.iter().map(|t| t.name.as_str()).collect();

        assert!(names.contains(&"capability_search"));
        assert!(names.contains(&"tool_search"));
        assert!(!names.contains(&"file_read"));
        assert!(!names.contains(&"shell_exec"));
        assert!(
            !names.contains(&"browser_navigate"),
            "non-CORE leaked: {names:?}"
        );
        assert!(
            names.contains(&"text_to_speech"),
            "native voice CORE missing: {names:?}"
        );
    }

    #[test]
    fn ts2_unrestricted_full_profile_keeps_only_core() {
        let result = filter_builtins_for_agent(fixture_builtins(), &[], &Some(ToolProfile::Full));
        let names: Vec<&str> = result.iter().map(|t| t.name.as_str()).collect();

        assert!(!names.contains(&"browser_navigate"));
        assert!(!names.contains(&"file_read"));
        assert!(names.contains(&"capability_search"));
    }

    #[test]
    fn ts2_unrestricted_custom_profile_keeps_only_core() {
        let result = filter_builtins_for_agent(fixture_builtins(), &[], &Some(ToolProfile::Custom));
        let names: Vec<&str> = result.iter().map(|t| t.name.as_str()).collect();

        assert!(!names.contains(&"browser_navigate"));
        assert!(!names.contains(&"shell_exec"));
        assert!(names.contains(&"tool_search"));
    }

    #[test]
    fn ts2_curated_profile_overrides_core_filter() {
        let mut all = fixture_builtins();
        all.push(td("web_fetch"));

        let result = filter_builtins_for_agent(all, &[], &Some(ToolProfile::Coding));
        let names: Vec<&str> = result.iter().map(|t| t.name.as_str()).collect();

        assert!(names.contains(&"file_read"));
        assert!(names.contains(&"shell_exec"));
        assert!(names.contains(&"web_fetch"));
        assert!(names.contains(&"capability_search"));
        assert!(names.contains(&"tool_search"));
        assert!(!names.contains(&"browser_navigate"));
        assert!(names.contains(&"text_to_speech"));
    }

    #[test]
    fn ts2_declared_tools_override_everything() {
        let result = filter_builtins_for_agent(
            fixture_builtins(),
            &["browser_navigate".to_string()],
            &Some(ToolProfile::Full),
        );
        let names: Vec<&str> = result.iter().map(|t| t.name.as_str()).collect();

        assert_eq!(names, vec!["browser_navigate"]);
    }

    #[test]
    fn ts2_declared_wildcard_means_unrestricted() {
        let result = filter_builtins_for_agent(fixture_builtins(), &["*".to_string()], &None);
        let names: Vec<&str> = result.iter().map(|t| t.name.as_str()).collect();

        assert!(!names.contains(&"browser_navigate"));
        assert!(!names.contains(&"file_read"));
        assert!(names.contains(&"capability_search"));
    }

    #[test]
    fn ts2_real_builtins_collapse_to_core_size() {
        let real = captain_runtime::tool_runner::builtin_tool_definitions();
        let visible = filter_builtins_for_agent(real, &[], &None);

        assert_eq!(
            visible.len(),
            CORE_TOOLS.len(),
            "TS.2: an unrestricted agent must see exactly the CORE tools, \
             got {} ({:?})",
            visible.len(),
            visible.iter().map(|t| t.name.as_str()).collect::<Vec<_>>()
        );
    }
}
