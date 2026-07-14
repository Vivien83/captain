pub(crate) fn session_deleted_message(id: &str) -> String {
    format!("Session {id} deleted.")
}

pub(crate) fn memory_key_saved_message(key: &str) -> String {
    format!("Saved key: {key}")
}

pub(crate) fn memory_key_deleted_message(key: &str) -> String {
    format!("Deleted key: {key}")
}

pub(crate) fn skill_installed_message(name: &str) -> String {
    format!("Installed: {name}")
}

pub(crate) fn skill_uninstalled_message(name: &str) -> String {
    format!("Uninstalled: {name}")
}

pub(crate) fn provider_key_saved_message(name: &str) -> String {
    format!("Key saved for {name}")
}

pub(crate) fn provider_key_deleted_message(name: &str) -> String {
    format!("Key deleted for {name}")
}

#[cfg(test)]
#[path = "resource_status/tests.rs"]
mod tests;
