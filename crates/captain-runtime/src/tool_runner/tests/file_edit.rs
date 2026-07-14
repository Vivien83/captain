use super::*;

/// Q.3 — `edit_file` is registered in builtin tools (visible to LLM).
#[test]
fn test_edit_file_in_tool_registry() {
    let tools = builtin_tool_definitions();
    let names: Vec<&str> = tools.iter().map(|t| t.name.as_str()).collect();
    assert!(
        names.contains(&"edit_file"),
        "edit_file must be registered, got: {names:?}"
    );
    let def = tools.iter().find(|t| t.name == "edit_file").unwrap();
    let required = def.input_schema["required"].as_array().unwrap();
    let req_names: Vec<&str> = required.iter().map(|v| v.as_str().unwrap()).collect();
    assert!(req_names.contains(&"path"));
    assert!(req_names.contains(&"old_string"));
    assert!(req_names.contains(&"new_string"));
}

/// Q.3 — `tool_edit_file` performs the actual file IO end-to-end.
#[tokio::test]
async fn test_tool_edit_file_integration_simple() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("hello.txt");
    tokio::fs::write(&path, "Hello world\n").await.unwrap();

    let input = serde_json::json!({
        "path": "hello.txt",
        "old_string": "world",
        "new_string": "Captain"
    });
    let result = tool_edit_file(&input, Some(dir.path()), None, None).await;
    let msg = result.expect("edit_file should succeed");
    assert!(
        msg.contains("simple"),
        "expected `simple` strategy, got: {msg}"
    );
    let content = tokio::fs::read_to_string(&path).await.unwrap();
    assert_eq!(content, "Hello Captain\n");
}

/// Q.3 — fallback chain kicks in when whitespace differs.
#[tokio::test]
async fn test_tool_edit_file_integration_whitespace_fallback() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("code.txt");
    tokio::fs::write(&path, "let   x   =   1;\n").await.unwrap();

    let input = serde_json::json!({
        "path": "code.txt",
        "old_string": "let x = 1;",
        "new_string": "let x = 99;"
    });
    let result = tool_edit_file(&input, Some(dir.path()), None, None).await;
    let msg = result.expect("whitespace_normalized should rescue this");
    assert!(
        msg.contains("whitespace_normalized") || msg.contains("simple"),
        "expected whitespace_normalized fallback, got: {msg}"
    );
    let content = tokio::fs::read_to_string(&path).await.unwrap();
    assert!(content.contains("let x = 99;"));
}

/// Q.3 — replace_all path applies multiple substitutions atomically.
#[tokio::test]
async fn test_tool_edit_file_integration_replace_all() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("todo.md");
    tokio::fs::write(&path, "TODO: a\nTODO: b\nTODO: c\n")
        .await
        .unwrap();

    let input = serde_json::json!({
        "path": "todo.md",
        "old_string": "TODO",
        "new_string": "DONE",
        "replace_all": true
    });
    let result = tool_edit_file(&input, Some(dir.path()), None, None).await;
    let msg = result.expect("replace_all should succeed");
    assert!(msg.contains("3 replacements"), "expected 3, got: {msg}");
    let content = tokio::fs::read_to_string(&path).await.unwrap();
    assert!(!content.contains("TODO"));
    assert_eq!(content.matches("DONE").count(), 3);
}

/// Q.4 — `multi_edit` is registered.
#[test]
fn test_multi_edit_in_tool_registry() {
    let tools = builtin_tool_definitions();
    let names: Vec<&str> = tools.iter().map(|t| t.name.as_str()).collect();
    assert!(
        names.contains(&"multi_edit"),
        "multi_edit must be registered, got: {names:?}"
    );
}

/// Q.4 — chained edits applied atomically; final content reflects all.
#[tokio::test]
async fn test_tool_multi_edit_integration_chain_success() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("config.toml");
    tokio::fs::write(&path, "name = \"old\"\nport = 3000\n")
        .await
        .unwrap();

    let input = serde_json::json!({
        "path": "config.toml",
        "edits": [
            { "old_string": "name = \"old\"", "new_string": "name = \"new\"" },
            { "old_string": "port = 3000", "new_string": "port = 4200" }
        ]
    });
    let msg = tool_multi_edit(&input, Some(dir.path()), None, None)
        .await
        .expect("multi_edit chain should succeed");
    assert!(msg.contains("Atomically applied 2 edits"), "got: {msg}");
    let content = tokio::fs::read_to_string(&path).await.unwrap();
    assert_eq!(content, "name = \"new\"\nport = 4200\n");
}

/// Q.4 — atomic guarantee: any failure leaves the file UNTOUCHED.
#[tokio::test]
async fn test_tool_multi_edit_integration_atomic_rollback() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("config.toml");
    let original = "alpha\nbeta\ngamma\n";
    tokio::fs::write(&path, original).await.unwrap();

    // Second edit references a string that doesn't exist → must abort
    // ALL writes, file stays bit-for-bit identical to `original`.
    let input = serde_json::json!({
        "path": "config.toml",
        "edits": [
            { "old_string": "alpha", "new_string": "ALPHA" },
            { "old_string": "ABSENT_NEEDLE", "new_string": "x" }
        ]
    });
    let result = tool_multi_edit(&input, Some(dir.path()), None, None).await;
    assert!(result.is_err(), "expected failure, got: {result:?}");
    let err = result.unwrap_err();
    assert!(
        err.contains("edit[1] failed"),
        "expected idx [1], got: {err}"
    );
    assert!(
        err.contains("Atomic abort"),
        "must signal rollback, got: {err}"
    );
    assert!(err.contains("1 prior edit rolled back"), "got: {err}");

    let content_after = tokio::fs::read_to_string(&path).await.unwrap();
    assert_eq!(
        content_after, original,
        "file must be untouched after atomic rollback"
    );
}

/// Q.4 — empty edits list is rejected (would silently no-op otherwise).
#[tokio::test]
async fn test_tool_multi_edit_rejects_empty_edits() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("x.txt");
    tokio::fs::write(&path, "hi").await.unwrap();
    let input = serde_json::json!({ "path": "x.txt", "edits": [] });
    let r = tool_multi_edit(&input, Some(dir.path()), None, None).await;
    assert!(r.is_err());
    assert!(r.unwrap_err().contains("empty"));
}
