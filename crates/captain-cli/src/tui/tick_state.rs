use super::navigation_state::{ConnectionsView, Phase, Tab};

const CTRL_C_RESET_TICKS: usize = 40;
const APPROVAL_POLL_INTERVAL_TICKS: usize = 24;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum AutoPollRoute {
    Logs,
    Peers,
    Comms,
    ProjectRuntime,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ScreenTickRoute {
    Welcome,
    Chat,
    Dashboard,
    Channels,
    Workflows,
    Triggers,
    Sessions,
    Memory,
    Skills,
    Hands,
    Extensions,
    Templates,
    Security,
    Audit,
    Usage,
    Settings,
    Peers,
    Comms,
    Logs,
    Projects,
    Learning,
    SkillsProposed,
    Cron,
    Approvals,
    Budget,
    Graph,
}

const SCREEN_TICK_ROUTES: &[ScreenTickRoute] = &[
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
];

pub(crate) fn screen_tick_routes() -> &'static [ScreenTickRoute] {
    SCREEN_TICK_ROUTES
}

pub(crate) fn next_tick_count(current: usize) -> usize {
    current.wrapping_add(1)
}

pub(crate) fn should_clear_ctrl_c_pending(
    ctrl_c_pending: bool,
    tick_count: usize,
    ctrl_c_tick: usize,
) -> bool {
    ctrl_c_pending && tick_count.wrapping_sub(ctrl_c_tick) > CTRL_C_RESET_TICKS
}

pub(crate) fn should_poll_pending_approval(
    is_streaming: bool,
    has_pending_approval: bool,
    tick_count: usize,
) -> bool {
    is_streaming && !has_pending_approval && tick_count.is_multiple_of(APPROVAL_POLL_INTERVAL_TICKS)
}

pub(crate) fn auto_poll_route_for_state(
    phase: Phase,
    active_tab: Tab,
    connections_view: ConnectionsView,
) -> Option<AutoPollRoute> {
    if phase != Phase::Main {
        return None;
    }

    match active_tab {
        Tab::Logs => Some(AutoPollRoute::Logs),
        Tab::Peers => Some(AutoPollRoute::Peers),
        Tab::Comms => Some(AutoPollRoute::Comms),
        Tab::Projects => Some(AutoPollRoute::ProjectRuntime),
        Tab::Channels => match connections_view {
            ConnectionsView::Peers => Some(AutoPollRoute::Peers),
            ConnectionsView::Comms => Some(AutoPollRoute::Comms),
            _ => None,
        },
        _ => None,
    }
}

#[cfg(test)]
#[path = "tick_state/tests.rs"]
mod tests;
