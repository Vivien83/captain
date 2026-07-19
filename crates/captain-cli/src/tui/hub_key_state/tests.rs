use super::*;
use crate::tui::hub_nav;
use crate::tui::navigation_state::{
    AutomationView, CapabilitiesView, ConnectionsView, LearningView,
};

#[test]
fn automation_view_key_routes_match_hub_views() {
    assert_eq!(
        automation_key_route_for_view(AutomationView::Workflows),
        AutomationKeyRoute::Workflows
    );
    assert_eq!(
        automation_key_route_for_view(AutomationView::Triggers),
        AutomationKeyRoute::Triggers
    );
    assert_eq!(
        automation_key_route_for_view(AutomationView::Cron),
        AutomationKeyRoute::Cron
    );
    assert_eq!(
        automation_key_route_for_view(AutomationView::Approvals),
        AutomationKeyRoute::Approvals
    );
}

#[test]
fn learning_view_key_routes_match_hub_views() {
    assert_eq!(
        learning_key_route_for_view(LearningView::Review),
        LearningKeyRoute::Review
    );
    assert_eq!(
        learning_key_route_for_view(LearningView::SkillProposals),
        LearningKeyRoute::SkillProposals
    );
    assert_eq!(
        learning_key_route_for_view(LearningView::Memory),
        LearningKeyRoute::Memory
    );
    assert_eq!(
        learning_key_route_for_view(LearningView::Graph),
        LearningKeyRoute::Graph
    );
}

#[test]
fn capabilities_view_key_routes_match_hub_views() {
    assert_eq!(
        capabilities_key_route_for_view(CapabilitiesView::Native),
        CapabilitiesKeyRoute::Native
    );
    assert_eq!(
        capabilities_key_route_for_view(CapabilitiesView::Skills),
        CapabilitiesKeyRoute::Skills
    );
}

#[test]
fn connections_view_key_routes_match_hub_views() {
    assert_eq!(
        connections_key_route_for_view(ConnectionsView::Channels),
        ConnectionsKeyRoute::Channels
    );
    assert_eq!(
        connections_key_route_for_view(ConnectionsView::Extensions),
        ConnectionsKeyRoute::Extensions
    );
    assert_eq!(
        connections_key_route_for_view(ConnectionsView::Peers),
        ConnectionsKeyRoute::Peers
    );
    assert_eq!(
        connections_key_route_for_view(ConnectionsView::Comms),
        ConnectionsKeyRoute::Comms
    );
}

#[test]
fn automation_shortcut_selects_wrapped_or_indexed_view() {
    assert!(matches!(
        automation_view_after_shortcut(AutomationView::Workflows, hub_nav::ShortcutAction::Prev),
        Some(AutomationView::Approvals)
    ));
    assert!(matches!(
        automation_view_after_shortcut(AutomationView::Cron, hub_nav::ShortcutAction::Next),
        Some(AutomationView::Approvals)
    ));
    assert!(matches!(
        automation_view_after_shortcut(
            AutomationView::Workflows,
            hub_nav::ShortcutAction::Index(1)
        ),
        Some(AutomationView::Triggers)
    ));
    assert!(automation_view_after_shortcut(
        AutomationView::Workflows,
        hub_nav::ShortcutAction::Index(9)
    )
    .is_none());
}

#[test]
fn learning_shortcut_selects_wrapped_or_indexed_view() {
    assert!(matches!(
        learning_view_after_shortcut(LearningView::Review, hub_nav::ShortcutAction::Prev),
        Some(LearningView::Graph)
    ));
    assert!(matches!(
        learning_view_after_shortcut(LearningView::Graph, hub_nav::ShortcutAction::Next),
        Some(LearningView::Review)
    ));
    assert!(matches!(
        learning_view_after_shortcut(LearningView::Review, hub_nav::ShortcutAction::Index(2)),
        Some(LearningView::Memory)
    ));
    assert!(
        learning_view_after_shortcut(LearningView::Review, hub_nav::ShortcutAction::Index(9))
            .is_none()
    );
}

#[test]
fn capabilities_shortcut_selects_wrapped_or_indexed_view() {
    assert!(matches!(
        capabilities_view_after_shortcut(CapabilitiesView::Native, hub_nav::ShortcutAction::Prev),
        Some(CapabilitiesView::Skills)
    ));
    assert!(matches!(
        capabilities_view_after_shortcut(CapabilitiesView::Skills, hub_nav::ShortcutAction::Next),
        Some(CapabilitiesView::Native)
    ));
    assert!(matches!(
        capabilities_view_after_shortcut(
            CapabilitiesView::Native,
            hub_nav::ShortcutAction::Index(1)
        ),
        Some(CapabilitiesView::Skills)
    ));
    assert!(capabilities_view_after_shortcut(
        CapabilitiesView::Native,
        hub_nav::ShortcutAction::Index(9)
    )
    .is_none());
}

#[test]
fn connections_shortcut_selects_wrapped_or_indexed_view() {
    assert!(matches!(
        connections_view_after_shortcut(ConnectionsView::Channels, hub_nav::ShortcutAction::Prev),
        Some(ConnectionsView::Comms)
    ));
    assert!(matches!(
        connections_view_after_shortcut(ConnectionsView::Comms, hub_nav::ShortcutAction::Next),
        Some(ConnectionsView::Channels)
    ));
    assert!(matches!(
        connections_view_after_shortcut(
            ConnectionsView::Channels,
            hub_nav::ShortcutAction::Index(2)
        ),
        Some(ConnectionsView::Peers)
    ));
    assert!(connections_view_after_shortcut(
        ConnectionsView::Channels,
        hub_nav::ShortcutAction::Index(9)
    )
    .is_none());
}
