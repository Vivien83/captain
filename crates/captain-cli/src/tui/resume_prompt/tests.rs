use super::{format_relative_age, session_store, summary_lines};
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

#[test]
fn relative_age_just_now() {
    assert_eq!(format_relative_age(now()), "à l'instant");
}

#[test]
fn relative_age_minutes() {
    let s = format_relative_age(now().saturating_sub(180));
    assert_eq!(s, "il y a 3m");
}

#[test]
fn relative_age_hours() {
    let s = format_relative_age(now().saturating_sub(2 * 3600));
    assert_eq!(s, "il y a 2h");
}

#[test]
fn relative_age_days() {
    let s = format_relative_age(now().saturating_sub(3 * 86_400));
    assert_eq!(s, "il y a 3j");
}

#[test]
fn relative_age_handles_future_timestamp_gracefully() {
    let s = format_relative_age(now() + 1_000_000);
    assert_eq!(s, "à l'instant");
}

#[test]
fn summary_lines_include_agent_pluralized_messages_and_file_name() {
    let summary = session_store::SessionSummary {
        agent_key: "daemon-a".to_string(),
        session_id: Some("aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa".to_string()),
        label: "Ops session".to_string(),
        agent_name: "Ops".to_string(),
        model_label: "codex/gpt-5".to_string(),
        path: PathBuf::from("/tmp/captain-session.json"),
        updated_at: now().saturating_sub(180),
        message_count: 2,
        session_input_tokens: 0,
        session_output_tokens: 0,
    };

    let (header, hint) = summary_lines(Some(&summary));

    assert_eq!(header, "Ops · 2 messages · il y a 3m");
    assert_eq!(hint, "session: captain-session.json");
}

#[test]
fn summary_lines_keep_fallback_defensive() {
    let (header, hint) = summary_lines(None);

    assert_eq!(header, "(aucune session disponible)");
    assert_eq!(hint, "");
}
