use super::*;
use crate::tui::screens::approvals::ApprovalRequest;
use crate::tui::screens::chat::PendingModelSwitch;

fn footer_text(state: &ChatState, width: usize) -> String {
    line_plain_text(&build_input_footer(state, width))
}

#[test]
fn idle_footer_surfaces_context_actions_and_telemetry() {
    let mut s = ChatState::new();
    s.model_label = "anthropic/claude-sonnet-4-6".into();
    s.session_input_tokens = 12_000;
    s.session_output_tokens = 2_000;
    s.last_tokens = Some((12_000, 2_000));
    s.session_cost_usd = 0.042;

    let text = footer_text(&s, 140);

    assert!(text.contains("prêt"));
    assert!(text.contains("ctx"));
    assert!(text.contains("14k tok"));
    assert!(text.contains("Enter envoyer"));
    assert!(text.contains("Ctrl+M modèle"));
    assert!(text.contains("$0.0420"));
}

#[test]
fn streaming_footer_surfaces_output_queue_and_running_tools() {
    let mut s = ChatState::new();
    s.is_streaming = true;
    s.streaming_chars = 168;
    s.spinner_frame = 7;
    s.session_input_tokens = 1_000;
    s.last_tokens = Some((1_000, 0));
    s.staged_messages.push("next question".into());
    s.active_tool = Some("shell_exec".into());

    let text = footer_text(&s, 140);

    assert!(text.contains("stream"));
    assert!(text.contains("out"));
    assert!(text.contains("~42 tok"));
    assert!(text.contains("queued 1"));
    assert!(text.contains("tools 1 run"));
    assert!(text.contains("Enter interject"));
}

#[test]
fn high_context_footer_recommends_compact() {
    let mut s = ChatState::new();
    s.session_input_tokens = 51_000;
    s.last_tokens = Some((51_000, 0));

    let text = footer_text(&s, 120);

    assert!(text.contains("ctx"));
    assert!(text.contains("/compact"));
}

#[test]
fn footer_context_uses_session_total_not_last_turn() {
    let mut s = ChatState::new();
    s.session_input_tokens = 30_000;
    s.session_output_tokens = 5_000;
    s.last_tokens = Some((2_000, 100));

    let text = footer_text(&s, 140);

    assert!(text.contains("35k tok"), "{text}");
    assert!(!text.contains("2.1k tok"), "{text}");
}

#[test]
fn streaming_footer_adds_current_turn_to_session_context() {
    let mut s = ChatState::new();
    s.is_streaming = true;
    s.session_input_tokens = 10_000;
    s.session_output_tokens = 2_000;
    s.last_tokens = Some((1_000, 500));
    s.streaming_chars = 2_000;

    let text = footer_text(&s, 140);

    assert!(text.contains("14k tok"), "{text}");
}

#[test]
fn slash_footer_uses_command_mode_actions() {
    let mut s = ChatState::new();
    s.input_insert_str("/mo");

    let text = footer_text(&s, 120);

    assert!(text.contains("commandes"));
    assert!(text.contains("Tab compléter"));
    assert!(text.contains("Enter appliquer"));
}

#[test]
fn footer_action_spec_prioritizes_modal_states() {
    let mut s = ChatState::new();
    s.pending_approval = Some(ApprovalRequest {
        id: "approval".into(),
        agent_name: "captain".into(),
        tool_name: "shell_exec".into(),
        description: "run".into(),
        action: "run".into(),
        risk_level: "high".into(),
        created_at: 0,
    });

    let (priority, actions) = footer_action_spec(&s);

    assert_eq!(priority, 1);
    assert_eq!(actions, APPROVAL_ACTIONS);

    s.pending_approval = None;
    s.pending_model_switch = Some(PendingModelSwitch {
        model_id: "openai/gpt-5".into(),
        current_provider: "anthropic".into(),
        current_model: "claude".into(),
        target_provider: "openai".into(),
        target_model: "gpt-5".into(),
        risk: "medium".into(),
        recommended_session_strategy: "compact_session".into(),
        active_message_count: 1,
        canonical_summary_present: true,
    });

    let (priority, actions) = footer_action_spec(&s);

    assert_eq!(priority, 1);
    assert_eq!(actions, MODEL_SWITCH_ACTIONS);
}

#[test]
fn footer_action_spec_reports_picker_and_idle_actions() {
    let mut s = ChatState::new();
    s.show_model_picker = true;

    let (priority, actions) = footer_action_spec(&s);

    assert_eq!(priority, 2);
    assert_eq!(actions, MODEL_PICKER_ACTIONS);

    s.show_model_picker = false;
    s.show_session_picker = true;

    let (priority, actions) = footer_action_spec(&s);

    assert_eq!(priority, 2);
    assert_eq!(actions, SESSION_PICKER_ACTIONS);

    s.show_session_picker = false;

    let (priority, actions) = footer_action_spec(&s);

    assert_eq!(priority, 3);
    assert_eq!(actions, IDLE_ACTIONS);
}

#[test]
fn footer_degrades_without_exceeding_narrow_width() {
    let mut s = ChatState::new();
    s.model_label = "anthropic/claude-sonnet-4-6".into();
    s.session_input_tokens = 51_000;
    s.session_output_tokens = 8_000;
    s.last_tokens = Some((51_000, 8_000));
    s.session_cost_usd = 0.1234;
    s.staged_messages.push("queued".into());
    s.active_tool = Some("shell_exec".into());

    let width = 36;
    let text = footer_text(&s, width);

    assert!(text.chars().count() <= width, "footer overflowed: {text:?}");
    assert!(text.contains("prêt") || text.contains("ctx"));
}
