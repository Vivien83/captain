use super::*;
use crate::llm_driver::CompletionResponse;
use captain_types::message::{ContentBlock, StopReason, TokenUsage};

struct FailDriver;

#[async_trait]
impl LlmDriver for FailDriver {
    async fn complete(&self, _req: CompletionRequest) -> Result<CompletionResponse, LlmError> {
        Err(LlmError::Api {
            status: 500,
            message: "Internal error".to_string(),
        })
    }
}

struct OkDriver;

#[async_trait]
impl LlmDriver for OkDriver {
    async fn complete(&self, _req: CompletionRequest) -> Result<CompletionResponse, LlmError> {
        Ok(CompletionResponse {
            content: vec![ContentBlock::Text {
                text: "OK".to_string(),
                provider_metadata: None,
            }],
            stop_reason: StopReason::EndTurn,
            tool_calls: vec![],
            usage: TokenUsage {
                input_tokens: 10,
                output_tokens: 5,
                ..Default::default()
            },
        })
    }
}

struct AuthFailDriver;

#[async_trait]
impl LlmDriver for AuthFailDriver {
    async fn complete(&self, _req: CompletionRequest) -> Result<CompletionResponse, LlmError> {
        Err(LlmError::AuthenticationFailed("bad credential".to_string()))
    }

    async fn stream(
        &self,
        _req: CompletionRequest,
        _tx: tokio::sync::mpsc::Sender<StreamEvent>,
    ) -> Result<CompletionResponse, LlmError> {
        Err(LlmError::AuthenticationFailed("bad credential".to_string()))
    }
}

struct BadRequestDriver;

#[async_trait]
impl LlmDriver for BadRequestDriver {
    async fn complete(&self, _req: CompletionRequest) -> Result<CompletionResponse, LlmError> {
        Err(LlmError::Api {
            status: 400,
            message: "Unsupported parameter: max_output_tokens".to_string(),
        })
    }

    async fn stream(
        &self,
        _req: CompletionRequest,
        _tx: tokio::sync::mpsc::Sender<StreamEvent>,
    ) -> Result<CompletionResponse, LlmError> {
        Err(LlmError::Api {
            status: 400,
            message: "Unsupported parameter: max_output_tokens".to_string(),
        })
    }
}

fn test_request() -> CompletionRequest {
    CompletionRequest {
        model: "test".to_string(),
        messages: vec![],
        tools: vec![],
        max_tokens: 100,
        temperature: 0.0,
        system: None,
        thinking: None,
        tool_choice: None,
        cache_hints: crate::llm_driver::CacheHints::default(),
    }
}

#[tokio::test]
async fn test_fallback_primary_succeeds() {
    let driver = FallbackDriver::new(vec![
        Arc::new(OkDriver) as Arc<dyn LlmDriver>,
        Arc::new(FailDriver) as Arc<dyn LlmDriver>,
    ]);
    let result = driver.complete(test_request()).await;
    assert!(result.is_ok());
    assert_eq!(result.unwrap().text(), "OK");
}

#[tokio::test]
async fn test_fallback_primary_fails_secondary_succeeds() {
    let driver = FallbackDriver::new(vec![
        Arc::new(FailDriver) as Arc<dyn LlmDriver>,
        Arc::new(OkDriver) as Arc<dyn LlmDriver>,
    ]);
    let result = driver.complete(test_request()).await;
    assert!(result.is_ok());
}

#[tokio::test]
async fn test_fallback_all_fail() {
    let driver = FallbackDriver::new(vec![
        Arc::new(FailDriver) as Arc<dyn LlmDriver>,
        Arc::new(FailDriver) as Arc<dyn LlmDriver>,
    ]);
    let result = driver.complete(test_request()).await;
    assert!(result.is_err());
}

#[tokio::test]
async fn auth_failure_does_not_fall_through() {
    let driver = FallbackDriver::new(vec![
        Arc::new(AuthFailDriver) as Arc<dyn LlmDriver>,
        Arc::new(OkDriver) as Arc<dyn LlmDriver>,
    ]);
    let result = driver.complete(test_request()).await;
    assert!(matches!(result, Err(LlmError::AuthenticationFailed(_))));
}

#[tokio::test]
async fn stream_auth_failure_does_not_emit_fallback_notice() {
    let driver = FallbackDriver::with_models(vec![
        (
            Arc::new(AuthFailDriver) as Arc<dyn LlmDriver>,
            "primary".into(),
        ),
        (Arc::new(OkDriver) as Arc<dyn LlmDriver>, "fallback".into()),
    ]);
    let (tx, mut rx) = tokio::sync::mpsc::channel::<StreamEvent>(32);
    let result = driver.stream(test_request(), tx).await;
    assert!(matches!(result, Err(LlmError::AuthenticationFailed(_))));
    while let Ok(ev) = rx.try_recv() {
        assert!(
            !matches!(ev, StreamEvent::PhaseChange { phase, .. } if phase == "model_fallback"),
            "auth/config errors must not look like a normal fallback hop"
        );
    }
}

#[tokio::test]
async fn bad_request_does_not_fall_through() {
    let driver = FallbackDriver::new(vec![
        Arc::new(BadRequestDriver) as Arc<dyn LlmDriver>,
        Arc::new(OkDriver) as Arc<dyn LlmDriver>,
    ]);
    let result = driver.complete(test_request()).await;
    assert!(matches!(result, Err(LlmError::Api { status: 400, .. })));
}

#[tokio::test]
async fn stream_bad_request_does_not_emit_fallback_notice() {
    let driver = FallbackDriver::with_models(vec![
        (
            Arc::new(BadRequestDriver) as Arc<dyn LlmDriver>,
            "primary".into(),
        ),
        (Arc::new(OkDriver) as Arc<dyn LlmDriver>, "fallback".into()),
    ]);
    let (tx, mut rx) = tokio::sync::mpsc::channel::<StreamEvent>(32);
    let result = driver.stream(test_request(), tx).await;
    assert!(matches!(result, Err(LlmError::Api { status: 400, .. })));
    while let Ok(ev) = rx.try_recv() {
        assert!(
            !matches!(ev, StreamEvent::PhaseChange { phase, .. } if phase == "model_fallback"),
            "request-contract errors must not look like a normal fallback hop"
        );
    }
}

#[tokio::test]
async fn stream_emits_internal_phase_on_each_fallback_hop() {
    let driver = FallbackDriver::with_models(vec![
        (
            Arc::new(FailDriver) as Arc<dyn LlmDriver>,
            "primary-x".into(),
        ),
        (
            Arc::new(FailDriver) as Arc<dyn LlmDriver>,
            "fallback-y".into(),
        ),
        (
            Arc::new(OkDriver) as Arc<dyn LlmDriver>,
            "fallback-z".into(),
        ),
    ]);
    let (tx, mut rx) = tokio::sync::mpsc::channel::<StreamEvent>(32);
    let result = driver.stream(test_request(), tx).await;
    assert!(result.is_ok());

    let mut notices = Vec::new();
    while let Ok(ev) = rx.try_recv() {
        if let StreamEvent::PhaseChange { phase, detail } = ev {
            if phase == "model_fallback" {
                notices.push(detail.unwrap_or_default());
            }
        }
    }
    assert_eq!(
        notices.len(),
        2,
        "expected 2 internal fallback notices, got {}: {:?}",
        notices.len(),
        notices
    );
    assert!(notices[0].contains("fallback-y"));
    assert!(notices[0].contains("⚠️"));
    assert!(notices[1].contains("fallback-z"));
    assert!(!notices[0].contains("HTTP 500"));
    assert!(notices[0].contains("Reason: provider_server_error"));
    assert!(notices[0].contains("Timestamp:"));
    assert!(notices[0].contains("backup model"));
}

#[tokio::test]
async fn stream_no_intermediate_message_on_primary_success() {
    let driver = FallbackDriver::with_models(vec![
        (Arc::new(OkDriver) as Arc<dyn LlmDriver>, "primary".into()),
        (Arc::new(FailDriver) as Arc<dyn LlmDriver>, "unused".into()),
    ]);
    let (tx, mut rx) = tokio::sync::mpsc::channel::<StreamEvent>(32);
    let _ = driver.stream(test_request(), tx).await.unwrap();
    let mut had_notice = false;
    while let Ok(ev) = rx.try_recv() {
        if matches!(ev, StreamEvent::PhaseChange { phase, .. } if phase == "model_fallback") {
            had_notice = true;
        }
    }
    assert!(!had_notice, "no fallback should not produce a notice");
}

#[tokio::test]
async fn stream_notice_uses_french_template_when_lang_fr() {
    let driver = FallbackDriver::with_models(vec![
        (
            Arc::new(FailDriver) as Arc<dyn LlmDriver>,
            "primary-model".into(),
        ),
        (
            Arc::new(OkDriver) as Arc<dyn LlmDriver>,
            "fallback-model".into(),
        ),
    ])
    .with_notice_template(notice_template_for("fr"));
    let (tx, mut rx) = tokio::sync::mpsc::channel::<StreamEvent>(32);
    let _ = driver.stream(test_request(), tx).await.unwrap();
    let mut found = false;
    while let Ok(ev) = rx.try_recv() {
        if let StreamEvent::PhaseChange { phase, detail } = ev {
            if phase != "model_fallback" {
                continue;
            }
            let content = detail.unwrap_or_default();
            assert!(
                content.contains("modèle de secours"),
                "FR template should mention continuity and explicit fallback metadata: {content}"
            );
            assert!(content.contains("Cible: fallback-model"));
            assert!(content.contains("Raison: erreur_serveur_provider"));
            assert!(content.contains("Horodatage:"));
            found = true;
            break;
        }
    }
    assert!(found, "expected at least one fallback notice");
}

#[tokio::test]
async fn stream_notice_uses_english_template_by_default() {
    let driver = FallbackDriver::with_models(vec![
        (
            Arc::new(FailDriver) as Arc<dyn LlmDriver>,
            "primary-model".into(),
        ),
        (
            Arc::new(OkDriver) as Arc<dyn LlmDriver>,
            "fallback-model".into(),
        ),
    ]);
    let (tx, mut rx) = tokio::sync::mpsc::channel::<StreamEvent>(32);
    let _ = driver.stream(test_request(), tx).await.unwrap();
    let mut found = false;
    while let Ok(ev) = rx.try_recv() {
        if let StreamEvent::PhaseChange { phase, detail } = ev {
            if phase != "model_fallback" {
                continue;
            }
            let content = detail.unwrap_or_default();
            assert!(
                content.contains("backup model"),
                "EN template default: {content}"
            );
            assert!(!content.contains("Fallback engaged"));
            assert!(content.contains("Target: fallback-model"));
            assert!(content.contains("Reason: provider_server_error"));
            assert!(content.contains("Timestamp:"));
            found = true;
            break;
        }
    }
    assert!(found);
}

#[test]
fn notice_template_for_recognizes_french_variants() {
    for tag in ["fr", "FR", "fr-FR", "fr_fr", "francais", "Français"] {
        assert_eq!(
            notice_template_for(tag),
            FALLBACK_NOTICE_TEMPLATE_FR,
            "tag={tag}"
        );
    }
    for tag in ["en", "de", "ja", ""] {
        assert_eq!(
            notice_template_for(tag),
            DEFAULT_FALLBACK_NOTICE_TEMPLATE,
            "tag={tag}"
        );
    }
}

#[tokio::test]
async fn test_rate_limit_falls_through() {
    struct RateLimitDriver;

    #[async_trait]
    impl LlmDriver for RateLimitDriver {
        async fn complete(&self, _req: CompletionRequest) -> Result<CompletionResponse, LlmError> {
            Err(LlmError::RateLimited {
                retry_after_ms: 5000,
            })
        }
    }

    let driver = FallbackDriver::new(vec![
        Arc::new(RateLimitDriver) as Arc<dyn LlmDriver>,
        Arc::new(OkDriver) as Arc<dyn LlmDriver>,
    ]);
    let result = driver.complete(test_request()).await;
    assert!(result.is_ok());
    assert_eq!(result.unwrap().text(), "OK");
}

#[tokio::test]
async fn test_rate_limit_all_fail() {
    struct RateLimitDriver;

    #[async_trait]
    impl LlmDriver for RateLimitDriver {
        async fn complete(&self, _req: CompletionRequest) -> Result<CompletionResponse, LlmError> {
            Err(LlmError::RateLimited {
                retry_after_ms: 5000,
            })
        }
    }

    let driver = FallbackDriver::new(vec![
        Arc::new(RateLimitDriver) as Arc<dyn LlmDriver>,
        Arc::new(RateLimitDriver) as Arc<dyn LlmDriver>,
    ]);
    let result = driver.complete(test_request()).await;
    assert!(matches!(result, Err(LlmError::RateLimited { .. })));
}
