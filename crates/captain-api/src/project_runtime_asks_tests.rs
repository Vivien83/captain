use super::*;

fn sample_ask<'a>(options: Option<&'a [String]>) -> RuntimeProjectAsk<'a> {
    RuntimeProjectAsk {
        ask_id: "abcdef12-3456",
        run_id: "run-1",
        phase: "build",
        worker_id: "worker-build",
        agent_id: "agent-1",
        worker_role: "builder",
        question: "Pick the implementation path.",
        options,
    }
}

#[test]
fn runtime_project_ask_records_pending_and_answer() {
    let options = vec!["Simple".to_string(), "Complex".to_string()];
    let mut runtime = serde_json::json!({});
    record_runtime_project_ask(&mut runtime, sample_ask(Some(&options)));

    assert_eq!(runtime["user_questions"][0]["status"], "pending");
    let answer =
        mark_runtime_project_ask_answered(&mut runtime, "abcdef12", "2", "delivered").unwrap();
    assert_eq!(answer.answer, "Complex");
    assert!(answer.was_pending);
    assert_eq!(runtime["user_questions"][0]["status"], "answered");
    assert_eq!(runtime["user_questions"][0]["delivery"], "delivered");
}

#[test]
fn runtime_project_ask_invalid_callback_option_leaves_question_pending() {
    let options = vec!["Simple".to_string(), "Complex".to_string()];
    let mut runtime = serde_json::json!({});
    record_runtime_project_ask(&mut runtime, sample_ask(Some(&options)));

    let err = mark_runtime_project_ask_answered(&mut runtime, "abcdef12", "@idx:9", "delivered")
        .unwrap_err();

    assert!(err.contains("Choix projet"));
    assert_eq!(runtime["user_questions"][0]["status"], "pending");
    assert!(runtime["user_questions"][0].get("answer").is_none());
    assert_eq!(runtime["user_questions"][0]["delivery"], "waiting_for_user");
}

#[test]
fn runtime_user_questions_context_warns_on_current_pending() {
    let mut runtime = serde_json::json!({});
    record_runtime_project_ask(&mut runtime, sample_ask(None));
    let context = runtime_user_questions_context(&runtime, "build");
    assert!(context.contains("Current phase has a pending user question"));
    assert!(context.contains("[abcdef12] build/pending"));
}

#[test]
fn close_runtime_project_asks_closes_only_pending_same_run() {
    let mut runtime = serde_json::json!({});
    record_runtime_project_ask(&mut runtime, sample_ask(None));
    close_runtime_project_asks_for_run(&mut runtime, "other-run");
    assert_eq!(runtime["user_questions"][0]["status"], "pending");

    close_runtime_project_asks_for_run(&mut runtime, "run-1");
    assert_eq!(runtime["user_questions"][0]["status"], "closed");
}

#[test]
fn runtime_project_ask_rejects_duplicate_answer() {
    let mut runtime = serde_json::json!({});
    record_runtime_project_ask(&mut runtime, sample_ask(None));
    mark_runtime_project_ask_answered(&mut runtime, "abcdef12", "first", "delivered").unwrap();

    let err = mark_runtime_project_ask_answered(&mut runtime, "abcdef12", "second", "delivered")
        .unwrap_err();
    assert!(err.contains("already answered"));
    assert_eq!(runtime["user_questions"][0]["answer"], "first");
}
