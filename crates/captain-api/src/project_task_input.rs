const PROJECT_TASK_ID_LIMIT: usize = 64;

pub(crate) const PROJECT_TASK_ID_ERROR: &str =
    "project task id must be alphanumeric, '-' or '_' and 3-64 chars";
pub(crate) const PROJECT_TASK_PROJECT_ID_ERROR: &str =
    "project task project id must be alphanumeric, '-' or '_' and 3-64 chars";
pub(crate) const PROJECT_TASK_NOT_FOUND_ERROR: &str = "project task not found";

pub(crate) fn normalize_project_task_project_id(id: String) -> Result<String, &'static str> {
    normalize_project_task_id(id, PROJECT_TASK_PROJECT_ID_ERROR)
}

pub(crate) fn normalize_project_task_lookup_id(id: String) -> Result<String, &'static str> {
    normalize_project_task_id(id, PROJECT_TASK_ID_ERROR)
}

pub(crate) fn normalize_project_task_parent_id(
    parent_id: Option<String>,
) -> Result<Option<String>, &'static str> {
    parent_id
        .map(|id| normalize_project_task_id(id, PROJECT_TASK_ID_ERROR))
        .transpose()
}

pub(crate) fn normalize_project_task_update_parent_id(
    parent_id: Option<Option<String>>,
) -> Result<Option<Option<String>>, &'static str> {
    match parent_id {
        None => Ok(None),
        Some(None) => Ok(Some(None)),
        Some(Some(id)) => {
            normalize_project_task_id(id, PROJECT_TASK_ID_ERROR).map(|id| Some(Some(id)))
        }
    }
}

fn normalize_project_task_id(id: String, error: &'static str) -> Result<String, &'static str> {
    let id = id.trim();
    let len = id.chars().count();
    if !(3..=PROJECT_TASK_ID_LIMIT).contains(&len)
        || !id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
    {
        return Err(error);
    }
    Ok(id.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn project_task_ids_trim_and_reject_raw_paths_or_tokens() {
        assert_eq!(
            normalize_project_task_lookup_id(" task-1_2 ".to_string()).unwrap(),
            "task-1_2"
        );
        assert_eq!(
            normalize_project_task_project_id(" project-1_2 ".to_string()).unwrap(),
            "project-1_2"
        );
        assert_eq!(
            normalize_project_task_lookup_id("/Users/example/private-ghp_secret".to_string()),
            Err(PROJECT_TASK_ID_ERROR)
        );
        assert_eq!(
            normalize_project_task_project_id("/Users/example/private-ghp_secret".to_string()),
            Err(PROJECT_TASK_PROJECT_ID_ERROR)
        );
    }

    #[test]
    fn project_task_parent_ids_preserve_explicit_clear() {
        assert_eq!(
            normalize_project_task_parent_id(Some(" parent-1 ".to_string())).unwrap(),
            Some("parent-1".to_string())
        );
        assert_eq!(normalize_project_task_parent_id(None).unwrap(), None);
        assert_eq!(normalize_project_task_update_parent_id(None).unwrap(), None);
        assert_eq!(
            normalize_project_task_update_parent_id(Some(None)).unwrap(),
            Some(None)
        );
        assert_eq!(
            normalize_project_task_update_parent_id(Some(Some("bad/path".to_string()))),
            Err(PROJECT_TASK_ID_ERROR)
        );
    }
}
