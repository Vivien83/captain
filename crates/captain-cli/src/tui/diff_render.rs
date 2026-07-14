//! Diff rendering for TUI display (Claude Code style: red `-` / green `+`).
//!
//! Two entry points:
//! - `render_unified_diff(before, after, file_path, border_style)` — compute
//!   a fresh diff via `similar` then render it (used by `edit_file`,
//!   `write_file` overwrites, etc.).
//! - `render_apply_patch_input(input, border_style)` — parse Captain's
//!   `*** Begin Patch` format directly (no recompute, the agent already
//!   produced a structured diff).
//!
//! Both return ratatui `Line`s ready to drop inside the existing tool
//! border (`┌─ ✔ name ─ … └─`) drawn by `chat.rs`.

use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use similar::{ChangeTag, TextDiff};

use crate::tui::theme;

/// Hard cap on rendered diff lines per tool call. Anything above is replaced
/// by a truncation marker.
pub const MAX_DIFF_LINES: usize = 80;
/// Lines of unchanged context shown around each change block.
const CONTEXT_LINES: usize = 3;

/// Semantic kind of a diff line (frontend-agnostic).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DiffLineKind {
    /// `@@ -a,b +c,d @@` hunk header
    Hunk,
    /// unchanged context line
    Context,
    /// `+` added line
    Add,
    /// `-` removed line
    Delete,
    /// truncation / informational marker
    Note,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiffLine {
    pub kind: DiffLineKind,
    pub text: String,
}

// ─── pure logic (similar) ────────────────────────────────────────────────────

/// Compute a unified diff between `before` and `after` as a list of semantic
/// lines. Returns `[]` if the two inputs are identical.
pub fn compute_diff(before: &str, after: &str) -> Vec<DiffLine> {
    let diff = TextDiff::from_lines(before, after);
    let mut out: Vec<DiffLine> = Vec::new();

    for group in diff.grouped_ops(CONTEXT_LINES).iter() {
        if group.is_empty() {
            continue;
        }
        let (old_start, old_len, new_start, new_len) = group_bounds(group);
        out.push(DiffLine {
            kind: DiffLineKind::Hunk,
            text: format!(
                "@@ -{},{} +{},{} @@",
                old_start + 1,
                old_len,
                new_start + 1,
                new_len
            ),
        });

        for op in group {
            for change in diff.iter_changes(op) {
                if out.len() >= MAX_DIFF_LINES {
                    out.push(DiffLine {
                        kind: DiffLineKind::Note,
                        text: format!("… (truncated, {MAX_DIFF_LINES} lines max)"),
                    });
                    return out;
                }
                let kind = match change.tag() {
                    ChangeTag::Equal => DiffLineKind::Context,
                    ChangeTag::Insert => DiffLineKind::Add,
                    ChangeTag::Delete => DiffLineKind::Delete,
                };
                let text = change.value().trim_end_matches('\n').to_string();
                out.push(DiffLine { kind, text });
            }
        }
    }
    out
}

fn group_bounds(group: &[similar::DiffOp]) -> (usize, usize, usize, usize) {
    let first = group.first().expect("non-empty group");
    let last = group.last().expect("non-empty group");
    let old_start = first.old_range().start;
    let old_end = last.old_range().end;
    let new_start = first.new_range().start;
    let new_end = last.new_range().end;
    (
        old_start,
        old_end - old_start,
        new_start,
        new_end - new_start,
    )
}

// ─── ratatui rendering ───────────────────────────────────────────────────────

/// Convert semantic diff lines to ratatui `Line`s, each prefixed by a
/// `│` border span styled with `border_style` (matches the tool box drawn
/// in `chat.rs`).
pub fn to_ratatui_lines(diff: &[DiffLine], border_style: Style) -> Vec<Line<'static>> {
    diff.iter().map(|d| line_for(d, border_style)).collect()
}

fn line_for(d: &DiffLine, border_style: Style) -> Line<'static> {
    let (sigil, color, modifier) = match d.kind {
        DiffLineKind::Hunk => ("  ", theme::ACCENT, Modifier::BOLD),
        DiffLineKind::Context => ("  ", theme::DIM, Modifier::empty()),
        DiffLineKind::Add => ("+ ", theme::GREEN, Modifier::empty()),
        DiffLineKind::Delete => ("- ", theme::RED, Modifier::empty()),
        DiffLineKind::Note => ("  ", theme::DIM, Modifier::ITALIC),
    };
    Line::from(vec![
        Span::styled("  \u{2502} ", border_style),
        Span::styled(
            format!("{sigil}{}", d.text),
            Style::default().fg(color).add_modifier(modifier),
        ),
    ])
}

/// Full pipeline (before, after) → ready-to-draw ratatui lines, prefixed by
/// the tool box border. Includes a dim header `--- file_path`.
pub fn render_unified_diff(
    before: &str,
    after: &str,
    file_path: &str,
    border_style: Style,
) -> Vec<Line<'static>> {
    let mut lines: Vec<Line<'static>> = Vec::new();
    lines.push(Line::from(vec![
        Span::styled("  \u{2502} ", border_style),
        Span::styled(format!("--- {file_path}"), Style::default().fg(theme::DIM)),
    ]));
    let diff = compute_diff(before, after);
    if diff.is_empty() {
        lines.push(Line::from(vec![
            Span::styled("  \u{2502} ", border_style),
            Span::styled("(no changes)".to_string(), Style::default().fg(theme::DIM)),
        ]));
        return lines;
    }
    lines.extend(to_ratatui_lines(&diff, border_style));
    lines
}

// ─── apply_patch parser ──────────────────────────────────────────────────────

/// Render Captain's `*** Begin Patch` format directly without recomputing the
/// diff (the agent already structured it). Used by the `apply_patch` tool.
///
/// Format reminder:
/// ```text
/// *** Begin Patch
/// *** Update File: src/main.rs
/// @@ context @@
///  unchanged
/// -old
/// +new
/// *** End Patch
/// ```
pub fn render_apply_patch_input(input: &str, border_style: Style) -> Vec<Line<'static>> {
    let mut lines: Vec<Line<'static>> = Vec::new();
    let mut in_file = false;
    let mut shown = 0usize;

    for raw in input.lines() {
        if shown >= MAX_DIFF_LINES {
            lines.push(line_for(
                &DiffLine {
                    kind: DiffLineKind::Note,
                    text: format!("… (truncated, {MAX_DIFF_LINES} lines max)"),
                },
                border_style,
            ));
            break;
        }

        if let Some(rest) = raw.strip_prefix("*** Add File: ") {
            in_file = true;
            lines.push(Line::from(vec![
                Span::styled("  \u{2502} ", border_style),
                Span::styled(
                    format!("+++ {rest} (new file)"),
                    Style::default()
                        .fg(theme::GREEN)
                        .add_modifier(Modifier::BOLD),
                ),
            ]));
            shown += 1;
            continue;
        }
        if let Some(rest) = raw.strip_prefix("*** Update File: ") {
            in_file = true;
            lines.push(Line::from(vec![
                Span::styled("  \u{2502} ", border_style),
                Span::styled(format!("--- {rest}"), Style::default().fg(theme::DIM)),
            ]));
            shown += 1;
            continue;
        }
        if let Some(rest) = raw.strip_prefix("*** Delete File: ") {
            in_file = true;
            lines.push(Line::from(vec![
                Span::styled("  \u{2502} ", border_style),
                Span::styled(
                    format!("--- {rest} (deleted)"),
                    Style::default().fg(theme::RED).add_modifier(Modifier::BOLD),
                ),
            ]));
            shown += 1;
            continue;
        }
        if let Some(rest) = raw.strip_prefix("*** Move to: ") {
            lines.push(Line::from(vec![
                Span::styled("  \u{2502} ", border_style),
                Span::styled(
                    format!("→ moved to {rest}"),
                    Style::default().fg(theme::ACCENT),
                ),
            ]));
            shown += 1;
            continue;
        }
        if raw.starts_with("*** Begin Patch") || raw.starts_with("*** End Patch") {
            continue;
        }
        if !in_file {
            continue;
        }

        if raw.starts_with("@@") {
            lines.push(line_for(
                &DiffLine {
                    kind: DiffLineKind::Hunk,
                    text: raw.to_string(),
                },
                border_style,
            ));
        } else if let Some(rest) = raw.strip_prefix('+') {
            lines.push(line_for(
                &DiffLine {
                    kind: DiffLineKind::Add,
                    text: rest.to_string(),
                },
                border_style,
            ));
        } else if let Some(rest) = raw.strip_prefix('-') {
            lines.push(line_for(
                &DiffLine {
                    kind: DiffLineKind::Delete,
                    text: rest.to_string(),
                },
                border_style,
            ));
        } else {
            let text = raw.strip_prefix(' ').unwrap_or(raw);
            lines.push(line_for(
                &DiffLine {
                    kind: DiffLineKind::Context,
                    text: text.to_string(),
                },
                border_style,
            ));
        }
        shown += 1;
    }
    lines
}

// ─── tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn line_text(line: &Line<'_>) -> String {
        line.spans.iter().map(|s| s.content.to_string()).collect()
    }

    #[test]
    fn test_no_change_returns_empty_diff() {
        let diff = compute_diff("hello\nworld\n", "hello\nworld\n");
        assert!(
            diff.is_empty(),
            "identical inputs should produce no diff, got: {diff:?}"
        );
    }

    #[test]
    fn test_simple_addition_has_add_kind() {
        let diff = compute_diff("a\nb\n", "a\nb\nc\n");
        assert!(diff.iter().any(|d| d.kind == DiffLineKind::Hunk));
        assert!(
            diff.iter()
                .any(|d| d.kind == DiffLineKind::Add && d.text == "c"),
            "expected `+ c`, got: {diff:?}"
        );
    }

    #[test]
    fn test_simple_deletion_has_delete_kind() {
        let diff = compute_diff("a\nb\nc\n", "a\nc\n");
        assert!(
            diff.iter()
                .any(|d| d.kind == DiffLineKind::Delete && d.text == "b"),
            "expected `- b`, got: {diff:?}"
        );
    }

    #[test]
    fn test_replacement_has_both_kinds() {
        let diff = compute_diff("let x = 1;\n", "let x = 2;\n");
        let has_del = diff.iter().any(|d| d.kind == DiffLineKind::Delete);
        let has_add = diff.iter().any(|d| d.kind == DiffLineKind::Add);
        assert!(has_del && has_add, "replacement must show both: {diff:?}");
    }

    #[test]
    fn test_truncation_marker_when_huge() {
        let mut before = String::new();
        let mut after = String::new();
        for i in 0..200 {
            before.push_str(&format!("line {i}\n"));
            after.push_str(&format!("modified {i}\n"));
        }
        let diff = compute_diff(&before, &after);
        assert!(
            diff.len() <= MAX_DIFF_LINES + 5,
            "should be capped, got {} lines",
            diff.len()
        );
        assert!(
            diff.last()
                .map(|d| matches!(d.kind, DiffLineKind::Note) && d.text.contains("truncated"))
                .unwrap_or(false),
            "last line should be truncation marker, got: {:?}",
            diff.last()
        );
    }

    #[test]
    fn test_render_unified_diff_includes_file_header() {
        let lines = render_unified_diff("a\n", "a\nb\n", "src/foo.rs", Style::default());
        let header = line_text(&lines[0]);
        assert!(
            header.contains("src/foo.rs") && header.contains("---"),
            "expected `--- src/foo.rs` header, got: {header:?}"
        );
    }

    #[test]
    fn test_render_unified_diff_no_changes_shows_marker() {
        let lines = render_unified_diff("same\n", "same\n", "f.rs", Style::default());
        let combined: String = lines.iter().map(line_text).collect();
        assert!(
            combined.contains("(no changes)"),
            "expected `(no changes)` marker, got: {combined:?}"
        );
    }

    #[test]
    fn test_to_ratatui_lines_uses_border_prefix() {
        let diff = vec![DiffLine {
            kind: DiffLineKind::Add,
            text: "x".into(),
        }];
        let lines = to_ratatui_lines(&diff, Style::default());
        let combined = line_text(&lines[0]);
        assert!(
            combined.starts_with("  \u{2502} "),
            "expected border prefix, got: {combined:?}"
        );
        assert!(
            combined.contains("+ x"),
            "expected `+ x` body, got: {combined:?}"
        );
    }

    #[test]
    fn test_apply_patch_input_renders_update_file() {
        let input = "*** Begin Patch\n\
            *** Update File: src/main.rs\n\
            @@ ctx @@\n\
             unchanged\n\
            -old line\n\
            +new line\n\
            *** End Patch\n";
        let lines = render_apply_patch_input(input, Style::default());
        let texts: Vec<String> = lines.iter().map(line_text).collect();
        assert!(
            texts.iter().any(|t| t.contains("--- src/main.rs")),
            "expected file header, got: {texts:?}"
        );
        assert!(
            texts.iter().any(|t| t.contains("+ new line")),
            "expected add line, got: {texts:?}"
        );
        assert!(
            texts.iter().any(|t| t.contains("- old line")),
            "expected delete line, got: {texts:?}"
        );
        assert!(
            texts.iter().any(|t| t.contains("@@ ctx @@")),
            "expected hunk header, got: {texts:?}"
        );
    }

    #[test]
    fn test_apply_patch_input_renders_add_file() {
        let input = "*** Begin Patch\n\
            *** Add File: new.rs\n\
            +fn main() {}\n\
            *** End Patch\n";
        let lines = render_apply_patch_input(input, Style::default());
        let texts: Vec<String> = lines.iter().map(line_text).collect();
        assert!(
            texts.iter().any(|t| t.contains("+++ new.rs (new file)")),
            "expected new file header, got: {texts:?}"
        );
        assert!(
            texts.iter().any(|t| t.contains("+ fn main() {}")),
            "expected `+ fn main…`, got: {texts:?}"
        );
    }

    #[test]
    fn test_apply_patch_input_renders_delete_file() {
        let input = "*** Begin Patch\n\
            *** Delete File: old.rs\n\
            *** End Patch\n";
        let lines = render_apply_patch_input(input, Style::default());
        let combined: String = lines.iter().map(line_text).collect();
        assert!(
            combined.contains("--- old.rs (deleted)"),
            "expected deletion header, got: {combined:?}"
        );
    }

    /// Visual smoke test — prints the rendered diff to stdout with real
    /// ANSI escape codes so a human can verify red `-` / green `+` /
    /// gold `@@` actually appear in the terminal.
    ///
    /// Run with: `cargo test -p captain-cli --bin captain
    ///   tui::diff_render::tests::demo_visual_apply_patch -- --ignored --nocapture`
    #[test]
    #[ignore = "visual demo: requires --ignored --nocapture"]
    fn demo_visual_apply_patch() {
        use ratatui::style::Color;
        let input = "*** Begin Patch\n\
            *** Update File: src/main.rs\n\
            @@ -1,5 +1,7 @@\n \
            fn main() {\n\
            -    let x = 1;\n\
            -    println!(\"{}\", x);\n\
            +    let x = 2;\n\
            +    let y = 3;\n\
            +    println!(\"x={}, y={}\", x, y);\n \
            }\n\
            *** Add File: README.md\n\
            +# New project\n\
            +Auto-created.\n\
            *** End Patch\n";
        let lines = render_apply_patch_input(input, Style::default().fg(theme::GREEN));
        println!("\n──── Q.5 visual demo (apply_patch) ────\n");
        for line in &lines {
            for span in &line.spans {
                let ansi = match span.style.fg {
                    Some(Color::Rgb(r, g, b)) => {
                        format!("\x1b[38;2;{r};{g};{b}m")
                    }
                    _ => String::new(),
                };
                let bold = if span
                    .style
                    .add_modifier
                    .contains(ratatui::style::Modifier::BOLD)
                {
                    "\x1b[1m"
                } else {
                    ""
                };
                print!("{ansi}{bold}{}\x1b[0m", span.content);
            }
            println!();
        }
        println!("\n────────────────────────────────────────\n");
    }

    #[test]
    fn test_apply_patch_input_skips_orphan_diff_lines() {
        let input = "+orphan line without file context\n";
        let lines = render_apply_patch_input(input, Style::default());
        assert!(
            lines.is_empty(),
            "diff lines without file context must be ignored, got: {lines:?}"
        );
    }
}
