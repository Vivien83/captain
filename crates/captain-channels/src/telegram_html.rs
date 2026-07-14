//! Telegram HTML sanitizing and plain-text fallback helpers.

pub(crate) fn telegram_html_to_plain_text(text: &str) -> String {
    let mut plain = String::with_capacity(text.len());
    let mut offset = 0;

    while let Some(start_rel) = text[offset..].find('<') {
        let start = offset + start_rel;
        plain.push_str(&text[offset..start]);
        let Some(end_rel) = text[start..].find('>') else {
            plain.push_str(&text[start..]);
            offset = text.len();
            break;
        };
        let end = start + end_rel;
        let tag_content = &text[start + 1..end];
        if !is_allowed_telegram_html_tag(tag_content) {
            plain.push_str(&text[start..=end]);
        }
        offset = end + 1;
    }

    plain.push_str(&text[offset..]);
    plain
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
        .replace("&amp;", "&")
}

/// Sanitize text for Telegram HTML parse mode.
///
/// Escapes angle brackets that are not part of Telegram-allowed HTML tags.
/// Allowed tags include formatting/code/blockquote/spoiler tags accepted by
/// Telegram. Everything else, including operational markers, is escaped.
pub(crate) fn sanitize_telegram_html(text: &str) -> String {
    let mut result = String::with_capacity(text.len());
    let mut offset = 0;

    while let Some(start_rel) = text[offset..].find('<') {
        let start = offset + start_rel;
        push_escaped_telegram_text(&mut result, &text[offset..start]);
        let Some(end_rel) = text[start..].find('>') else {
            push_escaped_telegram_text(&mut result, &text[start..]);
            offset = text.len();
            break;
        };
        let end = start + end_rel;
        let tag_content = &text[start + 1..end];
        if is_allowed_telegram_html_tag(tag_content) {
            result.push_str(&text[start..=end]);
        } else {
            push_escaped_telegram_text(&mut result, &text[start..=end]);
        }
        offset = end + 1;
    }

    push_escaped_telegram_text(&mut result, &text[offset..]);
    result
}

fn is_allowed_telegram_html_tag(tag_content: &str) -> bool {
    const ALLOWED: &[&str] = &[
        "b",
        "i",
        "u",
        "s",
        "em",
        "strong",
        "a",
        "code",
        "pre",
        "blockquote",
        "tg-spoiler",
        "tg-emoji",
    ];

    if tag_content.contains('<') {
        return false;
    }
    let trimmed = tag_content.trim();
    let without_slash = trimmed.strip_prefix('/').unwrap_or(trimmed).trim_start();
    let tag_name = without_slash
        .split(|c: char| c.is_whitespace() || c == '/' || c == '>')
        .next()
        .unwrap_or("")
        .to_lowercase();
    !tag_name.is_empty() && ALLOWED.contains(&tag_name.as_str())
}

fn push_escaped_telegram_text(out: &mut String, text: &str) {
    let mut offset = 0;
    while offset < text.len() {
        let remaining = &text[offset..];
        if remaining.starts_with('&') {
            if let Some(entity_len) = telegram_entity_len(remaining) {
                out.push_str(&remaining[..entity_len]);
                offset += entity_len;
            } else {
                out.push_str("&amp;");
                offset += 1;
            }
            continue;
        }
        let ch = remaining
            .chars()
            .next()
            .expect("offset is always inside a non-empty UTF-8 slice");
        match ch {
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            _ => out.push(ch),
        }
        offset += ch.len_utf8();
    }
}

fn telegram_entity_len(text: &str) -> Option<usize> {
    for entity in ["&lt;", "&gt;", "&amp;", "&quot;", "&#39;"] {
        if text.starts_with(entity) {
            return Some(entity.len());
        }
    }
    let rest = text.strip_prefix("&#")?;
    let end = rest.find(';')?;
    if end == 0 || end > 12 {
        return None;
    }
    let value = &rest[..end];
    let valid = if let Some(hex) = value.strip_prefix(['x', 'X']) {
        !hex.is_empty() && hex.chars().all(|c| c.is_ascii_hexdigit())
    } else {
        value.chars().all(|c| c.is_ascii_digit())
    };
    valid.then_some(2 + end + 1)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn telegram_html_sanitize_preserves_allowed_and_escapes_unknown_tags() {
        let input = "<b>bold</b> <thinking>hmm</thinking>";
        let output = sanitize_telegram_html(input);

        assert!(output.contains("<b>bold</b>"));
        assert!(output.contains("&lt;thinking&gt;"));
    }

    #[test]
    fn telegram_html_sanitize_escapes_external_content_markers() {
        let input = "Source <<<EXTCONTENT_057e50f8d39c>>> fin";
        let output = sanitize_telegram_html(input);

        assert!(!output.contains("<<<EXTCONTENT"));
        assert!(output.contains("&lt;&lt;&lt;EXTCONTENT_057e50f8d39c&gt;&gt;&gt;"));
    }

    #[test]
    fn telegram_html_sanitize_escapes_entity_closed_unknown_tag() {
        let input = "bad <extcontent_057e50f8d39c&gt; marker";
        let output = sanitize_telegram_html(input);

        assert!(!output.contains("<extcontent"));
        assert!(output.contains("&lt;extcontent_057e50f8d39c&gt;"));
    }

    #[test]
    fn telegram_html_to_plain_text_decodes_entities() {
        let input = "<b>bold</b> &lt;thinking&gt; A &amp; B";

        assert_eq!(telegram_html_to_plain_text(input), "bold <thinking> A & B");
    }

    #[test]
    fn telegram_html_to_plain_text_preserves_unknown_tags() {
        let input = "Source &lt;&lt;&lt;EXTCONTENT_abc&gt;&gt;&gt; <thinking>hmm</thinking>";

        assert_eq!(
            telegram_html_to_plain_text(input),
            "Source <<<EXTCONTENT_abc>>> <thinking>hmm</thinking>"
        );
    }

    #[test]
    fn telegram_html_to_plain_text_preserves_incomplete_tag_text() {
        assert_eq!(telegram_html_to_plain_text("prefix <co"), "prefix <co");
    }
}
