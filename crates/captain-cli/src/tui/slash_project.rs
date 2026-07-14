#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ProjectSlashAction<'a> {
    OpenProjects,
    Activate(&'a str),
}

pub(crate) fn project_slash_action(args: &str) -> ProjectSlashAction<'_> {
    if args.is_empty() {
        ProjectSlashAction::OpenProjects
    } else {
        ProjectSlashAction::Activate(args)
    }
}

pub(crate) fn project_workspace_active_message(slug: &str) -> String {
    format!("Project workspace active: {slug}")
}

#[cfg(test)]
#[path = "slash_project/tests.rs"]
mod tests;
