//! Cross-platform process tree termination helpers.

/// Default grace period before force-killing (milliseconds).
pub const DEFAULT_GRACE_MS: u64 = 3000;

/// Maximum grace period to prevent indefinite waits.
pub const MAX_GRACE_MS: u64 = 60_000;

type TerminationReason = captain_types::config::TerminationReason;

/// Kill a process and all its children (process tree kill).
///
/// 1. Send graceful termination signal (SIGTERM on Unix, taskkill on Windows)
/// 2. Wait `grace_ms` for the process to exit
/// 3. If still running, force kill (SIGKILL on Unix, taskkill /F on Windows)
///
/// Returns `Ok(true)` if the process was killed, `Ok(false)` if it was already
/// dead, or `Err` if the kill operation itself failed.
pub async fn kill_process_tree(pid: u32, grace_ms: u64) -> Result<bool, String> {
    let grace = grace_ms.min(MAX_GRACE_MS);

    #[cfg(unix)]
    {
        kill_tree_unix(pid, grace).await
    }

    #[cfg(windows)]
    {
        kill_tree_windows(pid, grace).await
    }
}

#[cfg(unix)]
async fn kill_tree_unix(pid: u32, grace_ms: u64) -> Result<bool, String> {
    use tokio::process::Command;

    let pid_i32 = pid as i32;

    let group_kill = Command::new("kill")
        .args(["-TERM", &format!("-{pid_i32}")])
        .output()
        .await;

    if group_kill.is_err() {
        let _ = Command::new("kill")
            .args(["-TERM", &pid.to_string()])
            .output()
            .await;
    }

    tokio::time::sleep(std::time::Duration::from_millis(grace_ms)).await;

    let check = Command::new("kill")
        .args(["-0", &pid.to_string()])
        .output()
        .await;

    match check {
        Ok(output) if output.status.success() => {
            tracing::warn!(
                pid,
                "Process still alive after grace period, sending SIGKILL"
            );

            let _ = Command::new("kill")
                .args(["-9", &format!("-{pid_i32}")])
                .output()
                .await;

            let _ = Command::new("kill")
                .args(["-9", &pid.to_string()])
                .output()
                .await;

            Ok(true)
        }
        _ => Ok(true),
    }
}

#[cfg(windows)]
async fn kill_tree_windows(pid: u32, grace_ms: u64) -> Result<bool, String> {
    use tokio::process::Command;

    let graceful = Command::new("taskkill")
        .args(["/T", "/PID", &pid.to_string()])
        .output()
        .await;

    match graceful {
        Ok(output) if output.status.success() => {
            return Ok(true);
        }
        _ => {}
    }

    tokio::time::sleep(std::time::Duration::from_millis(grace_ms)).await;

    let check = Command::new("tasklist")
        .args(["/FI", &format!("PID eq {pid}"), "/NH"])
        .output()
        .await;

    let still_alive = match &check {
        Ok(output) => {
            let stdout = String::from_utf8_lossy(&output.stdout);
            stdout.contains(&pid.to_string())
        }
        Err(_) => true,
    };

    if still_alive {
        tracing::warn!(pid, "Process still alive after grace period, force killing");
        let force = Command::new("taskkill")
            .args(["/F", "/T", "/PID", &pid.to_string()])
            .output()
            .await;

        match force {
            Ok(output) if output.status.success() => Ok(true),
            Ok(output) => {
                let stderr = String::from_utf8_lossy(&output.stderr);
                if stderr.contains("not found") || stderr.contains("no process") {
                    Ok(false)
                } else {
                    Err(format!("Force kill failed: {stderr}"))
                }
            }
            Err(e) => Err(format!("Failed to execute taskkill: {e}")),
        }
    } else {
        Ok(true)
    }
}

/// Kill a tokio child process with tree kill.
///
/// Extracts the PID from the `Child` handle and performs a tree kill.
/// This is the preferred way to clean up subprocesses spawned by Captain.
pub async fn kill_child_tree(
    child: &mut tokio::process::Child,
    grace_ms: u64,
) -> Result<bool, String> {
    match child.id() {
        Some(pid) => kill_process_tree(pid, grace_ms).await,
        None => Ok(false),
    }
}

/// Wait for a child process with timeout, then kill if necessary.
///
/// Returns the exit status if the process exits within the timeout,
/// or kills the process tree and returns an error.
pub async fn wait_or_kill(
    child: &mut tokio::process::Child,
    timeout: std::time::Duration,
    grace_ms: u64,
) -> Result<std::process::ExitStatus, String> {
    match tokio::time::timeout(timeout, child.wait()).await {
        Ok(Ok(status)) => Ok(status),
        Ok(Err(e)) => Err(format!("Wait error: {e}")),
        Err(_) => {
            tracing::warn!("Process timed out after {:?}, killing tree", timeout);
            kill_child_tree(child, grace_ms).await?;
            Err(format!("Process timed out after {:?}", timeout))
        }
    }
}

/// Wait for a child process with dual timeout: absolute + no-output idle.
///
/// - `absolute_timeout`: Maximum total execution time.
/// - `no_output_timeout`: Kill if no stdout/stderr output for this duration (0 = disabled).
/// - `grace_ms`: Grace period before force-killing.
///
/// Returns the termination reason and output collected.
pub async fn wait_or_kill_with_idle(
    child: &mut tokio::process::Child,
    absolute_timeout: std::time::Duration,
    no_output_timeout: std::time::Duration,
    grace_ms: u64,
) -> Result<(TerminationReason, String), String> {
    IdleWaitState::new(child, no_output_timeout)
        .run(child, absolute_timeout, grace_ms)
        .await
}

struct IdleWaitState {
    output: String,
    stdout: Option<tokio::process::ChildStdout>,
    stderr: Option<tokio::process::ChildStderr>,
    idle_enabled: bool,
    idle_deadline: Option<tokio::time::Instant>,
    no_output_timeout: std::time::Duration,
    stdout_buf: [u8; 4096],
    stderr_buf: [u8; 4096],
}

impl IdleWaitState {
    fn new(child: &mut tokio::process::Child, no_output_timeout: std::time::Duration) -> Self {
        let idle_enabled = !no_output_timeout.is_zero();
        Self {
            output: String::new(),
            stdout: child.stdout.take(),
            stderr: child.stderr.take(),
            idle_enabled,
            idle_deadline: idle_enabled.then(|| tokio::time::Instant::now() + no_output_timeout),
            no_output_timeout,
            stdout_buf: [0u8; 4096],
            stderr_buf: [0u8; 4096],
        }
    }

    async fn run(
        &mut self,
        child: &mut tokio::process::Child,
        absolute_timeout: std::time::Duration,
        grace_ms: u64,
    ) -> Result<(TerminationReason, String), String> {
        let deadline = tokio::time::Instant::now() + absolute_timeout;
        let poll_duration = std::time::Duration::from_millis(100);

        loop {
            if let Some(done) = self
                .elapsed_deadline_result(child, deadline, absolute_timeout, grace_ms)
                .await?
            {
                return Ok(done);
            }
            if let Some(done) = self
                .poll_once(child, deadline, absolute_timeout, grace_ms, poll_duration)
                .await?
            {
                return Ok(done);
            }
        }
    }

    async fn elapsed_deadline_result(
        &mut self,
        child: &mut tokio::process::Child,
        deadline: tokio::time::Instant,
        absolute_timeout: std::time::Duration,
        grace_ms: u64,
    ) -> Result<Option<(TerminationReason, String)>, String> {
        if tokio::time::Instant::now() >= deadline {
            tracing::warn!("Process hit absolute timeout after {:?}", absolute_timeout);
            return self
                .kill_with_reason(child, grace_ms, TerminationReason::AbsoluteTimeout)
                .await;
        }
        if let Some(idle_dl) = self.idle_deadline {
            if tokio::time::Instant::now() >= idle_dl {
                tracing::warn!(
                    "Process produced no output for {:?}, killing",
                    self.no_output_timeout
                );
                return self
                    .kill_with_reason(child, grace_ms, TerminationReason::NoOutputTimeout)
                    .await;
            }
        }
        Ok(None)
    }

    async fn poll_once(
        &mut self,
        child: &mut tokio::process::Child,
        deadline: tokio::time::Instant,
        absolute_timeout: std::time::Duration,
        grace_ms: u64,
        poll_duration: std::time::Duration,
    ) -> Result<Option<(TerminationReason, String)>, String> {
        use tokio::io::AsyncReadExt;

        tokio::select! {
            _ = tokio::time::sleep_until(deadline) => {
                tracing::warn!("Process hit absolute timeout after {:?}", absolute_timeout);
                self.kill_with_reason(child, grace_ms, TerminationReason::AbsoluteTimeout).await
            }
            _ = async {
                if let Some(idle_dl) = self.idle_deadline {
                    tokio::time::sleep_until(idle_dl).await;
                }
            }, if self.idle_deadline.is_some() => {
                tracing::warn!(
                    "Process produced no output for {:?}, killing",
                    self.no_output_timeout
                );
                self.kill_with_reason(child, grace_ms, TerminationReason::NoOutputTimeout).await
            }
            result = async {
                if let Some(ref mut out) = self.stdout {
                    out.read(&mut self.stdout_buf).await
                } else {
                    tokio::time::sleep(poll_duration).await;
                    Ok(0)
                }
            } => self.handle_stdout_read(child, deadline, grace_ms, result).await,
            result = async {
                if let Some(ref mut err) = self.stderr {
                    err.read(&mut self.stderr_buf).await
                } else {
                    tokio::time::sleep(poll_duration).await;
                    Ok(0)
                }
            } => {
                self.handle_stderr_read(result);
                Ok(None)
            }
            result = child.wait() => self.handle_child_wait(result).await,
        }
    }

    async fn handle_stdout_read(
        &mut self,
        child: &mut tokio::process::Child,
        deadline: tokio::time::Instant,
        grace_ms: u64,
        result: std::io::Result<usize>,
    ) -> Result<Option<(TerminationReason, String)>, String> {
        match result {
            Ok(0) => {
                self.stdout = None;
                if self.stderr.is_none() {
                    let output = self.take_output();
                    return wait_for_exit_before_deadline(child, deadline, grace_ms, output)
                        .await
                        .map(Some);
                }
            }
            Ok(n) => {
                self.output
                    .push_str(&String::from_utf8_lossy(&self.stdout_buf[..n]));
                self.reset_idle_deadline();
            }
            Err(e) => {
                tracing::debug!("Stdout read error: {e}");
                self.stdout = None;
            }
        }
        Ok(None)
    }

    fn handle_stderr_read(&mut self, result: std::io::Result<usize>) {
        match result {
            Ok(0) => self.stderr = None,
            Ok(n) => {
                self.output
                    .push_str(&String::from_utf8_lossy(&self.stderr_buf[..n]));
                self.reset_idle_deadline();
            }
            Err(e) => {
                tracing::debug!("Stderr read error: {e}");
                self.stderr = None;
            }
        }
    }

    async fn handle_child_wait(
        &mut self,
        result: std::io::Result<std::process::ExitStatus>,
    ) -> Result<Option<(TerminationReason, String)>, String> {
        match result {
            Ok(status) => {
                self.drain_remaining_output().await;
                Ok(Some((
                    TerminationReason::Exited(status.code().unwrap_or(-1)),
                    self.take_output(),
                )))
            }
            Err(e) => Err(format!("Wait error: {e}")),
        }
    }

    async fn drain_remaining_output(&mut self) {
        use tokio::io::AsyncReadExt;

        if let Some(mut stdout) = self.stdout.take() {
            let mut text = String::new();
            if stdout.read_to_string(&mut text).await.is_ok() {
                self.output.push_str(&text);
            }
        }
        if let Some(mut stderr) = self.stderr.take() {
            let mut text = String::new();
            if stderr.read_to_string(&mut text).await.is_ok() {
                self.output.push_str(&text);
            }
        }
    }

    async fn kill_with_reason(
        &mut self,
        child: &mut tokio::process::Child,
        grace_ms: u64,
        reason: TerminationReason,
    ) -> Result<Option<(TerminationReason, String)>, String> {
        kill_child_tree(child, grace_ms).await?;
        Ok(Some((reason, self.take_output())))
    }

    fn reset_idle_deadline(&mut self) {
        if self.idle_enabled {
            self.idle_deadline = Some(tokio::time::Instant::now() + self.no_output_timeout);
        }
    }

    fn take_output(&mut self) -> String {
        std::mem::take(&mut self.output)
    }
}

async fn wait_for_exit_before_deadline(
    child: &mut tokio::process::Child,
    deadline: tokio::time::Instant,
    grace_ms: u64,
    output: String,
) -> Result<(TerminationReason, String), String> {
    match tokio::time::timeout(
        deadline.saturating_duration_since(tokio::time::Instant::now()),
        child.wait(),
    )
    .await
    {
        Ok(Ok(status)) => Ok((
            TerminationReason::Exited(status.code().unwrap_or(-1)),
            output,
        )),
        Ok(Err(e)) => Err(format!("Wait error: {e}")),
        Err(_) => {
            kill_child_tree(child, grace_ms).await?;
            Ok((TerminationReason::AbsoluteTimeout, output))
        }
    }
}
