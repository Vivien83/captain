pub(crate) fn safe_project_launch_error(error: &str) -> String {
    let lower = error.to_ascii_lowercase();

    if lower.contains("github_full_name is required") {
        return "github_full_name is required for GitHub projects".to_string();
    }
    if lower.contains("github_full_name must look like owner/repo") {
        return "github_full_name must look like owner/repo".to_string();
    }
    if lower.contains("unknown project source_type") {
        return "project source_type must be local or github".to_string();
    }
    if lower.contains("project folder does not exist") {
        return "project workspace does not exist".to_string();
    }
    if lower.contains("target folder already exists and is not a git repo") {
        return "project workspace already exists and is not a git repository".to_string();
    }
    if lower.contains("failed to create project folder")
        || lower.contains("failed to create project parent folder")
    {
        return "project workspace could not be created".to_string();
    }
    if lower.contains("invalid project folder") || lower.contains("invalid cloned project folder") {
        return "project workspace could not be resolved".to_string();
    }
    if lower.contains("gh repo clone failed")
        || lower.contains("git clone failed")
        || lower.contains("github clone task failed")
    {
        return "GitHub repository could not be cloned; verify repository access, branch, and local workspace permissions".to_string();
    }
    if lower.contains("failed to inspect") {
        return "project workspace could not be inspected".to_string();
    }

    "project launch failed; verify source configuration and workspace permissions".to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn launch_error_redacts_paths_urls_and_external_stderr() {
        let error = "git clone failed: fatal: could not read Username for 'https://ghp_secret@example.test/private/repo.git': terminal prompts disabled; failed to inspect '/Users/example/private'";
        let safe = safe_project_launch_error(error);

        assert_eq!(
            safe,
            "GitHub repository could not be cloned; verify repository access, branch, and local workspace permissions"
        );
        for forbidden in [
            "ghp_secret",
            "example.test",
            "/Users/",
            "private/repo",
            "terminal prompts disabled",
        ] {
            assert!(!safe.contains(forbidden), "leaked {forbidden}");
        }
    }

    #[test]
    fn launch_error_keeps_only_actionable_validation_categories() {
        assert_eq!(
            safe_project_launch_error("unknown project source_type: https://secret.example.test"),
            "project source_type must be local or github"
        );
        assert_eq!(
            safe_project_launch_error("project folder does not exist: /private/path"),
            "project workspace does not exist"
        );
        assert_eq!(
            safe_project_launch_error("github_full_name must look like owner/repo"),
            "github_full_name must look like owner/repo"
        );
    }
}
