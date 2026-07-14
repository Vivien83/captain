use super::*;

#[test]
fn welcome_boot_connects_existing_daemon() {
    assert!(matches!(
        auto_route_action(true, false, true),
        Some(WelcomeAction::ConnectDaemon)
    ));
}

#[test]
fn welcome_boot_starts_inprocess_when_no_daemon() {
    assert!(matches!(
        auto_route_action(true, false, false),
        Some(WelcomeAction::InProcess)
    ));
}

#[test]
fn opt_out_keeps_welcome_menu_visible() {
    assert!(auto_route_action(true, true, true).is_none());
    assert!(auto_route_action(true, true, false).is_none());
}

#[test]
fn non_welcome_phase_does_not_reroute() {
    assert!(auto_route_action(false, false, true).is_none());
    assert!(auto_route_action(false, false, false).is_none());
}
