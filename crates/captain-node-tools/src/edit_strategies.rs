//! Nine fallback replacement strategies for the `edit_file` tool.
//!
//! The LLM frequently produces an `old_string` that *almost* matches the
//! file but diverges in whitespace, indentation, or escape encoding. A
//! single exact-match replacer fails on a meaningful share of real edits.
//! The strategy chain (opencode lineage) tries progressively more tolerant
//! matchers; the first one that succeeds wins.
//!
//! Order:
//! 1. `simple` — exact substring, single occurrence
//! 2. `line_trimmed` — match line-by-line ignoring leading/trailing whitespace
//! 3. `block_anchor` — anchor on first + last line of `old_string` (≥3 lines)
//! 4. `whitespace_normalized` — collapse whitespace runs to single spaces
//! 5. `indentation_flexible` — strip leading whitespace then compare
//! 6. `escape_normalized` — un-escape `\n`, `\t`, `\"`, `\\` in `old_string`
//! 7. `trimmed_boundary` — `old_string.trim()` then exact substring
//! 8. `context_aware` — anchor on first 2 + last 2 lines (≥5 lines)
//! 9. `multi_occurrence` — taken when `replace_all == true`
//!
//! Pure logic. Zero IO. Caller (`tool_edit_file`) handles the read/write.

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EditResult {
    /// Name of the strategy that produced the result.
    pub strategy: &'static str,
    /// File content after replacement.
    pub new_content: String,
    /// Number of replacements applied.
    pub replacements: usize,
}

/// Top-level entry. Tries each strategy in order; returns the first hit.
pub fn try_edit(
    content: &str,
    old: &str,
    new: &str,
    replace_all: bool,
) -> Result<EditResult, String> {
    if old.is_empty() {
        return Err("old_string cannot be empty".into());
    }
    if old == new {
        return Err("old_string == new_string, refusing no-op".into());
    }
    if replace_all {
        return multi_occurrence_replacer(content, old, new);
    }

    type StratFn = fn(&str, &str, &str) -> Option<String>;
    let strategies: &[(&'static str, StratFn)] = &[
        ("simple", simple_replacer),
        ("line_trimmed", line_trimmed_replacer),
        ("block_anchor", block_anchor_replacer),
        ("whitespace_normalized", whitespace_normalized_replacer),
        ("indentation_flexible", indentation_flexible_replacer),
        ("escape_normalized", escape_normalized_replacer),
        ("trimmed_boundary", trimmed_boundary_replacer),
        ("context_aware", context_aware_replacer),
    ];
    for (name, f) in strategies {
        if let Some(new_content) = f(content, old, new) {
            return Ok(EditResult {
                strategy: name,
                new_content,
                replacements: 1,
            });
        }
    }
    Err(format!(
        "No fallback strategy matched: old_string ({} chars) not found in content ({} chars). \
         Tried 8 strategies (simple, line_trimmed, block_anchor, whitespace_normalized, \
         indentation_flexible, escape_normalized, trimmed_boundary, context_aware).",
        old.len(),
        content.len()
    ))
}

// ─── 1. simple ───────────────────────────────────────────────────────────────

fn simple_replacer(content: &str, old: &str, new: &str) -> Option<String> {
    let count = content.matches(old).count();
    if count != 1 {
        return None;
    }
    Some(content.replacen(old, new, 1))
}

// ─── 2. line_trimmed ─────────────────────────────────────────────────────────

fn line_trimmed_replacer(content: &str, old: &str, new: &str) -> Option<String> {
    let content_lines: Vec<&str> = content.lines().collect();
    let old_lines: Vec<&str> = old.lines().collect();
    if old_lines.is_empty() || old_lines.len() > content_lines.len() {
        return None;
    }
    let mut hits: Vec<usize> = Vec::new();
    for i in 0..=content_lines.len() - old_lines.len() {
        if content_lines[i..i + old_lines.len()]
            .iter()
            .zip(old_lines.iter())
            .all(|(a, b)| a.trim() == b.trim())
        {
            hits.push(i);
        }
    }
    if hits.len() != 1 {
        return None; // ambiguous or no hit
    }
    Some(splice_lines(
        content,
        &content_lines,
        hits[0],
        old_lines.len(),
        new,
    ))
}

// ─── 3. block_anchor ─────────────────────────────────────────────────────────

fn block_anchor_replacer(content: &str, old: &str, new: &str) -> Option<String> {
    let old_lines: Vec<&str> = old.lines().collect();
    if old_lines.len() < 3 {
        return None;
    }
    let first = old_lines.first().map(|s| s.trim()).unwrap_or("");
    let last = old_lines.last().map(|s| s.trim()).unwrap_or("");
    if first.is_empty() || last.is_empty() {
        return None;
    }
    let content_lines: Vec<&str> = content.lines().collect();
    let mut hits: Vec<(usize, usize)> = Vec::new();
    for i in 0..content_lines.len() {
        if content_lines[i].trim() != first {
            continue;
        }
        let min_end = i + old_lines.len() - 1;
        if min_end >= content_lines.len() {
            continue;
        }
        for (j, line) in content_lines.iter().enumerate().skip(min_end) {
            if line.trim() == last {
                hits.push((i, j));
                break;
            }
        }
    }
    if hits.len() != 1 {
        return None;
    }
    let (start, end) = hits[0];
    Some(splice_lines(
        content,
        &content_lines,
        start,
        end - start + 1,
        new,
    ))
}

// ─── 4. whitespace_normalized ────────────────────────────────────────────────

fn whitespace_normalized_replacer(content: &str, old: &str, new: &str) -> Option<String> {
    let norm_old = collapse_ws(old);
    if norm_old.is_empty() {
        return None;
    }
    // Sliding char-boundary scan over `content`, normalize candidate, compare.
    let bytes = content.as_bytes();
    let len = bytes.len();
    let mut hits: Vec<(usize, usize)> = Vec::new();
    let mut start = 0;
    while start < len {
        if !content.is_char_boundary(start) {
            start += 1;
            continue;
        }
        // Tight upper bound: candidate length ≤ old.len() * 4 (worst-case
        // expansion when every old whitespace was collapsed from a long run).
        let cap = (old.len() * 4).min(len - start);
        let mut end = start + 1;
        while end <= start + cap {
            if !content.is_char_boundary(end) {
                end += 1;
                continue;
            }
            let cand = &content[start..end];
            if collapse_ws(cand) == norm_old {
                hits.push((start, end));
                break;
            }
            end += 1;
        }
        start += 1;
    }
    if hits.len() != 1 {
        return None;
    }
    let (s, e) = hits[0];
    let mut out = String::with_capacity(content.len() + new.len() - (e - s));
    out.push_str(&content[..s]);
    out.push_str(new);
    out.push_str(&content[e..]);
    Some(out)
}

fn collapse_ws(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut prev_ws = false;
    for ch in s.chars() {
        if ch.is_whitespace() {
            if !prev_ws {
                out.push(' ');
                prev_ws = true;
            }
        } else {
            out.push(ch);
            prev_ws = false;
        }
    }
    out.trim().to_string()
}

// ─── 5. indentation_flexible ─────────────────────────────────────────────────

fn indentation_flexible_replacer(content: &str, old: &str, new: &str) -> Option<String> {
    let content_lines: Vec<&str> = content.lines().collect();
    let old_lines: Vec<&str> = old.lines().collect();
    if old_lines.is_empty() || old_lines.len() > content_lines.len() {
        return None;
    }
    let mut hits: Vec<usize> = Vec::new();
    for i in 0..=content_lines.len() - old_lines.len() {
        if content_lines[i..i + old_lines.len()]
            .iter()
            .zip(old_lines.iter())
            .all(|(a, b)| a.trim_start() == b.trim_start())
        {
            hits.push(i);
        }
    }
    if hits.len() != 1 {
        return None;
    }
    Some(splice_lines(
        content,
        &content_lines,
        hits[0],
        old_lines.len(),
        new,
    ))
}

// ─── 6. escape_normalized ────────────────────────────────────────────────────

fn escape_normalized_replacer(content: &str, old: &str, new: &str) -> Option<String> {
    let unesc = unescape_literals(old);
    if unesc == old {
        return None; // no escape sequences present, would duplicate `simple`
    }
    let count = content.matches(unesc.as_str()).count();
    if count != 1 {
        return None;
    }
    Some(content.replacen(unesc.as_str(), new, 1))
}

fn unescape_literals(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch == '\\' {
            match chars.peek() {
                Some('n') => {
                    chars.next();
                    out.push('\n');
                }
                Some('t') => {
                    chars.next();
                    out.push('\t');
                }
                Some('r') => {
                    chars.next();
                    out.push('\r');
                }
                Some('"') => {
                    chars.next();
                    out.push('"');
                }
                Some('\\') => {
                    chars.next();
                    out.push('\\');
                }
                _ => out.push(ch),
            }
        } else {
            out.push(ch);
        }
    }
    out
}

// ─── 7. trimmed_boundary ─────────────────────────────────────────────────────

fn trimmed_boundary_replacer(content: &str, old: &str, new: &str) -> Option<String> {
    let trimmed = old.trim();
    if trimmed == old || trimmed.is_empty() {
        return None;
    }
    let count = content.matches(trimmed).count();
    if count != 1 {
        return None;
    }
    Some(content.replacen(trimmed, new, 1))
}

// ─── 8. context_aware ────────────────────────────────────────────────────────

fn context_aware_replacer(content: &str, old: &str, new: &str) -> Option<String> {
    let old_lines: Vec<&str> = old.lines().collect();
    if old_lines.len() < 5 {
        return None;
    }
    // Anchor on first 2 + last 2 lines (trim-compared)
    let head: Vec<String> = old_lines[..2]
        .iter()
        .map(|l| l.trim().to_string())
        .collect();
    let tail: Vec<String> = old_lines[old_lines.len() - 2..]
        .iter()
        .map(|l| l.trim().to_string())
        .collect();
    if head.iter().any(|s| s.is_empty()) || tail.iter().any(|s| s.is_empty()) {
        return None;
    }
    let content_lines: Vec<&str> = content.lines().collect();
    let mut hits: Vec<(usize, usize)> = Vec::new();
    for i in 0..content_lines.len() {
        if i + 1 >= content_lines.len() {
            break;
        }
        if content_lines[i].trim() != head[0] || content_lines[i + 1].trim() != head[1] {
            continue;
        }
        // Search for the tail anchor at >= old_lines.len() - 1
        let min_j = i + old_lines.len() - 1;
        if min_j >= content_lines.len() {
            continue;
        }
        for j in min_j..content_lines.len() {
            if j == 0 {
                continue;
            }
            if content_lines[j - 1].trim() == tail[0] && content_lines[j].trim() == tail[1] {
                hits.push((i, j));
                break;
            }
        }
    }
    if hits.len() != 1 {
        return None;
    }
    let (start, end) = hits[0];
    Some(splice_lines(
        content,
        &content_lines,
        start,
        end - start + 1,
        new,
    ))
}

// ─── 9. multi_occurrence (replace_all=true) ──────────────────────────────────

fn multi_occurrence_replacer(content: &str, old: &str, new: &str) -> Result<EditResult, String> {
    let count = content.matches(old).count();
    if count == 0 {
        return Err(format!(
            "replace_all: old_string not found ({} chars) in content ({} chars)",
            old.len(),
            content.len()
        ));
    }
    Ok(EditResult {
        strategy: "multi_occurrence",
        new_content: content.replace(old, new),
        replacements: count,
    })
}

// ─── helpers ─────────────────────────────────────────────────────────────────

/// Replace `take` lines starting at `start` in `content_lines` with `new`,
/// preserving the trailing newline policy of the original `content`.
fn splice_lines(
    content: &str,
    content_lines: &[&str],
    start: usize,
    take: usize,
    new: &str,
) -> String {
    let mut out = String::with_capacity(content.len() + new.len());
    for (k, line) in content_lines.iter().enumerate() {
        if k < start {
            out.push_str(line);
            out.push('\n');
        } else if k == start {
            out.push_str(new);
            // newline boundary: if `new` doesn't end in '\n' but more lines
            // follow in the file, add a separator.
            if !new.ends_with('\n') && k + take < content_lines.len() {
                out.push('\n');
            }
        } else if k < start + take {
            // skip — part of the replaced span
            continue;
        } else {
            out.push_str(line);
            out.push('\n');
        }
    }
    if !content.ends_with('\n') {
        // Original had no trailing newline: strip the one we just added.
        if out.ends_with('\n') {
            out.pop();
        }
    }
    out
}

// ─── tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
#[path = "edit_strategies_tests.rs"]
mod tests;
