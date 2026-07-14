use std::path::{Path, PathBuf};

use captain_kernel::CaptainKernel;

use crate::{boot_kernel_error, cli_captain_home, daemon_client, find_daemon, ui};

pub(crate) fn cmd_start(config: Option<PathBuf>, yolo: bool) {
    if let Some(base) = find_daemon() {
        ui::error_with_fix(
            &format!("Daemon already running at {base}"),
            "Use `captain status` to check it, or stop it first",
        );
        std::process::exit(1);
    }

    ui::banner();
    ui::blank();
    println!("  Starting daemon...");
    ui::blank();

    let rt = tokio::runtime::Runtime::new().unwrap();
    rt.block_on(async {
        let mut kernel_config = captain_kernel::config::load_config(config.as_deref());
        set_daemon_working_directory(&kernel_config.home_dir);
        if yolo {
            kernel_config.approval.auto_approve = true;
            kernel_config.approval.apply_shorthands();
        }
        let kernel = match CaptainKernel::boot_with_config(kernel_config) {
            Ok(k) => k,
            Err(e) => {
                boot_kernel_error(&e);
                std::process::exit(1);
            }
        };

        let listen_addr = kernel.config.api_listen.clone();
        let daemon_info_path = kernel.config.home_dir.join("daemon.json");
        let provider = kernel.config.default_model.provider.clone();
        let model = kernel.config.default_model.model.clone();
        let agent_count = kernel.registry.count();
        let model_count = kernel
            .model_catalog
            .read()
            .map(|c| c.list_models().len())
            .unwrap_or(0);

        ui::success(&format!("Kernel booted ({provider}/{model})"));
        if model_count > 0 {
            ui::success(&format!("{model_count} models available"));
        }
        if agent_count > 0 {
            ui::success(&format!("{agent_count} agent(s) loaded"));
        }
        ui::blank();
        ui::kv("API", &format!("http://{listen_addr}"));
        ui::kv("Web terminal", &format!("http://{listen_addr}/terminal"));
        ui::kv("Provider", &provider);
        ui::kv("Model", &model);
        ui::blank();
        ui::hint("Open the web terminal in your browser, or run `captain chat`");
        ui::hint("Press Ctrl+C to stop the daemon");
        ui::blank();

        if let Err(e) =
            captain_api::server::run_daemon(kernel, &listen_addr, Some(&daemon_info_path)).await
        {
            ui::error(&format!("Daemon error: {e}"));
            std::process::exit(1);
        }

        ui::blank();
        println!("  Captain daemon stopped.");
    });
}

pub(crate) fn cmd_stop() {
    let _ = cmd_stop_result();
}

pub(crate) fn cmd_stop_result() -> bool {
    match find_daemon() {
        Some(base) => {
            let client = daemon_client();
            match client.post(format!("{base}/api/shutdown")).send() {
                Ok(r) if r.status().is_success() => {
                    let body = r.json::<serde_json::Value>().unwrap_or_default();
                    if shutdown_is_draining(&body) {
                        ui::warn_with_fix(
                            &shutdown_drain_summary(&body),
                            "Run `captain status` to inspect active work, then retry `captain stop` after it finishes.",
                        );
                        return false;
                    }
                    for _ in 0..10 {
                        std::thread::sleep(std::time::Duration::from_millis(500));
                        if find_daemon().is_none() {
                            ui::success("Daemon stopped");
                            return true;
                        }
                    }
                    ui::warn_with_fix(
                        "Daemon is still responding after shutdown request; not forcing a healthy process.",
                        "Run `captain status` to inspect active work, then retry `captain stop` later.",
                    );
                    false
                }
                Ok(r) => {
                    ui::error(&format!("Shutdown request failed ({})", r.status()));
                    false
                }
                Err(e) => {
                    ui::error(&format!("Could not reach daemon: {e}"));
                    false
                }
            }
        }
        None => {
            ui::warn_with_fix(
                "No running daemon found",
                "Is it running? Check with: captain status",
            );
            true
        }
    }
}

fn shutdown_is_draining(body: &serde_json::Value) -> bool {
    body["status"] == "draining"
}

fn shutdown_drain_summary(body: &serde_json::Value) -> String {
    let active = body["active_work_count"]
        .as_u64()
        .unwrap_or_else(|| body["active_run_count"].as_u64().unwrap_or(0));
    let processes = body["active_process_count"].as_u64().unwrap_or(0);
    format!(
        "Daemon is draining {active} active work item(s), including {processes} background process(es); Captain will not stop healthy active work."
    )
}

pub(crate) fn start_daemon_background() -> Result<String, String> {
    let exe = std::env::current_exe().map_err(|e| format!("Cannot find executable: {e}"))?;

    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const DETACHED_PROCESS: u32 = 0x00000008;
        const CREATE_NEW_PROCESS_GROUP: u32 = 0x00000200;
        let daemon_cwd = cli_captain_home();
        let _ = std::fs::create_dir_all(&daemon_cwd);
        std::process::Command::new(&exe)
            .arg("start")
            .current_dir(&daemon_cwd)
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .creation_flags(DETACHED_PROCESS | CREATE_NEW_PROCESS_GROUP)
            .spawn()
            .map_err(|e| format!("Failed to spawn daemon: {e}"))?;
    }

    #[cfg(not(windows))]
    {
        let daemon_cwd = cli_captain_home();
        let _ = std::fs::create_dir_all(&daemon_cwd);
        std::process::Command::new(&exe)
            .arg("start")
            .current_dir(&daemon_cwd)
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
            .map_err(|e| format!("Failed to spawn daemon: {e}"))?;
    }

    for _ in 0..20 {
        std::thread::sleep(std::time::Duration::from_millis(500));
        if let Some(url) = find_daemon() {
            return Ok(url);
        }
    }

    Err("Daemon did not become ready within 10 seconds".to_string())
}

fn set_daemon_working_directory(home_dir: &Path) {
    if let Err(e) = std::fs::create_dir_all(home_dir) {
        ui::warn_with_fix(
            &format!(
                "Impossible de créer le cwd daemon {} : {e}",
                home_dir.display()
            ),
            "Le daemon continuera avec le répertoire courant du shell",
        );
        return;
    }
    if let Err(e) = std::env::set_current_dir(home_dir) {
        ui::warn_with_fix(
            &format!(
                "Impossible de placer le daemon dans {} : {e}",
                home_dir.display()
            ),
            "Lance `captain start` depuis ~/.captain si le contexte local affiché est incorrect",
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shutdown_drain_summary_uses_counts_only() {
        let body = serde_json::json!({
            "status": "draining",
            "active_work_count": 3,
            "active_run_count": 2,
            "active_process_count": 1,
            "prompt": "private prompt"
        });

        let summary = shutdown_drain_summary(&body);

        assert!(shutdown_is_draining(&body));
        assert_eq!(
            summary,
            "Daemon is draining 3 active work item(s), including 1 background process(es); Captain will not stop healthy active work."
        );
        assert!(!summary.contains("private prompt"));
    }
}
