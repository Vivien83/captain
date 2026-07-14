use std::path::Path;

use crate::ui;

pub(crate) fn setup_load_answers(path: Option<&Path>) -> Option<toml::Value> {
    let path = path?;
    let raw = std::fs::read_to_string(path).unwrap_or_else(|e| {
        ui::error(&format!(
            "Failed to read answers file {}: {e}",
            path.display()
        ));
        std::process::exit(1);
    });
    Some(raw.parse::<toml::Value>().unwrap_or_else(|e| {
        ui::error(&format!(
            "Failed to parse answers file {}: {e}",
            path.display()
        ));
        std::process::exit(1);
    }))
}

pub(crate) fn setup_answer_value<'a>(
    answers: Option<&'a toml::Value>,
    dotted: &str,
) -> Option<&'a toml::Value> {
    let mut cursor = answers?;
    for part in dotted.split('.') {
        cursor = cursor.get(part)?;
    }
    Some(cursor)
}

pub(crate) fn setup_answer_string_any(
    answers: Option<&toml::Value>,
    paths: &[&str],
) -> Option<String> {
    paths.iter().find_map(|path| {
        setup_answer_value(answers, path)
            .and_then(|value| value.as_str())
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(ToOwned::to_owned)
    })
}

pub(crate) fn setup_env_or_answer_any(
    env_key: &str,
    answers: Option<&toml::Value>,
    paths: &[&str],
) -> Option<String> {
    std::env::var(env_key)
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .or_else(|| setup_answer_string_any(answers, paths))
}

pub(crate) fn setup_csv_or_answer_array(
    env_key: &str,
    answers: Option<&toml::Value>,
    paths: &[&str],
) -> Vec<String> {
    if let Ok(raw) = std::env::var(env_key) {
        let out: Vec<String> = raw
            .split(',')
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(ToOwned::to_owned)
            .collect();
        if !out.is_empty() {
            return out;
        }
    }

    for path in paths {
        let Some(value) = setup_answer_value(answers, path) else {
            continue;
        };
        if let Some(array) = value.as_array() {
            let out: Vec<String> = array
                .iter()
                .filter_map(|item| item.as_str())
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(ToOwned::to_owned)
                .collect();
            if !out.is_empty() {
                return out;
            }
        }
        if let Some(raw) = value.as_str() {
            let out: Vec<String> = raw
                .split(',')
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(ToOwned::to_owned)
                .collect();
            if !out.is_empty() {
                return out;
            }
        }
    }

    Vec::new()
}

pub(crate) fn setup_parse_bool(value: &str) -> Option<bool> {
    match value.trim().to_ascii_lowercase().as_str() {
        "1" | "true" | "yes" | "y" | "on" => Some(true),
        "0" | "false" | "no" | "n" | "off" => Some(false),
        _ => None,
    }
}

pub(crate) fn setup_env_or_answer_bool(
    env_key: &str,
    answers: Option<&toml::Value>,
    paths: &[&str],
) -> Option<bool> {
    std::env::var(env_key)
        .ok()
        .and_then(|value| setup_parse_bool(&value))
        .or_else(|| setup_answer_bool_any(answers, paths))
}

pub(crate) fn setup_read_config_value(captain_dir: &Path) -> Option<toml::Value> {
    let raw = std::fs::read_to_string(captain_dir.join("config.toml")).ok()?;
    raw.parse::<toml::Value>().ok()
}

pub(crate) fn setup_config_string(root: Option<&toml::Value>, dotted: &str) -> Option<String> {
    setup_config_value(root, dotted)?
        .as_str()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
}

pub(crate) fn setup_config_string_array(root: Option<&toml::Value>, dotted: &str) -> Vec<String> {
    setup_config_value(root, dotted)
        .and_then(|value| value.as_array())
        .map(|items| {
            items
                .iter()
                .filter_map(|item| item.as_str())
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(ToOwned::to_owned)
                .collect()
        })
        .unwrap_or_default()
}

pub(crate) fn setup_config_value<'a>(
    root: Option<&'a toml::Value>,
    dotted: &str,
) -> Option<&'a toml::Value> {
    let mut cursor = root?;
    for part in dotted.split('.') {
        cursor = cursor.get(part)?;
    }
    Some(cursor)
}

pub(crate) fn setup_config_bool(root: Option<&toml::Value>, dotted: &str) -> Option<bool> {
    let mut cursor = root?;
    for part in dotted.split('.') {
        cursor = cursor.get(part)?;
    }
    cursor.as_bool()
}

pub(crate) fn setup_secret_env_value(env_name: &str) -> Option<String> {
    std::env::var(env_name)
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

fn setup_answer_bool_any(answers: Option<&toml::Value>, paths: &[&str]) -> Option<bool> {
    paths.iter().find_map(|path| {
        setup_answer_value(answers, path).and_then(|value| {
            value
                .as_bool()
                .or_else(|| value.as_str().and_then(setup_parse_bool))
        })
    })
}
