use crate::{find_daemon, open_in_browser, start_daemon_background, ui};

pub(crate) fn cmd_terminal() {
    let base = if let Some(url) = find_daemon() {
        url
    } else {
        ui::hint("No daemon running — starting one now...");
        match start_daemon_background() {
            Ok(url) => {
                ui::success("Daemon started");
                url
            }
            Err(e) => {
                ui::error_with_fix(
                    &format!("Could not start daemon: {e}"),
                    "Start it manually: captain start",
                );
                std::process::exit(1);
            }
        }
    };

    let url = format!("{base}/terminal");
    ui::success(&format!("Opening web terminal at {url}"));
    if copy_to_clipboard(&url) {
        ui::hint("URL copied to clipboard");
    }
    if !open_in_browser(&url) {
        ui::hint(&format!("Could not open browser. Visit: {url}"));
    }
}

fn copy_to_clipboard(text: &str) -> bool {
    #[cfg(target_os = "windows")]
    {
        std::process::Command::new("powershell")
            .args([
                "-NoProfile",
                "-Command",
                &format!("Set-Clipboard '{}'", text.replace('\'', "''")),
            ])
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .map(|s| s.success())
            .unwrap_or(false)
    }
    #[cfg(target_os = "macos")]
    {
        use std::io::Write as IoWrite;
        std::process::Command::new("pbcopy")
            .stdin(std::process::Stdio::piped())
            .spawn()
            .and_then(|mut child| {
                if let Some(ref mut stdin) = child.stdin {
                    let _ = stdin.write_all(text.as_bytes());
                }
                child.wait()
            })
            .map(|s| s.success())
            .unwrap_or(false)
    }
    #[cfg(target_os = "linux")]
    {
        use std::io::Write as IoWrite;
        let result = std::process::Command::new("xclip")
            .args(["-selection", "clipboard"])
            .stdin(std::process::Stdio::piped())
            .spawn()
            .and_then(|mut child| {
                if let Some(ref mut stdin) = child.stdin {
                    let _ = stdin.write_all(text.as_bytes());
                }
                child.wait()
            })
            .map(|s| s.success())
            .unwrap_or(false);
        if result {
            return true;
        }
        std::process::Command::new("xsel")
            .args(["--clipboard", "--input"])
            .stdin(std::process::Stdio::piped())
            .spawn()
            .and_then(|mut child| {
                if let Some(ref mut stdin) = child.stdin {
                    let _ = stdin.write_all(text.as_bytes());
                }
                child.wait()
            })
            .map(|s| s.success())
            .unwrap_or(false)
    }
    #[cfg(not(any(target_os = "windows", target_os = "macos", target_os = "linux")))]
    {
        let _ = text;
        false
    }
}
