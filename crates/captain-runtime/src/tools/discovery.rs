//! Deferred builtin tool discovery.
//!
//! This module owns the `tool_search` semantics so the main tool runner can
//! stay focused on dispatch and policy.

use captain_types::tool::ToolDefinition;
use serde_json::Value;

use super::ToolRegistry;

const TOOL_SEARCH_NO_MATCH_HINT: &str =
    "tool_search only searches deferred builtin Captain tools. If the capability may come from \
     an installed skill, MCP server, or docs family, call capability_search with concrete \
     keywords. Use captain_docs for builtin tool behaviour and recovery contracts.";

const TOOL_SEARCH_EMPTY_QUERY_HINT: &str =
    "Provide capability keywords, or use select:name1,name2 when the exact deferred builtin tool \
     names are already known.";

/// Score a tool's lexical relevance to the query tokens.
///
/// Token in `name` weighs 2; token in `description` weighs 1. Sum across
/// all tokens. Case-insensitive. No regex — purely substring.
pub(crate) fn lexical_tool_score(query_tokens: &[String], tool: &ToolDefinition) -> u32 {
    let name_lower = tool.name.to_lowercase();
    let desc_lower = tool.description.to_lowercase();
    let mut score = 0u32;
    for tok in query_tokens {
        if tok.is_empty() {
            continue;
        }
        if name_lower.contains(tok) {
            score += 2;
        }
        if desc_lower.contains(tok) {
            score += 1;
        }
    }
    score
}

fn tool_search_json(results: Vec<serde_json::Value>, hint: Option<&'static str>) -> String {
    let mut response = serde_json::Map::new();
    response.insert("results".to_string(), serde_json::Value::Array(results));
    if let Some(hint) = hint {
        response.insert(
            "hint".to_string(),
            serde_json::Value::String(hint.to_string()),
        );
    }
    serde_json::Value::Object(response).to_string()
}

fn tool_result(tool: &ToolDefinition) -> serde_json::Value {
    serde_json::json!({
        "name": tool.name,
        "description": tool.description,
        "input_schema": tool.input_schema,
    })
}

pub fn discovery_tool_definitions() -> Vec<ToolDefinition> {
    let mut definitions = docs_discovery_tool_definitions();
    definitions.extend(capability_discovery_tool_definitions());
    definitions.extend(skill_discovery_tool_definitions());
    definitions.extend(builtin_tool_discovery_definitions());
    definitions
}

fn docs_discovery_tool_definitions() -> Vec<ToolDefinition> {
    vec![captain_docs_tool_definition()]
}

fn capability_discovery_tool_definitions() -> Vec<ToolDefinition> {
    vec![capability_search_tool_definition()]
}

fn skill_discovery_tool_definitions() -> Vec<ToolDefinition> {
    vec![
        skill_search_tool_definition(),
        skill_view_tool_definition(),
        skill_check_tool_definition(),
    ]
}

fn builtin_tool_discovery_definitions() -> Vec<ToolDefinition> {
    vec![tool_search_tool_definition()]
}

fn tool_definition(name: &str, description: &str, input_schema: Value) -> ToolDefinition {
    ToolDefinition {
        name: name.to_string(),
        description: description.to_string(),
        input_schema,
    }
}

fn captain_docs_tool_definition() -> ToolDefinition {
    tool_definition(
        "captain_docs",
        "[RTFM — TON PROPRE MANUEL] Cherche dans la documentation structurée de Captain (docs/captain-tools/) avant de demander à l'utilisateur. À utiliser SPONTANÉMENT — sans qu'on te le demande — quand tu hésites sur le comportement d'un outil, ses paramètres, sa sandbox, ses limites, ou la différence entre deux outils similaires (file_write vs edit_file, memory_save vs memory_store, channel_send vs channel_reconfigure...). Filtre par `family` quand tu sais où chercher (parmi : file, shell-process, network, browser, ssh, memory, skill, channel, agent-coordination, scheduling, config-secret, mcp, knowledge, session-workspace, meta, project, multimedia, document, runtime-changelog) ; avec `family`, la réponse inclut aussi les Live Tool Schemas générés depuis le registre runtime et donc les paramètres exacts à utiliser. Pour expliquer une vraie mise à jour runtime, lis d'abord `family:\"runtime-changelog\"` au lieu de déduire depuis git log. Sinon laisse vide pour scanner toutes les familles. La recherche est multi-mots et insensible à la casse, les termes sont ANDés. Retourne des extraits avec contexte. EXEMPLE : captain_docs({\"query\":\"edit_file fallback strategies\"}) avant de demander comment l'outil fonctionne.",
        serde_json::json!({
            "type": "object",
            "properties": {
                "query": { "type": "string", "description": "Mots-clés à chercher dans la doc (ex: 'edit_file fallback', 'channel_reconfigure rotation')" },
                "family": { "type": "string", "description": "Filtre optionnel par famille de tools (file, shell-process, network, browser, ssh, memory, skill, channel, agent-coordination, scheduling, config-secret, mcp, knowledge, session-workspace, meta, project, multimedia, document, runtime-changelog). Quand fourni, retourne le corps complet de cette famille plus les Live Tool Schemas exacts quand la famille a des tools." },
                "max_results": { "type": "integer", "description": "Nombre max de hits (défaut 5, max 14)" }
            },
            "required": ["query"]
        }),
    )
}

fn capability_search_tool_definition() -> ToolDefinition {
    tool_definition(
        "capability_search",
        "[RESOLVEUR DE CAPACITÉ] Cherche quoi utiliser dans le cœur actif Captain : outils builtin CORE/différés, capacités natives CapSpec déposées en fichier .captain, tools de skills installés, tools MCP connectés, et familles captain_docs. Les surfaces gelées (Hands, A2A, peers, fleets) restent compilées mais ne sont pas proposées par défaut. À utiliser avant de dire \"je ne peux pas\", avant de demander à l'utilisateur quel outil utiliser, ou quand plusieurs surfaces se ressemblent. Retourne des candidats avec source, statut, usage recommandé, schéma quand disponible, et prochaine action. Pour récupérer le schéma exact d'un builtin différé connu, utiliser ensuite tool_search({\"query\":\"select:<nom>\"}) si nécessaire.",
        serde_json::json!({
            "type": "object",
            "properties": {
                "query": {
                    "type": "string",
                    "description": "Besoin ou capacité à résoudre (ex: 'envoyer un message telegram', 'lire une page web', 'deploy via ssh', 'outil projet milestone'). Supporte aussi 'select:name1,name2' pour un lookup exact."
                },
                "sources": {
                    "type": "array",
                    "items": { "type": "string", "enum": ["builtin", "capfile", "skill", "mcp", "docs"] },
                    "description": "Sources optionnelles à limiter. Par défaut: toutes."
                },
                "max_results": {
                    "type": "integer",
                    "description": "Nombre max de candidats (défaut 8, plafonné à 30)",
                    "default": 8
                },
                "include_schemas": {
                    "type": "boolean",
                    "description": "Inclure les input_schema pour les candidats outil. Défaut true pour rendre le résultat actionnable.",
                    "default": true
                }
            },
            "required": ["query"]
        }),
    )
}

fn skill_search_tool_definition() -> ToolDefinition {
    tool_definition(
        "skill_search",
        "[DÉCOUVRE-SKILL] Index court et recherche des skills procéduraux de Captain par famille ou mots-clés. À utiliser quand la tâche ressemble à un workflow réutilisable, à du développement, du debug, une review, du projet, ou quand un sous-agent doit savoir quel guide suivre. Sans query ni family, retourne l'index minimal. Retourne les familles disponibles, les skills pertinents, leur utilité et leurs outils requis/proposés ; charge ensuite le workflow exact avec skill_view.",
        serde_json::json!({
            "type": "object",
            "properties": {
                "query": {
                    "type": "string",
                    "description": "Besoin ou workflow recherché (ex: 'debug test failure', 'plan project', 'review release'). Peut être vide pour obtenir l'index minimal ou si `family` est fourni."
                },
                "family": {
                    "type": "string",
                    "enum": [
                        "software-development",
                        "project-management",
                        "review-release",
                        "platform-devops",
                        "data-ai",
                        "product-design",
                        "business-tools",
                        "security-compliance",
                        "general-automation"
                    ],
                    "description": "Famille optionnelle pour filtrer les skills."
                },
                "max_results": {
                    "type": "integer",
                    "description": "Nombre max de skills (défaut 8, plafonné à 30)",
                    "default": 8
                },
                "include_context": {
                    "type": "boolean",
                    "description": "Inclure un extrait court du SKILL.md/prompt_context pour appliquer le workflow immédiatement. Défaut false.",
                    "default": false
                },
                "include_families": {
                    "type": "boolean",
                    "description": "Inclure le catalogue des familles et leurs compteurs. Défaut true.",
                    "default": true
                }
            }
        }),
    )
}

fn skill_view_tool_definition() -> ToolDefinition {
    tool_definition(
        "skill_view",
        "[CHARGE-SKILL] Charge un skill installé par nom exact après skill_search. Retourne metadata, famille, source, outils requis/proposés, fichiers liés, validation opérationnelle et contexte SKILL.md plafonné. Peut aussi charger un fichier lié via file_path. À utiliser avant d'improviser quand skill_search a trouvé un candidat pertinent.",
        serde_json::json!({
            "type": "object",
            "properties": {
                "name": {
                    "type": "string",
                    "description": "Nom exact du skill à charger, tel que retourné par skill_search."
                },
                "include_context": {
                    "type": "boolean",
                    "description": "Inclure le contexte SKILL.md/prompt_context. Défaut true.",
                    "default": true
                },
                "max_context_chars": {
                    "type": "integer",
                    "description": "Budget maximal de caractères pour prompt_context. Défaut 8000, borné entre 500 et 20000.",
                    "default": 8000
                },
                "file_path": {
                    "type": "string",
                    "description": "Chemin optionnel d'un fichier lié dans le skill, par exemple references/api.md, templates/config.yaml, scripts/validate.py ou assets/example.json. Omis par défaut pour charger le contexte principal."
                }
            },
            "required": ["name"]
        }),
    )
}

fn skill_check_tool_definition() -> ToolDefinition {
    tool_definition(
        "skill_check",
        "[TEST-SKILL] Préflight statique et sans effet de bord d'un skill installé par nom exact. À utiliser après skill_view avant d'exécuter un skill fragile, scripté ou modifié. Retourne pass/warn/fail, reprend la validation file-backed, vérifie les prérequis bloquants et lance `bash -n` sur les blocs bash/sh ou l'entrée shell sans exécuter les commandes.",
        serde_json::json!({
            "type": "object",
            "properties": {
                "name": {
                    "type": "string",
                    "description": "Nom exact du skill à vérifier, tel que retourné par skill_search/skill_view."
                },
                "run_static_tests": {
                    "type": "boolean",
                    "description": "Lancer les tests statiques disponibles, actuellement bash -n sans exécution pour les blocs shell. Défaut true.",
                    "default": true
                },
                "max_shell_blocks": {
                    "type": "integer",
                    "description": "Nombre maximal de blocs bash/sh à vérifier. Défaut 20, borné entre 1 et 50.",
                    "default": 20
                }
            },
            "required": ["name"]
        }),
    )
}

fn tool_search_tool_definition() -> ToolDefinition {
    tool_definition(
        "tool_search",
        "[DÉCOUVRE-OUTIL] Cherche un outil par mots-clés ou par nom exact parmi les tools builtin NON-core du cœur actif (browser_*, image_*, secret_write, project_*, cron_*, etc.). Les surfaces gelées (Hands, A2A, peers, fleets) ne sont pas proposées par défaut. Retourne jusqu'à `max_results` definitions {name, description, input_schema} prêtes à être appelées au tour suivant. À utiliser après capability_search quand le candidat choisi est un builtin différé ou quand tu connais déjà le nom exact. Si tu ne sais pas si la capacité vient d'un builtin, skill, MCP ou docs, commence par capability_search. Supporte aussi 'select:name1,name2' pour récupérer un schema EXACT par nom. La recherche est lexicale (matching insensible à la casse sur name + description, name pèse double).",
        serde_json::json!({
            "type": "object",
            "properties": {
                "query": {
                    "type": "string",
                    "description": "Mots-clés à chercher (ex: 'browser navigation', 'parler à voix haute', 'générer une image'). Support spécial : 'select:name1,name2' pour exact-name lookup."
                },
                "max_results": {
                    "type": "integer",
                    "description": "Nombre max de résultats (défaut 5, plafonné à 20)",
                    "default": 5
                }
            },
            "required": ["query"]
        }),
    )
}

/// Search deferred non-core builtin tools by lexical match.
///
/// Returns up to `max_results` definitions, ranked by descending score.
/// Supports two query forms:
/// - `"select:name1,name2"` — exact-name lookup (no ranking, deduped, ordered as input)
/// - any other string — split on whitespace, lexical scoring
pub fn search_deferred_builtin_tools(
    input: &serde_json::Value,
    definitions: Vec<ToolDefinition>,
    is_core_tool: impl Fn(&str) -> bool,
) -> Result<String, String> {
    let query = input
        .get("query")
        .and_then(|v| v.as_str())
        .ok_or("missing 'query' parameter")?
        .trim();
    if query.is_empty() {
        return Ok(tool_search_json(vec![], Some(TOOL_SEARCH_EMPTY_QUERY_HINT)));
    }
    let max_results = tool_search_max_results(input);

    let registry = ToolRegistry::new(definitions);
    let deferred: Vec<&ToolDefinition> = registry
        .deferred_discoverable_definitions(is_core_tool)
        .collect();

    if let Some((matches, hint)) = exact_tool_search_results(query, max_results, &deferred) {
        return Ok(tool_search_json(matches, hint));
    }

    let (results, hint) = lexical_tool_search_results(query, max_results, &deferred);
    Ok(tool_search_json(results, hint))
}

fn tool_search_max_results(input: &serde_json::Value) -> usize {
    input
        .get("max_results")
        .and_then(|v| v.as_u64())
        .unwrap_or(5)
        .clamp(1, 20) as usize
}

fn exact_tool_search_results(
    query: &str,
    max_results: usize,
    deferred: &[&ToolDefinition],
) -> Option<(Vec<serde_json::Value>, Option<&'static str>)> {
    let rest = query.strip_prefix("select:")?;
    let matches: Vec<serde_json::Value> = selected_tool_names(rest)
        .iter()
        .filter_map(|name| {
            deferred
                .iter()
                .find(|tool| &tool.name == name)
                .map(|tool| tool_result(tool))
        })
        .take(max_results)
        .collect();
    let hint = no_match_hint_if_empty(&matches);
    Some((matches, hint))
}

fn selected_tool_names(input: &str) -> Vec<String> {
    input
        .split(',')
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect()
}

fn lexical_tool_search_results(
    query: &str,
    max_results: usize,
    deferred: &[&ToolDefinition],
) -> (Vec<serde_json::Value>, Option<&'static str>) {
    let tokens: Vec<String> = query
        .to_lowercase()
        .split_whitespace()
        .map(str::to_string)
        .collect();

    let mut scored: Vec<(u32, &ToolDefinition)> = deferred
        .iter()
        .copied()
        .map(|tool| (lexical_tool_score(&tokens, tool), tool))
        .filter(|(score, _)| *score > 0)
        .collect();
    scored.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.name.cmp(&b.1.name)));
    scored.truncate(max_results);

    let results: Vec<serde_json::Value> = scored
        .into_iter()
        .map(|(_, tool)| tool_result(tool))
        .collect();

    let hint = no_match_hint_if_empty(&results);
    (results, hint)
}

fn no_match_hint_if_empty(results: &[serde_json::Value]) -> Option<&'static str> {
    if results.is_empty() {
        Some(TOOL_SEARCH_NO_MATCH_HINT)
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn discovery_tool_definitions_keep_public_order() {
        let definitions = discovery_tool_definitions();
        let names: Vec<_> = definitions.iter().map(|tool| tool.name.as_str()).collect();

        assert_eq!(
            names,
            vec![
                "captain_docs",
                "capability_search",
                "skill_search",
                "skill_view",
                "skill_check",
                "tool_search",
            ]
        );
    }

    #[test]
    fn discovery_tool_definitions_keep_docs_and_capability_contracts() {
        let definitions = discovery_tool_definitions();
        let docs = registered_tool(&definitions, "captain_docs");
        let capability = registered_tool(&definitions, "capability_search");

        assert_eq!(required_fields(docs), vec!["query"]);
        assert_contains(&docs.description, "runtime-changelog");
        assert_contains(
            property(docs, "family")
                .get("description")
                .and_then(Value::as_str)
                .unwrap_or_default(),
            "Live Tool Schemas",
        );
        assert_eq!(required_fields(capability), vec!["query"]);
        assert_eq!(
            enum_values(
                property(capability, "sources")
                    .get("items")
                    .expect("sources should define array items")
            ),
            vec!["builtin", "capfile", "skill", "mcp", "docs"]
        );
        assert_eq!(
            integer_default(property(capability, "max_results")),
            Some(8)
        );
        assert_eq!(
            boolean_default(property(capability, "include_schemas")),
            Some(true)
        );
        assert_contains(&capability.description, "surfaces gelées");
    }

    #[test]
    fn discovery_tool_definitions_keep_skill_and_tool_search_contracts() {
        let definitions = discovery_tool_definitions();
        let skill_search = registered_tool(&definitions, "skill_search");
        let skill_view = registered_tool(&definitions, "skill_view");
        let skill_check = registered_tool(&definitions, "skill_check");
        let tool_search = registered_tool(&definitions, "tool_search");

        assert_eq!(
            enum_values(property(skill_search, "family")),
            vec![
                "software-development",
                "project-management",
                "review-release",
                "platform-devops",
                "data-ai",
                "product-design",
                "business-tools",
                "security-compliance",
                "general-automation",
            ]
        );
        assert_eq!(
            integer_default(property(skill_search, "max_results")),
            Some(8)
        );
        assert_eq!(
            boolean_default(property(skill_search, "include_families")),
            Some(true)
        );
        assert_eq!(required_fields(skill_view), vec!["name"]);
        assert_eq!(
            integer_default(property(skill_view, "max_context_chars")),
            Some(8000)
        );
        assert_eq!(required_fields(skill_check), vec!["name"]);
        assert_eq!(
            boolean_default(property(skill_check, "run_static_tests")),
            Some(true)
        );
        assert_eq!(required_fields(tool_search), vec!["query"]);
        assert_eq!(
            integer_default(property(tool_search, "max_results")),
            Some(5)
        );
        assert_contains(&tool_search.description, "select:name1,name2");
    }

    fn tool(name: &str, description: &str) -> ToolDefinition {
        ToolDefinition {
            name: name.to_string(),
            description: description.to_string(),
            input_schema: serde_json::json!({"type": "object"}),
        }
    }

    fn result_names(raw: &str) -> Vec<String> {
        let parsed: serde_json::Value = serde_json::from_str(raw).expect("valid JSON");
        parsed["results"]
            .as_array()
            .expect("results array")
            .iter()
            .filter_map(|item| item["name"].as_str().map(str::to_string))
            .collect()
    }

    fn registered_tool<'a>(definitions: &'a [ToolDefinition], name: &str) -> &'a ToolDefinition {
        definitions
            .iter()
            .find(|tool| tool.name == name)
            .unwrap_or_else(|| panic!("{name} should be registered"))
    }

    fn required_fields(tool: &ToolDefinition) -> Vec<String> {
        tool.input_schema
            .get("required")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
            .map(str::to_string)
            .collect()
    }

    fn property<'a>(tool: &'a ToolDefinition, name: &str) -> &'a Value {
        tool.input_schema
            .get("properties")
            .and_then(|properties| properties.get(name))
            .unwrap_or_else(|| panic!("{} should define property {name}", tool.name))
    }

    fn enum_values(property: &Value) -> Vec<String> {
        property
            .get("enum")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
            .map(str::to_string)
            .collect()
    }

    fn integer_default(property: &Value) -> Option<u64> {
        property.get("default").and_then(Value::as_u64)
    }

    fn boolean_default(property: &Value) -> Option<bool> {
        property.get("default").and_then(Value::as_bool)
    }

    fn assert_contains(haystack: &str, needle: &str) {
        assert!(
            haystack.contains(needle),
            "expected `{haystack}` to contain `{needle}`"
        );
    }

    #[test]
    fn select_filters_core_and_frozen_tools() {
        let raw = search_deferred_builtin_tools(
            &serde_json::json!({"query": "select:capability_search,browser_click,hand_activate"}),
            vec![
                tool("capability_search", "core resolver"),
                tool("browser_click", "Click in browser"),
                tool("hand_activate", "Activate a Hand"),
            ],
            |name| name == "capability_search",
        )
        .expect("search");
        assert_eq!(result_names(&raw), vec!["browser_click"]);
    }

    #[test]
    fn lexical_search_ranks_and_limits_deferred_tools() {
        let raw = search_deferred_builtin_tools(
            &serde_json::json!({"query": "browser", "max_results": 1}),
            vec![
                tool("browser_click", "Click an element"),
                tool("file_read", "Read a browser export from disk"),
            ],
            |_| false,
        )
        .expect("search");
        assert_eq!(result_names(&raw), vec!["browser_click"]);
    }

    #[test]
    fn tool_search_finds_shell_for_runtime_binary_version_questions() {
        let raw = search_deferred_builtin_tools(
            &serde_json::json!({"query": "version binaire runtime captain build"}),
            crate::tools::shell_definitions::shell_tool_definitions(),
            |_| false,
        )
        .expect("search");
        assert!(
            result_names(&raw).contains(&"shell_exec".to_string()),
            "{raw}"
        );
    }
}
