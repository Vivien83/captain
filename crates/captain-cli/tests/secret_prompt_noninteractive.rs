//! Regression test for a bug found while manually verifying the Lot E
//! secret-masking fix (2026-07-04): `prompt_secret()` used
//! `rpassword::prompt_password(...).unwrap_or_default()`. `rpassword` needs a
//! real controlling TTY and returns an `Err` (not an empty string) when
//! stdin is piped/non-interactive — e.g. CI or a setup script doing
//! `echo "$KEY" | captain config set-key openai`. Swallowing that error into
//! an empty string silently discarded the piped value ("No key provided.
//! Cancelled.") instead of using it.
//!
//! This spawns the real compiled binary (the only way to exercise the
//! actual TTY-vs-pipe behavior) with a piped, non-interactive stdin — the
//! same shape as automation would use — and asserts the secret is actually
//! captured and persisted.
//!
//! Uses an unrecognized provider name ("test-provider") so
//! `cmd_config_set_key`'s post-save verification step (`test_api_key`)
//! takes its no-op branch instead of making a real network call, which
//! would make this test flaky/slow/offline-unsafe.

use std::io::Write;
use std::process::{Command, Stdio};

#[test]
fn config_set_key_accepts_a_piped_secret_without_a_tty() {
    let home = tempfile::tempdir().unwrap();

    let mut child = Command::new(env!("CARGO_BIN_EXE_captain"))
        .args(["config", "set-key", "test-provider"])
        .env("CAPTAIN_HOME", home.path())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("failed to spawn captain binary");

    child
        .stdin
        .take()
        .unwrap()
        .write_all(b"sk-test-piped-key-1234567890\n")
        .unwrap();

    let output = child.wait_with_output().unwrap();
    assert!(
        output.status.success(),
        "command should succeed, stdout: {} stderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        !String::from_utf8_lossy(&output.stdout).contains("No key provided"),
        "the piped secret must not be silently discarded as empty"
    );

    let secrets_path = home.path().join("secrets.env");
    let content = std::fs::read_to_string(&secrets_path)
        .expect("secrets.env should exist after a successful set-key");
    assert!(
        content.contains("TEST-PROVIDER_API_KEY=sk-test-piped-key-1234567890")
            || content.contains("TEST_PROVIDER_API_KEY=sk-test-piped-key-1234567890"),
        "the piped secret must actually be persisted, got: {content}"
    );

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(&secrets_path)
            .unwrap()
            .permissions()
            .mode();
        assert_eq!(mode & 0o777, 0o600);
    }
}
