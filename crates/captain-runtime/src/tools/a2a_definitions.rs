//! Static frozen A2A outbound tool definitions.

use captain_types::tool::ToolDefinition;

pub fn a2a_tool_definitions() -> Vec<ToolDefinition> {
    vec![
        ToolDefinition {
            name: "a2a_discover".to_string(),
            description: "Découvre un agent A2A externe en récupérant sa carte agent depuis une URL. Utiliser pour explorer les capacités d'un agent distant avant de lui envoyer des tâches via a2a_send. Compatible avec le protocole Google A2A. Ne pas utiliser pour des agents internes — utiliser agent_list à la place. Retourne le nom, la description, les skills et les protocoles supportés de l'agent.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "url": { "type": "string", "description": "Base URL of the remote Captain/A2A-compatible agent (e.g., 'https://agent.example.com')" }
                },
                "required": ["url"]
            }),
        },
        ToolDefinition {
            name: "a2a_send".to_string(),
            description: "Envoie une tâche ou un message à un agent A2A externe et attend sa réponse. Utiliser agent_name pour envoyer à un agent précédemment découvert via a2a_discover, ou agent_url pour un adressage direct. Supporte les conversations multi-tours via session_id. Ne pas utiliser pour la communication entre agents internes — utiliser agent_send à la place. Retourne la réponse de l'agent distant.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "message": { "type": "string", "description": "The task/message to send to the remote agent" },
                    "agent_url": { "type": "string", "description": "Direct URL of the remote agent's A2A endpoint" },
                    "agent_name": { "type": "string", "description": "Name of a previously discovered A2A agent (looked up from kernel)" },
                    "session_id": { "type": "string", "description": "Optional session ID for multi-turn conversations" }
                },
                "required": ["message"]
            }),
        },
    ]
}
