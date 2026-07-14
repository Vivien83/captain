//! Docker sandbox runtime handler.

use std::{path::Path, process::Stdio};

use captain_types::config::DockerSandboxConfig;
use tokio::io::{AsyncRead, AsyncReadExt};
use tokio::sync::mpsc;
use tokio::task::JoinHandle;
use tracing::warn;

use super::{emit_tool_chunk, ensure_no_secret_literal};

const MAX_REVIEW_TIMEOUT_SECS: u64 = 7_200;
const MAX_DOCKER_OUTPUT_BYTES: usize = 50_000;

pub(crate) async fn tool_docker_exec(
    input: &serde_json::Value,
    docker_config: Option<&DockerSandboxConfig>,
    workspace_root: Option<&Path>,
    caller_agent_id: Option<&str>,
) -> Result<String, String> {
    let config = docker_config.ok_or("Docker sandbox not configured")?;
    if !config.enabled {
        return Err("Docker sandbox is disabled. Set docker.enabled=true in config.".into());
    }

    let command = input["command"]
        .as_str()
        .ok_or("Missing 'command' parameter")?;
    ensure_no_secret_literal("docker_exec", "command", command)?;
    let requested_timeout = input["timeout_secs"].as_u64().filter(|secs| *secs > 0);
    let timeout_secs = docker_exec_timeout_secs(config.timeout_secs, requested_timeout);

    let workspace = workspace_root.ok_or("Docker exec requires a workspace directory")?;
    let agent_id = caller_agent_id.unwrap_or("default");
    if !crate::docker_sandbox::is_docker_available().await {
        return Err(
            "Docker is not available on this system. Install Docker to use docker_exec.".into(),
        );
    }

    let container = crate::docker_sandbox::create_sandbox(config, agent_id, workspace).await?;
    let result = if requested_timeout.is_some() {
        exec_in_sandbox_with_renewing_reviews(&container, command, timeout_secs).await
    } else {
        let timeout = std::time::Duration::from_secs(timeout_secs);
        crate::docker_sandbox::exec_in_sandbox(&container, command, timeout).await
    };

    if let Err(e) = crate::docker_sandbox::destroy_sandbox(&container).await {
        warn!("Failed to destroy Docker sandbox: {e}");
    }

    let exec_result = result?;
    serde_json::to_string_pretty(&serde_json::json!({
        "exit_code": exec_result.exit_code,
        "stdout": exec_result.stdout,
        "stderr": exec_result.stderr,
        "container_id": container.container_id,
        "timeout_secs": timeout_secs,
        "timeout_mode": if requested_timeout.is_some() { "review_window" } else { "hard_timeout" },
    }))
    .map_err(|e| format!("Serialize error: {e}"))
}

fn docker_exec_timeout_secs(config_timeout_secs: u64, requested_timeout: Option<u64>) -> u64 {
    match requested_timeout {
        Some(secs) => secs.clamp(1, MAX_REVIEW_TIMEOUT_SECS),
        None => config_timeout_secs.max(1),
    }
}

enum DockerStreamEvent {
    Stdout(Vec<u8>),
    Stderr(Vec<u8>),
}

#[derive(Clone, Copy)]
enum DockerStreamKind {
    Stdout,
    Stderr,
}

impl DockerStreamKind {
    fn label(self) -> &'static str {
        match self {
            DockerStreamKind::Stdout => "stdout",
            DockerStreamKind::Stderr => "stderr",
        }
    }

    fn event(self, chunk: Vec<u8>) -> DockerStreamEvent {
        match self {
            DockerStreamKind::Stdout => DockerStreamEvent::Stdout(chunk),
            DockerStreamKind::Stderr => DockerStreamEvent::Stderr(chunk),
        }
    }
}

async fn exec_in_sandbox_with_renewing_reviews(
    container: &crate::docker_sandbox::SandboxContainer,
    command: &str,
    timeout_secs: u64,
) -> Result<crate::docker_sandbox::ExecResult, String> {
    crate::docker_sandbox::validate_command(command)?;

    let mut child = spawn_docker_exec_process(container, command)?;
    let (stdout, stderr) = take_docker_exec_pipes(&mut child)?;
    let (rx, stdout_task, stderr_task) = spawn_docker_stream_tasks(stdout, stderr);
    let (status, stdout_seen, stderr_seen) =
        wait_for_docker_exec_with_reviews(&mut child, rx, timeout_secs).await?;
    let (stdout, stderr) =
        collect_docker_streams(stdout_task, stderr_task, stdout_seen, stderr_seen).await;

    Ok(crate::docker_sandbox::ExecResult {
        stdout: truncate_docker_output(String::from_utf8_lossy(&stdout).to_string()),
        stderr: truncate_docker_output(String::from_utf8_lossy(&stderr).to_string()),
        exit_code: status.code().unwrap_or(-1),
    })
}

fn spawn_docker_exec_process(
    container: &crate::docker_sandbox::SandboxContainer,
    command: &str,
) -> Result<tokio::process::Child, String> {
    let mut cmd = tokio::process::Command::new("docker");
    cmd.arg("exec")
        .arg(&container.container_id)
        .arg("sh")
        .arg("-c")
        .arg(command);
    cmd.stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());

    cmd.spawn()
        .map_err(|error| format!("Docker exec failed: {error}"))
}

fn take_docker_exec_pipes(
    child: &mut tokio::process::Child,
) -> Result<(tokio::process::ChildStdout, tokio::process::ChildStderr), String> {
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| "Docker exec stdout pipe missing".to_string())?;
    let stderr = child
        .stderr
        .take()
        .ok_or_else(|| "Docker exec stderr pipe missing".to_string())?;
    Ok((stdout, stderr))
}

fn spawn_docker_stream_tasks(
    stdout: tokio::process::ChildStdout,
    stderr: tokio::process::ChildStderr,
) -> (
    mpsc::Receiver<DockerStreamEvent>,
    JoinHandle<Vec<u8>>,
    JoinHandle<Vec<u8>>,
) {
    let (tx, rx) = mpsc::channel::<DockerStreamEvent>(32);
    let stdout_task = spawn_docker_stream_reader(DockerStreamKind::Stdout, stdout, tx.clone());
    let stderr_task = spawn_docker_stream_reader(DockerStreamKind::Stderr, stderr, tx);
    (rx, stdout_task, stderr_task)
}

fn spawn_docker_stream_reader<R>(
    kind: DockerStreamKind,
    mut reader: R,
    tx: mpsc::Sender<DockerStreamEvent>,
) -> JoinHandle<Vec<u8>>
where
    R: AsyncRead + Unpin + Send + 'static,
{
    tokio::spawn(async move {
        let mut collected = Vec::new();
        let mut buf = [0u8; 4096];
        loop {
            match reader.read(&mut buf).await {
                Ok(0) => break,
                Ok(n) => {
                    let chunk = buf[..n].to_vec();
                    emit_tool_chunk(kind.label(), &String::from_utf8_lossy(&chunk));
                    collected.extend_from_slice(&chunk);
                    let _ = tx.send(kind.event(chunk)).await;
                }
                Err(_) => break,
            }
        }
        collected
    })
}

async fn wait_for_docker_exec_with_reviews(
    child: &mut tokio::process::Child,
    mut rx: mpsc::Receiver<DockerStreamEvent>,
    timeout_secs: u64,
) -> Result<(std::process::ExitStatus, Vec<u8>, Vec<u8>), String> {
    let review_interval = std::time::Duration::from_secs(timeout_secs.clamp(1, 30));
    let mut review = Box::pin(tokio::time::sleep(review_interval));
    let wait = child.wait();
    tokio::pin!(wait);
    let mut stdout_seen = Vec::new();
    let mut stderr_seen = Vec::new();

    let status = loop {
        tokio::select! {
            status = &mut wait => break status.map_err(|e| format!("Docker exec wait failed: {e}"))?,
            maybe_event = rx.recv() => {
                match maybe_event {
                    Some(DockerStreamEvent::Stdout(chunk)) => stdout_seen.extend_from_slice(&chunk),
                    Some(DockerStreamEvent::Stderr(chunk)) => stderr_seen.extend_from_slice(&chunk),
                    None => {}
                }
            }
            _ = &mut review => {
                let msg = format!(
                    "Docker exec still running; process is alive. timeout_secs={} is a review window, not a kill deadline.\n",
                    timeout_secs,
                );
                emit_tool_chunk("progress", &msg);
                review.as_mut().reset(tokio::time::Instant::now() + review_interval);
            }
        }
    };
    Ok((status, stdout_seen, stderr_seen))
}

async fn collect_docker_streams(
    stdout_task: JoinHandle<Vec<u8>>,
    stderr_task: JoinHandle<Vec<u8>>,
    stdout_seen: Vec<u8>,
    stderr_seen: Vec<u8>,
) -> (Vec<u8>, Vec<u8>) {
    let stdout_final = stdout_task.await.unwrap_or_default();
    let stderr_final = stderr_task.await.unwrap_or_default();
    (
        prefer_complete_stream(stdout_final, stdout_seen),
        prefer_complete_stream(stderr_final, stderr_seen),
    )
}

fn prefer_complete_stream(final_output: Vec<u8>, seen_output: Vec<u8>) -> Vec<u8> {
    if final_output.len() >= seen_output.len() {
        final_output
    } else {
        seen_output
    }
}

fn truncate_docker_output(output: String) -> String {
    if output.len() > MAX_DOCKER_OUTPUT_BYTES {
        let safe_end = crate::str_utils::safe_truncate_str(&output, MAX_DOCKER_OUTPUT_BYTES);
        format!("{}... [truncated, {} total bytes]", safe_end, output.len())
    } else {
        output
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_timeout_uses_config_with_minimum_one_second() {
        assert_eq!(docker_exec_timeout_secs(0, None), 1);
        assert_eq!(docker_exec_timeout_secs(90, None), 90);
    }

    #[test]
    fn explicit_timeout_is_bounded_review_window() {
        assert_eq!(docker_exec_timeout_secs(60, Some(1)), 1);
        assert_eq!(docker_exec_timeout_secs(60, Some(9_999)), 7_200);
    }

    #[test]
    fn docker_stream_kind_labels_and_events_match_pipe() {
        assert_eq!(DockerStreamKind::Stdout.label(), "stdout");
        assert_eq!(DockerStreamKind::Stderr.label(), "stderr");

        match DockerStreamKind::Stdout.event(b"out".to_vec()) {
            DockerStreamEvent::Stdout(chunk) => assert_eq!(chunk, b"out"),
            DockerStreamEvent::Stderr(_) => panic!("stdout kind must produce stdout event"),
        }
        match DockerStreamKind::Stderr.event(b"err".to_vec()) {
            DockerStreamEvent::Stderr(chunk) => assert_eq!(chunk, b"err"),
            DockerStreamEvent::Stdout(_) => panic!("stderr kind must produce stderr event"),
        }
    }

    #[test]
    fn prefer_complete_stream_uses_longer_collected_output() {
        assert_eq!(
            prefer_complete_stream(b"complete".to_vec(), b"seen".to_vec()),
            b"complete"
        );
        assert_eq!(
            prefer_complete_stream(b"short".to_vec(), b"longer-seen".to_vec()),
            b"longer-seen"
        );
    }
}
