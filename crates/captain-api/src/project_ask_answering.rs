use crate::project_runtime_ask_resume::mark_runtime_project_ask_resume_pending;
use crate::project_runtime_asks::{
    append_runtime_project_ask_answer_event, mark_runtime_project_ask_answered,
    RuntimeProjectAskAnswer,
};
use captain_memory::{project, MemorySubstrate};

pub(crate) fn record_project_ask_answer_runtime(
    memory: &MemorySubstrate,
    project_id_hint: Option<&str>,
    ask_id_prefix: &str,
    answer: &str,
    delivery: &str,
) -> Result<(project::Project, RuntimeProjectAskAnswer), String> {
    let projects = if let Some(project_id) = project_id_hint {
        match memory.project_get(project_id) {
            Ok(Some(project)) => vec![project],
            Ok(None) => return Err(format!("project '{project_id}' not found")),
            Err(e) => return Err(format!("{e}")),
        }
    } else {
        memory.project_list(true).map_err(|e| format!("{e}"))?
    };

    let mut matches = Vec::new();
    for project in projects {
        let Some(mut runtime) = project.metadata.get("runtime").cloned() else {
            continue;
        };
        let Ok(receipt) =
            mark_runtime_project_ask_answered(&mut runtime, ask_id_prefix, answer, delivery)
        else {
            continue;
        };
        matches.push((project, runtime, receipt));
    }

    let (project, mut runtime, receipt) = match matches.len() {
        0 => {
            return Err(format!(
                "no persisted project question matches '{ask_id_prefix}'"
            ))
        }
        1 => matches.remove(0),
        n => {
            return Err(format!(
                "{n} persisted project questions match '{ask_id_prefix}'. Use more characters."
            ))
        }
    };

    append_runtime_project_ask_answer_event(&mut runtime, &receipt, "user");
    if delivery == "recorded_for_resume" {
        mark_runtime_project_ask_resume_pending(&mut runtime, &receipt);
    }
    let mut metadata = project.metadata.clone();
    if !metadata.is_object() {
        metadata = serde_json::json!({});
    }
    if let Some(obj) = metadata.as_object_mut() {
        obj.insert("runtime".to_string(), runtime);
    }
    let updated = memory
        .project_update(
            &project.id,
            project::ProjectPatch {
                metadata: Some(metadata),
                ..Default::default()
            },
        )
        .map_err(|e| format!("{e}"))?
        .ok_or_else(|| format!("project '{}' not found", project.id))?;
    Ok((updated, receipt))
}

#[cfg(test)]
mod tests {
    use super::*;
    use captain_memory::project::{NewProject, ProjectPatch};

    fn blocked_runtime_with_pending_build_question() -> serde_json::Value {
        serde_json::json!({
            "runtime": {
                "status": "blocked",
                "current_phase": "observe",
                "control": { "paused": true, "takeover": false },
                "timeline": [],
                "workers": [
                    {
                        "id": "worker-build",
                        "phase": "build",
                        "status": "blocked",
                        "error": "Waiting for user"
                    }
                ],
                "worker_results": {
                    "build": {
                        "status": "blocked",
                        "blocked": true
                    }
                },
                "user_questions": [
                    {
                        "ask_id": "ask-abcdef",
                        "run_id": "run-1",
                        "phase": "build",
                        "worker_id": "worker-build",
                        "agent_id": "agent-build",
                        "worker_role": "builder",
                        "question": "Which path?",
                        "options": ["Simple", "Complex"],
                        "status": "pending",
                        "delivery": "waiting_for_user"
                    }
                ]
            }
        })
    }

    fn create_project_with_pending_build_question(memory: &MemorySubstrate) -> String {
        let project = memory
            .project_create(NewProject {
                name: "Demo".to_string(),
                slug: "demo".to_string(),
                goal: "Ship".to_string(),
                deadline: None,
            })
            .unwrap();
        memory
            .project_update(
                &project.id,
                ProjectPatch {
                    metadata: Some(blocked_runtime_with_pending_build_question()),
                    ..Default::default()
                },
            )
            .unwrap();
        project.id
    }

    fn assert_runtime_ready_for_resume(runtime: &serde_json::Value) {
        assert_eq!(runtime["status"], "ready");
        assert_eq!(runtime["current_phase"], "build");
        assert_eq!(runtime["resume_pending"]["ask_id"], "ask-abcdef");
        assert_eq!(runtime["workers"][0]["status"], "ready");
        assert_eq!(runtime["workers"][0]["resume_pending"], true);
        assert_eq!(runtime["worker_results"]["build"]["blocked"], false);
        assert_eq!(runtime["user_questions"][0]["status"], "answered");
        assert_eq!(
            runtime["user_questions"][0]["delivery"],
            "recorded_for_resume"
        );
        assert_eq!(runtime["timeline"][0]["kind"], "worker.ask_user_answered");
    }

    #[test]
    fn record_answer_marks_persisted_runtime_for_resume() {
        let memory = MemorySubstrate::open_in_memory(0.1).unwrap();
        let project_id = create_project_with_pending_build_question(&memory);

        let (updated, receipt) = record_project_ask_answer_runtime(
            &memory,
            Some(&project_id),
            "ask-abc",
            "1",
            "recorded_for_resume",
        )
        .unwrap();

        assert_eq!(receipt.ask_id, "ask-abcdef");
        assert_eq!(receipt.answer, "Simple");
        assert_runtime_ready_for_resume(&updated.metadata["runtime"]);
    }
}
