//! Generic retry with exponential backoff and jitter.
//!
//! Provides a configurable, async-aware retry utility that can be used for
//! LLM API calls, network operations, channel message delivery, and any
//! other fallible async operation across the Captain codebase.
//!
//! Jitter uses `std::time::SystemTime` UNIX nanos as a seed to avoid
//! requiring the `rand` crate as a dependency.

use tracing::{debug, warn};

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

/// Configuration for retry behavior.
#[derive(Debug, Clone)]
pub struct RetryConfig {
    /// Maximum number of attempts (including the first try).
    pub max_attempts: u32,
    /// Minimum delay between retries in milliseconds.
    pub min_delay_ms: u64,
    /// Maximum delay between retries in milliseconds.
    pub max_delay_ms: u64,
    /// Jitter factor (0.0 = no jitter, 1.0 = full jitter).
    ///
    /// The actual sleep is `delay * (1 + random_fraction * jitter)`, where
    /// `random_fraction` is in `[0, 1)`.
    pub jitter: f64,
}

impl Default for RetryConfig {
    fn default() -> Self {
        Self {
            max_attempts: 3,
            min_delay_ms: 300,
            max_delay_ms: 30_000,
            jitter: 0.2,
        }
    }
}

/// Result of a retry operation.
#[derive(Debug)]
pub enum RetryOutcome<T, E> {
    /// The operation succeeded.
    Success {
        /// The successful result.
        result: T,
        /// Total number of attempts made (1 = first try succeeded).
        attempts: u32,
    },
    /// All retries exhausted without success.
    Exhausted {
        /// The error from the last attempt.
        last_error: E,
        /// Total number of attempts made.
        attempts: u32,
    },
}

// ---------------------------------------------------------------------------
// Backoff computation
// ---------------------------------------------------------------------------

/// Compute the delay for a given attempt (0-indexed).
///
/// Formula: `min(min_delay * 2^attempt, max_delay) * (1 + random * jitter)`
///
/// Uses `std::time::SystemTime` nanos as a lightweight pseudo-random source
/// instead of requiring the `rand` crate.
pub fn compute_backoff(config: &RetryConfig, attempt: u32) -> u64 {
    // Exponential base: min_delay * 2^attempt, capped at max_delay.
    let base = config
        .min_delay_ms
        .saturating_mul(1u64.checked_shl(attempt).unwrap_or(u64::MAX));
    let capped = base.min(config.max_delay_ms);

    // Jitter: multiply by (1 + random_fraction * jitter).
    if config.jitter <= 0.0 {
        return capped;
    }

    let frac = pseudo_random_fraction();
    let jitter_offset = (capped as f64) * frac * config.jitter;
    let with_jitter = (capped as f64) + jitter_offset;

    // Clamp to max_delay (jitter can push slightly above).
    (with_jitter as u64).min(config.max_delay_ms)
}

/// Return a pseudo-random fraction in `[0, 1)` using the current system time
/// nanos. This is NOT cryptographically secure, but good enough for jitter.
fn pseudo_random_fraction() -> f64 {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .subsec_nanos();
    // Mix the bits a bit to reduce predictability.
    let mixed = nanos.wrapping_mul(2654435761); // Knuth multiplicative hash
    (mixed as f64) / (u32::MAX as f64)
}

// ---------------------------------------------------------------------------
// Core retry function
// ---------------------------------------------------------------------------

/// Execute an async operation with retry.
///
/// # Parameters
///
/// - `config` — retry configuration (attempts, delays, jitter).
/// - `operation` — the async closure to execute. Called once per attempt.
/// - `should_retry` — predicate that inspects the error and returns `true`
///   if the operation should be retried.
/// - `retry_after_hint` — optional hint extractor. If it returns `Some(ms)`,
///   that delay is used instead of the computed backoff (but still capped at
///   `max_delay_ms`).
///
/// # Returns
///
/// A `RetryOutcome` indicating success or exhaustion.
pub async fn retry_async<F, Fut, T, E, P, H>(
    config: &RetryConfig,
    mut operation: F,
    should_retry: P,
    retry_after_hint: H,
) -> RetryOutcome<T, E>
where
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = Result<T, E>>,
    P: Fn(&E) -> bool,
    H: Fn(&E) -> Option<u64>,
    E: std::fmt::Debug,
{
    let max = config.max_attempts.max(1);
    let mut last_error: Option<E> = None;

    for attempt in 0..max {
        match operation().await {
            Ok(result) => {
                if attempt > 0 {
                    debug!(
                        attempt = attempt + 1,
                        "retry succeeded after {} previous failures", attempt
                    );
                }
                return RetryOutcome::Success {
                    result,
                    attempts: attempt + 1,
                };
            }
            Err(err) => {
                let is_last = attempt + 1 >= max;

                if is_last || !should_retry(&err) {
                    if !should_retry(&err) {
                        debug!(
                            attempt = attempt + 1,
                            "error is not retryable, giving up: {:?}", err
                        );
                    } else {
                        warn!(
                            attempt = attempt + 1,
                            max_attempts = max,
                            "all retry attempts exhausted: {:?}",
                            err
                        );
                    }
                    return RetryOutcome::Exhausted {
                        last_error: err,
                        attempts: attempt + 1,
                    };
                }

                // Determine delay.
                let hint = retry_after_hint(&err);
                let delay_ms = if let Some(hinted) = hint {
                    // Respect the hint, but cap it.
                    hinted.min(config.max_delay_ms)
                } else {
                    compute_backoff(config, attempt)
                };

                debug!(
                    attempt = attempt + 1,
                    delay_ms, "retrying after error: {:?}", err
                );

                tokio::time::sleep(std::time::Duration::from_millis(delay_ms)).await;

                last_error = Some(err);
            }
        }
    }

    // Should not be reachable, but handle gracefully.
    RetryOutcome::Exhausted {
        last_error: last_error.expect("at least one attempt should have been made"),
        attempts: max,
    }
}

// ---------------------------------------------------------------------------
// Pre-built configs
// ---------------------------------------------------------------------------

/// Retry config for LLM API calls.
///
/// 3 attempts, 1s initial delay, up to 60s, 20% jitter.
pub fn llm_retry_config() -> RetryConfig {
    RetryConfig {
        max_attempts: 3,
        min_delay_ms: 1_000,
        max_delay_ms: 60_000,
        jitter: 0.2,
    }
}

/// Retry config for network operations (webhooks, fetches).
///
/// 3 attempts, 500ms initial delay, up to 30s, 10% jitter.
pub fn network_retry_config() -> RetryConfig {
    RetryConfig {
        max_attempts: 3,
        min_delay_ms: 500,
        max_delay_ms: 30_000,
        jitter: 0.1,
    }
}

/// Retry config for channel message delivery.
///
/// 3 attempts, 400ms initial delay, up to 15s, 10% jitter.
pub fn channel_retry_config() -> RetryConfig {
    RetryConfig {
        max_attempts: 3,
        min_delay_ms: 400,
        max_delay_ms: 15_000,
        jitter: 0.1,
    }
}

#[cfg(test)]
#[path = "retry_tests.rs"]
mod tests;
