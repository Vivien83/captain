//! Static skill execution and authoring tool definitions.

use captain_types::tool::ToolDefinition;

pub fn skill_tool_definitions() -> Vec<ToolDefinition> {
    vec![
        ToolDefinition {
            name: "skill_execute".to_string(),
            description: "Exécute une capacité définie dans un fichier skill .md (section ### heading). Avant de lancer bash, Captain exécute un préflight syntaxique sans effet de bord (`bash -n`) et bloque la capability si elle est invalide. Les credentials sont automatiquement injectés depuis la config. Les tokens issus d'une capability 'login' sont mis en cache pour les appels suivants. Utiliser pour déclencher des intégrations tierces sans gérer l'authentification manuellement. Ne pas utiliser pour des actions non définies dans un skill — créer d'abord la capability dans le .md. Retourne la sortie de l'exécution ou un blocage JSON actionnable.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "skill": { "type": "string", "description": "Nom du skill, correspondant au fichier .md dans le répertoire skills (sans extension). Ex: 'cal', 'notion', 'github'." },
                    "capability": { "type": "string", "description": "Nom de la capacité à exécuter, correspondant au titre ### dans le fichier skill (ex: 'login', 'list_slots', 'create_event')" },
                    "args": { "type": "object", "description": "Arguments supplémentaires injectés comme variables d'environnement dans le script (ex: {\"DATE\": \"2025-01-15\", \"TITLE\": \"Réunion\"})" }
                },
                "required": ["skill", "capability"]
            }),
        },
        ToolDefinition {
            name: "scaffold_skill".to_string(),
            description: "[EXTENSIBILITÉ CONTRÔLÉE] Crée un nouveau Skill (fichier .md avec capabilities) dans le workspace de l'agent. Génère skills/{name}/SKILL.md avec frontmatter YAML et sections de capabilities. À utiliser uniquement quand l'utilisateur demande explicitement un scaffold manuel. Les workflows observés spontanément appartiennent à Skill Learning V2: inspecte-les avec self_improvement_review / workflow_learning_list et laisse leur carte authentifiée gérer test, activation et rollback; ne contourne jamais ce lifecycle avec scaffold_skill. Les credentials doivent venir du vault/env_inject, jamais du texte brut. Retourne le chemin du fichier créé.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "name": { "type": "string", "description": "Skill name (lowercase, e.g. 'status-checker')" },
                    "description": { "type": "string", "description": "What the skill does" },
                    "capabilities": { "type": "array", "items": { "type": "string" }, "description": "List of capability names (e.g. ['check_status', 'summarize_incident'])" }
                },
                "required": ["name", "description"]
            }),
        },
    ]
}
