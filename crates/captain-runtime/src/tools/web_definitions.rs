//! Static network/web tool definitions.

use captain_types::tool::ToolDefinition;

pub fn web_tool_definitions() -> Vec<ToolDefinition> {
    vec![
        ToolDefinition {
            name: "web_citation_audit".to_string(),
            description: "[AUDIT DE CITATIONS] Audite un brouillon après la recherche : refetch en parallèle les URLs citées, vérifie que chaque lien vient du jeu de sources, que les extraits fournis apparaissent réellement dans les pages et que la couverture de provenance atteint le seuil demandé. Retourne un bloc Sources canonique. Une réussite prouve la récupération et la présence verbatim, pas que la source implique sémantiquement l'affirmation. Pour un fait important, litigieux ou à enjeu, appeler cet outil après web_research_batch et avant la réponse finale ; corriger puis relancer si valid=false.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "additionalProperties": false,
                "properties": {
                    "draft": { "type": "string", "description": "Brouillon avec citations Markdown inline exactes, par exemple [Source](https://example.com), et [unverified] pour les affirmations non sourçables." },
                    "sources": {
                        "type": "array",
                        "minItems": 1,
                        "maxItems": 12,
                        "items": {
                            "type": "object",
                            "additionalProperties": false,
                            "properties": {
                                "url": { "type": "string", "description": "URL citation_ready issue de web_research_batch." },
                                "title": { "type": "string", "description": "Libellé humain de la source." },
                                "quotes": { "type": "array", "maxItems": 3, "items": { "type": "string" }, "description": "Extraits exacts copiés depuis le contenu récupéré, jamais depuis un snippet de recherche." }
                            },
                            "required": ["url"]
                        }
                    },
                    "min_coverage": { "type": "number", "minimum": 0, "maximum": 1, "description": "Part minimale des phrases de prose portant une citation ou [unverified]. Défaut 0.5." },
                    "require_evidence": { "type": "boolean", "description": "Exiger au moins un extrait verbatim par source citée. Défaut true." }
                },
                "required": ["draft", "sources"]
            }),
        },
        ToolDefinition {
            name: "web_research_batch".to_string(),
            description: "[RECHERCHE WEB GROUPEE] Exécute en parallèle jusqu'à cinq recherches indépendantes puis récupère en parallèle un ensemble borné de pages. Retourne des sources dédupliquées avec ID stable, provenance de découverte, URL finale, statut HTTP, date, SHA-256, preview et citation_markdown uniquement quand la page a réellement été lue. Les snippets restent discovery_only et ne doivent jamais être cités. À utiliser avant toute recherche/synthèse ; pour les affirmations importantes, terminer avec web_citation_audit.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "query": { "type": "string", "description": "Requête unique." },
                    "queries": { "type": "array", "items": { "type": "string" }, "description": "Jusqu'à 5 requêtes." },
                    "urls": { "type": "array", "items": { "type": "string" }, "description": "URLs explicites à fetch en plus/à la place des résultats." },
                    "max_results": { "type": "integer", "description": "Résultats par requête, défaut 5, max 10." },
                    "auto_fetch": { "type": "boolean", "description": "Fetch automatiquement les URLs trouvées dans les résultats. Défaut true." },
                    "max_fetches": { "type": "integer", "description": "Nombre max de fetches, défaut 5, max 10." },
                    "preview_chars": { "type": "integer", "description": "Taille de preview par résultat/fetch, défaut 3000." }
                }
            }),
        },
        ToolDefinition {
            name: "web_download".to_string(),
            description: "[TELECHARGEMENT SOURCE] Télécharge une URL externe dans le workspace avec protection anti-SSRF et taille bornée. À utiliser pour PDF, rapports, CSV, datasets légers ou fichiers que web_fetch ne peut pas rendre correctement. Retourne le chemin local, le type MIME, la taille, le SHA-256 et le prochain outil conseillé, typiquement document_extract pour un PDF/texte. Ne pas utiliser pour des pages HTML simples lisibles avec web_fetch.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "url": { "type": "string", "description": "URL http/https externe à télécharger. Les IP privées/loopback et redirects vers IP privées sont refusés." },
                    "path": { "type": "string", "description": "Chemin de sortie relatif au workspace. Défaut: downloads/<nom-detecté>." },
                    "max_bytes": { "type": "integer", "description": "Taille max acceptée. Défaut 25MB, max 100MB." },
                    "overwrite": { "type": "boolean", "description": "Remplacer le fichier s'il existe déjà. Défaut false." }
                },
                "required": ["url"]
            }),
        },
        ToolDefinition {
            name: "web_fetch".to_string(),
            description: "Effectue une requête HTTP vers une URL externe avec protection anti-SSRF (IPs privées bloquées). Utiliser pour consommer des APIs REST publiques, récupérer des pages web ou envoyer des données à des services tiers sans secret brut. Ne pas utiliser pour accéder à des ressources internes du réseau local — utiliser les outils dédiés (agent_send, config_read, etc.) à la place. Les URL/headers/body contenant une clé API en clair sont refusés: utiliser une intégration native ou un skill env_inject. Pour les GET, le HTML est automatiquement converti en Markdown lisible ; pour les autres méthodes, retourne le corps brut de la réponse.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "url": { "type": "string", "description": "URL complète à contacter (http/https uniquement, IPs privées refusées)" },
                    "method": { "type": "string", "enum": ["GET","POST","PUT","PATCH","DELETE"], "description": "Méthode HTTP à utiliser (défaut : GET)" },
                    "headers": { "type": "object", "description": "En-têtes HTTP personnalisés sous forme de paires clé-valeur (ex: Authorization, Content-Type)" },
                    "body": { "type": "string", "description": "Corps de la requête pour POST/PUT/PATCH (JSON, form-data ou texte brut)" }
                },
                "required": ["url"]
            }),
        },
        ToolDefinition {
            name: "web_search".to_string(),
            description: "Recherche des informations sur le web via plusieurs fournisseurs (Tavily, Brave, Perplexity, DuckDuckGo) avec basculement automatique. Utiliser pour trouver des informations récentes, de la documentation ou vérifier des faits. Ne pas utiliser pour accéder à une URL précise (utiliser web_fetch) ni pour des données internes au système. Retourne une liste structurée avec titre, URL et extrait pour chaque résultat.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "query": { "type": "string", "description": "Requête de recherche en langage naturel ou mots-clés (ex: 'meilleure lib Rust pour HTTP async 2025')" },
                    "max_results": { "type": "integer", "description": "Nombre maximum de résultats à retourner (défaut : 5, max : 20)" }
                },
                "required": ["query"]
            }),
        },
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn web_research_tools_expose_grounding_contracts() {
        let tools = web_tool_definitions();
        let audit = tools
            .iter()
            .find(|tool| tool.name == "web_citation_audit")
            .expect("citation audit must be registered");
        let research = tools
            .iter()
            .find(|tool| tool.name == "web_research_batch")
            .expect("grouped research must be registered");

        assert_eq!(audit.input_schema["properties"]["sources"]["maxItems"], 12);
        assert!(audit.description.contains("pas que la source implique"));
        assert!(research.description.contains("discovery_only"));
        assert!(research.description.contains("SHA-256"));
    }
}
