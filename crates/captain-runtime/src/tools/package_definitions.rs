//! Static package-manager tool definitions.

use captain_types::tool::ToolDefinition;

pub fn package_tool_definitions() -> Vec<ToolDefinition> {
    vec![
        ToolDefinition {
            name: "cargo".to_string(),
            description: "Wrapper structuré autour de `cargo` (Rust) avec liste blanche de sous-commandes (build, test, run, check, clippy, fmt, doc, tree, update, install, version, search). Préférer cet outil à shell_exec pour exécuter une commande cargo standard — il bloque les arguments contenant des métacaractères shell pour éviter l'injection. Pour des builds/tests longs, définir timeout_seconds comme fenêtre de réévaluation renouvelable. Pour des invocations exotiques, utiliser shell_exec directement.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "subcommand": { "type": "string", "enum": ["build", "test", "run", "check", "clippy", "fmt", "doc", "tree", "update", "install", "version", "search"], "description": "Sous-commande cargo à exécuter" },
                    "args": { "type": "array", "items": { "type": "string" }, "description": "Arguments additionnels (ex: ['--release', '-p', 'mycrate'])" },
                    "timeout_seconds": { "type": "integer", "description": "Fenêtre de réévaluation en secondes. Si explicitement définie, une commande encore vivante/saine n'est pas tuée à l'échéance; défaut sans paramètre explicite: garde-fou court de shell_exec." }
                },
                "required": ["subcommand"]
            }),
        },
        ToolDefinition {
            name: "npm".to_string(),
            description: "Wrapper structuré autour de `npm` (Node.js) avec liste blanche de sous-commandes (install, ci, run, test, build, list, outdated, audit, version, view). Bloque les arguments avec métacaractères shell. Pour install/build/test longs, définir timeout_seconds comme fenêtre de réévaluation renouvelable. Pour publish ou autres commandes mutantes, utiliser shell_exec.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "subcommand": { "type": "string", "enum": ["install", "ci", "run", "test", "build", "list", "outdated", "audit", "version", "view"], "description": "Sous-commande npm à exécuter" },
                    "args": { "type": "array", "items": { "type": "string" }, "description": "Arguments additionnels (ex: ['--save-dev', 'jest'])" },
                    "timeout_seconds": { "type": "integer", "description": "Fenêtre de réévaluation en secondes. Si explicitement définie, une commande encore vivante/saine n'est pas tuée à l'échéance; défaut sans paramètre explicite: garde-fou court de shell_exec." }
                },
                "required": ["subcommand"]
            }),
        },
        ToolDefinition {
            name: "pip".to_string(),
            description: "Wrapper structuré autour de `pip` (Python) avec liste blanche de sous-commandes (install, list, freeze, show, check, search, download). Bloque les arguments avec métacaractères shell. Pour install/download longs, définir timeout_seconds comme fenêtre de réévaluation renouvelable. Note : la sécurité des paquets Python repose sur la liste blanche externe (pip-allowlist) configurée par ailleurs.".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "subcommand": { "type": "string", "enum": ["install", "list", "freeze", "show", "check", "search", "download"], "description": "Sous-commande pip à exécuter" },
                    "args": { "type": "array", "items": { "type": "string" }, "description": "Arguments additionnels (ex: ['requests==2.32.0'])" },
                    "timeout_seconds": { "type": "integer", "description": "Fenêtre de réévaluation en secondes. Si explicitement définie, une commande encore vivante/saine n'est pas tuée à l'échéance; défaut sans paramètre explicite: garde-fou court de shell_exec." }
                },
                "required": ["subcommand"]
            }),
        },
    ]
}
