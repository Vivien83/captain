//! Static MCP integration tool definitions.

use captain_types::tool::ToolDefinition;

pub fn mcp_tool_definitions() -> Vec<ToolDefinition> {
    vec![
        ToolDefinition {
            name: "mcp_catalog_search".to_string(),
            description: "[MCP — CATALOGUE] Cherche les templates d'intégrations MCP packagées disponibles dans Captain (GitHub, Notion, MemPalace, Context-like services, bases de données, cloud, etc.) avec leur statut, transport et variables d'environnement requises. À utiliser avant d'écrire un [[mcp_servers]] manuel ou de lancer une commande shell. Retourne uniquement des métadonnées et indique l'étape suivante.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "query": { "type": "string", "description": "Recherche optionnelle par service, capacité ou tag. Vide = liste les templates principaux." },
                    "limit": { "type": "integer", "minimum": 1, "maximum": 50, "description": "Nombre max de résultats. Défaut: 10." }
                }
            }),
        },
        ToolDefinition {
            name: "mcp_integration_install".to_string(),
            description: "[MCP — INSTALL PACKAGÉE] Installe une intégration MCP depuis un template Captain connu, stocke les credentials fournis dans le vault/résolveur, écrit integrations.toml, puis tente un hot-reload MCP. Préférer cet outil à shell_exec('captain add ...') et à l'édition manuelle de [[mcp_servers]]. Ne jamais mettre de secret dans les args/config/docs; fournir les valeurs seulement dans credentials.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "id": { "type": "string", "description": "ID du template retourné par mcp_catalog_search, ex: github, notion, mempalace." },
                    "credentials": { "type": "object", "description": "Objet env_var -> valeur secrète, ex: {\"GITHUB_PERSONAL_ACCESS_TOKEN\":\"...\"}. Si le template n'a qu'une clé requise, alias acceptés: api_key, token, key, value. L'outil ne renvoie jamais les valeurs." },
                    "reload": { "type": "boolean", "description": "Tente de connecter l'intégration MCP immédiatement après installation. Défaut: true." }
                },
                "required": ["id"]
            }),
        },
        ToolDefinition {
            name: "mcp_status".to_string(),
            description: "[MCP — VERIFY] Liste la configuration MCP effective, les serveurs connectés et les tools mcp_* visibles depuis le runtime courant. À appeler après mcp_integration_install ou une modification de config pour vérifier avant d'annoncer le succès.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {}
            }),
        },
    ]
}
