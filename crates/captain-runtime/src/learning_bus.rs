//! LearningSignal bus (v3.12b).
//!
//! A lock-free mpsc channel that carries `LearningSignal` events from
//! three emission zones:
//! - **A. Real-time** (tool_runner, agent_loop): tool outcomes, retries,
//!   approval decisions.
//! - **B. Dialogue** (ws.rs, channel_bridge): user corrections,
//!   satisfaction, explicit remembers.
//! - **C. Workflow** (kernel.rs): end of agent loop, end of hand run,
//!   end of browser session (v3.13+).
//! - **D. Inactivity** (session_summarizer): post-idle review of a whole
//!   session through OBSERVE → THINK → PLAN → BUILD → EXECUTE → VERIFY → LEARN.
//!
//! The bus is intentionally dumb — it does not classify or reflect. The
//! `OutcomeDetector` (v3.12c) and `ReflectionJob` (v3.12d) pick up
//! signals and decide what to do.
//!
//! Guardrails:
//! - **Anti-loop**: any signal whose `source` begins with `learning.`
//!   is dropped; learning writes must never spawn a new learning job.
//! - **Rate limit**: only explicitly throttled real-time signals are
//!   limited per agent, kind, and action. Tool outcomes bypass this so
//!   the retry detector sees the full failure/success sequence; the
//!   detector decides which outcomes are worth reflecting on.
//! - **Non-blocking**: `emit()` uses `try_send`; if the channel is full
//!   the signal is dropped and `EmitResult::SkippedFull` is returned.
//!   We prefer lost learning over a backpressured hot path.

use dashmap::DashMap;
use serde::{Deserialize, Serialize};
use std::sync::{Arc, OnceLock};
use std::time::{Duration, Instant};
use tokio::sync::mpsc;
use tracing::debug;

/// Default channel capacity. Must comfortably absorb a busy minute
/// without dropping signals even if the consumer lags briefly.
pub const DEFAULT_CAPACITY: usize = 256;

/// One noisy throttled real-time signal per agent/action every 10 seconds.
/// Tool outcomes are not throttled here: they are cheap to classify and
/// must preserve ordering for retry-success detection.
pub const RATE_LIMIT_WINDOW: Duration = Duration::from_secs(10);

/// Cap on the size of free-form strings carried in a signal. Learning
/// reflection doesn't need the full message body — 2KB is plenty for
/// a heuristic detector and keeps the bus cheap.
pub const MAX_TEXT_LEN: usize = 2048;

/// All signal variants. Serde tags with `type` so logs and KG rows keep
/// the discriminator when flattened.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum LearningSignal {
    /// A. Real-time — tool executed successfully.
    ToolSuccess {
        agent_id: String,
        tool: String,
        duration_ms: u64,
        source: String,
    },
    /// A. Real-time — tool execution failed.
    ToolFailure {
        agent_id: String,
        tool: String,
        error: String,
        source: String,
    },
    /// A. Real-time — previously-failing tool succeeded on retry.
    RetrySuccess {
        agent_id: String,
        tool: String,
        prior_errors: u32,
        source: String,
    },
    /// A. Real-time — user approved or denied an action (v3.8).
    ApprovalDecision {
        agent_id: String,
        approved: bool,
        action: String,
        source: String,
    },
    /// B. Dialogue — user is correcting the assistant.
    UserCorrection {
        agent_id: String,
        user_msg: String,
        source: String,
    },
    /// B. Dialogue — user is satisfied (parfait, merci, top, ok…).
    UserSatisfaction {
        agent_id: String,
        user_msg: String,
        source: String,
    },
    /// B. Dialogue — user explicitly asked to remember something.
    ExplicitRemember {
        agent_id: String,
        user_msg: String,
        source: String,
    },
    /// C. Workflow — agent loop run terminated.
    WorkflowRunComplete {
        agent_id: String,
        outcome: String,
        tool_calls: u32,
        source: String,
    },
    /// C. Workflow — hand run terminated.
    HandRunComplete {
        agent_id: String,
        hand: String,
        outcome: String,
        source: String,
    },
    /// D. Inactivity — session-level learning review emitted after a
    /// persisted session is idle long enough to be checkpointed.
    SessionLearningReview {
        agent_id: String,
        session_id: String,
        review: String,
        source: String,
    },
    /// Commit-B — Universal turn signal. Emitted at the END of every
    /// agent reply (regardless of regex match), so the Haiku post-process
    /// reflection can review the whole exchange and decide on its own
    /// what (if anything) is worth remembering. Carries the origin
    /// `channel` so the resulting `🧠 mémorisé` notice can be routed
    /// back to the conversation it came from.
    ConversationTurn {
        agent_id: String,
        user_msg: String,
        agent_response: String,
        /// Origin canal: telegram, cli, web, discord, slack, … or
        /// `None` when called from a path that doesn't expose one.
        channel: Option<String>,
        /// Optional hint surfaced by the regex-based classifier
        /// (`explicit_remember` / `correction` / `satisfaction`).
        /// Acts as a confidence BOOST in the reflection prompt; never
        /// gates the call.
        regex_hint: Option<String>,
        source: String,
    },
}

impl LearningSignal {
    pub fn agent_id(&self) -> &str {
        match self {
            Self::ToolSuccess { agent_id, .. }
            | Self::ToolFailure { agent_id, .. }
            | Self::RetrySuccess { agent_id, .. }
            | Self::ApprovalDecision { agent_id, .. }
            | Self::UserCorrection { agent_id, .. }
            | Self::UserSatisfaction { agent_id, .. }
            | Self::ExplicitRemember { agent_id, .. }
            | Self::WorkflowRunComplete { agent_id, .. }
            | Self::HandRunComplete { agent_id, .. }
            | Self::SessionLearningReview { agent_id, .. }
            | Self::ConversationTurn { agent_id, .. } => agent_id,
        }
    }

    pub fn source(&self) -> &str {
        match self {
            Self::ToolSuccess { source, .. }
            | Self::ToolFailure { source, .. }
            | Self::RetrySuccess { source, .. }
            | Self::ApprovalDecision { source, .. }
            | Self::UserCorrection { source, .. }
            | Self::UserSatisfaction { source, .. }
            | Self::ExplicitRemember { source, .. }
            | Self::WorkflowRunComplete { source, .. }
            | Self::HandRunComplete { source, .. }
            | Self::SessionLearningReview { source, .. }
            | Self::ConversationTurn { source, .. } => source,
        }
    }

    /// Origin channel of the signal, when available.
    ///
    /// Only `ConversationTurn` currently carries a channel — the other
    /// variants either originate from background pipelines (no user-facing
    /// canal) or were defined before the channel field was introduced.
    /// Returning `None` is the explicit "I don't know where this came from"
    /// answer so downstream code can fall back to a broadcast.
    pub fn channel(&self) -> Option<&str> {
        match self {
            Self::ConversationTurn { channel, .. } => channel.as_deref(),
            _ => None,
        }
    }

    /// Stable short name used for metrics / logs.
    pub fn kind(&self) -> &'static str {
        match self {
            Self::ToolSuccess { .. } => "tool_success",
            Self::ToolFailure { .. } => "tool_failure",
            Self::RetrySuccess { .. } => "retry_success",
            Self::ApprovalDecision { .. } => "approval_decision",
            Self::UserCorrection { .. } => "user_correction",
            Self::UserSatisfaction { .. } => "user_satisfaction",
            Self::ExplicitRemember { .. } => "explicit_remember",
            Self::WorkflowRunComplete { .. } => "workflow_run_complete",
            Self::HandRunComplete { .. } => "hand_run_complete",
            Self::SessionLearningReview { .. } => "session_learning_review",
            Self::ConversationTurn { .. } => "conversation_turn",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EmitResult {
    /// Signal was queued on the channel.
    Emitted,
    /// Source begins with `learning.` — dropped to prevent feedback loop.
    SkippedLoop,
    /// Agent exceeded the throttle for a rate-limited signal.
    SkippedRateLimit,
    /// Channel buffer is full; consumer is lagging.
    SkippedFull,
    /// No bus installed (e.g. runtime-only test harness).
    NotInstalled,
}

pub struct LearningBus {
    tx: mpsc::Sender<LearningSignal>,
    rate_limiter: DashMap<String, Instant>,
}

impl LearningBus {
    /// Create a new bus with the given channel capacity. Returns the
    /// bus plus the receive-half that a consumer (v3.12c detector) owns.
    pub fn new(capacity: usize) -> (Arc<Self>, mpsc::Receiver<LearningSignal>) {
        let (tx, rx) = mpsc::channel(capacity);
        let bus = Arc::new(Self {
            tx,
            rate_limiter: DashMap::new(),
        });
        (bus, rx)
    }

    /// Emit one signal. Never panics, never blocks.
    pub fn emit(&self, mut signal: LearningSignal) -> EmitResult {
        // Anti-loop: never re-amplify a learning-generated event.
        if signal.source().starts_with("learning.") {
            debug!(kind = signal.kind(), "learning_bus skip (loop guard)");
            return EmitResult::SkippedLoop;
        }

        // Rate limit only explicitly throttled signals. Tool outcomes
        // must keep their order so the detector can see failure -> retry
        // -> success without the bus hiding the middle of the sequence.
        if let Some(rate_key) = rate_limit_key(&signal) {
            let now = Instant::now();
            let rate_limited = self
                .rate_limiter
                .get(&rate_key)
                .map(|last| now.duration_since(*last) < RATE_LIMIT_WINDOW)
                .unwrap_or(false);
            if rate_limited {
                debug!(
                    agent = %signal.agent_id(),
                    kind = signal.kind(),
                    "learning_bus skip (rate limit)"
                );
                return EmitResult::SkippedRateLimit;
            }
            self.rate_limiter.insert(rate_key, now);
        }

        // Truncate free-form fields to keep memory bounded.
        truncate_signal_strings(&mut signal);

        // Send non-blocking. Full channel = lost signal (acceptable).
        match self.tx.try_send(signal) {
            Ok(()) => EmitResult::Emitted,
            Err(mpsc::error::TrySendError::Full(s)) => {
                debug!(kind = s.kind(), "learning_bus skip (channel full)");
                EmitResult::SkippedFull
            }
            Err(mpsc::error::TrySendError::Closed(s)) => {
                debug!(kind = s.kind(), "learning_bus skip (receiver dropped)");
                EmitResult::SkippedFull
            }
        }
    }

    /// Drop the rate limiter entry for an agent — useful when an agent
    /// is killed so stale entries don't linger.
    pub fn forget_agent(&self, agent_id: &str) {
        let prefix = format!("{agent_id}:");
        self.rate_limiter
            .retain(|key, _| key != agent_id && !key.starts_with(&prefix));
    }
}

fn rate_limit_key(signal: &LearningSignal) -> Option<String> {
    match signal {
        LearningSignal::ApprovalDecision { action, .. } => Some(format!(
            "{}:{}:{}",
            signal.agent_id(),
            signal.kind(),
            action
        )),
        LearningSignal::ToolSuccess { .. }
        | LearningSignal::ToolFailure { .. }
        | LearningSignal::RetrySuccess { .. }
        | LearningSignal::UserCorrection { .. }
        | LearningSignal::UserSatisfaction { .. }
        | LearningSignal::ExplicitRemember { .. }
        | LearningSignal::WorkflowRunComplete { .. }
        | LearningSignal::HandRunComplete { .. }
        | LearningSignal::SessionLearningReview { .. }
        | LearningSignal::ConversationTurn { .. } => None,
    }
}

fn truncate_signal_strings(signal: &mut LearningSignal) {
    match signal {
        LearningSignal::ToolFailure { error, .. } => truncate_in_place(error, MAX_TEXT_LEN),
        LearningSignal::UserCorrection { user_msg, .. }
        | LearningSignal::UserSatisfaction { user_msg, .. }
        | LearningSignal::ExplicitRemember { user_msg, .. } => {
            truncate_in_place(user_msg, MAX_TEXT_LEN)
        }
        LearningSignal::ApprovalDecision { action, .. } => truncate_in_place(action, MAX_TEXT_LEN),
        LearningSignal::WorkflowRunComplete { outcome, .. }
        | LearningSignal::HandRunComplete { outcome, .. } => {
            truncate_in_place(outcome, MAX_TEXT_LEN)
        }
        LearningSignal::SessionLearningReview { review, .. } => {
            truncate_in_place(review, MAX_TEXT_LEN)
        }
        LearningSignal::ConversationTurn {
            user_msg,
            agent_response,
            ..
        } => {
            truncate_in_place(user_msg, MAX_TEXT_LEN);
            truncate_in_place(agent_response, MAX_TEXT_LEN);
        }
        _ => {}
    }
}

fn truncate_in_place(s: &mut String, max: usize) {
    if s.len() > max {
        // Keep UTF-8 boundary safe.
        let cut = (0..=max)
            .rev()
            .find(|i| s.is_char_boundary(*i))
            .unwrap_or(0);
        s.truncate(cut);
    }
}

// ---------------------------------------------------------------------------
// Global installation — OnceLock set at kernel boot
// ---------------------------------------------------------------------------

static GLOBAL: OnceLock<Arc<LearningBus>> = OnceLock::new();

/// Install the process-wide learning bus. Returns the receiver that the
/// consumer (v3.12c detector) must own. Subsequent calls return `None`
/// because `OnceLock::set` only succeeds once — the first installer owns
/// the receiver, later callers should look up the bus via `global()`.
pub fn install(capacity: usize) -> Option<mpsc::Receiver<LearningSignal>> {
    let (bus, rx) = LearningBus::new(capacity);
    if GLOBAL.set(bus).is_ok() {
        Some(rx)
    } else {
        None
    }
}

/// Return the installed global bus, if any.
pub fn global() -> Option<Arc<LearningBus>> {
    GLOBAL.get().cloned()
}

/// Convenience: emit through the global bus if installed, otherwise
/// return `NotInstalled`.
pub fn emit(signal: LearningSignal) -> EmitResult {
    match global() {
        Some(bus) => bus.emit(signal),
        None => EmitResult::NotInstalled,
    }
}

#[cfg(test)]
#[path = "learning_bus_tests.rs"]
mod tests;
