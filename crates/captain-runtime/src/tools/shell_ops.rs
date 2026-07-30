//! Shell execution handler.

use crate::tools::emit_tool_chunk;
use std::path::Path;
use tokio::io::{AsyncRead, AsyncReadExt};

pub(crate) async fn tool_shell_exec(
    input: &serde_json::Value,
    allowed_env: &[String],
    workspace_root: Option<&Path>,
    exec_policy: Option<&captain_types::config::ExecPolicy>,
    permit: crate::guarded_exec::ExecPermit,
) -> Result<String, String> {
    let command = input["command"].as_str().unwrap_or_default();
    if !permit.authorizes(crate::guarded_exec::ExecSurface::ShellTool, command) {
        return Err("Guarded execution permit does not authorize shell_exec".to_string());
    }
    crate::guarded_exec::validate_environment_inputs(allowed_env, &[])?;
    let options = parse_shell_exec_options(input, exec_policy)?;
    let mut cmd =
        crate::guarded_exec::build_shell_command(options.command, options.use_direct_exec, false)?;
    crate::guarded_exec::configure_tokio_command(&mut cmd, workspace_root, allowed_env, &[]);
    let max_output_bytes = exec_policy
        .map(|policy| policy.max_output_bytes)
        .unwrap_or(100_000);

    if options.requested_timeout {
        return run_shell_with_renewing_reviews(cmd, options.timeout_secs, max_output_bytes).await;
    }

    let outcome = crate::guarded_exec::run_tokio_command(
        crate::guarded_exec::ExecSurface::ShellTool,
        cmd,
        options.timeout_secs,
        0,
        max_output_bytes,
    )
    .await?;
    Ok(format_shell_output_with_totals(
        outcome.exit_code,
        outcome.stdout.as_bytes(),
        outcome.stdout_total_bytes,
        outcome.stderr.as_bytes(),
        outcome.stderr_total_bytes,
        max_output_bytes,
    ))
}

#[derive(Debug)]
struct ShellExecOptions<'a> {
    command: &'a str,
    timeout_secs: u64,
    requested_timeout: bool,
    use_direct_exec: bool,
}

fn parse_shell_exec_options<'a>(
    input: &'a serde_json::Value,
    exec_policy: Option<&captain_types::config::ExecPolicy>,
) -> Result<ShellExecOptions<'a>, String> {
    let command = input["command"]
        .as_str()
        .ok_or("Missing 'command' parameter")?;

    let policy_timeout = exec_policy.map(|p| p.timeout_secs).unwrap_or(30);
    let requested_timeout = input["timeout_seconds"].as_u64();
    let use_direct_exec = exec_policy
        .map(|p| p.mode == captain_types::config::ExecSecurityMode::Allowlist)
        .unwrap_or(true);

    Ok(ShellExecOptions {
        command,
        timeout_secs: requested_timeout.unwrap_or(policy_timeout),
        requested_timeout: requested_timeout.is_some(),
        use_direct_exec,
    })
}

enum ShellStreamEvent {
    Stdout(Vec<u8>),
    Stderr(Vec<u8>),
}

#[derive(Default)]
struct ShellStreamCapture {
    bytes: Vec<u8>,
    total_bytes: usize,
}

async fn run_shell_with_renewing_reviews(
    mut cmd: tokio::process::Command,
    timeout_secs: u64,
    max_output_bytes: usize,
) -> Result<String, String> {
    let started = std::time::Instant::now();
    crate::guarded_exec::audit_execution_started(
        crate::guarded_exec::ExecSurface::ShellTool,
        timeout_secs,
    );
    let ShellProcessParts {
        mut child,
        stdout,
        stderr,
    } = match spawn_shell_process_with_pipes(&mut cmd) {
        Ok(parts) => parts,
        Err(error) => {
            crate::guarded_exec::audit_execution_failed(
                crate::guarded_exec::ExecSurface::ShellTool,
                "spawn_failed",
                started.elapsed(),
            );
            return Err(error);
        }
    };
    let (tx, mut rx) = tokio::sync::mpsc::channel::<ShellStreamEvent>(32);

    let stdout_task = spawn_shell_stream_reader(
        stdout,
        "stdout",
        tx.clone(),
        ShellStreamEvent::Stdout,
        max_output_bytes,
    );
    let stderr_task = spawn_shell_stream_reader(
        stderr,
        "stderr",
        tx.clone(),
        ShellStreamEvent::Stderr,
        max_output_bytes,
    );
    drop(tx);

    let ShellWaitResult {
        status,
        stdout_seen,
        stderr_seen,
    } = match wait_for_shell_with_review_window(&mut child, &mut rx, timeout_secs, max_output_bytes)
        .await
    {
        Ok(result) => result,
        Err(error) => {
            crate::guarded_exec::audit_execution_failed(
                crate::guarded_exec::ExecSurface::ShellTool,
                "execution_failed",
                started.elapsed(),
            );
            return Err(error);
        }
    };

    let stdout_final = stdout_task.await.unwrap_or_default();
    let stderr_final = stderr_task.await.unwrap_or_default();
    let stdout = complete_shell_stream(stdout_final, stdout_seen);
    let stderr = complete_shell_stream(stderr_final, stderr_seen);
    let exit_code = status.code().unwrap_or(-1);
    crate::guarded_exec::audit_execution_finished(
        crate::guarded_exec::ExecSurface::ShellTool,
        exit_code,
        started.elapsed().as_millis() as u64,
    );
    Ok(format_shell_output_with_totals(
        exit_code,
        &stdout.bytes,
        stdout.total_bytes,
        &stderr.bytes,
        stderr.total_bytes,
        max_output_bytes,
    ))
}

struct ShellProcessParts {
    child: tokio::process::Child,
    stdout: tokio::process::ChildStdout,
    stderr: tokio::process::ChildStderr,
}

struct ShellWaitResult {
    status: std::process::ExitStatus,
    stdout_seen: Vec<u8>,
    stderr_seen: Vec<u8>,
}

fn spawn_shell_process_with_pipes(
    cmd: &mut tokio::process::Command,
) -> Result<ShellProcessParts, String> {
    cmd.stdout(std::process::Stdio::piped());
    cmd.stderr(std::process::Stdio::piped());
    let mut child = cmd
        .spawn()
        .map_err(|e| format!("Failed to execute command: {e}"))?;
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| "stdout pipe missing".to_string())?;
    let stderr = child
        .stderr
        .take()
        .ok_or_else(|| "stderr pipe missing".to_string())?;

    Ok(ShellProcessParts {
        child,
        stdout,
        stderr,
    })
}

fn spawn_shell_stream_reader<R>(
    mut reader: R,
    label: &'static str,
    tx: tokio::sync::mpsc::Sender<ShellStreamEvent>,
    event: fn(Vec<u8>) -> ShellStreamEvent,
    max_output_bytes: usize,
) -> tokio::task::JoinHandle<ShellStreamCapture>
where
    R: AsyncRead + Unpin + Send + 'static,
{
    tokio::spawn(async move {
        let mut capture = ShellStreamCapture::default();
        let mut buf = [0u8; 4096];
        loop {
            match reader.read(&mut buf).await {
                Ok(0) => break,
                Ok(n) => {
                    capture.total_bytes = capture.total_bytes.saturating_add(n);
                    let accepted = n.min(max_output_bytes.saturating_sub(capture.bytes.len()));
                    if accepted == 0 {
                        continue;
                    }
                    let chunk = buf[..accepted].to_vec();
                    emit_tool_chunk(label, &String::from_utf8_lossy(&chunk));
                    capture.bytes.extend_from_slice(&chunk);
                    let _ = tx.try_send(event(chunk));
                }
                Err(_) => break,
            }
        }
        capture
    })
}

fn schedule_shell_process_cleanup(child: &mut tokio::process::Child) -> bool {
    let Some(pid) = child.id() else {
        return false;
    };

    let _ = child.start_kill();
    tokio::spawn(async move {
        if let Err(error) = crate::subprocess_guard::kill_process_tree(
            pid,
            crate::subprocess_guard::DEFAULT_GRACE_MS,
        )
        .await
        {
            tracing::warn!(
                pid,
                %error,
                "Shell hard-cap cleanup failed after command returned to the agent"
            );
        }
    });
    true
}

async fn wait_for_shell_with_review_window(
    child: &mut tokio::process::Child,
    rx: &mut tokio::sync::mpsc::Receiver<ShellStreamEvent>,
    timeout_secs: u64,
    max_output_bytes: usize,
) -> Result<ShellWaitResult, String> {
    let review_interval = std::time::Duration::from_secs(timeout_secs.clamp(1, 30));
    let hard_cap = shell_review_hard_cap(timeout_secs);
    let mut review = Box::pin(tokio::time::sleep(review_interval));
    let mut poll = Box::pin(tokio::time::sleep(std::time::Duration::from_millis(250)));
    let mut deadline = Box::pin(tokio::time::sleep(hard_cap));
    let mut stdout_seen = Vec::new();
    let mut stderr_seen = Vec::new();
    let mut streams_open = true;

    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) => {}
            Err(error) => {
                schedule_shell_process_cleanup(child);
                return Err(format!("Failed to poll command: {error}"));
            }
        }
        tokio::select! {
            maybe_event = rx.recv(), if streams_open => {
                streams_open = handle_shell_stream_event(maybe_event, &mut stdout_seen, &mut stderr_seen);
            }
            _ = &mut review => {
                emit_shell_review_progress(timeout_secs, hard_cap.as_secs());
                review.as_mut().reset(tokio::time::Instant::now() + review_interval);
            }
            _ = &mut poll => {
                poll.as_mut().reset(tokio::time::Instant::now() + std::time::Duration::from_millis(250));
            }
            _ = &mut deadline => {
                let cleanup_scheduled = schedule_shell_process_cleanup(child);
                let cleanup_note = if cleanup_scheduled {
                    "Cleanup was scheduled asynchronously so the agent can inspect state and decide the next step"
                } else {
                    "No live process id remained to clean up"
                };
                return Err(format!(
                    "Command exceeded bounded review window after {}s (timeout_seconds={} is reviewed, not renewed indefinitely). {cleanup_note}. Partial output:\n{}",
                    hard_cap.as_secs(),
                    timeout_secs,
                    format_shell_output(-1, &stdout_seen, &stderr_seen, max_output_bytes)
                ));
            }
        }
    };

    Ok(ShellWaitResult {
        status,
        stdout_seen,
        stderr_seen,
    })
}

fn handle_shell_stream_event(
    event: Option<ShellStreamEvent>,
    stdout_seen: &mut Vec<u8>,
    stderr_seen: &mut Vec<u8>,
) -> bool {
    match event {
        Some(ShellStreamEvent::Stdout(chunk)) => {
            stdout_seen.extend_from_slice(&chunk);
            true
        }
        Some(ShellStreamEvent::Stderr(chunk)) => {
            stderr_seen.extend_from_slice(&chunk);
            true
        }
        None => false,
    }
}

fn shell_review_hard_cap(timeout_secs: u64) -> std::time::Duration {
    std::time::Duration::from_secs(timeout_secs.saturating_mul(3).max(timeout_secs + 2))
}

fn emit_shell_review_progress(timeout_secs: u64, hard_cap_secs: u64) {
    let msg = format!(
        "Command still running; process is alive. timeout_seconds={} is a bounded review window; hard cap={}s.\n",
        timeout_secs, hard_cap_secs,
    );
    emit_tool_chunk("progress", &msg);
}

fn complete_shell_stream(
    final_stream: ShellStreamCapture,
    seen_stream: Vec<u8>,
) -> ShellStreamCapture {
    if final_stream.bytes.len() >= seen_stream.len() {
        final_stream
    } else {
        ShellStreamCapture {
            total_bytes: final_stream.total_bytes.max(seen_stream.len()),
            bytes: seen_stream,
        }
    }
}

fn format_shell_output(exit_code: i32, stdout: &[u8], stderr: &[u8], max_output: usize) -> String {
    format_shell_output_with_totals(
        exit_code,
        stdout,
        stdout.len(),
        stderr,
        stderr.len(),
        max_output,
    )
}

fn format_shell_output_with_totals(
    exit_code: i32,
    stdout: &[u8],
    stdout_total_bytes: usize,
    stderr: &[u8],
    stderr_total_bytes: usize,
    max_output: usize,
) -> String {
    let stdout = String::from_utf8_lossy(stdout);
    let stderr = String::from_utf8_lossy(stderr);
    let stdout_str = if stdout_total_bytes > max_output {
        format!(
            "{}...\n[truncated, {} total bytes]",
            crate::str_utils::safe_truncate_str(&stdout, max_output),
            stdout_total_bytes
        )
    } else {
        stdout.to_string()
    };
    let stderr_str = if stderr_total_bytes > max_output {
        format!(
            "{}...\n[truncated, {} total bytes]",
            crate::str_utils::safe_truncate_str(&stderr, max_output),
            stderr_total_bytes
        )
    } else {
        stderr.to_string()
    };

    format!("Exit code: {exit_code}\n\nSTDOUT:\n{stdout_str}\nSTDERR:\n{stderr_str}")
}

pub(crate) fn command_sources_secrets_env(command: &str) -> bool {
    let lower = command.to_ascii_lowercase();
    if !lower.contains("secrets.env") {
        return false;
    }
    let normalized = format!(" {} ", lower.replace(['\n', '\r', '\t'], " "));
    normalized.contains(" source ")
        || normalized.contains(" . ~/.captain/secrets.env")
        || normalized.contains(" . $home/.captain/secrets.env")
        || normalized.contains(" . /root/.captain/secrets.env")
        || normalized.contains(" . /home/")
        || lower.contains("set -a")
        || lower.contains("set -o allexport")
}

pub(crate) fn unbounded_monitoring_command_reason(command: &str) -> Option<&'static str> {
    let lower = command.to_ascii_lowercase();
    if lower.contains("pmset -g thermlog") {
        return Some("`pmset -g thermlog` is a streaming thermal log and can wait forever");
    }
    if lower.contains("log stream") {
        return Some("`log stream` is an unbounded live log stream");
    }
    if lower.contains("tail -f") {
        return Some("`tail -f` is an unbounded file watcher");
    }
    if lower.contains("fs_usage") {
        return Some("`fs_usage` is an unbounded live system trace");
    }
    if lower.contains("tcpdump") {
        return Some("`tcpdump` is an unbounded packet capture unless carefully bounded");
    }
    if lower.split_whitespace().next() == Some("top")
        && !lower.contains("-l ")
        && !lower.contains("-l1")
    {
        return Some("`top` without a sample limit is an interactive monitor");
    }
    None
}

pub(crate) fn detached_process_command_reason(command: &str) -> Option<&'static str> {
    if contains_shell_command_word(command, "nohup") {
        return Some("`nohup` detaches process lifecycle from the tool result");
    }
    if contains_shell_command_word(command, "disown") {
        return Some("`disown` hides process lifecycle from Captain");
    }
    if contains_shell_background_operator(command) {
        return Some("background operator `&` can leave a hidden process after the tool returns");
    }
    if nested_shell_command_backgrounds(command) {
        return Some("nested shell background operator `&` can leave a hidden process after the tool returns");
    }
    None
}

fn contains_shell_command_word(command: &str, needle: &str) -> bool {
    let mut token = String::new();
    let mut single_quote = false;
    let mut double_quote = false;
    let mut escaped = false;
    let mut expecting_command = true;

    for ch in command.chars() {
        if escaped {
            token.push(ch);
            escaped = false;
            continue;
        }
        if ch == '\\' {
            escaped = true;
            continue;
        }
        if single_quote {
            if ch == '\'' {
                single_quote = false;
            } else {
                token.push(ch);
            }
            continue;
        }
        if double_quote {
            if ch == '"' {
                double_quote = false;
            } else {
                token.push(ch);
            }
            continue;
        }

        match ch {
            '\'' => single_quote = true,
            '"' => double_quote = true,
            ch if ch.is_whitespace() => {
                if shell_command_token_matches(&mut token, &mut expecting_command, needle) {
                    return true;
                }
            }
            ';' | '\n' | '|' | '&' | '(' => {
                if shell_command_token_matches(&mut token, &mut expecting_command, needle) {
                    return true;
                }
                expecting_command = true;
            }
            ')' => {
                if shell_command_token_matches(&mut token, &mut expecting_command, needle) {
                    return true;
                }
                expecting_command = false;
            }
            _ => token.push(ch),
        }
    }

    shell_command_token_matches(&mut token, &mut expecting_command, needle)
}

fn shell_command_token_matches(
    token: &mut String,
    expecting_command: &mut bool,
    needle: &str,
) -> bool {
    if token.is_empty() {
        return false;
    }
    let current = std::mem::take(token);
    if *expecting_command {
        if shell_word_basename_eq(&current, needle) {
            return true;
        }
        if !shell_prefix_keeps_command_expected(&current) {
            *expecting_command = false;
        }
    }
    false
}

fn shell_word_basename_eq(word: &str, needle: &str) -> bool {
    std::path::Path::new(word)
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or(word)
        .eq_ignore_ascii_case(needle)
}

fn shell_prefix_keeps_command_expected(word: &str) -> bool {
    word.contains('=')
        || shell_redirection_token(word)
        || matches!(word, "env" | "sudo" | "command" | "time")
}

fn shell_redirection_token(word: &str) -> bool {
    let without_fd = word.trim_start_matches(|ch: char| ch.is_ascii_digit());
    matches!(without_fd.chars().next(), Some('>' | '<'))
}

fn contains_shell_background_operator(command: &str) -> bool {
    let mut chars = command.char_indices().peekable();
    let mut single_quote = false;
    let mut double_quote = false;
    let mut escaped = false;

    while let Some((idx, ch)) = chars.next() {
        if escaped {
            escaped = false;
            continue;
        }
        if ch == '\\' {
            escaped = true;
            continue;
        }
        match ch {
            '\'' if !double_quote => {
                single_quote = !single_quote;
                continue;
            }
            '"' if !single_quote => {
                double_quote = !double_quote;
                continue;
            }
            '&' if !single_quote && !double_quote => {
                let previous = command[..idx].chars().next_back();
                let prev_is_amp = previous == Some('&');
                let prev_is_redirection = matches!(previous, Some('>' | '<'));
                let next_is_amp = chars.peek().map(|(_, next)| *next == '&').unwrap_or(false);
                if !prev_is_amp && !prev_is_redirection && !next_is_amp {
                    return true;
                }
            }
            _ => {}
        }
    }

    false
}

fn nested_shell_command_backgrounds(command: &str) -> bool {
    let Some(argv) = shlex::split(command) else {
        return false;
    };
    let Some(program) = argv.first().map(|value| {
        std::path::Path::new(value)
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or(value)
            .to_ascii_lowercase()
    }) else {
        return false;
    };
    if !matches!(program.as_str(), "bash" | "sh" | "zsh") {
        return false;
    }
    argv.windows(2).any(|pair| {
        let flag = pair[0].trim_start_matches('-');
        flag.contains('c') && contains_shell_background_operator(&pair[1])
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use captain_types::config::{ExecPolicy, ExecSecurityMode};
    use serde_json::json;

    #[test]
    fn parse_shell_exec_options_distinguishes_explicit_timeout_from_policy_timeout() {
        let policy = ExecPolicy {
            timeout_secs: 9,
            mode: ExecSecurityMode::Allowlist,
            ..ExecPolicy::default()
        };

        let implicit_input = json!({"command": "printf ok"});
        let implicit = parse_shell_exec_options(&implicit_input, Some(&policy))
            .expect("implicit timeout should parse");
        assert_eq!(implicit.timeout_secs, 9);
        assert!(!implicit.requested_timeout);
        assert!(implicit.use_direct_exec);

        let explicit_input = json!({"command": "printf ok", "timeout_seconds": 2});
        let explicit = parse_shell_exec_options(&explicit_input, Some(&policy))
            .expect("explicit timeout should parse");
        assert_eq!(explicit.timeout_secs, 2);
        assert!(explicit.requested_timeout);
    }

    #[test]
    fn shared_review_blocks_unbounded_monitoring_commands() {
        let policy = ExecPolicy {
            timeout_secs: 20,
            mode: ExecSecurityMode::Full,
            ..ExecPolicy::default()
        };

        for command in [
            "pmset -g thermlog | head -20",
            "log stream --style compact | head",
            "tail -f /tmp/captain.log",
            "fs_usage",
            "tcpdump -i en0",
            "top",
        ] {
            let err = crate::guarded_exec::review_shell(
                crate::guarded_exec::ExecSurface::ShellTool,
                command,
                Some(&policy),
                true,
            )
            .expect_err("unbounded monitoring command should be blocked");
            assert!(err.contains("unbounded command"), "{err}");
            assert!(err.contains("bounded snapshot"), "{err}");
        }

        crate::guarded_exec::review_shell(
            crate::guarded_exec::ExecSurface::ShellTool,
            "top -l 1 -n 0",
            Some(&policy),
            true,
        )
        .expect("bounded top snapshot should be allowed");
    }

    #[test]
    fn shared_review_blocks_detached_processes() {
        let policy = ExecPolicy {
            timeout_secs: 20,
            mode: ExecSecurityMode::Full,
            ..ExecPolicy::default()
        };

        for command in [
            "nohup python3 app.py >/tmp/app.log 2>&1",
            "2>/tmp/app.err nohup python3 app.py",
            "python3 app.py &",
            "bash -lc \"python3 app.py &\"",
            "sleep 1; disown",
        ] {
            let err = crate::guarded_exec::review_shell(
                crate::guarded_exec::ExecSurface::ShellTool,
                command,
                Some(&policy),
                true,
            )
            .expect_err("detached process command should be blocked");
            assert!(err.contains("detached command"), "{err}");
            assert!(err.contains("process_start"), "{err}");
        }

        crate::guarded_exec::review_shell(
            crate::guarded_exec::ExecSurface::ShellTool,
            "printf 'a & b' && printf done",
            Some(&policy),
            true,
        )
        .expect("quoted ampersand and && should remain valid");

        crate::guarded_exec::review_shell(
            crate::guarded_exec::ExecSurface::ShellTool,
            "printf 'nohup' && echo disown",
            Some(&policy),
            true,
        )
        .expect("textual nohup/disown mentions should remain valid");

        crate::guarded_exec::review_shell(
            crate::guarded_exec::ExecSurface::ShellTool,
            "printf ok 1>&2",
            Some(&policy),
            true,
        )
        .expect("shell redirections using >& must remain valid");
    }

    #[test]
    fn complete_shell_stream_keeps_longest_observed_buffer() {
        assert_eq!(
            complete_shell_stream(
                ShellStreamCapture {
                    bytes: b"final".to_vec(),
                    total_bytes: 5,
                },
                b"seen".to_vec(),
            )
            .bytes,
            b"final"
        );
        assert_eq!(
            complete_shell_stream(
                ShellStreamCapture {
                    bytes: b"fin".to_vec(),
                    total_bytes: 3,
                },
                b"seen-longer".to_vec(),
            )
            .bytes,
            b"seen-longer"
        );
    }

    #[test]
    fn shell_review_hard_cap_is_bounded_above_review_window() {
        assert_eq!(shell_review_hard_cap(1), std::time::Duration::from_secs(3));
        assert_eq!(
            shell_review_hard_cap(20),
            std::time::Duration::from_secs(60)
        );
        assert_eq!(
            shell_review_hard_cap(120),
            std::time::Duration::from_secs(360)
        );
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn explicit_timeout_is_review_window_for_healthy_shell_command() {
        let policy = ExecPolicy {
            mode: ExecSecurityMode::Full,
            ..ExecPolicy::default()
        };
        let output = tool_shell_exec(
            &json!({
                "command": "sleep 2; printf healthy",
                "timeout_seconds": 1
            }),
            &[],
            None,
            Some(&policy),
            shell_permit("sleep 2; printf healthy", &policy),
        )
        .await
        .expect("explicit timeout should not kill a healthy command");

        assert!(output.contains("Exit code: 0"));
        assert!(output.contains("healthy"));
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn explicit_timeout_review_window_has_hard_cap() {
        let policy = ExecPolicy {
            mode: ExecSecurityMode::Full,
            ..ExecPolicy::default()
        };
        let output = tool_shell_exec(
            &json!({
                "command": "sleep 5; printf late",
                "timeout_seconds": 1
            }),
            &[],
            None,
            Some(&policy),
            shell_permit("sleep 5; printf late", &policy),
        )
        .await
        .expect_err("bounded review window should kill a stuck command");

        assert!(output.contains("bounded review window"));
        assert!(output.contains("timeout_seconds=1"));
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn explicit_timeout_closed_streams_do_not_spin_forever() {
        let policy = ExecPolicy {
            mode: ExecSecurityMode::Full,
            ..ExecPolicy::default()
        };
        let completed = tokio::time::timeout(
            std::time::Duration::from_secs(4),
            tool_shell_exec(
                &json!({
                    "command": "exec 1>&- 2>&-; sleep 5",
                    "timeout_seconds": 1
                }),
                &[],
                None,
                Some(&policy),
                shell_permit("exec 1>&- 2>&-; sleep 5", &policy),
            ),
        )
        .await
        .expect("closed stdout/stderr must not spin past the hard cap");

        let output = completed.expect_err("closed stdout/stderr should hit the hard cap");
        assert!(output.contains("bounded review window"), "{output}");
        assert!(output.contains("timeout_seconds=1"), "{output}");
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn explicit_timeout_fast_output_is_bounded_and_cannot_deadlock() {
        let command = "printf 'x%.0s' {1..10000}; printf 'y%.0s' {1..10000} >&2";
        let policy = ExecPolicy {
            mode: ExecSecurityMode::Full,
            max_output_bytes: 128,
            ..ExecPolicy::default()
        };
        let output = tokio::time::timeout(
            std::time::Duration::from_secs(5),
            tool_shell_exec(
                &json!({"command": command, "timeout_seconds": 1}),
                &[],
                None,
                Some(&policy),
                shell_permit(command, &policy),
            ),
        )
        .await
        .expect("fast output must not deadlock")
        .expect("command succeeds");

        assert!(output.contains("Exit code: 0"), "{output}");
        assert!(
            output.len() < 1_000,
            "output was not bounded: {}",
            output.len()
        );
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn shell_exec_never_inherits_daemon_secrets() {
        let _guard = crate::guarded_exec::TEST_ASYNC_ENV_LOCK.lock().await;
        let key = "CAPTAIN_SHELL_INHERITED_SECRET";
        std::env::set_var(key, "must-not-leak");
        let command = "printf '%s' \"${CAPTAIN_SHELL_INHERITED_SECRET-unset}\"";
        let policy = ExecPolicy {
            mode: ExecSecurityMode::Full,
            ..ExecPolicy::default()
        };

        let output = tool_shell_exec(
            &json!({"command": command}),
            &[],
            None,
            Some(&policy),
            shell_permit(command, &policy),
        )
        .await
        .unwrap();
        std::env::remove_var(key);

        assert!(output.contains("STDOUT:\nunset"), "{output}");
        assert!(!output.contains("must-not-leak"), "{output}");
    }

    fn shell_permit(command: &str, policy: &ExecPolicy) -> crate::guarded_exec::ExecPermit {
        match crate::guarded_exec::review_shell(
            crate::guarded_exec::ExecSurface::ShellTool,
            command,
            Some(policy),
            true,
        )
        .expect("test command should pass shared review")
        {
            crate::guarded_exec::ReviewDecision::Proceed(permit) => permit,
            crate::guarded_exec::ReviewDecision::ApprovalRequired { pattern } => {
                panic!("unexpected approval for {pattern}")
            }
        }
    }
}
