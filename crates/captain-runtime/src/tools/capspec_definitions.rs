//! Agent-facing Captain Forge definition.

use captain_types::tool::ToolDefinition;

pub fn capspec_management_tool_definitions() -> Vec<ToolDefinition> {
    vec![ToolDefinition {
        name: "capability_forge".to_string(),
        description: "[CAPTAIN FORGE] Liste, inspecte, valide ou propose une capacité native `.captain` lisible. Utiliser après une demande explicite de l'utilisateur ou lorsqu'un workflow réutilisable est clairement établi. `propose` écrit durablement la source puis recharge sans restart : une capacité strictement read-only peut devenir opérationnelle immédiatement, tandis que toute écriture, commande, réseau, SSH, mémoire mutante ou secret reste en attente d'une approbation humaine liée au hash exact. Ce tool ne sait ni approuver, ni rejeter, ni contourner cette frontière. Toujours expliquer le statut retourné et l'action humaine restante.".to_string(),
        input_schema: serde_json::json!({
            "type": "object",
            "properties": {
                "action": {
                    "type": "string",
                    "enum": ["list", "inspect", "validate", "propose"],
                    "description": "Action sûre. Aucune action d'approbation n'existe côté agent."
                },
                "scope": {
                    "type": "string",
                    "enum": ["effective", "all", "global", "project"],
                    "description": "Portée. `project` utilise le workspace actif; `effective` applique l'override projet."
                },
                "name": {
                    "type": "string",
                    "description": "Nom de la capacité pour inspect. Pour propose, s'il est fourni il doit correspondre au champ name de la source."
                },
                "source": {
                    "type": "string",
                    "description": "Contenu TOML complet du fichier .captain pour validate ou propose."
                },
                "include_source": {
                    "type": "boolean",
                    "description": "Inclure la source versionnée dans inspect. false par défaut."
                }
            },
            "required": ["action"],
            "additionalProperties": false
        }),
    }]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn forge_schema_never_exposes_an_approval_action() {
        let definition = capspec_management_tool_definitions().pop().unwrap();
        assert_eq!(definition.name, "capability_forge");
        let actions = definition.input_schema["properties"]["action"]["enum"]
            .as_array()
            .unwrap();
        assert_eq!(actions.len(), 4);
        assert!(!actions.iter().any(|action| action == "approve"));
    }
}
