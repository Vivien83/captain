use std::fmt;

pub(crate) fn chat_session_help_message() -> &'static str {
    "/help for commands \u{2022} /exit to quit"
}

pub(crate) fn new_session_started_message(lang: crate::i18n::Lang) -> &'static str {
    match lang {
        crate::i18n::Lang::Fr => {
            "Nouvelle session persistée prête. La précédente reste restaurable."
        }
        crate::i18n::Lang::En => {
            "New persisted session ready. The previous one remains restorable."
        }
    }
}

pub(crate) fn reset_session_failed_message(
    lang: crate::i18n::Lang,
    err: impl fmt::Display,
) -> String {
    match lang {
        crate::i18n::Lang::Fr => format!("Échec reset session : {err}"),
        crate::i18n::Lang::En => format!("Reset session failed: {err}"),
    }
}

pub(crate) fn reset_daemon_agent_missing_message() -> &'static str {
    "No daemon agent bound to this chat."
}

pub(crate) fn reset_inprocess_agent_missing_message() -> &'static str {
    "No in-process agent bound to this chat."
}

pub(crate) fn reset_no_backend_connected_message() -> &'static str {
    "No backend connected."
}

pub(crate) fn no_saved_history_message(lang: crate::i18n::Lang) -> &'static str {
    match lang {
        crate::i18n::Lang::Fr => "Aucune session sauvegardée pour l'instant.",
        crate::i18n::Lang::En => "No saved session yet.",
    }
}

#[cfg(test)]
#[path = "slash_session/tests.rs"]
mod tests;
