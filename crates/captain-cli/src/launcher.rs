//! Legacy launcher helpers — the menu-driven splash was dropped in phase-f.3.1
//! in favour of the chat-first flow (direct TUI chat).
//!
//! ANSI boot banners must not be printed before ratatui enters the alternate
//! screen, otherwise they reappear in the user's shell when the TUI exits.
//! Only the `launch_desktop_app` helper is kept here, for callers that may want
//! to spawn the separate desktop app binary from a subcommand.

use crate::ui;
use std::path::PathBuf;

#[allow(dead_code)]
pub fn launch_desktop_app() {
    let desktop_bin = {
        let exe = std::env::current_exe().ok();
        let dir = exe.as_ref().and_then(|e| e.parent());

        #[cfg(windows)]
        let name = "captain-desktop.exe";
        #[cfg(not(windows))]
        let name = "captain-desktop";

        let sibling = dir.map(|d| d.join(name));

        match sibling {
            Some(ref path) if path.exists() => sibling,
            _ => which_lookup(name),
        }
    };

    match desktop_bin {
        Some(ref path) if path.exists() => match std::process::Command::new(path)
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
        {
            Ok(_) => ui::success("Desktop app launched."),
            Err(e) => ui::error_with_fix(
                &format!("Failed to launch desktop app: {e}"),
                "Build it: cargo build -p captain-desktop",
            ),
        },
        _ => ui::error_with_fix(
            "Desktop app not found",
            "Build it: cargo build -p captain-desktop",
        ),
    }
}

#[allow(dead_code)]
fn which_lookup(name: &str) -> Option<PathBuf> {
    let path_var = std::env::var("PATH").ok()?;
    let separator = if cfg!(windows) { ';' } else { ':' };
    for dir in path_var.split(separator) {
        let candidate = PathBuf::from(dir).join(name);
        if candidate.exists() {
            return Some(candidate);
        }
    }
    None
}
