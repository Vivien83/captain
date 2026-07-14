use super::*;

#[test]
fn full_tui_help_delegates_to_i18n_body() {
    assert_eq!(
        full_tui_help(crate::i18n::Lang::En),
        crate::i18n::t("help.body", crate::i18n::Lang::En)
    );
    let fr = full_tui_help(crate::i18n::Lang::Fr);
    assert!(fr.contains("/voice [secs]"));
    assert!(fr.contains("/dashboard"));
    assert!(fr.contains("Routes opérateur exactes"));
    assert!(!fr.contains("/hands"));
    assert!(!fr.contains("/connections"));
}

#[test]
fn standalone_help_keeps_captain_chat_scope() {
    let help = standalone_help(crate::i18n::Lang::En);
    assert!(help.contains("/health       — daemon health"));
    assert!(help.contains("TUI hubs: /projects"));
    assert!(help.contains("/kill         — kill the current agent (Captain protected)"));
    assert!(!help.contains("/image <path>"));
}

#[test]
fn standalone_help_is_localized_in_french() {
    let help = standalone_help(crate::i18n::Lang::Fr);
    assert!(help.contains("/help         — afficher cette aide"));
    assert!(help.contains("Hubs TUI: /projects"));
    assert!(help.contains("/kill         — tuer l'agent courant (Captain protégé)"));
}
