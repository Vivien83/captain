//! Static autopilot goal tool definitions.

use crate::tools::channel_policy::ACTIVE_CHANNELS;
use captain_types::tool::ToolDefinition;
use serde_json::Value;

pub fn goal_tool_definitions() -> Vec<ToolDefinition> {
    let mut definitions = goal_core_tool_definitions();
    definitions.extend(goal_suggestion_tool_definitions());
    definitions
}

fn goal_core_tool_definitions() -> Vec<ToolDefinition> {
    vec![
        goal_create_tool_definition(),
        goal_list_tool_definition(),
        goal_pause_tool_definition(),
        goal_resume_tool_definition(),
        goal_status_tool_definition(),
        goal_delete_tool_definition(),
    ]
}

fn goal_suggestion_tool_definitions() -> Vec<ToolDefinition> {
    vec![
        goal_list_suggestions_tool_definition(),
        goal_apply_suggestion_tool_definition(),
        goal_reject_suggestion_tool_definition(),
    ]
}

fn tool_definition(name: &str, description: &str, input_schema: Value) -> ToolDefinition {
    ToolDefinition {
        name: name.to_string(),
        description: description.to_string(),
        input_schema,
    }
}

fn goal_create_tool_definition() -> ToolDefinition {
    tool_definition(
        "goal_create",
        "[AUTOPILOT LONG-RUNNING] Crée un objectif autonome que Captain maintient en arrière-plan : check périodique d'une commande shell, recovery automatique si échec isolé, detection de non-progrès opt-in via CAPTAIN_PROGRESS=<token>, escalation au user via channel_send après N échecs/non-progrès consécutifs. Préférer cet outil à cron_create+schedule_create chaînés quand l'intention est de MAINTENIR ou FAIRE AVANCER un état (ex: « garde nginx green sur prod-server toutes les 5 min », « continue le build tant que le checkpoint change »). Si l'objectif appartient à un projet, renseigner project_slug ou project_id pour que Project Mode l'affiche et le pilote. L'état (recent_checks, consecutive_fails, escalated_at) est persisté dans ~/.captain/goals.json. Hard cap LLM intégré (max_llm_calls_per_hour) pour éviter dépenses runaway.",
        serde_json::json!({
            "type": "object",
            "properties": {
                "id": { "type": "string", "description": "Identifiant unique 3..=64 chars (alphanumérique + - ou _) — sert aussi de clé pour goal_pause/status/delete" },
                "name": { "type": "string", "description": "Label court (ex: 'nginx-uptime')" },
                "description": { "type": "string", "description": "Description en langage naturel de l'objectif (ex: 'Garde nginx green sur prod-server')" },
                "project_id": { "type": "string", "description": "Optionnel — id du projet Captain propriétaire si le goal suit un Project Mode." },
                "project_slug": { "type": "string", "description": "Optionnel — slug du projet Captain propriétaire si connu. A utiliser pour rattacher les goals aux projets web/TUI." },
                "interval_secs": { "type": "integer", "minimum": 10, "description": "Période entre 2 checks en secondes (≥ 10)" },
                "check_command": { "type": "string", "description": "Commande shell exécutée à chaque tick. Exit 0 = succès. Sur fail isolé, recovery_command est tenté. Pour prouver une convergence, imprimer une ligne CAPTAIN_PROGRESS=<token> ou un JSON {\"captain_progress\":\"<token>\"}; si le token ne change plus, Captain traite le tick comme non-progrès et escalade après le seuil. (ex: 'systemctl status nginx', 'curl -fsS https://example.com/health')" },
                "recovery_command": { "type": "string", "description": "Optionnel — commande shell tentée APRÈS un fail isolé pour ramener le système dans l'état souhaité (ex: 'systemctl restart nginx'). Si elle réussit puis le check repasse, le compteur de fails est reset." },
                "escalation_threshold": { "type": "integer", "minimum": 1, "description": "Nombre de fails ou non-progrès consécutifs (après recovery) avant escalation channel_send au user. Défaut: 3" },
                "max_llm_calls_per_hour": { "type": "integer", "minimum": 0, "maximum": 1000, "description": "Hard cap LLM par goal sur fenêtre 1h glissante (réflexion R.2.2). Défaut: 20. Plafond absolu: 1000." },
                "escalation_channel": {
                    "type": "object",
                    "description": "Optionnel — canal d'escalation explicite. Si omis, fallback au canal qui a créé le goal.",
                    "properties": {
                        "channel": {
                            "type": "string",
                            "enum": ACTIVE_CHANNELS,
                            "description": "Nom du canal actif: 'telegram', 'discord', 'signal' ou 'email'. Les autres channels sont gelés."
                        },
                        "recipient": { "type": "string", "description": "ID du destinataire (chat_id, user_id, ...)" }
                    },
                    "required": ["channel", "recipient"]
                }
            },
            "required": ["id", "name", "description", "interval_secs", "check_command"]
        }),
    )
}

fn goal_list_tool_definition() -> ToolDefinition {
    tool_definition(
        "goal_list",
        "Liste tous les goals autopilot persistés avec leur statut (active/paused/escalated), dernier check, compteur de fails consécutifs, et nombre d'appels LLM dans la dernière heure. Utiliser pour audit ou avant de créer un nouveau goal pour vérifier l'absence de doublon.",
        serde_json::json!({"type": "object", "properties": {}}),
    )
}

fn goal_pause_tool_definition() -> ToolDefinition {
    tool_definition(
        "goal_pause",
        "Suspend l'exécution périodique d'un goal sans le supprimer. Le loop interne s'arrête au prochain tick ; l'état (recent_checks, fails) est conservé. Réversible via goal_resume.",
        serde_json::json!({
            "type": "object",
            "properties": {
                "id": { "type": "string", "description": "Identifiant du goal (clé créée par goal_create)" }
            },
            "required": ["id"]
        }),
    )
}

fn goal_resume_tool_definition() -> ToolDefinition {
    tool_definition(
        "goal_resume",
        "Réactive un goal préalablement mis en pause. Le compteur de fails consécutifs est conservé tel quel (pas de reset).",
        serde_json::json!({
            "type": "object",
            "properties": {
                "id": { "type": "string", "description": "Identifiant du goal" }
            },
            "required": ["id"]
        }),
    )
}

fn goal_status_tool_definition() -> ToolDefinition {
    tool_definition(
        "goal_status",
        "Retourne le détail complet d'un goal : status, derniers checks (timestamps + ok/ko + output tronqué), consecutive_fails, escalated_at, llm_calls_last_hour. Utiliser pour diagnostic après une escalation ou pour vérifier que le loop tourne bien.",
        serde_json::json!({
            "type": "object",
            "properties": {
                "id": { "type": "string", "description": "Identifiant du goal" }
            },
            "required": ["id"]
        }),
    )
}

fn goal_delete_tool_definition() -> ToolDefinition {
    tool_definition(
        "goal_delete",
        "Supprime définitivement un goal et son historique. Action IRRÉVERSIBLE — préférer goal_pause si l'objectif peut être réactivé plus tard.",
        serde_json::json!({
            "type": "object",
            "properties": {
                "id": { "type": "string", "description": "Identifiant du goal à supprimer" }
            },
            "required": ["id"]
        }),
    )
}

fn goal_list_suggestions_tool_definition() -> ToolDefinition {
    tool_definition(
        "goal_list_suggestions",
        "Liste toutes les suggestions d'ajustement (Pending / Applied / Rejected) produites par le job de réflexion horaire pour un goal donné. Utiliser pour voir les recommandations en attente avant goal_apply_suggestion / goal_reject_suggestion.",
        serde_json::json!({
            "type": "object",
            "properties": {
                "id": { "type": "string", "description": "Identifiant du goal" }
            },
            "required": ["id"]
        }),
    )
}

fn goal_apply_suggestion_tool_definition() -> ToolDefinition {
    tool_definition(
        "goal_apply_suggestion",
        "Applique une suggestion Pending au goal (mute interval, threshold ou recovery_command selon kind). RE-VALIDE le goal avant de persister — refuse les valeurs hors-bornes (interval < 10s, threshold < 1, critical_patterns). La suggestion passe en Applied.",
        serde_json::json!({
            "type": "object",
            "properties": {
                "id": { "type": "string", "description": "Identifiant du goal" },
                "suggestion_id": { "type": "string", "description": "Identifiant de la suggestion (depuis goal_list_suggestions)" }
            },
            "required": ["id", "suggestion_id"]
        }),
    )
}

fn goal_reject_suggestion_tool_definition() -> ToolDefinition {
    tool_definition(
        "goal_reject_suggestion",
        "Refuse une suggestion Pending (aucune mutation du goal). La suggestion passe en Rejected. Utiliser quand la proposition du job de réflexion n'est pas pertinente.",
        serde_json::json!({
            "type": "object",
            "properties": {
                "id": { "type": "string", "description": "Identifiant du goal" },
                "suggestion_id": { "type": "string", "description": "Identifiant de la suggestion à rejeter" }
            },
            "required": ["id", "suggestion_id"]
        }),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn goal_tool_definitions_keep_public_order() {
        let tools = goal_tool_definitions();
        let names: Vec<_> = tools.iter().map(|tool| tool.name.as_str()).collect();

        assert_eq!(
            names,
            vec![
                "goal_create",
                "goal_list",
                "goal_pause",
                "goal_resume",
                "goal_status",
                "goal_delete",
                "goal_list_suggestions",
                "goal_apply_suggestion",
                "goal_reject_suggestion",
            ]
        );
    }

    #[test]
    fn goal_create_schema_keeps_runtime_bounds_and_active_channels() {
        let tools = goal_tool_definitions();
        let create = tool(&tools, "goal_create");
        let escalation = property(create, "escalation_channel");
        let channel = &escalation["properties"]["channel"];

        assert_eq!(
            required_fields(create),
            vec![
                "id",
                "name",
                "description",
                "interval_secs",
                "check_command"
            ]
        );
        assert_eq!(
            integer_field(property(create, "interval_secs"), "minimum"),
            Some(10)
        );
        assert_eq!(
            integer_field(property(create, "max_llm_calls_per_hour"), "maximum"),
            Some(1000)
        );
        assert_eq!(
            required_fields_from(escalation),
            vec!["channel", "recipient"]
        );
        assert_eq!(
            channel["enum"],
            serde_json::json!(["telegram", "discord", "signal", "email"])
        );
        assert!(!channel["description"]
            .as_str()
            .unwrap_or_default()
            .contains("slack"));
    }

    #[test]
    fn goal_state_and_suggestion_tools_keep_required_ids() {
        let tools = goal_tool_definitions();

        for name in [
            "goal_pause",
            "goal_resume",
            "goal_status",
            "goal_delete",
            "goal_list_suggestions",
        ] {
            assert_eq!(required_fields(tool(&tools, name)), vec!["id"]);
        }

        assert_eq!(
            required_fields(tool(&tools, "goal_apply_suggestion")),
            vec!["id", "suggestion_id"]
        );
        assert_eq!(
            required_fields(tool(&tools, "goal_reject_suggestion")),
            vec!["id", "suggestion_id"]
        );
        assert_contains(&tool(&tools, "goal_delete").description, "IRRÉVERSIBLE");
        assert_contains(
            &tool(&tools, "goal_apply_suggestion").description,
            "RE-VALIDE",
        );
    }

    fn tool<'a>(tools: &'a [ToolDefinition], name: &str) -> &'a ToolDefinition {
        tools
            .iter()
            .find(|tool| tool.name == name)
            .unwrap_or_else(|| panic!("{name} should be registered"))
    }

    fn required_fields(tool: &ToolDefinition) -> Vec<String> {
        required_fields_from(&tool.input_schema)
    }

    fn required_fields_from(schema: &Value) -> Vec<String> {
        schema
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

    fn integer_field(value: &Value, name: &str) -> Option<u64> {
        value.get(name).and_then(Value::as_u64)
    }

    fn assert_contains(haystack: &str, needle: &str) {
        assert!(
            haystack.contains(needle),
            "expected `{haystack}` to contain `{needle}`"
        );
    }
}
