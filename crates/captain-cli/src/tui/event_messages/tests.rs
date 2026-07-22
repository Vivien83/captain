use super::*;

#[test]
fn memory_stored_line_preserves_hermes_shape() {
    let line = memory_stored_line("project", "status", "ready", "agent");

    assert_eq!(line, "🧠 mémorisé · project/status = \"ready\"   (agent)");
}

#[test]
fn memory_queued_line_truncates_long_object() {
    let object = "a".repeat(81);
    let line = memory_queued_line("review-123", "subject", "predicate", &object, "tool");

    assert!(line.contains(&format!("\"{}…\"", "a".repeat(78))));
    assert!(line.contains("(tool, review-123)"));
}

#[test]
fn skill_proposal_line_marks_archive_and_points_to_learning() {
    let description = "b".repeat(101);
    let line = skill_proposal_line(
        "proposal-abcdef",
        "deploy-helper",
        &description,
        "",
        0.873,
        Some("platform-devops"),
    );

    assert!(line.contains(&format!("— {}…", "b".repeat(98))));
    assert!(line.contains(" · famille: devops   (87%, proposal)"));
    assert!(line.starts_with("skill archivé v3.13"));
    assert!(line.ends_with(" · consulte Learning"));
    assert!(!line.contains("/skills-proposed"));
}

#[test]
fn skill_proposal_line_uses_default_family_and_hint() {
    let line = skill_proposal_line(
        "123456789",
        "api-helper",
        "Calls an API safely",
        "when API calls repeat",
        0.42,
        None,
    );

    assert!(line.contains(" · quand : "));
    assert!(line.contains(" · famille: automatisation   (42%, 12345678)"));
}

#[test]
fn agent_lifecycle_line_reports_termination_reason() {
    let line = agent_lifecycle_line("terminated", "researcher-hand", Some("killed"));
    assert!(line.contains("researcher-hand"));
    assert!(line.contains("terminé"));
    assert!(line.contains("(killed)"));
}

#[test]
fn agent_lifecycle_line_reports_crash_error() {
    let line = agent_lifecycle_line("crashed", "researcher-hand", Some("unresponsive for 90s"));
    assert!(line.contains("planté"));
    assert!(line.contains("(unresponsive for 90s)"));
}

#[test]
fn agent_lifecycle_line_handles_missing_detail() {
    let line = agent_lifecycle_line("terminated", "researcher-hand", None);
    assert!(!line.contains('('));
}
