use std::path::{Path, PathBuf};

pub(crate) const SUPPORTED_UPLOAD_FORMATS: &str =
    "png/jpeg/gif/webp, pdf, txt/md/json/csv/html, mp3/wav/ogg/m4a";
pub(crate) const UPLOAD_USAGE: &str = "Usage: /image <path-to-image>";

const UPLOAD_CONTENT_TYPES: &[(&str, &str)] = &[
    ("png", "image/png"),
    ("jpg", "image/jpeg"),
    ("jpeg", "image/jpeg"),
    ("gif", "image/gif"),
    ("webp", "image/webp"),
    ("pdf", "application/pdf"),
    ("txt", "text/plain"),
    ("md", "text/markdown"),
    ("json", "application/json"),
    ("csv", "text/csv"),
    ("html", "text/html"),
    ("htm", "text/html"),
    ("mp3", "audio/mpeg"),
    ("wav", "audio/wav"),
    ("ogg", "audio/ogg"),
    ("m4a", "audio/mp4"),
];

pub(crate) fn upload_content_type_for_path(path: &Path) -> Option<&'static str> {
    let ext = path.extension().and_then(|e| e.to_str())?;
    upload_content_type_for_extension(ext)
}

#[derive(Debug)]
pub(crate) struct PreparedUpload {
    pub(crate) path: PathBuf,
    pub(crate) filename: String,
    pub(crate) content_type: &'static str,
    pub(crate) bytes: Vec<u8>,
}

pub(crate) fn prepare_upload(raw_path: &str) -> Result<PreparedUpload, String> {
    if raw_path.is_empty() {
        return Err(UPLOAD_USAGE.to_string());
    }

    let path = expand_upload_path(raw_path);
    let bytes = std::fs::read(&path)
        .map_err(|err| format!("Lecture impossible ({}): {err}", path.display()))?;
    let filename = upload_filename(&path);
    let content_type = upload_content_type_for_path(&path).ok_or_else(|| {
        let ext = path
            .extension()
            .and_then(|e| e.to_str())
            .map(|s| s.to_ascii_lowercase())
            .unwrap_or_default();
        format!("Format non supporté ({ext}). Acceptés: {SUPPORTED_UPLOAD_FORMATS}.")
    })?;

    Ok(PreparedUpload {
        path,
        filename,
        content_type,
        bytes,
    })
}

fn expand_upload_path(raw_path: &str) -> PathBuf {
    if let Some(rest) = raw_path.strip_prefix("~/") {
        dirs::home_dir()
            .map(|home| home.join(rest))
            .unwrap_or_else(|| PathBuf::from(raw_path))
    } else {
        PathBuf::from(raw_path)
    }
}

fn upload_filename(path: &Path) -> String {
    path.file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("image")
        .to_string()
}

pub(crate) fn upload_content_type_for_extension(ext: &str) -> Option<&'static str> {
    let normalized = ext.to_ascii_lowercase();
    UPLOAD_CONTENT_TYPES
        .iter()
        .find_map(|(candidate, content_type)| (*candidate == normalized).then_some(*content_type))
}

/// Detect when a clipboard paste is actually a file dropped onto the terminal.
///
/// Validation stays deliberately strict: only one-line paths to existing files
/// with an uploadable extension are routed to the upload pipeline.
pub(crate) fn parse_dropped_path(raw: &str) -> Option<PathBuf> {
    let trimmed = raw.trim();
    if trimmed.is_empty() || trimmed.contains('\n') {
        return None;
    }

    let unquoted = strip_matching_quotes(trimmed);
    let path_str = if let Some(rest) = unquoted.strip_prefix("file://") {
        rest.to_string()
    } else if let Some(rest) = unquoted.strip_prefix("~/") {
        dirs::home_dir()?.join(rest).to_string_lossy().into_owned()
    } else if unquoted.starts_with('/') {
        unquoted.replace("\\ ", " ")
    } else {
        return None;
    };

    let path = PathBuf::from(path_str);
    if path.is_file() && upload_content_type_for_path(&path).is_some() {
        Some(path)
    } else {
        None
    }
}

fn strip_matching_quotes(s: &str) -> &str {
    if s.len() >= 2 {
        let first = s.chars().next().unwrap();
        let last = s.chars().last().unwrap();
        if (first == '"' && last == '"') || (first == '\'' && last == '\'') {
            return &s[1..s.len() - 1];
        }
    }
    s
}

#[cfg(test)]
#[path = "file_upload/tests.rs"]
mod tests;
