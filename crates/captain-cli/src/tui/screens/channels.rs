//! Channels screen: live status, schema-driven setup, and connectivity tests.

use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::widgets::ListState;

#[path = "channels_draw.rs"]
mod channels_draw;
pub use channels_draw::draw;

// ── Data types ──────────────────────────────────────────────────────────────

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ChannelInfo {
    pub name: String,
    pub display_name: String,
    pub category: String,
    pub status: ChannelStatus,
    pub description: String,
    pub summary: String,
    pub setup_type: ChannelSetupType,
    pub setup_fields: Vec<ChannelSetupField>,
    pub email_accounts: Vec<ChannelEmailAccount>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ChannelStatus {
    Ready,
    Partial,
    Locked,
    Disabled,
    Invalid,
    NotConfigured,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ChannelSetupType {
    Form,
    EmailAccounts,
    Unavailable,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ChannelFieldType {
    Text,
    Secret,
    Number,
    List,
    Boolean,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ChannelSetupField {
    pub key: String,
    pub label: String,
    pub field_type: ChannelFieldType,
    pub required: bool,
    pub required_for_new: bool,
    pub placeholder: String,
    pub has_value: bool,
    pub value: Option<String>,
    pub request_scope: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ChannelEmailAccount {
    pub alias: String,
    pub credential_ready: bool,
}

// ── Channel definitions ─────────────────────────────────────────────────────

struct ChannelDef {
    name: &'static str,
    display_name: &'static str,
    category: &'static str,
    description: &'static str,
}

const CHANNEL_DEFS: &[ChannelDef] = &[
    ChannelDef {
        name: "telegram",
        display_name: "Telegram",
        category: "Messaging",
        description: "Telegram Bot API adapter",
    },
    ChannelDef {
        name: "discord",
        display_name: "Discord",
        category: "Messaging",
        description: "Discord bot adapter",
    },
    ChannelDef {
        name: "signal",
        display_name: "Signal",
        category: "Messaging",
        description: "Signal via signal-cli REST API",
    },
    ChannelDef {
        name: "email",
        display_name: "Email",
        category: "Messaging",
        description: "Named IMAP inboxes + SMTP outbound adapters",
    },
];

const CATEGORIES: &[&str] = &["All", "Messaging"];

impl ChannelInfo {
    pub fn from_api(value: &serde_json::Value) -> Option<Self> {
        let name = value.get("name")?.as_str()?.trim();
        if name.is_empty() {
            return None;
        }
        let setup_type = match value.get("setup_type").and_then(serde_json::Value::as_str) {
            Some("email_accounts") => ChannelSetupType::EmailAccounts,
            Some("form") | None => ChannelSetupType::Form,
            Some(_) => ChannelSetupType::Unavailable,
        };
        let field_key = if setup_type == ChannelSetupType::EmailAccounts {
            "account_fields"
        } else {
            "fields"
        };
        let setup_fields = value
            .get(field_key)
            .and_then(serde_json::Value::as_array)
            .map(|fields| {
                fields
                    .iter()
                    .filter_map(ChannelSetupField::from_api)
                    .collect()
            })
            .unwrap_or_default();
        let email_accounts = value
            .get("account_summary")
            .and_then(|summary| summary.get("accounts"))
            .and_then(serde_json::Value::as_array)
            .map(|accounts| {
                accounts
                    .iter()
                    .filter_map(ChannelEmailAccount::from_api)
                    .collect()
            })
            .unwrap_or_default();
        let category = value
            .get("category")
            .and_then(serde_json::Value::as_str)
            .map(normalize_category)
            .unwrap_or_else(|| "Messaging".to_string());

        Some(Self {
            name: name.to_string(),
            display_name: value
                .get("display_name")
                .and_then(serde_json::Value::as_str)
                .unwrap_or(name)
                .to_string(),
            category,
            status: channel_status_from_api(value),
            description: value
                .get("description")
                .and_then(serde_json::Value::as_str)
                .unwrap_or_default()
                .to_string(),
            summary: channel_summary_from_api(value, setup_type),
            setup_type,
            setup_fields,
            email_accounts,
        })
    }
}

impl ChannelSetupField {
    fn from_api(value: &serde_json::Value) -> Option<Self> {
        let key = value.get("key")?.as_str()?.trim();
        if key.is_empty() {
            return None;
        }
        let required = value
            .get("required")
            .and_then(serde_json::Value::as_bool)
            .unwrap_or(false);
        let required_for_new = value
            .get("required_for_new_enabled_account")
            .and_then(serde_json::Value::as_bool)
            .unwrap_or(false);
        let advanced = value
            .get("advanced")
            .and_then(serde_json::Value::as_bool)
            .unwrap_or(false);
        if advanced && !required && !required_for_new {
            return None;
        }
        let field_type = match value.get("type").and_then(serde_json::Value::as_str) {
            Some("secret") => ChannelFieldType::Secret,
            Some("number") => ChannelFieldType::Number,
            Some("list") => ChannelFieldType::List,
            Some("boolean") => ChannelFieldType::Boolean,
            _ => ChannelFieldType::Text,
        };
        let field_value = if field_type == ChannelFieldType::Secret {
            None
        } else {
            value
                .get("value")
                .or_else(|| value.get("default"))
                .and_then(json_value_as_input)
        };

        Some(Self {
            key: key.to_string(),
            label: value
                .get("label")
                .and_then(serde_json::Value::as_str)
                .unwrap_or(key)
                .to_string(),
            field_type,
            required,
            required_for_new,
            placeholder: value
                .get("placeholder")
                .and_then(serde_json::Value::as_str)
                .unwrap_or_default()
                .to_string(),
            has_value: value
                .get("has_value")
                .and_then(serde_json::Value::as_bool)
                .unwrap_or(false),
            value: field_value,
            request_scope: value.get("scope").and_then(serde_json::Value::as_str)
                == Some("request"),
        })
    }
}

impl ChannelEmailAccount {
    fn from_api(value: &serde_json::Value) -> Option<Self> {
        Some(Self {
            alias: value.get("alias")?.as_str()?.to_string(),
            credential_ready: value
                .get("credential_ready")
                .and_then(serde_json::Value::as_bool)
                .unwrap_or(false),
        })
    }
}

fn normalize_category(value: &str) -> String {
    let value = value.trim();
    if value.eq_ignore_ascii_case("messaging") {
        "Messaging".to_string()
    } else {
        value.to_string()
    }
}

fn json_value_as_input(value: &serde_json::Value) -> Option<String> {
    match value {
        serde_json::Value::String(value) => Some(value.clone()),
        serde_json::Value::Bool(value) => Some(value.to_string()),
        serde_json::Value::Number(value) => Some(value.to_string()),
        serde_json::Value::Array(values) => Some(
            values
                .iter()
                .filter_map(serde_json::Value::as_str)
                .collect::<Vec<_>>()
                .join(", "),
        ),
        _ => None,
    }
}

fn channel_status_from_api(value: &serde_json::Value) -> ChannelStatus {
    if let Some(state) = value
        .get("operational_state")
        .and_then(serde_json::Value::as_str)
    {
        return match state {
            "ready" => ChannelStatus::Ready,
            "partial" => ChannelStatus::Partial,
            "locked" => ChannelStatus::Locked,
            "disabled" => ChannelStatus::Disabled,
            "invalid" => ChannelStatus::Invalid,
            _ => ChannelStatus::NotConfigured,
        };
    }
    if value.get("ready").and_then(serde_json::Value::as_bool) == Some(true) {
        return ChannelStatus::Ready;
    }
    if value
        .get("configured")
        .and_then(serde_json::Value::as_bool)
        .unwrap_or(false)
    {
        return ChannelStatus::Locked;
    }
    ChannelStatus::NotConfigured
}

fn channel_summary_from_api(value: &serde_json::Value, setup_type: ChannelSetupType) -> String {
    if setup_type == ChannelSetupType::EmailAccounts {
        let summary = value
            .get("account_summary")
            .unwrap_or(&serde_json::Value::Null);
        let total = summary
            .get("total_accounts")
            .and_then(serde_json::Value::as_u64)
            .unwrap_or(0);
        let ready = summary
            .get("ready_accounts")
            .and_then(serde_json::Value::as_u64)
            .unwrap_or(0);
        if total == 0 {
            return "No mailbox connected".to_string();
        }
        let default = summary
            .get("default_account")
            .and_then(serde_json::Value::as_str)
            .map(|alias| format!(" · default {alias}"))
            .unwrap_or_default();
        return format!("{ready}/{total} mailboxes ready{default}");
    }
    if value.get("ready").and_then(serde_json::Value::as_bool) == Some(true) {
        return "Operational".to_string();
    }
    let missing = value
        .get("missing_required_fields")
        .and_then(serde_json::Value::as_array)
        .map(|fields| {
            fields
                .iter()
                .filter_map(serde_json::Value::as_str)
                .take(2)
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    if missing.is_empty() {
        "Not configured".to_string()
    } else {
        format!("Needs {}", missing.join(", "))
    }
}

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
    pub setup_values: Vec<(String, String)>,
    // Test
    pub test_result: Option<(bool, String)>,
    pub status_msg: String,
}

pub enum ChannelAction {
    Continue,
    Refresh,
    TestChannel(String),
    SaveChannel {
        name: String,
        body: serde_json::Value,
    },
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

    /// Build a safe fallback list until the daemon returns live metadata.
    pub fn build_default_channels(&mut self) {
        self.channels.clear();
        for def in CHANNEL_DEFS {
            self.channels.push(ChannelInfo {
                name: def.name.to_string(),
                display_name: def.display_name.to_string(),
                category: def.category.to_string(),
                status: ChannelStatus::NotConfigured,
                description: def.description.to_string(),
                summary: "Waiting for daemon status".to_string(),
                setup_type: ChannelSetupType::Unavailable,
                setup_fields: Vec::new(),
                email_accounts: Vec::new(),
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
        self.setup_values.clear();
        self.status_msg.clear();
        self.load_current_field_value();
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

    fn handle_setup(&mut self, key: KeyEvent) -> ChannelAction {
        match key.code {
            KeyCode::Esc => {
                self.clear_setup_values();
                self.sub = ChannelSubScreen::List;
            }
            KeyCode::Char(c) => {
                self.setup_input.push(c);
            }
            KeyCode::Backspace => {
                self.setup_input.pop();
            }
            KeyCode::Enter => {
                return self.accept_setup_field();
            }
            _ => {}
        }
        ChannelAction::Continue
    }

    fn current_setup_field(&self) -> Option<&ChannelSetupField> {
        let channel = self
            .setup_channel_idx
            .and_then(|index| self.channels.get(index))?;
        channel.setup_fields.get(self.setup_field_idx)
    }

    fn load_current_field_value(&mut self) {
        self.setup_input = self
            .current_setup_field()
            .and_then(|field| field.value.clone())
            .unwrap_or_default();
    }

    fn accept_setup_field(&mut self) -> ChannelAction {
        let Some(field) = self.current_setup_field().cloned() else {
            self.status_msg = "No editable fields were returned by the daemon.".to_string();
            return ChannelAction::Continue;
        };
        let value = self.setup_input.trim().to_string();
        if value.is_empty() && self.field_requires_input(&field) {
            self.status_msg = format!("{} is required.", field.label);
            return ChannelAction::Continue;
        }
        if !value.is_empty() {
            if let Some((_, stored)) = self
                .setup_values
                .iter_mut()
                .find(|(key, _)| key == &field.key)
            {
                *stored = value;
            } else {
                self.setup_values.push((field.key.clone(), value));
            }
        }
        self.status_msg.clear();

        let field_count = self
            .setup_channel_idx
            .and_then(|index| self.channels.get(index))
            .map(|channel| channel.setup_fields.len())
            .unwrap_or(0);
        if self.setup_field_idx + 1 < field_count {
            self.setup_field_idx += 1;
            self.load_current_field_value();
            return ChannelAction::Continue;
        }

        let Some(index) = self.setup_channel_idx else {
            return ChannelAction::Continue;
        };
        let Some(channel) = self.channels.get(index) else {
            return ChannelAction::Continue;
        };
        let name = channel.name.clone();
        let body = match build_channel_configure_body(channel, &self.setup_values) {
            Ok(body) => body,
            Err(error) => {
                self.status_msg = error;
                return ChannelAction::Continue;
            }
        };
        self.clear_setup_values();
        self.sub = ChannelSubScreen::List;
        self.status_msg = format!("Saving {name}...");
        ChannelAction::SaveChannel { name, body }
    }

    pub(crate) fn field_requires_input(&self, field: &ChannelSetupField) -> bool {
        let Some(channel) = self
            .setup_channel_idx
            .and_then(|index| self.channels.get(index))
        else {
            return field.required || field.required_for_new;
        };
        if channel.setup_type != ChannelSetupType::EmailAccounts {
            return field.required && !field.has_value;
        }
        if field.key == "alias" {
            return true;
        }
        let alias = self
            .setup_values
            .iter()
            .find(|(key, _)| key == "alias")
            .map(|(_, value)| value.as_str());
        let existing = alias.and_then(|alias| {
            channel
                .email_accounts
                .iter()
                .find(|account| account.alias == alias)
        });
        match existing {
            Some(account) if field.key == "password" => !account.credential_ready,
            Some(_) => false,
            None => field.required || field.required_for_new,
        }
    }

    fn clear_setup_values(&mut self) {
        self.setup_input.clear();
        for (_, value) in &mut self.setup_values {
            value.clear();
        }
        self.setup_values.clear();
        self.setup_field_idx = 0;
        self.setup_channel_idx = None;
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

fn build_channel_configure_body(
    channel: &ChannelInfo,
    values: &[(String, String)],
) -> Result<serde_json::Value, String> {
    if channel.setup_type == ChannelSetupType::Unavailable {
        return Err("Channel setup requires a connected daemon.".to_string());
    }
    if channel.setup_type == ChannelSetupType::EmailAccounts {
        let mut account = serde_json::Map::new();
        let mut request = serde_json::Map::new();
        for (key, value) in values {
            let field = channel
                .setup_fields
                .iter()
                .find(|field| field.key == *key)
                .ok_or_else(|| format!("Unknown Email setup field '{key}'."))?;
            let typed = typed_email_field_value(field.field_type, value)?;
            if field.request_scope {
                request.insert(key.clone(), typed);
            } else {
                account.insert(key.clone(), typed);
            }
        }
        request.insert("account".to_string(), serde_json::Value::Object(account));
        return Ok(serde_json::Value::Object(request));
    }

    Ok(serde_json::json!({
        "fields": values
            .iter()
            .map(|(key, value)| (key.clone(), serde_json::Value::String(value.clone())))
            .collect::<serde_json::Map<String, serde_json::Value>>()
    }))
}

fn typed_email_field_value(
    field_type: ChannelFieldType,
    value: &str,
) -> Result<serde_json::Value, String> {
    match field_type {
        ChannelFieldType::Number => value
            .parse::<u64>()
            .map(serde_json::Value::from)
            .map_err(|_| format!("'{value}' is not a valid positive number.")),
        ChannelFieldType::Boolean => match value.trim().to_ascii_lowercase().as_str() {
            "true" | "yes" | "y" | "1" => Ok(serde_json::Value::Bool(true)),
            "false" | "no" | "n" | "0" => Ok(serde_json::Value::Bool(false)),
            _ => Err(format!("'{value}' is not a valid boolean.")),
        },
        ChannelFieldType::List => Ok(serde_json::Value::Array(
            value
                .split(',')
                .map(str::trim)
                .filter(|item| !item.is_empty())
                .map(|item| serde_json::Value::String(item.to_string()))
                .collect(),
        )),
        ChannelFieldType::Text | ChannelFieldType::Secret => {
            Ok(serde_json::Value::String(value.to_string()))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::{backend::TestBackend, Terminal};

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
    fn api_channel_parser_keeps_email_account_state_and_quick_fields() {
        let channel = ChannelInfo::from_api(&serde_json::json!({
            "name": "email",
            "display_name": "Email",
            "category": "messaging",
            "configured": true,
            "operational_state": "partial",
            "setup_type": "email_accounts",
            "account_summary": {
                "total_accounts": 2,
                "ready_accounts": 1,
                "default_account": "work"
            },
            "account_fields": [
                {"key": "alias", "label": "Account name", "type": "text", "required": true},
                {"key": "password", "label": "App password", "type": "secret", "required_for_new_enabled_account": true},
                {"key": "imap_port", "type": "number", "advanced": true, "default": 993}
            ]
        }))
        .unwrap();

        assert_eq!(channel.category, "Messaging");
        assert_eq!(channel.status, ChannelStatus::Partial);
        assert_eq!(channel.summary, "1/2 mailboxes ready · default work");
        assert_eq!(channel.setup_fields.len(), 2);
        assert_eq!(channel.setup_fields[1].field_type, ChannelFieldType::Secret);
    }

    #[test]
    fn generic_setup_uses_api_field_keys_and_emits_daemon_body() {
        let mut state = ChannelState::new();
        state.channels = vec![ChannelInfo::from_api(&serde_json::json!({
            "name": "discord",
            "display_name": "Discord",
            "category": "messaging",
            "setup_type": "form",
            "fields": [{
                "key": "bot_token_env",
                "label": "Bot Token",
                "type": "secret",
                "required": true,
                "placeholder": "token"
            }]
        }))
        .unwrap()];
        state.list_state.select(Some(0));

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
            ChannelAction::SaveChannel { name, body } => {
                assert_eq!(name, "discord");
                assert_eq!(body["fields"]["bot_token_env"], "token");
            }
            _ => panic!("expected SaveChannel action"),
        }
        assert_eq!(state.sub, ChannelSubScreen::List);
    }

    #[test]
    fn email_setup_body_types_lists_numbers_and_request_scope() {
        let channel = ChannelInfo::from_api(&serde_json::json!({
            "name": "email",
            "category": "messaging",
            "setup_type": "email_accounts",
            "account_fields": [
                {"key": "alias", "type": "text", "required": true},
                {"key": "allowed_senders", "type": "list", "required": true},
                {"key": "imap_port", "type": "number", "required": true},
                {"key": "make_default", "type": "boolean", "required": true, "scope": "request"}
            ]
        }))
        .unwrap();
        let body = build_channel_configure_body(
            &channel,
            &[
                ("alias".to_string(), "work".to_string()),
                (
                    "allowed_senders".to_string(),
                    "me@example.com, @company.test".to_string(),
                ),
                ("imap_port".to_string(), "993".to_string()),
                ("make_default".to_string(), "yes".to_string()),
            ],
        )
        .unwrap();

        assert_eq!(body["account"]["alias"], "work");
        assert_eq!(body["account"]["imap_port"], 993);
        assert_eq!(
            body["account"]["allowed_senders"].as_array().unwrap().len(),
            2
        );
        assert_eq!(body["make_default"], true);
    }

    #[test]
    fn existing_email_account_can_keep_its_durable_fields_and_credential() {
        let mut state = ChannelState::new();
        state.channels = vec![ChannelInfo::from_api(&serde_json::json!({
            "name": "email",
            "category": "messaging",
            "setup_type": "email_accounts",
            "account_summary": {"accounts": [{
                "alias": "work",
                "address": "work@example.com",
                "credential_ready": true,
                "ready": true,
                "is_default": true
            }]},
            "account_fields": [
                {"key": "alias", "label": "Account name", "type": "text", "required": true},
                {"key": "username", "label": "Mailbox address", "type": "text", "required": true},
                {"key": "password", "label": "App password", "type": "secret", "required_for_new_enabled_account": true}
            ]
        }))
        .unwrap()];
        state.list_state.select(Some(0));
        state.handle_key(key(KeyCode::Enter));
        for character in "work".chars() {
            state.handle_key(key(KeyCode::Char(character)));
        }

        assert!(matches!(
            state.handle_key(key(KeyCode::Enter)),
            ChannelAction::Continue
        ));
        assert!(matches!(
            state.handle_key(key(KeyCode::Enter)),
            ChannelAction::Continue
        ));
        let action = state.handle_key(key(KeyCode::Enter));

        let ChannelAction::SaveChannel { body, .. } = action else {
            panic!("expected SaveChannel action");
        };
        assert_eq!(body["account"]["alias"], "work");
        assert!(body["account"].get("username").is_none());
        assert!(body["account"].get("password").is_none());
    }

    #[test]
    fn channel_screen_renders_on_desktop_and_compact_terminals() {
        for (width, height) in [(100, 26), (52, 18)] {
            let mut state = ChannelState::new();
            state.channels = vec![ChannelInfo::from_api(&serde_json::json!({
                "name": "email",
                "display_name": "Email",
                "category": "messaging",
                "operational_state": "partial",
                "setup_type": "email_accounts",
                "account_summary": {
                    "total_accounts": 2,
                    "ready_accounts": 1,
                    "default_account": "work"
                },
                "account_fields": []
            }))
            .unwrap()];
            state.list_state.select(Some(0));
            let backend = TestBackend::new(width, height);
            let mut terminal = Terminal::new(backend).unwrap();

            terminal
                .draw(|frame| draw(frame, frame.area(), &mut state))
                .unwrap();

            let rendered = terminal
                .backend()
                .buffer()
                .content
                .iter()
                .map(|cell| cell.symbol())
                .collect::<String>();
            assert!(rendered.contains("Channels"));
            assert!(rendered.contains("Email"));
        }
    }

    #[test]
    fn email_setup_render_never_exposes_typed_secret() {
        let mut state = ChannelState::new();
        state.channels = vec![ChannelInfo::from_api(&serde_json::json!({
            "name": "email",
            "display_name": "Email",
            "category": "messaging",
            "setup_type": "email_accounts",
            "account_fields": [
                {"key": "alias", "label": "Account name", "type": "text", "required": true},
                {"key": "password", "label": "App password", "type": "secret", "required_for_new_enabled_account": true}
            ]
        }))
        .unwrap()];
        state.list_state.select(Some(0));
        state.handle_key(key(KeyCode::Enter));
        for character in "work".chars() {
            state.handle_key(key(KeyCode::Char(character)));
        }
        state.handle_key(key(KeyCode::Enter));
        for character in "never-render-this".chars() {
            state.handle_key(key(KeyCode::Char(character)));
        }
        let backend = TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).unwrap();

        terminal
            .draw(|frame| draw(frame, frame.area(), &mut state))
            .unwrap();

        let rendered = terminal
            .backend()
            .buffer()
            .content
            .iter()
            .map(|cell| cell.symbol())
            .collect::<String>();
        assert!(!rendered.contains("never-render-this"));
        assert!(rendered.contains("*****************"));
    }
}
