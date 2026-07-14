//! Self-update tool definition.

use captain_types::tool::ToolDefinition;

pub fn update_tool_definitions() -> Vec<ToolDefinition> {
    vec![ToolDefinition {
        name: "system_update".to_string(),
        description: "Met à jour Captain lui-même vers la dernière version publiée (téléchargement, vérification, remplacement du binaire, redémarrage du daemon). Utiliser quand l'utilisateur demande de mettre Captain à jour. Avec check_only=true, vérifie seulement si une nouvelle version est disponible sans rien installer. La mise à jour réelle exige toujours une approbation utilisateur et redémarre le daemon : prévenir l'utilisateur que la session sera brièvement interrompue.".to_string(),
        input_schema: serde_json::json!({
            "type": "object",
            "properties": {
                "check_only": {
                    "type": "boolean",
                    "description": "true = vérifier la disponibilité d'une mise à jour sans l'installer (default: false)"
                }
            },
            "required": []
        }),
    }]
}
