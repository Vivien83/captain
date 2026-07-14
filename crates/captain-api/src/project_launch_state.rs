use crate::project_lifecycle::lifecycle_json;
use captain_memory::project;
use captain_runtime::project_rules::ProjectRulesFileStatus;
use serde_json::Value;
use std::path::Path;

pub(crate) struct ProjectLaunchStateInput<'a> {
    pub(crate) repo_path: Option<&'a str>,
    pub(crate) branch: Option<&'a str>,
    pub(crate) source: &'a Value,
    pub(crate) workspace_path: Option<&'a Path>,
    pub(crate) default_root: &'a Path,
    pub(crate) authorized: bool,
    pub(crate) authorization_error: Option<&'a str>,
    pub(crate) rules_file: Option<&'a ProjectRulesFileStatus>,
    pub(crate) create_worktree: bool,
    pub(crate) autonomy_level: &'a str,
    pub(crate) acceptance_criteria: &'a [String],
}

pub(crate) fn project_launch_state(input: ProjectLaunchStateInput<'_>) -> Value {
    serde_json::json!({
        "protocol": "captain.project_launch.v1",
        "repo_path": input.repo_path,
        "branch": input.branch,
        "source": input.source,
        "workspace": {
            "path": input.workspace_path.map(|path| path.display().to_string()),
            "default_root": input.default_root.display().to_string(),
            "authorized": input.authorized,
            "authorization_error": input.authorization_error,
            "platform": std::env::consts::OS,
            "rules_file": input.rules_file,
        },
        "create_worktree": input.create_worktree,
        "autonomy_level": input.autonomy_level,
        "acceptance_criteria": input.acceptance_criteria,
        "board_columns": [
            "triage", "planned", "ready", "running", "blocked", "review", "done"
        ],
        "lifecycle": lifecycle_json("observe"),
        "next_gate": "planning_review",
    })
}

pub(crate) fn project_created_event_payload(
    project: &project::Project,
    launch_state: &Value,
    task_count: usize,
    goal_count: usize,
    milestone_id: &str,
    checkpoint_id: &str,
    rules_file_created: bool,
) -> Value {
    serde_json::json!({
        "event": "project.created",
        "project_id": project.id,
        "slug": project.slug,
        "name": project.name,
        "source_type": launch_state
            .pointer("/source/type")
            .cloned()
            .unwrap_or_else(|| serde_json::json!("unknown")),
        "workspace_authorized": launch_state
            .pointer("/workspace/authorized")
            .cloned()
            .unwrap_or(Value::Null),
        "task_count": task_count,
        "goal_count": goal_count,
        "milestone_id": milestone_id,
        "checkpoint_id": checkpoint_id,
        "rules_file_created": rules_file_created,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use captain_memory::project::ProjectStatus;
    use std::path::PathBuf;

    fn project() -> project::Project {
        project::Project {
            id: "project-1".to_string(),
            name: "Demo Project".to_string(),
            slug: "demo-project".to_string(),
            goal: "Ship safely".to_string(),
            status: ProjectStatus::Active,
            deadline: None,
            created_at: 0,
            updated_at: 0,
            metadata: serde_json::json!({}),
        }
    }

    fn rules_file() -> ProjectRulesFileStatus {
        ProjectRulesFileStatus {
            path: "/private/tmp/demo/CAPTAIN.md".to_string(),
            existed: false,
            created: true,
            error: None,
        }
    }

    #[test]
    fn launch_state_includes_runtime_launch_contract() {
        let workspace_path = PathBuf::from("/private/tmp/demo");
        let default_root = PathBuf::from("/private/tmp/workspaces/projects");
        let criteria = vec!["Smoke passes".to_string(), "Docs updated".to_string()];
        let rules = rules_file();

        let state = project_launch_state(ProjectLaunchStateInput {
            repo_path: Some("/private/tmp/demo"),
            branch: Some("main"),
            source: &serde_json::json!({"type": "local"}),
            workspace_path: Some(&workspace_path),
            default_root: &default_root,
            authorized: true,
            authorization_error: None,
            rules_file: Some(&rules),
            create_worktree: true,
            autonomy_level: "supervised",
            acceptance_criteria: &criteria,
        });

        assert_eq!(state["protocol"], "captain.project_launch.v1");
        assert_eq!(state["repo_path"], "/private/tmp/demo");
        assert_eq!(state["branch"], "main");
        assert_eq!(state["source"]["type"], "local");
        assert_eq!(state["workspace"]["path"], "/private/tmp/demo");
        assert_eq!(
            state["workspace"]["default_root"],
            "/private/tmp/workspaces/projects"
        );
        assert_eq!(state["workspace"]["authorized"], true);
        assert_eq!(state["workspace"]["rules_file"]["created"], true);
        assert_eq!(state["create_worktree"], true);
        assert_eq!(state["autonomy_level"], "supervised");
        assert_eq!(state["acceptance_criteria"][0], "Smoke passes");
        assert_eq!(state["board_columns"].as_array().unwrap().len(), 7);
        assert_eq!(state["lifecycle"]["current_phase"], "observe");
        assert_eq!(state["next_gate"], "planning_review");
    }

    #[test]
    fn project_created_event_payload_is_operator_safe() {
        let launch_state = serde_json::json!({
            "source": {
                "type": "local",
                "path": "/private/tmp/demo"
            },
            "workspace": {
                "path": "/private/tmp/demo",
                "authorized": true
            }
        });

        let payload = project_created_event_payload(
            &project(),
            &launch_state,
            7,
            1,
            "milestone-1",
            "checkpoint-1",
            true,
        );

        assert_eq!(payload["event"], "project.created");
        assert_eq!(payload["project_id"], "project-1");
        assert_eq!(payload["slug"], "demo-project");
        assert_eq!(payload["source_type"], "local");
        assert_eq!(payload["workspace_authorized"], true);
        assert_eq!(payload["task_count"], 7);
        assert_eq!(payload["goal_count"], 1);
        assert_eq!(payload["milestone_id"], "milestone-1");
        assert_eq!(payload["checkpoint_id"], "checkpoint-1");
        assert_eq!(payload["rules_file_created"], true);
        let serialized = payload.to_string();
        assert!(!serialized.contains("/private/tmp/demo"));
        assert!(payload.get("launch").is_none());
        assert!(payload.get("workspace").is_none());
    }
}
