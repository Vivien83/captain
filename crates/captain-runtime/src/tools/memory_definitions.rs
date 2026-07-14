//! Static memory tool definitions.

use captain_types::tool::ToolDefinition;
use serde_json::Value;

pub fn memory_tool_definitions() -> Vec<ToolDefinition> {
    let mut definitions = shared_memory_tool_definitions();
    definitions.extend(memory_context_tool_definitions());
    definitions.extend(durable_memory_tool_definitions());
    definitions
}

fn shared_memory_tool_definitions() -> Vec<ToolDefinition> {
    vec![
        memory_store_tool_definition(),
        memory_recall_tool_definition(),
    ]
}

fn memory_context_tool_definitions() -> Vec<ToolDefinition> {
    vec![memory_context_batch_tool_definition()]
}

fn durable_memory_tool_definitions() -> Vec<ToolDefinition> {
    vec![
        memory_save_tool_definition(),
        memory_forget_tool_definition(),
    ]
}

fn tool_definition(name: &str, description: &str, input_schema: Value) -> ToolDefinition {
    ToolDefinition {
        name: name.to_string(),
        description: description.to_string(),
        input_schema,
    }
}

fn memory_store_tool_definition() -> ToolDefinition {
    tool_definition(
        "memory_store",
        "Stocke une valeur dans la mémoire partagée accessible par tous les agents du runtime. Utiliser pour transmettre des résultats entre agents, partager un état intermédiaire ou coordonner une tâche distribuée. Ne pas utiliser pour des données sensibles (secrets, clés API) — utiliser config_read/config_write à la place. Retourne une confirmation d'écriture avec la clé et la taille de la valeur stockée.",
        serde_json::json!({
            "type": "object",
            "properties": {
                "key": { "type": "string", "description": "Clé d'accès unique — utiliser des noms explicites avec namespace (ex: 'task:scrape:results', 'agent:captain:status')" },
                "value": { "type": "string", "description": "Valeur à stocker — encoder les objets/tableaux en JSON, passer les chaînes brutes directement" }
            },
            "required": ["key", "value"]
        }),
    )
}

fn memory_recall_tool_definition() -> ToolDefinition {
    tool_definition(
        "memory_recall",
        "Récupère une valeur depuis la mémoire partagée à partir de sa clé. Utiliser pour lire des résultats produits par un autre agent ou un état intermédiaire stocké. Ne pas utiliser si la clé est inconnue — il n'y a pas de mécanisme de liste des clés disponibles. Retourne la valeur brute telle qu'elle a été stockée, ou une erreur si la clé n'existe pas.",
        serde_json::json!({
            "type": "object",
            "properties": {
                "key": { "type": "string", "description": "Clé exacte à récupérer (doit correspondre exactement à la clé utilisée lors du memory_store)" }
            },
            "required": ["key"]
        }),
    )
}

fn memory_context_batch_tool_definition() -> ToolDefinition {
    tool_definition(
        "memory_context_batch",
        "[CONTEXTE MEMOIRE GROUPE] Lit en une seule fois memory_recall, session_recall et knowledge_query pour une ou plusieurs requêtes, puis retourne une capsule compacte filtrée haute confiance. Lecture seule. À utiliser avant de répondre sur un sujet passé ou personnel sans multiplier les appels mémoire/session/knowledge. Par défaut, filtre les résultats mémoire MemPalace faibles/bruyants au lieu d'injecter le retour brut.",
        serde_json::json!({
            "type": "object",
            "properties": {
                "query": { "type": "string" },
                "queries": { "type": "array", "items": { "type": "string" }, "description": "Jusqu'à 30 requêtes." },
                "include_memory": { "type": "boolean", "default": true },
                "include_sessions": { "type": "boolean", "default": true },
                "include_knowledge": { "type": "boolean", "default": false },
                "max_results": { "type": "integer", "description": "Résultats session par requête, défaut 5." },
                "memory_max_results": { "type": "integer", "description": "Candidats mémoire retournés par requête après filtrage, défaut max_results, max 10." },
                "memory_min_similarity": { "type": "number", "description": "Seuil de similarité quand MemPalace fournit un score 0..1. Défaut 0.75." },
                "strict_memory_filter": { "type": "boolean", "default": true, "description": "Si true, n'injecte que les résultats avec overlap lexical suffisant ou similarité forte." },
                "preview_chars": { "type": "integer", "description": "Taille de preview par bloc, défaut 2500." },
                "stop_on_error": { "type": "boolean", "default": false }
            }
        }),
    )
}

fn memory_save_tool_definition() -> ToolDefinition {
    tool_definition(
        "memory_save",
        "[MÉMOIRE LONG-TERME] Enregistre un fait, une compétence, une leçon apprise ou une préférence dans la mémoire persistante (MemPalace). À utiliser SPONTANÉMENT — sans que l'utilisateur ne le demande — quand tu détectes : (1) une INFO durable sur l'utilisateur ou son contexte (préférences, contacts, configuration récurrente), (2) une préférence de style/réponse (room user_preferences, ex predicate prefers_response_style), (3) une COMPÉTENCE acquise (un workflow qui a marché), (4) une ERREUR/RÉUSSITE qui mérite d'être retenue pour ne pas la répéter ou la reproduire, (5) une SOLUTION à un problème précis (commande, snippet, contournement). 4 PARAMS REQUIS : `subject` (entité, ex 'user', 'project:captain'), `predicate` (relation, ex 'prefers', 'requires'), `object` (la valeur, ≤1000 chars), `category` (UN DE : info | skill | error_success | solution | other — pas autre chose). Le filtre PII rejette automatiquement les credentials, emails, téléphones, IBAN, tokens. Le 🧠 apparaîtra dans le canal d'origine. Ne pas utiliser pour des données éphémères (état temporaire d'une tâche) — utiliser memory_store à la place. EXEMPLE : memory_save({\"subject\":\"user\",\"predicate\":\"prefers_response_style\",\"object\":\"réponses courtes en français\",\"category\":\"info\",\"room\":\"user_preferences\"}). EXEMPLE 2 : memory_save({\"subject\":\"deployment\",\"predicate\":\"requires\",\"object\":\"migrate avant build sinon schema cassé\",\"category\":\"skill\"}).",
        serde_json::json!({
            "type": "object",
            "properties": {
                "subject": { "type": "string", "description": "Entité que la mémoire concerne. Exemples: 'user', 'project:captain', 'tool:cron_create', 'host:prod-server'." },
                "predicate": { "type": "string", "description": "Relation/attribut. Exemples: 'prefers', 'fixed_by', 'has_quirk', 'works_with', 'failed_because'." },
                "object": { "type": "string", "description": "La valeur libre. Phrase courte (≤ 250 chars) qui décrit le fait. Pas de credentials/PII." },
                "category": {
                    "type": "string",
                    "enum": ["info", "skill", "error_success", "solution", "other"],
                    "description": "Type d'apprentissage. info = fait durable. skill = compétence/workflow validé. error_success = leçon d'un échec ou d'une réussite à reproduire. solution = recette pour un problème précis. other = autre."
                },
                "wing": { "type": "string", "description": "Optionnel — wing MemPalace (ex: 'learnings', 'project:captain'). Défaut: 'learnings'." },
                "room": { "type": "string", "description": "Optionnel — room MemPalace (ex: 'general', 'user_preferences', 'workarounds'). Défaut: 'general'." },
                "channel": { "type": "string", "description": "Optionnel — canal d'origine de la conversation (telegram, cli, web, discord). Si setté, le 🧠 sera renvoyé sur ce canal au lieu d'être broadcast." }
            },
            "required": ["subject", "predicate", "object", "category"]
        }),
    )
}

fn memory_forget_tool_definition() -> ToolDefinition {
    tool_definition(
        "memory_forget",
        "[MÉMOIRE — RETRACTATION] Supprime des faits incorrects ou obsolètes que tu (ou le job de reflection) avais stockés via memory_save / write_through. À utiliser SPONTANÉMENT — sans qu'on te le demande — quand l'utilisateur dit 'tu te trompes', 'oublie ça', 'corrige ce que tu sais sur X', 'ce n'est plus vrai'. Au moins UN des trois filtres (subject / predicate / object) doit être fourni — sans filtre la fonction retourne 0 sans rien supprimer (anti-wipe). Les filtres acceptent les wildcards SQL LIKE (% = n'importe quoi). Le DELETE est combiné en AND : les trois filtres doivent matcher pour qu'une ligne tombe. En plus du DELETE, Captain pose un garde-fou de rétraction: les anciennes traces archivées (checkpoints, journaux, snapshots .md, graph summary) restent dans le passé mais ne doivent plus être injectées comme contexte actif si elles matchent le terme oublié. Les résumés actifs mutables comme canonical_sessions.compacted_summary sont aussi nettoyés quand le kernel le supporte. EXEMPLE : memory_forget({\"subject\":\"user\",\"predicate\":\"prefers\"}) supprime les préférences utilisateur ciblées. EXEMPLE 2 : memory_forget({\"object\":\"%ancienne_valeur%\"}) supprime tout fait mentionnant cette valeur. Retourne le nombre de lignes supprimées, active_context_suppressed=true et active_context_sanitized avec les résumés nettoyés.",
        serde_json::json!({
            "type": "object",
            "properties": {
                "subject": { "type": "string", "description": "Filtre LIKE sur subject (ex: 'user', 'project:%')" },
                "predicate": { "type": "string", "description": "Filtre LIKE sur predicate (ex: 'has_dog', 'works_%')" },
                "object": { "type": "string", "description": "Filtre LIKE sur object (ex: '%ancienne_valeur%', 'remote%')" }
            }
        }),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn memory_tool_definitions_keep_public_order() {
        let tools = memory_tool_definitions();
        let names: Vec<_> = tools.iter().map(|tool| tool.name.as_str()).collect();

        assert_eq!(
            names,
            vec![
                "memory_store",
                "memory_recall",
                "memory_context_batch",
                "memory_save",
                "memory_forget",
            ]
        );
    }

    #[test]
    fn memory_tool_definitions_keep_shared_memory_contracts() {
        let tools = memory_tool_definitions();
        let store = tool(&tools, "memory_store");
        let recall = tool(&tools, "memory_recall");

        assert_eq!(required_fields(store), vec!["key", "value"]);
        assert_contains(
            &store.description,
            "Ne pas utiliser pour des données sensibles",
        );
        assert_contains(
            property(store, "key")["description"]
                .as_str()
                .unwrap_or_default(),
            "namespace",
        );

        assert_eq!(required_fields(recall), vec!["key"]);
        assert_contains(&recall.description, "il n'y a pas de mécanisme de liste");
        assert_contains(
            property(recall, "key")["description"]
                .as_str()
                .unwrap_or_default(),
            "exactement",
        );
    }

    #[test]
    fn memory_tool_definitions_keep_context_batch_filters() {
        let tools = memory_tool_definitions();
        let batch = tool(&tools, "memory_context_batch");

        assert!(required_fields(batch).is_empty());
        assert_contains(&batch.description, "haute confiance");
        assert_contains(&batch.description, "filtre les résultats mémoire");
        assert_eq!(items_type(property(batch, "queries")), "string");
        assert_eq!(
            boolean_field(property(batch, "include_memory"), "default"),
            Some(true)
        );
        assert_eq!(
            boolean_field(property(batch, "include_sessions"), "default"),
            Some(true)
        );
        assert_eq!(
            boolean_field(property(batch, "include_knowledge"), "default"),
            Some(false)
        );
        assert_eq!(
            boolean_field(property(batch, "strict_memory_filter"), "default"),
            Some(true)
        );
        assert_eq!(
            boolean_field(property(batch, "stop_on_error"), "default"),
            Some(false)
        );
    }

    #[test]
    fn memory_tool_definitions_keep_durable_save_contracts() {
        let tools = memory_tool_definitions();
        let save = tool(&tools, "memory_save");

        assert_eq!(
            required_fields(save),
            vec!["subject", "predicate", "object", "category"]
        );
        assert_eq!(
            enum_values(property(save, "category")),
            vec!["info", "skill", "error_success", "solution", "other"]
        );
        assert_contains(&save.description, "SPONTANÉMENT");
        assert_contains(&save.description, "Le filtre PII");
        assert_contains(&save.description, "utiliser memory_store à la place");
        assert_contains(
            property(save, "object")["description"]
                .as_str()
                .unwrap_or_default(),
            "PII",
        );
        assert_contains(
            property(save, "channel")["description"]
                .as_str()
                .unwrap_or_default(),
            "telegram",
        );
    }

    #[test]
    fn memory_tool_definitions_keep_forget_retraction_contracts() {
        let tools = memory_tool_definitions();
        let forget = tool(&tools, "memory_forget");

        assert!(required_fields(forget).is_empty());
        assert!(property(forget, "subject").is_object());
        assert!(property(forget, "predicate").is_object());
        assert!(property(forget, "object").is_object());
        assert_contains(&forget.description, "anti-wipe");
        assert_contains(&forget.description, "active_context_suppressed=true");
        assert_contains(&forget.description, "canonical_sessions.compacted_summary");
    }

    fn tool<'a>(tools: &'a [ToolDefinition], name: &str) -> &'a ToolDefinition {
        tools
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

    fn enum_values(value: &Value) -> Vec<&str> {
        value
            .get("enum")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
            .collect()
    }

    fn items_type(value: &Value) -> &str {
        value
            .get("items")
            .and_then(|items| items.get("type"))
            .and_then(Value::as_str)
            .unwrap_or_default()
    }

    fn boolean_field(value: &Value, name: &str) -> Option<bool> {
        value.get(name).and_then(Value::as_bool)
    }

    fn assert_contains(haystack: &str, needle: &str) {
        assert!(
            haystack.contains(needle),
            "expected `{haystack}` to contain `{needle}`"
        );
    }
}
