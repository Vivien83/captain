use crate::i18n::Lang;

pub(crate) fn is_daemon_forward_command(command: &str) -> bool {
    matches!(
        command,
        "/health" | "/version" | "/config" | "/restart" | "/shutdown"
    )
}

pub(crate) fn unavailable_message(lang: Lang) -> &'static str {
    match lang {
        Lang::Fr => "Commande daemon disponible uniquement en mode daemon.",
        Lang::En => "Daemon command is only available in daemon mode.",
    }
}

#[cfg(test)]
#[path = "slash_daemon/tests.rs"]
mod tests;
