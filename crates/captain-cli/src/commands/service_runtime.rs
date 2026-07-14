use std::path::PathBuf;

use crate::{cli_captain_home, find_captain_cli_on_path, find_daemon, ServiceManagerArg};

pub(super) const CAPTAIN_SERVICE_NAME: &str = "captain.service";
pub(super) const CAPTAIN_LAUNCHD_LABEL: &str = "ai.captain.daemon";
pub(super) const CAPTAIN_TMUX_SESSION: &str = "captain-daemon";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ServiceRuntime {
    Launchd,
    SystemdUser,
    SystemdSystem,
    Tmux,
    Background,
}

impl ServiceRuntime {
    pub(crate) fn label(self) -> &'static str {
        match self {
            ServiceRuntime::Launchd => "launchd",
            ServiceRuntime::SystemdUser => "systemd-user",
            ServiceRuntime::SystemdSystem => "systemd-system",
            ServiceRuntime::Tmux => "tmux",
            ServiceRuntime::Background => "background",
        }
    }
}

#[derive(Debug, Clone)]
pub(crate) struct ServiceSnapshot {
    pub(crate) selected: ServiceRuntime,
    pub(crate) daemon_url: Option<String>,
    pub(crate) launchd_installed: bool,
    pub(crate) launchd_loaded: bool,
    pub(crate) systemd_user_installed: bool,
    pub(crate) systemd_user_active: Option<bool>,
    pub(crate) systemd_system_installed: bool,
    pub(crate) systemd_system_active: Option<bool>,
    pub(crate) tmux_available: bool,
    pub(crate) tmux_active: bool,
    pub(crate) binary: PathBuf,
    pub(crate) home_dir: PathBuf,
    pub(crate) log_file: PathBuf,
}

pub(crate) fn service_snapshot(manager: ServiceManagerArg) -> ServiceSnapshot {
    let selected = resolve_control_runtime(manager);
    let tmux_available = command_exists("tmux");
    let tmux_active = tmux_available
        && std::process::Command::new("tmux")
            .args(["has-session", "-t", CAPTAIN_TMUX_SESSION])
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .map(|s| s.success())
            .unwrap_or(false);
    ServiceSnapshot {
        selected,
        daemon_url: find_daemon(),
        launchd_installed: launchd_plist_path().exists(),
        launchd_loaded: launchd_loaded(),
        systemd_user_installed: systemd_user_service_path().exists(),
        systemd_user_active: systemd_active(true),
        systemd_system_installed: systemd_system_service_path().exists(),
        systemd_system_active: systemd_active(false),
        tmux_available,
        tmux_active,
        binary: installed_captain_binary(),
        home_dir: cli_captain_home(),
        log_file: cli_captain_home().join("captain.log"),
    }
}

pub(super) fn resolve_install_runtime(manager: ServiceManagerArg) -> ServiceRuntime {
    match manager {
        ServiceManagerArg::Launchd => ServiceRuntime::Launchd,
        ServiceManagerArg::Systemd => preferred_systemd_runtime(),
        ServiceManagerArg::Tmux => ServiceRuntime::Tmux,
        ServiceManagerArg::Auto => {
            if cfg!(target_os = "macos") {
                ServiceRuntime::Launchd
            } else if cfg!(target_os = "linux") {
                preferred_systemd_runtime()
            } else if command_exists("tmux") {
                ServiceRuntime::Tmux
            } else {
                ServiceRuntime::Background
            }
        }
    }
}

pub(super) fn resolve_control_runtime(manager: ServiceManagerArg) -> ServiceRuntime {
    match manager {
        ServiceManagerArg::Launchd => ServiceRuntime::Launchd,
        ServiceManagerArg::Systemd => {
            if systemd_user_service_path().exists() {
                ServiceRuntime::SystemdUser
            } else {
                ServiceRuntime::SystemdSystem
            }
        }
        ServiceManagerArg::Tmux => ServiceRuntime::Tmux,
        ServiceManagerArg::Auto => {
            if launchd_plist_path().exists() {
                ServiceRuntime::Launchd
            } else if systemd_user_service_path().exists() {
                ServiceRuntime::SystemdUser
            } else if systemd_system_service_path().exists() {
                ServiceRuntime::SystemdSystem
            } else if command_exists("tmux") {
                ServiceRuntime::Tmux
            } else {
                ServiceRuntime::Background
            }
        }
    }
}

fn preferred_systemd_runtime() -> ServiceRuntime {
    if current_uid() == Some(0) {
        ServiceRuntime::SystemdSystem
    } else {
        ServiceRuntime::SystemdUser
    }
}

pub(super) fn wait_for_daemon(timeout: Option<std::time::Duration>) -> Option<String> {
    let deadline =
        std::time::Instant::now() + timeout.unwrap_or(std::time::Duration::from_secs(10));
    loop {
        if let Some(base) = find_daemon() {
            return Some(base);
        }
        if std::time::Instant::now() >= deadline {
            return None;
        }
        std::thread::sleep(std::time::Duration::from_millis(400));
    }
}

pub(super) fn installed_captain_binary() -> PathBuf {
    find_captain_cli_on_path()
        .or_else(|| std::env::current_exe().ok())
        .unwrap_or_else(|| PathBuf::from("captain"))
}

pub(super) fn command_exists(name: &str) -> bool {
    std::process::Command::new(name)
        .arg("--version")
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .is_ok()
}

pub(super) fn run_checked(program: &str, args: &[&str]) -> Result<(), String> {
    let output = std::process::Command::new(program)
        .args(args)
        .output()
        .map_err(|e| format!("Failed to run {program}: {e}"))?;
    if output.status.success() {
        Ok(())
    } else {
        Err(command_failure(program, &output))
    }
}

pub(super) fn stream_command(program: &str, args: &[&str]) {
    match std::process::Command::new(program)
        .args(args)
        .stdin(std::process::Stdio::inherit())
        .stdout(std::process::Stdio::inherit())
        .stderr(std::process::Stdio::inherit())
        .status()
    {
        Ok(status) if status.success() => {}
        Ok(status) => std::process::exit(status.code().unwrap_or(1)),
        Err(e) => {
            crate::ui::error(&format!("Failed to run {program}: {e}"));
            std::process::exit(1);
        }
    }
}

fn command_failure(program: &str, output: &std::process::Output) -> String {
    let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
    let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
    let detail = if !stderr.is_empty() { stderr } else { stdout };
    if detail.is_empty() {
        format!("{program} failed with status {}", output.status)
    } else {
        format!("{program} failed: {detail}")
    }
}

fn current_uid() -> Option<u32> {
    let output = std::process::Command::new("id").arg("-u").output().ok()?;
    if !output.status.success() {
        return None;
    }
    String::from_utf8_lossy(&output.stdout).trim().parse().ok()
}

pub(super) fn launchd_domain() -> Result<String, String> {
    current_uid()
        .map(|uid| format!("gui/{uid}"))
        .ok_or_else(|| "Cannot determine current UID for launchd domain".to_string())
}

pub(super) fn launchd_plist_path() -> PathBuf {
    dirs::home_dir()
        .unwrap_or_else(std::env::temp_dir)
        .join("Library/LaunchAgents")
        .join(format!("{CAPTAIN_LAUNCHD_LABEL}.plist"))
}

pub(super) fn systemd_user_service_path() -> PathBuf {
    if let Some(xdg) = std::env::var_os("XDG_CONFIG_HOME") {
        return PathBuf::from(xdg)
            .join("systemd/user")
            .join(CAPTAIN_SERVICE_NAME);
    }
    dirs::home_dir()
        .unwrap_or_else(std::env::temp_dir)
        .join(".config/systemd/user")
        .join(CAPTAIN_SERVICE_NAME)
}

pub(super) fn systemd_system_service_path() -> PathBuf {
    PathBuf::from("/etc/systemd/system").join(CAPTAIN_SERVICE_NAME)
}

fn launchd_loaded() -> bool {
    let Ok(domain) = launchd_domain() else {
        return false;
    };
    std::process::Command::new("launchctl")
        .args(["print", &format!("{domain}/{CAPTAIN_LAUNCHD_LABEL}")])
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

pub(super) fn service_runtime_active(runtime: ServiceRuntime) -> bool {
    match runtime {
        ServiceRuntime::Launchd => launchd_loaded(),
        ServiceRuntime::SystemdUser => systemd_active(true).unwrap_or(false),
        ServiceRuntime::SystemdSystem => systemd_active(false).unwrap_or(false),
        ServiceRuntime::Tmux => {
            command_exists("tmux")
                && std::process::Command::new("tmux")
                    .args(["has-session", "-t", CAPTAIN_TMUX_SESSION])
                    .stdin(std::process::Stdio::null())
                    .stdout(std::process::Stdio::null())
                    .stderr(std::process::Stdio::null())
                    .status()
                    .map(|s| s.success())
                    .unwrap_or(false)
        }
        ServiceRuntime::Background => false,
    }
}

fn systemd_active(user: bool) -> Option<bool> {
    if !command_exists("systemctl") {
        return None;
    }
    let mut command = std::process::Command::new("systemctl");
    if user {
        command.arg("--user");
    }
    let status = command
        .args(["is-active", "--quiet", CAPTAIN_SERVICE_NAME])
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .ok()?;
    Some(status.success())
}

pub(super) fn tmux_start() -> Result<(), String> {
    if !command_exists("tmux") {
        return Err("tmux is not installed; use `captain service start --manager auto` for background fallback".to_string());
    }
    if std::process::Command::new("tmux")
        .args(["has-session", "-t", CAPTAIN_TMUX_SESSION])
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
    {
        return Ok(());
    }
    let output = std::process::Command::new("tmux")
        .args(["new-session", "-d", "-s", CAPTAIN_TMUX_SESSION])
        .arg(installed_captain_binary())
        .arg("start")
        .output()
        .map_err(|e| format!("Failed to start tmux: {e}"))?;
    if output.status.success() {
        Ok(())
    } else {
        Err(command_failure("tmux", &output))
    }
}
