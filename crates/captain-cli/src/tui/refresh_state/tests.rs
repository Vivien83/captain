use super::*;
use crate::tui::navigation_state::{
    AutomationView, CapabilitiesView, ConnectionsView, LearningView, Tab,
};

#[test]
fn chat_tab_has_no_refresh_route() {
    assert_eq!(tab_refresh_route_for_tab(Tab::Chat), None);
}

#[test]
fn primary_hub_tabs_route_to_current_view_refreshes() {
    assert_eq!(
        tab_refresh_route_for_tab(Tab::Workflows),
        Some(TabRefreshRoute::AutomationCurrent)
    );
    assert_eq!(
        tab_refresh_route_for_tab(Tab::Learning),
        Some(TabRefreshRoute::LearningCurrent)
    );
    assert_eq!(
        tab_refresh_route_for_tab(Tab::Skills),
        Some(TabRefreshRoute::CapabilitiesCurrent)
    );
    assert_eq!(
        tab_refresh_route_for_tab(Tab::Channels),
        Some(TabRefreshRoute::ConnectionsCurrent)
    );
}

#[test]
fn direct_tabs_route_to_their_own_refreshes() {
    let cases = [
        (Tab::Dashboard, TabRefreshRoute::Dashboard),
        (Tab::Agents, TabRefreshRoute::Agents),
        (Tab::Projects, TabRefreshRoute::Projects),
        (Tab::Triggers, TabRefreshRoute::Triggers),
        (Tab::Cron, TabRefreshRoute::Cron),
        (Tab::Approvals, TabRefreshRoute::Approvals),
        (Tab::Budget, TabRefreshRoute::Budget),
        (Tab::Graph, TabRefreshRoute::Graph),
        (Tab::Sessions, TabRefreshRoute::Sessions),
        (Tab::Memory, TabRefreshRoute::Memory),
        (Tab::SkillsProposed, TabRefreshRoute::SkillsProposed),
        (Tab::Hands, TabRefreshRoute::Hands),
        (Tab::Extensions, TabRefreshRoute::Extensions),
        (Tab::Templates, TabRefreshRoute::Templates),
        (Tab::Security, TabRefreshRoute::Security),
        (Tab::Audit, TabRefreshRoute::Audit),
        (Tab::Usage, TabRefreshRoute::Usage),
        (Tab::Settings, TabRefreshRoute::SettingsProviders),
        (Tab::Peers, TabRefreshRoute::Peers),
        (Tab::Comms, TabRefreshRoute::Comms),
        (Tab::Logs, TabRefreshRoute::Logs),
    ];

    for (tab, route) in cases {
        assert_eq!(tab_refresh_route_for_tab(tab), Some(route));
    }
}

#[test]
fn automation_view_refresh_routes_match_hub_views() {
    assert_eq!(
        automation_refresh_route_for_view(AutomationView::Workflows),
        AutomationRefreshRoute::Workflows
    );
    assert_eq!(
        automation_refresh_route_for_view(AutomationView::Triggers),
        AutomationRefreshRoute::Triggers
    );
    assert_eq!(
        automation_refresh_route_for_view(AutomationView::Cron),
        AutomationRefreshRoute::Cron
    );
    assert_eq!(
        automation_refresh_route_for_view(AutomationView::Approvals),
        AutomationRefreshRoute::Approvals
    );
}

#[test]
fn learning_view_refresh_routes_match_hub_views() {
    assert_eq!(
        learning_refresh_route_for_view(LearningView::Review),
        LearningRefreshRoute::Review
    );
    assert_eq!(
        learning_refresh_route_for_view(LearningView::SkillProposals),
        LearningRefreshRoute::SkillProposals
    );
    assert_eq!(
        learning_refresh_route_for_view(LearningView::Memory),
        LearningRefreshRoute::Memory
    );
    assert_eq!(
        learning_refresh_route_for_view(LearningView::Graph),
        LearningRefreshRoute::Graph
    );
}

#[test]
fn capabilities_view_refresh_routes_match_hub_views() {
    assert_eq!(
        capabilities_refresh_route_for_view(CapabilitiesView::Skills),
        CapabilitiesRefreshRoute::Skills
    );
    assert_eq!(
        capabilities_refresh_route_for_view(CapabilitiesView::Hands),
        CapabilitiesRefreshRoute::Hands
    );
}

#[test]
fn connections_view_refresh_routes_match_hub_views() {
    assert_eq!(
        connections_refresh_route_for_view(ConnectionsView::Channels),
        ConnectionsRefreshRoute::Channels
    );
    assert_eq!(
        connections_refresh_route_for_view(ConnectionsView::Extensions),
        ConnectionsRefreshRoute::Extensions
    );
    assert_eq!(
        connections_refresh_route_for_view(ConnectionsView::Peers),
        ConnectionsRefreshRoute::Peers
    );
    assert_eq!(
        connections_refresh_route_for_view(ConnectionsView::Comms),
        ConnectionsRefreshRoute::Comms
    );
}
