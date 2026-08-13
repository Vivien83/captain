use super::*;
use tempfile::TempDir;

fn lease(effect: RunEffect) -> RunLease {
    RunLease {
        run_id: "run-policy-1".to_string(),
        attempt: 1,
        idempotency_key: "idem-policy-1".to_string(),
        workspace_id: "workspace-main".to_string(),
        tool_name: "file_read".to_string(),
        input: serde_json::json!({"path": "README.md"}),
        effect,
        lease_expires_at_ms: 10_000,
    }
}

fn policy(root: &Path, allow_mutation: bool, read_only: bool) -> NodeExecutionPolicy {
    NodeExecutionPolicy::new(
        DeviceGrant {
            workspace_ids: vec!["workspace-main".to_string()],
            tool_families: vec!["file".to_string()],
            allow_mutation,
        },
        [NodeWorkspaceBinding::new("workspace-main", root, read_only).unwrap()],
    )
    .unwrap()
}

fn reviewed(tool_name: &str, effect: RunEffect) -> NodeReviewedTool {
    NodeReviewedTool::new(tool_name, "file", effect).unwrap()
}

#[test]
fn exact_read_only_offer_gets_local_workspace_scope() {
    let root = TempDir::new().unwrap();
    let decision = policy(root.path(), false, true).authorize(
        &lease(RunEffect::ReadOnly),
        &reviewed("file_read", RunEffect::ReadOnly),
    );
    let NodeExecutionAuthorization::Authorized(authorized) = decision else {
        panic!("expected authorization");
    };
    assert_eq!(
        authorized.workspace_root(),
        root.path().canonicalize().unwrap()
    );
    assert_eq!(authorized.family(), "file");
    assert!(!format!("{authorized:?}").contains(root.path().to_str().unwrap()));
}

#[test]
fn hub_cannot_change_reviewed_tool_or_effect() {
    let root = TempDir::new().unwrap();
    let policy = policy(root.path(), true, false);
    let wrong_tool = policy.authorize(
        &lease(RunEffect::ReadOnly),
        &reviewed("file_list", RunEffect::ReadOnly),
    );
    let wrong_effect = policy.authorize(
        &lease(RunEffect::ReadOnly),
        &reviewed("file_read", RunEffect::LocalMutation),
    );
    assert_rejected(wrong_tool, "tool_contract_mismatch");
    assert_rejected(wrong_effect, "effect_contract_mismatch");
}

#[test]
fn workspace_and_family_grants_are_both_required() {
    let root = TempDir::new().unwrap();
    let no_workspace = NodeExecutionPolicy::new(
        DeviceGrant {
            workspace_ids: vec![],
            tool_families: vec!["file".to_string()],
            allow_mutation: false,
        },
        [NodeWorkspaceBinding::new("workspace-main", root.path(), false).unwrap()],
    )
    .unwrap();
    assert_rejected(
        no_workspace.authorize(
            &lease(RunEffect::ReadOnly),
            &reviewed("file_read", RunEffect::ReadOnly),
        ),
        "workspace_not_granted",
    );

    let no_family = NodeExecutionPolicy::new(
        DeviceGrant {
            workspace_ids: vec!["workspace-main".to_string()],
            tool_families: vec![],
            allow_mutation: false,
        },
        [NodeWorkspaceBinding::new("workspace-main", root.path(), false).unwrap()],
    )
    .unwrap();
    assert_rejected(
        no_family.authorize(
            &lease(RunEffect::ReadOnly),
            &reviewed("file_read", RunEffect::ReadOnly),
        ),
        "tool_family_not_granted",
    );
}

#[test]
fn mutation_requires_both_device_and_workspace_authority() {
    let root = TempDir::new().unwrap();
    let mutating = RunLease {
        tool_name: "file_write".to_string(),
        ..lease(RunEffect::LocalMutation)
    };
    assert_rejected(
        policy(root.path(), false, false)
            .authorize(&mutating, &reviewed("file_write", RunEffect::LocalMutation)),
        "mutation_not_granted",
    );
    assert_rejected(
        policy(root.path(), true, true)
            .authorize(&mutating, &reviewed("file_write", RunEffect::LocalMutation)),
        "mutation_not_granted",
    );
    assert!(matches!(
        policy(root.path(), true, false)
            .authorize(&mutating, &reviewed("file_write", RunEffect::LocalMutation),),
        NodeExecutionAuthorization::Authorized(_)
    ));
}

#[test]
fn vanished_workspace_fails_closed_without_disclosing_path() {
    let parent = TempDir::new().unwrap();
    let root = parent.path().join("workspace-secret-name");
    fs::create_dir(&root).unwrap();
    let policy = policy(&root, false, false);
    fs::remove_dir(&root).unwrap();
    let decision = policy.authorize(
        &lease(RunEffect::ReadOnly),
        &reviewed("file_read", RunEffect::ReadOnly),
    );
    let rendered = format!("{decision:?}");
    assert_rejected(decision, "workspace_unavailable");
    assert!(!rendered.contains("workspace-secret-name"));
}

#[test]
fn duplicate_workspace_bindings_are_rejected() {
    let first = TempDir::new().unwrap();
    let second = TempDir::new().unwrap();
    let result = NodeExecutionPolicy::new(
        DeviceGrant::default(),
        [
            NodeWorkspaceBinding::new("same", first.path(), false).unwrap(),
            NodeWorkspaceBinding::new("same", second.path(), false).unwrap(),
        ],
    );
    assert!(matches!(
        result,
        Err(NodeExecutionPolicyError::DuplicateWorkspace)
    ));
}

#[test]
fn debug_surfaces_never_expose_local_root() {
    let root = TempDir::new().unwrap();
    let binding = NodeWorkspaceBinding::new("workspace-main", root.path(), false).unwrap();
    let policy = policy(root.path(), false, false);
    let secret = root.path().to_string_lossy();
    assert!(!format!("{binding:?}").contains(secret.as_ref()));
    assert!(!format!("{policy:?}").contains(secret.as_ref()));
}

fn assert_rejected(decision: NodeExecutionAuthorization, code: &str) {
    let NodeExecutionAuthorization::Rejected(rejection) = decision else {
        panic!("expected rejection");
    };
    assert_eq!(rejection.code, code);
    rejection.validate().unwrap();
    assert!(rejection.path_policy_applied);
}
