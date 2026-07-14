//! Static frozen peer-federation tool definitions.

use captain_types::tool::ToolDefinition;

pub fn peer_tool_definitions() -> Vec<ToolDefinition> {
    vec![ToolDefinition {
        name: "peer_list".to_string(),
        description: "Liste les Captains pairs découverts automatiquement sur le LAN via mDNS (broadcast _captain._tcp.local) — chaque entrée contient son nom + sa carte agent A2A. Utiliser pour vérifier la fédération avant d'envoyer une tâche cross-instance avec a2a_send. La découverte se fait au boot et en continu ; les pairs sont auto-filtrés contre les self-broadcasts via l'instance_id du daemon.".to_string(),
        input_schema: serde_json::json!({"type": "object", "properties": {}}),
    }]
}
