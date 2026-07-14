use super::*;
use serde_json::json;

#[test]
fn github_repository_view_omits_clone_and_browser_urls() {
    let view = github_repository_view(&json!({
        "id": 42,
        "name": "repo",
        "full_name": "owner/repo",
        "private": true,
        "default_branch": "trunk",
        "updated_at": "2026-05-23T12:00:00Z",
        "clone_url": "https://token-secret@example.test/owner/repo.git",
        "ssh_url": "git@example.test:owner/repo.git",
        "git_url": "git://example.test/owner/repo.git",
        "html_url": "https://example.test/owner/repo",
        "permissions": {"admin": true}
    }));

    assert_eq!(view["id"], 42);
    assert_eq!(view["full_name"], "owner/repo");
    assert_eq!(view["default_branch"], "trunk");
    assert!(view.get("clone_url").is_none());
    assert!(view.get("ssh_url").is_none());
    assert!(view.get("git_url").is_none());
    assert!(view.get("html_url").is_none());
    assert!(view.get("permissions").is_none());

    let encoded = serde_json::to_string(&view).unwrap();
    assert!(!encoded.contains("token-secret"));
    assert!(!encoded.contains("example.test"));
}

#[test]
fn github_repositories_view_defaults_missing_branch() {
    let view = github_repositories_view(&json!([{
        "id": {"secret": "id-secret"},
        "name": "repo",
        "full_name": "owner/repo"
    }]));

    assert_eq!(view.len(), 1);
    assert!(view[0]["id"].is_null());
    assert_eq!(view[0]["default_branch"], "main");

    let encoded = serde_json::to_string(&view).unwrap();
    assert!(!encoded.contains("id-secret"));
}

#[test]
fn github_user_view_omits_profile_url_and_raw_payloads() {
    let view = github_user_view(&json!({
        "login": "octo",
        "id": {"secret": "id-secret"},
        "name": "Private Name",
        "email": "private@example.test",
        "html_url": "https://example.test/octo",
        "avatar_url": "https://example.test/avatar.png",
        "plan": {"name": "enterprise-secret"}
    }));

    assert_eq!(view["login"], "octo");
    assert!(view["id"].is_null());
    assert!(view.get("name").is_none());
    assert!(view.get("email").is_none());
    assert!(view.get("html_url").is_none());
    assert!(view.get("avatar_url").is_none());
    assert!(view.get("plan").is_none());

    let encoded = serde_json::to_string(&view).unwrap();
    for forbidden in [
        "Private Name",
        "private@example.test",
        "example.test",
        "enterprise-secret",
        "id-secret",
    ] {
        assert!(!encoded.contains(forbidden), "leaked {forbidden}");
    }
}

#[test]
fn github_repositories_error_view_omits_raw_body() {
    let raw_body = json!({
        "message": "Bad credentials",
        "documentation_url": "https://docs.example.test/private",
        "token_hint": "token-secret"
    })
    .to_string();
    let view = github_repositories_error_view("401 Unauthorized");

    assert_eq!(view["authenticated"], true);
    assert_eq!(view["repositories"].as_array().unwrap().len(), 0);
    assert!(view.get("detail").is_none());

    let encoded = serde_json::to_string(&view).unwrap();
    assert!(!encoded.contains(&raw_body));
    assert!(!encoded.contains("token-secret"));
    assert!(!encoded.contains("docs.example.test"));
}

#[test]
fn github_repositories_transport_error_view_omits_raw_network_error() {
    let view = github_repositories_transport_error_view(
        "GitHub request failed: error sending request for url (https://ghp_secret@example.test/repos): dns failed",
    );

    assert_eq!(
        view["error"],
        "GitHub repositories request failed; verify network access and token configuration"
    );
    assert_eq!(view["authenticated"], true);
    assert_eq!(view["repositories"].as_array().unwrap().len(), 0);

    let encoded = serde_json::to_string(&view).unwrap();
    for forbidden in [
        "ghp_secret",
        "example.test",
        "dns failed",
        "request for url",
    ] {
        assert!(!encoded.contains(forbidden), "leaked {forbidden}");
    }
}

#[test]
fn github_status_error_view_omits_raw_network_error() {
    let view = github_status_error_view(
        "GitHub request failed: error sending request for url (https://ghp_secret@example.test/user): dns failed",
    );

    assert_eq!(view["configured"], true);
    assert_eq!(view["authenticated"], false);
    assert_eq!(view["token_env"], "GITHUB_TOKEN");
    assert_eq!(
        view["error"],
        "GitHub authentication check failed; verify network access and token configuration"
    );

    let encoded = serde_json::to_string(&view).unwrap();
    for forbidden in [
        "ghp_secret",
        "example.test",
        "dns failed",
        "request for url",
    ] {
        assert!(!encoded.contains(forbidden), "leaked {forbidden}");
    }
}

#[test]
fn github_status_error_view_keeps_bounded_status_category() {
    let view = github_status_error_view("GitHub returned 401 Unauthorized");

    assert_eq!(view["error"], "GitHub returned 401 Unauthorized");
    assert!(view.get("detail").is_none());
}

#[test]
fn github_token_validation_error_view_omits_raw_network_error() {
    let view = github_token_validation_error_view(
        "GitHub request failed: error sending request for url (https://ghp_secret@example.test/user): dns failed",
    );

    assert_eq!(
        view["error"],
        "GitHub token validation failed; verify network access and token configuration"
    );

    let encoded = serde_json::to_string(&view).unwrap();
    for forbidden in [
        "ghp_secret",
        "example.test",
        "dns failed",
        "request for url",
    ] {
        assert!(!encoded.contains(forbidden), "leaked {forbidden}");
    }
}

#[test]
fn github_token_validation_error_view_keeps_bounded_status_category() {
    let view = github_token_validation_error_view("GitHub returned 401 Unauthorized");

    assert_eq!(
        view["error"],
        "GitHub token validation failed: GitHub returned 401 Unauthorized"
    );
}

#[test]
fn github_token_persist_error_view_omits_secret_storage_detail() {
    let view = github_token_persist_error_view(
        "failed to write temp secrets.env: permission denied at /Users/example/.captain/secrets.env with ghp_secret",
    );

    assert_eq!(
        view["error"],
        "GitHub token could not be saved; verify Captain secrets storage permissions"
    );

    let encoded = serde_json::to_string(&view).unwrap();
    for forbidden in [
        "/Users/example",
        "secrets.env",
        "ghp_secret",
        "permission denied",
    ] {
        assert!(!encoded.contains(forbidden), "leaked {forbidden}");
    }
}

#[test]
fn github_token_remove_error_view_omits_secret_storage_detail() {
    let view = github_token_remove_error_view(
        "failed to commit secrets.env: permission denied at /Users/example/.captain/secrets.env with ghp_secret",
    );

    assert_eq!(
        view["error"],
        "GitHub token could not be removed; verify Captain secrets storage permissions"
    );

    let encoded = serde_json::to_string(&view).unwrap();
    for forbidden in [
        "/Users/example",
        "secrets.env",
        "ghp_secret",
        "permission denied",
    ] {
        assert!(!encoded.contains(forbidden), "leaked {forbidden}");
    }
}

#[test]
fn github_full_name_normalization_rejects_urls_and_traversal() {
    assert_eq!(
        normalize_github_full_name(" owner/repo ").unwrap(),
        "owner/repo"
    );

    for invalid in [
        "https://github.com/owner/repo",
        "token@example.test/owner/repo",
        "owner/repo/extra",
        "owner/../repo",
        "owner/repo?token=secret",
        "/repo",
        "owner/",
    ] {
        assert!(
            normalize_github_full_name(invalid).is_err(),
            "accepted {invalid}"
        );
    }
}

#[test]
fn github_source_metadata_omits_clone_url_and_bounds_repo_id() {
    let long_repo_id = format!("{}SECRET_TAIL", "r".repeat(200));
    let source = github_source_metadata(
        "owner/repo",
        Some(&json!(long_repo_id)),
        Some("main"),
        "/private/project-path",
    );

    assert_eq!(source["type"], "github");
    assert_eq!(source["full_name"], "owner/repo");
    assert_eq!(source["branch"], "main");
    assert_eq!(source["repo_id"].as_str().unwrap().chars().count(), 120);
    assert!(source.get("clone_url").is_none());
    assert!(source.get("html_url").is_none());
    assert!(source.get("ssh_url").is_none());
    assert!(source.get("git_url").is_none());

    let encoded = serde_json::to_string(&source).unwrap();
    assert!(!encoded.contains("SECRET_TAIL"));
}
