use super::kernel_delivery_runtime::{cron_deliver_response, retry_due_cron_deliveries};
use super::CaptainKernel;
use captain_runtime::kernel_handle::KernelHandle;
use captain_types::agent::AgentId;
use captain_types::event::{Event, EventPayload, EventTarget};
use captain_types::scheduler::{CronAction, CronJob, WorkflowStep};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tracing::{debug, info, warn};

const CRON_TICK_SECS: u64 = 15;
const CRON_PERSIST_EVERY_TICKS: u32 = 20;

impl CaptainKernel {
    pub(super) fn spawn_cron_scheduler_loop(self: &Arc<Self>) {
        let kernel = Arc::clone(self);
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(Duration::from_secs(CRON_TICK_SECS));
            interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
            let mut persist_counter = 0u32;
            interval.tick().await;
            loop {
                interval.tick().await;
                if kernel.supervisor.is_shutting_down() {
                    let _ = kernel.cron_scheduler.persist();
                    break;
                }

                retry_due_cron_deliveries(&kernel).await;
                for job in kernel.cron_scheduler.due_jobs() {
                    run_due_cron_job(&kernel, job).await;
                }

                if cron_persist_tick(&mut persist_counter) {
                    if let Err(e) = kernel.cron_scheduler.persist() {
                        warn!("Cron persist failed: {e}");
                    }
                }
            }
        });

        if self.cron_scheduler.total_jobs() > 0 {
            info!(
                "Cron scheduler active with {} job(s)",
                self.cron_scheduler.total_jobs()
            );
        }
    }
}

async fn run_due_cron_job(kernel: &Arc<CaptainKernel>, job: CronJob) {
    let job_id = job.id;
    let agent_id = job.agent_id;
    let job_name = job.name.clone();

    match &job.action {
        CronAction::SystemEvent { text } => {
            debug!(job = %job_name, "Cron: firing system event");
            let payload_bytes = serde_json::to_vec(&serde_json::json!({
                "type": format!("cron.{job_name}"),
                "text": text,
                "job_id": job_id.to_string(),
            }))
            .unwrap_or_default();
            let event = Event::new(
                AgentId::new(),
                EventTarget::Broadcast,
                EventPayload::Custom(payload_bytes),
            );
            kernel.publish_event(event).await;
            kernel.cron_scheduler.record_success(job_id);
        }
        CronAction::AgentTurn {
            message,
            timeout_secs,
            ..
        } => {
            run_cron_agent_turn(
                kernel,
                job_id,
                agent_id,
                &job_name,
                message,
                *timeout_secs,
                &job.delivery,
            )
            .await;
        }
        CronAction::WorkflowRun {
            workflow_id,
            input,
            timeout_secs,
        } => {
            run_cron_workflow(
                kernel,
                job_id,
                agent_id,
                &job_name,
                workflow_id,
                input.clone().unwrap_or_default(),
                *timeout_secs,
                &job.delivery,
            )
            .await;
        }
        CronAction::InlineWorkflow { steps } => {
            run_cron_inline_workflow(kernel, job_id, agent_id, &job_name, steps, &job.delivery)
                .await;
        }
    }
}

async fn run_cron_agent_turn(
    kernel: &Arc<CaptainKernel>,
    job_id: captain_types::scheduler::CronJobId,
    agent_id: AgentId,
    job_name: &str,
    message: &str,
    timeout_secs: Option<u64>,
    delivery: &captain_types::scheduler::CronDelivery,
) {
    debug!(job = %job_name, agent = %agent_id, "Cron: firing agent turn");
    let inactivity_timeout = Duration::from_secs(
        timeout_secs.unwrap_or(crate::cron_agent_turn::DEFAULT_CRON_AGENT_INACTIVITY_TIMEOUT_SECS),
    );
    let kh: Arc<dyn KernelHandle> = kernel.clone();
    let cron_start = Instant::now();
    let (cron_status, cron_detail) =
        match crate::cron_agent_turn::run_agent_turn_with_inactivity_timeout(
            kernel,
            agent_id,
            message,
            kh,
            inactivity_timeout,
        )
        .await
        {
            Ok(result) if result.delivered_via_channel_tool() => {
                info!(job = %job_name, "Cron: agent already delivered via channel_send, skipping cron_deliver_response");
                kernel.cron_scheduler.record_success(job_id);
                ("ok".to_string(), String::new())
            }
            Ok(result) => {
                match cron_deliver_response(kernel, agent_id, &result.response, delivery).await {
                    Ok(()) => {
                        info!(job = %job_name, "Cron job completed successfully");
                        kernel.cron_scheduler.record_success(job_id);
                        ("ok".to_string(), String::new())
                    }
                    Err(e) => {
                        warn!(job = %job_name, error = %e, "Cron job delivery failed");
                        kernel
                            .cron_scheduler
                            .record_delivery_failure(job_id, &e, &result.response);
                        ("delivery_failed".to_string(), e.to_string())
                    }
                }
            }
            Err(crate::cron_agent_turn::CronAgentTurnError::Inactivity {
                idle_secs,
                limit_secs,
                last_activity,
            }) => {
                let detail = format!(
                    "idle for {idle_secs}s (limit {limit_secs}s); last activity: {last_activity}"
                );
                warn!(
                    job = %job_name,
                    idle_secs,
                    limit_secs,
                    last_activity = %last_activity,
                    "Cron job timed out after inactivity"
                );
                kernel.cron_scheduler.record_failure(job_id, &detail);
                ("timeout".to_string(), detail)
            }
            Err(e) => {
                let err_msg = format!("{e}");
                warn!(job = %job_name, error = %err_msg, "Cron job failed");
                kernel.cron_scheduler.record_failure(job_id, &err_msg);
                ("error".to_string(), err_msg)
            }
        };

    spawn_cron_execution_record(
        kernel,
        job_name.to_string(),
        job_id,
        agent_id,
        cron_start,
        cron_status,
        cron_detail,
    );
}

#[allow(clippy::too_many_arguments)]
async fn run_cron_workflow(
    kernel: &Arc<CaptainKernel>,
    job_id: captain_types::scheduler::CronJobId,
    agent_id: AgentId,
    job_name: &str,
    workflow_id: &str,
    input: String,
    timeout_secs: Option<u64>,
    delivery: &captain_types::scheduler::CronDelivery,
) {
    debug!(job = %job_name, workflow = %workflow_id, "Cron: firing workflow run");
    let Some(wf_id) = resolve_cron_workflow_id(kernel, workflow_id).await else {
        let err_msg = format!("workflow not found: {workflow_id}");
        warn!(job = %job_name, %err_msg);
        kernel.cron_scheduler.record_failure(job_id, &err_msg);
        return;
    };

    let inactivity_timeout = Duration::from_secs(
        timeout_secs.unwrap_or(crate::cron_agent_turn::DEFAULT_CRON_AGENT_INACTIVITY_TIMEOUT_SECS),
    );
    match kernel
        .run_workflow_with_inactivity_timeout(wf_id, input, inactivity_timeout)
        .await
    {
        Ok((_run_id, output)) => {
            match cron_deliver_response(kernel, agent_id, &output, delivery).await {
                Ok(()) => {
                    info!(job = %job_name, "Cron workflow completed");
                    kernel.cron_scheduler.record_success(job_id);
                }
                Err(e) => {
                    warn!(job = %job_name, error = %e, "Cron workflow delivery failed");
                    kernel
                        .cron_scheduler
                        .record_delivery_failure(job_id, &e, &output);
                }
            }
        }
        Err(e) => {
            let err_msg = format!("{e}");
            warn!(job = %job_name, error = %err_msg, "Cron workflow failed");
            kernel.cron_scheduler.record_failure(job_id, &err_msg);
        }
    }
}

async fn run_cron_inline_workflow(
    kernel: &Arc<CaptainKernel>,
    job_id: captain_types::scheduler::CronJobId,
    agent_id: AgentId,
    job_name: &str,
    steps: &[WorkflowStep],
    delivery: &captain_types::scheduler::CronDelivery,
) {
    debug!(job = %job_name, steps = steps.len(), "Cron: executing inline workflow");
    let now = chrono::Local::now();
    let today = now.format("%Y-%m-%d").to_string();
    let now_str = now.format("%Y-%m-%d %H:%M").to_string();
    let mut previous_output = String::new();
    let mut delivered_via_channel = false;

    for step in steps {
        let args = render_inline_step_args(&step.args, &today, &now_str, &previous_output);
        let kh: Arc<dyn KernelHandle> = kernel.clone();
        let step_tool = step.tool.clone();
        let result = captain_runtime::tool_runner::execute_tool(
            &uuid::Uuid::new_v4().to_string(),
            &step.tool,
            &args,
            Some(&kh),
            None,
            Some(&agent_id.to_string()),
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
        )
        .await;

        if result.is_error {
            warn!(job = %job_name, tool = %step_tool, error = %result.content, "Inline step failed");
            kernel
                .cron_scheduler
                .record_failure(job_id, "inline workflow step failed");
            return;
        }

        if step.pipe_output {
            previous_output = result.content.clone();
        }
        if step_tool == "channel_send" {
            delivered_via_channel = true;
        }
        debug!(job = %job_name, tool = %step_tool, "Inline step ok");
    }

    if delivered_via_channel {
        info!(job = %job_name, "Cron workflow: a step already delivered via channel_send, skipping cron_deliver_response");
        kernel.cron_scheduler.record_success(job_id);
    } else if !previous_output.is_empty() {
        match cron_deliver_response(kernel, agent_id, &previous_output, delivery).await {
            Ok(()) => kernel.cron_scheduler.record_success(job_id),
            Err(e) => {
                warn!(job = %job_name, error = %e, "Cron inline workflow delivery failed");
                kernel
                    .cron_scheduler
                    .record_delivery_failure(job_id, &e, &previous_output);
            }
        }
    } else {
        kernel.cron_scheduler.record_success(job_id);
    }
}

async fn resolve_cron_workflow_id(
    kernel: &Arc<CaptainKernel>,
    workflow_id: &str,
) -> Option<crate::workflow::WorkflowId> {
    if let Ok(uuid) = uuid::Uuid::parse_str(workflow_id) {
        return Some(crate::workflow::WorkflowId(uuid));
    }
    kernel
        .workflows
        .list_workflows()
        .await
        .into_iter()
        .find(|workflow| workflow.name == workflow_id)
        .map(|workflow| workflow.id)
}

fn spawn_cron_execution_record(
    kernel: &Arc<CaptainKernel>,
    job_name: String,
    job_id: captain_types::scheduler::CronJobId,
    agent_id: AgentId,
    started_at: Instant,
    status: String,
    detail: String,
) {
    let graph = kernel.graph_memory.clone();
    let duration_ms = started_at.elapsed().as_millis().to_string();
    let jid = job_id.to_string();
    let aid = agent_id.to_string();
    tokio::spawn(async move {
        let timestamp = chrono::Utc::now().timestamp_millis();
        let _ = graph.record_event(
            "_sys::cron_exec",
            &format!("{job_name}@{timestamp}"),
            vec![
                ("job_id", &jid),
                ("agent_id", &aid),
                ("status", &status),
                ("detail", &detail),
                ("duration_ms", &duration_ms),
            ],
            None,
        );
        let _ = graph.save();
    });
}

fn render_inline_step_args(
    args: &serde_json::Value,
    today: &str,
    now: &str,
    previous_output: &str,
) -> serde_json::Value {
    let rendered = args
        .to_string()
        .replace("{{today}}", today)
        .replace("{{now}}", now)
        .replace("{{previous_output}}", previous_output);
    serde_json::from_str(&rendered).unwrap_or_else(|_| args.clone())
}

fn cron_persist_tick(counter: &mut u32) -> bool {
    *counter += 1;
    if *counter >= CRON_PERSIST_EVERY_TICKS {
        *counter = 0;
        true
    } else {
        false
    }
}

#[cfg(test)]
#[path = "kernel_cron_runtime_tests.rs"]
mod tests;
