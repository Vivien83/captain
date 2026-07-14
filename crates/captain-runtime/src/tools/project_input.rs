const PROJECT_TOOL_ID_LIMIT: usize = 64;
const PROJECT_TOOL_SLUG_LIMIT: usize = 64;
const PROJECT_TOOL_NAME_LIMIT: usize = 160;
const PROJECT_TOOL_GOAL_LIMIT: usize = 2_000;
const PROJECT_TOOL_TASK_TITLE_LIMIT: usize = 180;
const PROJECT_TOOL_TASK_DESCRIPTION_LIMIT: usize = 2_000;
const PROJECT_TOOL_MILESTONE_NAME_LIMIT: usize = 180;
const PROJECT_TOOL_CHECKPOINT_SUMMARY_LIMIT: usize = 600;

pub(crate) const PROJECT_TOOL_ID_ERROR: &str =
    "project tool id must be alphanumeric, '-' or '_' and 3-64 chars";
pub(crate) const PROJECT_TOOL_SLUG_ERROR: &str =
    "project tool slug must be lowercase alphanumeric with hyphens";
pub(crate) const PROJECT_TOOL_STATUS_ERROR: &str =
    "project task status must be one of todo, doing, blocked, review, done, cancelled";
pub(crate) const PROJECT_TOOL_NAME_EMPTY_ERROR: &str = "project name cannot be empty";
pub(crate) const PROJECT_TOOL_NAME_LONG_ERROR: &str = "project name is too long";
pub(crate) const PROJECT_TOOL_GOAL_LONG_ERROR: &str = "project goal is too long";
pub(crate) const PROJECT_TOOL_TASK_TITLE_EMPTY_ERROR: &str = "task title cannot be empty";
pub(crate) const PROJECT_TOOL_TASK_TITLE_LONG_ERROR: &str = "task title is too long";
pub(crate) const PROJECT_TOOL_TASK_DESCRIPTION_LONG_ERROR: &str = "task description is too long";
pub(crate) const PROJECT_TOOL_MILESTONE_NAME_EMPTY_ERROR: &str = "milestone name cannot be empty";
pub(crate) const PROJECT_TOOL_MILESTONE_NAME_LONG_ERROR: &str = "milestone name is too long";
pub(crate) const PROJECT_TOOL_CHECKPOINT_SUMMARY_EMPTY_ERROR: &str =
    "checkpoint summary cannot be empty";
pub(crate) const PROJECT_TOOL_CHECKPOINT_SUMMARY_LONG_ERROR: &str =
    "checkpoint summary is too long";

pub(crate) fn normalize_project_tool_id(id: &str) -> Result<String, &'static str> {
    let id = id.trim();
    let len = id.chars().count();
    if !(3..=PROJECT_TOOL_ID_LIMIT).contains(&len)
        || !id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
    {
        return Err(PROJECT_TOOL_ID_ERROR);
    }
    Ok(id.to_string())
}

pub(crate) fn normalize_project_tool_optional_id(
    id: Option<&str>,
) -> Result<Option<String>, &'static str> {
    id.map(normalize_project_tool_id).transpose()
}

pub(crate) fn normalize_project_tool_slug(slug: &str) -> Result<String, &'static str> {
    let slug = slug.trim();
    if slug.is_empty()
        || slug.chars().count() > PROJECT_TOOL_SLUG_LIMIT
        || !slug
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
    {
        return Err(PROJECT_TOOL_SLUG_ERROR);
    }
    Ok(slug.to_string())
}

pub(crate) fn normalize_project_tool_name(name: &str) -> Result<String, &'static str> {
    normalize_required_text(
        name,
        PROJECT_TOOL_NAME_EMPTY_ERROR,
        PROJECT_TOOL_NAME_LONG_ERROR,
        PROJECT_TOOL_NAME_LIMIT,
    )
}

pub(crate) fn normalize_project_tool_goal(goal: &str) -> Result<String, &'static str> {
    normalize_optional_text(goal, PROJECT_TOOL_GOAL_LONG_ERROR, PROJECT_TOOL_GOAL_LIMIT)
}

pub(crate) fn normalize_project_tool_task_title(title: &str) -> Result<String, &'static str> {
    normalize_required_text(
        title,
        PROJECT_TOOL_TASK_TITLE_EMPTY_ERROR,
        PROJECT_TOOL_TASK_TITLE_LONG_ERROR,
        PROJECT_TOOL_TASK_TITLE_LIMIT,
    )
}

pub(crate) fn normalize_project_tool_task_description(
    description: &str,
) -> Result<String, &'static str> {
    normalize_optional_text(
        description,
        PROJECT_TOOL_TASK_DESCRIPTION_LONG_ERROR,
        PROJECT_TOOL_TASK_DESCRIPTION_LIMIT,
    )
}

pub(crate) fn normalize_project_tool_task_status(status: &str) -> Result<String, &'static str> {
    let status = status.trim();
    match status {
        "todo" | "doing" | "blocked" | "review" | "done" | "cancelled" => Ok(status.to_string()),
        _ => Err(PROJECT_TOOL_STATUS_ERROR),
    }
}

pub(crate) fn normalize_project_tool_milestone_name(name: &str) -> Result<String, &'static str> {
    normalize_required_text(
        name,
        PROJECT_TOOL_MILESTONE_NAME_EMPTY_ERROR,
        PROJECT_TOOL_MILESTONE_NAME_LONG_ERROR,
        PROJECT_TOOL_MILESTONE_NAME_LIMIT,
    )
}

pub(crate) fn normalize_project_tool_checkpoint_summary(
    summary: &str,
) -> Result<String, &'static str> {
    normalize_required_text(
        summary,
        PROJECT_TOOL_CHECKPOINT_SUMMARY_EMPTY_ERROR,
        PROJECT_TOOL_CHECKPOINT_SUMMARY_LONG_ERROR,
        PROJECT_TOOL_CHECKPOINT_SUMMARY_LIMIT,
    )
}

fn normalize_required_text(
    value: &str,
    empty_error: &'static str,
    long_error: &'static str,
    limit: usize,
) -> Result<String, &'static str> {
    let value = value.trim();
    if value.is_empty() {
        return Err(empty_error);
    }
    if value.chars().count() > limit {
        return Err(long_error);
    }
    Ok(value.to_string())
}

fn normalize_optional_text(
    value: &str,
    long_error: &'static str,
    limit: usize,
) -> Result<String, &'static str> {
    let value = value.trim();
    if value.chars().count() > limit {
        return Err(long_error);
    }
    Ok(value.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn project_tool_ids_and_slugs_trim_and_reject_raw_paths() {
        assert_eq!(
            normalize_project_tool_id(" project-1_2 ").unwrap(),
            "project-1_2"
        );
        assert_eq!(
            normalize_project_tool_slug(" demo-project ").unwrap(),
            "demo-project"
        );
        assert_eq!(
            normalize_project_tool_id("../private/leaky-fragment"),
            Err(PROJECT_TOOL_ID_ERROR)
        );
        assert_eq!(
            normalize_project_tool_slug("bad-../private/leaky-fragment"),
            Err(PROJECT_TOOL_SLUG_ERROR)
        );
    }

    #[test]
    fn project_tool_text_and_status_are_bounded() {
        assert_eq!(normalize_project_tool_name(" Demo ").unwrap(), "Demo");
        assert_eq!(
            normalize_project_tool_task_title("  "),
            Err(PROJECT_TOOL_TASK_TITLE_EMPTY_ERROR)
        );
        assert_eq!(
            normalize_project_tool_task_status("rm -rf ../private/leaky-fragment"),
            Err(PROJECT_TOOL_STATUS_ERROR)
        );
        assert_eq!(
            normalize_project_tool_checkpoint_summary(
                &"x".repeat(PROJECT_TOOL_CHECKPOINT_SUMMARY_LIMIT + 1)
            ),
            Err(PROJECT_TOOL_CHECKPOINT_SUMMARY_LONG_ERROR)
        );
    }
}
