use super::*;

#[test]
fn projects_environment_view_omits_local_workspace_paths() {
    let view = projects_environment_view(true);

    assert_eq!(view["default_source_type"], "local");
    assert_eq!(view["local_default_available"], true);
    assert_eq!(view["github_authenticated"], true);
    assert!(view.get("workspaces_dir").is_none());
    assert!(view.get("project_root").is_none());

    let encoded = serde_json::to_string(&view).unwrap();
    assert!(!encoded.contains("/Users/"));
    assert!(!encoded.contains("/private/"));
    assert!(!encoded.contains("workspaces"));
}
