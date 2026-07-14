use super::screens::welcome::WelcomeAction;

pub(crate) fn auto_route_action(
    is_welcome_boot: bool,
    opt_out: bool,
    daemon_detected: bool,
) -> Option<WelcomeAction> {
    if !is_welcome_boot || opt_out {
        return None;
    }

    if daemon_detected {
        Some(WelcomeAction::ConnectDaemon)
    } else {
        Some(WelcomeAction::InProcess)
    }
}

#[cfg(test)]
#[path = "welcome_route/tests.rs"]
mod tests;
