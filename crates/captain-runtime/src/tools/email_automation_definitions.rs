//! Deferred tool definitions for deterministic Gmail-to-agent automations.

use captain_types::tool::ToolDefinition;
use serde_json::{json, Value};

pub(super) fn email_automation_tool_definitions() -> Vec<ToolDefinition> {
    vec![
        tool_definition(
            "email_automation_rules",
            "[EMAIL AUTOMATION — INVENTAIRE] Liste les règles Gmail durables ou inspecte une règle précise. Les versions retournées sont requises pour toute mutation compare-and-swap. Aucun contenu d'email n'est exposé.",
            object_schema(
                json!({
                    "rule_id": identifier("ID exact d'une règle à inspecter."),
                    "account": account_property(),
                    "limit": { "type": "integer", "minimum": 1, "maximum": 1000, "default": 100 }
                }),
                &[],
            ),
        ),
        tool_definition(
            "email_automation_rule_save",
            "[EMAIL AUTOMATION — MUTATION] Crée une règle déterministe ou met à jour une règle existante avec id et expected_version. Utiliser uniquement après une demande utilisateur explicite et fournir confirm_automation=true. Si id est omis à la création, Captain en dérive un stable. Une mise à jour ne peut pas changer de compte.",
            save_schema(),
        ),
        tool_definition(
            "email_automation_rule_set_enabled",
            "[EMAIL AUTOMATION — MUTATION] Active ou désactive une règle avec sa version exacte. Fournir confirm_change=true uniquement après une demande utilisateur explicite.",
            object_schema(
                json!({
                    "rule_id": identifier("ID exact retourné par email_automation_rules."),
                    "expected_version": { "type": "integer", "minimum": 1 },
                    "enabled": { "type": "boolean" },
                    "confirm_change": { "type": "boolean", "default": false }
                }),
                &["rule_id", "expected_version", "enabled", "confirm_change"],
            ),
        ),
        tool_definition(
            "email_automation_rule_remove",
            "[EMAIL AUTOMATION — SUPPRESSION SÛRE] Supprime seulement une règle inutilisée avec sa version exacte. Une règle ayant un historique d'audit ne peut jamais être supprimée et doit être désactivée. Fournir confirm_delete_unused=true après confirmation utilisateur.",
            object_schema(
                json!({
                    "rule_id": identifier("ID exact retourné par email_automation_rules."),
                    "expected_version": { "type": "integer", "minimum": 1 },
                    "confirm_delete_unused": { "type": "boolean", "default": false }
                }),
                &["rule_id", "expected_version", "confirm_delete_unused"],
            ),
        ),
        tool_definition(
            "email_automation_deliveries",
            "[EMAIL AUTOMATION — LIVRAISONS] Liste les états crash-safe sans contenu d'email, ou inspecte une livraison précise avec delivery_id. Les métadonnées du message ne sont alors retournées que dans une enveloppe externe non fiable. Inspecter la session indiquée avant toute reprise uncertain.",
            object_schema(
                json!({
                    "delivery_id": identifier("ID exact d'une livraison à inspecter."),
                    "status": { "type": "string", "enum": ["pending", "delivering", "retry_wait", "delivered", "dead", "uncertain"] },
                    "limit": { "type": "integer", "minimum": 1, "maximum": 1000, "default": 50 }
                }),
                &[],
            ),
        ),
        tool_definition(
            "email_automation_delivery_requeue",
            "[EMAIL AUTOMATION — REPRISE EXPLICITE] Remet en file une livraison dead ou uncertain déjà inspectée. Une livraison uncertain peut avoir exécuté son tour avant le crash : vérifier d'abord sa session et fournir confirm_duplicate_risk=true uniquement après acceptation explicite du risque de doublon.",
            object_schema(
                json!({
                    "delivery_id": identifier("ID exact retourné par email_automation_deliveries."),
                    "expected_status": { "type": "string", "enum": ["dead", "uncertain"] },
                    "confirm_duplicate_risk": { "type": "boolean", "default": false }
                }),
                &["delivery_id", "expected_status", "confirm_duplicate_risk"],
            ),
        ),
    ]
}

fn save_schema() -> Value {
    json!({
        "type": "object",
        "additionalProperties": false,
        "properties": {
            "id": identifier("ID stable. Requis avec expected_version; dérivé à la création si omis."),
            "expected_version": { "type": "integer", "minimum": 1 },
            "account": account_property(),
            "name": { "type": "string", "minLength": 1, "maxLength": 160 },
            "from_contains": { "type": "string", "minLength": 1, "maxLength": 512 },
            "recipient_contains": { "type": "string", "minLength": 1, "maxLength": 512 },
            "subject_contains": { "type": "string", "minLength": 1, "maxLength": 512 },
            "all_label_ids": label_array("Tous ces labels doivent être présents."),
            "any_label_ids": label_array("Au moins un de ces labels doit être présent."),
            "target_agent": { "type": "string", "minLength": 1, "maxLength": 128, "default": "captain", "description": "Nom exact ou UUID d'un agent actuellement enregistré." },
            "instruction": { "type": "string", "minLength": 1, "maxLength": 16384, "description": "Instruction opérateur de confiance exécutée pour chaque match." },
            "include_body": { "type": "boolean", "default": false },
            "max_body_bytes": { "type": "integer", "minimum": 1, "maximum": 262144, "default": 32768 },
            "max_delivery_attempts": { "type": "integer", "minimum": 1, "maximum": 10, "default": 3 },
            "enabled": { "type": "boolean", "default": true },
            "max_fires_per_hour": { "type": "integer", "minimum": 1, "maximum": 1000, "default": 20 },
            "confirm_automation": { "type": "boolean", "default": false, "description": "True uniquement après une demande utilisateur explicite de créer ou modifier cette automation." }
        },
        "required": ["name", "instruction", "confirm_automation"],
        "anyOf": [
            { "required": ["from_contains"] },
            { "required": ["recipient_contains"] },
            { "required": ["subject_contains"] },
            { "required": ["all_label_ids"] },
            { "required": ["any_label_ids"] }
        ]
    })
}

fn tool_definition(name: &str, description: &str, input_schema: Value) -> ToolDefinition {
    ToolDefinition {
        name: name.to_string(),
        description: description.to_string(),
        input_schema,
    }
}

fn object_schema(properties: Value, required: &[&str]) -> Value {
    json!({
        "type": "object",
        "additionalProperties": false,
        "properties": properties,
        "required": required
    })
}

fn account_property() -> Value {
    json!({
        "type": "string",
        "maxLength": 48,
        "description": "Alias Gmail connecté. Si omis, utilise le compte par défaut."
    })
}

fn identifier(description: &str) -> Value {
    json!({
        "type": "string",
        "minLength": 1,
        "maxLength": 96,
        "description": description
    })
}

fn label_array(description: &str) -> Value {
    json!({
        "type": "array",
        "minItems": 1,
        "maxItems": 100,
        "items": { "type": "string", "minLength": 1, "maxLength": 256 },
        "description": description
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn automation_definitions_are_strict_and_confirm_every_mutation() {
        let tools = email_automation_tool_definitions();
        assert_eq!(tools.len(), 6);
        assert!(tools
            .iter()
            .all(|tool| tool.input_schema["additionalProperties"] == json!(false)));
        for (name, confirmation) in [
            ("email_automation_rule_save", "confirm_automation"),
            ("email_automation_rule_set_enabled", "confirm_change"),
            ("email_automation_rule_remove", "confirm_delete_unused"),
            (
                "email_automation_delivery_requeue",
                "confirm_duplicate_risk",
            ),
        ] {
            let tool = tools.iter().find(|tool| tool.name == name).unwrap();
            assert_eq!(
                tool.input_schema["properties"][confirmation]["default"],
                json!(false)
            );
        }
    }

    #[test]
    fn rule_save_schema_requires_a_deterministic_condition() {
        let save = email_automation_tool_definitions()
            .into_iter()
            .find(|tool| tool.name == "email_automation_rule_save")
            .unwrap();
        assert_eq!(save.input_schema["anyOf"].as_array().unwrap().len(), 5);
        assert!(save.description.contains("expected_version"));
    }
}
