use super::daemon::cmd_stop_result;
pub(crate) use super::service_render::{
    installed_text, print_service_snapshot, service_snapshot_json,
};
use super::service_render::{launchd_plist_content, systemd_unit_content};
pub(crate) use super::service_runtime::service_snapshot;
use super::service_runtime::{
    command_exists, installed_captain_binary, launchd_domain, launchd_plist_path,
    resolve_control_runtime, resolve_install_runtime, run_checked, service_runtime_active,
    stream_command, systemd_system_service_path, systemd_user_service_path, tmux_start,
    wait_for_daemon, ServiceRuntime, CAPTAIN_LAUNCHD_LABEL, CAPTAIN_SERVICE_NAME,
    CAPTAIN_TMUX_SESSION,
};
use crate::{
    cli_captain_home, commands, daemon_client, find_daemon, start_daemon_background, ui, LogTarget,
    ServiceManagerArg,
};

pub(crate) fn cmd_service_install(
    manager: ServiceManagerArg,
    force: bool,
    dry_run: bool,
    start: bool,
) {
    let runtime = resolve_install_runtime(manager);
    match runtime {
        ServiceRuntime::Launchd => install_launchd_service(force, dry_run, start),
        ServiceRuntime::SystemdUser | ServiceRuntime::SystemdSystem => {
            install_systemd_service(runtime, force, dry_run, start)
        }
        ServiceRuntime::Tmux | ServiceRuntime::Background => {
            if dry_run {
                println!("manager = {}", runtime.label());
                println!("No persistent service file is written for this fallback.");
                return;
            }
            ui::check_warn("No native service manager selected; using fallback mode.");
            if start {
                cmd_service_start(manager);
            } else {
                ui::hint("Start with: captain service start --manager tmux");
            }
        }
    }
}

pub(crate) fn cmd_service_start(manager: ServiceManagerArg) {
    let runtime = resolve_control_runtime(manager);
    if find_daemon().is_some() {
        if service_runtime_active(runtime) {
            ui::success(&format!(
                "Captain daemon is already running via {}",
                runtime.label()
            ));
        } else {
            ui::check_warn(&format!(
                "Captain daemon is running, but not under {}",
                runtime.label()
            ));
            ui::hint(
                "Stop the current daemon first, then run this command again to migrate managers.",
            );
        }
        return;
    }

    let result = match runtime {
        ServiceRuntime::Launchd => launchd_start(),
        ServiceRuntime::SystemdUser => {
            run_checked("systemctl", &["--user", "start", CAPTAIN_SERVICE_NAME])
        }
        ServiceRuntime::SystemdSystem => run_checked("systemctl", &["start", CAPTAIN_SERVICE_NAME]),
        ServiceRuntime::Tmux => tmux_start(),
        ServiceRuntime::Background => start_daemon_background().map(|_| ()),
    };

    match result {
        Ok(()) => {
            if wait_for_daemon(Some(std::time::Duration::from_secs(12))).is_some() {
                ui::success(&format!("Captain started via {}", runtime.label()));
            } else {
                ui::check_warn(&format!(
                    "Start command sent via {}, but the daemon is not healthy yet",
                    runtime.label()
                ));
            }
        }
        Err(e) => {
            ui::error(&e);
            ui::hint("Run `captain service status` and `captain service logs` for details");
            std::process::exit(1);
        }
    }
}

pub(crate) fn cmd_service_stop(manager: ServiceManagerArg) {
    let _ = service_stop_result(manager);
}

fn service_stop_result(manager: ServiceManagerArg) -> bool {
    if defer_service_control_for_active_work("Stop", "`captain service stop`") {
        return false;
    }

    let runtime = resolve_control_runtime(manager);
    let result = match runtime {
        ServiceRuntime::Launchd => launchd_stop(),
        ServiceRuntime::SystemdUser => {
            run_checked("systemctl", &["--user", "stop", CAPTAIN_SERVICE_NAME])
        }
        ServiceRuntime::SystemdSystem => run_checked("systemctl", &["stop", CAPTAIN_SERVICE_NAME]),
        ServiceRuntime::Tmux => {
            if cmd_stop_result() {
                let _ = std::process::Command::new("tmux")
                    .args(["kill-session", "-t", CAPTAIN_TMUX_SESSION])
                    .output();
            }
            Ok(())
        }
        ServiceRuntime::Background => {
            let _ = cmd_stop_result();
            Ok(())
        }
    };

    match result {
        Ok(()) => {
            for _ in 0..12 {
                if find_daemon().is_none() {
                    ui::success(&format!("Captain stopped via {}", runtime.label()));
                    return true;
                }
                std::thread::sleep(std::time::Duration::from_millis(500));
            }
            ui::check_warn("Stop command sent, but the daemon still answers health checks");
            false
        }
        Err(e) => {
            ui::error(&e);
            std::process::exit(1);
        }
    }
}

pub(crate) fn cmd_service_restart(manager: ServiceManagerArg) {
    if defer_service_control_for_active_work("Restart", "`captain service restart`") {
        return;
    }

    let runtime = resolve_control_runtime(manager);
    let result = match runtime {
        ServiceRuntime::Launchd => launchd_restart(),
        ServiceRuntime::SystemdUser => {
            run_checked("systemctl", &["--user", "restart", CAPTAIN_SERVICE_NAME])
        }
        ServiceRuntime::SystemdSystem => {
            run_checked("systemctl", &["restart", CAPTAIN_SERVICE_NAME])
        }
        ServiceRuntime::Tmux | ServiceRuntime::Background => {
            if !service_stop_result(manager) {
                return;
            }
            std::thread::sleep(std::time::Duration::from_millis(800));
            cmd_service_start(manager);
            return;
        }
    };

    match result {
        Ok(()) => {
            if wait_for_daemon(Some(std::time::Duration::from_secs(15))).is_some() {
                ui::success(&format!("Captain restarted via {}", runtime.label()));
            } else {
                ui::check_warn(&format!(
                    "Restart command sent via {}, but the daemon is not healthy yet",
                    runtime.label()
                ));
            }
        }
        Err(e) => {
            ui::error(&e);
            std::process::exit(1);
        }
    }
}

pub(crate) fn cmd_service_status(json: bool) {
    let snapshot = service_snapshot(ServiceManagerArg::Auto);
    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(&service_snapshot_json(&snapshot)).unwrap_or_default()
        );
        return;
    }

    print_service_snapshot(&snapshot);
}

pub(crate) fn cmd_service_logs(lines: usize, follow: bool) {
    let snapshot = service_snapshot(ServiceManagerArg::Auto);
    match snapshot.selected {
        ServiceRuntime::SystemdUser if command_exists("journalctl") => {
            stream_command(
                "journalctl",
                &[
                    "--user",
                    "-u",
                    CAPTAIN_SERVICE_NAME,
                    "-n",
                    &lines.to_string(),
                    if follow { "-f" } else { "--no-pager" },
                ],
            );
        }
        ServiceRuntime::SystemdSystem if command_exists("journalctl") => {
            stream_command(
                "journalctl",
                &[
                    "-u",
                    CAPTAIN_SERVICE_NAME,
                    "-n",
                    &lines.to_string(),
                    if follow { "-f" } else { "--no-pager" },
                ],
            );
        }
        ServiceRuntime::Tmux if snapshot.tmux_active => {
            if follow {
                ui::hint("Attaching to tmux session. Detach with Ctrl+B then D.");
                stream_command("tmux", &["attach-session", "-t", CAPTAIN_TMUX_SESSION]);
            } else {
                let start = format!("-{lines}");
                stream_command(
                    "tmux",
                    &["capture-pane", "-pt", CAPTAIN_TMUX_SESSION, "-S", &start],
                );
            }
        }
        _ => {
            commands::logs::print_log_file(
                &snapshot.log_file,
                LogTarget::Daemon,
                lines,
                follow,
                None,
                None,
                None,
            );
        }
    }
}

fn install_launchd_service(force: bool, dry_run: bool, start: bool) {
    if !cfg!(target_os = "macos") && !dry_run {
        ui::error("launchd service install is only supported on macOS.");
        std::process::exit(1);
    }
    let path = launchd_plist_path();
    let content = launchd_plist_content(&installed_captain_binary(), &cli_captain_home());
    if dry_run {
        println!("path = {}", path.display());
        println!("{content}");
        return;
    }
    if path.exists() && !force {
        ui::error_with_fix(
            &format!("LaunchAgent already exists: {}", path.display()),
            "Re-run with --force to overwrite",
        );
        std::process::exit(1);
    }
    if let Some(parent) = path.parent() {
        if let Err(e) = std::fs::create_dir_all(parent) {
            ui::error(&format!("Failed to create {}: {e}", parent.display()));
            std::process::exit(1);
        }
    }
    if let Err(e) = std::fs::write(&path, content) {
        ui::error(&format!("Failed to write {}: {e}", path.display()));
        std::process::exit(1);
    }
    ui::success(&format!("Installed launchd service: {}", path.display()));
    if start {
        cmd_service_start(ServiceManagerArg::Launchd);
    } else {
        ui::hint("Start with: captain service start");
    }
}

fn install_systemd_service(runtime: ServiceRuntime, force: bool, dry_run: bool, start: bool) {
    if !cfg!(target_os = "linux") && !dry_run {
        ui::error("systemd service install is only supported on Linux.");
        std::process::exit(1);
    }
    let path = match runtime {
        ServiceRuntime::SystemdSystem => systemd_system_service_path(),
        _ => systemd_user_service_path(),
    };
    let content = systemd_unit_content(&installed_captain_binary(), &cli_captain_home(), runtime);
    if dry_run {
        println!("path = {}", path.display());
        println!("{content}");
        return;
    }
    if path.exists() && !force {
        ui::error_with_fix(
            &format!("Systemd unit already exists: {}", path.display()),
            "Re-run with --force to overwrite",
        );
        std::process::exit(1);
    }
    if let Some(parent) = path.parent() {
        if let Err(e) = std::fs::create_dir_all(parent) {
            ui::error(&format!("Failed to create {}: {e}", parent.display()));
            std::process::exit(1);
        }
    }
    if let Err(e) = std::fs::write(&path, content) {
        ui::error(&format!("Failed to write {}: {e}", path.display()));
        std::process::exit(1);
    }
    let reload = match runtime {
        ServiceRuntime::SystemdSystem => run_checked("systemctl", &["daemon-reload"]),
        _ => run_checked("systemctl", &["--user", "daemon-reload"]),
    };
    if let Err(e) = reload {
        ui::check_warn(&e);
    }
    let enable = match runtime {
        ServiceRuntime::SystemdSystem => {
            run_checked("systemctl", &["enable", CAPTAIN_SERVICE_NAME])
        }
        _ => run_checked("systemctl", &["--user", "enable", CAPTAIN_SERVICE_NAME]),
    };
    if let Err(e) = enable {
        ui::check_warn(&e);
    }
    ui::success(&format!("Installed systemd unit: {}", path.display()));
    if start {
        cmd_service_start(ServiceManagerArg::Systemd);
    } else {
        ui::hint("Start with: captain service start");
    }
}

fn defer_service_control_for_active_work(action_label: &str, retry_command: &str) -> bool {
    let Some((active_runs, active_processes)) = daemon_active_work_counts() else {
        return false;
    };
    if active_runs + active_processes == 0 {
        return false;
    }

    ui::warn_with_fix(
        &service_control_deferred_summary(action_label, active_runs, active_processes),
        &format!(
            "Run `captain status` to inspect active work, then retry {retry_command} after it finishes."
        ),
    );
    true
}

fn daemon_active_work_counts() -> Option<(u64, u64)> {
    let base = find_daemon()?;
    let response = daemon_client()
        .get(format!("{base}/api/status"))
        .send()
        .ok()?;
    if !response.status().is_success() {
        return None;
    }
    let body = response.json::<serde_json::Value>().ok()?;
    let active_runs = body["active_run_count"]
        .as_u64()
        .or_else(|| body["active_runs"].as_array().map(|runs| runs.len() as u64))
        .unwrap_or(0);
    let active_processes = body["active_process_count"].as_u64().unwrap_or_else(|| {
        body["active_processes"]
            .as_array()
            .map(|items| {
                items
                    .iter()
                    .filter(|item| item["alive"].as_bool().unwrap_or(false))
                    .count() as u64
            })
            .unwrap_or(0)
    });
    Some((active_runs, active_processes))
}

fn service_control_deferred_summary(
    action_label: &str,
    active_runs: u64,
    active_processes: u64,
) -> String {
    let total = active_runs + active_processes;
    format!(
        "{action_label} deferred: {total} active work item(s) ({active_runs} run(s), {active_processes} process). Captain will not stop a healthy active task."
    )
}

fn launchd_start() -> Result<(), String> {
    let domain = launchd_domain()?;
    let plist = launchd_plist_path();
    if !plist.exists() {
        return Err(format!(
            "LaunchAgent is not installed at {}",
            plist.display()
        ));
    }
    let _ = std::process::Command::new("launchctl")
        .args(["bootstrap", &domain, &plist.display().to_string()])
        .output();
    run_checked(
        "launchctl",
        &[
            "kickstart",
            "-k",
            &format!("{domain}/{CAPTAIN_LAUNCHD_LABEL}"),
        ],
    )
}

fn launchd_restart() -> Result<(), String> {
    let domain = launchd_domain()?;
    run_checked(
        "launchctl",
        &[
            "kickstart",
            "-k",
            &format!("{domain}/{CAPTAIN_LAUNCHD_LABEL}"),
        ],
    )
}

fn launchd_stop() -> Result<(), String> {
    let domain = launchd_domain()?;
    let target = format!("{domain}/{CAPTAIN_LAUNCHD_LABEL}");
    match run_checked("launchctl", &["bootout", &target]) {
        Ok(()) => Ok(()),
        Err(_) if find_daemon().is_none() => Ok(()),
        Err(bootout_error) => {
            if cmd_stop_result() {
                Ok(())
            } else {
                Err(format!(
                    "Failed to unload launchd service ({bootout_error}) and the daemon did not stop"
                ))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn service_control_deferred_summary_is_operator_safe() {
        let text = service_control_deferred_summary("Restart", 3, 1);

        assert!(text.contains("Restart deferred"));
        assert!(text.contains("4 active work item(s)"));
        assert!(text.contains("3 run(s), 1 process"));
        assert!(text.contains("healthy active task"));
        assert!(!text.contains("agent_id"));
        assert!(!text.contains("prompt"));
    }
}
