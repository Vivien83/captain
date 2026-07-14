pub(crate) fn safe_project_storage_error(error: &str) -> String {
    let lower = error.to_ascii_lowercase();
    if lower.contains("not found") {
        return "Project data could not be found".to_string();
    }
    if lower.contains("locked") || lower.contains("busy") {
        return "Project storage is busy; retry shortly".to_string();
    }
    if lower.contains("permission") || lower.contains("readonly") || lower.contains("read-only") {
        return "Project storage is not writable; verify Captain data permissions".to_string();
    }
    "Project storage operation failed; verify Captain data availability".to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn storage_error_omits_paths_sql_and_tokens() {
        let safe = safe_project_storage_error(
            "sqlite failed at /Users/example/.captain/data/projects.db with ghp_secret",
        );

        assert_eq!(
            safe,
            "Project storage operation failed; verify Captain data availability"
        );
        for forbidden in [
            "sqlite",
            "/Users/example",
            "projects.db",
            "ghp_secret",
            ".captain",
        ] {
            assert!(!safe.contains(forbidden), "leaked {forbidden}");
        }
    }

    #[test]
    fn storage_error_keeps_actionable_categories() {
        assert_eq!(
            safe_project_storage_error("database is locked"),
            "Project storage is busy; retry shortly"
        );
        assert_eq!(
            safe_project_storage_error("readonly database"),
            "Project storage is not writable; verify Captain data permissions"
        );
    }
}
