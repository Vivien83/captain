use super::file_upload;
use super::navigation_state::{Phase, Tab};
use super::screens::chat;
use std::path::PathBuf;

const CHAT_SCROLL_STEP: u16 = 3;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum NonKeyInputRoute {
    Chat,
}

pub(crate) fn non_key_input_route_for_state(
    phase: Phase,
    active_tab: Tab,
) -> Option<NonKeyInputRoute> {
    match (phase, active_tab) {
        (Phase::Main, Tab::Chat) => Some(NonKeyInputRoute::Chat),
        _ => None,
    }
}

pub(crate) fn chat_scroll_offset_after_wheel(current: u16, up: bool) -> u16 {
    if up {
        current.saturating_add(CHAT_SCROLL_STEP)
    } else {
        current.saturating_sub(CHAT_SCROLL_STEP)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum ChatMouseEffect {
    CopyCommand(String),
    ApplyModelSwitch {
        model_id: String,
        session_strategy: String,
    },
    ChatAction(chat::ChatAction),
}

pub(crate) fn chat_mouse_effect_for_action(
    action: chat::ChatMouseAction,
) -> Option<ChatMouseEffect> {
    match action {
        chat::ChatMouseAction::CopyCommand(command) => Some(ChatMouseEffect::CopyCommand(command)),
        chat::ChatMouseAction::ApplyModelSwitch {
            model_id,
            session_strategy,
        } => Some(ChatMouseEffect::ApplyModelSwitch {
            model_id,
            session_strategy,
        }),
        chat::ChatMouseAction::ApproveRequest(id) => Some(ChatMouseEffect::ChatAction(
            chat::ChatAction::ApproveRequest(id),
        )),
        chat::ChatMouseAction::ApproveSessionRequest(id) => Some(ChatMouseEffect::ChatAction(
            chat::ChatAction::ApproveSessionRequest(id),
        )),
        chat::ChatMouseAction::ApproveAlwaysRequest(id) => Some(ChatMouseEffect::ChatAction(
            chat::ChatAction::ApproveAlwaysRequest(id),
        )),
        chat::ChatMouseAction::RejectRequest(id) => Some(ChatMouseEffect::ChatAction(
            chat::ChatAction::RejectRequest(id),
        )),
        chat::ChatMouseAction::ModelSwitchCancelled | chat::ChatMouseAction::ToolToggled => None,
    }
}

pub(crate) fn mouse_capture_after_slash_arg(current_enabled: bool, args: &str) -> Option<bool> {
    let normalized_arg = args.trim().to_ascii_lowercase();
    match normalized_arg.as_str() {
        "" | "toggle" => Some(!current_enabled),
        "on" | "true" | "1" | "yes" | "oui" => Some(true),
        "off" | "false" | "0" | "no" | "non" => Some(false),
        _ => None,
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum PasteEffect {
    AttachPath(PathBuf),
    PasteText,
}

pub(crate) fn paste_effect_for_state(
    phase: Phase,
    active_tab: Tab,
    data: &str,
) -> Option<PasteEffect> {
    non_key_input_route_for_state(phase, active_tab)?;
    if let Some(path) = file_upload::parse_dropped_path(data) {
        Some(PasteEffect::AttachPath(path))
    } else {
        Some(PasteEffect::PasteText)
    }
}

#[cfg(test)]
#[path = "input_state/tests.rs"]
mod tests;
