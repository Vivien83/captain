use super::*;

#[test]
fn test_tool_grouping() {
    let tools = vec![
        "web_search".to_string(),
        "web_fetch".to_string(),
        "file_read".to_string(),
        "browser_navigate".to_string(),
    ];
    let section = build_tools_section(&tools);
    assert!(section.contains("**Browser**"));
    assert!(section.contains("**Files**"));
    assert!(section.contains("**Web**"));
}

#[test]
fn test_tool_categories() {
    assert_eq!(tool_category("file_read"), "Files");
    assert_eq!(tool_category("web_search"), "Web");
    assert_eq!(tool_category("browser_navigate"), "Browser");
    assert_eq!(tool_category("shell_exec"), "Shell");
    assert_eq!(tool_category("memory_store"), "Memory");
    assert_eq!(tool_category("project_list"), "Projects");
    assert_eq!(tool_category("agent_send"), "Agents");
    assert_eq!(tool_category("mcp_github_search"), "MCP");
    assert_eq!(tool_category("unknown_tool"), "Other");
}

#[test]
fn test_tool_hints() {
    assert!(!tool_hint("web_search").is_empty());
    assert!(!tool_hint("file_read").is_empty());
    assert!(!tool_hint("browser_navigate").is_empty());
    assert!(tool_hint("some_unknown_tool").is_empty());
}

/// v3.7d — Critical tools carry a WHEN/WHY/SKIP triptych so the LLM
/// sees the decision framework next to the JSON schema. Matches the
/// format expected by the memory tool schema.
#[test]
fn test_critical_tools_have_full_doc() {
    let mut critical = crate::core_tools::CORE_TOOLS.to_vec();
    critical.extend([
        "file_delete",
        "knowledge_add_entity",
        "skill_execute",
        "mcp_setup",
        "process_start",
    ]);
    critical.sort_unstable();
    critical.dedup();
    for name in critical {
        let doc = tool_doc(name).unwrap_or_else(|| panic!("tool_doc missing for {name}"));
        assert!(
            doc.contains("WHEN"),
            "tool_doc({name}) must contain WHEN section"
        );
        assert!(
            doc.contains("WHY"),
            "tool_doc({name}) must contain WHY section"
        );
        assert!(
            doc.contains("SKIP"),
            "tool_doc({name}) must contain SKIP section"
        );
    }
}

#[test]
fn test_core_tools_have_prompt_hints() {
    for name in crate::core_tools::CORE_TOOLS {
        assert!(
            !tool_hint(name).is_empty(),
            "tool_hint missing for core tool {name}"
        );
        assert_ne!(
            tool_category(name),
            "Other",
            "tool_category missing for core tool {name}"
        );
    }
}

#[test]
fn test_tool_doc_fallback_to_hint() {
    assert!(tool_doc("unknown_nonexistent_tool").is_none());
    let tools = vec![
        "memory_save".to_string(),
        "file_read".to_string(),
        "unknown_custom_tool".to_string(),
    ];
    let section = build_tools_section(&tools);
    assert!(
        section.contains("memory_save"),
        "memory_save must appear in tools section"
    );
}

#[test]
fn test_tool_doc_table_lookup_keeps_sensitive_contracts() {
    let shell = tool_doc("shell_exec").expect("shell_exec doc must be in static lookup");
    assert!(shell.contains("~/.captain/secrets.env"));
    assert!(shell.contains("process_start"));

    let capability =
        tool_doc("capability_search").expect("capability_search doc must be in static lookup");
    assert!(capability.contains("unified resolver"));
    assert!(capability.contains("captain_docs"));

    assert!(tool_doc("capability_search_extra").is_none());
}
