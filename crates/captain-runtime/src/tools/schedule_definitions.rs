//! Static raw scheduler tool definitions.

use captain_types::tool::ToolDefinition;
use serde_json::Value;

pub fn schedule_tool_definitions() -> Vec<ToolDefinition> {
    let mut definitions = cron_tool_definitions();
    definitions.extend(schedule_compat_tool_definitions());
    definitions.extend(file_trigger_tool_definitions());
    definitions.extend(todo_tool_definitions());
    definitions
}

fn cron_tool_definitions() -> Vec<ToolDefinition> {
    vec![
        cron_create_tool_definition(),
        cron_list_tool_definition(),
        cron_update_tool_definition(),
        cron_cancel_tool_definition(),
        reminder_set_tool_definition(),
    ]
}

fn schedule_compat_tool_definitions() -> Vec<ToolDefinition> {
    vec![
        schedule_create_tool_definition(),
        schedule_list_tool_definition(),
        schedule_delete_tool_definition(),
    ]
}

fn file_trigger_tool_definitions() -> Vec<ToolDefinition> {
    vec![
        file_trigger_register_tool_definition(),
        file_trigger_list_tool_definition(),
        file_trigger_set_enabled_tool_definition(),
        file_trigger_remove_tool_definition(),
    ]
}

fn todo_tool_definitions() -> Vec<ToolDefinition> {
    vec![
        todo_create_tool_definition(),
        todo_list_tool_definition(),
        todo_complete_tool_definition(),
        todo_reopen_tool_definition(),
        todo_delete_tool_definition(),
    ]
}

fn tool_definition(name: &str, description: &str, input_schema: Value) -> ToolDefinition {
    ToolDefinition {
        name: name.to_string(),
        description: description.to_string(),
        input_schema,
    }
}

fn cron_create_tool_definition() -> ToolDefinition {
    tool_definition(
        "cron_create",
        "Crée une tâche planifiée pour cet agent. Supporte trois modes : one-shot à une date précise (`at`), récurrent toutes les N secondes (`every`), ou expression cron standard (`cron`). Utiliser pour automatiser des rappels, des rapports périodiques ou des actions différées. Ne pas utiliser pour des délais courts — préférer `reminder_set`. Limité à 50 jobs par agent. Retourne l'ID UUID du job créé.",
        serde_json::json!({
            "type": "object",
            "properties": {
                "name": { "type": "string", "description": "Nom descriptif du job (max 128 caractères, alphanumérique + espaces, tirets, underscores)" },
                "schedule": {
                    "type": "object",
                    "description": "Planification. Trois formes : {\"kind\":\"at\",\"at\":\"2025-01-01T00:00:00Z\"} (date unique ISO 8601), {\"kind\":\"every\",\"every_secs\":300} (intervalle fixe en secondes), ou {\"kind\":\"cron\",\"expr\":\"0 */6 * * *\",\"tz\":\"Europe/Paris\"} (expression cron). IMPORTANT : toujours spécifier \"tz\":\"Europe/Paris\" pour les expressions cron."
                },
                "action": {
                    "type": "object",
                    "description": "Action à exécuter au déclenchement. Deux formes : {\"kind\":\"system_event\",\"text\":\"...\"} pour injecter un événement système, ou {\"kind\":\"agent_turn\",\"message\":\"...\",\"timeout_secs\":300} pour déclencher un tour LLM complet de l'agent. Pour agent_turn, timeout_secs est une fenêtre d'inactivité/réévaluation (défaut 600s, max 7200s), pas une limite de durée totale: une tâche active et saine ne doit pas être tuée."
                },
                "delivery": {
                    "type": "object",
                    "description": "Canal de livraison. Formes : {\"kind\":\"none\"}, {\"kind\":\"channel\",\"channel\":\"telegram\"}, {\"kind\":\"last_channel\"}, ou {\"kind\":\"webhook\",\"url\":\"https://...\"}. Les webhooks vers localhost, IP privées ou metadata cloud sont refusés."
                },
                "one_shot": { "type": "boolean", "description": "Si true, supprimé automatiquement après exécution. Pour les rappels uniques. Défaut : false." }
            },
            "required": ["name", "schedule", "action"]
        }),
    )
}

fn cron_list_tool_definition() -> ToolDefinition {
    tool_definition(
        "cron_list",
        "Liste toutes les tâches planifiées actives pour l'agent courant. Utiliser pour vérifier quels jobs existent avant d'en créer un doublon ou pour obtenir les IDs nécessaires à `cron_update` / `cron_cancel`. Ne retourne que les jobs de cet agent, pas ceux des autres agents. Retourne un tableau JSON avec id, name, schedule, next_run et statut.",
        serde_json::json!({
            "type": "object",
            "properties": {}
        }),
    )
}

fn cron_update_tool_definition() -> ToolDefinition {
    tool_definition(
        "cron_update",
        "Met à jour une tâche planifiée existante sans la supprimer/recréer. Préférer à cron_cancel+cron_create quand l'utilisateur veut modifier l'heure, le message, la livraison, l'activation ou le caractère one-shot d'un cron existant: l'ID, le propriétaire et l'historique d'exécution sont conservés. Appeler cron_list d'abord si l'ID n'est pas connu; job_id accepte un préfixe non ambigu.",
        serde_json::json!({
            "type": "object",
            "properties": {
                "job_id": { "type": "string", "description": "ID UUID complet ou préfixe non ambigu du job à modifier, tel que retourné par cron_list ou cron_create" },
                "name": { "type": "string", "description": "Nouveau nom descriptif du job" },
                "schedule": {
                    "type": "object",
                    "description": "Nouvelle planification: {\"kind\":\"at\",\"at\":\"2026-05-06T09:00:00+02:00\"}, {\"kind\":\"every\",\"every_secs\":300}, ou {\"kind\":\"cron\",\"expr\":\"0 8 * * 1\",\"tz\":\"Europe/Paris\"}. Appeler system_time avant de calculer une date relative."
                },
                "action": {
                    "type": "object",
                    "description": "Nouvelle action: {\"kind\":\"system_event\",\"text\":\"...\"} ou {\"kind\":\"agent_turn\",\"message\":\"...\",\"timeout_secs\":300}. Pour agent_turn, timeout_secs est une fenêtre d'inactivité/réévaluation (défaut 600s, max 7200s), pas une limite de durée totale. Une tâche active et saine ne doit pas être tuée. Ne jamais inclure de secret brut."
                },
                "delivery": {
                    "type": "object",
                    "description": "Nouvelle livraison: {\"kind\":\"none\"}, {\"kind\":\"channel\",\"channel\":\"telegram\",\"to\":\"<recipient>\"}, {\"kind\":\"last_channel\"}, ou {\"kind\":\"webhook\",\"url\":\"https://...\"}. Les destinations localhost, IP privées et metadata cloud sont refusées."
                },
                "enabled": { "type": "boolean", "description": "true pour activer/replanifier, false pour désactiver sans supprimer" },
                "one_shot": { "type": "boolean", "description": "true pour supprimer après le prochain succès, false pour garder récurrent" }
            },
            "required": ["job_id"]
        }),
    )
}

fn cron_cancel_tool_definition() -> ToolDefinition {
    tool_definition(
        "cron_cancel",
        "Annule et supprime définitivement une tâche planifiée par son ID UUID. Utiliser quand un job n'est plus nécessaire ou doit être remplacé. Appeler `cron_list` d'abord pour confirmer l'ID. L'opération est irréversible. Retourne un message de confirmation.",
        serde_json::json!({
            "type": "object",
            "properties": {
                "job_id": { "type": "string", "description": "ID UUID du job à annuler, tel que retourné par `cron_list` ou `cron_create`" }
            },
            "required": ["job_id"]
        }),
    )
}

fn reminder_set_tool_definition() -> ToolDefinition {
    tool_definition(
        "reminder_set",
        "Programme un rappel simple avec délai en minutes et message. Version simplifiée de cron_create — pas besoin de syntaxe cron. Livraison automatique via Telegram par défaut. Utiliser pour des rappels ponctuels rapides. Pour des tâches récurrentes ou complexes, préférer cron_create. Retourne l'ID du rappel créé.",
        serde_json::json!({
            "type": "object",
            "properties": {
                "delay_minutes": { "type": "number", "description": "Delay in minutes from now (e.g., 30 for half an hour, 1440 for 24 hours)" },
                "message": { "type": "string", "description": "The reminder message to send" },
                "channel": { "type": "string", "description": "Delivery channel. Default: telegram", "default": "telegram" }
            },
            "required": ["delay_minutes", "message"]
        }),
    )
}

fn schedule_create_tool_definition() -> ToolDefinition {
    tool_definition(
        "schedule_create",
        "Planifie une tâche récurrente en langage naturel ou en syntaxe cron. Utiliser pour des tâches régulières simples. Pour un contrôle fin (one-shot, delivery canal, action types), préférer cron_create. Exemples : 'every 5 minutes', 'daily at 9am', '0 */5 * * *'. Retourne l'ID du schedule créé.",
        serde_json::json!({
            "type": "object",
            "properties": {
                "description": { "type": "string", "description": "What this schedule does (e.g., 'Check for new emails')" },
                "schedule": { "type": "string", "description": "Natural language or cron expression (e.g., 'every 5 minutes', 'daily at 9am', '0 */5 * * *')" },
                "agent": { "type": "string", "description": "Agent name or ID to run this task (optional, defaults to self)" }
            },
            "required": ["description", "schedule"]
        }),
    )
}

fn schedule_list_tool_definition() -> ToolDefinition {
    tool_definition(
        "schedule_list",
        "Liste toutes les tâches planifiées avec leurs IDs, descriptions, expressions de schedule et prochaines exécutions. Utiliser pour vérifier les schedules existants avant d'en créer un nouveau. Retourne un tableau JSON.",
        serde_json::json!({
            "type": "object",
            "properties": {}
        }),
    )
}

fn schedule_delete_tool_definition() -> ToolDefinition {
    tool_definition(
        "schedule_delete",
        "Supprime définitivement une tâche planifiée par son ID. Utiliser quand un schedule n'est plus nécessaire. Appeler schedule_list d'abord pour confirmer l'ID. L'opération est irréversible. Retourne une confirmation de suppression.",
        serde_json::json!({
            "type": "object",
            "properties": {
                "id": { "type": "string", "description": "The schedule ID to remove" }
            },
            "required": ["id"]
        }),
    )
}

fn file_trigger_register_tool_definition() -> ToolDefinition {
    tool_definition(
        "file_trigger_register",
        "Pose un watcher système sur un ou plusieurs chemins. Quand un fichier surveillé change, l'agent reçoit un prompt rendu depuis prompt_template ({path}, {kind}, {previous_path}). Idéal pour réagir à des dépôts d'inbox, des modifications de configuration, ou un signal externe. Les chemins sensibles (~/.ssh, secrets.env, vault.enc, etc.) sont refusés. Le watcher reste armé entre redémarrages tant que ses chemins existent. Rate-limit: max 10 fires / 60s avant auto-pause. Retourne le trigger_id.",
        serde_json::json!({
            "type": "object",
            "properties": {
                "paths": {
                    "type": "array",
                    "items": { "type": "string" },
                    "description": "Liste de chemins à surveiller. Fichier individuel, dossier (recursive selon le flag), ou chemin vers un fichier qui n'existe pas encore (le watcher s'arme sur le parent existant)."
                },
                "events": {
                    "type": "array",
                    "items": { "type": "string", "enum": ["create", "modify", "remove", "rename", "any"] },
                    "description": "Types d'événements à capturer. Défaut: ['any']. 'modify' couvre l'écriture/append. 'rename' inclut les move-to et move-from."
                },
                "recursive": { "type": "boolean", "description": "Si paths contient un dossier, surveille aussi ses descendants. Défaut: true." },
                "prompt_template": { "type": "string", "description": "Template du prompt envoyé à l'agent au fire. Variables: {path}, {kind}, {previous_path}. Défaut: 'File {kind}: {path}'." },
                "debounce_ms": { "type": "integer", "description": "Fenêtre de debounce en ms. Bornée à [200, 60000]. Défaut: 800." },
                "enabled": { "type": "boolean", "description": "Si false, persiste le trigger sans armer le watcher. Défaut: true." }
            },
            "required": ["paths"]
        }),
    )
}

fn file_trigger_list_tool_definition() -> ToolDefinition {
    tool_definition(
        "file_trigger_list",
        "Liste les file-change triggers. Par défaut, ne retourne que ceux de l'agent appelant. Passer scope='all' pour voir tous les triggers du système (debug/admin). Retourne id, paths, events, recursive, prompt_template, debounce_ms, enabled pour chaque entrée.",
        serde_json::json!({
            "type": "object",
            "properties": {
                "scope": { "type": "string", "enum": ["self", "all"], "description": "'self' (défaut) liste les triggers de l'agent courant. 'all' liste tous les triggers (utile pour audit/debug)." }
            }
        }),
    )
}

fn file_trigger_set_enabled_tool_definition() -> ToolDefinition {
    tool_definition(
        "file_trigger_set_enabled",
        "Active ou désactive un file-change trigger sans le supprimer. Disable arrête le watcher mais conserve la définition (utile pour pauser temporairement). Enable réarme le watcher. Retourne {trigger_id, enabled}.",
        serde_json::json!({
            "type": "object",
            "properties": {
                "trigger_id": { "type": "string", "description": "ID UUID du trigger, tel que retourné par file_trigger_register ou file_trigger_list" },
                "enabled": { "type": "boolean", "description": "true pour réarmer, false pour pauser" }
            },
            "required": ["trigger_id", "enabled"]
        }),
    )
}

fn file_trigger_remove_tool_definition() -> ToolDefinition {
    tool_definition(
        "file_trigger_remove",
        "Supprime définitivement un file-change trigger et arrête son watcher. Pour pauser temporairement sans perdre la config, utiliser file_trigger_set_enabled(enabled=false). Opération irréversible.",
        serde_json::json!({
            "type": "object",
            "properties": {
                "trigger_id": { "type": "string", "description": "ID UUID du trigger à supprimer" }
            },
            "required": ["trigger_id"]
        }),
    )
}

fn todo_create_tool_definition() -> ToolDefinition {
    tool_definition(
        "todo_create",
        "Capture un item à faire qui survit aux redémarrages et aux compactions. Utiliser pour des choses simples sans projet ni boucle d'autopilot : « ne pas oublier de répondre à X », « lire ce papier ». Pour un travail récurrent ou un suivi par projet, préférer cron_create / project_task_create. Pour une boucle continue à maintenir, préférer goal_create. Retourne l'objet todo créé avec son id UUID.",
        serde_json::json!({
            "type": "object",
            "properties": {
                "title": { "type": "string", "description": "Texte court (≤ 1 ligne). Obligatoire, non vide." },
                "body": { "type": "string", "description": "Détail libre (optionnel). Markdown autorisé." }
            },
            "required": ["title"]
        }),
    )
}

fn todo_list_tool_definition() -> ToolDefinition {
    tool_definition(
        "todo_list",
        "Liste les todos cross-session. Par défaut retourne uniquement les ouverts (status='open'). Passer status='done' pour la done list ou status='all' pour tout. Le tri privilégie les ouverts récents en haut, puis les plus récemment complétés. Pagination via limit (défaut 200, max 1000).",
        serde_json::json!({
            "type": "object",
            "properties": {
                "status": {
                    "type": "string",
                    "enum": ["open", "done", "all"],
                    "description": "Filtre. Défaut: 'open'."
                },
                "limit": {
                    "type": "integer",
                    "minimum": 1,
                    "maximum": 1000,
                    "description": "Nombre maximum d'entrées. Défaut: 200. Borné en dur à 1000."
                }
            }
        }),
    )
}

fn todo_complete_tool_definition() -> ToolDefinition {
    tool_definition(
        "todo_complete",
        "Marque un todo comme fait. Le todo est conservé dans la done list (consultable via todo_list status='done') jusqu'à un éventuel todo_delete. Retourne l'objet mis à jour. Erreur si l'id n'existe pas.",
        serde_json::json!({
            "type": "object",
            "properties": {
                "id": { "type": "string", "description": "ID UUID du todo, tel que retourné par todo_create / todo_list." }
            },
            "required": ["id"]
        }),
    )
}

fn todo_reopen_tool_definition() -> ToolDefinition {
    tool_definition(
        "todo_reopen",
        "Remet un todo complété en ouvert (efface completed_at). Utile quand un todo a été marqué fait par erreur. Idempotent sur un todo déjà ouvert.",
        serde_json::json!({
            "type": "object",
            "properties": {
                "id": { "type": "string", "description": "ID UUID du todo." }
            },
            "required": ["id"]
        }),
    )
}

fn todo_delete_tool_definition() -> ToolDefinition {
    tool_definition(
        "todo_delete",
        "Supprime définitivement un todo (ouvert ou complété). Opération irréversible. Pour vider la done list garder cette commande pour le nettoyage manuel ; la done list n'est pas un journal d'audit.",
        serde_json::json!({
            "type": "object",
            "properties": {
                "id": { "type": "string", "description": "ID UUID du todo à supprimer." }
            },
            "required": ["id"]
        }),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn schedule_tool_definitions_keep_public_order() {
        let definitions = schedule_tool_definitions();
        let names: Vec<_> = definitions
            .iter()
            .map(|definition| definition.name.as_str())
            .collect();

        assert_eq!(
            names,
            vec![
                "cron_create",
                "cron_list",
                "cron_update",
                "cron_cancel",
                "reminder_set",
                "schedule_create",
                "schedule_list",
                "schedule_delete",
                "file_trigger_register",
                "file_trigger_list",
                "file_trigger_set_enabled",
                "file_trigger_remove",
                "todo_create",
                "todo_list",
                "todo_complete",
                "todo_reopen",
                "todo_delete",
            ]
        );
    }

    #[test]
    fn schedule_tool_definitions_keep_cron_contracts() {
        let definitions = schedule_tool_definitions();
        let cron_create = definition(&definitions, "cron_create");
        let cron_update = definition(&definitions, "cron_update");

        assert_eq!(
            required_fields(cron_create),
            vec!["name", "schedule", "action"]
        );
        assert_contains(property(cron_create, "schedule"), "\"tz\":\"Europe/Paris\"");
        assert_contains(
            property(cron_create, "action"),
            "timeout_secs est une fenêtre",
        );
        assert_eq!(required_fields(cron_update), vec!["job_id"]);
        assert_contains(
            property(cron_update, "action"),
            "Ne jamais inclure de secret brut",
        );
        assert_contains(property(cron_update, "delivery"), "metadata cloud");
    }

    #[test]
    fn schedule_tool_definitions_keep_file_trigger_and_todo_contracts() {
        let definitions = schedule_tool_definitions();
        let file_trigger = definition(&definitions, "file_trigger_register");
        let todo_list = definition(&definitions, "todo_list");

        assert_eq!(required_fields(file_trigger), vec!["paths"]);
        assert_eq!(
            enum_values(&property(file_trigger, "events")["items"]["enum"]),
            vec!["create", "modify", "remove", "rename", "any"]
        );
        assert_eq!(
            enum_values(&property(todo_list, "status")["enum"]),
            vec!["open", "done", "all"]
        );
        assert_eq!(property(todo_list, "limit")["maximum"].as_i64(), Some(1000));
    }

    fn definition<'a>(definitions: &'a [ToolDefinition], name: &str) -> &'a ToolDefinition {
        definitions
            .iter()
            .find(|definition| definition.name == name)
            .unwrap_or_else(|| panic!("missing tool definition {name}"))
    }

    fn required_fields(definition: &ToolDefinition) -> Vec<&str> {
        definition
            .input_schema
            .get("required")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
            .collect()
    }

    fn property<'a>(definition: &'a ToolDefinition, name: &str) -> &'a Value {
        definition
            .input_schema
            .get("properties")
            .and_then(Value::as_object)
            .and_then(|properties| properties.get(name))
            .unwrap_or_else(|| panic!("missing property {name} on {}", definition.name))
    }

    fn enum_values(value: &Value) -> Vec<&str> {
        value
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
            .collect()
    }

    fn assert_contains(property: &Value, expected: &str) {
        let description = property["description"]
            .as_str()
            .unwrap_or_else(|| panic!("missing description on property {property:?}"));
        assert!(
            description.contains(expected),
            "description did not contain {expected:?}: {description}"
        );
    }
}
