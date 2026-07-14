use super::*;

#[test]
fn decision_status_messages_preserve_hermes_text() {
    assert_eq!(decision_message("review-1", true), "review-1 approuvé");
    assert_eq!(decision_message("review-1", false), "review-1 refusé");
}
