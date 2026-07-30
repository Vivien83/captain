//! Production worker for the durable Skill Learning V2 pre-approval pipeline.

use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use captain_memory::workflow_learning::WorkflowEpisodeStore;
use captain_memory::workflow_learning_control::WorkflowLearningStore;
use captain_memory::workflow_learning_status::WorkflowLearningWorkerHeartbeat;
use captain_runtime::workflow_learning_engine::{
    WorkflowJobRunOutcome, WorkflowLearningEngine, WorkflowLearningEngineConfig,
};
use captain_runtime::workflow_learning_proposer::ActiveModelIdentity;
use captain_runtime::workflow_learning_staging::WorkflowStagingRoot;
use captain_types::config::LearningMode;
use captain_types::workflow_learning::{
    WorkflowLearningModelIdentity, WorkflowLearningWorkerPhase,
};
use tracing::{debug, info, warn};

use super::CaptainKernel;

const SCAN_INTERVAL: Duration = Duration::from_secs(60);
const RECOVERY_INTERVAL: Duration = Duration::from_secs(30);
const IDLE_DELAY: Duration = Duration::from_secs(2);
const ACTIVE_DELAY: Duration = Duration::from_millis(25);
const ERROR_DELAY: Duration = Duration::from_secs(15);
const HEARTBEAT_INTERVAL: Duration = Duration::from_secs(15);

pub(super) fn spawn_workflow_learning_worker(kernel: Arc<CaptainKernel>) {
    if !workflow_learning_enabled(kernel.config.skills.enabled, kernel.config.skills.mode) {
        info!(
            enabled = kernel.config.skills.enabled,
            mode = ?kernel.config.skills.mode,
            "workflow learning V2 worker disabled by configuration"
        );
        return;
    }
    tokio::spawn(run_workflow_learning_worker(kernel));
}

async fn run_workflow_learning_worker(kernel: Arc<CaptainKernel>) {
    let episodes = WorkflowEpisodeStore::new(kernel.memory.usage_conn());
    let control = WorkflowLearningStore::new(kernel.memory.usage_conn());
    let staging = match WorkflowStagingRoot::new(kernel.config.home_dir.clone()) {
        Ok(staging) => staging,
        Err(error) => {
            warn!(error = %error, "workflow learning V2 staging root is unavailable");
            return;
        }
    };
    let engine_config = worker_config(kernel.config.skills.rate_limit_per_day);
    let started_at_unix_ms = chrono::Utc::now().timestamp_millis();
    let mut health = WorkerHealthReporter::new(
        control.clone(),
        engine_config.worker_id.clone(),
        started_at_unix_ms,
    );
    health.persist_starting(started_at_unix_ms);
    let mut engine: Option<(ActiveModelIdentity, WorkflowLearningEngine)> = None;
    let mut next_scan = Instant::now();
    let mut next_recovery = Instant::now();
    let mut errors = ErrorLatch::default();

    loop {
        let requested_model = kernel.workflow_learning_active_model();
        let model_changed = engine
            .as_ref()
            .is_none_or(|(active_model, _)| active_model != &requested_model);
        if model_changed {
            engine = None;
            health.mark_starting(chrono::Utc::now().timestamp_millis());
            match build_engine(&kernel, &episodes, &control, &staging, &engine_config) {
                Ok((active_model, built)) => {
                    errors.clear("model");
                    info!(
                        provider = %active_model.provider,
                        model = %active_model.model,
                        "workflow learning V2 worker bound to active model"
                    );
                    health.bind_model(
                        &active_model,
                        &errors,
                        chrono::Utc::now().timestamp_millis(),
                    );
                    engine = Some((active_model, built));
                }
                Err(error) => {
                    errors.report("model", error);
                    health.persist(&errors, chrono::Utc::now().timestamp_millis(), true);
                    tokio::time::sleep(ERROR_DELAY).await;
                    continue;
                }
            }
        }
        let Some((_, active_engine)) = engine.as_ref() else {
            tokio::time::sleep(ERROR_DELAY).await;
            continue;
        };

        let now_unix_ms = chrono::Utc::now().timestamp_millis();
        if Instant::now() >= next_recovery {
            reconcile_worker_state(&control, active_engine, now_unix_ms, &mut errors);
            next_recovery = Instant::now() + RECOVERY_INTERVAL;
        }
        if Instant::now() >= next_scan {
            match active_engine.scan_once(now_unix_ms) {
                Ok(summary) => {
                    errors.clear("scan");
                    health.mark_scan(now_unix_ms);
                    if summary.proposals_created > 0
                        || summary.linked_existing > 0
                        || summary.rejected > 0
                    {
                        health.mark_progress(now_unix_ms);
                        info!(
                            episodes_seen = summary.episodes_seen,
                            rejected = summary.rejected,
                            deferred = summary.deferred,
                            linked_existing = summary.linked_existing,
                            proposals_created = summary.proposals_created,
                            "workflow learning V2 scan committed"
                        );
                    } else if summary.episodes_seen > 0 {
                        debug!(
                            episodes_seen = summary.episodes_seen,
                            deferred = summary.deferred,
                            "workflow learning V2 evidence remains below proposal threshold"
                        );
                    }
                }
                Err(error) => errors.report("scan", error),
            }
            next_scan = Instant::now() + SCAN_INTERVAL;
        }

        let mut force_heartbeat = false;
        let delay = match active_engine.run_next_job(now_unix_ms).await {
            Ok(WorkflowJobRunOutcome::Idle) => {
                errors.clear("job");
                IDLE_DELAY
            }
            Ok(WorkflowJobRunOutcome::Advanced {
                kind,
                job_id,
                proposal_id,
            }) => {
                errors.clear("job");
                health.mark_progress(now_unix_ms);
                force_heartbeat = true;
                info!(
                    job_kind = kind.as_str(),
                    job_id, proposal_id, "workflow learning V2 job advanced"
                );
                ACTIVE_DELAY
            }
            Ok(WorkflowJobRunOutcome::Retrying {
                kind,
                job_id,
                proposal_id,
            }) => {
                errors.clear("job");
                health.mark_progress(now_unix_ms);
                force_heartbeat = true;
                warn!(
                    job_kind = kind.as_str(),
                    job_id, proposal_id, "workflow learning V2 job scheduled a bounded retry"
                );
                IDLE_DELAY
            }
            Ok(WorkflowJobRunOutcome::Rejected {
                kind,
                job_id,
                proposal_id,
            }) => {
                errors.clear("job");
                health.mark_progress(now_unix_ms);
                force_heartbeat = true;
                info!(
                    job_kind = kind.as_str(),
                    job_id, proposal_id, "workflow learning V2 candidate rejected"
                );
                ACTIVE_DELAY
            }
            Err(error) => {
                errors.report("job", error);
                force_heartbeat = true;
                ERROR_DELAY
            }
        };
        health.persist(
            &errors,
            chrono::Utc::now().timestamp_millis(),
            force_heartbeat,
        );
        tokio::time::sleep(delay).await;
    }
}

fn build_engine(
    kernel: &CaptainKernel,
    episodes: &WorkflowEpisodeStore,
    control: &WorkflowLearningStore,
    staging: &WorkflowStagingRoot,
    config: &WorkflowLearningEngineConfig,
) -> Result<(ActiveModelIdentity, WorkflowLearningEngine), String> {
    let proposer = kernel.build_workflow_learning_proposer()?;
    let active_model = proposer.active_model().clone();
    let engine = WorkflowLearningEngine::new(
        episodes.clone(),
        control.clone(),
        proposer,
        staging.clone(),
        config.clone(),
    )
    .map_err(|error| error.to_string())?;
    Ok((active_model, engine))
}

fn reconcile_worker_state(
    control: &WorkflowLearningStore,
    engine: &WorkflowLearningEngine,
    now_unix_ms: i64,
    errors: &mut ErrorLatch,
) {
    match control.reconcile_expired_jobs(now_unix_ms) {
        Ok(summary) => {
            errors.clear("leases");
            if summary.uncertain_effects > 0 {
                warn!(
                    retried_without_effect = summary.retried_without_effect,
                    uncertain_effects = summary.uncertain_effects,
                    dead = summary.dead,
                    "workflow learning V2 blocked replay after an expired effect lease"
                );
            } else if summary.retried_without_effect > 0 || summary.dead > 0 {
                info!(
                    retried_without_effect = summary.retried_without_effect,
                    dead = summary.dead,
                    "workflow learning V2 reconciled expired job leases"
                );
            }
        }
        Err(error) => errors.report("leases", error),
    }
    match engine.recover_staged_drafts(now_unix_ms) {
        Ok(summary) => {
            if summary.unresolved > 0 || summary.blocked > 0 {
                errors.report(
                    "staging-recovery",
                    format!(
                        "recovered={} unresolved={} blocked={}",
                        summary.recovered, summary.unresolved, summary.blocked
                    ),
                );
            } else {
                errors.clear("staging-recovery");
                if summary.recovered > 0 {
                    info!(
                        recovered = summary.recovered,
                        "workflow learning V2 recovered immutable staged drafts"
                    );
                }
            }
        }
        Err(error) => errors.report("staging-recovery", error),
    }
}

pub(super) fn workflow_learning_enabled(enabled: bool, mode: LearningMode) -> bool {
    enabled && mode != LearningMode::Off
}

fn worker_config(daily_proposal_limit: u32) -> WorkflowLearningEngineConfig {
    WorkflowLearningEngineConfig {
        worker_id: format!("captain:workflow-learning-v2:{}", std::process::id()),
        daily_proposal_limit,
        ..WorkflowLearningEngineConfig::default()
    }
}

#[derive(Default)]
struct ErrorLatch {
    messages: BTreeMap<&'static str, String>,
}

impl ErrorLatch {
    fn report(&mut self, scope: &'static str, error: impl ToString) {
        let message = error.to_string();
        if self.messages.get(scope) != Some(&message) {
            warn!(scope = scope, error = %message, "workflow learning V2 worker error");
            self.messages.insert(scope, message);
        }
    }

    fn clear(&mut self, scope: &'static str) {
        if self.messages.remove(scope).is_some() {
            info!(scope = scope, "workflow learning V2 worker recovered");
        }
    }

    fn primary_scope(&self) -> Option<&'static str> {
        self.messages.keys().next().copied()
    }

    fn is_empty(&self) -> bool {
        self.messages.is_empty()
    }
}

struct WorkerHealthReporter {
    control: WorkflowLearningStore,
    heartbeat: WorkflowLearningWorkerHeartbeat,
    next_write: Instant,
    write_error_latched: bool,
}

impl WorkerHealthReporter {
    fn new(control: WorkflowLearningStore, worker_id: String, started_at_unix_ms: i64) -> Self {
        Self {
            control,
            heartbeat: WorkflowLearningWorkerHeartbeat {
                worker_id,
                phase: WorkflowLearningWorkerPhase::Starting,
                bound_model: None,
                started_at_unix_ms,
                heartbeat_at_unix_ms: started_at_unix_ms,
                last_scan_at_unix_ms: None,
                last_progress_at_unix_ms: None,
                last_error_scope: None,
            },
            next_write: Instant::now(),
            write_error_latched: false,
        }
    }

    fn persist_starting(&mut self, now_unix_ms: i64) {
        self.heartbeat.phase = WorkflowLearningWorkerPhase::Starting;
        self.heartbeat.bound_model = None;
        self.heartbeat.last_error_scope = None;
        self.write(now_unix_ms);
    }

    fn mark_starting(&mut self, now_unix_ms: i64) {
        self.heartbeat.phase = WorkflowLearningWorkerPhase::Starting;
        self.heartbeat.bound_model = None;
        self.heartbeat.last_error_scope = None;
        self.write(now_unix_ms);
    }

    fn bind_model(&mut self, model: &ActiveModelIdentity, errors: &ErrorLatch, now_unix_ms: i64) {
        self.heartbeat.bound_model = Some(WorkflowLearningModelIdentity {
            provider: model.provider.clone(),
            model: model.model.clone(),
        });
        self.persist(errors, now_unix_ms, true);
    }

    fn mark_scan(&mut self, now_unix_ms: i64) {
        self.heartbeat.last_scan_at_unix_ms = Some(now_unix_ms);
    }

    fn mark_progress(&mut self, now_unix_ms: i64) {
        self.heartbeat.last_progress_at_unix_ms = Some(now_unix_ms);
    }

    fn persist(&mut self, errors: &ErrorLatch, now_unix_ms: i64, force: bool) {
        if !force && Instant::now() < self.next_write {
            return;
        }
        self.heartbeat.phase = if errors.is_empty() {
            WorkflowLearningWorkerPhase::Running
        } else {
            WorkflowLearningWorkerPhase::Degraded
        };
        self.heartbeat.last_error_scope = errors.primary_scope().map(str::to_string);
        self.write(now_unix_ms);
    }

    fn write(&mut self, now_unix_ms: i64) {
        self.heartbeat.heartbeat_at_unix_ms = now_unix_ms
            .max(self.heartbeat.started_at_unix_ms)
            .max(self.heartbeat.last_scan_at_unix_ms.unwrap_or_default())
            .max(self.heartbeat.last_progress_at_unix_ms.unwrap_or_default());
        match self.control.record_worker_heartbeat(&self.heartbeat) {
            Ok(()) => {
                if self.write_error_latched {
                    info!("workflow learning V2 heartbeat persistence recovered");
                }
                self.write_error_latched = false;
            }
            Err(error) => {
                if !self.write_error_latched {
                    warn!(error = %error, "workflow learning V2 heartbeat persistence failed");
                }
                self.write_error_latched = true;
            }
        }
        self.next_write = Instant::now() + HEARTBEAT_INTERVAL;
    }
}

#[cfg(test)]
mod tests {
    use super::{worker_config, workflow_learning_enabled, WorkerHealthReporter};
    use captain_memory::workflow_learning_control::WorkflowLearningStore;
    use captain_memory::MemorySubstrate;
    use captain_types::config::LearningMode;

    #[test]
    fn worker_runs_only_when_skills_are_enabled_and_not_off() {
        assert!(workflow_learning_enabled(true, LearningMode::Approval));
        assert!(workflow_learning_enabled(true, LearningMode::Auto));
        assert!(!workflow_learning_enabled(true, LearningMode::Off));
        assert!(!workflow_learning_enabled(false, LearningMode::Approval));
    }

    #[test]
    fn worker_uses_the_configured_daily_limit_and_process_scoped_owner() {
        let config = worker_config(7);
        assert_eq!(config.daily_proposal_limit, 7);
        assert!(config
            .worker_id
            .starts_with("captain:workflow-learning-v2:"));
        assert_eq!(config.lease_duration_ms, 120_000);
    }

    #[test]
    fn heartbeat_remains_consistent_when_wall_clock_moves_backwards() {
        let memory = MemorySubstrate::open_in_memory(0.01).unwrap();
        let store = WorkflowLearningStore::new(memory.usage_conn());
        let mut reporter = WorkerHealthReporter::new(store.clone(), "worker:test".into(), 1_000);
        reporter.mark_scan(2_000);
        reporter.mark_progress(2_500);
        reporter.write(1_500);

        let heartbeat = store.operational_snapshot().unwrap().worker.unwrap();
        assert_eq!(heartbeat.heartbeat_at_unix_ms, 2_500);
        assert_eq!(heartbeat.last_scan_at_unix_ms, Some(2_000));
        assert_eq!(heartbeat.last_progress_at_unix_ms, Some(2_500));
    }
}
