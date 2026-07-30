use super::*;
use captain_kernel::goals::Suggestion;
use chrono::Utc;
use std::collections::VecDeque;

fn tool(name: &str, is_error: bool) -> ToolCallRecord {
    ToolCallRecord {
        tool_name: name.to_string(),
        reason: "Verify the project outcome.".to_string(),
        is_error,
        duration_ms: 12,
        input_summary: "cargo test".to_string(),
        output_summary: if is_error { "failed" } else { "ok" }.to_string(),
    }
}

#[test]
fn verify_phase_rejects_a_completion_claim_without_runtime_evidence() {
    let contract = phase_completion_contract(
        "verify",
        "STATUS: complete\nSUMMARY: Looks good.\nVERIFY: cargo test passed",
        &[],
        false,
        &[],
    );

    assert_eq!(contract.decision, "insufficient_evidence");
    assert!(contract
        .blocking_reason
        .as_deref()
        .unwrap_or_default()
        .contains("phase_execution_receipt"));
}

#[test]
fn verify_phase_accepts_a_structured_handoff_with_real_tool_receipt() {
    let contract = phase_completion_contract(
        "verify",
        "STATUS: complete\nSUMMARY: Tests passed.\nVERIFY: cargo test completed with exit 0\nNEXT: none",
        &[tool("shell_exec", false)],
        false,
        &[],
    );

    assert!(contract.is_satisfied());
    assert_eq!(contract.evidence_count, 1);
    assert_eq!(contract.passed_evidence_count, 1);
    let encoded = serde_json::to_string(&contract).unwrap();
    assert!(!encoded.contains("cargo test"));
    assert!(!encoded.contains("output_summary"));
}

#[test]
fn failed_goal_check_blocks_even_when_a_tool_receipt_passed() {
    let receipt = ProjectGoalCheckReceipt {
        goal_id: "health".to_string(),
        status: "failed",
        latency_ms: 10,
        recovery_attempted: false,
        recorded: true,
        receipt_id: "receipt".to_string(),
        checked_at: "2026-07-22T00:00:00Z".to_string(),
    };
    let contract = phase_completion_contract(
        "verify",
        "STATUS: complete\nSUMMARY: Verified.\nVERIFY: smoke passed",
        &[tool("shell_exec", false)],
        false,
        &[receipt],
    );

    assert_eq!(contract.decision, "insufficient_evidence");
    assert!(contract
        .requirements
        .iter()
        .any(
            |requirement| requirement.id == "project_goal_checks" && requirement.status == "failed"
        ));
}

#[test]
fn failed_verification_receipt_blocks_even_when_another_receipt_passed() {
    let contract = phase_completion_contract(
        "verify",
        "STATUS: complete\nSUMMARY: Verified.\nVERIFY: smoke passed",
        &[tool("shell_exec", true), tool("git_status", false)],
        false,
        &[],
    );

    assert_eq!(contract.decision, "insufficient_evidence");
    assert!(contract.requirements.iter().any(|requirement| {
        requirement.id == "verification_receipts_clean" && requirement.status == "failed"
    }));
}

#[test]
fn evidence_outside_the_durable_bound_cannot_satisfy_completion() {
    let mut receipts = (0..24)
        .map(|_| tool("shell_exec", true))
        .collect::<Vec<_>>();
    receipts.push(tool("shell_exec", false));
    let contract = phase_completion_contract(
        "verify",
        "STATUS: complete\nSUMMARY: Verified.\nVERIFY: final check passed",
        &receipts,
        false,
        &[],
    );

    assert_eq!(contract.evidence_count, 24);
    assert_eq!(contract.passed_evidence_count, 0);
    assert_eq!(contract.decision, "insufficient_evidence");
}

#[test]
fn aggregate_requires_every_project_phase_contract() {
    let workers = PROJECT_PHASES
        .iter()
        .map(|phase| {
            json!({
                "phase": phase,
                "completion_contract": {
                    "decision": if *phase == "learn" { "missing" } else { "satisfied" },
                    "evidence_count": 1,
                }
            })
        })
        .collect::<Vec<_>>();
    let contract = aggregate_project_completion_contract(&json!({ "workers": workers }));

    assert!(!aggregate_contract_is_satisfied(&contract));
    assert_eq!(contract["decision"], "insufficient_evidence");
    assert_eq!(contract["evidence_count"], 7);
}

#[test]
fn completed_worker_from_old_runtime_is_not_proven_without_contract() {
    let runtime = json!({
        "workers": [{ "phase": "build", "status": "done" }]
    });

    assert!(!runtime_phase_has_satisfied_contract(&runtime, "build"));
}

#[tokio::test]
async fn project_goal_check_receipt_is_executed_and_persisted() {
    let dir = tempfile::TempDir::new().unwrap();
    std::fs::write(dir.path().join("ready"), "ok").unwrap();
    let store = GoalStore::new(dir.path());
    let goal = Goal {
        id: "verify-ready".to_string(),
        name: "Ready marker".to_string(),
        description: "Verify a local marker".to_string(),
        project_id: Some("project-1".to_string()),
        project_slug: Some("demo".to_string()),
        status: GoalStatus::Active,
        interval_secs: 60,
        check_command: "test -f ready".to_string(),
        recovery_command: None,
        escalation_threshold: 3,
        max_llm_calls_per_hour: 10,
        escalation_channel: None,
        created_at: Utc::now(),
        updated_at: Utc::now(),
        last_check_ts: None,
        consecutive_fails: 0,
        escalated_at: None,
        recent_checks: VecDeque::new(),
        llm_call_log: Vec::new(),
        suggestions: Vec::<Suggestion>::new(),
    };
    store.add(goal.clone()).unwrap();

    let receipt =
        execute_and_record_project_goal_check(&goal, Some(dir.path().to_str().unwrap()), &store)
            .await;

    assert!(receipt.passed());
    assert_eq!(receipt.goal_id, "verify-ready");
    assert_eq!(receipt.receipt_id.len(), 64);
    let persisted = store.get("verify-ready").unwrap();
    assert_eq!(persisted.recent_checks.len(), 1);
    assert!(persisted.recent_checks[0].ok);
}

#[tokio::test]
async fn project_goal_check_rejects_a_stalled_progress_marker() {
    let dir = tempfile::TempDir::new().unwrap();
    let store = GoalStore::new(dir.path());
    let mut recent_checks = VecDeque::new();
    recent_checks.push_back(CheckResult::new(
        true,
        "CAPTAIN_PROGRESS=same".to_string(),
        1,
    ));
    let goal = Goal {
        id: "verify-progress".to_string(),
        name: "Progress marker".to_string(),
        description: "Reject a stalled project check".to_string(),
        project_id: Some("project-1".to_string()),
        project_slug: Some("demo".to_string()),
        status: GoalStatus::Active,
        interval_secs: 60,
        check_command: "printf 'CAPTAIN_PROGRESS=same\\n'".to_string(),
        recovery_command: None,
        escalation_threshold: 3,
        max_llm_calls_per_hour: 10,
        escalation_channel: None,
        created_at: Utc::now(),
        updated_at: Utc::now(),
        last_check_ts: None,
        consecutive_fails: 0,
        escalated_at: None,
        recent_checks,
        llm_call_log: Vec::new(),
        suggestions: Vec::<Suggestion>::new(),
    };
    store.add(goal.clone()).unwrap();

    let receipt =
        execute_and_record_project_goal_check(&goal, Some(dir.path().to_str().unwrap()), &store)
            .await;

    assert!(!receipt.passed());
    let persisted = store.get("verify-progress").unwrap();
    assert_eq!(persisted.recent_checks.len(), 2);
    assert!(!persisted.recent_checks[1].ok);
    assert!(persisted.recent_checks[1]
        .output
        .contains("[Captain non-progress]"));
}
