use super::*;

#[test]
fn copy_slash_target_defaults_to_response() {
    assert_eq!(
        copy_slash_target_for_arg(""),
        Some(CopySlashTarget::Response)
    );
}

#[test]
fn copy_slash_target_accepts_command_aliases() {
    assert_eq!(
        copy_slash_target_for_arg("command"),
        Some(CopySlashTarget::Command)
    );
    assert_eq!(
        copy_slash_target_for_arg("CMD"),
        Some(CopySlashTarget::Command)
    );
}

#[test]
fn copy_slash_target_accepts_response_alias() {
    assert_eq!(
        copy_slash_target_for_arg("response"),
        Some(CopySlashTarget::Response)
    );
}

#[test]
fn copy_slash_target_accepts_french_aliases() {
    assert_eq!(
        copy_slash_target_for_arg("commande"),
        Some(CopySlashTarget::Command)
    );
    assert_eq!(
        copy_slash_target_for_arg("réponse"),
        Some(CopySlashTarget::Response)
    );
    assert_eq!(
        copy_slash_target_for_arg("reponse"),
        Some(CopySlashTarget::Response)
    );
}

#[test]
fn copy_slash_target_rejects_unknown_values() {
    assert_eq!(copy_slash_target_for_arg("clipboard"), None);
}
