use super::navigation_state::{
    AutomationView, BootScreen, CapabilitiesView, ConnectionsView, LearningView, Phase, Tab,
};
use ratatui::layout::{Constraint, Layout, Rect};

pub(crate) const MIN_DRAW_WIDTH: u16 = 60;
pub(crate) const MIN_DRAW_HEIGHT: u16 = 16;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum FrameDrawRoute {
    TooSmall { min_width: u16, min_height: u16 },
    Welcome,
    Wizard,
    ResumePrompt,
    Main,
}

pub(crate) fn frame_draw_route_for_state(area: Rect, phase: Phase) -> FrameDrawRoute {
    if area.width < MIN_DRAW_WIDTH || area.height < MIN_DRAW_HEIGHT {
        return FrameDrawRoute::TooSmall {
            min_width: MIN_DRAW_WIDTH,
            min_height: MIN_DRAW_HEIGHT,
        };
    }

    match phase {
        Phase::Boot(BootScreen::Welcome) => FrameDrawRoute::Welcome,
        Phase::Boot(BootScreen::Wizard) => FrameDrawRoute::Wizard,
        Phase::Boot(BootScreen::ResumePrompt) => FrameDrawRoute::ResumePrompt,
        Phase::Main => FrameDrawRoute::Main,
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum MainDrawLayerRoute {
    Overlay(Tab),
    FilePicker,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct MainDrawComposition {
    pub(crate) tab_bar_area: Rect,
    pub(crate) content_area: Rect,
    pub(crate) layer_routes: [Option<MainDrawLayerRoute>; 2],
}

pub(crate) fn main_draw_composition_for_state(
    area: Rect,
    overlay_tab: Option<Tab>,
    file_picker_open: bool,
) -> MainDrawComposition {
    let chunks = Layout::vertical([Constraint::Length(1), Constraint::Min(1)]).split(area);

    MainDrawComposition {
        tab_bar_area: chunks[0],
        content_area: chunks[1],
        layer_routes: [
            overlay_tab.map(MainDrawLayerRoute::Overlay),
            file_picker_open.then_some(MainDrawLayerRoute::FilePicker),
        ],
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct HubDrawComposition {
    pub(crate) nav_area: Rect,
    pub(crate) content_area: Rect,
}

pub(crate) fn hub_draw_composition_for_area(area: Rect) -> HubDrawComposition {
    let chunks = Layout::vertical([Constraint::Length(1), Constraint::Min(1)]).split(area);

    HubDrawComposition {
        nav_area: chunks[0],
        content_area: chunks[1],
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum MainDrawRoute {
    Dashboard,
    Agents,
    Chat,
    Projects,
    ConnectionsHub,
    AutomationHub,
    Triggers,
    Cron,
    Approvals,
    Budget,
    Graph,
    Sessions,
    Memory,
    LearningHub,
    SkillsProposed,
    CapabilitiesHub,
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
}

pub(crate) fn main_draw_route_for_tab(tab: Tab) -> MainDrawRoute {
    match tab {
        Tab::Dashboard => MainDrawRoute::Dashboard,
        Tab::Agents => MainDrawRoute::Agents,
        Tab::Chat => MainDrawRoute::Chat,
        Tab::Projects => MainDrawRoute::Projects,
        Tab::Channels => MainDrawRoute::ConnectionsHub,
        Tab::Workflows => MainDrawRoute::AutomationHub,
        Tab::Triggers => MainDrawRoute::Triggers,
        Tab::Cron => MainDrawRoute::Cron,
        Tab::Approvals => MainDrawRoute::Approvals,
        Tab::Budget => MainDrawRoute::Budget,
        Tab::Graph => MainDrawRoute::Graph,
        Tab::Sessions => MainDrawRoute::Sessions,
        Tab::Memory => MainDrawRoute::Memory,
        Tab::Learning => MainDrawRoute::LearningHub,
        Tab::SkillsProposed => MainDrawRoute::SkillsProposed,
        Tab::Skills => MainDrawRoute::CapabilitiesHub,
        Tab::Hands => MainDrawRoute::Hands,
        Tab::Extensions => MainDrawRoute::Extensions,
        Tab::Templates => MainDrawRoute::Templates,
        Tab::Security => MainDrawRoute::Security,
        Tab::Audit => MainDrawRoute::Audit,
        Tab::Usage => MainDrawRoute::Usage,
        Tab::Settings => MainDrawRoute::Settings,
        Tab::Peers => MainDrawRoute::Peers,
        Tab::Comms => MainDrawRoute::Comms,
        Tab::Logs => MainDrawRoute::Logs,
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum OverlayDrawRoute {
    Memory,
    Learning,
    SkillsProposed,
    Cron,
    Approvals,
    Budget,
    Graph,
    Logs,
    Settings,
    Unsupported,
}

pub(crate) fn overlay_draw_route_for_tab(tab: Tab) -> OverlayDrawRoute {
    match tab {
        Tab::Memory => OverlayDrawRoute::Memory,
        Tab::Learning => OverlayDrawRoute::Learning,
        Tab::SkillsProposed => OverlayDrawRoute::SkillsProposed,
        Tab::Cron => OverlayDrawRoute::Cron,
        Tab::Approvals => OverlayDrawRoute::Approvals,
        Tab::Budget => OverlayDrawRoute::Budget,
        Tab::Graph => OverlayDrawRoute::Graph,
        Tab::Logs => OverlayDrawRoute::Logs,
        Tab::Settings => OverlayDrawRoute::Settings,
        _ => OverlayDrawRoute::Unsupported,
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum AutomationHubDrawRoute {
    Workflows,
    Triggers,
    Cron,
    Approvals,
}

pub(crate) fn automation_hub_draw_route_for_view(view: AutomationView) -> AutomationHubDrawRoute {
    match view {
        AutomationView::Workflows => AutomationHubDrawRoute::Workflows,
        AutomationView::Triggers => AutomationHubDrawRoute::Triggers,
        AutomationView::Cron => AutomationHubDrawRoute::Cron,
        AutomationView::Approvals => AutomationHubDrawRoute::Approvals,
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum LearningHubDrawRoute {
    Review,
    SkillProposals,
    Memory,
    Graph,
}

pub(crate) fn learning_hub_draw_route_for_view(view: LearningView) -> LearningHubDrawRoute {
    match view {
        LearningView::Review => LearningHubDrawRoute::Review,
        LearningView::SkillProposals => LearningHubDrawRoute::SkillProposals,
        LearningView::Memory => LearningHubDrawRoute::Memory,
        LearningView::Graph => LearningHubDrawRoute::Graph,
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum CapabilitiesHubDrawRoute {
    Skills,
    Hands,
}

pub(crate) fn capabilities_hub_draw_route_for_view(
    view: CapabilitiesView,
) -> CapabilitiesHubDrawRoute {
    match view {
        CapabilitiesView::Skills => CapabilitiesHubDrawRoute::Skills,
        CapabilitiesView::Hands => CapabilitiesHubDrawRoute::Hands,
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ConnectionsHubDrawRoute {
    Channels,
    Extensions,
    Peers,
    Comms,
}

pub(crate) fn connections_hub_draw_route_for_view(
    view: ConnectionsView,
) -> ConnectionsHubDrawRoute {
    match view {
        ConnectionsView::Channels => ConnectionsHubDrawRoute::Channels,
        ConnectionsView::Extensions => ConnectionsHubDrawRoute::Extensions,
        ConnectionsView::Peers => ConnectionsHubDrawRoute::Peers,
        ConnectionsView::Comms => ConnectionsHubDrawRoute::Comms,
    }
}

#[cfg(test)]
#[path = "render_state/tests.rs"]
mod tests;
