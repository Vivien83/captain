//! Background reads for the immutable artifact operator surface.

use super::event::{AppEvent, BackendRef};
use captain_types::artifact::{ArtifactInventory, ArtifactVersion};
use std::sync::mpsc;
use std::time::Duration;
use uuid::Uuid;

const INVENTORY_LIMIT: usize = 100;

pub fn spawn_fetch_inventory(backend: BackendRef, tx: mpsc::Sender<AppEvent>) {
    std::thread::spawn(move || {
        let result = match backend {
            BackendRef::Daemon(base_url) => fetch_daemon_inventory(&base_url),
            BackendRef::InProcess(kernel) => runtime().and_then(|runtime| {
                runtime.block_on(kernel.operator_artifact_inventory(INVENTORY_LIMIT))
            }),
        };
        let _ = tx.send(AppEvent::ArtifactsLoaded(result));
    });
}

pub fn spawn_fetch_versions(backend: BackendRef, artifact_id: Uuid, tx: mpsc::Sender<AppEvent>) {
    std::thread::spawn(move || {
        let result = match backend {
            BackendRef::Daemon(base_url) => fetch_daemon_versions(&base_url, artifact_id),
            BackendRef::InProcess(kernel) => runtime().and_then(|runtime| {
                runtime.block_on(kernel.operator_artifact_versions(artifact_id))
            }),
        };
        let _ = tx.send(AppEvent::ArtifactVersionsLoaded {
            artifact_id,
            result,
        });
    });
}

fn fetch_daemon_inventory(base_url: &str) -> Result<ArtifactInventory, String> {
    let response = daemon_client()
        .get(format!("{base_url}/api/artifacts"))
        .query(&[("limit", INVENTORY_LIMIT)])
        .send()
        .map_err(|error| format!("Artifact inventory unavailable: {error}"))?;
    decode_response(response, "Artifact inventory")
}

fn fetch_daemon_versions(
    base_url: &str,
    artifact_id: Uuid,
) -> Result<Vec<ArtifactVersion>, String> {
    let response = daemon_client()
        .get(format!("{base_url}/api/artifacts/{artifact_id}/versions"))
        .send()
        .map_err(|error| format!("Artifact versions unavailable: {error}"))?;
    let body: serde_json::Value = decode_response(response, "Artifact versions")?;
    serde_json::from_value(
        body.get("items")
            .cloned()
            .unwrap_or_else(|| serde_json::Value::Array(Vec::new())),
    )
    .map_err(|error| format!("Artifact versions response invalid: {error}"))
}

fn daemon_client() -> reqwest::blocking::Client {
    reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(5))
        .default_headers(crate::daemon_auth_headers())
        .build()
        .unwrap_or_else(|_| reqwest::blocking::Client::new())
}

fn decode_response<T: serde::de::DeserializeOwned>(
    response: reqwest::blocking::Response,
    label: &str,
) -> Result<T, String> {
    if !response.status().is_success() {
        return Err(format!("{label} failed: HTTP {}", response.status()));
    }
    response
        .json::<T>()
        .map_err(|error| format!("{label} response invalid: {error}"))
}

fn runtime() -> Result<tokio::runtime::Runtime, String> {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|error| format!("Artifact runtime unavailable: {error}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use captain_types::artifact::{ArtifactPreviewKind, ArtifactStoreStatus};
    use chrono::Utc;

    #[test]
    fn typed_inventory_response_rejects_unknown_artifact_fields() {
        let payload = serde_json::json!({
            "items": [{
                "artifact_id": Uuid::nil(),
                "version": 1,
                "agent_id": "captain",
                "title": "Report",
                "filename": "report.md",
                "mime_type": "text/markdown",
                "preview_kind": ArtifactPreviewKind::Markdown,
                "size_bytes": 12,
                "sha256": "a".repeat(64),
                "created_at": Utc::now(),
                "unexpected": "must fail"
            }],
            "status": ArtifactStoreStatus {
                healthy: true,
                artifacts: 1,
                versions: 1,
                bytes: 12,
                invalid_entries: 0,
                recovered_staging_entries: 0,
                max_artifact_bytes: 50 * 1024 * 1024,
                max_total_bytes: 512 * 1024 * 1024,
            }
        });
        assert!(serde_json::from_value::<ArtifactInventory>(payload).is_err());
    }
}
