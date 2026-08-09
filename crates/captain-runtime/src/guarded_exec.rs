//! Shared security boundary for agent-controlled subprocesses.
//!
//! Every execution surface that accepts model, skill, workflow, or remote
//! input must pass through this module before spawning a process. The
//! boundary reviews content and policy, scrubs the inherited environment,
//! applies an explicit workspace and timeout, and emits structured audit
//! events without logging command contents.

use std::io::Read;
use std::path::Path;
use std::process::Stdio;
use std::time::{Duration, Instant};

use captain_types::config::{CriticalMode, ExecPolicy, ExecSecurityMode};
use sha2::{Digest, Sha256};

const PROGRAM_AUTHORIZATION_DOMAIN: &[u8] = b"captain.exec-permit.program.v1\0";

#[cfg(test)]
pub(crate) static TEST_ASYNC_ENV_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());
#[cfg(test)]
pub(crate) static TEST_SYNC_ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// Agent-controlled execution surfaces covered by the shared boundary.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExecSurface {
    ShellTool,
    ProcessTool,
    GoalCheck,
    GoalRecovery,
    SkillCapability,
    CodeExecution,
    Workflow,
    SkillCheck,
    HandInstall,
    WasmHost,
}

impl ExecSurface {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::ShellTool => "shell_tool",
            Self::ProcessTool => "process_tool",
            Self::GoalCheck => "goal_check",
            Self::GoalRecovery => "goal_recovery",
            Self::SkillCapability => "skill_capability",
            Self::CodeExecution => "code_execution",
            Self::Workflow => "workflow",
            Self::SkillCheck => "skill_check",
            Self::HandInstall => "hand_install",
            Self::WasmHost => "wasm_host",
        }
    }
}

/// Result of policy review when an interactive approval rail is available.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReviewDecision {
    Proceed(ExecPermit),
    ApprovalRequired { pattern: &'static str },
}

/// Opaque proof that the shared boundary reviewed an execution request.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ExecPermit {
    surface: ExecSurface,
    authorization_sha256: [u8; 32],
}

impl ExecPermit {
    pub fn authorizes(self, surface: ExecSurface, content: &str) -> bool {
        self.surface == surface && self.authorization_sha256 == content_digest(content)
    }

    pub fn authorizes_program(
        self,
        surface: ExecSurface,
        executable: &str,
        args: &[String],
    ) -> bool {
        self.surface == surface
            && self.authorization_sha256 == program_authorization_digest(executable, args)
    }
}

#[derive(Debug, Clone, Copy)]
enum ContentKind<'a> {
    Shell,
    Script {
        interpreter: &'a str,
    },
    Program {
        executable: &'a str,
        args: &'a [String],
    },
}

/// Captured result from a guarded subprocess.
#[derive(Debug)]
pub struct ExecOutcome {
    pub exit_code: i32,
    pub stdout: String,
    pub stderr: String,
    pub stdout_total_bytes: usize,
    pub stderr_total_bytes: usize,
    pub elapsed_ms: u64,
}

impl ExecOutcome {
    pub fn success(&self) -> bool {
        self.exit_code == 0
    }
}

/// Request for an unattended shell or bash script.
pub struct ShellExecRequest<'a> {
    pub surface: ExecSurface,
    pub command: &'a str,
    pub policy: Option<&'a ExecPolicy>,
    pub workspace: Option<&'a Path>,
    /// Names that may be copied from the daemon environment after `env_clear`.
    pub allowed_env_vars: &'a [String],
    /// Values supplied explicitly by the caller after `env_clear`.
    pub explicit_env: &'a [(String, String)],
    pub timeout_secs: u64,
    pub no_output_timeout_secs: Option<u64>,
    pub bash_required: bool,
}

/// Request for a direct executable with no shell parsing.
pub struct ProgramExecRequest<'a> {
    pub surface: ExecSurface,
    pub executable: &'a str,
    pub args: &'a [String],
    pub policy: Option<&'a ExecPolicy>,
    pub workspace: Option<&'a Path>,
    pub allowed_env_vars: &'a [String],
    pub explicit_env: &'a [(String, String)],
    pub timeout_secs: u64,
}

/// Conservative policy used by unattended runtime surfaces that do not own
/// an agent manifest. It preserves Captain's Full execution compatibility
/// while blocking catastrophic patterns instead of requesting an approval
/// that no UI can answer.
pub fn unattended_policy(timeout_secs: u64) -> ExecPolicy {
    ExecPolicy {
        mode: ExecSecurityMode::Full,
        timeout_secs: timeout_secs.max(1),
        critical_mode: CriticalMode::Safe,
        ..ExecPolicy::default()
    }
}

pub fn review_shell(
    surface: ExecSurface,
    command: &str,
    policy: Option<&ExecPolicy>,
    allow_approval: bool,
) -> Result<ReviewDecision, String> {
    review_content(surface, command, ContentKind::Shell, policy, allow_approval)
}

pub fn review_script(
    surface: ExecSurface,
    interpreter: &str,
    script: &str,
    policy: Option<&ExecPolicy>,
) -> Result<(), String> {
    match review_content(
        surface,
        script,
        ContentKind::Script { interpreter },
        policy,
        false,
    )? {
        ReviewDecision::Proceed(_) => Ok(()),
        ReviewDecision::ApprovalRequired { .. } => {
            Err("guarded execution internal error: unattended review requested approval".into())
        }
    }
}

pub fn review_program(
    surface: ExecSurface,
    executable: &str,
    args: &[String],
    policy: Option<&ExecPolicy>,
) -> Result<ExecPermit, String> {
    let reviewed_content = program_review_content(executable, args);
    match review_content(
        surface,
        &reviewed_content,
        ContentKind::Program { executable, args },
        policy,
        false,
    )? {
        ReviewDecision::Proceed(permit) => Ok(permit),
        ReviewDecision::ApprovalRequired { .. } => {
            Err("guarded execution internal error: unattended review requested approval".into())
        }
    }
}

fn program_review_content(executable: &str, args: &[String]) -> String {
    let mut content = program_review_component(executable);
    for arg in args {
        content.push(' ');
        content.push_str(&program_review_component(arg));
    }
    content
}

fn program_review_component(value: &str) -> String {
    let is_plain = !value.is_empty()
        && value.bytes().all(|byte| {
            byte.is_ascii_alphanumeric()
                || matches!(
                    byte,
                    b'_' | b'-' | b'.' | b'/' | b':' | b'=' | b'+' | b'@' | b'%' | b','
                )
        });
    if is_plain {
        value.to_string()
    } else {
        format!("{value:?}")
    }
}

fn program_authorization_digest(executable: &str, args: &[String]) -> [u8; 32] {
    let mut hasher = Sha256::new();
    hasher.update(PROGRAM_AUTHORIZATION_DOMAIN);
    update_length_prefixed(&mut hasher, executable.as_bytes());
    hasher.update(
        u64::try_from(args.len())
            .expect("program argument count must fit in u64")
            .to_be_bytes(),
    );
    for arg in args {
        update_length_prefixed(&mut hasher, arg.as_bytes());
    }
    hasher.finalize().into()
}

fn update_length_prefixed(hasher: &mut Sha256, bytes: &[u8]) {
    hasher.update(
        u64::try_from(bytes.len())
            .expect("program component length must fit in u64")
            .to_be_bytes(),
    );
    hasher.update(bytes);
}

fn review_content(
    surface: ExecSurface,
    content: &str,
    kind: ContentKind<'_>,
    policy: Option<&ExecPolicy>,
    allow_approval: bool,
) -> Result<ReviewDecision, String> {
    let fallback = if allow_approval {
        ExecPolicy::default()
    } else {
        unattended_policy(30)
    };
    let policy = policy.unwrap_or(&fallback);
    let surface_name = surface.as_str();

    if let ContentKind::Program { executable, .. }
    | ContentKind::Script {
        interpreter: executable,
    } = kind
    {
        crate::subprocess_guard::validate_executable_path(executable).map_err(|reason| {
            audit_block(surface, "executable_path");
            format!("Guarded execution blocked executable path: {reason}")
        })?;
    }
    if let Err(error) =
        crate::tools::ensure_no_secret_literal(surface_name, "execution_input", content)
    {
        audit_block(surface, "literal_secret");
        return Err(error);
    }
    if crate::tools::shell_ops::command_sources_secrets_env(content) {
        audit_block(surface, "secrets_env_source");
        return Err(
            "Guarded execution blocked unsafe secrets.env sourcing. Use secret_read, a native integration, or a skill with explicit [requirements.env_inject]."
                .to_string(),
        );
    }
    if matches!(kind, ContentKind::Shell | ContentKind::Script { .. }) {
        if let Some(reason) = crate::tools::shell_ops::unbounded_monitoring_command_reason(content)
        {
            audit_block(surface, "unbounded_monitor");
            return Err(format!(
                "Guarded execution blocked an unbounded command: {reason}. Use a bounded snapshot or the managed process tools."
            ));
        }
        if let Some(reason) = crate::tools::shell_ops::detached_process_command_reason(content) {
            audit_block(surface, "detached_process");
            return Err(format!(
                "Guarded execution blocked a detached command: {reason}. Use process_start for managed long-running work."
            ));
        }
    }

    let critical_decision = match kind {
        ContentKind::Program { executable, args } => {
            crate::critical_patterns::decide_program(executable, args, policy.critical_mode)
        }
        ContentKind::Shell | ContentKind::Script { .. } => {
            crate::critical_patterns::decide(content, policy.critical_mode)
        }
    };
    match critical_decision {
        crate::critical_patterns::CriticalDecision::Proceed => {}
        crate::critical_patterns::CriticalDecision::Block(pattern) => {
            audit_block(surface, "critical_pattern");
            return Err(format!(
                "Guarded execution blocked critical pattern `{pattern}` under {:?} mode.",
                policy.critical_mode
            ));
        }
        crate::critical_patterns::CriticalDecision::AskUser(pattern) if allow_approval => {
            tracing::warn!(
                target: "captain::guarded_exec",
                surface = surface_name,
                reason = "approval_required",
                "Agent-controlled execution requires operator approval"
            );
            return Ok(ReviewDecision::ApprovalRequired { pattern });
        }
        crate::critical_patterns::CriticalDecision::AskUser(pattern) => {
            audit_block(surface, "approval_unavailable");
            return Err(format!(
                "Guarded execution blocked pattern `{pattern}` because this unattended surface cannot request approval."
            ));
        }
    }

    enforce_exec_policy(content, kind, policy).map_err(|reason| {
        audit_block(surface, "exec_policy");
        format!("Guarded execution blocked by exec policy: {reason}")
    })?;

    tracing::debug!(
        target: "captain::guarded_exec",
        surface = surface_name,
        "Agent-controlled execution passed shared review"
    );
    let authorization_sha256 = match kind {
        ContentKind::Program { executable, args } => program_authorization_digest(executable, args),
        ContentKind::Shell | ContentKind::Script { .. } => content_digest(content),
    };
    Ok(ReviewDecision::Proceed(ExecPermit {
        surface,
        authorization_sha256,
    }))
}

/// Convert a successful operator decision into the permit required by the
/// shell runner. The approval transport owns the user interaction; this
/// function records only the typed transition and never the command text.
pub fn permit_after_operator_approval(surface: ExecSurface, content: &str) -> ExecPermit {
    tracing::info!(
        target: "captain::guarded_exec",
        surface = surface.as_str(),
        "Operator approved guarded execution"
    );
    ExecPermit {
        surface,
        authorization_sha256: content_digest(content),
    }
}

fn content_digest(content: &str) -> [u8; 32] {
    Sha256::digest(content.as_bytes()).into()
}

fn enforce_exec_policy(
    content: &str,
    kind: ContentKind<'_>,
    policy: &ExecPolicy,
) -> Result<(), String> {
    let effective_mode = policy.effective_mode();
    match effective_mode {
        ExecSecurityMode::Deny => {
            return Err(format!(
                "host execution is disabled by execution profile `{}` and effective policy `{}`; use an explicitly configured docker_exec rail or a WASM agent when isolation is required",
                policy.profile.as_str(),
                effective_mode.as_str()
            ));
        }
        ExecSecurityMode::Full => {
            crate::subprocess_guard::validate_command_allowlist(content, policy)?;
        }
        ExecSecurityMode::Allowlist => {
            let executable = match kind {
                ContentKind::Shell => {
                    if let Some(reason) =
                        crate::subprocess_guard::contains_shell_metacharacters(content)
                    {
                        return Err(format!(
                            "shell input contains {reason}; metacharacters are forbidden in allowlist mode"
                        ));
                    }
                    crate::subprocess_guard::validate_command_allowlist(content, policy)?;
                    None
                }
                ContentKind::Script { interpreter } => Some(interpreter),
                ContentKind::Program { executable, .. } => Some(executable),
            };
            if let Some(executable) = executable {
                crate::subprocess_guard::validate_command_allowlist(executable, policy)?;
            }
        }
    }

    if effective_mode != ExecSecurityMode::Full && matches!(kind, ContentKind::Shell) {
        if let Some(reason) = crate::tools::check_shell_content_guard(content) {
            return Err(reason);
        }
    }
    Ok(())
}

fn audit_block(surface: ExecSurface, reason: &'static str) {
    tracing::warn!(
        target: "captain::guarded_exec",
        surface = surface.as_str(),
        reason,
        "Agent-controlled execution blocked by shared boundary"
    );
}

pub fn audit_execution_started(surface: ExecSurface, timeout_secs: u64) {
    tracing::info!(
        target: "captain::guarded_exec",
        surface = surface.as_str(),
        timeout_secs,
        "Starting guarded subprocess"
    );
}

pub fn audit_execution_finished(surface: ExecSurface, exit_code: i32, elapsed_ms: u64) {
    tracing::info!(
        target: "captain::guarded_exec",
        surface = surface.as_str(),
        exit_code,
        elapsed_ms,
        "Guarded subprocess finished"
    );
}

pub(crate) fn audit_execution_failed(
    surface: ExecSurface,
    reason: &'static str,
    elapsed: Duration,
) {
    tracing::warn!(
        target: "captain::guarded_exec",
        surface = surface.as_str(),
        reason,
        elapsed_ms = elapsed.as_millis() as u64,
        "Guarded subprocess failed"
    );
}

pub fn build_shell_command(
    command: &str,
    direct_exec: bool,
    bash_required: bool,
) -> Result<tokio::process::Command, String> {
    if direct_exec {
        let argv = shlex::split(command).ok_or_else(|| {
            "Command contains unmatched quotes or invalid shell syntax".to_string()
        })?;
        let Some(executable) = argv.first() else {
            return Err("Empty command after parsing".to_string());
        };
        let mut cmd = tokio::process::Command::new(executable);
        cmd.args(&argv[1..]);
        return Ok(cmd);
    }

    let (shell, flag) = shell_program(bash_required);
    let mut cmd = tokio::process::Command::new(shell);
    cmd.arg(flag).arg(command);
    Ok(cmd)
}

pub fn build_program_command(executable: &str, args: &[String]) -> tokio::process::Command {
    let mut cmd = tokio::process::Command::new(executable);
    cmd.args(args);
    cmd
}

pub fn build_std_program_command(executable: &str, args: &[String]) -> std::process::Command {
    let mut cmd = std::process::Command::new(executable);
    cmd.args(args);
    cmd
}

fn shell_program(bash_required: bool) -> (&'static str, &'static str) {
    if bash_required {
        return ("bash", "-c");
    }
    #[cfg(windows)]
    {
        ("cmd", "/C")
    }
    #[cfg(not(windows))]
    {
        ("sh", "-c")
    }
}

pub fn configure_tokio_command(
    cmd: &mut tokio::process::Command,
    workspace: Option<&Path>,
    allowed_env_vars: &[String],
    explicit_env: &[(String, String)],
) {
    apply_tokio_env(cmd, allowed_env_vars, explicit_env);
    if let Some(workspace) = workspace {
        cmd.current_dir(workspace);
    }
    #[cfg(unix)]
    cmd.process_group(0);
    #[cfg(windows)]
    cmd.env("PYTHONIOENCODING", "utf-8");
    cmd.stdin(Stdio::null());
    cmd.kill_on_drop(true);
}

pub fn configure_std_command(
    cmd: &mut std::process::Command,
    workspace: Option<&Path>,
    allowed_env_vars: &[String],
    explicit_env: &[(String, String)],
) {
    apply_std_env(cmd, allowed_env_vars, explicit_env);
    if let Some(workspace) = workspace {
        cmd.current_dir(workspace);
    }
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        cmd.process_group(0);
    }
    #[cfg(windows)]
    cmd.env("PYTHONIOENCODING", "utf-8");
    cmd.stdin(Stdio::null());
}

fn apply_tokio_env(
    cmd: &mut tokio::process::Command,
    allowed_env_vars: &[String],
    explicit_env: &[(String, String)],
) {
    cmd.env_clear();
    copy_safe_env(|key, value| {
        cmd.env(key, value);
    });
    copy_allowed_env(allowed_env_vars, |key, value| {
        cmd.env(key, value);
    });
    cmd.envs(explicit_env.iter().map(|(key, value)| (key, value)));
}

fn apply_std_env(
    cmd: &mut std::process::Command,
    allowed_env_vars: &[String],
    explicit_env: &[(String, String)],
) {
    cmd.env_clear();
    copy_safe_env(|key, value| {
        cmd.env(key, value);
    });
    copy_allowed_env(allowed_env_vars, |key, value| {
        cmd.env(key, value);
    });
    cmd.envs(explicit_env.iter().map(|(key, value)| (key, value)));
}

fn copy_safe_env(mut set: impl FnMut(&str, String)) {
    crate::subprocess_env_scrub::copy_safe_env(|key, value| set(key, value));
}

fn copy_allowed_env(allowed_env_vars: &[String], mut set: impl FnMut(&str, String)) {
    crate::subprocess_env_scrub::copy_allowed_env(allowed_env_vars, |key, value| set(key, value));
}

pub(crate) fn validate_environment_inputs(
    allowed_env_vars: &[String],
    explicit_env: &[(String, String)],
) -> Result<(), String> {
    for key in allowed_env_vars {
        if !valid_env_name(key) {
            return Err("Guarded execution rejected an invalid allowed environment name".into());
        }
    }
    for (key, value) in explicit_env {
        if !valid_env_name(key) {
            return Err("Guarded execution rejected an invalid explicit environment name".into());
        }
        if value.contains('\0') {
            return Err("Guarded execution rejected an environment value containing NUL".into());
        }
    }
    Ok(())
}

fn valid_env_name(name: &str) -> bool {
    let mut chars = name.chars();
    matches!(chars.next(), Some('_' | 'A'..='Z' | 'a'..='z'))
        && chars.all(|ch| matches!(ch, '_' | 'A'..='Z' | 'a'..='z' | '0'..='9'))
}

pub async fn run_unattended_shell(request: ShellExecRequest<'_>) -> Result<ExecOutcome, String> {
    let fallback = unattended_policy(request.timeout_secs);
    let policy = request.policy.unwrap_or(&fallback);
    if request.bash_required {
        review_script(request.surface, "bash", request.command, Some(policy))?;
    } else {
        match review_shell(request.surface, request.command, Some(policy), false)? {
            ReviewDecision::Proceed(_) => {}
            ReviewDecision::ApprovalRequired { .. } => {
                return Err(
                    "guarded execution internal error: unattended review requested approval".into(),
                );
            }
        }
    }

    validate_environment_inputs(request.allowed_env_vars, request.explicit_env)?;
    let direct_exec =
        policy.effective_mode() == ExecSecurityMode::Allowlist && !request.bash_required;
    let mut cmd = build_shell_command(request.command, direct_exec, request.bash_required)?;
    configure_tokio_command(
        &mut cmd,
        request.workspace,
        request.allowed_env_vars,
        request.explicit_env,
    );
    run_tokio_command(
        request.surface,
        cmd,
        request.timeout_secs.max(1),
        request
            .no_output_timeout_secs
            .unwrap_or(policy.no_output_timeout_secs),
        policy.max_output_bytes,
    )
    .await
}

pub async fn run_unattended_program(
    request: ProgramExecRequest<'_>,
) -> Result<ExecOutcome, String> {
    let fallback = unattended_policy(request.timeout_secs);
    let policy = request.policy.unwrap_or(&fallback);
    let permit = review_program(
        request.surface,
        request.executable,
        request.args,
        Some(policy),
    )?;
    if !permit.authorizes_program(request.surface, request.executable, request.args) {
        return Err("guarded execution permit/content mismatch".to_string());
    }
    validate_environment_inputs(request.allowed_env_vars, request.explicit_env)?;
    let mut cmd = build_program_command(request.executable, request.args);
    configure_tokio_command(
        &mut cmd,
        request.workspace,
        request.allowed_env_vars,
        request.explicit_env,
    );
    run_tokio_command(
        request.surface,
        cmd,
        request.timeout_secs.max(1),
        policy.no_output_timeout_secs,
        policy.max_output_bytes,
    )
    .await
}

pub(crate) async fn run_tokio_command(
    surface: ExecSurface,
    mut cmd: tokio::process::Command,
    timeout_secs: u64,
    no_output_timeout_secs: u64,
    max_output_bytes: usize,
) -> Result<ExecOutcome, String> {
    cmd.stdout(Stdio::piped()).stderr(Stdio::piped());
    let started = Instant::now();
    audit_execution_started(surface, timeout_secs);
    let mut child = cmd.spawn().map_err(|error| {
        audit_execution_failed(surface, "spawn_failed", started.elapsed());
        format!("Guarded execution failed to spawn: {error}")
    })?;
    let stdout = match child.stdout.take() {
        Some(stdout) => stdout,
        None => {
            terminate_timed_out_child(&mut child).await;
            audit_execution_failed(surface, "stdout_pipe_missing", started.elapsed());
            return Err("Guarded execution stdout pipe missing".to_string());
        }
    };
    let stderr = match child.stderr.take() {
        Some(stderr) => stderr,
        None => {
            terminate_timed_out_child(&mut child).await;
            audit_execution_failed(surface, "stderr_pipe_missing", started.elapsed());
            return Err("Guarded execution stderr pipe missing".to_string());
        }
    };
    let (activity_tx, mut activity_rx) = tokio::sync::mpsc::channel(32);
    let stdout_task = tokio::spawn(read_pipe(
        stdout,
        "stdout",
        max_output_bytes,
        activity_tx.clone(),
    ));
    let stderr_task = tokio::spawn(read_pipe(stderr, "stderr", max_output_bytes, activity_tx));

    let deadline = tokio::time::Instant::now() + Duration::from_secs(timeout_secs);
    let mut idle_deadline = (no_output_timeout_secs > 0)
        .then(|| tokio::time::Instant::now() + Duration::from_secs(no_output_timeout_secs));
    let mut activity_open = true;
    let exit_code = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status.code().unwrap_or(-1),
            Ok(None) => {}
            Err(error) => {
                terminate_timed_out_child(&mut child).await;
                stdout_task.abort();
                stderr_task.abort();
                audit_execution_failed(surface, "wait_failed", started.elapsed());
                return Err(format!("Guarded execution wait failed: {error}"));
            }
        }
        let now = tokio::time::Instant::now();
        if now >= deadline {
            terminate_timed_out_child(&mut child).await;
            stdout_task.abort();
            stderr_task.abort();
            audit_timeout(surface, "absolute_timeout", started.elapsed());
            return Err(format!("Guarded execution timed out after {timeout_secs}s"));
        }
        if idle_deadline.is_some_and(|idle| now >= idle) {
            terminate_timed_out_child(&mut child).await;
            stdout_task.abort();
            stderr_task.abort();
            audit_timeout(surface, "no_output_timeout", started.elapsed());
            return Err(format!(
                "Guarded execution produced no output for {no_output_timeout_secs}s"
            ));
        }

        tokio::select! {
            activity = activity_rx.recv(), if activity_open => {
                match activity {
                    Some(()) if no_output_timeout_secs > 0 => {
                        idle_deadline = Some(
                            tokio::time::Instant::now()
                                + Duration::from_secs(no_output_timeout_secs),
                        );
                    }
                    Some(()) => {}
                    None => activity_open = false,
                }
            }
            _ = tokio::time::sleep(Duration::from_millis(25)) => {}
        }
    };

    let stdout = stdout_task
        .await
        .map_err(|error| format!("Guarded execution stdout reader failed: {error}"))?;
    let stderr = stderr_task
        .await
        .map_err(|error| format!("Guarded execution stderr reader failed: {error}"))?;
    let elapsed_ms = started.elapsed().as_millis() as u64;
    audit_execution_finished(surface, exit_code, elapsed_ms);
    let stdout_total_bytes = stdout.total_bytes;
    let stderr_total_bytes = stderr.total_bytes;
    Ok(ExecOutcome {
        exit_code,
        stdout: stdout.render(),
        stderr: stderr.render(),
        stdout_total_bytes,
        stderr_total_bytes,
        elapsed_ms,
    })
}

struct CapturedPipe {
    bytes: Vec<u8>,
    total_bytes: usize,
}

impl CapturedPipe {
    fn render(self) -> String {
        let mut text = String::from_utf8_lossy(&self.bytes).to_string();
        if self.total_bytes > self.bytes.len() {
            text.push_str(&format!("\n[truncated, {} total bytes]", self.total_bytes));
        }
        text
    }
}

async fn read_pipe<R>(
    mut pipe: R,
    stream_name: &'static str,
    max_output_bytes: usize,
    activity: tokio::sync::mpsc::Sender<()>,
) -> CapturedPipe
where
    R: tokio::io::AsyncRead + Unpin,
{
    use tokio::io::AsyncReadExt;

    let mut bytes = Vec::new();
    let mut total_bytes = 0usize;
    let mut buffer = [0u8; 4096];
    loop {
        match pipe.read(&mut buffer).await {
            Ok(0) | Err(_) => break,
            Ok(read) => {
                total_bytes = total_bytes.saturating_add(read);
                crate::tools::emit_tool_chunk(
                    stream_name,
                    &String::from_utf8_lossy(&buffer[..read]),
                );
                let remaining = max_output_bytes.saturating_sub(bytes.len());
                bytes.extend_from_slice(&buffer[..read.min(remaining)]);
                let _ = activity.try_send(());
            }
        }
    }
    CapturedPipe { bytes, total_bytes }
}

pub(crate) async fn terminate_timed_out_child(child: &mut tokio::process::Child) {
    if let Some(pid) = child.id() {
        let _ = crate::subprocess_tree_kill::kill_process_tree(pid, 250).await;
    } else {
        let _ = child.start_kill();
    }
    let _ = child.wait().await;
}

fn audit_timeout(surface: ExecSurface, reason: &'static str, elapsed: Duration) {
    tracing::warn!(
        target: "captain::guarded_exec",
        surface = surface.as_str(),
        reason,
        elapsed_ms = elapsed.as_millis() as u64,
        "Guarded subprocess terminated"
    );
}

/// Run `bash -n` after the same content review and environment scrub used by
/// real skill execution. The script is supplied on stdin, never executed.
pub fn check_bash_syntax(
    surface: ExecSurface,
    script: &str,
    policy: Option<&ExecPolicy>,
) -> Result<(), String> {
    let fallback = unattended_policy(30);
    let policy = policy.unwrap_or(&fallback);
    review_script(surface, "bash", script, Some(policy))?;
    let args = vec!["-n".to_string()];
    let mut command = build_std_program_command("bash", &args);
    configure_std_command(&mut command, None, &[], &[]);
    command
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::piped());
    let started = Instant::now();
    let timeout_secs = policy.timeout_secs.max(1);
    audit_execution_started(surface, timeout_secs);
    let mut child = command.spawn().map_err(|error| {
        audit_execution_failed(surface, "spawn_failed", started.elapsed());
        format!("bash unavailable: {error}")
    })?;
    let stderr = match child.stderr.take() {
        Some(stderr) => stderr,
        None => {
            terminate_blocking_child(&mut child);
            audit_execution_failed(surface, "stderr_pipe_missing", started.elapsed());
            return Err("Guarded execution stderr pipe missing".to_string());
        }
    };
    let max_output = policy.max_output_bytes;
    let stderr_task = std::thread::spawn(move || read_pipe_blocking(stderr, max_output));
    let Some(mut stdin) = child.stdin.take() else {
        terminate_blocking_child(&mut child);
        let _ = stderr_task.join();
        audit_execution_failed(surface, "stdin_pipe_missing", started.elapsed());
        return Err("Guarded execution stdin pipe missing".to_string());
    };
    use std::io::Write;
    if let Err(error) = stdin.write_all(script.as_bytes()) {
        terminate_blocking_child(&mut child);
        let _ = stderr_task.join();
        audit_execution_failed(surface, "stdin_write_failed", started.elapsed());
        return Err(format!("write syntax input: {error}"));
    }
    drop(stdin);
    let deadline = Instant::now() + Duration::from_secs(timeout_secs);
    let exit_code = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status.code().unwrap_or(-1),
            Ok(None) if Instant::now() < deadline => {
                std::thread::sleep(Duration::from_millis(10));
            }
            Ok(None) => {
                terminate_blocking_child(&mut child);
                let _ = stderr_task.join();
                audit_timeout(surface, "absolute_timeout", started.elapsed());
                return Err(format!(
                    "Guarded syntax check timed out after {timeout_secs}s"
                ));
            }
            Err(error) => {
                terminate_blocking_child(&mut child);
                let _ = stderr_task.join();
                audit_execution_failed(surface, "wait_failed", started.elapsed());
                return Err(format!("wait for bash -n: {error}"));
            }
        }
    };
    let stderr = stderr_task
        .join()
        .map_err(|_| "Guarded execution stderr reader panicked".to_string())?
        .render();
    audit_execution_finished(surface, exit_code, started.elapsed().as_millis() as u64);
    if exit_code == 0 {
        Ok(())
    } else {
        let stderr = stderr.trim().to_string();
        Err(if stderr.is_empty() {
            "bash -n failed".to_string()
        } else {
            stderr
        })
    }
}

/// Blocking direct-program bridge for the synchronous WASM host API.
pub fn run_program_blocking(request: ProgramExecRequest<'_>) -> Result<ExecOutcome, String> {
    let fallback = unattended_policy(request.timeout_secs);
    let policy = request.policy.unwrap_or(&fallback);
    let permit = review_program(
        request.surface,
        request.executable,
        request.args,
        Some(policy),
    )?;
    if !permit.authorizes_program(request.surface, request.executable, request.args) {
        return Err("guarded execution permit/content mismatch".to_string());
    }
    validate_environment_inputs(request.allowed_env_vars, request.explicit_env)?;
    let mut command = build_std_program_command(request.executable, request.args);
    configure_std_command(
        &mut command,
        request.workspace,
        request.allowed_env_vars,
        request.explicit_env,
    );
    command.stdout(Stdio::piped()).stderr(Stdio::piped());

    let started = Instant::now();
    audit_execution_started(request.surface, request.timeout_secs.max(1));
    let mut child = command.spawn().map_err(|error| {
        audit_execution_failed(request.surface, "spawn_failed", started.elapsed());
        format!("Guarded execution failed to spawn: {error}")
    })?;
    let stdout = match child.stdout.take() {
        Some(stdout) => stdout,
        None => {
            terminate_blocking_child(&mut child);
            audit_execution_failed(request.surface, "stdout_pipe_missing", started.elapsed());
            return Err("Guarded execution stdout pipe missing".to_string());
        }
    };
    let stderr = match child.stderr.take() {
        Some(stderr) => stderr,
        None => {
            terminate_blocking_child(&mut child);
            audit_execution_failed(request.surface, "stderr_pipe_missing", started.elapsed());
            return Err("Guarded execution stderr pipe missing".to_string());
        }
    };
    let max_output = policy.max_output_bytes;
    let stdout_task = std::thread::spawn(move || read_pipe_blocking(stdout, max_output));
    let stderr_task = std::thread::spawn(move || read_pipe_blocking(stderr, max_output));
    let deadline = Instant::now() + Duration::from_secs(request.timeout_secs.max(1));

    let exit_code = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status.code().unwrap_or(-1),
            Ok(None) => {}
            Err(error) => {
                terminate_blocking_child(&mut child);
                let _ = stdout_task.join();
                let _ = stderr_task.join();
                audit_execution_failed(request.surface, "wait_failed", started.elapsed());
                return Err(format!("Guarded execution wait failed: {error}"));
            }
        }
        if Instant::now() >= deadline {
            terminate_blocking_child(&mut child);
            let _ = stdout_task.join();
            let _ = stderr_task.join();
            audit_timeout(request.surface, "absolute_timeout", started.elapsed());
            return Err(format!(
                "Guarded execution timed out after {}s",
                request.timeout_secs.max(1)
            ));
        }
        std::thread::sleep(Duration::from_millis(20));
    };

    let stdout = stdout_task
        .join()
        .map_err(|_| "Guarded execution stdout reader panicked".to_string())?;
    let stderr = stderr_task
        .join()
        .map_err(|_| "Guarded execution stderr reader panicked".to_string())?;
    let stdout_total_bytes = stdout.total_bytes;
    let stderr_total_bytes = stderr.total_bytes;
    let outcome = ExecOutcome {
        exit_code,
        stdout: stdout.render(),
        stderr: stderr.render(),
        stdout_total_bytes,
        stderr_total_bytes,
        elapsed_ms: started.elapsed().as_millis() as u64,
    };
    audit_execution_finished(request.surface, outcome.exit_code, outcome.elapsed_ms);
    Ok(outcome)
}

fn read_pipe_blocking(mut pipe: impl Read, max_output_bytes: usize) -> CapturedPipe {
    let mut bytes = Vec::new();
    let mut total_bytes = 0usize;
    let mut buffer = [0u8; 4096];
    loop {
        match pipe.read(&mut buffer) {
            Ok(0) | Err(_) => break,
            Ok(read) => {
                total_bytes = total_bytes.saturating_add(read);
                let remaining = max_output_bytes.saturating_sub(bytes.len());
                bytes.extend_from_slice(&buffer[..read.min(remaining)]);
            }
        }
    }
    CapturedPipe { bytes, total_bytes }
}

fn terminate_blocking_child(child: &mut std::process::Child) {
    #[cfg(unix)]
    {
        let group = format!("-{}", child.id());
        let args = ["-TERM", group.as_str()];
        let mut killer = std::process::Command::new("kill");
        killer
            .args(args)
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        configure_std_command(&mut killer, None, &[], &[]);
        let _ = killer.status();
    }
    #[cfg(windows)]
    {
        let pid = child.id().to_string();
        let args = ["/F", "/T", "/PID", pid.as_str()];
        let mut killer = std::process::Command::new("taskkill");
        killer
            .args(args)
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        configure_std_command(&mut killer, None, &[], &[]);
        let _ = killer.status();
    }
    let _ = child.kill();
    let _ = child.wait();
}

#[cfg(test)]
#[path = "guarded_exec_tests.rs"]
mod tests;
