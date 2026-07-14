use super::screens::chat::{ChatMessage, Role};
use crate::i18n::Lang;

pub(crate) fn last_user_message(messages: &[ChatMessage]) -> Option<String> {
    messages
        .iter()
        .rev()
        .find(|message| message.role == Role::User)
        .map(|message| message.text.clone())
}

pub(crate) fn retry_nothing_message(lang: Lang) -> &'static str {
    crate::i18n::t("retry.nothing", lang)
}

#[cfg(test)]
#[path = "slash_retry/tests.rs"]
mod tests;
