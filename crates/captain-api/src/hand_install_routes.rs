use crate::hand_routes::server_platform;
use crate::state::AppState;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::Json;
use captain_hands::{HandInstallInfo, HandRequirement};
use std::sync::Arc;

/// POST /api/hands/{hand_id}/install-deps - Auto-install missing dependencies.
pub async fn install_hand_deps(
    State(state): State<Arc<AppState>>,
    Path(hand_id): Path<String>,
) -> impl IntoResponse {
    let def = match state.kernel.hand_registry.get_definition(&hand_id) {
        Some(d) => d.clone(),
        None => {
            return (
                StatusCode::NOT_FOUND,
                Json(serde_json::json!({"error": format!("Hand not found: {hand_id}")})),
            );
        }
    };

    let reqs = state
        .kernel
        .hand_registry
        .check_requirements(&hand_id)
        .unwrap_or_default();

    let platform = server_platform();
    let results = install_missing_requirements(&hand_id, &reqs, platform).await;

    refresh_windows_install_path();

    let reqs_after = state
        .kernel
        .hand_registry
        .check_requirements(&hand_id)
        .unwrap_or_default();
    let all_satisfied = reqs_after.iter().all(|(_, ok)| *ok);

    (
        StatusCode::OK,
        Json(serde_json::json!({
            "hand_id": def.id,
            "results": results,
            "requirements_met": all_satisfied,
            "requirements": reqs_after.iter().map(requirement_status_json).collect::<Vec<_>>(),
        })),
    )
}

async fn install_missing_requirements(
    hand_id: &str,
    reqs: &[(HandRequirement, bool)],
    platform: &str,
) -> Vec<serde_json::Value> {
    let mut results = Vec::new();
    for (req, already_satisfied) in reqs {
        results.push(install_requirement(hand_id, req, *already_satisfied, platform).await);
    }
    results
}

async fn install_requirement(
    hand_id: &str,
    req: &HandRequirement,
    already_satisfied: bool,
    platform: &str,
) -> serde_json::Value {
    if already_satisfied {
        return already_installed_result(req);
    }

    let install = match &req.install {
        Some(i) => i,
        None => return skipped_install_result(req),
    };
    let cmd = match install_command_for_platform(install, platform) {
        Some(c) => c,
        None => return no_command_result(req, platform),
    };

    let (shell, flag) = install_shell();
    let final_cmd = command_for_shell(cmd, cfg!(windows));
    tracing::info!(hand = %hand_id, dep = %req.key, cmd = %final_cmd, "Auto-installing dependency");

    let output = match run_install_command(shell, flag, &final_cmd).await {
        Ok(out) => out,
        Err(InstallCommandError::Launch(error)) => {
            return command_launch_error_result(req, &final_cmd, &error);
        }
        Err(InstallCommandError::Timeout) => return command_timeout_result(req, &final_cmd),
    };

    let exit_code = output.status.code().unwrap_or(-1);
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    command_exit_result(req, &final_cmd, exit_code, &stdout, &stderr)
}

enum InstallCommandError {
    Launch(std::io::Error),
    Timeout,
}

async fn run_install_command(
    shell: &str,
    flag: &str,
    command: &str,
) -> Result<std::process::Output, InstallCommandError> {
    match tokio::time::timeout(
        std::time::Duration::from_secs(300),
        tokio::process::Command::new(shell)
            .arg(flag)
            .arg(command)
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .stdin(std::process::Stdio::null())
            .output(),
    )
    .await
    {
        Ok(Ok(output)) => Ok(output),
        Ok(Err(error)) => Err(InstallCommandError::Launch(error)),
        Err(_) => Err(InstallCommandError::Timeout),
    }
}

fn install_command_for_platform<'a>(
    install: &'a HandInstallInfo,
    platform: &str,
) -> Option<&'a str> {
    match platform {
        "windows" => install.windows.as_deref().or(install.pip.as_deref()),
        "macos" => install.macos.as_deref().or(install.pip.as_deref()),
        _ => install
            .linux_apt
            .as_deref()
            .or(install.linux_dnf.as_deref())
            .or(install.linux_pacman.as_deref())
            .or(install.pip.as_deref()),
    }
}

fn install_shell() -> (&'static str, &'static str) {
    if cfg!(windows) {
        ("cmd", "/C")
    } else {
        ("sh", "-c")
    }
}

fn command_for_shell(command: &str, is_windows: bool) -> String {
    if is_windows && command.starts_with("winget ") {
        format!("{command} --accept-source-agreements --accept-package-agreements")
    } else {
        command.to_string()
    }
}

fn already_installed_result(req: &HandRequirement) -> serde_json::Value {
    serde_json::json!({
        "key": req.key,
        "status": "already_installed",
        "message": format!("{} is already available", req.label),
    })
}

fn skipped_install_result(req: &HandRequirement) -> serde_json::Value {
    serde_json::json!({
        "key": req.key,
        "status": "skipped",
        "message": "No install instructions available",
    })
}

fn no_command_result(req: &HandRequirement, platform: &str) -> serde_json::Value {
    serde_json::json!({
        "key": req.key,
        "status": "no_command",
        "message": format!("No install command for platform: {platform}"),
    })
}

fn command_launch_error_result(
    req: &HandRequirement,
    command: &str,
    error: &std::io::Error,
) -> serde_json::Value {
    serde_json::json!({
        "key": req.key,
        "status": "error",
        "command": command,
        "message": format!("Failed to execute: {error}"),
    })
}

fn command_timeout_result(req: &HandRequirement, command: &str) -> serde_json::Value {
    serde_json::json!({
        "key": req.key,
        "status": "timeout",
        "command": command,
        "message": "Installation timed out after 5 minutes",
    })
}

fn command_exit_result(
    req: &HandRequirement,
    command: &str,
    exit_code: i32,
    stdout: &str,
    stderr: &str,
) -> serde_json::Value {
    if exit_code == 0 {
        return serde_json::json!({
            "key": req.key,
            "status": "installed",
            "command": command,
            "message": format!("{} installed successfully", req.label),
        });
    }

    let combined = format!("{stdout}{stderr}");
    if is_likely_already_installed(&combined) {
        return serde_json::json!({
            "key": req.key,
            "status": "installed",
            "command": command,
            "exit_code": exit_code,
            "message": format!("{} is already installed", req.label),
        });
    }

    let msg = stderr.chars().take(500).collect::<String>();
    serde_json::json!({
        "key": req.key,
        "status": "error",
        "command": command,
        "exit_code": exit_code,
        "message": format!("Install failed (exit {}): {}", exit_code, msg.trim()),
    })
}

fn is_likely_already_installed(output: &str) -> bool {
    output.contains("already installed")
        || output.contains("No applicable update")
        || output.contains("No available upgrade")
        || output.contains("already an App at")
        || output.contains("is already installed")
}

fn requirement_status_json((req, satisfied): &(HandRequirement, bool)) -> serde_json::Value {
    serde_json::json!({
        "key": req.key,
        "label": req.label,
        "satisfied": satisfied,
    })
}

#[cfg(not(windows))]
fn refresh_windows_install_path() {}

#[cfg(windows)]
fn refresh_windows_install_path() {
    let home = std::env::var("USERPROFILE").unwrap_or_default();
    if home.is_empty() {
        return;
    }

    let winget_pkgs =
        std::path::Path::new(&home).join("AppData\\Local\\Microsoft\\WinGet\\Packages");
    if !winget_pkgs.is_dir() {
        return;
    }

    let mut extra_paths = Vec::new();
    if let Ok(entries) = std::fs::read_dir(&winget_pkgs) {
        for entry in entries.flatten() {
            let pkg_dir = entry.path();
            collect_winget_package_paths(&pkg_dir, &mut extra_paths);
        }
    }
    collect_python_script_paths(&home, &mut extra_paths);
    if extra_paths.is_empty() {
        return;
    }

    let current_path = std::env::var("PATH").unwrap_or_default();
    let new_path = format!("{};{}", extra_paths.join(";"), current_path);
    std::env::set_var("PATH", &new_path);
    tracing::info!(
        added = extra_paths.len(),
        "Refreshed PATH with winget/pip directories"
    );
}

#[cfg(windows)]
fn collect_winget_package_paths(pkg_dir: &std::path::Path, extra_paths: &mut Vec<String>) {
    if let Ok(sub_entries) = std::fs::read_dir(pkg_dir) {
        for sub in sub_entries.flatten() {
            let bin_dir = sub.path().join("bin");
            if bin_dir.is_dir() {
                extra_paths.push(bin_dir.to_string_lossy().to_string());
            }
        }
    }
    if std::fs::read_dir(pkg_dir)
        .map(|rd| {
            rd.flatten()
                .any(|e| e.path().extension().map(|x| x == "exe").unwrap_or(false))
        })
        .unwrap_or(false)
    {
        extra_paths.push(pkg_dir.to_string_lossy().to_string());
    }
}

#[cfg(windows)]
fn collect_python_script_paths(home: &str, extra_paths: &mut Vec<String>) {
    let pip_scripts = std::path::Path::new(home).join("AppData\\Local\\Programs\\Python");
    if !pip_scripts.is_dir() {
        return;
    }
    if let Ok(entries) = std::fs::read_dir(&pip_scripts) {
        for entry in entries.flatten() {
            let scripts = entry.path().join("Scripts");
            if scripts.is_dir() {
                extra_paths.push(scripts.to_string_lossy().to_string());
            }
        }
    }
}

/// POST /api/hands/install - Install a hand from TOML content.
pub async fn install_hand(
    State(state): State<Arc<AppState>>,
    Json(body): Json<serde_json::Value>,
) -> impl IntoResponse {
    let (toml_content, skill_content) = hand_content_from_body(&body);

    if toml_content.is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"error": "Missing toml_content field"})),
        );
    }

    match state
        .kernel
        .hand_registry
        .install_from_content(toml_content, skill_content)
    {
        Ok(def) => (
            StatusCode::OK,
            Json(serde_json::json!({
                "id": def.id,
                "name": def.name,
                "description": def.description,
                "category": format!("{:?}", def.category),
            })),
        ),
        Err(e) => (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"error": format!("{e}")})),
        ),
    }
}

/// POST /api/hands/upsert - Install or update a hand definition.
pub async fn upsert_hand(
    State(state): State<Arc<AppState>>,
    Json(body): Json<serde_json::Value>,
) -> impl IntoResponse {
    let (toml_content, skill_content) = hand_content_from_body(&body);

    if toml_content.is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"error": "Missing toml_content field"})),
        );
    }

    match state
        .kernel
        .hand_registry
        .upsert_from_content(toml_content, skill_content)
    {
        Ok(def) => (
            StatusCode::OK,
            Json(serde_json::json!({
                "id": def.id,
                "name": def.name,
                "description": def.description,
                "category": format!("{:?}", def.category),
            })),
        ),
        Err(e) => (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"error": format!("{e}")})),
        ),
    }
}

fn hand_content_from_body(body: &serde_json::Value) -> (&str, &str) {
    (
        body["toml_content"].as_str().unwrap_or(""),
        body["skill_content"].as_str().unwrap_or(""),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use captain_hands::RequirementType;

    fn test_requirement() -> HandRequirement {
        HandRequirement {
            key: "ffmpeg".to_string(),
            label: "FFmpeg".to_string(),
            requirement_type: RequirementType::Binary,
            check_value: "ffmpeg".to_string(),
            description: None,
            optional: false,
            install: None,
        }
    }

    #[test]
    fn hand_content_from_body_reads_optional_skill_content() {
        let body = serde_json::json!({
            "toml_content": "id = 'demo'",
            "skill_content": "# Demo"
        });

        assert_eq!(hand_content_from_body(&body), ("id = 'demo'", "# Demo"));
    }

    #[test]
    fn hand_content_from_body_defaults_missing_fields_to_empty_strings() {
        assert_eq!(hand_content_from_body(&serde_json::json!({})), ("", ""));
    }

    #[test]
    fn install_command_for_platform_prefers_native_command_then_fallback() {
        let install = HandInstallInfo {
            macos: Some("brew install ffmpeg".to_string()),
            windows: Some("winget install Gyan.FFmpeg".to_string()),
            linux_apt: Some("sudo apt install ffmpeg".to_string()),
            linux_dnf: Some("sudo dnf install ffmpeg-free".to_string()),
            linux_pacman: Some("sudo pacman -S ffmpeg".to_string()),
            pip: Some("pip install ffmpeg-python".to_string()),
            ..HandInstallInfo::default()
        };

        assert_eq!(
            install_command_for_platform(&install, "macos"),
            Some("brew install ffmpeg")
        );
        assert_eq!(
            install_command_for_platform(&install, "windows"),
            Some("winget install Gyan.FFmpeg")
        );
        assert_eq!(
            install_command_for_platform(&install, "linux"),
            Some("sudo apt install ffmpeg")
        );

        let pip_only = HandInstallInfo {
            pip: Some("pip install browser-use".to_string()),
            ..HandInstallInfo::default()
        };
        assert_eq!(
            install_command_for_platform(&pip_only, "macos"),
            Some("pip install browser-use")
        );
    }

    #[test]
    fn command_for_shell_adds_winget_accept_flags_only_on_windows() {
        let command = "winget install Gyan.FFmpeg";

        assert_eq!(
            command_for_shell(command, true),
            "winget install Gyan.FFmpeg --accept-source-agreements --accept-package-agreements"
        );
        assert_eq!(command_for_shell(command, false), command);
        assert_eq!(
            command_for_shell("pip install rich", true),
            "pip install rich"
        );
    }

    #[test]
    fn command_exit_result_classifies_success_and_known_already_installed_output() {
        let req = test_requirement();

        let success = command_exit_result(&req, "brew install ffmpeg", 0, "", "");
        assert_eq!(success["status"], "installed");
        assert_eq!(success["message"], "FFmpeg installed successfully");

        let already_installed = command_exit_result(
            &req,
            "winget install Gyan.FFmpeg",
            1,
            "",
            "No applicable update",
        );
        assert_eq!(already_installed["status"], "installed");
        assert_eq!(already_installed["exit_code"], 1);
        assert_eq!(already_installed["message"], "FFmpeg is already installed");
    }

    #[test]
    fn command_exit_result_truncates_error_message() {
        let req = test_requirement();
        let long_stderr = "x".repeat(620);

        let error = command_exit_result(&req, "brew install ffmpeg", 12, "", &long_stderr);

        assert_eq!(error["status"], "error");
        assert_eq!(error["exit_code"], 12);
        assert_eq!(
            error["message"].as_str().unwrap(),
            format!("Install failed (exit 12): {}", "x".repeat(500))
        );
    }

    #[test]
    fn requirement_status_json_keeps_public_shape() {
        let req = test_requirement();
        let value = requirement_status_json(&(req, true));

        assert_eq!(
            value,
            serde_json::json!({
                "key": "ffmpeg",
                "label": "FFmpeg",
                "satisfied": true,
            })
        );
    }

    #[tokio::test]
    async fn install_requirement_handles_non_exec_branches_without_shelling_out() {
        let req = test_requirement();
        let already = install_requirement("demo", &req, true, "macos").await;
        assert_eq!(already["status"], "already_installed");

        let skipped = install_requirement("demo", &req, false, "macos").await;
        assert_eq!(skipped["status"], "skipped");

        let mut windows_only = test_requirement();
        windows_only.install = Some(HandInstallInfo {
            windows: Some("winget install Example.Tool".to_string()),
            ..HandInstallInfo::default()
        });
        let no_command = install_requirement("demo", &windows_only, false, "macos").await;
        assert_eq!(no_command["status"], "no_command");
        assert_eq!(
            no_command["message"],
            "No install command for platform: macos"
        );
    }

    #[tokio::test]
    async fn install_missing_requirements_preserves_requirement_order() {
        let mut first = test_requirement();
        first.key = "first".to_string();
        let mut second = test_requirement();
        second.key = "second".to_string();

        let results =
            install_missing_requirements("demo", &[(first, true), (second, true)], "macos").await;

        assert_eq!(results.len(), 2);
        assert_eq!(results[0]["key"], "first");
        assert_eq!(results[1]["key"], "second");
    }
}
