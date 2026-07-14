//! Sandboxed code execution tool handler.

use crate::tools::{emit_tool_chunk, ensure_no_secret_literal};
use std::path::Path;
use tokio::io::{AsyncRead, AsyncReadExt};

pub(crate) async fn tool_execute_code(
    input: &serde_json::Value,
    workspace_root: Option<&Path>,
) -> Result<String, String> {
    let code = input["code"].as_str().ok_or("Missing 'code' parameter")?;
    if code.trim().is_empty() {
        return Err("'code' cannot be empty".to_string());
    }
    ensure_no_secret_literal("execute_code", "code", code)?;
    let language = input["language"]
        .as_str()
        .unwrap_or("python")
        .to_lowercase();
    let requested_timeout = input["timeout_secs"].as_u64();
    let timeout_secs = requested_timeout.unwrap_or(60).clamp(1, 300);

    let (interpreter, args_prefix) = match language.as_str() {
        "python" | "py" => (
            find_python_interpreter(),
            vec!["-u".to_string(), "-c".to_string()],
        ),
        "node" | "js" | "javascript" => ("node".to_string(), vec!["-e".to_string()]),
        "bash" | "sh" | "shell" => ("bash".to_string(), vec!["-c".to_string()]),
        other => return Err(format!("Unsupported language: {other}")),
    };

    // Models routinely send `pip_install: []` as a boilerplate default field
    // on every execute_code call regardless of language — reject only when
    // it actually asks to install something, not merely when the (empty)
    // field is present, or every non-Python call from such a model fails.
    let pip_install_requests_packages = input["pip_install"]
        .as_array()
        .is_some_and(|pkgs| !pkgs.is_empty());
    if pip_install_requests_packages && language != "python" && language != "py" {
        return Err("pip_install is only valid for language=python".into());
    }

    if let Some(pkgs) = input["pip_install"].as_array() {
        let packages: Vec<String> = pkgs
            .iter()
            .filter_map(|value| value.as_str().map(String::from))
            .collect();
        validate_pip_allowlist(&packages)?;
        run_pip_install(&interpreter, &packages).await?;
    }

    let mut cmd = tokio::process::Command::new(&interpreter);
    cmd.args(&args_prefix);
    cmd.arg(code);
    if let Some(workspace) = workspace_root {
        cmd.current_dir(workspace);
    }
    crate::env_sandbox::apply_minimal_env(&mut cmd);
    cmd.stdin(std::process::Stdio::null());
    cmd.stdout(std::process::Stdio::piped());
    cmd.stderr(std::process::Stdio::piped());

    let mut child = cmd
        .spawn()
        .map_err(|e| format!("Failed to spawn {interpreter}: {e}"))?;

    let (stdout_collected, stderr_collected, status) =
        run_with_streaming(&mut child, timeout_secs, requested_timeout.is_some()).await?;
    let stdout = String::from_utf8_lossy(&stdout_collected).to_string();
    let stderr = String::from_utf8_lossy(&stderr_collected).to_string();

    Ok(serde_json::json!({
        "language": language,
        "interpreter": interpreter,
        "exit_code": status.code(),
        "stdout": truncate_for_output(&stdout),
        "stderr": truncate_for_output(&stderr),
    })
    .to_string())
}

enum CodeStreamEvent {
    Stdout(Vec<u8>),
    Stderr(Vec<u8>),
}

struct CodeStreamTasks {
    stdout: tokio::task::JoinHandle<Vec<u8>>,
    stderr: tokio::task::JoinHandle<Vec<u8>>,
    rx: tokio::sync::mpsc::Receiver<CodeStreamEvent>,
}

async fn run_with_streaming(
    child: &mut tokio::process::Child,
    timeout_secs: u64,
    renew_while_alive: bool,
) -> Result<(Vec<u8>, Vec<u8>, std::process::ExitStatus), String> {
    let (stdout, stderr) = take_child_pipes(child)?;
    let mut streams = spawn_code_stream_tasks(stdout, stderr);

    let (mut stdout_seen, mut stderr_seen, status) = if renew_while_alive {
        wait_with_review_window(child, timeout_secs, &mut streams.rx).await?
    } else {
        let status =
            wait_with_hard_timeout(child, timeout_secs, &streams.stdout, &streams.stderr).await?;
        (Vec::new(), Vec::new(), status)
    };

    drain_stream_events(&mut streams.rx, &mut stdout_seen, &mut stderr_seen);
    let (stdout_final, stderr_final) = await_stream_tasks(streams).await;
    Ok((
        prefer_complete_stream(stdout_final, stdout_seen),
        prefer_complete_stream(stderr_final, stderr_seen),
        status,
    ))
}

fn take_child_pipes(
    child: &mut tokio::process::Child,
) -> Result<(tokio::process::ChildStdout, tokio::process::ChildStderr), String> {
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| "stdout pipe missing".to_string())?;
    let stderr = child
        .stderr
        .take()
        .ok_or_else(|| "stderr pipe missing".to_string())?;
    Ok((stdout, stderr))
}

fn spawn_code_stream_tasks(
    stdout: tokio::process::ChildStdout,
    stderr: tokio::process::ChildStderr,
) -> CodeStreamTasks {
    let (tx, rx) = tokio::sync::mpsc::channel::<CodeStreamEvent>(32);
    let stdout_tx = tx.clone();
    let stderr_tx = tx.clone();
    let stdout = tokio::spawn(read_streaming_pipe(
        stdout,
        "stdout",
        stdout_tx,
        CodeStreamEvent::Stdout,
    ));
    let stderr = tokio::spawn(read_streaming_pipe(
        stderr,
        "stderr",
        stderr_tx,
        CodeStreamEvent::Stderr,
    ));
    drop(tx);
    CodeStreamTasks { stdout, stderr, rx }
}

async fn read_streaming_pipe<R>(
    mut reader: R,
    stream_name: &'static str,
    tx: tokio::sync::mpsc::Sender<CodeStreamEvent>,
    event: fn(Vec<u8>) -> CodeStreamEvent,
) -> Vec<u8>
where
    R: AsyncRead + Unpin + Send + 'static,
{
    let mut buf = [0u8; 4096];
    let mut collected = Vec::new();
    loop {
        match reader.read(&mut buf).await {
            Ok(0) => break,
            Ok(n) => {
                collected.extend_from_slice(&buf[..n]);
                let chunk = String::from_utf8_lossy(&buf[..n]).to_string();
                emit_tool_chunk(stream_name, &chunk);
                let _ = tx.send(event(buf[..n].to_vec())).await;
            }
            Err(_) => break,
        }
    }
    collected
}

async fn wait_with_review_window(
    child: &mut tokio::process::Child,
    timeout_secs: u64,
    rx: &mut tokio::sync::mpsc::Receiver<CodeStreamEvent>,
) -> Result<(Vec<u8>, Vec<u8>, std::process::ExitStatus), String> {
    let mut stdout_seen = Vec::new();
    let mut stderr_seen = Vec::new();
    let wait_fut = child.wait();
    tokio::pin!(wait_fut);
    let review_interval = std::time::Duration::from_secs(timeout_secs.clamp(1, 30));
    let review = tokio::time::sleep(review_interval);
    tokio::pin!(review);

    let status = loop {
        tokio::select! {
            status = &mut wait_fut => break status.map_err(|e| format!("wait failed: {e}"))?,
            maybe_event = rx.recv() => {
                if let Some(event) = maybe_event {
                    append_stream_event(event, &mut stdout_seen, &mut stderr_seen);
                }
            },
            _ = &mut review => {
                emit_review_progress(timeout_secs);
                review
                    .as_mut()
                    .reset(tokio::time::Instant::now() + review_interval);
            }
        }
    };
    Ok((stdout_seen, stderr_seen, status))
}

async fn wait_with_hard_timeout(
    child: &mut tokio::process::Child,
    timeout_secs: u64,
    stdout_task: &tokio::task::JoinHandle<Vec<u8>>,
    stderr_task: &tokio::task::JoinHandle<Vec<u8>>,
) -> Result<std::process::ExitStatus, String> {
    let timed = {
        let wait_fut = child.wait();
        tokio::pin!(wait_fut);
        tokio::time::timeout(std::time::Duration::from_secs(timeout_secs), &mut wait_fut).await
    };
    match timed {
        Ok(status) => status.map_err(|e| format!("wait failed: {e}")),
        Err(_) => {
            let _ = child.start_kill();
            stdout_task.abort();
            stderr_task.abort();
            Err(format!("process timed out after {timeout_secs}s"))
        }
    }
}

fn emit_review_progress(timeout_secs: u64) {
    emit_tool_chunk(
        "progress",
        &format!(
            "Code execution still running; process is alive. timeout_secs={} is a review window, not a kill deadline.\n",
            timeout_secs,
        ),
    );
}

fn drain_stream_events(
    rx: &mut tokio::sync::mpsc::Receiver<CodeStreamEvent>,
    stdout_seen: &mut Vec<u8>,
    stderr_seen: &mut Vec<u8>,
) {
    while let Ok(event) = rx.try_recv() {
        append_stream_event(event, stdout_seen, stderr_seen);
    }
}

fn append_stream_event(
    event: CodeStreamEvent,
    stdout_seen: &mut Vec<u8>,
    stderr_seen: &mut Vec<u8>,
) {
    match event {
        CodeStreamEvent::Stdout(chunk) => stdout_seen.extend_from_slice(&chunk),
        CodeStreamEvent::Stderr(chunk) => stderr_seen.extend_from_slice(&chunk),
    }
}

async fn await_stream_tasks(streams: CodeStreamTasks) -> (Vec<u8>, Vec<u8>) {
    let stdout = streams.stdout.await.unwrap_or_default();
    let stderr = streams.stderr.await.unwrap_or_default();
    (stdout, stderr)
}

fn prefer_complete_stream(final_bytes: Vec<u8>, seen_bytes: Vec<u8>) -> Vec<u8> {
    if final_bytes.len() >= seen_bytes.len() {
        final_bytes
    } else {
        seen_bytes
    }
}

const PIP_ALLOWLIST: &[&str] = &[
    "requests",
    "httpx",
    "beautifulsoup4",
    "lxml",
    "pandas",
    "numpy",
    "pyyaml",
    "python-dateutil",
    "pyobjc-framework-Quartz",
    "pillow",
    "pydantic",
    "rich",
];

pub(crate) fn validate_pip_allowlist(packages: &[String]) -> Result<(), String> {
    for package in packages {
        let normalized = package
            .split(&['=', '<', '>', '!', '~'][..])
            .next()
            .unwrap_or("")
            .trim();
        if normalized.is_empty() {
            return Err(format!("Invalid package spec: '{package}'"));
        }
        if !PIP_ALLOWLIST.contains(&normalized) {
            return Err(format!(
                "Package '{normalized}' not in pip allowlist. \
                 Extend PIP_ALLOWLIST in tools/code_execution.rs to authorize it."
            ));
        }
    }
    Ok(())
}

async fn run_pip_install(interpreter: &str, packages: &[String]) -> Result<(), String> {
    if packages.is_empty() {
        return Ok(());
    }
    let mut cmd = tokio::process::Command::new(interpreter);
    cmd.args(["-m", "pip", "install", "--user", "--quiet", "--no-input"]);
    for package in packages {
        cmd.arg(package);
    }
    let output = tokio::time::timeout(std::time::Duration::from_secs(60), cmd.output())
        .await
        .map_err(|_| "pip install timed out after 60s".to_string())?
        .map_err(|e| format!("pip install spawn failed: {e}"))?;

    if !output.status.success() {
        return Err(format!(
            "pip install failed (exit {}): {}",
            output.status.code().unwrap_or(-1),
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    Ok(())
}

pub(crate) fn find_python_interpreter() -> String {
    for candidate in ["python3", "python"] {
        if std::process::Command::new("which")
            .arg(candidate)
            .output()
            .map(|output| output.status.success())
            .unwrap_or(false)
        {
            return candidate.to_string();
        }
    }
    "python3".to_string()
}

fn truncate_for_output(text: &str) -> String {
    const MAX: usize = 100_000;
    if text.chars().count() <= MAX {
        text.to_string()
    } else {
        let head: String = text.chars().take(MAX).collect();
        format!("{head}...[truncated {} chars]", text.chars().count() - MAX)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(unix)]
    #[tokio::test]
    async fn explicit_timeout_is_review_window_for_live_code() {
        use serde_json::json;
        let output = tool_execute_code(
            &json!({
                "language": "bash",
                "code": "sleep 2; printf healthy",
                "timeout_secs": 1
            }),
            None,
        )
        .await
        .expect("explicit timeout should not kill live code");

        assert!(output.contains("\"exit_code\":0"));
        assert!(output.contains("healthy"));
    }

    #[test]
    fn prefer_complete_stream_keeps_longest_buffer() {
        assert_eq!(
            prefer_complete_stream(vec![1, 2, 3], vec![1]),
            vec![1, 2, 3]
        );
        assert_eq!(
            prefer_complete_stream(vec![1], vec![1, 2, 3]),
            vec![1, 2, 3]
        );
    }

    #[test]
    fn append_stream_event_routes_stdout_and_stderr() {
        let mut stdout = Vec::new();
        let mut stderr = Vec::new();

        append_stream_event(
            CodeStreamEvent::Stdout(b"out".to_vec()),
            &mut stdout,
            &mut stderr,
        );
        append_stream_event(
            CodeStreamEvent::Stderr(b"err".to_vec()),
            &mut stdout,
            &mut stderr,
        );

        assert_eq!(stdout, b"out");
        assert_eq!(stderr, b"err");
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn default_timeout_stays_a_hard_guard() {
        let mut child = tokio::process::Command::new("bash")
            .arg("-c")
            .arg("sleep 2")
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()
            .expect("spawn bash");

        let err = run_with_streaming(&mut child, 1, false)
            .await
            .expect_err("default guard should time out");

        assert!(err.contains("process timed out after 1s"));
    }
}
