use super::*;

#[test]
fn agent_spawn_child_tools_must_stay_within_parent_scope() {
    let parent = vec!["agent_spawn".to_string(), "file_read".to_string()];
    let child = r#"
name = "reader"

[capabilities]
tools = ["file_read"]
"#;

    assert!(validate_child_agent_tool_scope(child, Some(&parent)).is_ok());
}

#[test]
fn agent_spawn_grants_default_discovery_tools_without_parent_escalation() {
    let parent = vec!["agent_spawn".to_string(), "file_read".to_string()];
    let child = r#"
name = "reader"

[capabilities]
tools = ["file_read", "capability_search"]
"#;

    assert!(validate_child_agent_tool_scope(child, Some(&parent)).is_ok());
}

#[test]
fn agent_spawn_rejects_child_tool_escalation() {
    let parent = vec!["agent_spawn".to_string(), "file_read".to_string()];
    let child = r#"
name = "shell-worker"

[capabilities]
tools = ["file_read", "shell_exec"]
"#;

    let err = validate_child_agent_tool_scope(child, Some(&parent)).unwrap_err();
    assert!(err.contains("shell_exec"));
}

#[test]
fn agent_spawn_rejects_unrestricted_child_from_scoped_parent() {
    let parent = vec!["agent_spawn".to_string(), "file_read".to_string()];
    let child = r#"
name = "unrestricted"
"#;

    let err = validate_child_agent_tool_scope(child, Some(&parent)).unwrap_err();
    assert!(err.contains("explicit non-wildcard"));
}

#[test]
fn unrestricted_parent_still_requires_explicit_child_tools() {
    let child = r#"
name = "unrestricted"
"#;

    let err = validate_child_agent_tool_scope(child, None).unwrap_err();
    assert!(err.contains("explicit non-wildcard"));
}

#[test]
fn agent_spawn_rejects_profile_only_child_manifest() {
    let child = r#"
name = "profile-worker"
profile = "coding"
"#;

    let err = validate_child_agent_tool_scope(child, None).unwrap_err();
    assert!(err.contains("explicit non-wildcard"));
}

#[test]
fn agent_spawn_parse_error_explains_model_table_shape() {
    let child = r#"
name = "veille-technologique"
model = "codex:gpt-5.5"
tool_allowlist = ["web_research_batch"]
"#;

    let err = validate_child_agent_tool_scope(child, None).unwrap_err();
    assert!(err.contains("`model` must be a TOML table"));
    assert!(err.contains("[model]"));
    assert!(err.contains("provider = \"codex\""));
    assert!(!err.contains("codex:gpt-5.5"));
}

#[test]
fn agent_spawn_parse_error_explains_tool_allowlist_shape() {
    let child = r#"
name = "veille-technologique"

[model]
provider = "codex"
model = "gpt-5.5"

[tools]
allow = ["web_search"]
"#;

    let err = validate_child_agent_tool_scope(child, None).unwrap_err();
    assert!(err.contains("`tools` is a map"));
    assert!(err.contains("tool_allowlist = [...]"));
    assert!(err.contains("[capabilities] tools"));
    assert!(!err.contains("web_search\"]"));
}
