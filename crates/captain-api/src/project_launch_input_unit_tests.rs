use super::*;

fn req() -> LaunchProjectReq {
    LaunchProjectReq {
        name: Some(" Demo Project ".to_string()),
        slug: Some(" demo-project ".to_string()),
        goal: " Ship safely ".to_string(),
        repo_path: None,
        local_path: None,
        source_type: None,
        github_full_name: None,
        github_clone_url: None,
        github_branch: Some(" main ".to_string()),
        github_repo_id: None,
        branch: Some(" dev ".to_string()),
        create_worktree: None,
        create_folder: None,
        autonomy_level: Some(" supervised ".to_string()),
        acceptance_criteria: vec![" Done ".to_string(), " ".to_string()],
        deadline: None,
        goal_check_command: Some(" true ".to_string()),
        goal_recovery_command: Some(" echo recover ".to_string()),
        goal_interval_secs: Some(60),
    }
}

#[test]
fn project_launch_request_normalizes_fields_and_rewrites_route_input() {
    let mut req = req();

    let normalized = normalize_project_launch_request(&mut req).unwrap();

    assert_eq!(normalized.goal, "Ship safely");
    assert_eq!(normalized.name, "Demo Project");
    assert_eq!(normalized.slug, "demo-project");
    assert_eq!(normalized.criteria, vec!["Done"]);
    assert_eq!(normalized.autonomy_level, "supervised");
    assert_eq!(
        normalized.goal_guard,
        ProjectLaunchGoalGuard {
            check_command: Some("true".to_string()),
            recovery_command: Some("echo recover".to_string()),
            interval_secs: Some(60),
        }
    );
    assert_eq!(req.branch.as_deref(), Some("dev"));
    assert_eq!(req.github_branch.as_deref(), Some("main"));
    assert_eq!(req.goal_check_command.as_deref(), Some("true"));
    assert_eq!(req.goal_recovery_command.as_deref(), Some("echo recover"));
}

#[test]
fn project_launch_request_defaults_name_slug_criteria_and_autonomy() {
    let mut req = req();
    req.name = None;
    req.slug = None;
    req.autonomy_level = None;
    req.acceptance_criteria.clear();

    let normalized = normalize_project_launch_request(&mut req).unwrap();

    assert_eq!(normalized.name, "Ship safely");
    assert_eq!(normalized.slug, "ship-safely");
    assert_eq!(normalized.autonomy_level, "default");
    assert_eq!(normalized.criteria.len(), 3);
    assert!(normalized.criteria[0].contains("Ship safely"));
}

#[test]
fn project_launch_text_fields_trim_bound_and_default() {
    assert_eq!(
        normalize_project_launch_goal(" Ship safely ").unwrap(),
        "Ship safely"
    );
    assert_eq!(
        normalize_project_launch_name(Some(" Demo Project ")).unwrap(),
        Some("Demo Project".to_string())
    );
    assert_eq!(normalize_project_launch_name(Some(" ")).unwrap(), None);
    assert_eq!(
        normalize_project_launch_slug(Some(" demo-project ")).unwrap(),
        Some("demo-project".to_string())
    );
    assert_eq!(
        normalize_project_launch_branch(Some(" main ")).unwrap(),
        Some("main".to_string())
    );
    assert_eq!(
        normalize_project_launch_autonomy_level(Some(" supervised ")).unwrap(),
        "supervised"
    );
    assert_eq!(
        normalize_project_launch_autonomy_level(None).unwrap(),
        "default"
    );
}

#[test]
fn project_launch_acceptance_criteria_are_bounded() {
    assert_eq!(
        normalize_project_launch_acceptance_criteria(&[" Done ".to_string()], "Goal").unwrap(),
        vec!["Done".to_string()]
    );
    assert_eq!(
        normalize_project_launch_acceptance_criteria(&[], "Goal")
            .unwrap()
            .len(),
        3
    );
    assert_eq!(
        normalize_project_launch_acceptance_criteria(
            &["x".repeat(PROJECT_LAUNCH_CRITERION_LIMIT + 1)],
            "Goal",
        ),
        Err(PROJECT_LAUNCH_CRITERION_LONG_ERROR)
    );
    assert_eq!(
        normalize_project_launch_acceptance_criteria(
            &vec!["item".to_string(); PROJECT_LAUNCH_CRITERIA_LIMIT + 1],
            "Goal",
        ),
        Err(PROJECT_LAUNCH_CRITERIA_COUNT_ERROR)
    );
}

#[test]
fn project_launch_goal_guard_rejects_dangling_or_unsafe_config() {
    assert_eq!(
        normalize_project_launch_goal_guard(Some(" true "), Some(" echo recover "), Some(60))
            .unwrap(),
        ProjectLaunchGoalGuard {
            check_command: Some("true".to_string()),
            recovery_command: Some("echo recover".to_string()),
            interval_secs: Some(60),
        }
    );
    assert_eq!(
        normalize_project_launch_goal_guard(None, Some("echo recover"), None),
        Err(PROJECT_LAUNCH_GOAL_RECOVERY_WITHOUT_CHECK_ERROR)
    );
    assert_eq!(
        normalize_project_launch_goal_guard(None, None, Some(60)),
        Err(PROJECT_LAUNCH_GOAL_INTERVAL_WITHOUT_CHECK_ERROR)
    );
    assert_eq!(
        normalize_project_launch_goal_guard(Some("true"), None, Some(1)),
        Err(PROJECT_LAUNCH_GOAL_INTERVAL_LOW_ERROR)
    );
    assert_eq!(
        normalize_project_launch_goal_guard(Some("rm -rf /"), None, Some(60)),
        Err(PROJECT_LAUNCH_GOAL_CHECK_COMMAND_CRITICAL_ERROR)
    );
}
