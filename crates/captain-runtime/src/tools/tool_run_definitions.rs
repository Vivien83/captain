//! Tool run supervision definitions.

use captain_types::tool::ToolDefinition;

pub fn tool_run_tool_definitions() -> Vec<ToolDefinition> {
    vec![
        ToolDefinition {
            name: "tool_run_start".to_string(),
            description: "[OUTIL DETACHE] Lance un outil long eligible en arriere-plan et retourne immediatement un run_id consultable avec tool_run_status/tool_run_result. Utiliser quand une verification shell/SSH/build peut durer, quand plusieurs sondes independantes peuvent tourner en parallele, ou quand il faut eviter de bloquer le tour agent. Ne lancer en parallele que des outils independants; si un outil depend du resultat d'un autre, renseigner depends_on avec les run_id requis et Captain refusera le depart tant qu'ils ne sont pas completed. Outils detachables au premier niveau: shell_exec, ssh_exec, ssh_health_check, execute_code, cargo, npm, pip. Les validations habituelles de l'outil cible restent appliquees.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "tool_name": { "type": "string", "description": "Nom de l'outil cible a lancer en arriere-plan." },
                    "input": { "type": "object", "description": "Input JSON a transmettre a l'outil cible." },
                    "reason": { "type": "string", "description": "Raison courte operator-safe expliquant pourquoi le run est detache." },
                    "depends_on": { "type": "array", "items": { "type": "string" }, "description": "Run ids qui doivent etre completed avant de lancer cet outil. Utiliser quand l'input depend du resultat d'un run precedent." }
                },
                "required": ["tool_name", "input"]
            }),
        },
        ToolDefinition {
            name: "tool_run_status".to_string(),
            description: "[OUTIL DETACHE] Consulte l'etat courant d'un run d'outil: running, completed, failed, cancelled ou interrupted apres un restart, avec duree et preview bornee. A utiliser pour revenir voir une execution lancee via tool_run_start.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "run_id": { "type": "string", "description": "Identifiant retourne par tool_run_start." }
                },
                "required": ["run_id"]
            }),
        },
        ToolDefinition {
            name: "tool_run_retry".to_string(),
            description: "[LIVE RUN] Relance explicitement un run failed, cancelled ou interrupted avec le meme outil et exactement le meme input. Le ledger Live Runs ne conserve pas l'input brut: il faut le fournir de nouveau et son SHA-256 doit correspondre. Un run interrupted peut avoir deja produit un effet externe et exige acknowledge_uncertain_effect egal au run_id apres inspection du resultat. La lignee est auditee et coupee apres trois retries. Pour corriger ou changer l'input, utiliser tool_run_start comme nouvelle intention.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "run_id": { "type": "string", "description": "Run failed, cancelled ou interrupted a relancer." },
                    "input": { "type": "object", "description": "Input JSON exact du run precedent; il n'est pas recupere depuis le ledger." },
                    "acknowledge_uncertain_effect": { "type": "string", "description": "Pour un run interrupted uniquement: recopier exactement run_id apres avoir verifie que repeter l'effet est acceptable." },
                    "reason": { "type": "string", "description": "Raison courte et operator-safe de la reprise." }
                },
                "required": ["run_id", "input"]
            }),
        },
        ToolDefinition {
            name: "tool_run_result".to_string(),
            description: "[LIVE RUN] Lit le resultat borne d'un run d'outil termine ou son etat courant s'il tourne encore. Si output_available=true, utiliser tool_run_read, tool_run_tail ou tool_run_search pour consulter la preuve complete retenue sans relancer l'outil ni charger toute la sortie dans le contexte.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "run_id": { "type": "string", "description": "Identifiant retourne par tool_run_start." }
                },
                "required": ["run_id"]
            }),
        },
        ToolDefinition {
            name: "tool_run_read".to_string(),
            description: "[LIVE RUN] Lit une page de lignes de la sortie retenue d'un run foreground ou detache. La sortie durable est bornee, redigee, verifiee par checksum et reste hors du contexte tant que cet outil n'en demande pas une page.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "run_id": { "type": "string", "description": "Identifiant du run a inspecter." },
                    "start_line": { "type": "integer", "minimum": 1, "description": "Premiere ligne, base 1. Defaut 1." },
                    "max_lines": { "type": "integer", "minimum": 1, "maximum": 500, "description": "Nombre de lignes, defaut 100, max 500." }
                },
                "required": ["run_id"]
            }),
        },
        ToolDefinition {
            name: "tool_run_tail".to_string(),
            description: "[LIVE RUN] Lit les dernieres lignes actuellement disponibles d'un run foreground ou detache. Utiliser pour suivre un outil actif sans bloquer la conversation ni reinjecter toute sa sortie.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "run_id": { "type": "string", "description": "Identifiant du run a suivre." },
                    "max_lines": { "type": "integer", "minimum": 1, "maximum": 500, "description": "Nombre de lignes finales, defaut 100, max 500." }
                },
                "required": ["run_id"]
            }),
        },
        ToolDefinition {
            name: "tool_run_search".to_string(),
            description: "[LIVE RUN] Recherche un texte dans la sortie retenue d'un run foreground ou detache et retourne seulement les lignes correspondantes. Preferer cet outil a une reexecution ou a la lecture complete d'un gros log.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "run_id": { "type": "string", "description": "Identifiant du run a rechercher." },
                    "query": { "type": "string", "description": "Texte non vide a rechercher." },
                    "max_matches": { "type": "integer", "minimum": 1, "maximum": 100, "description": "Nombre maximal de lignes, defaut 20." },
                    "case_sensitive": { "type": "boolean", "description": "Recherche sensible a la casse, defaut false." }
                },
                "required": ["run_id", "query"]
            }),
        },
        ToolDefinition {
            name: "tool_run_cancel".to_string(),
            description: "[OUTIL DETACHE] Annule un run d'outil detache encore actif, si Captain possede un handle d'annulation.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "run_id": { "type": "string", "description": "Identifiant retourne par tool_run_start." }
                },
                "required": ["run_id"]
            }),
        },
        ToolDefinition {
            name: "tool_run_list".to_string(),
            description: "[OUTIL DETACHE] Liste les runs d'outils recents, optionnellement filtres par statut. Utile pour reprendre une verification en cours ou auditer les derniers outils lances.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "status": { "type": "string", "enum": ["running", "completed", "failed", "cancelled", "interrupted"], "description": "Filtre optionnel par statut, y compris interrupted apres un restart." },
                    "limit": { "type": "integer", "description": "Nombre max de runs a retourner, defaut 20, max 50." }
                }
            }),
        },
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tool_run_definitions_expose_supervision_contract() {
        let definitions = tool_run_tool_definitions();
        let names: Vec<_> = definitions.iter().map(|tool| tool.name.as_str()).collect();
        assert!(names.contains(&"tool_run_start"));
        assert!(names.contains(&"tool_run_retry"));
        assert!(names.contains(&"tool_run_status"));
        assert!(names.contains(&"tool_run_result"));
        assert!(names.contains(&"tool_run_read"));
        assert!(names.contains(&"tool_run_tail"));
        assert!(names.contains(&"tool_run_search"));
        assert!(names.contains(&"tool_run_cancel"));
        assert!(names.contains(&"tool_run_list"));
        let start = definitions
            .iter()
            .find(|tool| tool.name == "tool_run_start")
            .unwrap();
        assert!(start.description.contains("arriere-plan"));
        assert!(start.description.contains("ssh_exec"));
        assert!(start.description.contains("depends_on"));
        assert!(start.description.contains("independants"));
        assert!(start.description.contains("validations habituelles"));
        let list = definitions
            .iter()
            .find(|tool| tool.name == "tool_run_list")
            .unwrap();
        assert!(list.input_schema["properties"]["status"]["enum"]
            .as_array()
            .unwrap()
            .iter()
            .any(|status| status == "interrupted"));
    }
}
