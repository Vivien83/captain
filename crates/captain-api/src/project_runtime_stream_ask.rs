use crate::project_ask::{register_project_ask, ProjectAskRegistration};
use crate::project_runtime_asks::{record_runtime_project_ask_event, RuntimeProjectAsk};
use crate::project_runtime_mutation::update_project_runtime_state;
use crate::project_runtime_workers::{runtime_worker_id, RuntimeWorkerSpec};
use crate::routes::AppState;
use captain_memory::project;
use captain_types::agent::AgentId;
use captain_types::event::ChatStreamEvent;
use chrono::Utc;
use std::sync::Arc;
use tokio::sync::mpsc;

#[allow(clippy::too_many_arguments)]
pub(crate) async fn record_project_worker_ask_user(
    state: &Arc<AppState>,
    project: &project::Project,
    spec: &RuntimeWorkerSpec,
    run_id: &str,
    phase: &str,
    agent_id: AgentId,
    question: String,
    options: Option<Vec<String>>,
    user_input_tx: mpsc::Sender<String>,
) -> Result<(), String> {
    let worker_id = runtime_worker_id(project, phase);
    let agent_id_string = agent_id.to_string();
    let ask_id = register_project_ask(
        project_ask_registration(project, spec, phase, question.clone(), options.clone()),
        user_input_tx,
    );

    update_project_runtime_state(state, &project.id, |runtime, _project| {
        record_runtime_project_ask_event(
            runtime,
            RuntimeProjectAsk {
                ask_id: &ask_id,
                run_id,
                phase,
                worker_id: &worker_id,
                agent_id: &agent_id_string,
                worker_role: spec.role,
                question: &question,
                options: options.as_deref(),
            },
        );
        runtime["updated_at"] = serde_json::json!(Utc::now().to_rfc3339());
    })
    .await?;

    publish_project_worker_ask_user(
        state, agent_id, ask_id, project, spec, phase, question, options,
    )
    .await;
    Ok(())
}

fn project_ask_registration(
    project: &project::Project,
    spec: &RuntimeWorkerSpec,
    phase: &str,
    question: String,
    options: Option<Vec<String>>,
) -> ProjectAskRegistration {
    ProjectAskRegistration {
        project_id: project.id.clone(),
        project_slug: project.slug.clone(),
        project_name: project.name.clone(),
        phase: phase.to_string(),
        worker_role: spec.role.to_string(),
        question,
        options,
    }
}

#[allow(clippy::too_many_arguments)]
async fn publish_project_worker_ask_user(
    state: &AppState,
    agent_id: AgentId,
    ask_id: String,
    project: &project::Project,
    spec: &RuntimeWorkerSpec,
    phase: &str,
    question: String,
    options: Option<Vec<String>>,
) {
    crate::chat_broadcast_publish(
        &state.kernel.event_bus,
        agent_id,
        project_ask_chat_event(agent_id, ask_id, project, spec, phase, question, options),
    )
    .await;
}

fn project_ask_chat_event(
    agent_id: AgentId,
    ask_id: String,
    project: &project::Project,
    spec: &RuntimeWorkerSpec,
    phase: &str,
    question: String,
    options: Option<Vec<String>>,
) -> ChatStreamEvent {
    ChatStreamEvent::ProjectAskUser {
        agent_id,
        ask_id,
        project_id: project.id.clone(),
        project_slug: project.slug.clone(),
        project_name: project.name.clone(),
        phase: phase.to_string(),
        worker_role: spec.role.to_string(),
        question,
        options,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::project_runtime_workers::RUNTIME_WORKER_SPECS;
    use captain_memory::project::ProjectStatus;

    fn project_fixture() -> project::Project {
        project::Project {
            id: "project-1".to_string(),
            name: "Demo Project".to_string(),
            slug: "demo-project".to_string(),
            goal: "Ship safely".to_string(),
            status: ProjectStatus::Active,
            deadline: None,
            created_at: 0,
            updated_at: 0,
            metadata: serde_json::json!({}),
        }
    }

    #[test]
    fn project_ask_registration_carries_worker_context() {
        let project = project_fixture();
        let spec = &RUNTIME_WORKER_SPECS[2];

        let registration = project_ask_registration(
            &project,
            spec,
            "plan",
            "Choose a path?".to_string(),
            Some(vec!["A".to_string(), "B".to_string()]),
        );

        assert_eq!(registration.project_id, "project-1");
        assert_eq!(registration.project_slug, "demo-project");
        assert_eq!(registration.project_name, "Demo Project");
        assert_eq!(registration.phase, "plan");
        assert_eq!(registration.worker_role, "planner");
        assert_eq!(registration.question, "Choose a path?");
        assert_eq!(
            registration.options.as_deref(),
            Some(&["A".to_string(), "B".to_string()][..])
        );
    }

    #[test]
    fn project_ask_chat_event_matches_registered_project_question() {
        let project = project_fixture();
        let spec = &RUNTIME_WORKER_SPECS[3];
        let agent_id = AgentId::new();

        let event = project_ask_chat_event(
            agent_id,
            "ask-1".to_string(),
            &project,
            spec,
            "build",
            "Continue?".to_string(),
            None,
        );

        match event {
            ChatStreamEvent::ProjectAskUser {
                agent_id: emitted_agent_id,
                ask_id,
                project_id,
                project_slug,
                phase,
                worker_role,
                question,
                options,
                ..
            } => {
                assert_eq!(emitted_agent_id, agent_id);
                assert_eq!(ask_id, "ask-1");
                assert_eq!(project_id, "project-1");
                assert_eq!(project_slug, "demo-project");
                assert_eq!(phase, "build");
                assert_eq!(worker_role, "builder");
                assert_eq!(question, "Continue?");
                assert!(options.is_none());
            }
            other => panic!("unexpected event: {other:?}"),
        }
    }
}
