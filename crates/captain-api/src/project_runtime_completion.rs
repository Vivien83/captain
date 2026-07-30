use crate::project_runtime_worker_support::project_workspace_path_for_runtime;
use crate::routes::AppState;
use captain_kernel::goals::{CheckResult, Goal, GoalStatus, GoalStore};
use captain_memory::project;
use captain_runtime::agent_loop::ToolCallRecord;
use captain_runtime::goal_loop::{execute_goal_check, goal_progress_signature};
use chrono::Utc;
use serde::Serialize;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

pub(crate) const COMPLETION_CONTRACT_PROTOCOL: &str = "captain.completion.v1";
const PROJECT_PHASES: &[&str] = &[
    "observe", "think", "plan", "build", "execute", "verify", "learn",
];
const MAX_PROJECT_COMPLETION_GOAL_CHECKS: usize = 12;

#[derive(Debug, Clone, Serialize)]
pub(crate) struct ProjectGoalCheckReceipt {
    goal_id: String,
    status: &'static str,
    latency_ms: u64,
    recovery_attempted: bool,
    recorded: bool,
    receipt_id: String,
    checked_at: String,
}

impl ProjectGoalCheckReceipt {
    fn passed(&self) -> bool {
        self.status == "passed" && self.recorded
    }
}

#[derive(Debug, Clone, Serialize)]
pub(crate) struct CompletionEvidence {
    id: String,
    kind: &'static str,
    source: String,
    status: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    duration_ms: Option<u64>,
}

#[derive(Debug, Clone, Serialize)]
pub(crate) struct CompletionRequirement {
    id: &'static str,
    status: &'static str,
    required: bool,
}

#[derive(Debug, Clone, Serialize)]
pub(crate) struct CompletionContract {
    pub protocol: &'static str,
    pub scope: &'static str,
    pub phase: String,
    pub decision: &'static str,
    pub claim: &'static str,
    pub evidence_count: usize,
    pub passed_evidence_count: usize,
    pub requirements: Vec<CompletionRequirement>,
    pub evidence: Vec<CompletionEvidence>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub blocking_reason: Option<String>,
    pub evaluated_at: String,
}

impl CompletionContract {
    pub(crate) fn is_satisfied(&self) -> bool {
        self.decision == "satisfied"
    }
}

pub(crate) async fn collect_project_goal_check_receipts(
    state: &AppState,
    project: &project::Project,
    phase: &str,
) -> Vec<ProjectGoalCheckReceipt> {
    if phase != "verify" {
        return Vec::new();
    }

    let workspace = project_workspace_path_for_runtime(state, project);
    let mut goals = state
        .kernel
        .goal_store
        .list_for_project(&project.id, &project.slug)
        .into_iter()
        .filter(|goal| goal.status == GoalStatus::Active)
        .collect::<Vec<_>>();
    goals.sort_by(|left, right| left.id.cmp(&right.id));
    let overflow = goals
        .len()
        .saturating_sub(MAX_PROJECT_COMPLETION_GOAL_CHECKS);
    let mut receipts = Vec::new();
    for goal in goals.into_iter().take(MAX_PROJECT_COMPLETION_GOAL_CHECKS) {
        let receipt = execute_and_record_project_goal_check(
            &goal,
            workspace.as_deref(),
            &state.kernel.goal_store,
        )
        .await;
        if !receipt.recorded {
            tracing::warn!(
                project_id = %project.id,
                goal_id = %goal.id,
                "project verification could not persist goal check receipt"
            );
        }
        receipts.push(receipt);
    }
    if overflow > 0 {
        receipts.push(ProjectGoalCheckReceipt {
            goal_id: "goal_check_limit".to_string(),
            status: "failed",
            latency_ms: 0,
            recovery_attempted: false,
            recorded: false,
            receipt_id: digest_receipt(&[phase, "goal_check_limit", &overflow.to_string()]),
            checked_at: Utc::now().to_rfc3339(),
        });
    }
    receipts
}

async fn execute_and_record_project_goal_check(
    goal: &Goal,
    workspace: Option<&str>,
    store: &GoalStore,
) -> ProjectGoalCheckReceipt {
    let previous_progress_signature = goal
        .recent_checks
        .iter()
        .rev()
        .find_map(|check| goal_progress_signature(&check.output));
    let execution = execute_goal_check(
        &goal.check_command,
        goal.recovery_command.as_deref(),
        workspace,
        previous_progress_signature.as_deref(),
    )
    .await;
    let mut check = CheckResult::new(execution.ok, execution.output.clone(), execution.latency_ms);
    check.recovery_attempted = execution.recovery_attempted;
    let checked_at = check.ts.to_rfc3339();
    let recorded = store.record_check(&goal.id, check).is_ok();
    let receipt_id = digest_receipt(&[
        "verify",
        &goal.id,
        if execution.ok { "passed" } else { "failed" },
        &execution.output,
    ]);
    ProjectGoalCheckReceipt {
        goal_id: goal.id.clone(),
        status: if execution.ok { "passed" } else { "failed" },
        latency_ms: execution.latency_ms,
        recovery_attempted: execution.recovery_attempted,
        recorded,
        receipt_id,
        checked_at,
    }
}

pub(crate) fn phase_completion_contract(
    phase: &str,
    summary: &str,
    tool_calls: &[ToolCallRecord],
    declared_blocked: bool,
    goal_checks: &[ProjectGoalCheckReceipt],
) -> CompletionContract {
    let handoff = parse_handoff(summary);
    let evidence = completion_evidence(phase, tool_calls, goal_checks);
    let passed_evidence_count = evidence
        .iter()
        .filter(|receipt| receipt.status == "passed")
        .count();
    let execution_required = requires_execution_receipt(phase);
    let persisted_tool_evidence = evidence
        .iter()
        .filter(|receipt| receipt.kind == "tool_receipt")
        .collect::<Vec<_>>();
    let has_execution_receipt = persisted_tool_evidence
        .iter()
        .any(|receipt| receipt.status == "passed");
    let has_passing_goal_check = goal_checks.iter().any(ProjectGoalCheckReceipt::passed);
    let all_goal_checks_pass = goal_checks.iter().all(ProjectGoalCheckReceipt::passed);
    let verify_summary_ok = handoff
        .verify
        .as_deref()
        .map(meaningful_verification_summary)
        .unwrap_or(false);
    let verification_tool_evidence = persisted_tool_evidence
        .iter()
        .filter(|receipt| tool_is_verification_evidence(&receipt.source))
        .collect::<Vec<_>>();
    let independent_verification = phase != "verify"
        || has_passing_goal_check
        || verification_tool_evidence
            .iter()
            .any(|receipt| receipt.status == "passed");
    let verification_receipts_clean = verification_tool_evidence
        .iter()
        .all(|receipt| receipt.status == "passed");

    let requirements = vec![
        CompletionRequirement {
            id: "structured_handoff",
            status: if handoff.status.is_some() && handoff.summary.is_some() {
                "passed"
            } else {
                "failed"
            },
            required: true,
        },
        CompletionRequirement {
            id: "verification_summary",
            status: if verify_summary_ok {
                "passed"
            } else {
                "failed"
            },
            required: true,
        },
        CompletionRequirement {
            id: "phase_execution_receipt",
            status: if !execution_required {
                "not_required"
            } else if has_execution_receipt {
                "passed"
            } else {
                "failed"
            },
            required: execution_required,
        },
        CompletionRequirement {
            id: "project_goal_checks",
            status: if phase != "verify" || goal_checks.is_empty() {
                "not_required"
            } else if all_goal_checks_pass {
                "passed"
            } else {
                "failed"
            },
            required: phase == "verify" && !goal_checks.is_empty(),
        },
        CompletionRequirement {
            id: "independent_verification",
            status: if phase != "verify" {
                "not_required"
            } else if independent_verification {
                "passed"
            } else {
                "failed"
            },
            required: phase == "verify",
        },
        CompletionRequirement {
            id: "verification_receipts_clean",
            status: if phase != "verify" {
                "not_required"
            } else if verification_receipts_clean {
                "passed"
            } else {
                "failed"
            },
            required: phase == "verify",
        },
    ];

    let missing = requirements
        .iter()
        .filter(|requirement| requirement.required && requirement.status != "passed")
        .map(|requirement| requirement.id)
        .collect::<Vec<_>>();
    let model_blocked = declared_blocked || handoff.status.as_deref() == Some("blocked");
    let (decision, blocking_reason) = if model_blocked {
        (
            "blocked",
            Some(
                "The worker declared a blocker; Captain preserved the phase for review."
                    .to_string(),
            ),
        )
    } else if handoff.status.as_deref() != Some("complete") || !missing.is_empty() {
        let detail = if missing.is_empty() {
            "declared completion status".to_string()
        } else {
            missing.join(", ")
        };
        (
            "insufficient_evidence",
            Some(format!(
                "Completion was rejected because required proof is missing: {detail}."
            )),
        )
    } else {
        ("satisfied", None)
    };

    CompletionContract {
        protocol: COMPLETION_CONTRACT_PROTOCOL,
        scope: "project_phase",
        phase: phase.to_string(),
        decision,
        claim: "phase_complete",
        evidence_count: evidence.len(),
        passed_evidence_count,
        requirements,
        evidence,
        blocking_reason,
        evaluated_at: Utc::now().to_rfc3339(),
    }
}

pub(crate) fn runtime_phase_has_satisfied_contract(runtime: &Value, phase: &str) -> bool {
    runtime
        .get("workers")
        .and_then(Value::as_array)
        .and_then(|workers| {
            workers
                .iter()
                .find(|worker| worker.get("phase").and_then(Value::as_str) == Some(phase))
        })
        .and_then(|worker| worker.pointer("/completion_contract/decision"))
        .and_then(Value::as_str)
        == Some("satisfied")
}

pub(crate) fn aggregate_project_completion_contract(runtime: &Value) -> Value {
    let phases = PROJECT_PHASES
        .iter()
        .map(|phase| {
            let worker = runtime
                .get("workers")
                .and_then(Value::as_array)
                .and_then(|workers| {
                    workers
                        .iter()
                        .find(|worker| worker.get("phase").and_then(Value::as_str) == Some(*phase))
                });
            let decision = worker
                .and_then(|worker| worker.pointer("/completion_contract/decision"))
                .and_then(Value::as_str)
                .unwrap_or("missing");
            let evidence_count = worker
                .and_then(|worker| worker.pointer("/completion_contract/evidence_count"))
                .and_then(Value::as_u64)
                .unwrap_or(0);
            json!({
                "phase": phase,
                "decision": decision,
                "evidence_count": evidence_count,
            })
        })
        .collect::<Vec<_>>();
    let satisfied = phases
        .iter()
        .all(|phase| phase.get("decision").and_then(Value::as_str) == Some("satisfied"));
    let evidence_count = phases
        .iter()
        .filter_map(|phase| phase.get("evidence_count").and_then(Value::as_u64))
        .sum::<u64>();
    json!({
        "protocol": COMPLETION_CONTRACT_PROTOCOL,
        "scope": "project_run",
        "decision": if satisfied { "satisfied" } else { "insufficient_evidence" },
        "evidence_count": evidence_count,
        "phases": phases,
        "evaluated_at": Utc::now().to_rfc3339(),
    })
}

pub(crate) fn aggregate_contract_is_satisfied(contract: &Value) -> bool {
    contract.get("decision").and_then(Value::as_str) == Some("satisfied")
}

#[derive(Default)]
struct ParsedHandoff {
    status: Option<String>,
    summary: Option<String>,
    verify: Option<String>,
}

fn parse_handoff(text: &str) -> ParsedHandoff {
    let mut parsed = ParsedHandoff::default();
    for line in text.lines() {
        let Some((name, value)) = line.split_once(':') else {
            continue;
        };
        let value = value.trim();
        if value.is_empty() {
            continue;
        }
        match name.trim().to_ascii_uppercase().as_str() {
            "STATUS" => {
                let status = value.to_ascii_lowercase();
                if matches!(status.as_str(), "complete" | "blocked") {
                    parsed.status = Some(status);
                }
            }
            "SUMMARY" => parsed.summary = Some(value.to_string()),
            "VERIFY" => parsed.verify = Some(value.to_string()),
            _ => {}
        }
    }
    parsed
}

fn meaningful_verification_summary(value: &str) -> bool {
    let normalized = value.trim().to_ascii_lowercase();
    !normalized.is_empty()
        && !matches!(
            normalized.as_str(),
            "none"
                | "n/a"
                | "na"
                | "not run"
                | "no checks"
                | "aucun"
                | "aucune"
                | "non exécuté"
                | "non execute"
        )
        && !normalized.contains("no tool errors were reported")
        && !normalized.contains("no execution evidence was recorded")
}

fn completion_evidence(
    phase: &str,
    tool_calls: &[ToolCallRecord],
    goal_checks: &[ProjectGoalCheckReceipt],
) -> Vec<CompletionEvidence> {
    let mut evidence = tool_calls
        .iter()
        .enumerate()
        .filter(|(_, call)| tool_is_phase_evidence(phase, call.tool_name.as_str()))
        .take(24)
        .map(|(index, call)| CompletionEvidence {
            id: digest_receipt(&[
                phase,
                &index.to_string(),
                &call.tool_name,
                if call.is_error { "failed" } else { "passed" },
                &call.input_summary,
                &call.output_summary,
            ]),
            kind: "tool_receipt",
            source: call.tool_name.clone(),
            status: if call.is_error { "failed" } else { "passed" },
            duration_ms: Some(call.duration_ms),
        })
        .collect::<Vec<_>>();
    evidence.extend(goal_checks.iter().map(|receipt| CompletionEvidence {
        id: receipt.receipt_id.clone(),
        kind: "project_goal_check",
        source: receipt.goal_id.clone(),
        status: if receipt.passed() { "passed" } else { "failed" },
        duration_ms: Some(receipt.latency_ms),
    }));
    evidence
}

fn requires_execution_receipt(phase: &str) -> bool {
    matches!(phase, "observe" | "build" | "execute" | "verify")
}

fn tool_is_phase_evidence(phase: &str, tool: &str) -> bool {
    let tool = tool.to_ascii_lowercase();
    match phase {
        "observe" => matches_tool_prefix(&tool, &["file_", "git_", "shell_", "project_"]),
        "build" => matches_tool_prefix(
            &tool,
            &[
                "apply_patch",
                "file_",
                "git_",
                "shell_",
                "process_",
                "browser_",
            ],
        ),
        "execute" => matches_tool_prefix(
            &tool,
            &[
                "shell_", "process_", "browser_", "http_", "web_", "ssh_", "docker_",
            ],
        ),
        "verify" => tool_is_verification_evidence(&tool),
        _ => !matches_tool_prefix(
            &tool,
            &[
                "tool_search",
                "capability_search",
                "captain_docs",
                "ask_user",
                "channel_",
            ],
        ),
    }
}

fn tool_is_verification_evidence(tool: &str) -> bool {
    let tool = tool.to_ascii_lowercase();
    matches_tool_prefix(
        &tool,
        &[
            "shell_",
            "process_",
            "git_",
            "browser_",
            "http_",
            "web_",
            "ssh_",
            "docker_",
            "file_read",
            "file_stat",
        ],
    )
}

fn matches_tool_prefix(tool: &str, prefixes: &[&str]) -> bool {
    prefixes.iter().any(|prefix| tool.starts_with(prefix))
}

fn digest_receipt(parts: &[&str]) -> String {
    let mut digest = Sha256::new();
    for part in parts {
        digest.update((part.len() as u64).to_le_bytes());
        digest.update(part.as_bytes());
    }
    hex::encode(digest.finalize())
}

#[cfg(test)]
#[path = "project_runtime_completion_tests.rs"]
mod tests;
