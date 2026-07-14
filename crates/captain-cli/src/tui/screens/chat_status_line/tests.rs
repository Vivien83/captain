use super::*;
use std::time::Duration;

fn line_text(line: Line<'static>) -> String {
    line.spans
        .into_iter()
        .map(|span| span.content.into_owned())
        .collect::<String>()
}

#[test]
fn compact_token_count_keeps_small_counts_exact() {
    assert_eq!(compact_token_count(999), "999 tok");
}

#[test]
fn compact_token_count_uses_one_decimal_below_ten_k() {
    assert_eq!(compact_token_count(1_250), "1.2k tok");
}

#[test]
fn compact_token_count_rounds_large_counts() {
    assert_eq!(compact_token_count(12_500), "12k tok");
}

#[test]
fn duration_label_formats_minutes_and_seconds() {
    assert_eq!(duration_label(Duration::from_secs(65)), "1m05s");
}

#[test]
fn token_usage_label_reports_effective_cached_input() {
    assert_eq!(
        token_usage_label(1_500, 250, 500),
        "1500\u{2191} 250\u{2193} · eff 1.0k tok"
    );
}

#[test]
fn status_line_includes_model_mode_tokens_and_cost() {
    let mut state = ChatState::new();
    state.model_label = "codex/gpt-5.5".to_string();
    state.mode_label = "daemon".to_string();
    state.last_tokens = Some((1_500, 250));
    state.last_cached_input_tokens = 500;
    state.last_cost_usd = Some(0.0123);
    state.session_input_tokens = 2_000;
    state.session_output_tokens = 1_000;
    state.session_cost_usd = 0.0456;

    let text = line_text(build_status_line(&state));

    assert!(text.contains("codex/gpt-5.5"));
    assert!(text.contains("daemon"));
    assert!(text.contains("1500\u{2191} 250\u{2193} · eff 1.0k tok"));
    assert!(text.contains("$0.0123"));
    assert!(text.contains("\u{03A3} 3000 tok"));
    assert!(text.contains("/ $0.0456"));
}

#[test]
fn status_line_shows_spinner_while_streaming() {
    let mut state = ChatState::new();
    state.is_streaming = true;
    state.spinner_frame = 2;

    let text = line_text(build_status_line(&state));

    assert!(text.contains(theme::SPINNER_FRAMES[2]));
}

#[test]
fn status_line_hides_background_badge_when_nothing_in_flight() {
    let state = ChatState::new();
    let text = line_text(build_status_line(&state));
    assert!(!text.contains("en arrière-plan"));
}

#[test]
fn status_line_shows_background_badge_with_count() {
    let mut state = ChatState::new();
    state.track_background_activity("agent-1".to_string(), "agent researcher".to_string());
    state.track_background_activity("toolrun-1".to_string(), "tool_run shell_exec".to_string());

    let text = line_text(build_status_line(&state));

    assert!(text.contains("2 en arrière-plan"));
}

#[test]
fn status_line_background_badge_disappears_once_cleared() {
    let mut state = ChatState::new();
    state.track_background_activity("agent-1".to_string(), "agent researcher".to_string());
    state.clear_background_activity("agent-1");

    let text = line_text(build_status_line(&state));

    assert!(!text.contains("en arrière-plan"));
}
