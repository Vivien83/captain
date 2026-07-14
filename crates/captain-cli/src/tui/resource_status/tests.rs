use super::*;

#[test]
fn resource_status_messages_preserve_hermes_text() {
    assert_eq!(session_deleted_message("s-1"), "Session s-1 deleted.");
    assert_eq!(memory_key_saved_message("pref"), "Saved key: pref");
    assert_eq!(memory_key_deleted_message("pref"), "Deleted key: pref");
    assert_eq!(skill_installed_message("docs"), "Installed: docs");
    assert_eq!(skill_uninstalled_message("docs"), "Uninstalled: docs");
    assert_eq!(provider_key_saved_message("openai"), "Key saved for openai");
    assert_eq!(
        provider_key_deleted_message("openai"),
        "Key deleted for openai"
    );
}
