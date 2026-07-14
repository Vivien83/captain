use crate::project_lifecycle::runtime_progress_for_phase;
use crate::project_runtime_ask_resume::clear_runtime_project_ask_resume_pending_for_phase;
use crate::project_runtime_defaults::default_project_parallelism;
use crate::project_runtime_events::append_runtime_event;
use crate::project_runtime_worker_tools::runtime_worker_authorized_tools;
use captain_memory::project;
use captain_types::agent::ToolProfile;
use chrono::Utc;

#[derive(Debug, Clone)]
pub(crate) struct RuntimeWorkerSpec {
    pub(crate) phase: &'static str,
    pub(crate) role: &'static str,
    pub(crate) task: &'static str,
    pub(crate) mode: &'static str,
    pub(crate) dependencies: &'static [&'static str],
    pub(crate) profile: ToolProfile,
}

pub(crate) const RUNTIME_WORKER_SPECS: &[RuntimeWorkerSpec] = &[
    RuntimeWorkerSpec {
        phase: "observe",
        role: "observer",
        task: "Capture repo state, constraints, existing files, and blockers before editing.",
        mode: "parallel_read",
        dependencies: &[],
        profile: ToolProfile::Coding,
    },
    RuntimeWorkerSpec {
        phase: "think",
        role: "architect",
        task: "Compare options, risks, provider/model fit, and opportunities for parallel slices.",
        mode: "parallel_read",
        dependencies: &[],
        profile: ToolProfile::Coding,
    },
    RuntimeWorkerSpec {
        phase: "plan",
        role: "planner",
        task: "Create the execution graph, dependencies, verification gates, and ownership boundaries.",
        mode: "gated",
        dependencies: &["observe", "think"],
        profile: ToolProfile::Coding,
    },
    RuntimeWorkerSpec {
        phase: "build",
        role: "builder",
        task: "Implement focused slices while preserving unrelated user work.",
        mode: "gated",
        dependencies: &["plan"],
        profile: ToolProfile::Coding,
    },
    RuntimeWorkerSpec {
        phase: "execute",
        role: "runner",
        task: "Run the workflow end to end and surface action logs.",
        mode: "gated",
        dependencies: &["build"],
        profile: ToolProfile::Coding,
    },
    RuntimeWorkerSpec {
        phase: "verify",
        role: "verifier",
        task: "Run tests, smoke checks, review graph impact, and record blockers.",
        mode: "gated",
        dependencies: &["execute"],
        profile: ToolProfile::Coding,
    },
    RuntimeWorkerSpec {
        phase: "learn",
        role: "librarian",
        task: "Checkpoint the session and propose memory/skill improvements without duplicates.",
        mode: "gated",
        dependencies: &["verify"],
        profile: ToolProfile::Automation,
    },
];

pub(crate) fn runtime_workers_for_project(project: &project::Project) -> serde_json::Value {
    serde_json::json!(RUNTIME_WORKER_SPECS
        .iter()
        .map(|spec| runtime_worker_json(project, spec))
        .collect::<Vec<_>>())
}

pub(crate) fn runtime_worker_id(project: &project::Project, phase: &str) -> String {
    format!("{}-{}", project.slug, phase)
}

pub(crate) fn runtime_worker_json(
    project: &project::Project,
    spec: &RuntimeWorkerSpec,
) -> serde_json::Value {
    serde_json::json!({
        "id": runtime_worker_id(project, spec.phase),
        "role": spec.role,
        "phase": spec.phase,
        "status": if spec.phase == "observe" || spec.phase == "think" { "ready" } else { "planned" },
        "mode": spec.mode,
        "provider_policy": "same_provider",
        "model_policy": "fit_to_task",
        "task": spec.task,
        "depends_on": spec.dependencies,
        "authorized_tools": runtime_worker_authorized_tools(&spec.profile),
        "agent_id": serde_json::Value::Null,
    })
}

pub(crate) fn upsert_runtime_worker<F>(
    runtime: &mut serde_json::Value,
    project: &project::Project,
    spec: &RuntimeWorkerSpec,
    mutate: F,
) where
    F: FnOnce(&mut serde_json::Map<String, serde_json::Value>),
{
    if !runtime
        .get("workers")
        .map(|value| value.is_array())
        .unwrap_or(false)
    {
        runtime["workers"] = runtime_workers_for_project(project);
    }
    let id = runtime_worker_id(project, spec.phase);
    let Some(workers) = runtime
        .get_mut("workers")
        .and_then(|value| value.as_array_mut())
    else {
        return;
    };
    if !workers
        .iter()
        .any(|worker| worker.get("id").and_then(|value| value.as_str()) == Some(id.as_str()))
    {
        workers.push(runtime_worker_json(project, spec));
    }
    if let Some(worker) = workers
        .iter_mut()
        .find(|worker| worker.get("id").and_then(|value| value.as_str()) == Some(id.as_str()))
        .and_then(|worker| worker.as_object_mut())
    {
        mutate(worker);
    }
}

pub(crate) fn runtime_existing_worker_status(
    runtime: &serde_json::Value,
    phase: &str,
) -> Option<String> {
    runtime
        .get("workers")
        .and_then(|value| value.as_array())
        .and_then(|workers| {
            workers
                .iter()
                .find(|worker| worker.get("phase").and_then(|value| value.as_str()) == Some(phase))
        })
        .and_then(|worker| worker.get("status").and_then(|value| value.as_str()))
        .map(str::to_string)
}

pub(crate) fn mark_runtime_worker_started(
    runtime: &mut serde_json::Value,
    project: &project::Project,
    spec: &RuntimeWorkerSpec,
    run_id: &str,
    agent_id: &str,
    authorized_tools: &[String],
) {
    let phase = spec.phase;
    runtime["status"] = serde_json::json!("running");
    runtime["current_phase"] = serde_json::json!(phase);
    runtime["progress"] = serde_json::json!(runtime_progress_for_phase(phase, "running"));
    runtime["updated_at"] = serde_json::json!(Utc::now().to_rfc3339());
    upsert_runtime_worker(runtime, project, spec, |worker| {
        worker.insert("status".to_string(), serde_json::json!("running"));
        worker.insert(
            "authorized_tools".to_string(),
            serde_json::json!(authorized_tools),
        );
        worker.insert("agent_id".to_string(), serde_json::json!(agent_id));
        worker.insert(
            "started_at".to_string(),
            serde_json::json!(Utc::now().to_rfc3339()),
        );
        worker.insert("run_id".to_string(), serde_json::json!(run_id));
    });
    clear_runtime_project_ask_resume_pending_for_phase(runtime, phase);
    append_runtime_event(
        runtime,
        "worker.started",
        &format!("{} started", spec.role),
        spec.task,
        agent_id,
        phase,
        "running",
        serde_json::json!({
            "run_id": run_id,
            "worker_id": runtime_worker_id(project, phase),
            "agent_id": agent_id,
            "role": spec.role,
            "authorized_tools": authorized_tools,
        }),
    );
}

pub(crate) fn mark_runtime_worker_skipped(
    runtime: &mut serde_json::Value,
    project: &project::Project,
    spec: &RuntimeWorkerSpec,
    run_id: &str,
) {
    let phase = spec.phase;
    runtime["current_phase"] = serde_json::json!(phase);
    runtime["progress"] = serde_json::json!(runtime_progress_for_phase(phase, "running"));
    append_runtime_event(
        runtime,
        "worker.skipped",
        &format!("{} already completed", spec.role),
        "Captain resumed the run and skipped this phase because it already has a completed worker result.",
        "captain",
        phase,
        "done",
        serde_json::json!({
            "run_id": run_id,
            "worker_id": runtime_worker_id(project, phase),
        }),
    );
}

pub(crate) fn recompute_runtime_parallelism(runtime: &mut serde_json::Value) {
    let running = runtime
        .get("workers")
        .and_then(|value| value.as_array())
        .map(|workers| {
            workers
                .iter()
                .filter(|worker| {
                    worker
                        .get("status")
                        .and_then(|value| value.as_str())
                        .map(|status| status == "running")
                        .unwrap_or(false)
                })
                .count()
        })
        .unwrap_or(0);
    if !runtime
        .get("parallelism")
        .map(|value| value.is_object())
        .unwrap_or(false)
    {
        runtime["parallelism"] = serde_json::json!({
            "max_parallel_agents": default_project_parallelism(),
            "running": running,
            "policy": "same_provider_model_fit",
        });
        return;
    }
    if let Some(obj) = runtime
        .get_mut("parallelism")
        .and_then(|value| value.as_object_mut())
    {
        obj.insert("running".to_string(), serde_json::json!(running));
    }
}

#[cfg(test)]
#[path = "project_runtime_worker_started_tests.rs"]
mod project_runtime_worker_started_tests;

#[cfg(test)]
#[path = "project_runtime_worker_skip_tests.rs"]
mod project_runtime_worker_skip_tests;

#[cfg(test)]
#[path = "project_runtime_workers_tests.rs"]
mod project_runtime_workers_tests;
