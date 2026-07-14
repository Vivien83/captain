pub(crate) fn workflow_created_message() -> &'static str {
    "Workflow created!"
}

pub(crate) fn trigger_created_message() -> &'static str {
    "Trigger created!"
}

pub(crate) fn trigger_deleted_message(id: &str) -> String {
    format!("Trigger {id} deleted.")
}

pub(crate) fn trigger_toggled_message(id: &str, enabled: bool) -> String {
    format!(
        "Trigger {id} {}",
        if enabled { "activé" } else { "désactivé" }
    )
}

pub(crate) fn cron_job_mutated_message(id: &str, what: &str) -> String {
    format!("cron {what}: {id}")
}

#[cfg(test)]
#[path = "automation_status/tests.rs"]
mod tests;
