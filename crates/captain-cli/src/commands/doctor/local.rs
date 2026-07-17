use captain_types::agent::AgentManifest;

use crate::{
    captain_version, cli_captain_home, command_version, detect_best_provider,
    find_captain_cli_on_path, find_daemon, path_eq_best_effort, prompt_input,
    restrict_dir_permissions, restrict_file_permissions, ui, ServiceManagerArg,
};

use super::DoctorReport;

pub(super) fn check_cli_install(report: &mut DoctorReport) {
    let current_exe = std::env::current_exe().ok();
    match find_captain_cli_on_path() {
        Some(path_cli) => {
            let version = command_version(&path_cli);
            let same_binary = current_exe
                .as_deref()
                .map(|exe| path_eq_best_effort(exe, &path_cli))
                .unwrap_or(false);
            let version_ok = version
                .as_deref()
                .map(|v| {
                    let current_version = captain_version();
                    v.contains(&current_version)
                })
                .unwrap_or(false);

            if version.is_some() && (same_binary || version_ok) {
                if !report.json {
                    ui::check_ok(&format!("CLI on PATH: {}", path_cli.display()));
                }
                report.push(serde_json::json!({
                    "check": "cli_installed",
                    "status": "ok",
                    "path": path_cli.display().to_string(),
                    "version": version,
                    "current_exe": current_exe.as_ref().map(|p| p.display().to_string()),
                    "same_binary": same_binary,
                }));
            } else {
                if !report.json {
                    ui::check_warn(&format!(
                        "CLI on PATH may be stale or broken: {}",
                        path_cli.display()
                    ));
                    ui::hint("Reinstall with: curl -fsSL https://captain.sh/install | sh");
                }
                report.push(serde_json::json!({
                    "check": "cli_installed",
                    "status": "warn",
                    "path": path_cli.display().to_string(),
                    "version": version,
                    "current_exe": current_exe.as_ref().map(|p| p.display().to_string()),
                    "same_binary": same_binary,
                }));
            }
        }
        None => {
            if !report.json {
                ui::check_fail("Captain CLI is not installed on PATH.");
                ui::hint("Install it with: curl -fsSL https://captain.sh/install | sh");
            }
            report.push(serde_json::json!({
                "check": "cli_installed",
                "status": "fail",
                "current_exe": current_exe.as_ref().map(|p| p.display().to_string()),
            }));
            report.fail();
        }
    }
}

pub(super) fn check_home(report: &mut DoctorReport) {
    let captain_dir = cli_captain_home();

    check_captain_dir(report, &captain_dir);
    check_env_file(report, &captain_dir);
    check_config_file(report, &captain_dir);
    check_daemon_and_port(report, &captain_dir);
    check_database(report, &captain_dir);
    check_disk_space(report, &captain_dir);
    check_agent_manifests(report, &captain_dir);
}

fn check_captain_dir(report: &mut DoctorReport, captain_dir: &std::path::Path) {
    if captain_dir.exists() {
        if !report.json {
            ui::check_ok(&format!("Captain directory: {}", captain_dir.display()));
        }
        report.push(serde_json::json!({"check": "captain_dir", "status": "ok", "path": captain_dir.display().to_string()}));
    } else if report.repair {
        if !report.json {
            ui::check_fail("Captain directory not found.");
        }
        let answer = prompt_input("    Create it now? [Y/n] ");
        let should_create = answer.is_empty() || answer.starts_with('y') || answer.starts_with('Y');
        if should_create && std::fs::create_dir_all(captain_dir).is_ok() {
            restrict_dir_permissions(captain_dir);
            for sub in ["data", "agents"] {
                let _ = std::fs::create_dir_all(captain_dir.join(sub));
            }
            if !report.json {
                ui::check_ok("Created Captain directory");
            }
            report.mark_repaired();
            report.push(serde_json::json!({"check": "captain_dir", "status": "repaired"}));
        } else {
            if should_create && !report.json {
                ui::check_fail("Failed to create directory");
            }
            report.push(serde_json::json!({"check": "captain_dir", "status": "fail"}));
            report.fail();
        }
    } else {
        if !report.json {
            ui::check_fail("Captain directory not found. Run `captain init` first.");
        }
        report.push(serde_json::json!({"check": "captain_dir", "status": "fail"}));
        report.fail();
    }
}

fn check_env_file(report: &mut DoctorReport, captain_dir: &std::path::Path) {
    let env_path = captain_dir.join(".env");
    if env_path.exists() {
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            if let Ok(meta) = std::fs::metadata(&env_path) {
                let mode = meta.permissions().mode() & 0o777;
                if mode == 0o600 {
                    if !report.json {
                        ui::check_ok(".env file (permissions OK)");
                    }
                } else if report.repair {
                    let _ =
                        std::fs::set_permissions(&env_path, std::fs::Permissions::from_mode(0o600));
                    if !report.json {
                        ui::check_ok(".env file (permissions fixed to 0600)");
                    }
                    report.mark_repaired();
                } else if !report.json {
                    ui::check_warn(&format!(
                        ".env file has loose permissions ({:o}), should be 0600",
                        mode
                    ));
                }
            } else if !report.json {
                ui::check_ok(".env file");
            }
        }
        #[cfg(not(unix))]
        if !report.json {
            ui::check_ok(".env file");
        }
        report.push(serde_json::json!({"check": "env_file", "status": "ok"}));
    } else {
        if !report.json {
            ui::check_warn(".env file not found (create with: captain config set-key <provider>)");
        }
        report.push(serde_json::json!({"check": "env_file", "status": "warn"}));
    }
}

fn check_config_file(report: &mut DoctorReport, captain_dir: &std::path::Path) {
    let config_path = captain_dir.join("config.toml");
    if config_path.exists() {
        let config_content = std::fs::read_to_string(&config_path).unwrap_or_default();
        match toml::from_str::<toml::Value>(&config_content) {
            Ok(_) => {
                if !report.json {
                    ui::check_ok(&format!("Config file: {}", config_path.display()));
                }
                report.push(serde_json::json!({"check": "config_file", "status": "ok"}));
            }
            Err(e) => {
                if !report.json {
                    ui::check_fail(&format!("Config file has syntax errors: {e}"));
                    ui::hint("Fix with: captain config edit");
                }
                report.push(serde_json::json!({"check": "config_syntax", "status": "fail", "error": e.to_string()}));
                report.fail();
            }
        }
    } else if report.repair {
        repair_config_file(report, &config_path);
    } else {
        if !report.json {
            ui::check_fail("Config file not found.");
        }
        report.push(serde_json::json!({"check": "config_file", "status": "fail"}));
        report.fail();
    }
}

fn repair_config_file(report: &mut DoctorReport, config_path: &std::path::Path) {
    if !report.json {
        ui::check_fail("Config file not found.");
    }
    let answer = prompt_input("    Create default config? [Y/n] ");
    let should_create = answer.is_empty() || answer.starts_with('y') || answer.starts_with('Y');
    if !should_create {
        report.push(serde_json::json!({"check": "config_file", "status": "fail"}));
        report.fail();
        return;
    }

    let (provider, api_key_env, model) = detect_best_provider();
    let default_config = format!(
        r#"# Captain Agent OS configuration
# See https://captain.sh/docs for documentation

# For VPS/container installs, change to "0.0.0.0:50051" or set CAPTAIN_LISTEN.
api_listen = "127.0.0.1:50051"

[default_model]
provider = "{provider}"
model = "{model}"
api_key_env = "{api_key_env}"

[memory]
decay_rate = 0.05
"#
    );
    if captain_types::durable_fs::atomic_write(config_path, default_config.as_bytes()).is_ok() {
        restrict_file_permissions(config_path);
        if !report.json {
            ui::check_ok("Created default config.toml");
        }
        report.mark_repaired();
        report.push(serde_json::json!({"check": "config_file", "status": "repaired"}));
    } else {
        if !report.json {
            ui::check_fail("Failed to create config.toml");
        }
        report.push(serde_json::json!({"check": "config_file", "status": "fail"}));
        report.fail();
    }
}

fn check_daemon_and_port(report: &mut DoctorReport, captain_dir: &std::path::Path) {
    let api_listen = read_api_listen(captain_dir);
    if !report.json {
        println!();
    }

    let daemon_running = find_daemon();
    if let Some(ref base) = daemon_running {
        if !report.json {
            ui::check_ok(&format!("Daemon running at {base}"));
        }
        report.push(serde_json::json!({"check": "daemon", "status": "ok", "url": base}));
    } else {
        if !report.json {
            ui::check_warn("Daemon not running (start with `captain start`)");
        }
        report.push(serde_json::json!({"check": "daemon", "status": "warn"}));
        check_port(report, &api_listen);
    }

    let daemon_json_path = captain_dir.join("daemon.json");
    if daemon_json_path.exists() && daemon_running.is_none() {
        if report.repair {
            let _ = captain_types::durable_fs::remove_file(&daemon_json_path);
            if !report.json {
                ui::check_ok("Removed stale daemon.json");
            }
            report.mark_repaired();
        } else if !report.json {
            ui::check_warn(
                "Stale daemon.json found (daemon not running). Run with --repair to clean up.",
            );
        }
        report.push(serde_json::json!({"check": "stale_daemon_json", "status": if report.repair { "repaired" } else { "warn" }}));
    }

    if report.full {
        check_service_lifecycle(report);
    }
}

fn read_api_listen(captain_dir: &std::path::Path) -> String {
    let cfg_path = captain_dir.join("config.toml");
    if !cfg_path.exists() {
        return "127.0.0.1:50051".to_string();
    }
    std::fs::read_to_string(&cfg_path)
        .ok()
        .and_then(|s| toml::from_str::<captain_types::config::KernelConfig>(&s).ok())
        .map(|c| c.api_listen)
        .unwrap_or_else(|| "127.0.0.1:50051".to_string())
}

fn check_port(report: &mut DoctorReport, api_listen: &str) {
    let bind_addr = if api_listen.starts_with("0.0.0.0") {
        api_listen.replacen("0.0.0.0", "127.0.0.1", 1)
    } else {
        api_listen.to_string()
    };
    match std::net::TcpListener::bind(&bind_addr) {
        Ok(_) => {
            if !report.json {
                ui::check_ok(&format!("Port {api_listen} is available"));
            }
            report
                .push(serde_json::json!({"check": "port", "status": "ok", "address": api_listen}));
        }
        Err(_) => {
            if !report.json {
                ui::check_warn(&format!("Port {api_listen} is in use by another process"));
            }
            report.push(
                serde_json::json!({"check": "port", "status": "warn", "address": api_listen}),
            );
        }
    }
}

fn check_service_lifecycle(report: &mut DoctorReport) {
    if !report.json {
        println!("\n  Service Lifecycle:");
    }
    let snapshot = crate::commands::service::service_snapshot(ServiceManagerArg::Auto);
    if !report.json {
        if snapshot.daemon_url.is_some() {
            ui::check_ok(&format!(
                "Daemon controlled by {}",
                snapshot.selected.label()
            ));
        } else {
            ui::check_warn(&format!(
                "Daemon not running; selected manager is {}",
                snapshot.selected.label()
            ));
        }
        ui::check_ok(&format!("Binary: {}", snapshot.binary.display()));
        ui::check_ok(&format!("Home: {}", snapshot.home_dir.display()));
        ui::check_ok(&format!(
            "Managers: launchd={}, systemd-user={}, systemd-system={}, tmux={}",
            crate::commands::service::installed_text(snapshot.launchd_installed),
            crate::commands::service::installed_text(snapshot.systemd_user_installed),
            crate::commands::service::installed_text(snapshot.systemd_system_installed),
            if snapshot.tmux_available {
                "available"
            } else {
                "missing"
            }
        ));
    }
    report.push(serde_json::json!({
        "check": "service_lifecycle",
        "status": if snapshot.daemon_url.is_some() { "ok" } else { "warn" },
        "service": crate::commands::service::service_snapshot_json(&snapshot),
    }));
}

fn check_database(report: &mut DoctorReport, captain_dir: &std::path::Path) {
    let db_path = captain_dir.join("data").join("captain.db");
    if db_path.exists() {
        if let Ok(bytes) = std::fs::read(&db_path) {
            if bytes.len() >= 16 && bytes.starts_with(b"SQLite format 3") {
                if !report.json {
                    ui::check_ok("Database file (valid SQLite)");
                }
                report.push(serde_json::json!({"check": "database", "status": "ok"}));
            } else {
                if !report.json {
                    ui::check_fail("Database file exists but is not valid SQLite");
                }
                report.push(serde_json::json!({"check": "database", "status": "fail"}));
                report.fail();
            }
        }
    } else {
        if !report.json {
            ui::check_warn("No database file (will be created on first run)");
        }
        report.push(serde_json::json!({"check": "database", "status": "warn"}));
    }
}

fn check_disk_space(report: &mut DoctorReport, captain_dir: &std::path::Path) {
    #[cfg(unix)]
    if let Ok(output) = std::process::Command::new("df")
        .args(["-m", &captain_dir.display().to_string()])
        .output()
    {
        let stdout = String::from_utf8_lossy(&output.stdout);
        let Some(line) = stdout.lines().nth(1) else {
            return;
        };
        let cols: Vec<&str> = line.split_whitespace().collect();
        if cols.len() < 4 {
            return;
        }
        let Ok(available_mb) = cols[3].parse::<u64>() else {
            return;
        };
        if available_mb < 100 {
            if !report.json {
                ui::check_warn(&format!("Low disk space: {available_mb}MB available"));
            }
            report.push(serde_json::json!({"check": "disk_space", "status": "warn", "available_mb": available_mb}));
        } else {
            if !report.json {
                ui::check_ok(&format!("Disk space: {available_mb}MB available"));
            }
            report.push(serde_json::json!({"check": "disk_space", "status": "ok", "available_mb": available_mb}));
        }
    }
    #[cfg(not(unix))]
    {
        let _ = (report, captain_dir);
    }
}

fn check_agent_manifests(report: &mut DoctorReport, captain_dir: &std::path::Path) {
    let agents_dir = captain_dir.join("agents");
    if !agents_dir.exists() {
        return;
    }
    let mut agent_errors = Vec::new();
    if let Ok(entries) = std::fs::read_dir(&agents_dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) == Some("toml") {
                if let Ok(content) = std::fs::read_to_string(&path) {
                    if let Err(e) = toml::from_str::<AgentManifest>(&content) {
                        agent_errors.push((
                            path.file_name()
                                .unwrap_or_default()
                                .to_string_lossy()
                                .to_string(),
                            e.to_string(),
                        ));
                    }
                }
            }
        }
    }

    if agent_errors.is_empty() {
        if !report.json {
            ui::check_ok("Agent manifests are valid");
        }
        report.push(serde_json::json!({"check": "agent_manifests", "status": "ok"}));
    } else {
        for (file, err) in &agent_errors {
            if !report.json {
                ui::check_fail(&format!("Invalid manifest {file}: {err}"));
            }
        }
        report.push(serde_json::json!({"check": "agent_manifests", "status": "fail", "errors": agent_errors.len()}));
        report.fail();
    }
}
