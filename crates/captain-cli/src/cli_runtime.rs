use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;
#[cfg(windows)]
use std::sync::atomic::Ordering;

use captain_kernel::CaptainKernel;

use crate::ui;

static CTRLC_PRESSED: AtomicBool = AtomicBool::new(false);

pub(crate) fn install_ctrlc_handler() {
    #[cfg(windows)]
    {
        extern "system" {
            fn SetConsoleCtrlHandler(
                handler: Option<unsafe extern "system" fn(u32) -> i32>,
                add: i32,
            ) -> i32;
        }
        unsafe extern "system" fn handler(_ctrl_type: u32) -> i32 {
            if CTRLC_PRESSED.swap(true, Ordering::SeqCst) {
                std::process::exit(130);
            }
            let _ = std::io::Write::write_all(&mut std::io::stderr(), b"\nInterrupted.\n");
            std::process::exit(0);
        }
        unsafe { SetConsoleCtrlHandler(Some(handler), 1) };
    }

    #[cfg(not(windows))]
    {
        let _ = &CTRLC_PRESSED;
    }
}

pub(crate) fn captain_version() -> String {
    captain_types::version::captain_version()
}

pub(crate) fn maybe_print_version_and_exit() {
    let mut args = std::env::args_os();
    let _ = args.next();
    let Some(first) = args.next() else {
        return;
    };
    if args.next().is_some() {
        return;
    }
    let first = first.to_string_lossy();
    if matches!(first.as_ref(), "--version" | "-V") {
        println!("captain {}", captain_version());
        std::process::exit(0);
    }
}

fn config_log_level() -> String {
    let config_path = if let Ok(home) = std::env::var("CAPTAIN_HOME") {
        PathBuf::from(home).join("config.toml")
    } else {
        dirs::home_dir()
            .unwrap_or_else(std::env::temp_dir)
            .join(".captain")
            .join("config.toml")
    };
    if let Ok(content) = std::fs::read_to_string(config_path) {
        for line in content.lines() {
            let trimmed = line.trim();
            if trimmed.starts_with("log_level") {
                if let Some(val) = trimmed.split('=').nth(1) {
                    let level = val.trim().trim_matches('"').trim_matches('\'');
                    if !level.is_empty() {
                        return level.to_string();
                    }
                }
            }
        }
    }
    "info".to_string()
}

pub(crate) fn init_tracing_stderr() {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new(config_log_level())),
        )
        .init();
}

pub(crate) fn cli_captain_home() -> PathBuf {
    if let Ok(home) = std::env::var("CAPTAIN_HOME") {
        return PathBuf::from(home);
    }
    let home = dirs::home_dir().unwrap_or_else(std::env::temp_dir);
    let default_dir = home.join(".captain");
    let bootstrap_config = default_dir.join("config.toml");
    if let Ok(text) = std::fs::read_to_string(&bootstrap_config) {
        if let Ok(t) = text.parse::<toml::Value>() {
            if let Some(p) = t.get("home_dir").and_then(|v| v.as_str()) {
                let p = p.trim();
                if !p.is_empty() {
                    let candidate = PathBuf::from(p);
                    if candidate == default_dir || home_has_real_data(&candidate) {
                        return candidate;
                    }
                }
            }
        }
    }
    default_dir
}

fn home_has_real_data(p: &Path) -> bool {
    if p.join("config.toml").exists() {
        return true;
    }
    if let Ok(meta) = std::fs::metadata(p.join("graph.hora")) {
        if meta.len() > 1_000_000 {
            return true;
        }
    }
    false
}

pub(crate) fn init_tracing_file() {
    let log_dir = cli_captain_home();
    let _ = std::fs::create_dir_all(&log_dir);
    let log_path = log_dir.join("tui.log");

    match std::fs::File::create(&log_path) {
        Ok(file) => {
            tracing_subscriber::fmt()
                .with_env_filter(
                    tracing_subscriber::EnvFilter::try_from_default_env()
                        .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new(config_log_level())),
                )
                .with_writer(std::sync::Mutex::new(file))
                .with_ansi(false)
                .init();
        }
        Err(_) => {
            tracing_subscriber::fmt()
                .with_max_level(tracing::Level::ERROR)
                .with_writer(std::io::sink)
                .init();
        }
    }
}

pub(crate) fn boot_kernel_error(e: &captain_kernel::error::KernelError) {
    let msg = e.to_string();
    if msg.contains("parse") || msg.contains("toml") || msg.contains("config") {
        ui::error_with_fix(
            "Failed to parse configuration",
            "Check your config.toml syntax: captain config show",
        );
    } else if msg.contains("database") || msg.contains("locked") || msg.contains("sqlite") {
        ui::error_with_fix(
            "Database error (file may be locked)",
            "Check if another Captain process is running: captain status",
        );
    } else if msg.contains("key") || msg.contains("API") || msg.contains("auth") {
        ui::error_with_fix(
            "LLM provider authentication failed",
            "Run `captain doctor` to check your API key configuration",
        );
    } else {
        ui::error_with_fix(
            &format!("Failed to boot kernel: {msg}"),
            "Run `captain doctor` to diagnose the issue",
        );
    }
}

pub(crate) fn boot_kernel(config: Option<PathBuf>) -> CaptainKernel {
    let kernel_config =
        match crate::commands::memory_native::prepare_kernel_config(config.as_deref()) {
            Ok(config) => config,
            Err(error) => {
                ui::error_with_fix(
                    &format!("Managed MemPalace is not production-ready: {error}"),
                    "Run `captain memory doctor`, then `captain memory install --force`.",
                );
                std::process::exit(1);
            }
        };
    match CaptainKernel::boot_with_config(kernel_config) {
        Ok(k) => k,
        Err(e) => {
            boot_kernel_error(&e);
            std::process::exit(1);
        }
    }
}
