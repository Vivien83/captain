use super::*;

#[test]
fn tab_surface_slash_commands_switch_tabs() {
    assert_eq!(
        surface_slash_route_for_command("/home"),
        Some(SurfaceSlashRoute::SwitchTab(Tab::Dashboard))
    );
    assert_eq!(
        surface_slash_route_for_command("/dashboard"),
        Some(SurfaceSlashRoute::SwitchTab(Tab::Dashboard))
    );
    assert_eq!(
        surface_slash_route_for_command("/projects"),
        Some(SurfaceSlashRoute::SwitchTab(Tab::Projects))
    );
}

#[test]
fn overlay_surface_slash_commands_open_overlays() {
    assert_eq!(
        surface_slash_route_for_command("/budget"),
        Some(SurfaceSlashRoute::OpenOverlay(Tab::Budget))
    );
    assert_eq!(
        surface_slash_route_for_command("/logs"),
        Some(SurfaceSlashRoute::OpenOverlay(Tab::Logs))
    );
    assert_eq!(
        surface_slash_route_for_command("/settings"),
        Some(SurfaceSlashRoute::OpenOverlay(Tab::Settings))
    );
}

#[test]
fn non_surface_slash_commands_are_ignored() {
    assert_eq!(surface_slash_route_for_command("/project"), None);
    assert_eq!(surface_slash_route_for_command("/automation"), None);
    assert_eq!(surface_slash_route_for_command(""), None);
}
