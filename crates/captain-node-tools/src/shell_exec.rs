use captain_types::config::ExecPolicy;
use std::{path::Path, process::Stdio, time::Duration};
use tokio::io::AsyncReadExt;

pub(crate) async fn execute(
    input: &serde_json::Value,
    workspace_root: &Path,
    policy: &ExecPolicy,
) -> Result<String, String> {
    let command = input
        .get("command")
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| "Missing 'command' parameter".to_string())?;
    let requested_timeout = input
        .get("timeout_seconds")
        .and_then(serde_json::Value::as_u64);
    let timeout_secs = requested_timeout.unwrap_or(policy.timeout_secs).max(1);
    let hard_cap_secs = if requested_timeout.is_some() {
        timeout_secs.saturating_mul(3).max(timeout_secs + 2)
    } else {
        timeout_secs
    };
    let mut process = crate::guarded_exec::reviewed_command(command, workspace_root, policy)?;
    process.stdout(Stdio::piped()).stderr(Stdio::piped());

    let mut child = process
        .spawn()
        .map_err(|error| format!("Failed to execute command: {error}"))?;
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| "stdout pipe missing".to_string())?;
    let stderr = child
        .stderr
        .take()
        .ok_or_else(|| "stderr pipe missing".to_string())?;
    let cap = policy.max_output_bytes.max(1);
    let stdout_task = tokio::spawn(read_stream(stdout, cap));
    let stderr_task = tokio::spawn(read_stream(stderr, cap));
    let status = match tokio::time::timeout(Duration::from_secs(hard_cap_secs), child.wait()).await
    {
        Ok(Ok(status)) => status,
        Ok(Err(error)) => {
            crate::guarded_exec::terminate(&mut child).await;
            return Err(format!("Failed to wait for command: {error}"));
        }
        Err(_) => {
            crate::guarded_exec::terminate(&mut child).await;
            return Err(format!(
                "Command exceeded bounded review window after {hard_cap_secs}s"
            ));
        }
    };
    let stdout = stdout_task
        .await
        .map_err(|_| "stdout reader failed".to_string())?;
    let stderr = stderr_task
        .await
        .map_err(|_| "stderr reader failed".to_string())?;
    Ok(format_output(
        status.code().unwrap_or(-1),
        stdout,
        stderr,
        cap,
    ))
}

struct Captured {
    bytes: Vec<u8>,
    total: usize,
}

async fn read_stream<R>(mut stream: R, cap: usize) -> Captured
where
    R: tokio::io::AsyncRead + Unpin,
{
    let mut bytes = Vec::new();
    let mut total = 0usize;
    let mut buffer = [0u8; 4096];
    loop {
        match stream.read(&mut buffer).await {
            Ok(0) | Err(_) => break,
            Ok(read) => {
                total = total.saturating_add(read);
                let remaining = cap.saturating_sub(bytes.len());
                bytes.extend_from_slice(&buffer[..read.min(remaining)]);
            }
        }
    }
    Captured { bytes, total }
}

fn format_output(exit_code: i32, stdout: Captured, stderr: Captured, cap: usize) -> String {
    fn render(capture: Captured, cap: usize) -> String {
        let mut value = String::from_utf8_lossy(&capture.bytes).to_string();
        if capture.total > cap {
            value.push_str(&format!("...\n[truncated, {} total bytes]", capture.total));
        }
        value
    }
    format!(
        "Exit code: {exit_code}\n\nSTDOUT:\n{}\nSTDERR:\n{}",
        render(stdout, cap),
        render(stderr, cap)
    )
}
