use std::path::Path;

use captain_memory::project::{Project, ProjectStatus};
use captain_memory::project_task::{ProjectTask, TaskStatus};
use captain_memory::MemorySubstrate;
use captain_runtime::prompt_builder::RecentProjectSummary;
use captain_types::agent::AgentId;

use crate::goals::{Goal, GoalStatus, GoalStore};

/// Resolve the active project for an agent into the compact summary
/// the prompt builder expects. Returns `None` when no project is set
/// or when the registered slug no longer resolves (archived, renamed).
pub(super) fn resolve_active_project(
    memory: &MemorySubstrate,
    goal_store: &GoalStore,
    agent_id: AgentId,
) -> Option<captain_runtime::prompt_builder::ActiveProjectSummary> {
    let reg = captain_runtime::active_project::global()?;
    let slug = reg.get(&agent_id.to_string())?;
    let project = memory.project_find_by_slug(&slug).ok().flatten()?;
    let source = project_prompt_source(&project);
    let project_rules = project_rules_for_prompt(&project, source.workspace_path.as_deref());
    let latest_checkpoint = latest_checkpoint_for_prompt(memory, &project.id);

    let tasks = memory
        .task_list_for_project(&project.id)
        .unwrap_or_default();
    let task_summary = project_tasks_for_prompt(&tasks);
    let milestone_status = milestone_status_for_prompt(memory, &project.id);
    let project_goals = project_goals_for_prompt(goal_store, &project);

    Some(captain_runtime::prompt_builder::ActiveProjectSummary {
        id: project.id,
        slug: project.slug,
        name: project.name,
        goal: project.goal,
        status: project.status.as_str().to_string(),
        source_type: source.source_type,
        workspace_path: source.workspace_path,
        repository: source.repository,
        latest_checkpoint,
        active_tasks: task_summary.active_tasks,
        blocked_tasks: task_summary.blocked_tasks,
        next_actions: task_summary.next_actions,
        milestone_status,
        project_goals,
        project_rules,
    })
}

pub(super) fn resolve_recent_projects(
    memory: &MemorySubstrate,
    active_project_slug: Option<&str>,
) -> Vec<RecentProjectSummary> {
    memory
        .project_list(false)
        .unwrap_or_default()
        .into_iter()
        .filter(|project| Some(project.slug.as_str()) != active_project_slug)
        .filter(|project| {
            matches!(
                project.status,
                ProjectStatus::Planning | ProjectStatus::Active | ProjectStatus::Paused
            )
        })
        .take(5)
        .map(recent_project_summary_for_prompt)
        .collect()
}

fn recent_project_summary_for_prompt(project: Project) -> RecentProjectSummary {
    let runtime = project
        .metadata
        .get("runtime")
        .cloned()
        .unwrap_or(serde_json::Value::Null);
    let runtime_status = runtime
        .get("status")
        .and_then(|value| value.as_str())
        .unwrap_or(match project.status {
            ProjectStatus::Active => "running",
            ProjectStatus::Paused => "paused",
            _ => "ready",
        })
        .to_string();
    let runtime_phase = runtime
        .get("current_phase")
        .and_then(|value| value.as_str())
        .or_else(|| {
            project
                .metadata
                .pointer("/lifecycle/current_phase")
                .and_then(|value| value.as_str())
        })
        .unwrap_or("observe")
        .to_string();
    let progress = runtime
        .get("progress")
        .and_then(|value| value.as_u64())
        .unwrap_or_else(|| recent_project_progress_for_phase(&runtime_phase, &runtime_status))
        .min(100);
    let slug = project.slug.clone();

    RecentProjectSummary {
        slug: project.slug,
        name: project.name,
        goal: project.goal,
        status: project.status.as_str().to_string(),
        runtime_status: runtime_status.clone(),
        runtime_phase,
        progress,
        next_actions: recent_project_next_actions(&slug, &runtime_status),
    }
}

fn recent_project_progress_for_phase(phase: &str, status: &str) -> u64 {
    if status == "done" {
        return 100;
    }
    if status == "paused" {
        return match phase {
            "observe" => 8,
            "think" => 18,
            "plan" => 32,
            "build" => 52,
            "execute" => 70,
            "verify" => 86,
            "learn" => 96,
            _ => 0,
        };
    }
    match phase {
        "observe" => 10,
        "think" => 22,
        "plan" => 36,
        "build" => 56,
        "execute" => 74,
        "verify" => 88,
        "learn" => 98,
        _ => 0,
    }
}

fn recent_project_next_actions(slug: &str, runtime_status: &str) -> Vec<String> {
    let mut actions = vec![format!("project_list {{\"query\":\"{slug}\"}}")];
    actions.push(format!("project_get {{\"slug\":\"{slug}\"}}"));
    match runtime_status {
        "paused" | "blocked" | "failed" => {
            actions.push(format!("project_resume {{\"slug\":\"{slug}\"}}"));
        }
        "ready" => {
            actions.push(format!("captain project start {slug}"));
        }
        "running" => {
            actions.push(format!("captain project status {slug}"));
        }
        _ => {}
    }
    actions
}

struct ProjectPromptSource {
    workspace_path: Option<String>,
    repository: Option<String>,
    source_type: Option<String>,
}

struct ProjectTaskPromptSummary {
    active_tasks: Vec<String>,
    blocked_tasks: Vec<String>,
    next_actions: Vec<String>,
}

fn project_prompt_source(project: &Project) -> ProjectPromptSource {
    let source = project
        .metadata
        .pointer("/launch/source")
        .or_else(|| project.metadata.get("source"));
    let workspace_path = project
        .metadata
        .pointer("/launch/workspace/path")
        .and_then(|v| v.as_str())
        .or_else(|| {
            source
                .and_then(|s| s.get("local_path"))
                .and_then(|v| v.as_str())
        })
        .or_else(|| source.and_then(|s| s.get("path")).and_then(|v| v.as_str()))
        .map(str::to_string);
    let repository = source
        .and_then(|s| s.get("full_name"))
        .and_then(|v| v.as_str())
        .map(str::to_string);
    let source_type = source
        .and_then(|s| s.get("type"))
        .and_then(|v| v.as_str())
        .map(str::to_string);

    ProjectPromptSource {
        workspace_path,
        repository,
        source_type,
    }
}

fn project_rules_for_prompt(project: &Project, workspace_path: Option<&str>) -> Option<String> {
    workspace_path
        .map(Path::new)
        .map(|path| {
            captain_runtime::project_rules::seed_captain_project_rules_file(
                path,
                &project.name,
                &project.slug,
                &project.goal,
            );
            path.join(captain_runtime::project_rules::CAPTAIN_PROJECT_RULES_FILE)
        })
        .and_then(|path| std::fs::read_to_string(path).ok())
        .map(|rules| compact_prompt_text(&rules, 1_400))
}

fn latest_checkpoint_for_prompt(memory: &MemorySubstrate, project_id: &str) -> Option<String> {
    memory
        .checkpoint_latest(project_id)
        .ok()
        .flatten()
        .map(|checkpoint| compact_prompt_text(&checkpoint.summary, 900))
}

fn project_tasks_for_prompt(tasks: &[ProjectTask]) -> ProjectTaskPromptSummary {
    let active_tasks = tasks
        .iter()
        .filter(|task| !task.status.is_terminal())
        .take(8)
        .map(format_project_task_for_prompt)
        .collect();
    let blocked_tasks = tasks
        .iter()
        .filter(|task| task.status == TaskStatus::Blocked)
        .take(5)
        .map(format_project_task_for_prompt)
        .collect();
    let next_actions = next_project_actions_for_prompt(tasks);

    ProjectTaskPromptSummary {
        active_tasks,
        blocked_tasks,
        next_actions,
    }
}

fn milestone_status_for_prompt(memory: &MemorySubstrate, project_id: &str) -> Option<String> {
    let now_ms = chrono::Utc::now().timestamp_millis();
    memory
        .milestone_progress(project_id, now_ms)
        .ok()
        .map(|progress| {
            format!(
                "{}/{} completed ({:.0}%), {} missed",
                progress.completed,
                progress.total,
                progress.pct * 100.0,
                progress.missed
            )
        })
}

fn project_goals_for_prompt(goal_store: &GoalStore, project: &Project) -> Vec<String> {
    goal_store
        .list_for_project(&project.id, &project.slug)
        .into_iter()
        .take(5)
        .map(|goal| format_goal_for_prompt(&goal))
        .collect()
}

fn compact_prompt_text(input: &str, max_chars: usize) -> String {
    let compact = input.split_whitespace().collect::<Vec<_>>().join(" ");
    if compact.chars().count() <= max_chars {
        compact
    } else {
        let mut out: String = compact.chars().take(max_chars.saturating_sub(3)).collect();
        out.push_str("...");
        out
    }
}

fn format_project_task_for_prompt(task: &ProjectTask) -> String {
    let mut line = format!(
        "{}: {}",
        task.status.as_str().to_ascii_uppercase(),
        compact_prompt_text(&task.title, 160)
    );
    if let Some(agent) = task.assignee_agent_id.as_deref().filter(|s| !s.is_empty()) {
        line.push_str(&format!(" (agent: {})", compact_prompt_text(agent, 80)));
    }
    if task.priority != 0 {
        line.push_str(&format!(" [p{}]", task.priority));
    }
    line
}

fn next_project_actions_for_prompt(tasks: &[ProjectTask]) -> Vec<String> {
    let mut out = Vec::new();
    for status in [TaskStatus::Doing, TaskStatus::Review, TaskStatus::Todo] {
        for task in tasks.iter().filter(|task| task.status == status) {
            if out.len() >= 5 {
                return out;
            }
            out.push(format_project_task_for_prompt(task));
        }
    }
    out
}

fn format_goal_for_prompt(goal: &Goal) -> String {
    let status = match goal.status {
        GoalStatus::Active => "active",
        GoalStatus::Paused => "paused",
        GoalStatus::Escalated => "escalated",
    };
    format!(
        "{}: {} [{}] every {}s",
        compact_prompt_text(&goal.id, 80),
        compact_prompt_text(&goal.name, 140),
        status,
        goal.interval_secs
    )
}

/// Read the most recent journal entries from the agent's memory/ directory.
/// Returns concatenated content from the last `days` files, capped at 4KB.
pub(super) fn read_recent_journal(memory_dir: &Path, days: usize) -> Option<String> {
    let entries = std::fs::read_dir(memory_dir).ok()?;
    let mut files: Vec<_> = entries
        .filter_map(|e| e.ok())
        .filter(|e| e.path().extension().map(|ext| ext == "md").unwrap_or(false))
        .collect();
    // Sort by filename descending (YYYY-MM-DD.md -> newest first)
    files.sort_by_key(|e| std::cmp::Reverse(e.file_name()));
    files.truncate(days);

    let mut combined = String::new();
    const MAX_JOURNAL_BYTES: usize = 4096;
    for entry in files.iter().rev() {
        if combined.len() >= MAX_JOURNAL_BYTES {
            break;
        }
        let name = entry.file_name();
        let name_str = name.to_string_lossy();
        if let Ok(content) = std::fs::read_to_string(entry.path()) {
            if !content.trim().is_empty() {
                combined.push_str(&format!("### {}\n", name_str.trim_end_matches(".md")));
                let remaining = MAX_JOURNAL_BYTES.saturating_sub(combined.len());
                if content.len() > remaining {
                    combined.push_str(&content[..remaining]);
                } else {
                    combined.push_str(&content);
                }
                combined.push('\n');
            }
        }
    }

    if combined.trim().is_empty() {
        None
    } else {
        Some(combined)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn task(status: TaskStatus, title: &str, priority: i32) -> ProjectTask {
        ProjectTask {
            id: format!("task-{title}"),
            project_id: "project-1".to_string(),
            parent_id: None,
            title: title.to_string(),
            description: String::new(),
            status,
            assignee_agent_id: None,
            priority,
            deadline: None,
            created_at: 0,
            updated_at: 0,
            completed_at: None,
        }
    }

    fn project(metadata: serde_json::Value) -> Project {
        Project {
            id: "project-1".to_string(),
            name: "Project One".to_string(),
            slug: "project-one".to_string(),
            goal: "Ship it".to_string(),
            status: captain_memory::project::ProjectStatus::Active,
            deadline: None,
            created_at: 0,
            updated_at: 0,
            metadata,
        }
    }

    fn project_with_slug(
        slug: &str,
        status: ProjectStatus,
        metadata: serde_json::Value,
    ) -> Project {
        Project {
            id: format!("id-{slug}"),
            name: format!("Project {slug}"),
            slug: slug.to_string(),
            goal: format!("Goal for {slug}"),
            status,
            deadline: None,
            created_at: 0,
            updated_at: 0,
            metadata,
        }
    }

    #[test]
    fn compact_prompt_text_collapses_and_truncates() {
        assert_eq!(
            compact_prompt_text("  alpha\n\n beta\tgamma  ", 100),
            "alpha beta gamma"
        );
        assert_eq!(compact_prompt_text("abcdef", 6), "abcdef");
        assert_eq!(compact_prompt_text("abcdef", 5), "ab...");
    }

    #[test]
    fn next_project_actions_prioritizes_doing_review_todo() {
        let tasks = vec![
            task(TaskStatus::Todo, "todo", 0),
            task(TaskStatus::Review, "review", 0),
            task(TaskStatus::Doing, "doing", 3),
            task(TaskStatus::Blocked, "blocked", 0),
            task(TaskStatus::Done, "done", 0),
        ];
        assert_eq!(
            next_project_actions_for_prompt(&tasks),
            vec![
                "DOING: doing [p3]".to_string(),
                "REVIEW: review".to_string(),
                "TODO: todo".to_string(),
            ]
        );
    }

    #[test]
    fn project_prompt_source_prefers_launch_workspace_and_reads_source_metadata() {
        let project = project(serde_json::json!({
            "launch": {
                "workspace": {"path": "/workspace/launch"},
                "source": {
                    "type": "github",
                    "full_name": "owner/repo",
                    "local_path": "/workspace/source"
                }
            }
        }));

        let source = project_prompt_source(&project);

        assert_eq!(source.workspace_path.as_deref(), Some("/workspace/launch"));
        assert_eq!(source.repository.as_deref(), Some("owner/repo"));
        assert_eq!(source.source_type.as_deref(), Some("github"));
    }

    #[test]
    fn project_tasks_for_prompt_groups_active_blocked_and_next_actions() {
        let tasks = vec![
            task(TaskStatus::Done, "done", 0),
            task(TaskStatus::Blocked, "blocked", 0),
            task(TaskStatus::Review, "review", 0),
            task(TaskStatus::Doing, "doing", 1),
        ];

        let summary = project_tasks_for_prompt(&tasks);

        assert_eq!(summary.active_tasks.len(), 3);
        assert_eq!(summary.blocked_tasks, vec!["BLOCKED: blocked"]);
        assert_eq!(
            summary.next_actions,
            vec![
                "DOING: doing [p1]".to_string(),
                "REVIEW: review".to_string()
            ]
        );
    }

    #[test]
    fn recent_project_summary_keeps_runtime_state_and_actions() {
        let summary = recent_project_summary_for_prompt(project_with_slug(
            "projet1-documents-couple",
            ProjectStatus::Planning,
            serde_json::json!({
                "runtime": {
                    "status": "ready",
                    "current_phase": "observe",
                    "progress": 10
                }
            }),
        ));

        assert_eq!(summary.slug, "projet1-documents-couple");
        assert_eq!(summary.status, "planning");
        assert_eq!(summary.runtime_status, "ready");
        assert_eq!(summary.runtime_phase, "observe");
        assert_eq!(summary.progress, 10);
        assert!(summary
            .next_actions
            .iter()
            .any(|action| action.contains("project_get")));
        assert!(summary
            .next_actions
            .iter()
            .any(|action| action.contains("captain project start")));
    }

    #[test]
    fn recent_project_summary_uses_phase_progress_fallback() {
        let summary = recent_project_summary_for_prompt(project_with_slug(
            "projet1-documents-couple",
            ProjectStatus::Planning,
            serde_json::json!({
                "runtime": {
                    "status": "ready",
                    "current_phase": "observe"
                }
            }),
        ));

        assert_eq!(summary.progress, 10);
    }

    #[test]
    fn recent_journal_reads_latest_days_in_chronological_order() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("2026-05-29.md"), "old").unwrap();
        std::fs::write(dir.path().join("2026-05-30.md"), "middle").unwrap();
        std::fs::write(dir.path().join("2026-05-31.md"), "new").unwrap();
        std::fs::write(dir.path().join("notes.txt"), "ignored").unwrap();

        let journal = read_recent_journal(dir.path(), 2).unwrap();
        assert!(journal.contains("### 2026-05-30\nmiddle"));
        assert!(journal.contains("### 2026-05-31\nnew"));
        assert!(!journal.contains("old"));
        assert!(journal.find("2026-05-30").unwrap() < journal.find("2026-05-31").unwrap());
    }
}
