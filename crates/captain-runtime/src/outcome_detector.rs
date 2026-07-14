//! Outcome classifier + background detector (v3.12c).
//!
//! Consumes the `LearningSignal` stream from `learning_bus` and
//! classifies each signal into an `Outcome`. The output stream is
//! picked up by the `ReflectionJob` (v3.12d) which decides whether
//! to actually call a reflection model and produce memory candidates.
//!
//! Design choices:
//! - **Deterministic**. No LLM in this module. Classification is pure
//!   regex + a rolling event buffer. Speed matters: the detector sits
//!   on every tool call and user message.
//! - **Rolling buffer**. A 50-event window of recent tool outcomes
//!   enables `RetrySuccess` detection (same tool failed N times then
//!   succeeded on the N+1th attempt).
//! - **Heuristics first**. Regex patterns for French + English catch
//!   the common corrections / satisfactions / explicit remembers
//!   without a model round-trip.
//! - **Unknown ⇒ skip**. Plain tool successes and first-time tool
//!   failures update the rolling buffer but do not trigger reflection.
//!   Repeated failures, retry successes, conversation summaries, and
//!   explicit user signals carry the durable learning surface.

use regex_lite::Regex;
use serde::{Deserialize, Serialize};
use std::collections::VecDeque;
use std::sync::LazyLock;
use tokio::sync::mpsc;
use tracing::{debug, trace};

use crate::learning_bus::LearningSignal;

/// Max number of recent tool events kept in the rolling buffer. Used
/// to detect `RetrySuccess` (same tool failing repeatedly then finally
/// succeeding).
pub const EVENT_BUFFER_CAP: usize = 50;

/// Minimum number of prior consecutive failures on the same tool to
/// consider a subsequent success a `RetrySuccess`.
pub const RETRY_SUCCESS_MIN_FAILURES: u32 = 2;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Outcome {
    /// Tool completed normally, nothing notable.
    Success,
    /// Tool failed after retries exhausted.
    Failure,
    /// Agent loop was cancelled or interrupted.
    Cancelled,
    /// User corrected the assistant (starts with "non", "pas", "c'est pas…").
    UserCorrected,
    /// Same tool failed N times then succeeded — a recoverable pattern
    /// worth remembering.
    RetrySuccess,
    /// A human approval/denial event was recorded (v3.8 approval flow).
    ApprovalDecision,
    /// Conversation went idle after a terminal assistant reply. The
    /// idle timer lives in v3.12d ReflectionJob; here we just forward
    /// the signal type if we ever receive it.
    ConversationIdle,
    /// User said "retiens/souviens-toi/note que" — bypasses reflection
    /// and should be committed directly.
    ExplicitRemember,
    /// Commit-B — every user→agent turn flows through reflection,
    /// regardless of regex match. The Haiku reviewer decides itself
    /// whether anything is worth remembering. Optional regex hints
    /// (`explicit_remember` etc.) act as confidence boosts inside the
    /// reflection prompt, never as gates.
    ConversationTurn,
    /// Nothing actionable — skip reflection entirely.
    Unknown,
}

/// Sub-classification of a free-form user message. Public so emitters
/// (ws.rs, channel_bridge) can choose the right `LearningSignal`
/// variant before pushing to the bus.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UserMessageKind {
    Correction,
    Satisfaction,
    ExplicitRemember,
}

// ---------------------------------------------------------------------------
// Lazy-compiled regex patterns
// ---------------------------------------------------------------------------

static RE_CORRECTION: LazyLock<Regex> = LazyLock::new(|| {
    // FR: non / ce n'est pas / c'est pas / je t'avais dit / pas / mais
    // EN: no / not / wrong / that's not
    Regex::new(
        r"(?i)^\s*(non\b|ce\s*n'?est\s*pas\b|c'?est\s*pas\b|je\s*t'?avais\s*dit\b|pas\s|mais\s|no\b|not\b|wrong\b|that'?s\s*not\b)"
    ).expect("static regex")
});

static RE_SATISFACTION: LazyLock<Regex> = LazyLock::new(|| {
    // FR: parfait / c'est bon / merci / top / ok / super / genial
    // EN: perfect / thanks / thank you / great / awesome / nice
    // Anchored at start to avoid matching "pas parfait" etc.
    Regex::new(
        r"(?i)^\s*(parfait\b|c'?est\s*bon\b|merci\b|top\b|ok\b|super\b|g[eé]nial\b|perfect\b|thanks?\b|thank\s+you\b|great\b|awesome\b|nice\b)"
    ).expect("static regex")
});

static RE_EXPLICIT_REMEMBER: LazyLock<Regex> = LazyLock::new(|| {
    // Commit-B — broadened FR list. Still acts as a HINT for the
    // reflection prompt (boosts confidence) — does NOT gate the call.
    // FR: retiens / souviens-toi / note que / retenez / mémorise /
    //     rappelle-toi / garde en tête / n'oublie pas / prends note /
    //     enregistre / sache que
    // EN: remember that / keep in mind / make a note / don't forget
    Regex::new(
        r"(?i)(retiens\b|souviens[-\s]toi\b|note\s*(que)?\b|retenez\b|m[eé]morise\b|rappelle[-\s]toi\b|garde\s+(?:en|à)\s+t[eê]te\b|n'?oublie\s+pas\b|prends\s+note\b|enregistre\b|sache\s+que\b|remember\s+that\b|keep\s+in\s+mind\b|make\s+a\s+note\b|don'?t\s+forget\b)"
    ).expect("static regex")
});

/// Pure function: classify a user message by running the three heuristic
/// regexes. `ExplicitRemember` takes precedence (strongest signal),
/// then `Correction`, then `Satisfaction`.
pub fn classify_user_message(msg: &str) -> Option<UserMessageKind> {
    if msg.is_empty() {
        return None;
    }
    if RE_EXPLICIT_REMEMBER.is_match(msg) {
        return Some(UserMessageKind::ExplicitRemember);
    }
    if RE_CORRECTION.is_match(msg) {
        return Some(UserMessageKind::Correction);
    }
    if RE_SATISFACTION.is_match(msg) {
        return Some(UserMessageKind::Satisfaction);
    }
    None
}

// ---------------------------------------------------------------------------
// Classifier with rolling event buffer
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
struct ToolEvent {
    agent_id: String,
    tool: String,
    is_error: bool,
}

pub struct Classifier {
    events: VecDeque<ToolEvent>,
    capacity: usize,
}

impl Classifier {
    pub fn new() -> Self {
        Self::with_capacity(EVENT_BUFFER_CAP)
    }

    pub fn with_capacity(capacity: usize) -> Self {
        Self {
            events: VecDeque::with_capacity(capacity),
            capacity,
        }
    }

    /// Core classification entry point. Mutates `self.events` so the
    /// rolling buffer stays current for future calls. Pure functions of
    /// the buffer state + the incoming signal.
    pub fn classify(&mut self, signal: &LearningSignal) -> Outcome {
        match signal {
            LearningSignal::ToolSuccess { agent_id, tool, .. } => {
                let prior = self.count_consecutive_failures(agent_id, tool);
                self.push_event(ToolEvent {
                    agent_id: agent_id.clone(),
                    tool: tool.clone(),
                    is_error: false,
                });
                if prior >= RETRY_SUCCESS_MIN_FAILURES {
                    Outcome::RetrySuccess
                } else {
                    Outcome::Unknown
                }
            }
            LearningSignal::ToolFailure { agent_id, tool, .. } => {
                let prior = self.count_consecutive_failures(agent_id, tool);
                self.push_event(ToolEvent {
                    agent_id: agent_id.clone(),
                    tool: tool.clone(),
                    is_error: true,
                });
                if prior + 1 >= RETRY_SUCCESS_MIN_FAILURES {
                    Outcome::Failure
                } else {
                    Outcome::Unknown
                }
            }
            LearningSignal::RetrySuccess { .. } => Outcome::RetrySuccess,
            LearningSignal::ApprovalDecision { .. } => Outcome::ApprovalDecision,
            LearningSignal::UserCorrection { .. } => Outcome::UserCorrected,
            LearningSignal::UserSatisfaction { .. } => Outcome::Success,
            LearningSignal::ExplicitRemember { .. } => Outcome::ExplicitRemember,
            LearningSignal::WorkflowRunComplete { outcome, .. } => {
                // Distinguish cancellation from success/failure narratives
                let lower = outcome.to_ascii_lowercase();
                if lower.contains("cancel") || lower.contains("interrupt") {
                    Outcome::Cancelled
                } else if lower.starts_with("failure") || lower.contains("error") {
                    Outcome::Failure
                } else {
                    Outcome::Success
                }
            }
            LearningSignal::HandRunComplete { outcome, .. } => {
                let lower = outcome.to_ascii_lowercase();
                if lower.starts_with("failure") || lower.contains("error") {
                    Outcome::Failure
                } else {
                    Outcome::Success
                }
            }
            LearningSignal::SessionLearningReview { .. } => Outcome::ConversationIdle,
            LearningSignal::ConversationTurn { .. } => Outcome::ConversationTurn,
        }
    }

    fn push_event(&mut self, ev: ToolEvent) {
        if self.events.len() >= self.capacity {
            self.events.pop_front();
        }
        self.events.push_back(ev);
    }

    /// Count consecutive trailing failures on `(agent_id, tool)` in the
    /// buffer, scanning from newest to oldest and stopping at the first
    /// non-matching event. A non-failure success on the same tool
    /// resets the chain (returns 0).
    fn count_consecutive_failures(&self, agent_id: &str, tool: &str) -> u32 {
        let mut n: u32 = 0;
        for ev in self.events.iter().rev() {
            if ev.agent_id == agent_id && ev.tool == tool {
                if ev.is_error {
                    n += 1;
                } else {
                    break;
                }
            }
        }
        n
    }
}

impl Default for Classifier {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// Background detector worker
// ---------------------------------------------------------------------------

/// One classified event — the pair delivered to downstream consumers.
#[derive(Debug, Clone)]
pub struct ClassifiedSignal {
    pub outcome: Outcome,
    pub signal: LearningSignal,
}

pub struct OutcomeDetector;

impl OutcomeDetector {
    /// Spawn the detector loop. Reads from `signal_rx` (the bus
    /// receiver from v3.12b), classifies each signal, and forwards the
    /// classified pair onto the returned output receiver.
    ///
    /// The detector drops `Outcome::Unknown` silently; the output
    /// stream only carries actionable classifications.
    pub fn spawn(
        mut signal_rx: mpsc::Receiver<LearningSignal>,
        output_capacity: usize,
    ) -> (
        tokio::task::JoinHandle<()>,
        mpsc::Receiver<ClassifiedSignal>,
    ) {
        let (tx, rx) = mpsc::channel::<ClassifiedSignal>(output_capacity);
        let handle = tokio::spawn(async move {
            let mut classifier = Classifier::new();
            while let Some(signal) = signal_rx.recv().await {
                let outcome = classifier.classify(&signal);
                if matches!(outcome, Outcome::Unknown) {
                    trace!(kind = signal.kind(), "outcome_detector drop Unknown");
                    continue;
                }
                let classified = ClassifiedSignal { outcome, signal };
                if let Err(e) = tx.try_send(classified) {
                    debug!(error = %e, "outcome_detector drop (downstream full)");
                }
            }
        });
        (handle, rx)
    }
}

#[cfg(test)]
#[path = "outcome_detector_tests.rs"]
mod tests;
