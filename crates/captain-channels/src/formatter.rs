//! Channel-specific message formatting.
//!
//! Converts standard Markdown into platform-specific markup:
//! - Telegram HTML: `**bold**` → `<b>bold</b>`
//! - Slack mrkdwn: `**bold**` → `*bold*`, `[text](url)` → `<url|text>`
//! - Plain text: strips all formatting

use captain_types::config::OutputFormat;

/// Format a message for a specific channel output format.
pub fn format_for_channel(text: &str, format: OutputFormat) -> String {
    match format {
        OutputFormat::Markdown => text.to_string(),
        OutputFormat::TelegramHtml => markdown_to_telegram_html(text),
        OutputFormat::SlackMrkdwn => markdown_to_slack_mrkdwn(text),
        OutputFormat::PlainText => markdown_to_plain(text),
    }
}

/// Format a message for WeCom, using a stronger plain-text conversion to avoid
/// leaking Markdown syntax into enterprise chat replies.
pub fn format_for_wecom(text: &str, format: OutputFormat) -> String {
    match format {
        OutputFormat::PlainText => markdown_to_wecom_plain(text),
        _ => format_for_channel(text, format),
    }
}

/// Convert Markdown to Telegram HTML subset.
///
/// Supported tags: `<b>`, `<i>`, `<code>`, `<pre>`, `<a href="">`, `<blockquote>`.
fn markdown_to_telegram_html(text: &str) -> String {
    let normalized = text.replace("\r\n", "\n").replace('\r', "\n");
    let mut blocks = Vec::new();
    let lines: Vec<&str> = normalized.lines().collect();
    let mut i = 0;

    while i < lines.len() {
        let line = lines[i];
        let trimmed = line.trim();

        if trimmed.is_empty() {
            i += 1;
            continue;
        }

        if let Some((block, next_index)) = render_telegram_structured_block(&lines, i, trimmed) {
            blocks.push(block);
            i = next_index;
            continue;
        }

        // Paragraph
        let (block, next_index) = render_telegram_paragraph_block(&lines, i, trimmed);
        blocks.push(block);
        i = next_index;
    }

    blocks.join("\n\n")
}

fn render_telegram_structured_block(
    lines: &[&str],
    index: usize,
    trimmed: &str,
) -> Option<(String, usize)> {
    // Fenced code block. Telegram HTML supports:
    // <pre><code class="language-python">...</code></pre>
    if let Some(fence) = fence_delimiter(trimmed) {
        return Some(render_telegram_fenced_code_block(
            lines, index, trimmed, fence,
        ));
    }

    // ATX heading (#, ##, ...)
    if let Some(content) = heading_text(trimmed) {
        return Some((
            format!("<b>{}</b>", render_inline_markdown(content.trim())),
            index + 1,
        ));
    }

    if trimmed.starts_with('>') {
        return Some(render_telegram_blockquote_block(lines, index));
    }

    // Telegram has no table entity; render a compact monospaced block instead
    // of leaking hard-to-read pipe syntax.
    if is_markdown_table_start(lines, index) {
        return Some(render_telegram_table_block(lines, index));
    }

    if task_list_item(trimmed).is_some() {
        return Some(render_telegram_task_list_block(lines, index));
    }
    if unordered_list_item(trimmed).is_some() {
        return Some(render_telegram_unordered_list_block(lines, index));
    }
    if ordered_list_item(trimmed).is_some() {
        return Some(render_telegram_ordered_list_block(lines, index));
    }

    None
}

fn render_telegram_fenced_code_block(
    lines: &[&str],
    mut index: usize,
    trimmed: &str,
    fence: &str,
) -> (String, usize) {
    let language = fence_language(trimmed, fence);
    index += 1;
    let mut code_lines = Vec::new();
    while index < lines.len() {
        let candidate = lines[index].trim();
        if candidate.starts_with(fence) {
            index += 1;
            break;
        }
        code_lines.push(lines[index]);
        index += 1;
    }
    let code = escape_html(&code_lines.join("\n"));
    let block = if let Some(language) = language {
        format!(
            "<pre><code class=\"language-{}\">{}</code></pre>",
            escape_html_attr(language),
            code
        )
    } else {
        format!("<pre><code>{}</code></pre>", code)
    };
    (block, index)
}

fn render_telegram_blockquote_block(lines: &[&str], mut index: usize) -> (String, usize) {
    let mut quote_lines = Vec::new();
    while index < lines.len() {
        let current = lines[index].trim();
        if current.is_empty() || !current.starts_with('>') {
            break;
        }
        let content = current.strip_prefix('>').unwrap_or(current).trim_start();
        quote_lines.push(render_inline_markdown(content));
        index += 1;
    }
    (
        format!("<blockquote>{}</blockquote>", quote_lines.join("\n")),
        index,
    )
}

fn render_telegram_table_block(lines: &[&str], mut index: usize) -> (String, usize) {
    let mut table_lines = Vec::new();
    while index < lines.len() {
        let current = lines[index].trim();
        if current.is_empty() || !looks_like_table_row(current) {
            break;
        }
        table_lines.push(current);
        index += 1;
    }
    (format_markdown_table_for_telegram(&table_lines), index)
}

fn render_telegram_task_list_block(lines: &[&str], index: usize) -> (String, usize) {
    render_telegram_list_block(lines, index, |line, counter| {
        let (checked, item) = task_list_item(line)?;
        let marker = if checked { "☑" } else { "☐" };
        Some((
            format!("{marker} {}", render_inline_markdown(item.trim())),
            counter,
        ))
    })
}

fn render_telegram_unordered_list_block(lines: &[&str], index: usize) -> (String, usize) {
    render_telegram_list_block(lines, index, |line, counter| {
        unordered_list_item(line).map(|item| {
            (
                format!("• {}", render_inline_markdown(item.trim())),
                counter,
            )
        })
    })
}

fn render_telegram_ordered_list_block(lines: &[&str], index: usize) -> (String, usize) {
    render_telegram_list_block(lines, index, |line, counter| {
        ordered_list_item(line).map(|item| {
            (
                format!("{}. {}", counter, render_inline_markdown(item.trim())),
                counter + 1,
            )
        })
    })
}

fn render_telegram_list_block<F>(lines: &[&str], mut index: usize, mut render: F) -> (String, usize)
where
    F: FnMut(&str, usize) -> Option<(String, usize)>,
{
    let mut items = Vec::new();
    let mut counter = 1;
    while index < lines.len() {
        let current = lines[index].trim();
        if let Some((item, next_counter)) = render(current, counter) {
            items.push(item);
            counter = next_counter;
            index += 1;
        } else if current.is_empty() {
            index += 1;
            break;
        } else {
            break;
        }
    }
    (items.join("\n"), index)
}

fn render_telegram_paragraph_block(
    lines: &[&str],
    mut index: usize,
    trimmed: &str,
) -> (String, usize) {
    let mut paragraph_lines = vec![trimmed];
    index += 1;
    while index < lines.len() {
        let current = lines[index].trim();
        if is_telegram_paragraph_boundary(current) {
            break;
        }
        paragraph_lines.push(current);
        index += 1;
    }
    let joined = paragraph_lines.join("\n");
    (render_inline_markdown(&joined), index)
}

fn is_telegram_paragraph_boundary(current: &str) -> bool {
    current.is_empty()
        || fence_delimiter(current).is_some()
        || heading_text(current).is_some()
        || current.starts_with('>')
        || unordered_list_item(current).is_some()
        || ordered_list_item(current).is_some()
}

fn render_inline_markdown(text: &str) -> String {
    let (mut result, protected_code) = protect_inline_code(escape_html(text));
    result = render_inline_links(result);
    result = replace_paired_delimiter(result, "__", "<u>", "</u>");
    result = replace_paired_delimiter(result, "**", "<b>", "</b>");
    result = replace_paired_delimiter(result, "~~", "<s>", "</s>");
    result = replace_paired_delimiter(result, "||", "<tg-spoiler>", "</tg-spoiler>");

    let mut out = render_single_star_italic(&result);
    restore_inline_code(&mut out, &protected_code);
    out
}

fn protect_inline_code(mut result: String) -> (String, Vec<String>) {
    let mut protected_code = Vec::new();
    while let Some(start) = result.find('`') {
        let Some(end_rel) = result[start + 1..].find('`') else {
            break;
        };
        let end = start + 1 + end_rel;
        let inner = result[start + 1..end].to_string();
        let token = inline_code_token(protected_code.len());
        protected_code.push(format!("<code>{inner}</code>"));
        result = format!("{}{}{}", &result[..start], token, &result[end + 1..]);
    }
    (result, protected_code)
}

fn render_inline_links(mut result: String) -> String {
    // Links: [text](url) → <a href="url">text</a>
    while let Some(bracket_start) = result.find('[') {
        if let Some(bracket_end_rel) = result[bracket_start..].find("](") {
            let bracket_end = bracket_start + bracket_end_rel;
            if let Some(paren_end_rel) = result[bracket_end + 2..].find(')') {
                let paren_end = bracket_end + 2 + paren_end_rel;
                let link_text = result[bracket_start + 1..bracket_end].to_string();
                let url = result[bracket_end + 2..paren_end].replace('"', "&quot;");
                result = format!(
                    "{}<a href=\"{}\">{}</a>{}",
                    &result[..bracket_start],
                    url,
                    link_text,
                    &result[paren_end + 1..]
                );
            } else {
                break;
            }
        } else {
            break;
        }
    }
    result
}

fn replace_paired_delimiter(
    mut result: String,
    delimiter: &str,
    open_tag: &str,
    close_tag: &str,
) -> String {
    while let Some(start) = result.find(delimiter) {
        let content_start = start + delimiter.len();
        let Some(end_rel) = result[content_start..].find(delimiter) else {
            break;
        };
        let end = content_start + end_rel;
        let inner = result[content_start..end].to_string();
        result = format!(
            "{}{}{}{}{}",
            &result[..start],
            open_tag,
            inner,
            close_tag,
            &result[end + delimiter.len()..]
        );
    }
    result
}

fn render_single_star_italic(result: &str) -> String {
    // Italic: *text* → <i>text</i> (single star only)
    let mut out = String::with_capacity(result.len());
    let chars: Vec<char> = result.chars().collect();
    let mut i = 0;
    let mut in_italic = false;
    while i < chars.len() {
        if chars[i] == '*'
            && (i == 0 || chars[i - 1] != '*')
            && (i + 1 >= chars.len() || chars[i + 1] != '*')
        {
            if in_italic {
                out.push_str("</i>");
            } else {
                out.push_str("<i>");
            }
            in_italic = !in_italic;
        } else {
            out.push(chars[i]);
        }
        i += 1;
    }
    out
}

fn restore_inline_code(out: &mut String, protected_code: &[String]) {
    for (idx, replacement) in protected_code.iter().enumerate() {
        *out = out.replace(&inline_code_token(idx), replacement);
    }
}

fn inline_code_token(idx: usize) -> String {
    format!("\u{E000}{idx}\u{E001}")
}

fn escape_html(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

fn escape_html_attr(text: &str) -> String {
    escape_html(text).replace('"', "&quot;")
}

fn fence_delimiter(line: &str) -> Option<&'static str> {
    if line.starts_with("```") {
        Some("```")
    } else if line.starts_with("~~~") {
        Some("~~~")
    } else {
        None
    }
}

fn fence_language<'a>(line: &'a str, fence: &str) -> Option<&'a str> {
    let language = line
        .trim()
        .strip_prefix(fence)?
        .split_whitespace()
        .next()
        .unwrap_or("");
    if language.is_empty()
        || !language
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '+' | '#'))
    {
        None
    } else {
        Some(language)
    }
}

fn heading_text(line: &str) -> Option<&str> {
    let hashes = line.chars().take_while(|c| *c == '#').count();
    if (1..=6).contains(&hashes) && line.chars().nth(hashes) == Some(' ') {
        Some(&line[hashes + 1..])
    } else {
        None
    }
}

fn unordered_list_item(line: &str) -> Option<&str> {
    for prefix in ["- ", "* ", "+ "] {
        if let Some(rest) = line.strip_prefix(prefix) {
            return Some(rest);
        }
    }
    None
}

fn task_list_item(line: &str) -> Option<(bool, &str)> {
    for prefix in ["- ", "* ", "+ "] {
        let Some(rest) = line.strip_prefix(prefix) else {
            continue;
        };
        if let Some(item) = rest.strip_prefix("[ ] ") {
            return Some((false, item));
        }
        if let Some(item) = rest
            .strip_prefix("[x] ")
            .or_else(|| rest.strip_prefix("[X] "))
        {
            return Some((true, item));
        }
    }
    None
}

fn ordered_list_item(line: &str) -> Option<&str> {
    let digit_count = line.chars().take_while(|c| c.is_ascii_digit()).count();
    if digit_count == 0 {
        return None;
    }
    let rest = &line[digit_count..];
    if let Some(item) = rest.strip_prefix(". ") {
        Some(item)
    } else if let Some(item) = rest.strip_prefix(") ") {
        Some(item)
    } else {
        None
    }
}

fn looks_like_table_row(line: &str) -> bool {
    let trimmed = line.trim();
    trimmed.contains('|') && trimmed.matches('|').count() >= 2
}

fn is_markdown_table_start(lines: &[&str], index: usize) -> bool {
    if index + 1 >= lines.len() {
        return false;
    }
    looks_like_table_row(lines[index].trim()) && is_table_divider_row(lines[index + 1].trim())
}

fn is_table_divider_row(line: &str) -> bool {
    if !looks_like_table_row(line) {
        return false;
    }
    line.trim_matches('|')
        .split('|')
        .map(str::trim)
        .filter(|cell| !cell.is_empty())
        .all(|cell| {
            let hyphen_count = cell.chars().filter(|c| *c == '-').count();
            hyphen_count >= 3 && cell.chars().all(|c| matches!(c, '-' | ':' | ' ' | '\t'))
        })
}

fn split_table_row(line: &str) -> Vec<String> {
    line.trim()
        .trim_matches('|')
        .split('|')
        .map(|cell| strip_inline_markdown(cell.trim().to_string()))
        .collect()
}

fn format_markdown_table_for_telegram(lines: &[&str]) -> String {
    let rows: Vec<Vec<String>> = lines
        .iter()
        .filter(|line| !is_table_divider_row(line.trim()))
        .map(|line| split_table_row(line))
        .filter(|row| !row.is_empty())
        .collect();
    if rows.is_empty() {
        return String::new();
    }

    let columns = rows.iter().map(Vec::len).max().unwrap_or(0);
    let mut widths = vec![0usize; columns];
    for row in &rows {
        for (idx, cell) in row.iter().enumerate() {
            widths[idx] = widths[idx].max(cell.chars().count());
        }
    }

    let body = rows
        .iter()
        .map(|row| {
            (0..columns)
                .map(|idx| {
                    let cell = row.get(idx).map(String::as_str).unwrap_or("");
                    format!("{cell:<width$}", width = widths[idx])
                })
                .collect::<Vec<_>>()
                .join("  ")
                .trim_end()
                .to_string()
        })
        .collect::<Vec<_>>()
        .join("\n");
    format!("<pre><code>{}</code></pre>", escape_html(&body))
}

/// Convert Markdown to Slack mrkdwn format.
fn markdown_to_slack_mrkdwn(text: &str) -> String {
    let mut result = text.to_string();

    // Bold: **text** → *text*
    while let Some(start) = result.find("**") {
        if let Some(end) = result[start + 2..].find("**") {
            let end = start + 2 + end;
            let inner = result[start + 2..end].to_string();
            result = format!("{}*{}*{}", &result[..start], inner, &result[end + 2..]);
        } else {
            break;
        }
    }

    // Links: [text](url) → <url|text>
    while let Some(bracket_start) = result.find('[') {
        if let Some(bracket_end) = result[bracket_start..].find("](") {
            let bracket_end = bracket_start + bracket_end;
            if let Some(paren_end) = result[bracket_end + 2..].find(')') {
                let paren_end = bracket_end + 2 + paren_end;
                let link_text = &result[bracket_start + 1..bracket_end];
                let url = &result[bracket_end + 2..paren_end];
                result = format!(
                    "{}<{}|{}>{}",
                    &result[..bracket_start],
                    url,
                    link_text,
                    &result[paren_end + 1..]
                );
            } else {
                break;
            }
        } else {
            break;
        }
    }

    result
}

fn strip_atx_heading(line: &str) -> String {
    let trimmed = line.trim_start();
    let heading_level = trimmed.chars().take_while(|c| *c == '#').count();
    if !(1..=6).contains(&heading_level) {
        return line.to_string();
    }

    if trimmed.chars().nth(heading_level) != Some(' ') {
        return line.to_string();
    }

    trimmed[heading_level..]
        .trim()
        .trim_end_matches('#')
        .trim_end()
        .to_string()
}

fn strip_blockquote_prefix(line: &str) -> String {
    let mut trimmed = line.trim_start();
    while let Some(rest) = trimmed.strip_prefix('>') {
        trimmed = rest.trim_start();
    }
    trimmed.to_string()
}

fn strip_task_list_prefix(line: &str) -> String {
    let trimmed = line.trim_start();
    for prefix in [
        "- [ ] ", "- [x] ", "- [X] ", "* [ ] ", "* [x] ", "* [X] ", "+ [ ] ", "+ [x] ", "+ [X] ",
    ] {
        if let Some(rest) = trimmed.strip_prefix(prefix) {
            return rest.to_string();
        }
    }
    line.to_string()
}

fn is_fenced_code_marker(line: &str) -> bool {
    let trimmed = line.trim();
    let mut chars = trimmed.chars();
    let Some(marker) = chars.next() else {
        return false;
    };
    if marker != '`' && marker != '~' {
        return false;
    }
    chars.all(|c| c == marker || c.is_ascii_alphanumeric())
}

fn is_setext_heading_underline(line: &str) -> bool {
    let trimmed = line.trim();
    if trimmed.len() < 3 {
        return false;
    }
    trimmed.chars().all(|c| c == '=' || c == '-') && trimmed.contains(['=', '-'])
}

fn is_table_divider(line: &str) -> bool {
    let trimmed = line.trim();
    !trimmed.is_empty() && trimmed.chars().all(|c| matches!(c, '|' | ':' | '-' | ' '))
}

fn strip_inline_markdown(mut text: String) -> String {
    while let Some(start) = text.find("![") {
        if let Some(mid) = text[start..].find("](") {
            let mid = start + mid;
            if let Some(end) = text[mid + 2..].find(')') {
                let end = mid + 2 + end;
                let alt = &text[start + 2..mid];
                let url = &text[mid + 2..end];
                let replacement = if alt.is_empty() {
                    url.to_string()
                } else {
                    format!("{alt} ({url})")
                };
                text = format!("{}{}{}", &text[..start], replacement, &text[end + 1..]);
                continue;
            }
        }
        break;
    }

    while let Some(start) = text.find('[') {
        if let Some(mid) = text[start..].find("](") {
            let mid = start + mid;
            if let Some(end) = text[mid + 2..].find(')') {
                let end = mid + 2 + end;
                let label = &text[start + 1..mid];
                let url = &text[mid + 2..end];
                text = format!("{}{} ({}){}", &text[..start], label, url, &text[end + 1..]);
                continue;
            }
        }
        break;
    }

    while let Some(start) = text.find('<') {
        if let Some(end) = text[start + 1..].find('>') {
            let end = start + 1 + end;
            let inner = &text[start + 1..end];
            if inner.starts_with("http://")
                || inner.starts_with("https://")
                || inner.starts_with("mailto:")
            {
                text = format!("{}{}{}", &text[..start], inner, &text[end + 1..]);
                continue;
            }
        }
        break;
    }

    text = text.replace("**", "");
    text = text.replace("__", "");
    text = text.replace("~~", "");
    text = text.replace('`', "");

    let mut out = String::with_capacity(text.len());
    let chars: Vec<char> = text.chars().collect();
    for (i, &ch) in chars.iter().enumerate() {
        if ch == '*'
            && (i == 0 || chars[i - 1] != '*')
            && (i + 1 >= chars.len() || chars[i + 1] != '*')
        {
            continue;
        }
        out.push(ch);
    }
    out
}

/// Strip common Markdown blocks for WeCom plain-text replies.
fn markdown_to_wecom_plain(text: &str) -> String {
    let mut result_lines = Vec::new();
    let mut in_fenced_code = false;

    for raw_line in text.replace("\r\n", "\n").lines() {
        let trimmed = raw_line.trim();

        if is_fenced_code_marker(trimmed) {
            in_fenced_code = !in_fenced_code;
            continue;
        }

        if in_fenced_code {
            result_lines.push(raw_line.trim_end().to_string());
            continue;
        }

        if is_setext_heading_underline(trimmed) || is_table_divider(trimmed) {
            continue;
        }

        let mut line = strip_atx_heading(raw_line);
        line = strip_blockquote_prefix(&line);
        line = strip_task_list_prefix(&line);

        let trimmed_line = line.trim();
        if trimmed_line.starts_with('|') && trimmed_line.ends_with('|') && trimmed_line.len() > 2 {
            line = trimmed_line
                .trim_matches('|')
                .split('|')
                .map(|cell| cell.trim())
                .collect::<Vec<_>>()
                .join("    ");
        }

        line = strip_inline_markdown(line);
        result_lines.push(line.trim().to_string());
    }

    let mut collapsed = Vec::new();
    for line in result_lines {
        if line.is_empty()
            && collapsed
                .last()
                .is_some_and(|prev: &String| prev.is_empty())
        {
            continue;
        }
        collapsed.push(line);
    }

    collapsed.join("\n").trim().to_string()
}

/// Strip all Markdown formatting, producing plain text.
fn markdown_to_plain(text: &str) -> String {
    let mut result = text.to_string();

    // Remove bold markers
    result = result.replace("**", "");

    // Remove italic markers (single *)
    // Simple approach: remove isolated *
    let mut out = String::with_capacity(result.len());
    let chars: Vec<char> = result.chars().collect();
    for (i, &ch) in chars.iter().enumerate() {
        if ch == '*'
            && (i == 0 || chars[i - 1] != '*')
            && (i + 1 >= chars.len() || chars[i + 1] != '*')
        {
            continue;
        }
        out.push(ch);
    }
    result = out;

    // Remove inline code markers
    result = result.replace('`', "");

    // Convert links: [text](url) → text (url)
    while let Some(bracket_start) = result.find('[') {
        if let Some(bracket_end) = result[bracket_start..].find("](") {
            let bracket_end = bracket_start + bracket_end;
            if let Some(paren_end) = result[bracket_end + 2..].find(')') {
                let paren_end = bracket_end + 2 + paren_end;
                let link_text = &result[bracket_start + 1..bracket_end];
                let url = &result[bracket_end + 2..paren_end];
                result = format!(
                    "{}{} ({}){}",
                    &result[..bracket_start],
                    link_text,
                    url,
                    &result[paren_end + 1..]
                );
            } else {
                break;
            }
        } else {
            break;
        }
    }

    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_format_markdown_passthrough() {
        let text = "**bold** and *italic*";
        assert_eq!(format_for_channel(text, OutputFormat::Markdown), text);
    }

    #[test]
    fn test_telegram_html_bold() {
        let result = markdown_to_telegram_html("Hello **world**!");
        assert_eq!(result, "Hello <b>world</b>!");
    }

    #[test]
    fn test_telegram_html_italic() {
        let result = markdown_to_telegram_html("Hello *world*!");
        assert_eq!(result, "Hello <i>world</i>!");
    }

    #[test]
    fn test_telegram_html_code() {
        let result = markdown_to_telegram_html("Use `println!`");
        assert_eq!(result, "Use <code>println!</code>");
    }

    #[test]
    fn test_telegram_html_code_protects_markdown_inside_code() {
        let result = markdown_to_telegram_html("Use `**literal**` then **bold**");
        assert_eq!(result, "Use <code>**literal**</code> then <b>bold</b>");
    }

    #[test]
    fn test_telegram_html_link() {
        let result = markdown_to_telegram_html("[click here](https://example.com)");
        assert_eq!(result, "<a href=\"https://example.com\">click here</a>");
    }

    #[test]
    fn test_telegram_html_heading() {
        let result = markdown_to_telegram_html("## Result");
        assert_eq!(result, "<b>Result</b>");
    }

    #[test]
    fn test_telegram_html_unordered_list() {
        let result = markdown_to_telegram_html("- alpha\n- beta");
        assert_eq!(result, "• alpha\n• beta");
    }

    #[test]
    fn test_telegram_html_task_list() {
        let result = markdown_to_telegram_html("- [x] shipped\n- [ ] pending");
        assert_eq!(result, "☑ shipped\n☐ pending");
    }

    #[test]
    fn test_telegram_html_ordered_list() {
        let result = markdown_to_telegram_html("1. alpha\n2. beta");
        assert_eq!(result, "1. alpha\n2. beta");
    }

    #[test]
    fn test_telegram_html_fenced_code_block() {
        let result = markdown_to_telegram_html("```rust\nfn main() {}\n```");
        assert_eq!(
            result,
            "<pre><code class=\"language-rust\">fn main() {}</code></pre>"
        );
    }

    #[test]
    fn test_telegram_html_blockquote() {
        let result = markdown_to_telegram_html("> note\n> second line");
        assert_eq!(result, "<blockquote>note\nsecond line</blockquote>");
    }

    #[test]
    fn test_telegram_html_spoiler_underline_strike() {
        let result = markdown_to_telegram_html("__ok__ ~~old~~ ||secret||");
        assert_eq!(
            result,
            "<u>ok</u> <s>old</s> <tg-spoiler>secret</tg-spoiler>"
        );
    }

    #[test]
    fn test_telegram_html_table_renders_as_pre() {
        let result = markdown_to_telegram_html("| Nom | Etat |\n| --- | --- |\n| API | OK |\n");
        assert_eq!(result, "<pre><code>Nom  Etat\nAPI  OK</code></pre>");
    }

    #[test]
    fn test_slack_mrkdwn_bold() {
        let result = markdown_to_slack_mrkdwn("Hello **world**!");
        assert_eq!(result, "Hello *world*!");
    }

    #[test]
    fn test_slack_mrkdwn_link() {
        let result = markdown_to_slack_mrkdwn("[click](https://example.com)");
        assert_eq!(result, "<https://example.com|click>");
    }

    #[test]
    fn test_plain_text_strips_formatting() {
        let result = markdown_to_plain("**bold** and `code` and *italic*");
        assert_eq!(result, "bold and code and italic");
    }

    #[test]
    fn test_plain_text_converts_links() {
        let result = markdown_to_plain("[click](https://example.com)");
        assert_eq!(result, "click (https://example.com)");
    }

    #[test]
    fn test_wecom_plain_text_strips_common_markdown_blocks() {
        let result = markdown_to_wecom_plain(
            "# Title\n\
             \n\
             > quoted text\n\
             \n\
             - [x] done item\n\
             - [ ] todo item\n\
             \n\
             ```rust\n\
             let value = 1;\n\
             ```\n\
             \n\
             [docs](https://example.com)\n",
        );
        assert_eq!(
            result,
            "Title\n\nquoted text\n\ndone item\ntodo item\n\nlet value = 1;\n\ndocs (https://example.com)"
        );
    }
}
