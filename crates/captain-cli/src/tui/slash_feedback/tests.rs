use super::*;

#[test]
fn like_and_dislike_map_to_feedback_values() {
    let like = feedback_for("/like", "  useful answer  ").unwrap();
    assert_eq!(like.value, "up");
    assert_eq!(like.note, "useful answer");

    let dislike = feedback_for("/dislike", "\tmissed the file\n").unwrap();
    assert_eq!(dislike.value, "down");
    assert_eq!(dislike.note, "missed the file");
}

#[test]
fn non_feedback_commands_are_ignored() {
    assert!(feedback_for("/status", "nice").is_none());
}

#[test]
fn response_preview_is_char_bounded() {
    let text = format!("{}{}", "a".repeat(119), "\u{00e9}clair");

    assert_eq!(response_preview(&text).chars().count(), 120);
    assert!(response_preview(&text).ends_with('\u{00e9}'));
}

#[test]
fn payload_keeps_feedback_contract() {
    let payload = feedback_payload("up", "good", "preview", 42);

    assert_eq!(payload["type"], "thumbs");
    assert_eq!(payload["value"], "up");
    assert_eq!(payload["note"], "good");
    assert_eq!(payload["preview"], "preview");
    assert_eq!(payload["ts"], 42);
}

#[test]
fn status_messages_keep_hermes_full_tui_text() {
    assert_eq!(
        feedback_requires_daemon_message(),
        "Le feedback nécessite le mode daemon avec un agent actif."
    );
    assert_eq!(feedback_saved_message("up"), "👍 feedback enregistré.");
    assert_eq!(feedback_saved_message("down"), "👎 feedback enregistré.");
    assert_eq!(
        feedback_http_error_message("500 Internal Server Error"),
        "Feedback échoué: HTTP 500 Internal Server Error"
    );
    assert_eq!(
        feedback_error_message("network down"),
        "Feedback échoué: network down"
    );
}
