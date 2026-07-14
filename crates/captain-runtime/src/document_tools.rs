//! Native document generation for Captain.
//!
//! The first implementation intentionally avoids external renderers. Captain
//! must be able to create a useful report on a fresh machine, then richer
//! backends such as Typst can layer on top without making the base rail brittle.

use std::io::Write;
use std::path::{Path, PathBuf};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum DocumentFormat {
    Pdf,
    Docx,
    Html,
    Markdown,
}

impl DocumentFormat {
    fn parse(value: &str) -> Result<Self, String> {
        match value.trim().to_ascii_lowercase().as_str() {
            "pdf" => Ok(Self::Pdf),
            "docx" | "word" => Ok(Self::Docx),
            "html" | "htm" => Ok(Self::Html),
            "md" | "markdown" => Ok(Self::Markdown),
            other => Err(format!(
                "Unsupported document format '{other}'. Use one of: pdf, docx, html, markdown."
            )),
        }
    }

    fn from_path(path: &str) -> Option<Self> {
        Path::new(path)
            .extension()
            .and_then(|e| e.to_str())
            .and_then(|e| Self::parse(e).ok())
    }

    fn extension(self) -> &'static str {
        match self {
            Self::Pdf => "pdf",
            Self::Docx => "docx",
            Self::Html => "html",
            Self::Markdown => "md",
        }
    }

    fn as_str(self) -> &'static str {
        match self {
            Self::Pdf => "pdf",
            Self::Docx => "docx",
            Self::Html => "html",
            Self::Markdown => "markdown",
        }
    }

    fn mime(self) -> &'static str {
        match self {
            Self::Pdf => "application/pdf",
            Self::Docx => "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
            Self::Html => "text/html",
            Self::Markdown => "text/markdown",
        }
    }
}

#[derive(Clone, Debug)]
struct Citation {
    id: String,
    title: String,
    url: Option<String>,
    accessed_at: Option<String>,
}

#[derive(Clone, Debug)]
enum DocElement {
    Heading {
        level: u8,
        text: String,
    },
    Paragraph(String),
    Bullet(String),
    Table {
        headers: Vec<String>,
        rows: Vec<Vec<String>>,
    },
    Spacer,
}

#[derive(Clone, Debug)]
struct Document {
    title: String,
    subtitle: Option<String>,
    author: Option<String>,
    elements: Vec<DocElement>,
    citations: Vec<Citation>,
}

struct RenderedDocument {
    bytes: Vec<u8>,
    pages: Option<usize>,
}

/// Native `document_create` implementation.
pub async fn create_document(
    input: &serde_json::Value,
    workspace_root: Option<&Path>,
) -> Result<String, String> {
    let title = input["title"]
        .as_str()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .unwrap_or("Captain document")
        .to_string();
    crate::tool_runner::ensure_no_secret_literal("document_create", "title", &title)?;

    let subtitle = optional_string(input, "subtitle");
    let author = optional_string(input, "author");
    if let Some(value) = &subtitle {
        crate::tool_runner::ensure_no_secret_literal("document_create", "subtitle", value)?;
    }
    if let Some(value) = &author {
        crate::tool_runner::ensure_no_secret_literal("document_create", "author", value)?;
    }

    let format = match input["format"].as_str() {
        Some(raw) => DocumentFormat::parse(raw)?,
        None => input["path"]
            .as_str()
            .and_then(DocumentFormat::from_path)
            .unwrap_or(DocumentFormat::Pdf),
    };

    let raw_path = input["path"]
        .as_str()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(ToOwned::to_owned)
        .unwrap_or_else(|| default_document_path(&title, format));
    let resolved = resolve_output_path(&raw_path, workspace_root)?;
    let overwrite = input["overwrite"].as_bool().unwrap_or(false);
    if !overwrite && tokio::fs::try_exists(&resolved).await.unwrap_or(false) {
        return Err(format!(
            "Refusing to overwrite existing document '{}'. Pass overwrite=true to replace it.",
            resolved.display()
        ));
    }

    let mut elements = Vec::new();
    if let Some(content) = input["content"].as_str() {
        crate::tool_runner::ensure_no_secret_literal("document_create", "content", content)?;
        elements.extend(parse_markdownish(content));
    }
    if let Some(sections) = input["sections"].as_array() {
        elements.extend(parse_sections(sections)?);
    }
    trim_edge_spacers(&mut elements);
    dedupe_leading_title_heading(&mut elements, &title);
    if elements.is_empty() {
        return Err(
            "Missing document body: provide 'content' or at least one 'sections' item.".into(),
        );
    }

    let citations = parse_citations(input.get("citations"))?;
    let document = Document {
        title,
        subtitle,
        author,
        elements,
        citations,
    };

    let rendered = match format {
        DocumentFormat::Pdf => render_pdf(&document),
        DocumentFormat::Docx => render_docx(&document)?,
        DocumentFormat::Html => RenderedDocument {
            bytes: render_html(&document).into_bytes(),
            pages: None,
        },
        DocumentFormat::Markdown => RenderedDocument {
            bytes: render_markdown(&document).into_bytes(),
            pages: None,
        },
    };

    if let Some(parent) = resolved.parent() {
        tokio::fs::create_dir_all(parent)
            .await
            .map_err(|e| format!("Failed to create document directory: {e}"))?;
    }
    tokio::fs::write(&resolved, &rendered.bytes)
        .await
        .map_err(|e| format!("Failed to write document '{}': {e}", resolved.display()))?;

    Ok(serde_json::json!({
        "success": true,
        "tool": "document_create",
        "format": format.as_str(),
        "mime_type": format.mime(),
        "path": resolved.display().to_string(),
        "size_bytes": rendered.bytes.len(),
        "pages": rendered.pages,
        "elements": document.elements.len(),
        "citations": document.citations.len(),
        "next_action": "Use channel_send with file_path if the document should be sent to a chat channel."
    })
    .to_string())
}

fn optional_string(input: &serde_json::Value, key: &str) -> Option<String> {
    input[key]
        .as_str()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(ToOwned::to_owned)
}

fn default_document_path(title: &str, format: DocumentFormat) -> String {
    let slug = slugify(title);
    format!("documents/{slug}.{}", format.extension())
}

fn slugify(title: &str) -> String {
    let mut out = String::new();
    let mut last_dash = false;
    for ch in title.chars() {
        if ch.is_ascii_alphanumeric() {
            out.push(ch.to_ascii_lowercase());
            last_dash = false;
        } else if !last_dash && !out.is_empty() {
            out.push('-');
            last_dash = true;
        }
        if out.len() >= 80 {
            break;
        }
    }
    let trimmed = out.trim_matches('-');
    if trimmed.is_empty() {
        "captain-document".to_string()
    } else {
        trimmed.to_string()
    }
}

fn resolve_output_path(raw_path: &str, workspace_root: Option<&Path>) -> Result<PathBuf, String> {
    for component in Path::new(raw_path).components() {
        if matches!(component, std::path::Component::ParentDir) {
            return Err("Path traversal denied: '..' components are forbidden".to_string());
        }
    }
    if let Some(root) = workspace_root {
        let path = Path::new(raw_path);
        if path.is_absolute() {
            return Err("document_create path must be relative to the workspace".to_string());
        }
        Ok(root.join(path))
    } else {
        Ok(PathBuf::from(raw_path))
    }
}

fn parse_sections(sections: &[serde_json::Value]) -> Result<Vec<DocElement>, String> {
    let mut out = Vec::new();
    for section in sections {
        if let Some(heading) = section["heading"]
            .as_str()
            .map(str::trim)
            .filter(|s| !s.is_empty())
        {
            crate::tool_runner::ensure_no_secret_literal(
                "document_create",
                "sections.heading",
                heading,
            )?;
            let level = section["level"].as_u64().unwrap_or(1).clamp(1, 6) as u8;
            out.push(DocElement::Heading {
                level,
                text: heading.to_string(),
            });
        }
        if let Some(body) = section["body"]
            .as_str()
            .map(str::trim)
            .filter(|s| !s.is_empty())
        {
            crate::tool_runner::ensure_no_secret_literal("document_create", "sections.body", body)?;
            out.extend(parse_markdownish(body));
        }
        if let Some(items) = section["bullets"].as_array() {
            for item in items {
                let text = item
                    .as_str()
                    .ok_or("sections[].bullets must contain only strings")?
                    .trim();
                if !text.is_empty() {
                    crate::tool_runner::ensure_no_secret_literal(
                        "document_create",
                        "sections.bullets",
                        text,
                    )?;
                    out.push(DocElement::Bullet(text.to_string()));
                }
            }
        }
        if let Some(table) = section.get("table").filter(|v| v.is_object()) {
            out.push(parse_structured_table(table)?);
        }
        out.push(DocElement::Spacer);
    }
    Ok(out)
}

fn parse_structured_table(table: &serde_json::Value) -> Result<DocElement, String> {
    let headers = table["headers"]
        .as_array()
        .ok_or("table.headers must be an array of strings")?
        .iter()
        .map(json_string)
        .collect::<Result<Vec<_>, _>>()?;
    let rows = table["rows"]
        .as_array()
        .ok_or("table.rows must be an array of row arrays")?
        .iter()
        .map(|row| {
            row.as_array()
                .ok_or("table.rows items must be arrays".to_string())?
                .iter()
                .map(json_string)
                .collect::<Result<Vec<_>, _>>()
        })
        .collect::<Result<Vec<_>, _>>()?;
    for value in headers.iter().chain(rows.iter().flatten()) {
        crate::tool_runner::ensure_no_secret_literal("document_create", "table", value)?;
    }
    Ok(DocElement::Table { headers, rows })
}

fn json_string(value: &serde_json::Value) -> Result<String, String> {
    value
        .as_str()
        .map(ToOwned::to_owned)
        .ok_or_else(|| "Expected string value".to_string())
}

fn parse_citations(value: Option<&serde_json::Value>) -> Result<Vec<Citation>, String> {
    let Some(array) = value.and_then(|v| v.as_array()) else {
        return Ok(Vec::new());
    };
    let mut out = Vec::new();
    for (idx, item) in array.iter().enumerate() {
        let title = item["title"]
            .as_str()
            .or_else(|| item["url"].as_str())
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .ok_or("citations[].title or citations[].url is required")?
            .to_string();
        crate::tool_runner::ensure_no_secret_literal("document_create", "citations.title", &title)?;
        let id = item["id"]
            .as_str()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(ToOwned::to_owned)
            .unwrap_or_else(|| format!("{}", idx + 1));
        let url = optional_string(item, "url");
        let accessed_at = optional_string(item, "accessed_at");
        if let Some(value) = &url {
            crate::tool_runner::ensure_no_secret_literal(
                "document_create",
                "citations.url",
                value,
            )?;
        }
        out.push(Citation {
            id,
            title,
            url,
            accessed_at,
        });
    }
    Ok(out)
}

fn parse_markdownish(input: &str) -> Vec<DocElement> {
    let lines: Vec<&str> = input.lines().collect();
    let mut out = Vec::new();
    let mut idx = 0;
    while idx < lines.len() {
        let line = lines[idx].trim_end();
        let trimmed = line.trim();
        if trimmed.is_empty() {
            out.push(DocElement::Spacer);
            idx += 1;
            continue;
        }
        if let Some((level, text)) = parse_heading(trimmed) {
            out.push(DocElement::Heading { level, text });
            idx += 1;
            continue;
        }
        if looks_like_table_start(&lines, idx) {
            let (table, next_idx) = parse_pipe_table(&lines, idx);
            out.push(table);
            idx = next_idx;
            continue;
        }
        if let Some(text) = parse_bullet(trimmed) {
            out.push(DocElement::Bullet(text));
            idx += 1;
            continue;
        }

        let mut paragraph = String::from(trimmed);
        idx += 1;
        while idx < lines.len() {
            let next = lines[idx].trim();
            if next.is_empty()
                || parse_heading(next).is_some()
                || parse_bullet(next).is_some()
                || looks_like_table_start(&lines, idx)
            {
                break;
            }
            paragraph.push(' ');
            paragraph.push_str(next);
            idx += 1;
        }
        out.push(DocElement::Paragraph(paragraph));
    }
    out
}

fn parse_heading(line: &str) -> Option<(u8, String)> {
    let level = line.chars().take_while(|&c| c == '#').count();
    if (1..=6).contains(&level) && line.chars().nth(level) == Some(' ') {
        Some((level as u8, line[level + 1..].trim().to_string()))
    } else {
        None
    }
}

fn parse_bullet(line: &str) -> Option<String> {
    line.strip_prefix("- ")
        .or_else(|| line.strip_prefix("* "))
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(ToOwned::to_owned)
}

fn looks_like_table_start(lines: &[&str], idx: usize) -> bool {
    idx + 1 < lines.len()
        && lines[idx].trim().contains('|')
        && lines[idx + 1]
            .trim()
            .chars()
            .all(|c| matches!(c, '|' | '-' | ':' | ' '))
        && lines[idx + 1].trim().contains("---")
}

fn parse_pipe_table(lines: &[&str], start: usize) -> (DocElement, usize) {
    let headers = split_pipe_row(lines[start]);
    let mut rows = Vec::new();
    let mut idx = start + 2;
    while idx < lines.len() && lines[idx].trim().contains('|') && !lines[idx].trim().is_empty() {
        rows.push(split_pipe_row(lines[idx]));
        idx += 1;
    }
    (DocElement::Table { headers, rows }, idx)
}

fn split_pipe_row(line: &str) -> Vec<String> {
    line.trim()
        .trim_matches('|')
        .split('|')
        .map(|cell| cell.trim().to_string())
        .collect()
}

fn trim_edge_spacers(elements: &mut Vec<DocElement>) {
    while matches!(elements.first(), Some(DocElement::Spacer)) {
        elements.remove(0);
    }
    while matches!(elements.last(), Some(DocElement::Spacer)) {
        elements.pop();
    }
}

fn dedupe_leading_title_heading(elements: &mut Vec<DocElement>, title: &str) {
    let Some(DocElement::Heading { level: 1, text }) = elements.first() else {
        return;
    };
    if normalized_heading_key(text) == normalized_heading_key(title) {
        elements.remove(0);
        trim_edge_spacers(elements);
    }
}

fn normalized_heading_key(value: &str) -> String {
    value
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase()
}

fn render_markdown(doc: &Document) -> String {
    let mut out = String::new();
    out.push_str("# ");
    out.push_str(&doc.title);
    out.push_str("\n\n");
    if let Some(subtitle) = &doc.subtitle {
        out.push_str("**");
        out.push_str(subtitle);
        out.push_str("**\n\n");
    }
    if let Some(author) = &doc.author {
        out.push('_');
        out.push_str(author);
        out.push_str("_\n\n");
    }
    for element in &doc.elements {
        match element {
            DocElement::Heading { level, text } => {
                out.push_str(&"#".repeat((*level).clamp(1, 6) as usize));
                out.push(' ');
                out.push_str(text);
                out.push_str("\n\n");
            }
            DocElement::Paragraph(text) => {
                out.push_str(text);
                out.push_str("\n\n");
            }
            DocElement::Bullet(text) => {
                out.push_str("- ");
                out.push_str(text);
                out.push('\n');
            }
            DocElement::Table { headers, rows } => {
                out.push('|');
                out.push_str(&headers.join(" | "));
                out.push_str("|\n|");
                out.push_str(
                    &headers
                        .iter()
                        .map(|_| "---")
                        .collect::<Vec<_>>()
                        .join(" | "),
                );
                out.push_str("|\n");
                for row in rows {
                    out.push('|');
                    out.push_str(&row.join(" | "));
                    out.push_str("|\n");
                }
                out.push('\n');
            }
            DocElement::Spacer => out.push('\n'),
        }
    }
    append_markdown_citations(doc, &mut out);
    out
}

fn append_markdown_citations(doc: &Document, out: &mut String) {
    if doc.citations.is_empty() {
        return;
    }
    out.push_str("## Sources\n\n");
    for citation in &doc.citations {
        out.push_str("- [");
        out.push_str(&citation.id);
        out.push_str("] ");
        out.push_str(&citation.title);
        if let Some(url) = &citation.url {
            out.push_str(" — ");
            out.push_str(url);
        }
        if let Some(accessed_at) = &citation.accessed_at {
            out.push_str(" (consulted ");
            out.push_str(accessed_at);
            out.push(')');
        }
        out.push('\n');
    }
}

fn render_html(doc: &Document) -> String {
    let mut out = String::from(
        "<!doctype html><html><head><meta charset=\"utf-8\"><style>\
body{font-family:-apple-system,BlinkMacSystemFont,'Segoe UI',sans-serif;line-height:1.5;margin:48px;max-width:860px;color:#18212f}\
h1{font-size:32px;margin-bottom:4px}h2{margin-top:28px;border-bottom:1px solid #d7dde8;padding-bottom:4px}\
p{margin:10px 0}table{border-collapse:collapse;width:100%;margin:16px 0}th,td{border:1px solid #c9d1dc;padding:7px;text-align:left}th{background:#eef3f8}\
.subtitle{font-size:18px;color:#58657a}.author{color:#6b7280}.sources{margin-top:32px}\
</style></head><body>",
    );
    out.push_str("<h1>");
    out.push_str(&escape_html(&doc.title));
    out.push_str("</h1>");
    if let Some(subtitle) = &doc.subtitle {
        out.push_str("<p class=\"subtitle\">");
        out.push_str(&escape_html(subtitle));
        out.push_str("</p>");
    }
    if let Some(author) = &doc.author {
        out.push_str("<p class=\"author\">");
        out.push_str(&escape_html(author));
        out.push_str("</p>");
    }
    let mut open_list = false;
    for element in &doc.elements {
        match element {
            DocElement::Heading { level, text } => {
                close_list(&mut out, &mut open_list);
                let level = (*level).clamp(1, 6);
                out.push_str(&format!("<h{level}>"));
                out.push_str(&escape_html(text));
                out.push_str(&format!("</h{level}>"));
            }
            DocElement::Paragraph(text) => {
                close_list(&mut out, &mut open_list);
                out.push_str("<p>");
                out.push_str(&escape_html(text));
                out.push_str("</p>");
            }
            DocElement::Bullet(text) => {
                if !open_list {
                    out.push_str("<ul>");
                    open_list = true;
                }
                out.push_str("<li>");
                out.push_str(&escape_html(text));
                out.push_str("</li>");
            }
            DocElement::Table { headers, rows } => {
                close_list(&mut out, &mut open_list);
                out.push_str("<table><thead><tr>");
                for header in headers {
                    out.push_str("<th>");
                    out.push_str(&escape_html(header));
                    out.push_str("</th>");
                }
                out.push_str("</tr></thead><tbody>");
                for row in rows {
                    out.push_str("<tr>");
                    for cell in row {
                        out.push_str("<td>");
                        out.push_str(&escape_html(cell));
                        out.push_str("</td>");
                    }
                    out.push_str("</tr>");
                }
                out.push_str("</tbody></table>");
            }
            DocElement::Spacer => {
                close_list(&mut out, &mut open_list);
            }
        }
    }
    close_list(&mut out, &mut open_list);
    if !doc.citations.is_empty() {
        out.push_str("<section class=\"sources\"><h2>Sources</h2><ol>");
        for citation in &doc.citations {
            out.push_str("<li>");
            out.push_str(&escape_html(&citation.title));
            if let Some(url) = &citation.url {
                out.push_str(" <a href=\"");
                out.push_str(&escape_html(url));
                out.push_str("\">");
                out.push_str(&escape_html(url));
                out.push_str("</a>");
            }
            out.push_str("</li>");
        }
        out.push_str("</ol></section>");
    }
    out.push_str("</body></html>");
    out
}

fn close_list(out: &mut String, open_list: &mut bool) {
    if *open_list {
        out.push_str("</ul>");
        *open_list = false;
    }
}

fn escape_html(input: &str) -> String {
    input
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#39;")
}

fn render_docx(doc: &Document) -> Result<RenderedDocument, String> {
    let mut cursor = std::io::Cursor::new(Vec::new());
    let mut zip = zip::ZipWriter::new(&mut cursor);
    let options = zip::write::SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Deflated);

    zip.start_file("[Content_Types].xml", options)
        .map_err(zip_err)?;
    zip.write_all(CONTENT_TYPES_XML.as_bytes())
        .map_err(zip_err)?;
    zip.add_directory("_rels/", options).map_err(zip_err)?;
    zip.start_file("_rels/.rels", options).map_err(zip_err)?;
    zip.write_all(RELS_XML.as_bytes()).map_err(zip_err)?;
    zip.add_directory("word/", options).map_err(zip_err)?;
    zip.start_file("word/styles.xml", options)
        .map_err(zip_err)?;
    zip.write_all(STYLES_XML.as_bytes()).map_err(zip_err)?;
    zip.start_file("word/document.xml", options)
        .map_err(zip_err)?;
    zip.write_all(build_docx_document_xml(doc).as_bytes())
        .map_err(zip_err)?;
    zip.finish().map_err(zip_err)?;

    Ok(RenderedDocument {
        bytes: cursor.into_inner(),
        pages: None,
    })
}

fn zip_err<E: std::fmt::Display>(err: E) -> String {
    format!("DOCX archive error: {err}")
}

fn build_docx_document_xml(doc: &Document) -> String {
    let mut out = String::from(
        r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main">
<w:body>"#,
    );
    push_docx_paragraph(&mut out, Some("Title"), &doc.title);
    if let Some(subtitle) = &doc.subtitle {
        push_docx_paragraph(&mut out, Some("Subtitle"), subtitle);
    }
    if let Some(author) = &doc.author {
        push_docx_paragraph(&mut out, None, author);
    }
    for element in &doc.elements {
        match element {
            DocElement::Heading { level, text } => {
                let style = match level {
                    1 => "Heading1",
                    2 => "Heading2",
                    _ => "Heading3",
                };
                push_docx_paragraph(&mut out, Some(style), text);
            }
            DocElement::Paragraph(text) => push_docx_paragraph(&mut out, None, text),
            DocElement::Bullet(text) => {
                push_docx_paragraph(&mut out, Some("ListParagraph"), &format!("• {text}"))
            }
            DocElement::Table { headers, rows } => push_docx_table(&mut out, headers, rows),
            DocElement::Spacer => push_docx_paragraph(&mut out, None, ""),
        }
    }
    if !doc.citations.is_empty() {
        push_docx_paragraph(&mut out, Some("Heading2"), "Sources");
        for citation in &doc.citations {
            let mut line = format!("[{}] {}", citation.id, citation.title);
            if let Some(url) = &citation.url {
                line.push_str(" — ");
                line.push_str(url);
            }
            push_docx_paragraph(&mut out, None, &line);
        }
    }
    out.push_str(r#"<w:sectPr><w:pgSz w:w="11906" w:h="16838"/><w:pgMar w:top="1440" w:right="1440" w:bottom="1440" w:left="1440"/></w:sectPr></w:body></w:document>"#);
    out
}

fn push_docx_paragraph(out: &mut String, style: Option<&str>, text: &str) {
    out.push_str("<w:p>");
    if let Some(style) = style {
        out.push_str("<w:pPr><w:pStyle w:val=\"");
        out.push_str(style);
        out.push_str("\"/></w:pPr>");
    }
    out.push_str("<w:r><w:t xml:space=\"preserve\">");
    out.push_str(&escape_xml(text));
    out.push_str("</w:t></w:r></w:p>");
}

fn push_docx_table(out: &mut String, headers: &[String], rows: &[Vec<String>]) {
    out.push_str("<w:tbl><w:tblPr><w:tblW w:w=\"0\" w:type=\"auto\"/><w:tblBorders><w:top w:val=\"single\" w:sz=\"4\"/><w:left w:val=\"single\" w:sz=\"4\"/><w:bottom w:val=\"single\" w:sz=\"4\"/><w:right w:val=\"single\" w:sz=\"4\"/><w:insideH w:val=\"single\" w:sz=\"4\"/><w:insideV w:val=\"single\" w:sz=\"4\"/></w:tblBorders></w:tblPr>");
    push_docx_row(out, headers, true);
    for row in rows {
        push_docx_row(out, row, false);
    }
    out.push_str("</w:tbl>");
}

fn push_docx_row(out: &mut String, cells: &[String], bold: bool) {
    out.push_str("<w:tr>");
    for cell in cells {
        out.push_str("<w:tc><w:p><w:r>");
        if bold {
            out.push_str("<w:rPr><w:b/></w:rPr>");
        }
        out.push_str("<w:t xml:space=\"preserve\">");
        out.push_str(&escape_xml(cell));
        out.push_str("</w:t></w:r></w:p></w:tc>");
    }
    out.push_str("</w:tr>");
}

fn escape_xml(input: &str) -> String {
    escape_html(input)
}

const CONTENT_TYPES_XML: &str = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types">
<Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/>
<Default Extension="xml" ContentType="application/xml"/>
<Override PartName="/word/document.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml"/>
<Override PartName="/word/styles.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.styles+xml"/>
</Types>"#;

const RELS_XML: &str = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">
<Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="word/document.xml"/>
</Relationships>"#;

const STYLES_XML: &str = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<w:styles xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main">
<w:style w:type="paragraph" w:default="1" w:styleId="Normal"><w:name w:val="Normal"/><w:qFormat/></w:style>
<w:style w:type="paragraph" w:styleId="Title"><w:name w:val="Title"/><w:basedOn w:val="Normal"/><w:qFormat/><w:rPr><w:b/><w:sz w:val="40"/></w:rPr></w:style>
<w:style w:type="paragraph" w:styleId="Subtitle"><w:name w:val="Subtitle"/><w:basedOn w:val="Normal"/><w:qFormat/><w:rPr><w:color w:val="5B677A"/><w:sz w:val="24"/></w:rPr></w:style>
<w:style w:type="paragraph" w:styleId="Heading1"><w:name w:val="heading 1"/><w:basedOn w:val="Normal"/><w:qFormat/><w:rPr><w:b/><w:sz w:val="30"/></w:rPr></w:style>
<w:style w:type="paragraph" w:styleId="Heading2"><w:name w:val="heading 2"/><w:basedOn w:val="Normal"/><w:qFormat/><w:rPr><w:b/><w:sz w:val="26"/></w:rPr></w:style>
<w:style w:type="paragraph" w:styleId="Heading3"><w:name w:val="heading 3"/><w:basedOn w:val="Normal"/><w:qFormat/><w:rPr><w:b/><w:sz w:val="22"/></w:rPr></w:style>
<w:style w:type="paragraph" w:styleId="ListParagraph"><w:name w:val="List Paragraph"/><w:basedOn w:val="Normal"/><w:qFormat/><w:pPr><w:ind w:left="720"/></w:pPr></w:style>
</w:styles>"#;

#[derive(Default)]
struct PdfLayout {
    pages: Vec<Vec<PdfOp>>,
    y: f32,
}

#[derive(Clone)]
enum PdfOp {
    Text {
        x: f32,
        y: f32,
        size: f32,
        bold: bool,
        text: String,
    },
}

const PAGE_W: f32 = 595.0;
const PAGE_H: f32 = 842.0;
const MARGIN: f32 = 54.0;
const BOTTOM: f32 = 58.0;

fn render_pdf(doc: &Document) -> RenderedDocument {
    let mut layout = PdfLayout::default();
    layout.new_page();
    layout.add_text_block(&doc.title, 24.0, true, MARGIN, 1.25);
    if let Some(subtitle) = &doc.subtitle {
        layout.add_text_block(subtitle, 13.0, false, MARGIN, 1.35);
    }
    if let Some(author) = &doc.author {
        layout.add_text_block(author, 10.0, false, MARGIN, 1.35);
    }
    layout.add_gap(12.0);
    for element in &doc.elements {
        match element {
            DocElement::Heading { level, text } => {
                let size = match level {
                    1 => 18.0,
                    2 => 15.0,
                    _ => 13.0,
                };
                layout.add_gap(8.0);
                layout.add_text_block(text, size, true, MARGIN, 1.3);
            }
            DocElement::Paragraph(text) => layout.add_text_block(text, 11.0, false, MARGIN, 1.35),
            DocElement::Bullet(text) => {
                layout.add_text_block(&format!("- {text}"), 10.5, false, MARGIN + 12.0, 1.35)
            }
            DocElement::Table { headers, rows } => layout.add_pdf_table(headers, rows),
            DocElement::Spacer => layout.add_gap(6.0),
        }
    }
    if !doc.citations.is_empty() {
        layout.add_gap(12.0);
        layout.add_text_block("Sources", 14.0, true, MARGIN, 1.3);
        for citation in &doc.citations {
            let mut line = format!("[{}] {}", citation.id, citation.title);
            if let Some(url) = &citation.url {
                line.push_str(" - ");
                line.push_str(url);
            }
            layout.add_text_block(&line, 9.0, false, MARGIN, 1.3);
        }
    }

    let pages = layout.pages.len();
    RenderedDocument {
        bytes: build_pdf_bytes(layout.pages),
        pages: Some(pages),
    }
}

impl PdfLayout {
    fn new_page(&mut self) {
        self.pages.push(Vec::new());
        self.y = PAGE_H - MARGIN;
    }

    fn ensure_space(&mut self, needed: f32) {
        if self.y - needed < BOTTOM {
            self.new_page();
        }
    }

    fn current_page(&mut self) -> &mut Vec<PdfOp> {
        self.pages
            .last_mut()
            .expect("PdfLayout must always have at least one page")
    }

    fn add_gap(&mut self, amount: f32) {
        self.ensure_space(amount);
        self.y -= amount;
    }

    fn add_text_block(&mut self, text: &str, size: f32, bold: bool, x: f32, line_factor: f32) {
        let max_width = PAGE_W - x - MARGIN;
        let lines = wrap_text(text, size, max_width);
        let line_height = size * line_factor;
        let needed = line_height * lines.len().max(1) as f32 + 4.0;
        self.ensure_space(needed);
        for line in lines {
            let y = self.y;
            self.current_page().push(PdfOp::Text {
                x,
                y,
                size,
                bold,
                text: line,
            });
            self.y -= line_height;
        }
        self.y -= 4.0;
    }

    fn add_pdf_table(&mut self, headers: &[String], rows: &[Vec<String>]) {
        let cols = headers
            .len()
            .max(rows.iter().map(Vec::len).max().unwrap_or(0))
            .max(1);
        let col_width = (PAGE_W - MARGIN * 2.0) / cols as f32;
        let header = headers
            .iter()
            .map(|h| truncate_cell(h, 28))
            .collect::<Vec<_>>()
            .join(" | ");
        self.add_text_block(&header, 9.5, true, MARGIN, 1.3);
        for row in rows {
            let mut rendered = Vec::new();
            for idx in 0..cols {
                rendered.push(truncate_cell(
                    row.get(idx).map(String::as_str).unwrap_or(""),
                    28,
                ));
            }
            let line = rendered.join(" | ");
            self.add_text_block(&line, 9.0, false, MARGIN, 1.25);
        }
        let _ = col_width;
        self.add_gap(4.0);
    }
}

fn truncate_cell(value: &str, max_chars: usize) -> String {
    let count = value.chars().count();
    if count <= max_chars {
        value.to_string()
    } else {
        let mut out: String = value.chars().take(max_chars.saturating_sub(1)).collect();
        out.push('…');
        out
    }
}

fn wrap_text(text: &str, size: f32, max_width: f32) -> Vec<String> {
    let max_chars = (max_width / (size * 0.52)).floor().max(12.0) as usize;
    let mut lines = Vec::new();
    for hard_line in text.lines() {
        let mut current = String::new();
        for word in hard_line.split_whitespace() {
            if current.is_empty() {
                current.push_str(word);
            } else if current.chars().count() + 1 + word.chars().count() <= max_chars {
                current.push(' ');
                current.push_str(word);
            } else {
                lines.push(current);
                current = split_long_word(word, max_chars, &mut lines);
            }
        }
        if !current.is_empty() {
            lines.push(current);
        }
    }
    if lines.is_empty() {
        lines.push(String::new());
    }
    lines
}

fn split_long_word(word: &str, max_chars: usize, lines: &mut Vec<String>) -> String {
    if word.chars().count() <= max_chars {
        return word.to_string();
    }
    let mut chars = word.chars().peekable();
    while chars.clone().count() > max_chars {
        let chunk: String = chars.by_ref().take(max_chars.saturating_sub(1)).collect();
        lines.push(format!("{chunk}-"));
    }
    chars.collect()
}

fn build_pdf_bytes(pages: Vec<Vec<PdfOp>>) -> Vec<u8> {
    let page_count = pages.len();
    let mut objects: Vec<Vec<u8>> = Vec::new();
    objects.push(b"<< /Type /Catalog /Pages 2 0 R >>".to_vec());
    let first_page_obj = 5;
    let kids = (0..page_count)
        .map(|idx| format!("{} 0 R", first_page_obj + idx * 2))
        .collect::<Vec<_>>()
        .join(" ");
    objects.push(format!("<< /Type /Pages /Kids [{kids}] /Count {page_count} >>").into_bytes());
    objects.push(
        b"<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica /Encoding /WinAnsiEncoding >>"
            .to_vec(),
    );
    objects.push(
        b"<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica-Bold /Encoding /WinAnsiEncoding >>"
            .to_vec(),
    );
    for (idx, ops) in pages.iter().enumerate() {
        let content_obj = first_page_obj + idx * 2 + 1;
        objects.push(
            format!("<< /Type /Page /Parent 2 0 R /MediaBox [0 0 {PAGE_W} {PAGE_H}] /Resources << /Font << /F1 3 0 R /F2 4 0 R >> >> /Contents {content_obj} 0 R >>")
                .into_bytes(),
        );
        let stream = render_pdf_stream(ops);
        objects.push(
            format!(
                "<< /Length {} >>\nstream\n{}endstream",
                stream.len(),
                String::from_utf8_lossy(&stream)
            )
            .into_bytes(),
        );
    }

    let mut out = Vec::new();
    out.extend_from_slice(b"%PDF-1.4\n%\xE2\xE3\xCF\xD3\n");
    let mut offsets = Vec::with_capacity(objects.len() + 1);
    offsets.push(0usize);
    for (idx, body) in objects.iter().enumerate() {
        offsets.push(out.len());
        out.extend_from_slice(format!("{} 0 obj\n", idx + 1).as_bytes());
        out.extend_from_slice(body);
        out.extend_from_slice(b"\nendobj\n");
    }
    let xref = out.len();
    out.extend_from_slice(format!("xref\n0 {}\n", objects.len() + 1).as_bytes());
    out.extend_from_slice(b"0000000000 65535 f \n");
    for offset in offsets.iter().skip(1) {
        out.extend_from_slice(format!("{offset:010} 00000 n \n").as_bytes());
    }
    out.extend_from_slice(
        format!(
            "trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n",
            objects.len() + 1
        )
        .as_bytes(),
    );
    out
}

fn render_pdf_stream(ops: &[PdfOp]) -> Vec<u8> {
    let mut out = String::new();
    for op in ops {
        match op {
            PdfOp::Text {
                x,
                y,
                size,
                bold,
                text,
            } => {
                let font = if *bold { "F2" } else { "F1" };
                out.push_str(&format!(
                    "BT /{font} {size:.1} Tf 1 0 0 1 {x:.1} {y:.1} Tm {} Tj ET\n",
                    pdf_literal(text)
                ));
            }
        }
    }
    out.into_bytes()
}

fn pdf_literal(text: &str) -> String {
    let mut out = String::from("(");
    for byte in win_ansi_bytes(text) {
        match byte {
            b'(' | b')' | b'\\' => {
                out.push('\\');
                out.push(byte as char);
            }
            b'\n' => out.push_str("\\n"),
            b'\r' => out.push_str("\\r"),
            b'\t' => out.push_str("\\t"),
            0..=31 | 127..=255 => out.push_str(&format!("\\{byte:03o}")),
            _ => out.push(byte as char),
        }
    }
    out.push(')');
    out
}

fn win_ansi_bytes(text: &str) -> Vec<u8> {
    let mut out = Vec::new();
    for ch in text.chars() {
        match ch {
            '\u{2018}' | '\u{2019}' => out.push(b'\''),
            '\u{201C}' | '\u{201D}' => out.push(b'"'),
            '\u{2013}' | '\u{2014}' => out.push(b'-'),
            '\u{2022}' => out.push(b'-'),
            '\u{2026}' => out.extend_from_slice(b"..."),
            '\u{00A0}' => out.push(b' '),
            '€' => out.extend_from_slice(b"EUR"),
            ch if (ch as u32) <= 255 => out.push(ch as u8),
            _ => out.push(b'?'),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn document_create_pdf_writes_nonempty_file() {
        let dir = tempfile::tempdir().unwrap();
        let input = serde_json::json!({
            "format": "pdf",
            "path": "reports/test.pdf",
            "title": "Rapport NFC",
            "content": "# Synthèse\nLe service va bien.\n\n| Point | Statut |\n| --- | --- |\n| RAM | OK |"
        });
        let raw = create_document(&input, Some(dir.path())).await.unwrap();
        let value: serde_json::Value = serde_json::from_str(&raw).unwrap();
        let path = PathBuf::from(value["path"].as_str().unwrap());
        let bytes = std::fs::read(&path).unwrap();
        assert!(bytes.starts_with(b"%PDF-1.4"));
        assert!(bytes.len() > 500);
        assert_eq!(value["format"], "pdf");
        assert_eq!(value["pages"], 1);
    }

    #[tokio::test]
    async fn document_create_docx_writes_openxml_package() {
        let dir = tempfile::tempdir().unwrap();
        let input = serde_json::json!({
            "format": "docx",
            "path": "rapport.docx",
            "title": "Synthèse",
            "sections": [{
                "heading": "Décision",
                "body": "Captain crée le document nativement.",
                "bullets": ["PDF", "DOCX"]
            }]
        });
        let raw = create_document(&input, Some(dir.path())).await.unwrap();
        let value: serde_json::Value = serde_json::from_str(&raw).unwrap();
        let file = std::fs::File::open(value["path"].as_str().unwrap()).unwrap();
        let mut archive = zip::ZipArchive::new(file).unwrap();
        let mut document_xml = String::new();
        std::io::Read::read_to_string(
            &mut archive.by_name("word/document.xml").unwrap(),
            &mut document_xml,
        )
        .unwrap();
        assert!(document_xml.contains("Captain crée le document nativement."));
        assert_eq!(value["format"], "docx");
    }

    #[tokio::test]
    async fn document_create_refuses_overwrite_by_default() {
        let dir = tempfile::tempdir().unwrap();
        let input = serde_json::json!({
            "format": "markdown",
            "path": "same.md",
            "title": "A",
            "content": "First"
        });
        create_document(&input, Some(dir.path())).await.unwrap();
        let err = create_document(&input, Some(dir.path())).await.unwrap_err();
        assert!(err.contains("Refusing to overwrite"));
    }

    #[tokio::test]
    async fn document_create_markdown_appends_citations() {
        let dir = tempfile::tempdir().unwrap();
        let input = serde_json::json!({
            "format": "markdown",
            "path": "sources.md",
            "title": "Research",
            "content": "Résumé.",
            "citations": [{"id": "a", "title": "Official docs", "url": "https://example.com"}]
        });
        let raw = create_document(&input, Some(dir.path())).await.unwrap();
        let value: serde_json::Value = serde_json::from_str(&raw).unwrap();
        let body = std::fs::read_to_string(value["path"].as_str().unwrap()).unwrap();
        assert!(body.contains("## Sources"));
        assert!(body.contains("https://example.com"));
    }

    #[tokio::test]
    async fn document_create_dedupes_leading_h1_equal_to_title() {
        let dir = tempfile::tempdir().unwrap();
        let input = serde_json::json!({
            "format": "markdown",
            "path": "dedupe.md",
            "title": "Captain API Real Test",
            "content": "# Captain API Real Test\n\nRésumé."
        });
        let raw = create_document(&input, Some(dir.path())).await.unwrap();
        let value: serde_json::Value = serde_json::from_str(&raw).unwrap();
        let body = std::fs::read_to_string(value["path"].as_str().unwrap()).unwrap();
        assert_eq!(body.matches("# Captain API Real Test").count(), 1);
        assert!(body.contains("Résumé."));
    }
}
