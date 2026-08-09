//! Agent-facing immutable artifact tool definitions.

use captain_types::tool::ToolDefinition;

pub fn artifact_tool_definitions() -> Vec<ToolDefinition> {
    vec![
        ToolDefinition {
            name: "artifact_publish".to_string(),
            description: "[ARTEFACT DURABLE] Copie un fichier final du workspace dans le store immutable Captain et retourne son artifact_id, sa version et son SHA-256. Utiliser apres document_create ou toute production de livrable que l'utilisateur doit pouvoir retrouver, verifier, previsualiser ou telecharger. Pour publier une nouvelle version, reutiliser artifact_id. La copie est bornee a 50 Mio, rattachee a l'agent/session reels et refuse les secrets litteraux detectes. Cette operation ecrit durablement et forme une barriere sequentielle; ne pas la lancer en parallele avec artifact_deliver qui depend de son resultat.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "additionalProperties": false,
                "properties": {
                    "path": { "type": "string", "description": "Fichier existant dans le workspace autorise." },
                    "title": { "type": "string", "description": "Titre utilisateur, 1 a 160 caracteres." },
                    "artifact_id": { "type": "string", "format": "uuid", "description": "Optionnel: artefact existant du meme agent pour creer sa version suivante." },
                    "filename": { "type": "string", "description": "Nom de telechargement sans repertoire. Par defaut, basename de path." },
                    "mime_type": { "type": "string", "description": "Type MIME explicite. Par defaut, derive de filename." },
                    "summary": { "type": "string", "description": "Resume utilisateur optionnel, max 1000 caracteres." }
                },
                "required": ["path", "title"]
            }),
        },
        ToolDefinition {
            name: "artifact_list".to_string(),
            description: "[ARTEFACT DURABLE] Liste uniquement les dernieres versions des artefacts appartenant a l'agent appelant. Lecture sans effet externe, utilisable en parallele avec d'autres lectures independantes.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "additionalProperties": false,
                "properties": {
                    "limit": { "type": "integer", "minimum": 1, "maximum": 100, "description": "Nombre maximal, defaut 20." }
                }
            }),
        },
        ToolDefinition {
            name: "artifact_inspect".to_string(),
            description: "[ARTEFACT DURABLE] Relit le manifeste immutable et verifie taille plus SHA-256 d'une version appartenant a l'agent. Omettre version pour la plus recente. Lecture sans effet externe, utilisable en parallele avec d'autres inspections independantes.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "additionalProperties": false,
                "properties": {
                    "artifact_id": { "type": "string", "format": "uuid" },
                    "version": { "type": "integer", "minimum": 1 }
                },
                "required": ["artifact_id"]
            }),
        },
        ToolDefinition {
            name: "artifact_deliver".to_string(),
            description: "[LIVRAISON ARTEFACT] Verifie puis envoie une version immutable via un canal actif en upload natif, sans exposer son chemin local. Cette operation produit un effet externe et doit rester sequentielle. Appeler seulement apres artifact_publish ou artifact_inspect; ne jamais la mettre en parallele avec l'appel dont elle utilise l'artifact_id.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "additionalProperties": false,
                "properties": {
                    "artifact_id": { "type": "string", "format": "uuid" },
                    "version": { "type": "integer", "minimum": 1 },
                    "channel": { "type": "string", "description": "Canal actif: telegram, discord, signal ou email." },
                    "recipient": { "type": "string", "description": "Destinataire; optionnel seulement si le canal a un destinataire par defaut." },
                    "caption": { "type": "string", "description": "Legende optionnelle pour une image; par defaut le titre." },
                    "thread_id": { "type": "string", "description": "Thread/topic optionnel." }
                },
                "required": ["artifact_id", "channel"]
            }),
        },
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn definitions_pin_immutable_and_dependency_contracts() {
        let definitions = artifact_tool_definitions();
        let names: Vec<_> = definitions.iter().map(|tool| tool.name.as_str()).collect();
        assert_eq!(
            names,
            vec![
                "artifact_publish",
                "artifact_list",
                "artifact_inspect",
                "artifact_deliver"
            ]
        );
        let publish = &definitions[0];
        assert!(publish.description.contains("50 Mio"));
        assert!(publish.description.contains("barriere sequentielle"));
        assert_eq!(
            publish.input_schema["required"],
            serde_json::json!(["path", "title"])
        );
        let deliver = &definitions[3];
        assert!(deliver.description.contains("effet externe"));
        assert!(deliver
            .description
            .contains("ne jamais la mettre en parallele"));
    }

    #[test]
    fn builtin_registry_exposes_each_artifact_tool_once() {
        let all = crate::tools::builtin_tool_definitions();
        for name in [
            "artifact_publish",
            "artifact_list",
            "artifact_inspect",
            "artifact_deliver",
        ] {
            assert_eq!(
                all.iter().filter(|tool| tool.name == name).count(),
                1,
                "{name}"
            );
        }
    }
}
