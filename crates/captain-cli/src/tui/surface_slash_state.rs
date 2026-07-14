use super::navigation_state::Tab;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum SurfaceSlashRoute {
    SwitchTab(Tab),
    OpenOverlay(Tab),
}

pub(crate) fn surface_slash_route_for_command(command: &str) -> Option<SurfaceSlashRoute> {
    match command {
        "/home" | "/dashboard" => Some(SurfaceSlashRoute::SwitchTab(Tab::Dashboard)),
        "/projects" => Some(SurfaceSlashRoute::SwitchTab(Tab::Projects)),
        "/budget" => Some(SurfaceSlashRoute::OpenOverlay(Tab::Budget)),
        "/logs" => Some(SurfaceSlashRoute::OpenOverlay(Tab::Logs)),
        "/settings" => Some(SurfaceSlashRoute::OpenOverlay(Tab::Settings)),
        _ => None,
    }
}

#[cfg(test)]
#[path = "surface_slash_state/tests.rs"]
mod tests;
