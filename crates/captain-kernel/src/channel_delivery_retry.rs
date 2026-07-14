//! Retry wrapper for direct channel deliveries.
//!
//! Cron has its own durable redelivery queue. Direct `channel_send` calls stay
//! synchronous, but they still need Hermes-style resilience for transient
//! channel faults: retry with jitter, avoid ambiguous timeouts, then return a
//! clear failure to the agent.

use std::future::Future;
use std::time::Duration;

use crate::delivery_reliability::{
    is_retryable_delivery_error, jittered_backoff_delay_ms, DeliveryFailure,
    DEFAULT_MAX_DELIVERY_ATTEMPTS,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChannelDeliveryOutcome<T> {
    pub value: T,
    pub attempts: usize,
}

pub async fn retry_channel_delivery<T, F, Fut>(
    target: &str,
    operation: F,
) -> Result<ChannelDeliveryOutcome<T>, String>
where
    F: FnMut() -> Fut,
    Fut: Future<Output = Result<T, String>>,
{
    retry_channel_delivery_with_sleep(target, operation, |delay_ms| async move {
        tokio::time::sleep(Duration::from_millis(delay_ms)).await;
    })
    .await
}

async fn retry_channel_delivery_with_sleep<T, F, Fut, S, SleepFut>(
    target: &str,
    mut operation: F,
    mut sleep: S,
) -> Result<ChannelDeliveryOutcome<T>, String>
where
    F: FnMut() -> Fut,
    Fut: Future<Output = Result<T, String>>,
    S: FnMut(u64) -> SleepFut,
    SleepFut: Future<Output = ()>,
{
    for attempt in 1..=DEFAULT_MAX_DELIVERY_ATTEMPTS {
        match operation().await {
            Ok(value) => {
                return Ok(ChannelDeliveryOutcome {
                    value,
                    attempts: attempt,
                })
            }
            Err(error) => {
                let retryable = is_retryable_delivery_error(&error);
                if !retryable || attempt == DEFAULT_MAX_DELIVERY_ATTEMPTS {
                    return Err(DeliveryFailure::new(target, error, attempt).to_string());
                }

                let delay_ms = jittered_backoff_delay_ms(attempt);
                tracing::warn!(
                    target = %target,
                    attempt,
                    next_delay_ms = delay_ms,
                    error = %error,
                    "Direct channel delivery failed transiently, retrying"
                );
                sleep(delay_ms).await;
            }
        }
    }

    Err(DeliveryFailure::new(
        target,
        "delivery failed after retries",
        DEFAULT_MAX_DELIVERY_ATTEMPTS,
    )
    .to_string())
}

#[cfg(test)]
mod tests {
    use std::sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    };

    use super::*;

    #[tokio::test]
    async fn retries_transient_channel_error() {
        let calls = Arc::new(AtomicUsize::new(0));
        let seen = calls.clone();

        let result = retry_channel_delivery_with_sleep(
            "channel:telegram:42",
            || {
                let seen = seen.clone();
                async move {
                    let call = seen.fetch_add(1, Ordering::SeqCst);
                    if call == 0 {
                        Err("HTTP 503 service unavailable".to_string())
                    } else {
                        Ok("sent")
                    }
                }
            },
            |_| async {},
        )
        .await
        .expect("retryable send should recover");

        assert_eq!(result.value, "sent");
        assert_eq!(result.attempts, 2);
        assert_eq!(calls.load(Ordering::SeqCst), 2);
    }

    #[tokio::test]
    async fn does_not_retry_ambiguous_timeout() {
        let calls = Arc::new(AtomicUsize::new(0));
        let seen = calls.clone();

        let err = retry_channel_delivery_with_sleep(
            "channel:telegram:42",
            || {
                let seen = seen.clone();
                async move {
                    seen.fetch_add(1, Ordering::SeqCst);
                    Err::<(), _>("ReadTimeout: request timed out".to_string())
                }
            },
            |_| async {},
        )
        .await
        .expect_err("ambiguous timeout must not retry");

        assert!(err.contains("channel:telegram:42"));
        assert!(err.contains("after 1 attempt"));
        assert_eq!(calls.load(Ordering::SeqCst), 1);
    }
}
