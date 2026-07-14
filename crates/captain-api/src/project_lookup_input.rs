const PROJECT_LOOKUP_KEY_LIMIT: usize = 120;

pub(crate) const PROJECT_LOOKUP_KEY_ERROR: &str = "project identifier is invalid";
pub(crate) const PROJECT_LOOKUP_NOT_FOUND_ERROR: &str = "project not found";

pub(crate) fn normalize_project_lookup_key(raw: &str) -> Result<String, &'static str> {
    let key = raw.trim();
    if key.is_empty()
        || key.len() > PROJECT_LOOKUP_KEY_LIMIT
        || !key
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
    {
        return Err(PROJECT_LOOKUP_KEY_ERROR);
    }
    Ok(key.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn project_lookup_key_trims_valid_slug_or_id() {
        assert_eq!(
            normalize_project_lookup_key(" demo-project ").unwrap(),
            "demo-project"
        );
        assert_eq!(
            normalize_project_lookup_key(" 123e4567-e89b-12d3-a456-426614174000 ").unwrap(),
            "123e4567-e89b-12d3-a456-426614174000"
        );
    }

    #[test]
    fn project_lookup_key_rejects_paths_tokens_and_huge_values() {
        let huge = "x".repeat(PROJECT_LOOKUP_KEY_LIMIT + 1);
        for raw in [
            "",
            "bad-/Users/example/private",
            "bad-ghp_secret",
            "UPPERCASE",
            huge.as_str(),
        ] {
            assert_eq!(
                normalize_project_lookup_key(raw),
                Err(PROJECT_LOOKUP_KEY_ERROR)
            );
        }
    }
}

#[cfg(test)]
#[path = "project_lookup_input_tests.rs"]
mod project_lookup_input_tests;
