use regex::Regex;
use regex_lite::Regex as LiteRegex;
use std::{io, sync::LazyLock};

static PRIVATE_KEY_BLOCK: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?s)-----BEGIN[A-Z ]*PRIVATE KEY-----.*?-----END[A-Z ]*PRIVATE KEY-----")
        .expect("private-key regex")
});
static NAMED_SECRET: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r#"(?i)([\"']?(?:api[_-]?key|access[_-]?key|access[_-]?token|refresh[_-]?token|password|passwd|secret|authorization|cookie|session[_-]?token)[\"']?\s*[:=]\s*[\"']?)([^\"'\s,;]+)"#,
    )
    .expect("named-secret regex")
});
static BEARER_TOKEN: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)\bBearer\s+[A-Za-z0-9_\-\./+=]{12,}").expect("bearer regex"));
static KNOWN_TOKEN: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"(?:sk-[A-Za-z0-9_-]{16,}|gh[pousr]_[A-Za-z0-9_]{20,}|github_pat_[A-Za-z0-9_]{20,}|xox[baprs]-[A-Za-z0-9-]{10,}|AKIA[A-Z0-9]{16}|\b\d{6,12}:[A-Za-z0-9_-]{30,}\b)",
    )
    .expect("known-token regex")
});

static SECRET_RULES: LazyLock<Vec<(&'static str, LiteRegex)>> = LazyLock::new(|| {
    vec![
        (
            "anthropic_api_key",
            LiteRegex::new(r"sk-ant-[a-zA-Z0-9_\-]{40,}").unwrap(),
        ),
        (
            "openai_api_key",
            LiteRegex::new(r"sk-(proj-)?[a-zA-Z0-9]{32,}").unwrap(),
        ),
        (
            "openrouter_api_key",
            LiteRegex::new(r"sk-or-v1-[A-Za-z0-9]{32,}").unwrap(),
        ),
        (
            "groq_api_key",
            LiteRegex::new(r"gsk_[A-Za-z0-9]{20,}").unwrap(),
        ),
        (
            "aws_access_key",
            LiteRegex::new(r"AKIA[0-9A-Z]{16}").unwrap(),
        ),
        (
            "google_api_key",
            LiteRegex::new(r"AIza[0-9A-Za-z_\-]{35}").unwrap(),
        ),
        (
            "elevenlabs_api_key",
            LiteRegex::new(r"\bxi-[A-Za-z0-9]{20,}\b").unwrap(),
        ),
        (
            "github_token",
            LiteRegex::new(r"gh[pousr]_[A-Za-z0-9]{36,}").unwrap(),
        ),
        (
            "github_fine_grained_token",
            LiteRegex::new(r"github_pat_[A-Za-z0-9_]{30,}").unwrap(),
        ),
        (
            "stripe_key",
            LiteRegex::new(r"(sk|pk)_live_[A-Za-z0-9]{24,}").unwrap(),
        ),
        (
            "slack_token",
            LiteRegex::new(r"xox[baprs]-[A-Za-z0-9\-]{10,}").unwrap(),
        ),
        ("twilio_sid", LiteRegex::new(r"AC[a-fA-F0-9]{32}").unwrap()),
        (
            "discord_bot_token",
            LiteRegex::new(r"[MN][A-Za-z\d]{23}\.[\w\-]{6}\.[\w\-]{27}").unwrap(),
        ),
        (
            "telegram_bot_token",
            LiteRegex::new(r"\b\d{6,12}:[A-Za-z0-9_\-]{30,}\b").unwrap(),
        ),
        (
            "jwt",
            LiteRegex::new(r"eyJ[A-Za-z0-9_\-]+\.[A-Za-z0-9_\-]+\.[A-Za-z0-9_\-]+").unwrap(),
        ),
        (
            "bearer_token",
            LiteRegex::new(r"(?i)Bearer\s+[A-Za-z0-9_\-\.=]{20,}").unwrap(),
        ),
        (
            "ssh_private_key",
            LiteRegex::new(r"-----BEGIN[A-Z ]*PRIVATE KEY-----").unwrap(),
        ),
        (
            "env_assignment",
            LiteRegex::new(
                r"(?i)(API_KEY|SECRET|PASSWORD|TOKEN|ACCESS_KEY)\s*=\s*[A-Za-z0-9/+_\-\.=]{12,}",
            )
            .unwrap(),
        ),
        (
            "credit_card_candidate",
            LiteRegex::new(r"\b(?:\d[ \-]?){13,19}\b").unwrap(),
        ),
    ]
});

pub(crate) fn ensure_no_secret_literal(text: &str) -> Result<(), String> {
    if scan_for_secrets(text).is_some() {
        return Err(
            "Security blocked a literal secret-looking value in local Node input".to_string(),
        );
    }
    Ok(())
}

pub(crate) fn sanitize_for_retention(input: &str) -> io::Result<(String, bool)> {
    let stripped = strip_ansi(input);
    let redacted = PRIVATE_KEY_BLOCK.replace_all(&stripped, "[REDACTED PRIVATE KEY]");
    let redacted = NAMED_SECRET.replace_all(&redacted, "$1[REDACTED]");
    let redacted = BEARER_TOKEN.replace_all(&redacted, "Bearer [REDACTED]");
    let redacted = KNOWN_TOKEN.replace_all(&redacted, "[REDACTED TOKEN]");
    if let Some(kind) = scan_for_secrets(&redacted) {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            format!("Node output still matches secret policy: {kind}"),
        ));
    }
    let sanitized = redacted.into_owned();
    let changed = stripped != input || sanitized != stripped;
    Ok((sanitized, changed))
}

fn scan_for_secrets(text: &str) -> Option<&'static str> {
    for (label, regex) in SECRET_RULES.iter() {
        if let Some(found) = regex.find(text) {
            if *label == "credit_card_candidate" {
                let digits: String = found
                    .as_str()
                    .chars()
                    .filter(char::is_ascii_digit)
                    .collect();
                if !luhn_valid(&digits) {
                    continue;
                }
                return Some("credit_card");
            }
            return Some(label);
        }
    }
    None
}

fn luhn_valid(digits: &str) -> bool {
    if !(13..=19).contains(&digits.len()) {
        return false;
    }
    let mut sum = 0u32;
    for (index, character) in digits.chars().rev().enumerate() {
        let Some(digit) = character.to_digit(10) else {
            return false;
        };
        let doubled = if index % 2 == 1 { digit * 2 } else { digit };
        sum += if doubled > 9 { doubled - 9 } else { doubled };
    }
    sum.is_multiple_of(10)
}

fn strip_ansi(input: &str) -> String {
    #[derive(Clone, Copy)]
    enum State {
        Text,
        Escape,
        Csi,
        Osc,
        OscEscape,
        ControlString,
        ControlStringEscape,
    }

    let mut output = String::with_capacity(input.len());
    let mut state = State::Text;
    for character in input.chars() {
        state = match state {
            State::Text => match character {
                '\u{1b}' => State::Escape,
                '\n' | '\r' | '\t' => {
                    output.push(character);
                    State::Text
                }
                value if value.is_control() => State::Text,
                _ => {
                    output.push(character);
                    State::Text
                }
            },
            State::Escape => match character {
                '[' => State::Csi,
                ']' => State::Osc,
                'P' | 'X' | '^' | '_' => State::ControlString,
                _ => State::Text,
            },
            State::Csi if ('@'..='~').contains(&character) => State::Text,
            State::Csi => State::Csi,
            State::Osc if character == '\u{7}' => State::Text,
            State::Osc if character == '\u{1b}' => State::OscEscape,
            State::Osc => State::Osc,
            State::OscEscape if character == '\\' => State::Text,
            State::OscEscape if character == '\u{1b}' => State::OscEscape,
            State::OscEscape => State::Osc,
            State::ControlString if character == '\u{1b}' => State::ControlStringEscape,
            State::ControlString => State::ControlString,
            State::ControlStringEscape if character == '\\' => State::Text,
            State::ControlStringEscape if character == '\u{1b}' => State::ControlStringEscape,
            State::ControlStringEscape => State::ControlString,
        };
    }
    output
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn output_is_redacted_and_terminal_controls_are_removed() {
        let input = "\u{1b}[31mpassword=very-secret-value\u{1b}[0m";
        let (output, changed) = sanitize_for_retention(input).unwrap();
        assert!(changed);
        assert_eq!(output, "password=[REDACTED]");
    }
}
