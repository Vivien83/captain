//! Cron job scheduler engine for the Captain kernel.
//!
//! Manages scheduled jobs (recurring and one-shot) across all agents.
//! This is separate from `scheduler.rs` which handles agent resource tracking.
//!
//! The scheduler stores jobs in a `DashMap` for concurrent access, persists
//! them to a JSON file on disk, and exposes methods for the kernel tick loop
//! to query due jobs and record outcomes.

use captain_types::agent::AgentId;
use captain_types::error::{CaptainError, CaptainResult};
use captain_types::scheduler::{CronAction, CronDelivery, CronJob, CronJobId, CronSchedule};
use chrono::{Duration, Utc};
use dashmap::DashMap;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use tracing::{debug, info, warn};

use crate::cron_delivery_queue::{
    push_redelivery, read_payload_file, remove_payload_file, write_payload_file, CronRedelivery,
};
use crate::delivery_reliability::{
    make_dead_letter, push_dead_letter, DeliveryDeadLetter, DeliveryFailure,
};

/// Maximum consecutive errors before a job is auto-disabled.
const MAX_CONSECUTIVE_ERRORS: u32 = 5;
/// Recurring cron jobs missed by more than this at daemon boot are advanced
/// instead of executed immediately. This avoids replay storms after days or
/// weeks offline while still allowing short restart catch-up.
const MAX_BOOT_CATCH_UP_LAG_SECS: i64 = 10 * 60;

// ---------------------------------------------------------------------------
// JobMeta — extra runtime state not stored in CronJob itself
// ---------------------------------------------------------------------------

/// Runtime metadata for a cron job that extends the base `CronJob` type.
///
/// The `CronJob` struct in `captain-types` is intentionally lean (no
/// `one_shot`, `last_status`, or error tracking). The scheduler tracks
/// these operational details separately.
/// A single execution record for a cron job.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JobRun {
    pub timestamp: chrono::DateTime<Utc>,
    pub status: String,
    pub detail: String,
    #[serde(default)]
    pub duration_ms: u64,
}

const MAX_RUN_HISTORY: usize = 20;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JobMeta {
    /// The underlying job definition.
    pub job: CronJob,
    /// Whether this job should be removed after a single successful execution.
    pub one_shot: bool,
    /// Human-readable status of the last execution (e.g. `"ok"` or `"error: ..."`).
    pub last_status: Option<String>,
    /// Last delivery-layer failure, separate from agent/job execution errors.
    #[serde(default)]
    pub last_delivery_error: Option<String>,
    /// Number of consecutive failed executions.
    pub consecutive_errors: u32,
    /// Ring buffer of last N executions.
    #[serde(default)]
    pub run_history: Vec<JobRun>,
    /// Bounded dead-letter history for outputs that could not be delivered.
    #[serde(default)]
    pub dead_letters: Vec<DeliveryDeadLetter>,
    /// Persistent redelivery queue metadata. Payload bodies live on disk.
    #[serde(default)]
    pub redelivery_queue: Vec<CronRedelivery>,
}

impl JobMeta {
    /// Wrap a `CronJob` with default metadata.
    pub fn new(job: CronJob, one_shot: bool) -> Self {
        Self {
            job,
            one_shot,
            last_status: None,
            last_delivery_error: None,
            consecutive_errors: 0,
            run_history: Vec::new(),
            dead_letters: Vec::new(),
            redelivery_queue: Vec::new(),
        }
    }
}

/// Partial update accepted by [`CronScheduler::update_job`].
///
/// `one_shot` lives in scheduler metadata, not in `CronJob`, so the patch
/// intentionally spans both layers while preserving the job id, owner, run
/// history, and creation timestamp.
#[derive(Debug, Clone, Default)]
pub struct CronJobPatch {
    pub name: Option<String>,
    pub schedule: Option<CronSchedule>,
    pub action: Option<CronAction>,
    pub delivery: Option<CronDelivery>,
    pub enabled: Option<bool>,
    pub one_shot: Option<bool>,
}

// ---------------------------------------------------------------------------
// CronScheduler
// ---------------------------------------------------------------------------

/// Cron job scheduler — manages scheduled jobs for all agents.
///
/// Thread-safe via `DashMap`. The kernel should call [`due_jobs`] on a
/// regular interval (e.g. every 10-30 seconds) to discover jobs that need
/// to fire, then call [`record_success`] or [`record_failure`] after
/// execution completes.
pub struct CronScheduler {
    /// All tracked jobs, keyed by their unique ID.
    jobs: DashMap<CronJobId, JobMeta>,
    /// Path to the persistence file (`<home>/cron_jobs.json`).
    persist_path: PathBuf,
    /// Global cap on total jobs across all agents (atomic for hot-reload).
    max_total_jobs: AtomicUsize,
    /// Default IANA timezone from config (e.g., "Europe/Paris").
    default_tz: String,
}

impl CronScheduler {
    /// Create a new scheduler.
    ///
    /// `home_dir` is the Captain data directory; jobs are persisted to
    /// `<home_dir>/cron_jobs.json`. `max_total_jobs` caps the total number
    /// of jobs across all agents.
    pub fn new(home_dir: &Path, max_total_jobs: usize) -> Self {
        Self {
            jobs: DashMap::new(),
            persist_path: home_dir.join("cron_jobs.json"),
            max_total_jobs: AtomicUsize::new(max_total_jobs),
            default_tz: "UTC".to_string(),
        }
    }

    /// Set the default timezone (called after config is loaded).
    pub fn set_default_tz(&mut self, tz: &str) {
        self.default_tz = tz.to_string();
    }

    /// Update the max total jobs limit (for hot-reload).
    pub fn set_max_total_jobs(&self, new_max: usize) {
        self.max_total_jobs.store(new_max, Ordering::Relaxed);
    }

    // -- Persistence --------------------------------------------------------

    /// Load persisted jobs from disk.
    ///
    /// Returns the number of jobs loaded. If the persistence file does not
    /// exist, returns `Ok(0)` without error.
    pub fn load(&self) -> CaptainResult<usize> {
        if !self.persist_path.exists() {
            return Ok(0);
        }
        let data = std::fs::read_to_string(&self.persist_path)
            .map_err(|e| CaptainError::Internal(format!("Failed to read cron jobs: {e}")))?;
        let metas: Vec<JobMeta> = serde_json::from_str(&data)
            .map_err(|e| CaptainError::Internal(format!("Failed to parse cron jobs: {e}")))?;
        let count = metas.len();
        let now = Utc::now();
        for meta in metas {
            let meta = normalize_loaded_job_schedule(meta, now, Some(&self.default_tz));
            self.jobs.insert(meta.job.id, meta);
        }
        info!(count, "Loaded cron jobs from disk");
        Ok(count)
    }

    /// Persist all jobs atomically and synchronize the committed file to disk.
    pub fn persist(&self) -> CaptainResult<()> {
        let metas: Vec<JobMeta> = self.jobs.iter().map(|r| r.value().clone()).collect();
        let data = serde_json::to_string_pretty(&metas)
            .map_err(|e| CaptainError::Internal(format!("Failed to serialize cron jobs: {e}")))?;
        captain_types::durable_fs::atomic_write(&self.persist_path, data.as_bytes())
            .map_err(|e| CaptainError::Internal(format!("Failed to persist cron jobs: {e}")))?;
        debug!(count = metas.len(), "Persisted cron jobs");
        Ok(())
    }

    // -- CRUD ---------------------------------------------------------------

    /// Add a new job. Validates fields, computes the initial `next_run`,
    /// and inserts it into the scheduler.
    ///
    /// `one_shot` controls whether the job is removed after a single
    /// successful execution.
    pub fn add_job(&self, mut job: CronJob, one_shot: bool) -> CaptainResult<CronJobId> {
        // Global limit
        let max_jobs = self.max_total_jobs.load(Ordering::Relaxed);
        if self.jobs.len() >= max_jobs {
            return Err(CaptainError::Internal(format!(
                "Global cron job limit reached ({})",
                max_jobs
            )));
        }

        // Per-agent count
        let agent_count = self
            .jobs
            .iter()
            .filter(|r| r.value().job.agent_id == job.agent_id)
            .count();

        // CronJob.validate returns Result<(), String>
        job.validate(agent_count)
            .map_err(CaptainError::InvalidInput)?;

        // Compute initial next_run
        job.next_run = Some(compute_next_run_after_with_tz(
            &job.schedule,
            Utc::now(),
            Some(&self.default_tz),
        ));

        let id = job.id;
        self.jobs.insert(id, JobMeta::new(job, one_shot));
        Ok(id)
    }

    /// Remove a job by ID. Returns the removed `CronJob`.
    pub fn remove_job(&self, id: CronJobId) -> CaptainResult<CronJob> {
        self.jobs
            .remove(&id)
            .map(|(_, meta)| meta.job)
            .ok_or_else(|| CaptainError::Internal(format!("Cron job {id} not found")))
    }

    /// Enable or disable a job. Re-enabling resets errors and recomputes
    /// `next_run`.
    pub fn set_enabled(&self, id: CronJobId, enabled: bool) -> CaptainResult<()> {
        match self.jobs.get_mut(&id) {
            Some(mut meta) => {
                meta.job.enabled = enabled;
                if enabled {
                    meta.consecutive_errors = 0;
                    meta.job.next_run = Some(compute_next_run_after_with_tz(
                        &meta.job.schedule,
                        Utc::now(),
                        Some(&self.default_tz),
                    ));
                }
                Ok(())
            }
            None => Err(CaptainError::Internal(format!("Cron job {id} not found"))),
        }
    }

    /// Update a job in place while preserving id, owner, created_at, last_run,
    /// last_status and run_history. The modified job is validated before it
    /// becomes visible; invalid patches are rolled back atomically.
    pub fn update_job(&self, id: CronJobId, patch: CronJobPatch) -> CaptainResult<CronJob> {
        let current = self
            .get_job(id)
            .ok_or_else(|| CaptainError::Internal(format!("Cron job {id} not found")))?;
        let agent_count_excluding_current = self
            .jobs
            .iter()
            .filter(|r| *r.key() != id && r.value().job.agent_id == current.agent_id)
            .count();
        let schedule_changed = patch.schedule.is_some();
        let enabled_changed = patch.enabled.is_some();

        let mut entry = self
            .jobs
            .get_mut(&id)
            .ok_or_else(|| CaptainError::Internal(format!("Cron job {id} not found")))?;
        let meta = entry.value_mut();
        let original = meta.clone();

        if let Some(name) = patch.name {
            meta.job.name = name;
        }
        if let Some(schedule) = patch.schedule {
            meta.job.schedule = schedule;
        }
        if let Some(action) = patch.action {
            meta.job.action = action;
        }
        if let Some(delivery) = patch.delivery {
            meta.job.delivery = delivery;
        }
        if let Some(enabled) = patch.enabled {
            meta.job.enabled = enabled;
            if enabled {
                meta.consecutive_errors = 0;
            }
        }
        if let Some(one_shot) = patch.one_shot {
            meta.one_shot = one_shot;
        }

        if let Err(e) = meta.job.validate(agent_count_excluding_current) {
            *meta = original;
            return Err(CaptainError::InvalidInput(e));
        }

        if meta.job.enabled && (schedule_changed || enabled_changed) {
            meta.job.next_run = Some(compute_next_run_after_with_tz(
                &meta.job.schedule,
                Utc::now(),
                Some(&self.default_tz),
            ));
        } else if enabled_changed && !meta.job.enabled {
            meta.job.next_run = None;
        }

        Ok(meta.job.clone())
    }

    // -- Queries ------------------------------------------------------------

    /// Get a single job by ID.
    pub fn get_job(&self, id: CronJobId) -> Option<CronJob> {
        self.jobs.get(&id).map(|r| r.value().job.clone())
    }

    /// Get the full metadata for a job (includes `one_shot`, `last_status`,
    /// `consecutive_errors`).
    pub fn get_meta(&self, id: CronJobId) -> Option<JobMeta> {
        self.jobs.get(&id).map(|r| r.value().clone())
    }

    /// List all jobs for a specific agent.
    pub fn list_jobs(&self, agent_id: AgentId) -> Vec<CronJob> {
        self.jobs
            .iter()
            .filter(|r| r.value().job.agent_id == agent_id)
            .map(|r| r.value().job.clone())
            .collect()
    }

    /// List all jobs across all agents.
    pub fn list_all_jobs(&self) -> Vec<CronJob> {
        self.jobs.iter().map(|r| r.value().job.clone()).collect()
    }

    /// List all jobs with full metadata (including run history).
    pub fn list_all_jobs_with_meta(&self) -> Vec<JobMeta> {
        self.jobs.iter().map(|r| r.value().clone()).collect()
    }

    /// Reassign all cron jobs from `old_agent_id` to `new_agent_id`.
    ///
    /// Used when a hand agent is respawned (e.g. after daemon restart) and
    /// gets a new UUID. Without this, persisted cron jobs would reference
    /// the stale old agent ID and fail silently.
    ///
    /// Returns the number of jobs reassigned.
    pub fn reassign_agent_jobs(&self, old_agent_id: AgentId, new_agent_id: AgentId) -> usize {
        let mut count = 0;
        for mut entry in self.jobs.iter_mut() {
            if entry.value().job.agent_id == old_agent_id {
                entry.value_mut().job.agent_id = new_agent_id;
                // Reset consecutive errors so the job gets a fresh start
                // with the new agent.
                entry.value_mut().consecutive_errors = 0;
                if !entry.value().job.enabled {
                    // Re-enable jobs that were auto-disabled due to the stale
                    // agent ID causing repeated failures.
                    if entry
                        .value()
                        .last_status
                        .as_deref()
                        .is_some_and(|s| s.contains("not found") || s.contains("No such agent"))
                    {
                        entry.value_mut().job.enabled = true;
                        entry.value_mut().job.next_run = Some(compute_next_run_after_with_tz(
                            &entry.value().job.schedule,
                            Utc::now(),
                            Some(&self.default_tz),
                        ));
                    }
                }
                count += 1;
            }
        }
        if count > 0 {
            info!(
                old_agent = %old_agent_id,
                new_agent = %new_agent_id,
                count,
                "Reassigned cron jobs to new agent"
            );
        }
        count
    }

    /// Remove all cron jobs belonging to a specific agent.
    ///
    /// Used when an agent is deleted so its cron entries don't linger as
    /// orphans pointing at a dead UUID. Returns the number of jobs removed.
    pub fn remove_agent_jobs(&self, agent_id: AgentId) -> usize {
        let ids: Vec<CronJobId> = self
            .jobs
            .iter()
            .filter(|r| r.value().job.agent_id == agent_id)
            .map(|r| *r.key())
            .collect();
        let count = ids.len();
        for id in ids {
            self.jobs.remove(&id);
        }
        if count > 0 {
            info!(agent = %agent_id, count, "Removed cron jobs for deleted agent");
        }
        count
    }

    /// Total number of tracked jobs.
    pub fn total_jobs(&self) -> usize {
        self.jobs.len()
    }

    /// Return jobs whose `next_run` is at or before `now` and are enabled.
    ///
    /// **Important**: This also pre-advances each due job's `next_run` to the
    /// next scheduled time. This prevents the same job from being returned as
    /// "due" on subsequent tick iterations while it's still executing.
    pub fn due_jobs(&self) -> Vec<CronJob> {
        let now = Utc::now();
        let mut due = Vec::new();
        for mut entry in self.jobs.iter_mut() {
            let meta = entry.value_mut();
            if meta.job.enabled && meta.job.next_run.map(|t| t <= now).unwrap_or(false) {
                due.push(meta.job.clone());
                // Pre-advance next_run so the job won't fire again on the next
                // tick while it's still executing. Use `now` as the base so the
                // next fire time is computed strictly after the current moment.
                meta.job.next_run = Some(compute_next_run_after_with_tz(
                    &meta.job.schedule,
                    now,
                    Some(&self.default_tz),
                ));
            }
        }
        due
    }

    // -- Outcome recording --------------------------------------------------

    /// Record a successful execution for a job.
    ///
    /// Updates `last_run`, resets errors, and either removes the job (if
    /// one-shot) or advances `next_run`.
    pub fn record_success(&self, id: CronJobId) {
        let should_remove = {
            if let Some(mut meta) = self.jobs.get_mut(&id) {
                let now = Utc::now();
                meta.job.last_run = Some(now);
                meta.last_status = Some("ok".to_string());
                meta.last_delivery_error = None;
                meta.consecutive_errors = 0;
                meta.run_history.push(JobRun {
                    timestamp: now,
                    status: "ok".to_string(),
                    detail: String::new(),
                    duration_ms: 0,
                });
                if meta.run_history.len() > MAX_RUN_HISTORY {
                    meta.run_history.remove(0);
                }
                meta.one_shot
            } else {
                return;
            }
        };
        if should_remove {
            self.jobs.remove(&id);
        }
    }

    /// Record a failed execution for a job.
    ///
    /// Increments the consecutive error counter. If it reaches
    /// [`MAX_CONSECUTIVE_ERRORS`], the job is automatically disabled.
    pub fn record_failure(&self, id: CronJobId, error_msg: &str) {
        if let Some(mut meta) = self.jobs.get_mut(&id) {
            let now = Utc::now();
            meta.job.last_run = Some(now);
            let truncated = captain_types::truncate_str(error_msg, 256);
            meta.last_status = Some(format!("error: {truncated}"));
            meta.consecutive_errors += 1;
            meta.run_history.push(JobRun {
                timestamp: now,
                status: "error".to_string(),
                detail: truncated.to_string(),
                duration_ms: 0,
            });
            if meta.run_history.len() > MAX_RUN_HISTORY {
                meta.run_history.remove(0);
            }
            if meta.consecutive_errors >= MAX_CONSECUTIVE_ERRORS {
                warn!(
                    job_id = %id,
                    errors = meta.consecutive_errors,
                    "Auto-disabling cron job after repeated failures"
                );
                meta.job.enabled = false;
            } else {
                meta.job.next_run = Some(compute_next_run_after_with_tz(
                    &meta.job.schedule,
                    Utc::now(),
                    Some(&self.default_tz),
                ));
            }
        }
    }

    /// Record a successful job execution whose output could not be delivered.
    ///
    /// Delivery failures are operational transport problems, not agent/job
    /// failures. They must remain visible without burning the job's
    /// consecutive-error budget or disabling recurring schedules.
    pub fn record_delivery_failure(&self, id: CronJobId, failure: &DeliveryFailure, payload: &str) {
        if let Some(mut meta) = self.jobs.get_mut(&id) {
            let now = Utc::now();
            meta.job.last_run = Some(now);
            meta.last_delivery_error = Some(failure.to_string());
            meta.last_status = Some("delivery_failed".to_string());
            meta.consecutive_errors = 0;
            meta.run_history.push(JobRun {
                timestamp: now,
                status: "delivery_failed".to_string(),
                detail: failure.to_string(),
                duration_ms: 0,
            });
            if meta.run_history.len() > MAX_RUN_HISTORY {
                meta.run_history.remove(0);
            }
            let entry = make_dead_letter(failure, payload, now);
            push_dead_letter(&mut meta.dead_letters, entry);
            match write_payload_file(&self.home_dir(), id, payload, now) {
                Ok(path) => {
                    let queued = CronRedelivery::new(
                        &meta.job,
                        meta.job.delivery.clone(),
                        failure,
                        path,
                        now,
                    );
                    let dropped_paths = push_redelivery(&mut meta.redelivery_queue, queued);
                    for path in dropped_paths {
                        remove_payload_file(&path);
                    }
                }
                Err(e) => {
                    warn!(job_id = %id, error = %e, "Failed to persist cron redelivery payload");
                }
            }
            if meta.one_shot {
                meta.job.enabled = false;
                meta.job.next_run = None;
            }
        }
    }

    /// Return due redelivery entries without mutating state.
    pub fn due_redeliveries(&self) -> Vec<CronRedelivery> {
        let now = Utc::now();
        self.jobs
            .iter()
            .flat_map(|entry| entry.value().redelivery_queue.clone())
            .filter(|entry| entry.next_attempt_at <= now)
            .collect()
    }

    /// Read a queued redelivery payload from its sidecar file.
    pub fn read_redelivery_payload(&self, queued: &CronRedelivery) -> Result<String, String> {
        read_payload_file(&queued.payload_path)
    }

    /// Mark a queued redelivery as delivered and clean up its payload file.
    pub fn record_redelivery_success(&self, job_id: CronJobId, redelivery_id: &str) {
        if let Some(mut meta) = self.jobs.get_mut(&job_id) {
            if let Some(index) = meta
                .redelivery_queue
                .iter()
                .position(|entry| entry.id == redelivery_id)
            {
                let entry = meta.redelivery_queue.remove(index);
                remove_payload_file(&entry.payload_path);
                meta.last_delivery_error = None;
                meta.last_status = Some("ok".to_string());
                meta.run_history.push(JobRun {
                    timestamp: Utc::now(),
                    status: "redelivery_ok".to_string(),
                    detail: entry.target,
                    duration_ms: 0,
                });
                if meta.run_history.len() > MAX_RUN_HISTORY {
                    meta.run_history.remove(0);
                }
            }
        }
    }

    /// Mark a queued redelivery as failed. Exhausted entries become dead letters.
    pub fn record_redelivery_failure(
        &self,
        job_id: CronJobId,
        redelivery_id: &str,
        failure: &DeliveryFailure,
        payload: &str,
    ) {
        if let Some(mut meta) = self.jobs.get_mut(&job_id) {
            let Some(index) = meta
                .redelivery_queue
                .iter()
                .position(|entry| entry.id == redelivery_id)
            else {
                return;
            };
            let now = Utc::now();
            let keep_queued = {
                let entry = &mut meta.redelivery_queue[index];
                entry.schedule_failure(failure, now)
            };
            meta.last_delivery_error = Some(failure.to_string());
            meta.last_status = Some("delivery_failed".to_string());
            meta.run_history.push(JobRun {
                timestamp: now,
                status: "redelivery_failed".to_string(),
                detail: failure.to_string(),
                duration_ms: 0,
            });
            if meta.run_history.len() > MAX_RUN_HISTORY {
                meta.run_history.remove(0);
            }
            if !keep_queued {
                let entry = meta.redelivery_queue.remove(index);
                remove_payload_file(&entry.payload_path);
                push_dead_letter(
                    &mut meta.dead_letters,
                    make_dead_letter(failure, payload, now),
                );
            }
        }
    }

    fn home_dir(&self) -> PathBuf {
        self.persist_path
            .parent()
            .map(Path::to_path_buf)
            .unwrap_or_else(|| PathBuf::from("."))
    }
}

fn normalize_loaded_job_schedule(
    mut meta: JobMeta,
    now: chrono::DateTime<Utc>,
    default_tz: Option<&str>,
) -> JobMeta {
    if !meta.job.enabled {
        return meta;
    }

    match meta.job.next_run {
        Some(next_run) if stale_recurring_run(&meta.job.schedule, next_run, now) => {
            meta.last_status = Some(format!(
                "skipped_stale_missed_run: next_run was {}",
                next_run.to_rfc3339()
            ));
            meta.job.next_run = Some(compute_next_run_after_with_tz(
                &meta.job.schedule,
                now,
                default_tz,
            ));
            debug!(
                job_id = %meta.job.id,
                job = %meta.job.name,
                previous_next_run = %next_run,
                "Skipped stale missed recurring cron run at boot"
            );
        }
        None if recurring_schedule(&meta.job.schedule) => {
            meta.job.next_run = Some(compute_next_run_after_with_tz(
                &meta.job.schedule,
                now,
                default_tz,
            ));
        }
        _ => {}
    }

    meta
}

fn stale_recurring_run(
    schedule: &CronSchedule,
    next_run: chrono::DateTime<Utc>,
    now: chrono::DateTime<Utc>,
) -> bool {
    if next_run > now {
        return false;
    }
    if !recurring_schedule(schedule) {
        return false;
    }
    now.signed_duration_since(next_run) > Duration::seconds(MAX_BOOT_CATCH_UP_LAG_SECS)
}

fn recurring_schedule(schedule: &CronSchedule) -> bool {
    matches!(
        schedule,
        CronSchedule::Every { .. } | CronSchedule::Cron { .. }
    )
}

// ---------------------------------------------------------------------------
// compute_next_run
// ---------------------------------------------------------------------------

/// Compute the next fire time for a schedule, based on `now`.
///
/// - `At { at }` — returns `at` directly.
/// - `Every { every_secs }` — returns `now + every_secs`.
/// - `Cron { expr, tz }` — parses the cron expression and computes the next
///   matching time. Supports standard 5-field (`min hour dom month dow`) and
///   6-field (`sec min hour dom month dow`) formats by converting to the
///   7-field format required by the `cron` crate.
pub fn compute_next_run(schedule: &CronSchedule) -> chrono::DateTime<Utc> {
    compute_next_run_after(schedule, Utc::now())
}

/// Compute the next fire time for a schedule, strictly after `after`.
///
/// Uses `after + 1 second` as the base time so the `cron` crate's
/// inclusive `.after()` always returns a strictly future time. Without
/// this offset, calling `compute_next_run` right after a job fires can
/// return the same minute (or even the same second), causing the
/// scheduler to re-fire immediately.
pub fn compute_next_run_after(
    schedule: &CronSchedule,
    after: chrono::DateTime<Utc>,
) -> chrono::DateTime<Utc> {
    compute_next_run_after_with_tz(schedule, after, None)
}

/// Like `compute_next_run_after` but applies a default timezone when the
/// schedule's `tz` field is empty. This ensures cron expressions like
/// `"0 12 * * *"` fire at 12:00 local time, not 12:00 UTC.
/// Normalize DOW field from Unix cron (0=Sun) to Rust cron crate (7=Sun, 1=Mon).
/// Handles ranges (0-6 → 7,1-6), lists (0,3 → 7,3), and standalone 0 → 7.
fn normalize_dow(dow: &str) -> String {
    if dow == "*" || dow == "?" {
        return dow.to_string();
    }
    // The Rust `cron` crate uses 1=SUN, 2=MON, ..., 7=SAT
    // Unix cron uses 0=SUN, 1=MON, ..., 6=SAT (and 7=SUN as alias)
    // Convert: 0→1 (SUN), 1→2 (MON), ..., 6→7 (SAT)
    dow.split(',')
        .map(|part| {
            let trimmed = part.trim();
            // Handle range like "1-5" → "2-6"
            if let Some((start, end)) = trimmed.split_once('-') {
                let s: i32 = start.parse().unwrap_or(-1);
                let e: i32 = end.parse().unwrap_or(-1);
                if s >= 0 && e >= 0 {
                    return format!("{}-{}", (s % 7) + 1, (e % 7) + 1);
                }
                return trimmed.to_string();
            }
            // Handle single number
            if let Ok(n) = trimmed.parse::<i32>() {
                return ((n % 7) + 1).to_string();
            }
            // Already a name (SUN, MON, etc.) — pass through
            trimmed.to_string()
        })
        .collect::<Vec<_>>()
        .join(",")
}

pub fn compute_next_run_after_with_tz(
    schedule: &CronSchedule,
    after: chrono::DateTime<Utc>,
    default_tz: Option<&str>,
) -> chrono::DateTime<Utc> {
    match schedule {
        CronSchedule::At { at } => *at,
        CronSchedule::Every { every_secs } => after + Duration::seconds(*every_secs as i64),
        CronSchedule::Cron { expr, tz } => {
            // Use job-specific tz, fall back to config default, then UTC
            let effective_tz = tz
                .as_deref()
                .filter(|s| !s.is_empty())
                .or(default_tz)
                .filter(|s| !s.is_empty() && *s != "UTC");
            // Convert standard 5/6-field cron to 7-field for the `cron` crate.
            // Standard 5-field: min hour dom month dow
            // 6-field:          sec min hour dom month dow
            // cron crate:       sec min hour dom month dow year
            //
            // IMPORTANT: The Rust `cron` crate uses 1-7 for DOW (Mon=1, Sun=7)
            // but standard Unix cron uses 0=Sunday. We must convert 0→7 in the
            // DOW field to avoid treating Sunday as a wildcard.
            let trimmed = expr.trim();
            let fields: Vec<&str> = trimmed.split_whitespace().collect();
            let seven_field = match fields.len() {
                5 => {
                    // Normalize DOW: replace standalone "0" with "7" (Sunday)
                    let dow = fields[4];
                    let fixed_dow = normalize_dow(dow);
                    format!(
                        "0 {} {} {} {} {} *",
                        fields[0], fields[1], fields[2], fields[3], fixed_dow
                    )
                }
                6 => {
                    let dow = fields[5];
                    let fixed_dow = normalize_dow(dow);
                    format!(
                        "{} {} {} {} {} {} *",
                        fields[0], fields[1], fields[2], fields[3], fields[4], fixed_dow
                    )
                }
                _ => expr.clone(),
            };

            // Add 1 second so `.after()` (inclusive) skips the current second.
            let base = after + Duration::seconds(1);

            match seven_field.parse::<cron::Schedule>() {
                Ok(sched) => {
                    // If a timezone is specified, compute the next fire time in
                    // that timezone so DST and local offsets are respected, then
                    // convert back to UTC for storage.
                    let next_utc = match effective_tz {
                        Some(tz_str) => match tz_str.parse::<chrono_tz::Tz>() {
                            Ok(timezone) => {
                                let base_local = base.with_timezone(&timezone);
                                sched
                                    .after(&base_local)
                                    .next()
                                    .map(|dt| dt.with_timezone(&Utc))
                            }
                            Err(_) => {
                                warn!(
                                    "Invalid timezone '{}' in cron job, falling back to UTC",
                                    tz_str
                                );
                                sched.after(&base).next()
                            }
                        },
                        _ => sched.after(&base).next(),
                    };
                    next_utc.unwrap_or_else(|| after + Duration::hours(1))
                }
                Err(e) => {
                    warn!("Failed to parse cron expression '{}': {}", expr, e);
                    after + Duration::hours(1)
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use captain_types::scheduler::{CronAction, CronDelivery};
    use chrono::{Duration, Timelike};

    /// Build a minimal valid `CronJob` with an `Every` schedule.
    fn make_job(agent_id: AgentId) -> CronJob {
        CronJob {
            id: CronJobId::new(),
            agent_id,
            name: "test-job".into(),
            enabled: true,
            schedule: CronSchedule::Every { every_secs: 3600 },
            action: CronAction::SystemEvent {
                text: "ping".into(),
            },
            delivery: CronDelivery::None,
            created_at: Utc::now(),
            last_run: None,
            next_run: None,
        }
    }

    /// Create a scheduler backed by a temp directory.
    fn make_scheduler(max_total: usize) -> (CronScheduler, tempfile::TempDir) {
        let tmp = tempfile::tempdir().unwrap();
        let sched = CronScheduler::new(tmp.path(), max_total);
        (sched, tmp)
    }

    // -- test_add_job_and_list ----------------------------------------------

    #[test]
    fn test_add_job_and_list() {
        let (sched, _tmp) = make_scheduler(100);
        let agent = AgentId::new();
        let job = make_job(agent);

        let id = sched.add_job(job, false).unwrap();

        // Should appear in agent list
        let jobs = sched.list_jobs(agent);
        assert_eq!(jobs.len(), 1);
        assert_eq!(jobs[0].id, id);
        assert_eq!(jobs[0].name, "test-job");

        // Should appear in global list
        let all = sched.list_all_jobs();
        assert_eq!(all.len(), 1);

        // get_job should return it
        let fetched = sched.get_job(id).unwrap();
        assert_eq!(fetched.agent_id, agent);

        // next_run should have been computed
        assert!(fetched.next_run.is_some());
        assert_eq!(sched.total_jobs(), 1);
    }

    // -- test_remove_job ----------------------------------------------------

    #[test]
    fn test_remove_job() {
        let (sched, _tmp) = make_scheduler(100);
        let agent = AgentId::new();
        let job = make_job(agent);
        let id = sched.add_job(job, false).unwrap();

        let removed = sched.remove_job(id).unwrap();
        assert_eq!(removed.name, "test-job");
        assert_eq!(sched.total_jobs(), 0);

        // Removing again should fail
        assert!(sched.remove_job(id).is_err());
    }

    #[test]
    fn test_update_job_preserves_identity_and_history() {
        let (sched, _tmp) = make_scheduler(100);
        let agent = AgentId::new();
        let job = make_job(agent);
        let id = sched.add_job(job, false).unwrap();
        let created_at = sched.get_job(id).unwrap().created_at;

        sched.record_success(id);
        let updated = sched
            .update_job(
                id,
                CronJobPatch {
                    name: Some("updated-job".into()),
                    schedule: Some(CronSchedule::Every { every_secs: 7200 }),
                    one_shot: Some(true),
                    ..Default::default()
                },
            )
            .unwrap();

        assert_eq!(updated.id, id);
        assert_eq!(updated.agent_id, agent);
        assert_eq!(updated.created_at, created_at);
        assert_eq!(updated.name, "updated-job");
        assert!(matches!(
            updated.schedule,
            CronSchedule::Every { every_secs: 7200 }
        ));
        assert!(updated.next_run.is_some());

        let meta = sched.get_meta(id).unwrap();
        assert!(meta.one_shot);
        assert_eq!(meta.last_status.as_deref(), Some("ok"));
        assert_eq!(meta.run_history.len(), 1);
    }

    #[test]
    fn test_update_job_rolls_back_invalid_patch() {
        let (sched, _tmp) = make_scheduler(100);
        let agent = AgentId::new();
        let id = sched.add_job(make_job(agent), false).unwrap();

        let err = sched
            .update_job(
                id,
                CronJobPatch {
                    name: Some(String::new()),
                    one_shot: Some(true),
                    ..Default::default()
                },
            )
            .unwrap_err();
        assert!(err.to_string().contains("name"));

        let meta = sched.get_meta(id).unwrap();
        assert_eq!(meta.job.name, "test-job");
        assert!(!meta.one_shot);
    }

    // -- test_add_job_global_limit ------------------------------------------

    #[test]
    fn test_add_job_global_limit() {
        let (sched, _tmp) = make_scheduler(2);
        let agent = AgentId::new();

        let j1 = make_job(agent);
        let j2 = make_job(agent);
        let j3 = make_job(agent);

        sched.add_job(j1, false).unwrap();
        sched.add_job(j2, false).unwrap();

        // Third should hit global limit
        let err = sched.add_job(j3, false).unwrap_err();
        let msg = err.to_string();
        assert!(
            msg.contains("limit"),
            "Expected global limit error, got: {msg}"
        );
    }

    // -- test_add_job_per_agent_limit ---------------------------------------

    #[test]
    fn test_add_job_per_agent_limit() {
        // MAX_JOBS_PER_AGENT = 50 in captain-types
        let (sched, _tmp) = make_scheduler(1000);
        let agent = AgentId::new();

        for i in 0..50 {
            let mut job = make_job(agent);
            job.name = format!("job-{i}");
            sched.add_job(job, false).unwrap();
        }

        // 51st should be rejected by validate()
        let mut overflow = make_job(agent);
        overflow.name = "overflow".into();
        let err = sched.add_job(overflow, false).unwrap_err();
        let msg = err.to_string();
        assert!(
            msg.contains("50"),
            "Expected per-agent limit error, got: {msg}"
        );
    }

    // -- test_record_success_removes_one_shot --------------------------------

    #[test]
    fn test_record_success_removes_one_shot() {
        let (sched, _tmp) = make_scheduler(100);
        let agent = AgentId::new();
        let job = make_job(agent);
        let id = sched.add_job(job, true).unwrap(); // one_shot = true

        assert_eq!(sched.total_jobs(), 1);

        sched.record_success(id);

        // One-shot job should have been removed
        assert_eq!(sched.total_jobs(), 0);
        assert!(sched.get_job(id).is_none());
    }

    #[test]
    fn test_record_success_keeps_recurring() {
        let (sched, _tmp) = make_scheduler(100);
        let agent = AgentId::new();
        let job = make_job(agent);
        let id = sched.add_job(job, false).unwrap(); // one_shot = false

        sched.record_success(id);

        // Recurring job should still be there
        assert_eq!(sched.total_jobs(), 1);
        let meta = sched.get_meta(id).unwrap();
        assert_eq!(meta.last_status.as_deref(), Some("ok"));
        assert_eq!(meta.consecutive_errors, 0);
        assert!(meta.job.last_run.is_some());
    }

    #[test]
    fn test_record_delivery_failure_keeps_recurring_enabled() {
        let (sched, _tmp) = make_scheduler(100);
        let agent = AgentId::new();
        let id = sched.add_job(make_job(agent), false).unwrap();
        let failure = DeliveryFailure::new("channel:telegram:42", "HTTP 503", 3);

        sched.record_delivery_failure(id, &failure, "payload body");

        let meta = sched.get_meta(id).unwrap();
        assert!(meta.job.enabled);
        assert_eq!(meta.consecutive_errors, 0);
        assert_eq!(meta.last_status.as_deref(), Some("delivery_failed"));
        assert!(meta
            .last_delivery_error
            .as_deref()
            .unwrap()
            .contains("HTTP 503"));
        assert_eq!(meta.dead_letters.len(), 1);
        assert_eq!(meta.dead_letters[0].attempts, 3);
        assert_eq!(meta.redelivery_queue.len(), 1);
        assert_eq!(
            sched
                .read_redelivery_payload(&meta.redelivery_queue[0])
                .unwrap(),
            "payload body"
        );
    }

    #[test]
    fn test_record_delivery_failure_disables_one_shot_without_removing_history() {
        let (sched, _tmp) = make_scheduler(100);
        let agent = AgentId::new();
        let id = sched.add_job(make_job(agent), true).unwrap();
        let failure = DeliveryFailure::new("webhook:https://example.com/hook", "HTTP 500", 5);

        sched.record_delivery_failure(id, &failure, "important result");

        let meta = sched.get_meta(id).unwrap();
        assert!(!meta.job.enabled);
        assert!(meta.job.next_run.is_none());
        assert_eq!(meta.dead_letters.len(), 1);
        assert_eq!(meta.dead_letters[0].payload_preview, "important result");
    }

    #[test]
    fn test_redelivery_success_removes_queue_payload() {
        let (sched, _tmp) = make_scheduler(100);
        let agent = AgentId::new();
        let id = sched.add_job(make_job(agent), false).unwrap();
        let failure = DeliveryFailure::new("channel:telegram:42", "HTTP 503", 3);
        sched.record_delivery_failure(id, &failure, "payload body");
        assert!(sched.due_redeliveries().is_empty());

        let meta = sched.get_meta(id).unwrap();
        let entry = meta.redelivery_queue[0].clone();
        let payload_path = entry.payload_path.clone();
        assert!(std::path::Path::new(&payload_path).exists());

        sched.record_redelivery_success(id, &entry.id);

        let meta = sched.get_meta(id).unwrap();
        assert!(meta.redelivery_queue.is_empty());
        assert!(meta.last_delivery_error.is_none());
        assert!(!std::path::Path::new(&payload_path).exists());
    }

    #[test]
    fn test_redelivery_failure_exhaustion_moves_back_to_dead_letter() {
        let (sched, _tmp) = make_scheduler(100);
        let agent = AgentId::new();
        let id = sched.add_job(make_job(agent), false).unwrap();
        let failure = DeliveryFailure::new("channel:telegram:42", "HTTP 503", 3);
        sched.record_delivery_failure(id, &failure, "payload body");
        let mut entry = sched.get_meta(id).unwrap().redelivery_queue[0].clone();

        for round in 0..entry.max_attempts {
            sched.record_redelivery_failure(id, &entry.id, &failure, "payload body");
            let meta = sched.get_meta(id).unwrap();
            if round + 1 < entry.max_attempts {
                entry = meta.redelivery_queue[0].clone();
                assert_eq!(entry.attempts, round + 1);
            } else {
                assert!(meta.redelivery_queue.is_empty());
                assert_eq!(meta.dead_letters.len(), 2);
            }
        }
    }

    // -- test_record_failure_auto_disable -----------------------------------

    #[test]
    fn test_record_failure_auto_disable() {
        let (sched, _tmp) = make_scheduler(100);
        let agent = AgentId::new();
        let job = make_job(agent);
        let id = sched.add_job(job, false).unwrap();

        // Fail MAX_CONSECUTIVE_ERRORS - 1 times: should still be enabled
        for i in 0..(MAX_CONSECUTIVE_ERRORS - 1) {
            sched.record_failure(id, &format!("error {i}"));
            let meta = sched.get_meta(id).unwrap();
            assert!(
                meta.job.enabled,
                "Job should still be enabled after {} failures",
                i + 1
            );
            assert_eq!(meta.consecutive_errors, i + 1);
        }

        // One more failure should auto-disable
        sched.record_failure(id, "final error");
        let meta = sched.get_meta(id).unwrap();
        assert!(
            !meta.job.enabled,
            "Job should be auto-disabled after {MAX_CONSECUTIVE_ERRORS} failures"
        );
        assert_eq!(meta.consecutive_errors, MAX_CONSECUTIVE_ERRORS);
        assert!(
            meta.last_status.as_ref().unwrap().starts_with("error:"),
            "last_status should record the error"
        );
    }

    // -- test_due_jobs_only_enabled -----------------------------------------

    #[test]
    fn test_due_jobs_only_enabled() {
        let (sched, _tmp) = make_scheduler(100);
        let agent = AgentId::new();

        // Job 1: enabled, next_run in the past
        let mut j1 = make_job(agent);
        j1.name = "enabled-due".into();
        let id1 = sched.add_job(j1, false).unwrap();

        // Job 2: disabled
        let mut j2 = make_job(agent);
        j2.name = "disabled-job".into();
        let id2 = sched.add_job(j2, false).unwrap();
        sched.set_enabled(id2, false).unwrap();

        // Force job 1's next_run to the past
        if let Some(mut meta) = sched.jobs.get_mut(&id1) {
            meta.job.next_run = Some(Utc::now() - Duration::seconds(10));
        }

        // Force job 2's next_run to the past too (but it's disabled)
        if let Some(mut meta) = sched.jobs.get_mut(&id2) {
            meta.job.next_run = Some(Utc::now() - Duration::seconds(10));
        }

        let due = sched.due_jobs();
        assert_eq!(due.len(), 1);
        assert_eq!(due[0].name, "enabled-due");
    }

    #[test]
    fn test_due_jobs_future_not_included() {
        let (sched, _tmp) = make_scheduler(100);
        let agent = AgentId::new();

        let job = make_job(agent);
        sched.add_job(job, false).unwrap();

        // The job was just added with next_run = now + 3600s, so it should
        // not be due yet.
        let due = sched.due_jobs();
        assert!(due.is_empty());
    }

    // -- test_set_enabled ---------------------------------------------------

    #[test]
    fn test_set_enabled() {
        let (sched, _tmp) = make_scheduler(100);
        let agent = AgentId::new();

        let job = make_job(agent);
        let id = sched.add_job(job, false).unwrap();

        // Disable
        sched.set_enabled(id, false).unwrap();
        let meta = sched.get_meta(id).unwrap();
        assert!(!meta.job.enabled);

        // Re-enable resets error count
        sched.record_failure(id, "ignored because disabled");
        // Actually the job is disabled so record_failure still updates it.
        // Let's first re-enable to test reset.
        sched.set_enabled(id, true).unwrap();
        let meta = sched.get_meta(id).unwrap();
        assert!(meta.job.enabled);
        assert_eq!(meta.consecutive_errors, 0);
        assert!(meta.job.next_run.is_some());

        // Non-existent ID should fail
        let fake_id = CronJobId::new();
        assert!(sched.set_enabled(fake_id, true).is_err());
    }

    // -- test_persist_and_load ----------------------------------------------

    #[test]
    fn test_persist_and_load() {
        let tmp = tempfile::tempdir().unwrap();
        let agent = AgentId::new();

        // Create scheduler, add jobs, persist
        {
            let sched = CronScheduler::new(tmp.path(), 100);
            let mut j1 = make_job(agent);
            j1.name = "persist-a".into();
            let mut j2 = make_job(agent);
            j2.name = "persist-b".into();

            sched.add_job(j1, false).unwrap();
            sched.add_job(j2, true).unwrap(); // one_shot

            sched.persist().unwrap();
        }

        // Create a new scheduler and load from disk
        {
            let sched = CronScheduler::new(tmp.path(), 100);
            let count = sched.load().unwrap();
            assert_eq!(count, 2);
            assert_eq!(sched.total_jobs(), 2);

            let jobs = sched.list_jobs(agent);
            assert_eq!(jobs.len(), 2);

            let names: Vec<&str> = jobs.iter().map(|j| j.name.as_str()).collect();
            assert!(names.contains(&"persist-a"));
            assert!(names.contains(&"persist-b"));

            // Verify one_shot flag was preserved
            let b_id = jobs.iter().find(|j| j.name == "persist-b").unwrap().id;
            let meta = sched.get_meta(b_id).unwrap();
            assert!(meta.one_shot);
        }
    }

    #[test]
    fn load_advances_stale_recurring_runs_instead_of_firing_boot_backlog() {
        let tmp = tempfile::tempdir().unwrap();
        let agent = AgentId::new();
        let stale_next_run = Utc::now() - Duration::days(21);

        {
            let sched = CronScheduler::new(tmp.path(), 100);
            let mut job = make_job(agent);
            job.name = "stale-recurring".into();
            let id = sched.add_job(job, false).unwrap();
            {
                let mut meta = sched.jobs.get_mut(&id).unwrap();
                meta.job.next_run = Some(stale_next_run);
            }
            sched.persist().unwrap();
        }

        let sched = CronScheduler::new(tmp.path(), 100);
        assert_eq!(sched.load().unwrap(), 1);

        assert!(sched.due_jobs().is_empty());
        let job = sched.list_jobs(agent).pop().unwrap();
        assert!(
            job.next_run.unwrap() > Utc::now(),
            "stale recurring run should advance to a future next_run"
        );
        let meta = sched.get_meta(job.id).unwrap();
        assert!(meta
            .last_status
            .as_deref()
            .unwrap_or_default()
            .starts_with("skipped_stale_missed_run"));
    }

    #[test]
    fn load_keeps_recently_missed_recurring_runs_due_for_short_restart_catchup() {
        let tmp = tempfile::tempdir().unwrap();
        let agent = AgentId::new();

        {
            let sched = CronScheduler::new(tmp.path(), 100);
            let id = sched.add_job(make_job(agent), false).unwrap();
            {
                let mut meta = sched.jobs.get_mut(&id).unwrap();
                meta.job.next_run = Some(Utc::now() - Duration::seconds(30));
            }
            sched.persist().unwrap();
        }

        let sched = CronScheduler::new(tmp.path(), 100);
        assert_eq!(sched.load().unwrap(), 1);

        let due = sched.due_jobs();
        assert_eq!(due.len(), 1);
        assert_eq!(due[0].agent_id, agent);
    }

    #[test]
    fn test_load_no_file_returns_zero() {
        let tmp = tempfile::tempdir().unwrap();
        let sched = CronScheduler::new(tmp.path(), 100);
        assert_eq!(sched.load().unwrap(), 0);
    }

    // -- compute_next_run ---------------------------------------------------

    #[test]
    fn test_compute_next_run_at() {
        let target = Utc::now() + Duration::hours(2);
        let schedule = CronSchedule::At { at: target };
        let next = compute_next_run(&schedule);
        assert_eq!(next, target);
    }

    #[test]
    fn test_compute_next_run_every() {
        let before = Utc::now();
        let schedule = CronSchedule::Every { every_secs: 300 };
        let next = compute_next_run(&schedule);
        let after = Utc::now();

        // Should be roughly now + 300s
        assert!(next >= before + Duration::seconds(300));
        assert!(next <= after + Duration::seconds(300));
    }

    #[test]
    fn test_compute_next_run_cron_daily() {
        let now = Utc::now();
        let schedule = CronSchedule::Cron {
            expr: "0 9 * * *".into(),
            tz: None,
        };
        let next = compute_next_run(&schedule);

        // Should be within the next 24 hours (next 09:00 UTC)
        assert!(next > now);
        assert!(next <= now + Duration::hours(24));
        assert_eq!(next.format("%M").to_string(), "00");
        assert_eq!(next.format("%H").to_string(), "09");
    }

    #[test]
    fn test_compute_next_run_cron_with_dow() {
        let now = Utc::now();
        let schedule = CronSchedule::Cron {
            expr: "30 14 * * 1-5".into(),
            tz: None,
        };
        let next = compute_next_run(&schedule);

        // Should be within the next 7 days and at 14:30
        assert!(next > now);
        assert!(next <= now + Duration::days(7));
        assert_eq!(next.format("%H:%M").to_string(), "14:30");
    }

    #[test]
    fn test_compute_next_run_cron_invalid_expr() {
        let now = Utc::now();
        let schedule = CronSchedule::Cron {
            expr: "not a cron".into(),
            tz: None,
        };
        let next = compute_next_run(&schedule);
        // Invalid expression falls back to 1 hour from now
        assert!(next > now + Duration::minutes(59));
        assert!(next <= now + Duration::minutes(61));
    }

    #[test]
    fn test_compute_next_run_cron_with_tz_europe_paris() {
        // Simulate: Friday 2026-03-27 21:00 UTC, cron "0 20 * * 0" (Sunday 20h), tz Europe/Paris
        let after = chrono::DateTime::parse_from_rfc3339("2026-03-27T21:00:00Z")
            .unwrap()
            .with_timezone(&Utc);
        let schedule = CronSchedule::Cron {
            expr: "0 20 * * 0".into(),
            tz: Some("Europe/Paris".into()),
        };
        let next = compute_next_run_after_with_tz(&schedule, after, None);

        // 2026-03-27 is Friday. Next Sunday is March 29.
        // DST change: CET→CEST on March 29 2026 at 2:00 AM
        // So Sunday 20h Paris = Sunday 18h UTC (CEST = UTC+2)
        assert!(
            next > after,
            "next_run must be after the base time, got {next}"
        );
        assert_eq!(
            next.format("%A").to_string(),
            "Sunday",
            "Should be a Sunday, got {} ({})",
            next.format("%A"),
            next
        );
    }

    #[test]
    fn test_compute_next_run_cron_tz_not_today() {
        // Cron "0 9 * * *" (daily 9h), tz Europe/Paris
        // If it's Friday 21:00 UTC → next should be Saturday 08:00 UTC (9h Paris CET)
        let after = chrono::DateTime::parse_from_rfc3339("2026-03-27T21:00:00Z")
            .unwrap()
            .with_timezone(&Utc);
        let schedule = CronSchedule::Cron {
            expr: "0 9 * * *".into(),
            tz: Some("Europe/Paris".into()),
        };
        let next = compute_next_run_after_with_tz(&schedule, after, None);

        assert!(next > after);
        assert_eq!(
            next.format("%Y-%m-%d").to_string(),
            "2026-03-28",
            "Should be tomorrow, got {}",
            next
        );
        let hour = next.hour();
        assert!(
            hour == 7 || hour == 8,
            "Should be 7 or 8 UTC (9h Paris), got {}h — full: {}",
            hour,
            next
        );
    }

    // -- error message truncation in record_failure -------------------------

    #[test]
    fn test_compute_next_run_after_skips_current_second() {
        // A "every 4 hours" cron: next_run should be strictly after `after`.
        // Use a fixed time mid-hour to avoid landing exactly on a boundary.
        let schedule = CronSchedule::Cron {
            expr: "0 */4 * * *".into(),
            tz: None,
        };
        let after = chrono::DateTime::parse_from_rfc3339("2026-03-28T01:30:00Z")
            .unwrap()
            .with_timezone(&Utc);
        let next = compute_next_run_after(&schedule, after);
        // Next 4-hourly boundary after 01:30 is 04:00 → 2.5 hours away
        assert!(next > after, "next_run should be strictly after `after`");
        let diff = next - after;
        assert!(
            diff.num_minutes() >= 60,
            "Expected next_run at least 60 min away, got {} seconds",
            diff.num_seconds()
        );
    }

    #[test]
    fn test_record_failure_truncates_long_error() {
        let (sched, _tmp) = make_scheduler(100);
        let agent = AgentId::new();
        let job = make_job(agent);
        let id = sched.add_job(job, false).unwrap();

        let long_error = "x".repeat(1000);
        sched.record_failure(id, &long_error);

        let meta = sched.get_meta(id).unwrap();
        let status = meta.last_status.unwrap();
        // "error: " is 7 chars + 256 chars of truncated message = 263 max
        assert!(
            status.len() <= 263,
            "Status should be truncated, got {} chars",
            status.len()
        );
    }

    // -- timezone-aware cron (#473) -----------------------------------------

    #[test]
    fn test_cron_tz_shifts_next_run() {
        // "0 9 * * *" in America/New_York (UTC-5 or UTC-4 depending on DST).
        // The next fire time in UTC should differ from a plain UTC "0 9 * * *".
        let schedule_utc = CronSchedule::Cron {
            expr: "0 9 * * *".into(),
            tz: None,
        };
        let schedule_ny = CronSchedule::Cron {
            expr: "0 9 * * *".into(),
            tz: Some("America/New_York".into()),
        };
        let now = Utc::now();
        let next_utc = compute_next_run_after(&schedule_utc, now);
        let next_ny = compute_next_run_after(&schedule_ny, now);

        // The New York schedule should fire at 09:00 Eastern, which is 13:00
        // or 14:00 UTC (depending on DST). In either case, it should NOT
        // equal the plain UTC 09:00 result.
        assert_ne!(
            next_utc, next_ny,
            "Timezone-aware schedule should produce a different UTC time"
        );

        // Verify the New York result, when converted to ET, shows hour 09.
        let ny_tz: chrono_tz::Tz = "America/New_York".parse().unwrap();
        let next_ny_local = next_ny.with_timezone(&ny_tz);
        assert_eq!(
            next_ny_local.hour(),
            9,
            "Expected 09:00 in America/New_York, got {:02}:{:02}",
            next_ny_local.hour(),
            next_ny_local.minute()
        );
    }

    #[test]
    fn test_cron_tz_none_defaults_to_utc() {
        // tz: None should behave identically to tz: Some("UTC").
        let schedule_none = CronSchedule::Cron {
            expr: "30 12 * * *".into(),
            tz: None,
        };
        let schedule_utc = CronSchedule::Cron {
            expr: "30 12 * * *".into(),
            tz: Some("UTC".into()),
        };
        let now = Utc::now();
        let next_none = compute_next_run_after(&schedule_none, now);
        let next_utc = compute_next_run_after(&schedule_utc, now);
        assert_eq!(next_none, next_utc);
    }

    #[test]
    fn test_cron_tz_empty_string_defaults_to_utc() {
        let schedule_empty = CronSchedule::Cron {
            expr: "30 12 * * *".into(),
            tz: Some(String::new()),
        };
        let schedule_none = CronSchedule::Cron {
            expr: "30 12 * * *".into(),
            tz: None,
        };
        let now = Utc::now();
        assert_eq!(
            compute_next_run_after(&schedule_empty, now),
            compute_next_run_after(&schedule_none, now)
        );
    }

    #[test]
    fn test_cron_tz_invalid_falls_back_to_utc() {
        // An invalid timezone string should fall back to UTC, not panic.
        let schedule_bad = CronSchedule::Cron {
            expr: "0 9 * * *".into(),
            tz: Some("Not/A_Timezone".into()),
        };
        let schedule_utc = CronSchedule::Cron {
            expr: "0 9 * * *".into(),
            tz: None,
        };
        let now = Utc::now();
        let next_bad = compute_next_run_after(&schedule_bad, now);
        let next_utc = compute_next_run_after(&schedule_utc, now);
        // Invalid tz falls back to UTC computation — same result.
        assert_eq!(next_bad, next_utc);
    }

    #[test]
    fn test_cron_tz_asia_shanghai() {
        // "0 8 * * *" in Asia/Shanghai (UTC+8) should fire at 00:00 UTC.
        let schedule = CronSchedule::Cron {
            expr: "0 8 * * *".into(),
            tz: Some("Asia/Shanghai".into()),
        };
        let now = Utc::now();
        let next = compute_next_run_after(&schedule, now);

        let shanghai_tz: chrono_tz::Tz = "Asia/Shanghai".parse().unwrap();
        let local = next.with_timezone(&shanghai_tz);
        assert_eq!(local.hour(), 8);
        assert_eq!(local.minute(), 0);

        // In UTC, 08:00 Shanghai = 00:00 UTC.
        assert_eq!(next.hour(), 0, "08:00 CST should be 00:00 UTC");
    }

    // -- reassign_agent_jobs (#461) -----------------------------------------

    #[test]
    fn test_reassign_agent_jobs_basic() {
        let (sched, _tmp) = make_scheduler(100);
        let old_agent = AgentId::new();
        let new_agent = AgentId::new();

        let mut j1 = make_job(old_agent);
        j1.name = "cron-a".into();
        let mut j2 = make_job(old_agent);
        j2.name = "cron-b".into();

        let id1 = sched.add_job(j1, false).unwrap();
        let id2 = sched.add_job(j2, false).unwrap();

        let count = sched.reassign_agent_jobs(old_agent, new_agent);
        assert_eq!(count, 2);

        // Both jobs should now belong to the new agent
        let job1 = sched.get_job(id1).unwrap();
        assert_eq!(job1.agent_id, new_agent);
        let job2 = sched.get_job(id2).unwrap();
        assert_eq!(job2.agent_id, new_agent);

        // Old agent should have zero jobs
        assert!(sched.list_jobs(old_agent).is_empty());
        // New agent should have both
        assert_eq!(sched.list_jobs(new_agent).len(), 2);
    }

    #[test]
    fn test_reassign_agent_jobs_does_not_touch_other_agents() {
        let (sched, _tmp) = make_scheduler(100);
        let agent_a = AgentId::new();
        let agent_b = AgentId::new();
        let agent_c = AgentId::new();

        let mut ja = make_job(agent_a);
        ja.name = "job-a".into();
        let mut jb = make_job(agent_b);
        jb.name = "job-b".into();

        let _id_a = sched.add_job(ja, false).unwrap();
        let id_b = sched.add_job(jb, false).unwrap();

        // Reassign agent_a -> agent_c
        let count = sched.reassign_agent_jobs(agent_a, agent_c);
        assert_eq!(count, 1);

        // agent_b's job should be untouched
        let job_b = sched.get_job(id_b).unwrap();
        assert_eq!(job_b.agent_id, agent_b);
    }

    #[test]
    fn test_reassign_agent_jobs_no_match_returns_zero() {
        let (sched, _tmp) = make_scheduler(100);
        let agent = AgentId::new();
        let other = AgentId::new();

        let job = make_job(agent);
        sched.add_job(job, false).unwrap();

        // Reassign a non-existent agent
        let count = sched.reassign_agent_jobs(AgentId::new(), other);
        assert_eq!(count, 0);
    }

    #[test]
    fn test_reassign_agent_jobs_resets_consecutive_errors() {
        let (sched, _tmp) = make_scheduler(100);
        let old_agent = AgentId::new();
        let new_agent = AgentId::new();

        let job = make_job(old_agent);
        let id = sched.add_job(job, false).unwrap();

        // Simulate some failures
        sched.record_failure(id, "agent not found");
        sched.record_failure(id, "agent not found");
        let meta = sched.get_meta(id).unwrap();
        assert_eq!(meta.consecutive_errors, 2);

        // Reassign
        sched.reassign_agent_jobs(old_agent, new_agent);

        // Errors should be reset
        let meta = sched.get_meta(id).unwrap();
        assert_eq!(meta.consecutive_errors, 0);
        assert_eq!(meta.job.agent_id, new_agent);
    }

    #[test]
    fn test_reassign_agent_jobs_reenables_disabled_stale_jobs() {
        let (sched, _tmp) = make_scheduler(100);
        let old_agent = AgentId::new();
        let new_agent = AgentId::new();

        let job = make_job(old_agent);
        let id = sched.add_job(job, false).unwrap();

        // Simulate enough failures to auto-disable (with "not found" message)
        for _ in 0..MAX_CONSECUTIVE_ERRORS {
            sched.record_failure(id, "No such agent");
        }
        let meta = sched.get_meta(id).unwrap();
        assert!(!meta.job.enabled, "Job should be auto-disabled");

        // Reassign should re-enable it
        sched.reassign_agent_jobs(old_agent, new_agent);

        let meta = sched.get_meta(id).unwrap();
        assert!(
            meta.job.enabled,
            "Job should be re-enabled after reassignment"
        );
        assert_eq!(meta.consecutive_errors, 0);
        assert_eq!(meta.job.agent_id, new_agent);
    }

    #[test]
    fn test_reassign_agent_jobs_persists_after_roundtrip() {
        let tmp = tempfile::tempdir().unwrap();
        let old_agent = AgentId::new();
        let new_agent = AgentId::new();

        // Create scheduler, add job, reassign, persist
        let id = {
            let sched = CronScheduler::new(tmp.path(), 100);
            let job = make_job(old_agent);
            let id = sched.add_job(job, false).unwrap();

            sched.reassign_agent_jobs(old_agent, new_agent);
            sched.persist().unwrap();
            id
        };

        // Load from disk and verify the agent_id was persisted
        {
            let sched = CronScheduler::new(tmp.path(), 100);
            sched.load().unwrap();

            let job = sched.get_job(id).unwrap();
            assert_eq!(job.agent_id, new_agent);
            assert!(sched.list_jobs(old_agent).is_empty());
        }
    }

    // -- remove_agent_jobs (#504) -------------------------------------------

    #[test]
    fn test_remove_agent_jobs_basic() {
        let (sched, _tmp) = make_scheduler(100);
        let agent = AgentId::new();
        let other = AgentId::new();

        let mut j1 = make_job(agent);
        j1.name = "job-a".into();
        let mut j2 = make_job(agent);
        j2.name = "job-b".into();
        let mut j3 = make_job(other);
        j3.name = "job-other".into();

        sched.add_job(j1, false).unwrap();
        sched.add_job(j2, false).unwrap();
        let id3 = sched.add_job(j3, false).unwrap();

        assert_eq!(sched.total_jobs(), 3);

        let removed = sched.remove_agent_jobs(agent);
        assert_eq!(removed, 2);
        assert_eq!(sched.total_jobs(), 1);

        // The other agent's job should still exist
        assert!(sched.list_jobs(agent).is_empty());
        assert_eq!(sched.list_jobs(other).len(), 1);
        assert!(sched.get_job(id3).is_some());
    }

    #[test]
    fn test_remove_agent_jobs_no_match() {
        let (sched, _tmp) = make_scheduler(100);
        let agent = AgentId::new();

        let job = make_job(agent);
        sched.add_job(job, false).unwrap();

        // Remove for a non-existent agent
        let removed = sched.remove_agent_jobs(AgentId::new());
        assert_eq!(removed, 0);
        assert_eq!(sched.total_jobs(), 1);
    }

    #[test]
    fn test_remove_agent_jobs_persists() {
        let tmp = tempfile::tempdir().unwrap();
        let agent = AgentId::new();
        let other = AgentId::new();

        // Add jobs for two agents, remove one agent's jobs, persist
        {
            let sched = CronScheduler::new(tmp.path(), 100);
            let mut j1 = make_job(agent);
            j1.name = "doomed".into();
            let mut j2 = make_job(other);
            j2.name = "survivor".into();

            sched.add_job(j1, false).unwrap();
            sched.add_job(j2, false).unwrap();

            sched.remove_agent_jobs(agent);
            sched.persist().unwrap();
        }

        // Reload and verify
        {
            let sched = CronScheduler::new(tmp.path(), 100);
            sched.load().unwrap();
            assert_eq!(sched.total_jobs(), 1);
            assert!(sched.list_jobs(agent).is_empty());
            assert_eq!(sched.list_jobs(other).len(), 1);
        }
    }
}
