use super::screens::chat::{ChatState, Role};
use crate::i18n::Lang;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CopyTarget {
    Response,
    Command,
}

pub(crate) struct CopyTargetText {
    pub(crate) label: &'static str,
    pub(crate) empty_message: &'static str,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CopyUsageSurface {
    FullTui,
    StandaloneChat,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CopyStatusSurface {
    FullTui,
    StandaloneChat,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum MouseMessageSurface {
    FullTui,
    StandaloneChat,
}

pub(crate) fn copy_target(args: &str) -> Result<CopyTarget, &'static str> {
    match args.to_ascii_lowercase().as_str() {
        "command" | "cmd" | "commande" => Ok(CopyTarget::Command),
        "" | "response" | "réponse" | "reponse" => Ok(CopyTarget::Response),
        _ => Err("Usage: /copy ou /copy command"),
    }
}

pub(crate) fn copy_target_text(target: CopyTarget, lang: Lang) -> CopyTargetText {
    match (target, lang) {
        (CopyTarget::Command, Lang::Fr) => CopyTargetText {
            label: "Commande",
            empty_message: "Aucune commande tool-call à copier.",
        },
        (CopyTarget::Command, Lang::En) => CopyTargetText {
            label: "Command",
            empty_message: "No tool-call command to copy.",
        },
        (CopyTarget::Response, Lang::Fr) => CopyTargetText {
            label: "Réponse",
            empty_message: "Aucune réponse à copier.",
        },
        (CopyTarget::Response, Lang::En) => CopyTargetText {
            label: "Response",
            empty_message: "No response to copy.",
        },
    }
}

pub(crate) fn copy_usage_message(lang: Lang, surface: CopyUsageSurface) -> &'static str {
    match (lang, surface) {
        (Lang::Fr, CopyUsageSurface::StandaloneChat) => "Usage : /copy ou /copy command",
        (Lang::Fr, CopyUsageSurface::FullTui) => "Usage: /copy ou /copy command",
        (Lang::En, _) => "Usage: /copy or /copy command",
    }
}

pub(crate) fn copy_success_message(
    surface: CopyStatusSurface,
    label: &str,
    byte_len: usize,
) -> String {
    match surface {
        CopyStatusSurface::FullTui => {
            format!("{label} copiée dans le clipboard ({byte_len} caractères).")
        }
        CopyStatusSurface::StandaloneChat => {
            format!("{label} copied to clipboard ({byte_len} chars).")
        }
    }
}

pub(crate) fn copy_failure_message(
    surface: CopyStatusSurface,
    error: impl std::fmt::Display,
) -> String {
    match surface {
        CopyStatusSurface::FullTui => format!("Échec copie clipboard: {error}"),
        CopyStatusSurface::StandaloneChat => format!("Clipboard copy failed: {error}"),
    }
}

pub(crate) fn mouse_capture_target(args: &str, current: bool) -> Option<bool> {
    match args.to_ascii_lowercase().as_str() {
        "" | "toggle" => Some(!current),
        "on" | "true" | "1" | "yes" | "oui" => Some(true),
        "off" | "false" | "0" | "no" | "non" => Some(false),
        _ => None,
    }
}

pub(crate) fn mouse_enabled_message(lang: Lang, surface: MouseMessageSurface) -> &'static str {
    match surface {
        MouseMessageSurface::FullTui => {
            "Mode souris activé: clics tool calls + molette. Pour sélectionner/copier, utilise `/mouse off`."
        }
        MouseMessageSurface::StandaloneChat => match lang {
            Lang::Fr => {
                "Mode souris activé: clics tool calls + scroll TUI. Utilise `/mouse off` pour la sélection native."
            }
            Lang::En => {
                "Mouse mode enabled: tool-call clicks + TUI scrolling. Use `/mouse off` for native selection."
            }
        },
    }
}

pub(crate) fn mouse_disabled_message(lang: Lang) -> &'static str {
    match lang {
        Lang::Fr => "Mode souris désactivé: sélection native + clic droit copier disponibles.",
        Lang::En => "Mouse mode disabled: native selection + right-click copy are available.",
    }
}

pub(crate) fn mouse_error_message(
    lang: Lang,
    surface: MouseMessageSurface,
    error: impl std::fmt::Display,
) -> String {
    match (lang, surface) {
        (Lang::Fr, MouseMessageSurface::StandaloneChat) => {
            format!("Échec changement mode souris : {error}")
        }
        (Lang::Fr, MouseMessageSurface::FullTui) => {
            format!("Échec changement mode souris: {error}")
        }
        (Lang::En, _) => format!("Mouse mode change failed: {error}"),
    }
}

pub(crate) fn mouse_usage_message(lang: Lang, surface: MouseMessageSurface) -> &'static str {
    match (lang, surface) {
        (Lang::Fr, MouseMessageSurface::StandaloneChat) => "Usage : /mouse, /mouse on, /mouse off",
        (Lang::Fr, MouseMessageSurface::FullTui) => "Usage: /mouse, /mouse on, /mouse off",
        (Lang::En, _) => "Usage: /mouse, /mouse on, /mouse off",
    }
}

pub(crate) fn queue_message(staged_messages: &[String], empty: &str, header: &str) -> String {
    if staged_messages.is_empty() {
        return empty.to_string();
    }

    let mut lines = vec![header.to_string()];
    for (idx, message) in staged_messages.iter().enumerate() {
        lines.push(format!("  {}. {}", idx + 1, message));
    }
    lines.join("\n")
}

pub(crate) fn queue_message_for_lang(staged_messages: &[String], lang: Lang) -> String {
    queue_message(
        staged_messages,
        crate::i18n::t("queue.empty", lang),
        crate::i18n::t("queue.header", lang),
    )
}

pub(crate) fn undo_result_message(dropped_user: bool, lang: Lang) -> &'static str {
    let key = if dropped_user {
        "undo.done"
    } else {
        "undo.nothing"
    };
    crate::i18n::t(key, lang)
}

pub(crate) fn clear_message(lang: Lang) -> &'static str {
    crate::i18n::t("chat.cleared", lang)
}

pub(crate) fn voice_record_secs(args: &str) -> u64 {
    args.parse().unwrap_or(5)
}

pub(crate) fn voice_recording_message(secs: u64) -> String {
    format!("🎙 Enregistrement {secs}s en cours...")
}

pub(crate) fn voice_uploading_message(path: impl std::fmt::Display) -> String {
    format!("📤 Envoi de l'audio: {path}")
}

pub(crate) fn voice_error_message(error: impl std::fmt::Display) -> String {
    format!("🎙 {error}")
}

pub(crate) fn undo_last_exchange(chat: &mut ChatState) -> bool {
    while let Some(message) = chat.messages.last() {
        let is_user = message.role == Role::User;
        chat.messages.pop();
        if is_user {
            return true;
        }
    }
    false
}

pub(crate) fn clear_chat_preserving_identity(chat: &mut ChatState) {
    let name = chat.agent_name.clone();
    let model = chat.model_label.clone();
    let mode = chat.mode_label.clone();
    chat.reset();
    chat.agent_name = name;
    chat.model_label = model;
    chat.mode_label = mode;
}

#[cfg(test)]
#[path = "slash_local/tests.rs"]
mod tests;
