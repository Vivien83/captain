use super::*;

#[test]
fn think_command_matches_exact_normalized_command() {
    assert!(is_think_command("/think"));
}

#[test]
fn non_think_commands_stay_in_slash_handler() {
    assert!(!is_think_command("/thinking"));
    assert!(!is_think_command("/Think"));
    assert!(!is_think_command(""));
}
