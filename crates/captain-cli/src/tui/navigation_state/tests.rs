use super::*;
use ratatui::crossterm::event::{KeyCode, KeyModifiers};

#[test]
fn startup_phase_prefers_wizard_when_setup_is_required() {
    assert!(matches!(
        startup_phase_for_state(true, true),
        Phase::Boot(BootScreen::Wizard)
    ));
}

#[test]
fn startup_phase_offers_resume_when_session_candidate_exists() {
    assert!(matches!(
        startup_phase_for_state(false, true),
        Phase::Boot(BootScreen::ResumePrompt)
    ));
}

#[test]
fn startup_phase_falls_back_to_welcome_without_resume_candidate() {
    assert!(matches!(
        startup_phase_for_state(false, false),
        Phase::Boot(BootScreen::Welcome)
    ));
}

#[test]
fn primary_tabs_keep_operational_order_and_labels() {
    let labels: Vec<_> = TABS.iter().map(|tab| tab.label()).collect();

    assert_eq!(
        labels,
        vec![
            "Chat",
            "Projects",
            "Automation",
            "Learning",
            "Capabilities",
            "Status"
        ]
    );
}

#[test]
fn tab_index_uses_primary_tab_position() {
    assert_eq!(Tab::Chat.index(), 0);
    assert_eq!(Tab::Projects.index(), 1);
    assert_eq!(Tab::Dashboard.index(), 5);
}

#[test]
fn main_phase_entry_lands_on_chat() {
    assert_eq!(main_phase_entry_plan().active_tab, Tab::Chat);
}

#[test]
fn main_phase_entry_routes_keep_bootstrap_order() {
    assert_eq!(
        main_phase_entry_plan().routes,
        &[
            MainPhaseEntryRoute::RefreshAgents,
            MainPhaseEntryRoute::RefreshDashboard,
            MainPhaseEntryRoute::RefreshChannels,
            MainPhaseEntryRoute::AutoSelectDefaultAgent,
            MainPhaseEntryRoute::ApplyWorkspaceExtraPaths,
        ]
    );
}

#[test]
fn function_keys_map_to_primary_tabs() {
    assert_eq!(tab_for_function_key(1), Some(Tab::Chat));
    assert_eq!(tab_for_function_key(2), Some(Tab::Projects));
    assert_eq!(tab_for_function_key(3), Some(Tab::Workflows));
    assert_eq!(tab_for_function_key(6), Some(Tab::Dashboard));
}

#[test]
fn unsupported_function_keys_are_ignored() {
    assert_eq!(tab_for_function_key(0), None);
    assert_eq!(tab_for_function_key(7), None);
}

#[test]
fn alt_digits_map_to_primary_tabs() {
    assert_eq!(tab_for_alt_digit('1'), Some(Tab::Chat));
    assert_eq!(tab_for_alt_digit('2'), Some(Tab::Projects));
    assert_eq!(tab_for_alt_digit('3'), Some(Tab::Workflows));
    assert_eq!(tab_for_alt_digit('6'), Some(Tab::Dashboard));
}

#[test]
fn unsupported_alt_digits_are_ignored() {
    assert_eq!(tab_for_alt_digit('x'), None);
    assert_eq!(tab_for_alt_digit('/'), None);
}

#[test]
fn file_picker_esc_closes_when_open() {
    assert_eq!(
        file_picker_key_action_for_key(true, KeyCode::Esc),
        Some(FilePickerKeyAction::Close)
    );
}

#[test]
fn file_picker_other_keys_route_to_picker() {
    assert_eq!(
        file_picker_key_action_for_key(true, KeyCode::Enter),
        Some(FilePickerKeyAction::RouteToPicker)
    );
    assert_eq!(
        file_picker_key_action_for_key(true, KeyCode::Down),
        Some(FilePickerKeyAction::RouteToPicker)
    );
}

#[test]
fn file_picker_closed_ignores_keys() {
    assert_eq!(file_picker_key_action_for_key(false, KeyCode::Esc), None);
    assert_eq!(file_picker_key_action_for_key(false, KeyCode::Enter), None);
}

#[test]
fn main_global_ctrl_q_quits() {
    assert_eq!(
        main_global_key_action_for_key(
            KeyCode::Char('q'),
            KeyModifiers::CONTROL | KeyModifiers::ALT,
            false
        ),
        Some(MainGlobalKeyAction::Quit)
    );
}

#[test]
fn main_global_function_key_switches_tab() {
    assert_eq!(
        main_global_key_action_for_key(KeyCode::F(3), KeyModifiers::empty(), false),
        Some(MainGlobalKeyAction::SwitchTab(Tab::Workflows))
    );
}

#[test]
fn main_global_tab_cycles_when_chat_completion_is_unprotected() {
    assert_eq!(
        main_global_key_action_for_key(KeyCode::Tab, KeyModifiers::empty(), false),
        Some(MainGlobalKeyAction::CycleTab(TabCycle::Next))
    );
}

#[test]
fn main_global_tab_preserves_chat_completion() {
    assert_eq!(
        main_global_key_action_for_key(KeyCode::Tab, KeyModifiers::empty(), true),
        None
    );
}

#[test]
fn main_global_alt_digit_switches_tab() {
    assert_eq!(
        main_global_key_action_for_key(KeyCode::Char('6'), KeyModifiers::ALT, false),
        Some(MainGlobalKeyAction::SwitchTab(Tab::Dashboard))
    );
}

#[test]
fn main_global_ignores_non_global_keys() {
    assert_eq!(
        main_global_key_action_for_key(KeyCode::Char('x'), KeyModifiers::empty(), false),
        None
    );
}

#[test]
fn hub_shortcuts_require_alt_modifier() {
    assert_eq!(
        hub_shortcut_route_for_key(Tab::Workflows, KeyCode::Left, KeyModifiers::empty()),
        None
    );
}

#[test]
fn hub_shortcuts_route_to_active_hub_family() {
    assert_eq!(
        hub_shortcut_route_for_key(Tab::Workflows, KeyCode::Left, KeyModifiers::ALT),
        Some(HubShortcutRoute::Automation(hub_nav::ShortcutAction::Prev))
    );
    assert_eq!(
        hub_shortcut_route_for_key(Tab::Learning, KeyCode::Right, KeyModifiers::ALT),
        Some(HubShortcutRoute::Learning(hub_nav::ShortcutAction::Next))
    );
    assert_eq!(
        hub_shortcut_route_for_key(Tab::Skills, KeyCode::Char('2'), KeyModifiers::ALT),
        Some(HubShortcutRoute::Capabilities(
            hub_nav::ShortcutAction::Index(1)
        ))
    );
    assert_eq!(
        hub_shortcut_route_for_key(Tab::Channels, KeyCode::Char('4'), KeyModifiers::ALT),
        Some(HubShortcutRoute::Connections(
            hub_nav::ShortcutAction::Index(3)
        ))
    );
}

#[test]
fn hub_shortcuts_ignore_non_hub_tabs_and_out_of_range_indexes() {
    assert_eq!(
        hub_shortcut_route_for_key(Tab::Chat, KeyCode::Left, KeyModifiers::ALT),
        None
    );
    assert_eq!(
        hub_shortcut_route_for_key(Tab::Skills, KeyCode::Char('3'), KeyModifiers::ALT),
        None
    );
}

#[test]
fn overlay_esc_closes_in_main_phase() {
    assert_eq!(
        overlay_key_action_for_key(Phase::Main, Some(Tab::Memory), KeyCode::Esc),
        Some(OverlayKeyAction::Close)
    );
}

#[test]
fn overlay_non_esc_routes_to_overlay_tab() {
    assert_eq!(
        overlay_key_action_for_key(Phase::Main, Some(Tab::Logs), KeyCode::Down),
        Some(OverlayKeyAction::RouteTo(Tab::Logs))
    );
}

#[test]
fn overlay_without_tab_is_ignored() {
    assert_eq!(
        overlay_key_action_for_key(Phase::Main, None, KeyCode::Esc),
        None
    );
}

#[test]
fn overlay_in_boot_phase_is_ignored() {
    assert_eq!(
        overlay_key_action_for_key(
            Phase::Boot(BootScreen::Welcome),
            Some(Tab::Memory),
            KeyCode::Esc
        ),
        None
    );
}

#[test]
fn overlay_open_state_sets_target_and_enters_tab() {
    assert_eq!(
        overlay_state_after_open(Tab::Budget),
        OverlayState {
            overlay_tab: Some(Tab::Budget),
            enter_tab: Some(Tab::Budget),
        }
    );
}

#[test]
fn overlay_close_state_clears_target_without_enter_effect() {
    assert_eq!(
        overlay_state_after_close(),
        OverlayState {
            overlay_tab: None,
            enter_tab: None,
        }
    );
}

#[test]
fn screen_key_route_maps_boot_screens() {
    assert_eq!(
        screen_key_route_for_state(Phase::Boot(BootScreen::Welcome), Tab::Chat),
        ScreenKeyRoute::Welcome
    );
    assert_eq!(
        screen_key_route_for_state(Phase::Boot(BootScreen::Wizard), Tab::Chat),
        ScreenKeyRoute::Wizard
    );
    assert_eq!(
        screen_key_route_for_state(Phase::Boot(BootScreen::ResumePrompt), Tab::Chat),
        ScreenKeyRoute::ResumePrompt
    );
}

#[test]
fn screen_key_route_maps_main_active_tab() {
    assert_eq!(
        screen_key_route_for_state(Phase::Main, Tab::Projects),
        ScreenKeyRoute::Main(Tab::Projects)
    );
    assert_eq!(
        screen_key_route_for_state(Phase::Main, Tab::Logs),
        ScreenKeyRoute::Main(Tab::Logs)
    );
}

#[test]
fn first_ctrl_c_in_main_arms_and_stops_routing() {
    assert_eq!(
        ctrl_c_action_for_key(
            KeyCode::Char('c'),
            KeyModifiers::CONTROL,
            false,
            Phase::Main
        ),
        CtrlCAction::ArmAndStopRouting
    );
}

#[test]
fn first_ctrl_c_in_boot_arms_and_continues_routing() {
    assert_eq!(
        ctrl_c_action_for_key(
            KeyCode::Char('c'),
            KeyModifiers::CONTROL,
            false,
            Phase::Boot(BootScreen::Welcome)
        ),
        CtrlCAction::ArmAndContinueRouting
    );
}

#[test]
fn second_ctrl_c_quits_from_any_phase() {
    assert_eq!(
        ctrl_c_action_for_key(
            KeyCode::Char('c'),
            KeyModifiers::CONTROL,
            true,
            Phase::Boot(BootScreen::Wizard)
        ),
        CtrlCAction::Quit
    );
}

#[test]
fn non_ctrl_c_clears_pending_quit() {
    assert_eq!(
        ctrl_c_action_for_key(KeyCode::Char('x'), KeyModifiers::CONTROL, true, Phase::Main),
        CtrlCAction::ClearPending
    );
}

#[test]
fn ctrl_c_keeps_extra_modifiers_compatible() {
    assert_eq!(
        ctrl_c_action_for_key(
            KeyCode::Char('c'),
            KeyModifiers::CONTROL | KeyModifiers::ALT,
            false,
            Phase::Main
        ),
        CtrlCAction::ArmAndStopRouting
    );
}

#[test]
fn resume_prompt_accepts_yes_and_enter() {
    assert_eq!(
        resume_prompt_action_for_key(KeyCode::Char('y')),
        Some(ResumePromptAction::Accept)
    );
    assert_eq!(
        resume_prompt_action_for_key(KeyCode::Char('Y')),
        Some(ResumePromptAction::Accept)
    );
    assert_eq!(
        resume_prompt_action_for_key(KeyCode::Enter),
        Some(ResumePromptAction::Accept)
    );
}

#[test]
fn resume_prompt_declines_no_and_escape() {
    assert_eq!(
        resume_prompt_action_for_key(KeyCode::Char('n')),
        Some(ResumePromptAction::Decline)
    );
    assert_eq!(
        resume_prompt_action_for_key(KeyCode::Char('N')),
        Some(ResumePromptAction::Decline)
    );
    assert_eq!(
        resume_prompt_action_for_key(KeyCode::Esc),
        Some(ResumePromptAction::Decline)
    );
}

#[test]
fn resume_prompt_ignores_other_keys() {
    assert_eq!(resume_prompt_action_for_key(KeyCode::Char('x')), None);
    assert_eq!(resume_prompt_action_for_key(KeyCode::Tab), None);
}

#[test]
fn tab_key_cycles_forward_outside_chat_completion() {
    assert_eq!(
        tab_cycle_for_key(KeyCode::Tab, KeyModifiers::empty(), false),
        Some(TabCycle::Next)
    );
}

#[test]
fn tab_key_is_preserved_for_chat_slash_completion() {
    assert_eq!(
        tab_cycle_for_key(KeyCode::Tab, KeyModifiers::empty(), true),
        None
    );
}

#[test]
fn backtab_cycles_backward() {
    assert_eq!(
        tab_cycle_for_key(KeyCode::BackTab, KeyModifiers::SHIFT, false),
        Some(TabCycle::Previous)
    );
}

#[test]
fn ctrl_arrows_cycle_tabs() {
    assert_eq!(
        tab_cycle_for_key(KeyCode::Left, KeyModifiers::CONTROL, false),
        Some(TabCycle::Previous)
    );
    assert_eq!(
        tab_cycle_for_key(KeyCode::Right, KeyModifiers::CONTROL, false),
        Some(TabCycle::Next)
    );
}

#[test]
fn ctrl_brackets_cycle_tabs() {
    assert_eq!(
        tab_cycle_for_key(KeyCode::Char('['), KeyModifiers::CONTROL, false),
        Some(TabCycle::Previous)
    );
    assert_eq!(
        tab_cycle_for_key(KeyCode::Char(']'), KeyModifiers::CONTROL, false),
        Some(TabCycle::Next)
    );
}

#[test]
fn unsupported_cycle_shortcuts_are_ignored() {
    assert_eq!(
        tab_cycle_for_key(KeyCode::Char('x'), KeyModifiers::CONTROL, false),
        None
    );
    assert_eq!(
        tab_cycle_for_key(KeyCode::Tab, KeyModifiers::ALT, false),
        None
    );
}

#[test]
fn non_primary_tabs_default_to_first_index() {
    assert_eq!(Tab::Logs.index(), 0);
    assert_eq!(Tab::Approvals.index(), 0);
    assert_eq!(Tab::Settings.index(), 0);
    assert_eq!(Tab::Channels.index(), 0);
}

#[test]
fn automation_views_keep_order_and_labels() {
    let labels: Vec<_> = AUTOMATION_VIEWS.iter().map(|view| view.label()).collect();

    assert_eq!(labels, vec!["Workflows", "Triggers", "Cron", "Approvals"]);
    assert_eq!(AutomationView::Cron.index(), 2);
}

#[test]
fn learning_views_keep_order_and_labels() {
    let labels: Vec<_> = LEARNING_VIEWS.iter().map(|view| view.label()).collect();

    assert_eq!(labels, vec!["Review", "Workflows", "Memory", "Graph"]);
    assert_eq!(LearningView::Graph.index(), 3);
}

#[test]
fn capabilities_views_keep_order_and_labels() {
    let labels: Vec<_> = CAPABILITIES_VIEWS.iter().map(|view| view.label()).collect();

    assert_eq!(labels, vec!["Natives", "Skills"]);
    assert_eq!(CapabilitiesView::Native.index(), 0);
    assert_eq!(CapabilitiesView::Skills.index(), 1);
}

#[test]
fn connections_views_keep_order_and_labels() {
    let labels: Vec<_> = CONNECTIONS_VIEWS.iter().map(|view| view.label()).collect();

    assert_eq!(labels, vec!["Channels", "Extensions", "Peers", "Comms"]);
    assert_eq!(ConnectionsView::Comms.index(), 3);
}
