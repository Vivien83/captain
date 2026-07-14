use std::path::{Path, PathBuf};

use super::service_runtime::{
    launchd_plist_path, systemd_system_service_path, systemd_user_service_path,
    CAPTAIN_LAUNCHD_LABEL, CAPTAIN_TMUX_SESSION,
};
use super::service_runtime::{ServiceRuntime, ServiceSnapshot};
use crate::ui;

pub(crate) fn service_snapshot_json(snapshot: &ServiceSnapshot) -> serde_json::Value {
    serde_json::json!({
        "selected_manager": snapshot.selected.label(),
        "daemon_running": snapshot.daemon_url.is_some(),
        "daemon_url": snapshot.daemon_url,
        "binary": snapshot.binary.display().to_string(),
        "home_dir": snapshot.home_dir.display().to_string(),
        "log_file": snapshot.log_file.display().to_string(),
        "launchd": {
            "installed": snapshot.launchd_installed,
            "loaded": snapshot.launchd_loaded,
            "label": CAPTAIN_LAUNCHD_LABEL,
            "plist": launchd_plist_path().display().to_string(),
        },
        "systemd_user": {
            "installed": snapshot.systemd_user_installed,
            "active": snapshot.systemd_user_active,
            "unit": systemd_user_service_path().display().to_string(),
        },
        "systemd_system": {
            "installed": snapshot.systemd_system_installed,
            "active": snapshot.systemd_system_active,
            "unit": systemd_system_service_path().display().to_string(),
        },
        "tmux": {
            "available": snapshot.tmux_available,
            "active": snapshot.tmux_active,
            "session": CAPTAIN_TMUX_SESSION,
        }
    })
}

pub(crate) fn print_service_snapshot(snapshot: &ServiceSnapshot) {
    ui::section("Captain Service");
    ui::blank();
    ui::kv("Manager", snapshot.selected.label());
    if let Some(url) = &snapshot.daemon_url {
        ui::kv_ok("Daemon", &format!("running at {url}"));
    } else {
        ui::kv_warn("Daemon", "not running");
    }
    ui::kv("Binary", &snapshot.binary.display().to_string());
    ui::kv("Home", &snapshot.home_dir.display().to_string());
    ui::kv("Logs", &snapshot.log_file.display().to_string());

    ui::blank();
    ui::section("Managers");
    ui::kv(
        "launchd",
        &format!(
            "{} / {}",
            installed_text(snapshot.launchd_installed),
            if snapshot.launchd_loaded {
                "loaded"
            } else {
                "not loaded"
            }
        ),
    );
    ui::kv(
        "systemd-u",
        &format!(
            "{} / {}",
            installed_text(snapshot.systemd_user_installed),
            active_text(snapshot.systemd_user_active)
        ),
    );
    ui::kv(
        "systemd-s",
        &format!(
            "{} / {}",
            installed_text(snapshot.systemd_system_installed),
            active_text(snapshot.systemd_system_active)
        ),
    );
    ui::kv(
        "tmux",
        &format!(
            "{} / {}",
            if snapshot.tmux_available {
                "available"
            } else {
                "missing"
            },
            if snapshot.tmux_active {
                "session active"
            } else {
                "session inactive"
            }
        ),
    );
}

pub(crate) fn installed_text(value: bool) -> &'static str {
    if value {
        "installed"
    } else {
        "not installed"
    }
}

fn active_text(value: Option<bool>) -> &'static str {
    match value {
        Some(true) => "active",
        Some(false) => "inactive",
        None => "unknown",
    }
}

pub(super) fn launchd_plist_content(binary: &Path, home_dir: &Path) -> String {
    let log_file = home_dir.join("captain.log");
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>Label</key>
  <string>{CAPTAIN_LAUNCHD_LABEL}</string>
  <key>ProgramArguments</key>
  <array>
    <string>{}</string>
    <string>start</string>
  </array>
  <key>WorkingDirectory</key>
  <string>{}</string>
  <key>EnvironmentVariables</key>
  <dict>
    <key>CAPTAIN_HOME</key>
    <string>{}</string>
  </dict>
  <key>RunAtLoad</key>
  <true/>
  <key>KeepAlive</key>
  <false/>
  <key>StandardOutPath</key>
  <string>{}</string>
  <key>StandardErrorPath</key>
  <string>{}</string>
</dict>
</plist>
"#,
        xml_escape(&binary.display().to_string()),
        xml_escape(&home_dir.display().to_string()),
        xml_escape(&home_dir.display().to_string()),
        xml_escape(&log_file.display().to_string()),
        xml_escape(&log_file.display().to_string())
    )
}

fn service_user_home(home_dir: &Path) -> PathBuf {
    if home_dir.file_name().and_then(|name| name.to_str()) == Some(".captain") {
        if let Some(parent) = home_dir.parent() {
            return parent.to_path_buf();
        }
    }
    dirs::home_dir().unwrap_or_else(|| home_dir.to_path_buf())
}

pub(super) fn systemd_unit_content(
    binary: &Path,
    home_dir: &Path,
    runtime: ServiceRuntime,
) -> String {
    let wanted_by = if runtime == ServiceRuntime::SystemdSystem {
        "multi-user.target"
    } else {
        "default.target"
    };
    let user_home = service_user_home(home_dir);
    let codex_home = user_home.join(".codex");
    format!(
        r#"[Unit]
Description=Captain Agent OS daemon
After=network-online.target
Wants=network-online.target

[Service]
Type=simple
Environment=HOME={}
Environment=CODEX_HOME={}
Environment=CAPTAIN_HOME={}
ExecStart={} start
Restart=on-failure
RestartForceExitStatus=75
RestartSec=5
WorkingDirectory={}

[Install]
WantedBy={}
"#,
        user_home.display(),
        codex_home.display(),
        home_dir.display(),
        binary.display(),
        home_dir.display(),
        wanted_by
    )
}

fn xml_escape(raw: &str) -> String {
    raw.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn service_launchd_plist_runs_captain_start() {
        let plist = launchd_plist_content(
            Path::new("/Users/me/.captain/bin/captain"),
            Path::new("/Users/me/.captain"),
        );
        assert!(plist.contains("<string>ai.captain.daemon</string>"));
        assert!(plist.contains("<string>/Users/me/.captain/bin/captain</string>"));
        assert!(plist.contains("<string>start</string>"));
        assert!(plist.contains("<key>CAPTAIN_HOME</key>"));
        assert!(plist.contains("<key>StandardOutPath</key>"));
    }

    #[test]
    fn service_systemd_unit_has_restart_force_code() {
        let unit = systemd_unit_content(
            Path::new("/home/me/.captain/bin/captain"),
            Path::new("/home/me/.captain"),
            ServiceRuntime::SystemdUser,
        );
        assert!(unit.contains("ExecStart=/home/me/.captain/bin/captain start"));
        assert!(unit.contains("Environment=HOME=/home/me"));
        assert!(unit.contains("Environment=CODEX_HOME=/home/me/.codex"));
        assert!(unit.contains("Environment=CAPTAIN_HOME=/home/me/.captain"));
        assert!(unit.contains("Restart=on-failure"));
        assert!(unit.contains("RestartForceExitStatus=75"));
        assert!(unit.contains("WantedBy=default.target"));

        let system_unit = systemd_unit_content(
            Path::new("/usr/local/bin/captain"),
            Path::new("/var/lib/captain"),
            ServiceRuntime::SystemdSystem,
        );
        assert!(system_unit.contains("WantedBy=multi-user.target"));
    }

    #[test]
    fn service_xml_escape() {
        assert_eq!(
            xml_escape("/tmp/Captain & \"Agent\" <test>"),
            "/tmp/Captain &amp; &quot;Agent&quot; &lt;test&gt;"
        );
    }
}
