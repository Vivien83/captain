use super::*;

#[test]
fn narrow_width_skips_summary_before_file_reads() {
    let state = ChatState::new();

    assert!(welcome_summary_lines(&state, WELCOME_SUMMARY_WIDTH + 3).is_empty());
}

#[test]
fn summary_rows_include_live_state_config_memory_and_project() {
    let rows = welcome_summary_rows(
        "captain",
        "codex/gpt-5",
        vec!["telegram".into(), "discord".into()],
        Vec::new(),
        1536,
        Some("atlas".into()),
    );

    assert_eq!(
        rows,
        vec![
            ("agent".into(), "captain".into()),
            ("provider".into(), "codex/gpt-5".into()),
            ("canaux".into(), "telegram, discord".into()),
            ("mémoire".into(), "1.5 KB".into()),
            ("projet".into(), "atlas".into()),
        ]
    );
}

#[test]
fn summary_rows_reports_orphan_tokens_without_declared_channel() {
    let rows = welcome_summary_rows(
        "",
        "",
        vec!["telegram".into()],
        vec!["discord".into()],
        0,
        None,
    );

    assert_eq!(
        rows,
        vec![("canaux".into(), "telegram (orphans: discord)".into())]
    );
}

#[test]
fn declared_channels_filters_frozen_sections() {
    let config = r#"
        [channels.telegram]
        allowed_users = ["123"]

        [channels.whatsapp]
        phone_number_id = "legacy"

        [channels.slack]
        default_agent = "captain"

        [channels.email]
        username = "captain@example.com"
    "#
    .parse::<toml::Value>()
    .unwrap();

    assert_eq!(
        declared_channels(Some(&config)),
        vec!["email".to_string(), "telegram".to_string()]
    );
}

#[test]
fn orphan_token_scan_ignores_frozen_channel_tokens() {
    let names = channel_tokens()
        .into_iter()
        .map(|(name, _)| name)
        .collect::<Vec<_>>();

    assert_eq!(names, vec!["telegram", "discord", "email"]);
    assert!(!names.contains(&"slack"));
    assert!(!names.contains(&"whatsapp"));
}

#[test]
fn format_bytes_uses_compact_units() {
    assert_eq!(format_bytes(42), "42 B");
    assert_eq!(format_bytes(1024), "1.0 KB");
    assert_eq!(format_bytes(1_048_576), "1.0 MB");
    assert_eq!(format_bytes(1_073_741_824), "1.0 GB");
}
