//! Self-update dispatch: shells out to `captain update` so the CLI stays the
//! single source of truth for version resolution, checksum verification, the
//! binary swap recipe and the container-detection message.

use std::sync::Arc;

use crate::kernel_handle::KernelHandle;

pub(crate) async fn dispatch_system_update(
    input: &serde_json::Value,
    kernel: Option<&Arc<dyn KernelHandle>>,
    caller_agent_id: Option<&str>,
) -> Result<String, String> {
    let check_only = input["check_only"].as_bool().unwrap_or(false);
    let exe = std::env::current_exe().map_err(|e| format!("cannot locate captain binary: {e}"))?;

    // Both paths start with a check: it produces the "installed vs available"
    // summary shown in the approval prompt, and short-circuits when already
    // up to date. Run on a blocking thread — this does network I/O.
    let exe_for_check = exe.clone();
    let check_output = tokio::task::spawn_blocking(move || {
        std::process::Command::new(&exe_for_check)
            .args(["update", "--check"])
            .output()
    })
    .await
    .map_err(|e| format!("update check task failed: {e}"))?
    .map_err(|e| format!("failed to run `captain update --check`: {e}"))?;

    let check_text = format!(
        "{}{}",
        String::from_utf8_lossy(&check_output.stdout),
        String::from_utf8_lossy(&check_output.stderr)
    )
    .trim()
    .to_string();

    if check_only || !check_output.status.success() {
        return Ok(check_text);
    }
    if check_text.contains("already up to date") || check_text.contains("runs inside a container") {
        return Ok(check_text);
    }

    // A binary swap + daemon restart is always user-approved, regardless of
    // the configurable ApprovalPolicy — same forced-approval stance as
    // shell_exec's critical patterns.
    let agent_id = caller_agent_id.unwrap_or("unknown");
    let Some(kernel) = kernel else {
        return Err("system_update requires a kernel handle for user approval".to_string());
    };
    let summary = format!("Update Captain and restart the daemon:\n{check_text}");
    let approved = kernel
        .request_approval(agent_id, "system_update", &summary)
        .await
        .map_err(|e| format!("approval request failed: {e}"))?;
    if !approved {
        return Ok("Mise à jour refusée par l'utilisateur — rien n'a été modifié.".to_string());
    }

    // The updater must outlive this daemon (the restart kills us), so it is
    // detached nohup-style with a small delay letting the current turn's
    // response reach the user first. Output goes to a log for post-mortem.
    let home = crate::native_embeddings::captain_home_dir();
    let log_path = home.join("update.log");
    let script = format!(
        "sleep 3; {} update --yes >> {} 2>&1",
        shell_quote(&exe.display().to_string()),
        shell_quote(&log_path.display().to_string()),
    );
    std::process::Command::new("/bin/sh")
        .arg("-c")
        .arg(format!("nohup /bin/sh -c '{script}' >/dev/null 2>&1 &"))
        .spawn()
        .map_err(|e| format!("failed to launch detached updater: {e}"))?;

    Ok(format!(
        "Mise à jour lancée en arrière-plan — le daemon va redémarrer dans quelques secondes \
         (journal: {}).\n{check_text}",
        log_path.display()
    ))
}

fn shell_quote(value: &str) -> String {
    format!("\"{}\"", value.replace('"', "\\\""))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shell_quote_wraps_and_escapes() {
        assert_eq!(shell_quote("/plain/path"), "\"/plain/path\"");
        assert_eq!(shell_quote("a\"b"), "\"a\\\"b\"");
    }
}
