use crate::execution_targets::{ExecutionTargetScope, ExecutionTargetStoreError};
use crate::project::NewProject;
use crate::MemorySubstrate;
use captain_types::agent::AgentId;
use captain_wire::ExecutionTarget;

#[test]
fn bindings_survive_reopen_and_scopes_remain_independent() {
    let temp = tempfile::tempdir().unwrap();
    let database = temp.path().join("memory.db");
    let node = ExecutionTarget::Node {
        device_id: "node-00000000-0000-0000-0000-000000000001".to_string(),
        workspace_id: "workspace-main".to_string(),
    };
    {
        let memory = MemorySubstrate::open(&database, 0.01).unwrap();
        memory
            .execution_targets()
            .set(ExecutionTargetScope::Session, "session-1", &node, 10)
            .unwrap();
        memory
            .execution_targets()
            .set(
                ExecutionTargetScope::Project,
                "project-1",
                &ExecutionTarget::Hub,
                11,
            )
            .unwrap();
    }

    let memory = MemorySubstrate::open(&database, 0.01).unwrap();
    let session = memory
        .execution_targets()
        .get(ExecutionTargetScope::Session, "session-1")
        .unwrap()
        .unwrap();
    assert_eq!(session.target, node);
    assert_eq!(session.updated_at_ms, 10);
    let project = memory
        .execution_targets()
        .get(ExecutionTargetScope::Project, "project-1")
        .unwrap()
        .unwrap();
    assert_eq!(project.target, ExecutionTarget::Hub);
}

#[test]
fn updates_are_atomic_and_delete_is_idempotent() {
    let memory = MemorySubstrate::open_in_memory(0.01).unwrap();
    let store = memory.execution_targets();
    store
        .set(
            ExecutionTargetScope::Session,
            "session-1",
            &ExecutionTarget::Hub,
            1,
        )
        .unwrap();
    let updated = store
        .set(
            ExecutionTargetScope::Session,
            "session-1",
            &ExecutionTarget::Auto,
            2,
        )
        .unwrap();
    assert_eq!(updated.target, ExecutionTarget::Auto);
    assert_eq!(updated.updated_at_ms, 2);
    assert!(store
        .delete(ExecutionTargetScope::Session, "session-1")
        .unwrap());
    assert!(!store
        .delete(ExecutionTargetScope::Session, "session-1")
        .unwrap());
}

#[test]
fn invalid_identifiers_targets_and_timestamps_fail_before_sql() {
    let memory = MemorySubstrate::open_in_memory(0.01).unwrap();
    let store = memory.execution_targets();
    assert!(matches!(
        store.get(ExecutionTargetScope::Session, "/private/session"),
        Err(ExecutionTargetStoreError::InvalidScopeId)
    ));
    assert!(matches!(
        store.set(
            ExecutionTargetScope::Project,
            "project-1",
            &ExecutionTarget::Node {
                device_id: "node-invalid".to_string(),
                workspace_id: "main".to_string(),
            },
            1,
        ),
        Err(ExecutionTargetStoreError::InvalidTarget)
    ));
    assert!(matches!(
        store.set(
            ExecutionTargetScope::Project,
            "project-1",
            &ExecutionTarget::Hub,
            -1,
        ),
        Err(ExecutionTargetStoreError::InvalidTimestamp)
    ));
}

#[test]
fn deleting_sessions_and_projects_removes_their_target_bindings() {
    let memory = MemorySubstrate::open_in_memory(0.01).unwrap();
    let session = memory.create_session(AgentId::new()).unwrap();
    let project = memory
        .project_create(NewProject {
            name: "Target cleanup".to_string(),
            slug: "target-cleanup".to_string(),
            goal: "Prove deletion".to_string(),
            deadline: None,
        })
        .unwrap();
    memory
        .execution_targets()
        .set(
            ExecutionTargetScope::Session,
            &session.id.to_string(),
            &ExecutionTarget::Hub,
            1,
        )
        .unwrap();
    memory
        .execution_targets()
        .set(
            ExecutionTargetScope::Project,
            &project.id,
            &ExecutionTarget::Hub,
            1,
        )
        .unwrap();

    memory.delete_session(session.id).unwrap();
    assert!(memory
        .execution_targets()
        .get(ExecutionTargetScope::Session, &session.id.to_string())
        .unwrap()
        .is_none());
    assert!(memory.project_delete(&project.id).unwrap());
    assert!(memory
        .execution_targets()
        .get(ExecutionTargetScope::Project, &project.id)
        .unwrap()
        .is_none());
}
