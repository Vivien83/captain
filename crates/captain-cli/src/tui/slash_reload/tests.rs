use super::*;
use crate::i18n::Lang;

#[test]
fn daemon_reload_args_are_forwarded() {
    assert_eq!(reload_for("config"), SlashReload::ForwardDaemon);
    assert_eq!(reload_for(" daemon "), SlashReload::ForwardDaemon);
    assert_eq!(reload_for("daemon-config"), SlashReload::ForwardDaemon);
}

#[test]
fn empty_or_unknown_args_reload_local_session() {
    assert_eq!(reload_for(""), SlashReload::ReloadSession);
    assert_eq!(reload_for("latest"), SlashReload::ReloadSession);
}

#[test]
fn matching_stays_case_sensitive_like_hermes() {
    assert_eq!(reload_for("CONFIG"), SlashReload::ReloadSession);
}

#[test]
fn reload_messages_preserve_hermes_full_tui_text() {
    assert_eq!(
        no_active_session_message(Lang::Fr),
        "Pas de session active à recharger."
    );
    assert_eq!(
        no_saved_session_message(Lang::Fr),
        "Aucune session sauvegardée trouvée."
    );
}

#[test]
fn reload_messages_support_standalone_english() {
    assert_eq!(
        no_active_session_message(Lang::En),
        "No active session to reload."
    );
    assert_eq!(
        no_saved_session_message(Lang::En),
        "No saved session found."
    );
}
