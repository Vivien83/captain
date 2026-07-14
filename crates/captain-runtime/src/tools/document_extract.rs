//! Document text extraction handler.

use crate::kernel_handle::KernelHandle;
use crate::tools::{hex_nibble, resolve_file_path_for_caller};
use flate2::read::ZlibDecoder;
use std::io::Read;
use std::path::Path;
use std::sync::Arc;

const DEFAULT_DOCUMENT_EXTRACT_MAX_CHARS: usize = 50_000;
const HARD_DOCUMENT_EXTRACT_MAX_CHARS: usize = 200_000;

pub(crate) async fn tool_document_extract(
    input: &serde_json::Value,
    workspace_root: Option<&Path>,
    kernel: Option<&Arc<dyn KernelHandle>>,
    caller_agent_id: Option<&str>,
) -> Result<String, String> {
    let raw_path = input["path"].as_str().ok_or("Missing 'path' parameter")?;
    let max_chars = input["max_chars"]
        .as_u64()
        .map(|v| v as usize)
        .unwrap_or(DEFAULT_DOCUMENT_EXTRACT_MAX_CHARS)
        .clamp(1_000, HARD_DOCUMENT_EXTRACT_MAX_CHARS);
    let resolved = resolve_file_path_for_caller(raw_path, workspace_root, kernel, caller_agent_id)?;
    let bytes = tokio::fs::read(&resolved)
        .await
        .map_err(|e| format!("Failed to read document: {e}"))?;

    let ext = resolved
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase();
    let is_pdf = ext == "pdf" || bytes.starts_with(b"%PDF");
    let (format, content, extraction_meta) = if is_pdf {
        let extracted = extract_pdf_text_from_bytes(&bytes)?;
        if extracted.text.trim().is_empty() {
            return Err("No extractable text found in this PDF. It is likely scanned/image-only; use OCR/vision or another source, and do not cite it for unsupported claims.".to_string());
        }
        (
            "pdf".to_string(),
            extracted.text,
            serde_json::json!({
                "streams_seen": extracted.streams_seen,
                "streams_decoded": extracted.streams_decoded,
                "method": "pdf_literal_text_streams"
            }),
        )
    } else if is_text_like_extension(&ext) || looks_like_utf8_text(&bytes) {
        (
            if ext.is_empty() {
                "text".to_string()
            } else {
                ext.clone()
            },
            String::from_utf8_lossy(&bytes).into_owned(),
            serde_json::json!({ "method": "utf8_lossy" }),
        )
    } else {
        return Err(format!(
            "Unsupported document type '{}'. Use a media-specific tool or OCR/vision for binary files.",
            if ext.is_empty() { "unknown" } else { &ext }
        ));
    };

    let content_chars = content.chars().count();
    let truncated = content_chars > max_chars;
    let output = if truncated {
        truncate_chars(&content, max_chars)
    } else {
        content
    };

    serde_json::to_string_pretty(&serde_json::json!({
        "success": true,
        "tool": "document_extract",
        "path": raw_path,
        "format": format,
        "size_bytes": bytes.len(),
        "chars": content_chars,
        "truncated": truncated,
        "extraction": extraction_meta,
        "content": output,
        "note": "Use this extracted text as evidence only for this source. Cite the original URL or local path in the final Sources section.",
    }))
    .map_err(|e| format!("Serialize error: {e}"))
}

fn is_text_like_extension(ext: &str) -> bool {
    matches!(
        ext,
        "txt"
            | "md"
            | "markdown"
            | "html"
            | "htm"
            | "csv"
            | "json"
            | "xml"
            | "yaml"
            | "yml"
            | "toml"
            | "log"
            | "rs"
            | "py"
            | "js"
            | "ts"
            | "tsx"
            | "jsx"
            | "css"
            | "scss"
            | "sh"
    )
}

fn looks_like_utf8_text(bytes: &[u8]) -> bool {
    if bytes.is_empty() {
        return true;
    }
    if std::str::from_utf8(bytes).is_err() {
        return false;
    }
    let sample = &bytes[..bytes.len().min(4096)];
    let control = sample
        .iter()
        .filter(|b| **b < 0x20 && !matches!(**b, b'\n' | b'\r' | b'\t'))
        .count();
    control * 20 < sample.len()
}

#[derive(Debug)]
pub(crate) struct PdfExtractedText {
    pub(crate) text: String,
    pub(crate) streams_seen: usize,
    pub(crate) streams_decoded: usize,
}

pub(crate) fn extract_pdf_text_from_bytes(bytes: &[u8]) -> Result<PdfExtractedText, String> {
    if !bytes.starts_with(b"%PDF") {
        return Err("Invalid PDF: missing %PDF header".to_string());
    }

    let mut pieces = Vec::new();
    let mut streams_seen = 0usize;
    let mut streams_decoded = 0usize;
    let mut pos = 0usize;
    while let Some(stream_rel) = find_subslice(&bytes[pos..], b"stream") {
        let stream_idx = pos + stream_rel;
        let mut data_start = stream_idx + b"stream".len();
        if bytes.get(data_start..data_start + 2) == Some(b"\r\n") {
            data_start += 2;
        } else if matches!(bytes.get(data_start), Some(b'\n' | b'\r')) {
            data_start += 1;
        }
        let Some(end_rel) = find_subslice(&bytes[data_start..], b"endstream") else {
            break;
        };
        let data_end = data_start + end_rel;
        let dict_start = stream_idx.saturating_sub(2048);
        let dict = &bytes[dict_start..stream_idx];
        let stream = &bytes[data_start..data_end];
        streams_seen += 1;
        let decoded = if dict
            .windows(b"FlateDecode".len())
            .any(|w| w == b"FlateDecode")
        {
            let mut decoder = ZlibDecoder::new(stream);
            let mut out = Vec::new();
            match decoder.read_to_end(&mut out) {
                Ok(_) => {
                    streams_decoded += 1;
                    out
                }
                Err(_) => stream.to_vec(),
            }
        } else {
            stream.to_vec()
        };
        pieces.extend(extract_pdf_strings_from_stream(&decoded));
        pos = data_end + b"endstream".len();
    }

    if pieces.is_empty() {
        pieces.extend(extract_pdf_strings_from_stream(bytes));
    }

    let text = normalize_extracted_text(&pieces.join(" "));
    Ok(PdfExtractedText {
        text,
        streams_seen,
        streams_decoded,
    })
}

fn extract_pdf_strings_from_stream(bytes: &[u8]) -> Vec<String> {
    let text = String::from_utf8_lossy(bytes);
    let mut pieces = Vec::new();
    let mut start = 0usize;
    let mut any_text_block = false;
    while let Some(bt_rel) = text[start..].find("BT") {
        let block_start = start + bt_rel + 2;
        let Some(et_rel) = text[block_start..].find("ET") else {
            break;
        };
        let block = &text[block_start..block_start + et_rel];
        pieces.extend(extract_pdf_literal_strings(block));
        pieces.extend(extract_pdf_hex_strings(block));
        any_text_block = true;
        start = block_start + et_rel + 2;
    }
    if !any_text_block {
        pieces.extend(extract_pdf_literal_strings(&text));
        pieces.extend(extract_pdf_hex_strings(&text));
    }
    pieces
        .into_iter()
        .map(|s| normalize_extracted_text(&s))
        .filter(|s| is_useful_pdf_string(s))
        .collect()
}

fn extract_pdf_literal_strings(text: &str) -> Vec<String> {
    let bytes = text.as_bytes();
    let mut out = Vec::new();
    let mut i = 0usize;
    while i < bytes.len() {
        if bytes[i] != b'(' {
            i += 1;
            continue;
        }
        i += 1;
        let mut depth = 1i32;
        let mut buf = Vec::new();
        while i < bytes.len() && depth > 0 {
            let b = bytes[i];
            if b == b'\\' {
                i += 1;
                if i >= bytes.len() {
                    break;
                }
                let esc = bytes[i];
                match esc {
                    b'n' => buf.push(b'\n'),
                    b'r' => buf.push(b'\r'),
                    b't' => buf.push(b'\t'),
                    b'b' => buf.push(0x08),
                    b'f' => buf.push(0x0c),
                    b'(' | b')' | b'\\' => buf.push(esc),
                    b'\n' => {}
                    b'\r' => {
                        if bytes.get(i + 1) == Some(&b'\n') {
                            i += 1;
                        }
                    }
                    b'0'..=b'7' => {
                        let mut value = esc - b'0';
                        let mut consumed = 1;
                        while consumed < 3 {
                            let Some(next) = bytes.get(i + 1) else {
                                break;
                            };
                            if !(b'0'..=b'7').contains(next) {
                                break;
                            }
                            value = value.saturating_mul(8).saturating_add(next - b'0');
                            i += 1;
                            consumed += 1;
                        }
                        buf.push(value);
                    }
                    other => buf.push(other),
                }
            } else if b == b'(' {
                depth += 1;
                buf.push(b);
            } else if b == b')' {
                depth -= 1;
                if depth > 0 {
                    buf.push(b);
                }
            } else {
                buf.push(b);
            }
            i += 1;
        }
        let s = String::from_utf8_lossy(&buf).into_owned();
        if is_useful_pdf_string(&s) {
            out.push(s);
        }
    }
    out
}

fn extract_pdf_hex_strings(text: &str) -> Vec<String> {
    let bytes = text.as_bytes();
    let mut out = Vec::new();
    let mut i = 0usize;
    while i < bytes.len() {
        if bytes[i] != b'<' || bytes.get(i + 1) == Some(&b'<') {
            i += 1;
            continue;
        }
        i += 1;
        let start = i;
        while i < bytes.len() && bytes[i] != b'>' {
            i += 1;
        }
        if i >= bytes.len() {
            break;
        }
        let hex = text[start..i]
            .chars()
            .filter(|c| !c.is_whitespace())
            .collect::<String>();
        if hex.len() >= 4 && hex.len() % 2 == 0 && hex.chars().all(|c| c.is_ascii_hexdigit()) {
            let mut data = Vec::with_capacity(hex.len() / 2);
            let mut ok = true;
            for chunk in hex.as_bytes().chunks(2) {
                let (Some(hi), Some(lo)) = (hex_nibble(chunk[0]), hex_nibble(chunk[1])) else {
                    ok = false;
                    break;
                };
                data.push((hi << 4) | lo);
            }
            if ok {
                let decoded = if data.starts_with(&[0xfe, 0xff]) {
                    decode_utf16be_lossy(&data[2..])
                } else {
                    String::from_utf8_lossy(&data).into_owned()
                };
                if is_useful_pdf_string(&decoded) {
                    out.push(decoded);
                }
            }
        }
        i += 1;
    }
    out
}

fn decode_utf16be_lossy(bytes: &[u8]) -> String {
    let units = bytes
        .chunks_exact(2)
        .map(|c| u16::from_be_bytes([c[0], c[1]]))
        .collect::<Vec<_>>();
    String::from_utf16_lossy(&units)
}

fn is_useful_pdf_string(text: &str) -> bool {
    let trimmed = text.trim();
    if trimmed.len() < 2 {
        return false;
    }
    if !trimmed.chars().any(|c| c.is_alphanumeric()) {
        return false;
    }
    let chars = trimmed.chars().count().max(1);
    let printable = trimmed
        .chars()
        .filter(|c| !c.is_control() || matches!(c, '\n' | '\r' | '\t'))
        .count();
    printable * 100 / chars >= 70
}

fn normalize_extracted_text(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn truncate_chars(text: &str, max_chars: usize) -> String {
    let mut out = String::new();
    for ch in text.chars().take(max_chars) {
        out.push(ch);
    }
    out.push_str("… [truncated]");
    out
}

fn find_subslice(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    if needle.is_empty() {
        return Some(0);
    }
    haystack.windows(needle.len()).position(|w| w == needle)
}
