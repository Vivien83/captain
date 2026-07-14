use super::*;
use crate::tui::screens::approvals::ApprovalRequest;
use crate::tui::screens::chat::{
    ChatAction, PendingAskUser, PendingModelSwitch, QuickActionChoiceId,
};
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

fn key(code: KeyCode) -> KeyEvent {
    KeyEvent::new(code, KeyModifiers::NONE)
}

#[test]
fn approval_prompt_highlights_tool_and_omits_duplicate_description() {
    let mut state = ChatState::new();
    state.pending_approval = Some(ApprovalRequest {
        id: "req-1".into(),
        agent_name: "captain".into(),
        tool_name: "shell_exec".into(),
        description: "run tests".into(),
        action: "run tests".into(),
        risk_level: "high".into(),
        created_at: 0,
    });

    let prompt = build_quick_action_prompt(&state).expect("approval prompt");

    assert_eq!(prompt.title, "Approbation requise");
    assert_eq!(prompt.risk, "high");
    assert_eq!(
        prompt.details,
        vec![
            ("agent".into(), "captain".into(), false),
            ("tool".into(), "shell_exec".into(), true),
            ("action".into(), "run tests".into(), false),
        ]
    );
    assert_eq!(prompt.choices[0].id, QuickActionChoiceId::ApprovalOnce);
    assert_eq!(prompt.choices[3].style, QuickActionChoiceStyle::Danger);
}

#[test]
fn approval_prompt_keeps_distinct_detail() {
    let mut state = ChatState::new();
    state.pending_approval = Some(ApprovalRequest {
        id: "req-2".into(),
        agent_name: "captain".into(),
        tool_name: "cargo".into(),
        description: "lance la suite complete".into(),
        action: "cargo test".into(),
        risk_level: "medium".into(),
        created_at: 0,
    });

    let prompt = build_quick_action_prompt(&state).expect("approval prompt");

    assert!(prompt
        .details
        .contains(&("detail".into(), "lance la suite complete".into(), false)));
}

#[test]
fn model_switch_prompt_marks_compact_recommended_and_context_summary() {
    let mut state = ChatState::new();
    state.pending_model_switch = Some(PendingModelSwitch {
        model_id: "openai/gpt-5.4".into(),
        current_provider: "anthropic".into(),
        current_model: "claude-sonnet-4-6".into(),
        target_provider: "openai".into(),
        target_model: "gpt-5.4".into(),
        risk: "high".into(),
        recommended_session_strategy: "compact_session".into(),
        active_message_count: 12,
        canonical_summary_present: true,
    });

    let prompt = build_quick_action_prompt(&state).expect("model switch prompt");

    assert_eq!(prompt.title, "Changement de modele");
    assert_eq!(
        prompt.details,
        vec![
            ("actuel".into(), "anthropic/claude-sonnet-4-6".into(), false),
            ("cible".into(), "openai/gpt-5.4".into(), true),
            (
                "contexte".into(),
                "12 messages actifs + resume canonique".into(),
                false
            ),
        ]
    );
    assert_eq!(prompt.choices[1].label, "[2] Resume compact  recommande");
    assert_eq!(prompt.choices[2].id, QuickActionChoiceId::ModelSwitchCancel);
}

#[test]
fn model_switch_prompt_reports_empty_context() {
    let mut state = ChatState::new();
    state.pending_model_switch = Some(PendingModelSwitch {
        model_id: "openai/gpt-5.4".into(),
        current_provider: "anthropic".into(),
        current_model: "claude-sonnet-4-6".into(),
        target_provider: "openai".into(),
        target_model: "gpt-5.4".into(),
        risk: "low".into(),
        recommended_session_strategy: "new_session".into(),
        active_message_count: 0,
        canonical_summary_present: false,
    });

    let prompt = build_quick_action_prompt(&state).expect("model switch prompt");

    assert!(prompt
        .details
        .contains(&("contexte".into(), "aucun contexte actif".into(), false)));
    assert_eq!(prompt.choices[0].label, "[1] Nouvelle session  recommande");
}

#[test]
fn choice_lines_register_click_zones_and_wrap_when_needed() {
    let choices = vec![
        QuickActionChoice {
            id: QuickActionChoiceId::ApprovalOnce,
            label: "[o] Une fois".into(),
            style: QuickActionChoiceStyle::Primary,
        },
        QuickActionChoice {
            id: QuickActionChoiceId::ApprovalReject,
            label: "[n/Esc] Refuser".into(),
            style: QuickActionChoiceStyle::Danger,
        },
    ];
    let mut lines = vec![Line::from("lead")];
    let mut zones = Vec::new();

    push_quick_action_choice_lines(&mut lines, &mut zones, &choices, Rect::new(10, 5, 18, 4));

    assert_eq!(lines.len(), 3);
    assert_eq!(zones.len(), 2);
    assert_eq!(zones[0].x_start, 10);
    assert_eq!(zones[0].y, 6);
    assert_eq!(zones[0].choice, QuickActionChoiceId::ApprovalOnce);
    assert_eq!(zones[1].x_start, 10);
    assert_eq!(zones[1].y, 7);
    assert_eq!(zones[1].choice, QuickActionChoiceId::ApprovalReject);
}

#[test]
fn approval_quick_action_keys_match_hermes_mapping() {
    let cases = [
        (KeyCode::Char('y'), Some(QuickActionChoiceId::ApprovalOnce)),
        (KeyCode::Char('o'), Some(QuickActionChoiceId::ApprovalOnce)),
        (
            KeyCode::Char('s'),
            Some(QuickActionChoiceId::ApprovalSession),
        ),
        (
            KeyCode::Char('A'),
            Some(QuickActionChoiceId::ApprovalAlways),
        ),
        (
            KeyCode::Char('n'),
            Some(QuickActionChoiceId::ApprovalReject),
        ),
        (
            KeyCode::Char('d'),
            Some(QuickActionChoiceId::ApprovalReject),
        ),
        (KeyCode::Esc, Some(QuickActionChoiceId::ApprovalReject)),
        (KeyCode::Enter, None),
        (KeyCode::Char('x'), None),
    ];

    for (code, expected) in cases {
        assert_eq!(approval_quick_action_choice_for_key(key(code)), expected);
    }
}

#[test]
fn model_switch_quick_action_keys_choose_prompt_actions() {
    let cases = [
        (
            KeyCode::Esc,
            "",
            None,
            ModelSwitchQuickActionKey::Choice(QuickActionChoiceId::ModelSwitchCancel),
        ),
        (
            KeyCode::Char('1'),
            "",
            None,
            ModelSwitchQuickActionKey::Choice(QuickActionChoiceId::ModelSwitchNewSession),
        ),
        (
            KeyCode::Char('2'),
            " ",
            None,
            ModelSwitchQuickActionKey::Choice(QuickActionChoiceId::ModelSwitchCompactSession),
        ),
        (
            KeyCode::Enter,
            "",
            Some("new_session"),
            ModelSwitchQuickActionKey::Choice(QuickActionChoiceId::ModelSwitchNewSession),
        ),
        (
            KeyCode::Enter,
            "",
            Some("compact_session"),
            ModelSwitchQuickActionKey::Choice(QuickActionChoiceId::ModelSwitchCompactSession),
        ),
    ];

    for (code, answer, recommended, expected) in cases {
        assert_eq!(
            model_switch_quick_action_for_key(key(code), answer, recommended),
            expected
        );
    }
}

#[test]
fn model_switch_quick_action_enter_accepts_natural_language() {
    let cases = [
        (
            "nouvelle session",
            QuickActionChoiceId::ModelSwitchNewSession,
        ),
        ("sans contexte", QuickActionChoiceId::ModelSwitchNewSession),
        ("fresh reset", QuickActionChoiceId::ModelSwitchNewSession),
        (
            "garde le contexte",
            QuickActionChoiceId::ModelSwitchCompactSession,
        ),
        (
            "keep summary",
            QuickActionChoiceId::ModelSwitchCompactSession,
        ),
        ("annule", QuickActionChoiceId::ModelSwitchCancel),
    ];

    for (answer, choice) in cases {
        assert_eq!(
            model_switch_quick_action_for_key(key(KeyCode::Enter), answer, None),
            ModelSwitchQuickActionKey::Choice(choice)
        );
    }
}

#[test]
fn model_switch_quick_action_enter_reports_invalid_answer() {
    assert_eq!(
        model_switch_quick_action_for_key(key(KeyCode::Enter), "", None),
        ModelSwitchQuickActionKey::InvalidAnswer
    );
    assert_eq!(
        model_switch_quick_action_for_key(key(KeyCode::Enter), "banane", Some("new_session")),
        ModelSwitchQuickActionKey::InvalidAnswer
    );
}

#[test]
fn model_switch_quick_action_keys_route_input_editing() {
    let cases = [
        (KeyCode::Backspace, ModelSwitchQuickActionKey::Backspace),
        (KeyCode::Delete, ModelSwitchQuickActionKey::Delete),
        (KeyCode::Left, ModelSwitchQuickActionKey::Left),
        (KeyCode::Right, ModelSwitchQuickActionKey::Right),
        (KeyCode::Home, ModelSwitchQuickActionKey::Home),
        (KeyCode::End, ModelSwitchQuickActionKey::End),
        (KeyCode::Char('x'), ModelSwitchQuickActionKey::Insert('x')),
        (KeyCode::Tab, ModelSwitchQuickActionKey::Continue),
    ];

    for (code, expected) in cases {
        assert_eq!(
            model_switch_quick_action_for_key(key(code), "deja saisi", None),
            expected
        );
    }
}

#[test]
fn ask_user_prompt_lists_numbered_choices() {
    let pending = PendingAskUser {
        question: "Couleur ?".into(),
        options: vec!["bleu".into(), "rouge".into()],
    };
    let prompt = build_ask_user_prompt(&pending);

    assert_eq!(prompt.title, "Question");
    assert_eq!(prompt.lead, "Couleur ?");
    assert_eq!(prompt.choices.len(), 2);
    assert_eq!(prompt.choices[0].id, QuickActionChoiceId::AskUserOption(0));
    assert_eq!(prompt.choices[0].label, "[1] bleu");
    assert_eq!(prompt.choices[1].id, QuickActionChoiceId::AskUserOption(1));
    assert_eq!(prompt.choices[1].label, "[2] rouge");
}

#[test]
fn build_quick_action_prompt_prefers_ask_user_over_model_switch() {
    let mut state = ChatState::new();
    state.pending_ask_user = Some(PendingAskUser {
        question: "Pret ?".into(),
        options: vec!["oui".into()],
    });

    let prompt = build_quick_action_prompt(&state).expect("ask_user prompt");
    assert_eq!(prompt.title, "Question");
}

#[test]
fn ask_user_quick_action_keys_bounded_to_option_count() {
    let cases = [
        (
            KeyCode::Char('1'),
            2,
            Some(QuickActionChoiceId::AskUserOption(0)),
        ),
        (
            KeyCode::Char('2'),
            2,
            Some(QuickActionChoiceId::AskUserOption(1)),
        ),
        (KeyCode::Char('3'), 2, None), // out of range
        (KeyCode::Char('0'), 2, None), // no zero-th option
        (KeyCode::Enter, 2, None),
    ];

    for (code, n, expected) in cases {
        assert_eq!(ask_user_quick_action_choice_for_key(key(code), n), expected);
    }
}

#[test]
fn ask_user_option_selected_via_keyboard_answers_and_clears_pending() {
    let mut state = ChatState::new();
    state.pending_ask_user = Some(PendingAskUser {
        question: "Couleur ?".into(),
        options: vec!["bleu".into(), "rouge".into()],
    });

    let action = state.handle_key(key(KeyCode::Char('1')));

    assert_eq!(action, ChatAction::AnswerAskUser("bleu".to_string()));
    assert!(state.pending_ask_user.is_none());
}

#[test]
fn ask_user_without_options_does_not_take_over_key_handling() {
    let mut state = ChatState::new();
    // No pending_ask_user is set when options is empty (see T3) — normal
    // typing must keep working, not get absorbed by the quick-action modal.
    let action = state.handle_key(key(KeyCode::Char('x')));
    assert_eq!(action, ChatAction::Continue);
    assert!(state.pending_ask_user.is_none());
}
