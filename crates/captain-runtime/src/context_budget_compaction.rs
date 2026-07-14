//! Compaction helpers for tool results before LLM replay.

use crate::context_budget::ContextBudget;

const NOISY_TOOL_THRESHOLD_CHARS: usize = 6_000;
const DEFAULT_TOOL_THRESHOLD_CHARS: usize = 12_000;
const TARGET_TOOL_RESULT_CHARS: usize = 8_000;

/// RTK-inspired compaction for tool results before they enter LLM history.
///
/// This is intentionally conservative: small outputs pass through untouched,
/// errors keep signal lines, and long outputs preserve head + tail so the model
/// can still synthesize a high-quality answer without paying for every repeated
/// log/progress line.
pub fn compact_tool_result_for_context(
    tool_name: &str,
    content: &str,
    is_error: bool,
    budget: &ContextBudget,
) -> String {
    let cleaned = strip_ansi(content);
    let threshold = if is_noisy_tool(tool_name) {
        NOISY_TOOL_THRESHOLD_CHARS
    } else {
        DEFAULT_TOOL_THRESHOLD_CHARS
    };
    let dynamic_target = budget
        .per_result_cap()
        .clamp(2_000, TARGET_TOOL_RESULT_CHARS);

    if cleaned.len() <= threshold && !has_many_duplicate_lines(&cleaned) {
        return cleaned;
    }

    let deduped = dedupe_repeated_lines(&cleaned);
    let mut sections = Vec::new();
    let signal = signal_lines(&deduped, is_error);
    if !signal.is_empty() {
        sections.push(format!("signal:\n{}", signal.join("\n")));
    }

    let head_tail = head_tail(&deduped, dynamic_target.saturating_sub(600));
    if !head_tail.trim().is_empty() {
        sections.push(head_tail);
    }

    let body = sections.join("\n\n");
    let compacted = format!(
        "[CAPTAIN CONTEXT ECONOMY: compacted {tool_name} result from {} to about {} chars. Full raw output was not injected back into the model; rerun with a narrower command if a missing detail matters.]\n{}",
        content.len(),
        body.len(),
        body
    );

    if compacted.len() >= content.len().saturating_sub(content.len() / 20) {
        cleaned
    } else {
        compacted
    }
}

/// Truncate content to `max_chars` with a marker.
pub(crate) fn truncate_to(content: &str, max_chars: usize) -> String {
    if content.len() <= max_chars {
        return content.to_string();
    }
    let mut keep = max_chars.saturating_sub(80).min(content.len());
    while keep > 0 && !content.is_char_boundary(keep) {
        keep -= 1;
    }
    let mut search_start = keep.saturating_sub(100);
    while search_start > 0 && !content.is_char_boundary(search_start) {
        search_start -= 1;
    }
    let break_point = content[search_start..keep]
        .rfind('\n')
        .map(|pos| search_start + pos)
        .unwrap_or(keep);
    format!(
        "{}\n\n[COMPACTED: {} → {} chars by context guard]",
        &content[..break_point],
        content.len(),
        break_point
    )
}

fn is_noisy_tool(tool_name: &str) -> bool {
    matches!(
        tool_name,
        "shell_exec"
            | "execute_code"
            | "ssh_exec"
            | "cargo"
            | "npm"
            | "pip"
            | "docker_exec"
            | "docker_build"
            | "docker_run"
            | "process_start"
            | "process_poll"
    )
}

fn strip_ansi(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    let mut chars = input.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch == '\u{1b}' && chars.peek() == Some(&'[') {
            chars.next();
            for next in chars.by_ref() {
                if next.is_ascii_alphabetic() {
                    break;
                }
            }
            continue;
        }
        out.push(ch);
    }
    out
}

fn has_many_duplicate_lines(content: &str) -> bool {
    let mut previous = "";
    let mut run = 0usize;
    for line in content.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        if trimmed == previous {
            run += 1;
            if run >= 4 {
                return true;
            }
        } else {
            previous = trimmed;
            run = 1;
        }
    }
    false
}

fn dedupe_repeated_lines(content: &str) -> String {
    let mut out = Vec::new();
    let mut previous: Option<&str> = None;
    let mut count = 0usize;

    let flush = |out: &mut Vec<String>, previous: Option<&str>, count: usize| {
        let Some(line) = previous else {
            return;
        };
        if count > 2 {
            out.push(format!("{line} (repeated x{count})"));
        } else {
            for _ in 0..count {
                out.push(line.to_string());
            }
        }
    };

    for line in content.lines() {
        if previous == Some(line) {
            count += 1;
        } else {
            flush(&mut out, previous, count);
            previous = Some(line);
            count = 1;
        }
    }
    flush(&mut out, previous, count);
    out.join("\n")
}

fn signal_lines(content: &str, is_error: bool) -> Vec<String> {
    let keywords = [
        "error",
        "failed",
        "failure",
        "fatal",
        "panic",
        "exception",
        "traceback",
        "critical",
        "denied",
        "duplicate",
        "invalid",
        "timeout",
        "not found",
        "failed services",
        "active (running)",
        "load average",
        "use%",
        "listening",
    ];
    let mut lines = Vec::new();
    for line in content.lines() {
        let lower = line.to_lowercase();
        if is_error || keywords.iter().any(|kw| lower.contains(kw)) {
            let trimmed = line.trim();
            if !trimmed.is_empty() && !lines.iter().any(|l| l == trimmed) {
                lines.push(trimmed.to_string());
            }
        }
        if lines.len() >= 80 {
            break;
        }
    }
    lines
}

fn head_tail(content: &str, max_chars: usize) -> String {
    if content.len() <= max_chars {
        return content.to_string();
    }
    let head_budget = max_chars.saturating_mul(55) / 100;
    let tail_budget = max_chars.saturating_sub(head_budget).saturating_sub(160);
    let head = truncate_prefix_at_line(content, head_budget);
    let tail = truncate_suffix_at_line(content, tail_budget);
    format!(
        "{head}\n\n[... omitted middle: {} chars ...]\n\n{tail}",
        content.len().saturating_sub(head.len() + tail.len())
    )
}

fn truncate_prefix_at_line(content: &str, max_chars: usize) -> String {
    if content.len() <= max_chars {
        return content.to_string();
    }
    let mut end = max_chars.min(content.len());
    while end > 0 && !content.is_char_boundary(end) {
        end -= 1;
    }
    if let Some(pos) = content[..end].rfind('\n') {
        content[..pos].to_string()
    } else {
        content[..end].to_string()
    }
}

fn truncate_suffix_at_line(content: &str, max_chars: usize) -> String {
    if content.len() <= max_chars {
        return content.to_string();
    }
    let mut start = content.len().saturating_sub(max_chars);
    while start < content.len() && !content.is_char_boundary(start) {
        start += 1;
    }
    if let Some(pos) = content[start..].find('\n') {
        content[start + pos + 1..].to_string()
    } else {
        content[start..].to_string()
    }
}

#[cfg(test)]
#[path = "context_budget_compaction_tests.rs"]
mod tests;
