use super::*;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;

#[test]
fn test_retry_config_defaults() {
    let config = RetryConfig::default();
    assert_eq!(config.max_attempts, 3);
    assert_eq!(config.min_delay_ms, 300);
    assert_eq!(config.max_delay_ms, 30_000);
    assert!((config.jitter - 0.2).abs() < f64::EPSILON);
}

#[test]
fn test_compute_backoff_exponential() {
    let config = RetryConfig {
        max_attempts: 5,
        min_delay_ms: 100,
        max_delay_ms: 100_000,
        jitter: 0.0,
    };

    assert_eq!(compute_backoff(&config, 0), 100);
    assert_eq!(compute_backoff(&config, 1), 200);
    assert_eq!(compute_backoff(&config, 2), 400);
    assert_eq!(compute_backoff(&config, 3), 800);
}

#[test]
fn test_compute_backoff_capped() {
    let config = RetryConfig {
        max_attempts: 10,
        min_delay_ms: 1_000,
        max_delay_ms: 5_000,
        jitter: 0.0,
    };

    assert_eq!(compute_backoff(&config, 0), 1_000);
    assert_eq!(compute_backoff(&config, 1), 2_000);
    assert_eq!(compute_backoff(&config, 2), 4_000);
    assert_eq!(compute_backoff(&config, 3), 5_000);
    assert_eq!(compute_backoff(&config, 10), 5_000);
}

#[tokio::test]
async fn test_retry_success_first_try() {
    let config = RetryConfig {
        max_attempts: 3,
        min_delay_ms: 10,
        max_delay_ms: 100,
        jitter: 0.0,
    };

    let outcome = retry_async(
        &config,
        || async { Ok::<&str, &str>("hello") },
        |_| true,
        |_: &&str| None,
    )
    .await;

    match outcome {
        RetryOutcome::Success { result, attempts } => {
            assert_eq!(result, "hello");
            assert_eq!(attempts, 1);
        }
        _ => panic!("expected success"),
    }
}

#[tokio::test]
async fn test_retry_success_after_failures() {
    let config = RetryConfig {
        max_attempts: 5,
        min_delay_ms: 1,
        max_delay_ms: 10,
        jitter: 0.0,
    };

    let counter = Arc::new(AtomicU32::new(0));
    let counter_clone = counter.clone();

    let outcome = retry_async(
        &config,
        move || {
            let c = counter_clone.clone();
            async move {
                let n = c.fetch_add(1, Ordering::SeqCst);
                if n < 2 {
                    Err("not yet")
                } else {
                    Ok("finally")
                }
            }
        },
        |_| true,
        |_: &&str| None,
    )
    .await;

    match outcome {
        RetryOutcome::Success { result, attempts } => {
            assert_eq!(result, "finally");
            assert_eq!(attempts, 3);
        }
        _ => panic!("expected success"),
    }
}

#[tokio::test]
async fn test_retry_exhausted() {
    let config = RetryConfig {
        max_attempts: 3,
        min_delay_ms: 1,
        max_delay_ms: 10,
        jitter: 0.0,
    };

    let outcome = retry_async(
        &config,
        || async { Err::<(), &str>("always fails") },
        |_| true,
        |_: &&str| None,
    )
    .await;

    match outcome {
        RetryOutcome::Exhausted {
            last_error,
            attempts,
        } => {
            assert_eq!(last_error, "always fails");
            assert_eq!(attempts, 3);
        }
        _ => panic!("expected exhausted"),
    }
}

#[tokio::test]
async fn test_retry_non_retryable_error() {
    let config = RetryConfig {
        max_attempts: 5,
        min_delay_ms: 1,
        max_delay_ms: 10,
        jitter: 0.0,
    };

    let counter = Arc::new(AtomicU32::new(0));
    let counter_clone = counter.clone();

    let outcome = retry_async(
        &config,
        move || {
            let c = counter_clone.clone();
            async move {
                c.fetch_add(1, Ordering::SeqCst);
                Err::<(), &str>("fatal error")
            }
        },
        |_| false,
        |_: &&str| None,
    )
    .await;

    match outcome {
        RetryOutcome::Exhausted {
            last_error,
            attempts,
        } => {
            assert_eq!(last_error, "fatal error");
            assert_eq!(attempts, 1);
        }
        _ => panic!("expected exhausted"),
    }

    assert_eq!(counter.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn test_retry_with_hint_delay() {
    let config = RetryConfig {
        max_attempts: 3,
        min_delay_ms: 10_000,
        max_delay_ms: 60_000,
        jitter: 0.0,
    };

    let counter = Arc::new(AtomicU32::new(0));
    let counter_clone = counter.clone();

    let start = std::time::Instant::now();

    let outcome = retry_async(
        &config,
        move || {
            let c = counter_clone.clone();
            async move {
                let n = c.fetch_add(1, Ordering::SeqCst);
                if n < 1 {
                    Err("transient")
                } else {
                    Ok("ok")
                }
            }
        },
        |_| true,
        |_: &&str| Some(1),
    )
    .await;

    let elapsed = start.elapsed();

    match outcome {
        RetryOutcome::Success { result, attempts } => {
            assert_eq!(result, "ok");
            assert_eq!(attempts, 2);
            assert!(
                elapsed.as_millis() < 5_000,
                "retry took too long: {:?}; hint should have overridden base delay",
                elapsed
            );
        }
        _ => panic!("expected success"),
    }
}

#[test]
fn test_llm_retry_config() {
    let config = llm_retry_config();
    assert_eq!(config.max_attempts, 3);
    assert_eq!(config.min_delay_ms, 1_000);
    assert_eq!(config.max_delay_ms, 60_000);
    assert!((config.jitter - 0.2).abs() < f64::EPSILON);
}

#[test]
fn test_channel_retry_config() {
    let config = channel_retry_config();
    assert_eq!(config.max_attempts, 3);
    assert_eq!(config.min_delay_ms, 400);
    assert_eq!(config.max_delay_ms, 15_000);
    assert!((config.jitter - 0.1).abs() < f64::EPSILON);
}

#[test]
fn test_network_retry_config() {
    let config = network_retry_config();
    assert_eq!(config.max_attempts, 3);
    assert_eq!(config.min_delay_ms, 500);
    assert_eq!(config.max_delay_ms, 30_000);
    assert!((config.jitter - 0.1).abs() < f64::EPSILON);
}
