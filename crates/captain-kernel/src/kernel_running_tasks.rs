use crate::error::{KernelError, KernelResult};
use crate::kernel::CaptainKernel;
use captain_runtime::agent_loop::AgentLoopResult;
use captain_types::agent::AgentId;
use captain_types::error::CaptainError;
use futures::FutureExt;
use std::any::Any;
use std::future::Future;
use std::panic::AssertUnwindSafe;
use std::sync::Arc;
use tracing::{error, info};

/// Abort handle registered for the currently running stream of an agent.
///
/// `run_id` prevents an older finished task from removing the cancellation
/// handle of a newer run for the same agent.
#[derive(Clone)]
pub struct RunningTaskHandle {
    pub run_id: uuid::Uuid,
    pub abort_handle: tokio::task::AbortHandle,
    pub started_at: chrono::DateTime<chrono::Utc>,
}

pub(super) struct RunningTaskCleanup {
    kernel: Arc<CaptainKernel>,
    agent_id: AgentId,
    run_id: uuid::Uuid,
}

impl RunningTaskCleanup {
    pub(super) fn new(kernel: Arc<CaptainKernel>, agent_id: AgentId, run_id: uuid::Uuid) -> Self {
        Self {
            kernel,
            agent_id,
            run_id,
        }
    }
}

impl Drop for RunningTaskCleanup {
    fn drop(&mut self) {
        clear_running_task_entry(&self.kernel.running_tasks, self.agent_id, self.run_id);
    }
}

pub(super) fn clear_running_task_entry(
    tasks: &dashmap::DashMap<AgentId, RunningTaskHandle>,
    agent_id: AgentId,
    run_id: uuid::Uuid,
) {
    tasks.remove_if(&agent_id, |_, task| task.run_id == run_id);
}

impl CaptainKernel {
    /// Cancel an agent's currently running LLM task.
    pub fn stop_agent_run(&self, agent_id: AgentId) -> KernelResult<bool> {
        if let Some((_, task)) = self.running_tasks.remove(&agent_id) {
            task.abort_handle.abort();
            info!(agent_id = %agent_id, "Agent run cancelled");
            Ok(true)
        } else {
            Ok(false)
        }
    }

    pub(super) fn track_running_task(
        &self,
        agent_id: AgentId,
        run_id: uuid::Uuid,
        abort_handle: tokio::task::AbortHandle,
    ) {
        self.running_tasks.insert(
            agent_id,
            RunningTaskHandle {
                run_id,
                abort_handle,
                started_at: chrono::Utc::now(),
            },
        );
    }

    pub(super) fn clear_running_task(&self, agent_id: AgentId, run_id: uuid::Uuid) {
        clear_running_task_entry(&self.running_tasks, agent_id, run_id);
    }

    pub(super) fn spawn_supervised_agent_task<F>(
        self: &Arc<Self>,
        agent_id: AgentId,
        future: F,
    ) -> tokio::task::JoinHandle<KernelResult<AgentLoopResult>>
    where
        F: Future<Output = KernelResult<AgentLoopResult>> + Send + 'static,
    {
        let kernel = Arc::clone(self);
        tokio::spawn(
            async move { supervise_agent_future(&kernel.supervisor, agent_id, future).await },
        )
    }
}

async fn supervise_agent_future<F, T>(
    supervisor: &crate::supervisor::Supervisor,
    agent_id: AgentId,
    future: F,
) -> KernelResult<T>
where
    F: Future<Output = KernelResult<T>>,
{
    match AssertUnwindSafe(future).catch_unwind().await {
        Ok(result) => result,
        Err(payload) => {
            let detail = panic_payload_message(payload.as_ref());
            supervisor.record_panic();
            error!(agent_id = %agent_id, panic = %detail, "Supervised agent task panicked");
            Err(KernelError::Captain(CaptainError::Internal(
                "supervised agent task panicked".to_string(),
            )))
        }
    }
}

fn panic_payload_message(payload: &(dyn Any + Send)) -> String {
    if let Some(message) = payload.downcast_ref::<&str>() {
        (*message).to_string()
    } else if let Some(message) = payload.downcast_ref::<String>() {
        message.clone()
    } else {
        "non-string panic payload".to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn running_task_cleanup_is_scoped_to_run_id() {
        let tasks = dashmap::DashMap::new();
        let agent_id = AgentId::new();
        let old_run = uuid::Uuid::new_v4();
        let new_run = uuid::Uuid::new_v4();
        let handle = tokio::spawn(async {});

        tasks.insert(
            agent_id,
            RunningTaskHandle {
                run_id: new_run,
                abort_handle: handle.abort_handle(),
                started_at: chrono::Utc::now(),
            },
        );

        clear_running_task_entry(&tasks, agent_id, old_run);
        assert!(
            tasks.contains_key(&agent_id),
            "an older finished run must not clear a newer run handle"
        );

        clear_running_task_entry(&tasks, agent_id, new_run);
        assert!(!tasks.contains_key(&agent_id));
        let _ = handle.await;
    }

    #[tokio::test]
    async fn supervised_future_records_a_real_panic() {
        let supervisor = crate::supervisor::Supervisor::new();
        let agent_id = AgentId::new();
        let result: KernelResult<()> = supervise_agent_future(&supervisor, agent_id, async {
            panic!("intentional supervised panic");
        })
        .await;

        assert!(matches!(
            result,
            Err(KernelError::Captain(CaptainError::Internal(message)))
                if message == "supervised agent task panicked"
        ));
        assert_eq!(supervisor.panic_count(), 1);
        assert_eq!(supervisor.failure_count(), 0);
    }

    #[tokio::test]
    async fn supervised_future_keeps_normal_errors_out_of_panic_count() {
        let supervisor = crate::supervisor::Supervisor::new();
        let agent_id = AgentId::new();
        let result: KernelResult<()> = supervise_agent_future(&supervisor, agent_id, async {
            Err(KernelError::BootFailed("expected".to_string()))
        })
        .await;

        assert!(matches!(result, Err(KernelError::BootFailed(message)) if message == "expected"));
        assert_eq!(supervisor.panic_count(), 0);
    }
}
