use crate::project_lifecycle::{is_valid_lifecycle_phase, runtime_progress_for_phase};
use crate::project_runtime_ask_resume::{
    runtime_resume_pending_phase, runtime_resume_pending_reason,
};
use crate::project_runtime_defaults::project_session_id;
use crate::project_runtime_events::append_runtime_event;
use crate::project_runtime_orchestrator::{
    activate_runtime_orchestrator, resume_runtime_orchestrator, runtime_resume_event_metadata,
};
use crate::project_runtime_resume::runtime_should_resume_stale_run;
use crate::project_runtime_workers::runtime_workers_for_project;
use captain_memory::project;
use chrono::Utc;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ProjectRuntimeStartAction {
    AlreadyRunning,
    ResumeAfterRestart,
    ResumePending,
    FreshStart,
}

pub(crate) fn apply_project_runtime_start(
    runtime: &mut serde_json::Value,
    project: &project::Project,
    process_running: bool,
) -> ProjectRuntimeStartAction {
    if process_running {
        mark_runtime_already_running(runtime);
        return ProjectRuntimeStartAction::AlreadyRunning;
    }

    if runtime_should_resume_stale_run(process_running, runtime) {
        resume_stale_project_runtime(runtime);
        return ProjectRuntimeStartAction::ResumeAfterRestart;
    }

    if resume_pending_project_runtime(runtime) {
        return ProjectRuntimeStartAction::ResumePending;
    }

    start_fresh_project_runtime(runtime, project);
    ProjectRuntimeStartAction::FreshStart
}

fn mark_runtime_already_running(runtime: &mut serde_json::Value) {
    let phase = runtime_phase_or_observe(runtime);
    append_runtime_event(
        runtime,
        "orchestrator.already_running",
        "Run already active",
        "Captain already has an in-process project orchestrator for this project.",
        "captain",
        &phase,
        "running",
        serde_json::json!({}),
    );
}

fn resume_stale_project_runtime(runtime: &mut serde_json::Value) {
    let phase = valid_runtime_phase_or_observe(runtime);
    resume_runtime_orchestrator(
        runtime,
        &phase,
        "resume_after_restart",
        "orchestrator.resume_after_restart",
        "Run recovered after restart",
        "Captain found a persisted active project runtime and is resuming it without resetting completed worker phases.",
        "captain",
    );
}

fn resume_pending_project_runtime(runtime: &mut serde_json::Value) -> bool {
    let Some(phase) =
        runtime_resume_pending_phase(runtime).filter(|phase| is_valid_lifecycle_phase(phase))
    else {
        return false;
    };
    let reason = runtime_resume_pending_reason(runtime);
    let resume = runtime_resume_event_metadata(reason.as_deref());
    resume_runtime_orchestrator(
        runtime,
        &phase,
        resume.trigger,
        resume.kind,
        resume.title,
        resume.detail,
        "captain",
    );
    true
}

fn start_fresh_project_runtime(runtime: &mut serde_json::Value, project: &project::Project) {
    let now = Utc::now().to_rfc3339();
    let phase = valid_runtime_phase_or_observe(runtime);
    let run_id = activate_runtime_orchestrator(runtime, "start");
    runtime["status"] = serde_json::json!("running");
    runtime["current_phase"] = serde_json::json!(phase.clone());
    runtime["progress"] = serde_json::json!(runtime_progress_for_phase(&phase, "running"));
    runtime["started_at"] = runtime
        .get("started_at")
        .cloned()
        .filter(|value| !value.is_null())
        .unwrap_or_else(|| serde_json::json!(now.clone()));
    runtime["updated_at"] = serde_json::json!(now);
    runtime["control"] = serde_json::json!({
        "paused": false,
        "takeover": false,
    });
    runtime["workers"] = runtime_workers_for_project(project);
    runtime["worker_results"] = serde_json::json!({});
    runtime["user_questions"] = serde_json::json!([]);
    append_fresh_start_events(runtime, project, &run_id);
}

fn append_fresh_start_events(
    runtime: &mut serde_json::Value,
    project: &project::Project,
    run_id: &str,
) {
    append_runtime_event(
        runtime,
        "project.started",
        "Autonomous run started",
        "Captain is now managing this project through OBSERVE -> THINK -> PLAN -> BUILD -> EXECUTE -> VERIFY -> LEARN.",
        "captain",
        "observe",
        "running",
        serde_json::json!({
            "run_id": run_id,
            "session_id": project_session_id(project),
            "parallelism": runtime.get("parallelism").cloned().unwrap_or_else(|| serde_json::json!({})),
        }),
    );
    let worker_count = runtime
        .get("workers")
        .and_then(|workers| workers.as_array())
        .map(|workers| workers.len())
        .unwrap_or(0);
    append_runtime_event(
        runtime,
        "task_graph.created",
        "Task graph prepared",
        "Read-only OBSERVE/THINK workers are parallelizable; PLAN/BUILD/EXECUTE/VERIFY/LEARN stay gated until their dependencies are satisfied.",
        "captain",
        "plan",
        "ready",
        serde_json::json!({ "worker_count": worker_count }),
    );
}

fn runtime_phase_or_observe(runtime: &serde_json::Value) -> String {
    runtime
        .get("current_phase")
        .and_then(|value| value.as_str())
        .unwrap_or("observe")
        .to_string()
}

fn valid_runtime_phase_or_observe(runtime: &serde_json::Value) -> String {
    runtime
        .get("current_phase")
        .and_then(|value| value.as_str())
        .filter(|phase| is_valid_lifecycle_phase(phase))
        .unwrap_or("observe")
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use captain_memory::project::ProjectStatus;

    fn project_fixture() -> project::Project {
        project::Project {
            id: "project-1".to_string(),
            name: "Alpha".to_string(),
            slug: "alpha".to_string(),
            goal: "Ship alpha".to_string(),
            status: ProjectStatus::Active,
            deadline: None,
            created_at: 1_779_660_000_000,
            updated_at: 1_779_660_000_000,
            metadata: serde_json::json!({}),
        }
    }

    fn last_event(runtime: &serde_json::Value) -> &serde_json::Value {
        runtime["timeline"].as_array().unwrap().last().unwrap()
    }

    #[test]
    fn runtime_start_records_already_running_without_resetting_runtime() {
        let mut runtime = serde_json::json!({
            "status": "running",
            "current_phase": "build",
            "workers": [{"phase": "build", "status": "running"}],
            "worker_results": {"observe": {"status": "done"}},
            "user_questions": [{"ask_id": "ask-1"}],
            "timeline": []
        });

        let action = apply_project_runtime_start(&mut runtime, &project_fixture(), true);

        assert_eq!(action, ProjectRuntimeStartAction::AlreadyRunning);
        assert_eq!(runtime["status"], "running");
        assert_eq!(runtime["current_phase"], "build");
        assert_eq!(runtime["worker_results"]["observe"]["status"], "done");
        assert_eq!(runtime["user_questions"].as_array().unwrap().len(), 1);
        assert_eq!(last_event(&runtime)["kind"], "orchestrator.already_running");
        assert_eq!(last_event(&runtime)["phase"], "build");
    }

    #[test]
    fn runtime_start_resumes_stale_active_run_without_resetting_results() {
        let mut runtime = serde_json::json!({
            "status": "running",
            "current_phase": "verify",
            "orchestrator": {"run_id": "run-1", "active": true},
            "control": {"paused": true, "takeover": true},
            "workers": [{"phase": "observe", "status": "done"}],
            "worker_results": {"observe": {"status": "done"}},
            "timeline": []
        });

        let action = apply_project_runtime_start(&mut runtime, &project_fixture(), false);

        assert_eq!(action, ProjectRuntimeStartAction::ResumeAfterRestart);
        assert_eq!(runtime["status"], "running");
        assert_eq!(runtime["current_phase"], "verify");
        assert_eq!(runtime["orchestrator"]["run_id"], "run-1");
        assert_eq!(runtime["orchestrator"]["trigger"], "resume_after_restart");
        assert_eq!(runtime["control"]["paused"], false);
        assert_eq!(runtime["worker_results"]["observe"]["status"], "done");
        assert_eq!(
            last_event(&runtime)["kind"],
            "orchestrator.resume_after_restart"
        );
    }

    #[test]
    fn runtime_start_resumes_pending_phase_with_reason_metadata() {
        let mut runtime = serde_json::json!({
            "status": "ready",
            "current_phase": "build",
            "orchestrator": {"run_id": "run-2", "active": false},
            "resume_pending": {
                "reason": "project_ask_answered",
                "phase": "build"
            },
            "control": {"paused": true, "takeover": true},
            "timeline": []
        });

        let action = apply_project_runtime_start(&mut runtime, &project_fixture(), false);

        assert_eq!(action, ProjectRuntimeStartAction::ResumePending);
        assert_eq!(runtime["status"], "running");
        assert_eq!(runtime["current_phase"], "build");
        assert_eq!(
            runtime["orchestrator"]["trigger"],
            "resume_after_user_answer"
        );
        assert_eq!(runtime["control"]["takeover"], false);
        assert_eq!(
            last_event(&runtime)["kind"],
            "orchestrator.resume_after_user_answer"
        );
    }

    #[test]
    fn runtime_start_ignores_invalid_pending_phase_and_starts_fresh() {
        let mut runtime = serde_json::json!({
            "status": "ready",
            "current_phase": "observe",
            "resume_pending": {
                "reason": "project_ask_answered",
                "phase": "invalid-phase"
            },
            "timeline": []
        });

        let action = apply_project_runtime_start(&mut runtime, &project_fixture(), false);

        assert_eq!(action, ProjectRuntimeStartAction::FreshStart);
        assert_eq!(runtime["status"], "running");
        assert_eq!(runtime["current_phase"], "observe");
        assert_eq!(runtime["timeline"][0]["kind"], "project.started");
    }

    #[test]
    fn runtime_start_creates_fresh_runtime_contract() {
        let mut runtime = serde_json::json!({
            "current_phase": "unknown",
            "started_at": null,
            "parallelism": {"max_parallel": 2},
            "worker_results": {"old": {"status": "done"}},
            "user_questions": [{"ask_id": "old"}],
            "timeline": []
        });

        let action = apply_project_runtime_start(&mut runtime, &project_fixture(), false);

        assert_eq!(action, ProjectRuntimeStartAction::FreshStart);
        assert_eq!(runtime["status"], "running");
        assert_eq!(runtime["current_phase"], "observe");
        assert_eq!(
            runtime["progress"],
            serde_json::json!(runtime_progress_for_phase("observe", "running"))
        );
        assert_eq!(runtime["control"]["paused"], false);
        assert_eq!(runtime["workers"].as_array().unwrap().len(), 7);
        assert_eq!(runtime["worker_results"], serde_json::json!({}));
        assert_eq!(runtime["user_questions"], serde_json::json!([]));
        assert!(runtime["started_at"].as_str().is_some());
        assert_eq!(runtime["timeline"].as_array().unwrap().len(), 2);
        assert_eq!(runtime["timeline"][0]["kind"], "project.started");
        assert_eq!(
            runtime["timeline"][0]["data"]["session_id"],
            "project-alpha"
        );
        assert_eq!(runtime["timeline"][1]["kind"], "task_graph.created");
        assert_eq!(runtime["timeline"][1]["data"]["worker_count"], 7);
    }
}
