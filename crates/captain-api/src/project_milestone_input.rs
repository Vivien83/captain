const PROJECT_MILESTONE_ID_LIMIT: usize = 64;

pub(crate) const PROJECT_MILESTONE_ID_ERROR: &str =
    "project milestone id must be alphanumeric, '-' or '_' and 3-64 chars";
pub(crate) const PROJECT_MILESTONE_NOT_FOUND_ERROR: &str = "project milestone not found";

pub(crate) fn normalize_project_milestone_project_id(id: String) -> Result<String, &'static str> {
    normalize_project_milestone_id(id)
}

pub(crate) fn normalize_project_milestone_lookup_id(id: String) -> Result<String, &'static str> {
    normalize_project_milestone_id(id)
}

fn normalize_project_milestone_id(id: String) -> Result<String, &'static str> {
    let id = id.trim();
    let len = id.chars().count();
    if !(3..=PROJECT_MILESTONE_ID_LIMIT).contains(&len)
        || !id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
    {
        return Err(PROJECT_MILESTONE_ID_ERROR);
    }
    Ok(id.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn project_milestone_ids_trim_and_reject_raw_paths_or_tokens() {
        assert_eq!(
            normalize_project_milestone_lookup_id(" milestone-1_2 ".to_string()).unwrap(),
            "milestone-1_2"
        );
        assert_eq!(
            normalize_project_milestone_project_id("/Users/example/private-ghp_secret".to_string()),
            Err(PROJECT_MILESTONE_ID_ERROR)
        );
    }
}
