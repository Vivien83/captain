//! Static project tool definitions.

use captain_types::tool::ToolDefinition;
use serde_json::Value;

pub fn project_tool_definitions() -> Vec<ToolDefinition> {
    let mut definitions = project_lifecycle_tool_definitions();
    definitions.extend(project_task_tool_definitions());
    definitions.extend(milestone_tool_definitions());
    definitions.extend(checkpoint_tool_definitions());
    definitions
}

fn project_lifecycle_tool_definitions() -> Vec<ToolDefinition> {
    vec![
        project_create_tool_definition(),
        project_list_tool_definition(),
        project_get_tool_definition(),
        project_archive_tool_definition(),
        project_delete_tool_definition(),
        project_resume_tool_definition(),
    ]
}

fn project_task_tool_definitions() -> Vec<ToolDefinition> {
    vec![
        project_task_create_tool_definition(),
        project_task_list_tool_definition(),
        project_task_update_tool_definition(),
    ]
}

fn milestone_tool_definitions() -> Vec<ToolDefinition> {
    vec![
        milestone_create_tool_definition(),
        milestone_list_tool_definition(),
        milestone_complete_tool_definition(),
        milestone_progress_tool_definition(),
    ]
}

fn checkpoint_tool_definitions() -> Vec<ToolDefinition> {
    vec![checkpoint_save_tool_definition()]
}

fn tool_definition(name: &str, description: &str, input_schema: Value) -> ToolDefinition {
    ToolDefinition {
        name: name.to_string(),
        description: description.to_string(),
        input_schema,
    }
}

fn project_create_tool_definition() -> ToolDefinition {
    tool_definition(
        "project_create",
        "Crée un projet. Le slug doit être unique et kebab-case (ex: 'mlx-finetune'). Retourne la ligne complète avec son id.",
        serde_json::json!({
            "type": "object",
            "properties": {
                "name": { "type": "string" },
                "slug": { "type": "string", "description": "Identifiant unique kebab-case" },
                "goal": { "type": "string", "description": "Objectif en une phrase (optionnel)" },
                "deadline": { "type": "number", "description": "Deadline en unix ms (optionnel)" }
            },
            "required": ["name", "slug"]
        }),
    )
}

fn project_list_tool_definition() -> ToolDefinition {
    tool_definition(
        "project_list",
        "Liste les projets sous forme compacte agent-safe (archivés exclus par défaut), triés par updated_at desc. Utiliser `query` dès que l'utilisateur donne un nom partiel, un slug ou une référence comme `projet1`; toujours comparer aux slugs/noms avant d'interpréter un chiffre comme un choix de menu.",
        serde_json::json!({
            "type": "object",
            "properties": {
                "include_archived": { "type": "boolean", "description": "Inclure les projets archivés (default false)" },
                "query": { "type": "string", "description": "Filtre texte optionnel appliqué au slug, nom et objectif (ex: 'projet1', 'documents couple')." }
            }
        }),
    )
}

fn project_get_tool_definition() -> ToolDefinition {
    tool_definition(
        "project_get",
        "Récupère un projet par son slug. Par défaut retourne un RÉSUMÉ compact (identité, objectif, statut runtime/phase/progression, prochaines actions, dernier checkpoint) avec seulement les compteurs des sections lourdes — pas besoin de tout recharger. Utilise include_events/include_worker_results/include_tasks pour réintégrer une section complète si nécessaire.",
        serde_json::json!({
            "type": "object",
            "properties": {
                "slug": { "type": "string" },
                "include_events": { "type": "boolean", "description": "Réintègre la timeline d'événements runtime (plafonnée aux 50 derniers, total toujours indiqué). Défaut: false." },
                "include_worker_results": { "type": "boolean", "description": "Réintègre les résultats détaillés par phase (worker_results). Défaut: false." },
                "include_tasks": { "type": "boolean", "description": "Réintègre la liste complète des tâches. Défaut: false — seuls les compteurs et les tâches ouvertes (next_actions) sont retournés." }
            },
            "required": ["slug"]
        }),
    )
}

fn project_archive_tool_definition() -> ToolDefinition {
    tool_definition(
        "project_archive",
        "Archive un projet. Réversible via project_update status=active. Identifié par id UUID.",
        serde_json::json!({
            "type": "object",
            "properties": { "id": { "type": "string" } },
            "required": ["id"]
        }),
    )
}

fn project_delete_tool_definition() -> ToolDefinition {
    tool_definition(
        "project_delete",
        "Supprime DÉFINITIVEMENT un projet et ses goals associés — IRRÉVERSIBLE, contrairement à project_archive. Confirme explicitement avec l'utilisateur avant d'appeler cet outil. Identifié par id UUID.",
        serde_json::json!({
            "type": "object",
            "properties": { "id": { "type": "string" } },
            "required": ["id"]
        }),
    )
}

fn project_resume_tool_definition() -> ToolDefinition {
    tool_definition(
        "project_resume",
        "Recharge le contexte d'un projet : dernier checkpoint, tasks en cours, progression milestones. Utilise quand l'utilisateur dit 'reprends <projet>'.",
        serde_json::json!({
            "type": "object",
            "properties": { "slug": { "type": "string" } },
            "required": ["slug"]
        }),
    )
}

fn project_task_create_tool_definition() -> ToolDefinition {
    tool_definition(
        "project_task_create",
        "Crée une tâche pour un projet. parent_id optionnel pour les sous-tâches. Status initial 'todo'.",
        serde_json::json!({
            "type": "object",
            "properties": {
                "project_id": { "type": "string" },
                "title": { "type": "string" },
                "description": { "type": "string" },
                "parent_id": { "type": "string", "description": "Optional parent task id for sub-tasks" }
            },
            "required": ["project_id", "title"]
        }),
    )
}

fn project_task_list_tool_definition() -> ToolDefinition {
    tool_definition(
        "project_task_list",
        "Liste les tâches d'un projet, triées par priorité desc puis created_at asc.",
        serde_json::json!({
            "type": "object",
            "properties": { "project_id": { "type": "string" } },
            "required": ["project_id"]
        }),
    )
}

fn project_task_update_tool_definition() -> ToolDefinition {
    tool_definition(
        "project_task_update",
        "Met à jour le status d'une tâche. Values: todo, doing, blocked, review, done, cancelled. 'done' stamp automatiquement completed_at.",
        serde_json::json!({
            "type": "object",
            "properties": {
                "id": { "type": "string" },
                "status": { "type": "string", "enum": ["todo","doing","blocked","review","done","cancelled"] }
            },
            "required": ["id", "status"]
        }),
    )
}

fn milestone_create_tool_definition() -> ToolDefinition {
    tool_definition(
        "milestone_create",
        "Crée un milestone pour un projet avec deadline optionnelle en unix ms.",
        serde_json::json!({
            "type": "object",
            "properties": {
                "project_id": { "type": "string" },
                "name": { "type": "string" },
                "due_date": { "type": "number", "description": "Unix milliseconds (optional)" }
            },
            "required": ["project_id", "name"]
        }),
    )
}

fn milestone_list_tool_definition() -> ToolDefinition {
    tool_definition(
        "milestone_list",
        "Liste les milestones d'un projet, triés par due_date asc puis created_at asc.",
        serde_json::json!({
            "type": "object",
            "properties": { "project_id": { "type": "string" } },
            "required": ["project_id"]
        }),
    )
}

fn milestone_complete_tool_definition() -> ToolDefinition {
    tool_definition(
        "milestone_complete",
        "Marque un milestone comme completed. Stamp completed_at automatiquement.",
        serde_json::json!({
            "type": "object",
            "properties": { "id": { "type": "string" } },
            "required": ["id"]
        }),
    )
}

fn milestone_progress_tool_definition() -> ToolDefinition {
    tool_definition(
        "milestone_progress",
        "Retourne {total, completed, missed, pct} pour un projet. 'missed' = deadline dépassée et pas completed.",
        serde_json::json!({
            "type": "object",
            "properties": { "project_id": { "type": "string" } },
            "required": ["project_id"]
        }),
    )
}

fn checkpoint_save_tool_definition() -> ToolDefinition {
    tool_definition(
        "checkpoint_save",
        "Sauvegarde un checkpoint (summary + state) pour un projet — utilisé en fin de session pour préparer un handoff. project_resume(slug) le relira plus tard.",
        serde_json::json!({
            "type": "object",
            "properties": {
                "project_id": { "type": "string" },
                "summary": { "type": "string", "description": "Prose narration — where I left off" },
                "state": { "type": "object", "description": "Free-form structured payload" }
            },
            "required": ["project_id", "summary"]
        }),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn project_tool_definitions_keep_public_order() {
        let definitions = project_tool_definitions();
        let names: Vec<_> = definitions.iter().map(|tool| tool.name.as_str()).collect();

        assert_eq!(
            names,
            vec![
                "project_create",
                "project_list",
                "project_get",
                "project_archive",
                "project_delete",
                "project_resume",
                "project_task_create",
                "project_task_list",
                "project_task_update",
                "milestone_create",
                "milestone_list",
                "milestone_complete",
                "milestone_progress",
                "checkpoint_save",
            ]
        );
    }

    #[test]
    fn project_tool_definitions_keep_lifecycle_contracts() {
        let definitions = project_tool_definitions();
        let create = tool(&definitions, "project_create");
        let list = tool(&definitions, "project_list");
        let get = tool(&definitions, "project_get");
        let archive = tool(&definitions, "project_archive");
        let delete = tool(&definitions, "project_delete");
        let resume = tool(&definitions, "project_resume");

        assert_eq!(required_fields(create), vec!["name", "slug"]);
        assert_contains(
            property(create, "slug")
                .get("description")
                .and_then(Value::as_str)
                .unwrap_or_default(),
            "kebab-case",
        );
        assert!(property(list, "include_archived").is_object());
        assert!(property(list, "query").is_object());
        assert_contains(&list.description, "projet1");
        assert_contains(&list.description, "slugs/noms");
        assert_eq!(required_fields(get), vec!["slug"]);
        assert_eq!(required_fields(archive), vec!["id"]);
        assert_eq!(required_fields(delete), vec!["id"]);
        assert_contains(&delete.description, "IRRÉVERSIBLE");
        assert_eq!(required_fields(resume), vec!["slug"]);
        assert_contains(&resume.description, "dernier checkpoint");
    }

    #[test]
    fn project_tool_definitions_keep_task_milestone_checkpoint_contracts() {
        let definitions = project_tool_definitions();
        let task_create = tool(&definitions, "project_task_create");
        let task_update = tool(&definitions, "project_task_update");
        let milestone_create = tool(&definitions, "milestone_create");
        let milestone_progress = tool(&definitions, "milestone_progress");
        let checkpoint = tool(&definitions, "checkpoint_save");

        assert_eq!(required_fields(task_create), vec!["project_id", "title"]);
        assert_eq!(required_fields(task_update), vec!["id", "status"]);
        assert_eq!(
            enum_values(property(task_update, "status")),
            vec!["todo", "doing", "blocked", "review", "done", "cancelled"]
        );
        assert_eq!(
            required_fields(milestone_create),
            vec!["project_id", "name"]
        );
        assert_eq!(required_fields(milestone_progress), vec!["project_id"]);
        assert_eq!(required_fields(checkpoint), vec!["project_id", "summary"]);
        assert_eq!(
            property(checkpoint, "state")
                .get("type")
                .and_then(Value::as_str),
            Some("object")
        );
        assert_contains(&checkpoint.description, "project_resume(slug)");
    }

    fn tool<'a>(definitions: &'a [ToolDefinition], name: &str) -> &'a ToolDefinition {
        definitions
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
