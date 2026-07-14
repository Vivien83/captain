//! Static session recall and workspace access tool definitions.

use captain_types::tool::ToolDefinition;

pub fn session_workspace_tool_definitions() -> Vec<ToolDefinition> {
    vec![
        ToolDefinition {
            name: "session_recall".to_string(),
            description: "[CROSS-SESSION] Cherche dans les résumés (checkpoint.md) des sessions précédentes de Captain. À utiliser SPONTANÉMENT — sans qu'on te le demande — quand l'utilisateur dit 'on avait dit', 'l'autre fois', 'tu m'avais dit', 'rappelle-moi ce qu'on a fait sur X', ou fait référence à une conversation passée. Les checkpoints sont produits en arrière-plan par un job Haiku qui résume chaque session inactive en 5 sections : Sujets, Décisions, Erreurs / Échecs, Réussites, Infos durables. Retourne les sessions correspondantes triées par fraîcheur. EXEMPLE : session_recall({\"query\":\"deployment script\"}) — retourne les checkpoints qui mentionnent ces mots.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "query": { "type": "string", "description": "Mots-clés à rechercher (insensibles à la casse, multi-mots ANDés)" },
                    "max_results": { "type": "integer", "description": "Nombre max de checkpoints à retourner (défaut 5, max 20)" },
                    "agent_filter": { "type": "string", "description": "Optionnel — restreint aux checkpoints d'un agent (ex: 'daemon-67eae65a-...'). Par défaut tous agents." }
                },
                "required": ["query"]
            }),
        },
        ToolDefinition {
            name: "session_tool_call_summary".to_string(),
            description: "[VÉRIFICATION] Retourne les outils réellement exécutés dans TA session courante (source: le même journal que `captain replay`), avec compteurs par outil et horodatages. Utiliser AVANT d'affirmer qu'une capacité a été testée dans un rapport, un résumé, ou une réponse à l'utilisateur — ne jamais écrire \"testé avec succès\" ou \"échoué\" pour un outil sans vérifier ici qu'il apparaît bien dans `distinct_tools_called` pour cette session. Ne prend aucun identifiant de session en paramètre: il s'applique toujours à ta propre session en cours. EXEMPLE : session_tool_call_summary({}) — retourne distinct_tools_called, call_counts, calls.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "limit": { "type": "integer", "description": "Nombre max d'évènements de session à scanner (défaut 200, max 2000)" }
                }
            }),
        },
        ToolDefinition {
            name: "workspace_add".to_string(),
            description: "[ACCÈS] Étend le sandbox de Captain à un dossier additionnel. À utiliser SPONTANÉMENT quand l'utilisateur dit 'donne-toi accès à X', 'ouvre Y comme workspace', 'travaille sur le dossier Z'. Le path est canonicalisé et persisté dans config.toml ([workspace] extra_paths). Refusé si le path tombe dans une zone protégée (~/.ssh, ~/.gnupg). N'a d'effet que pour l'agent principal Captain. EXEMPLE : workspace_add({\"path\":\"/home/user/projects/example-service\"}).".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "path": { "type": "string", "description": "Chemin absolu d'un dossier existant à autoriser" }
                },
                "required": ["path"]
            }),
        },
    ]
}
