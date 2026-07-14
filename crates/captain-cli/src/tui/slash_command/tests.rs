use super::*;

#[test]
fn split_slash_command_matches_hermes_space_split() {
    assert_eq!(
        split_slash_command("/shutdown confirm"),
        ("/shutdown".to_string(), "confirm")
    );
}

#[test]
fn split_slash_command_handles_spaces_tabs_and_args() {
    assert_eq!(
        split_slash_command("  /mouse   off "),
        ("/mouse".to_string(), "off")
    );
    assert_eq!(
        split_slash_command("/model\tgpt-5.5 --new"),
        ("/model".to_string(), "gpt-5.5 --new")
    );
}

#[test]
fn split_slash_command_strips_invisible_command_chars() {
    assert_eq!(
        split_slash_command("\u{200b}/help\r"),
        ("/help".to_string(), "")
    );
    assert_eq!(
        split_slash_command("/status\u{200d}   "),
        ("/status".to_string(), "")
    );
}

#[test]
fn canonical_slash_command_preserves_clean_args() {
    assert_eq!(
        canonical_slash_command("/shutdown", "confirm"),
        "/shutdown confirm"
    );
    assert_eq!(canonical_slash_command("/health", ""), "/health");
}
