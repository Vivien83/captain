use super::{parse_model_switch_args, path_segment};

#[test]
fn parse_model_switch_args_keeps_first_token_as_model() {
    assert_eq!(parse_model_switch_args("gpt-5.5"), ("gpt-5.5", None));
    assert_eq!(
        parse_model_switch_args("  codex/gpt-5.5   "),
        ("codex/gpt-5.5", None)
    );
}

#[test]
fn parse_model_switch_args_detects_session_strategy_flags() {
    assert_eq!(
        parse_model_switch_args("gpt-5 --new"),
        ("gpt-5", Some("new_session"))
    );
    assert_eq!(
        parse_model_switch_args("gpt-5 --compact-session"),
        ("gpt-5", Some("compact_session"))
    );
}

#[test]
fn parse_model_switch_args_ignores_unknown_flags_and_preserves_legacy_edge() {
    assert_eq!(parse_model_switch_args("--new"), ("--new", None));
    assert_eq!(
        parse_model_switch_args("gpt-5 --unknown --new-session"),
        ("gpt-5", Some("new_session"))
    );
}

#[test]
fn path_segment_keeps_url_safe_ascii() {
    assert_eq!(path_segment("project-1._~AZaz09"), "project-1._~AZaz09");
}

#[test]
fn path_segment_percent_encodes_spaces_slashes_and_utf8_bytes() {
    assert_eq!(path_segment("hello world/é"), "hello%20world%2F%C3%A9");
}
