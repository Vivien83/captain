//! Static location/meta tool definitions.

use captain_types::tool::ToolDefinition;

pub fn location_tool_definitions() -> Vec<ToolDefinition> {
    vec![ToolDefinition {
        name: "location_get".to_string(),
        description: "Obtient la localisation géographique approximative basée sur l'adresse IP du serveur. Utiliser pour connaître le fuseau horaire, la ville et le pays avant de planifier des événements ou personnaliser du contenu. Ne pas utiliser pour la localisation précise d'un utilisateur. Retourne ville, pays, coordonnées GPS et timezone.".to_string(),
        input_schema: serde_json::json!({
            "type": "object",
            "properties": {}
        }),
    }]
}
