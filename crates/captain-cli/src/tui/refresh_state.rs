use super::navigation_state::{
    AutomationView, CapabilitiesView, ConnectionsView, LearningView, Tab,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum TabRefreshRoute {
    Dashboard,
    Agents,
    Projects,
    ConnectionsCurrent,
    AutomationCurrent,
    Triggers,
    Cron,
    Approvals,
    Budget,
    Graph,
    Sessions,
    Memory,
    LearningCurrent,
    SkillsProposed,
    CapabilitiesCurrent,
    Hands,
    Extensions,
    Templates,
    Security,
    Audit,
    Usage,
    SettingsProviders,
    Peers,
    Comms,
    Logs,
}

pub(crate) fn tab_refresh_route_for_tab(tab: Tab) -> Option<TabRefreshRoute> {
    match tab {
        Tab::Dashboard => Some(TabRefreshRoute::Dashboard),
        Tab::Agents => Some(TabRefreshRoute::Agents),
        Tab::Projects => Some(TabRefreshRoute::Projects),
        Tab::Channels => Some(TabRefreshRoute::ConnectionsCurrent),
        Tab::Workflows => Some(TabRefreshRoute::AutomationCurrent),
        Tab::Triggers => Some(TabRefreshRoute::Triggers),
        Tab::Cron => Some(TabRefreshRoute::Cron),
        Tab::Approvals => Some(TabRefreshRoute::Approvals),
        Tab::Budget => Some(TabRefreshRoute::Budget),
        Tab::Graph => Some(TabRefreshRoute::Graph),
        Tab::Sessions => Some(TabRefreshRoute::Sessions),
        Tab::Memory => Some(TabRefreshRoute::Memory),
        Tab::Learning => Some(TabRefreshRoute::LearningCurrent),
        Tab::SkillsProposed => Some(TabRefreshRoute::SkillsProposed),
        Tab::Skills => Some(TabRefreshRoute::CapabilitiesCurrent),
        Tab::Hands => Some(TabRefreshRoute::Hands),
        Tab::Extensions => Some(TabRefreshRoute::Extensions),
        Tab::Templates => Some(TabRefreshRoute::Templates),
        Tab::Security => Some(TabRefreshRoute::Security),
        Tab::Audit => Some(TabRefreshRoute::Audit),
        Tab::Usage => Some(TabRefreshRoute::Usage),
        Tab::Settings => Some(TabRefreshRoute::SettingsProviders),
        Tab::Peers => Some(TabRefreshRoute::Peers),
        Tab::Comms => Some(TabRefreshRoute::Comms),
        Tab::Logs => Some(TabRefreshRoute::Logs),
        Tab::Chat => None,
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum AutomationRefreshRoute {
    Workflows,
    Triggers,
    Cron,
    Approvals,
}

pub(crate) fn automation_refresh_route_for_view(view: AutomationView) -> AutomationRefreshRoute {
    match view {
        AutomationView::Workflows => AutomationRefreshRoute::Workflows,
        AutomationView::Triggers => AutomationRefreshRoute::Triggers,
        AutomationView::Cron => AutomationRefreshRoute::Cron,
        AutomationView::Approvals => AutomationRefreshRoute::Approvals,
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum LearningRefreshRoute {
    Review,
    SkillProposals,
    Memory,
    Graph,
}

pub(crate) fn learning_refresh_route_for_view(view: LearningView) -> LearningRefreshRoute {
    match view {
        LearningView::Review => LearningRefreshRoute::Review,
        LearningView::SkillProposals => LearningRefreshRoute::SkillProposals,
        LearningView::Memory => LearningRefreshRoute::Memory,
        LearningView::Graph => LearningRefreshRoute::Graph,
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum CapabilitiesRefreshRoute {
    Native,
    Skills,
}

pub(crate) fn capabilities_refresh_route_for_view(
    view: CapabilitiesView,
) -> CapabilitiesRefreshRoute {
    match view {
        CapabilitiesView::Native => CapabilitiesRefreshRoute::Native,
        CapabilitiesView::Skills => CapabilitiesRefreshRoute::Skills,
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ConnectionsRefreshRoute {
    Channels,
    Extensions,
    Peers,
    Comms,
}

pub(crate) fn connections_refresh_route_for_view(view: ConnectionsView) -> ConnectionsRefreshRoute {
    match view {
        ConnectionsView::Channels => ConnectionsRefreshRoute::Channels,
        ConnectionsView::Extensions => ConnectionsRefreshRoute::Extensions,
        ConnectionsView::Peers => ConnectionsRefreshRoute::Peers,
        ConnectionsView::Comms => ConnectionsRefreshRoute::Comms,
    }
}

#[cfg(test)]
#[path = "refresh_state/tests.rs"]
mod tests;
