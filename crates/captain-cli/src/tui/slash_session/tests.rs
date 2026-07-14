use super::*;

#[test]
fn chat_session_help_message_matches_hermes_startup_text() {
    assert_eq!(
        chat_session_help_message(),
        "/help for commands \u{2022} /exit to quit"
    );
}

#[test]
fn new_session_message_keeps_legacy_french_and_standalone_english() {
    assert_eq!(
        new_session_started_message(crate::i18n::Lang::Fr),
        "Nouvelle session persistée prête. La précédente reste restaurable."
    );
    assert_eq!(
        new_session_started_message(crate::i18n::Lang::En),
        "New persisted session ready. The previous one remains restorable."
    );
}

#[test]
fn reset_failure_message_formats_lang_specific_error() {
    assert_eq!(
        reset_session_failed_message(crate::i18n::Lang::Fr, "boom"),
        "Échec reset session : boom"
    );
    assert_eq!(
        reset_session_failed_message(crate::i18n::Lang::En, "boom"),
        "Reset session failed: boom"
    );
}

#[test]
fn reset_backend_error_messages_preserve_hermes_text() {
    assert_eq!(
        reset_daemon_agent_missing_message(),
        "No daemon agent bound to this chat."
    );
    assert_eq!(
        reset_inprocess_agent_missing_message(),
        "No in-process agent bound to this chat."
    );
    assert_eq!(
        reset_no_backend_connected_message(),
        "No backend connected."
    );
}

#[test]
fn no_saved_history_message_matches_surface_language() {
    assert_eq!(
        no_saved_history_message(crate::i18n::Lang::Fr),
        "Aucune session sauvegardée pour l'instant."
    );
    assert_eq!(
        no_saved_history_message(crate::i18n::Lang::En),
        "No saved session yet."
    );
}
