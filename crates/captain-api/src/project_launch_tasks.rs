use captain_memory::project_task;

pub(crate) fn project_launch_tasks(
    project_id: &str,
    goal: &str,
    criteria: &[String],
) -> Vec<project_task::NewProjectTask> {
    let criteria_text = criteria
        .iter()
        .map(|item| format!("- {item}"))
        .collect::<Vec<_>>()
        .join("\n");
    let specs = [
        (
            "OBSERVE: capture current state",
            format!("Goal:\n{goal}\n\nAcceptance criteria:\n{criteria_text}\n\nInventory repo state, constraints, user context, and visible blockers before changing anything."),
            100,
        ),
        (
            "THINK: compare options and risks",
            "Reason about the smallest coherent path, likely regressions, required context, and whether an existing skill or learning already covers this work.".to_string(),
            90,
        ),
        (
            "PLAN: create executable slices",
            "Turn the chosen path into independently reviewable tasks, define verification commands, and keep statuses current.".to_string(),
            80,
        ),
        (
            "BUILD: implement project slices",
            "Apply the code, config, docs, or workflow changes in focused slices while preserving unrelated user work.".to_string(),
            70,
        ),
        (
            "EXECUTE: wire runtime behavior",
            "Run the changed workflow end to end when possible, connect web/TUI/API surfaces, and make live state observable.".to_string(),
            60,
        ),
        (
            "VERIFY: run the gate",
            "Run relevant build, test, lint, smoke, and review checks. Record failures as blockers instead of hiding them.".to_string(),
            50,
        ),
        (
            "LEARN: checkpoint and improve",
            "Create the handoff checkpoint, capture reusable learning, and identify skill or memory updates without duplicating existing knowledge.".to_string(),
            40,
        ),
    ];

    specs
        .into_iter()
        .map(
            |(title, description, priority)| project_task::NewProjectTask {
                project_id: project_id.to_string(),
                parent_id: None,
                title: title.to_string(),
                description,
                priority,
                deadline: None,
                assignee_agent_id: None,
            },
        )
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn project_launch_tasks_create_the_runtime_phase_backbone() {
        let tasks = project_launch_tasks(
            "project-1",
            "Ship the release",
            &["Smoke passes".to_string(), "Docs updated".to_string()],
        );

        assert_eq!(tasks.len(), 7);
        assert_eq!(tasks[0].title, "OBSERVE: capture current state");
        assert_eq!(tasks[6].title, "LEARN: checkpoint and improve");
        assert_eq!(
            tasks.iter().map(|task| task.priority).collect::<Vec<_>>(),
            vec![100, 90, 80, 70, 60, 50, 40]
        );
        assert!(tasks.iter().all(|task| task.project_id == "project-1"));
    }

    #[test]
    fn observe_task_carries_goal_and_acceptance_criteria() {
        let tasks = project_launch_tasks(
            "project-1",
            "Ship the release",
            &["Smoke passes".to_string(), "Docs updated".to_string()],
        );

        let observe = &tasks[0];
        assert!(observe.description.contains("Goal:\nShip the release"));
        assert!(observe.description.contains("- Smoke passes"));
        assert!(observe.description.contains("- Docs updated"));
    }

    #[test]
    fn launch_tasks_are_root_tasks_without_hidden_assignment() {
        let tasks = project_launch_tasks("project-1", "Ship the release", &[]);

        assert!(tasks.iter().all(|task| task.parent_id.is_none()));
        assert!(tasks.iter().all(|task| task.deadline.is_none()));
        assert!(tasks.iter().all(|task| task.assignee_agent_id.is_none()));
    }
}
