use captain_runtime::kernel_handle::KernelHandle;
use captain_types::agent::{AgentId, AgentManifest};
use captain_types::approval::ApprovalRequest;
use captain_types::capability::Capability;
use captain_types::event::{ChatStreamEvent, Event, EventPayload, EventTarget};

use super::kernel_model_support::manifest_to_capabilities;
use super::CaptainKernel;

impl CaptainKernel {
    pub(super) fn handle_requires_approval(&self, tool_name: &str) -> bool {
        self.approval_manager.requires_approval(tool_name)
    }

    pub(super) async fn handle_request_approval(
        &self,
        agent_id: &str,
        tool_name: &str,
        action_summary: &str,
    ) -> Result<bool, String> {
        if self.is_hand_agent(agent_id) {
            tracing::info!(agent_id, tool_name, "Auto-approved for hand agent");
            return Ok(true);
        }

        let policy = self.approval_manager.policy();
        let req = ApprovalRequest {
            id: uuid::Uuid::new_v4(),
            agent_id: agent_id.to_string(),
            tool_name: tool_name.to_string(),
            description: approval_description(agent_id, tool_name),
            action_summary: bounded_action_summary(action_summary),
            risk_level: crate::approval::ApprovalManager::classify_risk(tool_name),
            requested_at: chrono::Utc::now(),
            timeout_secs: policy.timeout_secs,
        };

        self.publish_approval_requested(agent_id, &req).await;

        let decision = self.approval_manager.request_approval(req).await;
        Ok(decision.is_approved())
    }

    pub(super) async fn handle_spawn_agent_checked(
        &self,
        manifest_toml: &str,
        parent_id: Option<&str>,
        parent_caps: &[Capability],
    ) -> Result<(String, String), String> {
        let (child_manifest, child_caps) = child_manifest_capabilities(manifest_toml)?;

        captain_types::capability::validate_capability_inheritance(parent_caps, &child_caps)?;

        tracing::info!(
            parent = parent_id.unwrap_or("kernel"),
            child = %child_manifest.name,
            child_caps = child_caps.len(),
            "Capability inheritance validated — spawning child agent"
        );

        KernelHandle::spawn_agent(self, manifest_toml, parent_id).await
    }

    fn is_hand_agent(&self, agent_id: &str) -> bool {
        let Ok(aid) = agent_id.parse::<AgentId>() else {
            return false;
        };
        self.registry
            .get(aid)
            .map(|entry| has_hand_tag(&entry.tags))
            .unwrap_or(false)
    }

    async fn publish_approval_requested(&self, agent_id: &str, req: &ApprovalRequest) {
        let Ok(aid) = agent_id.parse::<AgentId>() else {
            return;
        };

        let payload = EventPayload::ChatStream(ChatStreamEvent::ApprovalRequested {
            agent_id: aid,
            request_id: req.id.to_string(),
            tool_name: req.tool_name.clone(),
            description: req.action_summary.clone(),
            risk_level: format!("{:?}", req.risk_level).to_lowercase(),
            timeout_secs: req.timeout_secs,
        });
        let event = Event::new(aid, EventTarget::Agent(aid), payload);
        self.event_bus.publish(event).await;
    }
}

fn has_hand_tag(tags: &[String]) -> bool {
    tags.iter().any(|tag| tag.starts_with("hand:"))
}

fn approval_description(agent_id: &str, tool_name: &str) -> String {
    format!("Agent {agent_id} requests to execute {tool_name}")
}

fn bounded_action_summary(action_summary: &str) -> String {
    action_summary.chars().take(512).collect()
}

fn child_manifest_capabilities(
    manifest_toml: &str,
) -> Result<(AgentManifest, Vec<Capability>), String> {
    let child_manifest: AgentManifest = toml::from_str(manifest_toml)
        .map_err(|e| captain_types::agent::format_agent_manifest_parse_error(&e, manifest_toml))?;
    let child_caps = manifest_to_capabilities(&child_manifest);
    Ok((child_manifest, child_caps))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hand_tag_detection_requires_hand_prefix() {
        assert!(has_hand_tag(&["hand:browser".to_string()]));
        assert!(!has_hand_tag(&["hands:browser".to_string()]));
        assert!(!has_hand_tag(&["manager".to_string()]));
    }

    #[test]
    fn action_summary_is_bounded_by_chars() {
        let input = "x".repeat(600);
        let summary = bounded_action_summary(&input);

        assert_eq!(summary.chars().count(), 512);
        assert!(summary.is_char_boundary(summary.len()));
    }

    #[test]
    fn child_manifest_capabilities_reports_invalid_manifest() {
        let err = child_manifest_capabilities("name = [").unwrap_err();

        assert!(err.starts_with("Invalid manifest:"));
    }

    #[test]
    fn child_manifest_capabilities_extracts_tool_grants() {
        let mut manifest = AgentManifest {
            name: "worker".to_string(),
            ..AgentManifest::default()
        };
        manifest.capabilities.tools.push("file_read".to_string());
        let manifest_toml = toml::to_string(&manifest).expect("serialize manifest");

        let (parsed, caps) =
            child_manifest_capabilities(&manifest_toml).expect("parse manifest caps");

        assert_eq!(parsed.name, "worker");
        assert!(caps.contains(&Capability::ToolInvoke("file_read".to_string())));
    }
}
