use super::*;
use captain_memory::project::{self, ProjectStatus};

fn project() -> project::Project {
    project::Project {
        id: "project-1".to_string(),
        name: "Demo".to_string(),
        slug: "demo".to_string(),
        goal: "Ship safely".to_string(),
        status: ProjectStatus::Active,
        deadline: None,
        created_at: 0,
        updated_at: 0,
        metadata: serde_json::json!({}),
    }
}

#[test]
fn worker_specs_keep_hermes_phase_order_and_dependencies() {
    let phases = RUNTIME_WORKER_SPECS
        .iter()
        .map(|spec| spec.phase)
        .collect::<Vec<_>>();
    assert_eq!(
        phases,
        vec!["observe", "think", "plan", "build", "execute", "verify", "learn"]
    );

    let plan = RUNTIME_WORKER_SPECS
        .iter()
        .find(|spec| spec.phase == "plan")
        .unwrap();
    assert_eq!(plan.dependencies, &["observe", "think"]);
    let learn = RUNTIME_WORKER_SPECS.last().unwrap();
    assert_eq!(learn.dependencies, &["verify"]);
}

#[test]
fn worker_json_marks_parallel_read_phases_ready_only() {
    let project = project();
    let workers = runtime_workers_for_project(&project);
    let workers = workers.as_array().unwrap();

    let observe = &workers[0];
    assert_eq!(observe["id"], "demo-observe");
    assert_eq!(observe["status"], "ready");
    assert_eq!(observe["mode"], "parallel_read");

    let build = workers
        .iter()
        .find(|worker| worker["phase"] == "build")
        .unwrap();
    assert_eq!(build["status"], "planned");
    assert_eq!(build["mode"], "gated");
}

#[test]
fn worker_json_preserves_provider_policy_and_tool_allowlist() {
    let project = project();
    let worker = runtime_worker_json(&project, &RUNTIME_WORKER_SPECS[3]);

    assert_eq!(worker["provider_policy"], "same_provider");
    assert_eq!(worker["model_policy"], "fit_to_task");
    assert!(worker["agent_id"].is_null());
    let tools = worker["authorized_tools"].as_array().unwrap();
    assert!(tools.iter().any(|tool| tool == "file_read"));
    assert!(tools.iter().any(|tool| tool == "tool_search"));
}

#[test]
fn upsert_runtime_worker_initializes_missing_workers_and_mutates_target() {
    let project = project();
    let mut runtime = serde_json::json!({"workers": "legacy-invalid"});

    upsert_runtime_worker(&mut runtime, &project, &RUNTIME_WORKER_SPECS[3], |worker| {
        worker.insert("status".to_string(), serde_json::json!("running"));
        worker.insert("agent_id".to_string(), serde_json::json!("agent-build"));
    });

    let workers = runtime["workers"].as_array().unwrap();
    let build = workers
        .iter()
        .find(|worker| worker["id"] == "demo-build")
        .unwrap();
    assert_eq!(build["status"], "running");
    assert_eq!(build["agent_id"], "agent-build");
    assert_eq!(workers.len(), RUNTIME_WORKER_SPECS.len());
}

#[test]
fn upsert_runtime_worker_adds_missing_worker_without_duplicates() {
    let project = project();
    let mut runtime = serde_json::json!({
        "workers": [
            {"id": "demo-observe", "phase": "observe", "status": "running"}
        ]
    });

    upsert_runtime_worker(&mut runtime, &project, &RUNTIME_WORKER_SPECS[0], |worker| {
        worker.insert("summary".to_string(), serde_json::json!("observing"));
    });
    upsert_runtime_worker(&mut runtime, &project, &RUNTIME_WORKER_SPECS[1], |worker| {
        worker.insert("status".to_string(), serde_json::json!("running"));
    });

    let workers = runtime["workers"].as_array().unwrap();
    assert_eq!(
        workers
            .iter()
            .filter(|worker| worker["id"] == "demo-observe")
            .count(),
        1
    );
    assert!(workers.iter().any(|worker| worker["id"] == "demo-think"));
    assert_eq!(workers[0]["summary"], "observing");
}

#[test]
fn runtime_existing_worker_status_reads_matching_phase_only() {
    let runtime = serde_json::json!({
        "workers": [
            {"phase": "observe", "status": "done"},
            {"phase": "build", "status": "running"},
            {"phase": "verify", "status": "blocked"}
        ]
    });

    assert_eq!(
        runtime_existing_worker_status(&runtime, "build"),
        Some("running".to_string())
    );
    assert_eq!(
        runtime_existing_worker_status(&runtime, "verify"),
        Some("blocked".to_string())
    );
}

#[test]
fn runtime_existing_worker_status_ignores_missing_or_malformed_workers() {
    assert_eq!(
        runtime_existing_worker_status(&serde_json::json!({}), "build"),
        None
    );
    assert_eq!(
        runtime_existing_worker_status(&serde_json::json!({"workers": "legacy"}), "build"),
        None
    );
    assert_eq!(
        runtime_existing_worker_status(
            &serde_json::json!({
                "workers": [
                    {"phase": "build"},
                    {"phase": "verify", "status": 42}
                ]
            }),
            "build"
        ),
        None
    );
}

#[test]
fn recompute_runtime_parallelism_counts_running_workers_and_preserves_policy() {
    let mut runtime = serde_json::json!({
        "workers": [
            {"status": "running"},
            {"status": "done"},
            {"status": "running"},
            {"status": 42}
        ],
        "parallelism": {
            "max_parallel_agents": 3,
            "running": 99,
            "policy": "same_provider_model_fit"
        }
    });

    recompute_runtime_parallelism(&mut runtime);

    assert_eq!(runtime["parallelism"]["running"], 2);
    assert_eq!(runtime["parallelism"]["max_parallel_agents"], 3);
    assert_eq!(runtime["parallelism"]["policy"], "same_provider_model_fit");
}

#[test]
fn recompute_runtime_parallelism_initializes_missing_parallelism() {
    let mut runtime = serde_json::json!({
        "workers": [
            {"status": "planned"},
            {"status": "running"}
        ]
    });

    recompute_runtime_parallelism(&mut runtime);

    assert_eq!(runtime["parallelism"]["running"], 1);
    assert_eq!(
        runtime["parallelism"]["max_parallel_agents"],
        default_project_parallelism()
    );
    assert_eq!(runtime["parallelism"]["policy"], "same_provider_model_fit");
}
