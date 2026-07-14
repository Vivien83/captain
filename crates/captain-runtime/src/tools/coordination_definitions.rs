//! Static cross-agent coordination tool definitions.

use captain_types::tool::ToolDefinition;
use serde_json::Value;

pub fn coordination_tool_definitions() -> Vec<ToolDefinition> {
    let mut definitions = agent_discovery_tool_definitions();
    definitions.extend(task_queue_tool_definitions());
    definitions.extend(event_bus_tool_definitions());
    definitions.extend(user_question_tool_definitions());
    definitions
}

fn agent_discovery_tool_definitions() -> Vec<ToolDefinition> {
    vec![agent_find_tool_definition()]
}

fn task_queue_tool_definitions() -> Vec<ToolDefinition> {
    vec![
        task_post_tool_definition(),
        task_claim_tool_definition(),
        task_complete_tool_definition(),
        task_list_tool_definition(),
    ]
}

fn event_bus_tool_definitions() -> Vec<ToolDefinition> {
    vec![event_publish_tool_definition()]
}

fn user_question_tool_definitions() -> Vec<ToolDefinition> {
    vec![ask_user_tool_definition()]
}

fn tool_definition(name: &str, description: &str, input_schema: Value) -> ToolDefinition {
    ToolDefinition {
        name: name.to_string(),
        description: description.to_string(),
        input_schema,
    }
}

fn agent_find_tool_definition() -> ToolDefinition {
    tool_definition(
        "agent_find",
        "Recherche des agents par nom, tag, outil ou description dans le runtime. Utiliser pour trouver un agent spécialisé avant de lui déléguer du travail via agent_send. Supporte la recherche floue sur les métadonnées. Ne pas utiliser pour lister tous les agents — utiliser agent_list pour une vue exhaustive. Retourne les agents correspondants avec leur id, nom, description et outils.",
        serde_json::json!({
            "type": "object",
            "properties": {
                "query": { "type": "string", "description": "Search query (matches agent name, tags, tools, description)" }
            },
            "required": ["query"]
        }),
    )
}

fn task_post_tool_definition() -> ToolDefinition {
    tool_definition(
        "task_post",
        "Publie une tâche dans la file partagée pour qu'un autre agent la prenne en charge. Utiliser pour orchestrer du travail asynchrone entre agents sans attendre de réponse immédiate. Optionnellement assigner à un agent spécifique. Ne pas utiliser pour une communication synchrone — utiliser agent_send à la place. Retourne l'ID de la tâche créée.",
        serde_json::json!({
            "type": "object",
            "properties": {
                "title": { "type": "string", "description": "Short task title" },
                "description": { "type": "string", "description": "Detailed task description" },
                "assigned_to": { "type": "string", "description": "Agent name or ID to assign the task to (optional)" }
            },
            "required": ["title", "description"]
        }),
    )
}

fn task_claim_tool_definition() -> ToolDefinition {
    tool_definition(
        "task_claim",
        "Réclame la prochaine tâche disponible dans la file partagée, assignée à cet agent ou non assignée. Utiliser dans une boucle de travail pour traiter les tâches en attente. L'état de la tâche passe de 'pending' à 'in_progress'. Ne prend aucun paramètre. Retourne la tâche réclamée avec son id, titre et description, ou null si aucune tâche disponible.",
        serde_json::json!({
            "type": "object",
            "properties": {}
        }),
    )
}

fn task_complete_tool_definition() -> ToolDefinition {
    tool_definition(
        "task_complete",
        "Marque une tâche précédemment réclamée comme terminée avec un résultat. Utiliser après avoir traité une tâche obtenue via task_claim. Le résultat est persisté et consultable par l'agent qui a posté la tâche. Ne pas utiliser sur une tâche non réclamée par cet agent. Retourne une confirmation de complétion.",
        serde_json::json!({
            "type": "object",
            "properties": {
                "task_id": { "type": "string", "description": "The task ID to complete" },
                "result": { "type": "string", "description": "The result or outcome of the task" }
            },
            "required": ["task_id", "result"]
        }),
    )
}

fn task_list_tool_definition() -> ToolDefinition {
    tool_definition(
        "task_list",
        "Liste les tâches dans la file partagée avec filtrage optionnel par statut (pending, in_progress, completed). Utiliser pour surveiller l'avancement des tâches ou trouver des tâches en attente. Retourne un tableau d'objets avec id, titre, description, statut, agent assigné et timestamps.",
        serde_json::json!({
            "type": "object",
            "properties": {
                "status": { "type": "string", "description": "Filter by status: pending, in_progress, completed (optional)" }
            }
        }),
    )
}

fn event_publish_tool_definition() -> ToolDefinition {
    tool_definition(
        "event_publish",
        "Publie un événement personnalisé sur le bus d'événements du runtime, pouvant déclencher des agents proactifs en écoute. Utiliser pour signaler un changement d'état, une alerte ou coordonner un flux multi-agents. Ne pas utiliser pour la communication directe entre deux agents — utiliser agent_send à la place. Retourne une confirmation de publication.",
        serde_json::json!({
            "type": "object",
            "properties": {
                "event_type": { "type": "string", "description": "Type identifier for the event (e.g., 'code_review_requested')" },
                "payload": { "type": "object", "description": "JSON payload data for the event" }
            },
            "required": ["event_type"]
        }),
    )
}

fn ask_user_tool_definition() -> ToolDefinition {
    tool_definition(
        "ask_user",
        "Pose une question à l'utilisateur et attend sa réponse avant de continuer. Utiliser quand une clarification est indispensable, qu'il existe plusieurs chemins valides, ou que la préférence de l'utilisateur est déterminante. Peut proposer des options prédéfinies pour une sélection rapide. Ne pas utiliser pour des questions rhétoriques ou quand la réponse peut être inférée du contexte — agir directement dans ce cas. Retourne la réponse textuelle de l'utilisateur.",
        serde_json::json!({
            "type": "object",
            "properties": {
                "question": { "type": "string", "description": "La question à poser à l'utilisateur. Être naturel et conversationnel. Une seule question par appel." },
                "options": {
                    "type": "array",
                    "items": { "type": "string" },
                    "description": "Choix prédéfinis optionnels présentés comme boutons. Si fournis, l'utilisateur peut cliquer plutôt que saisir. Ex: [\"Oui\", \"Non\", \"Plus tard\"]."
                }
            },
            "required": ["question"]
        }),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn coordination_tool_definitions_keep_public_order() {
        let tools = coordination_tool_definitions();
        let names: Vec<_> = tools.iter().map(|tool| tool.name.as_str()).collect();

        assert_eq!(
            names,
            vec![
                "agent_find",
                "task_post",
                "task_claim",
                "task_complete",
                "task_list",
                "event_publish",
                "ask_user",
            ]
        );
    }

    #[test]
    fn coordination_tool_definitions_keep_task_queue_contracts() {
        let tools = coordination_tool_definitions();
        let task_post = tool(&tools, "task_post");
        let task_claim = tool(&tools, "task_claim");
        let task_complete = tool(&tools, "task_complete");
        let task_list = tool(&tools, "task_list");

        assert_eq!(required_fields(task_post), vec!["title", "description"]);
        assert!(property(task_post, "assigned_to").is_object());
        assert_contains(&task_post.description, "agent_send à la place");

        assert!(required_fields(task_claim).is_empty());
        assert_eq!(properties_len(task_claim), Some(0));
        assert_contains(&task_claim.description, "pending");
        assert_contains(&task_claim.description, "in_progress");

        assert_eq!(required_fields(task_complete), vec!["task_id", "result"]);
        assert_contains(&task_complete.description, "non réclamée");

        assert!(required_fields(task_list).is_empty());
        assert_contains(
            property(task_list, "status")["description"]
                .as_str()
                .unwrap_or_default(),
            "pending, in_progress, completed",
        );
    }

    #[test]
    fn coordination_tool_definitions_keep_agent_event_and_user_contracts() {
        let tools = coordination_tool_definitions();
        let agent_find = tool(&tools, "agent_find");
        let event_publish = tool(&tools, "event_publish");
        let ask_user = tool(&tools, "ask_user");

        assert_eq!(required_fields(agent_find), vec!["query"]);
        assert_contains(&agent_find.description, "agent_list");

        assert_eq!(required_fields(event_publish), vec!["event_type"]);
        assert_eq!(property(event_publish, "payload")["type"], "object");
        assert_contains(&event_publish.description, "agent_send à la place");

        assert_eq!(required_fields(ask_user), vec!["question"]);
        assert_eq!(items_type(property(ask_user, "options")), "string");
        assert_contains(&ask_user.description, "questions rhétoriques");
        assert_contains(
            property(ask_user, "question")["description"]
                .as_str()
                .unwrap_or_default(),
            "Une seule question par appel",
        );
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

    fn items_type(value: &Value) -> &str {
        value
            .get("items")
            .and_then(|items| items.get("type"))
            .and_then(Value::as_str)
            .unwrap_or_default()
    }

    fn assert_contains(haystack: &str, needle: &str) {
        assert!(
            haystack.contains(needle),
            "expected `{haystack}` to contain `{needle}`"
        );
    }
}
