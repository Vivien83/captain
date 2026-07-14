//! Minimal async SSH command executor for the `tool_ssh_exec` tool.
//!
//! Uses [`russh`] (pure-Rust, tokio-native) — no shell out to `/usr/bin/ssh`.
//! Authenticates with the private key stored in the Captain vault (see
//! `ssh_vault::SshKey`), opens an exec channel, captures stdout/stderr,
//! and returns the exit code.
//!
//! Host-key verification is currently TOFU (trust on first use, every time
//! — known_hosts persistence will follow). Q.9's `CriticalMode` already
//! gates the remote command on the agent side BEFORE it reaches this
//! function, so root-level destructive patterns are caught.

use russh::client::{self, AuthResult, Handle, Handler};
use russh::keys::{decode_secret_key, PrivateKey, PublicKey};
use russh::{Channel, ChannelMsg, Disconnect, Sig};
use std::fmt::Display;
use std::future::Future;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;
use tracing::{info, warn};

use crate::ssh_known_hosts::{verify_or_learn, KhDecision, KhVerificationMode};
use crate::ssh_vault::SshKey;

/// Result of a remote command execution.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct SshExecOutput {
    pub stdout: String,
    pub stderr: String,
    pub exit_code: Option<u32>,
}

/// Q.7c — known_hosts-aware handler. TOFU at first connect, strict on
/// subsequent connects (mismatch = refuse). The key is recorded in
/// `~/.captain/known_hosts` (overridable via `CAPTAIN_KNOWN_HOSTS`).
struct CaptainHostKeyHandler {
    host: String,
    port: u16,
    kh_path: PathBuf,
    mode: KhVerificationMode,
}

impl Handler for CaptainHostKeyHandler {
    type Error = russh::Error;

    async fn check_server_key(
        &mut self,
        server_public_key: &PublicKey,
    ) -> Result<bool, Self::Error> {
        match verify_or_learn(
            &self.host,
            self.port,
            server_public_key,
            &self.kh_path,
            self.mode,
        ) {
            KhDecision::Trusted => {
                info!(host = %self.host, port = self.port, "ssh: host key trusted");
                Ok(true)
            }
            KhDecision::Learned => {
                info!(host = %self.host, port = self.port, "ssh: host key learned (TOFU)");
                Ok(true)
            }
            KhDecision::Refuse(reason) => {
                warn!(host = %self.host, port = self.port, reason = %reason, "ssh: host key REFUSED");
                Ok(false)
            }
        }
    }
}

/// Execute `command` on the remote host described by `key`. Returns
/// stdout, stderr, and the exit code. The whole call is wrapped in a
/// `tokio::time::timeout(timeout)`.
pub async fn ssh_exec(
    key: &SshKey,
    command: &str,
    timeout: Duration,
) -> Result<SshExecOutput, String> {
    tokio::time::timeout(timeout, ssh_exec_inner(key, command, None))
        .await
        .map_err(|_| format!("SSH exec timed out after {} s", timeout.as_secs()))?
}

/// Execute a remote command with `review_window` as a bounded progress window.
///
/// Connection, authentication, channel open and exec setup are still bounded by
/// the window because no healthy remote command is running yet. Once the remote
/// process is started, a live SSH channel may renew a few progress windows, but
/// a final hard cap closes the channel so Telegram/API runs cannot stay active
/// forever on a remote watcher or stalled command.
pub async fn ssh_exec_with_review_window(
    key: &SshKey,
    command: &str,
    review_window: Duration,
) -> Result<SshExecOutput, String> {
    ssh_exec_inner(
        key,
        command,
        Some(review_window.max(Duration::from_secs(1))),
    )
    .await
}

async fn with_optional_timeout<T, E, F>(
    label: &str,
    timeout: Option<Duration>,
    fut: F,
) -> Result<T, String>
where
    E: Display,
    F: Future<Output = Result<T, E>>,
{
    match timeout {
        Some(timeout) => tokio::time::timeout(timeout, fut)
            .await
            .map_err(|_| format!("SSH {label} timed out after {} s", timeout.as_secs()))?
            .map_err(|e| format!("SSH {label} failed: {e}")),
        None => fut.await.map_err(|e| format!("SSH {label} failed: {e}")),
    }
}

async fn ssh_exec_inner(
    key: &SshKey,
    command: &str,
    review_window: Option<Duration>,
) -> Result<SshExecOutput, String> {
    let key_pair = decode_stored_key(key)?;
    let mut session = connect_session(key, review_window).await?;
    authenticate_session(&mut session, key, key_pair, review_window).await?;
    let chan = open_exec_channel(&mut session, command, review_window).await?;
    let output = collect_command_output(chan, review_window).await;
    disconnect_session(session).await;

    output
}

fn decode_stored_key(key: &SshKey) -> Result<PrivateKey, String> {
    let pp = key.passphrase.as_ref().map(|p| p.as_str());
    decode_secret_key(key.private_key.as_str(), pp)
        .map_err(|e| format!("Failed to parse stored private key: {e}"))
}

async fn connect_session(
    key: &SshKey,
    review_window: Option<Duration>,
) -> Result<Handle<CaptainHostKeyHandler>, String> {
    let config = Arc::new(client::Config::default());
    let addr = (key.host.as_str(), key.port);
    let handler = host_key_handler_for(key);
    with_optional_timeout(
        "connect",
        review_window,
        client::connect(config, addr, handler),
    )
    .await
    .map_err(|e| format!("Failed to connect to {}:{}: {e}", key.host, key.port))
}

fn host_key_handler_for(key: &SshKey) -> CaptainHostKeyHandler {
    CaptainHostKeyHandler {
        host: key.host.clone(),
        port: key.port,
        kh_path: crate::ssh_known_hosts::default_known_hosts_path(),
        mode: crate::ssh_known_hosts::current_mode(),
    }
}

async fn authenticate_session(
    session: &mut Handle<CaptainHostKeyHandler>,
    key: &SshKey,
    key_pair: PrivateKey,
    review_window: Option<Duration>,
) -> Result<(), String> {
    // For RSA keys the server may not accept the legacy ssh-rsa (SHA-1)
    // signature scheme; negotiate the strongest scheme it advertises via
    // the server-sig-algs extension. Non-RSA keys ignore this hint.
    let hash_alg = session
        .best_supported_rsa_hash()
        .await
        .map_err(|e| format!("Authentication error: {e}"))?
        .flatten();
    let auth = with_optional_timeout(
        "authenticate",
        review_window,
        session.authenticate_publickey(
            &key.user,
            russh::keys::PrivateKeyWithHashAlg::new(Arc::new(key_pair), hash_alg),
        ),
    )
    .await
    .map_err(|e| format!("Authentication error: {e}"))?;
    if !matches!(auth, AuthResult::Success) {
        return Err(format!(
            "Authentication failed for user '{}' at {}",
            key.user, key.host
        ));
    }
    Ok(())
}

async fn open_exec_channel(
    session: &mut Handle<CaptainHostKeyHandler>,
    command: &str,
    review_window: Option<Duration>,
) -> Result<Channel<client::Msg>, String> {
    let chan = with_optional_timeout(
        "open session",
        review_window,
        session.channel_open_session(),
    )
    .await
    .map_err(|e| format!("Failed to open session channel: {e}"))?;
    with_optional_timeout("exec", review_window, chan.exec(true, command))
        .await
        .map_err(|e| format!("Failed to exec command: {e}"))?;
    Ok(chan)
}

async fn collect_command_output(
    mut chan: Channel<client::Msg>,
    review_window: Option<Duration>,
) -> Result<SshExecOutput, String> {
    let mut output = SshExecOutput::default();
    if let Some(review_window) = review_window {
        collect_with_review_window(&mut chan, &mut output, review_window).await?;
    } else {
        collect_until_close(&mut chan, &mut output, false).await;
    }
    Ok(output)
}

async fn collect_with_review_window(
    chan: &mut Channel<client::Msg>,
    output: &mut SshExecOutput,
    review_window: Duration,
) -> Result<(), String> {
    let progress_interval = review_progress_interval(review_window);
    let hard_cap = ssh_review_hard_cap(review_window);
    let mut review = Box::pin(tokio::time::sleep(progress_interval));
    let mut deadline = Box::pin(tokio::time::sleep(hard_cap));
    loop {
        tokio::select! {
            msg = chan.wait() => {
                if !collect_channel_msg(msg, output, true) {
                    break;
                }
            }
            _ = &mut review => {
                emit_review_window_progress(review_window, hard_cap);
                review.as_mut().reset(tokio::time::Instant::now() + progress_interval);
            }
            _ = &mut deadline => {
                terminate_timed_out_channel(chan).await;
                return Err(format!(
                    "SSH exec exceeded bounded review window after {}s (timeout_secs={} is reviewed, not renewed indefinitely). Partial output:\n{}",
                    hard_cap.as_secs(),
                    review_window.as_secs(),
                    format_partial_ssh_output(output)
                ));
            }
        }
    }
    Ok(())
}

async fn collect_until_close(
    chan: &mut Channel<client::Msg>,
    output: &mut SshExecOutput,
    stream_chunks: bool,
) {
    while let Some(msg) = chan.wait().await {
        if !collect_channel_msg(Some(msg), output, stream_chunks) {
            break;
        }
    }
}

fn review_progress_interval(review_window: Duration) -> Duration {
    Duration::from_secs(review_window.as_secs().clamp(1, 30))
}

fn ssh_review_hard_cap(review_window: Duration) -> Duration {
    let secs = review_window.as_secs().max(1);
    Duration::from_secs(secs.saturating_mul(3).max(secs + 2))
}

fn emit_review_window_progress(review_window: Duration, hard_cap: Duration) {
    crate::tools::emit_tool_chunk(
        "progress",
        &format!(
            "SSH exec still running; remote channel is alive. timeout_secs={} is a bounded review window; hard cap={}s.\n",
            review_window.as_secs(),
            hard_cap.as_secs(),
        ),
    );
}

async fn terminate_timed_out_channel(chan: &Channel<client::Msg>) {
    let _ = chan.signal(Sig::TERM).await;
    let _ = chan.close().await;
}

async fn disconnect_session(session: Handle<CaptainHostKeyHandler>) {
    let _ = session
        .disconnect(Disconnect::ByApplication, "captain done", "en")
        .await;
}

fn format_partial_ssh_output(output: &SshExecOutput) -> String {
    let mut payload = String::new();
    if !output.stdout.is_empty() {
        payload.push_str("--- stdout ---\n");
        payload.push_str(&output.stdout);
        if !output.stdout.ends_with('\n') {
            payload.push('\n');
        }
    }
    if !output.stderr.is_empty() {
        payload.push_str("--- stderr ---\n");
        payload.push_str(&output.stderr);
        if !output.stderr.ends_with('\n') {
            payload.push('\n');
        }
    }
    if payload.is_empty() {
        payload.push_str("(no output)\n");
    }
    payload
}

fn collect_channel_msg(
    msg: Option<ChannelMsg>,
    output: &mut SshExecOutput,
    stream_chunks: bool,
) -> bool {
    let Some(msg) = msg else {
        return false;
    };
    match msg {
        ChannelMsg::Data { ref data } => {
            let chunk = String::from_utf8_lossy(data);
            if stream_chunks {
                crate::tools::emit_tool_chunk("stdout", &chunk);
            }
            output.stdout.push_str(&chunk);
        }
        ChannelMsg::ExtendedData { ref data, ext: 1 } => {
            let chunk = String::from_utf8_lossy(data);
            if stream_chunks {
                crate::tools::emit_tool_chunk("stderr", &chunk);
            }
            output.stderr.push_str(&chunk);
        }
        ChannelMsg::ExitStatus { exit_status } => {
            output.exit_code = Some(exit_status);
        }
        ChannelMsg::Eof | ChannelMsg::Close => return false,
        _ => {}
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ssh_vault::SshKey;
    use bytes::Bytes;
    use zeroize::Zeroizing;

    fn fake_key(host: &str, port: u16) -> SshKey {
        SshKey {
            name: "test".into(),
            host: host.to_string(),
            port,
            user: "captain".into(),
            // Real ed25519 PEM (same fixture as ssh_vault tests). Doesn't
            // matter that the host doesn't exist — connect fails first.
            private_key: Zeroizing::new(
                "-----BEGIN OPENSSH PRIVATE KEY-----\n\
                 b3BlbnNzaC1rZXktdjEAAAAABG5vbmUAAAAEbm9uZQAAAAAAAAABAAAAMwAAAAtzc2gtZW\n\
                 QyNTUxOQAAACC+h2XHFRvMhz24O6tMKm+B4QWriqoCGRDOYMa9suc91wAAAJjaN0w+2jdM\n\
                 PgAAAAtzc2gtZWQyNTUxOQAAACC+h2XHFRvMhz24O6tMKm+B4QWriqoCGRDOYMa9suc91w\n\
                 AAAEC6CAU3QqHvG1dbSzfbmLdSAVxzjYbVbfM+hPRn8M3p5b6HZccVG8yHPbg7q0wqb4Hh\n\
                 BauKqgIZEM5gxr2y5z3XAAAAEXRlc3QtcTYtdGhyb3dhd2F5AQIDBA==\n\
                 -----END OPENSSH PRIVATE KEY-----\n"
                    .to_string(),
            ),
            passphrase: None,
            fingerprint: "SHA256:cn5IEOhe/2DG5+14DcUbPM6kcab6TKj0pknTjrhyf5E".into(),
            added_at: 0,
            last_used: None,
        }
    }

    #[tokio::test]
    async fn returns_clear_error_on_unreachable_host() {
        // Reserved-for-documentation IP, blocked at the protocol level.
        let key = fake_key("192.0.2.1", 22);
        let result = ssh_exec(&key, "true", Duration::from_secs(2)).await;
        assert!(result.is_err(), "got: {result:?}");
        let err = result.unwrap_err();
        assert!(
            err.contains("connect") || err.contains("timed out") || err.contains("Failed"),
            "expected connect/timeout error, got: {err}"
        );
    }

    #[tokio::test]
    async fn rejects_invalid_pem_with_clear_error() {
        let mut key = fake_key("127.0.0.1", 22);
        key.private_key = Zeroizing::new("not a key".to_string());
        let r = ssh_exec(&key, "true", Duration::from_secs(2)).await;
        assert!(r.is_err());
        assert!(r.unwrap_err().contains("Failed to parse"));
    }

    #[tokio::test]
    async fn timeout_is_enforced() {
        // Same reserved-IP trick — the connect will hang and we want the
        // outer tokio::time::timeout to fire well before the OS gives up.
        let key = fake_key("192.0.2.1", 22);
        let start = std::time::Instant::now();
        let _ = ssh_exec(&key, "true", Duration::from_millis(500)).await;
        let elapsed = start.elapsed();
        assert!(
            elapsed < Duration::from_secs(3),
            "outer timeout must bound the call (elapsed: {elapsed:?})"
        );
    }

    #[test]
    fn review_progress_interval_is_bounded() {
        assert_eq!(
            review_progress_interval(Duration::from_millis(500)),
            Duration::from_secs(1)
        );
        assert_eq!(
            review_progress_interval(Duration::from_secs(12)),
            Duration::from_secs(12)
        );
        assert_eq!(
            review_progress_interval(Duration::from_secs(120)),
            Duration::from_secs(30)
        );
    }

    #[test]
    fn ssh_review_hard_cap_is_bounded_above_review_window() {
        assert_eq!(
            ssh_review_hard_cap(Duration::from_millis(500)),
            Duration::from_secs(3)
        );
        assert_eq!(
            ssh_review_hard_cap(Duration::from_secs(20)),
            Duration::from_secs(60)
        );
        assert_eq!(
            ssh_review_hard_cap(Duration::from_secs(120)),
            Duration::from_secs(360)
        );
    }

    #[test]
    fn format_partial_ssh_output_keeps_stream_sections() {
        let output = SshExecOutput {
            stdout: "ok".into(),
            stderr: "warn".into(),
            exit_code: None,
        };

        assert_eq!(
            format_partial_ssh_output(&output),
            "--- stdout ---\nok\n--- stderr ---\nwarn\n"
        );
        assert_eq!(
            format_partial_ssh_output(&SshExecOutput::default()),
            "(no output)\n"
        );
    }

    #[test]
    fn collect_channel_msg_updates_output_and_stop_state() {
        let mut output = SshExecOutput::default();

        assert!(collect_channel_msg(
            Some(ChannelMsg::Data {
                data: Bytes::from_static(b"hello")
            }),
            &mut output,
            false,
        ));
        assert!(collect_channel_msg(
            Some(ChannelMsg::ExtendedData {
                data: Bytes::from_static(b"warn"),
                ext: 1,
            }),
            &mut output,
            false,
        ));
        assert!(collect_channel_msg(
            Some(ChannelMsg::ExitStatus { exit_status: 7 }),
            &mut output,
            false,
        ));

        assert_eq!(output.stdout, "hello");
        assert_eq!(output.stderr, "warn");
        assert_eq!(output.exit_code, Some(7));
        assert!(!collect_channel_msg(
            Some(ChannelMsg::Close),
            &mut output,
            false
        ));
        assert!(!collect_channel_msg(None, &mut output, false));
    }
}
