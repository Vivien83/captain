use std::io::Write;
use std::process::{Command, Stdio};

#[test]
fn guided_email_setup_writes_one_ready_named_mailbox_without_leaking_secret() {
    let home = tempfile::tempdir().expect("isolated Captain home");
    let mut child = Command::new(env!("CARGO_BIN_EXE_captain"))
        .args(["channel", "setup", "email"])
        .env("CAPTAIN_HOME", home.path())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn Captain CLI");
    child
        .stdin
        .take()
        .expect("stdin pipe")
        .write_all(b"\nperson@gmail.com\n\n\n\n\noperator@example.com\n\n\ntest-app-password\n")
        .expect("write guided answers");
    let output = child.wait_with_output().expect("Captain CLI output");
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);

    assert!(output.status.success(), "stdout={stdout}\nstderr={stderr}");
    assert!(stdout.contains("Email account 'default' configured"));
    assert!(!stdout.contains("test-app-password"));
    assert!(!stderr.contains("test-app-password"));

    let config_raw =
        std::fs::read_to_string(home.path().join("config.toml")).expect("guided setup config");
    let config: captain_types::config::KernelConfig =
        toml::from_str(&config_raw).expect("valid Captain config");
    let email = config.channels.email.expect("Email config");
    let account = &email.effective_accounts()[0];
    assert_eq!(account.alias, "default");
    assert_eq!(account.imap_host, "imap.gmail.com");
    assert_eq!(account.smtp_host, "smtp.gmail.com");
    assert_eq!(account.allowed_senders, ["operator@example.com"]);
    assert_eq!(
        email.effective_default_account().as_deref(),
        Some("default")
    );
    assert!(!config_raw.contains("test-app-password"));

    let secrets = std::fs::read_to_string(home.path().join("secrets.env"))
        .expect("guided setup secret store");
    assert!(secrets.contains(&format!("{}=test-app-password", account.password_env)));
}
