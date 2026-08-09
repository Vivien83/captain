use crate::agent_loop_stream_delivery::StreamDeliveryBuffer;
use crate::auth_cooldown::{CooldownVerdict, ProviderCooldown};
use crate::llm_driver::{CompletionRequest, CompletionResponse, LlmDriver, LlmError, StreamEvent};
use crate::llm_errors;
use captain_types::error::{CaptainError, CaptainResult};
use tokio::sync::mpsc;
use tracing::{debug, warn};

/// Maximum retries for rate-limited or overloaded API calls.
pub(crate) const MAX_RETRIES: u32 = 3;

/// Base delay for exponential backoff (milliseconds).
pub(crate) const BASE_RETRY_DELAY_MS: u64 = 1000;

/// Call an LLM driver with automatic retry on rate-limit and overload errors.
///
/// Uses the `llm_errors` classifier for smart error handling and the
/// `ProviderCooldown` circuit breaker to prevent request storms.
pub(crate) async fn call_with_retry(
    driver: &dyn LlmDriver,
    request: CompletionRequest,
    provider: Option<&str>,
    cooldown: Option<&ProviderCooldown>,
) -> CaptainResult<CompletionResponse> {
    if let (Some(provider), Some(cooldown)) = (provider, cooldown) {
        match cooldown.check(provider) {
            CooldownVerdict::Reject {
                reason,
                retry_after_secs,
            } => {
                return Err(CaptainError::LlmDriver(format!(
                    "Provider '{provider}' is in cooldown ({reason}). Retry in {retry_after_secs}s."
                )));
            }
            CooldownVerdict::AllowProbe => {
                debug!(provider, "Allowing probe request through circuit breaker");
            }
            CooldownVerdict::Allow => {}
        }
    }

    let mut last_error = None;

    for attempt in 0..=MAX_RETRIES {
        match driver.complete(request.clone()).await {
            Ok(response) => {
                if let (Some(provider), Some(cooldown)) = (provider, cooldown) {
                    cooldown.record_success(provider);
                }
                return Ok(response);
            }
            Err(LlmError::SubscriptionQuotaExceeded { info }) => {
                warn!(
                    quota_code = %info.code,
                    provider = info.provider.as_deref().unwrap_or("unknown"),
                    resets_at = ?info.resets_at,
                    "Provider subscription quota exhausted; refusing automatic retry"
                );
                return Err(CaptainError::quota_exceeded(*info));
            }
            Err(LlmError::RateLimited { retry_after_ms }) => {
                if attempt == MAX_RETRIES {
                    if let (Some(provider), Some(cooldown)) = (provider, cooldown) {
                        cooldown.record_failure(provider, false);
                    }
                    return Err(CaptainError::LlmDriver(format!(
                        "Rate limited after {} retries",
                        MAX_RETRIES
                    )));
                }
                let delay = retry_delay_ms(retry_after_ms, attempt);
                warn!(
                    attempt,
                    delay_ms = delay,
                    "Rate limited, retrying after delay"
                );
                tokio::time::sleep(std::time::Duration::from_millis(delay)).await;
                last_error = Some("Rate limited".to_string());
            }
            Err(LlmError::Overloaded { retry_after_ms }) => {
                if attempt == MAX_RETRIES {
                    if let (Some(provider), Some(cooldown)) = (provider, cooldown) {
                        cooldown.record_failure(provider, false);
                    }
                    return Err(CaptainError::LlmDriver(format!(
                        "Model overloaded after {} retries",
                        MAX_RETRIES
                    )));
                }
                let delay = retry_delay_ms(retry_after_ms, attempt);
                warn!(
                    attempt,
                    delay_ms = delay,
                    "Model overloaded, retrying after delay"
                );
                tokio::time::sleep(std::time::Duration::from_millis(delay)).await;
                last_error = Some("Overloaded".to_string());
            }
            Err(e) => {
                let classified = classify_driver_error(&e);
                warn!(
                    category = ?classified.error.category,
                    retryable = classified.error.is_retryable,
                    raw = %classified.raw_error,
                    "LLM error classified: {}",
                    classified.error.sanitized_message
                );

                if let (Some(provider), Some(cooldown)) = (provider, cooldown) {
                    cooldown.record_failure(provider, classified.error.is_billing);
                }

                return Err(CaptainError::LlmDriver(classified.user_message));
            }
        }
    }

    Err(CaptainError::LlmDriver(
        last_error.unwrap_or_else(|| "Unknown error".to_string()),
    ))
}

/// Call an LLM driver in streaming mode with automatic retry on rate-limit and overload errors.
///
/// Uses the `llm_errors` classifier and `ProviderCooldown` circuit breaker.
pub(crate) async fn stream_with_retry(
    driver: &dyn LlmDriver,
    request: CompletionRequest,
    tx: mpsc::Sender<StreamEvent>,
    provider: Option<&str>,
    cooldown: Option<&ProviderCooldown>,
    mut held_delivery: Option<&mut StreamDeliveryBuffer>,
) -> CaptainResult<CompletionResponse> {
    if let (Some(provider), Some(cooldown)) = (provider, cooldown) {
        match cooldown.check(provider) {
            CooldownVerdict::Reject {
                reason,
                retry_after_secs,
            } => {
                return Err(CaptainError::LlmDriver(format!(
                    "Provider '{provider}' is in cooldown ({reason}). Retry in {retry_after_secs}s."
                )));
            }
            CooldownVerdict::AllowProbe => {
                debug!(
                    provider,
                    "Allowing probe request through circuit breaker (stream)"
                );
            }
            CooldownVerdict::Allow => {}
        }
    }

    let mut last_error = None;

    for attempt in 0..=MAX_RETRIES {
        let attempt_result = if held_delivery.is_some() {
            buffered_stream_attempt(driver, request.clone()).await
        } else {
            forwarded_stream_attempt(driver, request.clone(), &tx)
                .await
                .map(|response| (response, None))
        };
        match attempt_result {
            Ok((response, attempt_events)) => {
                if let Some(attempt_events) = attempt_events {
                    let Some(delivery) = held_delivery.as_deref_mut() else {
                        return Err(CaptainError::LlmDriver(
                            "buffered stream completed without a delivery transaction".to_string(),
                        ));
                    };
                    delivery.append(attempt_events)?;
                }
                if let (Some(provider), Some(cooldown)) = (provider, cooldown) {
                    cooldown.record_success(provider);
                }
                return Ok(response);
            }
            Err(StreamAttemptError::Delivery(error)) => return Err(error),
            Err(StreamAttemptError::Driver {
                error: LlmError::SubscriptionQuotaExceeded { info },
                ..
            }) => {
                warn!(
                    quota_code = %info.code,
                    provider = info.provider.as_deref().unwrap_or("unknown"),
                    resets_at = ?info.resets_at,
                    "Provider subscription quota exhausted; refusing automatic stream retry"
                );
                return Err(CaptainError::quota_exceeded(*info));
            }
            Err(StreamAttemptError::Driver {
                error: LlmError::RateLimited { retry_after_ms },
                events_emitted,
            }) => {
                if events_emitted {
                    if let (Some(provider), Some(cooldown)) = (provider, cooldown) {
                        cooldown.record_failure(provider, false);
                    }
                    return Err(committed_stream_retry_error("rate limited"));
                }
                if attempt == MAX_RETRIES {
                    if let (Some(provider), Some(cooldown)) = (provider, cooldown) {
                        cooldown.record_failure(provider, false);
                    }
                    return Err(CaptainError::LlmDriver(format!(
                        "Rate limited after {} retries",
                        MAX_RETRIES
                    )));
                }
                let delay = retry_delay_ms(retry_after_ms, attempt);
                warn!(
                    attempt,
                    delay_ms = delay,
                    "Rate limited (stream), retrying after delay"
                );
                tokio::time::sleep(std::time::Duration::from_millis(delay)).await;
                last_error = Some("Rate limited".to_string());
            }
            Err(StreamAttemptError::Driver {
                error: LlmError::Overloaded { retry_after_ms },
                events_emitted,
            }) => {
                if events_emitted {
                    if let (Some(provider), Some(cooldown)) = (provider, cooldown) {
                        cooldown.record_failure(provider, false);
                    }
                    return Err(committed_stream_retry_error("model overloaded"));
                }
                if attempt == MAX_RETRIES {
                    if let (Some(provider), Some(cooldown)) = (provider, cooldown) {
                        cooldown.record_failure(provider, false);
                    }
                    return Err(CaptainError::LlmDriver(format!(
                        "Model overloaded after {} retries",
                        MAX_RETRIES
                    )));
                }
                let delay = retry_delay_ms(retry_after_ms, attempt);
                warn!(
                    attempt,
                    delay_ms = delay,
                    "Model overloaded (stream), retrying after delay"
                );
                tokio::time::sleep(std::time::Duration::from_millis(delay)).await;
                last_error = Some("Overloaded".to_string());
            }
            Err(StreamAttemptError::Driver { error: e, .. }) => {
                let classified = classify_driver_error(&e);
                warn!(
                    category = ?classified.error.category,
                    retryable = classified.error.is_retryable,
                    raw = %classified.raw_error,
                    "LLM stream error classified: {}",
                    classified.error.sanitized_message
                );

                if let (Some(provider), Some(cooldown)) = (provider, cooldown) {
                    cooldown.record_failure(provider, classified.error.is_billing);
                }

                return Err(CaptainError::LlmDriver(classified.user_message));
            }
        }
    }

    Err(CaptainError::LlmDriver(
        last_error.unwrap_or_else(|| "Unknown error".to_string()),
    ))
}

enum StreamAttemptError {
    Driver {
        error: LlmError,
        events_emitted: bool,
    },
    Delivery(CaptainError),
}

fn committed_stream_retry_error(reason: &str) -> CaptainError {
    CaptainError::LlmDriver(format!(
        "Streaming request stopped after output began ({reason}); automatic retry was refused to prevent duplicate delivery."
    ))
}

async fn forwarded_stream_attempt(
    driver: &dyn LlmDriver,
    request: CompletionRequest,
    output_tx: &mpsc::Sender<StreamEvent>,
) -> Result<CompletionResponse, StreamAttemptError> {
    let (attempt_tx, mut attempt_rx) = mpsc::channel(64);
    let mut stream = Box::pin(driver.stream(request, attempt_tx));
    let mut events_emitted = false;

    loop {
        tokio::select! {
            result = &mut stream => {
                while let Ok(event) = attempt_rx.try_recv() {
                    events_emitted |= output_tx.send(event).await.is_ok();
                }
                return result.map_err(|error| StreamAttemptError::Driver {
                    error,
                    events_emitted,
                });
            }
            event = attempt_rx.recv() => {
                let Some(event) = event else {
                    return stream.await.map_err(|error| StreamAttemptError::Driver {
                        error,
                        events_emitted,
                    });
                };
                events_emitted |= output_tx.send(event).await.is_ok();
            }
        }
    }
}

async fn buffered_stream_attempt(
    driver: &dyn LlmDriver,
    request: CompletionRequest,
) -> Result<(CompletionResponse, Option<StreamDeliveryBuffer>), StreamAttemptError> {
    let (attempt_tx, mut attempt_rx) = mpsc::channel(64);
    let mut stream = Box::pin(driver.stream(request, attempt_tx));
    let mut events = StreamDeliveryBuffer::default();

    loop {
        tokio::select! {
            result = &mut stream => {
                while let Ok(event) = attempt_rx.try_recv() {
                    events.push(event).map_err(StreamAttemptError::Delivery)?;
                }
                return result
                    .map(|response| (response, Some(events)))
                    .map_err(|error| StreamAttemptError::Driver {
                        error,
                        events_emitted: false,
                    });
            }
            event = attempt_rx.recv() => {
                let Some(event) = event else {
                    let result = stream.await;
                    return result
                        .map(|response| (response, Some(events)))
                        .map_err(|error| StreamAttemptError::Driver {
                            error,
                            events_emitted: false,
                        });
                };
                events.push(event).map_err(StreamAttemptError::Delivery)?;
            }
        }
    }
}

fn retry_delay_ms(retry_after_ms: u64, attempt: u32) -> u64 {
    std::cmp::max(retry_after_ms, BASE_RETRY_DELAY_MS * 2u64.pow(attempt))
}

struct ClassifiedDriverError {
    error: llm_errors::ClassifiedError,
    raw_error: String,
    user_message: String,
}

fn classify_driver_error(error: &LlmError) -> ClassifiedDriverError {
    let raw_error = error.to_string();
    let status = match error {
        LlmError::Api { status, .. } => Some(*status),
        _ => None,
    };
    let classified = llm_errors::classify_error(&raw_error, status);
    let user_message = if classified.category == llm_errors::LlmErrorCategory::Format {
        format!("{} — raw: {}", classified.sanitized_message, raw_error)
    } else {
        classified.sanitized_message.clone()
    };

    ClassifiedDriverError {
        error: classified,
        raw_error,
        user_message,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use captain_types::message::{ContentBlock, StopReason, TokenUsage};
    use captain_types::quota::{QuotaExceededInfo, QuotaScope, QuotaUnit};
    use std::sync::atomic::{AtomicUsize, Ordering};

    struct SubscriptionQuotaDriver {
        calls: AtomicUsize,
    }

    struct RetryingStreamDriver {
        calls: AtomicUsize,
    }

    struct SilentRetryingStreamDriver {
        calls: AtomicUsize,
    }

    struct FailingStreamDriver;

    #[async_trait::async_trait]
    impl LlmDriver for SubscriptionQuotaDriver {
        async fn complete(
            &self,
            _request: CompletionRequest,
        ) -> Result<CompletionResponse, LlmError> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            Err(LlmError::SubscriptionQuotaExceeded {
                info: Box::new(QuotaExceededInfo {
                    code: "provider_subscription_quota".to_string(),
                    scope: QuotaScope::ProviderSubscription,
                    provider: Some("codex".to_string()),
                    agent_id: None,
                    used: 100.0,
                    limit: 100.0,
                    unit: QuotaUnit::Percent,
                    window_seconds: Some(18_000),
                    resets_at: None,
                    retry_after_seconds: Some(60),
                    message: "Codex subscription exhausted".to_string(),
                }),
            })
        }
    }

    #[async_trait::async_trait]
    impl LlmDriver for RetryingStreamDriver {
        async fn complete(
            &self,
            _request: CompletionRequest,
        ) -> Result<CompletionResponse, LlmError> {
            unreachable!("stream test driver only")
        }

        async fn stream(
            &self,
            _request: CompletionRequest,
            tx: mpsc::Sender<StreamEvent>,
        ) -> Result<CompletionResponse, LlmError> {
            let attempt = self.calls.fetch_add(1, Ordering::SeqCst);
            if attempt == 0 {
                tx.send(StreamEvent::TextDelta {
                    text: "discarded-attempt".to_string(),
                })
                .await
                .unwrap();
                return Err(LlmError::RateLimited { retry_after_ms: 0 });
            }

            tx.send(StreamEvent::TextDelta {
                text: "kept-attempt".to_string(),
            })
            .await
            .unwrap();
            tx.send(StreamEvent::ContentComplete {
                stop_reason: StopReason::EndTurn,
                usage: TokenUsage {
                    output_tokens: 2,
                    ..Default::default()
                },
            })
            .await
            .unwrap();
            Ok(text_response("kept-attempt"))
        }
    }

    #[async_trait::async_trait]
    impl LlmDriver for SilentRetryingStreamDriver {
        async fn complete(
            &self,
            _request: CompletionRequest,
        ) -> Result<CompletionResponse, LlmError> {
            unreachable!("stream test driver only")
        }

        async fn stream(
            &self,
            _request: CompletionRequest,
            tx: mpsc::Sender<StreamEvent>,
        ) -> Result<CompletionResponse, LlmError> {
            if self.calls.fetch_add(1, Ordering::SeqCst) == 0 {
                return Err(LlmError::RateLimited { retry_after_ms: 0 });
            }
            tx.send(StreamEvent::TextDelta {
                text: "success".to_string(),
            })
            .await
            .unwrap();
            tx.send(StreamEvent::ContentComplete {
                stop_reason: StopReason::EndTurn,
                usage: TokenUsage::default(),
            })
            .await
            .unwrap();
            Ok(text_response("success"))
        }
    }

    #[async_trait::async_trait]
    impl LlmDriver for FailingStreamDriver {
        async fn complete(
            &self,
            _request: CompletionRequest,
        ) -> Result<CompletionResponse, LlmError> {
            unreachable!("stream test driver only")
        }

        async fn stream(
            &self,
            _request: CompletionRequest,
            tx: mpsc::Sender<StreamEvent>,
        ) -> Result<CompletionResponse, LlmError> {
            tx.send(StreamEvent::TextDelta {
                text: "must-not-escape".to_string(),
            })
            .await
            .unwrap();
            Err(LlmError::Api {
                status: 500,
                message: "stream failed".to_string(),
            })
        }
    }

    fn request() -> CompletionRequest {
        CompletionRequest {
            model: "test".to_string(),
            messages: vec![],
            tools: vec![],
            max_tokens: 100,
            temperature: 0.0,
            system: None,
            thinking: None,
            reasoning_effort: None,
            tool_choice: None,
            cache_hints: crate::llm_driver::CacheHints::default(),
        }
    }

    fn text_response(text: &str) -> CompletionResponse {
        CompletionResponse {
            content: vec![ContentBlock::Text {
                text: text.to_string(),
                provider_metadata: None,
            }],
            stop_reason: StopReason::EndTurn,
            tool_calls: Vec::new(),
            usage: TokenUsage {
                output_tokens: 2,
                ..Default::default()
            },
        }
    }

    #[test]
    fn retry_constants_stay_stable() {
        assert_eq!(MAX_RETRIES, 3);
        assert_eq!(BASE_RETRY_DELAY_MS, 1000);
    }

    #[test]
    fn retry_delay_uses_exponential_floor_or_provider_hint() {
        assert_eq!(retry_delay_ms(0, 0), 1000);
        assert_eq!(retry_delay_ms(0, 2), 4000);
        assert_eq!(retry_delay_ms(7000, 1), 7000);
    }

    #[test]
    fn format_errors_keep_raw_detail_for_operator_debugging() {
        let classified = classify_driver_error(&LlmError::Api {
            status: 400,
            message: "missing messages field".to_string(),
        });

        assert_eq!(
            classified.error.category,
            llm_errors::LlmErrorCategory::Format
        );
        assert!(classified.user_message.contains("raw: API error (400)"));
    }

    #[tokio::test]
    async fn subscription_quota_is_returned_without_retry() {
        let driver = SubscriptionQuotaDriver {
            calls: AtomicUsize::new(0),
        };

        let result = call_with_retry(&driver, request(), Some("codex"), None).await;

        assert!(matches!(result, Err(CaptainError::QuotaExceeded(_))));
        assert_eq!(driver.calls.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn held_stream_discards_failed_attempt_and_releases_success_once() {
        let driver = RetryingStreamDriver {
            calls: AtomicUsize::new(0),
        };
        let (tx, mut rx) = mpsc::channel(8);
        let mut delivery = StreamDeliveryBuffer::default();
        let checkpoint = delivery.checkpoint();

        let response = stream_with_retry(
            &driver,
            request(),
            tx.clone(),
            Some("test"),
            None,
            Some(&mut delivery),
        )
        .await
        .unwrap();

        assert_eq!(response.text(), "kept-attempt");
        assert_eq!(driver.calls.load(Ordering::SeqCst), 2);
        assert!(matches!(
            rx.try_recv(),
            Err(tokio::sync::mpsc::error::TryRecvError::Empty)
        ));

        delivery
            .validate_segment(checkpoint, &response.text(), response.stop_reason)
            .unwrap();
        delivery.release(&tx).await.unwrap();
        drop(tx);
        let first = rx.recv().await.unwrap();
        let second = rx.recv().await.unwrap();
        assert!(rx.recv().await.is_none());
        assert!(matches!(first, StreamEvent::TextDelta { text } if text == "kept-attempt"));
        assert!(matches!(
            second,
            StreamEvent::ContentComplete {
                stop_reason: StopReason::EndTurn,
                ..
            }
        ));
    }

    #[tokio::test]
    async fn held_stream_failure_exposes_no_partial_event() {
        let (tx, mut rx) = mpsc::channel(4);
        let mut delivery = StreamDeliveryBuffer::default();

        let result = stream_with_retry(
            &FailingStreamDriver,
            request(),
            tx,
            Some("test"),
            None,
            Some(&mut delivery),
        )
        .await;

        assert!(result.is_err());
        assert!(delivery.is_empty());
        assert!(rx.recv().await.is_none());
    }

    #[tokio::test]
    async fn direct_stream_never_retries_after_committing_a_delta() {
        let driver = RetryingStreamDriver {
            calls: AtomicUsize::new(0),
        };
        let (tx, mut rx) = mpsc::channel(4);

        let result = stream_with_retry(&driver, request(), tx, Some("test"), None, None).await;

        assert!(result
            .unwrap_err()
            .to_string()
            .contains("prevent duplicate delivery"));
        assert_eq!(driver.calls.load(Ordering::SeqCst), 1);
        assert!(matches!(
            rx.recv().await,
            Some(StreamEvent::TextDelta { text }) if text == "discarded-attempt"
        ));
        assert!(rx.recv().await.is_none());
    }

    #[tokio::test]
    async fn direct_stream_still_retries_before_any_delta_is_committed() {
        let driver = SilentRetryingStreamDriver {
            calls: AtomicUsize::new(0),
        };
        let (tx, mut rx) = mpsc::channel(4);

        let response = stream_with_retry(&driver, request(), tx, Some("test"), None, None)
            .await
            .unwrap();

        assert_eq!(response.text(), "success");
        assert_eq!(driver.calls.load(Ordering::SeqCst), 2);
        assert!(matches!(
            rx.recv().await,
            Some(StreamEvent::TextDelta { text }) if text == "success"
        ));
        assert!(matches!(
            rx.recv().await,
            Some(StreamEvent::ContentComplete {
                stop_reason: StopReason::EndTurn,
                ..
            })
        ));
        assert!(rx.recv().await.is_none());
    }
}
