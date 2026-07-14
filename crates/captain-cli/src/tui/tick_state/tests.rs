use super::super::navigation_state::BootScreen;
use super::*;

#[test]
fn next_tick_count_wraps() {
    assert_eq!(next_tick_count(0), 1);
    assert_eq!(next_tick_count(usize::MAX), 0);
}

#[test]
fn ctrl_c_pending_clears_only_after_threshold() {
    assert!(!should_clear_ctrl_c_pending(false, 100, 0));
    assert!(!should_clear_ctrl_c_pending(true, 140, 100));
    assert!(should_clear_ctrl_c_pending(true, 141, 100));
}

#[test]
fn ctrl_c_pending_uses_wrapping_delta() {
    assert!(should_clear_ctrl_c_pending(true, 5, usize::MAX - 40));
}

#[test]
fn pending_approval_poll_requires_streaming_without_modal_and_interval() {
    assert!(should_poll_pending_approval(true, false, 24));
    assert!(!should_poll_pending_approval(false, false, 24));
    assert!(!should_poll_pending_approval(true, true, 24));
    assert!(!should_poll_pending_approval(true, false, 23));
}

#[test]
fn auto_poll_ignores_boot_phase() {
    assert_eq!(
        auto_poll_route_for_state(
            Phase::Boot(BootScreen::Welcome),
            Tab::Logs,
            ConnectionsView::Channels
        ),
        None
    );
}

#[test]
fn auto_poll_routes_primary_tabs() {
    assert_eq!(
        auto_poll_route_for_state(Phase::Main, Tab::Logs, ConnectionsView::Channels),
        Some(AutoPollRoute::Logs)
    );
    assert_eq!(
        auto_poll_route_for_state(Phase::Main, Tab::Peers, ConnectionsView::Channels),
        Some(AutoPollRoute::Peers)
    );
    assert_eq!(
        auto_poll_route_for_state(Phase::Main, Tab::Comms, ConnectionsView::Channels),
        Some(AutoPollRoute::Comms)
    );
    assert_eq!(
        auto_poll_route_for_state(Phase::Main, Tab::Projects, ConnectionsView::Channels),
        Some(AutoPollRoute::ProjectRuntime)
    );
}

#[test]
fn auto_poll_routes_connections_subviews_only_for_polling_views() {
    assert_eq!(
        auto_poll_route_for_state(Phase::Main, Tab::Channels, ConnectionsView::Peers),
        Some(AutoPollRoute::Peers)
    );
    assert_eq!(
        auto_poll_route_for_state(Phase::Main, Tab::Channels, ConnectionsView::Comms),
        Some(AutoPollRoute::Comms)
    );
    assert_eq!(
        auto_poll_route_for_state(Phase::Main, Tab::Channels, ConnectionsView::Channels),
        None
    );
    assert_eq!(
        auto_poll_route_for_state(Phase::Main, Tab::Channels, ConnectionsView::Extensions),
        None
    );
}

#[test]
fn auto_poll_ignores_non_polling_tabs() {
    assert_eq!(
        auto_poll_route_for_state(Phase::Main, Tab::Chat, ConnectionsView::Channels),
        None
    );
}

#[test]
fn screen_tick_routes_keep_hermes_order() {
    assert_eq!(
        screen_tick_routes(),
        &[
            ScreenTickRoute::Welcome,
            ScreenTickRoute::Chat,
            ScreenTickRoute::Dashboard,
            ScreenTickRoute::Channels,
            ScreenTickRoute::Workflows,
            ScreenTickRoute::Triggers,
            ScreenTickRoute::Sessions,
            ScreenTickRoute::Memory,
            ScreenTickRoute::Skills,
            ScreenTickRoute::Hands,
            ScreenTickRoute::Extensions,
            ScreenTickRoute::Templates,
            ScreenTickRoute::Security,
            ScreenTickRoute::Audit,
            ScreenTickRoute::Usage,
            ScreenTickRoute::Settings,
            ScreenTickRoute::Peers,
            ScreenTickRoute::Comms,
            ScreenTickRoute::Logs,
            ScreenTickRoute::Projects,
            ScreenTickRoute::Learning,
            ScreenTickRoute::SkillsProposed,
            ScreenTickRoute::Cron,
            ScreenTickRoute::Approvals,
            ScreenTickRoute::Budget,
            ScreenTickRoute::Graph,
        ]
    );
}
