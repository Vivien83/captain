const PROJECT_CHECKPOINT_PROJECT_ID_LIMIT: usize = 64;
const DEFAULT_CHECKPOINT_HISTORY_LIMIT: usize = 20;
const MAX_CHECKPOINT_HISTORY_LIMIT: usize = 100;

pub(crate) const PROJECT_CHECKPOINT_PROJECT_ID_ERROR: &str =
    "project checkpoint project id must be alphanumeric, '-' or '_' and 3-64 chars";
pub(crate) const PROJECT_CHECKPOINT_LIMIT_ERROR: &str =
    "checkpoint limit must be an integer between 1 and 100";

pub(crate) fn normalize_project_checkpoint_project_id(id: String) -> Result<String, &'static str> {
    let id = id.trim();
    let len = id.chars().count();
    if !(3..=PROJECT_CHECKPOINT_PROJECT_ID_LIMIT).contains(&len)
        || !id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
    {
        return Err(PROJECT_CHECKPOINT_PROJECT_ID_ERROR);
    }
    Ok(id.to_string())
}

pub(crate) fn normalize_project_checkpoint_limit(
    limit: Option<&String>,
) -> Result<usize, &'static str> {
    let Some(limit) = limit else {
        return Ok(DEFAULT_CHECKPOINT_HISTORY_LIMIT);
    };
    let limit = limit
        .trim()
        .parse::<usize>()
        .map_err(|_| PROJECT_CHECKPOINT_LIMIT_ERROR)?;
    if !(1..=MAX_CHECKPOINT_HISTORY_LIMIT).contains(&limit) {
        return Err(PROJECT_CHECKPOINT_LIMIT_ERROR);
    }
    Ok(limit)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn checkpoint_project_ids_trim_and_reject_paths_or_tokens() {
        assert_eq!(
            normalize_project_checkpoint_project_id(" project-1_2 ".to_string()).unwrap(),
            "project-1_2"
        );
        assert_eq!(
            normalize_project_checkpoint_project_id(
                "/Users/example/private-ghp_secret".to_string()
            ),
            Err(PROJECT_CHECKPOINT_PROJECT_ID_ERROR)
        );
    }

    #[test]
    fn checkpoint_limit_defaults_and_rejects_invalid_values() {
        assert_eq!(normalize_project_checkpoint_limit(None).unwrap(), 20);
        assert_eq!(
            normalize_project_checkpoint_limit(Some(&" 100 ".to_string())).unwrap(),
            100
        );
        assert_eq!(
            normalize_project_checkpoint_limit(Some(&"0".to_string())),
            Err(PROJECT_CHECKPOINT_LIMIT_ERROR)
        );
        assert_eq!(
            normalize_project_checkpoint_limit(Some(&"101".to_string())),
            Err(PROJECT_CHECKPOINT_LIMIT_ERROR)
        );
        assert_eq!(
            normalize_project_checkpoint_limit(Some(&"bad-/Users/example".to_string())),
            Err(PROJECT_CHECKPOINT_LIMIT_ERROR)
        );
    }
}
