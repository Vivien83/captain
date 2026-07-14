use captain_runtime::a2a::AgentCard;

use super::CaptainKernel;

impl CaptainKernel {
    pub(super) fn handle_list_a2a_agents(&self) -> Vec<(String, String)> {
        let agents = self
            .a2a_external_agents
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        list_agent_names_and_urls(&agents)
    }

    pub(super) fn handle_get_a2a_agent_url(&self, name: &str) -> Option<String> {
        let agents = self
            .a2a_external_agents
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        find_agent_url_by_name(&agents, name)
    }

    pub(super) fn handle_has_external_agent(&self, name: &str) -> bool {
        let store = match self.a2a_external_agents.lock() {
            Ok(g) => g,
            Err(p) => p.into_inner(),
        };
        has_external_agent_entry(&store, name)
    }

    pub(super) fn handle_list_external_agents(&self) -> Result<String, String> {
        let store = self
            .a2a_external_agents
            .lock()
            .map_err(|e| format!("a2a store poisoned: {e}"))?;
        let entries = external_agent_entries(&store);
        serde_json::to_string_pretty(&entries).map_err(|e| format!("serialize: {e}"))
    }
}

fn list_agent_names_and_urls(store: &[(String, AgentCard)]) -> Vec<(String, String)> {
    store
        .iter()
        .map(|(_, card)| (card.name.clone(), card.url.clone()))
        .collect()
}

fn find_agent_url_by_name(store: &[(String, AgentCard)], name: &str) -> Option<String> {
    let name_lower = name.to_lowercase();
    store
        .iter()
        .find(|(_, card)| card.name.to_lowercase() == name_lower)
        .map(|(_, card)| card.url.clone())
}

fn has_external_agent_entry(store: &[(String, AgentCard)], name: &str) -> bool {
    store.iter().any(|(stored_name, _)| stored_name == name)
}

fn external_agent_entries(store: &[(String, AgentCard)]) -> Vec<serde_json::Value> {
    store
        .iter()
        .map(|(name, card)| {
            serde_json::json!({
                "name": name,
                "card": card,
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use captain_runtime::a2a::{AgentCapabilities, AgentSkill};

    fn sample_card(name: &str, url: &str) -> AgentCard {
        AgentCard {
            name: name.to_string(),
            description: "external agent".to_string(),
            url: url.to_string(),
            version: "1.0.0".to_string(),
            capabilities: AgentCapabilities::default(),
            skills: vec![AgentSkill {
                id: "plan".to_string(),
                name: "Plan".to_string(),
                description: "Plan work".to_string(),
                tags: vec!["planning".to_string()],
                examples: vec![],
            }],
            default_input_modes: vec!["text/plain".to_string()],
            default_output_modes: vec!["text/plain".to_string()],
        }
    }

    #[test]
    fn list_agents_uses_card_name_and_url() {
        let store = vec![(
            "store-name".to_string(),
            sample_card("Card Name", "https://agent.example/a2a"),
        )];

        assert_eq!(
            list_agent_names_and_urls(&store),
            vec![(
                "Card Name".to_string(),
                "https://agent.example/a2a".to_string()
            )]
        );
    }

    #[test]
    fn find_agent_url_is_case_insensitive_on_card_name() {
        let store = vec![(
            "planner".to_string(),
            sample_card("Research Planner", "https://agent.example/research"),
        )];

        assert_eq!(
            find_agent_url_by_name(&store, "research planner").as_deref(),
            Some("https://agent.example/research")
        );
    }

    #[test]
    fn external_agent_json_uses_store_key_and_card() {
        let store = vec![(
            "planner".to_string(),
            sample_card("Research Planner", "https://agent.example/research"),
        )];
        let entries = external_agent_entries(&store);

        assert_eq!(entries[0]["name"], "planner");
        assert_eq!(entries[0]["card"]["name"], "Research Planner");
        assert!(has_external_agent_entry(&store, "planner"));
        assert!(!has_external_agent_entry(&store, "Research Planner"));
    }
}
