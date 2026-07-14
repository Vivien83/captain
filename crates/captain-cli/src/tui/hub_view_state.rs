use super::navigation_state::{
    AutomationView, CapabilitiesView, ConnectionsView, LearningView, Tab,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum HubViewEffect {
    RefreshAutomationCurrent,
    RefreshLearningCurrent,
    RefreshCapabilitiesCurrent,
    RefreshConnectionsCurrent,
    SwitchTab(Tab),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct HubViewState<V> {
    pub(crate) view: V,
    pub(crate) effect: HubViewEffect,
}

pub(crate) fn automation_view_state_after_switch(
    view: AutomationView,
) -> HubViewState<AutomationView> {
    HubViewState {
        view,
        effect: HubViewEffect::RefreshAutomationCurrent,
    }
}

pub(crate) fn automation_view_state_after_open(
    view: AutomationView,
) -> HubViewState<AutomationView> {
    HubViewState {
        view,
        effect: HubViewEffect::SwitchTab(Tab::Workflows),
    }
}

pub(crate) fn learning_view_state_after_switch(view: LearningView) -> HubViewState<LearningView> {
    HubViewState {
        view,
        effect: HubViewEffect::RefreshLearningCurrent,
    }
}

pub(crate) fn learning_view_state_after_open(view: LearningView) -> HubViewState<LearningView> {
    HubViewState {
        view,
        effect: HubViewEffect::SwitchTab(Tab::Learning),
    }
}

pub(crate) fn capabilities_view_state_after_switch(
    view: CapabilitiesView,
) -> HubViewState<CapabilitiesView> {
    HubViewState {
        view,
        effect: HubViewEffect::RefreshCapabilitiesCurrent,
    }
}

pub(crate) fn capabilities_view_state_after_open(
    view: CapabilitiesView,
) -> HubViewState<CapabilitiesView> {
    HubViewState {
        view,
        effect: HubViewEffect::SwitchTab(Tab::Skills),
    }
}

pub(crate) fn connections_view_state_after_switch(
    view: ConnectionsView,
) -> HubViewState<ConnectionsView> {
    HubViewState {
        view,
        effect: HubViewEffect::RefreshConnectionsCurrent,
    }
}

pub(crate) fn connections_view_state_after_open(
    view: ConnectionsView,
) -> HubViewState<ConnectionsView> {
    HubViewState {
        view,
        effect: HubViewEffect::SwitchTab(Tab::Channels),
    }
}

#[cfg(test)]
#[path = "hub_view_state/tests.rs"]
mod tests;
