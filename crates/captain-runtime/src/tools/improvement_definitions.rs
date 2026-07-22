//! Static controlled-improvement tool definitions.

use captain_types::tool::ToolDefinition;
use serde_json::Value;

pub fn improvement_tool_definitions() -> Vec<ToolDefinition> {
    let mut definitions = improvement_review_tool_definitions();
    definitions.extend(system_bug_tool_definitions());
    definitions.extend(learning_review_tool_definitions());
    definitions.extend(workflow_learning_tool_definitions());
    definitions.extend(skill_refinement_tool_definitions());
    definitions
}

fn improvement_review_tool_definitions() -> Vec<ToolDefinition> {
    vec![self_improvement_review_tool_definition()]
}

fn system_bug_tool_definitions() -> Vec<ToolDefinition> {
    vec![
        system_bug_report_tool_definition(),
        system_bug_list_tool_definition(),
        system_bug_update_tool_definition(),
    ]
}

fn learning_review_tool_definitions() -> Vec<ToolDefinition> {
    vec![
        learning_review_list_tool_definition(),
        learning_review_decide_tool_definition(),
    ]
}

fn workflow_learning_tool_definitions() -> Vec<ToolDefinition> {
    vec![workflow_learning_list_tool_definition()]
}

fn skill_refinement_tool_definitions() -> Vec<ToolDefinition> {
    vec![
        skill_refinement_propose_tool_definition(),
        skill_refinement_list_tool_definition(),
        skill_refinement_decide_tool_definition(),
        skill_refinement_update_tool_definition(),
        skill_refinement_restore_tool_definition(),
    ]
}

fn tool_definition(name: &str, description: &str, input_schema: Value) -> ToolDefinition {
    ToolDefinition {
        name: name.to_string(),
        description: description.to_string(),
        input_schema,
    }
}

fn self_improvement_review_tool_definition() -> ToolDefinition {
    tool_definition(
        "self_improvement_review",
        "[AUTO-AMÉLIORATION CONTRÔLÉE] Inspecte en une seule lecture les files durables de Captain: learnings en attente d'approbation, bugs système ouverts, raffinements de skills et workflows Skill Learning V2. Read-only: ne modifie rien. À appeler spontanément après une tâche longue/tool-heavy, un échec répété, un `Security blocked`, ou quand l'utilisateur demande ce que Captain a appris. Les capacités critiques se décident uniquement depuis leur carte opérateur authentifiée Telegram/TUI/Web/Desktop.",
        serde_json::json!({
            "type": "object",
            "properties": {
                "limit": { "type": "integer", "minimum": 1, "maximum": 50, "description": "Nombre max d'items par file (default 10)" }
            }
        }),
    )
}

fn system_bug_report_tool_definition() -> ToolDefinition {
    tool_definition(
        "system_bug_report",
        "[AUTO-DIAGNOSTIC] Enregistre un bug ou une faiblesse du système Captain détecté pendant le travail: tool absent/mal documenté, erreur répétée, incohérence de sécurité, problème de performance, canal, scheduler, MCP, skill, mémoire, etc. Utiliser quand Captain identifie une cause reproductible ou un défaut produit, pas pour une simple erreur utilisateur. Les secrets bruts sont refusés; les chemins locaux des champs texte stockés sont redigés avant stockage.",
        serde_json::json!({
            "type": "object",
            "properties": {
                "title": { "type": "string", "description": "Résumé court et générique du bug" },
                "description": { "type": "string", "description": "Ce qui échoue, impact utilisateur, contexte reproductible" },
                "category": { "type": "string", "enum": ["tool","scheduler","channel","memory","security","performance","mcp","skill","docs","ui","unknown"], "description": "Famille touchée" },
                "severity": { "type": "string", "enum": ["low","medium","high","critical"], "description": "Impact estimé" },
                "evidence": { "type": "string", "description": "Message d'erreur ou observation redigée, sans secret ni chemin local" },
                "suggested_fix": { "type": "string", "description": "Correction ou piste proposée, si connue; les chemins locaux sont redigés" },
                "source": { "type": "string", "description": "Origine courte: tool_failure, user_report, self_review, etc.; chemins locaux redigés" }
            },
            "required": ["title", "description", "category", "severity"]
        }),
    )
}

fn system_bug_list_tool_definition() -> ToolDefinition {
    tool_definition(
        "system_bug_list",
        "Liste le registre persistant des bugs/faiblesses système Captain, filtrable par statut, catégorie ou sévérité. Utiliser avant de corriger ou proposer une auto-amélioration afin de ne pas redécouvrir le même défaut. La sortie est operator-safe, y compris pour les anciens items du registre.",
        serde_json::json!({
            "type": "object",
            "properties": {
                "status": { "type": "string", "enum": ["open","investigating","fixed","wont_fix","duplicate","reported"] },
                "category": { "type": "string", "enum": ["tool","scheduler","channel","memory","security","performance","mcp","skill","docs","ui","unknown"] },
                "severity": { "type": "string", "enum": ["low","medium","high","critical"] },
                "limit": { "type": "integer", "minimum": 1, "maximum": 50, "description": "Default 20" }
            }
        }),
    )
}

fn system_bug_update_tool_definition() -> ToolDefinition {
    tool_definition(
        "system_bug_update",
        "Met à jour un bug système existant (statut/catégorie/sévérité/note/proposition de fix). Utiliser après vérification, correction, escalade ou déduplication. id accepte un préfixe non ambigu. La sortie et le store normalisé ne publient pas les secrets ou chemins locaux legacy.",
        serde_json::json!({
            "type": "object",
            "properties": {
                "id": { "type": "string", "description": "ID complet ou préfixe non ambigu" },
                "status": { "type": "string", "enum": ["open","investigating","fixed","wont_fix","duplicate","reported"] },
                "category": { "type": "string", "enum": ["tool","scheduler","channel","memory","security","performance","mcp","skill","docs","ui","unknown"] },
                "severity": { "type": "string", "enum": ["low","medium","high","critical"] },
                "note": { "type": "string", "description": "Observation ou décision ajoutée à l'historique" },
                "suggested_fix": { "type": "string", "description": "Correction proposée ou appliquée" }
            },
            "required": ["id"]
        }),
    )
}

fn learning_review_list_tool_definition() -> ToolDefinition {
    tool_definition(
        "learning_review_list",
        "Liste les learnings en attente de validation humaine (mode approval). Chaque item est projeté sur id/subject/predicate/object/wing/room/confidence avec sortie operator-safe: secrets masqués, chemins locaux redigés, et champs d'audit internes masqués. Utiliser learning_review_decide(id, approve) pour approuver ou refuser.",
        serde_json::json!({
            "type": "object",
            "properties": {
                "limit": { "type": "number", "description": "Max items returned (1-50, default 50)" }
            }
        }),
    )
}

fn learning_review_decide_tool_definition() -> ToolDefinition {
    tool_definition(
        "learning_review_decide",
        "Refuse un learning en attente depuis un appel outil, ou applique une décision positive seulement depuis une surface humaine/API/canal explicite. Sur approve=true depuis un outil, l'appel est bloqué pour éviter l'auto-approbation d'un fait mémorisé qui sera ensuite rejoué dans les prompts futurs; sur approve=false, marque denied (conservé 30j pour audit). Une approbation humaine fait un write_through vers MemPalace avec la room routée. La sortie outil est une projection logique status/id/memory, sans payload kernel brut, secret, chemin local ni champ d'audit interne.",
        serde_json::json!({
            "type": "object",
            "properties": {
                "id": { "type": "string", "description": "Review item id from learning_review_list" },
                "approve": { "type": "boolean", "description": "true = write_through (blocked from tool calls), false = mark denied" }
            },
            "required": ["id", "approve"]
        }),
    )
}

fn workflow_learning_list_tool_definition() -> ToolDefinition {
    tool_definition(
        "workflow_learning_list",
        "Liste la projection durable Skill Learning V2: génération, validation, proposition, test isolé, installation, canary, activation, échec et rollback. Read-only et operator-safe. Utiliser pour expliquer ce que Captain apprend ou diagnostiquer son état; ne jamais simuler une décision. Les actions exactes restent réservées aux cartes authentifiées Telegram, TUI, Web et Desktop.",
        serde_json::json!({
            "type": "object",
            "properties": {
                "limit": { "type": "number", "description": "Max items returned (1-50, default 50)" }
            }
        }),
    )
}

fn skill_refinement_propose_tool_definition() -> ToolDefinition {
    tool_definition(
        "skill_refinement_propose",
        "[REMISE EN QUESTION SKILL PROACTIVE] Crée une proposition de raffinement pour un skill existant après usage réel: précondition manquante, meilleur routage d'outil, erreur récupérable, doc interne à préciser, env_inject à ajouter, etc. À utiliser spontanément dès qu'un skill utilisé révèle une amélioration réutilisable. Ne modifie pas le fichier skill; crée automatiquement un snapshot restaurable si le skill est file-backed, rend la proposition visible, puis attend approbation avant toute mutation durable. Les secrets bruts sont refusés; les chemins locaux des champs texte stockés sont redigés avant stockage.",
        serde_json::json!({
            "type": "object",
            "properties": {
                "skill": { "type": "string", "description": "Nom du skill concerné" },
                "finding": { "type": "string", "description": "Ce que l'usage du skill a révélé" },
                "suggested_change": { "type": "string", "description": "Changement concret proposé dans le skill" },
                "evidence": { "type": "string", "description": "Observation redigée, sans secret ni chemin privé inutile" },
                "current_version": { "type": "string", "description": "Version actuelle du skill si connue (ex: 0.1.0 ou v1); chemins locaux redigés" },
                "proposed_version": { "type": "string", "description": "Version cible proposée si connue (ex: 0.2.0 ou v2); chemins locaux redigés" },
                "risk": { "type": "string", "enum": ["low","medium","high"], "description": "Risque du changement proposé; default medium" },
                "source": { "type": "string", "description": "Origine courte: skill_use, user_correction, failed_run...; chemins locaux redigés" },
                "channel": { "type": "string", "description": "Canal d'origine optionnel; sinon le runtime le déduit du tour actif; chemins locaux redigés." }
            },
            "required": ["skill", "finding", "suggested_change"]
        }),
    )
}

fn skill_refinement_list_tool_definition() -> ToolDefinition {
    tool_definition(
        "skill_refinement_list",
        "Liste les propositions de raffinement de skills existants. Utiliser après self_improvement_review ou avant de modifier un skill afin de vérifier les améliorations déjà identifiées. La sortie est operator-safe même pour les anciens items du registre.",
        serde_json::json!({
            "type": "object",
            "properties": {
                "skill": { "type": "string" },
                "status": { "type": "string", "enum": ["pending","approved","denied","applied","restored"] },
                "risk": { "type": "string", "enum": ["low","medium","high"] },
                "limit": { "type": "integer", "minimum": 1, "maximum": 50, "description": "Default 20" }
            }
        }),
    )
}

fn skill_refinement_decide_tool_definition() -> ToolDefinition {
    tool_definition(
        "skill_refinement_decide",
        "Refuse une proposition de raffinement depuis un appel outil, ou applique une décision positive seulement depuis une surface humaine/API/canal explicite. approve=true depuis un outil est bloqué pour éviter l'auto-approbation de mutation durable; approve=false interdit la mutation. id accepte un préfixe non ambigu.",
        serde_json::json!({
            "type": "object",
            "properties": {
                "id": { "type": "string", "description": "ID complet ou préfixe non ambigu" },
                "approve": { "type": "boolean", "description": "true = approuvé pour patch ultérieur, false = refusé" },
                "note": { "type": "string", "description": "Raison ou consigne utilisateur" }
            },
            "required": ["id", "approve"]
        }),
    )
}

fn skill_refinement_update_tool_definition() -> ToolDefinition {
    tool_definition(
        "skill_refinement_update",
        "Met à jour le suivi d'une proposition de raffinement de skill après travail réel: marquer applied après patch/test, restored après rollback vérifié, ajouter une note, corriger le risque ou la version cible. N'applique pas le patch lui-même; c'est le journal de contrôle.",
        serde_json::json!({
            "type": "object",
            "properties": {
                "id": { "type": "string", "description": "ID complet ou préfixe non ambigu" },
                "status": { "type": "string", "enum": ["pending","approved","denied","applied","restored"] },
                "risk": { "type": "string", "enum": ["low","medium","high"] },
                "note": { "type": "string", "description": "Observation, résultat de test, ou détail du patch" },
                "proposed_version": { "type": "string", "description": "Version cible mise à jour" }
            },
            "required": ["id"]
        }),
    )
}

fn skill_refinement_restore_tool_definition() -> ToolDefinition {
    tool_definition(
        "skill_refinement_restore",
        "Restaure un skill file-backed depuis le snapshot créé automatiquement lors de skill_refinement_propose. À utiliser si une amélioration approuvée/appliquée doit être annulée. Crée aussi un backup pre-restore de l'état courant avant de remplacer le dossier du skill.",
        serde_json::json!({
            "type": "object",
            "properties": {
                "id": { "type": "string", "description": "ID complet ou préfixe non ambigu du raffinement" },
                "note": { "type": "string", "description": "Raison du restore" }
            },
            "required": ["id"]
        }),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn improvement_tool_definitions_keep_public_order() {
        let definitions = improvement_tool_definitions();
        let names: Vec<_> = definitions.iter().map(|tool| tool.name.as_str()).collect();

        assert_eq!(
            names,
            vec![
                "self_improvement_review",
                "system_bug_report",
                "system_bug_list",
                "system_bug_update",
                "learning_review_list",
                "learning_review_decide",
                "workflow_learning_list",
                "skill_refinement_propose",
                "skill_refinement_list",
                "skill_refinement_decide",
                "skill_refinement_update",
                "skill_refinement_restore",
            ]
        );
    }

    #[test]
    fn improvement_tool_definitions_keep_bug_contracts() {
        let definitions = improvement_tool_definitions();
        let report = tool(&definitions, "system_bug_report");
        let update = tool(&definitions, "system_bug_update");

        assert_eq!(
            required_fields(report),
            vec!["title", "description", "category", "severity"]
        );
        assert_eq!(required_fields(update), vec!["id"]);
        assert_eq!(
            enum_values(property(report, "category")),
            vec![
                "tool",
                "scheduler",
                "channel",
                "memory",
                "security",
                "performance",
                "mcp",
                "skill",
                "docs",
                "ui",
                "unknown",
            ]
        );
        assert_eq!(
            enum_values(property(report, "severity")),
            vec!["low", "medium", "high", "critical"]
        );
        assert_contains(
            property(report, "evidence")
                .get("description")
                .and_then(Value::as_str)
                .unwrap_or_default(),
            "sans secret ni chemin local",
        );
        assert_contains(&update.description, "chemins locaux legacy");
    }

    #[test]
    fn improvement_tool_definitions_keep_approval_guardrails() {
        let definitions = improvement_tool_definitions();
        let learning_decide = tool(&definitions, "learning_review_decide");
        let workflow_learning = tool(&definitions, "workflow_learning_list");
        let refinement_propose = tool(&definitions, "skill_refinement_propose");
        let refinement_list = tool(&definitions, "skill_refinement_list");
        let refinement_restore = tool(&definitions, "skill_refinement_restore");

        assert_eq!(required_fields(learning_decide), vec!["id", "approve"]);
        assert!(required_fields(workflow_learning).is_empty());
        assert_eq!(
            required_fields(refinement_propose),
            vec!["skill", "finding", "suggested_change"]
        );
        assert_eq!(required_fields(refinement_restore), vec!["id"]);
        assert_eq!(
            enum_values(property(refinement_propose, "risk")),
            vec!["low", "medium", "high"]
        );
        assert_eq!(
            enum_values(property(refinement_list, "status")),
            vec!["pending", "approved", "denied", "applied", "restored"]
        );
        assert_contains(&workflow_learning.description, "Read-only");
        assert_contains(&workflow_learning.description, "cartes authentifiées");
        assert_contains(&refinement_propose.description, "snapshot restaurable");
        assert_contains(&refinement_restore.description, "backup pre-restore");
    }

    fn tool<'a>(definitions: &'a [ToolDefinition], name: &str) -> &'a ToolDefinition {
        definitions
            .iter()
            .find(|tool| tool.name == name)
            .unwrap_or_else(|| panic!("{name} should be registered"))
    }

    fn required_fields(tool: &ToolDefinition) -> Vec<String> {
        required_fields_from(&tool.input_schema)
    }

    fn required_fields_from(schema: &Value) -> Vec<String> {
        schema
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

    fn assert_contains(haystack: &str, needle: &str) {
        assert!(
            haystack.contains(needle),
            "expected `{haystack}` to contain `{needle}`"
        );
    }
}
