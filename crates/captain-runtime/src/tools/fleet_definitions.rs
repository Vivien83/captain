//! Static fleet-management tool definitions.

use captain_types::tool::ToolDefinition;
use serde_json::Value;

pub fn fleet_tool_definitions() -> Vec<ToolDefinition> {
    let mut definitions = fleet_lifecycle_tool_definitions();
    definitions.extend(fleet_mission_tool_definitions());
    definitions.extend(fleet_scaling_observability_tool_definitions());
    definitions
}

fn fleet_lifecycle_tool_definitions() -> Vec<ToolDefinition> {
    vec![
        fleet_create_manager_tool_definition(),
        fleet_list_managers_tool_definition(),
        fleet_close_manager_tool_definition(),
    ]
}

fn fleet_mission_tool_definitions() -> Vec<ToolDefinition> {
    vec![fleet_set_mission_tool_definition()]
}

fn fleet_scaling_observability_tool_definitions() -> Vec<ToolDefinition> {
    vec![
        fleet_configure_autoscale_tool_definition(),
        fleet_metrics_tool_definition(),
    ]
}

fn tool_definition(name: &str, description: &str, input_schema: Value) -> ToolDefinition {
    ToolDefinition {
        name: name.to_string(),
        description: description.to_string(),
        input_schema,
    }
}

fn fleet_create_manager_tool_definition() -> ToolDefinition {
    tool_definition(
        "fleet_create_manager",
        "Crée un Manager de flotte — un agent autonome spécialisé dans un domaine. Le Manager peut spawner ses propres workers, les surveiller, les corriger et les tuer. Retourne l'ID du Manager créé.",
        serde_json::json!({
            "type": "object",
            "properties": {
                "name": { "type": "string", "description": "Nom du manager (ex: 'research', 'trading', 'ops')" },
                "domain": { "type": "string", "description": "Description du domaine (ex: 'Recherche web et analyse de données')" },
                "model": { "type": "string", "description": "Modèle LLM pour le manager (défaut: modèle par défaut du système)" },
                "budget_tokens": { "type": "integer", "description": "Budget tokens/heure pour toute la flotte (0 = illimité)", "default": 10000 }
            },
            "required": ["name", "domain"]
        }),
    )
}

fn fleet_list_managers_tool_definition() -> ToolDefinition {
    tool_definition(
        "fleet_list_managers",
        "Liste tous les Managers de flottes actifs avec leurs stats (workers, tokens, état).",
        serde_json::json!({
            "type": "object",
            "properties": {}
        }),
    )
}

fn fleet_close_manager_tool_definition() -> ToolDefinition {
    tool_definition(
        "fleet_close_manager",
        "Ferme un Manager et tous ses workers. Utiliser quand la mission est terminée ou le Manager ne répond plus.",
        serde_json::json!({
            "type": "object",
            "properties": {
                "manager_id": { "type": "string", "description": "UUID du Manager à fermer" }
            },
            "required": ["manager_id"]
        }),
    )
}

fn fleet_set_mission_tool_definition() -> ToolDefinition {
    tool_definition(
        "fleet_set_mission",
        "Enregistre la mission courante d'un Manager. La mission persiste au reboot du daemon — à son retour, le Manager la reçoit comme rappel. Passer mission vide ou null pour effacer.",
        serde_json::json!({
            "type": "object",
            "properties": {
                "manager_id": { "type": "string" },
                "mission": { "type": "string", "description": "Mission en cours (1-2 phrases). Laisser vide pour effacer." }
            },
            "required": ["manager_id"]
        }),
    )
}

fn fleet_configure_autoscale_tool_definition() -> ToolDefinition {
    tool_definition(
        "fleet_configure_autoscale",
        "Configure l'auto-scaling d'une flotte. L'auto-scaling spawne/tue des workers en fonction de la demande. kill_threshold doit être < spawn_threshold pour éviter l'oscillation.",
        serde_json::json!({
            "type": "object",
            "properties": {
                "manager_id": { "type": "string" },
                "enabled": { "type": "boolean", "default": true },
                "min_workers": { "type": "integer", "minimum": 0, "default": 0 },
                "max_workers": { "type": "integer", "minimum": 1, "default": 3 },
                "spawn_threshold": { "type": "integer", "minimum": 1, "default": 2 },
                "kill_threshold": { "type": "integer", "minimum": 0, "default": 0 },
                "cooldown_secs": { "type": "integer", "minimum": 5, "default": 60 },
                "worker_template": { "type": "string", "description": "Manifest TOML optionnel utilisé pour spawner un worker automatiquement." }
            },
            "required": ["manager_id"]
        }),
    )
}

fn fleet_metrics_tool_definition() -> ToolDefinition {
    tool_definition(
        "fleet_metrics",
        "Retourne les métriques de charge d'une flotte : workers actifs/idles, profondeur de queue, tokens consommés, config autoscale.",
        serde_json::json!({
            "type": "object",
            "properties": {
                "manager_id": { "type": "string" }
            },
            "required": ["manager_id"]
        }),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fleet_tool_definitions_keep_public_order() {
        let tools = fleet_tool_definitions();
        let names: Vec<_> = tools.iter().map(|tool| tool.name.as_str()).collect();

        assert_eq!(
            names,
            vec![
                "fleet_create_manager",
                "fleet_list_managers",
                "fleet_close_manager",
                "fleet_set_mission",
                "fleet_configure_autoscale",
                "fleet_metrics",
            ]
        );
    }

    #[test]
    fn fleet_tool_definitions_keep_lifecycle_contracts() {
        let tools = fleet_tool_definitions();
        let create = tool(&tools, "fleet_create_manager");
        let list = tool(&tools, "fleet_list_managers");
        let close = tool(&tools, "fleet_close_manager");

        assert_eq!(required_fields(create), vec!["name", "domain"]);
        assert_eq!(
            integer_field(property(create, "budget_tokens"), "default"),
            Some(10000)
        );
        assert_contains(&create.description, "spawner ses propres workers");

        assert!(required_fields(list).is_empty());
        assert_eq!(properties_len(list), Some(0));

        assert_eq!(required_fields(close), vec!["manager_id"]);
        assert_contains(&close.description, "tous ses workers");
    }

    #[test]
    fn fleet_tool_definitions_keep_mission_autoscale_and_metrics_contracts() {
        let tools = fleet_tool_definitions();
        let mission = tool(&tools, "fleet_set_mission");
        let autoscale = tool(&tools, "fleet_configure_autoscale");
        let metrics = tool(&tools, "fleet_metrics");

        assert_eq!(required_fields(mission), vec!["manager_id"]);
        assert_contains(&mission.description, "persiste au reboot");
        assert_contains(
            property(mission, "mission")["description"]
                .as_str()
                .unwrap_or_default(),
            "Laisser vide pour effacer",
        );

        assert_eq!(required_fields(autoscale), vec!["manager_id"]);
        assert_contains(
            &autoscale.description,
            "kill_threshold doit être < spawn_threshold",
        );
        assert_eq!(
            boolean_field(property(autoscale, "enabled"), "default"),
            Some(true)
        );
        assert_eq!(
            integer_field(property(autoscale, "min_workers"), "minimum"),
            Some(0)
        );
        assert_eq!(
            integer_field(property(autoscale, "max_workers"), "default"),
            Some(3)
        );
        assert_eq!(
            integer_field(property(autoscale, "spawn_threshold"), "default"),
            Some(2)
        );
        assert_eq!(
            integer_field(property(autoscale, "kill_threshold"), "default"),
            Some(0)
        );
        assert_eq!(
            integer_field(property(autoscale, "cooldown_secs"), "default"),
            Some(60)
        );

        assert_eq!(required_fields(metrics), vec!["manager_id"]);
        assert_contains(&metrics.description, "tokens consommés");
        assert_contains(&metrics.description, "config autoscale");
    }

    fn tool<'a>(tools: &'a [ToolDefinition], name: &str) -> &'a ToolDefinition {
        tools
            .iter()
            .find(|tool| tool.name == name)
            .unwrap_or_else(|| panic!("{name} should be registered"))
    }

    fn required_fields(tool: &ToolDefinition) -> Vec<String> {
        tool.input_schema
            .get("required")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
            .map(str::to_string)
            .collect()
    }

    fn property<'a>(tool: &'a ToolDefinition, name: &str) -> &'a Value {
        tool.input_schema
            .get("properties")
            .and_then(|properties| properties.get(name))
            .unwrap_or_else(|| panic!("{} should define property {name}", tool.name))
    }

    fn properties_len(tool: &ToolDefinition) -> Option<usize> {
        tool.input_schema
            .get("properties")
            .and_then(Value::as_object)
            .map(serde_json::Map::len)
    }

    fn integer_field(value: &Value, name: &str) -> Option<u64> {
        value.get(name).and_then(Value::as_u64)
    }

    fn boolean_field(value: &Value, name: &str) -> Option<bool> {
        value.get(name).and_then(Value::as_bool)
    }

    fn assert_contains(haystack: &str, needle: &str) {
        assert!(
            haystack.contains(needle),
            "expected `{haystack}` to contain `{needle}`"
        );
    }
}
