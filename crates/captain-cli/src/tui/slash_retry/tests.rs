use super::*;

fn msg(role: Role, text: &str) -> ChatMessage {
    ChatMessage {
        role,
        text: text.to_string(),
        tool: None,
    }
}

#[test]
fn returns_latest_user_message_from_tail() {
    let messages = vec![
        msg(Role::User, "first"),
        msg(Role::Agent, "answer"),
        msg(Role::User, "second"),
        msg(Role::System, "status"),
    ];

    assert_eq!(last_user_message(&messages).as_deref(), Some("second"));
}

#[test]
fn ignores_non_user_messages() {
    let messages = vec![msg(Role::System, "status"), msg(Role::Agent, "answer")];

    assert!(last_user_message(&messages).is_none());
}

#[test]
fn empty_history_has_no_retry_target() {
    assert!(last_user_message(&[]).is_none());
}

#[test]
fn retry_nothing_message_preserves_hermes_i18n_text() {
    assert_eq!(
        retry_nothing_message(crate::i18n::Lang::Fr),
        "Rien à renvoyer (aucun message précédent)."
    );
    assert_eq!(
        retry_nothing_message(crate::i18n::Lang::En),
        "Nothing to retry (no previous message)."
    );
}
