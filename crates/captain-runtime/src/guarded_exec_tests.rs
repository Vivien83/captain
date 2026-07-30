use captain_types::config::{CriticalMode, ExecPolicy, ExecSecurityMode};

use super::*;

#[test]
fn every_unattended_surface_blocks_critical_content() {
    let policy = unattended_policy(30);
    for surface in [
        ExecSurface::GoalCheck,
        ExecSurface::GoalRecovery,
        ExecSurface::SkillCapability,
        ExecSurface::CodeExecution,
        ExecSurface::Workflow,
        ExecSurface::SkillCheck,
        ExecSurface::HandInstall,
        ExecSurface::WasmHost,
    ] {
        let error = review_script(surface, "bash", "rm -rf /", Some(&policy))
            .expect_err("critical content must be blocked");
        assert!(
            error.contains("critical pattern"),
            "surface {surface:?}: {error}"
        );
    }
}

#[test]
fn interactive_shell_returns_an_approval_decision() {
    let policy = ExecPolicy {
        critical_mode: CriticalMode::Open,
        ..ExecPolicy::default()
    };

    assert_eq!(
        review_shell(ExecSurface::ShellTool, "rm -rf /", Some(&policy), true).unwrap(),
        ReviewDecision::ApprovalRequired {
            pattern: "rm -rf /"
        }
    );
}

#[test]
fn default_interactive_policy_blocks_normalized_catastrophic_variants() {
    for command in ["rm -fr /", "rm --recursive --force /", "bash -c 'rm -fr /'"] {
        let error = review_shell(ExecSurface::ShellTool, command, None, true)
            .expect_err("the default Safe critical floor must block");
        assert!(error.contains("critical pattern"), "{command}: {error}");
        assert!(error.contains("Safe"), "{command}: {error}");
    }
}

#[test]
fn deny_policy_blocks_direct_programs() {
    let policy = ExecPolicy {
        mode: ExecSecurityMode::Deny,
        ..ExecPolicy::default()
    };

    let error = review_program(
        ExecSurface::CodeExecution,
        "python3",
        &["-c".to_string(), "print('ok')".to_string()],
        Some(&policy),
    )
    .unwrap_err();
    assert!(error.contains("mode = deny"));
}

#[test]
fn execution_permit_is_bound_to_surface_and_exact_content() {
    let permit = match review_shell(ExecSurface::ShellTool, "printf ok", None, true).unwrap() {
        ReviewDecision::Proceed(permit) => permit,
        ReviewDecision::ApprovalRequired { .. } => panic!("safe command requested approval"),
    };

    assert!(permit.authorizes(ExecSurface::ShellTool, "printf ok"));
    assert!(!permit.authorizes(ExecSurface::ShellTool, "printf changed"));
    assert!(!permit.authorizes(ExecSurface::GoalCheck, "printf ok"));
}

#[test]
fn direct_program_permit_covers_executable_and_every_argument() {
    let args = vec!["-c".to_string(), "print('ok')".to_string()];
    let permit = review_program(ExecSurface::CodeExecution, "python3", &args, None).unwrap();
    let exact = program_review_content("python3", &args);
    let changed = program_review_content(
        "python3",
        &["-c".to_string(), "print('changed')".to_string()],
    );

    assert!(permit.authorizes(ExecSurface::CodeExecution, &exact));
    assert!(permit.authorizes_program(ExecSurface::CodeExecution, "python3", &args));
    assert!(!permit.authorizes(ExecSurface::CodeExecution, &changed));
    assert!(!permit.authorizes(ExecSurface::WasmHost, &exact));
}

#[test]
fn direct_shell_program_blocks_a_critical_payload_before_spawn() {
    let error = review_program(
        ExecSurface::CodeExecution,
        "/bin/sh",
        &["-c".to_string(), "rm -fr /".to_string()],
        None,
    )
    .expect_err("critical shell payload must be blocked");

    assert!(error.contains("critical pattern `rm -rf /`"), "{error}");
    assert!(error.contains("Safe"), "{error}");
}

#[cfg(unix)]
#[tokio::test]
async fn scrubbed_environment_keeps_explicit_values_only() {
    let _guard = TEST_ASYNC_ENV_LOCK.lock().await;
    let inherited_key = "CAPTAIN_GUARDED_EXEC_INHERITED_SECRET";
    std::env::set_var(inherited_key, "must-not-leak");
    let explicit = vec![(
        "CAPTAIN_GUARDED_EXEC_EXPLICIT".to_string(),
        "visible".to_string(),
    )];

    let outcome = run_unattended_shell(ShellExecRequest {
        surface: ExecSurface::GoalCheck,
        command: "printf '%s|%s' \"${CAPTAIN_GUARDED_EXEC_INHERITED_SECRET-unset}\" \"$CAPTAIN_GUARDED_EXEC_EXPLICIT\"",
        policy: None,
        workspace: None,
        allowed_env_vars: &[],
        explicit_env: &explicit,
        timeout_secs: 5,
        no_output_timeout_secs: Some(0),
        bash_required: true,
    })
    .await
    .unwrap();
    std::env::remove_var(inherited_key);

    assert_eq!(outcome.stdout, "unset|visible");
}

#[tokio::test]
async fn invalid_environment_names_fail_before_spawn() {
    let explicit = vec![("INVALID=NAME".to_string(), "value".to_string())];
    let error = run_unattended_shell(ShellExecRequest {
        surface: ExecSurface::GoalCheck,
        command: "printf unreachable",
        policy: None,
        workspace: None,
        allowed_env_vars: &[],
        explicit_env: &explicit,
        timeout_secs: 5,
        no_output_timeout_secs: Some(0),
        bash_required: true,
    })
    .await
    .expect_err("invalid env name must fail closed");

    assert!(
        error.contains("invalid explicit environment name"),
        "{error}"
    );
}

#[cfg(unix)]
#[tokio::test]
async fn closed_output_streams_still_reach_the_no_output_deadline() {
    let outcome = tokio::time::timeout(
        Duration::from_secs(3),
        run_unattended_shell(ShellExecRequest {
            surface: ExecSurface::GoalCheck,
            command: "exec 1>&- 2>&-; sleep 2",
            policy: None,
            workspace: None,
            allowed_env_vars: &[],
            explicit_env: &[],
            timeout_secs: 3,
            no_output_timeout_secs: Some(1),
            bash_required: true,
        }),
    )
    .await
    .expect("closed streams must not spin forever")
    .expect_err("silent child should hit no-output timeout");

    assert!(outcome.contains("no output for 1s"), "{outcome}");
}

#[cfg(unix)]
#[test]
fn bash_syntax_check_ignores_inherited_bash_env() {
    let _guard = TEST_SYNC_ENV_LOCK.lock().unwrap();
    let dir = tempfile::tempdir().unwrap();
    let poison = dir.path().join("poison.sh");
    std::fs::write(&poison, "if then invalid startup syntax").unwrap();
    std::env::set_var("BASH_ENV", &poison);

    let result = check_bash_syntax(ExecSurface::SkillCheck, "printf ok\n", None);
    std::env::remove_var("BASH_ENV");

    assert!(
        result.is_ok(),
        "BASH_ENV must not reach bash -n: {result:?}"
    );
}

#[test]
fn output_capture_is_bounded_by_policy() {
    let captured = CapturedPipe {
        bytes: b"abc".to_vec(),
        total_bytes: 12,
    }
    .render();

    assert!(captured.starts_with("abc"));
    assert!(captured.contains("12 total bytes"));
}
