use super::*;

#[test]
fn project_runtime_run_registry_tracks_started_runs() {
    let project_id = format!("runner-test-{}", uuid::Uuid::new_v4());

    project_runtime_mark_finished(&project_id);
    assert!(!project_runtime_is_running(&project_id));
    assert!(project_runtime_mark_started(&project_id));
    assert!(project_runtime_is_running(&project_id));
    assert!(!project_runtime_mark_started(&project_id));

    project_runtime_mark_finished(&project_id);
    assert!(!project_runtime_is_running(&project_id));
}
