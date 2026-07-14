use super::*;

#[test]
fn process_command_display_joins_command_and_args() {
    assert_eq!(process_command_display("cat", &[]), "cat");
    assert_eq!(
        process_command_display("bash", &["-lc".to_string(), "echo ok".to_string()]),
        "bash -lc echo ok"
    );
}

#[test]
fn push_process_output_line_caps_buffer_in_chunks() {
    let mut buffer = (0..1000)
        .map(|index| format!("line-{index}"))
        .collect::<Vec<_>>();

    push_process_output_line(&mut buffer, "line-1000".to_string());

    assert_eq!(buffer.len(), 901);
    assert_eq!(buffer.first().unwrap(), "line-100");
    assert_eq!(buffer.last().unwrap(), "line-1000");
}

#[tokio::test]
async fn process_start_strips_inherited_secret() {
    std::env::set_var("CAPTAIN_TEST_SECRET_B2", "topsecret_42");
    let pm = ProcessManager::new(5);
    let id = pm
        .start(
            "agent1",
            "bash",
            &[
                "-c".to_string(),
                "echo MARKER=$CAPTAIN_TEST_SECRET_B2".to_string(),
            ],
        )
        .await
        .expect("start ok");
    tokio::time::sleep(std::time::Duration::from_millis(200)).await;
    let (stdout, _stderr) = pm.read(&id).await.expect("read ok");
    let joined = stdout.join("\n");
    assert!(
        !joined.contains("topsecret_42"),
        "process_start must not leak ambient secrets - got stdout: {joined}"
    );
    let _ = pm.kill(&id).await;
    std::env::remove_var("CAPTAIN_TEST_SECRET_B2");
}

#[tokio::test]
async fn process_start_preserves_path() {
    let pm = ProcessManager::new(5);
    let id = pm
        .start(
            "agent2",
            "bash",
            &[
                "-c".to_string(),
                "if [ -n \"$PATH\" ]; then echo PATH_OK; else echo PATH_MISSING; fi".to_string(),
            ],
        )
        .await
        .expect("start ok");
    tokio::time::sleep(std::time::Duration::from_millis(200)).await;
    let (stdout, _stderr) = pm.read(&id).await.expect("read ok");
    let joined = stdout.join("\n");
    assert!(
        joined.contains("PATH_OK"),
        "PATH must propagate through whitelist - got stdout: {joined}"
    );
    let _ = pm.kill(&id).await;
}

#[tokio::test]
async fn process_start_can_run_from_project_cwd() {
    let tmp = tempfile::tempdir().unwrap();
    std::fs::write(tmp.path().join("marker.txt"), "cwd-ok\n").unwrap();
    let pm = ProcessManager::new(5);
    let id = pm
        .start_in_dir(
            "agent-cwd",
            "bash",
            &["-c".to_string(), "cat marker.txt".to_string()],
            Some(tmp.path()),
        )
        .await
        .expect("start ok");
    tokio::time::sleep(std::time::Duration::from_millis(200)).await;
    let (stdout, _stderr) = pm.read(&id).await.expect("read ok");
    let joined = stdout.join("\n");
    assert!(
        joined.contains("cwd-ok"),
        "process_start should honor cwd - got stdout: {joined}"
    );
    let _ = pm.kill(&id).await;
}

#[tokio::test]
async fn test_start_and_list() {
    let pm = ProcessManager::new(5);

    let cmd = if cfg!(windows) { "cmd" } else { "cat" };
    let args: Vec<String> = if cfg!(windows) {
        vec!["/C".to_string(), "echo".to_string(), "hello".to_string()]
    } else {
        vec![]
    };

    let id = pm.start("agent1", cmd, &args).await.unwrap();
    assert!(id.starts_with("proc_"));

    let list = pm.list("agent1");
    assert_eq!(list.len(), 1);
    assert_eq!(list[0].agent_id, "agent1");
    assert!(list[0].attached);
    assert!(list[0].pid.is_some() || cfg!(windows));
    assert!(list[0].idle_secs <= list[0].uptime_secs);

    let _ = pm.kill(&id).await;
}

#[tokio::test]
async fn test_per_agent_limit() {
    let pm = ProcessManager::new(1);

    let cmd = if cfg!(windows) { "cmd" } else { "cat" };
    let args: Vec<String> = if cfg!(windows) {
        vec![
            "/C".to_string(),
            "timeout".to_string(),
            "/t".to_string(),
            "10".to_string(),
        ]
    } else {
        vec![]
    };

    let id1 = pm.start("agent1", cmd, &args).await.unwrap();
    let result = pm.start("agent1", cmd, &args).await;
    assert!(result.is_err());
    assert!(result.unwrap_err().contains("max: 1"));

    let _ = pm.kill(&id1).await;
}

#[tokio::test]
async fn test_kill_nonexistent() {
    let pm = ProcessManager::new(5);
    let result = pm.kill("nonexistent").await;
    assert!(result.is_err());
}

#[tokio::test]
async fn test_read_nonexistent() {
    let pm = ProcessManager::new(5);
    let result = pm.read("nonexistent").await;
    assert!(result.is_err());
}

#[tokio::test]
async fn cleanup_never_kills_live_long_runner() {
    let pm = ProcessManager::new(5);
    let id = pm.start("agent1", "cat", &[]).await.unwrap();
    {
        let mut entry = pm.processes.get_mut(&id).unwrap();
        entry.started_at = Instant::now() - std::time::Duration::from_secs(10);
    }

    pm.cleanup(1).await;

    assert!(
        pm.processes.contains_key(&id),
        "live process must not be killed by age-only cleanup"
    );
    let _ = pm.kill(&id).await;
}

#[tokio::test]
async fn cleanup_reaps_old_exited_process() {
    let pm = ProcessManager::new(5);
    let id = pm
        .start("agent1", "bash", &["-c".to_string(), "exit 0".to_string()])
        .await
        .unwrap();
    tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    {
        let mut entry = pm.processes.get_mut(&id).unwrap();
        entry.started_at = Instant::now() - std::time::Duration::from_secs(10);
    }

    pm.cleanup(1).await;

    assert!(
        !pm.processes.contains_key(&id),
        "old exited process handles should be reaped"
    );
}

#[test]
fn test_default_process_manager() {
    let pm = ProcessManager::default();
    assert_eq!(pm.max_per_agent, 5);
    assert_eq!(pm.count(), 0);
}

#[tokio::test]
async fn registry_persists_started_process_metadata() {
    let tmp = tempfile::tempdir().unwrap();
    let registry_path = tmp.path().join("process_registry.json");
    let pm = ProcessManager::with_registry_path(5, registry_path.clone());
    let id = pm.start("agent1", "cat", &[]).await.unwrap();

    let records = crate::process_registry::ProcessRegistryStore::new(&registry_path).load_records();

    assert_eq!(records.len(), 1);
    assert_eq!(records[0].id, id);
    assert_eq!(records[0].agent_id, "agent1");
    assert!(records[0].pid.is_some());

    let _ = pm.kill(&id).await;
}

#[cfg(unix)]
#[tokio::test]
async fn registry_recovers_live_process_as_detached() {
    let tmp = tempfile::tempdir().unwrap();
    let registry_path = tmp.path().join("process_registry.json");
    let id = {
        let pm = ProcessManager::with_registry_path(5, registry_path.clone());
        pm.start("agent1", "sleep", &["30".to_string()])
            .await
            .unwrap()
    };

    let pm = ProcessManager::with_registry_path(5, registry_path.clone());
    let list = pm.list_all();
    let process = list.iter().find(|process| process.id == id).unwrap();

    assert!(process.alive);
    assert!(!process.attached);
    assert!(process.pid.is_some());
    assert!(pm.read(&id).await.unwrap_err().contains("not attached"));

    pm.kill(&id).await.unwrap();
}

#[test]
fn registry_skips_dead_recovered_pid() {
    let tmp = tempfile::tempdir().unwrap();
    let registry_path = tmp.path().join("process_registry.json");
    let store = crate::process_registry::ProcessRegistryStore::new(&registry_path);
    store
        .save_records(&[crate::process_registry::ProcessRegistryRecord {
            id: "proc_9".to_string(),
            agent_id: "agent1".to_string(),
            command: "sleep 999".to_string(),
            pid: Some(999_999_999),
            started_at_unix_secs: 1,
            last_activity_unix_secs: 1,
        }])
        .unwrap();

    let pm = ProcessManager::with_registry_path(5, registry_path.clone());

    assert!(pm.list_all().is_empty());
    assert!(store.load_records().is_empty());
}

#[tokio::test]
async fn process_start_quota_error_points_to_recovery_options() {
    let pm = ProcessManager::new(1);
    let id = pm
        .start("agent-quota-test", "sleep", &["5".to_string()])
        .await
        .expect("first process should start under the cap");

    let err = pm
        .start("agent-quota-test", "sleep", &["5".to_string()])
        .await
        .expect_err("second process should be rejected once the per-agent cap is hit");

    assert!(err.contains("process_list"), "got: {err}");
    assert!(err.contains("process_kill"), "got: {err}");
    assert!(err.contains("tool_run_start"), "got: {err}");

    let _ = pm.kill(&id).await;
}
