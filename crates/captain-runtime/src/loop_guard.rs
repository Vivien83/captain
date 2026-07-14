//! Tool loop detection for the agent execution loop.
//!
//! Tracks tool calls within a single agent loop execution using SHA-256
//! hashes of `(tool_name, serialized_params)`. Detects when the agent is
//! stuck calling the same tool repeatedly and provides graduated responses:
//! warn, block, or circuit-break the entire loop.
//!
//! Enhanced features beyond basic hash-counting:
//! - **Outcome-aware detection**: tracks result hashes so identical call+result
//!   pairs escalate faster than just repeated calls.
//! - **Ping-pong detection**: identifies A-B-A-B or A-B-C-A-B-C alternating
//!   patterns that evade single-hash counting.
//! - **Poll tool handling**: relaxed thresholds for tools expected to be called
//!   repeatedly (e.g. `shell_exec` status checks).
//! - **Backoff suggestions**: recommends increasing wait times for polling.
//! - **Warning bucket**: prevents spam by upgrading to Block after repeated
//!   warnings for the same call.
//! - **Statistics snapshot**: exposes internal state for debugging and API.

use serde::Serialize;
use sha2::{Digest, Sha256};
use std::collections::{HashMap, HashSet};

use crate::loop_guard_patterns::{count_ping_pong_repeats, detect_ping_pong};

/// Tools that are expected to be polled repeatedly.
const POLL_TOOLS: &[&str] = &[
    "shell_exec", // checking command output
];

/// Maximum recent call history size for ping-pong detection.
const HISTORY_SIZE: usize = 30;

/// Backoff schedule in milliseconds for polling tools.
const BACKOFF_SCHEDULE_MS: &[u64] = &[5000, 10000, 30000, 60000];

/// Configuration for the loop guard.
#[derive(Debug, Clone)]
pub struct LoopGuardConfig {
    /// Number of identical calls before a warning is appended.
    pub warn_threshold: u32,
    /// Number of identical calls before the call is blocked.
    pub block_threshold: u32,
    /// Total tool calls across all tools before circuit-breaking.
    pub global_circuit_breaker: u32,
    /// Multiplier for poll tool thresholds (poll tools get thresholds * this).
    pub poll_multiplier: u32,
    /// Number of identical outcome pairs before a warning.
    pub outcome_warn_threshold: u32,
    /// Number of identical outcome pairs before the next call is auto-blocked.
    pub outcome_block_threshold: u32,
    /// Minimum repeats of a ping-pong pattern before blocking.
    pub ping_pong_min_repeats: u32,
    /// Max warnings per unique tool call hash before upgrading to Block.
    pub max_warnings_per_call: u32,
    /// Consecutive failures of the SAME tool (regardless of exact
    /// parameters) before a warning is appended to the failing result.
    /// Catches a model that varies an unrelated argument (e.g. a name) on
    /// every retry to route around a rejection — the per-hash counters
    /// above never fire in that case since every call has different params.
    pub consecutive_error_warn_threshold: u32,
    /// Consecutive failures of the same tool before the NEXT attempt is
    /// blocked outright, independent of its (possibly different) params.
    pub consecutive_error_block_threshold: u32,
}

impl Default for LoopGuardConfig {
    fn default() -> Self {
        Self {
            warn_threshold: 3,
            block_threshold: 5,
            global_circuit_breaker: 30,
            poll_multiplier: 3,
            outcome_warn_threshold: 2,
            outcome_block_threshold: 3,
            ping_pong_min_repeats: 3,
            max_warnings_per_call: 3,
            consecutive_error_warn_threshold: 3,
            consecutive_error_block_threshold: 5,
        }
    }
}

/// Verdict from the loop guard on whether a tool call should proceed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LoopGuardVerdict {
    /// Proceed normally.
    Allow,
    /// Proceed, but append a warning to the tool result.
    Warn(String),
    /// Block this specific tool call (skip execution).
    Block(String),
    /// Circuit-break the entire agent loop.
    CircuitBreak(String),
}

/// Snapshot of the loop guard state (for debugging/API).
#[derive(Debug, Clone, Serialize)]
pub struct LoopGuardStats {
    /// Total tool calls made in this loop execution.
    pub total_calls: u32,
    /// Number of unique (tool_name + params) combinations seen.
    pub unique_calls: u32,
    /// Number of calls that were blocked.
    pub blocked_calls: u32,
    /// Whether a ping-pong pattern has been detected.
    pub ping_pong_detected: bool,
    /// The tool name that has been repeated the most (if any).
    pub most_repeated_tool: Option<String>,
    /// The count of the most repeated tool call.
    pub most_repeated_count: u32,
}

/// Tracks tool calls within a single agent loop to detect loops.
pub struct LoopGuard {
    config: LoopGuardConfig,
    /// Count of identical (tool_name + params) calls, keyed by SHA-256 hex hash.
    call_counts: HashMap<String, u32>,
    /// Total tool calls in this loop execution.
    total_calls: u32,
    /// Count of identical (tool_call_hash + result_hash) pairs.
    outcome_counts: HashMap<String, u32>,
    /// Call hashes that are blocked due to repeated identical outcomes.
    blocked_outcomes: HashSet<String>,
    /// Recent tool call hashes (ring buffer of last HISTORY_SIZE).
    recent_calls: Vec<String>,
    /// Warnings already emitted (to prevent spam). Key = call hash, value = count emitted.
    warnings_emitted: HashMap<String, u32>,
    /// Tracks poll counts per command hash for backoff suggestions.
    poll_counts: HashMap<String, u32>,
    /// Total calls that were blocked.
    blocked_calls: u32,
    /// Map from call hash to tool name (for stats reporting).
    hash_to_tool: HashMap<String, String>,
    /// Consecutive failure count per tool_name (not per exact-params hash).
    /// Reset to 0 on any success for that tool.
    consecutive_tool_errors: HashMap<String, u32>,
}

impl LoopGuard {
    /// Create a new loop guard with the given configuration.
    pub fn new(config: LoopGuardConfig) -> Self {
        Self {
            config,
            call_counts: HashMap::new(),
            total_calls: 0,
            outcome_counts: HashMap::new(),
            blocked_outcomes: HashSet::new(),
            recent_calls: Vec::with_capacity(HISTORY_SIZE),
            warnings_emitted: HashMap::new(),
            poll_counts: HashMap::new(),
            blocked_calls: 0,
            hash_to_tool: HashMap::new(),
            consecutive_tool_errors: HashMap::new(),
        }
    }

    /// Check whether a tool call should proceed.
    ///
    /// Returns a verdict indicating whether to allow, warn, block, or
    /// circuit-break. The caller should act on the verdict before executing
    /// the tool.
    pub fn check(&mut self, tool_name: &str, params: &serde_json::Value) -> LoopGuardVerdict {
        self.total_calls += 1;

        // Global circuit breaker
        if self.total_calls > self.config.global_circuit_breaker {
            self.blocked_calls += 1;
            return LoopGuardVerdict::CircuitBreak(format!(
                "Circuit breaker: exceeded {} total tool calls in this loop. \
                 The agent appears to be stuck.",
                self.config.global_circuit_breaker
            ));
        }

        // Consecutive-failure check, independent of the per-hash counters
        // below: catches a tool that keeps failing while the model varies
        // an unrelated argument on every call (a per-hash count would never
        // reach its threshold in that case since every hash is unique).
        if let Some(&streak) = self.consecutive_tool_errors.get(tool_name) {
            if streak >= self.config.consecutive_error_block_threshold {
                self.blocked_calls += 1;
                return LoopGuardVerdict::Block(format!(
                    "Blocked: tool '{tool_name}' has failed {streak} times in a row \
                     (parameters varied each time, so this isn't the identical-call \
                     detector above). Varying one argument to route around each \
                     rejection isn't working — read the last error message carefully \
                     and address what it actually says, or stop using this tool for \
                     this task."
                ));
            }
        }

        let hash = Self::compute_hash(tool_name, params);
        self.hash_to_tool
            .entry(hash.clone())
            .or_insert_with(|| tool_name.to_string());

        // Track recent calls for ping-pong detection
        if self.recent_calls.len() >= HISTORY_SIZE {
            self.recent_calls.remove(0);
        }
        self.recent_calls.push(hash.clone());

        // Check if this call hash was blocked by outcome detection
        if self.blocked_outcomes.contains(&hash) {
            self.blocked_calls += 1;
            return LoopGuardVerdict::Block(format!(
                "Blocked: tool '{}' is returning identical results repeatedly. \
                 The current approach is not working — try something different.",
                tool_name
            ));
        }

        let count = self.call_counts.entry(hash.clone()).or_insert(0);
        *count += 1;
        let count_val = *count;

        // Determine effective thresholds (poll tools get relaxed thresholds)
        let is_poll = Self::is_poll_call(tool_name, params);
        let multiplier = if is_poll {
            self.config.poll_multiplier
        } else {
            1
        };
        let effective_warn = self.config.warn_threshold * multiplier;
        let effective_block = self.config.block_threshold * multiplier;

        // Check per-hash thresholds
        if count_val >= effective_block {
            self.blocked_calls += 1;
            return LoopGuardVerdict::Block(format!(
                "Blocked: tool '{}' called {} times with identical parameters. \
                 Try a different approach or different parameters.",
                tool_name, count_val
            ));
        }

        if count_val >= effective_warn {
            // Warning bucket: check if we've already warned too many times
            let warning_count = self.warnings_emitted.entry(hash.clone()).or_insert(0);
            *warning_count += 1;
            if *warning_count > self.config.max_warnings_per_call {
                // Upgrade to block after too many warnings
                self.blocked_calls += 1;
                return LoopGuardVerdict::Block(format!(
                    "Blocked: tool '{}' called {} times with identical parameters \
                     (warnings exhausted). Try a different approach.",
                    tool_name, count_val
                ));
            }
            return LoopGuardVerdict::Warn(format!(
                "Warning: tool '{}' has been called {} times with identical parameters. \
                 Consider a different approach.",
                tool_name, count_val
            ));
        }

        // Ping-pong detection (runs even if individual hash counts are low)
        if let Some(ping_pong_msg) = detect_ping_pong(&self.recent_calls, &self.hash_to_tool) {
            let repeats = count_ping_pong_repeats(&self.recent_calls);
            if repeats >= self.config.ping_pong_min_repeats {
                self.blocked_calls += 1;
                return LoopGuardVerdict::Block(ping_pong_msg);
            }
            // Below min_repeats, just warn
            let warning_count = self
                .warnings_emitted
                .entry(format!("pingpong_{}", hash))
                .or_insert(0);
            *warning_count += 1;
            if *warning_count <= self.config.max_warnings_per_call {
                return LoopGuardVerdict::Warn(ping_pong_msg);
            }
        }

        LoopGuardVerdict::Allow
    }

    /// Record the outcome of a tool call. Call this AFTER tool execution.
    ///
    /// Hashes `(tool_name | params_json | result_truncated)` and tracks how
    /// many times an identical call produces an identical result. Returns a
    /// warning string if outcome repetition is detected.
    pub fn record_outcome(
        &mut self,
        tool_name: &str,
        params: &serde_json::Value,
        result: &str,
    ) -> Option<String> {
        let outcome_hash = Self::compute_outcome_hash(tool_name, params, result);
        let call_hash = Self::compute_hash(tool_name, params);

        let count = self.outcome_counts.entry(outcome_hash).or_insert(0);
        *count += 1;
        let count_val = *count;

        if count_val >= self.config.outcome_block_threshold {
            // Mark the call hash so the NEXT check() auto-blocks it
            self.blocked_outcomes.insert(call_hash);
            return Some(format!(
                "Tool '{}' is returning identical results — the approach isn't working.",
                tool_name
            ));
        }

        if count_val >= self.config.outcome_warn_threshold {
            return Some(format!(
                "Tool '{}' is returning identical results — the approach isn't working.",
                tool_name
            ));
        }

        None
    }

    /// Record whether a tool call succeeded or failed, tracked per tool_name
    /// regardless of its exact parameters. Call this AFTER tool execution,
    /// alongside (not instead of) `check()`. A run of failures builds toward
    /// the pre-execution block in `check()` above; any success resets the
    /// streak. Returns a warning to append to the current (already-executed)
    /// result once the streak crosses the warn threshold but hasn't yet hit
    /// the block threshold.
    pub fn record_tool_error(&mut self, tool_name: &str, is_error: bool) -> Option<String> {
        if !is_error {
            self.consecutive_tool_errors.remove(tool_name);
            return None;
        }
        let count = self
            .consecutive_tool_errors
            .entry(tool_name.to_string())
            .or_insert(0);
        *count += 1;
        let count_val = *count;

        if count_val >= self.config.consecutive_error_warn_threshold {
            return Some(format!(
                "Warning: '{tool_name}' has failed {count_val} times in a row. \
                 If you're varying one argument each time hoping it'll work, \
                 re-read the error message instead — it usually says what's \
                 actually wrong."
            ));
        }
        None
    }

    /// Get the suggested backoff delay (in milliseconds) for a polling tool call.
    ///
    /// Returns `None` if this is not a poll call. Returns `Some(ms)` with a
    /// suggested delay from the backoff schedule, capping at the last entry.
    pub fn get_poll_backoff(&mut self, tool_name: &str, params: &serde_json::Value) -> Option<u64> {
        if !Self::is_poll_call(tool_name, params) {
            return None;
        }
        let hash = Self::compute_hash(tool_name, params);
        let count = self.poll_counts.entry(hash).or_insert(0);
        *count += 1;
        // count is 1-indexed; backoff starts on the second call
        if *count <= 1 {
            return None;
        }
        let idx = (*count as usize).saturating_sub(2);
        let delay = BACKOFF_SCHEDULE_MS
            .get(idx)
            .copied()
            .unwrap_or(*BACKOFF_SCHEDULE_MS.last().unwrap_or(&60000));
        Some(delay)
    }

    /// Get a snapshot of current loop guard statistics.
    pub fn stats(&self) -> LoopGuardStats {
        let unique_calls = self.call_counts.len() as u32;

        // Find the most repeated tool call
        let mut most_repeated_tool: Option<String> = None;
        let mut most_repeated_count: u32 = 0;
        for (hash, &count) in &self.call_counts {
            if count > most_repeated_count {
                most_repeated_count = count;
                most_repeated_tool = self.hash_to_tool.get(hash).cloned();
            }
        }

        LoopGuardStats {
            total_calls: self.total_calls,
            unique_calls,
            blocked_calls: self.blocked_calls,
            ping_pong_detected: detect_ping_pong(&self.recent_calls, &self.hash_to_tool).is_some(),
            most_repeated_tool,
            most_repeated_count,
        }
    }

    /// Check if a tool call looks like a polling operation.
    ///
    /// Poll tools (like `shell_exec` for status checks) are expected to be
    /// called repeatedly and get relaxed loop detection thresholds.
    fn is_poll_call(tool_name: &str, params: &serde_json::Value) -> bool {
        // Known poll tools with short commands that look like status checks
        if POLL_TOOLS.contains(&tool_name) {
            if let Some(cmd) = params.get("command").and_then(|v| v.as_str()) {
                let cmd_lower = cmd.to_lowercase();
                // Commands that explicitly check status/wait/poll
                if cmd_lower.contains("status")
                    || cmd_lower.contains("poll")
                    || cmd_lower.contains("wait")
                    || cmd_lower.contains("watch")
                    || cmd_lower.contains("tail")
                    || cmd_lower.contains("ps ")
                    || cmd_lower.contains("jobs")
                    || cmd_lower.contains("pgrep")
                    || cmd_lower.contains("docker ps")
                    || cmd_lower.contains("kubectl get")
                {
                    return true;
                }
            }
        }
        // Generic poll detection via params keywords
        let params_str = serde_json::to_string(params)
            .unwrap_or_default()
            .to_lowercase();
        params_str.contains("status") || params_str.contains("poll") || params_str.contains("wait")
    }

    /// Compute a SHA-256 hash of the tool name and parameters.
    fn compute_hash(tool_name: &str, params: &serde_json::Value) -> String {
        let mut hasher = Sha256::new();
        hasher.update(tool_name.as_bytes());
        hasher.update(b"|");
        // Serialize params deterministically (serde_json sorts object keys)
        let params_str = serde_json::to_string(params).unwrap_or_default();
        hasher.update(params_str.as_bytes());
        hex::encode(hasher.finalize())
    }

    /// Compute a SHA-256 hash of the tool name, parameters, AND result.
    ///
    /// Result is truncated to 1000 chars to avoid hashing huge outputs
    /// while still catching identical short results.
    fn compute_outcome_hash(tool_name: &str, params: &serde_json::Value, result: &str) -> String {
        let mut hasher = Sha256::new();
        hasher.update(tool_name.as_bytes());
        hasher.update(b"|");
        let params_str = serde_json::to_string(params).unwrap_or_default();
        hasher.update(params_str.as_bytes());
        hasher.update(b"|");
        let truncated = crate::str_utils::safe_truncate_str(result, 1000);
        hasher.update(truncated.as_bytes());
        hex::encode(hasher.finalize())
    }
}

/// Merge the pre-execution loop-guard verdict (from `check()`) with a
/// post-execution consecutive-error warning (from `record_tool_error()`),
/// for attaching to the tool result that already ran. Both are informational
/// (`Allow`/`Warn` only reach here — `Block`/`CircuitBreak` divert before
/// execution), so this only needs to combine their messages when both fire.
pub fn combine_verdict_with_error_streak(
    verdict: LoopGuardVerdict,
    streak_warning: Option<String>,
) -> LoopGuardVerdict {
    match (verdict, streak_warning) {
        (LoopGuardVerdict::Warn(existing), Some(streak)) => {
            LoopGuardVerdict::Warn(format!("{existing}\n{streak}"))
        }
        (LoopGuardVerdict::Allow, Some(streak)) => LoopGuardVerdict::Warn(streak),
        (verdict, _) => verdict,
    }
}

#[cfg(test)]
#[path = "loop_guard_tests.rs"]
mod tests;
