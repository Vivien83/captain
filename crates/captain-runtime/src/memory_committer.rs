//! MemoryCommitter (v3.12f).
//!
//! Terminal stage of the learning pipeline. Given a classified batch
//! of policy-accepted candidates, it:
//!
//! 1. Routes each candidate to the canonical `(wing, room)` per the
//!    v3.12 taxonomy. The reflector's own suggestion is treated as
//!    advisory; the committer is the authority.
//! 2. If the agent has an active project and the outcome warrants it
//!    (`decisions` / `retrospective` outcomes), the wing is rewritten
//!    to `project:<slug>`.
//! 3. Hands every triple to `memory_writer::write_through` so the
//!    local SQLite buffer captures the row whether or not MemPalace
//!    is reachable.
//!
//! The committer is cheap and deterministic — no LLM, no I/O besides
//! SQLite + best-effort MCP — so it can sit in a hot path without
//! backpressuring upstream stages.

use rusqlite::Connection;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::{Arc, Mutex as StdMutex};
use std::time::{Duration, Instant};
use tokio::sync::Mutex as AsyncMutex;
use tracing::{debug, info, warn};

use crate::active_project;
use crate::mcp::McpConnection;
use crate::memory_writer::{self, McpMemPalaceSender, MemPalaceSender};
use crate::outcome_detector::Outcome;
use crate::reflection_job::{MemoryCandidate, ReflectionBatch};
use captain_types::config::LearningMode;

/// Canonical rooms used by the v3.12 LearningEngine.
pub const WING_LEARNINGS: &str = "learnings";
pub const ROOM_GENERAL: &str = "general";
pub const ROOM_FAILURES: &str = "failures";
pub const ROOM_WORKAROUNDS: &str = "workarounds";
pub const ROOM_USER_PREFERENCES: &str = "user_preferences";
pub const ROOM_DECISIONS: &str = "decisions";
pub const ROOM_RETROSPECTIVE: &str = "retrospective";

/// Result of a single committed write. The id comes from the
/// `memory_writes` row — the authoritative reference for later audit.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct CommittedLearning {
    pub id: String,
    pub wing: String,
    pub room: String,
    pub subject: String,
    pub predicate: String,
    pub object: String,
    pub confidence: f32,
    /// Commit-B — Haiku-assigned category propagated from the
    /// `MemoryCandidate`. `None` for legacy entries written before
    /// Commit-B.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub category: Option<String>,
    /// Commit-C — origin channel propagated from the upstream
    /// `ReflectionBatch`. Lets the kernel route the `🧠 mémorisé`
    /// notice back to the conversation it came from (Telegram chat,
    /// CLI session, …). `None` when the source had no channel context.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub channel: Option<String>,
}

/// Outcome of running a batch under a given `LearningMode`. In Auto
/// mode candidates are written through immediately (`Committed`). In
/// Approval mode they go to the review queue (`Queued`). Off mode
/// skips the batch entirely.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum CommitResult {
    Committed(CommittedLearning),
    Queued {
        review_id: String,
        wing: String,
        room: String,
        subject: String,
        /// Commit-D — full triple needed to render the approval prompt.
        #[serde(default)]
        predicate: String,
        #[serde(default)]
        object: String,
        /// Origin channel to route the approval request back to the
        /// conversation it came from.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        channel: Option<String>,
    },
}

/// Map an outcome to its default `(wing, room)`. The wing is always
/// `learnings`; project-scoped wings are resolved later from the
/// agent's active project.
pub fn default_room(outcome: Outcome) -> (&'static str, &'static str) {
    match outcome {
        Outcome::ExplicitRemember | Outcome::UserCorrected => {
            (WING_LEARNINGS, ROOM_USER_PREFERENCES)
        }
        Outcome::RetrySuccess => (WING_LEARNINGS, ROOM_WORKAROUNDS),
        Outcome::Failure => (WING_LEARNINGS, ROOM_FAILURES),
        Outcome::Cancelled => (WING_LEARNINGS, ROOM_FAILURES),
        // Room used at end-of-session narration — pairs well with a
        // project wing when the agent has one.
        Outcome::ConversationIdle => (WING_LEARNINGS, ROOM_RETROSPECTIVE),
        // Approval decisions and bare successes land in the generic
        // learnings/general bucket; the reflector emits [] most of
        // the time so this is rare.
        Outcome::Success
        | Outcome::ApprovalDecision
        | Outcome::ConversationTurn
        | Outcome::Unknown => (WING_LEARNINGS, ROOM_GENERAL),
    }
}

/// True for outcomes whose learning is naturally project-scoped. A
/// project wing override applies only when one of these outcomes
/// fires *and* the agent has an active project.
fn outcome_is_project_scoped(outcome: Outcome) -> bool {
    matches!(
        outcome,
        Outcome::RetrySuccess | Outcome::Failure | Outcome::ConversationIdle
    )
}

/// Apply routing to a single candidate. Overrides `wing` and `room`
/// based on the classified outcome and (if relevant) the agent's
/// active project. The reflector's proposal is discarded.
pub fn apply_routing(
    candidate: &MemoryCandidate,
    outcome: Outcome,
    agent_id: &str,
) -> MemoryCandidate {
    let (mut wing, mut room) = default_room(outcome);

    // Promote to project wing when applicable.
    if outcome_is_project_scoped(outcome) {
        if let Some(slug) = active_project::global().and_then(|r| r.get(agent_id)) {
            // Room picks project-scoped rooms only for the two variants
            // that have them; the rest stay in learnings/<room>.
            match outcome {
                Outcome::ConversationIdle => {
                    room = ROOM_RETROSPECTIVE;
                    // Leak-free: wing ends up owned via `format!` below.
                }
                Outcome::Failure | Outcome::RetrySuccess => {
                    room = ROOM_DECISIONS;
                }
                _ => {}
            }
            return MemoryCandidate {
                wing: format!("project:{slug}"),
                room: room.to_string(),
                subject: candidate.subject.clone(),
                predicate: candidate.predicate.clone(),
                object: candidate.object.clone(),
                confidence: candidate.confidence,
                category: candidate.category.clone(),
            };
        }
        // Preserve `wing` to avoid the unused-mut warning when no
        // active project is found.
        let _ = &mut wing;
    }

    MemoryCandidate {
        wing: wing.to_string(),
        room: room.to_string(),
        subject: candidate.subject.clone(),
        predicate: candidate.predicate.clone(),
        object: candidate.object.clone(),
        confidence: candidate.confidence,
        category: candidate.category.clone(),
    }
}

/// Number of consecutive commit failures for the same agent before the
/// committer starts spacing out its attempts.
const BACKOFF_THRESHOLD: u32 = 3;

/// Minimum time to wait, once backing off, before retrying an agent
/// whose recent commits keep failing.
const BACKOFF_WINDOW: Duration = Duration::from_secs(10 * 60);

/// Per-agent consecutive-failure state. Reset entirely on the first
/// success after a run of failures.
#[derive(Debug, Clone, Copy, Default)]
struct FailureTracker {
    consecutive: u32,
    last_failure: Option<Instant>,
    /// Whether the "backing off" line has already been logged for the
    /// failure run currently tracked here — keeps the log to one line
    /// per backoff episode instead of one per skipped batch.
    backoff_logged: bool,
}

/// Pure decision: given `consecutive` failures and the time elapsed
/// since the last one, should the agent be backed off from right now?
/// Free of `Instant`/locking so it's directly unit testable without
/// depending on wall-clock timing.
fn should_back_off(consecutive: u32, elapsed_since_last_failure: Option<Duration>) -> bool {
    consecutive >= BACKOFF_THRESHOLD
        && elapsed_since_last_failure.is_some_and(|elapsed| elapsed < BACKOFF_WINDOW)
}

/// Result of consulting the backoff state for a given agent.
enum BackoffDecision {
    /// No backoff in effect — proceed with the batch.
    Proceed,
    /// Backoff in effect — skip the batch. `should_log` is true only
    /// the first time we enter (or re-enter) the backoff window, so
    /// callers log a single line instead of one per skipped batch.
    Skip { should_log: bool },
}

pub struct MemoryCommitter {
    conn: Arc<StdMutex<Connection>>,
    mode: LearningMode,
    failures: StdMutex<HashMap<String, FailureTracker>>,
}

impl MemoryCommitter {
    pub fn new(conn: Arc<StdMutex<Connection>>) -> Self {
        Self::with_mode(conn, LearningMode::Auto)
    }

    pub fn with_mode(conn: Arc<StdMutex<Connection>>, mode: LearningMode) -> Self {
        Self {
            conn,
            mode,
            failures: StdMutex::new(HashMap::new()),
        }
    }

    /// Consult (and update) the backoff state for `agent_id`.
    fn evaluate_backoff(&self, agent_id: &str) -> BackoffDecision {
        let mut guard = self
            .failures
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        match guard.get_mut(agent_id) {
            Some(tracker)
                if should_back_off(
                    tracker.consecutive,
                    tracker
                        .last_failure
                        .map(|last| Instant::now().saturating_duration_since(last)),
                ) =>
            {
                let should_log = !tracker.backoff_logged;
                tracker.backoff_logged = true;
                BackoffDecision::Skip { should_log }
            }
            _ => BackoffDecision::Proceed,
        }
    }

    /// Record a commit failure for `agent_id`, growing its consecutive
    /// failure count.
    fn record_failure(&self, agent_id: &str) {
        let mut guard = self
            .failures
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let tracker = guard.entry(agent_id.to_string()).or_default();
        tracker.consecutive += 1;
        tracker.last_failure = Some(Instant::now());
    }

    /// Record a commit success for `agent_id` — clears any backoff
    /// state so the next failure run starts from zero.
    fn record_success(&self, agent_id: &str) {
        let mut guard = self
            .failures
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        guard.remove(agent_id);
    }

    pub fn mode(&self) -> LearningMode {
        self.mode
    }

    /// Commit one routed candidate via `write_through`.
    ///
    /// `channel` carries the origin canal of the upstream signal so the
    /// committed entry can later be routed back (e.g. the `🧠` notice
    /// returns to the Telegram chat the user was talking from). Pass
    /// `None` when committing from a context with no canal (background
    /// pipelines, legacy paths, tests).
    pub async fn commit_one(
        &self,
        routed: &MemoryCandidate,
        sender: Option<&dyn MemPalaceSender>,
        source: &str,
        channel: Option<&str>,
    ) -> Result<CommittedLearning, String> {
        let record = captain_memory::memory_writer::NewMemoryWrite {
            subject: routed.subject.clone(),
            predicate: routed.predicate.clone(),
            object: routed.object.clone(),
            wing: Some(routed.wing.clone()),
            room: Some(routed.room.clone()),
            source: source.to_string(),
        };
        let id = memory_writer::write_through(Arc::clone(&self.conn), sender, record).await?;
        Ok(CommittedLearning {
            id,
            wing: routed.wing.clone(),
            room: routed.room.clone(),
            subject: routed.subject.clone(),
            predicate: routed.predicate.clone(),
            object: routed.object.clone(),
            confidence: routed.confidence,
            category: routed.category.clone(),
            channel: channel.map(|s| s.to_string()),
        })
    }

    /// Enqueue a routed candidate for human review. Returns the queue
    /// row id. Never touches MemPalace — the write happens later when
    /// the user approves via `approve_pending`.
    pub fn queue_for_review(
        &self,
        routed: &MemoryCandidate,
        outcome: Outcome,
        agent_id: &str,
        source: &str,
    ) -> Result<String, String> {
        let input = captain_memory::learning_review::NewReviewItem {
            outcome: format!("{outcome:?}").to_ascii_lowercase(),
            agent_id: agent_id.to_string(),
            wing: routed.wing.clone(),
            room: routed.room.clone(),
            subject: routed.subject.clone(),
            predicate: routed.predicate.clone(),
            object: routed.object.clone(),
            confidence: routed.confidence,
            source: source.to_string(),
        };
        let guard = self
            .conn
            .lock()
            .map_err(|e| format!("sqlite poisoned: {e}"))?;
        captain_memory::learning_review::enqueue(&guard, input)
            .map(|item| item.id)
            .map_err(|e| format!("enqueue: {e}"))
    }

    /// Commit an entire batch. Behaviour depends on `self.mode`:
    /// - `Off`: returns empty — caller should skip.
    /// - `Auto`: write_through every candidate (results are `Committed`).
    /// - `Approval`: enqueue every candidate (results are `Queued`).
    ///
    /// Single-candidate failures are logged (with their exact cause,
    /// the agent and the mode) and skipped so a bad entry never loses
    /// the whole batch. After `BACKOFF_THRESHOLD` consecutive failures
    /// for the same agent, subsequent batches are skipped entirely
    /// until `BACKOFF_WINDOW` has elapsed since the last failure.
    pub async fn commit_batch(
        &self,
        batch: &ReflectionBatch,
        sender: Option<&dyn MemPalaceSender>,
    ) -> Vec<CommitResult> {
        if matches!(self.mode, LearningMode::Off) {
            return Vec::new();
        }

        if let BackoffDecision::Skip { should_log } = self.evaluate_backoff(&batch.agent_id) {
            if should_log {
                warn!(
                    agent = %batch.agent_id,
                    mode = ?self.mode,
                    "memory committer backing off"
                );
            }
            return Vec::new();
        }

        let mut out = Vec::with_capacity(batch.candidates.len());
        let source = source_label(batch.outcome);

        for c in &batch.candidates {
            let routed = apply_routing(c, batch.outcome, &batch.agent_id);
            match self.mode {
                LearningMode::Auto => {
                    match self
                        .commit_one(&routed, sender, source, batch.channel.as_deref())
                        .await
                    {
                        Ok(committed) => {
                            self.record_success(&batch.agent_id);
                            out.push(CommitResult::Committed(committed));
                        }
                        Err(e) => {
                            self.record_failure(&batch.agent_id);
                            warn!(
                                error = %e,
                                subject = %c.subject,
                                agent = %batch.agent_id,
                                mode = ?self.mode,
                                "memory_committer auto: skipping candidate"
                            );
                        }
                    }
                }
                LearningMode::Approval => {
                    match self.queue_for_review(&routed, batch.outcome, &batch.agent_id, source) {
                        Ok(id) => {
                            self.record_success(&batch.agent_id);
                            out.push(CommitResult::Queued {
                                review_id: id,
                                wing: routed.wing.clone(),
                                room: routed.room.clone(),
                                subject: routed.subject.clone(),
                                predicate: routed.predicate.clone(),
                                object: routed.object.clone(),
                                channel: batch.channel.clone(),
                            });
                        }
                        Err(e) => {
                            self.record_failure(&batch.agent_id);
                            warn!(
                                error = %e,
                                subject = %c.subject,
                                agent = %batch.agent_id,
                                mode = ?self.mode,
                                "memory_committer approval: enqueue failed"
                            );
                        }
                    }
                }
                LearningMode::Off => unreachable!("handled above"),
            }
        }

        if !out.is_empty() {
            info!(
                count = out.len(),
                outcome = ?batch.outcome,
                agent = %batch.agent_id,
                mode = ?self.mode,
                "memory_committer: batch processed"
            );
        }
        out
    }

    /// Approve a pending review item: decide-approved, then
    /// write_through. Returns the CommittedLearning on success.
    pub async fn approve_pending(
        &self,
        review_id: &str,
        decided_by: Option<&str>,
        sender: Option<&dyn MemPalaceSender>,
    ) -> Result<CommittedLearning, String> {
        let item = {
            let guard = self
                .conn
                .lock()
                .map_err(|e| format!("sqlite poisoned: {e}"))?;
            captain_memory::learning_review::decide(
                &guard,
                review_id,
                captain_memory::learning_review::Decision::Approved,
                decided_by,
            )
            .map_err(|e| format!("decide: {e}"))?
        };

        let routed = MemoryCandidate {
            wing: item.wing.clone(),
            room: item.room.clone(),
            subject: item.subject.clone(),
            predicate: item.predicate.clone(),
            object: item.object.clone(),
            confidence: item.confidence,
            category: None,
        };
        // Approval-path: the review item doesn't carry the original channel
        // (legacy schema). Pass None — the user is approving from the dashboard
        // anyway, so there's no obvious chat to route the notice back to.
        let committed = self.commit_one(&routed, sender, &item.source, None).await?;
        {
            let guard = self
                .conn
                .lock()
                .map_err(|e| format!("sqlite poisoned: {e}"))?;
            let _ = captain_memory::learning_review::mark_written_write_id(
                &guard,
                review_id,
                &committed.id,
            );
        }
        Ok(committed)
    }

    /// Deny a pending review item — no MemPalace call, no write_through.
    pub fn deny_pending(&self, review_id: &str, decided_by: Option<&str>) -> Result<(), String> {
        let guard = self
            .conn
            .lock()
            .map_err(|e| format!("sqlite poisoned: {e}"))?;
        captain_memory::learning_review::decide(
            &guard,
            review_id,
            captain_memory::learning_review::Decision::Denied,
            decided_by,
        )
        .map(|_| ())
        .map_err(|e| format!("decide: {e}"))
    }
}

fn source_label(outcome: Outcome) -> &'static str {
    match outcome {
        Outcome::ExplicitRemember => "learning.explicit_remember",
        Outcome::UserCorrected => "learning.user_correction",
        Outcome::RetrySuccess => "learning.retry_success",
        Outcome::Failure => "learning.failure",
        Outcome::Cancelled => "learning.cancelled",
        Outcome::ConversationIdle => "learning.retrospective",
        Outcome::Success => "learning.success",
        Outcome::ApprovalDecision => "learning.approval",
        Outcome::ConversationTurn => "learning.conversation_turn",
        Outcome::Unknown => "learning.unknown",
    }
}

/// Phase O.2: notification hook fired right after a candidate is
/// committed (synced or pending) to MemPalace. The kernel implements
/// this trait to bridge to its `event_bus`, broadcasting a
/// `ChatStreamEvent::MemoryStored` so connected clients (web, CLI,
/// Telegram) can render a discreet 🧠 line in the chat. Optional —
/// when `None`, the committer behaves exactly like before.
#[async_trait::async_trait]
pub trait CommitNotifier: Send + Sync {
    async fn on_committed(&self, committed: &CommittedLearning, source: &str);

    /// Commit-D — fired when a candidate is queued for human review
    /// (LearningMode::Approval). Allows the kernel to broadcast a
    /// `MemoryQueued` event so each surface (Telegram, CLI, …) can
    /// surface an interactive approval prompt to the user.
    ///
    /// Default no-op so existing notifiers compile unchanged.
    async fn on_queued(
        &self,
        review_id: &str,
        subject: &str,
        predicate: &str,
        object: &str,
        channel: Option<&str>,
        source: &str,
    ) {
        let _ = (review_id, subject, predicate, object, channel, source);
    }
}

/// Spawn the committer consumer. Reads `ReflectionBatch`es, runs the
/// commit flow appropriate for the chosen `LearningMode`, and forwards
/// the mixed `CommitResult` stream to the returned receiver. The UI /
/// dashboard (v3.12h) tails this channel to show recent commits AND
/// pending queue entries.
pub fn spawn_consumer(
    mut rx: tokio::sync::mpsc::Receiver<ReflectionBatch>,
    conn: Arc<StdMutex<Connection>>,
    mcp_conns: Arc<AsyncMutex<Vec<McpConnection>>>,
    mode: LearningMode,
    output_capacity: usize,
    reflection_model_label: String,
    notifier: Option<Arc<dyn CommitNotifier>>,
) -> (
    tokio::task::JoinHandle<()>,
    tokio::sync::mpsc::Receiver<Vec<CommitResult>>,
) {
    let (tx, out_rx) = tokio::sync::mpsc::channel(output_capacity);
    let handle = tokio::spawn(async move {
        let committer = MemoryCommitter::with_mode(conn, mode);
        while let Some(batch) = rx.recv().await {
            let sender = McpMemPalaceSender {
                mcp_conns: &mcp_conns,
            };
            let sender_ref: Option<&dyn MemPalaceSender> = Some(&sender);
            let results = committer.commit_batch(&batch, sender_ref).await;
            if results.is_empty() {
                continue;
            }
            if let Some(n) = &notifier {
                for r in &results {
                    match r {
                        CommitResult::Committed(c) => {
                            n.on_committed(c, &reflection_model_label).await;
                        }
                        CommitResult::Queued {
                            review_id,
                            subject,
                            predicate,
                            object,
                            channel,
                            ..
                        } => {
                            n.on_queued(
                                review_id,
                                subject,
                                predicate,
                                object,
                                channel.as_deref(),
                                &reflection_model_label,
                            )
                            .await;
                        }
                    }
                }
            }
            if let Err(e) = tx.try_send(results) {
                debug!(error = %e, "committer consumer: downstream full");
            }
        }
    });
    (handle, out_rx)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;
    use captain_memory::memory_writer as store;
    use captain_memory::migration::run_migrations;

    fn fresh_db() -> Arc<StdMutex<Connection>> {
        let conn = Connection::open_in_memory().unwrap();
        run_migrations(&conn).unwrap();
        Arc::new(StdMutex::new(conn))
    }

    fn cand(subject: &str, object: &str) -> MemoryCandidate {
        MemoryCandidate {
            wing: "LLM_said".into(),
            room: "LLM_said".into(),
            subject: subject.into(),
            predicate: "prefers".into(),
            object: object.into(),
            confidence: 0.9,
            category: None,
        }
    }

    struct OkSender;
    #[async_trait]
    impl MemPalaceSender for OkSender {
        async fn send(
            &self,
            _row: &captain_memory::memory_writer::MemoryWrite,
        ) -> Result<(), String> {
            Ok(())
        }
    }

    // ---- routing ----

    #[test]
    fn routing_explicit_remember_to_user_preferences() {
        assert_eq!(
            default_room(Outcome::ExplicitRemember),
            (WING_LEARNINGS, ROOM_USER_PREFERENCES)
        );
    }

    #[test]
    fn routing_user_corrected_to_user_preferences() {
        assert_eq!(
            default_room(Outcome::UserCorrected),
            (WING_LEARNINGS, ROOM_USER_PREFERENCES)
        );
    }

    #[test]
    fn routing_retry_success_to_workarounds() {
        assert_eq!(
            default_room(Outcome::RetrySuccess),
            (WING_LEARNINGS, ROOM_WORKAROUNDS)
        );
    }

    #[test]
    fn routing_failure_to_failures() {
        assert_eq!(
            default_room(Outcome::Failure),
            (WING_LEARNINGS, ROOM_FAILURES)
        );
    }

    #[test]
    fn routing_conversation_idle_to_retrospective() {
        assert_eq!(
            default_room(Outcome::ConversationIdle),
            (WING_LEARNINGS, ROOM_RETROSPECTIVE)
        );
    }

    #[test]
    fn routing_success_to_general() {
        assert_eq!(
            default_room(Outcome::Success),
            (WING_LEARNINGS, ROOM_GENERAL)
        );
    }

    #[test]
    fn apply_routing_overrides_reflector_wing_room() {
        let c = cand("user", "likes coffee black and strong in the morning");
        let r = apply_routing(&c, Outcome::ExplicitRemember, "agent-123");
        assert_eq!(r.wing, WING_LEARNINGS);
        assert_eq!(r.room, ROOM_USER_PREFERENCES);
        assert_eq!(r.subject, "user");
    }

    #[test]
    fn apply_routing_without_active_project_stays_in_learnings() {
        // No active project installed in this test process.
        let c = cand("db", "fails on concurrent writes under load daily");
        let r = apply_routing(&c, Outcome::Failure, "agent-123");
        assert_eq!(r.wing, WING_LEARNINGS);
        assert_eq!(r.room, ROOM_FAILURES);
    }

    #[test]
    fn apply_routing_preserves_triple_content() {
        let c = cand(
            "retry_logic",
            "succeeded after exponential backoff when API returned 503",
        );
        let r = apply_routing(&c, Outcome::RetrySuccess, "agent-x");
        assert_eq!(r.subject, c.subject);
        assert_eq!(r.predicate, c.predicate);
        assert_eq!(r.object, c.object);
        assert!((r.confidence - c.confidence).abs() < f32::EPSILON);
    }

    // ---- commit_one ----

    #[tokio::test]
    async fn commit_one_writes_local_row_and_marks_synced_with_ok_sender() {
        let db = fresh_db();
        let committer = MemoryCommitter::new(Arc::clone(&db));
        let routed = MemoryCandidate {
            wing: WING_LEARNINGS.into(),
            room: ROOM_USER_PREFERENCES.into(),
            subject: "user".into(),
            predicate: "prefers".into(),
            object: "dark mode UI on displays brighter than 100 nits".into(),
            confidence: 0.95,
            category: None,
        };
        let committed = committer
            .commit_one(&routed, Some(&OkSender), "learning.explicit_remember", None)
            .await
            .unwrap();
        assert_eq!(committed.wing, WING_LEARNINGS);
        assert_eq!(committed.room, ROOM_USER_PREFERENCES);
        assert_eq!(committed.channel, None);

        // Verify persisted row + sync status.
        let guard = db.lock().unwrap();
        let row = store::get(&guard, &committed.id).unwrap().unwrap();
        assert_eq!(row.sync_status, store::SyncStatus::Synced);
        assert_eq!(row.wing.as_deref(), Some(WING_LEARNINGS));
        assert_eq!(row.room.as_deref(), Some(ROOM_USER_PREFERENCES));
    }

    #[tokio::test]
    async fn commit_one_without_sender_keeps_row_pending() {
        let db = fresh_db();
        let committer = MemoryCommitter::new(Arc::clone(&db));
        let routed = MemoryCandidate {
            wing: WING_LEARNINGS.into(),
            room: ROOM_FAILURES.into(),
            subject: "migration_v7".into(),
            predicate: "fails_if".into(),
            object: "run without PRAGMA foreign_keys=ON in the same transaction".into(),
            confidence: 0.8,
            category: None,
        };
        let committed = committer
            .commit_one(&routed, None, "learning.failure", Some("telegram"))
            .await
            .unwrap();
        let guard = db.lock().unwrap();
        let row = store::get(&guard, &committed.id).unwrap().unwrap();
        assert_eq!(row.sync_status, store::SyncStatus::Pending);
        // Commit-C: channel is faithfully echoed back through the committer.
        assert_eq!(committed.channel.as_deref(), Some("telegram"));
    }

    // ---- commit_batch ----

    #[tokio::test]
    async fn commit_batch_auto_mode_commits_every_candidate() {
        let db = fresh_db();
        let committer = MemoryCommitter::with_mode(Arc::clone(&db), LearningMode::Auto);
        let batch = ReflectionBatch {
            outcome: Outcome::ExplicitRemember,
            agent_id: "agent-x".into(),
            candidates: vec![
                cand("user", "likes coffee black and strong in the morning"),
                cand("user", "dislikes animations longer than 300ms on buttons"),
            ],
            channel: None,
        };
        let out = committer.commit_batch(&batch, Some(&OkSender)).await;
        assert_eq!(out.len(), 2);
        for r in &out {
            match r {
                CommitResult::Committed(c) => {
                    assert_eq!(c.wing, WING_LEARNINGS);
                    assert_eq!(c.room, ROOM_USER_PREFERENCES);
                }
                CommitResult::Queued { .. } => panic!("auto mode should not queue"),
            }
        }
    }

    #[tokio::test]
    async fn commit_batch_empty_returns_empty() {
        let db = fresh_db();
        let committer = MemoryCommitter::new(Arc::clone(&db));
        let batch = ReflectionBatch {
            outcome: Outcome::Success,
            agent_id: "agent-x".into(),
            candidates: vec![],
            channel: None,
        };
        let out = committer.commit_batch(&batch, Some(&OkSender)).await;
        assert!(out.is_empty());
    }

    #[tokio::test]
    async fn commit_batch_source_label_matches_outcome() {
        let db = fresh_db();
        let committer = MemoryCommitter::new(Arc::clone(&db));
        let batch = ReflectionBatch {
            outcome: Outcome::RetrySuccess,
            agent_id: "agent-x".into(),
            candidates: vec![cand(
                "shell_exec",
                "succeeds on the 2nd attempt after 500ms sleep on Darwin",
            )],
            channel: None,
        };
        let out = committer.commit_batch(&batch, Some(&OkSender)).await;
        let id = match &out[0] {
            CommitResult::Committed(c) => c.id.clone(),
            _ => panic!("expected committed"),
        };
        let guard = db.lock().unwrap();
        let row = store::get(&guard, &id).unwrap().unwrap();
        assert_eq!(row.source, "learning.retry_success");
    }

    #[tokio::test]
    async fn commit_batch_off_mode_skips() {
        let db = fresh_db();
        let committer = MemoryCommitter::with_mode(Arc::clone(&db), LearningMode::Off);
        let batch = ReflectionBatch {
            outcome: Outcome::ExplicitRemember,
            agent_id: "agent-x".into(),
            candidates: vec![cand("user", "likes coffee black and strong in the morning")],
            channel: None,
        };
        let out = committer.commit_batch(&batch, Some(&OkSender)).await;
        assert!(out.is_empty());
        // No row written.
        let guard = db.lock().unwrap();
        let pending = captain_memory::memory_writer::list_pending(&guard, 10).unwrap();
        assert!(pending.is_empty());
    }

    #[tokio::test]
    async fn commit_batch_approval_mode_enqueues_and_does_not_write_through() {
        let db = fresh_db();
        let committer = MemoryCommitter::with_mode(Arc::clone(&db), LearningMode::Approval);
        let batch = ReflectionBatch {
            outcome: Outcome::ExplicitRemember,
            agent_id: "agent-x".into(),
            candidates: vec![
                cand("user", "likes coffee black and strong in the morning"),
                cand("user", "dislikes animations longer than 300ms on buttons"),
            ],
            channel: None,
        };
        let out = committer.commit_batch(&batch, Some(&OkSender)).await;
        assert_eq!(out.len(), 2);
        for r in &out {
            match r {
                CommitResult::Queued { wing, room, .. } => {
                    assert_eq!(wing, WING_LEARNINGS);
                    assert_eq!(room, ROOM_USER_PREFERENCES);
                }
                CommitResult::Committed(_) => panic!("approval mode should queue"),
            }
        }
        // memory_writes stays empty; review queue has 2 pending.
        let guard = db.lock().unwrap();
        let mem_pending = captain_memory::memory_writer::list_pending(&guard, 10).unwrap();
        assert!(mem_pending.is_empty());
        let review_pending = captain_memory::learning_review::list_pending(&guard, 10).unwrap();
        assert_eq!(review_pending.len(), 2);
    }

    #[tokio::test]
    async fn approve_pending_writes_through_and_marks_written_id() {
        let db = fresh_db();
        let committer = MemoryCommitter::with_mode(Arc::clone(&db), LearningMode::Approval);
        let batch = ReflectionBatch {
            outcome: Outcome::ExplicitRemember,
            agent_id: "agent-x".into(),
            candidates: vec![cand("user", "likes coffee black and strong in the morning")],
            channel: None,
        };
        let out = committer.commit_batch(&batch, Some(&OkSender)).await;
        let review_id = match &out[0] {
            CommitResult::Queued { review_id, .. } => review_id.clone(),
            _ => panic!("expected queued"),
        };

        let committed = committer
            .approve_pending(&review_id, Some("reviewer"), Some(&OkSender))
            .await
            .unwrap();

        // memory_writes row exists and is synced.
        let guard = db.lock().unwrap();
        let row = captain_memory::memory_writer::get(&guard, &committed.id)
            .unwrap()
            .unwrap();
        assert_eq!(
            row.sync_status,
            captain_memory::memory_writer::SyncStatus::Synced
        );

        // Review item decided + linked.
        let review = captain_memory::learning_review::get(&guard, &review_id)
            .unwrap()
            .unwrap();
        assert_eq!(
            review.decision,
            Some(captain_memory::learning_review::Decision::Approved)
        );
        assert_eq!(
            review.written_write_id.as_deref(),
            Some(committed.id.as_str())
        );
    }

    #[tokio::test]
    async fn deny_pending_marks_denied_without_writing() {
        let db = fresh_db();
        let committer = MemoryCommitter::with_mode(Arc::clone(&db), LearningMode::Approval);
        let batch = ReflectionBatch {
            outcome: Outcome::ExplicitRemember,
            agent_id: "agent-x".into(),
            candidates: vec![cand("user", "likes coffee black and strong in the morning")],
            channel: None,
        };
        let out = committer.commit_batch(&batch, Some(&OkSender)).await;
        let review_id = match &out[0] {
            CommitResult::Queued { review_id, .. } => review_id.clone(),
            _ => panic!("expected queued"),
        };

        committer
            .deny_pending(&review_id, Some("reviewer"))
            .unwrap();

        let guard = db.lock().unwrap();
        let review = captain_memory::learning_review::get(&guard, &review_id)
            .unwrap()
            .unwrap();
        assert_eq!(
            review.decision,
            Some(captain_memory::learning_review::Decision::Denied)
        );
        assert!(review.written_write_id.is_none());
        // No memory_writes row created.
        let mem_pending = captain_memory::memory_writer::list_pending(&guard, 10).unwrap();
        assert!(mem_pending.is_empty());
    }

    // ---- backoff ----

    #[test]
    fn should_back_off_pure_below_threshold_never_backs_off() {
        assert!(!should_back_off(0, None));
        assert!(!should_back_off(2, Some(Duration::from_secs(1))));
    }

    #[test]
    fn should_back_off_pure_at_threshold_within_window() {
        assert!(should_back_off(3, Some(Duration::from_secs(1))));
        assert!(should_back_off(
            5,
            Some(BACKOFF_WINDOW - Duration::from_secs(1))
        ));
    }

    #[test]
    fn should_back_off_pure_at_threshold_after_window_elapsed() {
        assert!(!should_back_off(3, Some(BACKOFF_WINDOW)));
        assert!(!should_back_off(
            3,
            Some(BACKOFF_WINDOW + Duration::from_secs(60))
        ));
    }

    #[test]
    fn should_back_off_pure_at_threshold_without_last_failure_is_false() {
        assert!(!should_back_off(3, None));
    }

    #[test]
    fn evaluate_backoff_proceeds_until_threshold_then_skips_and_logs_once() {
        let db = fresh_db();
        let committer = MemoryCommitter::new(Arc::clone(&db));
        for _ in 0..2 {
            committer.record_failure("agent-y");
            assert!(matches!(
                committer.evaluate_backoff("agent-y"),
                BackoffDecision::Proceed
            ));
        }
        committer.record_failure("agent-y");
        match committer.evaluate_backoff("agent-y") {
            BackoffDecision::Skip { should_log } => assert!(should_log),
            BackoffDecision::Proceed => panic!("expected backoff after 3 consecutive failures"),
        }
        // A second consult within the same backoff episode must not
        // re-log — only the very first skip does.
        match committer.evaluate_backoff("agent-y") {
            BackoffDecision::Skip { should_log } => assert!(!should_log),
            BackoffDecision::Proceed => panic!("expected backoff to persist"),
        }
    }

    #[test]
    fn record_success_resets_backoff_state() {
        let db = fresh_db();
        let committer = MemoryCommitter::new(Arc::clone(&db));
        committer.record_failure("agent-z");
        committer.record_failure("agent-z");
        committer.record_failure("agent-z");
        assert!(matches!(
            committer.evaluate_backoff("agent-z"),
            BackoffDecision::Skip { .. }
        ));
        committer.record_success("agent-z");
        assert!(matches!(
            committer.evaluate_backoff("agent-z"),
            BackoffDecision::Proceed
        ));
    }

    #[test]
    fn evaluate_backoff_is_independent_per_agent() {
        let db = fresh_db();
        let committer = MemoryCommitter::new(Arc::clone(&db));
        committer.record_failure("agent-a");
        committer.record_failure("agent-a");
        committer.record_failure("agent-a");
        assert!(matches!(
            committer.evaluate_backoff("agent-a"),
            BackoffDecision::Skip { .. }
        ));
        assert!(matches!(
            committer.evaluate_backoff("agent-b"),
            BackoffDecision::Proceed
        ));
    }
}
