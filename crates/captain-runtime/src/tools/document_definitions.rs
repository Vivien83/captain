//! Static document tool definitions.

use captain_types::tool::ToolDefinition;
use serde_json::Value;

pub fn document_tool_definitions() -> Vec<ToolDefinition> {
    let mut definitions = native_document_tool_definitions();
    definitions.extend(document_intake_tool_definitions());
    definitions.extend(document_delivery_pipeline_tool_definitions());
    definitions
}

fn native_document_tool_definitions() -> Vec<ToolDefinition> {
    vec![document_create_tool_definition()]
}

fn document_intake_tool_definitions() -> Vec<ToolDefinition> {
    vec![document_extract_tool_definition()]
}

fn document_delivery_pipeline_tool_definitions() -> Vec<ToolDefinition> {
    vec![document_pipeline_tool_definition()]
}

fn tool_definition(name: &str, description: &str, input_schema: Value) -> ToolDefinition {
    ToolDefinition {
        name: name.to_string(),
        description: description.to_string(),
        input_schema,
    }
}

fn document_create_tool_definition() -> ToolDefinition {
    tool_definition(
        "document_create",
        "[DOCUMENT NATIF] Génère un document propre dans l'espace de travail, sans dépendre d'un binaire externe. À utiliser pour rapports de recherche, synthèses, factures simples, courriers, comptes rendus et livrables partageables. Formats supportés: pdf, docx, html, markdown. Accepte du contenu Markdown simple ou des sections structurées avec titres, paragraphes, listes, tableaux et sources. Refuse d'écraser un fichier existant sauf overwrite=true. Pour envoyer ensuite le fichier sur Telegram, Discord, Signal ou Email, utiliser channel_send avec file_path.",
        serde_json::json!({
            "type": "object",
            "properties": {
                "format": { "type": "string", "enum": ["pdf", "docx", "html", "markdown"], "description": "Format de sortie. Défaut: inféré depuis path, sinon pdf." },
                "path": { "type": "string", "description": "Chemin de sortie relatif au workspace. Défaut: documents/<slug-du-titre>.<extension>." },
                "title": { "type": "string", "description": "Titre du document. Défaut: Captain document." },
                "subtitle": { "type": "string", "description": "Sous-titre optionnel." },
                "author": { "type": "string", "description": "Auteur optionnel." },
                "content": { "type": "string", "description": "Corps Markdown simple: titres #/##, paragraphes, listes '- item', tableaux pipe Markdown." },
                "sections": {
                    "type": "array",
                    "description": "Sections structurées optionnelles, ajoutées après content.",
                    "items": {
                        "type": "object",
                        "properties": {
                            "heading": { "type": "string" },
                            "level": { "type": "integer", "minimum": 1, "maximum": 6 },
                            "body": { "type": "string" },
                            "bullets": { "type": "array", "items": { "type": "string" } },
                            "table": {
                                "type": "object",
                                "properties": {
                                    "headers": { "type": "array", "items": { "type": "string" } },
                                    "rows": {
                                        "type": "array",
                                        "items": { "type": "array", "items": { "type": "string" } }
                                    }
                                },
                                "required": ["headers", "rows"]
                            }
                        }
                    }
                },
                "citations": {
                    "type": "array",
                    "description": "Sources à ajouter en fin de document.",
                    "items": {
                        "type": "object",
                        "properties": {
                            "id": { "type": "string" },
                            "title": { "type": "string" },
                            "url": { "type": "string" },
                            "accessed_at": { "type": "string" }
                        }
                    }
                },
                "overwrite": { "type": "boolean", "description": "Remplacer le fichier s'il existe déjà. Défaut: false." }
            },
            "required": []
        }),
    )
}

fn document_extract_tool_definition() -> ToolDefinition {
    tool_definition(
        "document_extract",
        "[EXTRACTION DOCUMENT] Lit un document du workspace et extrait son texte pour recherche/synthèse. À utiliser après web_download sur PDF/HTML/Markdown/TXT/CSV/JSON/XML avant de citer ou résumer un rapport. Pour les PDF, extrait le texte embarqué sans OCR; si le PDF est scanné/image-only, retourne une erreur explicite afin d'éviter d'inventer. Ne remplace pas document_create: il sert à lire une source, pas à créer un livrable.",
        serde_json::json!({
            "type": "object",
            "properties": {
                "path": { "type": "string", "description": "Chemin du document dans le workspace, souvent retourné par web_download." },
                "max_chars": { "type": "integer", "description": "Nombre max de caractères extraits à retourner. Défaut 50000, max 200000." }
            },
            "required": ["path"]
        }),
    )
}

fn document_pipeline_tool_definition() -> ToolDefinition {
    tool_definition(
        "document_pipeline",
        "[DOCUMENT PIPELINE] Crée un document natif via document_create puis peut l'envoyer en une étape via channel_send. Utiliser pour livrables prêts à partager: rapport PDF/DOCX/HTML/Markdown + livraison Telegram/Discord/Signal/Email. Le champ document contient le schema document_create; send contient channel/recipient/message optionnels.",
        serde_json::json!({
            "type": "object",
            "properties": {
                "document": { "type": "object", "description": "Input complet de document_create: title, content/sections, format, path, citations, overwrite." },
                "send": { "type": "object", "description": "Optionnel. Input channel_send sans file_path: channel, recipient, message, topic_id/thread_id." }
            },
            "required": ["document"]
        }),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn document_tool_definitions_keep_public_order() {
        let tools = document_tool_definitions();
        let names: Vec<_> = tools.iter().map(|tool| tool.name.as_str()).collect();

        assert_eq!(
            names,
            vec!["document_create", "document_extract", "document_pipeline"]
        );
    }

    #[test]
    fn document_tool_definitions_keep_create_schema_contracts() {
        let tools = document_tool_definitions();
        let create = tool(&tools, "document_create");
        let section_item = &property(create, "sections")["items"];
        let table = &section_item["properties"]["table"];

        assert!(required_fields(create).is_empty());
        assert_eq!(
            enum_values(property(create, "format")),
            vec!["pdf", "docx", "html", "markdown"]
        );
        assert_eq!(
            integer_field(&section_item["properties"]["level"], "minimum"),
            Some(1)
        );
        assert_eq!(
            integer_field(&section_item["properties"]["level"], "maximum"),
            Some(6)
        );
        assert_eq!(required_fields_from(table), vec!["headers", "rows"]);
        assert_contains(&create.description, "sans dépendre d'un binaire externe");
        assert_contains(&create.description, "overwrite=true");
        assert_contains(&create.description, "Telegram, Discord, Signal ou Email");
    }

    #[test]
    fn document_tool_definitions_keep_extract_contracts() {
        let tools = document_tool_definitions();
        let extract = tool(&tools, "document_extract");

        assert_eq!(required_fields(extract), vec!["path"]);
        assert_contains(&extract.description, "web_download");
        assert_contains(&extract.description, "sans OCR");
        assert_contains(&extract.description, "éviter d'inventer");
        assert_contains(
            property(extract, "max_chars")["description"]
                .as_str()
                .unwrap_or_default(),
            "max 200000",
        );
    }

    #[test]
    fn document_tool_definitions_keep_pipeline_contracts() {
        let tools = document_tool_definitions();
        let pipeline = tool(&tools, "document_pipeline");

        assert_eq!(required_fields(pipeline), vec!["document"]);
        assert_contains(&pipeline.description, "document_create");
        assert_contains(&pipeline.description, "channel_send");
        assert_contains(&pipeline.description, "Telegram/Discord/Signal/Email");
        assert_contains(
            property(pipeline, "send")["description"]
                .as_str()
                .unwrap_or_default(),
            "sans file_path",
        );
    }

    fn tool<'a>(tools: &'a [ToolDefinition], name: &str) -> &'a ToolDefinition {
        tools
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

    fn enum_values(value: &Value) -> Vec<&str> {
        value
            .get("enum")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
            .collect()
    }

    fn integer_field(value: &Value, name: &str) -> Option<u64> {
        value.get(name).and_then(Value::as_u64)
    }

    fn assert_contains(haystack: &str, needle: &str) {
        assert!(
            haystack.contains(needle),
            "expected `{haystack}` to contain `{needle}`"
        );
    }
}
