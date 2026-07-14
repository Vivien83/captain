use super::*;

#[test]
fn session_recall_in_tool_registry() {
    let tools = builtin_tool_definitions();
    let names: Vec<&str> = tools.iter().map(|t| t.name.as_str()).collect();
    assert!(names.contains(&"session_recall"));
    let def = tools
        .iter()
        .find(|t| t.name == "session_recall")
        .expect("session_recall tool must exist");
    assert!(def.description.contains("CROSS-SESSION"));
    assert!(def.description.contains("SPONTANÉMENT"));
    let required = def.input_schema["required"].as_array().unwrap();
    assert!(required.iter().any(|v| v.as_str() == Some("query")));
}

#[tokio::test]
async fn session_recall_handles_empty_query_param() {
    let res = tool_session_recall(&serde_json::json!({})).await;
    assert!(res.is_err());
    assert!(res.unwrap_err().contains("query"));
}

#[test]
fn workspace_add_in_tool_registry() {
    let tools = builtin_tool_definitions();
    let names: Vec<&str> = tools.iter().map(|t| t.name.as_str()).collect();
    assert!(names.contains(&"workspace_add"));
    let def = tools
        .iter()
        .find(|t| t.name == "workspace_add")
        .expect("workspace_add tool must exist");
    assert!(def.description.contains("ACCÈS"));
    assert!(def.description.contains("SPONTANÉMENT"));
}

#[tokio::test]
async fn workspace_add_rejects_empty_path() {
    let kh: Arc<dyn KernelHandle> = Arc::new(MemSaveStubKernel);
    let input = serde_json::json!({ "path": "   " });
    let res = tool_workspace_add(&input, Some(&kh)).await;
    assert!(res.is_err());
    assert!(res.unwrap_err().contains("empty"));
}

#[tokio::test]
async fn workspace_add_propagates_kernel_error() {
    let kh: Arc<dyn KernelHandle> = Arc::new(MemSaveStubKernel);
    let input = serde_json::json!({ "path": "/tmp/captain_workspace_add_test_dir" });
    let _ = std::fs::create_dir_all("/tmp/captain_workspace_add_test_dir");
    let res = tool_workspace_add(&input, Some(&kh)).await;
    assert!(res.is_err());
    let _ = std::fs::remove_dir_all("/tmp/captain_workspace_add_test_dir");
}
