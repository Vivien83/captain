use super::{AutomationView, CapabilitiesView, ConnectionsView, LearningView, Tab};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum FetchErrorTarget {
    Workflows,
    Triggers,
    Cron,
    Approvals,
    Projects,
    Channels,
    Extensions,
    PeersLoading,
    Comms,
    Sessions,
    Learning,
    SkillsProposed,
    Memory,
    Graph,
    NativeCapabilities,
    Skills,
    Hands,
    Templates,
    Settings,
}

pub(crate) fn fetch_error_target(
    active_tab: Tab,
    automation_view: AutomationView,
    connections_view: ConnectionsView,
    learning_view: LearningView,
    capabilities_view: CapabilitiesView,
) -> Option<FetchErrorTarget> {
    match active_tab {
        Tab::Workflows => match automation_view {
            AutomationView::Workflows => Some(FetchErrorTarget::Workflows),
            AutomationView::Triggers => Some(FetchErrorTarget::Triggers),
            AutomationView::Cron => Some(FetchErrorTarget::Cron),
            AutomationView::Approvals => Some(FetchErrorTarget::Approvals),
        },
        Tab::Projects => Some(FetchErrorTarget::Projects),
        Tab::Triggers => Some(FetchErrorTarget::Triggers),
        Tab::Channels => match connections_view {
            ConnectionsView::Channels => Some(FetchErrorTarget::Channels),
            ConnectionsView::Extensions => Some(FetchErrorTarget::Extensions),
            ConnectionsView::Peers => Some(FetchErrorTarget::PeersLoading),
            ConnectionsView::Comms => Some(FetchErrorTarget::Comms),
        },
        Tab::Sessions => Some(FetchErrorTarget::Sessions),
        Tab::Learning => match learning_view {
            LearningView::Review => Some(FetchErrorTarget::Learning),
            LearningView::SkillProposals => Some(FetchErrorTarget::SkillsProposed),
            LearningView::Memory => Some(FetchErrorTarget::Memory),
            LearningView::Graph => Some(FetchErrorTarget::Graph),
        },
        Tab::Memory => Some(FetchErrorTarget::Memory),
        Tab::Skills => match capabilities_view {
            CapabilitiesView::Native => Some(FetchErrorTarget::NativeCapabilities),
            CapabilitiesView::Skills => Some(FetchErrorTarget::Skills),
        },
        Tab::Hands => Some(FetchErrorTarget::Hands),
        Tab::Extensions => Some(FetchErrorTarget::Extensions),
        Tab::Templates => Some(FetchErrorTarget::Templates),
        Tab::Settings => Some(FetchErrorTarget::Settings),
        _ => None,
    }
}

#[cfg(test)]
#[path = "error_route/tests.rs"]
mod tests;
