use super::*;
use crate::tui::screens::chat::{PendingAskUser, PendingModelSwitch};

fn pending_model_switch() -> PendingModelSwitch {
    PendingModelSwitch {
        model_id: "codex-pro".to_string(),
        current_provider: "openai".to_string(),
        current_model: "codex".to_string(),
        target_provider: "openai".to_string(),
        target_model: "codex-pro".to_string(),
        risk: "low".to_string(),
        recommended_session_strategy: "new_session".to_string(),
        active_message_count: 1,
        canonical_summary_present: false,
    }
}

#[test]
fn overlay_state_tracks_slash_model_and_session_overlays() {
    let mut state = ChatState::new();
    state.input = "/help".to_string();

    let overlays = chat_overlay_state(&state);
    assert!(overlays.slash_picker);
    assert!(!overlays.model_picker);
    assert!(!overlays.session_picker);

    state.show_model_picker = true;
    state.show_session_picker = true;
    let overlays = chat_overlay_state(&state);
    assert!(!overlays.slash_picker);
    assert!(overlays.model_picker);
    assert!(overlays.session_picker);
}

#[test]
fn overlay_state_tracks_quick_action_prompt() {
    let mut state = ChatState::new();
    assert!(!chat_overlay_state(&state).quick_action);

    state.pending_model_switch = Some(pending_model_switch());

    let overlays = chat_overlay_state(&state);
    assert!(overlays.quick_action);
}

#[test]
fn overlay_state_tracks_pending_ask_user() {
    // Regression: chat_overlay_state() used to hand-copy the pending-state
    // list instead of delegating to ChatState::has_quick_action_prompt(),
    // so it silently missed pending_ask_user and the modal never rendered.
    let mut state = ChatState::new();
    state.pending_ask_user = Some(PendingAskUser {
        question: "Couleur ?".to_string(),
        options: vec!["bleu".to_string(), "rouge".to_string()],
    });

    assert!(chat_overlay_state(&state).quick_action);
}
