//! Durable user-facing artifact metadata shared across Captain surfaces.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Browser representation allowed for an immutable artifact version.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ArtifactPreviewKind {
    Text,
    Markdown,
    Html,
    Image,
    Pdf,
    None,
}

/// Immutable metadata stored beside one artifact payload.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ArtifactVersion {
    pub artifact_id: Uuid,
    pub version: u32,
    pub agent_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    pub title: String,
    pub filename: String,
    pub mime_type: String,
    pub preview_kind: ArtifactPreviewKind,
    pub size_bytes: u64,
    pub sha256: String,
    pub created_at: DateTime<Utc>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub summary: Option<String>,
}

/// Public-safe inventory health. Corrupt entries are counted, not hidden.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ArtifactStoreStatus {
    pub healthy: bool,
    pub artifacts: usize,
    pub versions: usize,
    pub bytes: u64,
    pub invalid_entries: usize,
    pub recovered_staging_entries: usize,
    pub max_artifact_bytes: u64,
    pub max_total_bytes: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ArtifactInventory {
    pub items: Vec<ArtifactVersion>,
    pub status: ArtifactStoreStatus,
}
