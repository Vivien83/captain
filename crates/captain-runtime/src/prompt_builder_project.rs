use crate::prompt_sanitizer::sanitize;

use super::prompt_builder_types::RecentProjectSummary;
use super::{cap_str, ActiveProjectSummary};

pub(super) fn format_active_project_section(
    project: &ActiveProjectSummary,
    compact: bool,
) -> String {
    let mut out = format_project_header(project, compact);
    push_project_metadata(&mut out, project, compact);
    push_project_work_lists(&mut out, project, compact);
    push_project_loop_instruction(&mut out, project, compact);
    out
}

pub(super) fn format_recent_projects_section(
    projects: &[RecentProjectSummary],
    compact: bool,
) -> String {
    let max_projects = if compact { 3 } else { 5 };
    let mut out = String::from("## Recent Projects");
    if compact {
        out.push_str("\nResolve user refs like `projet1` against slugs/names before treating numbers as menu choices. Projects store first; do not search files first.");
    } else {
        out.push_str(
            "\nThese are durable Captain projects visible to the user. When the user asks where a project stands or mentions a partial name/slug like `projet1`, resolve against this list before any filesystem/workspace search and before interpreting a number as a menu option. Use `project_list({\"query\":\"...\"})` or `project_get` for details. Only inspect files after the durable project state points to a concrete workspace need.",
        );
    }

    for project in projects.iter().take(max_projects) {
        push_recent_project_line(&mut out, project, compact);
    }
    out
}

fn push_recent_project_line(out: &mut String, project: &RecentProjectSummary, compact: bool) {
    let goal_limit = if compact { 120 } else { 180 };
    out.push_str(&format!(
        "\n- {} ({}) — project {}, runtime {}/{}, {}%",
        cap_str(&project.name, if compact { 90 } else { 140 }),
        cap_str(&project.slug, 80),
        cap_str(&project.status, 40),
        cap_str(&project.runtime_status, 40),
        cap_str(&project.runtime_phase, 40),
        project.progress.min(100)
    ));
    if !project.goal.trim().is_empty() {
        out.push_str(&format!(" — {}", cap_str(&project.goal, goal_limit)));
    }
    if !compact && !project.next_actions.is_empty() {
        out.push_str(" — next: ");
        out.push_str(
            &project
                .next_actions
                .iter()
                .take(2)
                .map(|action| cap_str(action, 100))
                .collect::<Vec<_>>()
                .join(" | "),
        );
    }
}

fn format_project_header(project: &ActiveProjectSummary, compact: bool) -> String {
    if compact {
        return format!(
            "## Active Project\nname: {} ({})\nstatus: {}",
            cap_str(&project.name, 120),
            cap_str(&project.slug, 80),
            cap_str(&project.status, 48)
        );
    }
    format!(
        "## Active Project\n{} ({}) — status: {}",
        cap_str(&project.name, 160),
        cap_str(&project.slug, 100),
        cap_str(&project.status, 64)
    )
}

fn push_project_metadata(out: &mut String, project: &ActiveProjectSummary, compact: bool) {
    push_optional_line(
        out,
        project_label(compact, "goal", "Goal"),
        non_empty(&project.goal).map(|s| cap_str(s, if compact { 240 } else { 500 })),
    );
    push_optional_line(
        out,
        project_label(compact, "source", "Source"),
        project.source_type.as_deref().filter(|s| !s.is_empty()),
    );
    push_optional_line(
        out,
        project_label(compact, "workspace", "Workspace"),
        project.workspace_path.as_deref().filter(|s| !s.is_empty()),
    );
    push_optional_line(
        out,
        project_label(compact, "repository", "Repository"),
        project.repository.as_deref().filter(|s| !s.is_empty()),
    );
    push_optional_line(
        out,
        project_label(compact, "milestones", "Milestone progress"),
        project
            .milestone_status
            .as_deref()
            .filter(|s| !s.is_empty()),
    );
    push_optional_line(
        out,
        project_label(compact, "latest_checkpoint", "Latest checkpoint"),
        project
            .latest_checkpoint
            .as_deref()
            .filter(|s| !s.is_empty())
            .map(|s| cap_str(s, if compact { 420 } else { 900 })),
    );

    push_project_list(
        out,
        project_label(compact, "goals", "Project goals"),
        &project.project_goals,
        if compact { 3 } else { 5 },
        if compact { 140 } else { 220 },
    );
    push_optional_line(
        out,
        project_label(compact, "rules", "Project rules (CAPTAIN.md)"),
        project
            .project_rules
            .as_deref()
            .filter(|s| !s.is_empty())
            .map(|s| cap_str(&sanitize("CAPTAIN.md", s), if compact { 520 } else { 1200 })),
    );
}

fn push_project_work_lists(out: &mut String, project: &ActiveProjectSummary, compact: bool) {
    push_project_list(
        out,
        project_label(compact, "active_tasks", "Active tasks"),
        &project.active_tasks,
        if compact { 4 } else { 8 },
        if compact { 140 } else { 220 },
    );
    push_project_list(
        out,
        project_label(compact, "blockers", "Blockers"),
        &project.blocked_tasks,
        if compact { 3 } else { 5 },
        if compact { 140 } else { 220 },
    );
    push_project_list(
        out,
        project_label(compact, "next", "Next actions"),
        &project.next_actions,
        if compact { 3 } else { 5 },
        if compact { 160 } else { 240 },
    );
}

fn push_project_loop_instruction(out: &mut String, project: &ActiveProjectSummary, compact: bool) {
    if compact {
        out.push_str(
            "\nloop: OBSERVE -> THINK -> PLAN -> BUILD -> EXECUTE -> VERIFY -> LEARN. \
             Keep project tasks/checkpoints current; call project_resume/checkpoint tools when detail is missing.",
        );
    } else {
        out.push_str(&format!(
            "\n\nScope reasoning and file operations to this project workspace. Follow the development loop: OBSERVE -> THINK -> PLAN -> BUILD -> EXECUTE -> VERIFY -> LEARN. Keep `project_task_list`, `milestone_list`, project goals, and `checkpoint_save` current as you work. Use `project_resume` when the checkpoint is insufficient. The user set this with `/project {}` and can clear via `/project clear`.",
            project.slug
        ));
    }
}

fn project_label(
    compact: bool,
    compact_label: &'static str,
    full_label: &'static str,
) -> &'static str {
    if compact {
        compact_label
    } else {
        full_label
    }
}

fn non_empty(value: &str) -> Option<&str> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed)
    }
}

fn push_optional_line<T: AsRef<str>>(out: &mut String, label: &str, value: Option<T>) {
    if let Some(value) = value {
        let value = value.as_ref().trim();
        if !value.is_empty() {
            out.push_str(&format!("\n{label}: {value}"));
        }
    }
}

fn push_project_list(
    out: &mut String,
    label: &str,
    items: &[String],
    max_items: usize,
    max_chars: usize,
) {
    let visible: Vec<&String> = items
        .iter()
        .filter(|item| !item.trim().is_empty())
        .collect();
    if visible.is_empty() {
        return;
    }
    out.push_str(&format!("\n{label}:"));
    for item in visible.into_iter().take(max_items) {
        out.push_str(&format!("\n- {}", cap_str(item.trim(), max_chars)));
    }
}
