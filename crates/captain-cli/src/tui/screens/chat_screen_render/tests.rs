use super::*;
use crate::tui::provider_quota::{ProviderQuota, ProviderQuotaStatus, ProviderQuotaWindow};
use crate::tui::screens::chat::{PendingAskUser, PendingModelSwitch};
use chrono::{Duration, Utc};

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

#[test]
fn chat_renders_live_provider_quota_in_terminal_and_web_widths() {
    for (width, height) in [(120, 30), (60, 20)] {
        let mut state = ChatState::new();
        state.model_label = "codex/gpt-5.6-sol".to_string();
        state.input = "Question courante".to_string();
        state.provider_quota_status = ProviderQuotaStatus {
            state: "warning".to_string(),
            reported_by_provider: true,
            quotas: vec![ProviderQuota {
                provider: "codex".to_string(),
                limit_id: "codex".to_string(),
                limit_name: "Codex".to_string(),
                plan_type: Some("pro".to_string()),
                alert_level: "warning".to_string(),
                stale: false,
                primary: Some(ProviderQuotaWindow {
                    used_percent: 63.0,
                    window_seconds: Some(18_000),
                    reset_after_seconds: Some(3_600),
                    resets_at: Some(Utc::now() + Duration::hours(1)),
                }),
                secondary: None,
                credits: None,
                rate_limit_reached_type: None,
                observed_at: Some(Utc::now()),
            }],
        };
        let mut image_cache = ImagePreviewCache::new();
        let backend = ratatui::backend::TestBackend::new(width, height);
        let mut terminal = ratatui::Terminal::new(backend).expect("terminal");

        terminal
            .draw(|frame| draw_chat_screen(frame, frame.area(), &mut state, &mut image_cache))
            .expect("draw chat");

        let rendered = format!("{:?}", terminal.backend().buffer());
        assert!(rendered.contains("Question courante"), "{rendered}");
        assert!(rendered.contains("Codex [pro]"), "{rendered}");
        assert!(rendered.contains("63%"), "{rendered}");
        assert!(rendered.contains('█'), "{rendered}");
        assert!(rendered.contains('░'), "{rendered}");
    }
}
