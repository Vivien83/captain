use std::fmt;

pub(crate) fn is_protected_agent(name: &str) -> bool {
    name.trim().eq_ignore_ascii_case("captain")
}

pub(crate) fn protected_agent_message(lang: crate::i18n::Lang) -> &'static str {
    crate::i18n::t("kill.captain_protected", lang)
}

pub(crate) fn kill_success_message(lang: crate::i18n::Lang, name: &str) -> String {
    crate::i18n::t("kill.success", lang).replace("{name}", name)
}

pub(crate) fn kill_failed_message(lang: crate::i18n::Lang, name: &str) -> String {
    crate::i18n::t("kill.failed", lang).replace("{name}", name)
}

pub(crate) fn kill_error_message(lang: crate::i18n::Lang, err: impl fmt::Display) -> String {
    crate::i18n::t("kill.error", lang).replace("{err}", &err.to_string())
}

pub(crate) fn no_backend_message(lang: crate::i18n::Lang) -> &'static str {
    crate::i18n::t("chat.no_backend", lang)
}

#[cfg(test)]
#[path = "slash_kill/tests.rs"]
mod tests;
