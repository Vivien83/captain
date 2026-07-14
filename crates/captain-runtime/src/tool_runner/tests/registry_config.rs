use super::*;

#[test]
fn test_builtin_tool_definitions() {
    let tools = builtin_tool_definitions();
    assert!(
        tools.len() >= 39,
        "Expected at least 39 tools, got {}",
        tools.len()
    );
    let names: Vec<&str> = tools.iter().map(|t| t.name.as_str()).collect();
    assert!(names.contains(&"file_inspect_batch"));
    assert!(names.contains(&"file_read"));
    assert!(names.contains(&"shell_exec"));
    assert!(names.contains(&"agent_send"));
    assert!(names.contains(&"agent_spawn"));
    assert!(names.contains(&"agent_list"));
    assert!(names.contains(&"agent_kill"));
    assert!(names.contains(&"memory_store"));
    assert!(names.contains(&"memory_recall"));
    assert!(names.contains(&"agent_find"));
    assert!(names.contains(&"task_post"));
    assert!(names.contains(&"task_claim"));
    assert!(names.contains(&"task_complete"));
    assert!(names.contains(&"task_list"));
    assert!(names.contains(&"event_publish"));
    assert!(names.contains(&"schedule_create"));
    assert!(names.contains(&"schedule_list"));
    assert!(names.contains(&"schedule_delete"));
    assert!(names.contains(&"image_analyze"));
    assert!(names.contains(&"location_get"));
    assert!(names.contains(&"system_time"));
    assert!(names.contains(&"codex_tool_probe"));
    assert!(names.contains(&"web_research_batch"));
    assert!(names.contains(&"web_download"));
    assert!(names.contains(&"ssh_health_check"));
    assert!(names.contains(&"document_pipeline"));
    assert!(names.contains(&"document_extract"));
    assert!(names.contains(&"memory_context_batch"));
    assert!(names.contains(&"web_credentials_update"));
    assert!(names.contains(&"browser_batch"));
    assert!(names.contains(&"browser_navigate"));
    assert!(names.contains(&"browser_click"));
    assert!(names.contains(&"browser_type"));
    assert!(names.contains(&"browser_keys"));
    assert!(names.contains(&"browser_select"));
    assert!(names.contains(&"browser_hover"));
    assert!(names.contains(&"browser_screenshot"));
    assert!(names.contains(&"browser_read_page"));
    assert!(names.contains(&"browser_close"));
    assert!(names.contains(&"browser_scroll"));
    assert!(names.contains(&"browser_wait"));
    assert!(names.contains(&"browser_run_js"));
    assert!(names.contains(&"browser_back"));
    assert!(names.contains(&"browser_status"));
    assert!(names.contains(&"browser_network_log"));
    assert!(names.contains(&"browser_observe"));
    assert!(names.contains(&"browser_diagnostics"));
    assert!(names.contains(&"media_describe"));
    assert!(names.contains(&"media_transcribe"));
    assert!(names.contains(&"video_analyze"));
    assert!(names.contains(&"image_generate"));
    assert!(names.contains(&"media_pipeline"));
    assert!(names.contains(&"cron_create"));
    assert!(names.contains(&"cron_list"));
    assert!(names.contains(&"cron_update"));
    assert!(names.contains(&"cron_cancel"));
    assert!(names.contains(&"channel_delivery_batch"));
    assert!(names.contains(&"channel_send"));
    assert!(names.contains(&"hand_list"));
    assert!(names.contains(&"hand_activate"));
    assert!(names.contains(&"hand_status"));
    assert!(names.contains(&"hand_deactivate"));
    assert!(names.contains(&"text_to_speech"));
    assert!(names.contains(&"speech_to_text"));
    assert!(names.contains(&"docker_exec"));
    assert!(names.contains(&"canvas_present"));
}

/// R.3.1 — `config_setup` tool is registered with the expected schema.
#[test]
fn r31_config_setup_in_tool_registry() {
    let tools = builtin_tool_definitions();
    let setup = tools
        .iter()
        .find(|t| t.name == "config_setup")
        .expect("config_setup must be registered in builtin_tool_definitions");

    let required = setup.input_schema["required"]
        .as_array()
        .expect("required must be an array");
    let required: Vec<&str> = required.iter().filter_map(|v| v.as_str()).collect();
    assert!(required.contains(&"integration"));
    assert!(required.contains(&"credentials"));

    let enum_values = setup.input_schema["properties"]["integration"]["enum"]
        .as_array()
        .expect("integration must have an enum");
    let enum_values: Vec<&str> = enum_values.iter().filter_map(|v| v.as_str()).collect();
    assert!(enum_values.contains(&"telegram"));
    assert!(enum_values.contains(&"tts_elevenlabs"));
    assert!(enum_values.contains(&"tts_openai"));
    assert!(enum_values.contains(&"stt_whisper"));

    assert!(setup.description.contains("atomique") || setup.description.contains("AUTO-INSTALL"));
    assert!(setup.description.contains("rollback") || setup.description.contains("backup"));
}

#[test]
fn mcp_typed_tools_are_registered() {
    let names: Vec<String> = builtin_tool_definitions()
        .into_iter()
        .map(|t| t.name)
        .collect();
    for name in [
        "mcp_catalog_search",
        "mcp_integration_install",
        "mcp_status",
    ] {
        assert!(names.contains(&name.to_string()), "{name} missing");
    }
}

#[test]
fn test_collaboration_tool_schemas() {
    let tools = builtin_tool_definitions();
    let collab_tools = [
        "agent_find",
        "task_post",
        "task_claim",
        "task_complete",
        "task_list",
        "event_publish",
    ];
    for name in &collab_tools {
        let tool = tools
            .iter()
            .find(|t| t.name == *name)
            .unwrap_or_else(|| panic!("Tool '{}' not found", name));
        assert!(
            tool.input_schema.is_object(),
            "Tool '{}' schema should be an object",
            name
        );
        assert_eq!(
            tool.input_schema["type"], "object",
            "Tool '{}' should have type=object",
            name
        );
    }
}

#[test]
fn web_credentials_update_tool_schema_supports_generation() {
    let tools = builtin_tool_definitions();
    let tool = tools
        .iter()
        .find(|t| t.name == "web_credentials_update")
        .expect("web_credentials_update must be registered");
    let props = &tool.input_schema["properties"];
    assert!(props.get("username").is_some());
    assert!(props.get("password").is_some());
    assert!(props.get("generate_password").is_some());
    assert!(props.get("session_ttl_hours").is_some());
    assert!(tool.description.contains("config.toml"));
    assert!(tool.description.contains("hot-reload"));
}

#[test]
fn write_web_credentials_config_hashes_and_preserves_config() {
    let dir = tempfile::TempDir::new().unwrap();
    let config_path = dir.path().join("config.toml");
    std::fs::write(
        &config_path,
        r#"
api_key = "captain_api_test"
log_level = "info"

[auth]
enabled = false
username = "admin"
password_hash = ""
session_ttl_hours = 72

[web_terminal]
enabled = true
"#,
    )
    .unwrap();

    let hash = hash_web_password("new-password-123");
    let backup =
        write_web_credentials_config(dir.path(), Some("owner"), Some(&hash), Some(24)).unwrap();
    assert!(backup.exists());
    let updated = std::fs::read_to_string(&config_path).unwrap();
    assert!(updated.contains("api_key = \"captain_api_test\""));
    assert!(updated.contains("log_level = \"info\""));
    assert!(updated.contains("enabled = true"));
    assert!(updated.contains("username = \"owner\""));
    assert!(updated.contains(&format!("password_hash = \"{hash}\"")));
    assert!(updated.contains("session_ttl_hours = 24"));
    assert!(!updated.contains("new-password-123"));
}
