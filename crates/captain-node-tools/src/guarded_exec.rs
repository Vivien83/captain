use captain_types::config::{ExecPolicy, ExecSecurityMode};
use std::{path::Path, process::Stdio};

pub(crate) fn reviewed_command(
    command: &str,
    workspace_root: &Path,
    policy: &ExecPolicy,
) -> Result<tokio::process::Command, String> {
    crate::shell_guard::review(command, policy)
        .map_err(|_| "The local Node execution policy denied this shell command".to_string())?;

    let mut process = if policy.effective_mode() == ExecSecurityMode::Allowlist {
        direct_command(command)?
    } else {
        shell_command(command)
    };
    apply_workspace_boundary(&mut process, workspace_root);
    Ok(process)
}

fn direct_command(command: &str) -> Result<tokio::process::Command, String> {
    let arguments = shlex::split(command)
        .ok_or_else(|| "Command contains unmatched quotes or invalid shell syntax".to_string())?;
    let executable = arguments
        .first()
        .ok_or_else(|| "Empty command after parsing".to_string())?;
    let mut process = tokio::process::Command::new(executable);
    process.args(&arguments[1..]);
    Ok(process)
}

fn shell_command(command: &str) -> tokio::process::Command {
    #[cfg(windows)]
    let process = {
        let mut process = tokio::process::Command::new("cmd");
        process.arg("/C").arg(command);
        process
    };
    #[cfg(not(windows))]
    let process = {
        let mut process = tokio::process::Command::new("sh");
        process.arg("-c").arg(command);
        process
    };
    process
}

fn apply_workspace_boundary(process: &mut tokio::process::Command, workspace_root: &Path) {
    process.env_clear();
    for name in [
        "PATH", "HOME", "TMPDIR", "TMP", "TEMP", "LANG", "LC_ALL", "TERM",
    ] {
        if let Ok(value) = std::env::var(name) {
            process.env(name, value);
        }
    }
    #[cfg(windows)]
    for name in [
        "USERPROFILE",
        "SYSTEMROOT",
        "APPDATA",
        "LOCALAPPDATA",
        "COMSPEC",
        "WINDIR",
        "PATHEXT",
    ] {
        if let Ok(value) = std::env::var(name) {
            process.env(name, value);
        }
    }
    process.current_dir(workspace_root);
    process.stdin(Stdio::null());
    process.kill_on_drop(true);
    #[cfg(unix)]
    process.process_group(0);
    #[cfg(windows)]
    process.env("PYTHONIOENCODING", "utf-8");
}

pub(crate) async fn terminate(child: &mut tokio::process::Child) {
    #[cfg(unix)]
    if let Some(pid) = child.id() {
        let _ = tokio::process::Command::new("kill")
            .args(["-TERM", &format!("-{}", pid as i32)])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .await;
    }
    let _ = child.start_kill();
    let _ = child.wait().await;
}
