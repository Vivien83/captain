use captain_memory::session::Session;
use captain_memory::MemorySubstrate;
use captain_types::agent::{AgentId, SessionId};
use captain_types::memory::{Memory, MemoryFilter, MemorySource};
use captain_types::message::Message;
use std::collections::HashMap;
use std::io::{BufRead, BufReader, Write};
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::time::Duration;

const CHILD_DB_ENV: &str = "CAPTAIN_CRASH_TEST_DB";
const CHILD_STATE_ENV: &str = "CAPTAIN_CRASH_TEST_STATE";
const CHILD_AGENT_ENV: &str = "CAPTAIN_CRASH_TEST_AGENT";
const CHILD_SESSION_ENV: &str = "CAPTAIN_CRASH_TEST_SESSION";
const COMMITTED_MARKER: &str = "CAPTAIN_DURABILITY_COMMITTED";

#[test]
#[ignore = "helper process for committed_state_survives_sigkill"]
fn crash_writer_child() {
    let db_path = required_path(CHILD_DB_ENV);
    let state_path = required_path(CHILD_STATE_ENV);
    let agent_id = AgentId(parse_uuid(CHILD_AGENT_ENV));
    let session_id = SessionId(parse_uuid(CHILD_SESSION_ENV));
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let substrate = MemorySubstrate::open(&db_path, 0.05).unwrap();

    runtime.block_on(async {
        substrate
            .set(
                agent_id,
                "power_loss_preference",
                serde_json::json!("remember committed state"),
            )
            .await
            .unwrap();
        substrate
            .remember(
                agent_id,
                "Vivien expects Captain to survive abrupt power loss",
                MemorySource::Conversation,
                "long_term",
                HashMap::new(),
            )
            .await
            .unwrap();
    });

    substrate
        .save_session(&Session {
            id: session_id,
            agent_id,
            messages: vec![
                Message::user("Retain this session after a crash"),
                Message::assistant("The committed session is durable"),
            ],
            context_window_tokens: 272_000,
            label: Some("Crash recovery proof".to_string()),
        })
        .unwrap();
    substrate
        .append_session_event(
            &session_id.to_string(),
            "durability.committed",
            &serde_json::json!({"status": "ready_for_sigkill"}),
        )
        .unwrap();
    captain_types::durable_fs::atomic_write(
        &state_path,
        br#"{"status":"committed","generation":1}"#,
    )
    .unwrap();

    println!("{COMMITTED_MARKER}");
    std::io::stdout().flush().unwrap();
    std::thread::sleep(Duration::from_secs(120));
}

#[test]
fn committed_state_survives_sigkill() {
    let root = tempfile::tempdir().unwrap();
    let db_path = root.path().join("data/captain.db");
    let state_path = root.path().join("state/runtime.json");
    std::fs::create_dir_all(db_path.parent().unwrap()).unwrap();
    let agent_id = AgentId::new();
    let session_id = SessionId::new();
    let mut child = Command::new(std::env::current_exe().unwrap())
        .args(["--ignored", "--exact", "crash_writer_child", "--nocapture"])
        .env(CHILD_DB_ENV, &db_path)
        .env(CHILD_STATE_ENV, &state_path)
        .env(CHILD_AGENT_ENV, agent_id.to_string())
        .env(CHILD_SESSION_ENV, session_id.to_string())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();

    let stdout = child.stdout.take().unwrap();
    let (marker_tx, marker_rx) = mpsc::channel();
    std::thread::spawn(move || {
        for line in BufReader::new(stdout).lines().map_while(Result::ok) {
            if line.contains(COMMITTED_MARKER) {
                let _ = marker_tx.send(());
                break;
            }
        }
    });

    if marker_rx.recv_timeout(Duration::from_secs(30)).is_err() {
        let _ = child.kill();
        let stderr = child
            .stderr
            .take()
            .map(|stream| {
                BufReader::new(stream)
                    .lines()
                    .map_while(Result::ok)
                    .collect::<Vec<_>>()
                    .join("\n")
            })
            .unwrap_or_default();
        let _ = child.wait();
        panic!("crash writer never confirmed its commit: {stderr}");
    }

    child.kill().unwrap();
    let status = child.wait().unwrap();
    assert!(!status.success(), "child must be terminated abruptly");

    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let substrate = MemorySubstrate::open(&db_path, 0.05).unwrap();
    let value = runtime
        .block_on(substrate.get(agent_id, "power_loss_preference"))
        .unwrap();
    assert_eq!(value, Some(serde_json::json!("remember committed state")));

    let recalled = runtime
        .block_on(substrate.recall("abrupt power loss", 10, Some(MemoryFilter::agent(agent_id))))
        .unwrap();
    assert_eq!(recalled.len(), 1);
    assert!(recalled[0].content.contains("survive abrupt power loss"));

    let session = substrate.get_session(session_id).unwrap().unwrap();
    assert_eq!(session.label.as_deref(), Some("Crash recovery proof"));
    assert_eq!(session.messages.len(), 2);
    assert_eq!(
        substrate
            .count_session_events(&session_id.to_string())
            .unwrap(),
        1
    );
    assert_eq!(
        std::fs::read_to_string(&state_path).unwrap(),
        r#"{"status":"committed","generation":1}"#
    );

    let connection = substrate.usage_conn();
    let connection = connection.lock().unwrap();
    let integrity: String = connection
        .query_row("PRAGMA integrity_check", [], |row| row.get(0))
        .unwrap();
    assert_eq!(integrity, "ok");
}

fn required_path(name: &str) -> PathBuf {
    std::env::var_os(name)
        .map(PathBuf::from)
        .unwrap_or_else(|| panic!("missing {name}"))
}

fn parse_uuid(name: &str) -> uuid::Uuid {
    let value = std::env::var(name).unwrap_or_else(|_| panic!("missing {name}"));
    uuid::Uuid::parse_str(&value).unwrap_or_else(|_| panic!("invalid {name}"))
}
