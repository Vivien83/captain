//! Known-hosts persistence for the SSH tools (Q.7c).
//!
//! Wraps `russh_keys::check_known_hosts_path` / `learn_known_hosts_path`
//! with a Captain-shaped decision enum so the SSH client handler can
//! choose the right action (trust / learn / refuse) and emit an audit
//! event.
//!
//! File format is the standard OpenSSH `known_hosts` (so `cat ~/.captain/
//! known_hosts` is human-readable and can be diffed / rotated). We keep
//! Captain's store separate from `~/.ssh/known_hosts` to avoid colliding
//! with the user's interactive ssh client.

use russh::keys::PublicKey;
use std::path::{Path, PathBuf};

/// What to do when a host's key is unknown.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum KhVerificationMode {
    /// **Trust on First Use, then learn**. Unknown hosts are silently
    /// trusted on the first connect and recorded; subsequent connects
    /// must match. Mismatches are always refused. Default — matches
    /// `ssh -o StrictHostKeyChecking=accept-new`.
    #[default]
    TofuLearn,
    /// Refuse anything not already in the store. Mismatch also refused.
    /// For paranoid mode and CI.
    Strict,
    /// Trust everything (legacy `AcceptAll`). Provided for tests and
    /// the rare case where the user explicitly opts in.
    Insecure,
}

/// Decision produced by `verify_or_learn`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum KhDecision {
    /// Host key matches a stored entry — proceed.
    Trusted,
    /// Host was unknown; we just appended it to the store. Proceed.
    Learned,
    /// Host key does NOT match the stored one (potential MITM) — refuse.
    /// `reason` is a human-readable summary for logs and audit.
    Refuse(String),
}

impl KhDecision {
    pub fn is_trusted(&self) -> bool {
        matches!(self, KhDecision::Trusted | KhDecision::Learned)
    }
}

/// Default Captain known_hosts path: `$CAPTAIN_HOME/known_hosts` or
/// `~/.captain/known_hosts`. Caller can also pass any other path
/// (useful for tests).
pub fn default_known_hosts_path() -> PathBuf {
    if let Ok(p) = std::env::var("CAPTAIN_KNOWN_HOSTS") {
        return PathBuf::from(p);
    }
    if let Ok(home) = std::env::var("CAPTAIN_HOME") {
        return PathBuf::from(home).join("known_hosts");
    }
    dirs::home_dir()
        .map(|h| h.join(".captain").join("known_hosts"))
        .unwrap_or_else(|| PathBuf::from(".captain/known_hosts"))
}

/// Sidecar file storing the user-chosen mode. `~/.captain/ssh_kh_mode`.
/// Single line containing `tofu_learn` / `strict` / `insecure`.
pub fn mode_path() -> PathBuf {
    if let Ok(home) = std::env::var("CAPTAIN_HOME") {
        return PathBuf::from(home).join("ssh_kh_mode");
    }
    dirs::home_dir()
        .map(|h| h.join(".captain").join("ssh_kh_mode"))
        .unwrap_or_else(|| PathBuf::from(".captain/ssh_kh_mode"))
}

impl KhVerificationMode {
    /// Stable canonical name (used for sidecar file persistence).
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::TofuLearn => "tofu_learn",
            Self::Strict => "strict",
            Self::Insecure => "insecure",
        }
    }
}

impl std::str::FromStr for KhVerificationMode {
    type Err = ();
    /// Parse a user-supplied label (CLI flag, sidecar file, env var).
    /// `Err(())` for unknown values — caller decides the fallback.
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.trim().to_ascii_lowercase().as_str() {
            "strict" => Ok(Self::Strict),
            "tofu" | "tofu_learn" | "tofulearn" | "default" => Ok(Self::TofuLearn),
            "insecure" | "accept_all" => Ok(Self::Insecure),
            _ => Err(()),
        }
    }
}

/// Pure-fn version of mode resolution. `env_value` simulates `$CAPTAIN_SSH_KH_MODE`
/// (Some = present), `sidecar` is the path to the persisted file. Used by the
/// process-level `current_mode()` and directly by unit tests (no env races).
pub fn resolve_mode(env_value: Option<&str>, sidecar: &Path) -> KhVerificationMode {
    use std::str::FromStr as _;
    if let Some(s) = env_value {
        if let Ok(m) = KhVerificationMode::from_str(s) {
            return m;
        }
    }
    if let Ok(content) = std::fs::read_to_string(sidecar) {
        if let Ok(m) = KhVerificationMode::from_str(&content) {
            return m;
        }
    }
    KhVerificationMode::default()
}

/// Resolve the active mode for the current process.
/// Lookup order:
///   1. `CAPTAIN_SSH_KH_MODE` env var (one-shot override)
///   2. Sidecar `mode_path()` (`captain ssh known-hosts mode <m>`)
///   3. Default `TofuLearn`
pub fn current_mode() -> KhVerificationMode {
    resolve_mode(
        std::env::var("CAPTAIN_SSH_KH_MODE").ok().as_deref(),
        &mode_path(),
    )
}

/// Pure-fn version of persistence: writes `mode` to `path`. Used by the
/// process-level `set_mode()` and directly by tests (no env races).
pub fn write_mode(mode: KhVerificationMode, path: &Path) -> Result<(), String> {
    captain_types::durable_fs::atomic_write(path, mode.as_str().as_bytes())
        .map_err(|e| format!("persist {}: {e}", path.display()))
}

/// Persist `mode` to the sidecar `mode_path()`.
pub fn set_mode(mode: KhVerificationMode) -> Result<(), String> {
    write_mode(mode, &mode_path())
}

/// Verify `pubkey` against the store at `path`, learning it on first use
/// when `mode == TofuLearn`. Pure logic, no audit emission — caller is
/// expected to log the decision.
pub fn verify_or_learn(
    host: &str,
    port: u16,
    pubkey: &PublicKey,
    path: &Path,
    mode: KhVerificationMode,
) -> KhDecision {
    if mode == KhVerificationMode::Insecure {
        return KhDecision::Trusted;
    }

    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }

    match russh::keys::known_hosts::check_known_hosts_path(host, port, pubkey, path) {
        Ok(true) => KhDecision::Trusted,
        Ok(false) => match mode {
            KhVerificationMode::TofuLearn => {
                match russh::keys::known_hosts::learn_known_hosts_path(host, port, pubkey, path) {
                    Ok(()) => KhDecision::Learned,
                    Err(e) => KhDecision::Refuse(format!(
                        "Failed to record host key for {host}:{port}: {e}"
                    )),
                }
            }
            KhVerificationMode::Strict => KhDecision::Refuse(format!(
                "Host {host}:{port} not in known_hosts (strict mode). \
                 Run `captain ssh test <key>` first or add it manually."
            )),
            KhVerificationMode::Insecure => unreachable!("handled above"),
        },
        Err(e) => {
            // russh_keys::Error::KeyChanged { line } is the MITM signal.
            KhDecision::Refuse(format!(
                "Host key VERIFICATION FAILED for {host}:{port}: {e}. \
                 Possible man-in-the-middle attack — refused. \
                 Inspect {} and remove the offending line if you trust the change.",
                path.display()
            ))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use russh::keys::key::safe_rng;
    use russh::keys::{Algorithm, PrivateKey};

    fn fresh_pubkey() -> PublicKey {
        let kp =
            PrivateKey::random(&mut safe_rng(), Algorithm::Ed25519).expect("ed25519 generation");
        kp.public_key().clone()
    }

    // Env-touching test removed (race-prone with parallel tests). The pure
    // path-resolution logic is exercised via `resolve_mode` /
    // `write_mode` in `q7cb_write_and_resolve_mode_round_trip_no_env_race`.
    // The env-var lookup itself is trivial (one-line `var().ok()`).

    #[test]
    fn insecure_mode_always_trusts() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("kh");
        let key = fresh_pubkey();
        let d = verify_or_learn(
            "server.example.com",
            22,
            &key,
            &path,
            KhVerificationMode::Insecure,
        );
        assert_eq!(d, KhDecision::Trusted);
        assert!(d.is_trusted());
        assert!(!path.exists(), "Insecure must NOT write to the store");
    }

    #[test]
    fn tofu_learns_unknown_host_then_trusts_subsequent_calls() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("kh");
        let key = fresh_pubkey();

        let first = verify_or_learn(
            "a.example.com",
            22,
            &key,
            &path,
            KhVerificationMode::TofuLearn,
        );
        assert_eq!(first, KhDecision::Learned);
        assert!(path.exists(), "Learned must write the store");
        let stored = std::fs::read_to_string(&path).unwrap();
        assert!(stored.contains("a.example.com"));

        let second = verify_or_learn(
            "a.example.com",
            22,
            &key,
            &path,
            KhVerificationMode::TofuLearn,
        );
        assert_eq!(second, KhDecision::Trusted);
    }

    #[test]
    fn strict_refuses_unknown_host() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("kh");
        let key = fresh_pubkey();
        let d = verify_or_learn(
            "ghost.example.com",
            22,
            &key,
            &path,
            KhVerificationMode::Strict,
        );
        match d {
            KhDecision::Refuse(reason) => {
                assert!(reason.contains("not in known_hosts"), "got: {reason}");
                assert!(reason.contains("strict"), "got: {reason}");
            }
            other => panic!("expected Refuse, got: {other:?}"),
        }
        assert!(!path.exists(), "Strict must NOT learn");
    }

    #[test]
    fn mismatched_key_is_refused_with_clear_reason() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("kh");
        let key1 = fresh_pubkey();
        let key2 = fresh_pubkey();

        // Establish trust with key1.
        let learned = verify_or_learn(
            "server.example.com",
            22,
            &key1,
            &path,
            KhVerificationMode::TofuLearn,
        );
        assert_eq!(learned, KhDecision::Learned);

        // Then try with key2 — same host:port, different key. Must refuse,
        // even in TofuLearn mode (mismatch always wins).
        let attack = verify_or_learn(
            "server.example.com",
            22,
            &key2,
            &path,
            KhVerificationMode::TofuLearn,
        );
        match attack {
            KhDecision::Refuse(reason) => {
                assert!(
                    reason.contains("VERIFICATION FAILED") || reason.contains("man-in-the-middle"),
                    "expected MITM warning, got: {reason}"
                );
                assert!(reason.contains("server.example.com"));
            }
            other => panic!("expected Refuse on mismatch, got: {other:?}"),
        }
    }

    #[test]
    fn distinct_ports_for_same_host_are_treated_separately() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("kh");
        let key_22 = fresh_pubkey();
        let key_2222 = fresh_pubkey();

        assert_eq!(
            verify_or_learn(
                "h.example.com",
                22,
                &key_22,
                &path,
                KhVerificationMode::TofuLearn
            ),
            KhDecision::Learned
        );
        // Different port → still unknown → still learns (no mismatch).
        assert_eq!(
            verify_or_learn(
                "h.example.com",
                2222,
                &key_2222,
                &path,
                KhVerificationMode::TofuLearn
            ),
            KhDecision::Learned
        );
    }

    #[test]
    fn is_trusted_helper_covers_both_positive_variants() {
        assert!(KhDecision::Trusted.is_trusted());
        assert!(KhDecision::Learned.is_trusted());
        assert!(!KhDecision::Refuse("x".into()).is_trusted());
    }

    // ─── Q.7c.b — KhVerificationMode round-trip + persistence ──────────────

    #[test]
    fn q7cb_mode_from_str_accepts_canonical_and_aliases() {
        use std::str::FromStr as _;
        assert_eq!(
            "strict".parse::<KhVerificationMode>(),
            Ok(KhVerificationMode::Strict)
        );
        assert_eq!(
            "STRICT".parse::<KhVerificationMode>(),
            Ok(KhVerificationMode::Strict)
        );
        assert_eq!(
            "tofu".parse::<KhVerificationMode>(),
            Ok(KhVerificationMode::TofuLearn)
        );
        assert_eq!(
            "tofu_learn".parse::<KhVerificationMode>(),
            Ok(KhVerificationMode::TofuLearn)
        );
        assert_eq!(
            "default".parse::<KhVerificationMode>(),
            Ok(KhVerificationMode::TofuLearn)
        );
        assert_eq!(
            "insecure".parse::<KhVerificationMode>(),
            Ok(KhVerificationMode::Insecure)
        );
        assert_eq!(
            "accept_all".parse::<KhVerificationMode>(),
            Ok(KhVerificationMode::Insecure)
        );
        assert_eq!(
            " strict\n".parse::<KhVerificationMode>(),
            Ok(KhVerificationMode::Strict)
        );
        assert!("nope".parse::<KhVerificationMode>().is_err());
        assert!("".parse::<KhVerificationMode>().is_err());
        // Direct trait usage too
        assert_eq!(
            KhVerificationMode::from_str("tofu"),
            Ok(KhVerificationMode::TofuLearn)
        );
    }

    #[test]
    fn q7cb_mode_as_str_round_trip() {
        for m in [
            KhVerificationMode::TofuLearn,
            KhVerificationMode::Strict,
            KhVerificationMode::Insecure,
        ] {
            let s = m.as_str();
            assert_eq!(
                s.parse::<KhVerificationMode>(),
                Ok(m),
                "round-trip broken for {s}"
            );
        }
    }

    #[test]
    fn q7cb_write_and_resolve_mode_round_trip_no_env_race() {
        // Pure-fn version : pas de set_var (donc safe avec test parallelisme).
        let dir = tempfile::tempdir().unwrap();
        let sidecar = dir.path().join("ssh_kh_mode");

        // 1. Pas de fichier + pas d'env → default TofuLearn
        assert_eq!(resolve_mode(None, &sidecar), KhVerificationMode::TofuLearn);

        // 2. write_mode(Strict) crée le sidecar
        write_mode(KhVerificationMode::Strict, &sidecar).expect("write");
        let on_disk = std::fs::read_to_string(&sidecar).unwrap();
        assert_eq!(on_disk, "strict");

        // 3. resolve sans env → lit le sidecar Strict
        assert_eq!(resolve_mode(None, &sidecar), KhVerificationMode::Strict);

        // 4. env var "insecure" override le sidecar
        assert_eq!(
            resolve_mode(Some("insecure"), &sidecar),
            KhVerificationMode::Insecure
        );

        // 5. env value invalide → fallback sidecar (Strict)
        assert_eq!(
            resolve_mode(Some("garbage"), &sidecar),
            KhVerificationMode::Strict
        );

        // 6. write_mode crée les dossiers parents si absents
        let nested = dir.path().join("a/b/c/ssh_kh_mode");
        write_mode(KhVerificationMode::Insecure, &nested).expect("create parents");
        assert_eq!(resolve_mode(None, &nested), KhVerificationMode::Insecure);
    }
}
