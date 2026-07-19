use super::*;
use crate::tui::navigation_state::{
    AutomationView, BootScreen, CapabilitiesView, ConnectionsView, LearningView, Phase, Tab,
};
use ratatui::layout::Rect;

#[test]
fn frame_route_rejects_too_small_terminals_before_phase() {
    let cases = [
        Rect::new(0, 0, MIN_DRAW_WIDTH - 1, MIN_DRAW_HEIGHT),
        Rect::new(0, 0, MIN_DRAW_WIDTH, MIN_DRAW_HEIGHT - 1),
        Rect::new(0, 0, 1, 1),
    ];

    for area in cases {
        assert_eq!(
            frame_draw_route_for_state(area, Phase::Main),
            FrameDrawRoute::TooSmall {
                min_width: MIN_DRAW_WIDTH,
                min_height: MIN_DRAW_HEIGHT,
            }
        );
    }
}

#[test]
fn frame_route_accepts_exact_minimum_size() {
    let area = Rect::new(0, 0, MIN_DRAW_WIDTH, MIN_DRAW_HEIGHT);

    assert_eq!(
        frame_draw_route_for_state(area, Phase::Boot(BootScreen::Welcome)),
        FrameDrawRoute::Welcome
    );
}

#[test]
fn frame_route_maps_each_phase_to_renderer_family() {
    let area = Rect::new(0, 0, MIN_DRAW_WIDTH + 20, MIN_DRAW_HEIGHT + 10);
    let cases = [
        (Phase::Boot(BootScreen::Welcome), FrameDrawRoute::Welcome),
        (Phase::Boot(BootScreen::Wizard), FrameDrawRoute::Wizard),
        (
            Phase::Boot(BootScreen::ResumePrompt),
            FrameDrawRoute::ResumePrompt,
        ),
        (Phase::Main, FrameDrawRoute::Main),
    ];

    for (phase, route) in cases {
        assert_eq!(frame_draw_route_for_state(area, phase), route);
    }
}

#[test]
fn main_draw_composition_splits_tab_bar_and_content() {
    let composition = main_draw_composition_for_state(Rect::new(3, 4, 100, 30), None, false);

    assert_eq!(composition.tab_bar_area, Rect::new(3, 4, 100, 1));
    assert_eq!(composition.content_area, Rect::new(3, 5, 100, 29));
}

#[test]
fn main_draw_composition_routes_overlay_before_file_picker() {
    let composition =
        main_draw_composition_for_state(Rect::new(0, 0, 100, 30), Some(Tab::Logs), true);

    assert_eq!(
        composition.layer_routes,
        [
            Some(MainDrawLayerRoute::Overlay(Tab::Logs)),
            Some(MainDrawLayerRoute::FilePicker),
        ]
    );
}

#[test]
fn main_draw_composition_omits_absent_layers() {
    let composition = main_draw_composition_for_state(Rect::new(0, 0, 80, 20), None, false);

    assert_eq!(composition.layer_routes, [None, None]);
}

#[test]
fn hub_draw_composition_splits_nav_and_content() {
    let composition = hub_draw_composition_for_area(Rect::new(7, 9, 90, 18));

    assert_eq!(composition.nav_area, Rect::new(7, 9, 90, 1));
    assert_eq!(composition.content_area, Rect::new(7, 10, 90, 17));
}

#[test]
fn hub_draw_composition_preserves_exact_width() {
    let composition = hub_draw_composition_for_area(Rect::new(2, 3, 1, 4));

    assert_eq!(composition.nav_area.width, 1);
    assert_eq!(composition.content_area.width, 1);
}

#[test]
fn primary_hub_tabs_route_to_hub_renderers() {
    assert_eq!(
        main_draw_route_for_tab(Tab::Workflows),
        MainDrawRoute::AutomationHub
    );
    assert_eq!(
        main_draw_route_for_tab(Tab::Learning),
        MainDrawRoute::LearningHub
    );
    assert_eq!(
        main_draw_route_for_tab(Tab::Skills),
        MainDrawRoute::CapabilitiesHub
    );
    assert_eq!(
        main_draw_route_for_tab(Tab::Channels),
        MainDrawRoute::ConnectionsHub
    );
}

#[test]
fn direct_tabs_route_to_direct_renderers() {
    let cases = [
        (Tab::Dashboard, MainDrawRoute::Dashboard),
        (Tab::Agents, MainDrawRoute::Agents),
        (Tab::Chat, MainDrawRoute::Chat),
        (Tab::Projects, MainDrawRoute::Projects),
        (Tab::Triggers, MainDrawRoute::Triggers),
        (Tab::Cron, MainDrawRoute::Cron),
        (Tab::Approvals, MainDrawRoute::Approvals),
        (Tab::Budget, MainDrawRoute::Budget),
        (Tab::Graph, MainDrawRoute::Graph),
        (Tab::Sessions, MainDrawRoute::Sessions),
        (Tab::Memory, MainDrawRoute::Memory),
        (Tab::SkillsProposed, MainDrawRoute::SkillsProposed),
        (Tab::Hands, MainDrawRoute::Hands),
        (Tab::Extensions, MainDrawRoute::Extensions),
        (Tab::Templates, MainDrawRoute::Templates),
        (Tab::Security, MainDrawRoute::Security),
        (Tab::Audit, MainDrawRoute::Audit),
        (Tab::Usage, MainDrawRoute::Usage),
        (Tab::Settings, MainDrawRoute::Settings),
        (Tab::Peers, MainDrawRoute::Peers),
        (Tab::Comms, MainDrawRoute::Comms),
        (Tab::Logs, MainDrawRoute::Logs),
    ];

    for (tab, route) in cases {
        assert_eq!(main_draw_route_for_tab(tab), route);
    }
}

#[test]
fn supported_overlay_tabs_route_to_overlay_renderers() {
    let cases = [
        (Tab::Memory, OverlayDrawRoute::Memory),
        (Tab::Learning, OverlayDrawRoute::Learning),
        (Tab::SkillsProposed, OverlayDrawRoute::SkillsProposed),
        (Tab::Cron, OverlayDrawRoute::Cron),
        (Tab::Approvals, OverlayDrawRoute::Approvals),
        (Tab::Budget, OverlayDrawRoute::Budget),
        (Tab::Graph, OverlayDrawRoute::Graph),
        (Tab::Logs, OverlayDrawRoute::Logs),
        (Tab::Settings, OverlayDrawRoute::Settings),
    ];

    for (tab, route) in cases {
        assert_eq!(overlay_draw_route_for_tab(tab), route);
    }
}

#[test]
fn unsupported_overlay_tabs_are_explicit() {
    let cases = [
        Tab::Dashboard,
        Tab::Agents,
        Tab::Chat,
        Tab::Projects,
        Tab::Channels,
        Tab::Workflows,
        Tab::Triggers,
        Tab::Sessions,
        Tab::Skills,
        Tab::Hands,
        Tab::Extensions,
        Tab::Templates,
        Tab::Security,
        Tab::Audit,
        Tab::Usage,
        Tab::Peers,
        Tab::Comms,
    ];

    for tab in cases {
        assert_eq!(
            overlay_draw_route_for_tab(tab),
            OverlayDrawRoute::Unsupported
        );
    }
}

#[test]
fn automation_hub_views_route_to_renderers() {
    let cases = [
        (AutomationView::Workflows, AutomationHubDrawRoute::Workflows),
        (AutomationView::Triggers, AutomationHubDrawRoute::Triggers),
        (AutomationView::Cron, AutomationHubDrawRoute::Cron),
        (AutomationView::Approvals, AutomationHubDrawRoute::Approvals),
    ];

    for (view, route) in cases {
        assert_eq!(automation_hub_draw_route_for_view(view), route);
    }
}

#[test]
fn learning_hub_views_route_to_renderers() {
    let cases = [
        (LearningView::Review, LearningHubDrawRoute::Review),
        (
            LearningView::SkillProposals,
            LearningHubDrawRoute::SkillProposals,
        ),
        (LearningView::Memory, LearningHubDrawRoute::Memory),
        (LearningView::Graph, LearningHubDrawRoute::Graph),
    ];

    for (view, route) in cases {
        assert_eq!(learning_hub_draw_route_for_view(view), route);
    }
}

#[test]
fn capabilities_hub_views_route_to_renderers() {
    let cases = [
        (CapabilitiesView::Native, CapabilitiesHubDrawRoute::Native),
        (CapabilitiesView::Skills, CapabilitiesHubDrawRoute::Skills),
    ];

    for (view, route) in cases {
        assert_eq!(capabilities_hub_draw_route_for_view(view), route);
    }
}

#[test]
fn connections_hub_views_route_to_renderers() {
    let cases = [
        (ConnectionsView::Channels, ConnectionsHubDrawRoute::Channels),
        (
            ConnectionsView::Extensions,
            ConnectionsHubDrawRoute::Extensions,
        ),
        (ConnectionsView::Peers, ConnectionsHubDrawRoute::Peers),
        (ConnectionsView::Comms, ConnectionsHubDrawRoute::Comms),
    ];

    for (view, route) in cases {
        assert_eq!(connections_hub_draw_route_for_view(view), route);
    }
}
