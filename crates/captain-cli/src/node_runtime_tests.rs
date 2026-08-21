use super::*;
use captain_node::{
    AuthorizedNodeRun, NodeExecutionAuthorization, NodeExecutionPolicy, NodeReviewedTool,
    NodeRunCancellation, NodeToolDriver, NodeWorkspaceBinding,
};
use captain_types::config::{CriticalMode, ExecPolicy, ExecSecurityMode};
use captain_wire::{DeviceGrant, RunEffect, RunLease};
use std::sync::Arc;

fn exec_policy() -> ExecPolicy {
    ExecPolicy {
        mode: ExecSecurityMode::Full,
        safe_bins: vec!["git".to_string(), "pwd".to_string()],
        allowed_commands: vec!["git".to_string(), "pwd".to_string()],
        critical_mode: CriticalMode::Open,
        ..ExecPolicy::default()
    }
}

fn lease(tool_name: &str, input: serde_json::Value, effect: RunEffect) -> RunLease {
    RunLease {
        run_id: format!("run-{tool_name}"),
        attempt: 1,
        idempotency_key: format!("idem-{tool_name}"),
        workspace_id: "project-main".to_string(),
        tool_name: tool_name.to_string(),
        input,
        effect,
        lease_expires_at_ms: i64::MAX,
    }
}

fn authorize(
    root: &std::path::Path,
    lease: &RunLease,
    reviewed: &NodeReviewedTool,
) -> AuthorizedNodeRun {
    let policy = NodeExecutionPolicy::new(
        DeviceGrant {
            workspace_ids: vec!["project-main".to_string()],
            tool_families: vec!["file".to_string(), "shell-process".to_string()],
            allow_mutation: true,
        },
        [NodeWorkspaceBinding::new("project-main", root, false).unwrap()],
    )
    .unwrap();
    match policy.authorize(lease, reviewed) {
        NodeExecutionAuthorization::Authorized(run) => run,
        NodeExecutionAuthorization::Rejected(rejection) => {
            panic!("unexpected local rejection: {}", rejection.code)
        }
    }
}

#[test]
fn runtime_review_maps_exact_tool_family_effect_and_digest() {
    let driver = CliNodeToolDriver::new(exec_policy());
    let read = lease(
        "file_read",
        serde_json::json!({"path": "README.md"}),
        RunEffect::ReadOnly,
    );
    let reviewed = driver.review(&read).unwrap();
    assert_eq!(reviewed.reviewed().tool_name, "file_read");
    assert_eq!(reviewed.reviewed().family, "file");
    assert_eq!(reviewed.reviewed().effect, RunEffect::ReadOnly);
    assert_eq!(reviewed.action_digest().len(), 64);
    assert!(!reviewed.approval_required());

    let external = lease(
        "shell_exec",
        serde_json::json!({"command": "git --version"}),
        RunEffect::ExternalEffect,
    );
    let external_review = driver.review(&external).unwrap();
    assert_eq!(external_review.reviewed().family, "shell-process");
    assert!(external_review.approval_required());
    assert!(!format!("{external_review:?}").contains("git --version"));
}

#[tokio::test]
async fn real_runtime_file_execution_is_virtualized_redacted_and_uncached() {
    let workspace = tempfile::tempdir().unwrap();
    let driver = Arc::new(CliNodeToolDriver::new(exec_policy()));
    let write_lease = lease(
        "file_write",
        serde_json::json!({
            "path": "nested/result.txt",
            "content": "initial"
        }),
        RunEffect::LocalMutation,
    );
    let write_review = driver.review(&write_lease).unwrap();
    let write_run = authorize(workspace.path(), &write_lease, write_review.reviewed());
    let written = Arc::clone(&driver)
        .execute(write_run, None, NodeRunCancellation::default())
        .await;
    assert!(written.succeeded(), "{}", written.content());
    assert!(written.content().contains("workspace://project-main"));
    assert!(!written
        .content()
        .contains(workspace.path().to_string_lossy().as_ref()));
    std::fs::write(
        workspace.path().join("nested/result.txt"),
        "password=private-node-value",
    )
    .unwrap();

    let read_lease = lease(
        "file_read",
        serde_json::json!({"path": "nested/result.txt"}),
        RunEffect::ReadOnly,
    );
    let read_review = driver.review(&read_lease).unwrap();
    let read_run = authorize(workspace.path(), &read_lease, read_review.reviewed());
    let read = Arc::clone(&driver)
        .execute(read_run, None, NodeRunCancellation::default())
        .await;
    assert!(read.succeeded());
    assert!(read.redacted());
    assert!(read.content().contains("password=[REDACTED]"));
    assert!(!read.content().contains("private-node-value"));
}

#[tokio::test]
async fn external_runtime_execution_requires_the_exact_approved_digest() {
    let workspace = tempfile::tempdir().unwrap();
    let driver = Arc::new(CliNodeToolDriver::new(exec_policy()));
    let lease = lease(
        "shell_exec",
        serde_json::json!({"command": "git --version"}),
        RunEffect::ExternalEffect,
    );
    let review = driver.review(&lease).unwrap();
    let wrong_run = authorize(workspace.path(), &lease, review.reviewed());
    let wrong = Arc::clone(&driver)
        .execute(
            wrong_run,
            Some("0".repeat(64)),
            NodeRunCancellation::default(),
        )
        .await;
    assert!(!wrong.succeeded());
    assert!(wrong.content().contains("approval_digest_mismatch"));

    let exact_run = authorize(workspace.path(), &lease, review.reviewed());
    let exact = Arc::clone(&driver)
        .execute(
            exact_run,
            Some(review.action_digest().to_string()),
            NodeRunCancellation::default(),
        )
        .await;
    assert!(exact.succeeded(), "{}", exact.content());
    assert!(exact.content().contains("git version"));
}

#[test]
fn driver_debug_does_not_expose_command_allowlist() {
    let driver = CliNodeToolDriver::new(exec_policy());
    let rendered = format!("{driver:?}");
    assert!(!rendered.contains("git"));
    assert!(rendered.contains("allowlist_entries"));
}
