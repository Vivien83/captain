use captain_memory::project;

pub(crate) fn project_session_id(project: &project::Project) -> String {
    let slug = project.slug.trim();
    if slug.is_empty() {
        format!("project-{}", project.id)
    } else {
        format!("project-{slug}")
    }
}

pub(crate) fn default_project_parallelism() -> usize {
    project_parallelism_from_available(std::thread::available_parallelism().ok().map(|n| n.get()))
}

fn project_parallelism_from_available(available: Option<usize>) -> usize {
    available.map(|n| n.clamp(1, 3)).unwrap_or(2)
}

#[cfg(test)]
mod tests {
    use super::*;
    use captain_memory::project::{Project, ProjectStatus};

    fn project_with_slug(slug: &str) -> Project {
        Project {
            id: "project-id".to_string(),
            name: "Demo".to_string(),
            slug: slug.to_string(),
            goal: "Ship".to_string(),
            status: ProjectStatus::Active,
            deadline: None,
            created_at: 0,
            updated_at: 0,
            metadata: serde_json::json!({}),
        }
    }

    #[test]
    fn project_session_id_prefers_trimmed_slug() {
        let project = project_with_slug(" demo-project ");
        assert_eq!(project_session_id(&project), "project-demo-project");
    }

    #[test]
    fn project_session_id_falls_back_to_id_when_slug_is_blank() {
        let project = project_with_slug("  ");
        assert_eq!(project_session_id(&project), "project-project-id");
    }

    #[test]
    fn project_parallelism_is_bounded_and_has_fallback() {
        assert_eq!(project_parallelism_from_available(None), 2);
        assert_eq!(project_parallelism_from_available(Some(0)), 1);
        assert_eq!(project_parallelism_from_available(Some(1)), 1);
        assert_eq!(project_parallelism_from_available(Some(2)), 2);
        assert_eq!(project_parallelism_from_available(Some(99)), 3);
    }
}
