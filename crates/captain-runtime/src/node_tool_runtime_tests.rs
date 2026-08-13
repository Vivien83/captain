use super::*;
use captain_types::config::{CriticalMode, ExecSecurityMode};
use tempfile::TempDir;

fn permissive_node_policy() -> ExecPolicy {
    ExecPolicy {
        mode: ExecSecurityMode::Full,
        safe_bins: vec!["echo".to_string(), "git".to_string(), "pwd".to_string()],
        allowed_commands: vec!["echo".to_string(), "git".to_string(), "pwd".to_string()],
        critical_mode: CriticalMode::Open,
        ..ExecPolicy::default()
    }
}

fn request<'a>(
    root: &'a Path,
    tool_name: &'a str,
    input: &'a serde_json::Value,
    policy: &'a ExecPolicy,
    approved_action_digest: Option<&'a str>,
) -> LocalNodeToolExecution<'a> {
    LocalNodeToolExecution {
        tool_use_id: "node-tool-test",
        tool_name,
        input,
        workspace_id: "project-main",
        workspace_root: root,
        exec_policy: policy,
        approved_action_digest,
    }
}

#[test]
fn review_exposes_only_the_exact_local_catalog() {
    let policy = permissive_node_policy();
    let read = review_local_node_tool(
        "file_read",
        &serde_json::json!({"path": "README.md"}),
        &policy,
    )
    .unwrap();
    assert_eq!(read.family(), "file");
    assert_eq!(read.effect(), LocalNodeToolEffect::ReadOnly);
    assert!(!read.approval_required());

    for alias in ["read", "Glob", "fs-read"] {
        assert_eq!(
            review_local_node_tool(alias, &serde_json::json!({"path": "README.md"}), &policy)
                .unwrap_err()
                .code(),
            "non_canonical_tool_name"
        );
    }
    assert_eq!(
        review_local_node_tool(
            "web_fetch",
            &serde_json::json!({"url": "https://example.com"}),
            &policy
        )
        .unwrap_err()
        .code(),
        "unsupported_local_tool"
    );
}

#[test]
fn hub_rail_accepts_only_workspace_relative_path_arguments() {
    let policy = permissive_node_policy();
    for input in [
        serde_json::json!({"path": "/Users/private/notes.txt"}),
        serde_json::json!({"path": "C:\\Users\\private\\notes.txt"}),
        serde_json::json!({"path": "../outside.txt"}),
        serde_json::json!({"path": "~/notes.txt"}),
    ] {
        assert_eq!(
            review_local_node_tool("file_read", &input, &policy)
                .unwrap_err()
                .code(),
            "path_policy_violation"
        );
    }

    for command in [
        "cat /etc/passwd",
        "cat C:\\Users\\private\\notes.txt",
        "cat ../outside.txt",
        "cat $HOME/notes.txt",
    ] {
        assert!(!local_node_input_uses_workspace_relative_paths(
            "shell_exec",
            &serde_json::json!({"command": command})
        ));
    }

    assert!(local_node_input_uses_workspace_relative_paths(
        "shell_exec",
        &serde_json::json!({"command": "git status --short"})
    ));
    assert!(local_node_input_uses_workspace_relative_paths(
        "apply_patch",
        &serde_json::json!({
            "patch": "*** Begin Patch\n*** Update File: src/lib.rs\n@@\n-old\n+new\n*** End Patch"
        })
    ));
    assert!(!local_node_input_uses_workspace_relative_paths(
        "apply_patch",
        &serde_json::json!({
            "patch": "*** Begin Patch\n*** Update File: /tmp/private\n@@\n-old\n+new\n*** End Patch"
        })
    ));
    assert!(local_node_input_uses_workspace_relative_paths(
        "apply_patch",
        &serde_json::json!({
            "patch": "*** Begin Patch\n*** Update File: src/old.rs -> src/new.rs\n@@ hunk @@\n-old\n+new\n*** End Patch"
        })
    ));
    assert!(!local_node_input_uses_workspace_relative_paths(
        "apply_patch",
        &serde_json::json!({
            "patch": "*** Begin Patch\n*** Update File: src/old.rs -> /tmp/private\n@@ hunk @@\n-old\n+new\n*** End Patch"
        })
    ));
}

#[test]
fn review_rejects_oversized_or_malformed_batch_input_before_claim() {
    let policy = permissive_node_policy();
    let oversized = serde_json::json!({"path": "large.txt", "content": "x".repeat(1_048_576)});
    assert_eq!(
        review_local_node_tool("file_write", &oversized, &policy)
            .unwrap_err()
            .code(),
        "tool_input_too_large"
    );

    for operations in [
        serde_json::json!([{"action": "read"}]),
        serde_json::json!([{"action": "glob"}]),
        serde_json::json!([{"action": "unknown", "path": "README.md"}]),
    ] {
        let input = serde_json::json!({"operations": operations});
        assert_eq!(
            review_local_node_tool("file_inspect_batch", &input, &policy)
                .unwrap_err()
                .code(),
            "invalid_tool_input"
        );
    }
}

#[test]
fn shell_review_is_conservative_and_keeps_critical_approval_local() {
    let policy = permissive_node_policy();
    let observation = review_local_node_tool(
        "shell_exec",
        &serde_json::json!({"command": "pwd"}),
        &policy,
    )
    .unwrap();
    assert_eq!(observation.effect(), LocalNodeToolEffect::ReadOnly);
    assert!(!observation.approval_required());

    let verification = review_local_node_tool(
        "shell_exec",
        &serde_json::json!({"command": "git status --short"}),
        &policy,
    )
    .unwrap();
    assert_eq!(verification.effect(), LocalNodeToolEffect::LocalMutation);
    assert!(!verification.approval_required());

    let push = review_local_node_tool(
        "shell_exec",
        &serde_json::json!({"command": "git push origin main"}),
        &policy,
    )
    .unwrap();
    assert_eq!(push.effect(), LocalNodeToolEffect::ExternalEffect);
    assert!(push.approval_required());
    assert_eq!(push.risk_level(), RiskLevel::High);
    assert!(!push.approval_summary().contains("origin main"));

    let critical = review_local_node_tool(
        "shell_exec",
        &serde_json::json!({"command": "rm -rf /"}),
        &policy,
    )
    .unwrap_err();
    assert_eq!(critical.code(), "critical_shell_denied");
}

#[tokio::test]
async fn identical_reads_are_isolated_between_workspace_roots() {
    let first = TempDir::new().unwrap();
    let second = TempDir::new().unwrap();
    std::fs::write(first.path().join("README.md"), "first workspace").unwrap();
    std::fs::write(second.path().join("README.md"), "second workspace").unwrap();
    let input = serde_json::json!({"path": "README.md"});
    let policy = permissive_node_policy();

    let first_result =
        execute_local_node_tool(request(first.path(), "file_read", &input, &policy, None))
            .await
            .unwrap();
    let second_result =
        execute_local_node_tool(request(second.path(), "file_read", &input, &policy, None))
            .await
            .unwrap();

    assert_eq!(first_result.content(), "first workspace");
    assert_eq!(second_result.content(), "second workspace");
}

#[tokio::test]
async fn file_tools_reject_traversal_and_redact_local_root() {
    let workspace = TempDir::new().unwrap();
    let outside = TempDir::new().unwrap();
    std::fs::write(outside.path().join("secret.txt"), "outside").unwrap();
    let policy = permissive_node_policy();
    let traversal = serde_json::json!({"path": "../secret.txt"});
    let denied = execute_local_node_tool(request(
        workspace.path(),
        "file_read",
        &traversal,
        &policy,
        None,
    ))
    .await
    .unwrap_err();
    assert_eq!(denied.code(), "path_policy_violation");
    assert!(!denied
        .to_string()
        .contains(outside.path().to_string_lossy().as_ref()));

    let write = serde_json::json!({"path": "nested/result.txt", "content": "done"});
    let written = execute_local_node_tool(request(
        workspace.path(),
        "file_write",
        &write,
        &policy,
        None,
    ))
    .await
    .unwrap();
    assert!(written.succeeded(), "{}", written.content());
    assert!(written.content().contains("workspace://project-main"));
    assert!(!written
        .content()
        .contains(workspace.path().to_string_lossy().as_ref()));
    assert_eq!(
        std::fs::read_to_string(workspace.path().join("nested/result.txt")).unwrap(),
        "done"
    );
}

#[tokio::test]
async fn external_effect_needs_the_exact_content_bound_digest() {
    let workspace = TempDir::new().unwrap();
    let policy = permissive_node_policy();
    let input = serde_json::json!({"command": "git push origin main"});
    let review = review_local_node_tool("shell_exec", &input, &policy).unwrap();

    let missing = execute_local_node_tool(request(
        workspace.path(),
        "shell_exec",
        &input,
        &policy,
        None,
    ))
    .await
    .unwrap_err();
    assert_eq!(missing.code(), "approval_required");

    let wrong = execute_local_node_tool(request(
        workspace.path(),
        "shell_exec",
        &input,
        &policy,
        Some(&"0".repeat(64)),
    ))
    .await
    .unwrap_err();
    assert_eq!(wrong.code(), "approval_digest_mismatch");
    assert_eq!(review.action_digest().len(), 64);

    // `git --version` is intentionally not on the distributed observation
    // classifier's narrow list. It exercises conservative approval without
    // performing an external effect.
    let harmless = serde_json::json!({"command": "git --version"});
    let harmless_review = review_local_node_tool("shell_exec", &harmless, &policy).unwrap();
    assert_eq!(
        harmless_review.effect(),
        LocalNodeToolEffect::ExternalEffect
    );
    let dispatched = execute_local_node_tool(request(
        workspace.path(),
        "shell_exec",
        &harmless,
        &policy,
        Some(harmless_review.action_digest()),
    ))
    .await
    .unwrap();
    assert!(dispatched.succeeded(), "{}", dispatched.content());
    assert!(dispatched.content().contains("git version"));
}

#[tokio::test]
async fn canonical_glob_reaches_glob_instead_of_legacy_file_list_alias() {
    let workspace = TempDir::new().unwrap();
    std::fs::create_dir(workspace.path().join("src")).unwrap();
    std::fs::write(workspace.path().join("src/lib.rs"), "pub fn works() {}").unwrap();
    let policy = permissive_node_policy();
    let input = serde_json::json!({"pattern": "**/*.rs"});
    let result = execute_local_node_tool(request(workspace.path(), "glob", &input, &policy, None))
        .await
        .unwrap();
    assert!(result.succeeded(), "{}", result.content());
    assert!(result.content().contains("src/lib.rs"));
}

#[test]
fn debug_output_never_contains_input_or_workspace_path() {
    let workspace = TempDir::new().unwrap();
    let policy = permissive_node_policy();
    let input = serde_json::json!({"path": "private-name.txt"});
    let execution = request(workspace.path(), "file_read", &input, &policy, None);
    let rendered = format!("{execution:?}");
    assert!(!rendered.contains("private-name.txt"));
    assert!(!rendered.contains(workspace.path().to_string_lossy().as_ref()));
}

#[test]
fn wire_output_virtualizes_paths_redacts_secrets_and_caps_utf8() {
    let workspace = TempDir::new().unwrap();
    let raw = format!(
        "{} path:/srv/private file:///Users/private/notes vscode://file/C:/private password=very-secret-value https://example.com/docs HTTPS://example.com/UPPER\n{}é",
        workspace.path().display(),
        "x".repeat(MAX_LOCAL_NODE_RESULT_BYTES)
    );
    let output = finalize_local_node_output(
        ToolResult {
            tool_use_id: "node-output-test".to_string(),
            content: raw,
            is_error: false,
            transient_content: Vec::new(),
        },
        workspace.path(),
        "project-main",
    )
    .unwrap();

    assert!(output.succeeded());
    assert!(output.capped());
    assert!(output.redacted());
    assert_eq!(output.content().len(), MAX_LOCAL_NODE_RESULT_BYTES);
    assert!(output.total_output_bytes() > output.content().len() as u64);
    assert!(output.content().contains("workspace://project-main"));
    assert!(output.content().contains("<local-path>"));
    assert!(!output.content().contains("/Users/private"));
    assert!(!output.content().contains("C:/private"));
    assert!(output.content().contains("password=[REDACTED]"));
    assert!(output.content().contains("https://example.com/docs"));
    assert!(output.content().contains("HTTPS://example.com/UPPER"));
    assert!(!output
        .content()
        .contains(workspace.path().to_string_lossy().as_ref()));
    assert!(!output.content().contains("very-secret-value"));
    assert!(!format!("{output:?}").contains("very-secret-value"));
}
