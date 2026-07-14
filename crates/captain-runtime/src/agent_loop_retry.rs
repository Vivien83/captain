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
        match driver.stream(request.clone(), tx.clone()).await {
            Ok(response) => {
                if let (Some(provider), Some(cooldown)) = (provider, cooldown) {
                    cooldown.record_success(provider);
                }
                return Ok(response);
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
                    "Rate limited (stream), retrying after delay"
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
                    "Model overloaded (stream), retrying after delay"
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

    #[test]
    fn retry_constants_match_hermes_contract() {
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
}
