use super::hub_nav;
use ratatui::crossterm::event::{KeyCode, KeyModifiers};

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum Phase {
    Boot(BootScreen),
    Main,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum BootScreen {
    Welcome,
    Wizard,
    /// Offer to resume the most recent saved session before falling back
    /// to the empty Welcome screen. The candidate `SessionSummary` lives
    /// in `App::pending_resume` so this enum can stay `Copy`.
    ResumePrompt,
}

pub(crate) fn startup_phase_for_state(needs_setup: bool, has_resume_candidate: bool) -> Phase {
    if needs_setup {
        Phase::Boot(BootScreen::Wizard)
    } else if has_resume_candidate {
        Phase::Boot(BootScreen::ResumePrompt)
    } else {
        Phase::Boot(BootScreen::Welcome)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[allow(dead_code)]
pub(crate) enum Tab {
    Dashboard,
    Agents,
    Chat,
    Projects,
    Sessions,
    Workflows,
    Triggers,
    Cron,
    Memory,
    Graph,
    Learning,
    Channels,
    Skills,
    SkillsProposed,
    Hands,
    Extensions,
    Templates,
    Peers,
    Comms,
    Security,
    Audit,
    Approvals,
    Budget,
    Usage,
    Settings,
    Logs,
}

pub(crate) const TABS: &[Tab] = &[
    Tab::Chat,
    Tab::Projects,
    Tab::Workflows,
    Tab::Learning,
    Tab::Skills,
    Tab::Dashboard,
];

impl Tab {
    pub(crate) fn label(self) -> &'static str {
        match self {
            Tab::Dashboard => "Status",
            Tab::Agents => "Agents",
            Tab::Chat => "Chat",
            Tab::Projects => "Projects",
            Tab::Sessions => "Sessions",
            Tab::Workflows => "Automation",
            Tab::Triggers => "Triggers",
            Tab::Cron => "Cron",
            Tab::Memory => "Memory",
            Tab::Graph => "Graph",
            Tab::Learning => "Learning",
            Tab::Channels => "Connections",
            Tab::Skills => "Capabilities",
            Tab::SkillsProposed => "Learned workflows",
            Tab::Hands => "Hands",
            Tab::Extensions => "Extensions",
            Tab::Templates => "Templates",
            Tab::Peers => "Peers",
            Tab::Comms => "Comms",
            Tab::Security => "Security",
            Tab::Audit => "Audit",
            Tab::Approvals => "Approvals",
            Tab::Budget => "Budget",
            Tab::Usage => "Usage",
            Tab::Settings => "Settings",
            Tab::Logs => "Logs",
        }
    }

    pub(crate) fn index(self) -> usize {
        hub_nav::index(TABS, self)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum MainPhaseEntryRoute {
    RefreshAgents,
    RefreshDashboard,
    RefreshChannels,
    AutoSelectDefaultAgent,
    ApplyWorkspaceExtraPaths,
}

pub(crate) struct MainPhaseEntryPlan {
    pub(crate) active_tab: Tab,
    pub(crate) routes: &'static [MainPhaseEntryRoute],
}

const MAIN_PHASE_ENTRY_ROUTES: &[MainPhaseEntryRoute] = &[
    MainPhaseEntryRoute::RefreshAgents,
    MainPhaseEntryRoute::RefreshDashboard,
    MainPhaseEntryRoute::RefreshChannels,
    MainPhaseEntryRoute::AutoSelectDefaultAgent,
    MainPhaseEntryRoute::ApplyWorkspaceExtraPaths,
];

pub(crate) fn main_phase_entry_plan() -> MainPhaseEntryPlan {
    MainPhaseEntryPlan {
        active_tab: Tab::Chat,
        routes: MAIN_PHASE_ENTRY_ROUTES,
    }
}

pub(crate) fn tab_for_function_key(number: u8) -> Option<Tab> {
    tab_for_shortcut_number(number as usize)
}

pub(crate) fn tab_for_alt_digit(digit: char) -> Option<Tab> {
    let number = match digit {
        '1'..='9' => digit as usize - '0' as usize,
        '0' => 10,
        _ => return None,
    };
    tab_for_shortcut_number(number)
}

fn tab_for_shortcut_number(number: usize) -> Option<Tab> {
    if number == 0 {
        return None;
    }
    TABS.get(number - 1).copied()
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum FilePickerKeyAction {
    Close,
    RouteToPicker,
}

pub(crate) fn file_picker_key_action_for_key(
    is_open: bool,
    code: KeyCode,
) -> Option<FilePickerKeyAction> {
    if !is_open {
        return None;
    }

    if matches!(code, KeyCode::Esc) {
        Some(FilePickerKeyAction::Close)
    } else {
        Some(FilePickerKeyAction::RouteToPicker)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum MainGlobalKeyAction {
    Quit,
    SwitchTab(Tab),
    CycleTab(TabCycle),
}

pub(crate) fn main_global_key_action_for_key(
    code: KeyCode,
    modifiers: KeyModifiers,
    protect_chat_completion: bool,
) -> Option<MainGlobalKeyAction> {
    if code == KeyCode::Char('q') && modifiers.contains(KeyModifiers::CONTROL) {
        return Some(MainGlobalKeyAction::Quit);
    }

    if let KeyCode::F(number) = code {
        return tab_for_function_key(number).map(MainGlobalKeyAction::SwitchTab);
    }

    if let Some(cycle) = tab_cycle_for_key(code, modifiers, protect_chat_completion) {
        return Some(MainGlobalKeyAction::CycleTab(cycle));
    }

    if modifiers.contains(KeyModifiers::ALT) {
        if let KeyCode::Char(digit) = code {
            return tab_for_alt_digit(digit).map(MainGlobalKeyAction::SwitchTab);
        }
    }

    None
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum HubShortcutRoute {
    Automation(hub_nav::ShortcutAction),
    Learning(hub_nav::ShortcutAction),
    Capabilities(hub_nav::ShortcutAction),
    Connections(hub_nav::ShortcutAction),
}

pub(crate) fn hub_shortcut_route_for_key(
    active_tab: Tab,
    code: KeyCode,
    modifiers: KeyModifiers,
) -> Option<HubShortcutRoute> {
    if !modifiers.contains(KeyModifiers::ALT) {
        return None;
    }

    match active_tab {
        Tab::Workflows => {
            hub_nav::shortcut_action(code, AUTOMATION_VIEWS.len()).map(HubShortcutRoute::Automation)
        }
        Tab::Learning => {
            hub_nav::shortcut_action(code, LEARNING_VIEWS.len()).map(HubShortcutRoute::Learning)
        }
        Tab::Skills => hub_nav::shortcut_action(code, CAPABILITIES_VIEWS.len())
            .map(HubShortcutRoute::Capabilities),
        Tab::Channels => hub_nav::shortcut_action(code, CONNECTIONS_VIEWS.len())
            .map(HubShortcutRoute::Connections),
        _ => None,
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum OverlayKeyAction {
    Close,
    RouteTo(Tab),
}

pub(crate) fn overlay_key_action_for_key(
    phase: Phase,
    overlay_tab: Option<Tab>,
    code: KeyCode,
) -> Option<OverlayKeyAction> {
    if !matches!(phase, Phase::Main) {
        return None;
    }

    let overlay = overlay_tab?;
    if matches!(code, KeyCode::Esc) {
        Some(OverlayKeyAction::Close)
    } else {
        Some(OverlayKeyAction::RouteTo(overlay))
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct OverlayState {
    pub(crate) overlay_tab: Option<Tab>,
    pub(crate) enter_tab: Option<Tab>,
}

pub(crate) fn overlay_state_after_open(tab: Tab) -> OverlayState {
    OverlayState {
        overlay_tab: Some(tab),
        enter_tab: Some(tab),
    }
}

pub(crate) fn overlay_state_after_close() -> OverlayState {
    OverlayState {
        overlay_tab: None,
        enter_tab: None,
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ScreenKeyRoute {
    Welcome,
    Wizard,
    ResumePrompt,
    Main(Tab),
}

pub(crate) fn screen_key_route_for_state(phase: Phase, active_tab: Tab) -> ScreenKeyRoute {
    match phase {
        Phase::Boot(BootScreen::Welcome) => ScreenKeyRoute::Welcome,
        Phase::Boot(BootScreen::Wizard) => ScreenKeyRoute::Wizard,
        Phase::Boot(BootScreen::ResumePrompt) => ScreenKeyRoute::ResumePrompt,
        Phase::Main => ScreenKeyRoute::Main(active_tab),
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum CtrlCAction {
    Quit,
    ArmAndStopRouting,
    ArmAndContinueRouting,
    ClearPending,
}

pub(crate) fn ctrl_c_action_for_key(
    code: KeyCode,
    modifiers: KeyModifiers,
    pending: bool,
    phase: Phase,
) -> CtrlCAction {
    if code != KeyCode::Char('c') || !modifiers.contains(KeyModifiers::CONTROL) {
        return CtrlCAction::ClearPending;
    }

    if pending {
        return CtrlCAction::Quit;
    }

    if matches!(phase, Phase::Main) {
        CtrlCAction::ArmAndStopRouting
    } else {
        CtrlCAction::ArmAndContinueRouting
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ResumePromptAction {
    Accept,
    Decline,
}

pub(crate) fn resume_prompt_action_for_key(code: KeyCode) -> Option<ResumePromptAction> {
    match code {
        KeyCode::Char('y') | KeyCode::Char('Y') | KeyCode::Enter => {
            Some(ResumePromptAction::Accept)
        }
        KeyCode::Char('n') | KeyCode::Char('N') | KeyCode::Esc => Some(ResumePromptAction::Decline),
        _ => None,
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum TabCycle {
    Previous,
    Next,
}

pub(crate) fn tab_cycle_for_key(
    code: KeyCode,
    modifiers: KeyModifiers,
    protect_chat_completion: bool,
) -> Option<TabCycle> {
    if code == KeyCode::Tab && modifiers.is_empty() {
        return (!protect_chat_completion).then_some(TabCycle::Next);
    }

    if code == KeyCode::BackTab {
        return Some(TabCycle::Previous);
    }

    if modifiers.contains(KeyModifiers::CONTROL) {
        match code {
            KeyCode::Left | KeyCode::Char('[') => Some(TabCycle::Previous),
            KeyCode::Right | KeyCode::Char(']') => Some(TabCycle::Next),
            _ => None,
        }
    } else {
        None
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum AutomationView {
    Workflows,
    Triggers,
    Cron,
    Approvals,
}

pub(crate) const AUTOMATION_VIEWS: &[AutomationView] = &[
    AutomationView::Workflows,
    AutomationView::Triggers,
    AutomationView::Cron,
    AutomationView::Approvals,
];

impl AutomationView {
    pub(crate) fn label(self) -> &'static str {
        match self {
            AutomationView::Workflows => "Workflows",
            AutomationView::Triggers => "Triggers",
            AutomationView::Cron => "Cron",
            AutomationView::Approvals => "Approvals",
        }
    }

    pub(crate) fn index(self) -> usize {
        hub_nav::index(AUTOMATION_VIEWS, self)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum LearningView {
    Review,
    SkillProposals,
    Memory,
    Graph,
}

pub(crate) const LEARNING_VIEWS: &[LearningView] = &[
    LearningView::Review,
    LearningView::SkillProposals,
    LearningView::Memory,
    LearningView::Graph,
];

impl LearningView {
    pub(crate) fn label(self) -> &'static str {
        match self {
            LearningView::Review => "Review",
            LearningView::SkillProposals => "Workflows",
            LearningView::Memory => "Memory",
            LearningView::Graph => "Graph",
        }
    }

    pub(crate) fn index(self) -> usize {
        hub_nav::index(LEARNING_VIEWS, self)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum CapabilitiesView {
    Native,
    Skills,
}

pub(crate) const CAPABILITIES_VIEWS: &[CapabilitiesView] =
    &[CapabilitiesView::Native, CapabilitiesView::Skills];

impl CapabilitiesView {
    pub(crate) fn label(self) -> &'static str {
        match self {
            CapabilitiesView::Native => "Natives",
            CapabilitiesView::Skills => "Skills",
        }
    }

    pub(crate) fn index(self) -> usize {
        hub_nav::index(CAPABILITIES_VIEWS, self)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ConnectionsView {
    Channels,
    Extensions,
    Peers,
    Comms,
}

pub(crate) const CONNECTIONS_VIEWS: &[ConnectionsView] = &[
    ConnectionsView::Channels,
    ConnectionsView::Extensions,
    ConnectionsView::Peers,
    ConnectionsView::Comms,
];

impl ConnectionsView {
    pub(crate) fn label(self) -> &'static str {
        match self {
            ConnectionsView::Channels => "Channels",
            ConnectionsView::Extensions => "Extensions",
            ConnectionsView::Peers => "Peers",
            ConnectionsView::Comms => "Comms",
        }
    }

    pub(crate) fn index(self) -> usize {
        hub_nav::index(CONNECTIONS_VIEWS, self)
    }
}

#[cfg(test)]
#[path = "navigation_state/tests.rs"]
mod tests;
