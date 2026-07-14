pub(crate) use super::config_secrets::{
    cmd_config_delete_key, cmd_config_set_key, cmd_config_test_key,
};
pub(crate) use super::config_workspace::cmd_config_workspace;

use crate::{captain_home, restrict_file_permissions, ui};

pub(crate) fn cmd_config_show() {
    let home = captain_home();
    let config_path = home.join("config.toml");

    if !config_path.exists() {
        println!("No configuration found at: {}", config_path.display());
        println!("Run `captain init` to create one.");
        return;
    }

    let content = std::fs::read_to_string(&config_path).unwrap_or_else(|e| {
        eprintln!("Error reading config: {e}");
        std::process::exit(1);
    });

    println!("# {}\n", config_path.display());
    println!("{content}");
}

pub(crate) fn cmd_config_init_full(force: bool) {
    use captain_types::config_template::render_default_toml_with_header;

    let home = captain_home();
    if !home.exists() {
        if let Err(e) = std::fs::create_dir_all(&home) {
            ui::error(&format!("create {}: {e}", home.display()));
            std::process::exit(1);
        }
    }
    let config_path = home.join("config.toml");

    if config_path.exists() && !force {
        ui::hint(&format!(
            "{} already exists. Use --force (existing file is backed up).",
            config_path.display()
        ));
        return;
    }
    backup_existing_config(&home, &config_path);

    let body = match render_default_toml_with_header() {
        Ok(s) => s,
        Err(e) => {
            ui::error(&format!("render template: {e}"));
            std::process::exit(1);
        }
    };
    if let Err(e) = std::fs::write(&config_path, &body) {
        ui::error(&format!("write {}: {e}", config_path.display()));
        std::process::exit(1);
    }
    ui::success(&format!(
        "Wrote {} ({} lines)",
        config_path.display(),
        body.lines().count()
    ));
}

fn backup_existing_config(home: &std::path::Path, config_path: &std::path::Path) {
    if !config_path.exists() {
        return;
    }
    let ts = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let backup = home.join(format!("config.toml.bak-{ts}"));
    if let Err(e) = std::fs::copy(config_path, &backup) {
        ui::error(&format!("backup failed: {e}"));
        std::process::exit(1);
    }
    ui::kv("Backup", &backup.display().to_string());
}

pub(crate) fn cmd_config_doctor() {
    use captain_types::config_template::scan_missing_top_level_keys;

    let config_path = captain_home().join("config.toml");
    if !config_path.exists() {
        ui::error_with_fix(
            "No config file found",
            "Run `captain config init-full` to create a complete one.",
        );
        std::process::exit(1);
    }
    let content = match std::fs::read_to_string(&config_path) {
        Ok(c) => c,
        Err(e) => {
            ui::error(&format!("read {}: {e}", config_path.display()));
            std::process::exit(1);
        }
    };
    match scan_missing_top_level_keys(&content) {
        Ok(missing) if missing.is_empty() => {
            ui::success("Config is complete - every top-level section is present.");
        }
        Ok(missing) => {
            println!("Missing top-level sections ({}):", missing.len());
            for k in &missing {
                println!("  - {k}");
            }
            ui::hint("Run `captain config reconcile` to append them with their defaults.");
        }
        Err(e) => {
            ui::error(&format!("parse error: {e}"));
            std::process::exit(1);
        }
    }
}

pub(crate) fn cmd_config_schema() {
    use captain_types::config_template::render_default_toml_with_header;
    match render_default_toml_with_header() {
        Ok(s) => print!("{s}"),
        Err(e) => {
            ui::error(&format!("render template: {e}"));
            std::process::exit(1);
        }
    }
}

pub(crate) fn cmd_config_reconcile() {
    use captain_types::config_template::reconcile_file;

    let config_path = captain_home().join("config.toml");
    if !config_path.exists() {
        ui::error_with_fix(
            "No config file found",
            "Run `captain config init-full` first.",
        );
        std::process::exit(1);
    }
    match reconcile_file(&config_path, &crate::cli_runtime::captain_version()) {
        Ok(added) if added.is_empty() => {
            ui::success("Nothing to reconcile - config is already complete.");
        }
        Ok(added) => {
            ui::success(&format!(
                "Added {} missing section{} to {}",
                added.len(),
                if added.len() == 1 { "" } else { "s" },
                config_path.display()
            ));
            for k in &added {
                println!("  + {k}");
            }
        }
        Err(e) => {
            ui::error(&format!("reconcile failed: {e}"));
            std::process::exit(1);
        }
    }
}

pub(crate) fn cmd_config_edit() {
    let config_path = captain_home().join("config.toml");
    let editor = std::env::var("EDITOR")
        .or_else(|_| std::env::var("VISUAL"))
        .unwrap_or_else(|_| {
            if cfg!(windows) {
                "notepad".to_string()
            } else {
                "vi".to_string()
            }
        });

    match std::process::Command::new(&editor)
        .arg(&config_path)
        .status()
    {
        Ok(s) if s.success() => {}
        Ok(s) => eprintln!("Editor exited with: {s}"),
        Err(e) => {
            eprintln!("Failed to open editor '{editor}': {e}");
            eprintln!("Set $EDITOR to your preferred editor.");
        }
    }
}

pub(crate) fn cmd_config_get(key: &str) {
    let table = read_config_table();
    let mut current = &table;
    for part in key.split('.') {
        match current.get(part) {
            Some(v) => current = v,
            None => {
                ui::error(&format!("Key not found: {key}"));
                std::process::exit(1);
            }
        }
    }

    match current {
        toml::Value::String(s) => println!("{s}"),
        toml::Value::Integer(i) => println!("{i}"),
        toml::Value::Float(f) => println!("{f}"),
        toml::Value::Boolean(b) => println!("{b}"),
        other => println!("{other}"),
    }
}

pub(crate) fn cmd_config_set(key: &str, value: &str) {
    let config_path = require_config_path();
    let mut table = read_config_table();
    set_config_value(&mut table, key, value);
    write_config_table(&config_path, &table);
    ui::success(&format!("Set {key} = {value}"));
}

pub(crate) fn cmd_config_unset(key: &str) {
    let config_path = require_config_path();
    let mut table = read_config_table();
    unset_config_value(&mut table, key);
    write_config_table(&config_path, &table);
    ui::success(&format!("Removed key: {key}"));
}

fn require_config_path() -> std::path::PathBuf {
    let config_path = captain_home().join("config.toml");
    if !config_path.exists() {
        ui::error_with_fix("No config file found", "Run `captain init` first");
        std::process::exit(1);
    }
    config_path
}

fn read_config_table() -> toml::Value {
    let config_path = require_config_path();
    let content = std::fs::read_to_string(&config_path).unwrap_or_else(|e| {
        ui::error(&format!("Failed to read config: {e}"));
        std::process::exit(1);
    });
    toml::from_str(&content).unwrap_or_else(|e| {
        ui::error_with_fix(
            &format!("Config parse error: {e}"),
            "Fix your config.toml syntax, or run `captain config edit`",
        );
        std::process::exit(1);
    })
}

fn set_config_value(table: &mut toml::Value, key: &str, value: &str) {
    let parts: Vec<&str> = key.split('.').collect();
    if parts.is_empty() {
        ui::error("Empty key");
        std::process::exit(1);
    }
    let mut current = table;
    for part in &parts[..parts.len() - 1] {
        current = current
            .as_table_mut()
            .and_then(|t| t.get_mut(*part))
            .unwrap_or_else(|| {
                ui::error(&format!("Key path not found: {key}"));
                std::process::exit(1);
            });
    }

    let last_key = parts[parts.len() - 1];
    validate_top_level_scalar(&parts, last_key);
    let tbl = current.as_table_mut().unwrap_or_else(|| {
        ui::error(&format!("Parent of '{key}' is not a table"));
        std::process::exit(1);
    });
    tbl.insert(
        last_key.to_string(),
        infer_config_value(value, tbl.get(last_key)),
    );
}

fn unset_config_value(table: &mut toml::Value, key: &str) {
    let parts: Vec<&str> = key.split('.').collect();
    if parts.is_empty() {
        ui::error("Empty key");
        std::process::exit(1);
    }
    let mut current = table;
    for part in &parts[..parts.len() - 1] {
        current = current
            .as_table_mut()
            .and_then(|t| t.get_mut(*part))
            .unwrap_or_else(|| {
                ui::error(&format!("Key path not found: {key}"));
                std::process::exit(1);
            });
    }

    let last_key = parts[parts.len() - 1];
    let tbl = current.as_table_mut().unwrap_or_else(|| {
        ui::error(&format!("Parent of '{key}' is not a table"));
        std::process::exit(1);
    });
    if tbl.remove(last_key).is_none() {
        ui::error(&format!("Key not found: {key}"));
        std::process::exit(1);
    }
}

fn validate_top_level_scalar(parts: &[&str], last_key: &str) {
    if parts.len() != 1 {
        return;
    }
    let known_scalars = [
        "home_dir",
        "data_dir",
        "log_level",
        "api_listen",
        "network_enabled",
        "api_key",
        "language",
        "max_cron_jobs",
        "usage_footer",
        "workspaces_dir",
    ];
    if !known_scalars.contains(&last_key) {
        ui::error_with_fix(
            &format!("'{last_key}' is a section, not a scalar"),
            &format!("Use dotted notation: {last_key}.field_name"),
        );
        std::process::exit(1);
    }
}

fn infer_config_value(value: &str, existing: Option<&toml::Value>) -> toml::Value {
    if let Some(existing) = existing {
        return match existing {
            toml::Value::Integer(_) => value
                .parse::<u64>()
                .map(|v| toml::Value::Integer(v as i64))
                .or_else(|_| value.parse::<i64>().map(toml::Value::Integer))
                .unwrap_or_else(|_| toml::Value::String(value.to_string())),
            toml::Value::Float(_) => value
                .parse::<f64>()
                .map(toml::Value::Float)
                .unwrap_or_else(|_| toml::Value::String(value.to_string())),
            toml::Value::Boolean(_) => value
                .parse::<bool>()
                .map(toml::Value::Boolean)
                .unwrap_or_else(|_| toml::Value::String(value.to_string())),
            _ => toml::Value::String(value.to_string()),
        };
    }
    if let Ok(b) = value.parse::<bool>() {
        toml::Value::Boolean(b)
    } else if let Ok(i) = value.parse::<u64>() {
        toml::Value::Integer(i as i64)
    } else if let Ok(i) = value.parse::<i64>() {
        toml::Value::Integer(i)
    } else if let Ok(f) = value.parse::<f64>() {
        toml::Value::Float(f)
    } else {
        toml::Value::String(value.to_string())
    }
}

fn write_config_table(config_path: &std::path::Path, table: &toml::Value) {
    let serialized = toml::to_string_pretty(table).unwrap_or_else(|e| {
        ui::error(&format!("Failed to serialize config: {e}"));
        std::process::exit(1);
    });
    let _ = std::fs::copy(config_path, config_path.with_extension("toml.bak"));
    std::fs::write(config_path, &serialized).unwrap_or_else(|e| {
        ui::error(&format!("Failed to write config: {e}"));
        std::process::exit(1);
    });
    restrict_file_permissions(config_path);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn infer_config_value_preserves_existing_bool_type() {
        assert_eq!(
            infer_config_value("true", Some(&toml::Value::Boolean(false))),
            toml::Value::Boolean(true)
        );
    }

    #[test]
    fn infer_config_value_guesses_integer_without_existing_value() {
        assert_eq!(infer_config_value("42", None), toml::Value::Integer(42));
    }
}
