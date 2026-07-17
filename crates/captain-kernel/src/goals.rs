//! R.2.1 — Goal-driven autopilot store.
//!
//! Persists user-defined long-running objectives in `~/.captain/goals.json`
//! and exposes a thread-safe API for the goal-loop runtime
//! (`captain-runtime::goal_loop`) to record check outcomes and decide
//! when to escalate.
//!
//! Hard caps applied at insert time to prevent runaway behaviour:
//! * `interval_secs` ≥ 10 — no spamming
//! * `escalation_threshold` ≥ 1
//! * `max_llm_calls_per_hour` ≤ 1000 — anti-runaway hard ceiling for the
//!   future R.2.2 reflection job
//! * `recent_checks` ring buffer capped at 50 entries
//! * `llm_call_log` sliding window cleaned on every read

use chrono::{DateTime, Duration as ChronoDuration, Utc};
use dashmap::DashMap;
use serde::{Deserialize, Serialize};
use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use tracing::{debug, info, warn};

use captain_types::error::{CaptainError, CaptainResult};

/// Maximum entries in the [`Goal::recent_checks`] ring buffer.
pub const RECENT_CHECKS_CAPACITY: usize = 50;

/// Hard ceiling applied to `Goal::max_llm_calls_per_hour` at insert time.
/// Beyond this the system would burn unbounded API spend on a single
/// goal, so we refuse to accept the request.
pub const MAX_LLM_CALLS_PER_HOUR_CEILING: u32 = 1000;

/// Minimum value for `Goal::interval_secs`. Lower than this and the
/// scheduler thrashes (and downstream rate limits would trip anyway).
pub const MIN_INTERVAL_SECS: u64 = 10;

/// Lifecycle state for a [`Goal`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GoalStatus {
    /// Loop runs at every `interval_secs` tick.
    Active,
    /// Stored but loop suspended (manually paused by the user).
    Paused,
    /// Crossed `escalation_threshold` consecutive failures and
    /// notified the user via channel_send.
    Escalated,
}

/// One observation of the goal's `check_command`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CheckResult {
    pub ts: DateTime<Utc>,
    pub ok: bool,
    /// Truncated stdout/stderr for forensics (≤ 4 KB).
    pub output: String,
    pub latency_ms: u64,
    /// Optional recovery action attempted after a failure.
    #[serde(default)]
    pub recovery_attempted: bool,
}

impl CheckResult {
    /// Truncate the captured output to keep `goals.json` bounded.
    const MAX_OUTPUT_BYTES: usize = 4096;

    pub fn new(ok: bool, output: String, latency_ms: u64) -> Self {
        let mut out = output;
        if out.len() > Self::MAX_OUTPUT_BYTES {
            out.truncate(Self::MAX_OUTPUT_BYTES);
            out.push_str("…[truncated]");
        }
        Self {
            ts: Utc::now(),
            ok,
            output: out,
            latency_ms,
            recovery_attempted: false,
        }
    }
}

/// A long-running objective the autopilot enforces.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Goal {
    /// Caller-provided identifier (alphanumeric + `-`/`_`, 3..=64 chars).
    pub id: String,
    /// Short human label.
    pub name: String,
    /// Free-text description of what is being maintained.
    pub description: String,
    /// Optional owning project. Global goals keep these fields empty so
    /// existing goals.json files remain valid.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub project_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub project_slug: Option<String>,
    pub status: GoalStatus,
    /// Seconds between two checks. ≥ [`MIN_INTERVAL_SECS`].
    pub interval_secs: u64,
    /// Shell command (or `ssh_exec`-formatted command) executed at every
    /// tick. Exit code 0 → success.
    pub check_command: String,
    /// Optional recovery shell command run on a single failure before
    /// counting it against `escalation_threshold`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub recovery_command: Option<String>,
    /// Number of consecutive failures (after recovery attempts) before
    /// the goal is escalated to the user via channel_send. ≥ 1.
    pub escalation_threshold: u32,
    /// Hard cap on LLM calls (R.2.2 reflection job) per goal per hour.
    /// Capped to [`MAX_LLM_CALLS_PER_HOUR_CEILING`] at insert time.
    pub max_llm_calls_per_hour: u32,
    /// Optional channel + recipient to escalate to. If `None`, falls
    /// back to the channel that created the goal (resolved at runtime).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub escalation_channel: Option<EscalationTarget>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_check_ts: Option<DateTime<Utc>>,
    /// Live counter of consecutive failed checks (after recovery
    /// attempts). Reset on success. Crosses `escalation_threshold` →
    /// status flips to `Escalated`.
    #[serde(default)]
    pub consecutive_fails: u32,
    /// When the goal was last escalated (None until first escalation).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub escalated_at: Option<DateTime<Utc>>,
    /// Ring buffer of the last [`RECENT_CHECKS_CAPACITY`] check results
    /// for forensics + future LLM reflection.
    #[serde(default)]
    pub recent_checks: VecDeque<CheckResult>,
    /// Sliding window of LLM call timestamps used to enforce
    /// `max_llm_calls_per_hour`. Reflection job (R.2.2) appends here
    /// before issuing a call; entries older than 1h are pruned on read.
    #[serde(default)]
    pub llm_call_log: Vec<DateTime<Utc>>,
    /// R.2.2 — pending / applied / rejected reflection suggestions.
    /// The reflection job appends Pending entries; the user accepts
    /// them via `goal_apply_suggestion` (which mutates the goal and
    /// flips status to Applied). Bounded at 20 entries — older Applied
    /// or Rejected entries are pruned on insert to keep `goals.json`
    /// small.
    #[serde(default)]
    pub suggestions: Vec<Suggestion>,
}

/// R.2.2 — kind of adjustment proposed by the reflection job.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum SuggestionKind {
    /// Change `interval_secs` to `new_secs` (must remain ≥ MIN_INTERVAL).
    AdjustInterval { new_secs: u64 },
    /// Change `escalation_threshold` to `new_value` (must remain ≥ 1).
    AdjustThreshold { new_value: u32 },
    /// Set or replace `recovery_command`.
    EnableRecovery { command: String },
    /// Drop `recovery_command` (back to None).
    DisableRecovery,
}

/// Suggestion lifecycle.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SuggestionStatus {
    Pending,
    Applied,
    Rejected,
}

/// One reflection-job proposal awaiting user review.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Suggestion {
    /// Compact unique id (8-char timestamp+rand).
    pub id: String,
    /// What the reflection job suggests changing.
    #[serde(flatten)]
    pub kind: SuggestionKind,
    /// One-line natural-language reason from the LLM.
    pub reason: String,
    pub created_at: DateTime<Utc>,
    pub status: SuggestionStatus,
    /// Set when the user resolves the suggestion (apply or reject).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resolved_at: Option<DateTime<Utc>>,
}

impl Suggestion {
    /// Build a fresh Pending suggestion with a generated id.
    pub fn new(kind: SuggestionKind, reason: String) -> Self {
        // Cheap unique id: ms-since-epoch in base36 + 4 random hex chars.
        // Avoids pulling uuid as a dep just for this.
        let ts = Utc::now().timestamp_millis() as u64;
        let mut rand_part: u32 = 0;
        for b in std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.subsec_nanos().to_le_bytes())
            .unwrap_or([0; 4])
        {
            rand_part = rand_part.wrapping_mul(31).wrapping_add(b as u32);
        }
        Self {
            id: format!("s{ts:x}{rand_part:04x}"),
            kind,
            reason,
            created_at: Utc::now(),
            status: SuggestionStatus::Pending,
            resolved_at: None,
        }
    }
}

/// Maximum number of suggestions retained per goal. Older Applied /
/// Rejected entries are pruned first; Pending entries always survive.
pub const MAX_SUGGESTIONS_PER_GOAL: usize = 20;

/// Where to escalate when a goal crosses the failure threshold.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EscalationTarget {
    /// Channel name e.g. `"telegram"`, `"discord"`, `"slack"`.
    pub channel: String,
    /// Channel-specific recipient ID (chat_id, user, …).
    pub recipient: String,
}

impl Goal {
    /// Validate input fields. Returns `Err` listing every problem.
    pub fn validate(&self) -> Result<(), String> {
        let mut errs = Vec::new();
        if self.id.is_empty() || self.id.len() > 64 || self.id.len() < 3 {
            errs.push("id must be 3..=64 chars".into());
        }
        if !self
            .id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
        {
            errs.push("id must be alphanumeric with - or _".into());
        }
        if self.name.trim().is_empty() {
            errs.push("name is required".into());
        }
        if self.check_command.trim().is_empty() {
            errs.push("check_command is required".into());
        }
        // R.2.1 — refuse hyper-critical commands at insert time. A goal
        // runs the check_command in a loop, so even a single rm -rf in a
        // compromised LLM call would be catastrophic.
        if let Some(pat) = captain_runtime::critical_patterns::is_critical(&self.check_command) {
            errs.push(format!(
                "check_command contains critical pattern '{pat}' — refused"
            ));
        }
        if let Some(rec) = &self.recovery_command {
            if let Some(pat) = captain_runtime::critical_patterns::is_critical(rec) {
                errs.push(format!(
                    "recovery_command contains critical pattern '{pat}' — refused"
                ));
            }
        }
        if self.interval_secs < MIN_INTERVAL_SECS {
            errs.push(format!("interval_secs must be ≥ {MIN_INTERVAL_SECS}"));
        }
        if self.escalation_threshold == 0 {
            errs.push("escalation_threshold must be ≥ 1".into());
        }
        if self.max_llm_calls_per_hour > MAX_LLM_CALLS_PER_HOUR_CEILING {
            errs.push(format!(
                "max_llm_calls_per_hour must be ≤ {MAX_LLM_CALLS_PER_HOUR_CEILING}"
            ));
        }
        if errs.is_empty() {
            Ok(())
        } else {
            Err(errs.join("; "))
        }
    }
}

/// Concurrent, file-backed store of [`Goal`]s.
pub struct GoalStore {
    goals: DashMap<String, Goal>,
    persist_path: PathBuf,
}

impl GoalStore {
    /// Construct a store at `<home_dir>/goals.json`. The file is not
    /// touched until [`Self::load`] is called.
    pub fn new(home_dir: &Path) -> Self {
        Self {
            goals: DashMap::new(),
            persist_path: home_dir.join("goals.json"),
        }
    }

    /// Load all persisted goals into memory. Returns count loaded.
    /// Missing file → `Ok(0)` (first-boot case).
    pub fn load(&self) -> CaptainResult<usize> {
        if !self.persist_path.exists() {
            return Ok(0);
        }
        let data = std::fs::read_to_string(&self.persist_path)
            .map_err(|e| CaptainError::Internal(format!("read goals.json: {e}")))?;
        let goals: Vec<Goal> = serde_json::from_str(&data)
            .map_err(|e| CaptainError::Internal(format!("parse goals.json: {e}")))?;
        let count = goals.len();
        for g in goals {
            self.goals.insert(g.id.clone(), g);
        }
        info!(count, "Loaded goals from disk");
        Ok(count)
    }

    /// Persist atomically and synchronize the committed file to disk.
    pub fn persist(&self) -> CaptainResult<()> {
        let goals: Vec<Goal> = self.goals.iter().map(|r| r.value().clone()).collect();
        let data = serde_json::to_string_pretty(&goals)
            .map_err(|e| CaptainError::Internal(format!("serialize goals: {e}")))?;
        captain_types::durable_fs::atomic_write(&self.persist_path, data.as_bytes())
            .map_err(|e| CaptainError::Internal(format!("persist goals: {e}")))?;
        debug!(count = goals.len(), "Persisted goals");
        Ok(())
    }

    /// Insert a new goal. Validates first; refuses duplicate IDs.
    pub fn add(&self, goal: Goal) -> CaptainResult<()> {
        goal.validate().map_err(CaptainError::InvalidInput)?;
        if self.goals.contains_key(&goal.id) {
            return Err(CaptainError::InvalidInput(format!(
                "goal id '{}' already exists",
                goal.id
            )));
        }
        self.goals.insert(goal.id.clone(), goal);
        self.persist()?;
        Ok(())
    }

    /// Remove a goal by id. Returns the removed goal (or None).
    pub fn remove(&self, id: &str) -> CaptainResult<Option<Goal>> {
        let removed = self.goals.remove(id).map(|(_, g)| g);
        if removed.is_some() {
            self.persist()?;
        }
        Ok(removed)
    }

    /// Replace an existing goal with a validated version. Returns the updated
    /// goal, or `None` when the id does not exist.
    pub fn update(&self, goal: Goal) -> CaptainResult<Option<Goal>> {
        goal.validate().map_err(CaptainError::InvalidInput)?;
        let id = goal.id.clone();
        let updated = match self.goals.get_mut(&id) {
            Some(mut entry) => {
                *entry = goal.clone();
                Some(goal)
            }
            None => None,
        };
        if updated.is_some() {
            self.persist()?;
        }
        Ok(updated)
    }

    pub fn get(&self, id: &str) -> Option<Goal> {
        self.goals.get(id).map(|r| r.value().clone())
    }

    pub fn list(&self) -> Vec<Goal> {
        self.goals.iter().map(|r| r.value().clone()).collect()
    }

    pub fn list_active(&self) -> Vec<Goal> {
        self.goals
            .iter()
            .filter(|r| r.value().status == GoalStatus::Active)
            .map(|r| r.value().clone())
            .collect()
    }

    pub fn list_for_project(&self, project_id: &str, project_slug: &str) -> Vec<Goal> {
        self.goals
            .iter()
            .filter(|r| {
                let goal = r.value();
                goal.project_id.as_deref() == Some(project_id)
                    || goal.project_slug.as_deref() == Some(project_slug)
            })
            .map(|r| r.value().clone())
            .collect()
    }

    pub fn remove_for_project(&self, project_id: &str, project_slug: &str) -> CaptainResult<usize> {
        let ids: Vec<String> = self
            .goals
            .iter()
            .filter(|r| {
                let goal = r.value();
                goal.project_id.as_deref() == Some(project_id)
                    || goal.project_slug.as_deref() == Some(project_slug)
            })
            .map(|r| r.key().clone())
            .collect();
        let removed = ids.len();
        for id in ids {
            self.goals.remove(&id);
        }
        if removed > 0 {
            self.persist()?;
        }
        Ok(removed)
    }

    /// Flip status. Returns true if the goal exists.
    pub fn set_status(&self, id: &str, status: GoalStatus) -> CaptainResult<bool> {
        let updated = match self.goals.get_mut(id) {
            Some(mut entry) => {
                entry.status = status;
                entry.updated_at = Utc::now();
                if status == GoalStatus::Escalated && entry.escalated_at.is_none() {
                    entry.escalated_at = Some(Utc::now());
                }
                true
            }
            None => false,
        };
        if updated {
            self.persist()?;
        }
        Ok(updated)
    }

    /// Append a check result to the ring buffer, update consecutive
    /// failure counter, and return the live `consecutive_fails` value
    /// (so the loop can decide whether to escalate). Persists on every
    /// call — cheap because the file is small (≤ 50 entries × N goals).
    pub fn record_check(&self, id: &str, result: CheckResult) -> CaptainResult<u32> {
        let fails = match self.goals.get_mut(id) {
            Some(mut entry) => {
                entry.last_check_ts = Some(result.ts);
                if result.ok {
                    entry.consecutive_fails = 0;
                } else {
                    entry.consecutive_fails = entry.consecutive_fails.saturating_add(1);
                }
                if entry.recent_checks.len() >= RECENT_CHECKS_CAPACITY {
                    entry.recent_checks.pop_front();
                }
                entry.recent_checks.push_back(result);
                entry.updated_at = Utc::now();
                entry.consecutive_fails
            }
            None => return Err(CaptainError::InvalidInput(format!("no such goal: {id}"))),
        };
        // Persist outside the DashMap guard to avoid holding the lock
        // through I/O.
        if let Err(e) = self.persist() {
            warn!("Failed to persist after record_check: {e}");
        }
        Ok(fails)
    }

    /// Sliding-window LLM rate limiter. Prunes entries older than 1h
    /// then checks the count against `max_llm_calls_per_hour`. If the
    /// budget remains, appends `Utc::now()` and returns true. If
    /// exhausted, returns false (caller must NOT issue the LLM call).
    pub fn try_consume_llm_quota(&self, id: &str) -> bool {
        let mut entry = match self.goals.get_mut(id) {
            Some(e) => e,
            None => return false,
        };
        let cutoff = Utc::now() - ChronoDuration::hours(1);
        entry.llm_call_log.retain(|t| *t > cutoff);
        let cap = entry.max_llm_calls_per_hour as usize;
        if entry.llm_call_log.len() >= cap {
            warn!(
                goal = %id,
                cap,
                "LLM quota exhausted for goal — refusing call"
            );
            return false;
        }
        entry.llm_call_log.push(Utc::now());
        true
    }

    /// R.2.2 — append a Pending suggestion to a goal. Prunes the oldest
    /// resolved (Applied/Rejected) entries first if we hit the cap.
    /// Pending entries are never auto-pruned: the user must explicitly
    /// resolve them.
    pub fn add_suggestion(&self, goal_id: &str, suggestion: Suggestion) -> CaptainResult<()> {
        match self.goals.get_mut(goal_id) {
            Some(mut entry) => {
                if entry.suggestions.len() >= MAX_SUGGESTIONS_PER_GOAL {
                    // Prune oldest resolved (drop Pending only as last resort).
                    let to_drop = entry
                        .suggestions
                        .iter()
                        .position(|s| s.status != SuggestionStatus::Pending);
                    match to_drop {
                        Some(idx) => {
                            entry.suggestions.remove(idx);
                        }
                        None => {
                            // All entries are Pending — drop the oldest one.
                            entry.suggestions.remove(0);
                        }
                    }
                }
                entry.suggestions.push(suggestion);
                entry.updated_at = Utc::now();
            }
            None => {
                return Err(CaptainError::InvalidInput(format!(
                    "no such goal: {goal_id}"
                )));
            }
        }
        self.persist()?;
        Ok(())
    }

    /// R.2.2 — apply a Pending suggestion to the goal it belongs to.
    /// Re-validates the resulting Goal so a malformed LLM proposal can
    /// not bypass the safety net (interval ≥ 10, threshold ≥ 1, etc.).
    /// Returns `true` if the suggestion existed AND was Pending.
    pub fn apply_suggestion(&self, goal_id: &str, suggestion_id: &str) -> CaptainResult<bool> {
        let applied = match self.goals.get_mut(goal_id) {
            Some(mut entry) => {
                let pos = entry
                    .suggestions
                    .iter()
                    .position(|s| s.id == suggestion_id && s.status == SuggestionStatus::Pending);
                let Some(idx) = pos else { return Ok(false) };

                // Snapshot the current goal so we can roll back if the
                // mutation produces an invalid state.
                let kind = entry.suggestions[idx].kind.clone();
                let snapshot_interval = entry.interval_secs;
                let snapshot_threshold = entry.escalation_threshold;
                let snapshot_recovery = entry.recovery_command.clone();

                match kind {
                    SuggestionKind::AdjustInterval { new_secs } => {
                        entry.interval_secs = new_secs;
                    }
                    SuggestionKind::AdjustThreshold { new_value } => {
                        entry.escalation_threshold = new_value;
                    }
                    SuggestionKind::EnableRecovery { command } => {
                        entry.recovery_command = Some(command);
                    }
                    SuggestionKind::DisableRecovery => {
                        entry.recovery_command = None;
                    }
                }

                if let Err(e) = entry.validate() {
                    // Roll back — refuse to persist an invalid goal.
                    entry.interval_secs = snapshot_interval;
                    entry.escalation_threshold = snapshot_threshold;
                    entry.recovery_command = snapshot_recovery;
                    return Err(CaptainError::InvalidInput(format!(
                        "applying suggestion would invalidate goal: {e}"
                    )));
                }

                entry.suggestions[idx].status = SuggestionStatus::Applied;
                entry.suggestions[idx].resolved_at = Some(Utc::now());
                entry.updated_at = Utc::now();
                true
            }
            None => {
                return Err(CaptainError::InvalidInput(format!(
                    "no such goal: {goal_id}"
                )))
            }
        };
        if applied {
            self.persist()?;
        }
        Ok(applied)
    }

    /// Mark a Pending suggestion as Rejected (no goal mutation).
    pub fn reject_suggestion(&self, goal_id: &str, suggestion_id: &str) -> CaptainResult<bool> {
        let rejected = match self.goals.get_mut(goal_id) {
            Some(mut entry) => match entry
                .suggestions
                .iter_mut()
                .find(|s| s.id == suggestion_id && s.status == SuggestionStatus::Pending)
            {
                Some(s) => {
                    s.status = SuggestionStatus::Rejected;
                    s.resolved_at = Some(Utc::now());
                    entry.updated_at = Utc::now();
                    true
                }
                None => false,
            },
            None => {
                return Err(CaptainError::InvalidInput(format!(
                    "no such goal: {goal_id}"
                )))
            }
        };
        if rejected {
            self.persist()?;
        }
        Ok(rejected)
    }

    /// All suggestions for a goal (any status).
    pub fn list_suggestions(&self, goal_id: &str) -> Vec<Suggestion> {
        self.goals
            .get(goal_id)
            .map(|r| r.value().suggestions.clone())
            .unwrap_or_default()
    }

    /// Number of LLM calls already counted in the current 1h window.
    /// Useful for `goal_status` reporting.
    pub fn llm_calls_last_hour(&self, id: &str) -> u32 {
        let entry = match self.goals.get(id) {
            Some(e) => e,
            None => return 0,
        };
        let cutoff = Utc::now() - ChronoDuration::hours(1);
        entry.llm_call_log.iter().filter(|t| **t > cutoff).count() as u32
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn sample_goal(id: &str) -> Goal {
        Goal {
            id: id.to_string(),
            name: "nginx-uptime".into(),
            description: "Keep nginx green on prod-server".into(),
            project_id: None,
            project_slug: None,
            status: GoalStatus::Active,
            interval_secs: 30,
            check_command: "systemctl status nginx".into(),
            recovery_command: Some("systemctl restart nginx".into()),
            escalation_threshold: 2,
            max_llm_calls_per_hour: 20,
            escalation_channel: None,
            created_at: Utc::now(),
            updated_at: Utc::now(),
            last_check_ts: None,
            consecutive_fails: 0,
            escalated_at: None,
            recent_checks: VecDeque::new(),
            llm_call_log: Vec::new(),
            suggestions: Vec::new(),
        }
    }

    #[test]
    fn validate_accepts_well_formed_goal() {
        assert!(sample_goal("nginx-prod").validate().is_ok());
    }

    #[test]
    fn validate_rejects_short_or_long_id() {
        let mut g = sample_goal("ab");
        assert!(g.validate().is_err());
        g.id = "x".repeat(70);
        assert!(g.validate().is_err());
    }

    #[test]
    fn validate_rejects_bad_id_chars() {
        let g = sample_goal("nginx prod");
        assert!(g.validate().is_err());
        let g = sample_goal("nginx/prod");
        assert!(g.validate().is_err());
    }

    #[test]
    fn validate_rejects_too_short_interval() {
        let mut g = sample_goal("nginx-prod");
        g.interval_secs = 5;
        assert!(g.validate().is_err());
    }

    #[test]
    fn validate_rejects_zero_threshold() {
        let mut g = sample_goal("nginx-prod");
        g.escalation_threshold = 0;
        assert!(g.validate().is_err());
    }

    #[test]
    fn validate_rejects_critical_patterns_in_check_command() {
        let mut g = sample_goal("nginx-prod");
        g.check_command = "echo hello && rm -rf / --no-preserve-root".into();
        assert!(g.validate().is_err());
    }

    #[test]
    fn validate_rejects_critical_patterns_in_recovery_command() {
        let mut g = sample_goal("nginx-prod");
        g.recovery_command = Some("dd if=/dev/zero of=/dev/sda".into());
        assert!(g.validate().is_err());
    }

    #[test]
    fn validate_rejects_excessive_llm_cap() {
        let mut g = sample_goal("nginx-prod");
        g.max_llm_calls_per_hour = MAX_LLM_CALLS_PER_HOUR_CEILING + 1;
        assert!(g.validate().is_err());
    }

    #[test]
    fn add_persist_load_roundtrip() {
        let dir = TempDir::new().unwrap();
        let store = GoalStore::new(dir.path());
        store.add(sample_goal("nginx-prod")).unwrap();
        store.add(sample_goal("redis-prod")).unwrap();

        let store2 = GoalStore::new(dir.path());
        let n = store2.load().unwrap();
        assert_eq!(n, 2);
        assert!(store2.get("nginx-prod").is_some());
        assert!(store2.get("redis-prod").is_some());
    }

    #[test]
    fn add_rejects_duplicate_id() {
        let dir = TempDir::new().unwrap();
        let store = GoalStore::new(dir.path());
        store.add(sample_goal("nginx-prod")).unwrap();
        let err = store.add(sample_goal("nginx-prod")).unwrap_err();
        assert!(err.to_string().contains("already exists"));
    }

    #[test]
    fn add_rejects_invalid_goal_without_writing() {
        let dir = TempDir::new().unwrap();
        let store = GoalStore::new(dir.path());
        let mut bad = sample_goal("nginx-prod");
        bad.interval_secs = 1;
        assert!(store.add(bad).is_err());
        assert!(!dir.path().join("goals.json").exists());
    }

    #[test]
    fn record_check_increments_then_resets_on_success() {
        let dir = TempDir::new().unwrap();
        let store = GoalStore::new(dir.path());
        store.add(sample_goal("nginx-prod")).unwrap();

        let n = store
            .record_check("nginx-prod", CheckResult::new(false, "exit 1".into(), 12))
            .unwrap();
        assert_eq!(n, 1);
        let n = store
            .record_check("nginx-prod", CheckResult::new(false, "exit 1".into(), 14))
            .unwrap();
        assert_eq!(n, 2);
        let n = store
            .record_check("nginx-prod", CheckResult::new(true, "active".into(), 8))
            .unwrap();
        assert_eq!(n, 0);
    }

    #[test]
    fn record_check_caps_recent_history_at_50() {
        let dir = TempDir::new().unwrap();
        let store = GoalStore::new(dir.path());
        store.add(sample_goal("nginx-prod")).unwrap();
        for i in 0..120 {
            let _ = store
                .record_check("nginx-prod", CheckResult::new(true, format!("ok {i}"), 1))
                .unwrap();
        }
        let g = store.get("nginx-prod").unwrap();
        assert_eq!(g.recent_checks.len(), RECENT_CHECKS_CAPACITY);
        // Oldest dropped, newest preserved.
        assert!(g.recent_checks.back().unwrap().output.contains("ok 119"));
        assert!(g.recent_checks.front().unwrap().output.contains("ok 70"));
    }

    #[test]
    fn check_result_truncates_huge_output() {
        let huge = "x".repeat(10_000);
        let r = CheckResult::new(false, huge, 0);
        assert!(r.output.len() < 5000);
        assert!(r.output.ends_with("…[truncated]"));
    }

    #[test]
    fn try_consume_llm_quota_respects_max() {
        let dir = TempDir::new().unwrap();
        let store = GoalStore::new(dir.path());
        let mut g = sample_goal("nginx-prod");
        g.max_llm_calls_per_hour = 3;
        store.add(g).unwrap();
        assert!(store.try_consume_llm_quota("nginx-prod"));
        assert!(store.try_consume_llm_quota("nginx-prod"));
        assert!(store.try_consume_llm_quota("nginx-prod"));
        // 4th call refused
        assert!(!store.try_consume_llm_quota("nginx-prod"));
        assert_eq!(store.llm_calls_last_hour("nginx-prod"), 3);
    }

    #[test]
    fn try_consume_llm_quota_returns_false_for_unknown_goal() {
        let dir = TempDir::new().unwrap();
        let store = GoalStore::new(dir.path());
        assert!(!store.try_consume_llm_quota("ghost"));
    }

    #[test]
    fn set_status_paused_then_escalated_records_timestamp() {
        let dir = TempDir::new().unwrap();
        let store = GoalStore::new(dir.path());
        store.add(sample_goal("nginx-prod")).unwrap();

        store.set_status("nginx-prod", GoalStatus::Paused).unwrap();
        assert_eq!(store.get("nginx-prod").unwrap().status, GoalStatus::Paused);
        assert!(store.get("nginx-prod").unwrap().escalated_at.is_none());

        store
            .set_status("nginx-prod", GoalStatus::Escalated)
            .unwrap();
        let g = store.get("nginx-prod").unwrap();
        assert_eq!(g.status, GoalStatus::Escalated);
        assert!(g.escalated_at.is_some());
    }

    #[test]
    fn list_active_filters_paused() {
        let dir = TempDir::new().unwrap();
        let store = GoalStore::new(dir.path());
        store.add(sample_goal("nginx-prod")).unwrap();
        store.add(sample_goal("redis-prod")).unwrap();
        store.set_status("redis-prod", GoalStatus::Paused).unwrap();

        let active = store.list_active();
        assert_eq!(active.len(), 1);
        assert_eq!(active[0].id, "nginx-prod");
    }

    #[test]
    fn list_and_remove_project_goals_only_touch_matching_project() {
        let dir = TempDir::new().unwrap();
        let store = GoalStore::new(dir.path());
        let mut project_goal = sample_goal("nginx-prod");
        project_goal.project_id = Some("project-1".into());
        project_goal.project_slug = Some("demo".into());
        store.add(project_goal).unwrap();
        store.add(sample_goal("redis-prod")).unwrap();

        let listed = store.list_for_project("project-1", "demo");
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].id, "nginx-prod");

        let removed = store.remove_for_project("project-1", "demo").unwrap();
        assert_eq!(removed, 1);
        assert!(store.get("nginx-prod").is_none());
        assert!(store.get("redis-prod").is_some());
    }

    #[test]
    fn update_validates_and_persists_existing_goal() {
        let dir = TempDir::new().unwrap();
        let store = GoalStore::new(dir.path());
        store.add(sample_goal("nginx-prod")).unwrap();

        let mut goal = store.get("nginx-prod").unwrap();
        goal.name = "nginx-live".into();
        goal.check_command = "systemctl status nginx --no-pager".into();
        goal.interval_secs = 120;
        goal.updated_at = Utc::now();

        let updated = store.update(goal).unwrap().unwrap();
        assert_eq!(updated.name, "nginx-live");
        assert_eq!(store.get("nginx-prod").unwrap().interval_secs, 120);

        let store2 = GoalStore::new(dir.path());
        store2.load().unwrap();
        assert_eq!(store2.get("nginx-prod").unwrap().name, "nginx-live");
    }

    #[test]
    fn update_rejects_invalid_goal_without_mutating() {
        let dir = TempDir::new().unwrap();
        let store = GoalStore::new(dir.path());
        store.add(sample_goal("nginx-prod")).unwrap();

        let mut goal = store.get("nginx-prod").unwrap();
        goal.interval_secs = 1;

        let err = store.update(goal).unwrap_err();
        assert!(err.to_string().contains("interval_secs"));
        assert_eq!(store.get("nginx-prod").unwrap().interval_secs, 30);
    }

    #[test]
    fn add_suggestion_appends_pending_entry() {
        let dir = TempDir::new().unwrap();
        let store = GoalStore::new(dir.path());
        store.add(sample_goal("nginx-prod")).unwrap();

        let s = Suggestion::new(
            SuggestionKind::AdjustInterval { new_secs: 60 },
            "Average latency suggests slower polling is fine".into(),
        );
        store.add_suggestion("nginx-prod", s.clone()).unwrap();
        let listed = store.list_suggestions("nginx-prod");
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].id, s.id);
        assert_eq!(listed[0].status, SuggestionStatus::Pending);
    }

    #[test]
    fn add_suggestion_for_unknown_goal_errors() {
        let dir = TempDir::new().unwrap();
        let store = GoalStore::new(dir.path());
        let s = Suggestion::new(SuggestionKind::DisableRecovery, "n/a".into());
        let res = store.add_suggestion("ghost", s);
        assert!(res.is_err());
    }

    #[test]
    fn apply_suggestion_mutates_goal_and_marks_applied() {
        let dir = TempDir::new().unwrap();
        let store = GoalStore::new(dir.path());
        store.add(sample_goal("nginx-prod")).unwrap();
        let s = Suggestion::new(
            SuggestionKind::AdjustInterval { new_secs: 120 },
            "Quieter polling".into(),
        );
        let sid = s.id.clone();
        store.add_suggestion("nginx-prod", s).unwrap();

        let ok = store.apply_suggestion("nginx-prod", &sid).unwrap();
        assert!(ok);
        let g = store.get("nginx-prod").unwrap();
        assert_eq!(g.interval_secs, 120);
        let s = g.suggestions.iter().find(|s| s.id == sid).unwrap();
        assert_eq!(s.status, SuggestionStatus::Applied);
        assert!(s.resolved_at.is_some());
    }

    #[test]
    fn apply_suggestion_revalidates_goal_and_rolls_back() {
        let dir = TempDir::new().unwrap();
        let store = GoalStore::new(dir.path());
        store.add(sample_goal("nginx-prod")).unwrap();
        // Suggestion proposing a sub-MIN_INTERVAL value
        let s = Suggestion::new(
            SuggestionKind::AdjustInterval { new_secs: 1 },
            "Way too aggressive".into(),
        );
        let sid = s.id.clone();
        store.add_suggestion("nginx-prod", s).unwrap();

        let res = store.apply_suggestion("nginx-prod", &sid);
        assert!(res.is_err(), "must refuse invalid mutation");
        let g = store.get("nginx-prod").unwrap();
        // Original value preserved
        assert_eq!(g.interval_secs, 30);
        // Suggestion still Pending (not Applied)
        let s = g.suggestions.iter().find(|s| s.id == sid).unwrap();
        assert_eq!(s.status, SuggestionStatus::Pending);
    }

    #[test]
    fn apply_suggestion_returns_false_for_already_applied() {
        let dir = TempDir::new().unwrap();
        let store = GoalStore::new(dir.path());
        store.add(sample_goal("nginx-prod")).unwrap();
        let s = Suggestion::new(
            SuggestionKind::AdjustThreshold { new_value: 5 },
            "More tolerant".into(),
        );
        let sid = s.id.clone();
        store.add_suggestion("nginx-prod", s).unwrap();
        assert!(store.apply_suggestion("nginx-prod", &sid).unwrap());
        // Second apply is a no-op
        assert!(!store.apply_suggestion("nginx-prod", &sid).unwrap());
    }

    #[test]
    fn reject_suggestion_keeps_goal_unchanged() {
        let dir = TempDir::new().unwrap();
        let store = GoalStore::new(dir.path());
        store.add(sample_goal("nginx-prod")).unwrap();
        let s = Suggestion::new(
            SuggestionKind::EnableRecovery {
                command: "systemctl restart redis".into(),
            },
            "Not the right service".into(),
        );
        let sid = s.id.clone();
        store.add_suggestion("nginx-prod", s).unwrap();
        assert!(store.reject_suggestion("nginx-prod", &sid).unwrap());
        let g = store.get("nginx-prod").unwrap();
        // recovery_command unchanged
        assert_eq!(
            g.recovery_command.as_deref(),
            Some("systemctl restart nginx")
        );
        let s = g.suggestions.iter().find(|s| s.id == sid).unwrap();
        assert_eq!(s.status, SuggestionStatus::Rejected);
    }

    #[test]
    fn add_suggestion_prunes_oldest_resolved_at_cap() {
        let dir = TempDir::new().unwrap();
        let store = GoalStore::new(dir.path());
        store.add(sample_goal("nginx-prod")).unwrap();
        // Fill with resolved suggestions
        for i in 0..MAX_SUGGESTIONS_PER_GOAL {
            let mut s = Suggestion::new(SuggestionKind::DisableRecovery, format!("filler {i}"));
            s.status = SuggestionStatus::Rejected;
            s.resolved_at = Some(Utc::now());
            store.add_suggestion("nginx-prod", s).unwrap();
        }
        // The next insert should evict the oldest resolved entry, not crash.
        let s = Suggestion::new(SuggestionKind::DisableRecovery, "newest".into());
        store.add_suggestion("nginx-prod", s).unwrap();
        let g = store.get("nginx-prod").unwrap();
        assert_eq!(g.suggestions.len(), MAX_SUGGESTIONS_PER_GOAL);
        assert!(g.suggestions.last().unwrap().reason.contains("newest"));
    }

    #[test]
    fn apply_suggestion_disable_recovery() {
        let dir = TempDir::new().unwrap();
        let store = GoalStore::new(dir.path());
        store.add(sample_goal("nginx-prod")).unwrap();
        let s = Suggestion::new(
            SuggestionKind::DisableRecovery,
            "Recovery is making things worse".into(),
        );
        let sid = s.id.clone();
        store.add_suggestion("nginx-prod", s).unwrap();
        assert!(store.apply_suggestion("nginx-prod", &sid).unwrap());
        let g = store.get("nginx-prod").unwrap();
        assert!(g.recovery_command.is_none());
    }

    #[test]
    fn remove_persists_and_returns_old_value() {
        let dir = TempDir::new().unwrap();
        let store = GoalStore::new(dir.path());
        store.add(sample_goal("nginx-prod")).unwrap();
        let removed = store.remove("nginx-prod").unwrap();
        assert!(removed.is_some());
        assert_eq!(removed.unwrap().id, "nginx-prod");
        // Reload from disk should give 0
        let store2 = GoalStore::new(dir.path());
        store2.load().unwrap();
        assert_eq!(store2.list().len(), 0);
    }
}
