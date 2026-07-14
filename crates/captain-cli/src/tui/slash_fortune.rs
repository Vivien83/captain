use crate::i18n::Lang;

pub(crate) const FORTUNE_KEYS: [&str; 5] = [
    "fortune.quote.1",
    "fortune.quote.2",
    "fortune.quote.3",
    "fortune.quote.4",
    "fortune.quote.5",
];

pub(crate) fn quote_key_for_timestamp_secs(timestamp_secs: u64) -> &'static str {
    let idx = (timestamp_secs % FORTUNE_KEYS.len() as u64) as usize;
    FORTUNE_KEYS[idx]
}

pub(crate) fn fortune_message_for_timestamp_secs(timestamp_secs: u64, lang: Lang) -> &'static str {
    crate::i18n::t(quote_key_for_timestamp_secs(timestamp_secs), lang)
}

#[cfg(test)]
#[path = "slash_fortune/tests.rs"]
mod tests;
