//! Active channel registry exposed by the Captain API.

use crate::channel_registry_email::EMAIL_CHANNEL_META;

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum FieldType {
    Secret,
    Text,
    Number,
    List,
}

impl FieldType {
    fn as_str(self) -> &'static str {
        match self {
            Self::Secret => "secret",
            Self::Text => "text",
            Self::Number => "number",
            Self::List => "list",
        }
    }
}

#[derive(Clone)]
pub(crate) struct ChannelField {
    pub(crate) key: &'static str,
    pub(crate) label: &'static str,
    pub(crate) field_type: FieldType,
    pub(crate) env_var: Option<&'static str>,
    pub(crate) required: bool,
    pub(crate) placeholder: &'static str,
    pub(crate) advanced: bool,
}

pub(crate) struct ChannelMeta {
    pub(crate) name: &'static str,
    pub(crate) display_name: &'static str,
    pub(crate) icon: &'static str,
    pub(crate) description: &'static str,
    pub(crate) difficulty: &'static str,
    pub(crate) setup_time: &'static str,
    pub(crate) quick_setup: &'static str,
    pub(crate) fields: &'static [ChannelField],
    pub(crate) setup_steps: &'static [&'static str],
    pub(crate) operator_notes: &'static [&'static str],
    pub(crate) config_template: &'static str,
}

pub(crate) const FROZEN_CHANNEL_NAMES: &[&str] = &[
    "bluebubbles",
    "dingtalk",
    "feishu",
    "homeassistant",
    "matrix",
    "mattermost",
    "qq",
    "qqbot",
    "slack",
    "sms",
    "wecom",
    "weixin",
    "whatsapp",
];

pub(crate) const CHANNEL_REGISTRY: &[ChannelMeta] = &[
    ChannelMeta {
        name: "telegram",
        display_name: "Telegram",
        icon: "TG",
        description: "Telegram Bot API long-polling adapter",
        difficulty: "Easy",
        setup_time: "~2 min",
        quick_setup: "Paste your bot token from @BotFather",
        fields: &[
            ChannelField {
                key: "bot_token_env",
                label: "Bot Token",
                field_type: FieldType::Secret,
                env_var: Some("TELEGRAM_BOT_TOKEN"),
                required: true,
                placeholder: "123456:ABC-DEF...",
                advanced: false,
            },
            ChannelField {
                key: "allowed_users",
                label: "Allowed User IDs",
                field_type: FieldType::List,
                env_var: None,
                required: true,
                placeholder: "12345, 67890",
                advanced: true,
            },
            ChannelField {
                key: "default_agent",
                label: "Default Agent",
                field_type: FieldType::Text,
                env_var: None,
                required: false,
                placeholder: "assistant",
                advanced: true,
            },
            ChannelField {
                key: "default_chat_id",
                label: "Default Chat ID",
                field_type: FieldType::Text,
                env_var: None,
                required: false,
                placeholder: "552070840",
                advanced: true,
            },
            ChannelField {
                key: "poll_interval_secs",
                label: "Poll Interval (sec)",
                field_type: FieldType::Number,
                env_var: None,
                required: false,
                placeholder: "1",
                advanced: true,
            },
        ],
        setup_steps: &[
            "Open @BotFather on Telegram",
            "Create a bot with /newbot",
            "Paste the token below",
            "Add your Telegram user ID to allowed_users",
        ],
        operator_notes: &[
            "Telegram is deny-by-default: allowed_users must contain explicit user IDs or \"*\".",
            "For groups, BotFather privacy mode can hide messages unless the bot is mentioned or made admin.",
        ],
        config_template:
            "[channels.telegram]\nbot_token_env = \"TELEGRAM_BOT_TOKEN\"\nallowed_users = []",
    },
    ChannelMeta {
        name: "discord",
        display_name: "Discord",
        icon: "DC",
        description: "Discord Gateway bot adapter",
        difficulty: "Easy",
        setup_time: "~3 min",
        quick_setup: "Paste your bot token from the Discord Developer Portal",
        fields: &[
            ChannelField {
                key: "bot_token_env",
                label: "Bot Token",
                field_type: FieldType::Secret,
                env_var: Some("DISCORD_BOT_TOKEN"),
                required: true,
                placeholder: "MTIz...",
                advanced: false,
            },
            ChannelField {
                key: "default_channel_id",
                label: "Default Channel ID",
                field_type: FieldType::Text,
                env_var: None,
                required: false,
                placeholder: "1234567890",
                advanced: false,
            },
            ChannelField {
                key: "allowed_guilds",
                label: "Allowed Guild IDs",
                field_type: FieldType::List,
                env_var: None,
                required: false,
                placeholder: "123456789, 987654321",
                advanced: true,
            },
            ChannelField {
                key: "allowed_users",
                label: "Allowed User IDs",
                field_type: FieldType::List,
                env_var: None,
                required: true,
                placeholder: "123456789, 987654321",
                advanced: true,
            },
            ChannelField {
                key: "default_agent",
                label: "Default Agent",
                field_type: FieldType::Text,
                env_var: None,
                required: false,
                placeholder: "assistant",
                advanced: true,
            },
            ChannelField {
                key: "intents",
                label: "Intents Bitmask",
                field_type: FieldType::Number,
                env_var: None,
                required: false,
                placeholder: "37376",
                advanced: true,
            },
        ],
        setup_steps: &[
            "Open discord.com/developers/applications",
            "Create a bot and copy the token",
            "Paste it below",
            "Enable Message Content Intent and add allowed user IDs",
        ],
        operator_notes: &[
            "Discord is deny-by-default: allowed_users must contain explicit user IDs or \"*\".",
            "Message Content Intent must be enabled in the Discord Developer Portal or messages arrive empty.",
        ],
        config_template:
            "[channels.discord]\nbot_token_env = \"DISCORD_BOT_TOKEN\"\nallowed_users = []",
    },
    ChannelMeta {
        name: "signal",
        display_name: "Signal",
        icon: "SG",
        description: "Signal via signal-cli REST API",
        difficulty: "Medium",
        setup_time: "~10 min",
        quick_setup: "Enter your signal-cli API URL and account number",
        fields: &[
            ChannelField {
                key: "api_url",
                label: "signal-cli API URL",
                field_type: FieldType::Text,
                env_var: None,
                required: true,
                placeholder: "http://localhost:8080",
                advanced: false,
            },
            ChannelField {
                key: "phone_number",
                label: "Account Phone Number",
                field_type: FieldType::Text,
                env_var: None,
                required: true,
                placeholder: "+1234567890",
                advanced: false,
            },
            ChannelField {
                key: "allowed_users",
                label: "Allowed Phone Numbers",
                field_type: FieldType::List,
                env_var: None,
                required: true,
                placeholder: "+1234567890, +1987654321",
                advanced: true,
            },
            ChannelField {
                key: "default_agent",
                label: "Default Agent",
                field_type: FieldType::Text,
                env_var: None,
                required: false,
                placeholder: "assistant",
                advanced: true,
            },
        ],
        setup_steps: &[
            "Install signal-cli and run its REST API",
            "Register or link your account",
            "Enter the API URL and phone number",
            "Add allowed Signal phone numbers or UUIDs",
        ],
        operator_notes: &[
            "Signal is deny-by-default: allowed_users must contain explicit senders or \"*\".",
            "The signal-cli REST API must stay reachable while the adapter is active.",
        ],
        config_template:
            "[channels.signal]\napi_url = \"http://localhost:8080\"\nphone_number = \"\"\nallowed_users = []",
    },
    EMAIL_CHANNEL_META,
];

pub(crate) fn is_channel_configured(
    config: &captain_types::config::ChannelsConfig,
    name: &str,
) -> bool {
    match name {
        "telegram" => config.telegram.is_some(),
        "discord" => config.discord.is_some(),
        "signal" => config.signal.is_some(),
        "email" => config.email.is_some(),
        _ => false,
    }
}

pub(crate) fn find_channel_meta(name: &str) -> Option<&'static ChannelMeta> {
    CHANNEL_REGISTRY.iter().find(|channel| channel.name == name)
}

pub(crate) fn active_channel_names() -> Vec<&'static str> {
    CHANNEL_REGISTRY
        .iter()
        .map(|channel| channel.name)
        .collect()
}

pub(crate) fn is_frozen_channel(name: &str) -> bool {
    FROZEN_CHANNEL_NAMES
        .iter()
        .any(|frozen| frozen.eq_ignore_ascii_case(name))
}

pub(crate) fn channel_config_values(
    config: &captain_types::config::ChannelsConfig,
    name: &str,
) -> Option<serde_json::Value> {
    match name {
        "telegram" => config
            .telegram
            .as_ref()
            .and_then(|value| serde_json::to_value(value).ok()),
        "discord" => config
            .discord
            .as_ref()
            .and_then(|value| serde_json::to_value(value).ok()),
        "signal" => config
            .signal
            .as_ref()
            .and_then(|value| serde_json::to_value(value).ok()),
        "email" => config
            .email
            .as_ref()
            .and_then(|value| serde_json::to_value(value).ok()),
        _ => None,
    }
}

pub(crate) fn build_field_json(
    field: &ChannelField,
    config_values: Option<&serde_json::Value>,
) -> serde_json::Value {
    let env_var = field_env_name(field, config_values);
    let has_secret = env_var
        .as_deref()
        .map(|env| {
            std::env::var(env)
                .map(|value| !value.is_empty())
                .unwrap_or(false)
        })
        .unwrap_or(false);
    let mut result = serde_json::json!({
        "key": field.key,
        "label": field.label,
        "type": field.field_type.as_str(),
        "env_var": env_var,
        "required": field.required,
        "has_value": has_secret,
        "placeholder": field.placeholder,
        "advanced": field.advanced,
    });
    if field.env_var.is_none() {
        if let Some(value) = config_values
            .and_then(|config| config.as_object())
            .and_then(|object| object.get(field.key))
        {
            result["value"] = display_config_value(field.field_type, value);
            result["has_value"] = serde_json::Value::Bool(!value_is_empty(value));
        }
    }
    result
}

pub(crate) fn field_env_name(
    field: &ChannelField,
    config_values: Option<&serde_json::Value>,
) -> Option<String> {
    let default_env = field.env_var?;
    let configured_env = config_values
        .and_then(|config| config.as_object())
        .and_then(|object| object.get(field.key))
        .and_then(|value| value.as_str())
        .map(str::trim)
        .filter(|value| !value.is_empty());
    Some(configured_env.unwrap_or(default_env).to_string())
}

fn display_config_value(field_type: FieldType, value: &serde_json::Value) -> serde_json::Value {
    if field_type == FieldType::List {
        if let Some(items) = value.as_array() {
            return serde_json::Value::String(
                items
                    .iter()
                    .filter_map(|item| item.as_str().map(String::from))
                    .collect::<Vec<_>>()
                    .join(", "),
            );
        }
    }
    value.clone()
}

fn value_is_empty(value: &serde_json::Value) -> bool {
    value.is_null()
        || value.as_str().map(|text| text.is_empty()).unwrap_or(false)
        || value
            .as_array()
            .map(|items| items.is_empty())
            .unwrap_or(false)
        || value
            .as_object()
            .map(|items| items.is_empty())
            .unwrap_or(false)
}

pub(crate) fn field_is_ready(
    field: &ChannelField,
    config_values: Option<&serde_json::Value>,
) -> bool {
    if !field.required {
        return true;
    }
    if let Some(env) = field_env_name(field, config_values) {
        return std::env::var(env)
            .map(|value| !value.is_empty())
            .unwrap_or(false);
    }
    config_values
        .and_then(|config| config.as_object())
        .and_then(|object| object.get(field.key))
        .map(|value| !value_is_empty(value))
        .unwrap_or(false)
}

#[cfg(test)]
#[path = "channel_registry_tests.rs"]
mod tests;
