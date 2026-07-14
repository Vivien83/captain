//! Native screenshot handler.

use std::path::Path;

pub(crate) async fn tool_screenshot(
    input: &serde_json::Value,
    workspace_root: Option<&Path>,
) -> Result<String, String> {
    let save_path = input["save_path"]
        .as_str()
        .map(String::from)
        .unwrap_or_else(|| {
            let ts = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_secs())
                .unwrap_or(0);
            match workspace_root {
                Some(root) => root
                    .join("screenshots")
                    .join(format!("captain_{ts}.png"))
                    .to_string_lossy()
                    .into_owned(),
                None => format!("/tmp/captain_screenshot_{ts}.png"),
            }
        });

    if let Some(parent) = Path::new(&save_path).parent() {
        if !parent.as_os_str().is_empty() {
            tokio::fs::create_dir_all(parent).await.map_err(|e| {
                format!("Failed to create screenshot dir {}: {e}", parent.display())
            })?;
        }
    }

    let (cmd, args) = screenshot_command(&save_path).ok_or_else(screenshot_unavailable_message)?;
    let output = tokio::process::Command::new(cmd)
        .args(&args)
        .output()
        .await
        .map_err(|e| format!("Screenshot command '{cmd}' failed to spawn: {e}"))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!(
            "Screenshot command '{cmd}' exited with status {}: {stderr}",
            output.status
        ));
    }

    let size_bytes = tokio::fs::metadata(&save_path)
        .await
        .map(|m| m.len())
        .map_err(|e| format!("Screenshot saved but stat failed on '{save_path}': {e}"))?;

    if size_bytes == 0 {
        return Err(format!("Screenshot saved but file '{save_path}' is empty"));
    }

    Ok(serde_json::json!({
        "success": true,
        "path": save_path,
        "size_bytes": size_bytes,
        "platform": std::env::consts::OS,
        "command": cmd,
    })
    .to_string())
}

pub(crate) fn screenshot_command(save_path: &str) -> Option<(&'static str, Vec<String>)> {
    match std::env::consts::OS {
        "macos" => Some(("screencapture", vec!["-x".into(), save_path.into()])),
        "linux" => {
            for program in ["grim", "gnome-screenshot", "scrot", "import"] {
                if which_is_available(program) {
                    let args = match program {
                        "grim" => vec![save_path.into()],
                        "gnome-screenshot" => vec!["-f".into(), save_path.into()],
                        "scrot" => vec![save_path.into()],
                        "import" => vec!["-window".into(), "root".into(), save_path.into()],
                        _ => unreachable!(),
                    };
                    return Some((program, args));
                }
            }
            None
        }
        "windows" => {
            if which_is_available("nircmd") {
                Some(("nircmd", vec!["savescreenshot".into(), save_path.into()]))
            } else {
                None
            }
        }
        _ => None,
    }
}

fn which_is_available(program: &str) -> bool {
    std::process::Command::new("which")
        .arg(program)
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

fn screenshot_unavailable_message() -> String {
    match std::env::consts::OS {
        "macos" => "macOS screencapture not found (should be built-in)".into(),
        "linux" => "No screenshot command available. Install one of: grim (wayland), \
             gnome-screenshot, scrot, or imagemagick (import)."
            .into(),
        "windows" => {
            "nircmd not installed. Install from https://www.nirsoft.net/utils/nircmd.html".into()
        }
        other => format!("Screenshot not supported on platform: {other}"),
    }
}
