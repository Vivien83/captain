use crate::node_tool_runtime_legacy as legacy;
use captain_node_tools::node_tool_runtime as extracted;
use captain_types::config::{CriticalMode, ExecPolicy, ExecSecurityMode};
use std::path::Path;

fn policy() -> ExecPolicy {
    ExecPolicy {
        mode: ExecSecurityMode::Full,
        safe_bins: vec![
            "echo".to_string(),
            "git".to_string(),
            "printf".to_string(),
            "pwd".to_string(),
        ],
        allowed_commands: vec![
            "echo".to_string(),
            "git".to_string(),
            "printf".to_string(),
            "pwd".to_string(),
        ],
        critical_mode: CriticalMode::Open,
        ..ExecPolicy::default()
    }
}

fn effect_name_legacy(effect: legacy::LocalNodeToolEffect) -> &'static str {
    match effect {
        legacy::LocalNodeToolEffect::ReadOnly => "read_only",
        legacy::LocalNodeToolEffect::LocalMutation => "local_mutation",
        legacy::LocalNodeToolEffect::ExternalEffect => "external_effect",
    }
}

fn effect_name_extracted(effect: extracted::LocalNodeToolEffect) -> &'static str {
    match effect {
        extracted::LocalNodeToolEffect::ReadOnly => "read_only",
        extracted::LocalNodeToolEffect::LocalMutation => "local_mutation",
        extracted::LocalNodeToolEffect::ExternalEffect => "external_effect",
    }
}

#[test]
fn review_contract_matches_the_legacy_runtime_for_the_complete_catalog() {
    let policy = policy();
    let cases = [
        ("file_read", serde_json::json!({"path": "README.md"})),
        (
            "file_write",
            serde_json::json!({"path": "note.txt", "content": "safe"}),
        ),
        ("file_list", serde_json::json!({"path": "."})),
        ("glob", serde_json::json!({"pattern": "**/*.rs"})),
        ("grep", serde_json::json!({"pattern": "Captain"})),
        (
            "edit_file",
            serde_json::json!({"path": "note.txt", "old_string": "a", "new_string": "b"}),
        ),
        (
            "multi_edit",
            serde_json::json!({"path": "note.txt", "edits": [{"old_string": "a", "new_string": "b"}]}),
        ),
        (
            "apply_patch",
            serde_json::json!({"patch": "*** Begin Patch\n*** Add File: note.txt\n+safe\n*** End Patch"}),
        ),
        (
            "file_inspect_batch",
            serde_json::json!({"operations": [{"action": "read", "path": "README.md"}]}),
        ),
        ("shell_exec", serde_json::json!({"command": "pwd"})),
        (
            "shell_exec",
            serde_json::json!({"command": "git status --short"}),
        ),
        (
            "shell_exec",
            serde_json::json!({"command": "git push origin main"}),
        ),
    ];

    for (tool, input) in cases {
        let legacy = legacy::review_local_node_tool(tool, &input, &policy).unwrap();
        let extracted = extracted::review_local_node_tool(tool, &input, &policy).unwrap();
        assert_eq!(legacy.tool_name(), extracted.tool_name(), "{tool}");
        assert_eq!(legacy.family(), extracted.family(), "{tool}");
        assert_eq!(
            effect_name_legacy(legacy.effect()),
            effect_name_extracted(extracted.effect()),
            "{tool}"
        );
        assert_eq!(legacy.action_digest(), extracted.action_digest(), "{tool}");
        assert_eq!(
            legacy.approval_required(),
            extracted.approval_required(),
            "{tool}"
        );
        assert_eq!(legacy.risk_level(), extracted.risk_level(), "{tool}");
        assert_eq!(
            legacy.approval_summary(),
            extracted.approval_summary(),
            "{tool}"
        );
    }
}

#[test]
fn rejection_codes_match_for_security_boundaries() {
    let policy = policy();
    let cases = [
        ("file_read", serde_json::json!({"path": "../outside"})),
        ("file_read", serde_json::json!({"path": "/etc/passwd"})),
        ("shell_exec", serde_json::json!({"command": "rm -rf /"})),
        (
            "shell_exec",
            serde_json::json!({"command": "curl https://example.com"}),
        ),
        (
            "web_fetch",
            serde_json::json!({"url": "https://example.com"}),
        ),
        ("read", serde_json::json!({"path": "README.md"})),
    ];
    for (tool, input) in cases {
        let legacy = legacy::review_local_node_tool(tool, &input, &policy).unwrap_err();
        let extracted = extracted::review_local_node_tool(tool, &input, &policy).unwrap_err();
        assert_eq!(legacy.code(), extracted.code(), "{tool}: {input}");
        assert_eq!(
            legacy.is_retryable(),
            extracted.is_retryable(),
            "{tool}: {input}"
        );
    }
}

fn seed(root: &Path) {
    std::fs::create_dir_all(root.join("src")).unwrap();
    std::fs::write(root.join("README.md"), "Captain\nsecond line\n").unwrap();
    std::fs::write(root.join("src/lib.rs"), "fn old() {}\n").unwrap();
    std::fs::write(root.join("edit.txt"), "alpha\nbeta\ngamma\n").unwrap();
}

async fn execute_legacy(
    root: &Path,
    tool: &str,
    input: &serde_json::Value,
) -> legacy::LocalNodeToolOutput {
    legacy::execute_local_node_tool(legacy::LocalNodeToolExecution {
        tool_use_id: "legacy-parity",
        tool_name: tool,
        input,
        workspace_id: "project-main",
        workspace_root: root,
        exec_policy: &policy(),
        approved_action_digest: None,
    })
    .await
    .unwrap()
}

async fn execute_extracted(
    root: &Path,
    tool: &str,
    input: &serde_json::Value,
) -> extracted::LocalNodeToolOutput {
    extracted::execute_local_node_tool(extracted::LocalNodeToolExecution {
        tool_use_id: "extracted-parity",
        tool_name: tool,
        input,
        workspace_id: "project-main",
        workspace_root: root,
        exec_policy: &policy(),
        approved_action_digest: None,
    })
    .await
    .unwrap()
}

#[tokio::test]
async fn deterministic_file_execution_matches_the_legacy_runtime() {
    let cases = [
        ("file_read", serde_json::json!({"path": "README.md"})),
        ("file_list", serde_json::json!({"path": "."})),
        ("glob", serde_json::json!({"pattern": "**/*.rs"})),
        (
            "grep",
            serde_json::json!({"pattern": "Captain", "path": ".", "output_mode": "content"}),
        ),
        (
            "file_write",
            serde_json::json!({"path": "nested/new.txt", "content": "safe value"}),
        ),
        (
            "edit_file",
            serde_json::json!({"path": "edit.txt", "old_string": "beta", "new_string": "BETA"}),
        ),
        (
            "multi_edit",
            serde_json::json!({"path": "edit.txt", "edits": [{"old_string": "alpha", "new_string": "ALPHA"}, {"old_string": "gamma", "new_string": "GAMMA"}]}),
        ),
        (
            "apply_patch",
            serde_json::json!({"patch": "*** Begin Patch\n*** Update File: src/lib.rs\n@@ hunk @@\n-fn old() {}\n+fn new() {}\n*** End Patch"}),
        ),
        (
            "file_inspect_batch",
            serde_json::json!({"operations": [{"action": "read", "path": "README.md"}, {"action": "glob", "pattern": "**/*.rs"}]}),
        ),
    ];

    for (tool, input) in cases {
        let legacy_root = tempfile::tempdir().unwrap();
        let extracted_root = tempfile::tempdir().unwrap();
        seed(legacy_root.path());
        seed(extracted_root.path());
        let legacy = execute_legacy(legacy_root.path(), tool, &input).await;
        let extracted = execute_extracted(extracted_root.path(), tool, &input).await;
        assert_eq!(legacy.succeeded(), extracted.succeeded(), "{tool}");
        assert_eq!(legacy.content(), extracted.content(), "{tool}");
        assert_eq!(legacy.capped(), extracted.capped(), "{tool}");
        assert_eq!(legacy.redacted(), extracted.redacted(), "{tool}");
    }
}

#[tokio::test]
async fn shell_output_and_redaction_match_the_legacy_runtime() {
    let legacy_root = tempfile::tempdir().unwrap();
    let extracted_root = tempfile::tempdir().unwrap();
    let input = serde_json::json!({"command": "printf password=abc12345"});
    let legacy = execute_legacy(legacy_root.path(), "shell_exec", &input).await;
    let extracted = execute_extracted(extracted_root.path(), "shell_exec", &input).await;
    assert_eq!(legacy.succeeded(), extracted.succeeded());
    assert_eq!(legacy.content(), extracted.content());
    assert_eq!(legacy.redacted(), extracted.redacted());
    assert!(!extracted.content().contains("abc12345"));
}
