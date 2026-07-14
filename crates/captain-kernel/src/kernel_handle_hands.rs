use std::collections::HashMap;

use captain_hands::{HandDefinition, HandInstance};
use serde_json::Value;
use uuid::Uuid;

use super::CaptainKernel;

impl CaptainKernel {
    pub(super) async fn handle_hand_list(&self) -> Result<Vec<Value>, String> {
        let defs = self.hand_registry.list_definitions();
        let instances = self.hand_registry.list_instances();

        Ok(defs
            .iter()
            .map(|def| {
                let active_instance = instances.iter().find(|i| i.hand_id == def.id);
                hand_list_entry(def, active_instance)
            })
            .collect())
    }

    pub(super) async fn handle_hand_install(
        &self,
        toml_content: &str,
        skill_content: &str,
    ) -> Result<Value, String> {
        let def = self
            .hand_registry
            .install_from_content(toml_content, skill_content)
            .map_err(|e| format!("{e}"))?;

        Ok(hand_install_response(&def))
    }

    pub(super) async fn handle_hand_activate(
        &self,
        hand_id: &str,
        config: HashMap<String, Value>,
    ) -> Result<Value, String> {
        let instance = self
            .activate_hand(hand_id, config)
            .map_err(|e| format!("{e}"))?;

        Ok(hand_activation_response(&instance))
    }

    pub(super) async fn handle_hand_status(&self, hand_id: &str) -> Result<Value, String> {
        let instances = self.hand_registry.list_instances();
        let instance = instances
            .iter()
            .find(|i| i.hand_id == hand_id)
            .ok_or_else(|| format!("No active instance found for hand '{hand_id}'"))?;

        let def = self.hand_registry.get_definition(hand_id);
        Ok(hand_status_response(hand_id, def.as_ref(), instance))
    }

    pub(super) async fn handle_hand_deactivate(&self, instance_id: &str) -> Result<(), String> {
        let uuid = parse_hand_instance_id(instance_id)?;
        self.deactivate_hand(uuid).map_err(|e| format!("{e}"))
    }
}

fn hand_list_entry(def: &HandDefinition, active_instance: Option<&HandInstance>) -> Value {
    let (status, instance_id, agent_id) = match active_instance {
        Some(inst) => (
            format!("{}", inst.status),
            Some(inst.instance_id.to_string()),
            inst.agent_id.map(|a| a.to_string()),
        ),
        None => ("available".to_string(), None, None),
    };

    let mut entry = serde_json::json!({
        "id": def.id.clone(),
        "name": def.name.clone(),
        "icon": def.icon.clone(),
        "category": format!("{:?}", def.category),
        "description": def.description.clone(),
        "status": status,
        "tools": def.tools.clone(),
    });
    if let Some(iid) = instance_id {
        entry["instance_id"] = serde_json::json!(iid);
    }
    if let Some(aid) = agent_id {
        entry["agent_id"] = serde_json::json!(aid);
    }
    entry
}

fn hand_install_response(def: &HandDefinition) -> Value {
    serde_json::json!({
        "id": def.id.clone(),
        "name": def.name.clone(),
        "description": def.description.clone(),
        "category": format!("{:?}", def.category),
    })
}

fn hand_activation_response(instance: &HandInstance) -> Value {
    serde_json::json!({
        "instance_id": instance.instance_id.to_string(),
        "hand_id": instance.hand_id.clone(),
        "agent_name": instance.agent_name.clone(),
        "agent_id": instance.agent_id.map(|a| a.to_string()),
        "status": format!("{}", instance.status),
    })
}

fn hand_status_response(
    hand_id: &str,
    def: Option<&HandDefinition>,
    instance: &HandInstance,
) -> Value {
    let def_name = def.map(|d| d.name.clone()).unwrap_or_default();
    let def_icon = def.map(|d| d.icon.clone()).unwrap_or_default();

    serde_json::json!({
        "hand_id": hand_id,
        "name": def_name,
        "icon": def_icon,
        "instance_id": instance.instance_id.to_string(),
        "status": format!("{}", instance.status),
        "agent_id": instance.agent_id.map(|a| a.to_string()),
        "agent_name": instance.agent_name.clone(),
        "activated_at": instance.activated_at.to_rfc3339(),
        "updated_at": instance.updated_at.to_rfc3339(),
    })
}

fn parse_hand_instance_id(instance_id: &str) -> Result<Uuid, String> {
    Uuid::parse_str(instance_id).map_err(|e| format!("Invalid instance ID: {e}"))
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use captain_hands::{
        HandAgentConfig, HandCategory, HandDashboard, HandDefinition, HandInstance,
    };
    use captain_types::agent::AgentId;

    use super::{
        hand_activation_response, hand_list_entry, hand_status_response, parse_hand_instance_id,
    };

    fn test_definition() -> HandDefinition {
        HandDefinition {
            id: "clip".to_string(),
            name: "Clip".to_string(),
            description: "Summarize inbox clips".to_string(),
            category: HandCategory::Productivity,
            icon: "C".to_string(),
            tools: vec!["web_search".to_string()],
            skills: Vec::new(),
            mcp_servers: Vec::new(),
            requires: Vec::new(),
            settings: Vec::new(),
            agent: HandAgentConfig::default(),
            dashboard: HandDashboard::default(),
            skill_content: None,
        }
    }

    fn active_instance() -> HandInstance {
        let mut instance = HandInstance::new("clip", "Clip Agent", HashMap::new());
        instance.agent_id = Some(AgentId::new());
        instance
    }

    #[test]
    fn hand_list_entry_marks_available_without_instance() {
        let entry = hand_list_entry(&test_definition(), None);

        assert_eq!(entry["id"].as_str(), Some("clip"));
        assert_eq!(entry["status"].as_str(), Some("available"));
        assert!(entry.get("instance_id").is_none());
        assert!(entry.get("agent_id").is_none());
    }

    #[test]
    fn hand_list_entry_includes_active_instance() {
        let instance = active_instance();
        let instance_id = instance.instance_id.to_string();
        let agent_id = instance.agent_id.unwrap().to_string();
        let entry = hand_list_entry(&test_definition(), Some(&instance));

        assert_eq!(entry["status"].as_str(), Some("Active"));
        assert_eq!(entry["instance_id"].as_str(), Some(instance_id.as_str()));
        assert_eq!(entry["agent_id"].as_str(), Some(agent_id.as_str()));
    }

    #[test]
    fn hand_status_response_uses_definition_when_available() {
        let def = test_definition();
        let instance = active_instance();
        let status = hand_status_response("clip", Some(&def), &instance);
        let activation = hand_activation_response(&instance);

        assert_eq!(status["name"].as_str(), Some("Clip"));
        assert_eq!(status["icon"].as_str(), Some("C"));
        assert_eq!(status["agent_name"].as_str(), Some("Clip Agent"));
        assert_eq!(activation["hand_id"].as_str(), Some("clip"));
    }

    #[test]
    fn parse_hand_instance_id_reports_invalid_input() {
        let err = parse_hand_instance_id("not-a-uuid").unwrap_err();
        assert!(err.starts_with("Invalid instance ID:"));
    }
}
