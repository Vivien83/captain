//! ReflectionJob (v3.12d).
//!
//! Given a `ClassifiedSignal` from the `OutcomeDetector`, this module
//! calls a cheap LLM with a compact Given/When/Then/WhatToRemember
//! prompt, parses the JSON response into `MemoryCandidate`s, and
//! returns them to the caller. The candidates travel downstream to
//! `MemoryPolicy` (v3.12e) and `MemoryCommitter` (v3.12f).
//!
//! Guardrails:
//! - **Timeout** every call. A slow reflection must never block the
//!   consumer loop — drop the candidates rather than stall.
//! - **Fallback chain**. Primary model → fallback 1 → fallback 2. Each
//!   is attempted with the same timeout budget.
//! - **Confidence floor**. Candidates under `min_confidence` are
//!   dropped before they leave the reflector.
//! - **Strict parsing**. Malformed or partially-valid JSON never
//!   panics; bad entries are skipped.
//! - **Injection-resistant**. The reflector is told to return ONLY
//!   JSON; the sanitizer in v3.12e rejects anything else.

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::time::Duration;
use tokio::time::timeout;
use tracing::{debug, info, warn};

use crate::learning_bus::LearningSignal;
use crate::outcome_detector::{ClassifiedSignal, Outcome};

/// A memory write proposed by the reflection model. One classified
/// signal may yield 0..N candidates.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct MemoryCandidate {
    pub wing: String,
    pub room: String,
    pub subject: String,
    pub predicate: String,
    pub object: String,
    pub confidence: f32,
    /// Commit-B — Haiku-assigned category: `info` (durable fact),
    /// `skill` (validated capability), `error_success` (lesson from a
    /// failure or reproduce a success), `solution` (recipe for a
    /// specific problem), `other`. `Option` for backward-compat with
    /// pre-Commit-B parsed entries.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub category: Option<String>,
}

/// A batch forwarded downstream (MemoryPolicy, MemoryCommitter)
/// carrying the classification context alongside the candidates so
/// the committer can route correctly without re-running detection.
#[derive(Debug, Clone)]
pub struct ReflectionBatch {
    pub outcome: Outcome,
    pub agent_id: String,
    pub candidates: Vec<MemoryCandidate>,
    /// Origin channel of the signal that produced this batch (telegram,
    /// cli, web, …). Propagated end-to-end so the committed `🧠` notice
    /// can be routed back to the conversation it came from. `None` when
    /// the signal had no channel context (background pipelines).
    pub channel: Option<String>,
}

/// Minimal trait around the reflection LLM call. Makes the whole module
/// mockable in tests without spinning up a real driver.
#[async_trait]
pub trait ReflectionCompleter: Send + Sync {
    /// Run one completion. `model` is the model identifier. `system` is
    /// the system prompt, `user` is the user turn. Returns the raw text
    /// from the model or an error.
    async fn complete(&self, model: &str, system: &str, user: &str) -> Result<String, String>;
}

/// No-op completer. Used as the default until a real driver adapter is
/// plugged in. Always returns an empty candidate list (by returning an
/// empty JSON array).
pub struct NoopCompleter;

#[async_trait]
impl ReflectionCompleter for NoopCompleter {
    async fn complete(&self, _model: &str, _system: &str, _user: &str) -> Result<String, String> {
        Ok("[]".to_string())
    }
}

/// Adapter: wraps any `LlmDriver` and turns its `complete()` method
/// into the `ReflectionCompleter` trait. Keeps reflection prompts
/// small and deterministic (low temperature, capped tokens).
pub struct LlmDriverCompleter {
    pub driver: std::sync::Arc<dyn crate::llm_driver::LlmDriver>,
    pub max_tokens: u32,
    pub temperature: f32,
}

impl LlmDriverCompleter {
    pub fn new(driver: std::sync::Arc<dyn crate::llm_driver::LlmDriver>) -> Self {
        Self {
            driver,
            max_tokens: 1024,
            temperature: 0.2,
        }
    }
}

#[async_trait]
impl ReflectionCompleter for LlmDriverCompleter {
    async fn complete(&self, model: &str, system: &str, user: &str) -> Result<String, String> {
        use crate::llm_driver::CompletionRequest;
        use captain_types::message::{Message, MessageContent, Role};

        let request = CompletionRequest {
            model: model.to_string(),
            messages: vec![Message {
                role: Role::User,
                content: MessageContent::Text(user.to_string()),
            }],
            tools: Vec::new(),
            max_tokens: self.max_tokens,
            temperature: self.temperature,
            system: Some(system.to_string()),
            thinking: None,
            tool_choice: None,
            cache_hints: crate::llm_driver::CacheHints::default(),
        };
        let response = self
            .driver
            .complete(request)
            .await
            .map_err(|e| format!("llm completion: {e}"))?;
        Ok(response.text())
    }
}

/// Minimal per-call config. Derived from `LearningConfig` at boot.
#[derive(Debug, Clone)]
pub struct ReflectionConfig {
    pub primary_model: String,
    pub fallback_models: Vec<String>,
    pub timeout_secs: u64,
    pub min_confidence: f32,
}

impl From<&captain_types::config::LearningConfig> for ReflectionConfig {
    fn from(lc: &captain_types::config::LearningConfig) -> Self {
        let aggressiveness = lc.effective_autonomy_aggressiveness();
        Self {
            primary_model: lc.reflection_model.clone(),
            fallback_models: lc.fallback_models.clone(),
            timeout_secs: lc.reflection_timeout_secs,
            min_confidence: scale_confidence_floor(lc.min_confidence, aggressiveness),
        }
    }
}

pub fn scale_confidence_floor(base: f32, aggressiveness: f32) -> f32 {
    if base <= 0.0 {
        return 0.0;
    }
    (base / aggressiveness.sqrt()).clamp(0.0, 0.99)
}

// ---------------------------------------------------------------------------
// Prompt builder
// ---------------------------------------------------------------------------

const SYSTEM_PROMPT: &str = "You are a memory reflector for an AI assistant. \
Your job is to extract 0..3 durable facts worth remembering from a single \
interaction outcome. Each fact is a knowledge-graph triple (subject, \
predicate, object) that will be stored in a memory palace.\n\n\
RULES:\n\
1. Output ONLY a JSON array. No prose, no markdown, no code fences.\n\
2. Each element: {\"wing\":\"...\",\"room\":\"...\",\"subject\":\"...\",\"predicate\":\"...\",\"object\":\"...\",\"confidence\":0..1,\"category\":\"info|skill|error_success|solution|other\"}.\n\
3. Prefer the `learnings` wing for generic rules, `project:<slug>` wings for project-scoped facts.\n\
4. Use rooms: general, failures, workarounds, user_preferences, decisions, retrospective.\n\
   Route preferences, response style, preferred validation channel, tone, verbosity, and product-behaviour choices to `learnings/user_preferences` with category `info`.\n\
5. Confidence reflects how universally true the fact is. Use 0.5 for likely, 0.7 for clear, 0.9 for certain.\n\
6. Never include secrets, API keys, passwords, tokens, or PII.\n\
7. For ExplicitRemember / regex_hint == 'explicit_remember': ALWAYS emit at least one candidate with the literal fact the user asked you to remember. Confidence 0.9+.\n\
8. Category meanings:\n\
   - info: durable fact about user, context, environment, preference\n\
   - skill: a workflow / capability that worked and could be reused\n\
   - error_success: a lesson learned from a failure or a success worth reproducing\n\
   - solution: a precise recipe for a specific problem (command, snippet, workaround)\n\
   - other: anything else worth keeping but doesn't fit above\n\
9. For ConversationTurn (universal trigger): scan the user message AND the agent reply, decide if anything is durable. Most short turns produce []. Only emit when the exchange teaches something the assistant should still know in 30 days.\n\
10. For WorkflowRunComplete with a `tool_trace`, preserve reusable capability routing only when the trace teaches which tool/family/skill/MCP/Hand solved a general class of task. Prefer category `skill` or `solution`; skip one-off file paths, private aliases, and secrets.\n\
11. Procedural learnings (`skill` or `solution`) are routed to skill proposals, not raw memory. Emit them only when the workflow is reusable and self-contained.\n\
12. For SessionLearningReview: treat the stages OBSERVE, THINK, PLAN, BUILD, EXECUTE, VERIFY, LEARN as a post-idle retrospective. Compare against already-known facts implied by the review and emit only non-duplicate durable preferences, workflow recipes, or skill improvements.\n\
13. If nothing is worth remembering, return [].";

/// Build the (system, user) prompt pair for a classified signal.
pub fn build_prompt(classified: &ClassifiedSignal) -> (String, String) {
    let given = given_block(&classified.signal);
    let when = when_block(classified.outcome);
    let then = then_block(classified.outcome);
    let user = format!(
        "GIVEN:\n{given}\n\nWHEN:\n{when}\n\nTHEN WHAT TO REMEMBER:\n{then}\n\n\
         Return a JSON array of 0..3 memory candidates. JSON only."
    );
    (SYSTEM_PROMPT.to_string(), user)
}

fn given_block(signal: &LearningSignal) -> String {
    match signal {
        LearningSignal::ToolSuccess { agent_id, tool, duration_ms, .. } => format!(
            "Agent {agent_id} ran tool `{tool}` successfully in {duration_ms}ms."
        ),
        LearningSignal::ToolFailure { agent_id, tool, error, .. } => format!(
            "Agent {agent_id} ran tool `{tool}` — error: {error}"
        ),
        LearningSignal::RetrySuccess { agent_id, tool, prior_errors, .. } => format!(
            "Agent {agent_id} retried tool `{tool}` after {prior_errors} failures and it finally worked."
        ),
        LearningSignal::ApprovalDecision { agent_id, approved, action, .. } => format!(
            "Agent {agent_id} requested approval for: {action}. User decision: {}.",
            if *approved { "approved" } else { "denied" }
        ),
        LearningSignal::UserCorrection { agent_id, user_msg, .. } => format!(
            "Agent {agent_id} was corrected by the user: \"{user_msg}\""
        ),
        LearningSignal::UserSatisfaction { agent_id, user_msg, .. } => format!(
            "Agent {agent_id} received a satisfaction signal: \"{user_msg}\""
        ),
        LearningSignal::ExplicitRemember { agent_id, user_msg, .. } => format!(
            "Agent {agent_id} was explicitly told to remember: \"{user_msg}\""
        ),
        LearningSignal::WorkflowRunComplete {
            agent_id,
            outcome,
            tool_calls,
            ..
        } => format!("Agent {agent_id} finished a run ({tool_calls} tools).\nOUTCOME: {outcome}"),
        LearningSignal::HandRunComplete { agent_id, hand, outcome, .. } => format!(
            "Agent {agent_id} finished hand `{hand}` with outcome: {outcome}."
        ),
        LearningSignal::SessionLearningReview {
            agent_id,
            session_id,
            review,
            ..
        } => format!(
            "Agent {agent_id} reached inactivity on session `{session_id}` and produced this staged learning review:\n{review}"
        ),
        LearningSignal::ConversationTurn {
            agent_id,
            user_msg,
            agent_response,
            channel,
            regex_hint,
            ..
        } => {
            let chan = channel
                .as_deref()
                .map(|c| format!(" via {c}"))
                .unwrap_or_default();
            let hint = regex_hint
                .as_deref()
                .map(|h| format!(" [regex_hint: {h}]"))
                .unwrap_or_default();
            format!(
                "Agent {agent_id} just completed a turn{chan}{hint}.\n\
                USER: \"{user_msg}\"\n\
                AGENT: \"{agent_response}\""
            )
        }
    }
}

fn when_block(outcome: Outcome) -> &'static str {
    match outcome {
        Outcome::Success => "The interaction completed successfully.",
        Outcome::Failure => "The interaction failed.",
        Outcome::Cancelled => "The interaction was cancelled.",
        Outcome::UserCorrected => "The user corrected the assistant.",
        Outcome::RetrySuccess => "A retry eventually succeeded — likely a workaround worth capturing.",
        Outcome::ApprovalDecision => "A human approval decision was recorded.",
        Outcome::ConversationIdle => "The conversation went idle after this turn.",
        Outcome::ExplicitRemember => "The user explicitly asked to remember something — capture it verbatim.",
        Outcome::ConversationTurn => "A complete user→agent turn just ended. Decide on your own whether anything in this exchange is durable.",
        Outcome::Unknown => "No specific classification.",
    }
}

fn then_block(outcome: Outcome) -> &'static str {
    match outcome {
        Outcome::ExplicitRemember => "Extract the fact literally stated by the user. Use `learnings/user_preferences` unless scoped to a project. Category: info.",
        Outcome::UserCorrected => "Extract the rule implied by the correction (what NOT to do, or the correct behaviour). Use `learnings/failures` or `learnings/workarounds`. Category: error_success.",
        Outcome::RetrySuccess => "Extract the workaround: under what condition does the first attempt fail, and what made the retry succeed? Use `learnings/workarounds`. Category: solution.",
        Outcome::Failure => "Only emit if a durable pattern is clear (not a one-off). Use `learnings/failures`. Category: error_success.",
        Outcome::Success | Outcome::ConversationIdle => "Only emit if something notable was learned about the user, the domain, the tool, or a reusable capability route. If a tool_trace shows capability_search/skill_search/tool_search/captain_docs resolving a general class of task, capture the route as category skill or solution. Otherwise return [].",
        Outcome::ConversationTurn => "Read both turns carefully. Most often: return []. Emit only when the exchange teaches a durable preference, an acquired skill, a lesson worth retaining, or a precise solution. If the regex_hint is set, treat it as a nudge but make your own judgement.",
        Outcome::Cancelled | Outcome::ApprovalDecision | Outcome::Unknown => "Usually nothing worth remembering. Return []. unless a clear, universal rule emerged.",
    }
}

// ---------------------------------------------------------------------------
// Parser
// ---------------------------------------------------------------------------

/// Parse the raw model output into `Vec<MemoryCandidate>`. Tolerates
/// surrounding prose (extracts the first JSON array) and skips
/// individual entries that don't match the schema. Never panics.
pub fn parse_candidates(raw: &str) -> Vec<MemoryCandidate> {
    let json_slice = extract_json_array(raw).unwrap_or(raw);
    let Ok(value) = serde_json::from_str::<serde_json::Value>(json_slice) else {
        debug!("reflection parse: not JSON, returning []");
        return Vec::new();
    };
    let Some(array) = value.as_array() else {
        debug!("reflection parse: not a JSON array, returning []");
        return Vec::new();
    };
    array
        .iter()
        .filter_map(
            |v| match serde_json::from_value::<MemoryCandidate>(v.clone()) {
                Ok(c) => Some(c),
                Err(e) => {
                    debug!(error = %e, "reflection parse: skipping invalid entry");
                    None
                }
            },
        )
        .collect()
}

/// Best-effort extraction of a `[...]` JSON array from arbitrary text.
/// Returns `None` if no balanced array is found.
fn extract_json_array(text: &str) -> Option<&str> {
    let start = text.find('[')?;
    let bytes = text.as_bytes();
    let mut depth: i32 = 0;
    for (i, &b) in bytes.iter().enumerate().skip(start) {
        match b {
            b'[' => depth += 1,
            b']' => {
                depth -= 1;
                if depth == 0 {
                    return Some(&text[start..=i]);
                }
            }
            _ => {}
        }
    }
    None
}

// ---------------------------------------------------------------------------
// Runner
// ---------------------------------------------------------------------------

/// Execute the reflection call with timeout + fallback chain. Returns
/// the filtered list of candidates (confidence ≥ threshold).
pub async fn run_reflection(
    completer: &dyn ReflectionCompleter,
    cfg: &ReflectionConfig,
    classified: &ClassifiedSignal,
) -> Vec<MemoryCandidate> {
    let (system, user) = build_prompt(classified);
    let timeout_dur = Duration::from_secs(cfg.timeout_secs);

    // Try primary then each fallback.
    let mut chain = vec![cfg.primary_model.clone()];
    chain.extend(cfg.fallback_models.iter().cloned());

    for model in &chain {
        match timeout(timeout_dur, completer.complete(model, &system, &user)).await {
            Ok(Ok(raw)) => {
                let parsed_pre = parse_candidates(&raw);
                let parsed_count = parsed_pre.len();
                let candidates: Vec<MemoryCandidate> = parsed_pre
                    .into_iter()
                    .filter(|c| c.confidence >= cfg.min_confidence)
                    .collect();
                if candidates.is_empty() {
                    debug!(
                        model = %model,
                        parsed_count = parsed_count,
                        raw_preview = %raw.chars().take(200).collect::<String>(),
                        "reflection produced no candidates after filter"
                    );
                } else {
                    info!(
                        model = %model,
                        count = candidates.len(),
                        "reflection produced candidates"
                    );
                }
                return candidates;
            }
            Ok(Err(e)) => {
                warn!(model = %model, error = %e, "reflection call failed — trying next");
            }
            Err(_) => {
                warn!(
                    model = %model,
                    timeout_secs = cfg.timeout_secs,
                    "reflection timed out — trying next"
                );
            }
        }
    }
    warn!("reflection fallback chain exhausted — returning []");
    Vec::new()
}

/// Spawn the consumer loop. Reads `ClassifiedSignal`s, calls
/// `run_reflection` for each, and forwards non-empty candidate batches
/// (wrapped in `ReflectionBatch` with outcome + agent_id context) to
/// the returned receiver. Downstream stages (policy, committer) need
/// the outcome to route to the right room.
pub fn spawn_consumer(
    mut rx: tokio::sync::mpsc::Receiver<ClassifiedSignal>,
    completer: std::sync::Arc<dyn ReflectionCompleter>,
    cfg: ReflectionConfig,
    output_capacity: usize,
) -> (
    tokio::task::JoinHandle<()>,
    tokio::sync::mpsc::Receiver<ReflectionBatch>,
) {
    let (tx, out_rx) = tokio::sync::mpsc::channel(output_capacity);
    let handle = tokio::spawn(async move {
        while let Some(classified) = rx.recv().await {
            let cand = run_reflection(completer.as_ref(), &cfg, &classified).await;
            if cand.is_empty() {
                continue;
            }
            let batch = ReflectionBatch {
                outcome: classified.outcome,
                agent_id: classified.signal.agent_id().to_string(),
                candidates: cand,
                channel: classified.signal.channel().map(|c| c.to_string()),
            };
            if let Err(e) = tx.try_send(batch) {
                debug!(error = %e, "reflection consumer: downstream full, dropping batch");
            }
        }
    });
    (handle, out_rx)
}
