//! Channels screen: setup wizards, test & toggle.

use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::widgets::ListState;

#[path = "channels_draw.rs"]
mod channels_draw;
pub use channels_draw::draw;

// ── Data types ──────────────────────────────────────────────────────────────

#[derive(Clone)]
pub struct ChannelInfo {
    pub name: String,
    pub display_name: String,
    pub category: String,
    pub status: ChannelStatus,
    pub env_vars: Vec<(String, bool)>, // (var_name, is_set)
    pub enabled: bool,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum ChannelStatus {
    Ready,
    MissingEnv,
    NotConfigured,
}

// ── Channel definitions ─────────────────────────────────────────────────────

struct ChannelDef {
    name: &'static str,
    display_name: &'static str,
    category: &'static str,
    env_vars: &'static [&'static str],
    description: &'static str,
}

const CHANNEL_DEFS: &[ChannelDef] = &[
    ChannelDef {
        name: "telegram",
        display_name: "Telegram",
        category: "Messaging",
        env_vars: &["TELEGRAM_BOT_TOKEN"],
        description: "Telegram Bot API adapter",
    },
    ChannelDef {
        name: "discord",
        display_name: "Discord",
        category: "Messaging",
        env_vars: &["DISCORD_BOT_TOKEN"],
        description: "Discord bot adapter",
    },
    ChannelDef {
        name: "signal",
        display_name: "Signal",
        category: "Messaging",
        env_vars: &[],
        description: "Signal via signal-cli REST API",
    },
    ChannelDef {
        name: "email",
        display_name: "Email",
        category: "Messaging",
        env_vars: &["EMAIL_PASSWORD"],
        description: "IMAP inbox + SMTP outbound adapter",
    },
];

const CATEGORIES: &[&str] = &["All", "Messaging"];

// ── State ───────────────────────────────────────────────────────────────────

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ChannelSubScreen {
    List,
    Setup,
    Testing,
}

pub struct ChannelState {
    pub sub: ChannelSubScreen,
    pub channels: Vec<ChannelInfo>,
    pub list_state: ListState,
    pub loading: bool,
    pub tick: usize,
    // Category filter
    pub category_idx: usize,
    // Setup wizard
    pub setup_channel_idx: Option<usize>,
    pub setup_field_idx: usize,
    pub setup_input: String,
    pub setup_values: Vec<(String, String)>, // collected (env_var, value) pairs
    // Test
    pub test_result: Option<(bool, String)>,
    pub status_msg: String,
}

pub enum ChannelAction {
    Continue,
    Refresh,
    TestChannel(String),
    ToggleChannel(String, bool),
    SaveChannel(String, Vec<(String, String)>),
}

impl ChannelState {
    pub fn new() -> Self {
        Self {
            sub: ChannelSubScreen::List,
            channels: Vec::new(),
            list_state: ListState::default(),
            loading: false,
            tick: 0,
            category_idx: 0,
            setup_channel_idx: None,
            setup_field_idx: 0,
            setup_input: String::new(),
            setup_values: Vec::new(),
            test_result: None,
            status_msg: String::new(),
        }
    }

    pub fn tick(&mut self) {
        self.tick = self.tick.wrapping_add(1);
    }

    fn current_category(&self) -> &str {
        CATEGORIES[self.category_idx]
    }

    fn filtered_channels(&self) -> Vec<&ChannelInfo> {
        let cat = self.current_category();
        self.channels
            .iter()
            .filter(|ch| cat == "All" || ch.category == cat)
            .collect()
    }

    fn ready_count(&self) -> usize {
        self.channels
            .iter()
            .filter(|ch| ch.status == ChannelStatus::Ready)
            .count()
    }

    /// Build the default channel list from env var detection.
    pub fn build_default_channels(&mut self) {
        self.channels.clear();
        for def in CHANNEL_DEFS {
            let env_vars: Vec<(String, bool)> = def
                .env_vars
                .iter()
                .map(|v| (v.to_string(), std::env::var(v).is_ok()))
                .collect();
            let all_set = env_vars.is_empty() || env_vars.iter().all(|(_, set)| *set);
            let any_set = env_vars.iter().any(|(_, set)| *set);
            let status = if all_set && !env_vars.is_empty() {
                ChannelStatus::Ready
            } else if any_set {
                ChannelStatus::MissingEnv
            } else {
                ChannelStatus::NotConfigured
            };
            self.channels.push(ChannelInfo {
                name: def.name.to_string(),
                display_name: def.display_name.to_string(),
                category: def.category.to_string(),
                status,
                env_vars,
                enabled: false,
            });
        }
        self.list_state.select(Some(0));
    }

    pub fn handle_key(&mut self, key: KeyEvent) -> ChannelAction {
        if key.code == KeyCode::Char('c') && key.modifiers.contains(KeyModifiers::CONTROL) {
            return ChannelAction::Continue;
        }
        match self.sub {
            ChannelSubScreen::List => self.handle_list(key),
            ChannelSubScreen::Setup => self.handle_setup(key),
            ChannelSubScreen::Testing => self.handle_testing(key),
        }
    }

    fn handle_list(&mut self, key: KeyEvent) -> ChannelAction {
        let filtered = self.filtered_channels();
        let total = filtered.len();
        if total == 0 {
            return self.handle_empty_list_key(key);
        }
        match key.code {
            KeyCode::Up | KeyCode::Char('k') => {
                self.move_list_selection(total, -1);
            }
            KeyCode::Down | KeyCode::Char('j') => {
                self.move_list_selection(total, 1);
            }
            KeyCode::Tab => self.cycle_category(1),
            KeyCode::BackTab => self.cycle_category(-1),
            KeyCode::Enter => self.open_selected_channel_setup(),
            KeyCode::Char('t') => return self.test_selected_channel(),
            KeyCode::Char('e') => return self.toggle_selected_channel(true),
            KeyCode::Char('d') => return self.toggle_selected_channel(false),
            KeyCode::Char('r') => return ChannelAction::Refresh,
            _ => {}
        }
        ChannelAction::Continue
    }

    fn handle_empty_list_key(&mut self, key: KeyEvent) -> ChannelAction {
        match key.code {
            KeyCode::Char('r') => ChannelAction::Refresh,
            KeyCode::Tab => {
                self.cycle_category(1);
                ChannelAction::Continue
            }
            KeyCode::BackTab => {
                self.cycle_category(-1);
                ChannelAction::Continue
            }
            _ => ChannelAction::Continue,
        }
    }

    fn move_list_selection(&mut self, total: usize, delta: isize) {
        let current = self.list_state.selected().unwrap_or(0);
        let next = if delta < 0 {
            if current == 0 {
                total - 1
            } else {
                current - 1
            }
        } else {
            (current + 1) % total
        };
        self.list_state.select(Some(next));
    }

    fn cycle_category(&mut self, delta: isize) {
        self.category_idx = if delta < 0 {
            if self.category_idx == 0 {
                CATEGORIES.len() - 1
            } else {
                self.category_idx - 1
            }
        } else {
            (self.category_idx + 1) % CATEGORIES.len()
        };
        self.list_state.select(Some(0));
    }

    fn selected_channel_name(&self) -> Option<String> {
        let sel = self.list_state.selected()?;
        self.filtered_channels()
            .get(sel)
            .map(|channel| channel.name.clone())
    }

    fn open_selected_channel_setup(&mut self) {
        let Some(ch_name) = self.selected_channel_name() else {
            return;
        };
        let Some(idx) = self.channels.iter().position(|c| c.name == ch_name) else {
            return;
        };
        self.setup_channel_idx = Some(idx);
        self.setup_field_idx = 0;
        self.setup_input.clear();
        self.setup_values.clear();
        self.sub = ChannelSubScreen::Setup;
    }

    fn test_selected_channel(&mut self) -> ChannelAction {
        let Some(name) = self.selected_channel_name() else {
            return ChannelAction::Continue;
        };
        self.test_result = None;
        self.sub = ChannelSubScreen::Testing;
        ChannelAction::TestChannel(name)
    }

    fn toggle_selected_channel(&mut self, enabled: bool) -> ChannelAction {
        let Some(name) = self.selected_channel_name() else {
            return ChannelAction::Continue;
        };
        if let Some(channel) = self.channels.iter_mut().find(|c| c.name == name) {
            channel.enabled = enabled;
        }
        ChannelAction::ToggleChannel(name, enabled)
    }

    fn handle_setup(&mut self, key: KeyEvent) -> ChannelAction {
        match key.code {
            KeyCode::Esc => {
                self.sub = ChannelSubScreen::List;
            }
            KeyCode::Char(c) => {
                self.setup_input.push(c);
            }
            KeyCode::Backspace => {
                self.setup_input.pop();
            }
            KeyCode::Enter => {
                if let Some(idx) = self.setup_channel_idx {
                    if idx < self.channels.len() {
                        let env_vars = &CHANNEL_DEFS
                            .iter()
                            .find(|d| d.name == self.channels[idx].name)
                            .map(|d| d.env_vars)
                            .unwrap_or(&[]);

                        // Save current field value
                        if self.setup_field_idx < env_vars.len() && !self.setup_input.is_empty() {
                            self.setup_values.push((
                                env_vars[self.setup_field_idx].to_string(),
                                self.setup_input.clone(),
                            ));
                        }

                        if self.setup_field_idx + 1 < env_vars.len() {
                            self.setup_field_idx += 1;
                            self.setup_input.clear();
                        } else {
                            // All fields collected — emit save action
                            let name = self.channels[idx].name.clone();
                            let values = self.setup_values.clone();
                            self.sub = ChannelSubScreen::List;
                            if !values.is_empty() {
                                return ChannelAction::SaveChannel(name, values);
                            }
                        }
                    }
                }
            }
            _ => {}
        }
        ChannelAction::Continue
    }

    fn handle_testing(&mut self, key: KeyEvent) -> ChannelAction {
        match key.code {
            KeyCode::Esc | KeyCode::Enter => {
                self.sub = ChannelSubScreen::List;
            }
            _ => {}
        }
        ChannelAction::Continue
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    #[test]
    fn default_list_exposes_only_active_core_channels() {
        let mut state = ChannelState::new();

        state.build_default_channels();

        let names: Vec<&str> = state.channels.iter().map(|ch| ch.name.as_str()).collect();
        assert_eq!(names, vec!["telegram", "discord", "signal", "email"]);
        assert!(!names.contains(&"slack"));
        assert!(!names.contains(&"whatsapp"));
        assert!(!names.contains(&"matrix"));
        assert_eq!(CATEGORIES, ["All", "Messaging"]);
        assert_eq!(state.filtered_channels().len(), 4);
    }

    #[test]
    fn category_tabs_cycle_without_reopening_frozen_groups() {
        let mut state = ChannelState::new();
        state.build_default_channels();

        assert_eq!(state.current_category(), "All");
        assert!(matches!(
            state.handle_key(key(KeyCode::Tab)),
            ChannelAction::Continue
        ));
        assert_eq!(state.current_category(), "Messaging");
        assert_eq!(state.filtered_channels().len(), 4);

        assert!(matches!(
            state.handle_key(key(KeyCode::Tab)),
            ChannelAction::Continue
        ));
        assert_eq!(state.current_category(), "All");
    }

    #[test]
    fn test_selected_channel_enters_testing_state() {
        let mut state = ChannelState::new();
        state.build_default_channels();
        state.list_state.select(Some(0));
        state.test_result = Some((false, "old".to_string()));

        match state.handle_key(key(KeyCode::Char('t'))) {
            ChannelAction::TestChannel(name) => assert_eq!(name, "telegram"),
            _ => panic!("expected TestChannel action"),
        }
        assert_eq!(state.sub, ChannelSubScreen::Testing);
        assert_eq!(state.test_result, None);
    }

    #[test]
    fn toggle_selected_channel_updates_local_state_and_action() {
        let mut state = ChannelState::new();
        state.build_default_channels();
        state.list_state.select(Some(1));

        match state.handle_key(key(KeyCode::Char('e'))) {
            ChannelAction::ToggleChannel(name, enabled) => {
                assert_eq!(name, "discord");
                assert!(enabled);
            }
            _ => panic!("expected enable action"),
        }
        assert!(state.channels[1].enabled);

        match state.handle_key(key(KeyCode::Char('d'))) {
            ChannelAction::ToggleChannel(name, enabled) => {
                assert_eq!(name, "discord");
                assert!(!enabled);
            }
            _ => panic!("expected disable action"),
        }
        assert!(!state.channels[1].enabled);
    }

    #[test]
    fn setup_collects_env_values_and_emits_save_action() {
        let mut state = ChannelState::new();
        state.build_default_channels();
        state.list_state.select(Some(1));

        assert!(matches!(
            state.handle_key(key(KeyCode::Enter)),
            ChannelAction::Continue
        ));
        assert_eq!(state.sub, ChannelSubScreen::Setup);

        for c in "token".chars() {
            assert!(matches!(
                state.handle_key(key(KeyCode::Char(c))),
                ChannelAction::Continue
            ));
        }

        match state.handle_key(key(KeyCode::Enter)) {
            ChannelAction::SaveChannel(name, values) => {
                assert_eq!(name, "discord");
                assert_eq!(
                    values,
                    vec![("DISCORD_BOT_TOKEN".to_string(), "token".to_string())]
                );
            }
            _ => panic!("expected SaveChannel action"),
        }
        assert_eq!(state.sub, ChannelSubScreen::List);
    }
}
