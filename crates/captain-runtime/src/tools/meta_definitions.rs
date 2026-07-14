//! Static meta/introspection tool definitions.

use captain_types::tool::ToolDefinition;

pub fn meta_tool_definitions() -> Vec<ToolDefinition> {
    vec![
        ToolDefinition {
            name: "system_time".to_string(),
            description: "Retourne la date et l'heure courantes du système. Utiliser avant toute planification avec `cron_create` ou `reminder_set` pour connaître l'heure locale exacte. Ne pas utiliser si la date est déjà connue du contexte. Retourne un objet JSON avec utc (ISO 8601), local (ISO 8601 avec offset), unix_epoch (entier), timezone (ex: CET) et utc_offset (ex: +01:00).".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {},
                "required": []
            }),
        },
        ToolDefinition {
            name: "canvas_present".to_string(),
            description: "Présente un canvas HTML interactif à l'utilisateur sous forme d'artefact sauvegardé. Le HTML est assaini (pas de scripts ni event handlers) et sauvegardé dans le workspace. Utiliser pour des visualisations de données riches, des rapports formatés ou des interfaces UI personnalisées. Pour du texte simple, préférer une réponse directe. Retourne le chemin du fichier HTML sauvegardé.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "html": { "type": "string", "description": "The HTML content to present. Must not contain <script> tags, event handlers, or javascript: URLs." },
                    "title": { "type": "string", "description": "Optional title for the canvas panel" }
                },
                "required": ["html"]
            }),
        },
    ]
}
