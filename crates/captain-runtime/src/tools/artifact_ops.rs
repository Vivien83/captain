use std::path::Path;
use std::sync::Arc;

use captain_memory::artifacts::{
    mime_type_for_filename, PublishArtifactRequest, MAX_ARTIFACT_BYTES,
};
use captain_types::artifact::ArtifactPreviewKind;
use sha2::{Digest, Sha256};
use uuid::Uuid;

use crate::kernel_handle::KernelHandle;

use super::channel_policy::ensure_active_channel;
use super::{ensure_no_secret_literal, require_kernel, resolve_file_path_for_caller};

pub(crate) async fn tool_artifact_publish(
    input: &serde_json::Value,
    kernel: Option<&Arc<dyn KernelHandle>>,
    workspace_root: Option<&Path>,
    caller_agent_id: Option<&str>,
) -> Result<String, String> {
    let kh = require_kernel(kernel)?;
    let caller_agent_id = require_caller(caller_agent_id)?;
    let raw_path = required_text(input, "path")?;
    let title = required_text(input, "title")?;
    ensure_no_secret_literal("artifact_publish", "title", title)?;
    let resolved =
        resolve_file_path_for_caller(raw_path, workspace_root, kernel, Some(caller_agent_id))?;
    let metadata = tokio::fs::symlink_metadata(&resolved)
        .await
        .map_err(|error| format!("Cannot inspect artifact source: {error}"))?;
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return Err("artifact_publish path must resolve to a regular non-symlink file".to_string());
    }
    if metadata.len() == 0 {
        return Err("artifact_publish source is empty".to_string());
    }
    if metadata.len() > MAX_ARTIFACT_BYTES {
        return Err(format!(
            "artifact_publish source is too large ({} bytes, max {MAX_ARTIFACT_BYTES})",
            metadata.len()
        ));
    }

    let data = tokio::fs::read(&resolved)
        .await
        .map_err(|error| format!("Cannot read artifact source: {error}"))?;
    if let Ok(text) = std::str::from_utf8(&data) {
        ensure_no_secret_literal("artifact_publish", "file content", text)?;
    }
    let expected_sha256 = format!("{:x}", Sha256::digest(&data));
    drop(data);

    let filename = optional_text(input, "filename")
        .map(str::to_string)
        .unwrap_or_else(|| source_filename(&resolved));
    ensure_no_secret_literal("artifact_publish", "filename", &filename)?;
    let mime_type = optional_text(input, "mime_type")
        .map(str::to_string)
        .unwrap_or_else(|| mime_type_for_filename(&filename).to_string());
    let summary = optional_text(input, "summary").map(str::to_string);
    if let Some(summary) = summary.as_deref() {
        ensure_no_secret_literal("artifact_publish", "summary", summary)?;
    }
    let artifact_id = optional_uuid(input, "artifact_id")?;
    let artifact = kh
        .artifact_publish(
            caller_agent_id,
            PublishArtifactRequest {
                artifact_id,
                agent_id: caller_agent_id.to_string(),
                session_id: None,
                title: title.to_string(),
                filename,
                mime_type,
                summary,
                source_path: resolved,
                expected_sha256: Some(expected_sha256),
            },
        )
        .await?;

    render_json(serde_json::json!({
        "success": true,
        "tool": "artifact_publish",
        "integrity": "sha256_verified",
        "artifact": artifact,
        "next_actions": ["artifact_inspect", "artifact_deliver"]
    }))
}

pub(crate) async fn tool_artifact_list(
    input: &serde_json::Value,
    kernel: Option<&Arc<dyn KernelHandle>>,
    caller_agent_id: Option<&str>,
) -> Result<String, String> {
    let limit = input["limit"].as_u64().unwrap_or(20);
    if !(1..=100).contains(&limit) {
        return Err("artifact_list limit must be between 1 and 100".to_string());
    }
    let items = require_kernel(kernel)?
        .artifact_list(require_caller(caller_agent_id)?, limit as usize)
        .await?;
    render_json(serde_json::json!({
        "success": true,
        "tool": "artifact_list",
        "count": items.len(),
        "items": items
    }))
}

pub(crate) async fn tool_artifact_inspect(
    input: &serde_json::Value,
    kernel: Option<&Arc<dyn KernelHandle>>,
    caller_agent_id: Option<&str>,
) -> Result<String, String> {
    let artifact = require_kernel(kernel)?
        .artifact_inspect(
            require_caller(caller_agent_id)?,
            required_uuid(input, "artifact_id")?,
            optional_version(input)?,
        )
        .await?;
    render_json(serde_json::json!({
        "success": true,
        "tool": "artifact_inspect",
        "integrity": "sha256_verified",
        "artifact": artifact
    }))
}

pub(crate) async fn tool_artifact_deliver(
    input: &serde_json::Value,
    kernel: Option<&Arc<dyn KernelHandle>>,
    caller_agent_id: Option<&str>,
) -> Result<String, String> {
    let kh = require_kernel(kernel)?;
    let caller_agent_id = require_caller(caller_agent_id)?;
    let channel = required_text(input, "channel")?.to_ascii_lowercase();
    ensure_active_channel(&channel)?;
    let recipient = match optional_text(input, "recipient") {
        Some(recipient) => recipient.to_string(),
        None => kh
            .get_channel_default_recipient(&channel)
            .await
            .ok_or_else(|| {
                format!(
                    "artifact_deliver requires recipient because channel '{channel}' has no default"
                )
            })?,
    };
    let thread_id = optional_text(input, "thread_id");
    let (artifact, payload) = kh
        .artifact_payload(
            caller_agent_id,
            required_uuid(input, "artifact_id")?,
            optional_version(input)?,
        )
        .await?;
    let data = tokio::fs::read(&payload)
        .await
        .map_err(|error| format!("Cannot read verified artifact payload: {error}"))?;
    if data.len() as u64 != artifact.size_bytes {
        return Err("verified artifact payload changed before delivery".to_string());
    }
    let delivery_sha256 = format!("{:x}", Sha256::digest(&data));
    if delivery_sha256 != artifact.sha256 {
        return Err("verified artifact payload changed before delivery".to_string());
    }
    if let Ok(text) = std::str::from_utf8(&data) {
        ensure_no_secret_literal("artifact_deliver", "file content", text)?;
    }

    let delivery = if artifact.preview_kind == ArtifactPreviewKind::Image {
        let caption = optional_text(input, "caption").unwrap_or(&artifact.title);
        ensure_no_secret_literal("artifact_deliver", "caption", caption)?;
        kh.send_channel_image_data(
            &channel,
            &recipient,
            data,
            &artifact.mime_type,
            Some(caption),
            thread_id,
        )
        .await?
    } else {
        if optional_text(input, "caption").is_some() {
            return Err(
                "artifact_deliver caption is supported only for image artifacts".to_string(),
            );
        }
        kh.send_channel_file_data(
            &channel,
            &recipient,
            data,
            &artifact.filename,
            &artifact.mime_type,
            thread_id,
        )
        .await?
    };

    render_json(serde_json::json!({
        "success": true,
        "tool": "artifact_deliver",
        "artifact": artifact,
        "channel": channel,
        "recipient": recipient,
        "delivery": delivery
    }))
}

fn require_caller(caller_agent_id: Option<&str>) -> Result<&str, String> {
    caller_agent_id
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| "artifact tools require a registered caller agent".to_string())
}

fn required_text<'a>(input: &'a serde_json::Value, field: &str) -> Result<&'a str, String> {
    optional_text(input, field).ok_or_else(|| format!("Missing non-empty '{field}' parameter"))
}

fn optional_text<'a>(input: &'a serde_json::Value, field: &str) -> Option<&'a str> {
    input[field]
        .as_str()
        .map(str::trim)
        .filter(|value| !value.is_empty())
}

fn source_filename(path: &Path) -> String {
    path.file_name()
        .and_then(|value| value.to_str())
        .unwrap_or("artifact.bin")
        .to_string()
}

fn required_uuid(input: &serde_json::Value, field: &str) -> Result<Uuid, String> {
    let value = required_text(input, field)?;
    Uuid::parse_str(value).map_err(|_| format!("'{field}' must be a valid UUID"))
}

fn optional_uuid(input: &serde_json::Value, field: &str) -> Result<Option<Uuid>, String> {
    optional_text(input, field)
        .map(|value| Uuid::parse_str(value).map_err(|_| format!("'{field}' must be a valid UUID")))
        .transpose()
}

fn optional_version(input: &serde_json::Value) -> Result<Option<u32>, String> {
    let Some(value) = input.get("version").filter(|value| !value.is_null()) else {
        return Ok(None);
    };
    let version = value
        .as_u64()
        .and_then(|value| u32::try_from(value).ok())
        .filter(|value| *value > 0)
        .ok_or_else(|| "'version' must be a positive 32-bit integer".to_string())?;
    Ok(Some(version))
}

fn render_json(value: serde_json::Value) -> Result<String, String> {
    serde_json::to_string_pretty(&value)
        .map_err(|error| format!("Serialize artifact result: {error}"))
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use async_trait::async_trait;
    use captain_types::artifact::ArtifactVersion;
    use chrono::Utc;
    use tempfile::TempDir;

    use super::*;
    use crate::kernel_handle::AgentInfo;

    #[derive(Debug)]
    struct Delivery {
        channel: String,
        recipient: String,
        data: Vec<u8>,
        filename: String,
    }

    #[derive(Default)]
    struct ArtifactKernel {
        published: Mutex<Option<PublishArtifactRequest>>,
        artifact: Mutex<Option<ArtifactVersion>>,
        deliveries: Mutex<Vec<Delivery>>,
    }

    #[async_trait]
    impl KernelHandle for ArtifactKernel {
        async fn spawn_agent(
            &self,
            _manifest_toml: &str,
            _parent_id: Option<&str>,
        ) -> Result<(String, String), String> {
            unreachable!()
        }

        async fn send_to_agent(&self, _agent_id: &str, _message: &str) -> Result<String, String> {
            unreachable!()
        }

        fn list_agents(&self) -> Vec<AgentInfo> {
            Vec::new()
        }

        fn kill_agent(&self, _agent_id: &str) -> Result<(), String> {
            unreachable!()
        }

        fn memory_store(&self, _key: &str, _value: serde_json::Value) -> Result<(), String> {
            unreachable!()
        }

        fn memory_recall(&self, _key: &str) -> Result<Option<serde_json::Value>, String> {
            unreachable!()
        }

        fn find_agents(&self, _query: &str) -> Vec<AgentInfo> {
            Vec::new()
        }

        async fn task_post(
            &self,
            _title: &str,
            _description: &str,
            _assigned_to: Option<&str>,
            _created_by: Option<&str>,
        ) -> Result<String, String> {
            unreachable!()
        }

        async fn task_claim(&self, _agent_id: &str) -> Result<Option<serde_json::Value>, String> {
            unreachable!()
        }

        async fn task_complete(&self, _task_id: &str, _result: &str) -> Result<(), String> {
            unreachable!()
        }

        async fn artifact_publish(
            &self,
            _caller_agent_id: &str,
            request: PublishArtifactRequest,
        ) -> Result<ArtifactVersion, String> {
            let artifact = ArtifactVersion {
                artifact_id: request.artifact_id.unwrap_or_else(Uuid::new_v4),
                version: 1,
                agent_id: request.agent_id.clone(),
                session_id: Some("session-real".to_string()),
                title: request.title.clone(),
                filename: request.filename.clone(),
                mime_type: request.mime_type.clone(),
                preview_kind: captain_memory::artifacts::preview_kind(&request.mime_type),
                size_bytes: std::fs::metadata(&request.source_path).unwrap().len(),
                sha256: request.expected_sha256.clone().unwrap(),
                created_at: Utc::now(),
                summary: request.summary.clone(),
            };
            *self.published.lock().unwrap() = Some(request);
            *self.artifact.lock().unwrap() = Some(artifact.clone());
            Ok(artifact)
        }

        async fn artifact_list(
            &self,
            _caller_agent_id: &str,
            _limit: usize,
        ) -> Result<Vec<ArtifactVersion>, String> {
            Ok(self.artifact.lock().unwrap().clone().into_iter().collect())
        }

        async fn artifact_inspect(
            &self,
            _caller_agent_id: &str,
            artifact_id: Uuid,
            _version: Option<u32>,
        ) -> Result<ArtifactVersion, String> {
            self.artifact
                .lock()
                .unwrap()
                .clone()
                .filter(|artifact| artifact.artifact_id == artifact_id)
                .ok_or_else(|| "missing artifact".to_string())
        }

        async fn artifact_payload(
            &self,
            _caller_agent_id: &str,
            artifact_id: Uuid,
            _version: Option<u32>,
        ) -> Result<(ArtifactVersion, std::path::PathBuf), String> {
            let artifact = self
                .artifact
                .lock()
                .unwrap()
                .clone()
                .filter(|artifact| artifact.artifact_id == artifact_id)
                .ok_or_else(|| "missing artifact".to_string())?;
            let path = self
                .published
                .lock()
                .unwrap()
                .as_ref()
                .unwrap()
                .source_path
                .clone();
            Ok((artifact, path))
        }

        async fn get_channel_default_recipient(&self, _channel: &str) -> Option<String> {
            Some("default-recipient".to_string())
        }

        async fn send_channel_file_data(
            &self,
            channel: &str,
            recipient: &str,
            data: Vec<u8>,
            filename: &str,
            _mime_type: &str,
            _thread_id: Option<&str>,
        ) -> Result<String, String> {
            self.deliveries.lock().unwrap().push(Delivery {
                channel: channel.to_string(),
                recipient: recipient.to_string(),
                data,
                filename: filename.to_string(),
            });
            Ok("delivered".to_string())
        }
    }

    #[tokio::test]
    async fn publish_binds_scanned_digest_and_hides_source_path() {
        let temp = TempDir::new().unwrap();
        let source = temp.path().join("report.md");
        std::fs::write(&source, "# stable report\n").unwrap();
        let typed = Arc::new(ArtifactKernel::default());
        let kernel: Arc<dyn KernelHandle> = typed.clone();

        let output = tool_artifact_publish(
            &serde_json::json!({"path": "report.md", "title": "Stable report"}),
            Some(&kernel),
            Some(temp.path()),
            Some("agent-real"),
        )
        .await
        .unwrap();
        let published = typed.published.lock().unwrap();
        let request = published.as_ref().unwrap();
        assert_eq!(request.expected_sha256.as_deref().unwrap().len(), 64);
        assert_eq!(request.agent_id, "agent-real");
        assert!(!output.contains(&source.display().to_string()));
        assert!(output.contains("sha256_verified"));
    }

    #[tokio::test]
    async fn publish_rejects_secret_content_before_kernel_write() {
        let temp = TempDir::new().unwrap();
        std::fs::write(
            temp.path().join("secret.txt"),
            "ghp_abcdefghijklmnopqrstuvwxyz0123456789",
        )
        .unwrap();
        let typed = Arc::new(ArtifactKernel::default());
        let kernel: Arc<dyn KernelHandle> = typed.clone();

        let error = tool_artifact_publish(
            &serde_json::json!({"path": "secret.txt", "title": "Unsafe"}),
            Some(&kernel),
            Some(temp.path()),
            Some("agent-real"),
        )
        .await
        .unwrap_err();
        assert!(error.contains("literal secret-looking value"));
        assert!(typed.published.lock().unwrap().is_none());
    }

    #[tokio::test]
    async fn deliver_reads_verified_payload_and_uses_default_recipient() {
        let temp = TempDir::new().unwrap();
        std::fs::write(temp.path().join("report.md"), "delivery body").unwrap();
        let typed = Arc::new(ArtifactKernel::default());
        let kernel: Arc<dyn KernelHandle> = typed.clone();
        let published = tool_artifact_publish(
            &serde_json::json!({"path": "report.md", "title": "Delivery report"}),
            Some(&kernel),
            Some(temp.path()),
            Some("agent-real"),
        )
        .await
        .unwrap();
        let published: serde_json::Value = serde_json::from_str(&published).unwrap();
        let artifact_id = published["artifact"]["artifact_id"].as_str().unwrap();

        let delivered = tool_artifact_deliver(
            &serde_json::json!({"artifact_id": artifact_id, "channel": "telegram"}),
            Some(&kernel),
            Some("agent-real"),
        )
        .await
        .unwrap();
        assert!(!delivered.contains(&temp.path().display().to_string()));
        let deliveries = typed.deliveries.lock().unwrap();
        assert_eq!(deliveries.len(), 1);
        assert_eq!(deliveries[0].channel, "telegram");
        assert_eq!(deliveries[0].recipient, "default-recipient");
        assert_eq!(deliveries[0].filename, "report.md");
        assert_eq!(deliveries[0].data, b"delivery body");
    }

    #[tokio::test]
    async fn list_and_inspect_validate_bounds_and_identity() {
        let kernel: Arc<dyn KernelHandle> = Arc::new(ArtifactKernel::default());
        assert!(tool_artifact_list(
            &serde_json::json!({"limit": 101}),
            Some(&kernel),
            Some("agent-real")
        )
        .await
        .unwrap_err()
        .contains("between 1 and 100"));
        assert!(tool_artifact_inspect(
            &serde_json::json!({"artifact_id": "not-a-uuid"}),
            Some(&kernel),
            Some("agent-real")
        )
        .await
        .unwrap_err()
        .contains("valid UUID"));
    }
}
