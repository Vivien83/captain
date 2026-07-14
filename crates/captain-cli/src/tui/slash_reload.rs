use crate::i18n::Lang;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SlashReload {
    ForwardDaemon,
    ReloadSession,
}

pub(crate) fn reload_for(args: &str) -> SlashReload {
    match args.trim() {
        "config" | "daemon" | "daemon-config" => SlashReload::ForwardDaemon,
        _ => SlashReload::ReloadSession,
    }
}

pub(crate) fn no_active_session_message(lang: Lang) -> &'static str {
    match lang {
        Lang::Fr => "Pas de session active à recharger.",
        Lang::En => "No active session to reload.",
    }
}

pub(crate) fn no_saved_session_message(lang: Lang) -> &'static str {
    match lang {
        Lang::Fr => "Aucune session sauvegardée trouvée.",
        Lang::En => "No saved session found.",
    }
}

#[cfg(test)]
#[path = "slash_reload/tests.rs"]
mod tests;
