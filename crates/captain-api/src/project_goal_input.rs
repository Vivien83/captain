const PROJECT_GOAL_ID_LIMIT: usize = 64;
const PROJECT_GOAL_NAME_LIMIT: usize = 180;
const PROJECT_GOAL_DESCRIPTION_LIMIT: usize = 2_000;
const PROJECT_GOAL_COMMAND_LIMIT: usize = 2_000;

pub(crate) const PROJECT_GOAL_ID_ERROR: &str =
    "goal id must be 3..=64 chars and contain only alphanumeric, - or _";
pub(crate) const PROJECT_GOAL_NOT_FOUND_ERROR: &str = "project goal not found";
pub(crate) const PROJECT_GOAL_NAME_EMPTY_ERROR: &str = "name is required";
pub(crate) const PROJECT_GOAL_NAME_LONG_ERROR: &str = "name is too long";
pub(crate) const PROJECT_GOAL_DESCRIPTION_LONG_ERROR: &str = "description is too long";
pub(crate) const PROJECT_GOAL_CHECK_COMMAND_EMPTY_ERROR: &str = "check_command is required";
pub(crate) const PROJECT_GOAL_CHECK_COMMAND_LONG_ERROR: &str = "check_command is too long";
pub(crate) const PROJECT_GOAL_RECOVERY_COMMAND_LONG_ERROR: &str = "recovery_command is too long";

pub(crate) fn normalize_project_goal_create_id(
    id: Option<String>,
) -> Result<Option<String>, &'static str> {
    let Some(id) = normalize_optional_text(id, PROJECT_GOAL_ID_ERROR, PROJECT_GOAL_ID_LIMIT)?
    else {
        return Ok(None);
    };
    if id.len() < 3 || !id.chars().all(is_goal_id_char) {
        return Err(PROJECT_GOAL_ID_ERROR);
    }
    Ok(Some(id))
}

pub(crate) fn normalize_project_goal_lookup_id(id: String) -> Result<String, &'static str> {
    let Some(id) = normalize_optional_text(Some(id), PROJECT_GOAL_ID_ERROR, PROJECT_GOAL_ID_LIMIT)?
    else {
        return Err(PROJECT_GOAL_ID_ERROR);
    };
    if id.len() < 3 || !id.chars().all(is_goal_id_char) {
        return Err(PROJECT_GOAL_ID_ERROR);
    }
    Ok(id)
}

pub(crate) fn normalize_project_goal_create_name(
    name: Option<String>,
) -> Result<Option<String>, &'static str> {
    normalize_optional_text(name, PROJECT_GOAL_NAME_LONG_ERROR, PROJECT_GOAL_NAME_LIMIT)
}

pub(crate) fn normalize_project_goal_update_name(
    name: Option<String>,
) -> Result<Option<String>, &'static str> {
    let Some(name) = name else {
        return Ok(None);
    };
    normalize_required_text(
        name,
        PROJECT_GOAL_NAME_EMPTY_ERROR,
        PROJECT_GOAL_NAME_LONG_ERROR,
        PROJECT_GOAL_NAME_LIMIT,
    )
    .map(Some)
}

pub(crate) fn normalize_project_goal_description(
    description: Option<String>,
) -> Result<Option<String>, &'static str> {
    normalize_optional_text(
        description,
        PROJECT_GOAL_DESCRIPTION_LONG_ERROR,
        PROJECT_GOAL_DESCRIPTION_LIMIT,
    )
}

pub(crate) fn normalize_project_goal_update_description(
    description: Option<String>,
) -> Result<Option<String>, &'static str> {
    let Some(description) = description else {
        return Ok(None);
    };
    let description = normalize_bounded_text(
        description,
        PROJECT_GOAL_DESCRIPTION_LONG_ERROR,
        PROJECT_GOAL_DESCRIPTION_LIMIT,
    )?;
    Ok(Some(description))
}

pub(crate) fn normalize_project_goal_required_check_command(
    check_command: String,
) -> Result<String, &'static str> {
    normalize_required_text(
        check_command,
        PROJECT_GOAL_CHECK_COMMAND_EMPTY_ERROR,
        PROJECT_GOAL_CHECK_COMMAND_LONG_ERROR,
        PROJECT_GOAL_COMMAND_LIMIT,
    )
}

pub(crate) fn normalize_project_goal_update_check_command(
    check_command: Option<String>,
) -> Result<Option<String>, &'static str> {
    let Some(check_command) = check_command else {
        return Ok(None);
    };
    normalize_project_goal_required_check_command(check_command).map(Some)
}

pub(crate) fn normalize_project_goal_recovery_command(
    recovery_command: Option<String>,
) -> Result<Option<String>, &'static str> {
    normalize_optional_text(
        recovery_command,
        PROJECT_GOAL_RECOVERY_COMMAND_LONG_ERROR,
        PROJECT_GOAL_COMMAND_LIMIT,
    )
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
    value: Option<String>,
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

fn normalize_bounded_text(
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

fn is_goal_id_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '-' || c == '_'
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn project_goal_create_fields_trim_bound_and_keep_defaults() {
        assert_eq!(
            normalize_project_goal_create_id(Some(" goal-1 ".to_string())).unwrap(),
            Some("goal-1".to_string())
        );
        assert_eq!(
            normalize_project_goal_lookup_id(" goal-1 ".to_string()).unwrap(),
            "goal-1".to_string()
        );
        assert_eq!(
            normalize_project_goal_create_id(Some("  ".to_string())).unwrap(),
            None
        );
        assert_eq!(
            normalize_project_goal_create_name(Some(" Keep healthy ".to_string())).unwrap(),
            Some("Keep healthy".to_string())
        );
        assert_eq!(
            normalize_project_goal_create_name(Some(" ".to_string())).unwrap(),
            None
        );
        assert_eq!(
            normalize_project_goal_description(Some(" Watch tests ".to_string())).unwrap(),
            Some("Watch tests".to_string())
        );
        assert_eq!(
            normalize_project_goal_update_description(Some(" ".to_string())).unwrap(),
            Some(String::new())
        );
        assert_eq!(
            normalize_project_goal_required_check_command(" cargo test ".to_string()).unwrap(),
            "cargo test"
        );
        assert_eq!(
            normalize_project_goal_recovery_command(Some(" ".to_string())).unwrap(),
            None
        );
    }

    #[test]
    fn project_goal_input_errors_are_static() {
        assert_eq!(
            normalize_project_goal_create_id(Some("/Users/example/private-ghp_secret".to_string())),
            Err(PROJECT_GOAL_ID_ERROR)
        );
        assert_eq!(
            normalize_project_goal_lookup_id("/Users/example/private-ghp_secret".to_string()),
            Err(PROJECT_GOAL_ID_ERROR)
        );
        assert_eq!(
            normalize_project_goal_update_name(Some(" ".to_string())),
            Err(PROJECT_GOAL_NAME_EMPTY_ERROR)
        );
        assert_eq!(
            normalize_project_goal_required_check_command(" ".to_string()),
            Err(PROJECT_GOAL_CHECK_COMMAND_EMPTY_ERROR)
        );
        assert_eq!(
            normalize_project_goal_required_check_command(
                "x".repeat(PROJECT_GOAL_COMMAND_LIMIT + 1)
            ),
            Err(PROJECT_GOAL_CHECK_COMMAND_LONG_ERROR)
        );
        assert_eq!(
            normalize_project_goal_recovery_command(Some(
                "x".repeat(PROJECT_GOAL_COMMAND_LIMIT + 1)
            )),
            Err(PROJECT_GOAL_RECOVERY_COMMAND_LONG_ERROR)
        );
    }
}
