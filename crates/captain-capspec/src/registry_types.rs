use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, thiserror::Error)]
pub enum RegistryError {
    #[error("CapSpec filesystem error: {0}")]
    Io(#[from] std::io::Error),
    #[error("CapSpec database error: {0}")]
    Sqlite(#[from] rusqlite::Error),
    #[error("CapSpec serialization error: {0}")]
    Json(#[from] serde_json::Error),
    #[error("CapSpec registry lock was poisoned")]
    Poisoned,
    #[error("unknown CapSpec scope '{0}'")]
    UnknownScope(String),
    #[error("CapSpec '{name}' was not found in scope '{scope}'")]
    CapabilityNotFound { scope: String, name: String },
    #[error("CapSpec revision '{source_hash}' for '{name}' was not found in scope '{scope}'")]
    RevisionNotFound {
        scope: String,
        name: String,
        source_hash: String,
    },
    #[error("CapSpec '{name}' expects pending hash '{expected}', not '{actual}'")]
    PendingHashMismatch {
        name: String,
        expected: String,
        actual: String,
    },
    #[error("invalid CapSpec source path '{0}'")]
    InvalidSourcePath(String),
    #[error("invalid persisted CapSpec state: {0}")]
    InvalidPersistedState(String),
    #[error("CapSpec approval actor must not be empty")]
    EmptyActor,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(tag = "kind", content = "workspace", rename_all = "snake_case")]
pub enum CapabilityScope {
    Global,
    Project(PathBuf),
}

impl CapabilityScope {
    pub fn key(&self) -> String {
        match self {
            Self::Global => "global".to_string(),
            Self::Project(workspace) => format!("project:{}", workspace.display()),
        }
    }

    pub fn label(&self) -> String {
        match self {
            Self::Global => "global".to_string(),
            Self::Project(workspace) => format!("project ({})", workspace.display()),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CapabilityStatus {
    Operational,
    PendingApproval,
    UpdatePendingApproval,
    Invalid,
    InvalidUpdateRetained,
    Disabled,
    Rejected,
    UpdateRejected,
}

impl CapabilityStatus {
    pub(crate) fn as_storage(self) -> &'static str {
        match self {
            Self::Operational => "operational",
            Self::PendingApproval => "pending_approval",
            Self::UpdatePendingApproval => "update_pending_approval",
            Self::Invalid => "invalid",
            Self::InvalidUpdateRetained => "invalid_update_retained",
            Self::Disabled => "disabled",
            Self::Rejected => "rejected",
            Self::UpdateRejected => "update_rejected",
        }
    }

    pub(crate) fn from_storage(value: &str) -> Result<Self, RegistryError> {
        match value {
            "operational" => Ok(Self::Operational),
            "pending_approval" => Ok(Self::PendingApproval),
            "update_pending_approval" => Ok(Self::UpdatePendingApproval),
            "invalid" => Ok(Self::Invalid),
            "invalid_update_retained" => Ok(Self::InvalidUpdateRetained),
            "disabled" => Ok(Self::Disabled),
            "rejected" => Ok(Self::Rejected),
            "update_rejected" => Ok(Self::UpdateRejected),
            other => Err(RegistryError::InvalidPersistedState(format!(
                "unknown persisted status '{other}'"
            ))),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CapabilityView {
    pub scope: CapabilityScope,
    pub name: String,
    pub tool_name: Option<String>,
    pub source_path: PathBuf,
    pub status: CapabilityStatus,
    pub active_hash: Option<String>,
    pub pending_hash: Option<String>,
    pub last_error: Option<String>,
    pub updated_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RevisionInfo {
    pub scope: CapabilityScope,
    pub name: String,
    pub source_hash: String,
    pub version: String,
    pub permission_fingerprint: String,
    pub created_at: String,
    pub approved_by: Option<String>,
    pub approved_at: Option<String>,
    pub rejected_by: Option<String>,
    pub rejected_at: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ReloadReport {
    pub discovered: usize,
    pub activated: usize,
    pub pending_approval: usize,
    pub retained: usize,
    pub disabled: usize,
    pub issues: Vec<ReloadIssue>,
}

impl ReloadReport {
    pub(crate) fn merge(&mut self, other: Self) {
        self.discovered += other.discovered;
        self.activated += other.activated;
        self.pending_approval += other.pending_approval;
        self.retained += other.retained;
        self.disabled += other.disabled;
        self.issues.extend(other.issues);
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReloadIssue {
    pub source_path: PathBuf,
    pub message: String,
    pub retained_active_revision: bool,
}
