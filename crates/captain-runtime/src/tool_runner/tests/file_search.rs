use super::*;

/// Q.1 — `grep` is registered.
#[test]
fn test_grep_in_tool_registry() {
    let tools = builtin_tool_definitions();
    let names: Vec<&str> = tools.iter().map(|t| t.name.as_str()).collect();
    assert!(names.contains(&"grep"), "grep must be registered");
}

/// Q.1 — basic content match returns the file path (default mode).
#[tokio::test]
async fn test_tool_grep_files_with_matches_default() {
    let dir = tempfile::tempdir().unwrap();
    tokio::fs::write(dir.path().join("a.rs"), "fn main() {\n    // TODO\n}\n")
        .await
        .unwrap();
    tokio::fs::write(dir.path().join("b.rs"), "fn other() {}\n")
        .await
        .unwrap();
    tokio::fs::write(dir.path().join("c.txt"), "TODO in text\n")
        .await
        .unwrap();

    let input = serde_json::json!({ "pattern": "TODO" });
    let out = tool_grep(&input, Some(dir.path()), None, None)
        .await
        .unwrap();
    assert!(out.contains("a.rs"), "expected a.rs hit, got: {out}");
    assert!(out.contains("c.txt"), "expected c.txt hit, got: {out}");
    assert!(!out.contains("b.rs"), "b.rs has no TODO, got: {out}");
}

/// Q.1 — `type: rust` filters to .rs only.
#[tokio::test]
async fn test_tool_grep_type_filter() {
    let dir = tempfile::tempdir().unwrap();
    tokio::fs::write(dir.path().join("a.rs"), "TODO\n")
        .await
        .unwrap();
    tokio::fs::write(dir.path().join("c.txt"), "TODO\n")
        .await
        .unwrap();
    let input = serde_json::json!({ "pattern": "TODO", "type": "rust" });
    let out = tool_grep(&input, Some(dir.path()), None, None)
        .await
        .unwrap();
    assert!(out.contains("a.rs"));
    assert!(
        !out.contains("c.txt"),
        "type=rust must skip .txt, got: {out}"
    );
}

/// Q.1 — case-insensitive search.
#[tokio::test]
async fn test_tool_grep_case_insensitive() {
    let dir = tempfile::tempdir().unwrap();
    tokio::fs::write(dir.path().join("a.txt"), "Hello World\n")
        .await
        .unwrap();
    let case_sensitive = tool_grep(
        &serde_json::json!({ "pattern": "hello" }),
        Some(dir.path()),
        None,
        None,
    )
    .await
    .unwrap();
    assert!(
        case_sensitive.contains("No matches"),
        "got: {case_sensitive}"
    );
    let case_insensitive = tool_grep(
        &serde_json::json!({ "pattern": "hello", "-i": true }),
        Some(dir.path()),
        None,
        None,
    )
    .await
    .unwrap();
    assert!(
        case_insensitive.contains("a.txt"),
        "got: {case_insensitive}"
    );
}

/// Q.1 — content mode emits matched lines with line numbers.
#[tokio::test]
async fn test_tool_grep_content_mode_with_line_numbers() {
    let dir = tempfile::tempdir().unwrap();
    tokio::fs::write(
        dir.path().join("x.rs"),
        "use std;\nfn main() {\n    println!(\"hi\");\n}\n",
    )
    .await
    .unwrap();
    let out = tool_grep(
        &serde_json::json!({ "pattern": "println", "output_mode": "content" }),
        Some(dir.path()),
        None,
        None,
    )
    .await
    .unwrap();
    assert!(out.contains("x.rs"));
    assert!(out.contains(":3:"), "expected line number 3, got: {out}");
    assert!(out.contains("println!"));
}

/// Q.1 — count mode returns N per file.
#[tokio::test]
async fn test_tool_grep_count_mode() {
    let dir = tempfile::tempdir().unwrap();
    tokio::fs::write(dir.path().join("a.txt"), "x\nx\nx\ny\n")
        .await
        .unwrap();
    let out = tool_grep(
        &serde_json::json!({ "pattern": "x", "output_mode": "count" }),
        Some(dir.path()),
        None,
        None,
    )
    .await
    .unwrap();
    assert!(out.contains("a.txt:3"), "expected count 3, got: {out}");
}

/// Q.1 — head_limit keeps output bounded and reports truncation.
#[tokio::test]
async fn test_tool_grep_head_limit_truncates_results() {
    let dir = tempfile::tempdir().unwrap();
    tokio::fs::write(dir.path().join("a.txt"), "TODO\n")
        .await
        .unwrap();
    tokio::fs::write(dir.path().join("b.txt"), "TODO\n")
        .await
        .unwrap();
    let out = tool_grep(
        &serde_json::json!({ "pattern": "TODO", "head_limit": 1 }),
        Some(dir.path()),
        None,
        None,
    )
    .await
    .unwrap();
    assert!(
        out.contains("truncated at head_limit=1"),
        "expected truncation note, got: {out}"
    );
}

/// Q.1 — gitignore is respected (no shell out).
#[tokio::test]
async fn test_tool_grep_respects_gitignore() {
    let dir = tempfile::tempdir().unwrap();
    // Real .gitignore so the `ignore` crate honors it
    tokio::fs::write(dir.path().join(".gitignore"), "secret/\n")
        .await
        .unwrap();
    tokio::fs::create_dir(dir.path().join("secret"))
        .await
        .unwrap();
    tokio::fs::write(dir.path().join("secret/leak.txt"), "TOKEN=abc\n")
        .await
        .unwrap();
    tokio::fs::write(dir.path().join("ok.txt"), "TOKEN=visible\n")
        .await
        .unwrap();
    let out = tool_grep(
        &serde_json::json!({ "pattern": "TOKEN" }),
        Some(dir.path()),
        None,
        None,
    )
    .await
    .unwrap();
    assert!(out.contains("ok.txt"), "ok.txt must show, got: {out}");
    assert!(
        !out.contains("leak.txt"),
        "gitignored file must NOT show, got: {out}"
    );
}

/// Q.1 — invalid regex returns a clear error.
#[tokio::test]
async fn test_tool_grep_invalid_regex() {
    let dir = tempfile::tempdir().unwrap();
    let r = tool_grep(
        &serde_json::json!({ "pattern": "(unclosed" }),
        Some(dir.path()),
        None,
        None,
    )
    .await;
    assert!(r.is_err());
    assert!(r.unwrap_err().contains("Invalid regex"));
}

/// Q.2 — `glob` is registered.
#[test]
fn test_glob_in_tool_registry() {
    let tools = builtin_tool_definitions();
    let names: Vec<&str> = tools.iter().map(|t| t.name.as_str()).collect();
    assert!(names.contains(&"glob"));
}

/// Q.2 — basic flat glob `*.rs`.
#[tokio::test]
async fn test_tool_glob_flat_pattern() {
    let dir = tempfile::tempdir().unwrap();
    tokio::fs::write(dir.path().join("a.rs"), "").await.unwrap();
    tokio::fs::write(dir.path().join("b.rs"), "").await.unwrap();
    tokio::fs::write(dir.path().join("c.txt"), "")
        .await
        .unwrap();
    let out = tool_glob(
        &serde_json::json!({ "pattern": "*.rs" }),
        Some(dir.path()),
        None,
        None,
    )
    .await
    .unwrap();
    assert!(out.contains("a.rs"));
    assert!(out.contains("b.rs"));
    assert!(!out.contains("c.txt"));
}

/// Q.2 — recursive `**` glob.
#[tokio::test]
async fn test_tool_glob_recursive_double_star() {
    let dir = tempfile::tempdir().unwrap();
    tokio::fs::create_dir_all(dir.path().join("src/inner"))
        .await
        .unwrap();
    tokio::fs::write(dir.path().join("src/a.rs"), "")
        .await
        .unwrap();
    tokio::fs::write(dir.path().join("src/inner/b.rs"), "")
        .await
        .unwrap();
    tokio::fs::write(dir.path().join("top.rs"), "")
        .await
        .unwrap();
    let out = tool_glob(
        &serde_json::json!({ "pattern": "src/**/*.rs" }),
        Some(dir.path()),
        None,
        None,
    )
    .await
    .unwrap();
    assert!(out.contains("src/a.rs") || out.contains("src\\a.rs"));
    assert!(out.contains("inner") && out.contains("b.rs"));
    assert!(
        !out.contains("top.rs"),
        "top.rs is outside src/, got: {out}"
    );
}

/// Q.2 — gitignore is honored.
#[tokio::test]
async fn test_tool_glob_respects_gitignore() {
    let dir = tempfile::tempdir().unwrap();
    tokio::fs::write(dir.path().join(".gitignore"), "ignored/\n")
        .await
        .unwrap();
    tokio::fs::create_dir(dir.path().join("ignored"))
        .await
        .unwrap();
    tokio::fs::write(dir.path().join("ignored/x.rs"), "")
        .await
        .unwrap();
    tokio::fs::write(dir.path().join("ok.rs"), "")
        .await
        .unwrap();
    let out = tool_glob(
        &serde_json::json!({ "pattern": "**/*.rs" }),
        Some(dir.path()),
        None,
        None,
    )
    .await
    .unwrap();
    assert!(out.contains("ok.rs"));
    assert!(
        !out.contains("x.rs"),
        "ignored file must not appear, got: {out}"
    );
}

/// Q.2 — sort by mtime descending.
#[tokio::test]
async fn test_tool_glob_sort_by_mtime_desc() {
    let dir = tempfile::tempdir().unwrap();
    tokio::fs::write(dir.path().join("old.rs"), "")
        .await
        .unwrap();
    // Sleep a hair so mtime differs reliably even on coarse filesystems.
    std::thread::sleep(std::time::Duration::from_millis(20));
    tokio::fs::write(dir.path().join("new.rs"), "")
        .await
        .unwrap();
    let out = tool_glob(
        &serde_json::json!({ "pattern": "*.rs" }),
        Some(dir.path()),
        None,
        None,
    )
    .await
    .unwrap();
    let new_pos = out.find("new.rs").unwrap();
    let old_pos = out.find("old.rs").unwrap();
    assert!(
        new_pos < old_pos,
        "newer file should come first, got order: {out}"
    );
}

/// Q.2 — head_limit keeps glob output bounded and reports total matches.
#[tokio::test]
async fn test_tool_glob_head_limit_truncates_results() {
    let dir = tempfile::tempdir().unwrap();
    tokio::fs::write(dir.path().join("a.rs"), "").await.unwrap();
    tokio::fs::write(dir.path().join("b.rs"), "").await.unwrap();
    let out = tool_glob(
        &serde_json::json!({ "pattern": "*.rs", "head_limit": 1 }),
        Some(dir.path()),
        None,
        None,
    )
    .await
    .unwrap();
    assert!(
        out.contains("truncated at head_limit=1"),
        "expected truncation note, got: {out}"
    );
    assert!(
        out.contains("2 total matches"),
        "expected total match count, got: {out}"
    );
}

/// Q.2 — invalid glob gives a clear error.
#[tokio::test]
async fn test_tool_glob_invalid_pattern() {
    let dir = tempfile::tempdir().unwrap();
    let r = tool_glob(
        &serde_json::json!({ "pattern": "[unclosed" }),
        Some(dir.path()),
        None,
        None,
    )
    .await;
    assert!(r.is_err());
    assert!(r.unwrap_err().contains("Invalid glob"));
}

/// Q.2 — no match returns a friendly Ok message.
#[tokio::test]
async fn test_tool_glob_no_match_returns_ok_message() {
    let dir = tempfile::tempdir().unwrap();
    let out = tool_glob(
        &serde_json::json!({ "pattern": "*.absent" }),
        Some(dir.path()),
        None,
        None,
    )
    .await
    .unwrap();
    assert!(out.contains("No files matching"));
}

#[tokio::test]
async fn test_tool_file_inspect_batch_stop_on_error_stops_after_failure() {
    let dir = tempfile::tempdir().unwrap();
    tokio::fs::write(dir.path().join("ok.rs"), "")
        .await
        .unwrap();
    let out = tool_file_inspect_batch(
        &serde_json::json!({
            "stop_on_error": true,
            "operations": [
                { "action": "bad_action" },
                { "action": "glob", "pattern": "*.rs" }
            ]
        }),
        Some(dir.path()),
        None,
        None,
    )
    .await
    .unwrap();
    let body: serde_json::Value = serde_json::from_str(&out).unwrap();

    assert_eq!(body["success"].as_bool(), Some(false));
    assert_eq!(body["operations_executed"].as_u64(), Some(1));
    assert_eq!(body["results"][0]["action"].as_str(), Some("bad_action"));
    assert_eq!(body["results"][0]["success"].as_bool(), Some(false));
}
