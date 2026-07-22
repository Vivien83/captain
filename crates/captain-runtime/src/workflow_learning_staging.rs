//! Crash-safe, inactive staging for workflow-learning drafts.
//!
//! The root is deliberately outside every active Skill and CapSpec source
//! tree. A revision directory is immutable and becomes complete only when its
//! manifest has been durably written after the artifact bytes.

use std::fs;
use std::path::{Component, Path, PathBuf};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::workflow_learning_proposer::{
    validate_draft, ActiveModelIdentity, RefinementTargetKind, WorkflowDraft,
    WorkflowDraftArtifact, WorkflowDraftKind,
};

pub const STAGED_DRAFT_MANIFEST_VERSION: u16 = 1;

#[derive(Debug, Clone)]
pub struct WorkflowStagingRoot {
    captain_home: PathBuf,
    root: PathBuf,
}

#[derive(Debug, Clone)]
pub struct StageWorkflowDraftRequest<'a> {
    pub job_id: &'a str,
    pub workflow_signature: &'a str,
    pub draft: &'a WorkflowDraft,
    pub active_model: &'a ActiveModelIdentity,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StagedWorkflowDraft {
    pub manifest_version: u16,
    pub job_id: String,
    pub workflow_signature: String,
    pub kind: WorkflowDraftKind,
    pub name: String,
    pub model: ActiveModelIdentity,
    pub revision_sha256: String,
    pub artifact_sha256: String,
    pub artifact_file: String,
    pub draft: WorkflowDraft,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StagedWorkflowDraftReceipt {
    pub revision_sha256: String,
    pub artifact_sha256: String,
    pub revision_dir: PathBuf,
    pub artifact_path: PathBuf,
    pub manifest_path: PathBuf,
}

#[derive(Debug, Clone)]
pub struct LoadedStagedWorkflowDraft {
    pub manifest: StagedWorkflowDraft,
    pub revision_dir: PathBuf,
    pub artifact_path: PathBuf,
    pub artifact_bytes: Vec<u8>,
    pub manifest_path: PathBuf,
}

#[derive(Debug, thiserror::Error)]
pub enum WorkflowStagingError {
    #[error("invalid staging request: {0}")]
    InvalidRequest(String),
    #[error("draft validation failed: {0}")]
    InvalidDraft(String),
    #[error("unsafe staging filesystem: {0}")]
    UnsafeFilesystem(String),
    #[error("immutable staging conflict at {0}")]
    ImmutableConflict(String),
    #[error("staging I/O failed: {0}")]
    Io(#[from] std::io::Error),
    #[error("staging serialization failed: {0}")]
    Serialization(#[from] serde_json::Error),
}

impl WorkflowStagingRoot {
    pub fn new(captain_home: impl Into<PathBuf>) -> Result<Self, WorkflowStagingError> {
        let captain_home = captain_home.into();
        if !captain_home.is_absolute() {
            return Err(WorkflowStagingError::InvalidRequest(
                "Captain home must be absolute".to_string(),
            ));
        }
        let root = captain_home.join("learning").join("staging");
        Ok(Self { captain_home, root })
    }

    pub fn path(&self) -> &Path {
        &self.root
    }

    pub fn active_roots(&self) -> [PathBuf; 2] {
        [
            self.captain_home.join("skills"),
            self.captain_home.join("capabilities"),
        ]
    }

    pub fn stage(
        &self,
        request: StageWorkflowDraftRequest<'_>,
    ) -> Result<StagedWorkflowDraftReceipt, WorkflowStagingError> {
        validate_identifier("job_id", request.job_id, 96)?;
        validate_hash("workflow_signature", request.workflow_signature)?;
        validate_draft(request.draft, request.draft.kind)
            .map_err(|error| WorkflowStagingError::InvalidDraft(error.to_string()))?;
        self.ensure_inactive_root()?;

        let (artifact_file, artifact_bytes) = artifact_file_and_bytes(request.draft)?;
        let artifact_sha256 = sha256_hex(&artifact_bytes);
        let revision_seed = serde_json::to_vec(&RevisionSeed {
            workflow_signature: request.workflow_signature,
            model: request.active_model,
            draft: request.draft,
        })?;
        let revision_sha256 = sha256_hex(&revision_seed);
        let revision_dir = self.root.join(request.job_id).join(&revision_sha256);
        ensure_no_symlink_descendant(&self.root, &revision_dir)?;
        captain_types::durable_fs::create_dir_all(&revision_dir)?;
        make_private_directory(&revision_dir)?;

        let artifact_path = revision_dir.join(&artifact_file);
        write_immutable(&artifact_path, &artifact_bytes)?;
        let manifest = StagedWorkflowDraft {
            manifest_version: STAGED_DRAFT_MANIFEST_VERSION,
            job_id: request.job_id.to_string(),
            workflow_signature: request.workflow_signature.to_string(),
            kind: request.draft.kind,
            name: request.draft.name.clone(),
            model: request.active_model.clone(),
            revision_sha256: revision_sha256.clone(),
            artifact_sha256: artifact_sha256.clone(),
            artifact_file: artifact_file.clone(),
            draft: request.draft.clone(),
        };
        let manifest_bytes = serde_json::to_vec_pretty(&manifest)?;
        let manifest_path = revision_dir.join("draft.json");
        write_immutable(&manifest_path, &manifest_bytes)?;

        Ok(StagedWorkflowDraftReceipt {
            revision_sha256,
            artifact_sha256,
            revision_dir,
            artifact_path,
            manifest_path,
        })
    }

    /// Load one exact immutable revision and verify every hash and byte before
    /// it can cross into an active registry.
    pub fn load_exact(
        &self,
        job_id: &str,
        revision_sha256: &str,
    ) -> Result<LoadedStagedWorkflowDraft, WorkflowStagingError> {
        validate_identifier("job_id", job_id, 96)?;
        validate_hash("revision_sha256", revision_sha256)?;
        self.ensure_inactive_root()?;

        let revision_dir = self.root.join(job_id).join(revision_sha256);
        ensure_no_symlink_descendant(&self.root, &revision_dir)?;
        let manifest_path = revision_dir.join("draft.json");
        ensure_regular_file(&manifest_path)?;
        let manifest_bytes = fs::read(&manifest_path)?;
        let manifest: StagedWorkflowDraft = serde_json::from_slice(&manifest_bytes)?;
        validate_loaded_manifest(&manifest, job_id, revision_sha256)?;

        let revision_seed = serde_json::to_vec(&RevisionSeed {
            workflow_signature: &manifest.workflow_signature,
            model: &manifest.model,
            draft: &manifest.draft,
        })?;
        if sha256_hex(&revision_seed) != manifest.revision_sha256 {
            return Err(WorkflowStagingError::ImmutableConflict(
                "staged revision digest does not match its manifest".to_string(),
            ));
        }

        let (expected_file, expected_bytes) = artifact_file_and_bytes(&manifest.draft)?;
        if manifest.artifact_file != expected_file
            || manifest.artifact_sha256 != sha256_hex(&expected_bytes)
        {
            return Err(WorkflowStagingError::ImmutableConflict(
                "staged artifact metadata does not match the validated draft".to_string(),
            ));
        }
        let artifact_path = revision_dir.join(&expected_file);
        ensure_regular_file(&artifact_path)?;
        let artifact_bytes = fs::read(&artifact_path)?;
        if artifact_bytes != expected_bytes
            || sha256_hex(&artifact_bytes) != manifest.artifact_sha256
        {
            return Err(WorkflowStagingError::ImmutableConflict(
                "staged artifact bytes were modified".to_string(),
            ));
        }

        Ok(LoadedStagedWorkflowDraft {
            manifest,
            revision_dir,
            artifact_path,
            artifact_bytes,
            manifest_path,
        })
    }

    /// Recover the one complete immutable revision produced by a draft job.
    /// Incomplete directories are ignored because the manifest is written
    /// last; multiple complete revisions are ambiguous and require review.
    pub fn recover_job(
        &self,
        job_id: &str,
    ) -> Result<Option<LoadedStagedWorkflowDraft>, WorkflowStagingError> {
        validate_identifier("job_id", job_id, 96)?;
        self.ensure_inactive_root()?;
        let job_dir = self.root.join(job_id);
        ensure_no_symlink_descendant(&self.root, &job_dir)?;
        let entries = match fs::read_dir(&job_dir) {
            Ok(entries) => entries,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(error.into()),
        };
        let mut revisions = Vec::new();
        for entry in entries {
            let entry = entry?;
            let revision = entry.file_name().into_string().map_err(|_| {
                WorkflowStagingError::UnsafeFilesystem(
                    "staging revision name is not valid UTF-8".to_string(),
                )
            })?;
            validate_hash("staged revision directory", &revision)?;
            let revision_dir = job_dir.join(&revision);
            ensure_no_symlink_descendant(&self.root, &revision_dir)?;
            let manifest_path = revision_dir.join("draft.json");
            match fs::symlink_metadata(&manifest_path) {
                Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_file() => {
                    return Err(WorkflowStagingError::UnsafeFilesystem(format!(
                        "{} is not a regular file",
                        manifest_path.display()
                    )))
                }
                Ok(_) => revisions.push(revision),
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => return Err(error.into()),
            }
        }
        revisions.sort();
        match revisions.as_slice() {
            [] => Ok(None),
            [revision] => self.load_exact(job_id, revision).map(Some),
            _ => Err(WorkflowStagingError::ImmutableConflict(format!(
                "draft job {job_id} has multiple complete revisions"
            ))),
        }
    }

    fn ensure_inactive_root(&self) -> Result<(), WorkflowStagingError> {
        for active in self.active_roots() {
            if self.root.starts_with(&active) || active.starts_with(&self.root) {
                return Err(WorkflowStagingError::UnsafeFilesystem(format!(
                    "staging root overlaps active root {}",
                    active.display()
                )));
            }
        }
        ensure_existing_component_is_not_symlink(&self.captain_home)?;
        let learning = self.captain_home.join("learning");
        ensure_existing_component_is_not_symlink(&learning)?;
        ensure_existing_component_is_not_symlink(&self.root)?;
        captain_types::durable_fs::create_dir_all(&self.root)?;
        ensure_existing_component_is_not_symlink(&self.root)?;
        make_private_directory(&learning)?;
        make_private_directory(&self.root)
    }
}

#[derive(Serialize)]
struct RevisionSeed<'a> {
    workflow_signature: &'a str,
    model: &'a ActiveModelIdentity,
    draft: &'a WorkflowDraft,
}

fn artifact_file_and_bytes(
    draft: &WorkflowDraft,
) -> Result<(String, Vec<u8>), WorkflowStagingError> {
    match &draft.artifact {
        WorkflowDraftArtifact::SkillMarkdown { source } => {
            Ok(("SKILL.md".to_string(), source.as_bytes().to_vec()))
        }
        WorkflowDraftArtifact::CapspecToml { source } => Ok((
            format!("{}.captain", draft.name),
            source.as_bytes().to_vec(),
        )),
        WorkflowDraftArtifact::Automation {
            schedule,
            instruction,
        } => Ok((
            "automation.json".to_string(),
            serde_json::to_vec_pretty(&serde_json::json!({
                "schema_version": 1,
                "name": draft.name,
                "enabled": false,
                "schedule": schedule,
                "instruction": instruction,
            }))?,
        )),
        WorkflowDraftArtifact::Refinement {
            target_kind,
            target_name,
            source,
        } => {
            let file = match target_kind {
                RefinementTargetKind::Skill => "SKILL.md".to_string(),
                RefinementTargetKind::Capspec => format!("{target_name}.captain"),
            };
            Ok((file, source.as_bytes().to_vec()))
        }
    }
}

fn validate_loaded_manifest(
    manifest: &StagedWorkflowDraft,
    expected_job_id: &str,
    expected_revision_sha256: &str,
) -> Result<(), WorkflowStagingError> {
    if manifest.manifest_version != STAGED_DRAFT_MANIFEST_VERSION
        || manifest.job_id != expected_job_id
        || manifest.revision_sha256 != expected_revision_sha256
        || manifest.kind != manifest.draft.kind
        || manifest.name != manifest.draft.name
    {
        return Err(WorkflowStagingError::ImmutableConflict(
            "staged manifest identity does not match the requested revision".to_string(),
        ));
    }
    validate_hash("workflow_signature", &manifest.workflow_signature)?;
    validate_hash("artifact_sha256", &manifest.artifact_sha256)?;
    validate_draft(&manifest.draft, manifest.kind)
        .map_err(|error| WorkflowStagingError::InvalidDraft(error.to_string()))
}

fn validate_identifier(label: &str, value: &str, max: usize) -> Result<(), WorkflowStagingError> {
    let valid = !value.is_empty()
        && value.len() <= max
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'));
    if valid {
        Ok(())
    } else {
        Err(WorkflowStagingError::InvalidRequest(format!(
            "{label} is not a safe identifier"
        )))
    }
}

fn validate_hash(label: &str, value: &str) -> Result<(), WorkflowStagingError> {
    if value.len() == 64 && value.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        Ok(())
    } else {
        Err(WorkflowStagingError::InvalidRequest(format!(
            "{label} must be a 64-character hex digest"
        )))
    }
}

fn ensure_existing_component_is_not_symlink(path: &Path) -> Result<(), WorkflowStagingError> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_symlink() => Err(
            WorkflowStagingError::UnsafeFilesystem(format!("{} is a symlink", path.display())),
        ),
        Ok(metadata) if !metadata.is_dir() => Err(WorkflowStagingError::UnsafeFilesystem(format!(
            "{} is not a directory",
            path.display()
        ))),
        Ok(_) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error.into()),
    }
}

fn ensure_regular_file(path: &Path) -> Result<(), WorkflowStagingError> {
    let metadata = fs::symlink_metadata(path)?;
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return Err(WorkflowStagingError::UnsafeFilesystem(format!(
            "{} is not a regular file",
            path.display()
        )));
    }
    Ok(())
}

fn ensure_no_symlink_descendant(root: &Path, target: &Path) -> Result<(), WorkflowStagingError> {
    let relative = target.strip_prefix(root).map_err(|_| {
        WorkflowStagingError::UnsafeFilesystem("target escapes staging root".to_string())
    })?;
    let mut current = root.to_path_buf();
    for component in relative.components() {
        let Component::Normal(component) = component else {
            return Err(WorkflowStagingError::UnsafeFilesystem(
                "non-normal staging path component".to_string(),
            ));
        };
        current.push(component);
        ensure_existing_component_is_not_symlink(&current)?;
    }
    Ok(())
}

fn write_immutable(path: &Path, bytes: &[u8]) -> Result<(), WorkflowStagingError> {
    match fs::read(path) {
        Ok(existing) if existing == bytes => return Ok(()),
        Ok(_) => {
            return Err(WorkflowStagingError::ImmutableConflict(
                path.display().to_string(),
            ))
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(error.into()),
    }
    if captain_types::durable_fs::create_new(path, bytes)? {
        Ok(())
    } else if fs::read(path)? == bytes {
        Ok(())
    } else {
        Err(WorkflowStagingError::ImmutableConflict(
            path.display().to_string(),
        ))
    }
}

fn sha256_hex(bytes: &[u8]) -> String {
    hex::encode(Sha256::digest(bytes))
}

#[cfg(unix)]
fn make_private_directory(path: &Path) -> Result<(), WorkflowStagingError> {
    use std::os::unix::fs::PermissionsExt;

    if path.exists() {
        fs::set_permissions(path, fs::Permissions::from_mode(0o700))?;
    }
    Ok(())
}

#[cfg(not(unix))]
fn make_private_directory(_path: &Path) -> Result<(), WorkflowStagingError> {
    Ok(())
}
