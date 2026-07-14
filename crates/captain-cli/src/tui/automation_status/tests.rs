use super::*;

#[test]
fn automation_status_messages_preserve_hermes_text() {
    assert_eq!(workflow_created_message(), "Workflow created!");
    assert_eq!(trigger_created_message(), "Trigger created!");
    assert_eq!(
        trigger_deleted_message("nightly"),
        "Trigger nightly deleted."
    );
    assert_eq!(
        trigger_toggled_message("nightly", true),
        "Trigger nightly activé"
    );
    assert_eq!(
        trigger_toggled_message("nightly", false),
        "Trigger nightly désactivé"
    );
    assert_eq!(
        cron_job_mutated_message("job-1", "enabled"),
        "cron enabled: job-1"
    );
}
