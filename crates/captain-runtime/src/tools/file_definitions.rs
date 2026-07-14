//! Static filesystem tool definitions.

use captain_types::tool::ToolDefinition;
use serde_json::Value;

pub fn file_tool_definitions() -> Vec<ToolDefinition> {
    let mut definitions = file_batch_tool_definitions();
    definitions.extend(basic_file_tool_definitions());
    definitions.extend(file_patch_edit_tool_definitions());
    definitions.extend(file_search_tool_definitions());
    definitions.extend(atomic_file_edit_tool_definitions());
    definitions
}

fn file_batch_tool_definitions() -> Vec<ToolDefinition> {
    vec![file_inspect_batch_tool_definition()]
}

fn basic_file_tool_definitions() -> Vec<ToolDefinition> {
    vec![
        file_read_tool_definition(),
        file_write_tool_definition(),
        file_list_tool_definition(),
    ]
}

fn file_patch_edit_tool_definitions() -> Vec<ToolDefinition> {
    vec![apply_patch_tool_definition(), edit_file_tool_definition()]
}

fn file_search_tool_definitions() -> Vec<ToolDefinition> {
    vec![glob_tool_definition(), grep_tool_definition()]
}

fn atomic_file_edit_tool_definitions() -> Vec<ToolDefinition> {
    vec![multi_edit_tool_definition()]
}

fn tool_definition(name: &str, description: &str, input_schema: Value) -> ToolDefinition {
    ToolDefinition {
        name: name.to_string(),
        description: description.to_string(),
        input_schema,
    }
}

fn file_inspect_batch_tool_definition() -> ToolDefinition {
    tool_definition(
        "file_inspect_batch",
        "[LECTURE GROUPEE] Exécute plusieurs opérations de lecture filesystem en un appel: glob, grep, read, list. À utiliser pour explorer un repo/dossier sans multiplier les appels tool. Lecture seule: n'écrit jamais. Les reads sont tronqués par max_read_chars.",
        serde_json::json!({
            "type": "object",
            "properties": {
                "operations": {
                    "type": "array",
                    "maxItems": 30,
                    "items": {
                        "type": "object",
                        "properties": {
                            "action": { "type": "string", "description": "glob, grep, read, or list" },
                            "path": { "type": "string" },
                            "pattern": { "type": "string" },
                            "glob": { "type": "string" },
                            "type": { "type": "string" },
                            "output_mode": { "type": "string" },
                            "head_limit": { "type": "integer" }
                        },
                        "required": ["action"]
                    }
                },
                "max_read_chars": { "type": "integer", "description": "Troncature des lectures, défaut 12000, max 50000." },
                "stop_on_error": { "type": "boolean", "description": "Stoppe au premier échec. Défaut false." }
            },
            "required": ["operations"]
        }),
    )
}

fn file_read_tool_definition() -> ToolDefinition {
    tool_definition(
        "file_read",
        "Lit le contenu d'un fichier texte dans l'espace de travail de l'agent. Utiliser pour inspecter des fichiers de config, logs, scripts ou données avant de les modifier. Ne pas utiliser pour lister des répertoires (utiliser file_list) ni pour des fichiers binaires. Retourne le contenu brut sous forme de chaîne UTF-8.",
        serde_json::json!({
            "type": "object",
            "properties": {
                "path": { "type": "string", "description": "Chemin du fichier à lire, relatif à l'espace de travail de l'agent (ex: 'config/settings.json', 'logs/app.log')" }
            },
            "required": ["path"]
        }),
    )
}

fn file_write_tool_definition() -> ToolDefinition {
    tool_definition(
        "file_write",
        "[CRÉATION OU ÉCRASEMENT TOTAL] Écrit un fichier en remplaçant TOUT son contenu. À utiliser SEULEMENT pour : (1) CRÉER un fichier qui n'existe pas, (2) RÉ-ÉCRIRE intégralement un fichier dont on a généré l'ensemble du contenu. Pour MODIFIER un fichier existant (changer une ligne, ajuster une valeur, ajouter une ligne), utiliser `edit_file` (plus sûr — pas de risque d'effacer le reste). Pour multiples modifications atomiques d'un fichier existant : `multi_edit`. Pour patch diff multi-hunk : `apply_patch`. Ne pas utiliser pour fichiers binaires ni pour écrire un secret brut: stocker avec secret_write et référencer seulement le nom d'env.",
        serde_json::json!({
            "type": "object",
            "properties": {
                "path": { "type": "string", "description": "Chemin du fichier à écrire, relatif à l'espace de travail (les répertoires parents sont créés automatiquement)" },
                "content": { "type": "string", "description": "Contenu complet à écrire dans le fichier (écrase tout contenu existant)" }
            },
            "required": ["path", "content"]
        }),
    )
}

fn file_list_tool_definition() -> ToolDefinition {
    tool_definition(
        "file_list",
        "[NON-RÉCURSIF — UN SEUL NIVEAU] Liste les fichiers et sous-répertoires immédiats d'UN répertoire (n'entre PAS dans les sous-dossiers). Utiliser pour explorer la racine d'un dossier précis, vérifier l'existence d'un fichier connu, voir ce qui est juste sous un chemin. Pour parcourir RÉCURSIVEMENT plusieurs niveaux ou matcher un pattern (`*.rs`, `**/*.ts`), utiliser le tool `glob` à la place. Ne retourne pas le contenu des fichiers — utiliser file_read pour cela. Retourne un tableau de noms avec indication du type (fichier ou répertoire).",
        serde_json::json!({
            "type": "object",
            "properties": {
                "path": { "type": "string", "description": "Chemin du répertoire à lister, relatif à l'espace de travail (utiliser '.' pour la racine)" }
            },
            "required": ["path"]
        }),
    )
}

fn apply_patch_tool_definition() -> ToolDefinition {
    tool_definition(
        "apply_patch",
        "Applique un patch diff multi-hunk pour ajouter, modifier, déplacer ou supprimer des fichiers de manière chirurgicale. Préférer cet outil à file_write pour toute modification partielle d'un fichier existant — il est plus précis et évite d'écraser du contenu non ciblé. Ne pas utiliser pour créer un fichier entièrement nouveau (utiliser file_write à la place). Les lignes ajoutées contenant un secret brut sont refusées: utiliser le vault + env_inject. Retourne un rapport des fichiers modifiés et des hunks appliqués.",
        serde_json::json!({
            "type": "object",
            "properties": {
                "patch": {
                    "type": "string",
                    "description": "Patch au format '*** Begin Patch' / '*** End Patch'. Marqueurs de section : '*** Add File: <path>', '*** Update File: <path>', '*** Delete File: <path>'. Chaque hunk commence par un header '@@ ... @@' suivi de lignes préfixées par ' ' (contexte), '-' (supprimer), '+' (ajouter)."
                }
            },
            "required": ["patch"]
        }),
    )
}

fn edit_file_tool_definition() -> ToolDefinition {
    tool_definition(
        "edit_file",
        "[CHOIX PAR DÉFAUT pour MODIFIER un fichier existant] Remplace une chaîne par une autre dans un fichier existant (style str_replace de l'API Anthropic). À choisir AVANT apply_patch (plus simple, pas de hunks à fabriquer) et AVANT file_write (qui écrase tout — risqué). La chaîne `old_string` doit identifier sans ambiguïté une portion unique du fichier ; sinon une chaîne de 8 stratégies de fallback (whitespace, indentation, escape, ancres) tente de la matcher. Pour remplacer toutes les occurrences, passer `replace_all: true`. Ne pas utiliser pour CRÉER un fichier (file_write) ni pour PLUSIEURS substitutions atomiques sur le même fichier (multi_edit). `new_string` ne doit jamais contenir de secret brut. Retourne le nom de la stratégie qui a réussi et le nombre de remplacements.",
        serde_json::json!({
            "type": "object",
            "properties": {
                "path": { "type": "string", "description": "Chemin du fichier à modifier, relatif à l'espace de travail" },
                "old_string": { "type": "string", "description": "Chaîne à rechercher (doit être unique sauf si replace_all=true)" },
                "new_string": { "type": "string", "description": "Chaîne de remplacement (peut être vide pour supprimer)" },
                "replace_all": { "type": "boolean", "description": "Si true, remplace toutes les occurrences de old_string. Défaut : false (un seul remplacement attendu)." }
            },
            "required": ["path", "old_string", "new_string"]
        }),
    )
}

fn glob_tool_definition() -> ToolDefinition {
    tool_definition(
        "glob",
        "[RÉCURSIF + PATTERN MATCHING] Liste les fichiers du workspace dont le chemin correspond à un pattern glob style ripgrep, descendant RÉCURSIVEMENT dans tous les sous-répertoires, gitignore-aware. C'est le bon choix dès qu'il faut : (1) une RECHERCHE par extension (`*.rs`, `**/*.ts`), (2) un parcours MULTI-NIVEAUX (`src/**/*.py`), (3) un FILTRAGE par nom de fichier (`**/test_*.py`, `**/CHANGELOG*`). À l'inverse, pour lister un seul niveau d'un dossier connu, utiliser `file_list`. Préférer cet outil à shell_exec/find. Résultats triés par mtime décroissant (le plus récent en premier). Cap à `head_limit` (défaut 1000).",
        serde_json::json!({
            "type": "object",
            "properties": {
                "pattern": { "type": "string", "description": "Pattern glob (ex '*.rs', 'src/**/*.{ts,tsx}', '**/CHANGELOG*')" },
                "path": { "type": "string", "description": "Racine de la recherche, relative au workspace (défaut '.')" },
                "head_limit": { "type": "integer", "description": "Nombre max de résultats (défaut 1000)" }
            },
            "required": ["pattern"]
        }),
    )
}

fn grep_tool_definition() -> ToolDefinition {
    tool_definition(
        "grep",
        "Recherche de contenu dans les fichiers du workspace via une expression régulière, style ripgrep (gitignore-aware, embedded — sans shell out vers `rg`). Préférer cet outil à shell_exec quand il s'agit de chercher du code, des TODO, des références à un symbole, des occurrences d'une chaîne. Trois modes de sortie : `files_with_matches` (défaut, juste les chemins), `content` (lignes matchées avec contexte A/B/C optionnel), `count` (nombre par fichier). Filtres : `glob` (pattern style Unix), `type` (alias rust/ts/js/py/go/java/md/...), `-i` (case-insensitive), `multiline` (`.` matche `\\n`). Cap à `head_limit` résultats (défaut 250). Skippe automatiquement les fichiers > 5 MB et binaires.",
        serde_json::json!({
            "type": "object",
            "properties": {
                "pattern": { "type": "string", "description": "Expression régulière à rechercher" },
                "path": { "type": "string", "description": "Racine de la recherche, relative au workspace (défaut '.')" },
                "glob": { "type": "string", "description": "Filtre glob (ex '*.rs', 'src/**/*.ts')" },
                "type": { "type": "string", "description": "Alias d'extension : rust, ts, js, py, go, java, c, cpp, md, toml, yaml, json, html, css, sh" },
                "output_mode": { "type": "string", "enum": ["content", "files_with_matches", "count"], "description": "Format de sortie. Défaut : files_with_matches." },
                "-A": { "type": "integer", "description": "Lignes de contexte AFTER chaque match (output_mode=content uniquement)" },
                "-B": { "type": "integer", "description": "Lignes de contexte BEFORE chaque match" },
                "-C": { "type": "integer", "description": "Lignes de contexte AVANT ET APRÈS (alias de -A=-B=N)" },
                "-i": { "type": "boolean", "description": "Case-insensitive matching" },
                "multiline": { "type": "boolean", "description": "Le `.` du regex matche aussi `\\n` (patterns multi-lignes)" },
                "head_limit": { "type": "integer", "description": "Nombre max de résultats (défaut 250)" }
            },
            "required": ["pattern"]
        }),
    )
}

fn multi_edit_tool_definition() -> ToolDefinition {
    tool_definition(
        "multi_edit",
        "Applique une chaîne de plusieurs substitutions str_replace sur un même fichier de manière ATOMIQUE : si l'une des éditions échoue, aucune n'est écrite (le fichier reste inchangé). Préférer cet outil à plusieurs appels successifs d'edit_file lorsqu'un ensemble cohérent de modifications doit s'appliquer ensemble. Chaque édition utilise la même chaîne de 8 stratégies de fallback qu'edit_file. Les éditions sont appliquées séquentiellement en mémoire ; chacune voit le résultat de la précédente. Les `new_string` contenant un secret brut sont refusées. Retourne le nombre d'éditions appliquées et la stratégie utilisée pour chaque.",
        serde_json::json!({
            "type": "object",
            "properties": {
                "path": { "type": "string", "description": "Chemin du fichier à modifier, relatif à l'espace de travail" },
                "edits": {
                    "type": "array",
                    "description": "Liste ordonnée d'éditions à appliquer atomiquement",
                    "items": {
                        "type": "object",
                        "properties": {
                            "old_string": { "type": "string", "description": "Chaîne à rechercher" },
                            "new_string": { "type": "string", "description": "Chaîne de remplacement" },
                            "replace_all": { "type": "boolean", "description": "Si true, remplace toutes les occurrences. Défaut : false." }
                        },
                        "required": ["old_string", "new_string"]
                    }
                }
            },
            "required": ["path", "edits"]
        }),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn file_tool_definitions_keep_public_order() {
        let definitions = file_tool_definitions();
        let names: Vec<_> = definitions.iter().map(|tool| tool.name.as_str()).collect();

        assert_eq!(
            names,
            vec![
                "file_inspect_batch",
                "file_read",
                "file_write",
                "file_list",
                "apply_patch",
                "edit_file",
                "glob",
                "grep",
                "multi_edit",
            ]
        );
    }

    #[test]
    fn file_tool_definitions_keep_read_write_contracts() {
        let definitions = file_tool_definitions();
        let batch = tool(&definitions, "file_inspect_batch");
        let read = tool(&definitions, "file_read");
        let write = tool(&definitions, "file_write");
        let list = tool(&definitions, "file_list");

        assert_eq!(required_fields(batch), vec!["operations"]);
        assert_eq!(
            integer_field(property(batch, "operations"), "maxItems"),
            Some(30)
        );
        assert_eq!(
            required_fields_from(
                property(batch, "operations")
                    .get("items")
                    .expect("operations should define items")
            ),
            vec!["action"]
        );
        assert_eq!(required_fields(read), vec!["path"]);
        assert_eq!(required_fields(write), vec!["path", "content"]);
        assert_contains(&write.description, "edit_file");
        assert_contains(&write.description, "secret_write");
        assert_eq!(required_fields(list), vec!["path"]);
        assert_contains(&list.description, "NON-RÉCURSIF");
        assert_contains(&list.description, "glob");
    }

    #[test]
    fn file_tool_definitions_keep_edit_and_search_contracts() {
        let definitions = file_tool_definitions();
        let patch = tool(&definitions, "apply_patch");
        let edit = tool(&definitions, "edit_file");
        let glob = tool(&definitions, "glob");
        let grep = tool(&definitions, "grep");
        let multi = tool(&definitions, "multi_edit");

        assert_eq!(required_fields(patch), vec!["patch"]);
        assert_eq!(
            required_fields(edit),
            vec!["path", "old_string", "new_string"]
        );
        assert_contains(&edit.description, "CHOIX PAR DÉFAUT");
        assert_contains(&edit.description, "replace_all");
        assert_eq!(required_fields(glob), vec!["pattern"]);
        assert_eq!(
            enum_values(property(grep, "output_mode")),
            vec!["content", "files_with_matches", "count"]
        );
        assert_eq!(required_fields(grep), vec!["pattern"]);
        assert_eq!(required_fields(multi), vec!["path", "edits"]);
        assert_eq!(
            required_fields_from(
                property(multi, "edits")
                    .get("items")
                    .expect("edits should define items")
            ),
            vec!["old_string", "new_string"]
        );
        assert_contains(&multi.description, "ATOMIQUE");
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
