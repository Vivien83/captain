use crate::project_runtime_asks::RuntimeProjectAskAnswer;
use crate::project_runtime_checkpoints::trim_runtime_text;
use chrono::Utc;

pub(crate) fn mark_runtime_project_ask_resume_pending(
    runtime: &mut serde_json::Value,
    answer: &RuntimeProjectAskAnswer,
) {
    let now = Utc::now().to_rfc3339();
    runtime["status"] = serde_json::json!("ready");
    runtime["current_phase"] = serde_json::json!(answer.phase.clone());
    runtime["resume_pending"] = serde_json::json!({
        "reason": "project_ask_answered",
        "ask_id": answer.ask_id,
        "phase": answer.phase,
        "answer": trim_runtime_text(&answer.answer, 900),
        "marked_at": now,
    });
    runtime["control"] = serde_json::json!({
        "paused": false,
        "takeover": false,
    });
    reopen_worker_for_user_answer(runtime, answer);
}

#[cfg(test)]
pub(crate) fn runtime_project_ask_resume_phase(runtime: &serde_json::Value) -> Option<String> {
    let pending = runtime.get("resume_pending")?;
    if pending.get("reason").and_then(|v| v.as_str()) != Some("project_ask_answered") {
        return None;
    }
    runtime_resume_pending_phase(runtime)
}

pub(crate) fn runtime_resume_pending_phase(runtime: &serde_json::Value) -> Option<String> {
    let pending = runtime.get("resume_pending")?;
    pending
        .get("phase")
        .and_then(|v| v.as_str())
        .map(str::to_string)
        .filter(|phase| !phase.trim().is_empty())
}

pub(crate) fn runtime_resume_pending_reason(runtime: &serde_json::Value) -> Option<String> {
    runtime
        .pointer("/resume_pending/reason")
        .and_then(|v| v.as_str())
        .map(str::to_string)
        .filter(|reason| !reason.trim().is_empty())
}

pub(crate) fn clear_runtime_project_ask_resume_pending_for_phase(
    runtime: &mut serde_json::Value,
    phase: &str,
) {
    if runtime
        .pointer("/resume_pending/phase")
        .and_then(|v| v.as_str())
        == Some(phase)
    {
        if let Some(obj) = runtime.as_object_mut() {
            obj.remove("resume_pending");
        }
    }
    if let Some(workers) = runtime.get_mut("workers").and_then(|v| v.as_array_mut()) {
        for worker in workers {
            let same_phase = worker.get("phase").and_then(|v| v.as_str()) == Some(phase);
            if same_phase {
                if let Some(obj) = worker.as_object_mut() {
                    obj.remove("resume_pending");
                    obj.remove("resume_after_user_answer");
                }
            }
        }
    }
    if let Some(result) = runtime
        .pointer_mut(&format!("/worker_results/{phase}"))
        .and_then(|v| v.as_object_mut())
    {
        result.remove("resume_pending");
    }
}

fn reopen_worker_for_user_answer(
    runtime: &mut serde_json::Value,
    answer: &RuntimeProjectAskAnswer,
) {
    let now = Utc::now().to_rfc3339();
    if let Some(workers) = runtime.get_mut("workers").and_then(|v| v.as_array_mut()) {
        for worker in workers {
            let same_phase =
                worker.get("phase").and_then(|v| v.as_str()) == Some(answer.phase.as_str());
            if !same_phase {
                continue;
            }
            let status = worker
                .get("status")
                .and_then(|v| v.as_str())
                .unwrap_or("ready")
                .to_string();
            if status == "done" {
                continue;
            }
            if let Some(obj) = worker.as_object_mut() {
                obj.insert("status".to_string(), serde_json::json!("ready"));
                obj.insert("resume_pending".to_string(), serde_json::json!(true));
                obj.insert(
                    "resume_after_user_answer".to_string(),
                    serde_json::json!({
                        "ask_id": answer.ask_id,
                        "previous_status": status,
                        "marked_at": now,
                    }),
                );
                obj.remove("error");
            }
        }
    }
    if let Some(result) = runtime
        .pointer_mut(&format!("/worker_results/{}", answer.phase))
        .and_then(|v| v.as_object_mut())
    {
        let status = result
            .get("status")
            .and_then(|v| v.as_str())
            .unwrap_or("blocked")
            .to_string();
        if status != "done" {
            result.insert("status".to_string(), serde_json::json!("ready"));
            result.insert("blocked".to_string(), serde_json::json!(false));
        }
        result.insert(
            "resume_pending".to_string(),
            serde_json::json!({
                "reason": "project_ask_answered",
                "ask_id": answer.ask_id,
            }),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn answer() -> RuntimeProjectAskAnswer {
        RuntimeProjectAskAnswer {
            ask_id: "ask-1".to_string(),
            phase: "build".to_string(),
            question: "Which path?".to_string(),
            answer: "Use the simple path.".to_string(),
            was_pending: true,
        }
    }

    #[test]
    fn project_ask_answer_marks_runtime_ready_for_resume() {
        let mut runtime = serde_json::json!({
            "status": "blocked",
            "current_phase": "build",
            "control": {"paused": true, "takeover": false},
            "workers": [
                {"phase": "build", "status": "blocked", "error": "waiting user"}
            ],
            "worker_results": {
                "build": {"status": "blocked", "blocked": true}
            }
        });

        mark_runtime_project_ask_resume_pending(&mut runtime, &answer());

        assert_eq!(runtime["status"], "ready");
        assert_eq!(runtime["current_phase"], "build");
        assert_eq!(
            runtime_project_ask_resume_phase(&runtime).as_deref(),
            Some("build")
        );
        assert_eq!(runtime["workers"][0]["status"], "ready");
        assert_eq!(runtime["worker_results"]["build"]["blocked"], false);
        assert!(runtime["workers"][0].get("error").is_none());
    }

    #[test]
    fn project_ask_resume_clear_is_phase_scoped() {
        let mut runtime = serde_json::json!({
            "resume_pending": {"reason": "tool_request_approved", "phase": "build"},
            "workers": [
                {"phase": "build", "resume_pending": true, "resume_after_user_answer": {}},
                {"phase": "verify", "resume_pending": true}
            ],
            "worker_results": {
                "build": {"resume_pending": {}}
            }
        });

        clear_runtime_project_ask_resume_pending_for_phase(&mut runtime, "build");

        assert!(runtime.get("resume_pending").is_none());
        assert!(runtime["workers"][0].get("resume_pending").is_none());
        assert_eq!(runtime["workers"][1]["resume_pending"], true);
        assert!(runtime["worker_results"]["build"]
            .get("resume_pending")
            .is_none());
    }

    #[test]
    fn generic_resume_phase_keeps_tool_request_approval_resume_ready() {
        let runtime = serde_json::json!({
            "resume_pending": {
                "reason": "tool_request_approved",
                "phase": "verify"
            }
        });

        assert_eq!(
            runtime_resume_pending_phase(&runtime).as_deref(),
            Some("verify")
        );
        assert_eq!(
            runtime_resume_pending_reason(&runtime).as_deref(),
            Some("tool_request_approved")
        );
        assert_eq!(runtime_project_ask_resume_phase(&runtime), None);
    }
}
