use colored::Colorize;

use super::daemon::cmd_stop_result;
use crate::{cli_captain_home, find_daemon, prompt_input, ui};

pub(crate) fn cmd_uninstall(confirm: bool, keep_config: bool) {
    let captain_dir = cli_captain_home();
    let exe_path = std::env::current_exe().ok();

    println!();
    println!(
        "  {}",
        "This will completely uninstall Captain from your system."
            .bold()
            .red()
    );
    println!();
    if captain_dir.exists() {
        if keep_config {
            println!(
                "  • Remove data in {} (keeping config files)",
                captain_dir.display()
            );
        } else {
            println!("  • Remove {}", captain_dir.display());
        }
    }
    if let Some(ref exe) = exe_path {
        println!("  • Remove binary: {}", exe.display());
    }

    let cargo_bin = dirs::home_dir()
        .unwrap_or_else(std::env::temp_dir)
        .join(".cargo")
        .join("bin")
        .join(if cfg!(windows) {
            "captain.exe"
        } else {
            "captain"
        });
    if cargo_bin.exists() && exe_path.as_ref().is_none_or(|e| *e != cargo_bin) {
        println!("  • Remove cargo binary: {}", cargo_bin.display());
    }
    println!("  • Remove auto-start entries (if any)");
    println!("  • Clean PATH from shell configs (if any)");
    println!();

    if !confirm {
        let answer = prompt_input("  Type 'uninstall' to confirm: ");
        if answer.trim() != "uninstall" {
            println!("  Cancelled.");
            return;
        }
        println!();
    }

    if find_daemon().is_some() {
        println!("  Stopping running daemon...");
        if !cmd_stop_result() {
            ui::warn_with_fix(
                "Uninstall deferred because Captain is still running.",
                "Run `captain status` to inspect active work, then retry after the daemon stops cleanly.",
            );
            std::process::exit(1);
        }
        std::thread::sleep(std::time::Duration::from_secs(1));
        if find_daemon().is_some() {
            ui::warn_with_fix(
                "Uninstall deferred because the daemon still answers health checks.",
                "Run `captain status` before retrying; Captain will not remove data under a live daemon.",
            );
            std::process::exit(1);
        }
    }

    let user_home = dirs::home_dir().unwrap_or_else(std::env::temp_dir);
    remove_autostart_entries(&user_home);

    if let Some(ref exe) = exe_path {
        if let Some(bin_dir) = exe.parent() {
            clean_path_entries(&user_home, &bin_dir.to_string_lossy());
        }
    }

    if captain_dir.exists() {
        if keep_config {
            remove_dir_except_config(&captain_dir);
            ui::success("Removed data (kept config files)");
        } else {
            match std::fs::remove_dir_all(&captain_dir) {
                Ok(()) => ui::success(&format!("Removed {}", captain_dir.display())),
                Err(e) => ui::error(&format!("Failed to remove {}: {e}", captain_dir.display())),
            }
        }
    }

    if cargo_bin.exists() && exe_path.as_ref().is_none_or(|e| *e != cargo_bin) {
        match std::fs::remove_file(&cargo_bin) {
            Ok(()) => ui::success(&format!("Removed {}", cargo_bin.display())),
            Err(e) => ui::error(&format!("Failed to remove {}: {e}", cargo_bin.display())),
        }
    }

    if let Some(exe) = exe_path {
        remove_self_binary(&exe);
    }

    println!();
    ui::success("Captain has been uninstalled. Goodbye!");
}

#[allow(unused_variables)]
fn remove_autostart_entries(home: &std::path::Path) {
    #[cfg(windows)]
    {
        let output = std::process::Command::new("reg")
            .args([
                "delete",
                r"HKCU\Software\Microsoft\Windows\CurrentVersion\Run",
                "/v",
                "Captain",
                "/f",
            ])
            .output();
        match output {
            Ok(o) if o.status.success() => {
                ui::success("Removed Windows auto-start registry entry");
            }
            _ => {}
        }
    }

    #[cfg(target_os = "macos")]
    {
        let plist = home.join("Library/LaunchAgents/ai.captain.desktop.plist");
        if plist.exists() {
            let _ = std::process::Command::new("launchctl")
                .args(["unload", &plist.to_string_lossy()])
                .output();
            match std::fs::remove_file(&plist) {
                Ok(()) => ui::success("Removed macOS launch agent"),
                Err(e) => ui::error(&format!("Failed to remove launch agent: {e}")),
            }
        }
    }

    #[cfg(target_os = "linux")]
    {
        let desktop_file = home.join(".config/autostart/Captain.desktop");
        if desktop_file.exists() {
            match std::fs::remove_file(&desktop_file) {
                Ok(()) => ui::success("Removed Linux autostart entry"),
                Err(e) => ui::error(&format!("Failed to remove autostart entry: {e}")),
            }
        }

        let service_file = home.join(".config/systemd/user/captain.service");
        if service_file.exists() {
            let _ = std::process::Command::new("systemctl")
                .args(["--user", "disable", "--now", "captain.service"])
                .output();
            match std::fs::remove_file(&service_file) {
                Ok(()) => {
                    let _ = std::process::Command::new("systemctl")
                        .args(["--user", "daemon-reload"])
                        .output();
                    ui::success("Removed systemd user service");
                }
                Err(e) => ui::error(&format!("Failed to remove systemd service: {e}")),
            }
        }
    }
}

#[allow(unused_variables)]
fn clean_path_entries(home: &std::path::Path, captain_dir: &str) {
    #[cfg(not(windows))]
    {
        let shell_files = [
            home.join(".bashrc"),
            home.join(".bash_profile"),
            home.join(".profile"),
            home.join(".zshrc"),
            home.join(".config/fish/config.fish"),
        ];

        for path in &shell_files {
            if !path.exists() {
                continue;
            }
            let Ok(content) = std::fs::read_to_string(path) else {
                continue;
            };
            let filtered: Vec<&str> = content
                .lines()
                .filter(|line| !is_captain_path_line(line, captain_dir))
                .collect();
            if filtered.len() < content.lines().count() {
                // is_captain_path_line matches on the substring "captain" anywhere in the
                // line, which can also match an unrelated PATH entry that merely mentions
                // it. Back up the shell rc file before rewriting it so a wrongly-matched
                // line isn't lost irrecoverably.
                let ts = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map(|d| d.as_secs())
                    .unwrap_or(0);
                let backup_path = std::path::PathBuf::from(format!("{}.bak-{ts}", path.display()));
                if let Err(e) = std::fs::copy(path, &backup_path) {
                    ui::check_warn(&format!(
                        "Could not back up {} before cleaning PATH, skipping: {e}",
                        path.display()
                    ));
                    continue;
                }
                let new_content = filtered.join("\n");
                let new_content = if content.ends_with('\n') {
                    format!("{new_content}\n")
                } else {
                    new_content
                };
                if std::fs::write(path, &new_content).is_ok() {
                    ui::success(&format!(
                        "Cleaned PATH from {} (backup: {})",
                        path.display(),
                        backup_path.display()
                    ));
                }
            }
        }
    }

    #[cfg(windows)]
    {
        let output = std::process::Command::new("powershell")
            .args([
                "-NoProfile",
                "-Command",
                "[Environment]::GetEnvironmentVariable('PATH', 'User')",
            ])
            .output();
        if let Ok(out) = output {
            if out.status.success() {
                let current = String::from_utf8_lossy(&out.stdout);
                let current = current.trim();
                if !current.is_empty() {
                    let dir_lower = captain_dir.to_lowercase();
                    let filtered: Vec<&str> = current
                        .split(';')
                        .filter(|entry| {
                            let e = entry.trim().to_lowercase();
                            !e.is_empty() && !e.contains("captain") && !e.contains(&dir_lower)
                        })
                        .collect();
                    if filtered.len() < current.split(';').count() {
                        let new_path = filtered.join(";");
                        let ps_cmd = format!(
                            "[Environment]::SetEnvironmentVariable('PATH', '{}', 'User')",
                            new_path.replace('\'', "''")
                        );
                        let result = std::process::Command::new("powershell")
                            .args(["-NoProfile", "-Command", &ps_cmd])
                            .output();
                        if result.is_ok_and(|o| o.status.success()) {
                            ui::success("Cleaned PATH from Windows user environment");
                        }
                    }
                }
            }
        }
    }
}

#[cfg(any(not(windows), test))]
fn is_captain_path_line(line: &str, captain_dir: &str) -> bool {
    let lower = line.to_lowercase();
    let has_captain = lower.contains("captain") || lower.contains(&captain_dir.to_lowercase());
    if !has_captain {
        return false;
    }
    lower.contains("export path=")
        || lower.contains("export path =")
        || lower.starts_with("path=")
        || lower.contains("set -gx path")
        || lower.contains("fish_add_path")
}

fn remove_dir_except_config(captain_dir: &std::path::Path) {
    let keep = ["config.toml", ".env", "secrets.env"];
    let Ok(entries) = std::fs::read_dir(captain_dir) else {
        return;
    };
    for entry in entries.flatten() {
        let name = entry.file_name();
        let name_str = name.to_string_lossy();
        if keep.contains(&name_str.as_ref()) {
            continue;
        }
        let path = entry.path();
        if path.is_dir() {
            let _ = std::fs::remove_dir_all(&path);
        } else {
            let _ = std::fs::remove_file(&path);
        }
    }
}

fn remove_self_binary(exe_path: &std::path::Path) {
    #[cfg(unix)]
    {
        match std::fs::remove_file(exe_path) {
            Ok(()) => ui::success(&format!("Removed {}", exe_path.display())),
            Err(e) => ui::error(&format!(
                "Failed to remove binary {}: {e}",
                exe_path.display()
            )),
        }
    }

    #[cfg(windows)]
    {
        let old_path = exe_path.with_extension("exe.old");
        if std::fs::rename(exe_path, &old_path).is_err() {
            ui::error(&format!(
                "Could not rename binary for deferred deletion: {}",
                exe_path.display()
            ));
            return;
        }

        use std::os::windows::process::CommandExt;
        const CREATE_NEW_PROCESS_GROUP: u32 = 0x0000_0200;
        const DETACHED_PROCESS: u32 = 0x0000_0008;

        let del_cmd = format!(
            "ping -n 3 127.0.0.1 >nul & del /f /q \"{}\"",
            old_path.display()
        );
        let _ = std::process::Command::new("cmd.exe")
            .args(["/C", &del_cmd])
            .creation_flags(CREATE_NEW_PROCESS_GROUP | DETACHED_PROCESS)
            .spawn();

        ui::success(&format!(
            "Removed {} (deferred cleanup)",
            exe_path.display()
        ));
    }
}

#[cfg(test)]
mod tests {
    use super::is_captain_path_line;

    #[cfg(not(windows))]
    #[test]
    fn clean_path_entries_backs_up_before_rewrite() {
        use super::clean_path_entries;

        let dir = tempfile::tempdir().unwrap();
        let home = dir.path();
        let bashrc = home.join(".bashrc");
        let original = "export PATH=\"$HOME/.captain/bin:$PATH\"\nexport EDITOR=vim\n";
        std::fs::write(&bashrc, original).unwrap();

        clean_path_entries(home, "/home/user/.captain/bin");

        let cleaned = std::fs::read_to_string(&bashrc).unwrap();
        assert!(!cleaned.contains("captain"), "PATH line should be removed");
        assert!(cleaned.contains("EDITOR=vim"), "other lines must survive");

        let backups: Vec<_> = std::fs::read_dir(home)
            .unwrap()
            .filter_map(|e| e.ok())
            .map(|e| e.file_name().to_string_lossy().to_string())
            .filter(|name| name.starts_with(".bashrc.bak-"))
            .collect();
        assert_eq!(backups.len(), 1, "expected exactly one backup file");
        let backup_content = std::fs::read_to_string(home.join(&backups[0])).unwrap();
        assert_eq!(
            backup_content, original,
            "backup must contain the pre-cleanup content"
        );
    }

    #[test]
    fn uninstall_path_line_filter() {
        let dir = "/home/user/.captain/bin";

        assert!(is_captain_path_line(
            r#"export PATH="$HOME/.captain/bin:$PATH""#,
            dir
        ));
        assert!(is_captain_path_line(
            r#"export PATH="/home/user/.captain/bin:$PATH""#,
            dir
        ));
        assert!(is_captain_path_line(
            "set -gx PATH $HOME/.captain/bin $PATH",
            dir
        ));
        assert!(is_captain_path_line(
            "fish_add_path $HOME/.captain/bin",
            dir
        ));

        assert!(!is_captain_path_line(
            r#"export PATH="$HOME/.cargo/bin:$PATH""#,
            dir
        ));
        assert!(!is_captain_path_line(
            r#"export PATH="/usr/local/bin:$PATH""#,
            dir
        ));
        assert!(!is_captain_path_line("# captain config", dir));
        assert!(!is_captain_path_line("alias of=captain", dir));
    }
}
