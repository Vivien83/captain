use std::process::Command;

#[test]
fn node_cli_help_and_unconfigured_status_are_real_noninteractive_surfaces() {
    let help = Command::new(env!("CARGO_BIN_EXE_captain"))
        .args(["node", "--help"])
        .output()
        .expect("run Captain Node help");
    let help_stdout = String::from_utf8_lossy(&help.stdout);
    let help_stderr = String::from_utf8_lossy(&help.stderr);
    assert!(
        help.status.success(),
        "stdout={help_stdout}\nstderr={help_stderr}"
    );
    for command in ["pair", "run", "status", "reset"] {
        assert!(help_stdout.contains(command), "missing `{command}` in help");
    }

    let home = tempfile::tempdir().expect("isolated Captain home");
    let status = Command::new(env!("CARGO_BIN_EXE_captain"))
        .args(["node", "status", "--json"])
        .env("CAPTAIN_HOME", home.path())
        .output()
        .expect("run Captain Node status");
    let status_stdout = String::from_utf8_lossy(&status.stdout);
    let status_stderr = String::from_utf8_lossy(&status.stderr);
    assert!(
        status.status.success(),
        "stdout={status_stdout}\nstderr={status_stderr}"
    );
    let payload: serde_json::Value =
        serde_json::from_slice(&status.stdout).expect("Node status is exact JSON");
    assert_eq!(payload["configured"], false);
    assert_eq!(payload["state"], "unconfigured");
    assert_eq!(payload["runtime_active"], false);
    assert!(!status_stdout.contains(home.path().to_string_lossy().as_ref()));
    assert!(!home.path().join("node").exists());
}
