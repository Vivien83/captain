use super::*;

#[test]
fn automation_slash_commands_route_to_views() {
    assert_eq!(
        hub_slash_route_for_command("/automation"),
        Some(HubSlashRoute::Automation(AutomationView::Workflows))
    );
    assert_eq!(
        hub_slash_route_for_command("/workflows"),
        Some(HubSlashRoute::Automation(AutomationView::Workflows))
    );
    assert_eq!(
        hub_slash_route_for_command("/triggers"),
        Some(HubSlashRoute::Automation(AutomationView::Triggers))
    );
    assert_eq!(
        hub_slash_route_for_command("/scheduler"),
        Some(HubSlashRoute::Automation(AutomationView::Cron))
    );
    assert_eq!(
        hub_slash_route_for_command("/approvals"),
        Some(HubSlashRoute::Automation(AutomationView::Approvals))
    );
}

#[test]
fn learning_slash_commands_route_to_views() {
    assert_eq!(
        hub_slash_route_for_command("/learning"),
        Some(HubSlashRoute::Learning(LearningView::Review))
    );
    assert_eq!(
        hub_slash_route_for_command("/proposed"),
        Some(HubSlashRoute::Learning(LearningView::SkillProposals))
    );
    assert_eq!(
        hub_slash_route_for_command("/memory"),
        Some(HubSlashRoute::Learning(LearningView::Memory))
    );
    assert_eq!(
        hub_slash_route_for_command("/graph"),
        Some(HubSlashRoute::Learning(LearningView::Graph))
    );
}

#[test]
fn capabilities_slash_commands_route_to_views() {
    assert_eq!(
        hub_slash_route_for_command("/capabilities"),
        Some(HubSlashRoute::Capabilities(CapabilitiesView::Skills))
    );
    assert_eq!(
        hub_slash_route_for_command("/skills"),
        Some(HubSlashRoute::Capabilities(CapabilitiesView::Skills))
    );
    assert_eq!(hub_slash_route_for_command("/hands"), None);
}

#[test]
fn active_channel_slash_command_routes_to_channels_view() {
    assert_eq!(
        hub_slash_route_for_command("/channels"),
        Some(HubSlashRoute::Connections(ConnectionsView::Channels))
    );
    assert_eq!(hub_slash_route_for_command("/connections"), None);
    assert_eq!(hub_slash_route_for_command("/extensions"), None);
    assert_eq!(hub_slash_route_for_command("/peers"), None);
    assert_eq!(hub_slash_route_for_command("/comms"), None);
}

#[test]
fn non_hub_slash_commands_are_ignored() {
    assert_eq!(hub_slash_route_for_command("/project"), None);
    assert_eq!(hub_slash_route_for_command("/budget"), None);
    assert_eq!(hub_slash_route_for_command(""), None);
}
