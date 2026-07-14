use async_trait::async_trait;
use captain_types::agent::{AgentEntry, AgentId};
use captain_wire::message::RemoteAgentInfo;

use super::CaptainKernel;

#[async_trait]
impl captain_wire::peer::PeerHandle for CaptainKernel {
    fn local_agents(&self) -> Vec<RemoteAgentInfo> {
        self.registry
            .list()
            .iter()
            .map(remote_agent_info_from_entry)
            .collect()
    }

    async fn handle_agent_message(
        &self,
        agent: &str,
        message: &str,
        _sender: Option<&str>,
    ) -> Result<String, String> {
        let agent_id = resolve_peer_agent_id(self, agent)?;

        match self.send_message(agent_id, message).await {
            Ok(result) => Ok(result.response),
            Err(e) => Err(format!("{e}")),
        }
    }

    fn discover_agents(&self, query: &str) -> Vec<RemoteAgentInfo> {
        let q = query.to_lowercase();
        self.registry
            .list()
            .iter()
            .filter(|entry| agent_matches_query(entry, &q))
            .map(remote_agent_info_from_entry)
            .collect()
    }

    fn uptime_secs(&self) -> u64 {
        self.booted_at.elapsed().as_secs()
    }
}

fn resolve_peer_agent_id(kernel: &CaptainKernel, agent: &str) -> Result<AgentId, String> {
    if let Ok(uuid) = uuid::Uuid::parse_str(agent) {
        return Ok(AgentId(uuid));
    }

    kernel
        .registry
        .list()
        .iter()
        .find(|entry| entry.name == agent)
        .map(|entry| entry.id)
        .ok_or_else(|| format!("Agent not found: {agent}"))
}

fn remote_agent_info_from_entry(entry: &AgentEntry) -> RemoteAgentInfo {
    RemoteAgentInfo {
        id: entry.id.0.to_string(),
        name: entry.name.clone(),
        description: entry.manifest.description.clone(),
        tags: entry.manifest.tags.clone(),
        tools: entry.manifest.capabilities.tools.clone(),
        state: format!("{:?}", entry.state),
    }
}

fn agent_matches_query(entry: &AgentEntry, query_lower: &str) -> bool {
    entry.name.to_lowercase().contains(query_lower)
        || entry
            .manifest
            .description
            .to_lowercase()
            .contains(query_lower)
        || entry
            .manifest
            .tags
            .iter()
            .any(|tag| tag.to_lowercase().contains(query_lower))
}

#[cfg(test)]
mod tests {
    use super::*;
    use captain_types::agent::{
        AgentIdentity, AgentManifest, AgentMode, AgentState, ManifestCapabilities, SessionId,
    };

    fn test_entry() -> AgentEntry {
        let id = AgentId::new();
        let mut capabilities = ManifestCapabilities::default();
        capabilities.tools.push("web_search".to_string());
        capabilities.tools.push("memory_recall".to_string());

        let manifest = AgentManifest {
            name: "remote-research".to_string(),
            description: "Research specialist".to_string(),
            capabilities,
            tags: vec!["research".to_string(), "web".to_string()],
            ..AgentManifest::default()
        };

        AgentEntry {
            id,
            name: manifest.name.clone(),
            manifest,
            state: AgentState::Running,
            mode: AgentMode::Full,
            created_at: chrono::Utc::now(),
            last_active: chrono::Utc::now(),
            parent: None,
            children: Vec::new(),
            session_id: SessionId::new(),
            tags: Vec::new(),
            identity: AgentIdentity::default(),
            onboarding_completed: false,
            onboarding_completed_at: None,
            mission: None,
            mission_set_at: None,
            autoscale: None,
            last_scale_event: None,
        }
    }

    #[test]
    fn remote_agent_info_uses_manifest_public_fields() {
        let entry = test_entry();

        let info = remote_agent_info_from_entry(&entry);

        assert_eq!(info.id, entry.id.0.to_string());
        assert_eq!(info.name, "remote-research");
        assert_eq!(info.description, "Research specialist");
        assert_eq!(info.tags, vec!["research", "web"]);
        assert_eq!(info.tools, vec!["web_search", "memory_recall"]);
        assert_eq!(info.state, "Running");
    }

    #[test]
    fn query_matches_name_description_or_manifest_tags() {
        let entry = test_entry();

        assert!(agent_matches_query(&entry, "remote"));
        assert!(agent_matches_query(&entry, "specialist"));
        assert!(agent_matches_query(&entry, "web"));
        assert!(!agent_matches_query(&entry, "billing"));
    }
}
