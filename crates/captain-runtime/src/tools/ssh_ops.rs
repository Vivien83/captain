//! SSH execution and SFTP handlers.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

const MAX_SSH_REVIEW_TIMEOUT_SECS: u64 = 7_200;

struct VaultBridge<'a>(&'a mut captain_extensions::vault::CredentialVault);

impl crate::ssh_vault::SshSecretStore for VaultBridge<'_> {
    fn get(&self, key: &str) -> Option<zeroize::Zeroizing<String>> {
        self.0.get(key)
    }

    fn set(&mut self, key: String, value: zeroize::Zeroizing<String>) -> Result<(), String> {
        self.0.set(key, value).map_err(|e| e.to_string())
    }

    fn remove(&mut self, key: &str) -> Result<bool, String> {
        self.0.remove(key).map_err(|e| e.to_string())
    }

    fn list_keys(&self) -> Vec<String> {
        self.0.list_keys().into_iter().map(String::from).collect()
    }
}

fn captain_home_for_runtime() -> PathBuf {
    if let Ok(p) = std::env::var("CAPTAIN_HOME") {
        return PathBuf::from(p);
    }
    dirs::home_dir()
        .map(|h| h.join(".captain"))
        .unwrap_or_else(|| PathBuf::from(".captain"))
}

pub(crate) async fn tool_ssh_health_check(
    input: &serde_json::Value,
    exec_policy: Option<&captain_types::config::ExecPolicy>,
) -> Result<String, String> {
    let key_name = input["key_name"]
        .as_str()
        .ok_or("Missing 'key_name' parameter")?;
    let service = input["service"]
        .as_str()
        .map(str::trim)
        .filter(|s| !s.is_empty());
    if let Some(service) = service {
        if !service
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '.' | '@' | ':'))
        {
            return Err(
                "Invalid service name: only letters, digits, _, -, ., @ and : are allowed"
                    .to_string(),
            );
        }
    }

    let include_docker = input["include_docker"].as_bool().unwrap_or(true);
    let include_ports = input["include_ports"].as_bool().unwrap_or(true);
    let include_logs = input["include_logs"].as_bool().unwrap_or(true);
    let log_lines = input["log_lines"].as_u64().unwrap_or(80).clamp(10, 200);
    let timeout_secs = input["timeout_secs"].as_u64().unwrap_or(60).clamp(10, 180);

    let mut command = String::from(
        "set -o pipefail\n\
printf '=== host ===\\n'; hostname; date; uptime\n\
printf '\\n=== disk ===\\n'; df -h / /var /home 2>/dev/null || df -h\n\
printf '\\n=== memory ===\\n'; free -h 2>/dev/null || awk '/MemTotal|MemAvailable|SwapTotal|SwapFree/ {print}' /proc/meminfo\n\
printf '\\n=== load/cpu ===\\n'; nproc 2>/dev/null || true; top -bn1 | head -n 5 2>/dev/null || true\n\
printf '\\n=== failed services ===\\n'; systemctl --failed --no-pager 2>/dev/null || true\n",
    );
    if let Some(service) = service {
        command.push_str(&format!(
            "printf '\\n=== service: {service} ===\\n'; systemctl status {service} --no-pager -l 2>/dev/null || true\n"
        ));
        if include_logs {
            command.push_str(&format!(
                "printf '\\n=== service logs: {service} ===\\n'; journalctl -u {service} -n {log_lines} --no-pager 2>/dev/null || true\n"
            ));
        }
    }
    if include_docker {
        command.push_str("printf '\\n=== docker ===\\n'; docker ps --format 'table {{.Names}}\\t{{.Status}}\\t{{.Ports}}' 2>/dev/null || true\n");
    }
    if include_ports {
        command.push_str("printf '\\n=== listening ports ===\\n'; ss -tulpn 2>/dev/null | head -n 40 || netstat -tulpn 2>/dev/null | head -n 40 || true\n");
    }
    if include_logs && service.is_none() {
        command.push_str("printf '\\n=== recent critical logs ===\\n'; journalctl -p 0..3 -n 50 --no-pager 2>/dev/null || true\n");
    }

    run_ssh_exec(key_name, &command, timeout_secs, false, exec_policy).await
}

pub(crate) async fn tool_ssh_exec(
    input: &serde_json::Value,
    exec_policy: Option<&captain_types::config::ExecPolicy>,
) -> Result<String, String> {
    let key_name = input["key_name"]
        .as_str()
        .ok_or("Missing 'key_name' parameter")?;
    let command = input["command"]
        .as_str()
        .ok_or("Missing 'command' parameter")?;
    let requested_timeout = input["timeout_secs"].as_u64().filter(|secs| *secs > 0);
    let timeout_secs = ssh_exec_timeout_secs(requested_timeout);

    run_ssh_exec(
        key_name,
        command,
        timeout_secs,
        requested_timeout.is_some(),
        exec_policy,
    )
    .await
}

fn ssh_exec_timeout_secs(requested_timeout: Option<u64>) -> u64 {
    requested_timeout
        .unwrap_or(60)
        .clamp(1, MAX_SSH_REVIEW_TIMEOUT_SECS)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SshTimeoutMode {
    HardTimeout,
    ReviewWindow,
}

impl SshTimeoutMode {
    fn from_review_window(review_window: bool) -> Self {
        if review_window {
            Self::ReviewWindow
        } else {
            Self::HardTimeout
        }
    }

    fn label(self) -> &'static str {
        match self {
            Self::HardTimeout => "hard_timeout",
            Self::ReviewWindow => "review_window",
        }
    }
}

async fn run_ssh_exec(
    key_name: &str,
    command: &str,
    timeout_secs: u64,
    review_window: bool,
    exec_policy: Option<&captain_types::config::ExecPolicy>,
) -> Result<String, String> {
    assert_ssh_command_allowed(command, exec_policy)?;
    let (key, home, resolved_key_name) = open_vault_and_load_key(key_name)?;
    let alias_label = ssh_alias_label(key_name, &resolved_key_name);
    let timeout_mode = SshTimeoutMode::from_review_window(review_window);
    let started = Instant::now();
    let result = execute_ssh_command(&key, command, timeout_secs, timeout_mode).await;
    let elapsed = started.elapsed();

    audit_ssh_exec(&home, &resolved_key_name, command, elapsed, result.is_ok());

    let out = result?;
    Ok(render_ssh_exec_output(
        &out,
        &key,
        elapsed,
        &alias_label,
        timeout_mode,
    ))
}

fn assert_ssh_command_allowed(
    command: &str,
    exec_policy: Option<&captain_types::config::ExecPolicy>,
) -> Result<(), String> {
    if let Some(reason) = unbounded_remote_command_reason(command) {
        return Err(format!(
            "ssh_exec blocked: {reason}. Use a finite snapshot command instead, or process_start/process supervision for an intentional watcher."
        ));
    }

    let critical_mode = exec_policy.map(|p| p.critical_mode).unwrap_or_default();
    match crate::critical_patterns::decide(command, critical_mode) {
        crate::critical_patterns::CriticalDecision::Proceed => Ok(()),
        crate::critical_patterns::CriticalDecision::Block(pat) => Err(format!(
            "ssh_exec blocked: hyper-critical pattern `{pat}` detected in remote command. \
                 Current security.critical_mode = '{critical_mode:?}'. Refused before sending."
        )),
        crate::critical_patterns::CriticalDecision::AskUser(pat) => Err(format!(
            "ssh_exec blocked: hyper-critical pattern `{pat}` requires interactive \
                 approval, which is not yet available for the SSH path. \
                 Run the command manually via `ssh` or split it into safer steps."
        )),
    }
}

fn unbounded_remote_command_reason(command: &str) -> Option<&'static str> {
    let lower = command.to_ascii_lowercase();
    let normalized = format!(" {} ", lower.replace(['\n', '\r', '\t'], " "));

    if normalized.contains(" journalctl ")
        && (normalized.contains(" -f ") || normalized.contains(" --follow "))
    {
        return Some("`journalctl -f`/`--follow` is an unbounded remote log stream");
    }
    if normalized.contains(" docker logs ")
        && (normalized.contains(" -f ") || normalized.contains(" --follow "))
    {
        return Some("`docker logs -f`/`--follow` is an unbounded remote log stream");
    }
    if normalized.contains(" kubectl logs ")
        && (normalized.contains(" -f ") || normalized.contains(" --follow "))
    {
        return Some("`kubectl logs -f`/`--follow` is an unbounded remote log stream");
    }
    if normalized.contains(" tail -f ") || normalized.contains(" tail --follow ") {
        return Some("`tail -f`/`--follow` is an unbounded remote file watcher");
    }
    if normalized.contains(" docker events ") {
        return Some("`docker events` is an unbounded remote event stream");
    }
    if normalized.contains(" docker stats ") && !normalized.contains(" --no-stream ") {
        return Some("`docker stats` without `--no-stream` is an unbounded remote monitor");
    }
    if normalized.contains(" pm2 logs ") {
        return Some("`pm2 logs` follows process logs until interrupted");
    }
    if normalized.contains(" watch ") {
        return Some("`watch` is an unbounded remote command repeater");
    }
    if normalized.split_whitespace().next() == Some("top")
        && !normalized.contains(" -b")
        && !normalized.contains(" -n")
        && !normalized.contains(" -l")
    {
        return Some("`top` without batch/sample limits is an interactive monitor");
    }
    if normalized.contains(" tcpdump ") && !normalized.contains(" -c ") {
        return Some("`tcpdump` without packet count is an unbounded remote capture");
    }
    None
}

async fn execute_ssh_command(
    key: &crate::ssh_vault::SshKey,
    command: &str,
    timeout_secs: u64,
    timeout_mode: SshTimeoutMode,
) -> Result<crate::ssh_exec::SshExecOutput, String> {
    let timeout = Duration::from_secs(timeout_secs);
    match timeout_mode {
        SshTimeoutMode::HardTimeout => crate::ssh_exec::ssh_exec(key, command, timeout).await,
        SshTimeoutMode::ReviewWindow => {
            crate::ssh_exec::ssh_exec_with_review_window(key, command, timeout).await
        }
    }
}

fn audit_ssh_exec(
    home: &Path,
    resolved_key_name: &str,
    command: &str,
    elapsed: Duration,
    success: bool,
) {
    let detail = format!(
        "{} ({}ms)",
        captain_types::truncate_str(command, 200),
        elapsed.as_millis()
    );
    crate::ssh_vault::audit_log(
        &home.join("audit"),
        "exec",
        resolved_key_name,
        &detail,
        success,
    );
}

fn ssh_alias_label(requested: &str, resolved: &str) -> String {
    if requested == resolved {
        resolved.to_string()
    } else {
        format!("{requested} -> {resolved}")
    }
}

fn render_ssh_exec_output(
    out: &crate::ssh_exec::SshExecOutput,
    key: &crate::ssh_vault::SshKey,
    elapsed: Duration,
    alias_label: &str,
    timeout_mode: SshTimeoutMode,
) -> String {
    let exit_label = match out.exit_code {
        Some(0) => "exit 0".to_string(),
        Some(c) => format!("exit {c} (non-zero)"),
        None => "exit ?".to_string(),
    };
    let mut payload = format!(
        "[{} on {}@{}:{}, {}ms, {}, timeout_mode={}]\n",
        exit_label,
        key.user,
        key.host,
        key.port,
        elapsed.as_millis(),
        alias_label,
        timeout_mode.label()
    );
    if !out.stdout.is_empty() {
        payload.push_str("--- stdout ---\n");
        payload.push_str(&out.stdout);
        if !out.stdout.ends_with('\n') {
            payload.push('\n');
        }
    }
    if !out.stderr.is_empty() {
        payload.push_str("--- stderr ---\n");
        payload.push_str(&out.stderr);
        if !out.stderr.ends_with('\n') {
            payload.push('\n');
        }
    }
    if out.stdout.is_empty() && out.stderr.is_empty() {
        payload.push_str("(no output)\n");
    }
    payload
}

fn open_vault_and_load_key(
    key_name: &str,
) -> Result<(crate::ssh_vault::SshKey, PathBuf, String), String> {
    let home = captain_home_for_runtime();
    let vault_path = home.join("vault.enc");
    let mut vault = captain_extensions::vault::CredentialVault::new(vault_path);
    if !vault.exists() {
        return Err("Vault not initialized. Run: captain vault init".into());
    }
    vault
        .unlock()
        .map_err(|e| format!("Could not unlock vault: {e}"))?;
    let resolved = {
        let store = VaultBridge(&mut vault);
        crate::ssh_vault::resolve_ssh_key(&store, key_name)?
    };
    Ok((resolved.key, home, resolved.resolved))
}

pub(crate) async fn tool_ssh_upload(input: &serde_json::Value) -> Result<String, String> {
    let key_name = input["key_name"]
        .as_str()
        .ok_or("Missing 'key_name' parameter")?;
    let local_path = input["local_path"]
        .as_str()
        .ok_or("Missing 'local_path' parameter")?;
    let remote_path = input["remote_path"]
        .as_str()
        .ok_or("Missing 'remote_path' parameter")?;
    let timeout_secs = input["timeout_secs"].as_u64().unwrap_or(120);

    let (key, home, resolved_key_name) = open_vault_and_load_key(key_name)?;
    let alias_label = if key_name == resolved_key_name.as_str() {
        resolved_key_name.clone()
    } else {
        format!("{key_name} -> {resolved_key_name}")
    };
    let started = std::time::Instant::now();
    let result = crate::ssh_sftp::ssh_upload(
        &key,
        Path::new(local_path),
        remote_path,
        std::time::Duration::from_secs(timeout_secs),
    )
    .await;
    let elapsed = started.elapsed();

    let detail = format!(
        "{} -> {} ({}ms)",
        local_path,
        remote_path,
        elapsed.as_millis()
    );
    crate::ssh_vault::audit_log(
        &home.join("audit"),
        "upload",
        &resolved_key_name,
        &detail,
        result.is_ok(),
    );

    let bytes = result?;
    Ok(format!(
        "Uploaded {} bytes from '{local_path}' to {}@{}:{remote_path} ({}ms, key={alias_label})",
        bytes,
        key.user,
        key.host,
        elapsed.as_millis()
    ))
}

pub(crate) async fn tool_ssh_download(input: &serde_json::Value) -> Result<String, String> {
    let key_name = input["key_name"]
        .as_str()
        .ok_or("Missing 'key_name' parameter")?;
    let remote_path = input["remote_path"]
        .as_str()
        .ok_or("Missing 'remote_path' parameter")?;
    let local_path = input["local_path"]
        .as_str()
        .ok_or("Missing 'local_path' parameter")?;
    let timeout_secs = input["timeout_secs"].as_u64().unwrap_or(120);

    let (key, home, resolved_key_name) = open_vault_and_load_key(key_name)?;
    let alias_label = if key_name == resolved_key_name.as_str() {
        resolved_key_name.clone()
    } else {
        format!("{key_name} -> {resolved_key_name}")
    };
    let started = std::time::Instant::now();
    let result = crate::ssh_sftp::ssh_download(
        &key,
        remote_path,
        Path::new(local_path),
        std::time::Duration::from_secs(timeout_secs),
    )
    .await;
    let elapsed = started.elapsed();

    let detail = format!(
        "{} -> {} ({}ms)",
        remote_path,
        local_path,
        elapsed.as_millis()
    );
    crate::ssh_vault::audit_log(
        &home.join("audit"),
        "download",
        &resolved_key_name,
        &detail,
        result.is_ok(),
    );

    let bytes = result?;
    Ok(format!(
        "Downloaded {} bytes from {}@{}:{remote_path} to '{local_path}' ({}ms, key={alias_label})",
        bytes,
        key.user,
        key.host,
        elapsed.as_millis()
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use zeroize::Zeroizing;

    fn fake_key() -> crate::ssh_vault::SshKey {
        crate::ssh_vault::SshKey {
            name: "prod".into(),
            host: "srv.example".into(),
            port: 2222,
            user: "captain".into(),
            private_key: Zeroizing::new(String::new()),
            passphrase: None,
            fingerprint: "SHA256:test".into(),
            added_at: 0,
            last_used: None,
        }
    }

    #[test]
    fn ssh_exec_default_timeout_is_short_hard_guard() {
        assert_eq!(ssh_exec_timeout_secs(None), 60);
    }

    #[test]
    fn ssh_exec_explicit_timeout_is_bounded_review_window() {
        assert_eq!(ssh_exec_timeout_secs(Some(1)), 1);
        assert_eq!(ssh_exec_timeout_secs(Some(9_999)), 7_200);
    }

    #[test]
    fn ssh_exec_blocks_unbounded_remote_monitoring_commands() {
        for command in [
            "journalctl -u tempo -f",
            "docker logs --tail 80 -f tempo",
            "kubectl logs -f deploy/api",
            "tail -f /var/log/syslog",
            "docker events",
            "docker stats",
            "pm2 logs",
            "watch systemctl status tempo",
            "top",
            "tcpdump -i eth0",
        ] {
            let err = assert_ssh_command_allowed(command, None)
                .expect_err("remote monitor should be blocked");
            assert!(err.contains("ssh_exec blocked"), "{err}");
            assert!(err.contains("finite snapshot"), "{err}");
        }
    }

    #[test]
    fn ssh_exec_allows_bounded_remote_snapshots() {
        for command in [
            "journalctl -u tempo -n 80 --no-pager",
            "docker logs --tail 80 tempo",
            "docker stats --no-stream",
            "top -bn1 | head -n 5",
            "tcpdump -c 10 -i eth0",
        ] {
            assert_ssh_command_allowed(command, None)
                .unwrap_or_else(|err| panic!("bounded snapshot should pass: {command}: {err}"));
        }
    }

    #[test]
    fn ssh_alias_label_keeps_requested_and_resolved_names_visible() {
        assert_eq!(ssh_alias_label("prod", "prod"), "prod");
        assert_eq!(ssh_alias_label("prod", "prod-blue"), "prod -> prod-blue");
    }

    #[test]
    fn render_ssh_exec_output_includes_mode_sections_and_newlines() {
        let out = crate::ssh_exec::SshExecOutput {
            stdout: "ok".into(),
            stderr: "warn".into(),
            exit_code: Some(2),
        };
        let payload = render_ssh_exec_output(
            &out,
            &fake_key(),
            Duration::from_millis(12),
            "prod -> prod-blue",
            SshTimeoutMode::ReviewWindow,
        );

        assert!(payload.contains(
            "[exit 2 (non-zero) on captain@srv.example:2222, 12ms, prod -> prod-blue, timeout_mode=review_window]"
        ));
        assert!(payload.contains("--- stdout ---\nok\n"));
        assert!(payload.contains("--- stderr ---\nwarn\n"));
    }

    #[test]
    fn render_ssh_exec_output_marks_empty_hard_timeout_result() {
        let out = crate::ssh_exec::SshExecOutput {
            stdout: String::new(),
            stderr: String::new(),
            exit_code: None,
        };
        let payload = render_ssh_exec_output(
            &out,
            &fake_key(),
            Duration::from_millis(0),
            "prod",
            SshTimeoutMode::HardTimeout,
        );

        assert!(payload.contains("exit ?"));
        assert!(payload.contains("timeout_mode=hard_timeout"));
        assert!(payload.ends_with("(no output)\n"));
    }
}
