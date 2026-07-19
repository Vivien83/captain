use crate::tui::{AutomationView, CapabilitiesView, ConnectionsView, LearningView, Tab};

use super::*;

#[test]
fn automation_errors_follow_active_subview() {
    assert_eq!(
        fetch_error_target(
            Tab::Workflows,
            AutomationView::Workflows,
            ConnectionsView::Channels,
            LearningView::Review,
            CapabilitiesView::Skills,
        ),
        Some(FetchErrorTarget::Workflows)
    );
    assert_eq!(
        fetch_error_target(
            Tab::Workflows,
            AutomationView::Cron,
            ConnectionsView::Channels,
            LearningView::Review,
            CapabilitiesView::Skills,
        ),
        Some(FetchErrorTarget::Cron)
    );
}

#[test]
fn connections_errors_follow_active_subview() {
    assert_eq!(
        fetch_error_target(
            Tab::Channels,
            AutomationView::Workflows,
            ConnectionsView::Peers,
            LearningView::Review,
            CapabilitiesView::Skills,
        ),
        Some(FetchErrorTarget::PeersLoading)
    );
    assert_eq!(
        fetch_error_target(
            Tab::Channels,
            AutomationView::Workflows,
            ConnectionsView::Comms,
            LearningView::Review,
            CapabilitiesView::Skills,
        ),
        Some(FetchErrorTarget::Comms)
    );
}

#[test]
fn learning_errors_follow_active_subview() {
    assert_eq!(
        fetch_error_target(
            Tab::Learning,
            AutomationView::Workflows,
            ConnectionsView::Channels,
            LearningView::SkillProposals,
            CapabilitiesView::Skills,
        ),
        Some(FetchErrorTarget::SkillsProposed)
    );
    assert_eq!(
        fetch_error_target(
            Tab::Learning,
            AutomationView::Workflows,
            ConnectionsView::Channels,
            LearningView::Graph,
            CapabilitiesView::Skills,
        ),
        Some(FetchErrorTarget::Graph)
    );
}

#[test]
fn capability_errors_follow_active_subview() {
    assert_eq!(
        fetch_error_target(
            Tab::Skills,
            AutomationView::Workflows,
            ConnectionsView::Channels,
            LearningView::Review,
            CapabilitiesView::Native,
        ),
        Some(FetchErrorTarget::NativeCapabilities)
    );
}

#[test]
fn direct_status_tabs_route_fetch_errors() {
    assert_eq!(
        fetch_error_target(
            Tab::Sessions,
            AutomationView::Workflows,
            ConnectionsView::Channels,
            LearningView::Review,
            CapabilitiesView::Skills,
        ),
        Some(FetchErrorTarget::Sessions)
    );
    assert_eq!(
        fetch_error_target(
            Tab::Templates,
            AutomationView::Workflows,
            ConnectionsView::Channels,
            LearningView::Review,
            CapabilitiesView::Skills,
        ),
        Some(FetchErrorTarget::Templates)
    );
    assert_eq!(
        fetch_error_target(
            Tab::Settings,
            AutomationView::Workflows,
            ConnectionsView::Channels,
            LearningView::Review,
            CapabilitiesView::Skills,
        ),
        Some(FetchErrorTarget::Settings)
    );
}

#[test]
fn non_status_tabs_ignore_fetch_errors() {
    assert_eq!(
        fetch_error_target(
            Tab::Chat,
            AutomationView::Workflows,
            ConnectionsView::Channels,
            LearningView::Review,
            CapabilitiesView::Skills,
        ),
        None
    );
}
