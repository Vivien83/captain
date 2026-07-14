use std::sync::Arc;

use super::CaptainKernel;

const PROJECT_LIFECYCLE_PHASES: [&str; 7] = [
    "observe", "think", "plan", "build", "execute", "verify", "learn",
];

impl CaptainKernel {
    pub(super) fn handle_project_create(
        &self,
        name: &str,
        slug: &str,
        goal: &str,
        deadline: Option<i64>,
    ) -> Result<serde_json::Value, String> {
        let mut project = self
            .memory
            .project_create(captain_memory::project::NewProject {
                name: name.to_string(),
                slug: slug.to_string(),
                goal: goal.to_string(),
                deadline,
            })
            .map_err(|e| e.to_string())?;

        if let Ok(Some(updated)) = self.memory.project_update(
            &project.id,
            captain_memory::project::ProjectPatch {
                metadata: Some(project_lifecycle_metadata()),
                ..Default::default()
            },
        ) {
            project = updated;
        }

        for (idx, phase) in PROJECT_LIFECYCLE_PHASES.iter().enumerate() {
            let title = format!("{}: project phase", phase.to_ascii_uppercase());
            let _ = self
                .memory
                .task_create(captain_memory::project_task::NewProjectTask {
                    project_id: project.id.clone(),
                    parent_id: None,
                    title,
                    description: project_phase_description(phase).to_string(),
                    priority: 100 - (idx as i32 * 10),
                    deadline: None,
                    assignee_agent_id: None,
                });
        }

        // Best-effort: MemPalace outage must not fail project creation.
        let conns = Arc::clone(&self.mcp_connections);
        let slug_owned = project.slug.clone();
        let name_owned = project.name.clone();
        let goal_owned = project.goal.clone();
        tokio::spawn(async move {
            let _ = captain_runtime::project_memory::ensure_project_wing(
                Some(conns.as_ref()),
                &slug_owned,
                &name_owned,
                &goal_owned,
            )
            .await;
        });

        Ok(serde_json::to_value(&project).unwrap_or(serde_json::Value::Null))
    }

    pub(super) fn handle_project_list(
        &self,
        include_archived: bool,
    ) -> Result<serde_json::Value, String> {
        let rows = self
            .memory
            .project_list(include_archived)
            .map_err(|e| e.to_string())?;
        Ok(serde_json::to_value(&rows).unwrap_or(serde_json::Value::Array(vec![])))
    }

    pub(super) fn handle_project_find_by_slug(
        &self,
        slug: &str,
    ) -> Result<Option<serde_json::Value>, String> {
        let p = self
            .memory
            .project_find_by_slug(slug)
            .map_err(|e| e.to_string())?;
        Ok(p.map(|v| serde_json::to_value(&v).unwrap_or(serde_json::Value::Null)))
    }

    pub(super) fn handle_project_archive(
        &self,
        id: &str,
    ) -> Result<Option<serde_json::Value>, String> {
        let p = self.memory.project_archive(id).map_err(|e| e.to_string())?;
        Ok(p.map(|v| serde_json::to_value(&v).unwrap_or(serde_json::Value::Null)))
    }

    /// Permanently deletes a project and its goals. Unlike archive, this is
    /// not reversible — mirrors the web API's delete_project handler
    /// (project_delete_routes.rs) so both surfaces behave identically.
    pub(super) fn handle_project_delete(&self, id: &str) -> Result<bool, String> {
        let Some(project) = self.memory.project_get(id).map_err(|e| e.to_string())? else {
            return Ok(false);
        };
        self.goal_store
            .remove_for_project(&project.id, &project.slug)
            .map_err(|e| e.to_string())?;
        self.memory.project_delete(id).map_err(|e| e.to_string())
    }

    pub(super) fn handle_todo_create(
        &self,
        title: &str,
        body: &str,
    ) -> Result<serde_json::Value, String> {
        let t = self
            .memory
            .todo_create(captain_memory::todo::NewTodo {
                title: title.to_string(),
                body: body.to_string(),
            })
            .map_err(|e| e.to_string())?;
        Ok(serde_json::to_value(&t).unwrap_or(serde_json::Value::Null))
    }

    pub(super) fn handle_todo_list(
        &self,
        filter: &str,
        limit: Option<u32>,
    ) -> Result<serde_json::Value, String> {
        let rows = self
            .memory
            .todo_list(todo_filter_from_text(filter)?, limit)
            .map_err(|e| e.to_string())?;
        Ok(serde_json::to_value(&rows).unwrap_or(serde_json::Value::Array(vec![])))
    }

    pub(super) fn handle_todo_complete(
        &self,
        id: &str,
    ) -> Result<Option<serde_json::Value>, String> {
        let t = self.memory.todo_complete(id).map_err(|e| e.to_string())?;
        Ok(t.map(|v| serde_json::to_value(&v).unwrap_or(serde_json::Value::Null)))
    }

    pub(super) fn handle_todo_reopen(&self, id: &str) -> Result<Option<serde_json::Value>, String> {
        let t = self.memory.todo_reopen(id).map_err(|e| e.to_string())?;
        Ok(t.map(|v| serde_json::to_value(&v).unwrap_or(serde_json::Value::Null)))
    }

    pub(super) fn handle_todo_delete(&self, id: &str) -> Result<bool, String> {
        self.memory.todo_delete(id).map_err(|e| e.to_string())
    }

    pub(super) fn handle_project_task_create(
        &self,
        project_id: &str,
        title: &str,
        description: &str,
        parent_id: Option<&str>,
    ) -> Result<serde_json::Value, String> {
        let t = self
            .memory
            .task_create(captain_memory::project_task::NewProjectTask {
                project_id: project_id.to_string(),
                parent_id: parent_id.map(|s| s.to_string()),
                title: title.to_string(),
                description: description.to_string(),
                priority: 0,
                deadline: None,
                assignee_agent_id: None,
            })
            .map_err(|e| e.to_string())?;
        Ok(serde_json::to_value(&t).unwrap_or(serde_json::Value::Null))
    }

    pub(super) fn handle_project_task_list(
        &self,
        project_id: &str,
    ) -> Result<serde_json::Value, String> {
        let rows = self
            .memory
            .task_list_for_project(project_id)
            .map_err(|e| e.to_string())?;
        Ok(serde_json::to_value(&rows).unwrap_or(serde_json::Value::Array(vec![])))
    }

    pub(super) fn handle_project_task_update_status(
        &self,
        id: &str,
        status: &str,
    ) -> Result<Option<serde_json::Value>, String> {
        let t = self
            .memory
            .task_update(
                id,
                captain_memory::project_task::TaskPatch {
                    status: Some(project_task_status_from_text(status)?),
                    ..Default::default()
                },
            )
            .map_err(|e| e.to_string())?;
        Ok(t.map(|v| serde_json::to_value(&v).unwrap_or(serde_json::Value::Null)))
    }

    pub(super) fn handle_milestone_create(
        &self,
        project_id: &str,
        name: &str,
        due_date: Option<i64>,
    ) -> Result<serde_json::Value, String> {
        let m = self
            .memory
            .milestone_create(captain_memory::milestone::NewMilestone {
                project_id: project_id.to_string(),
                name: name.to_string(),
                due_date,
                deliverables: vec![],
            })
            .map_err(|e| e.to_string())?;
        Ok(serde_json::to_value(&m).unwrap_or(serde_json::Value::Null))
    }

    pub(super) fn handle_milestone_list(
        &self,
        project_id: &str,
    ) -> Result<serde_json::Value, String> {
        let rows = self
            .memory
            .milestone_list_for_project(project_id)
            .map_err(|e| e.to_string())?;
        Ok(serde_json::to_value(&rows).unwrap_or(serde_json::Value::Array(vec![])))
    }

    pub(super) fn handle_milestone_complete(
        &self,
        id: &str,
    ) -> Result<Option<serde_json::Value>, String> {
        let m = self
            .memory
            .milestone_complete(id)
            .map_err(|e| e.to_string())?;
        Ok(m.map(|v| serde_json::to_value(&v).unwrap_or(serde_json::Value::Null)))
    }

    pub(super) fn handle_milestone_progress(
        &self,
        project_id: &str,
    ) -> Result<serde_json::Value, String> {
        let now = chrono::Utc::now().timestamp_millis();
        let p = self
            .memory
            .milestone_progress(project_id, now)
            .map_err(|e| e.to_string())?;
        Ok(serde_json::to_value(p).unwrap_or(serde_json::Value::Null))
    }

    pub(super) fn handle_checkpoint_save(
        &self,
        project_id: &str,
        summary: &str,
        state: serde_json::Value,
        session_id: Option<&str>,
    ) -> Result<serde_json::Value, String> {
        let cp = self
            .memory
            .checkpoint_append(captain_memory::project_checkpoint::NewCheckpoint {
                project_id: project_id.to_string(),
                session_id: session_id.map(|s| s.to_string()),
                summary: summary.to_string(),
                state,
            })
            .map_err(|e| e.to_string())?;
        Ok(serde_json::to_value(&cp).unwrap_or(serde_json::Value::Null))
    }

    pub(super) fn handle_project_resume(&self, slug: &str) -> Result<serde_json::Value, String> {
        let project = self
            .memory
            .project_find_by_slug(slug)
            .map_err(|e| e.to_string())?
            .ok_or_else(|| format!("Project '{slug}' not found"))?;
        let latest_checkpoint = self
            .memory
            .checkpoint_latest(&project.id)
            .map_err(|e| e.to_string())?;
        let tasks = self
            .memory
            .task_list_for_project(&project.id)
            .map_err(|e| e.to_string())?;
        let now = chrono::Utc::now().timestamp_millis();
        let progress = self
            .memory
            .milestone_progress(&project.id, now)
            .map_err(|e| e.to_string())?;
        let goals = self.goal_store.list_for_project(&project.id, &project.slug);
        Ok(serde_json::json!({
            "project": project,
            "checkpoint": latest_checkpoint,
            "tasks": tasks,
            "goals": goals,
            "milestone_progress": progress,
        }))
    }

    pub(super) fn handle_active_project_set(
        &self,
        agent_id: &str,
        slug: Option<&str>,
    ) -> Result<(), String> {
        let Some(registry) = captain_runtime::active_project::global() else {
            return Err("active_project registry not installed".into());
        };
        match slug {
            Some(s) => registry.set(agent_id.to_string(), s.to_string()),
            None => {
                registry.clear(agent_id);
            }
        }
        Ok(())
    }

    pub(super) fn handle_active_project_get(&self, agent_id: &str) -> Option<String> {
        captain_runtime::active_project::global()?.get(agent_id)
    }
}

fn project_lifecycle_metadata() -> serde_json::Value {
    serde_json::json!({
        "lifecycle": {
            "protocol": "captain.project_lifecycle.v1",
            "required": true,
            "current_phase": "observe",
            "phases": PROJECT_LIFECYCLE_PHASES,
        },
        "product_target": "autonomous_development_project",
    })
}

fn project_phase_description(phase: &str) -> &'static str {
    match phase {
        "observe" => "Capture current state, constraints, user context, and blockers.",
        "think" => "Compare options, risks, existing skills, and likely regressions.",
        "plan" => "Create executable slices and define verification commands.",
        "build" => "Implement focused slices while preserving unrelated user work.",
        "execute" => "Wire runtime behavior and make live state observable.",
        "verify" => "Run the relevant checks and record failures as blockers.",
        "learn" => "Checkpoint reusable learning without duplicating existing knowledge.",
        _ => "Advance the project lifecycle.",
    }
}

fn todo_filter_from_text(filter: &str) -> Result<captain_memory::todo::TodoFilter, String> {
    match filter {
        "open" | "" => Ok(captain_memory::todo::TodoFilter::Open),
        "done" => Ok(captain_memory::todo::TodoFilter::Done),
        "all" => Ok(captain_memory::todo::TodoFilter::All),
        other => Err(format!(
            "Unknown todo filter: {other} (expected open|done|all)"
        )),
    }
}

fn project_task_status_from_text(
    status: &str,
) -> Result<captain_memory::project_task::TaskStatus, String> {
    captain_memory::project_task::TaskStatus::from_str(status)
        .ok_or_else(|| format!("Unknown task status: {status}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lifecycle_metadata_matches_project_contract() {
        let metadata = project_lifecycle_metadata();
        assert_eq!(
            metadata["lifecycle"]["protocol"],
            "captain.project_lifecycle.v1"
        );
        assert_eq!(metadata["lifecycle"]["current_phase"], "observe");
        assert_eq!(metadata["lifecycle"]["phases"].as_array().unwrap().len(), 7);
        assert_eq!(metadata["product_target"], "autonomous_development_project");
    }

    #[test]
    fn lifecycle_phase_descriptions_are_stable() {
        assert_eq!(
            project_phase_description("verify"),
            "Run the relevant checks and record failures as blockers."
        );
        assert_eq!(
            project_phase_description("unknown"),
            "Advance the project lifecycle."
        );
    }

    #[test]
    fn todo_filter_accepts_public_filters() {
        assert_eq!(
            todo_filter_from_text("").unwrap(),
            captain_memory::todo::TodoFilter::Open
        );
        assert_eq!(
            todo_filter_from_text("open").unwrap(),
            captain_memory::todo::TodoFilter::Open
        );
        assert_eq!(
            todo_filter_from_text("done").unwrap(),
            captain_memory::todo::TodoFilter::Done
        );
        assert_eq!(
            todo_filter_from_text("all").unwrap(),
            captain_memory::todo::TodoFilter::All
        );
    }

    #[test]
    fn todo_filter_rejects_unknown_filter_with_contract_error() {
        let err = todo_filter_from_text("later").unwrap_err();
        assert_eq!(err, "Unknown todo filter: later (expected open|done|all)");
    }

    #[test]
    fn project_task_status_rejects_unknown_status_with_contract_error() {
        let err = project_task_status_from_text("waiting").unwrap_err();
        assert_eq!(err, "Unknown task status: waiting");
    }
}
