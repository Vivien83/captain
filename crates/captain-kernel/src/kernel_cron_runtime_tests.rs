use super::{cron_persist_tick, render_inline_step_args, run_due_cron_job};
use crate::kernel::CaptainKernel;
use crate::workflow::{Workflow, WorkflowId};
use async_trait::async_trait;
use captain_runtime::llm_driver::{CompletionRequest, CompletionResponse, LlmDriver, LlmError};
use captain_types::agent::AgentId;
use captain_types::config::KernelConfig;
use captain_types::event::EventPayload;
use captain_types::message::{ContentBlock, StopReason, TokenUsage};
use captain_types::scheduler::{
    CronAction, CronDelivery, CronJob, CronJobId, CronSchedule, WorkflowStep,
};
use chrono::Utc;
use std::sync::Arc;

struct StaticDriver {
    text: &'static str,
}

#[async_trait]
impl LlmDriver for StaticDriver {
    async fn complete(&self, _request: CompletionRequest) -> Result<CompletionResponse, LlmError> {
        Ok(CompletionResponse {
            content: vec![ContentBlock::Text {
                text: self.text.to_string(),
                provider_metadata: None,
            }],
            stop_reason: StopReason::EndTurn,
            tool_calls: Vec::new(),
            usage: TokenUsage {
                input_tokens: 1,
                output_tokens: 1,
                ..Default::default()
            },
        })
    }
}

fn boot_test_kernel(
    name: &str,
    driver_text: &'static str,
) -> (tempfile::TempDir, Arc<CaptainKernel>) {
    let tmp = tempfile::tempdir().unwrap();
    let home_dir = tmp.path().join(name);
    let config = KernelConfig {
        home_dir: home_dir.clone(),
        data_dir: home_dir.join("data"),
        ..KernelConfig::default()
    };
    let mut kernel = Arc::new(CaptainKernel::boot_with_config(config).expect("kernel boot"));
    Arc::get_mut(&mut kernel)
        .expect("kernel has no shared references yet")
        .default_driver = Arc::new(StaticDriver { text: driver_text });
    kernel.set_self_handle();
    (tmp, kernel)
}

fn principal_agent_id(kernel: &CaptainKernel) -> AgentId {
    kernel
        .registry
        .list()
        .into_iter()
        .find(|entry| entry.name == "captain")
        .or_else(|| kernel.registry.list().into_iter().next())
        .expect("kernel should boot with at least one agent")
        .id
}

fn cron_job(agent_id: AgentId, name: &str, action: CronAction, delivery: CronDelivery) -> CronJob {
    CronJob {
        id: CronJobId::new(),
        agent_id,
        name: name.to_string(),
        enabled: true,
        schedule: CronSchedule::Every { every_secs: 60 },
        action,
        delivery,
        created_at: Utc::now(),
        last_run: None,
        next_run: Some(Utc::now()),
    }
}

fn register_job(kernel: &CaptainKernel, job: CronJob) -> CronJob {
    let id = kernel
        .cron_scheduler
        .add_job(job, false)
        .expect("cron job should validate");
    kernel
        .cron_scheduler
        .get_job(id)
        .expect("registered job should be readable")
}

#[test]
fn inline_step_args_replace_date_time_and_previous_output() {
    let args = serde_json::json!({
        "body": "{{today}} {{now}} {{previous_output}}",
        "count": 1
    });

    assert_eq!(
        render_inline_step_args(&args, "2026-05-31", "2026-05-31 17:30", "done"),
        serde_json::json!({
            "body": "2026-05-31 2026-05-31 17:30 done",
            "count": 1
        })
    );
}

#[test]
fn cron_persist_tick_fires_every_twentieth_tick_and_resets() {
    let mut counter = 0;
    for _ in 0..19 {
        assert!(!cron_persist_tick(&mut counter));
    }
    assert_eq!(counter, 19);
    assert!(cron_persist_tick(&mut counter));
    assert_eq!(counter, 0);
}

#[tokio::test]
async fn system_event_branch_publishes_payload_and_marks_success() {
    let (_tmp, kernel) = boot_test_kernel("cron-system-event", "unused");
    let agent_id = principal_agent_id(&kernel);
    let job = register_job(
        &kernel,
        cron_job(
            agent_id,
            "daily_event",
            CronAction::SystemEvent {
                text: "wake up".to_string(),
            },
            CronDelivery::None,
        ),
    );

    run_due_cron_job(&kernel, job.clone()).await;

    let meta = kernel.cron_scheduler.get_meta(job.id).unwrap();
    assert_eq!(meta.last_status.as_deref(), Some("ok"));
    assert_eq!(meta.consecutive_errors, 0);

    let history = kernel.event_bus.history(10).await;
    let payload = history
        .iter()
        .find_map(|event| match &event.payload {
            EventPayload::Custom(bytes) => Some(bytes),
            _ => None,
        })
        .expect("system cron event should be published");
    let payload: serde_json::Value = serde_json::from_slice(payload).unwrap();
    assert_eq!(payload["type"], "cron.daily_event");
    assert_eq!(payload["text"], "wake up");
    assert_eq!(payload["job_id"], job.id.to_string());

    kernel.shutdown();
}

#[tokio::test]
async fn agent_turn_delivery_failure_preserves_job_error_budget() {
    let (_tmp, kernel) = boot_test_kernel("cron-agent-delivery", "cron agent output");
    let agent_id = principal_agent_id(&kernel);
    let job = register_job(
        &kernel,
        cron_job(
            agent_id,
            "agent_delivery",
            CronAction::AgentTurn {
                message: "just say cron ok".to_string(),
                model_override: None,
                timeout_secs: Some(10),
            },
            CronDelivery::Webhook {
                url: "http://127.0.0.1:1/hook".to_string(),
            },
        ),
    );

    run_due_cron_job(&kernel, job.clone()).await;

    let meta = kernel.cron_scheduler.get_meta(job.id).unwrap();
    assert_eq!(meta.last_status.as_deref(), Some("delivery_failed"));
    assert_eq!(meta.consecutive_errors, 0);
    assert!(meta.job.enabled);
    assert!(meta
        .last_delivery_error
        .as_deref()
        .unwrap_or_default()
        .contains("webhook blocked by SSRF guard"));
    assert_eq!(meta.dead_letters.len(), 1);
    assert_eq!(meta.redelivery_queue.len(), 1);

    kernel.shutdown();
}

#[tokio::test]
async fn workflow_run_by_name_marks_success_without_llm_dependency() {
    let (_tmp, kernel) = boot_test_kernel("cron-workflow-run", "unused");
    let agent_id = principal_agent_id(&kernel);
    let workflow_id = WorkflowId::new();
    kernel
        .register_workflow(Workflow {
            id: workflow_id,
            name: "Daily Flow".to_string(),
            description: "No-step cron proof".to_string(),
            steps: Vec::new(),
            graph: None,
            created_at: Utc::now(),
        })
        .await;
    let job = register_job(
        &kernel,
        cron_job(
            agent_id,
            "workflow_run",
            CronAction::WorkflowRun {
                workflow_id: "Daily Flow".to_string(),
                input: Some("workflow-output".to_string()),
                timeout_secs: Some(10),
            },
            CronDelivery::None,
        ),
    );

    run_due_cron_job(&kernel, job.clone()).await;

    let meta = kernel.cron_scheduler.get_meta(job.id).unwrap();
    assert_eq!(meta.last_status.as_deref(), Some("ok"));
    let completed = kernel.workflows.list_runs(Some("completed")).await;
    assert!(completed.iter().any(|run| {
        run.workflow_id == workflow_id && run.output.as_deref() == Some("workflow-output")
    }));

    kernel.shutdown();
}

#[tokio::test]
async fn inline_workflow_system_time_success_records_ok() {
    let (_tmp, kernel) = boot_test_kernel("cron-inline-success", "unused");
    let agent_id = principal_agent_id(&kernel);
    let job = register_job(
        &kernel,
        cron_job(
            agent_id,
            "inline_success",
            CronAction::InlineWorkflow {
                steps: vec![WorkflowStep {
                    tool: "system_time".to_string(),
                    args: serde_json::json!({}),
                    pipe_output: true,
                }],
            },
            CronDelivery::None,
        ),
    );

    run_due_cron_job(&kernel, job.clone()).await;

    let meta = kernel.cron_scheduler.get_meta(job.id).unwrap();
    assert_eq!(meta.last_status.as_deref(), Some("ok"));
    assert_eq!(meta.consecutive_errors, 0);

    kernel.shutdown();
}

#[tokio::test]
async fn inline_workflow_tool_error_records_failure() {
    let (_tmp, kernel) = boot_test_kernel("cron-inline-failure", "unused");
    let agent_id = principal_agent_id(&kernel);
    let job = register_job(
        &kernel,
        cron_job(
            agent_id,
            "inline_failure",
            CronAction::InlineWorkflow {
                steps: vec![WorkflowStep {
                    tool: "definitely_unknown_tool".to_string(),
                    args: serde_json::json!({}),
                    pipe_output: true,
                }],
            },
            CronDelivery::None,
        ),
    );

    run_due_cron_job(&kernel, job.clone()).await;

    let meta = kernel.cron_scheduler.get_meta(job.id).unwrap();
    assert!(matches!(
        meta.last_status.as_deref(),
        Some(status) if status.starts_with("error: inline workflow step failed")
    ));
    assert_eq!(meta.consecutive_errors, 1);

    kernel.shutdown();
}
