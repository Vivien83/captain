use super::{
    hub_nav,
    navigation_state::{
        AutomationView, CapabilitiesView, ConnectionsView, LearningView, AUTOMATION_VIEWS,
        CAPABILITIES_VIEWS, CONNECTIONS_VIEWS, LEARNING_VIEWS,
    },
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum AutomationKeyRoute {
    Workflows,
    Triggers,
    Cron,
    Approvals,
}

pub(crate) fn automation_key_route_for_view(view: AutomationView) -> AutomationKeyRoute {
    match view {
        AutomationView::Workflows => AutomationKeyRoute::Workflows,
        AutomationView::Triggers => AutomationKeyRoute::Triggers,
        AutomationView::Cron => AutomationKeyRoute::Cron,
        AutomationView::Approvals => AutomationKeyRoute::Approvals,
    }
}

pub(crate) fn automation_view_after_shortcut(
    current: AutomationView,
    action: hub_nav::ShortcutAction,
) -> Option<AutomationView> {
    view_after_shortcut(AUTOMATION_VIEWS, current, action)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum LearningKeyRoute {
    Review,
    SkillProposals,
    Memory,
    Graph,
}

pub(crate) fn learning_key_route_for_view(view: LearningView) -> LearningKeyRoute {
    match view {
        LearningView::Review => LearningKeyRoute::Review,
        LearningView::SkillProposals => LearningKeyRoute::SkillProposals,
        LearningView::Memory => LearningKeyRoute::Memory,
        LearningView::Graph => LearningKeyRoute::Graph,
    }
}

pub(crate) fn learning_view_after_shortcut(
    current: LearningView,
    action: hub_nav::ShortcutAction,
) -> Option<LearningView> {
    view_after_shortcut(LEARNING_VIEWS, current, action)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum CapabilitiesKeyRoute {
    Skills,
    Hands,
}

pub(crate) fn capabilities_key_route_for_view(view: CapabilitiesView) -> CapabilitiesKeyRoute {
    match view {
        CapabilitiesView::Skills => CapabilitiesKeyRoute::Skills,
        CapabilitiesView::Hands => CapabilitiesKeyRoute::Hands,
    }
}

pub(crate) fn capabilities_view_after_shortcut(
    current: CapabilitiesView,
    action: hub_nav::ShortcutAction,
) -> Option<CapabilitiesView> {
    view_after_shortcut(CAPABILITIES_VIEWS, current, action)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ConnectionsKeyRoute {
    Channels,
    Extensions,
    Peers,
    Comms,
}

pub(crate) fn connections_key_route_for_view(view: ConnectionsView) -> ConnectionsKeyRoute {
    match view {
        ConnectionsView::Channels => ConnectionsKeyRoute::Channels,
        ConnectionsView::Extensions => ConnectionsKeyRoute::Extensions,
        ConnectionsView::Peers => ConnectionsKeyRoute::Peers,
        ConnectionsView::Comms => ConnectionsKeyRoute::Comms,
    }
}

pub(crate) fn connections_view_after_shortcut(
    current: ConnectionsView,
    action: hub_nav::ShortcutAction,
) -> Option<ConnectionsView> {
    view_after_shortcut(CONNECTIONS_VIEWS, current, action)
}

fn view_after_shortcut<T: Copy + PartialEq>(
    items: &[T],
    current: T,
    action: hub_nav::ShortcutAction,
) -> Option<T> {
    if items.is_empty() {
        return None;
    }

    match action {
        hub_nav::ShortcutAction::Prev => Some(hub_nav::prev(items, current)),
        hub_nav::ShortcutAction::Next => Some(hub_nav::next(items, current)),
        hub_nav::ShortcutAction::Index(index) => items.get(index).copied(),
    }
}

#[cfg(test)]
#[path = "hub_key_state/tests.rs"]
mod tests;
