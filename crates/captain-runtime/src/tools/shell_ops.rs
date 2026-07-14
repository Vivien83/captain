//! Shell execution handler.

use crate::tools::{emit_tool_chunk, ensure_no_secret_literal};
use std::path::Path;
use tokio::io::{AsyncRead, AsyncReadExt};

pub(crate) async fn tool_shell_exec(
    input: &serde_json::Value,
    allowed_env: &[String],
    workspace_root: Option<&Path>,
    exec_policy: Option<&captain_types::config::ExecPolicy>,
) -> Result<String, String> {
    let options = parse_shell_exec_options(input, exec_policy)?;
    let mut cmd = build_shell_command(options.command, options.use_direct_exec)?;
    configure_shell_command(&mut cmd, workspace_root, allowed_env);

    if options.requested_timeout {
        return run_shell_with_renewing_reviews(cmd, options.timeout_secs).await;
    }

    let result = tokio::time::timeout(
        std::time::Duration::from_secs(options.timeout_secs),
        cmd.output(),
    )
    .await;
    match result {
        Ok(Ok(output)) => Ok(format_shell_output(
            output.status.code().unwrap_or(-1),
            &output.stdout,
            &output.stderr,
        )),
        Ok(Err(e)) => Err(format!("Failed to execute command: {e}")),
        Err(_) => Err(format!("Command timed out after {}s", options.timeout_secs)),
    }
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
    ensure_no_secret_literal("shell_exec", "command", command)?;
    if command_sources_secrets_env(command) {
        return Err(
            "Blocked unsafe secrets.env sourcing. Do not run `source ~/.captain/secrets.env`, `. ~/.captain/secrets.env`, or `set -a` around Captain secrets: some keys are logical identifiers that are not valid shell variables. Use secret_read, a native integration, or a skill with env_inject instead."
                .to_string(),
        );
    }
    if let Some(reason) = unbounded_monitoring_command_reason(command) {
        return Err(format!(
            "shell_exec blocked: {reason}. Use a finite snapshot command instead, or process_start for an intentional watcher."
        ));
    }
    if let Some(reason) = detached_process_command_reason(command) {
        return Err(format!(
            "shell_exec blocked: {reason}. Use process_start with cwd/args for servers, watchers, REPLs, or any intentional background process; then inspect it with process_poll/process_list and stop it with process_kill."
        ));
    }

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

fn build_shell_command(
    command: &str,
    use_direct_exec: bool,
) -> Result<tokio::process::Command, String> {
    if use_direct_exec {
        direct_shell_command(command)
    } else {
        Ok(interpreted_shell_command(command))
    }
}

fn direct_shell_command(command: &str) -> Result<tokio::process::Command, String> {
    let argv = shlex::split(command)
        .ok_or_else(|| "Command contains unmatched quotes or invalid shell syntax".to_string())?;
    if argv.is_empty() {
        return Err("Empty command after parsing".to_string());
    }
    let mut cmd = tokio::process::Command::new(&argv[0]);
    if argv.len() > 1 {
        cmd.args(&argv[1..]);
    }
    Ok(cmd)
}

fn interpreted_shell_command(command: &str) -> tokio::process::Command {
    #[cfg(windows)]
    let git_sh: Option<&str> = {
        const SH_PATHS: &[&str] = &[
            "C:\\Program Files\\Git\\usr\\bin\\sh.exe",
            "C:\\Program Files (x86)\\Git\\usr\\bin\\sh.exe",
        ];
        SH_PATHS
            .iter()
            .copied()
            .find(|p| std::path::Path::new(p).exists())
    };
    let (shell, shell_arg) = if cfg!(windows) {
        #[cfg(windows)]
        {
            if let Some(sh) = git_sh {
                (sh, "-c")
            } else {
                ("cmd", "/C")
            }
        }
        #[cfg(not(windows))]
        {
            ("bash", "-c")
        }
    } else {
        ("bash", "-c")
    };
    let mut cmd = tokio::process::Command::new(shell);
    cmd.arg(shell_arg).arg(command);
    cmd
}

fn configure_shell_command(
    cmd: &mut tokio::process::Command,
    workspace_root: Option<&Path>,
    allowed_env: &[String],
) {
    if let Some(ws) = workspace_root {
        cmd.current_dir(ws);
    }
    crate::subprocess_sandbox::sandbox_command(cmd, allowed_env);
    #[cfg(unix)]
    {
        cmd.process_group(0);
    }

    #[cfg(windows)]
    cmd.env("PYTHONIOENCODING", "utf-8");
    cmd.stdin(std::process::Stdio::null());
}

enum ShellStreamEvent {
    Stdout(Vec<u8>),
    Stderr(Vec<u8>),
}

async fn run_shell_with_renewing_reviews(
    mut cmd: tokio::process::Command,
    timeout_secs: u64,
) -> Result<String, String> {
    let ShellProcessParts {
        mut child,
        stdout,
        stderr,
    } = spawn_shell_process_with_pipes(&mut cmd)?;
    let (tx, mut rx) = tokio::sync::mpsc::channel::<ShellStreamEvent>(32);

    let stdout_task =
        spawn_shell_stream_reader(stdout, "stdout", tx.clone(), ShellStreamEvent::Stdout);
    let stderr_task =
        spawn_shell_stream_reader(stderr, "stderr", tx.clone(), ShellStreamEvent::Stderr);
    drop(tx);

    let ShellWaitResult {
        status,
        stdout_seen,
        stderr_seen,
    } = wait_for_shell_with_review_window(&mut child, &mut rx, timeout_secs).await?;

    let stdout_final = stdout_task.await.unwrap_or_default();
    let stderr_final = stderr_task.await.unwrap_or_default();
    let stdout = complete_shell_stream(stdout_final, stdout_seen);
    let stderr = complete_shell_stream(stderr_final, stderr_seen);
    Ok(format_shell_output(
        status.code().unwrap_or(-1),
        &stdout,
        &stderr,
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
) -> tokio::task::JoinHandle<Vec<u8>>
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
                    emit_tool_chunk(label, &String::from_utf8_lossy(&chunk));
                    collected.extend_from_slice(&chunk);
                    let _ = tx.send(event(chunk)).await;
                }
                Err(_) => break,
            }
        }
        collected
    })
}

fn schedule_shell_process_cleanup(child: &mut tokio::process::Child) -> bool {
    let Some(pid) = child.id() else {
        return false;
    };

    let _ = child.start_kill();
    tokio::spawn(async move {
        if let Err(error) = crate::subprocess_sandbox::kill_process_tree(
            pid,
            crate::subprocess_sandbox::DEFAULT_GRACE_MS,
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
        if let Some(status) = child
            .try_wait()
            .map_err(|e| format!("Failed to poll command: {e}"))?
        {
            break status;
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
                    format_shell_output(-1, &stdout_seen, &stderr_seen)
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

fn complete_shell_stream(final_stream: Vec<u8>, seen_stream: Vec<u8>) -> Vec<u8> {
    if final_stream.len() >= seen_stream.len() {
        final_stream
    } else {
        seen_stream
    }
}

fn format_shell_output(exit_code: i32, stdout: &[u8], stderr: &[u8]) -> String {
    let stdout = String::from_utf8_lossy(stdout);
    let stderr = String::from_utf8_lossy(stderr);
    let max_output = 100_000;
    let stdout_str = if stdout.len() > max_output {
        format!(
            "{}...\n[truncated, {} total bytes]",
            crate::str_utils::safe_truncate_str(&stdout, max_output),
            stdout.len()
        )
    } else {
        stdout.to_string()
    };
    let stderr_str = if stderr.len() > max_output {
        format!(
            "{}...\n[truncated, {} total bytes]",
            crate::str_utils::safe_truncate_str(&stderr, max_output),
            stderr.len()
        )
    } else {
        stderr.to_string()
    };

    format!("Exit code: {exit_code}\n\nSTDOUT:\n{stdout_str}\nSTDERR:\n{stderr_str}")
}

fn command_sources_secrets_env(command: &str) -> bool {
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

fn unbounded_monitoring_command_reason(command: &str) -> Option<&'static str> {
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

fn detached_process_command_reason(command: &str) -> Option<&'static str> {
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
    fn parse_shell_exec_options_blocks_unbounded_monitoring_commands() {
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
            let err = parse_shell_exec_options(
                &json!({"command": command, "timeout_seconds": 20}),
                Some(&policy),
            )
            .expect_err("unbounded monitoring command should be blocked");
            assert!(err.contains("shell_exec blocked"), "{err}");
            assert!(err.contains("finite snapshot"), "{err}");
        }

        parse_shell_exec_options(
            &json!({"command": "top -l 1 -n 0", "timeout_seconds": 20}),
            Some(&policy),
        )
        .expect("bounded top snapshot should be allowed");
    }

    #[test]
    fn parse_shell_exec_options_blocks_detached_processes() {
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
            let err = parse_shell_exec_options(
                &json!({"command": command, "timeout_seconds": 20}),
                Some(&policy),
            )
            .expect_err("detached process command should be blocked");
            assert!(err.contains("shell_exec blocked"), "{err}");
            assert!(err.contains("process_start"), "{err}");
        }

        parse_shell_exec_options(
            &json!({"command": "printf 'a & b' && printf done", "timeout_seconds": 20}),
            Some(&policy),
        )
        .expect("quoted ampersand and && should remain valid");

        parse_shell_exec_options(
            &json!({"command": "printf 'nohup' && echo disown", "timeout_seconds": 20}),
            Some(&policy),
        )
        .expect("textual nohup/disown mentions should remain valid");

        parse_shell_exec_options(
            &json!({"command": "printf ok 1>&2", "timeout_seconds": 20}),
            Some(&policy),
        )
        .expect("shell redirections using >& must remain valid");
    }

    #[test]
    fn complete_shell_stream_keeps_longest_observed_buffer() {
        assert_eq!(
            complete_shell_stream(b"final".to_vec(), b"seen".to_vec()),
            b"final"
        );
        assert_eq!(
            complete_shell_stream(b"fin".to_vec(), b"seen-longer".to_vec()),
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
            ),
        )
        .await
        .expect("closed stdout/stderr must not spin past the hard cap");

        let output = completed.expect_err("closed stdout/stderr should hit the hard cap");
        assert!(output.contains("bounded review window"), "{output}");
        assert!(output.contains("timeout_seconds=1"), "{output}");
    }
}
