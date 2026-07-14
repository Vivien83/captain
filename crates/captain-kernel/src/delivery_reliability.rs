//! Small delivery reliability primitives shared by cron/channel paths.
//!
//! Keep this module policy-only: callers still perform their own I/O.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::sync::atomic::{AtomicU64, Ordering};

pub const DEFAULT_MAX_DELIVERY_ATTEMPTS: usize = 5;
pub const DEFAULT_BASE_DELAY_MS: u64 = 500;
pub const DEFAULT_MAX_DELAY_MS: u64 = 8_000;
pub const DEFAULT_JITTER_RATIO_PERCENT: u64 = 50;
pub const MAX_DEAD_LETTERS: usize = 20;
pub const MAX_ERROR_CHARS: usize = 512;
pub const MAX_PAYLOAD_PREVIEW_CHARS: usize = 2_048;

static BACKOFF_JITTER_TICK: AtomicU64 = AtomicU64::new(0);

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct DeliveryFailure {
    pub target: String,
    pub error: String,
    pub attempts: usize,
}

impl DeliveryFailure {
    pub fn new(target: impl Into<String>, error: impl Into<String>, attempts: usize) -> Self {
        Self {
            target: target.into(),
            error: truncate_chars(&error.into(), MAX_ERROR_CHARS),
            attempts: attempts.max(1),
        }
    }
}

impl std::fmt::Display for DeliveryFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{} after {} attempt(s): {}",
            self.target, self.attempts, self.error
        )
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct DeliveryDeadLetter {
    pub timestamp: DateTime<Utc>,
    pub target: String,
    pub error: String,
    pub payload_preview: String,
    pub attempts: usize,
}

pub fn make_dead_letter(
    failure: &DeliveryFailure,
    payload: &str,
    now: DateTime<Utc>,
) -> DeliveryDeadLetter {
    DeliveryDeadLetter {
        timestamp: now,
        target: failure.target.clone(),
        error: failure.error.clone(),
        payload_preview: truncate_chars(payload, MAX_PAYLOAD_PREVIEW_CHARS),
        attempts: failure.attempts.max(1),
    }
}

pub fn push_dead_letter(queue: &mut Vec<DeliveryDeadLetter>, entry: DeliveryDeadLetter) {
    queue.push(entry);
    if queue.len() > MAX_DEAD_LETTERS {
        let overflow = queue.len() - MAX_DEAD_LETTERS;
        queue.drain(0..overflow);
    }
}

pub fn channel_target(channel: &str, recipient: &str) -> String {
    format!("channel:{channel}:{recipient}")
}

pub fn webhook_target(url: &str) -> String {
    format!("webhook:{url}")
}

pub fn backoff_delay_ms(attempt: usize) -> u64 {
    let exponent = attempt.saturating_sub(1).min(10) as u32;
    let delay = DEFAULT_BASE_DELAY_MS.saturating_mul(2u64.saturating_pow(exponent));
    delay.min(DEFAULT_MAX_DELAY_MS)
}

pub fn jittered_backoff_delay_ms(attempt: usize) -> u64 {
    jittered_backoff_delay_ms_with_seed(attempt, jitter_seed())
}

fn jittered_backoff_delay_ms_with_seed(attempt: usize, seed: u64) -> u64 {
    let base = backoff_delay_ms(attempt);
    let jitter_cap = base.saturating_mul(DEFAULT_JITTER_RATIO_PERCENT) / 100;
    if jitter_cap == 0 {
        return base;
    }
    base.saturating_add(pseudo_random_bounded(seed, jitter_cap))
}

fn jitter_seed() -> u64 {
    let tick = BACKOFF_JITTER_TICK
        .fetch_add(1, Ordering::Relaxed)
        .wrapping_add(1);
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_nanos() as u64)
        .unwrap_or(0);
    nanos ^ tick.wrapping_mul(0x9E37_79B9_7F4A_7C15)
}

fn pseudo_random_bounded(seed: u64, max_inclusive: u64) -> u64 {
    let mut value = seed ^ (seed >> 30);
    value = value.wrapping_mul(0xBF58_476D_1CE4_E5B9);
    value ^= value >> 27;
    value = value.wrapping_mul(0x94D0_49BB_1331_11EB);
    value ^= value >> 31;
    value % (max_inclusive + 1)
}

pub fn is_retryable_delivery_error(error: &str) -> bool {
    let lower = error.to_ascii_lowercase();
    if lower.trim().is_empty() {
        return false;
    }

    // Read/write timeouts are ambiguous for non-idempotent sends: the platform
    // may have accepted the message. Avoid duplicate deliveries.
    if lower.contains("readtimeout")
        || lower.contains("write timeout")
        || lower.contains("read timeout")
        || lower.contains("timed out waiting")
    {
        return false;
    }

    let retryable_markers = [
        "available channels",
        "connecterror",
        "connect timeout",
        "connecttimeout",
        "connection refused",
        "connection reset",
        "connection closed",
        "connection aborted",
        "dns",
        "temporarily unavailable",
        "try again",
        "too many requests",
        "rate limit",
        "retry after",
        "flood",
        "http 429",
        "status 429",
        "http 500",
        "http 502",
        "http 503",
        "http 504",
        "status 500",
        "status 502",
        "status 503",
        "status 504",
        "server error",
        "service unavailable",
        "gateway timeout",
    ];
    retryable_markers
        .iter()
        .any(|marker| lower.contains(marker))
}

pub fn truncate_chars(value: &str, max_chars: usize) -> String {
    let mut out = String::new();
    for ch in value.chars().take(max_chars) {
        out.push(ch);
    }
    if value.chars().count() > max_chars {
        out.push_str("...");
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn backoff_is_exponential_and_capped() {
        assert_eq!(backoff_delay_ms(1), 500);
        assert_eq!(backoff_delay_ms(2), 1_000);
        assert_eq!(backoff_delay_ms(4), 4_000);
        assert_eq!(backoff_delay_ms(10), 8_000);
    }

    #[test]
    fn jittered_backoff_stays_inside_operational_window() {
        let base = backoff_delay_ms(3);
        let max = base + (base * DEFAULT_JITTER_RATIO_PERCENT / 100);

        for seed in 0..128 {
            let delay = jittered_backoff_delay_ms_with_seed(3, seed);
            assert!(delay >= base);
            assert!(delay <= max);
        }
    }

    #[test]
    fn jittered_backoff_decorrelates_equal_attempts() {
        let samples = (0..16)
            .map(|seed| jittered_backoff_delay_ms_with_seed(2, seed))
            .collect::<std::collections::BTreeSet<_>>();

        assert!(samples.len() > 1);
    }

    #[test]
    fn retryable_classification_avoids_ambiguous_read_timeouts() {
        assert!(is_retryable_delivery_error(
            "httpx.ConnectError: connection refused"
        ));
        assert!(is_retryable_delivery_error("HTTP 503 service unavailable"));
        assert!(is_retryable_delivery_error("rate limit: retry after 3"));
        assert!(!is_retryable_delivery_error(
            "ReadTimeout: request timed out"
        ));
        assert!(!is_retryable_delivery_error("Forbidden: bot was blocked"));
    }

    #[test]
    fn dead_letter_queue_is_bounded() {
        let failure = DeliveryFailure::new("channel:telegram:42", "boom", 2);
        let mut queue = Vec::new();
        for i in 0..(MAX_DEAD_LETTERS + 3) {
            let entry = make_dead_letter(&failure, &format!("payload {i}"), Utc::now());
            push_dead_letter(&mut queue, entry);
        }
        assert_eq!(queue.len(), MAX_DEAD_LETTERS);
        assert_eq!(queue[0].payload_preview, "payload 3");
    }
}
