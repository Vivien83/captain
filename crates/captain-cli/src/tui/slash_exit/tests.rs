use super::*;

#[test]
fn exit_commands_match_hermes_set() {
    assert!(is_exit_command("/exit"));
    assert!(is_exit_command("/quit"));
}

#[test]
fn non_exit_commands_stay_in_slash_handler() {
    for command in ["/Exit", "/q", "/shutdown", ""] {
        assert!(!is_exit_command(command), "{command}");
    }
}
