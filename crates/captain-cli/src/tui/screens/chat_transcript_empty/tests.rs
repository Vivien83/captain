use super::*;

fn line_texts(lines: &[Line<'static>]) -> Vec<String> {
    lines
        .iter()
        .map(|line| {
            line.spans
                .iter()
                .map(|span| span.content.as_ref())
                .collect()
        })
        .collect()
}

#[test]
fn narrow_width_skips_logo() {
    assert!(captain_logo_lines(6).is_empty());
}

#[test]
fn compact_identity_uses_plain_wordmark_and_version_only() {
    let texts = line_texts(&captain_logo_lines(80));

    assert!(texts.iter().any(|line| line.trim() == "CAPTAIN"));
    assert!(texts.iter().any(|line| line.trim().starts_with('v')));
    assert!(texts.iter().all(|line| !line.contains("Unleash")));
    assert_eq!(texts.len(), 4);
}

#[test]
fn empty_transcript_adds_summary_and_default_prompts() {
    let lines = build_empty_transcript_lines(
        vec![Line::from("logo")],
        1,
        vec![Line::from("summary")],
        "primary".to_string(),
        "secondary".to_string(),
        0,
    );

    assert_eq!(
        line_texts(&lines),
        vec!["logo", "summary", "", "primary", "secondary"]
    );
}

#[test]
fn bottom_anchor_padding_is_inserted_after_logo() {
    let lines = build_empty_transcript_lines(
        vec![Line::from("logo-a"), Line::from("logo-b")],
        2,
        vec![Line::from("summary")],
        "primary".to_string(),
        "secondary".to_string(),
        8,
    );

    assert_eq!(
        line_texts(&lines),
        vec![
            "logo-a",
            "logo-b",
            "",
            "",
            "summary",
            "",
            "primary",
            "secondary"
        ]
    );
}

#[test]
fn no_padding_when_content_exceeds_visible_height() {
    let lines = build_empty_transcript_lines(
        vec![Line::from("logo")],
        1,
        vec![Line::from("summary")],
        "primary".to_string(),
        "secondary".to_string(),
        2,
    );

    assert_eq!(
        line_texts(&lines),
        vec!["logo", "summary", "", "primary", "secondary"]
    );
}
