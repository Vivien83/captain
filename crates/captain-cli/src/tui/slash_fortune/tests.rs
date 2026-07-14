use super::*;

#[test]
fn timestamp_selects_quote_by_modulo() {
    assert_eq!(quote_key_for_timestamp_secs(0), "fortune.quote.1");
    assert_eq!(quote_key_for_timestamp_secs(1), "fortune.quote.2");
    assert_eq!(quote_key_for_timestamp_secs(4), "fortune.quote.5");
    assert_eq!(quote_key_for_timestamp_secs(5), "fortune.quote.1");
    assert_eq!(quote_key_for_timestamp_secs(6), "fortune.quote.2");
}

#[test]
fn large_timestamp_stays_in_known_key_set() {
    let key = quote_key_for_timestamp_secs(u64::MAX);

    assert!(FORTUNE_KEYS.contains(&key));
}

#[test]
fn fortune_message_preserves_hermes_i18n_text() {
    assert_eq!(
        fortune_message_for_timestamp_secs(0, crate::i18n::Lang::Fr),
        "L'ennemi de mon art, c'est l'improvisation sans préparation."
    );
    assert_eq!(
        fortune_message_for_timestamp_secs(4, crate::i18n::Lang::En),
        "The best abstraction is the one you can delete painlessly."
    );
}
