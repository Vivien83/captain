//! Kernel-owned artifact authority.
//!
//! Runtime tools never open the managed artifact directory directly. The
//! kernel binds writes and reads to a registered caller, adds the active
//! session identity, and audits completed publications without recording user
//! content or filesystem paths.

use std::path::PathBuf;
use std::sync::Arc;

use captain_memory::artifacts::PublishArtifactRequest;
use captain_runtime::audit::AuditAction;
use captain_types::agent::{AgentEntry, AgentId};
use captain_types::artifact::{ArtifactInventory, ArtifactStoreStatus, ArtifactVersion};
use uuid::Uuid;

use super::CaptainKernel;

impl CaptainKernel {
    fn artifact_caller(&self, caller_agent_id: &str) -> Result<AgentEntry, String> {
        let agent_id = caller_agent_id
            .parse::<AgentId>()
            .map_err(|_| "artifact caller_agent_id is invalid".to_string())?;
        self.registry
            .get(agent_id)
            .ok_or_else(|| "artifact caller is not registered".to_string())
    }

    pub(super) async fn handle_artifact_publish(
        &self,
        caller_agent_id: &str,
        mut request: PublishArtifactRequest,
    ) -> Result<ArtifactVersion, String> {
        let caller = self.artifact_caller(caller_agent_id)?;
        request.agent_id = caller.id.to_string();
        request.session_id = Some(caller.session_id.to_string());

        let store = Arc::clone(&self.artifact_store);
        let artifact = tokio::task::spawn_blocking(move || store.publish(request))
            .await
            .map_err(|error| format!("artifact publish worker failed: {error}"))??;
        self.audit_log.record_or_alert(
            caller.id.to_string(),
            AuditAction::FileAccess,
            format!(
                "artifact_publish id={} version={} sha256={}",
                artifact.artifact_id, artifact.version, artifact.sha256
            ),
            "ok",
        );
        Ok(artifact)
    }

    pub(super) async fn handle_artifact_list(
        &self,
        caller_agent_id: &str,
        limit: usize,
    ) -> Result<Vec<ArtifactVersion>, String> {
        let caller = self.artifact_caller(caller_agent_id)?;
        let caller_id = caller.id.to_string();
        let store = Arc::clone(&self.artifact_store);
        tokio::task::spawn_blocking(move || store.list(Some(&caller_id), limit).map(|v| v.items))
            .await
            .map_err(|error| format!("artifact list worker failed: {error}"))?
    }

    pub(super) async fn handle_artifact_inspect(
        &self,
        caller_agent_id: &str,
        artifact_id: Uuid,
        version: Option<u32>,
    ) -> Result<ArtifactVersion, String> {
        let caller = self.artifact_caller(caller_agent_id)?;
        let caller_id = caller.id.to_string();
        let store = Arc::clone(&self.artifact_store);
        tokio::task::spawn_blocking(move || store.inspect_owned(&caller_id, artifact_id, version))
            .await
            .map_err(|error| format!("artifact inspect worker failed: {error}"))?
    }

    pub(super) async fn handle_artifact_payload(
        &self,
        caller_agent_id: &str,
        artifact_id: Uuid,
        version: Option<u32>,
    ) -> Result<(ArtifactVersion, PathBuf), String> {
        let caller = self.artifact_caller(caller_agent_id)?;
        let caller_id = caller.id.to_string();
        let store = Arc::clone(&self.artifact_store);
        tokio::task::spawn_blocking(move || {
            store.verified_payload_path_owned(&caller_id, artifact_id, version)
        })
        .await
        .map_err(|error| format!("artifact payload worker failed: {error}"))?
    }

    /// Trusted operator inventory used by authenticated daemon surfaces.
    pub async fn operator_artifact_inventory(
        &self,
        limit: usize,
    ) -> Result<ArtifactInventory, String> {
        let store = Arc::clone(&self.artifact_store);
        tokio::task::spawn_blocking(move || store.list(None, limit))
            .await
            .map_err(|error| format!("artifact inventory worker failed: {error}"))?
    }

    pub async fn operator_artifact_versions(
        &self,
        artifact_id: Uuid,
    ) -> Result<Vec<ArtifactVersion>, String> {
        let store = Arc::clone(&self.artifact_store);
        tokio::task::spawn_blocking(move || store.list_versions(artifact_id))
            .await
            .map_err(|error| format!("artifact versions worker failed: {error}"))?
    }

    pub async fn operator_artifact_inspect(
        &self,
        artifact_id: Uuid,
        version: Option<u32>,
    ) -> Result<ArtifactVersion, String> {
        let store = Arc::clone(&self.artifact_store);
        tokio::task::spawn_blocking(move || store.inspect(artifact_id, version))
            .await
            .map_err(|error| format!("artifact inspect worker failed: {error}"))?
    }

    pub async fn operator_artifact_read(
        &self,
        artifact_id: Uuid,
        version: Option<u32>,
    ) -> Result<(ArtifactVersion, Vec<u8>), String> {
        let store = Arc::clone(&self.artifact_store);
        tokio::task::spawn_blocking(move || store.read_verified_payload(artifact_id, version))
            .await
            .map_err(|error| format!("artifact read worker failed: {error}"))?
    }

    pub async fn operator_artifact_status(&self) -> Result<ArtifactStoreStatus, String> {
        let store = Arc::clone(&self.artifact_store);
        tokio::task::spawn_blocking(move || store.status())
            .await
            .map_err(|error| format!("artifact status worker failed: {error}"))?
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use captain_runtime::kernel_handle::KernelHandle;
    use captain_types::config::{DefaultModelConfig, KernelConfig};
    use tempfile::TempDir;

    fn test_kernel(temp: &TempDir) -> CaptainKernel {
        CaptainKernel::boot_with_config(KernelConfig {
            home_dir: temp.path().join("home"),
            data_dir: temp.path().join("data"),
            default_model: DefaultModelConfig {
                provider: "ollama".to_string(),
                model: "test-model".to_string(),
                api_key_env: "OLLAMA_API_KEY".to_string(),
                base_url: None,
            },
            ..KernelConfig::default()
        })
        .expect("test kernel boot")
    }

    fn principal(kernel: &CaptainKernel) -> AgentEntry {
        kernel
            .registry
            .list()
            .into_iter()
            .find(|entry| entry.name == super::super::PRINCIPAL_AGENT_NAME)
            .expect("principal Captain agent")
    }

    #[tokio::test]
    async fn kernel_binds_artifact_to_real_agent_and_session() {
        let temp = TempDir::new().unwrap();
        let kernel = test_kernel(&temp);
        let caller = principal(&kernel);
        let source = temp.path().join("report.md");
        std::fs::write(&source, "# Verified report\n").unwrap();

        let artifact = KernelHandle::artifact_publish(
            &kernel,
            &caller.id.to_string(),
            PublishArtifactRequest {
                artifact_id: None,
                agent_id: "forged-agent".to_string(),
                session_id: Some("forged-session".to_string()),
                title: "Verified report".to_string(),
                filename: "report.md".to_string(),
                mime_type: "text/markdown".to_string(),
                summary: Some("Durable output".to_string()),
                source_path: source,
                expected_sha256: None,
            },
        )
        .await
        .unwrap();

        assert_eq!(artifact.agent_id, caller.id.to_string());
        let expected_session_id = caller.session_id.to_string();
        assert_eq!(
            artifact.session_id.as_deref(),
            Some(expected_session_id.as_str())
        );
        let listed = KernelHandle::artifact_list(&kernel, &caller.id.to_string(), 10)
            .await
            .unwrap();
        assert_eq!(listed, vec![artifact.clone()]);
        let inspected = KernelHandle::artifact_inspect(
            &kernel,
            &caller.id.to_string(),
            artifact.artifact_id,
            Some(artifact.version),
        )
        .await
        .unwrap();
        assert_eq!(inspected, artifact);
        let operator_inventory = kernel.operator_artifact_inventory(20).await.unwrap();
        assert_eq!(operator_inventory.items, vec![artifact.clone()]);
        assert_eq!(operator_inventory.status.artifacts, 1);
        assert_eq!(
            kernel
                .operator_artifact_versions(artifact.artifact_id)
                .await
                .unwrap(),
            vec![artifact.clone()]
        );
        let (operator_artifact, operator_bytes) = kernel
            .operator_artifact_read(artifact.artifact_id, Some(artifact.version))
            .await
            .unwrap();
        assert_eq!(operator_artifact, artifact);
        assert_eq!(operator_bytes, b"# Verified report\n");
        let audit = kernel
            .audit_log
            .recent(10)
            .into_iter()
            .find(|entry| entry.detail.starts_with("artifact_publish id="))
            .expect("artifact publication audit entry");
        assert_eq!(audit.action, AuditAction::FileAccess);
        assert!(audit.detail.contains(&artifact.sha256));
        assert!(!audit.detail.contains("Verified report"));
        assert!(!audit.detail.contains("report.md"));
    }

    #[tokio::test]
    async fn kernel_rejects_unregistered_artifact_caller() {
        let temp = TempDir::new().unwrap();
        let kernel = test_kernel(&temp);
        let error = KernelHandle::artifact_list(&kernel, &Uuid::new_v4().to_string(), 10)
            .await
            .unwrap_err();
        assert_eq!(error, "artifact caller is not registered");
    }

    #[tokio::test]
    async fn ownership_is_checked_before_foreign_payload_verification() {
        let temp = TempDir::new().unwrap();
        let kernel = test_kernel(&temp);
        let caller = principal(&kernel);
        let source = temp.path().join("private.txt");
        std::fs::write(&source, "owner-only").unwrap();
        let foreign_id = Uuid::new_v4().to_string();
        let artifact = kernel
            .artifact_store
            .publish(PublishArtifactRequest {
                artifact_id: None,
                agent_id: foreign_id,
                session_id: None,
                title: "Private output".to_string(),
                filename: "private.txt".to_string(),
                mime_type: "text/plain".to_string(),
                summary: None,
                source_path: source,
                expected_sha256: None,
            })
            .unwrap();
        let (_, payload) = kernel
            .artifact_store
            .verified_payload_path(artifact.artifact_id, Some(artifact.version))
            .unwrap();
        std::fs::write(payload, "wrong-only").unwrap();

        let error = KernelHandle::artifact_inspect(
            &kernel,
            &caller.id.to_string(),
            artifact.artifact_id,
            Some(artifact.version),
        )
        .await
        .unwrap_err();
        assert_eq!(error, "artifact is unavailable to the calling agent");
    }

    #[test]
    fn boot_fails_when_artifact_store_cannot_be_opened() {
        let temp = TempDir::new().unwrap();
        let data = temp.path().join("data");
        std::fs::create_dir_all(&data).unwrap();
        std::fs::write(data.join("artifacts"), b"not a directory").unwrap();
        let error = match CaptainKernel::boot_with_config(KernelConfig {
            home_dir: temp.path().join("home"),
            data_dir: data,
            default_model: DefaultModelConfig {
                provider: "ollama".to_string(),
                model: "test-model".to_string(),
                api_key_env: "OLLAMA_API_KEY".to_string(),
                base_url: None,
            },
            ..KernelConfig::default()
        }) {
            Ok(_) => panic!("boot should reject an unusable artifact store"),
            Err(error) => error,
        };
        assert!(error.to_string().contains("Artifact store unavailable"));
    }
}
