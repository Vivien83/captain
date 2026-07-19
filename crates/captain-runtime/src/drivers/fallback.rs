//! Fallback driver — tries multiple LLM drivers in sequence.
//!
//! If the primary driver fails with a non-retryable error, the fallback driver
//! moves to the next driver in the chain.

use crate::llm_driver::{CompletionRequest, CompletionResponse, LlmDriver, LlmError, StreamEvent};
use async_trait::async_trait;
use chrono::Utc;
use std::sync::Arc;
use tracing::warn;

/// Notice template fed to stream metadata when a fallback hop happens.
///
/// The notice must stay product-facing and explicit: the visible stream also
/// includes the target provider/model label, reason class, and timestamp so a
/// user never experiences an unexplained model change.
pub const DEFAULT_FALLBACK_NOTICE_TEMPLATE: &str =
    "⚠️ The primary model is temporarily unavailable; I am continuing with a backup model.";

/// French equivalent of [`DEFAULT_FALLBACK_NOTICE_TEMPLATE`].
pub const FALLBACK_NOTICE_TEMPLATE_FR: &str =
    "⚠️ Le modèle principal est temporairement indisponible ; je continue avec un modèle de secours.";

/// Pick the right notice template for the configured language. Defaults
/// to the English template for any unrecognized lang tag — the visible
/// emoji + arrow keep the meaning obvious even if the prose is wrong.
pub fn notice_template_for(lang: &str) -> &'static str {
    match lang.trim().to_lowercase().as_str() {
        "fr" | "fr-fr" | "fr_fr" | "french" | "francais" | "français" => {
            FALLBACK_NOTICE_TEMPLATE_FR
        }
        _ => DEFAULT_FALLBACK_NOTICE_TEMPLATE,
    }
}

struct FallbackTarget {
    driver: Arc<dyn LlmDriver>,
    model_name: String,
    display_label: String,
}

/// A driver that wraps multiple LLM drivers and tries each in order.
///
/// On failure (including rate-limit and overload), moves to the next driver.
/// Only returns an error when ALL drivers in the chain are exhausted.
/// Each driver is paired with the model name it should use and a public label
/// shown when a real fallback hop occurs.
pub struct FallbackDriver {
    drivers: Vec<FallbackTarget>,
    notice_template: String,
}

impl FallbackDriver {
    /// Create a new fallback driver from an ordered chain of (driver, model_name) pairs.
    ///
    /// The first entry is the primary; subsequent are fallbacks.
    pub fn new(drivers: Vec<Arc<dyn LlmDriver>>) -> Self {
        Self {
            drivers: drivers
                .into_iter()
                .enumerate()
                .map(|(index, driver)| FallbackTarget {
                    driver,
                    model_name: String::new(),
                    display_label: if index == 0 {
                        "primary model".to_string()
                    } else {
                        format!("fallback #{index}")
                    },
                })
                .collect(),
            notice_template: DEFAULT_FALLBACK_NOTICE_TEMPLATE.to_string(),
        }
    }

    /// Create a new fallback driver with explicit model names for each driver.
    pub fn with_models(drivers: Vec<(Arc<dyn LlmDriver>, String)>) -> Self {
        Self::with_targets(
            drivers
                .into_iter()
                .enumerate()
                .map(|(index, (driver, model_name))| {
                    let display = if model_name.trim().is_empty() {
                        if index == 0 {
                            "primary model".to_string()
                        } else {
                            format!("fallback #{index}")
                        }
                    } else {
                        model_name.clone()
                    };
                    (driver, model_name, display)
                })
                .collect(),
        )
    }

    /// Create a fallback driver with separate request model names and public
    /// display labels. Use this when a provider/model label differs from the
    /// model id sent to the provider.
    pub fn with_targets(drivers: Vec<(Arc<dyn LlmDriver>, String, String)>) -> Self {
        Self {
            drivers: drivers
                .into_iter()
                .map(|(driver, model_name, display_label)| FallbackTarget {
                    driver,
                    model_name,
                    display_label,
                })
                .collect(),
            notice_template: DEFAULT_FALLBACK_NOTICE_TEMPLATE.to_string(),
        }
    }

    /// Replace the localized notice template. Used by the kernel to
    /// match `config.language`. Builder-style (returns self).
    pub fn with_notice_template(mut self, template: impl Into<String>) -> Self {
        self.notice_template = template.into();
        self
    }
}

fn request_for_target(request: &CompletionRequest, target: &FallbackTarget) -> CompletionRequest {
    let mut req = request.clone();
    if !target.model_name.is_empty() {
        req.model = target.model_name.clone();
    }
    req
}

fn is_fallback_terminal_error(error: &LlmError) -> bool {
    matches!(
        error,
        LlmError::MissingApiKey(_)
            | LlmError::AuthenticationFailed(_)
            | LlmError::ModelNotFound(_)
            | LlmError::SubscriptionQuotaExceeded { .. }
    ) || matches!(error, LlmError::Api { status, .. } if is_client_request_error(*status))
}

fn log_fallback_error(index: usize, target: &FallbackTarget, error: &LlmError, streaming: bool) {
    let mode = if streaming { "stream" } else { "complete" };
    let message = if matches!(
        error,
        LlmError::MissingApiKey(_)
            | LlmError::AuthenticationFailed(_)
            | LlmError::ModelNotFound(_)
            | LlmError::SubscriptionQuotaExceeded { .. }
    ) {
        "Driver authentication/configuration failed; refusing silent fallback"
    } else if matches!(error, LlmError::Api { status, .. } if is_client_request_error(*status)) {
        "Driver request contract failed; refusing silent fallback"
    } else if matches!(
        error,
        LlmError::RateLimited { .. } | LlmError::Overloaded { .. }
    ) {
        "Driver rate-limited/overloaded, trying next fallback"
    } else {
        "Fallback driver failed, trying next"
    };
    warn!(
        driver_index = index,
        model = %target.display_label,
        mode = mode,
        error = %error,
        "{message}"
    );
}

#[async_trait]
impl LlmDriver for FallbackDriver {
    async fn complete(&self, request: CompletionRequest) -> Result<CompletionResponse, LlmError> {
        let mut last_error = None;

        for (i, target) in self.drivers.iter().enumerate() {
            let req = request_for_target(&request, target);
            match target.driver.complete(req).await {
                Ok(response) => return Ok(response),
                Err(e) => {
                    log_fallback_error(i, target, &e, false);
                    if is_fallback_terminal_error(&e) {
                        return Err(e);
                    }
                    last_error = Some(e);
                }
            }
        }

        Err(last_error.unwrap_or_else(|| LlmError::Api {
            status: 0,
            message: "No drivers configured in fallback chain".to_string(),
        }))
    }

    async fn stream(
        &self,
        request: CompletionRequest,
        tx: tokio::sync::mpsc::Sender<StreamEvent>,
    ) -> Result<CompletionResponse, LlmError> {
        let mut last_error = None;

        for (i, target) in self.drivers.iter().enumerate() {
            let req = request_for_target(&request, target);

            // R-fallback-UX — when we move to a fallback (i >= 1), emit an
            // explicit phase event. The target label, reason class, and
            // timestamp are intentionally visible so fallback cannot be silent.
            if i > 0 {
                let notice =
                    fallback_notice(&self.notice_template, &target.display_label, &last_error);
                let _ = tx
                    .send(StreamEvent::PhaseChange {
                        phase: "model_fallback".to_string(),
                        detail: Some(notice),
                    })
                    .await;
            }

            match target.driver.stream(req, tx.clone()).await {
                Ok(response) => return Ok(response),
                Err(e) => {
                    log_fallback_error(i, target, &e, true);
                    if is_fallback_terminal_error(&e) {
                        return Err(e);
                    }
                    last_error = Some(e);
                }
            }
        }

        Err(last_error.unwrap_or_else(|| LlmError::Api {
            status: 0,
            message: "No drivers configured in fallback chain".to_string(),
        }))
    }
}

fn is_client_request_error(status: u16) -> bool {
    matches!(status, 400 | 422)
}

fn fallback_notice(template: &str, target: &str, error: &Option<LlmError>) -> String {
    let french = template == FALLBACK_NOTICE_TEMPLATE_FR;
    let (target_label, reason_label, timestamp_label) = if french {
        ("Cible", "Raison", "Horodatage")
    } else {
        ("Target", "Reason", "Timestamp")
    };
    format!(
        "{} {}: {}. {}: {}. {}: {}.",
        template,
        target_label,
        target,
        reason_label,
        fallback_reason(error, french),
        timestamp_label,
        Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true),
    )
}

fn fallback_reason(error: &Option<LlmError>, french: bool) -> &'static str {
    match (error, french) {
        (Some(LlmError::RateLimited { .. }), true) => "limite_debit",
        (Some(LlmError::RateLimited { .. }), false) => "rate_limited",
        (Some(LlmError::Overloaded { .. }), true) => "surcharge",
        (Some(LlmError::Overloaded { .. }), false) => "overloaded",
        (Some(LlmError::Http(_)), true) => "erreur_reseau_http",
        (Some(LlmError::Http(_)), false) => "network_or_http_error",
        (Some(LlmError::Api { status, .. }), true) if *status >= 500 => "erreur_serveur_provider",
        (Some(LlmError::Api { status, .. }), false) if *status >= 500 => "provider_server_error",
        (Some(LlmError::Api { .. }), true) => "erreur_api_provider",
        (Some(LlmError::Api { .. }), false) => "provider_api_error",
        (Some(_), true) => "erreur_driver",
        (Some(_), false) => "driver_error",
        (None, true) => "inconnue",
        (None, false) => "unknown",
    }
}

#[cfg(test)]
#[path = "fallback_tests.rs"]
mod tests;
