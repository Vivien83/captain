use captain_memory::project::ProjectStatus;
use captain_memory::project_task::TaskStatus;
use serde_json::Value;

const PROJECT_NAME_LIMIT: usize = 160;
const PROJECT_GOAL_LIMIT: usize = 2_000;
const PROJECT_SLUG_LIMIT: usize = 64;
const PROJECT_TASK_TITLE_LIMIT: usize = 180;
const PROJECT_TASK_DESCRIPTION_LIMIT: usize = 2_000;
const PROJECT_MILESTONE_NAME_LIMIT: usize = 180;
const PROJECT_MILESTONE_DELIVERABLE_LIMIT: usize = 300;
const PROJECT_MILESTONE_DELIVERABLES_LIMIT: usize = 20;
const PROJECT_CHECKPOINT_SUMMARY_LIMIT: usize = 600;
const PROJECT_CHECKPOINT_SESSION_ID_LIMIT: usize = 120;

pub(crate) const PROJECT_METADATA_PATCH_ERROR: &str =
    "project metadata is managed by dedicated runtime endpoints; update fields only";
pub(crate) const PROJECT_STATUS_ERROR: &str =
    "unknown project status; expected planning, active, paused, done, or archived";
pub(crate) const PROJECT_SLUG_ERROR: &str = "slug must be lowercase alphanumeric with hyphens";
pub(crate) const PROJECT_TASK_STATUS_ERROR: &str =
    "unknown project task status; expected todo, doing, blocked, review, done, or cancelled";
pub(crate) const PROJECT_TASK_TITLE_EMPTY_ERROR: &str = "task title cannot be empty";
pub(crate) const PROJECT_TASK_TITLE_LONG_ERROR: &str = "task title is too long";
pub(crate) const PROJECT_TASK_DESCRIPTION_LONG_ERROR: &str = "task description is too long";
pub(crate) const PROJECT_MILESTONE_NAME_EMPTY_ERROR: &str = "milestone name cannot be empty";
pub(crate) const PROJECT_MILESTONE_NAME_LONG_ERROR: &str = "milestone name is too long";
pub(crate) const PROJECT_MILESTONE_DELIVERABLE_LONG_ERROR: &str =
    "milestone deliverable is too long";
pub(crate) const PROJECT_MILESTONE_DELIVERABLES_COUNT_ERROR: &str =
    "too many milestone deliverables";
pub(crate) const PROJECT_CHECKPOINT_SUMMARY_EMPTY_ERROR: &str =
    "checkpoint summary cannot be empty";
pub(crate) const PROJECT_CHECKPOINT_SUMMARY_LONG_ERROR: &str = "checkpoint summary is too long";
pub(crate) const PROJECT_CHECKPOINT_SESSION_ID_LONG_ERROR: &str =
    "checkpoint session_id is too long";
pub(crate) const PROJECT_CHECKPOINT_STATE_ERROR: &str =
    "checkpoint state is managed by runtime checkpoints; submit summary only";
pub(crate) const PROJECT_LIFECYCLE_PHASES: &[&str] = &[
    "observe", "think", "plan", "build", "execute", "verify", "learn",
];
pub(crate) const PROJECT_LIFECYCLE_PHASE_ERROR: &str =
    "unknown project lifecycle phase; expected observe, think, plan, build, execute, verify, or learn";

pub(crate) fn normalize_project_update_name(
    name: Option<String>,
) -> Result<Option<String>, &'static str> {
    let Some(name) = name else {
        return Ok(None);
    };
    normalize_required_text(
        name,
        "name cannot be empty",
        "name is too long",
        PROJECT_NAME_LIMIT,
    )
    .map(Some)
}

pub(crate) fn normalize_project_create_name(name: String) -> Result<String, &'static str> {
    normalize_required_text(
        name,
        "name cannot be empty",
        "name is too long",
        PROJECT_NAME_LIMIT,
    )
}

pub(crate) fn normalize_project_update_goal(
    goal: Option<String>,
) -> Result<Option<String>, &'static str> {
    let Some(goal) = goal else {
        return Ok(None);
    };
    normalize_required_text(
        goal,
        "goal cannot be empty",
        "goal is too long",
        PROJECT_GOAL_LIMIT,
    )
    .map(Some)
}

pub(crate) fn normalize_project_create_goal(goal: String) -> Result<String, &'static str> {
    normalize_optional_text(goal, "goal is too long", PROJECT_GOAL_LIMIT)
}

pub(crate) fn normalize_project_slug(slug: String) -> Result<String, &'static str> {
    let slug = slug.trim();
    if slug.is_empty()
        || slug.len() > PROJECT_SLUG_LIMIT
        || !slug
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
    {
        return Err(PROJECT_SLUG_ERROR);
    }
    Ok(slug.to_string())
}

pub(crate) fn normalize_project_update_status(
    status: Option<String>,
) -> Result<Option<ProjectStatus>, &'static str> {
    let Some(status) = status else {
        return Ok(None);
    };
    ProjectStatus::from_str(status.trim())
        .ok_or(PROJECT_STATUS_ERROR)
        .map(Some)
}

pub(crate) fn normalize_project_task_update_status(
    status: Option<String>,
) -> Result<Option<TaskStatus>, &'static str> {
    let Some(status) = status else {
        return Ok(None);
    };
    TaskStatus::from_str(status.trim())
        .ok_or(PROJECT_TASK_STATUS_ERROR)
        .map(Some)
}

pub(crate) fn normalize_project_task_title(title: String) -> Result<String, &'static str> {
    normalize_required_text(
        title,
        PROJECT_TASK_TITLE_EMPTY_ERROR,
        PROJECT_TASK_TITLE_LONG_ERROR,
        PROJECT_TASK_TITLE_LIMIT,
    )
}

pub(crate) fn normalize_project_task_update_title(
    title: Option<String>,
) -> Result<Option<String>, &'static str> {
    let Some(title) = title else {
        return Ok(None);
    };
    normalize_project_task_title(title).map(Some)
}

pub(crate) fn normalize_project_task_description(
    description: String,
) -> Result<String, &'static str> {
    normalize_optional_text(
        description,
        PROJECT_TASK_DESCRIPTION_LONG_ERROR,
        PROJECT_TASK_DESCRIPTION_LIMIT,
    )
}

pub(crate) fn normalize_project_task_update_description(
    description: Option<String>,
) -> Result<Option<String>, &'static str> {
    let Some(description) = description else {
        return Ok(None);
    };
    normalize_project_task_description(description).map(Some)
}

pub(crate) fn normalize_project_milestone_name(name: String) -> Result<String, &'static str> {
    normalize_required_text(
        name,
        PROJECT_MILESTONE_NAME_EMPTY_ERROR,
        PROJECT_MILESTONE_NAME_LONG_ERROR,
        PROJECT_MILESTONE_NAME_LIMIT,
    )
}

pub(crate) fn normalize_project_milestone_deliverables(
    deliverables: Vec<String>,
) -> Result<Vec<String>, &'static str> {
    let mut normalized = Vec::new();
    for deliverable in deliverables {
        let deliverable = normalize_optional_text(
            deliverable,
            PROJECT_MILESTONE_DELIVERABLE_LONG_ERROR,
            PROJECT_MILESTONE_DELIVERABLE_LIMIT,
        )?;
        if !deliverable.is_empty() {
            normalized.push(deliverable);
        }
    }
    if normalized.len() > PROJECT_MILESTONE_DELIVERABLES_LIMIT {
        return Err(PROJECT_MILESTONE_DELIVERABLES_COUNT_ERROR);
    }
    Ok(normalized)
}

pub(crate) fn normalize_project_checkpoint_summary(
    summary: String,
) -> Result<String, &'static str> {
    normalize_required_text(
        summary,
        PROJECT_CHECKPOINT_SUMMARY_EMPTY_ERROR,
        PROJECT_CHECKPOINT_SUMMARY_LONG_ERROR,
        PROJECT_CHECKPOINT_SUMMARY_LIMIT,
    )
}

pub(crate) fn normalize_project_checkpoint_session_id(
    session_id: Option<String>,
) -> Result<Option<String>, &'static str> {
    let Some(session_id) = session_id else {
        return Ok(None);
    };
    let session_id = normalize_optional_text(
        session_id,
        PROJECT_CHECKPOINT_SESSION_ID_LONG_ERROR,
        PROJECT_CHECKPOINT_SESSION_ID_LIMIT,
    )?;
    Ok((!session_id.is_empty()).then_some(session_id))
}

pub(crate) fn rejects_project_checkpoint_state(state: &Value) -> bool {
    match state {
        Value::Null => false,
        Value::Object(map) if map.is_empty() => false,
        _ => true,
    }
}

pub(crate) fn normalize_project_lifecycle_phase(phase: String) -> Result<String, &'static str> {
    let phase = phase.trim().to_ascii_lowercase();
    if PROJECT_LIFECYCLE_PHASES
        .iter()
        .any(|candidate| *candidate == phase)
    {
        Ok(phase)
    } else {
        Err(PROJECT_LIFECYCLE_PHASE_ERROR)
    }
}

pub(crate) fn rejects_project_metadata_patch(metadata: &Option<Value>) -> bool {
    metadata
        .as_ref()
        .map(|value| !value.is_null())
        .unwrap_or(false)
}

fn normalize_required_text(
    value: String,
    empty_error: &'static str,
    long_error: &'static str,
    limit: usize,
) -> Result<String, &'static str> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return Err(empty_error);
    }
    if trimmed.chars().count() > limit {
        return Err(long_error);
    }
    Ok(trimmed.to_string())
}

fn normalize_optional_text(
    value: String,
    long_error: &'static str,
    limit: usize,
) -> Result<String, &'static str> {
    let trimmed = value.trim();
    if trimmed.chars().count() > limit {
        return Err(long_error);
    }
    Ok(trimmed.to_string())
}

#[cfg(test)]
#[path = "project_update_input_unit_tests.rs"]
mod project_update_input_unit_tests;
