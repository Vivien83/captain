//! Bounded, owner-only output evidence for observable tool runs.
//!
//! The LLM receives a compact preview. When a streamed or final result exceeds
//! that preview budget, this store keeps a redacted copy on disk so Captain can
//! inspect the omitted lines without re-running the tool. Raw capture files are
//! never exposed and are deleted on the next boot after an interrupted write.

use regex::Regex;
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
use std::path::{Component, Path, PathBuf};
use std::sync::{LazyLock, Mutex};
use std::time::{Duration, SystemTime};

pub const DEFAULT_PER_RUN_CAP_BYTES: u64 = 5_000_000;
pub const DEFAULT_TOTAL_CAP_BYTES: u64 = 128 * 1024 * 1024;
pub const DEFAULT_TTL: Duration = Duration::from_secs(7 * 24 * 60 * 60);
const CAP_MARKER: &[u8] = b"\n[tool run output capped by Captain]\n";

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct ToolRunOutputMetadata {
    pub file_name: String,
    pub stored_bytes: u64,
    pub total_bytes: u64,
    pub sha256: String,
    pub capped: bool,
    pub redacted: bool,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct ToolRunOutputPage {
    pub start_line: usize,
    pub end_line: usize,
    pub total_lines: usize,
    pub content: String,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct ToolRunOutputMatch {
    pub line: usize,
    pub content: String,
}

#[derive(Debug)]
pub struct ToolRunOutputCapture {
    part_path: PathBuf,
    file: Option<File>,
    stored_bytes: u64,
    total_bytes: u64,
    capped: bool,
}

impl ToolRunOutputCapture {
    pub fn append(&mut self, bytes: &[u8], cap: u64) -> std::io::Result<()> {
        self.total_bytes = self.total_bytes.saturating_add(bytes.len() as u64);
        if self.capped {
            return Ok(());
        }

        let marker_len = CAP_MARKER.len() as u64;
        let content_cap = cap.saturating_sub(marker_len);
        let remaining = content_cap.saturating_sub(self.stored_bytes) as usize;
        let accepted = remaining.min(bytes.len());
        if accepted > 0 {
            self.file_mut()?.write_all(&bytes[..accepted])?;
            self.stored_bytes = self.stored_bytes.saturating_add(accepted as u64);
        }
        if accepted < bytes.len() {
            self.file_mut()?.write_all(CAP_MARKER)?;
            self.stored_bytes = self.stored_bytes.saturating_add(marker_len);
            self.capped = true;
        }
        Ok(())
    }

    fn file_mut(&mut self) -> std::io::Result<&mut File> {
        self.file
            .as_mut()
            .ok_or_else(|| std::io::Error::other("tool-run output capture is closed"))
    }
}

#[derive(Debug)]
pub struct ToolRunOutputStore {
    root: PathBuf,
    per_run_cap_bytes: u64,
    total_cap_bytes: u64,
    ttl: Duration,
    cleanup_lock: Mutex<()>,
}

impl ToolRunOutputStore {
    pub fn new(root: PathBuf) -> std::io::Result<Self> {
        Self::with_limits(
            root,
            DEFAULT_PER_RUN_CAP_BYTES,
            DEFAULT_TOTAL_CAP_BYTES,
            DEFAULT_TTL,
        )
    }

    pub fn with_limits(
        root: PathBuf,
        per_run_cap_bytes: u64,
        total_cap_bytes: u64,
        ttl: Duration,
    ) -> std::io::Result<Self> {
        if per_run_cap_bytes < CAP_MARKER.len() as u64 + 1 {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "tool-run per-file cap is too small",
            ));
        }
        fs::create_dir_all(&root)?;
        set_private_dir_permissions(&root)?;
        let store = Self {
            root,
            per_run_cap_bytes,
            total_cap_bytes: total_cap_bytes.max(per_run_cap_bytes),
            ttl,
            cleanup_lock: Mutex::new(()),
        };
        store.cleanup()?;
        Ok(store)
    }

    pub fn begin_capture(&self, run_id: &str) -> std::io::Result<ToolRunOutputCapture> {
        validate_run_id(run_id)?;
        let part_path = self.root.join(format!("{run_id}.part"));
        let file = private_file(&part_path, true)?;
        Ok(ToolRunOutputCapture {
            part_path,
            file: Some(file),
            stored_bytes: 0,
            total_bytes: 0,
            capped: false,
        })
    }

    pub fn append_capture(
        &self,
        capture: &Mutex<ToolRunOutputCapture>,
        content: &str,
    ) -> std::io::Result<()> {
        let mut capture = capture
            .lock()
            .map_err(|_| std::io::Error::other("tool-run output capture poisoned"))?;
        capture.append(content.as_bytes(), self.per_run_cap_bytes)
    }

    pub fn capture_stats(
        &self,
        capture: &Mutex<ToolRunOutputCapture>,
    ) -> std::io::Result<(u64, u64, bool)> {
        let capture = capture
            .lock()
            .map_err(|_| std::io::Error::other("tool-run output capture poisoned"))?;
        Ok((capture.stored_bytes, capture.total_bytes, capture.capped))
    }

    pub fn discard_capture(&self, capture: &Mutex<ToolRunOutputCapture>) {
        let Ok(mut capture) = capture.lock() else {
            return;
        };
        let path = capture.part_path.clone();
        capture.file.take();
        let _ = fs::remove_file(path);
    }

    pub fn finalize_capture(
        &self,
        run_id: &str,
        capture: &Mutex<ToolRunOutputCapture>,
    ) -> std::io::Result<ToolRunOutputMetadata> {
        let mut capture = capture
            .lock()
            .map_err(|_| std::io::Error::other("tool-run output capture poisoned"))?;
        let mut file = capture
            .file
            .take()
            .ok_or_else(|| std::io::Error::other("tool-run output capture is closed"))?;
        file.flush()?;
        file.sync_all()?;
        drop(file);
        let result = fs::read(&capture.part_path).and_then(|raw| {
            let raw = String::from_utf8_lossy(&raw);
            let (sanitized, redacted) = sanitize_for_retention(&raw)?;
            self.write_final(
                run_id,
                sanitized.as_bytes(),
                capture.total_bytes,
                capture.capped,
                redacted,
            )
        });
        let _ = fs::remove_file(&capture.part_path);
        let metadata = result?;
        if let Err(error) = self.cleanup() {
            tracing::warn!(
                run_id,
                "Tool-run output was committed but retention cleanup failed: {error}"
            );
        }
        Ok(metadata)
    }

    pub fn persist_content(
        &self,
        run_id: &str,
        content: &str,
    ) -> std::io::Result<ToolRunOutputMetadata> {
        let mut capture = self.begin_capture(run_id)?;
        capture.append(content.as_bytes(), self.per_run_cap_bytes)?;
        self.finalize_capture(run_id, &Mutex::new(capture))
    }

    pub fn read_lines(
        &self,
        metadata: &ToolRunOutputMetadata,
        start_line: usize,
        max_lines: usize,
    ) -> std::io::Result<ToolRunOutputPage> {
        let content = self.read_verified(metadata)?;
        page_lines(&content, start_line, max_lines)
    }

    pub fn tail_lines(
        &self,
        metadata: &ToolRunOutputMetadata,
        max_lines: usize,
    ) -> std::io::Result<ToolRunOutputPage> {
        let content = self.read_verified(metadata)?;
        let total_lines = content.lines().count();
        let start_line = total_lines
            .saturating_sub(max_lines.max(1))
            .saturating_add(1);
        page_lines(&content, start_line, max_lines)
    }

    pub fn search_lines(
        &self,
        metadata: &ToolRunOutputMetadata,
        query: &str,
        max_matches: usize,
        case_sensitive: bool,
    ) -> std::io::Result<Vec<ToolRunOutputMatch>> {
        let query = query.trim();
        if query.is_empty() {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "tool-run output query cannot be empty",
            ));
        }
        let content = self.read_verified(metadata)?;
        let folded_query = (!case_sensitive).then(|| query.to_lowercase());
        Ok(content
            .lines()
            .enumerate()
            .filter(|(_, line)| {
                if let Some(folded_query) = folded_query.as_deref() {
                    line.to_lowercase().contains(folded_query)
                } else {
                    line.contains(query)
                }
            })
            .take(max_matches.clamp(1, 100))
            .map(|(index, line)| ToolRunOutputMatch {
                line: index + 1,
                content: line.to_string(),
            })
            .collect())
    }

    pub fn read_capture_lines(
        &self,
        capture: &Mutex<ToolRunOutputCapture>,
        start_line: usize,
        max_lines: usize,
    ) -> std::io::Result<ToolRunOutputPage> {
        let content = self.read_capture_sanitized(capture)?;
        page_lines(&content, start_line, max_lines)
    }

    pub fn tail_capture_lines(
        &self,
        capture: &Mutex<ToolRunOutputCapture>,
        max_lines: usize,
    ) -> std::io::Result<ToolRunOutputPage> {
        let content = self.read_capture_sanitized(capture)?;
        tail_content(&content, max_lines)
    }

    pub fn search_capture_lines(
        &self,
        capture: &Mutex<ToolRunOutputCapture>,
        query: &str,
        max_matches: usize,
        case_sensitive: bool,
    ) -> std::io::Result<Vec<ToolRunOutputMatch>> {
        let content = self.read_capture_sanitized(capture)?;
        search_content(&content, query, max_matches, case_sensitive)
    }

    pub fn metadata_exists(&self, metadata: &ToolRunOutputMetadata) -> bool {
        self.safe_path(&metadata.file_name)
            .is_ok_and(|path| path.is_file())
    }

    /// Recover evidence left between the durable file commit and the SQLite
    /// terminal-state update, or sanitize a partial capture from an abrupt
    /// process stop. The caller attaches the returned metadata to the
    /// interrupted ledger row before removing unrelated partial files.
    pub fn recover_interrupted_output(
        &self,
        run_id: &str,
    ) -> std::io::Result<Option<ToolRunOutputMetadata>> {
        validate_run_id(run_id)?;
        let final_path = self.root.join(format!("{run_id}.log"));
        if final_path.is_file() {
            let raw = self.read_path_bounded(&final_path)?;
            let raw = String::from_utf8_lossy(&raw);
            let (sanitized, changed) = sanitize_for_retention(&raw)?;
            let redacted = changed || contains_redaction_marker(&sanitized);
            let capped = sanitized.contains(String::from_utf8_lossy(CAP_MARKER).trim());
            return self
                .write_final(
                    run_id,
                    sanitized.as_bytes(),
                    sanitized.len() as u64,
                    capped,
                    redacted,
                )
                .map(Some);
        }

        let part_path = self.root.join(format!("{run_id}.part"));
        if !part_path.is_file() {
            return Ok(None);
        }
        let raw = self.read_path_bounded(&part_path)?;
        let raw_len = raw.len() as u64;
        let raw = String::from_utf8_lossy(&raw);
        let (sanitized, redacted) = sanitize_for_retention(&raw)?;
        let capped = sanitized.contains(String::from_utf8_lossy(CAP_MARKER).trim());
        let metadata = self.write_final(run_id, sanitized.as_bytes(), raw_len, capped, redacted)?;
        let _ = fs::remove_file(part_path);
        Ok(Some(metadata))
    }

    /// Remove incomplete files that could not be associated with an
    /// interrupted ledger row. Call only after recovery has been attempted.
    pub fn discard_orphaned_captures(&self) -> std::io::Result<usize> {
        let _guard = self
            .cleanup_lock
            .lock()
            .map_err(|_| std::io::Error::other("tool-run cleanup lock poisoned"))?;
        let mut removed = 0usize;
        for entry in fs::read_dir(&self.root)? {
            let entry = entry?;
            if !entry.file_type()?.is_file() {
                continue;
            }
            let name = entry.file_name();
            let name = name.to_string_lossy();
            if name.ends_with(".part") && fs::remove_file(entry.path()).is_ok() {
                removed += 1;
            }
        }
        Ok(removed)
    }

    fn write_final(
        &self,
        run_id: &str,
        content: &[u8],
        total_bytes: u64,
        capped: bool,
        redacted: bool,
    ) -> std::io::Result<ToolRunOutputMetadata> {
        validate_run_id(run_id)?;
        let file_name = format!("{run_id}.log");
        let final_path = self.root.join(&file_name);
        let temp_path = self.root.join(format!("{run_id}.final.tmp"));
        let _ = fs::remove_file(&temp_path);
        let mut file = private_file(&temp_path, true)?;
        file.write_all(content)?;
        file.flush()?;
        file.sync_all()?;
        drop(file);
        fs::rename(&temp_path, &final_path)?;
        sync_directory(&self.root)?;
        let sha256 = format!("{:x}", Sha256::digest(content));
        Ok(ToolRunOutputMetadata {
            file_name,
            stored_bytes: content.len() as u64,
            total_bytes,
            sha256,
            capped,
            redacted,
        })
    }

    fn read_verified(&self, metadata: &ToolRunOutputMetadata) -> std::io::Result<String> {
        let path = self.safe_path(&metadata.file_name)?;
        let bytes = self.read_path_bounded(&path)?;
        let digest = format!("{:x}", Sha256::digest(&bytes));
        if digest != metadata.sha256 {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "tool-run output checksum mismatch",
            ));
        }
        Ok(String::from_utf8_lossy(&bytes).into_owned())
    }

    fn read_path_bounded(&self, path: &Path) -> std::io::Result<Vec<u8>> {
        let file = File::open(path)?;
        let mut bytes = Vec::new();
        file.take(self.per_run_cap_bytes.saturating_add(1))
            .read_to_end(&mut bytes)?;
        if bytes.len() as u64 > self.per_run_cap_bytes {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "tool-run output exceeds configured cap",
            ));
        }
        Ok(bytes)
    }

    fn read_capture_sanitized(
        &self,
        capture: &Mutex<ToolRunOutputCapture>,
    ) -> std::io::Result<String> {
        let mut capture = capture
            .lock()
            .map_err(|_| std::io::Error::other("tool-run output capture poisoned"))?;
        capture.file_mut()?.flush()?;
        let file = File::open(&capture.part_path)?;
        let mut bytes = Vec::new();
        file.take(self.per_run_cap_bytes.saturating_add(1))
            .read_to_end(&mut bytes)?;
        if bytes.len() as u64 > self.per_run_cap_bytes {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "tool-run output capture exceeds configured cap",
            ));
        }
        sanitize_output(&strip_ansi(&String::from_utf8_lossy(&bytes)))
    }

    fn safe_path(&self, file_name: &str) -> std::io::Result<PathBuf> {
        let path = Path::new(file_name);
        if path.components().count() != 1
            || !matches!(path.components().next(), Some(Component::Normal(_)))
            || !file_name.ends_with(".log")
        {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "invalid tool-run output file name",
            ));
        }
        Ok(self.root.join(path))
    }

    pub fn cleanup(&self) -> std::io::Result<()> {
        let _guard = self
            .cleanup_lock
            .lock()
            .map_err(|_| std::io::Error::other("tool-run cleanup lock poisoned"))?;
        let now = SystemTime::now();
        let mut retained = Vec::new();
        for entry in fs::read_dir(&self.root)? {
            let entry = entry?;
            let path = entry.path();
            if !entry.file_type()?.is_file() {
                continue;
            }
            let name = entry.file_name();
            let name = name.to_string_lossy();
            // Partial captures may be the only useful evidence after an
            // abrupt stop. Boot recovery decides whether they belong to an
            // interrupted ledger row before deleting the remaining orphans.
            if name.ends_with(".part") {
                continue;
            }
            if name.ends_with(".tmp") {
                let _ = fs::remove_file(path);
                continue;
            }
            if !name.ends_with(".log") {
                continue;
            }
            let metadata = entry.metadata()?;
            let modified = metadata.modified().unwrap_or(SystemTime::UNIX_EPOCH);
            if now.duration_since(modified).unwrap_or(Duration::ZERO) > self.ttl {
                let _ = fs::remove_file(path);
                continue;
            }
            retained.push((modified, metadata.len(), path));
        }
        retained.sort_by_key(|(modified, _, _)| *modified);
        let mut total = retained.iter().map(|(_, len, _)| *len).sum::<u64>();
        for (_, len, path) in retained {
            if total <= self.total_cap_bytes {
                break;
            }
            if fs::remove_file(path).is_ok() {
                total = total.saturating_sub(len);
            }
        }
        Ok(())
    }
}

fn page_lines(
    content: &str,
    start_line: usize,
    max_lines: usize,
) -> std::io::Result<ToolRunOutputPage> {
    let lines = content.lines().collect::<Vec<_>>();
    let total_lines = lines.len();
    let start_line = start_line.max(1);
    if total_lines > 0 && start_line > total_lines {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            format!("start_line {start_line} exceeds {total_lines} lines"),
        ));
    }
    let start = start_line.saturating_sub(1).min(total_lines);
    let end = start
        .saturating_add(max_lines.clamp(1, 500))
        .min(total_lines);
    Ok(ToolRunOutputPage {
        start_line,
        end_line: end,
        total_lines,
        content: lines[start..end].join("\n"),
    })
}

pub(crate) fn page_content(
    content: &str,
    start_line: usize,
    max_lines: usize,
) -> std::io::Result<ToolRunOutputPage> {
    page_lines(content, start_line, max_lines)
}

pub(crate) fn tail_content(content: &str, max_lines: usize) -> std::io::Result<ToolRunOutputPage> {
    let total_lines = content.lines().count();
    let start_line = total_lines
        .saturating_sub(max_lines.max(1))
        .saturating_add(1);
    page_lines(content, start_line, max_lines)
}

pub(crate) fn search_content(
    content: &str,
    query: &str,
    max_matches: usize,
    case_sensitive: bool,
) -> std::io::Result<Vec<ToolRunOutputMatch>> {
    let query = query.trim();
    if query.is_empty() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "tool-run output query cannot be empty",
        ));
    }
    let folded_query = (!case_sensitive).then(|| query.to_lowercase());
    Ok(content
        .lines()
        .enumerate()
        .filter(|(_, line)| {
            if let Some(folded_query) = folded_query.as_deref() {
                line.to_lowercase().contains(folded_query)
            } else {
                line.contains(query)
            }
        })
        .take(max_matches.clamp(1, 100))
        .map(|(index, line)| ToolRunOutputMatch {
            line: index + 1,
            content: line.to_string(),
        })
        .collect())
}

fn validate_run_id(run_id: &str) -> std::io::Result<()> {
    if run_id.len() > 96
        || !run_id.starts_with("toolrun-")
        || !run_id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
    {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "invalid tool-run id",
        ));
    }
    Ok(())
}

fn private_file(path: &Path, create_new: bool) -> std::io::Result<File> {
    let mut options = OpenOptions::new();
    options.write(true);
    if create_new {
        options.create_new(true);
    } else {
        options.create(true).truncate(true);
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let file = options.open(path)?;
    set_private_file_permissions(path)?;
    Ok(file)
}

fn set_private_dir_permissions(path: &Path) -> std::io::Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o700))?;
    }
    Ok(())
}

fn set_private_file_permissions(path: &Path) -> std::io::Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o600))?;
    }
    Ok(())
}

fn sync_directory(path: &Path) -> std::io::Result<()> {
    #[cfg(unix)]
    {
        File::open(path)?.sync_all()?;
    }
    Ok(())
}

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

fn sanitize_output(input: &str) -> std::io::Result<String> {
    let redacted = PRIVATE_KEY_BLOCK.replace_all(input, "[REDACTED PRIVATE KEY]");
    let redacted = NAMED_SECRET.replace_all(&redacted, "$1[REDACTED]");
    let redacted = BEARER_TOKEN.replace_all(&redacted, "Bearer [REDACTED]");
    let redacted = KNOWN_TOKEN.replace_all(&redacted, "[REDACTED TOKEN]");
    if let Some(kind) = crate::memory_policy::scan_for_secrets(&redacted) {
        return Err(std::io::Error::new(
            std::io::ErrorKind::PermissionDenied,
            format!("tool-run output still matches secret policy: {kind}"),
        ));
    }
    Ok(redacted.into_owned())
}

pub(crate) fn sanitize_for_retention(input: &str) -> std::io::Result<(String, bool)> {
    let stripped = strip_ansi(input);
    let sanitized = sanitize_output(&stripped)?;
    let changed = stripped != input || sanitized != stripped;
    Ok((sanitized, changed))
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

    let mut out = String::with_capacity(input.len());
    let mut state = State::Text;
    for ch in input.chars() {
        state = match state {
            State::Text => match ch {
                '\u{1b}' => State::Escape,
                '\n' | '\r' | '\t' => {
                    out.push(ch);
                    State::Text
                }
                value if value.is_control() => State::Text,
                _ => {
                    out.push(ch);
                    State::Text
                }
            },
            State::Escape => match ch {
                '[' => State::Csi,
                ']' => State::Osc,
                'P' | 'X' | '^' | '_' => State::ControlString,
                _ => State::Text,
            },
            State::Csi => {
                if ('@'..='~').contains(&ch) {
                    State::Text
                } else {
                    State::Csi
                }
            }
            State::Osc => match ch {
                '\u{7}' => State::Text,
                '\u{1b}' => State::OscEscape,
                _ => State::Osc,
            },
            State::OscEscape => match ch {
                '\\' => State::Text,
                '\u{1b}' => State::OscEscape,
                _ => State::Osc,
            },
            State::ControlString => {
                if ch == '\u{1b}' {
                    State::ControlStringEscape
                } else {
                    State::ControlString
                }
            }
            State::ControlStringEscape => {
                if ch == '\\' {
                    State::Text
                } else if ch == '\u{1b}' {
                    State::ControlStringEscape
                } else {
                    State::ControlString
                }
            }
        };
    }
    out
}

fn contains_redaction_marker(input: &str) -> bool {
    input.contains("[REDACTED")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn store(root: PathBuf) -> ToolRunOutputStore {
        ToolRunOutputStore::with_limits(root, 256, 512, Duration::from_secs(60)).unwrap()
    }

    #[test]
    fn capture_is_bounded_redacted_and_verified() {
        let dir = tempfile::tempdir().unwrap();
        let store = store(dir.path().join("runs"));
        let mut capture = store.begin_capture("toolrun-abc-123").unwrap();
        capture
            .append(
                format!("password=super-secret-value\n{}", "x".repeat(600)).as_bytes(),
                store.per_run_cap_bytes,
            )
            .unwrap();
        let metadata = store
            .finalize_capture("toolrun-abc-123", &Mutex::new(capture))
            .unwrap();
        assert!(metadata.capped);
        assert!(metadata.redacted);
        assert!(metadata.total_bytes > metadata.stored_bytes);
        let page = store.read_lines(&metadata, 1, 20).unwrap();
        assert!(page.content.contains("password=[REDACTED]"));
        assert!(!page.content.contains("super-secret-value"));
    }

    #[test]
    fn search_tail_and_checksum_guard_are_deterministic() {
        let dir = tempfile::tempdir().unwrap();
        let store = store(dir.path().join("runs"));
        let metadata = store
            .persist_content(
                "toolrun-search-1",
                "alpha\nbeta needle\ngamma\nneedle delta",
            )
            .unwrap();
        assert!(!metadata.redacted);
        let matches = store.search_lines(&metadata, "NEEDLE", 10, false).unwrap();
        assert_eq!(matches.iter().map(|m| m.line).collect::<Vec<_>>(), [2, 4]);
        assert_eq!(store.tail_lines(&metadata, 2).unwrap().start_line, 3);

        fs::write(store.root.join(&metadata.file_name), "tampered").unwrap();
        assert!(store.read_lines(&metadata, 1, 10).is_err());
    }

    #[test]
    fn boot_recovery_preserves_matching_partial_then_removes_orphans() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("runs");
        fs::create_dir_all(&root).unwrap();
        fs::write(
            root.join("toolrun-recover.part"),
            "\u{1b}]0;host-title\u{7}password=secret-value\npartial\n",
        )
        .unwrap();
        fs::write(root.join("toolrun-orphan.part"), "orphan").unwrap();
        let store =
            ToolRunOutputStore::with_limits(root.clone(), 256, 512, Duration::ZERO).unwrap();
        let metadata = store
            .recover_interrupted_output("toolrun-recover")
            .unwrap()
            .unwrap();
        assert!(metadata.redacted);
        let recovered = store.read_lines(&metadata, 1, 10).unwrap();
        assert_eq!(recovered.content, "password=[REDACTED]\npartial");
        assert!(!root.join("toolrun-recover.part").exists());
        assert_eq!(store.discard_orphaned_captures().unwrap(), 1);
        assert!(!root.join("toolrun-orphan.part").exists());
    }

    #[test]
    fn ansi_stripping_removes_csi_osc_and_control_strings() {
        let input = "a\u{1b}[31mred\u{1b}[0m\u{1b}]0;hidden title\u{7}b\u{1b}Ppayload\u{1b}\\c";
        assert_eq!(strip_ansi(input), "aredbc");
    }

    #[test]
    fn traversal_and_unknown_secret_shapes_fail_closed() {
        let dir = tempfile::tempdir().unwrap();
        let store = store(dir.path().join("runs"));
        assert!(store.begin_capture("../escape").is_err());
        let err = store
            .persist_content(
                "toolrun-jwt-1",
                "eyJaaaaaaaaaaaaaaaa.bbbbbbbbbbbbbbbb.cccccccccccccccc",
            )
            .unwrap_err();
        assert_eq!(err.kind(), std::io::ErrorKind::PermissionDenied);
    }
}
