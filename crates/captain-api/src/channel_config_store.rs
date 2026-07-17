//! TOML persistence helpers for active channel configuration.

use crate::channel_registry::FieldType;
use std::collections::HashMap;

pub(crate) fn upsert_channel_config(
    config_path: &std::path::Path,
    channel_name: &str,
    fields: &HashMap<String, (String, FieldType)>,
) -> Result<(), Box<dyn std::error::Error>> {
    let content = std::fs::read_to_string(config_path).unwrap_or_default();
    let mut doc: toml::Value = if content.trim().is_empty() {
        toml::Value::Table(toml::map::Map::new())
    } else {
        toml::from_str(&content)?
    };
    let root = doc.as_table_mut().ok_or("Config is not a TOML table")?;
    root.entry("channels".to_string())
        .or_insert_with(|| toml::Value::Table(toml::map::Map::new()));
    let channels = root
        .get_mut("channels")
        .and_then(|value| value.as_table_mut())
        .ok_or("channels is not a table")?;
    let mut channel = toml::map::Map::new();
    for (key, (value, field_type)) in fields {
        channel.insert(key.clone(), toml_value(value, *field_type));
    }
    channels.insert(channel_name.to_string(), toml::Value::Table(channel));
    let serialized = toml::to_string_pretty(&doc)?;
    captain_types::durable_fs::atomic_write(config_path, serialized.as_bytes())?;
    Ok(())
}

fn toml_value(value: &str, field_type: FieldType) -> toml::Value {
    match field_type {
        FieldType::Number => value
            .parse::<i64>()
            .map(toml::Value::Integer)
            .unwrap_or_else(|_| toml::Value::String(value.to_string())),
        FieldType::List => toml::Value::Array(
            value
                .split(',')
                .map(str::trim)
                .filter(|item| !item.is_empty())
                .map(|item| toml::Value::String(item.to_string()))
                .collect(),
        ),
        FieldType::Secret | FieldType::Text => toml::Value::String(value.to_string()),
    }
}

pub(crate) fn remove_channel_config(
    config_path: &std::path::Path,
    channel_name: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    if !config_path.exists() {
        return Ok(());
    }
    let content = std::fs::read_to_string(config_path)?;
    if content.trim().is_empty() {
        return Ok(());
    }
    let mut doc: toml::Value = toml::from_str(&content)?;
    if let Some(channels) = doc
        .as_table_mut()
        .and_then(|root| root.get_mut("channels"))
        .and_then(|value| value.as_table_mut())
    {
        channels.remove(channel_name);
    }
    let serialized = toml::to_string_pretty(&doc)?;
    captain_types::durable_fs::atomic_write(config_path, serialized.as_bytes())?;
    Ok(())
}
