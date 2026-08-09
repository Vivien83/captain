//! Crash-safe immutable artifacts produced by Captain.
//!
//! Every version is a self-contained directory finalized with one rename.
//! There is deliberately no mutable global index: after a power loss, a
//! complete version is discoverable and an incomplete staging directory is
//! disposable.

use captain_types::artifact::{
    ArtifactInventory, ArtifactPreviewKind, ArtifactStoreStatus, ArtifactVersion,
};
use chrono::Utc;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::fs::{self, File};
use std::io::{BufReader, Read};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use uuid::Uuid;

const MANIFEST_VERSION: u32 = 1;
const MANIFEST_FILE: &str = "manifest.json";
const PAYLOAD_FILE: &str = "payload";
const VERSIONS_DIR: &str = "versions";
const STAGING_PREFIX: &str = ".staging-";
pub const MAX_ARTIFACT_BYTES: u64 = 50 * 1024 * 1024;
const DEFAULT_MAX_TOTAL_BYTES: u64 = 512 * 1024 * 1024;
const MAX_TOTAL_VERSIONS: usize = 2048;

#[derive(Debug, Clone)]
pub struct PublishArtifactRequest {
    pub artifact_id: Option<Uuid>,
    pub agent_id: String,
    pub session_id: Option<String>,
    pub title: String,
    pub filename: String,
    pub mime_type: String,
    pub summary: Option<String>,
    pub source_path: PathBuf,
    /// Optional digest bound to content already inspected by the caller.
    pub expected_sha256: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct VersionManifest {
    schema_version: u32,
    artifact: ArtifactVersion,
}

#[derive(Debug)]
pub struct ArtifactStore {
    root: PathBuf,
    mutation_lock: Mutex<()>,
    recovered_staging_entries: usize,
    max_artifact_bytes: u64,
    max_total_bytes: u64,
}

impl ArtifactStore {
    pub fn open(root: PathBuf) -> Result<Self, String> {
        Self::open_with_limits(root, MAX_ARTIFACT_BYTES, DEFAULT_MAX_TOTAL_BYTES)
    }

    fn open_with_limits(
        root: PathBuf,
        max_artifact_bytes: u64,
        max_total_bytes: u64,
    ) -> Result<Self, String> {
        captain_types::durable_fs::create_dir_all(&root)
            .map_err(|error| format!("create artifact store {}: {error}", root.display()))?;
        make_directory_private(&root)?;
        let recovered_staging_entries = recover_staging_entries(&root)?;
        Ok(Self {
            root,
            mutation_lock: Mutex::new(()),
            recovered_staging_entries,
            max_artifact_bytes,
            max_total_bytes,
        })
    }

    pub fn publish(&self, request: PublishArtifactRequest) -> Result<ArtifactVersion, String> {
        validate_publish_request(&request)?;
        let source_meta = fs::symlink_metadata(&request.source_path).map_err(|error| {
            format!(
                "inspect artifact source {}: {error}",
                request.source_path.display()
            )
        })?;
        if source_meta.file_type().is_symlink() || !source_meta.is_file() {
            return Err("artifact source must be a regular non-symlink file".to_string());
        }
        if source_meta.len() == 0 {
            return Err("artifact source is empty".to_string());
        }
        if source_meta.len() > self.max_artifact_bytes {
            return Err(format!(
                "artifact is too large ({} bytes, max {})",
                source_meta.len(),
                self.max_artifact_bytes
            ));
        }

        let _guard = self
            .mutation_lock
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        let inventory = self.scan_inventory()?;
        if inventory.status.versions >= MAX_TOTAL_VERSIONS {
            return Err(format!(
                "artifact version limit reached ({MAX_TOTAL_VERSIONS}); delete an artifact first"
            ));
        }
        let projected = inventory
            .status
            .bytes
            .checked_add(source_meta.len())
            .ok_or("artifact quota calculation overflow")?;
        if projected > self.max_total_bytes {
            return Err(format!(
                "artifact store quota exceeded ({projected} bytes projected, max {}); delete an artifact first",
                self.max_total_bytes
            ));
        }

        let (artifact_id, version, is_new) = match request.artifact_id {
            Some(artifact_id) => {
                let latest = self.latest_manifest(artifact_id)?;
                if latest.agent_id != request.agent_id {
                    return Err(
                        "only the owning agent may publish a new artifact version".to_string()
                    );
                }
                let version = latest
                    .version
                    .checked_add(1)
                    .ok_or("artifact version overflow")?;
                (artifact_id, version, false)
            }
            None => (Uuid::new_v4(), 1, true),
        };

        let stage = self
            .root
            .join(format!("{STAGING_PREFIX}{}", Uuid::new_v4()));
        let stage_version = if is_new {
            stage.join(VERSIONS_DIR).join(version_dir(version))
        } else {
            stage.clone()
        };
        let publish_result = self.stage_version(&request, artifact_id, version, &stage_version);
        let artifact = match publish_result {
            Ok(artifact) => artifact,
            Err(error) => {
                let _ = remove_staging_entry(&stage);
                return Err(error);
            }
        };
        if request
            .expected_sha256
            .as_deref()
            .is_some_and(|expected| expected != artifact.sha256)
        {
            let _ = remove_staging_entry(&stage);
            return Err(
                "artifact source changed after inspection; publish the stable file again"
                    .to_string(),
            );
        }
        let actual_projected = inventory
            .status
            .bytes
            .checked_add(artifact.size_bytes)
            .ok_or("artifact quota calculation overflow")?;
        if actual_projected > self.max_total_bytes {
            let _ = remove_staging_entry(&stage);
            return Err(format!(
                "artifact store quota exceeded after copy ({actual_projected} bytes projected, max {}); source changed while it was read",
                self.max_total_bytes
            ));
        }

        let destination = if is_new {
            self.root.join(artifact_id.to_string())
        } else {
            let versions = self.root.join(artifact_id.to_string()).join(VERSIONS_DIR);
            captain_types::durable_fs::create_dir_all(&versions)
                .map_err(|error| format!("create artifact versions directory: {error}"))?;
            versions.join(version_dir(version))
        };
        if destination.exists() {
            let _ = remove_staging_entry(&stage);
            return Err("artifact version destination already exists".to_string());
        }
        if let Err(error) = fs::rename(&stage, &destination) {
            let _ = remove_staging_entry(&stage);
            return Err(format!("finalize artifact version: {error}"));
        }
        if let Err(error) = sync_directory(destination.parent().unwrap_or(&self.root)) {
            if fs::rename(&destination, &stage).is_ok() {
                let _ = sync_directory(destination.parent().unwrap_or(&self.root));
                let _ = remove_staging_entry(&stage);
            }
            return Err(error);
        }
        Ok(artifact)
    }

    fn stage_version(
        &self,
        request: &PublishArtifactRequest,
        artifact_id: Uuid,
        version: u32,
        stage_version: &Path,
    ) -> Result<ArtifactVersion, String> {
        captain_types::durable_fs::create_dir_all(stage_version)
            .map_err(|error| format!("create artifact staging directory: {error}"))?;
        let mut private_dir = Some(stage_version);
        while let Some(path) = private_dir {
            if path == self.root {
                break;
            }
            make_directory_private(path)?;
            private_dir = path.parent();
        }
        let payload = stage_version.join(PAYLOAD_FILE);
        captain_types::durable_fs::atomic_copy(&request.source_path, &payload)
            .map_err(|error| format!("copy artifact payload: {error}"))?;
        let payload_meta = fs::metadata(&payload)
            .map_err(|error| format!("inspect staged artifact payload: {error}"))?;
        if payload_meta.len() == 0 || payload_meta.len() > self.max_artifact_bytes {
            return Err("staged artifact payload violates the size limit".to_string());
        }
        let sha256 = sha256_file(&payload)?;
        let artifact = ArtifactVersion {
            artifact_id,
            version,
            agent_id: request.agent_id.clone(),
            session_id: request.session_id.clone(),
            title: request.title.clone(),
            filename: request.filename.clone(),
            mime_type: request.mime_type.clone(),
            preview_kind: preview_kind(&request.mime_type),
            size_bytes: payload_meta.len(),
            sha256,
            created_at: Utc::now(),
            summary: request.summary.clone(),
        };
        validate_artifact(&artifact, self.max_artifact_bytes)?;
        let mut manifest = serde_json::to_vec_pretty(&VersionManifest {
            schema_version: MANIFEST_VERSION,
            artifact: artifact.clone(),
        })
        .map_err(|error| format!("serialize artifact manifest: {error}"))?;
        manifest.push(b'\n');
        captain_types::durable_fs::atomic_write(&stage_version.join(MANIFEST_FILE), &manifest)
            .map_err(|error| format!("persist artifact manifest: {error}"))?;
        sync_directory(stage_version)?;
        Ok(artifact)
    }

    pub fn list(&self, agent_id: Option<&str>, limit: usize) -> Result<ArtifactInventory, String> {
        let mut inventory = self.scan_inventory()?;
        if let Some(agent_id) = agent_id {
            inventory.items.retain(|item| item.agent_id == agent_id);
        }
        inventory
            .items
            .sort_by_key(|item| std::cmp::Reverse(item.created_at));
        inventory.items.truncate(limit.clamp(1, 200));
        Ok(inventory)
    }

    pub fn status(&self) -> Result<ArtifactStoreStatus, String> {
        self.scan_inventory().map(|inventory| inventory.status)
    }

    pub fn inspect(
        &self,
        artifact_id: Uuid,
        version: Option<u32>,
    ) -> Result<ArtifactVersion, String> {
        let artifact = self.requested_manifest(artifact_id, version)?;
        self.verify_payload(artifact)
    }

    pub fn inspect_owned(
        &self,
        agent_id: &str,
        artifact_id: Uuid,
        version: Option<u32>,
    ) -> Result<ArtifactVersion, String> {
        let artifact = self.requested_manifest(artifact_id, version)?;
        if artifact.agent_id != agent_id {
            return Err("artifact is unavailable to the calling agent".to_string());
        }
        self.verify_payload(artifact)
    }

    fn verify_payload(&self, artifact: ArtifactVersion) -> Result<ArtifactVersion, String> {
        let payload = self.payload_path_unchecked(artifact.artifact_id, artifact.version);
        let metadata = fs::metadata(&payload)
            .map_err(|error| format!("artifact payload is unavailable: {error}"))?;
        if metadata.len() != artifact.size_bytes {
            return Err("artifact payload size does not match its immutable manifest".to_string());
        }
        if sha256_file(&payload)? != artifact.sha256 {
            return Err(
                "artifact payload checksum does not match its immutable manifest".to_string(),
            );
        }
        Ok(artifact)
    }

    pub fn verified_payload_path(
        &self,
        artifact_id: Uuid,
        version: Option<u32>,
    ) -> Result<(ArtifactVersion, PathBuf), String> {
        let artifact = self.inspect(artifact_id, version)?;
        let path = self.payload_path_unchecked(artifact_id, artifact.version);
        Ok((artifact, path))
    }

    pub fn verified_payload_path_owned(
        &self,
        agent_id: &str,
        artifact_id: Uuid,
        version: Option<u32>,
    ) -> Result<(ArtifactVersion, PathBuf), String> {
        let artifact = self.inspect_owned(agent_id, artifact_id, version)?;
        let path = self.payload_path_unchecked(artifact_id, artifact.version);
        Ok((artifact, path))
    }

    /// Read one exact/latest payload and revalidate the bytes returned to the
    /// caller, closing the gap between a path checksum and a later HTTP read.
    pub fn read_verified_payload(
        &self,
        artifact_id: Uuid,
        version: Option<u32>,
    ) -> Result<(ArtifactVersion, Vec<u8>), String> {
        let artifact = self.requested_manifest(artifact_id, version)?;
        let path = self.payload_path_unchecked(artifact_id, artifact.version);
        let data =
            fs::read(&path).map_err(|error| format!("read verified artifact payload: {error}"))?;
        if data.len() as u64 != artifact.size_bytes
            || format!("{:x}", Sha256::digest(&data)) != artifact.sha256
        {
            return Err("artifact payload changed after manifest inspection".to_string());
        }
        Ok((artifact, data))
    }

    /// Return immutable manifests newest-first without reading payload bytes.
    /// Call `inspect` or `read_verified_payload` before consuming one version.
    pub fn list_versions(&self, artifact_id: Uuid) -> Result<Vec<ArtifactVersion>, String> {
        let mut items = version_numbers(&self.root.join(artifact_id.to_string()))?
            .into_iter()
            .map(|version| self.read_manifest(artifact_id, version))
            .collect::<Result<Vec<_>, _>>()?;
        items.sort_by_key(|artifact| std::cmp::Reverse(artifact.version));
        Ok(items)
    }

    fn scan_inventory(&self) -> Result<ArtifactInventory, String> {
        let mut items = Vec::new();
        let mut versions = 0usize;
        let bytes = managed_store_bytes(&self.root)?;
        let mut invalid_entries = 0usize;
        for entry in fs::read_dir(&self.root)
            .map_err(|error| format!("read artifact store {}: {error}", self.root.display()))?
        {
            let entry = entry.map_err(|error| format!("read artifact entry: {error}"))?;
            let name = entry.file_name().to_string_lossy().to_string();
            if name.starts_with('.') {
                continue;
            }
            let Ok(artifact_id) = Uuid::parse_str(&name) else {
                invalid_entries += 1;
                continue;
            };
            let Ok(version_numbers) = version_numbers(&entry.path()) else {
                invalid_entries += 1;
                continue;
            };
            let mut latest = None;
            for version in version_numbers {
                match self.read_manifest(artifact_id, version) {
                    Ok(artifact) => {
                        versions += 1;
                        latest = Some(artifact);
                    }
                    Err(_) => invalid_entries += 1,
                }
            }
            if let Some(latest) = latest {
                items.push(latest);
            }
        }
        Ok(ArtifactInventory {
            status: ArtifactStoreStatus {
                healthy: invalid_entries == 0,
                artifacts: items.len(),
                versions,
                bytes,
                invalid_entries,
                recovered_staging_entries: self.recovered_staging_entries,
                max_artifact_bytes: self.max_artifact_bytes,
                max_total_bytes: self.max_total_bytes,
            },
            items,
        })
    }

    fn latest_manifest(&self, artifact_id: Uuid) -> Result<ArtifactVersion, String> {
        let versions = version_numbers(&self.root.join(artifact_id.to_string()))?;
        let version = versions
            .into_iter()
            .max()
            .ok_or_else(|| format!("artifact {artifact_id} has no versions"))?;
        self.read_manifest(artifact_id, version)
    }

    fn requested_manifest(
        &self,
        artifact_id: Uuid,
        version: Option<u32>,
    ) -> Result<ArtifactVersion, String> {
        match version {
            Some(version) => self.read_manifest(artifact_id, version),
            None => self.latest_manifest(artifact_id),
        }
    }

    fn read_manifest(&self, artifact_id: Uuid, version: u32) -> Result<ArtifactVersion, String> {
        if version == 0 {
            return Err("artifact version must be positive".to_string());
        }
        let path = self.version_path(artifact_id, version).join(MANIFEST_FILE);
        let raw = fs::read(&path)
            .map_err(|error| format!("read artifact manifest {}: {error}", path.display()))?;
        if raw.len() > 64 * 1024 {
            return Err("artifact manifest exceeds 64 KiB".to_string());
        }
        let manifest: VersionManifest = serde_json::from_slice(&raw)
            .map_err(|error| format!("parse artifact manifest {}: {error}", path.display()))?;
        if manifest.schema_version != MANIFEST_VERSION {
            return Err(format!(
                "unsupported artifact manifest schema {}",
                manifest.schema_version
            ));
        }
        if manifest.artifact.artifact_id != artifact_id || manifest.artifact.version != version {
            return Err("artifact manifest identity does not match its directory".to_string());
        }
        validate_artifact(&manifest.artifact, self.max_artifact_bytes)?;
        let payload_meta = fs::metadata(self.payload_path_unchecked(artifact_id, version))
            .map_err(|error| format!("inspect artifact payload: {error}"))?;
        if !payload_meta.is_file() || payload_meta.len() != manifest.artifact.size_bytes {
            return Err("artifact payload is missing or has an unexpected size".to_string());
        }
        Ok(manifest.artifact)
    }

    fn version_path(&self, artifact_id: Uuid, version: u32) -> PathBuf {
        self.root
            .join(artifact_id.to_string())
            .join(VERSIONS_DIR)
            .join(version_dir(version))
    }

    fn payload_path_unchecked(&self, artifact_id: Uuid, version: u32) -> PathBuf {
        self.version_path(artifact_id, version).join(PAYLOAD_FILE)
    }
}

fn version_numbers(artifact_root: &Path) -> Result<Vec<u32>, String> {
    let versions_root = artifact_root.join(VERSIONS_DIR);
    let mut out = Vec::new();
    for entry in fs::read_dir(&versions_root).map_err(|error| {
        format!(
            "read artifact versions {}: {error}",
            versions_root.display()
        )
    })? {
        let entry = entry.map_err(|error| format!("read artifact version entry: {error}"))?;
        let name = entry.file_name().to_string_lossy().to_string();
        if name.len() != 8 || !name.bytes().all(|byte| byte.is_ascii_digit()) {
            return Err(format!("invalid artifact version directory {name:?}"));
        }
        let version = name
            .parse::<u32>()
            .map_err(|_| format!("invalid artifact version {name:?}"))?;
        if version == 0 || !entry.path().is_dir() {
            return Err(format!("invalid artifact version entry {name:?}"));
        }
        out.push(version);
    }
    out.sort_unstable();
    Ok(out)
}

fn version_dir(version: u32) -> String {
    format!("{version:08}")
}

fn validate_publish_request(request: &PublishArtifactRequest) -> Result<(), String> {
    validate_text("agent_id", &request.agent_id, 128, false)?;
    if let Some(session_id) = request.session_id.as_deref() {
        validate_text("session_id", session_id, 128, false)?;
    }
    validate_text("title", &request.title, 160, false)?;
    validate_filename(&request.filename)?;
    validate_mime_type(&request.mime_type)?;
    if let Some(summary) = request.summary.as_deref() {
        validate_text("summary", summary, 1000, true)?;
    }
    if let Some(expected_sha256) = request.expected_sha256.as_deref() {
        validate_sha256("expected_sha256", expected_sha256)?;
    }
    Ok(())
}

fn validate_artifact(artifact: &ArtifactVersion, max_bytes: u64) -> Result<(), String> {
    validate_text("agent_id", &artifact.agent_id, 128, false)?;
    if let Some(session_id) = artifact.session_id.as_deref() {
        validate_text("session_id", session_id, 128, false)?;
    }
    validate_text("title", &artifact.title, 160, false)?;
    validate_filename(&artifact.filename)?;
    validate_mime_type(&artifact.mime_type)?;
    if artifact.preview_kind != preview_kind(&artifact.mime_type) {
        return Err("artifact preview kind does not match its MIME type".to_string());
    }
    if artifact.version == 0 || artifact.size_bytes == 0 || artifact.size_bytes > max_bytes {
        return Err("artifact manifest contains invalid version or size metadata".to_string());
    }
    validate_sha256("manifest sha256", &artifact.sha256)?;
    if let Some(summary) = artifact.summary.as_deref() {
        validate_text("summary", summary, 1000, true)?;
    }
    Ok(())
}

fn validate_sha256(name: &str, value: &str) -> Result<(), String> {
    if value.len() != 64
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        return Err(format!("artifact {name} is not a lowercase SHA-256"));
    }
    Ok(())
}

fn validate_text(name: &str, value: &str, max_chars: usize, multiline: bool) -> Result<(), String> {
    if value.trim().is_empty() || value.chars().count() > max_chars {
        return Err(format!(
            "artifact {name} must contain 1..={max_chars} characters"
        ));
    }
    if value
        .chars()
        .any(|ch| ch.is_control() && !(multiline && matches!(ch, '\n' | '\r' | '\t')))
    {
        return Err(format!("artifact {name} contains control characters"));
    }
    Ok(())
}

fn validate_filename(filename: &str) -> Result<(), String> {
    validate_text("filename", filename, 180, false)?;
    if filename == "." || filename == ".." || filename.contains('/') || filename.contains('\\') {
        return Err("artifact filename must be a single safe basename".to_string());
    }
    Ok(())
}

fn validate_mime_type(mime_type: &str) -> Result<(), String> {
    if mime_type.len() > 128
        || !mime_type.is_ascii()
        || mime_type.chars().any(char::is_control)
        || mime_type.matches('/').count() != 1
        || mime_type.contains(|ch: char| ch.is_ascii_whitespace())
    {
        return Err("artifact mime_type is invalid".to_string());
    }
    Ok(())
}

pub fn preview_kind(mime_type: &str) -> ArtifactPreviewKind {
    match mime_type {
        "text/markdown" => ArtifactPreviewKind::Markdown,
        "text/html" => ArtifactPreviewKind::Html,
        "application/pdf" => ArtifactPreviewKind::Pdf,
        "image/png" | "image/jpeg" | "image/gif" | "image/webp" => ArtifactPreviewKind::Image,
        value
            if value.starts_with("text/")
                || matches!(value, "application/json" | "application/xml") =>
        {
            ArtifactPreviewKind::Text
        }
        _ => ArtifactPreviewKind::None,
    }
}

pub fn mime_type_for_filename(filename: &str) -> &'static str {
    let extension = Path::new(filename)
        .extension()
        .and_then(|value| value.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    match extension.as_str() {
        "txt" => "text/plain",
        "md" | "markdown" => "text/markdown",
        "html" | "htm" => "text/html",
        "csv" => "text/csv",
        "json" => "application/json",
        "xml" => "application/xml",
        "pdf" => "application/pdf",
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "docx" => "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
        "xlsx" => "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
        "zip" => "application/zip",
        _ => "application/octet-stream",
    }
}

fn sha256_file(path: &Path) -> Result<String, String> {
    let file =
        File::open(path).map_err(|error| format!("open artifact payload for checksum: {error}"))?;
    let mut reader = BufReader::new(file);
    let mut hasher = Sha256::new();
    let mut buffer = [0u8; 64 * 1024];
    loop {
        let read = reader
            .read(&mut buffer)
            .map_err(|error| format!("read artifact payload for checksum: {error}"))?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    Ok(format!("{:x}", hasher.finalize()))
}

/// Count every physical payload, including versions whose manifest is corrupt.
/// This keeps malformed metadata from bypassing the disk quota while avoiding
/// expensive checksum reads on ordinary status requests.
fn managed_store_bytes(root: &Path) -> Result<u64, String> {
    let mut total = 0u64;
    let mut pending = vec![root.to_path_buf()];
    while let Some(directory) = pending.pop() {
        for entry in fs::read_dir(&directory)
            .map_err(|error| format!("measure artifact store {}: {error}", directory.display()))?
        {
            let entry = entry.map_err(|error| format!("measure artifact entry: {error}"))?;
            if directory == root
                && entry
                    .file_name()
                    .to_string_lossy()
                    .starts_with(STAGING_PREFIX)
            {
                continue;
            }
            let metadata = fs::symlink_metadata(entry.path())
                .map_err(|error| format!("measure artifact entry metadata: {error}"))?;
            if metadata.is_dir() {
                pending.push(entry.path());
            } else if metadata.is_file() && entry.file_name() == PAYLOAD_FILE {
                total = total
                    .checked_add(metadata.len())
                    .ok_or("artifact store byte count overflow")?;
            }
        }
    }
    Ok(total)
}

fn recover_staging_entries(root: &Path) -> Result<usize, String> {
    let mut recovered = 0usize;
    for entry in
        fs::read_dir(root).map_err(|error| format!("scan artifact staging entries: {error}"))?
    {
        let entry = entry.map_err(|error| format!("read artifact staging entry: {error}"))?;
        if entry
            .file_name()
            .to_string_lossy()
            .starts_with(STAGING_PREFIX)
        {
            remove_staging_entry(&entry.path())?;
            recovered += 1;
        }
    }
    if recovered > 0 {
        sync_directory(root)?;
    }
    Ok(recovered)
}

fn remove_staging_entry(path: &Path) -> Result<(), String> {
    let metadata = fs::symlink_metadata(path)
        .map_err(|error| format!("inspect artifact staging entry: {error}"))?;
    if metadata.file_type().is_symlink() || metadata.is_file() {
        fs::remove_file(path)
    } else {
        fs::remove_dir_all(path)
    }
    .map_err(|error| format!("remove artifact staging entry {}: {error}", path.display()))
}

fn sync_directory(path: &Path) -> Result<(), String> {
    File::open(path)
        .and_then(|directory| directory.sync_all())
        .map_err(|error| format!("sync artifact directory {}: {error}", path.display()))
}

#[cfg(unix)]
fn make_directory_private(path: &Path) -> Result<(), String> {
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(path, fs::Permissions::from_mode(0o700))
        .map_err(|error| format!("secure artifact directory {}: {error}", path.display()))
}

#[cfg(not(unix))]
fn make_directory_private(_path: &Path) -> Result<(), String> {
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn request(source_path: PathBuf) -> PublishArtifactRequest {
        PublishArtifactRequest {
            artifact_id: None,
            agent_id: "captain".to_string(),
            session_id: Some("session-1".to_string()),
            title: "Operational report".to_string(),
            filename: "report.md".to_string(),
            mime_type: "text/markdown".to_string(),
            summary: Some("Verified report".to_string()),
            source_path,
            expected_sha256: None,
        }
    }

    #[test]
    fn versions_are_immutable_and_survive_reopen() {
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("report.md");
        fs::write(&source, "version one").unwrap();
        let root = dir.path().join("artifacts");
        let store = ArtifactStore::open(root.clone()).unwrap();
        let first = store.publish(request(source.clone())).unwrap();

        fs::write(&source, "version two").unwrap();
        let mut second_request = request(source);
        second_request.artifact_id = Some(first.artifact_id);
        let second = store.publish(second_request).unwrap();

        assert_eq!(first.version, 1);
        assert_eq!(second.version, 2);
        assert_ne!(first.sha256, second.sha256);
        assert_eq!(store.inspect(first.artifact_id, Some(1)).unwrap(), first);
        assert_eq!(store.inspect(first.artifact_id, None).unwrap(), second);
        assert_eq!(
            store
                .list_versions(first.artifact_id)
                .unwrap()
                .iter()
                .map(|artifact| artifact.version)
                .collect::<Vec<_>>(),
            vec![2, 1]
        );
        let (read_back, bytes) = store
            .read_verified_payload(first.artifact_id, Some(1))
            .unwrap();
        assert_eq!(read_back, first);
        assert_eq!(bytes, b"version one");

        let reopened = ArtifactStore::open(root).unwrap();
        let inventory = reopened.list(Some("captain"), 20).unwrap();
        assert_eq!(inventory.items, vec![second]);
        assert_eq!(inventory.status.artifacts, 1);
        assert_eq!(inventory.status.versions, 2);
    }

    #[test]
    fn interrupted_staging_is_removed_without_touching_committed_versions() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("artifacts");
        let store = ArtifactStore::open(root.clone()).unwrap();
        let source = dir.path().join("report.txt");
        fs::write(&source, "committed").unwrap();
        let committed = store.publish(request(source)).unwrap();
        drop(store);

        let staging = root.join(format!("{STAGING_PREFIX}dead"));
        fs::create_dir_all(&staging).unwrap();
        fs::write(staging.join("partial"), "partial").unwrap();
        let reopened = ArtifactStore::open(root).unwrap();

        assert_eq!(reopened.status().unwrap().recovered_staging_entries, 1);
        assert_eq!(
            reopened.inspect(committed.artifact_id, None).unwrap(),
            committed
        );
    }

    #[test]
    fn payload_tampering_is_detected() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("artifacts");
        let source = dir.path().join("report.md");
        fs::write(&source, "trusted").unwrap();
        let store = ArtifactStore::open(root.clone()).unwrap();
        let artifact = store.publish(request(source)).unwrap();
        let payload = root
            .join(artifact.artifact_id.to_string())
            .join(VERSIONS_DIR)
            .join(version_dir(1))
            .join(PAYLOAD_FILE);
        fs::write(payload, "altered").unwrap();

        assert!(store
            .inspect(artifact.artifact_id, Some(1))
            .unwrap_err()
            .contains("checksum"));
    }

    #[test]
    fn inspected_digest_mismatch_never_becomes_visible() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("artifacts");
        let source = dir.path().join("report.md");
        fs::write(&source, "stable content").unwrap();
        let store = ArtifactStore::open(root).unwrap();
        let mut publish = request(source);
        publish.expected_sha256 = Some("0".repeat(64));

        let error = store.publish(publish).unwrap_err();
        assert!(error.contains("source changed after inspection"));
        let status = store.status().unwrap();
        assert_eq!(status.artifacts, 0);
        assert_eq!(status.versions, 0);
        assert_eq!(status.bytes, 0);
    }

    #[test]
    fn quota_refuses_new_data_without_pruning_old_versions() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("artifacts");
        let source = dir.path().join("report.md");
        fs::write(&source, "123456").unwrap();
        let store = ArtifactStore::open_with_limits(root, 10, 10).unwrap();
        let first = store.publish(request(source.clone())).unwrap();

        let error = store.publish(request(source)).unwrap_err();
        assert!(error.contains("quota exceeded"));
        assert_eq!(store.inspect(first.artifact_id, None).unwrap(), first);
    }

    #[test]
    fn concurrent_updates_allocate_distinct_versions() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("artifacts");
        let initial_source = dir.path().join("initial.md");
        fs::write(&initial_source, "initial").unwrap();
        let store = std::sync::Arc::new(ArtifactStore::open(root).unwrap());
        let first = store.publish(request(initial_source)).unwrap();

        let mut handles = Vec::new();
        for index in 0..8 {
            let source = dir.path().join(format!("report-{index}.md"));
            fs::write(&source, format!("version {index}")).unwrap();
            let store = std::sync::Arc::clone(&store);
            let artifact_id = first.artifact_id;
            handles.push(std::thread::spawn(move || {
                let mut update = request(source);
                update.artifact_id = Some(artifact_id);
                store.publish(update).unwrap().version
            }));
        }
        let mut versions = handles
            .into_iter()
            .map(|handle| handle.join().unwrap())
            .collect::<Vec<_>>();
        versions.sort_unstable();

        assert_eq!(versions, (2..=9).collect::<Vec<_>>());
        assert_eq!(store.inspect(first.artifact_id, None).unwrap().version, 9);
    }

    #[test]
    fn manifest_never_persists_the_source_path() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("artifacts");
        let source = dir.path().join("private-location.md");
        fs::write(&source, "report").unwrap();
        let store = ArtifactStore::open(root.clone()).unwrap();
        let artifact = store.publish(request(source.clone())).unwrap();
        let manifest = fs::read_to_string(
            root.join(artifact.artifact_id.to_string())
                .join(VERSIONS_DIR)
                .join(version_dir(1))
                .join(MANIFEST_FILE),
        )
        .unwrap();

        assert!(!manifest.contains(source.to_string_lossy().as_ref()));
        assert!(manifest.contains("report.md"));
    }

    #[cfg(unix)]
    #[test]
    fn symlink_sources_are_rejected() {
        use std::os::unix::fs::symlink;
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("target.md");
        let source = dir.path().join("source.md");
        fs::write(&target, "report").unwrap();
        symlink(target, &source).unwrap();
        let store = ArtifactStore::open(dir.path().join("artifacts")).unwrap();

        assert!(store
            .publish(request(source))
            .unwrap_err()
            .contains("symlink"));
    }

    #[cfg(unix)]
    #[test]
    fn committed_artifact_directories_are_owner_only() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("artifacts");
        let source = dir.path().join("report.md");
        fs::write(&source, "report").unwrap();
        let store = ArtifactStore::open(root.clone()).unwrap();
        let artifact = store.publish(request(source)).unwrap();

        for path in [
            root,
            store.root.join(artifact.artifact_id.to_string()),
            store.version_path(artifact.artifact_id, artifact.version),
        ] {
            assert_eq!(
                fs::metadata(path).unwrap().permissions().mode() & 0o777,
                0o700
            );
        }
    }

    #[test]
    fn active_content_preview_is_conservative() {
        assert_eq!(preview_kind("text/html"), ArtifactPreviewKind::Html);
        assert_eq!(preview_kind("image/png"), ArtifactPreviewKind::Image);
        assert_eq!(preview_kind("image/svg+xml"), ArtifactPreviewKind::None);
        assert_eq!(preview_kind("application/zip"), ArtifactPreviewKind::None);
    }
}
