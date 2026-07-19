use super::*;
use crate::tui::provider_quota::{
    ProviderCredits, ProviderQuota, ProviderQuotaStatus, ProviderQuotaWindow,
};
use chrono::{FixedOffset, TimeZone, Utc};
use std::time::Duration;

fn line_text(line: Line<'static>) -> String {
    line.spans
        .into_iter()
        .map(|span| span.content.into_owned())
        .collect::<String>()
}

fn quota_lines_text(lines: &[Line<'static>]) -> String {
    lines
        .iter()
        .map(|line| {
            line.spans
                .iter()
                .map(|span| span.content.as_ref())
                .collect::<String>()
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn codex_quota(name: &str, percent: f64, window_seconds: u64, reset: &str) -> ProviderQuota {
    ProviderQuota {
        provider: "codex".to_string(),
        limit_id: name.to_lowercase(),
        limit_name: name.to_string(),
        plan_type: Some("pro".to_string()),
        alert_level: if percent >= 70.0 { "warning" } else { "normal" }.to_string(),
        stale: false,
        primary: Some(ProviderQuotaWindow {
            used_percent: percent,
            window_seconds: Some(window_seconds),
            reset_after_seconds: None,
            resets_at: Some(
                chrono::DateTime::parse_from_rfc3339(reset)
                    .unwrap()
                    .with_timezone(&Utc),
            ),
        }),
        secondary: None,
        credits: Some(ProviderCredits {
            has_credits: true,
            unlimited: false,
            balance: Some("17.50".to_string()),
        }),
        rate_limit_reached_type: None,
        observed_at: Some(Utc::now()),
    }
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

#[test]
fn provider_status_band_prioritizes_the_active_model_over_alternative_limits() {
    let mut state = ChatState::new();
    state.model_label = "codex/gpt-5.6-sol".to_string();
    state.provider_quota_status = ProviderQuotaStatus {
        state: "warning".to_string(),
        reported_by_provider: true,
        quotas: vec![
            codex_quota("Codex", 63.0, 604_800, "2026-07-25T20:00:00Z"),
            codex_quota("GPT-5.3-Codex-Spark", 5.0, 604_800, "2026-07-25T20:00:00Z"),
        ],
    };

    let lines = build_provider_quota_lines(&state, 180);
    let text = quota_lines_text(&lines);

    assert!(text.contains("Actif gpt-5.6-sol · Codex [pro]"), "{text}");
    assert!(text.contains("crédits 17.50"), "{text}");
    assert!(text.contains("Codex 1sem"), "{text}");
    assert!(!text.contains("GPT-5.3-Codex-Spark"), "{text}");
    assert!(text.contains("+1 quota annexe"), "{text}");
    assert!(text.contains("hors modèle actif"), "{text}");
    assert!(text.contains("63%"), "{text}");
    assert!(!text.contains("5%"), "{text}");
    assert_eq!(text.matches('[').count(), 2, "plan plus one gauge: {text}");
    assert_eq!(text.matches('↻').count(), 1, "{text}");
}

#[test]
fn provider_status_band_promotes_a_quota_matching_the_active_model() {
    let mut state = ChatState::new();
    state.model_label = "codex/gpt-5.3-codex-spark".to_string();
    state.provider_quota_status = ProviderQuotaStatus {
        state: "ok".to_string(),
        reported_by_provider: true,
        quotas: vec![codex_quota(
            "GPT-5.3-Codex-Spark",
            5.0,
            604_800,
            "2026-07-25T20:00:00Z",
        )],
    };

    let text = quota_lines_text(&build_provider_quota_lines(&state, 180));

    assert!(text.contains("Actif gpt-5.3-codex-spark"), "{text}");
    assert!(text.contains("GPT-5.3-Codex-Spark 1sem"), "{text}");
    assert!(!text.contains("quota annexe"), "{text}");
}

#[test]
fn provider_status_band_surfaces_critical_pressure_without_a_false_gauge() {
    let mut state = ChatState::new();
    state.model_label = "codex/gpt-5.6-sol".to_string();
    let mut spark = codex_quota("GPT-5.3-Codex-Spark", 95.0, 604_800, "2026-07-25T20:00:00Z");
    spark.alert_level = "critical".to_string();
    state.provider_quota_status = ProviderQuotaStatus {
        state: "critical".to_string(),
        reported_by_provider: true,
        quotas: vec![
            codex_quota("Codex", 63.0, 604_800, "2026-07-25T20:00:00Z"),
            spark,
        ],
    };

    let text = quota_lines_text(&build_provider_quota_lines(&state, 180));

    assert!(text.contains("+1 quota annexe critique"), "{text}");
    assert!(text.contains("hors modèle actif"), "{text}");
    assert!(!text.contains("GPT-5.3-Codex-Spark"), "{text}");
    assert!(!text.contains("95%"), "{text}");
    assert_eq!(text.matches('↻').count(), 1, "{text}");
}

#[test]
fn provider_status_band_is_width_safe_for_terminal_and_xterm() {
    let mut state = ChatState::new();
    state.model_label = "codex/gpt-5.6-sol".to_string();
    state.provider_quota_status = ProviderQuotaStatus {
        state: "ok".to_string(),
        reported_by_provider: true,
        quotas: vec![{
            let mut quota = codex_quota(
                "A-provider-limit-name-that-is-intentionally-long",
                42.0,
                18_000,
                "2026-07-19T20:00:00Z",
            );
            quota.limit_id = "codex".to_string();
            quota
        }],
    };

    let lines = build_provider_quota_lines(&state, 36);

    assert!(!lines.is_empty());
    assert!(lines.len() <= MAX_PROVIDER_STATUS_ROWS);
    for line in lines {
        let text = line_text(line);
        assert!(UnicodeWidthStr::width(text.as_str()) <= 36, "{text:?}");
    }
    assert_eq!(UnicodeWidthStr::width("[████░░░░] ↻"), 12);
}

#[test]
fn provider_status_overflow_summary_stays_inside_narrow_terminals() {
    let mut state = ChatState::new();
    state.model_label = "codex/gpt-5.6-sol".to_string();
    state.provider_quota_status = ProviderQuotaStatus {
        state: "warning".to_string(),
        reported_by_provider: true,
        quotas: (0..7)
            .map(|index| {
                let mut quota = codex_quota(
                    &format!("Codex-{index}"),
                    50.0 + index as f64,
                    18_000,
                    "2026-07-19T20:00:00Z",
                );
                quota.limit_id = "codex".to_string();
                quota
            })
            .collect(),
    };

    let lines = build_provider_quota_lines(&state, 24);
    assert_eq!(lines.len(), MAX_PROVIDER_STATUS_ROWS);
    for line in lines {
        let text = line_text(line);
        assert!(UnicodeWidthStr::width(text.as_str()) <= 24, "{text:?}");
    }
}

#[test]
fn provider_resume_uses_the_local_recovery_time() {
    let offset = FixedOffset::east_opt(2 * 3600).unwrap();
    let now = offset.with_ymd_and_hms(2026, 7, 18, 21, 0, 0).unwrap();
    let window = ProviderQuotaWindow {
        used_percent: 100.0,
        window_seconds: Some(18_000),
        reset_after_seconds: None,
        resets_at: Some(Utc.with_ymd_and_hms(2026, 7, 18, 22, 30, 0).unwrap()),
    };

    assert_eq!(provider_resume_label(&window, now, false), "demain 00:30");
}

#[test]
fn unavailable_quota_is_visible_only_for_an_active_codex_model() {
    let mut state = ChatState::new();
    state.model_label = "anthropic/claude".to_string();
    assert!(build_provider_quota_lines(&state, 100).is_empty());

    state.model_label = "codex/gpt-5.6-sol".to_string();
    let text = quota_lines_text(&build_provider_quota_lines(&state, 100));
    assert!(text.contains("non observé"), "{text}");
    assert!(!text.contains("illimité"), "{text}");
    let compact = quota_lines_text(&build_provider_quota_lines(&state, 20));
    assert!(
        UnicodeWidthStr::width(compact.as_str()) <= 20,
        "{compact:?}"
    );
}
