use super::*;

#[test]
fn empty_args_open_projects_tab() {
    assert_eq!(project_slash_action(""), ProjectSlashAction::OpenProjects);
}

#[test]
fn non_empty_args_activate_project_slug_verbatim() {
    assert_eq!(
        project_slash_action("captain-core"),
        ProjectSlashAction::Activate("captain-core")
    );
}

#[test]
fn active_workspace_message_keeps_existing_text() {
    assert_eq!(
        project_workspace_active_message("demo"),
        "Project workspace active: demo"
    );
}
