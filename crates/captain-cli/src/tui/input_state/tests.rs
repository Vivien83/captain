use super::super::navigation_state::BootScreen;
use super::*;

#[test]
fn non_key_input_routes_only_main_chat() {
    assert_eq!(
        non_key_input_route_for_state(Phase::Main, Tab::Chat),
        Some(NonKeyInputRoute::Chat)
    );
}

#[test]
fn non_key_input_ignores_boot_and_non_chat_tabs() {
    assert_eq!(
        non_key_input_route_for_state(Phase::Boot(BootScreen::Welcome), Tab::Chat),
        None
    );
    assert_eq!(
        non_key_input_route_for_state(Phase::Main, Tab::Projects),
        None
    );
}

#[test]
fn chat_scroll_offset_moves_by_fixed_step() {
    assert_eq!(chat_scroll_offset_after_wheel(10, true), 13);
    assert_eq!(chat_scroll_offset_after_wheel(10, false), 7);
}

#[test]
fn chat_scroll_offset_saturates() {
    assert_eq!(chat_scroll_offset_after_wheel(u16::MAX - 1, true), u16::MAX);
    assert_eq!(chat_scroll_offset_after_wheel(2, false), 0);
}

#[test]
fn chat_mouse_actions_map_to_app_effects() {
    assert_eq!(
        chat_mouse_effect_for_action(chat::ChatMouseAction::CopyCommand("ls".into())),
        Some(ChatMouseEffect::CopyCommand("ls".into()))
    );
    assert_eq!(
        chat_mouse_effect_for_action(chat::ChatMouseAction::ApplyModelSwitch {
            model_id: "codex".into(),
            session_strategy: "compact".into(),
        }),
        Some(ChatMouseEffect::ApplyModelSwitch {
            model_id: "codex".into(),
            session_strategy: "compact".into(),
        })
    );
    assert_eq!(
        chat_mouse_effect_for_action(chat::ChatMouseAction::ApproveRequest("req-1".into())),
        Some(ChatMouseEffect::ChatAction(
            chat::ChatAction::ApproveRequest("req-1".into())
        ))
    );
    assert_eq!(
        chat_mouse_effect_for_action(chat::ChatMouseAction::ApproveSessionRequest("req-2".into())),
        Some(ChatMouseEffect::ChatAction(
            chat::ChatAction::ApproveSessionRequest("req-2".into())
        ))
    );
    assert_eq!(
        chat_mouse_effect_for_action(chat::ChatMouseAction::ApproveAlwaysRequest("req-3".into())),
        Some(ChatMouseEffect::ChatAction(
            chat::ChatAction::ApproveAlwaysRequest("req-3".into())
        ))
    );
    assert_eq!(
        chat_mouse_effect_for_action(chat::ChatMouseAction::RejectRequest("req-4".into())),
        Some(ChatMouseEffect::ChatAction(
            chat::ChatAction::RejectRequest("req-4".into())
        ))
    );
}

#[test]
fn chat_mouse_actions_without_app_effect_are_ignored() {
    assert_eq!(
        chat_mouse_effect_for_action(chat::ChatMouseAction::ToolToggled),
        None
    );
    assert_eq!(
        chat_mouse_effect_for_action(chat::ChatMouseAction::ModelSwitchCancelled),
        None
    );
}

#[test]
fn mouse_capture_slash_arg_toggles_when_empty_or_toggle() {
    assert_eq!(mouse_capture_after_slash_arg(false, ""), Some(true));
    assert_eq!(mouse_capture_after_slash_arg(true, ""), Some(false));
    assert_eq!(mouse_capture_after_slash_arg(false, "toggle"), Some(true));
}

#[test]
fn mouse_capture_slash_arg_accepts_on_off_aliases() {
    for arg in ["on", "true", "1", "yes", "oui", "ON"] {
        assert_eq!(mouse_capture_after_slash_arg(false, arg), Some(true));
    }
    for arg in ["off", "false", "0", "no", "non", "OFF"] {
        assert_eq!(mouse_capture_after_slash_arg(true, arg), Some(false));
    }
}

#[test]
fn mouse_capture_slash_arg_rejects_unknown_value() {
    assert_eq!(mouse_capture_after_slash_arg(false, "native"), None);
}

#[test]
fn paste_effect_ignores_non_chat_screens() {
    assert_eq!(
        paste_effect_for_state(Phase::Boot(BootScreen::Welcome), Tab::Chat, "hello"),
        None
    );
    assert_eq!(
        paste_effect_for_state(Phase::Main, Tab::Projects, "hello"),
        None
    );
}

#[test]
fn paste_effect_routes_plain_text_to_chat() {
    assert_eq!(
        paste_effect_for_state(Phase::Main, Tab::Chat, "hello"),
        Some(PasteEffect::PasteText)
    );
}

#[test]
fn paste_effect_routes_supported_dropped_file_to_attachment() {
    let path = std::env::temp_dir().join(format!(
        "captain-input-state-drop-{}-{}.png",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::write(&path, b"png").unwrap();

    assert_eq!(
        paste_effect_for_state(Phase::Main, Tab::Chat, &path.to_string_lossy()),
        Some(PasteEffect::AttachPath(path.clone()))
    );

    let _ = std::fs::remove_file(path);
}
