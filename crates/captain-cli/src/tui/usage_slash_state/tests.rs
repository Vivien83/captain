use super::*;

fn snapshot() -> UsageSlashSnapshot {
    UsageSlashSnapshot {
        session_input_tokens: 120,
        session_output_tokens: 80,
        session_cached_input_tokens: 30,
        session_cache_creation_tokens: 7,
        last_tokens: Some((40, 20)),
        last_cached_input_tokens: 10,
        session_cost_usd: 0.12345,
        last_cost_usd: Some(0.042),
        message_count: 4,
    }
}

#[test]
fn full_tui_token_usage_keeps_existing_french_average_format() {
    assert_eq!(
        token_usage_message(snapshot(), UsageSlashSurface::FullTui, Lang::Fr),
        "Session : 120↑ in / 80↓ out  (total 200)\nMoyenne par tour : ~30 in / ~20 out\nCache : 30 input cached / 7 creation\nDernier tour : 40↑ 20↓ dont 10 cached"
    );
}

#[test]
fn full_tui_zero_tokens_keeps_existing_french_message() {
    let mut usage = snapshot();
    usage.session_input_tokens = 0;
    usage.session_output_tokens = 0;
    assert_eq!(
        token_usage_message(usage, UsageSlashSurface::FullTui, Lang::En),
        "Aucun token consommé pour cette session encore."
    );
}

#[test]
fn standalone_token_usage_supports_english_hermes_format() {
    assert_eq!(
        token_usage_message(snapshot(), UsageSlashSurface::StandaloneChat, Lang::En),
        "Session: 120↑ in / 80↓ out (total 200)\nCache: 30 input cached / 7 creation\nLast turn: 40↑ 20↓ including 10 cached"
    );
}

#[test]
fn standalone_token_usage_supports_french_hermes_format() {
    let mut usage = snapshot();
    usage.session_cached_input_tokens = 0;
    usage.last_cached_input_tokens = 0;
    assert_eq!(
        token_usage_message(usage, UsageSlashSurface::StandaloneChat, Lang::Fr),
        "Session : 120↑ in / 80↓ out (total 200)\nDernier tour : 40↑ 20↓"
    );
}

#[test]
fn standalone_zero_tokens_supports_english_hermes_message() {
    let mut usage = snapshot();
    usage.session_input_tokens = 0;
    usage.session_output_tokens = 0;
    assert_eq!(
        token_usage_message(usage, UsageSlashSurface::StandaloneChat, Lang::En),
        "No tokens consumed in this session yet."
    );
}

#[test]
fn cost_usage_keeps_full_tui_french_format() {
    assert_eq!(
        cost_usage_message(snapshot(), UsageSlashSurface::FullTui, Lang::En),
        "Coût session : $0.1235\nDernier tour : $0.0420"
    );
}

#[test]
fn cost_usage_supports_standalone_english_format() {
    assert_eq!(
        cost_usage_message(snapshot(), UsageSlashSurface::StandaloneChat, Lang::En),
        "Session cost: $0.1235\nLast turn: $0.0420"
    );
}

#[test]
fn zero_cost_keeps_surface_message() {
    let mut usage = snapshot();
    usage.session_cost_usd = 0.0;
    assert_eq!(
        cost_usage_message(usage, UsageSlashSurface::StandaloneChat, Lang::En),
        "Session cost: $0.0000."
    );
    assert_eq!(
        cost_usage_message(usage, UsageSlashSurface::FullTui, Lang::En),
        "Coût session : $0.0000 (aucun usage facturé pour l'instant)."
    );
}
