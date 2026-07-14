//! Phase L.2 — minimal markdown rendering for chat agent messages.
//!
//! Maps pulldown-cmark events to ratatui `Line<'static>` with the Captain
//! palette. Supports inline emphasis/strong/code, fenced code blocks,
//! headings, lists, blockquotes, and links. NOT a full HTML-grade renderer
//! — anything we don't recognise falls back to plain wrapped text so the
//! user never sees raw markdown when something exotic shows up.

use pulldown_cmark::{CodeBlockKind, Event, HeadingLevel, Options, Parser, Tag, TagEnd};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

use crate::tui::theme;

/// Render `text` as markdown into ratatui `Line`s wrapped to `max_width`.
/// Each returned `Line` already has the leading 2-space indent that
/// `draw_messages` applies to agent bubbles, so callers can render directly.
pub fn render(text: &str, max_width: usize) -> Vec<Line<'static>> {
    render_inner(text, max_width, false)
}

/// Like [`render`] but a single newline (markdown soft break) stays a real
/// line break instead of collapsing into a space. Used for user messages:
/// the line structure of a pasted block must survive submit.
pub fn render_preserving_newlines(text: &str, max_width: usize) -> Vec<Line<'static>> {
    render_inner(text, max_width, true)
}

fn render_inner(text: &str, max_width: usize, keep_soft_breaks: bool) -> Vec<Line<'static>> {
    if max_width < 8 {
        return vec![Line::from(text.to_string())];
    }

    let mut opts = Options::empty();
    opts.insert(Options::ENABLE_STRIKETHROUGH);
    opts.insert(Options::ENABLE_TABLES);
    let parser = Parser::new_ext(text, opts);

    let mut renderer = Renderer::new(max_width);
    renderer.keep_soft_breaks = keep_soft_breaks;
    for ev in parser {
        renderer.event(ev);
    }
    renderer.finish()
}

struct Renderer {
    max_width: usize,
    out: Vec<Line<'static>>,
    current: Vec<Span<'static>>,
    current_width: usize,
    style_stack: Vec<Style>,
    in_code_block: bool,
    code_buf: String,
    list_depth: usize,
    in_blockquote: bool,
    pending_link_url: Option<String>,
    in_table: bool,
    in_table_cell: bool,
    table_rows: Vec<Vec<String>>,
    table_row: Vec<String>,
    table_cell: String,
    keep_soft_breaks: bool,
}

const INDENT: &str = "  ";

impl Renderer {
    fn new(max_width: usize) -> Self {
        Self {
            max_width,
            out: Vec::new(),
            current: Vec::new(),
            current_width: 0,
            style_stack: Vec::new(),
            in_code_block: false,
            code_buf: String::new(),
            list_depth: 0,
            in_blockquote: false,
            pending_link_url: None,
            in_table: false,
            in_table_cell: false,
            table_rows: Vec::new(),
            table_row: Vec::new(),
            table_cell: String::new(),
            keep_soft_breaks: false,
        }
    }

    fn current_style(&self) -> Style {
        self.style_stack
            .last()
            .copied()
            .unwrap_or_else(Style::default)
    }

    fn push_style(&mut self, modifier: Option<Modifier>, color: Option<Color>) {
        let mut s = self.current_style();
        if let Some(m) = modifier {
            s = s.add_modifier(m);
        }
        if let Some(c) = color {
            s = s.fg(c);
        }
        self.style_stack.push(s);
    }

    fn pop_style(&mut self) {
        self.style_stack.pop();
    }

    fn flush_line(&mut self) {
        let line = std::mem::take(&mut self.current);
        self.out.push(Line::from(line));
        self.current_width = 0;
    }

    fn ensure_indent(&mut self) {
        if !self.current.is_empty() || self.current_width > 0 {
            return;
        }
        let prefix = if self.in_blockquote {
            format!("{INDENT}\u{2502} ")
        } else {
            INDENT.to_string()
        };
        self.current_width = display_width(&prefix);
        self.current.push(Span::raw(prefix));
    }

    fn push_text(&mut self, text: &str) {
        for word_with_ws in word_chunks(text) {
            self.push_chunk(&word_with_ws);
        }
    }

    fn push_chunk(&mut self, chunk: &str) {
        if chunk.is_empty() {
            return;
        }
        if chunk == "\n" {
            self.flush_line();
            return;
        }
        self.ensure_indent();
        let chunk_width = display_width(chunk);
        if self.current_width + chunk_width > self.max_width
            && self.current_width > display_width(INDENT)
        {
            self.flush_line();
            self.ensure_indent();
            // Skip a leading space after wrap.
            let trimmed = chunk.trim_start();
            if trimmed.is_empty() {
                return;
            }
            let style = self.current_style();
            self.current_width += display_width(trimmed);
            self.current.push(Span::styled(trimmed.to_string(), style));
        } else {
            let style = self.current_style();
            self.current_width += chunk_width;
            self.current.push(Span::styled(chunk.to_string(), style));
        }
    }

    fn render_code_block(&mut self) {
        let body = std::mem::take(&mut self.code_buf);
        let style = Style::default().fg(theme::CYAN).bg(theme::BG_CODE);
        for raw_line in body.lines() {
            // Wrap long code lines without breaking words: hard cut at width.
            if raw_line.is_empty() {
                self.out.push(Line::from(vec![Span::styled(
                    format!("{INDENT}\u{2502} "),
                    Style::default().fg(theme::TEXT_TERTIARY),
                )]));
                continue;
            }
            for slice in hard_wrap_display_width(raw_line, self.max_width.saturating_sub(4)) {
                self.out.push(Line::from(vec![
                    Span::styled(
                        format!("{INDENT}\u{2502} "),
                        Style::default().fg(theme::TEXT_TERTIARY),
                    ),
                    Span::styled(slice, style),
                ]));
            }
        }
    }

    fn push_table_cell_text(&mut self, text: &str) {
        self.table_cell.push_str(text);
    }

    fn render_table(&mut self) {
        let rows = std::mem::take(&mut self.table_rows);
        let has_generic_header = rows.first().is_some_and(|row| is_generic_table_header(row));

        // Tables without a meaningful header stay in the compact key/value
        // form; tables with a real header get a proper aligned rendering.
        if has_generic_header {
            self.render_headerless_table(rows.into_iter().skip(1));
        } else {
            self.render_aligned_table(rows);
        }
    }

    fn render_headerless_table(&mut self, rows: impl Iterator<Item = Vec<String>>) {
        for row in rows {
            let cells: Vec<String> = row
                .into_iter()
                .map(|cell| collapse_ws(&cell))
                .filter(|cell| !cell.is_empty())
                .collect();
            if cells.is_empty() {
                continue;
            }

            if cells.len() == 2 {
                self.render_key_value_row(&cells[0], &cells[1]);
            } else {
                self.render_flat_table_row(&cells.join("  |  "));
            }
        }
    }

    /// Render a real table: columns aligned on their width, header styled
    /// and separated, oversized cells wrapped within their column.
    fn render_aligned_table(&mut self, rows: Vec<Vec<String>>) {
        let rows: Vec<Vec<String>> = rows
            .into_iter()
            .map(|row| row.into_iter().map(|c| collapse_ws(&c)).collect())
            .filter(|row: &Vec<String>| row.iter().any(|c| !c.is_empty()))
            .collect();
        let col_count = rows.iter().map(|r| r.len()).max().unwrap_or(0);
        if col_count == 0 {
            return;
        }

        let widths = self.table_column_widths(&rows, col_count);
        let header_style = Style::default()
            .fg(theme::ACCENT)
            .add_modifier(Modifier::BOLD);
        let body_style = Style::default().fg(theme::TEXT_PRIMARY);
        let frame_style = Style::default().fg(theme::TEXT_TERTIARY);

        for (row_idx, row) in rows.iter().enumerate() {
            let style = if row_idx == 0 {
                header_style
            } else {
                body_style
            };
            self.render_table_row(row, &widths, style, frame_style);
            if row_idx == 0 {
                let sep: Vec<String> = widths.iter().map(|w| "\u{2500}".repeat(*w)).collect();
                self.out.push(Line::from(vec![
                    Span::raw(INDENT),
                    Span::styled(sep.join("\u{2500}\u{253c}\u{2500}"), frame_style),
                ]));
            }
        }
    }

    /// Column widths: natural widths shrunk (widest column first) until the
    /// table fits the render width.
    fn table_column_widths(&self, rows: &[Vec<String>], col_count: usize) -> Vec<usize> {
        let mut widths = vec![1usize; col_count];
        for row in rows {
            for (i, cell) in row.iter().enumerate() {
                widths[i] = widths[i].max(display_width(cell));
            }
        }
        let separators = 3 * col_count.saturating_sub(1);
        let available = self
            .max_width
            .saturating_sub(display_width(INDENT) + separators)
            .max(col_count * 4);
        while widths.iter().sum::<usize>() > available {
            let Some(widest) = widths.iter_mut().max() else {
                break;
            };
            if *widest <= 4 {
                break;
            }
            *widest -= 1;
        }
        widths
    }

    /// Render one logical row; cells longer than their column wrap onto
    /// extra physical lines within the column.
    fn render_table_row(
        &mut self,
        row: &[String],
        widths: &[usize],
        cell_style: Style,
        frame_style: Style,
    ) {
        let wrapped: Vec<Vec<String>> = widths
            .iter()
            .enumerate()
            .map(|(i, w)| wrap_plain_text(row.get(i).map(String::as_str).unwrap_or(""), *w))
            .collect();
        let height = wrapped.iter().map(Vec::len).max().unwrap_or(1);

        for line_idx in 0..height {
            let mut spans = vec![Span::raw(INDENT)];
            for (col, cell_lines) in wrapped.iter().enumerate() {
                if col > 0 {
                    spans.push(Span::styled(" \u{2502} ", frame_style));
                }
                let content = cell_lines
                    .get(line_idx)
                    .map(String::as_str)
                    .unwrap_or_default();
                let pad = widths[col].saturating_sub(display_width(content));
                spans.push(Span::styled(
                    format!("{content}{}", " ".repeat(pad)),
                    cell_style,
                ));
            }
            self.out.push(Line::from(spans));
        }
    }

    fn render_key_value_row(&mut self, key: &str, value: &str) {
        let label = format!("{key}: ");
        let label_width = display_width(INDENT) + display_width(&label);
        let value_width = self.max_width.saturating_sub(label_width).max(8);
        let wrapped = wrap_plain_text(value, value_width);
        let label_style = Style::default()
            .fg(theme::ACCENT_DIM)
            .add_modifier(Modifier::BOLD);
        let value_style = Style::default().fg(theme::TEXT_PRIMARY);

        for (idx, line) in wrapped.into_iter().enumerate() {
            if idx == 0 {
                self.out.push(Line::from(vec![
                    Span::raw(INDENT),
                    Span::styled(label.clone(), label_style),
                    Span::styled(line, value_style),
                ]));
            } else {
                self.out.push(Line::from(vec![
                    Span::raw(" ".repeat(label_width)),
                    Span::styled(line, value_style),
                ]));
            }
        }
    }

    fn render_flat_table_row(&mut self, row: &str) {
        let width = self.max_width.saturating_sub(display_width(INDENT)).max(8);
        let style = Style::default().fg(theme::TEXT_PRIMARY);
        for line in wrap_plain_text(row, width) {
            self.out.push(Line::from(vec![
                Span::raw(INDENT),
                Span::styled(line, style),
            ]));
        }
    }

    fn event(&mut self, ev: Event<'_>) {
        match ev {
            Event::Start(tag) => self.start(tag),
            Event::End(tag) => self.end(tag),
            Event::Text(t) => {
                if self.in_table_cell {
                    self.push_table_cell_text(&t);
                } else if self.in_code_block {
                    self.code_buf.push_str(&t);
                } else {
                    self.push_text(&t);
                }
            }
            Event::Code(c) => {
                if self.in_table_cell {
                    self.push_table_cell_text(&c);
                    return;
                }
                let style = Style::default().fg(theme::CYAN).bg(theme::BG_CODE);
                self.ensure_indent();
                let chunk = format!(" {c} ");
                let w = display_width(&chunk);
                if self.current_width + w > self.max_width
                    && self.current_width > display_width(INDENT)
                {
                    self.flush_line();
                    self.ensure_indent();
                }
                self.current_width += w;
                self.current.push(Span::styled(chunk, style));
            }
            Event::SoftBreak => {
                if self.in_table_cell {
                    self.push_table_cell_text(" ");
                } else if self.keep_soft_breaks {
                    self.flush_line();
                } else {
                    self.push_text(" ");
                }
            }
            Event::HardBreak => {
                if self.in_table_cell {
                    self.push_table_cell_text(" ");
                } else {
                    self.flush_line();
                }
            }
            Event::Rule => {
                self.flush_line();
                let rule = "\u{2500}".repeat(self.max_width.saturating_sub(display_width(INDENT)));
                self.out.push(Line::from(vec![
                    Span::raw(INDENT),
                    Span::styled(rule, Style::default().fg(theme::TEXT_TERTIARY)),
                ]));
            }
            Event::Html(h) | Event::InlineHtml(h) => {
                if self.in_table_cell {
                    self.push_table_cell_text(&h);
                } else {
                    self.push_text(&h);
                }
            }
            Event::FootnoteReference(_) | Event::TaskListMarker(_) => {}
        }
    }

    fn start(&mut self, tag: Tag<'_>) {
        match tag {
            Tag::Paragraph => {}
            Tag::Heading { level, .. } => {
                self.flush_line();
                let prefix = match level {
                    HeadingLevel::H1 => "# ",
                    HeadingLevel::H2 => "## ",
                    HeadingLevel::H3 => "### ",
                    _ => "#### ",
                };
                self.ensure_indent();
                let style = Style::default()
                    .fg(theme::ACCENT)
                    .add_modifier(Modifier::BOLD);
                self.current_width += display_width(prefix);
                self.current.push(Span::styled(prefix.to_string(), style));
                self.push_style(Some(Modifier::BOLD), Some(theme::ACCENT));
            }
            Tag::BlockQuote => {
                self.in_blockquote = true;
            }
            Tag::CodeBlock(kind) => {
                self.flush_line();
                self.in_code_block = true;
                self.code_buf.clear();
                let lang = match kind {
                    CodeBlockKind::Fenced(l) => l.to_string(),
                    CodeBlockKind::Indented => String::new(),
                };
                if !lang.is_empty() {
                    self.out.push(Line::from(vec![
                        Span::styled(
                            format!("{INDENT}\u{250c}\u{2500} "),
                            Style::default().fg(theme::TEXT_TERTIARY),
                        ),
                        Span::styled(lang, Style::default().fg(theme::ACCENT_DIM)),
                    ]));
                }
            }
            Tag::List(_) => {
                self.list_depth += 1;
            }
            Tag::Item => {
                self.flush_line();
                self.ensure_indent();
                let bullet = if self.list_depth > 1 { "  - " } else { "- " };
                let style = Style::default().fg(theme::ACCENT);
                self.current_width += display_width(bullet);
                self.current.push(Span::styled(bullet.to_string(), style));
            }
            Tag::Emphasis => self.push_style(Some(Modifier::ITALIC), None),
            Tag::Strong => self.push_style(Some(Modifier::BOLD), None),
            Tag::Strikethrough => self.push_style(Some(Modifier::CROSSED_OUT), None),
            Tag::Link { dest_url, .. } => {
                self.pending_link_url = Some(dest_url.to_string());
                self.push_style(Some(Modifier::UNDERLINED), Some(theme::BLUE));
            }
            Tag::Table(_) => {
                self.flush_line();
                self.in_table = true;
                self.table_rows.clear();
                self.table_row.clear();
                self.table_cell.clear();
            }
            Tag::TableHead | Tag::TableRow => {
                self.table_row.clear();
            }
            Tag::TableCell => {
                self.in_table_cell = true;
                self.table_cell.clear();
            }
            _ => {}
        }
    }

    fn end(&mut self, tag: TagEnd) {
        match tag {
            TagEnd::Paragraph => self.flush_line(),
            TagEnd::Heading(_) => {
                self.flush_line();
                self.pop_style();
            }
            TagEnd::BlockQuote => {
                self.in_blockquote = false;
            }
            TagEnd::CodeBlock => {
                self.render_code_block();
                self.in_code_block = false;
                let footer =
                    "\u{2500}".repeat(self.max_width.saturating_sub(display_width(INDENT) + 2));
                self.out.push(Line::from(vec![Span::styled(
                    format!("{INDENT}\u{2514}{footer}"),
                    Style::default().fg(theme::TEXT_TERTIARY),
                )]));
            }
            TagEnd::List(_) => {
                self.list_depth = self.list_depth.saturating_sub(1);
            }
            TagEnd::Item => self.flush_line(),
            TagEnd::Emphasis | TagEnd::Strong | TagEnd::Strikethrough => self.pop_style(),
            TagEnd::Link => {
                self.pop_style();
                if let Some(url) = self.pending_link_url.take() {
                    if self.in_table_cell {
                        self.push_table_cell_text(&format!(" ({url})"));
                    } else {
                        self.push_chunk(&format!(" ({url})"));
                    }
                }
            }
            TagEnd::TableCell => {
                self.in_table_cell = false;
                self.table_row.push(std::mem::take(&mut self.table_cell));
            }
            TagEnd::TableHead | TagEnd::TableRow => {
                if !self.table_row.is_empty() {
                    self.table_rows.push(std::mem::take(&mut self.table_row));
                }
            }
            TagEnd::Table => {
                self.in_table = false;
                self.in_table_cell = false;
                self.render_table();
            }
            _ => {}
        }
    }

    fn finish(mut self) -> Vec<Line<'static>> {
        if self.in_table {
            if self.in_table_cell {
                self.table_row.push(std::mem::take(&mut self.table_cell));
            }
            if !self.table_row.is_empty() {
                self.table_rows.push(std::mem::take(&mut self.table_row));
            }
            self.render_table();
        }
        if !self.current.is_empty() {
            self.flush_line();
        }
        self.out
    }
}

/// Split text on whitespace boundaries while preserving the spaces, so
/// the wrapper can break only on boundaries. Returns a Vec of String
/// (vs slices) because pulldown's Cow makes lifetime juggling painful.
fn word_chunks(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut buf = String::new();
    let mut in_ws = false;
    for c in text.chars() {
        let is_ws = c.is_whitespace() && c != '\n';
        if c == '\n' {
            if !buf.is_empty() {
                out.push(std::mem::take(&mut buf));
            }
            out.push("\n".to_string());
            in_ws = false;
            continue;
        }
        if is_ws != in_ws && !buf.is_empty() {
            out.push(std::mem::take(&mut buf));
        }
        buf.push(c);
        in_ws = is_ws;
    }
    if !buf.is_empty() {
        out.push(buf);
    }
    out
}

fn collapse_ws(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn display_width(text: &str) -> usize {
    UnicodeWidthStr::width(text)
}

fn hard_wrap_display_width(text: &str, max_width: usize) -> Vec<String> {
    let max_width = max_width.max(1);
    let mut lines = Vec::new();
    let mut current = String::new();
    let mut current_width = 0;

    for character in text.chars() {
        let character_width = UnicodeWidthChar::width(character).unwrap_or(0);
        if !current.is_empty() && current_width + character_width > max_width {
            lines.push(std::mem::take(&mut current));
            current_width = 0;
        }
        current.push(character);
        current_width += character_width;
    }

    if !current.is_empty() {
        lines.push(current);
    }
    lines
}

fn normalize_table_heading(text: &str) -> String {
    collapse_ws(text)
        .trim_matches(|c: char| !c.is_alphanumeric())
        .to_ascii_lowercase()
}

fn is_generic_table_header(row: &[String]) -> bool {
    if row.iter().all(|cell| cell.trim().is_empty()) {
        return true;
    }

    if row.len() != 2 {
        return false;
    }

    let left = normalize_table_heading(&row[0]);
    let right = normalize_table_heading(&row[1]);
    let left_generic = matches!(
        left.as_str(),
        "indicateur" | "metric" | "métrique" | "metrique" | "clé" | "cle" | "key" | "item"
    );
    let right_generic = matches!(
        right.as_str(),
        "état" | "etat" | "status" | "value" | "valeur" | "result" | "résultat" | "resultat"
    );
    left_generic && right_generic
}

fn wrap_plain_text(text: &str, max_width: usize) -> Vec<String> {
    let max_width = max_width.max(1);
    let mut out = Vec::new();
    let mut current = String::new();
    let mut current_width = 0;

    for chunk in word_chunks(text) {
        let chunk_width = display_width(&chunk);
        if current_width + chunk_width > max_width && current_width > 0 {
            out.push(current.trim_end().to_string());
            current.clear();
            let trimmed = chunk.trim_start();
            current.push_str(trimmed);
            current_width = display_width(trimmed);
        } else {
            current.push_str(&chunk);
            current_width += chunk_width;
        }
    }

    if !current.is_empty() {
        out.push(current.trim_end().to_string());
    }
    if out.is_empty() {
        out.push(String::new());
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plain_text_wraps_with_indent() {
        let out = render("hello world", 80);
        assert!(!out.is_empty());
        let first = format!("{:?}", out[0]);
        assert!(first.contains("hello"));
    }

    #[test]
    fn bold_keeps_text_visible() {
        let out = render("this is **bold** text", 80);
        let joined: String = out
            .iter()
            .map(|l| {
                l.spans
                    .iter()
                    .map(|s| s.content.as_ref())
                    .collect::<String>()
            })
            .collect::<Vec<_>>()
            .join("\n");
        assert!(joined.contains("bold"));
    }

    #[test]
    fn fenced_code_block_renders() {
        let md = "```rust\nfn main() {}\n```";
        let out = render(md, 80);
        let joined: String = out
            .iter()
            .map(|l| {
                l.spans
                    .iter()
                    .map(|s| s.content.as_ref())
                    .collect::<String>()
            })
            .collect::<Vec<_>>()
            .join("\n");
        assert!(joined.contains("fn main()"));
        assert!(joined.contains("rust"));
    }

    #[test]
    fn heading_renders_with_prefix() {
        let out = render("# Title", 80);
        let joined: String = out
            .iter()
            .map(|l| {
                l.spans
                    .iter()
                    .map(|s| s.content.as_ref())
                    .collect::<String>()
            })
            .collect::<Vec<_>>()
            .join("\n");
        assert!(joined.contains("Title"));
        assert!(joined.contains('#'));
    }

    #[test]
    fn narrow_width_falls_back_safely() {
        let out = render("anything", 4);
        assert_eq!(out.len(), 1);
    }

    #[test]
    fn preserving_newlines_keeps_pasted_line_structure() {
        let pasted = "ligne un\nligne deux\nligne trois";
        let flat = render(pasted, 80);
        let kept = render_preserving_newlines(pasted, 80);
        assert_eq!(flat.len(), 1, "soft breaks collapse in the default mode");
        assert_eq!(kept.len(), 3, "each pasted line stays its own line");
    }

    fn lines_to_text(out: &[Line<'static>]) -> String {
        out.iter()
            .map(|l| {
                l.spans
                    .iter()
                    .map(|s| s.content.as_ref())
                    .collect::<String>()
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    /// A table with a real header renders as an aligned table: header row,
    /// separator, and column-aligned cells — not a flat "a | b | c" line.
    #[test]
    fn real_header_table_renders_aligned_columns() {
        let md = "| Commit | Contenu |\n|---|---|\n| d416397 | Élagage des sorties |\n| bc61686 | Checkpoint déterministe |";
        let out = render(md, 80);
        let joined = lines_to_text(&out);

        assert!(joined.contains("Commit"));
        assert!(joined.contains("\u{2502}"), "column separator present");
        assert!(joined.contains("\u{253c}"), "header separator junction");
        assert!(joined.contains("d416397"));
        assert!(joined.contains("Checkpoint déterministe"));
        // Header and body cells align on the same column boundary.
        let header_line = out
            .iter()
            .map(|l| {
                l.spans
                    .iter()
                    .map(|s| s.content.as_ref())
                    .collect::<String>()
            })
            .find(|l| l.contains("Commit"))
            .unwrap();
        let body_line = out
            .iter()
            .map(|l| {
                l.spans
                    .iter()
                    .map(|s| s.content.as_ref())
                    .collect::<String>()
            })
            .find(|l| l.contains("d416397"))
            .unwrap();
        assert_eq!(
            header_line.find('\u{2502}'),
            body_line.find('\u{2502}'),
            "columns are aligned"
        );
    }

    /// Long cells wrap inside their column instead of blowing up the row.
    #[test]
    fn aligned_table_wraps_long_cells() {
        let md = "| Nom | Description |\n|---|---|\n| x | Une description très longue qui dépasse largement la largeur disponible pour cette colonne du tableau rendu |";
        let out = render(md, 40);
        let joined = lines_to_text(&out);

        assert!(joined.contains("description"));
        assert!(joined.contains("tableau rendu"), "wrapped tail is kept");
        for line in joined.lines() {
            assert!(display_width(line) <= 40, "line exceeds width: {line:?}");
        }
    }

    #[test]
    fn emoji_wraps_by_terminal_cell_width() {
        let out = render("123456 👍", 10);
        let lines: Vec<String> = out
            .iter()
            .map(|line| {
                line.spans
                    .iter()
                    .map(|span| span.content.as_ref())
                    .collect()
            })
            .collect();

        assert_eq!(lines.len(), 2);
        assert_eq!(lines[0], "  123456 ");
        assert_eq!(lines[1], "  👍");
        assert!(lines.iter().all(|line| display_width(line) <= 10));
    }

    #[test]
    fn table_cells_do_not_merge() {
        let md = "| | |\n|---|---|\n| **Uptime** | 13 jours sans redémarrage |\n| **Disque** | 11G / 460G utilisés (17%) |";
        let out = render(md, 80);
        let joined: String = out
            .iter()
            .map(|l| {
                l.spans
                    .iter()
                    .map(|s| s.content.as_ref())
                    .collect::<String>()
            })
            .collect::<Vec<_>>()
            .join("\n");
        assert!(joined.contains("Uptime: 13 jours"));
        assert!(joined.contains("Disque: 11G / 460G"));
        assert!(!joined.contains("Uptime13"));
        assert!(!joined.contains("Disque11G"));
    }
}
