//! Agent registry — tracks all agents, their state, and indexes.

use captain_types::agent::{AgentEntry, AgentId, AgentMode, AgentState};
use captain_types::error::{CaptainError, CaptainResult};
use dashmap::DashMap;

/// Registry of all agents in the kernel.
pub struct AgentRegistry {
    /// Primary index: agent ID → entry.
    agents: DashMap<AgentId, AgentEntry>,
    /// Name index: human-readable name → agent ID.
    name_index: DashMap<String, AgentId>,
    /// Tag index: tag → list of agent IDs.
    tag_index: DashMap<String, Vec<AgentId>>,
}

impl AgentRegistry {
    /// Create a new empty registry.
    pub fn new() -> Self {
        Self {
            agents: DashMap::new(),
            name_index: DashMap::new(),
            tag_index: DashMap::new(),
        }
    }

    /// Register a new agent.
    pub fn register(&self, entry: AgentEntry) -> CaptainResult<()> {
        if self.name_index.contains_key(&entry.name) {
            return Err(CaptainError::AgentAlreadyExists(entry.name.clone()));
        }
        let id = entry.id;
        self.name_index.insert(entry.name.clone(), id);
        for tag in &entry.tags {
            self.tag_index.entry(tag.clone()).or_default().push(id);
        }
        self.agents.insert(id, entry);
        Ok(())
    }

    /// Get an agent entry by ID.
    pub fn get(&self, id: AgentId) -> Option<AgentEntry> {
        self.agents.get(&id).map(|e| e.value().clone())
    }

    /// Find an agent by name.
    pub fn find_by_name(&self, name: &str) -> Option<AgentEntry> {
        self.name_index
            .get(name)
            .and_then(|id| self.agents.get(id.value()).map(|e| e.value().clone()))
    }

    /// Refresh an agent's last_active timestamp without changing anything
    /// else. Called at turn start so long LLM turns (90s+) are not
    /// mistaken for unresponsiveness by the heartbeat (60s timeout).
    pub fn touch(&self, id: AgentId) -> CaptainResult<()> {
        let mut entry = self
            .agents
            .get_mut(&id)
            .ok_or_else(|| CaptainError::AgentNotFound(id.to_string()))?;
        entry.last_active = chrono::Utc::now();
        Ok(())
    }

    /// Update agent state.
    pub fn set_state(&self, id: AgentId, state: AgentState) -> CaptainResult<()> {
        let mut entry = self
            .agents
            .get_mut(&id)
            .ok_or_else(|| CaptainError::AgentNotFound(id.to_string()))?;
        entry.state = state;
        entry.last_active = chrono::Utc::now();
        Ok(())
    }

    /// Update agent operational mode.
    pub fn set_mode(&self, id: AgentId, mode: AgentMode) -> CaptainResult<()> {
        let mut entry = self
            .agents
            .get_mut(&id)
            .ok_or_else(|| CaptainError::AgentNotFound(id.to_string()))?;
        entry.mode = mode;
        entry.last_active = chrono::Utc::now();
        Ok(())
    }

    /// Remove an agent from the registry.
    pub fn remove(&self, id: AgentId) -> CaptainResult<AgentEntry> {
        let (_, entry) = self
            .agents
            .remove(&id)
            .ok_or_else(|| CaptainError::AgentNotFound(id.to_string()))?;
        self.name_index.remove(&entry.name);
        for tag in &entry.tags {
            if let Some(mut ids) = self.tag_index.get_mut(tag) {
                ids.retain(|&agent_id| agent_id != id);
            }
        }
        Ok(entry)
    }

    /// List all agents.
    pub fn list(&self) -> Vec<AgentEntry> {
        self.agents.iter().map(|e| e.value().clone()).collect()
    }

    /// Add a child agent ID to a parent's children list.
    pub fn add_child(&self, parent_id: AgentId, child_id: AgentId) {
        if let Some(mut entry) = self.agents.get_mut(&parent_id) {
            entry.children.push(child_id);
        }
    }

    /// Set (or clear) the current mission for an agent (used by fleet managers).
    pub fn set_mission(&self, id: AgentId, mission: Option<String>) -> CaptainResult<()> {
        let mut entry = self
            .agents
            .get_mut(&id)
            .ok_or_else(|| CaptainError::AgentNotFound(id.to_string()))?;
        entry.mission = mission;
        entry.mission_set_at = Some(chrono::Utc::now());
        entry.last_active = chrono::Utc::now();
        Ok(())
    }

    /// Update the autoscale configuration for an agent.
    pub fn set_autoscale(
        &self,
        id: AgentId,
        config: Option<captain_types::agent::AutoScaleConfig>,
    ) -> CaptainResult<()> {
        let mut entry = self
            .agents
            .get_mut(&id)
            .ok_or_else(|| CaptainError::AgentNotFound(id.to_string()))?;
        entry.autoscale = config;
        entry.last_active = chrono::Utc::now();
        Ok(())
    }

    /// Stamp the last scale event timestamp (for cooldown tracking).
    pub fn stamp_scale_event(&self, id: AgentId) -> CaptainResult<()> {
        let mut entry = self
            .agents
            .get_mut(&id)
            .ok_or_else(|| CaptainError::AgentNotFound(id.to_string()))?;
        entry.last_scale_event = Some(chrono::Utc::now());
        Ok(())
    }

    /// Count of registered agents.
    pub fn count(&self) -> usize {
        self.agents.len()
    }

    /// Update an agent's session ID (for session reset).
    pub fn update_session_id(
        &self,
        id: AgentId,
        new_session_id: captain_types::agent::SessionId,
    ) -> CaptainResult<()> {
        let mut entry = self
            .agents
            .get_mut(&id)
            .ok_or_else(|| CaptainError::AgentNotFound(id.to_string()))?;
        entry.session_id = new_session_id;
        entry.last_active = chrono::Utc::now();
        Ok(())
    }

    /// Update an agent's workspace path.
    pub fn update_workspace(
        &self,
        id: AgentId,
        workspace: Option<std::path::PathBuf>,
    ) -> CaptainResult<()> {
        let mut entry = self
            .agents
            .get_mut(&id)
            .ok_or_else(|| CaptainError::AgentNotFound(id.to_string()))?;
        entry.manifest.workspace = workspace;
        entry.last_active = chrono::Utc::now();
        Ok(())
    }

    /// Update an agent's visual identity (emoji, avatar, color).
    pub fn update_identity(
        &self,
        id: AgentId,
        identity: captain_types::agent::AgentIdentity,
    ) -> CaptainResult<()> {
        let mut entry = self
            .agents
            .get_mut(&id)
            .ok_or_else(|| CaptainError::AgentNotFound(id.to_string()))?;
        entry.identity = identity;
        entry.last_active = chrono::Utc::now();
        Ok(())
    }

    /// Update an agent's model configuration.
    pub fn update_model(&self, id: AgentId, new_model: String) -> CaptainResult<()> {
        let mut entry = self
            .agents
            .get_mut(&id)
            .ok_or_else(|| CaptainError::AgentNotFound(id.to_string()))?;
        entry.manifest.model.model = new_model;
        entry.last_active = chrono::Utc::now();
        Ok(())
    }

    /// Update an agent's model AND provider together.
    pub fn update_model_and_provider(
        &self,
        id: AgentId,
        new_model: String,
        new_provider: String,
    ) -> CaptainResult<()> {
        let mut entry = self
            .agents
            .get_mut(&id)
            .ok_or_else(|| CaptainError::AgentNotFound(id.to_string()))?;
        let provider_changed = entry.manifest.model.provider != new_provider;
        entry.manifest.model.model = new_model;
        entry.manifest.model.provider = new_provider;
        // When provider changes, clear provider-specific overrides so the new provider
        // falls back to its default credential chain (env var / vault / dotenv).
        // Keeping a stale api_key_env like "GEMINI_API_KEY" after switching to OpenRouter
        // would send the wrong key and trigger 401 errors.
        if provider_changed {
            entry.manifest.model.api_key_env = None;
            entry.manifest.model.base_url = None;
        }
        entry.last_active = chrono::Utc::now();
        Ok(())
    }

    /// Update an agent's orchestration mode (routing / delegation / pinned).
    pub fn update_orchestration_mode(
        &self,
        id: AgentId,
        mode: captain_types::agent::OrchestrationMode,
    ) -> CaptainResult<()> {
        let mut entry = self
            .agents
            .get_mut(&id)
            .ok_or_else(|| CaptainError::AgentNotFound(id.to_string()))?;
        entry.manifest.orchestration_mode = mode;
        entry.last_active = chrono::Utc::now();
        Ok(())
    }

    /// Replace an agent's routing config (used after model switch).
    pub fn update_routing(
        &self,
        id: AgentId,
        routing: Option<captain_types::agent::ModelRoutingConfig>,
    ) -> CaptainResult<()> {
        let mut entry = self
            .agents
            .get_mut(&id)
            .ok_or_else(|| CaptainError::AgentNotFound(id.to_string()))?;
        entry.manifest.routing = routing;
        entry.last_active = chrono::Utc::now();
        Ok(())
    }

    /// Update an agent's fallback model chain.
    pub fn update_fallback_models(
        &self,
        id: AgentId,
        fallback_models: Vec<captain_types::agent::FallbackModel>,
    ) -> CaptainResult<()> {
        let mut entry = self
            .agents
            .get_mut(&id)
            .ok_or_else(|| CaptainError::AgentNotFound(id.to_string()))?;
        entry.manifest.fallback_models = fallback_models;
        entry.last_active = chrono::Utc::now();
        Ok(())
    }

    /// Update an agent's skill allowlist.
    pub fn update_skills(&self, id: AgentId, skills: Vec<String>) -> CaptainResult<()> {
        let mut entry = self
            .agents
            .get_mut(&id)
            .ok_or_else(|| CaptainError::AgentNotFound(id.to_string()))?;
        entry.manifest.skills = skills;
        entry.last_active = chrono::Utc::now();
        Ok(())
    }

    /// Update an agent's MCP server allowlist.
    pub fn update_mcp_servers(&self, id: AgentId, servers: Vec<String>) -> CaptainResult<()> {
        let mut entry = self
            .agents
            .get_mut(&id)
            .ok_or_else(|| CaptainError::AgentNotFound(id.to_string()))?;
        entry.manifest.mcp_servers = servers;
        entry.last_active = chrono::Utc::now();
        Ok(())
    }

    /// Update an agent's tool allowlist and blocklist.
    pub fn update_tool_filters(
        &self,
        id: AgentId,
        allowlist: Option<Vec<String>>,
        blocklist: Option<Vec<String>>,
    ) -> CaptainResult<()> {
        let mut entry = self
            .agents
            .get_mut(&id)
            .ok_or_else(|| CaptainError::AgentNotFound(id.to_string()))?;
        if let Some(al) = allowlist {
            entry.manifest.tool_allowlist = al;
        }
        if let Some(bl) = blocklist {
            entry.manifest.tool_blocklist = bl;
        }
        entry.last_active = chrono::Utc::now();
        Ok(())
    }

    /// Update an agent's system prompt (hot-swap, takes effect on next message).
    pub fn update_system_prompt(&self, id: AgentId, new_prompt: String) -> CaptainResult<()> {
        let mut entry = self
            .agents
            .get_mut(&id)
            .ok_or_else(|| CaptainError::AgentNotFound(id.to_string()))?;
        entry.manifest.model.system_prompt = new_prompt;
        entry.last_active = chrono::Utc::now();
        Ok(())
    }

    /// Update an agent's name (also updates the name index).
    pub fn update_name(&self, id: AgentId, new_name: String) -> CaptainResult<()> {
        if let Some(existing_id) = self.name_index.get(&new_name).as_deref().copied() {
            if existing_id != id {
                return Err(CaptainError::AgentAlreadyExists(new_name));
            }
            // Same agent owns this name — no-op
            return Ok(());
        }
        let mut entry = self
            .agents
            .get_mut(&id)
            .ok_or_else(|| CaptainError::AgentNotFound(id.to_string()))?;
        let old_name = entry.name.clone();
        entry.name = new_name.clone();
        entry.manifest.name = new_name.clone();
        entry.last_active = chrono::Utc::now();
        // Update name index
        drop(entry);
        self.name_index.remove(&old_name);
        self.name_index.insert(new_name, id);
        Ok(())
    }

    /// Update an agent's description.
    pub fn update_description(&self, id: AgentId, new_desc: String) -> CaptainResult<()> {
        let mut entry = self
            .agents
            .get_mut(&id)
            .ok_or_else(|| CaptainError::AgentNotFound(id.to_string()))?;
        entry.manifest.description = new_desc;
        entry.last_active = chrono::Utc::now();
        Ok(())
    }

    /// Update an agent's resource quota (budget limits).
    pub fn update_resources(
        &self,
        id: AgentId,
        hourly: Option<f64>,
        daily: Option<f64>,
        monthly: Option<f64>,
        tokens_per_hour: Option<u64>,
    ) -> CaptainResult<()> {
        let mut entry = self
            .agents
            .get_mut(&id)
            .ok_or_else(|| CaptainError::AgentNotFound(id.to_string()))?;
        if let Some(v) = hourly {
            entry.manifest.resources.max_cost_per_hour_usd = v;
        }
        if let Some(v) = daily {
            entry.manifest.resources.max_cost_per_day_usd = v;
        }
        if let Some(v) = monthly {
            entry.manifest.resources.max_cost_per_month_usd = v;
        }
        if let Some(v) = tokens_per_hour {
            entry.manifest.resources.max_llm_tokens_per_hour = v;
        }
        entry.last_active = chrono::Utc::now();
        Ok(())
    }

    /// Mark an agent's onboarding as complete.
    pub fn mark_onboarding_complete(&self, id: AgentId) -> CaptainResult<()> {
        let mut entry = self
            .agents
            .get_mut(&id)
            .ok_or_else(|| CaptainError::AgentNotFound(id.to_string()))?;
        entry.onboarding_completed = true;
        entry.onboarding_completed_at = Some(chrono::Utc::now());
        entry.last_active = chrono::Utc::now();
        Ok(())
    }
}

impl Default for AgentRegistry {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use captain_types::agent::*;
    use chrono::Utc;
    use std::collections::HashMap;

    fn test_entry(name: &str) -> AgentEntry {
        AgentEntry {
            id: AgentId::new(),
            name: name.to_string(),
            manifest: AgentManifest {
                name: name.to_string(),
                version: "0.1.0".to_string(),
                description: "test".to_string(),
                author: "test".to_string(),
                module: "test".to_string(),
                schedule: ScheduleMode::default(),
                model: ModelConfig::default(),
                fallback_models: vec![],
                resources: ResourceQuota::default(),
                priority: Priority::default(),
                capabilities: ManifestCapabilities::default(),
                profile: None,
                tools: HashMap::new(),
                skills: vec![],
                mcp_servers: vec![],
                metadata: HashMap::new(),
                tags: vec![],
                routing: None,
                autonomous: None,
                pinned_model: None,
                workspace: None,
                generate_identity_files: true,
                exec_policy: None,
                tool_allowlist: vec![],
                tool_blocklist: vec![],
                orchestration_mode: captain_types::agent::OrchestrationMode::default(),
            },
            state: AgentState::Created,
            mode: AgentMode::default(),
            created_at: Utc::now(),
            last_active: Utc::now(),
            parent: None,
            children: vec![],
            session_id: SessionId::new(),
            tags: vec![],
            identity: Default::default(),
            onboarding_completed: false,
            onboarding_completed_at: None,
            mission: None,
            mission_set_at: None,
            autoscale: None,
            last_scale_event: None,
        }
    }

    #[test]
    fn test_register_and_get() {
        let registry = AgentRegistry::new();
        let entry = test_entry("test-agent");
        let id = entry.id;
        registry.register(entry).unwrap();
        assert!(registry.get(id).is_some());
    }

    #[test]
    fn test_find_by_name() {
        let registry = AgentRegistry::new();
        let entry = test_entry("my-agent");
        registry.register(entry).unwrap();
        assert!(registry.find_by_name("my-agent").is_some());
    }

    #[test]
    fn test_duplicate_name() {
        let registry = AgentRegistry::new();
        registry.register(test_entry("dup")).unwrap();
        assert!(registry.register(test_entry("dup")).is_err());
    }

    #[test]
    fn test_remove() {
        let registry = AgentRegistry::new();
        let entry = test_entry("removable");
        let id = entry.id;
        registry.register(entry).unwrap();
        registry.remove(id).unwrap();
        assert!(registry.get(id).is_none());
    }
}
