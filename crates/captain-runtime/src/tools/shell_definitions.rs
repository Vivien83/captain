//! Static shell/code execution tool definitions.

use captain_types::tool::ToolDefinition;
use serde_json::Value;

pub fn shell_tool_definitions() -> Vec<ToolDefinition> {
    let mut definitions = code_execution_tool_definitions();
    definitions.extend(shell_execution_tool_definitions());
    definitions.extend(process_tool_definitions());
    definitions
}

fn code_execution_tool_definitions() -> Vec<ToolDefinition> {
    vec![execute_code_tool_definition()]
}

fn shell_execution_tool_definitions() -> Vec<ToolDefinition> {
    vec![shell_exec_tool_definition(), docker_exec_tool_definition()]
}

fn process_tool_definitions() -> Vec<ToolDefinition> {
    vec![
        process_start_tool_definition(),
        process_poll_tool_definition(),
        process_write_tool_definition(),
        process_kill_tool_definition(),
        process_list_tool_definition(),
    ]
}

fn tool_definition(name: &str, description: &str, input_schema: Value) -> ToolDefinition {
    ToolDefinition {
        name: name.to_string(),
        description: description.to_string(),
        input_schema,
    }
}

fn execute_code_tool_definition() -> ToolDefinition {
    tool_definition(
        "execute_code",
        "Execute un extrait de code (Python par defaut, Node.js, Bash) directement — sans creer de fichier ni skill. Utiliser pour manipuler des donnees structurees ou prototyper un script a la volee. Ne pas utiliser pour des actions systeme simples (prefer shell_exec), du code persistent (prefer file_write puis shell_exec), ni pour coller une clé API en clair. Pour une API avec credential: vault + integration native ou skill env_inject. Timeout 60s par defaut. Si timeout_secs est explicitement defini, c'est une fenetre de reevaluation renouvelable: un processus encore vivant n'est pas tue a l'echeance. Retourne un JSON avec stdout, stderr, exit_code.",
        serde_json::json!({
            "type": "object",
            "properties": {
                "code": { "type": "string", "description": "Code source a executer" },
                "language": { "type": "string", "enum": ["python", "node", "bash"], "description": "Langage (default: python)" },
                "pip_install": {
                    "type": "array",
                    "items": { "type": "string" },
                    "description": "Packages Python a installer avant execution (allowlist: requests, httpx, beautifulsoup4, lxml, pandas, numpy, pyyaml, python-dateutil, pyobjc-framework-Quartz, pillow). Ignore pour node/bash."
                },
                "timeout_secs": { "type": "number", "description": "Fenetre de reevaluation en secondes (max 300) quand elle est explicite; defaut sans parametre explicite: garde-fou court de 60s." }
            },
            "required": ["code"]
        }),
    )
}

fn shell_exec_tool_definition() -> ToolDefinition {
    tool_definition(
        "shell_exec",
        "Exécute une commande shell dans l'environnement du serveur et retourne stdout+stderr. Utiliser pour des opérations système, scripts, compilations, ou toute tâche nécessitant le shell. Pour une question de version binaire/runtime Captain ou de statut live, utiliser `captain --version`, `captain status` ou l'API locale `/api/status` plutôt qu'un changelog historique. Ne pas utiliser comme premier réflexe sur une demande actionnable fraîche : commence par capability_search pour choisir le bon rail. Ne pas utiliser pour diagnostiquer le vault/SSH Captain (préférer ssh_exec + captain_docs), lire/écrire des fichiers simples (file_read/file_write), des commandes destructives irréversibles sans validation, ni passer une clé API brute en argument/env inline. Ne jamais sourcer `~/.captain/secrets.env` dans un shell : certaines clés sont des identifiants logiques non compatibles shell. Les secrets doivent venir du vault via secret_read, intégration native ou skill env_inject. Pour une compilation/test longue, définir timeout_seconds comme fenêtre de réévaluation bornée: Captain émet du progrès, renouvelle quelques fenêtres puis applique un plafond dur. Éviter les commandes de monitoring sans fin (`log stream`, `tail -f`, `pmset -g thermlog`) dans shell_exec; utiliser process_start pour serveurs/watchers. Retourne la sortie combinée (stdout et stderr) avec le code de retour.",
        serde_json::json!({
            "type": "object",
            "properties": {
                "command": { "type": "string", "description": "Commande shell complète à exécuter (ex: 'cargo build --release', 'ls -la /tmp')" },
                "timeout_seconds": { "type": "integer", "description": "Fenêtre de réévaluation en secondes. Si explicitement définie, une commande encore vivante/saine n'est pas tuée à l'échéance; Captain renouvelle et continue à surveiller. Défaut sans paramètre explicite : garde-fou court de 30s." }
            },
            "required": ["command"]
        }),
    )
}

fn docker_exec_tool_definition() -> ToolDefinition {
    tool_definition(
        "docker_exec",
        "Exécute une commande dans un conteneur Docker sandboxé avec isolation réseau, limites de ressources et suppression de capabilities. Utiliser pour exécuter du code non fiable ou des commandes potentiellement dangereuses en toute sécurité. Nécessite Docker installé et docker.enabled=true dans la config. Pour des commandes sûres, préférer shell_exec qui est plus rapide. Si timeout_secs est explicitement défini, c'est une fenêtre de réévaluation renouvelable: un processus encore vivant n'est pas tué à l'échéance. Retourne stdout/stderr de la commande.",
        serde_json::json!({
            "type": "object",
            "properties": {
                "command": { "type": "string", "description": "The command to execute inside the container" },
                "timeout_secs": { "type": "integer", "description": "Fenêtre de réévaluation en secondes (max 7200) quand elle est explicite; défaut sans paramètre explicite: garde-fou dur configuré par docker.timeout_secs." }
            },
            "required": ["command"]
        }),
    )
}

fn process_start_tool_definition() -> ToolDefinition {
    tool_definition(
        "process_start",
        "Démarre un processus persistant (REPL, serveur, watcher) qui continue de tourner en arrière-plan sans bloquer le tour agent. Utiliser obligatoirement à la place de shell_exec pour des serveurs de développement, apps locales, REPLs interactifs, watchers, nohup ou commandes avec `&`. Fournir `cwd` quand le serveur doit tourner depuis un dossier projet. Interagir ensuite via process_poll (lire la sortie), process_write (envoyer des commandes), process_list (surveiller), et process_kill (arrêter). Limité à 5 processus simultanés par agent. Retourne un process_id pour les opérations suivantes.",
        serde_json::json!({
            "type": "object",
            "properties": {
                "command": { "type": "string", "description": "The executable to run (e.g. 'python', 'node', 'npm')" },
                "args": {
                    "type": "array",
                    "items": { "type": "string" },
                    "description": "Command-line arguments (e.g. ['-i'] for interactive Python)"
                },
                "cwd": { "type": "string", "description": "Working directory for the process, e.g. a project folder containing app.py or package.json." }
            },
            "required": ["command"]
        }),
    )
}

fn process_poll_tool_definition() -> ToolDefinition {
    tool_definition(
        "process_poll",
        "Lit la sortie stdout/stderr accumulée depuis le dernier poll d'un processus persistant. Non-bloquant : retourne ce qui est disponible dans le buffer sans attendre. Utiliser en boucle pour surveiller l'avancement d'un processus démarré via process_start. Retourne une chaîne vide si aucune nouvelle sortie.",
        serde_json::json!({
            "type": "object",
            "properties": {
                "process_id": { "type": "string", "description": "The process ID returned by process_start" }
            },
            "required": ["process_id"]
        }),
    )
}

fn process_write_tool_definition() -> ToolDefinition {
    tool_definition(
        "process_write",
        "Envoie des données sur stdin d'un processus persistant en cours d'exécution. Un retour à la ligne est ajouté automatiquement s'il n'est pas présent. Utiliser pour envoyer des commandes à un REPL interactif ou des entrées à un programme. Combiner avec process_poll pour lire la réponse.",
        serde_json::json!({
            "type": "object",
            "properties": {
                "process_id": { "type": "string", "description": "The process ID returned by process_start" },
                "data": { "type": "string", "description": "The data to write to stdin" }
            },
            "required": ["process_id", "data"]
        }),
    )
}

fn process_kill_tool_definition() -> ToolDefinition {
    tool_definition(
        "process_kill",
        "Termine un processus persistant et libère ses ressources (mémoire, ports, descripteurs de fichiers). Utiliser pour arrêter proprement un serveur, REPL ou watcher démarré via process_start. L'opération est irréversible — le processus doit être redémarré si nécessaire. Retourne une confirmation de terminaison.",
        serde_json::json!({
            "type": "object",
            "properties": {
                "process_id": { "type": "string", "description": "The process ID returned by process_start" }
            },
            "required": ["process_id"]
        }),
    )
}

fn process_list_tool_definition() -> ToolDefinition {
    tool_definition(
        "process_list",
        "Liste tous les processus persistants en cours pour l'agent courant avec leurs IDs, commandes, durée d'exécution, idle_secs depuis la dernière activité observée et statut (alive/dead). Utiliser pour vérifier quels processus tournent avant d'en démarrer un nouveau ou pour obtenir un process_id. Retourne un tableau JSON.",
        serde_json::json!({
            "type": "object",
            "properties": {}
        }),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shell_tool_definitions_keep_public_order() {
        let tools = shell_tool_definitions();
        let names: Vec<_> = tools.iter().map(|tool| tool.name.as_str()).collect();

        assert_eq!(
            names,
            vec![
                "execute_code",
                "shell_exec",
                "docker_exec",
                "process_start",
                "process_poll",
                "process_write",
                "process_kill",
                "process_list",
            ]
        );
    }

    #[test]
    fn shell_tool_definitions_keep_execution_contracts() {
        let tools = shell_tool_definitions();
        let execute_code = tool(&tools, "execute_code");
        let shell_exec = tool(&tools, "shell_exec");
        let docker_exec = tool(&tools, "docker_exec");

        assert_eq!(required_fields(execute_code), vec!["code"]);
        assert_eq!(
            enum_values(property(execute_code, "language")),
            vec!["python", "node", "bash"]
        );
        assert_contains(&execute_code.description, "fenetre de reevaluation");
        assert_contains(
            property(execute_code, "pip_install")
                .get("description")
                .and_then(Value::as_str)
                .unwrap_or_default(),
            "allowlist",
        );

        assert_eq!(required_fields(shell_exec), vec!["command"]);
        assert_contains(&shell_exec.description, "capability_search");
        assert_contains(&shell_exec.description, "version binaire/runtime");
        assert_contains(&shell_exec.description, "/api/status");
        assert_contains(&shell_exec.description, "fenêtre de réévaluation bornée");
        assert_contains(&shell_exec.description, "pmset -g thermlog");
        assert_contains(&shell_exec.description, "process_start");
        assert_contains(&shell_exec.description, "secrets.env");
        assert_contains(
            property(shell_exec, "timeout_seconds")
                .get("description")
                .and_then(Value::as_str)
                .unwrap_or_default(),
            "Fenêtre de réévaluation",
        );

        assert_eq!(required_fields(docker_exec), vec!["command"]);
        assert_contains(&docker_exec.description, "Docker");
        assert_contains(&docker_exec.description, "fenêtre de réévaluation");
        assert_contains(
            property(docker_exec, "timeout_secs")
                .get("description")
                .and_then(Value::as_str)
                .unwrap_or_default(),
            "max 7200",
        );
    }

    #[test]
    fn shell_tool_definitions_keep_process_contracts() {
        let tools = shell_tool_definitions();

        assert_eq!(
            required_fields(tool(&tools, "process_start")),
            vec!["command"]
        );
        assert_eq!(
            items_type(property(tool(&tools, "process_start"), "args")),
            "string"
        );
        assert_contains(
            tool(&tools, "process_start").description.as_str(),
            "sans bloquer le tour agent",
        );
        assert_contains(tool(&tools, "process_start").description.as_str(), "cwd");
        assert_eq!(
            property(tool(&tools, "process_start"), "cwd")["type"],
            serde_json::json!("string")
        );

        assert_eq!(
            required_fields(tool(&tools, "process_poll")),
            vec!["process_id"]
        );
        assert_eq!(
            required_fields(tool(&tools, "process_write")),
            vec!["process_id", "data"]
        );
        assert_eq!(
            required_fields(tool(&tools, "process_kill")),
            vec!["process_id"]
        );
        assert!(required_fields(tool(&tools, "process_list")).is_empty());
        assert_contains(
            &tool(&tools, "process_list").description,
            "idle_secs depuis la dernière activité observée",
        );
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

    fn assert_contains(haystack: &str, needle: &str) {
        assert!(
            haystack.contains(needle),
            "expected `{haystack}` to contain `{needle}`"
        );
    }
}
