use super::*;
use crate::i18n::Lang;

#[test]
fn daemon_forward_commands_match_hermes_set() {
    for command in ["/health", "/version", "/config", "/restart", "/shutdown"] {
        assert!(is_daemon_forward_command(command), "{command}");
    }
}

#[test]
fn non_daemon_commands_stay_in_slash_handler() {
    for command in ["/reload", "/status", "/model", "/Health", ""] {
        assert!(!is_daemon_forward_command(command), "{command}");
    }
}

#[test]
fn unavailable_message_preserves_hermes_texts() {
    assert_eq!(
        unavailable_message(Lang::Fr),
        "Commande daemon disponible uniquement en mode daemon."
    );
    assert_eq!(
        unavailable_message(Lang::En),
        "Daemon command is only available in daemon mode."
    );
}
