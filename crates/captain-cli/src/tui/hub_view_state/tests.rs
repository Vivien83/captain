use super::*;

#[test]
fn switch_views_refresh_current_hub() {
    assert_eq!(
        automation_view_state_after_switch(AutomationView::Cron),
        HubViewState {
            view: AutomationView::Cron,
            effect: HubViewEffect::RefreshAutomationCurrent,
        }
    );
    assert_eq!(
        learning_view_state_after_switch(LearningView::Graph),
        HubViewState {
            view: LearningView::Graph,
            effect: HubViewEffect::RefreshLearningCurrent,
        }
    );
    assert_eq!(
        capabilities_view_state_after_switch(CapabilitiesView::Hands),
        HubViewState {
            view: CapabilitiesView::Hands,
            effect: HubViewEffect::RefreshCapabilitiesCurrent,
        }
    );
    assert_eq!(
        connections_view_state_after_switch(ConnectionsView::Comms),
        HubViewState {
            view: ConnectionsView::Comms,
            effect: HubViewEffect::RefreshConnectionsCurrent,
        }
    );
}

#[test]
fn open_views_switch_to_the_family_root_tab() {
    assert_eq!(
        automation_view_state_after_open(AutomationView::Approvals),
        HubViewState {
            view: AutomationView::Approvals,
            effect: HubViewEffect::SwitchTab(Tab::Workflows),
        }
    );
    assert_eq!(
        learning_view_state_after_open(LearningView::SkillProposals),
        HubViewState {
            view: LearningView::SkillProposals,
            effect: HubViewEffect::SwitchTab(Tab::Learning),
        }
    );
    assert_eq!(
        capabilities_view_state_after_open(CapabilitiesView::Hands),
        HubViewState {
            view: CapabilitiesView::Hands,
            effect: HubViewEffect::SwitchTab(Tab::Skills),
        }
    );
    assert_eq!(
        connections_view_state_after_open(ConnectionsView::Peers),
        HubViewState {
            view: ConnectionsView::Peers,
            effect: HubViewEffect::SwitchTab(Tab::Channels),
        }
    );
}
