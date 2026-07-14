use super::*;

#[test]
fn protected_agent_check_is_trimmed_and_case_insensitive() {
    assert!(is_protected_agent(" captain "));
    assert!(is_protected_agent("CAPTAIN"));
    assert!(!is_protected_agent("captain-worker"));
}

#[test]
fn kill_messages_preserve_hermes_english_standalone_text() {
    assert_eq!(
        kill_success_message(crate::i18n::Lang::En, "worker"),
        "Agent \"worker\" killed."
    );
    assert_eq!(
        kill_failed_message(crate::i18n::Lang::En, "worker"),
        "Failed to kill agent \"worker\"."
    );
    assert_eq!(
        kill_error_message(crate::i18n::Lang::En, "boom"),
        "Kill failed: boom"
    );
    assert_eq!(
        no_backend_message(crate::i18n::Lang::En),
        "No backend connected."
    );
}

#[test]
fn kill_messages_preserve_french_i18n_text() {
    assert_eq!(
        kill_success_message(crate::i18n::Lang::Fr, "worker"),
        "Agent « worker » tué."
    );
    assert_eq!(
        kill_failed_message(crate::i18n::Lang::Fr, "worker"),
        "Échec : impossible de tuer « worker »."
    );
    assert_eq!(
        kill_error_message(crate::i18n::Lang::Fr, "boom"),
        "Échec du kill : boom"
    );
    assert_eq!(
        no_backend_message(crate::i18n::Lang::Fr),
        "Aucun backend connecté."
    );
}
