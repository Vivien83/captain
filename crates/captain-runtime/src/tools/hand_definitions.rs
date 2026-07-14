//! Static frozen Hand tool definitions.

use captain_types::tool::ToolDefinition;

pub fn hand_tool_definitions() -> Vec<ToolDefinition> {
    vec![
        ToolDefinition {
            name: "hand_list".to_string(),
            description: "Liste toutes les Hands disponibles dans Captain avec leur statut d'activation. Une Hand est un agent spécialisé autonome pré-configuré (ex: researcher, browser, clip). Utiliser pour découvrir les Hands disponibles avant d'en activer une via `hand_activate`. Retourne un tableau JSON avec id, name, description, status (active/inactive) et capabilities.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {}
            }),
        },
        ToolDefinition {
            name: "hand_activate".to_string(),
            description: "Active une Hand et instancie son agent autonome spécialisé avec ses outils et skills pré-configurés. Utiliser quand une tâche requiert les capacités d'une Hand spécifique (ex: recherche web approfondie, navigation, clip). Appeler `hand_list` d'abord pour vérifier les IDs disponibles. Retourne l'instance_id de l'agent créé.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "hand_id": { "type": "string", "description": "Identifiant de la Hand à activer, tel que retourné par `hand_list` (ex: 'researcher', 'clip', 'browser')" },
                    "config": { "type": "object", "description": "Surcharges optionnelles de configuration pour les paramètres de la Hand (ex: model, temperature, system_prompt)" }
                },
                "required": ["hand_id"]
            }),
        },
        ToolDefinition {
            name: "hand_status".to_string(),
            description: "Vérifie le statut et les métriques d'une Hand active (uptime, nombre de requêtes traitées, dernière activité). Utiliser pour surveiller la santé d'une Hand ou décider si elle doit être redémarrée. Retourne un objet JSON avec statut, métriques et dernière activité.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "hand_id": { "type": "string", "description": "The ID of the hand to check status for" }
                },
                "required": ["hand_id"]
            }),
        },
        ToolDefinition {
            name: "hand_deactivate".to_string(),
            description: "Désactive une Hand en cours d'exécution et arrête son agent associé. Utiliser pour libérer les ressources d'une Hand qui n'est plus nécessaire. L'instance_id est obtenu via hand_activate ou hand_status. L'opération est irréversible — réactiver via hand_activate si besoin. Retourne une confirmation de désactivation.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "instance_id": { "type": "string", "description": "The UUID of the hand instance to deactivate" }
                },
                "required": ["instance_id"]
            }),
        },
        ToolDefinition {
            name: "scaffold_hand".to_string(),
            description: "Crée une nouvelle Hand (package agent autonome) avec une structure lean. Génère HAND.toml + workspace + fichiers d'identité utiles (SOUL.md, IDENTITY.md). La mémoire durable passe par MemPalace/tools, pas par MEMORY.md. La Hand est immédiatement disponible après création. Retourne le chemin du répertoire créé.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "id": { "type": "string", "description": "Unique hand ID (lowercase, no spaces, e.g. 'family-hand')" },
                    "name": { "type": "string", "description": "Human-readable name (e.g. 'OpsHand')" },
                    "description": { "type": "string", "description": "What this hand does (1-2 sentences)" },
                    "category": { "type": "string", "enum": ["content", "trading", "social", "analytics", "automation", "personal", "research", "security"], "description": "Category" },
                    "icon": { "type": "string", "description": "Emoji icon (e.g. '👨‍👩‍👦')" },
                    "tools": { "type": "array", "items": { "type": "string" }, "description": "Tools this hand needs (e.g. ['shell_exec', 'file_read', 'channel_send'])" }
                },
                "required": ["id", "name", "description"]
            }),
        },
    ]
}
