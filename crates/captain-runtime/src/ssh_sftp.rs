//! SFTP file transfer for the `tool_ssh_upload` and `tool_ssh_download`
//! tools — built on `russh-sftp` over a `russh` client session.
//!
//! Blocking transfers (whole file in memory) — fine for configs/scripts.
//! Streaming for large blobs is intentionally out of scope (the agent has
//! `shell_exec` for `dd` / `rsync` / etc. when needed).

use russh::client::{self, AuthResult, Handle, Handler};
use russh::keys::{decode_secret_key, PrivateKeyWithHashAlg, PublicKey};
use russh_sftp::client::SftpSession;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;
use tokio::io::AsyncReadExt;
use tracing::{info, warn};

use crate::ssh_known_hosts::{verify_or_learn, KhDecision, KhVerificationMode};
use crate::ssh_vault::SshKey;

/// Q.7c — same known_hosts-aware handler as `ssh_exec`. Kept duplicated
/// (rather than shared via a public crate type) because the trait
/// `Handler` is implemented per-module to avoid leaking russh types.
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
                info!(host = %self.host, port = self.port, "sftp: host key trusted");
                Ok(true)
            }
            KhDecision::Learned => {
                info!(host = %self.host, port = self.port, "sftp: host key learned (TOFU)");
                Ok(true)
            }
            KhDecision::Refuse(reason) => {
                warn!(host = %self.host, port = self.port, reason = %reason, "sftp: host key REFUSED");
                Ok(false)
            }
        }
    }
}

async fn open_sftp_session(
    key: &SshKey,
) -> Result<(Handle<CaptainHostKeyHandler>, SftpSession), String> {
    let pp = key.passphrase.as_ref().map(|p| p.as_str());
    let key_pair = decode_secret_key(key.private_key.as_str(), pp)
        .map_err(|e| format!("Failed to parse stored private key: {e}"))?;

    let config = Arc::new(client::Config::default());
    let handler = CaptainHostKeyHandler {
        host: key.host.clone(),
        port: key.port,
        kh_path: crate::ssh_known_hosts::default_known_hosts_path(),
        mode: crate::ssh_known_hosts::current_mode(),
    };
    let mut session: Handle<CaptainHostKeyHandler> =
        client::connect(config, (key.host.as_str(), key.port), handler)
            .await
            .map_err(|e| format!("Failed to connect to {}:{}: {e}", key.host, key.port))?;

    // For RSA keys the server may not accept the legacy ssh-rsa (SHA-1)
    // signature scheme; negotiate the strongest scheme it advertises via
    // the server-sig-algs extension. Non-RSA keys ignore this hint.
    let hash_alg = session
        .best_supported_rsa_hash()
        .await
        .map_err(|e| format!("Authentication error: {e}"))?
        .flatten();
    let auth = session
        .authenticate_publickey(
            &key.user,
            PrivateKeyWithHashAlg::new(Arc::new(key_pair), hash_alg),
        )
        .await
        .map_err(|e| format!("Authentication error: {e}"))?;
    if !matches!(auth, AuthResult::Success) {
        return Err(format!(
            "Authentication failed for user '{}' at {}",
            key.user, key.host
        ));
    }

    let chan = session
        .channel_open_session()
        .await
        .map_err(|e| format!("Failed to open session channel: {e}"))?;
    chan.request_subsystem(true, "sftp")
        .await
        .map_err(|e| format!("Failed to request sftp subsystem: {e}"))?;
    let sftp = SftpSession::new(chan.into_stream())
        .await
        .map_err(|e| format!("Failed to start SFTP session: {e}"))?;

    Ok((session, sftp))
}

/// Upload `local_path` to `remote_path` on the host described by `key`.
/// Returns the number of bytes written.
pub async fn ssh_upload(
    key: &SshKey,
    local_path: &std::path::Path,
    remote_path: &str,
    timeout: Duration,
) -> Result<u64, String> {
    tokio::time::timeout(timeout, ssh_upload_inner(key, local_path, remote_path))
        .await
        .map_err(|_| format!("SFTP upload timed out after {} s", timeout.as_secs()))?
}

async fn ssh_upload_inner(
    key: &SshKey,
    local_path: &std::path::Path,
    remote_path: &str,
) -> Result<u64, String> {
    let data = tokio::fs::read(local_path)
        .await
        .map_err(|e| format!("Failed to read local file '{}': {e}", local_path.display()))?;
    let bytes = data.len() as u64;

    let (_session, sftp) = open_sftp_session(key).await?;
    let mut file = sftp
        .create(remote_path)
        .await
        .map_err(|e| format!("Failed to create remote '{remote_path}': {e}"))?;
    use tokio::io::AsyncWriteExt;
    file.write_all(&data)
        .await
        .map_err(|e| format!("Failed to write remote '{remote_path}': {e}"))?;
    file.shutdown()
        .await
        .map_err(|e| format!("Failed to flush remote '{remote_path}': {e}"))?;
    Ok(bytes)
}

/// Download `remote_path` to `local_path`. Returns the number of bytes
/// downloaded.
pub async fn ssh_download(
    key: &SshKey,
    remote_path: &str,
    local_path: &std::path::Path,
    timeout: Duration,
) -> Result<u64, String> {
    tokio::time::timeout(timeout, ssh_download_inner(key, remote_path, local_path))
        .await
        .map_err(|_| format!("SFTP download timed out after {} s", timeout.as_secs()))?
}

async fn ssh_download_inner(
    key: &SshKey,
    remote_path: &str,
    local_path: &std::path::Path,
) -> Result<u64, String> {
    if let Some(parent) = local_path.parent() {
        tokio::fs::create_dir_all(parent)
            .await
            .map_err(|e| format!("Failed to create local parent dirs: {e}"))?;
    }

    let (_session, sftp) = open_sftp_session(key).await?;
    let mut file = sftp
        .open(remote_path)
        .await
        .map_err(|e| format!("Failed to open remote '{remote_path}': {e}"))?;
    let mut data = Vec::new();
    file.read_to_end(&mut data)
        .await
        .map_err(|e| format!("Failed to read remote '{remote_path}': {e}"))?;
    let bytes = data.len() as u64;

    tokio::fs::write(local_path, &data)
        .await
        .map_err(|e| format!("Failed to write local '{}': {e}", local_path.display()))?;
    Ok(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ssh_vault::SshKey;
    use zeroize::Zeroizing;

    fn fake_key() -> SshKey {
        SshKey {
            name: "test".into(),
            host: "192.0.2.1".into(),
            port: 22,
            user: "captain".into(),
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
    async fn upload_local_read_failure_propagates() {
        let key = fake_key();
        let r = ssh_upload(
            &key,
            std::path::Path::new("/nonexistent/path/to/file"),
            "/tmp/out",
            Duration::from_secs(2),
        )
        .await;
        assert!(r.is_err());
        assert!(r.unwrap_err().contains("Failed to read local file"));
    }

    #[tokio::test]
    async fn download_unreachable_host_returns_error() {
        let dir = tempfile::tempdir().unwrap();
        let local = dir.path().join("out");
        let key = fake_key();
        let r = ssh_download(&key, "/etc/hostname", &local, Duration::from_secs(2)).await;
        assert!(r.is_err());
        let err = r.unwrap_err();
        assert!(
            err.contains("connect") || err.contains("timed out") || err.contains("Failed"),
            "got: {err}"
        );
    }
}
