use crate::project_naming::{slugify_project_name, title_from_goal};
use captain_kernel::goals::MIN_INTERVAL_SECS;
use serde::Deserialize;

const PROJECT_LAUNCH_GOAL_LIMIT: usize = 2_000;
const PROJECT_LAUNCH_NAME_LIMIT: usize = 160;
const PROJECT_LAUNCH_SLUG_LIMIT: usize = 64;
const PROJECT_LAUNCH_BRANCH_LIMIT: usize = 200;
const PROJECT_LAUNCH_AUTONOMY_LIMIT: usize = 80;
const PROJECT_LAUNCH_CRITERION_LIMIT: usize = 400;
const PROJECT_LAUNCH_CRITERIA_LIMIT: usize = 12;
const PROJECT_LAUNCH_COMMAND_LIMIT: usize = 2_000;
const PROJECT_LAUNCH_GOAL_PREVIEW_LIMIT: usize = 320;

pub(crate) const PROJECT_LAUNCH_GOAL_EMPTY_ERROR: &str = "goal is required";
pub(crate) const PROJECT_LAUNCH_GOAL_LONG_ERROR: &str = "goal is too long";
pub(crate) const PROJECT_LAUNCH_NAME_LONG_ERROR: &str = "name is too long";
pub(crate) const PROJECT_LAUNCH_SLUG_ERROR: &str =
    "slug must be lowercase alphanumeric with hyphens";
pub(crate) const PROJECT_LAUNCH_BRANCH_LONG_ERROR: &str = "branch is too long";
pub(crate) const PROJECT_LAUNCH_AUTONOMY_LONG_ERROR: &str = "autonomy_level is too long";
pub(crate) const PROJECT_LAUNCH_CRITERION_LONG_ERROR: &str = "acceptance criterion is too long";
pub(crate) const PROJECT_LAUNCH_CRITERIA_COUNT_ERROR: &str = "too many acceptance criteria";
pub(crate) const PROJECT_LAUNCH_GOAL_CHECK_COMMAND_LONG_ERROR: &str =
    "goal_check_command is too long";
pub(crate) const PROJECT_LAUNCH_GOAL_RECOVERY_COMMAND_LONG_ERROR: &str =
    "goal_recovery_command is too long";
pub(crate) const PROJECT_LAUNCH_GOAL_CHECK_COMMAND_CRITICAL_ERROR: &str =
    "goal_check_command contains a critical pattern";
pub(crate) const PROJECT_LAUNCH_GOAL_RECOVERY_COMMAND_CRITICAL_ERROR: &str =
    "goal_recovery_command contains a critical pattern";
pub(crate) const PROJECT_LAUNCH_GOAL_RECOVERY_WITHOUT_CHECK_ERROR: &str =
    "goal_recovery_command requires goal_check_command";
pub(crate) const PROJECT_LAUNCH_GOAL_INTERVAL_WITHOUT_CHECK_ERROR: &str =
    "goal_interval_secs requires goal_check_command";
pub(crate) const PROJECT_LAUNCH_GOAL_INTERVAL_LOW_ERROR: &str =
    "goal_interval_secs must be at least 10";

#[derive(Debug, Deserialize)]
pub struct LaunchProjectReq {
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub slug: Option<String>,
    pub goal: String,
    #[serde(default)]
    pub repo_path: Option<String>,
    #[serde(default)]
    pub local_path: Option<String>,
    #[serde(default)]
    pub source_type: Option<String>,
    #[serde(default)]
    pub github_full_name: Option<String>,
    #[serde(default)]
    pub github_clone_url: Option<String>,
    #[serde(default)]
    pub github_branch: Option<String>,
    #[serde(default)]
    pub github_repo_id: Option<serde_json::Value>,
    #[serde(default)]
    pub branch: Option<String>,
    #[serde(default)]
    pub create_worktree: Option<bool>,
    #[serde(default)]
    pub create_folder: Option<bool>,
    #[serde(default)]
    pub autonomy_level: Option<String>,
    #[serde(default)]
    pub acceptance_criteria: Vec<String>,
    #[serde(default)]
    pub deadline: Option<i64>,
    #[serde(default)]
    pub goal_check_command: Option<String>,
    #[serde(default)]
    pub goal_recovery_command: Option<String>,
    #[serde(default)]
    pub goal_interval_secs: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ProjectLaunchGoalGuard {
    pub(crate) check_command: Option<String>,
    pub(crate) recovery_command: Option<String>,
    pub(crate) interval_secs: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct NormalizedProjectLaunch {
    pub(crate) goal: String,
    pub(crate) name: String,
    pub(crate) slug: String,
    pub(crate) criteria: Vec<String>,
    pub(crate) autonomy_level: String,
    pub(crate) goal_guard: ProjectLaunchGoalGuard,
}

pub(crate) fn normalize_project_launch_request(
    req: &mut LaunchProjectReq,
) -> Result<NormalizedProjectLaunch, &'static str> {
    let goal = normalize_project_launch_goal(&req.goal)?;
    let name = normalize_project_launch_name(req.name.as_deref())?
        .unwrap_or_else(|| title_from_goal(&goal));
    let slug = normalize_project_launch_slug(req.slug.as_deref())?
        .unwrap_or_else(|| slugify_project_name(&name));
    let criteria = normalize_project_launch_acceptance_criteria(&req.acceptance_criteria, &goal)?;
    let autonomy_level = normalize_project_launch_autonomy_level(req.autonomy_level.as_deref())?;
    let branch = normalize_project_launch_branch(req.branch.as_deref())?;
    let github_branch = normalize_project_launch_branch(req.github_branch.as_deref())?;
    let goal_guard = normalize_project_launch_goal_guard(
        req.goal_check_command.as_deref(),
        req.goal_recovery_command.as_deref(),
        req.goal_interval_secs,
    )?;

    req.branch = branch;
    req.github_branch = github_branch;
    req.goal_check_command = goal_guard.check_command.clone();
    req.goal_recovery_command = goal_guard.recovery_command.clone();
    req.goal_interval_secs = goal_guard.interval_secs;

    Ok(NormalizedProjectLaunch {
        goal,
        name,
        slug,
        criteria,
        autonomy_level,
        goal_guard,
    })
}

pub(crate) fn normalize_project_launch_goal(goal: &str) -> Result<String, &'static str> {
    normalize_required_text(
        goal,
        PROJECT_LAUNCH_GOAL_EMPTY_ERROR,
        PROJECT_LAUNCH_GOAL_LONG_ERROR,
        PROJECT_LAUNCH_GOAL_LIMIT,
    )
}

pub(crate) fn normalize_project_launch_name(
    name: Option<&str>,
) -> Result<Option<String>, &'static str> {
    normalize_optional_text(
        name,
        PROJECT_LAUNCH_NAME_LONG_ERROR,
        PROJECT_LAUNCH_NAME_LIMIT,
    )
}

pub(crate) fn normalize_project_launch_slug(
    slug: Option<&str>,
) -> Result<Option<String>, &'static str> {
    let Some(slug) =
        normalize_optional_text(slug, PROJECT_LAUNCH_SLUG_ERROR, PROJECT_LAUNCH_SLUG_LIMIT)?
    else {
        return Ok(None);
    };
    if !is_valid_slug(&slug) {
        return Err(PROJECT_LAUNCH_SLUG_ERROR);
    }
    Ok(Some(slug))
}

pub(crate) fn normalize_project_launch_branch(
    branch: Option<&str>,
) -> Result<Option<String>, &'static str> {
    normalize_optional_text(
        branch,
        PROJECT_LAUNCH_BRANCH_LONG_ERROR,
        PROJECT_LAUNCH_BRANCH_LIMIT,
    )
}

pub(crate) fn normalize_project_launch_autonomy_level(
    autonomy_level: Option<&str>,
) -> Result<String, &'static str> {
    Ok(normalize_optional_text(
        autonomy_level,
        PROJECT_LAUNCH_AUTONOMY_LONG_ERROR,
        PROJECT_LAUNCH_AUTONOMY_LIMIT,
    )?
    .unwrap_or_else(|| "default".to_string()))
}

pub(crate) fn normalize_project_launch_acceptance_criteria(
    criteria: &[String],
    goal: &str,
) -> Result<Vec<String>, &'static str> {
    let mut normalized = Vec::new();
    for criterion in criteria {
        let criterion = normalize_optional_text(
            Some(criterion),
            PROJECT_LAUNCH_CRITERION_LONG_ERROR,
            PROJECT_LAUNCH_CRITERION_LIMIT,
        )?;
        if let Some(criterion) = criterion {
            normalized.push(criterion);
        }
        if normalized.len() > PROJECT_LAUNCH_CRITERIA_LIMIT {
            return Err(PROJECT_LAUNCH_CRITERIA_COUNT_ERROR);
        }
    }
    if normalized.is_empty() {
        normalized.push(format!(
            "The project goal is demonstrably satisfied: {}",
            text_preview(goal, PROJECT_LAUNCH_GOAL_PREVIEW_LIMIT)
        ));
        normalized.push(
            "Build, test, or verification commands are recorded with their outcome.".to_string(),
        );
        normalized.push(
            "A handoff checkpoint explains what changed, what remains, and how to resume."
                .to_string(),
        );
    }
    Ok(normalized)
}

pub(crate) fn normalize_project_launch_goal_guard(
    check_command: Option<&str>,
    recovery_command: Option<&str>,
    interval_secs: Option<u64>,
) -> Result<ProjectLaunchGoalGuard, &'static str> {
    let check_command = normalize_optional_text(
        check_command,
        PROJECT_LAUNCH_GOAL_CHECK_COMMAND_LONG_ERROR,
        PROJECT_LAUNCH_COMMAND_LIMIT,
    )?;
    let recovery_command = normalize_optional_text(
        recovery_command,
        PROJECT_LAUNCH_GOAL_RECOVERY_COMMAND_LONG_ERROR,
        PROJECT_LAUNCH_COMMAND_LIMIT,
    )?;

    if check_command.is_none() && recovery_command.is_some() {
        return Err(PROJECT_LAUNCH_GOAL_RECOVERY_WITHOUT_CHECK_ERROR);
    }
    if check_command.is_none() && interval_secs.is_some() {
        return Err(PROJECT_LAUNCH_GOAL_INTERVAL_WITHOUT_CHECK_ERROR);
    }
    if interval_secs
        .map(|interval| interval < MIN_INTERVAL_SECS)
        .unwrap_or(false)
    {
        return Err(PROJECT_LAUNCH_GOAL_INTERVAL_LOW_ERROR);
    }
    if check_command
        .as_deref()
        .and_then(captain_runtime::critical_patterns::is_critical)
        .is_some()
    {
        return Err(PROJECT_LAUNCH_GOAL_CHECK_COMMAND_CRITICAL_ERROR);
    }
    if recovery_command
        .as_deref()
        .and_then(captain_runtime::critical_patterns::is_critical)
        .is_some()
    {
        return Err(PROJECT_LAUNCH_GOAL_RECOVERY_COMMAND_CRITICAL_ERROR);
    }

    Ok(ProjectLaunchGoalGuard {
        check_command,
        recovery_command,
        interval_secs,
    })
}

fn normalize_required_text(
    value: &str,
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
    value: Option<&str>,
    long_error: &'static str,
    limit: usize,
) -> Result<Option<String>, &'static str> {
    let Some(value) = value else {
        return Ok(None);
    };
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return Ok(None);
    }
    if trimmed.chars().count() > limit {
        return Err(long_error);
    }
    Ok(Some(trimmed.to_string()))
}

fn is_valid_slug(slug: &str) -> bool {
    !slug.is_empty()
        && slug.len() <= PROJECT_LAUNCH_SLUG_LIMIT
        && slug
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
}

fn text_preview(value: &str, limit: usize) -> String {
    value.chars().take(limit).collect()
}

#[cfg(test)]
#[path = "project_launch_input_unit_tests.rs"]
mod project_launch_input_unit_tests;
