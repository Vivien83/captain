pub(crate) fn unknown_slash_message(lang: crate::i18n::Lang, command: &str) -> String {
    let base = crate::i18n::t("chat.unknown_cmd", lang);
    if command.is_empty() {
        base.to_string()
    } else {
        format!("{base} ({command})")
    }
}

pub(crate) fn full_tui_navigation_message(lang: crate::i18n::Lang, command: &str) -> String {
    match lang {
        crate::i18n::Lang::Fr => format!(
            "{command} est une commande de navigation du TUI complet. Ouvre `captain tui` pour utiliser cet écran ; `captain chat` reste centré sur le chat SSH."
        ),
        crate::i18n::Lang::En => format!(
            "{command} is a full TUI navigation command. Open `captain tui` to use that screen; `captain chat` remains focused on the SSH chat."
        ),
    }
}

pub(crate) fn is_attachment_or_voice_command(command: &str) -> bool {
    matches!(command, "/image" | "/file" | "/voice")
}

pub(crate) fn is_feedback_command(command: &str) -> bool {
    matches!(command, "/like" | "/dislike")
}

pub(crate) fn is_full_tui_navigation_command(command: &str) -> bool {
    matches!(
        command,
        "/home"
            | "/projects"
            | "/project"
            | "/automation"
            | "/workflows"
            | "/triggers"
            | "/memory"
            | "/learning"
            | "/skills"
            | "/skills-proposed"
            | "/proposed"
            | "/cron"
            | "/scheduler"
            | "/approvals"
            | "/capabilities"
            | "/channels"
            | "/budget"
            | "/graph"
            | "/logs"
            | "/settings"
    )
}

pub(crate) fn attachments_voice_message(lang: crate::i18n::Lang) -> &'static str {
    match lang {
        crate::i18n::Lang::Fr => {
            "Les pièces jointes et l'enregistrement vocal sont disponibles dans `captain tui` ; `captain chat` garde une surface SSH minimale."
        }
        crate::i18n::Lang::En => {
            "Attachments and voice recording are available in `captain tui`; standalone `captain chat` keeps the SSH chat surface minimal."
        }
    }
}

pub(crate) fn feedback_unavailable_message(lang: crate::i18n::Lang) -> &'static str {
    match lang {
        crate::i18n::Lang::Fr => {
            "Les commandes de feedback sont disponibles dans `captain tui` ; ce chat SSH ne persiste pas encore le feedback."
        }
        crate::i18n::Lang::En => {
            "Feedback commands are available in `captain tui`; this standalone chat does not persist feedback yet."
        }
    }
}

pub(crate) fn stream_error_message(error: impl std::fmt::Display) -> String {
    format!("Error: {error}")
}

pub(crate) fn no_active_connection_message() -> &'static str {
    "No active connection"
}

pub(crate) fn daemon_agent_spawn_failed_message(error: impl std::fmt::Display) -> String {
    format!("Failed to spawn agent: {error}")
}

pub(crate) fn spawning_agent_message(name: &str) -> String {
    format!("Spawning '{name}' agent\u{2026}")
}

pub(crate) fn no_agent_templates_message() -> &'static str {
    "No agent templates found. Run `captain init`."
}

pub(crate) fn invalid_template_message(name: &str, error: impl std::fmt::Display) -> String {
    format!("Invalid template '{name}': {error}")
}

pub(crate) fn inprocess_agent_spawn_failed_message(error: impl std::fmt::Display) -> String {
    format!("Spawn failed: {error}")
}

#[cfg(test)]
#[path = "slash_standalone/tests.rs"]
mod tests;
