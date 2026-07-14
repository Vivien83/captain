//! Static knowledge graph tool definitions.

use captain_types::tool::ToolDefinition;

pub fn knowledge_tool_definitions() -> Vec<ToolDefinition> {
    vec![
        ToolDefinition {
            name: "knowledge_add_entity".to_string(),
            description: "Ajoute une entité au graphe de connaissances. Les entités représentent des personnes, organisations, projets, concepts, lieux, outils, etc. Utiliser pour enrichir la mémoire structurée de l'agent. Vérifier que l'entité n'existe pas déjà via knowledge_query avant de créer un doublon. Retourne l'ID de l'entité créée.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "name": { "type": "string", "description": "Display name of the entity" },
                    "entity_type": { "type": "string", "description": "Type: person, organization, project, concept, event, location, document, tool, or a custom type" },
                    "properties": { "type": "object", "description": "Arbitrary key-value properties (optional)" }
                },
                "required": ["name", "entity_type"]
            }),
        },
        ToolDefinition {
            name: "knowledge_add_relation".to_string(),
            description: "Crée une relation typée entre deux entités existantes dans le graphe de connaissances. Utiliser pour structurer les connexions entre concepts (works_at, knows_about, depends_on, etc.). Les deux entités doivent exister — les créer d'abord via knowledge_add_entity si nécessaire. Retourne la relation créée avec son score de confiance.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "source": { "type": "string", "description": "Source entity ID or name" },
                    "relation": { "type": "string", "description": "Relation type: works_at, knows_about, related_to, depends_on, owned_by, created_by, located_in, part_of, uses, produces, or a custom type" },
                    "target": { "type": "string", "description": "Target entity ID or name" },
                    "confidence": { "type": "number", "description": "Confidence score 0.0-1.0 (default: 1.0)" },
                    "properties": { "type": "object", "description": "Arbitrary key-value properties (optional)" }
                },
                "required": ["source", "relation", "target"]
            }),
        },
        ToolDefinition {
            name: "knowledge_query".to_string(),
            description: "Interroge le graphe de connaissances avec filtrage par entité source, type de relation et/ou entité cible. Utiliser pour retrouver des informations structurées, explorer les connexions d'une entité ou vérifier l'existence d'une relation. Supporte la traversée multi-niveaux via max_depth. Retourne les triplets entité-relation-entité correspondants.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "source": { "type": "string", "description": "Filter by source entity name or ID (optional)" },
                    "relation": { "type": "string", "description": "Filter by relation type (optional)" },
                    "target": { "type": "string", "description": "Filter by target entity name or ID (optional)" },
                    "max_depth": { "type": "integer", "description": "Maximum traversal depth (default: 1)" }
                }
            }),
        },
    ]
}
