use std::process::Command;

#[test]
fn client_cli_noninteractive_help_status_and_reset_are_real_surfaces() {
    let help = Command::new(env!("CARGO_BIN_EXE_captain"))
        .args(["client", "--help"])
        .output()
        .expect("run Captain Client help");
    let help_stdout = String::from_utf8_lossy(&help.stdout);
    let help_stderr = String::from_utf8_lossy(&help.stderr);
    assert!(
        help.status.success(),
        "stdout={help_stdout}\nstderr={help_stderr}"
    );
    for command in ["pair", "status", "reset"] {
        assert!(help_stdout.contains(command), "missing `{command}` in help");
    }

    let home = tempfile::tempdir().expect("isolated Captain home");
    let status = Command::new(env!("CARGO_BIN_EXE_captain"))
        .args(["client", "status", "--json"])
        .env("CAPTAIN_HOME", home.path())
        .output()
        .expect("run Captain Client status");
    let status_stdout = String::from_utf8_lossy(&status.stdout);
    let status_stderr = String::from_utf8_lossy(&status.stderr);
    assert!(
        status.status.success(),
        "stdout={status_stdout}\nstderr={status_stderr}"
    );
    let payload: serde_json::Value =
        serde_json::from_slice(&status.stdout).expect("Client status is exact JSON");
    assert_eq!(payload["configured"], false);
    assert_eq!(payload["state"], "unconfigured");
    assert!(!status_stdout.contains(home.path().to_string_lossy().as_ref()));
    assert!(!home.path().join("client").exists());

    let reset = Command::new(env!("CARGO_BIN_EXE_captain"))
        .args(["client", "reset"])
        .env("CAPTAIN_HOME", home.path())
        .output()
        .expect("run unconfirmed Client reset");
    assert!(!reset.status.success());
    let reset_output = format!(
        "{}{}",
        String::from_utf8_lossy(&reset.stdout),
        String::from_utf8_lossy(&reset.stderr)
    );
    assert!(reset_output.contains("--yes"));
}

#[test]
fn paired_client_chat_fails_closed_without_starting_a_local_runtime() {
    let home = tempfile::tempdir().expect("isolated Captain home");
    let client_dir = home.path().join("client");
    std::fs::create_dir_all(&client_dir).expect("create Client state directory");
    std::fs::write(client_dir.join("config.toml"), "not valid toml = [")
        .expect("write intentionally invalid Client profile");

    let output = Command::new(env!("CARGO_BIN_EXE_captain"))
        .args(["chat", "--plain"])
        .env("CAPTAIN_HOME", home.path())
        .output()
        .expect("run paired Client chat preflight");
    let combined = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    assert!(!output.status.success(), "output={combined}");
    assert!(combined.contains("Client Hub unavailable"), "{combined}");
    assert!(!combined.contains("Captain Chat"), "{combined}");
    assert!(!home.path().join("data").exists());
    assert!(!home.path().join("sessions").exists());
}
