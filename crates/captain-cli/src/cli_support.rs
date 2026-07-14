use std::io::{self, BufRead, Write};
use std::path::{Path, PathBuf};

use crate::cli_captain_home;

#[cfg(unix)]
pub(crate) fn restrict_file_permissions(path: &Path) {
    use std::os::unix::fs::PermissionsExt;
    let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600));
}

#[cfg(not(unix))]
pub(crate) fn restrict_file_permissions(_path: &Path) {}

#[cfg(unix)]
pub(crate) fn restrict_dir_permissions(path: &Path) {
    use std::os::unix::fs::PermissionsExt;
    let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700));
}

#[cfg(not(unix))]
pub(crate) fn restrict_dir_permissions(_path: &Path) {}

pub(crate) fn find_captain_cli_on_path() -> Option<PathBuf> {
    let path_var = std::env::var_os("PATH")?;
    let names: &[&str] = if cfg!(windows) {
        &["captain.exe", "captain.cmd", "captain.bat", "captain"]
    } else {
        &["captain"]
    };

    for dir in std::env::split_paths(&path_var) {
        for name in names {
            let candidate = dir.join(name);
            if candidate.is_file() {
                return Some(candidate);
            }
        }
    }
    None
}

pub(crate) fn command_version(path: &Path) -> Option<String> {
    let output = std::process::Command::new(path)
        .arg("--version")
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if !stdout.is_empty() {
        return Some(stdout);
    }
    let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
    if stderr.is_empty() {
        None
    } else {
        Some(stderr)
    }
}

pub(crate) fn path_eq_best_effort(a: &Path, b: &Path) -> bool {
    let left = std::fs::canonicalize(a).unwrap_or_else(|_| a.to_path_buf());
    let right = std::fs::canonicalize(b).unwrap_or_else(|_| b.to_path_buf());
    left == right
}

pub(crate) fn open_in_browser(url: &str) -> bool {
    #[cfg(target_os = "windows")]
    {
        std::process::Command::new("cmd")
            .args(["/C", "start", "", url])
            .spawn()
            .is_ok()
    }
    #[cfg(target_os = "macos")]
    {
        std::process::Command::new("open").arg(url).spawn().is_ok()
    }
    #[cfg(target_os = "linux")]
    {
        let openers = [
            "xdg-open",
            "sensible-browser",
            "x-www-browser",
            "firefox",
            "google-chrome",
            "chromium",
            "chromium-browser",
        ];
        for opener in &openers {
            let result = std::process::Command::new(opener)
                .arg(url)
                .stdin(std::process::Stdio::null())
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .spawn();
            if result.is_ok() {
                return true;
            }
        }
        false
    }
    #[cfg(not(any(target_os = "windows", target_os = "macos", target_os = "linux")))]
    {
        let _ = url;
        false
    }
}

pub(crate) fn test_api_key(provider: &str, env_var: &str) -> bool {
    let key = match std::env::var(env_var) {
        Ok(k) => k,
        Err(_) => return false,
    };

    let client = match reqwest::blocking::Client::builder()
        .timeout(std::time::Duration::from_secs(10))
        .build()
    {
        Ok(c) => c,
        Err(_) => return true,
    };

    let result = match provider.to_lowercase().as_str() {
        "groq" => client
            .get("https://api.groq.com/openai/v1/models")
            .bearer_auth(&key)
            .send(),
        "anthropic" => client
            .get("https://api.anthropic.com/v1/models")
            .header("x-api-key", &key)
            .header("anthropic-version", "2023-06-01")
            .send(),
        "openai" => client
            .get("https://api.openai.com/v1/models")
            .bearer_auth(&key)
            .send(),
        "gemini" | "google" => client
            .get(format!(
                "https://generativelanguage.googleapis.com/v1beta/models?key={key}"
            ))
            .send(),
        "deepseek" => client
            .get("https://api.deepseek.com/models")
            .bearer_auth(&key)
            .send(),
        "openrouter" => client
            .get("https://openrouter.ai/api/v1/models")
            .bearer_auth(&key)
            .send(),
        _ => return true,
    };

    match result {
        Ok(resp) => {
            let status = resp.status().as_u16();
            status != 401 && status != 403
        }
        Err(_) => true,
    }
}

pub(crate) fn captain_home() -> PathBuf {
    cli_captain_home()
}

pub(crate) fn prompt_input(prompt: &str) -> String {
    print!("{prompt}");
    // Piped/truncated output (e.g. `captain uninstall | head`) closes stdout
    // before we get here; a raw `.unwrap()` panicked with a backtrace on
    // BrokenPipe instead of just proceeding with whatever got printed.
    let _ = io::stdout().flush();
    let mut line = String::new();
    io::stdin().lock().read_line(&mut line).unwrap_or(0);
    line.trim().to_string()
}

/// Like `prompt_input`, but masks the typed input — for secrets (SSH
/// passphrases, API keys, vault values, bot tokens) that must not be echoed
/// to the terminal or end up in scrollback/screen-recording history.
///
/// `rpassword` needs a real controlling TTY to suppress echo and fails with
/// an error (not an empty string) when stdin is piped — e.g. non-interactive
/// setup scripts doing `echo "$TOKEN" | captain channel setup telegram`, or
/// CI. Silently swallowing that error into an empty string would make those
/// scripts fail with a confusing "No token provided" instead of using the
/// piped value. Fall back to a plain (unmasked) stdin read in that case,
/// matching `prompt_input`'s behavior — masking only matters for a real
/// interactive terminal in the first place.
pub(crate) fn prompt_secret(prompt: &str) -> String {
    match rpassword::prompt_password(prompt) {
        Ok(value) => value.trim().to_string(),
        // rpassword doesn't print the prompt before failing to open a TTY,
        // so prompt_input's own print!(prompt) is the only place it appears.
        Err(_) => prompt_input(prompt),
    }
}

pub(crate) fn copy_dir_recursive(src: &PathBuf, dst: &PathBuf) {
    std::fs::create_dir_all(dst).unwrap();
    if let Ok(entries) = std::fs::read_dir(src) {
        for entry in entries.flatten() {
            let path = entry.path();
            let dest_path = dst.join(entry.file_name());
            if path.is_dir() {
                copy_dir_recursive(&path, &dest_path);
            } else {
                let _ = std::fs::copy(&path, &dest_path);
            }
        }
    }
}

pub(crate) fn truncate_display(input: &str, max_chars: usize) -> String {
    if input.chars().count() <= max_chars {
        return input.to_string();
    }
    if max_chars <= 3 {
        return ".".repeat(max_chars);
    }
    let mut out = input
        .chars()
        .take(max_chars.saturating_sub(3))
        .collect::<String>();
    out.push_str("...");
    out
}
