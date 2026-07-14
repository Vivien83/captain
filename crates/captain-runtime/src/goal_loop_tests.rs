use super::*;
use std::sync::Mutex;

struct StubOps {
    recorded: Mutex<Vec<(bool, String)>>,
    sent: Mutex<Vec<(String, String, String)>>,
    next_consecutive_fails: Mutex<u32>,
    escalated: Mutex<bool>,
}

impl StubOps {
    fn new() -> Arc<Self> {
        Arc::new(Self {
            recorded: Mutex::new(Vec::new()),
            sent: Mutex::new(Vec::new()),
            next_consecutive_fails: Mutex::new(0),
            escalated: Mutex::new(false),
        })
    }
}

#[async_trait]
impl GoalLoopOps for StubOps {
    fn goal_list(&self) -> Result<String, String> {
        Ok("[]".into())
    }

    fn goal_status(&self, _id: &str) -> Result<String, String> {
        Ok("{}".into())
    }

    fn goal_record_check(
        &self,
        _id: &str,
        ok: bool,
        output: &str,
        _latency_ms: u64,
    ) -> Result<u32, String> {
        let n = if ok {
            let mut g = self.next_consecutive_fails.lock().unwrap();
            *g = 0;
            0
        } else {
            let mut g = self.next_consecutive_fails.lock().unwrap();
            *g += 1;
            *g
        };
        self.recorded.lock().unwrap().push((ok, output.to_string()));
        Ok(n)
    }

    fn goal_mark_escalated(&self, _id: &str) -> Result<bool, String> {
        *self.escalated.lock().unwrap() = true;
        Ok(true)
    }

    async fn send_channel_message(
        &self,
        channel: &str,
        recipient: &str,
        message: &str,
    ) -> Result<String, String> {
        self.sent.lock().unwrap().push((
            channel.to_string(),
            recipient.to_string(),
            message.to_string(),
        ));
        Ok("delivered".into())
    }
}

#[tokio::test]
async fn tick_once_records_success_and_does_not_escalate() {
    let ops = StubOps::new();
    let dyn_ops: Arc<dyn GoalLoopOps> = ops.clone();
    tick_once("g1", "goal-one", "true", None, 3, None, None, None, dyn_ops).await;
    assert_eq!(ops.recorded.lock().unwrap().len(), 1);
    assert!(ops.recorded.lock().unwrap()[0].0);
    assert!(!*ops.escalated.lock().unwrap());
    assert!(ops.sent.lock().unwrap().is_empty());
}

#[tokio::test]
async fn tick_once_escalates_after_threshold() {
    let ops = StubOps::new();
    let dyn_ops: Arc<dyn GoalLoopOps> = ops.clone();
    let ec = serde_json::json!({"channel": "telegram", "recipient": "1234"});
    for _ in 0..3 {
        tick_once(
            "g1",
            "goal-one",
            "false",
            None,
            3,
            Some(&ec),
            None,
            None,
            dyn_ops.clone(),
        )
        .await;
    }
    assert!(*ops.escalated.lock().unwrap());
    let sent = ops.sent.lock().unwrap();
    assert_eq!(sent.len(), 1);
    assert_eq!(sent[0].0, "telegram");
    assert_eq!(sent[0].1, "1234");
    assert!(sent[0].2.contains("goal-one"));
    assert!(sent[0].2.contains("3 consecutive"));
}

#[tokio::test]
async fn tick_once_recovery_attempt_counts_as_one_fail() {
    let ops = StubOps::new();
    let dyn_ops: Arc<dyn GoalLoopOps> = ops.clone();
    tick_once(
        "g1",
        "goal-one",
        "false",
        Some("false"),
        3,
        None,
        None,
        None,
        dyn_ops,
    )
    .await;
    assert_eq!(ops.recorded.lock().unwrap().len(), 1);
    let (ok, output) = ops.recorded.lock().unwrap()[0].clone();
    assert!(!ok);
    assert!(output.contains("[recovery"));
    assert!(output.contains("[recheck"));
    assert_eq!(*ops.next_consecutive_fails.lock().unwrap(), 1);
}

#[tokio::test]
async fn tick_once_no_escalation_channel_still_marks_escalated() {
    let ops = StubOps::new();
    let dyn_ops: Arc<dyn GoalLoopOps> = ops.clone();
    for _ in 0..2 {
        tick_once(
            "g1",
            "goal-one",
            "false",
            None,
            2,
            None,
            None,
            None,
            dyn_ops.clone(),
        )
        .await;
    }
    assert!(*ops.escalated.lock().unwrap());
    assert!(ops.sent.lock().unwrap().is_empty());
}

#[tokio::test]
async fn tick_once_escalates_when_progress_marker_stalls() {
    let ops = StubOps::new();
    let dyn_ops: Arc<dyn GoalLoopOps> = ops.clone();
    let ec = serde_json::json!({"channel": "telegram", "recipient": "1234"});

    for _ in 0..2 {
        tick_once(
            "g1",
            "goal-one",
            "printf 'CAPTAIN_PROGRESS=build-step-1\\n'",
            None,
            2,
            Some(&ec),
            None,
            Some("build-step-1"),
            dyn_ops.clone(),
        )
        .await;
    }

    assert!(*ops.escalated.lock().unwrap());
    let recorded = ops.recorded.lock().unwrap();
    assert_eq!(recorded.len(), 2);
    assert!(!recorded[0].0);
    assert!(recorded[0].1.contains("[Captain non-progress]"));
    assert!(recorded[0].1.contains("build-step-1"));
    let sent = ops.sent.lock().unwrap();
    assert_eq!(sent.len(), 1);
    assert!(sent[0].2.contains("no progress detected"));
    assert!(sent[0].2.contains("build-step-1"));
}

#[tokio::test]
async fn tick_once_recovery_succeeds_records_one_ok() {
    let ops = StubOps::new();
    let dyn_ops: Arc<dyn GoalLoopOps> = ops.clone();
    let dir = tempfile::TempDir::new().unwrap();
    let probe = dir.path().join("probe");
    let check = format!(
        "test -f {} && exit 0 || (touch {} && exit 1)",
        probe.display(),
        probe.display()
    );
    tick_once(
        "g1",
        "goal-one",
        &check,
        Some("true"),
        3,
        None,
        None,
        None,
        dyn_ops,
    )
    .await;
    let recorded = ops.recorded.lock().unwrap();
    assert_eq!(recorded.len(), 1);
    assert!(recorded[0].0, "recheck after recovery should be OK");
    assert_eq!(*ops.next_consecutive_fails.lock().unwrap(), 0);
}

#[tokio::test]
async fn run_shell_with_timeout_captures_exit_code() {
    let (ok, _out, _lat) = run_shell_with_timeout("true", None).await;
    assert!(ok);
    let (ok, _, _) = run_shell_with_timeout("false", None).await;
    assert!(!ok);
}

#[tokio::test]
async fn run_shell_with_timeout_captures_stderr() {
    let (ok, out, _) = run_shell_with_timeout("echo hello && echo bad >&2 && exit 2", None).await;
    assert!(!ok);
    assert!(out.contains("hello"));
    assert!(out.contains("bad"));
}

#[tokio::test]
async fn tick_once_runs_relative_checks_in_project_workspace() {
    let ops = StubOps::new();
    let dyn_ops: Arc<dyn GoalLoopOps> = ops.clone();
    let dir = tempfile::TempDir::new().unwrap();
    std::fs::write(dir.path().join("marker.txt"), "ok").unwrap();

    tick_once(
        "g1",
        "goal-one",
        "test -f marker.txt",
        None,
        3,
        None,
        Some(dir.path().to_str().unwrap()),
        None,
        dyn_ops,
    )
    .await;

    let recorded = ops.recorded.lock().unwrap();
    assert_eq!(recorded.len(), 1);
    assert!(recorded[0].0);
}

#[test]
fn project_workspace_path_reads_launch_workspace_path() {
    let project = serde_json::json!({
        "metadata": {
            "launch": {
                "workspace": { "path": "/tmp/captain-project" }
            }
        }
    });
    assert_eq!(
        project_workspace_path(&project).as_deref(),
        Some("/tmp/captain-project")
    );
}

#[test]
fn progress_signature_reads_line_and_json_markers() {
    assert_eq!(
        progress_signature("noise\nCAPTAIN_PROGRESS=phase-2\n"),
        Some("phase-2".to_string())
    );
    assert_eq!(
        progress_signature(r#"{"captain_progress":"phase-3"}"#),
        Some("phase-3".to_string())
    );
    assert_eq!(
        progress_signature(r#"{"progress":"generic-health-ok"}"#),
        None
    );
}

#[test]
fn latest_progress_signature_reads_recent_checks_from_newest() {
    let goal = serde_json::json!({
        "recent_checks": [
            {"output": "CAPTAIN_PROGRESS=old"},
            {"output": "CAPTAIN_PROGRESS=new"}
        ]
    });

    assert_eq!(latest_progress_signature(&goal), Some("new".to_string()));
}

#[test]
fn truncate_keeps_short_strings() {
    assert_eq!(truncate("abc", 10), "abc");
    assert_eq!(truncate("abcdefghij", 5), "abcde…");
}
