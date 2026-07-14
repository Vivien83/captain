use captain_kernel::CaptainKernel;
use captain_runtime::audit::AuditAction;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tracing::{info, warn};

#[derive(Debug, Clone)]
struct ShutdownDrainSnapshot {
    trigger: String,
    initial_work: ActiveShutdownWork,
    current_work: ActiveShutdownWork,
    started_at: chrono::DateTime<chrono::Utc>,
    updated_at: chrono::DateTime<chrono::Utc>,
}

#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct ActiveShutdownWork {
    pub(crate) active_run_count: usize,
    pub(crate) active_process_count: usize,
}

impl ActiveShutdownWork {
    pub(crate) fn new(active_run_count: usize, active_process_count: usize) -> Self {
        Self {
            active_run_count,
            active_process_count,
        }
    }

    pub(crate) fn total_count(self) -> usize {
        self.active_run_count + self.active_process_count
    }

    pub(crate) fn is_empty(self) -> bool {
        self.total_count() == 0
    }
}

#[derive(Debug, Default)]
pub(crate) struct ShutdownDrainState {
    inner: std::sync::Mutex<Option<ShutdownDrainSnapshot>>,
}

static SHUTDOWN_DRAIN_STATE: std::sync::OnceLock<Arc<ShutdownDrainState>> =
    std::sync::OnceLock::new();

pub(crate) fn shutdown_drain_state() -> Arc<ShutdownDrainState> {
    SHUTDOWN_DRAIN_STATE
        .get_or_init(|| Arc::new(ShutdownDrainState::new()))
        .clone()
}

impl ShutdownDrainState {
    pub(crate) fn new() -> Self {
        Self::default()
    }

    pub(crate) fn mark_draining(&self, trigger: &str, work: ActiveShutdownWork) {
        let now = chrono::Utc::now();
        let snapshot = ShutdownDrainSnapshot {
            trigger: trigger.to_string(),
            initial_work: work,
            current_work: work,
            started_at: now,
            updated_at: now,
        };
        *self.inner.lock().expect("shutdown drain lock poisoned") = Some(snapshot);
    }

    pub(crate) fn update_active_work(&self, work: ActiveShutdownWork) {
        if work.is_empty() {
            self.clear();
            return;
        }
        let mut guard = self.inner.lock().expect("shutdown drain lock poisoned");
        if let Some(snapshot) = guard.as_mut() {
            snapshot.current_work = work;
            snapshot.updated_at = chrono::Utc::now();
        }
    }

    pub(crate) fn clear(&self) {
        *self.inner.lock().expect("shutdown drain lock poisoned") = None;
    }

    pub(crate) fn status_json(&self, work: ActiveShutdownWork) -> serde_json::Value {
        if work.is_empty() {
            self.clear();
            return idle_shutdown_status(work);
        }

        let mut guard = self.inner.lock().expect("shutdown drain lock poisoned");
        let Some(snapshot) = guard.as_mut() else {
            return idle_shutdown_status(work);
        };
        let now = chrono::Utc::now();
        snapshot.current_work = work;
        snapshot.updated_at = now;
        let age_seconds = now
            .signed_duration_since(snapshot.started_at)
            .num_seconds()
            .max(0);

        serde_json::json!({
            "status": "draining",
            "trigger": snapshot.trigger,
            "initial_active_work_count": snapshot.initial_work.total_count(),
            "initial_active_run_count": snapshot.initial_work.active_run_count,
            "initial_active_process_count": snapshot.initial_work.active_process_count,
            "active_work_count": snapshot.current_work.total_count(),
            "active_run_count": snapshot.current_work.active_run_count,
            "active_process_count": snapshot.current_work.active_process_count,
            "started_at": snapshot.started_at.to_rfc3339(),
            "updated_at": snapshot.updated_at.to_rfc3339(),
            "age_seconds": age_seconds,
            "operator_actions": shutdown_operator_actions(),
        })
    }
}

pub(crate) fn active_shutdown_work(kernel: &CaptainKernel) -> ActiveShutdownWork {
    ActiveShutdownWork::new(
        kernel.running_tasks.len(),
        kernel
            .process_manager
            .list_all()
            .into_iter()
            .filter(|process| process.alive)
            .count(),
    )
}

pub(crate) fn shutdown_deferred_body(work: ActiveShutdownWork) -> serde_json::Value {
    serde_json::json!({
        "status": "draining",
        "active_work_count": work.total_count(),
        "active_run_count": work.active_run_count,
        "active_process_count": work.active_process_count,
        "message": "Active work is still running; Captain will not stop a healthy active task.",
        "operator_actions": shutdown_operator_actions(),
    })
}

fn idle_shutdown_status(work: ActiveShutdownWork) -> serde_json::Value {
    serde_json::json!({
        "status": "idle",
        "active_work_count": work.total_count(),
        "active_run_count": work.active_run_count,
        "active_process_count": work.active_process_count,
        "operator_actions": [],
    })
}

fn shutdown_operator_actions() -> [&'static str; 5] {
    [
        "Run captain status to inspect active work.",
        "Run captain process list to inspect process IDs.",
        "Stop a blocking process with captain process kill <process_id> if intentional.",
        "Let agent runs finish or stop background processes intentionally.",
        "Retry captain stop after active runs finish.",
    ]
}

pub(crate) fn record_shutdown_deferred(
    kernel: &CaptainKernel,
    trigger: &str,
    work: ActiveShutdownWork,
) {
    kernel.audit_log.record(
        "system",
        AuditAction::ConfigChange,
        format!("{trigger} shutdown deferred because active work is running"),
        format!(
            "active_work_count={}, active_run_count={}, active_process_count={}",
            work.total_count(),
            work.active_run_count,
            work.active_process_count
        ),
    );
}

pub(crate) async fn wait_for_active_runs_before_shutdown(
    kernel: Arc<CaptainKernel>,
    trigger: &'static str,
    drain_state: Arc<ShutdownDrainState>,
) {
    let work = active_shutdown_work(&kernel);
    if work.is_empty() {
        drain_state.clear();
        return;
    }

    drain_state.mark_draining(trigger, work);
    record_shutdown_deferred(&kernel, trigger, work);
    warn!(
        trigger,
        active_run_count = work.active_run_count,
        active_process_count = work.active_process_count,
        "shutdown signal deferred while active work is running"
    );

    let mut last_status = Instant::now();
    loop {
        tokio::time::sleep(Duration::from_secs(1)).await;
        let work = active_shutdown_work(&kernel);
        if work.is_empty() {
            drain_state.clear();
            info!(trigger, "active work drained; continuing shutdown");
            return;
        }
        drain_state.update_active_work(work);
        if last_status.elapsed() >= Duration::from_secs(30) {
            warn!(
                trigger,
                active_run_count = work.active_run_count,
                active_process_count = work.active_process_count,
                "still waiting for active work before shutdown"
            );
            last_status = Instant::now();
        }
    }
}

pub(crate) async fn shutdown_signal(
    api_shutdown: Arc<tokio::sync::Notify>,
    kernel: Arc<CaptainKernel>,
    drain_state: Arc<ShutdownDrainState>,
) {
    #[cfg(unix)]
    {
        use tokio::signal::unix::{signal, SignalKind};
        let mut sigint = signal(SignalKind::interrupt()).expect("Failed to listen for SIGINT");
        let mut sigterm = signal(SignalKind::terminate()).expect("Failed to listen for SIGTERM");

        let signal = tokio::select! {
            _ = sigint.recv() => {
                info!("Received SIGINT (Ctrl+C), shutting down...");
                Some("SIGINT")
            }
            _ = sigterm.recv() => {
                info!("Received SIGTERM, shutting down...");
                Some("SIGTERM")
            }
            _ = api_shutdown.notified() => {
                info!("Shutdown requested via API, shutting down...");
                None
            }
        };
        if let Some(trigger) = signal {
            wait_for_active_runs_before_shutdown(kernel, trigger, drain_state).await;
        }
    }

    #[cfg(not(unix))]
    {
        let signal = tokio::select! {
            _ = tokio::signal::ctrl_c() => {
                info!("Ctrl+C received, shutting down...");
                Some("ctrl_c")
            }
            _ = api_shutdown.notified() => {
                info!("Shutdown requested via API, shutting down...");
                None
            }
        };
        if let Some(trigger) = signal {
            wait_for_active_runs_before_shutdown(kernel, trigger, drain_state).await;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shutdown_deferred_body_is_operator_safe() {
        let body = shutdown_deferred_body(ActiveShutdownWork::new(2, 1));

        assert_eq!(body["status"], serde_json::json!("draining"));
        assert_eq!(body["active_work_count"], serde_json::json!(3));
        assert_eq!(body["active_run_count"], serde_json::json!(2));
        assert_eq!(body["active_process_count"], serde_json::json!(1));
        assert!(body["message"].as_str().unwrap().contains("will not stop"));
        assert!(body.get("agent_id").is_none());
        assert!(body.get("prompt").is_none());
    }

    #[test]
    fn shutdown_drain_state_reports_and_clears_operator_status() {
        let drain = ShutdownDrainState::new();

        assert_eq!(
            drain.status_json(ActiveShutdownWork::new(1, 0))["status"],
            serde_json::json!("idle")
        );

        drain.mark_draining("SIGTERM", ActiveShutdownWork::new(2, 1));
        let draining = drain.status_json(ActiveShutdownWork::new(1, 1));
        assert_eq!(draining["status"], serde_json::json!("draining"));
        assert_eq!(draining["trigger"], serde_json::json!("SIGTERM"));
        assert_eq!(draining["initial_active_work_count"], serde_json::json!(3));
        assert_eq!(draining["initial_active_run_count"], serde_json::json!(2));
        assert_eq!(
            draining["initial_active_process_count"],
            serde_json::json!(1)
        );
        assert_eq!(draining["active_work_count"], serde_json::json!(2));
        assert_eq!(draining["active_run_count"], serde_json::json!(1));
        assert_eq!(draining["active_process_count"], serde_json::json!(1));
        assert!(draining["operator_actions"].as_array().unwrap().len() >= 2);

        let idle = drain.status_json(ActiveShutdownWork::default());
        assert_eq!(idle["status"], serde_json::json!("idle"));
        assert_eq!(idle["active_work_count"], serde_json::json!(0));
        assert_eq!(idle["active_run_count"], serde_json::json!(0));
    }
}
