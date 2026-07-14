//! Transparent response cache for any [`LlmDriver`] (v3.10e).
//!
//! `CachedLlmDriver` wraps an inner driver and a [`Cache`] backend. On
//! `complete`, it canonicalizes the request into a stable hash key and
//! looks the key up; on miss it calls the inner driver and stores the
//! deserialized response.
//!
//! The cache is *bypassed entirely* when `temperature > 0.2`: sampling
//! at higher temperatures is supposed to vary, so reusing a cached
//! answer would defeat the user's explicit request for diversity.
//! Streaming calls (`stream`) are also passed straight through — the
//! cache targets the one-shot `complete` path that dominates agent
//! loops' reasoning turns.

use crate::cache::{Cache, MokaCache, MokaCacheConfig};
use crate::llm_driver::{CompletionRequest, CompletionResponse, LlmDriver, LlmError, StreamEvent};
use async_trait::async_trait;
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::sync::{Arc, OnceLock};
use std::time::Duration;
use tokio::sync::mpsc;

/// Temperatures above this are treated as "user wants variety" and
/// skip the cache entirely. 0.2 is the Anthropic-recommended cutoff
/// for deterministic-ish completions.
const CACHE_TEMPERATURE_CEILING: f32 = 0.2;

/// Process-wide cache singleton (v3.10e integration).
///
/// Initialized lazily on first access when the `CAPTAIN_LLM_CACHE`
/// environment variable is truthy (`1`, `true`, `yes`, case-insensitive).
/// Returns `None` when disabled so callers can skip the wrapper
/// altogether without branching at every call site.
///
/// A Moka-only cache is used for now — fast, zero-infra, survives the
/// daemon lifetime. Upgrading to HybridCache (Moka + Redb) is a one-line
/// change here when persistence-across-restarts becomes the priority.
static GLOBAL_LLM_CACHE: OnceLock<Option<Arc<dyn Cache>>> = OnceLock::new();

pub fn global_cache() -> Option<Arc<dyn Cache>> {
    GLOBAL_LLM_CACHE
        .get_or_init(|| {
            let enabled = std::env::var("CAPTAIN_LLM_CACHE")
                .ok()
                .map(|v| {
                    let lc = v.trim().to_ascii_lowercase();
                    matches!(lc.as_str(), "1" | "true" | "yes" | "on")
                })
                .unwrap_or(false);
            if !enabled {
                return None;
            }
            let cache = Arc::new(MokaCache::new(MokaCacheConfig {
                max_capacity: 2_000,
                ttl: Duration::from_secs(3_600),
                tti: Duration::from_secs(1_800),
            })) as Arc<dyn Cache>;
            tracing::info!("v3.10e LLM response cache enabled (Moka, cap=2000, ttl=1h)");
            Some(cache)
        })
        .clone()
}

/// Wrapper that sits between the agent loop and a real LLM driver and
/// dedupes identical `complete()` calls through any [`Cache`] backend.
pub struct CachedLlmDriver {
    inner: Arc<dyn LlmDriver>,
    cache: Arc<dyn Cache>,
    /// Static prefix so the same cache can store multiple logical
    /// layers (LLM responses vs tool results) without key collisions.
    key_prefix: String,
}

impl CachedLlmDriver {
    pub fn new(inner: Arc<dyn LlmDriver>, cache: Arc<dyn Cache>) -> Self {
        Self {
            inner,
            cache,
            key_prefix: "llm:v1:".to_string(),
        }
    }

    /// Overrides the default `"llm:v1:"` prefix. Useful for tests and
    /// for running two independent caches against the same Redis DB.
    pub fn with_prefix(mut self, prefix: impl Into<String>) -> Self {
        self.key_prefix = prefix.into();
        self
    }

    /// Build the cache key for a completion request. Exposed for tests
    /// so callers can assert stability across reorderings.
    pub fn hash_request(request: &CompletionRequest) -> String {
        let canon = CanonicalRequest::from(request);
        let bytes = serde_json::to_vec(&canon).unwrap_or_default();
        let digest = Sha256::digest(&bytes);
        hex::encode(digest)
    }

    fn key_for(&self, request: &CompletionRequest) -> String {
        format!("{}{}", self.key_prefix, Self::hash_request(request))
    }

    fn should_cache(request: &CompletionRequest) -> bool {
        request.temperature <= CACHE_TEMPERATURE_CEILING
    }
}

#[async_trait]
impl LlmDriver for CachedLlmDriver {
    async fn complete(&self, request: CompletionRequest) -> Result<CompletionResponse, LlmError> {
        if !Self::should_cache(&request) {
            return self.inner.complete(request).await;
        }

        let key = self.key_for(&request);

        // Cache hit — log.warn on any cache backend glitch, fall through
        // to the real call. A broken cache must never break the turn.
        match self.cache.get(&key).await {
            Ok(Some(cached)) => match serde_json::from_str::<CompletionResponse>(&cached) {
                Ok(resp) => {
                    tracing::debug!(key = %key, "llm cache hit");
                    return Ok(resp);
                }
                Err(err) => {
                    tracing::warn!("llm cache value malformed (will refetch): {err}");
                    let _ = self.cache.invalidate(&key).await;
                }
            },
            Ok(None) => {}
            Err(err) => {
                tracing::warn!("llm cache read failed (degrading to direct call): {err}");
            }
        }

        let resp = self.inner.complete(request).await?;

        if let Ok(serialized) = serde_json::to_string(&resp) {
            if let Err(err) = self.cache.insert(key, serialized).await {
                tracing::warn!("llm cache write failed: {err}");
            }
        }

        Ok(resp)
    }

    async fn stream(
        &self,
        request: CompletionRequest,
        tx: mpsc::Sender<StreamEvent>,
    ) -> Result<CompletionResponse, LlmError> {
        // Streaming is pass-through: the caller wanted incremental
        // events, replaying them from cache would be a different UX.
        self.inner.stream(request, tx).await
    }
}

// ---------------------------------------------------------------------------
// Canonical request representation (stable ordering for hashing)
// ---------------------------------------------------------------------------

/// Serializable projection of the request fields that actually affect
/// the output. Ordering matters — serde_json encodes fields in
/// declaration order so the hash is stable across runs.
#[derive(Serialize)]
struct CanonicalRequest<'a> {
    model: &'a str,
    system: Option<&'a str>,
    messages: &'a [captain_types::message::Message],
    tools: &'a [captain_types::tool::ToolDefinition],
    max_tokens: u32,
    /// Rounded to 2 decimals so `0.1999` and `0.2001` don't produce
    /// different keys due to floating-point drift.
    temperature_x100: i32,
    tool_choice: Option<&'a serde_json::Value>,
}

impl<'a> From<&'a CompletionRequest> for CanonicalRequest<'a> {
    fn from(r: &'a CompletionRequest) -> Self {
        Self {
            model: &r.model,
            system: r.system.as_deref(),
            messages: &r.messages,
            tools: &r.tools,
            max_tokens: r.max_tokens,
            temperature_x100: (r.temperature * 100.0).round() as i32,
            tool_choice: r.tool_choice.as_ref(),
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cache::{MokaCache, MokaCacheConfig};
    use captain_types::message::{
        ContentBlock, Message, MessageContent, Role, StopReason, TokenUsage,
    };
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::time::Duration;

    fn text_block(s: &str) -> ContentBlock {
        ContentBlock::Text {
            text: s.to_string(),
            provider_metadata: None,
        }
    }

    struct CountingDriver {
        calls: AtomicUsize,
        response: CompletionResponse,
    }
    impl CountingDriver {
        fn new(text: &str) -> Arc<Self> {
            Arc::new(Self {
                calls: AtomicUsize::new(0),
                response: CompletionResponse {
                    content: vec![text_block(text)],
                    stop_reason: StopReason::EndTurn,
                    tool_calls: vec![],
                    usage: TokenUsage::default(),
                },
            })
        }
        fn call_count(&self) -> usize {
            self.calls.load(Ordering::SeqCst)
        }
    }
    #[async_trait]
    impl LlmDriver for CountingDriver {
        async fn complete(
            &self,
            _request: CompletionRequest,
        ) -> Result<CompletionResponse, LlmError> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            Ok(self.response.clone())
        }
    }

    fn base_request() -> CompletionRequest {
        CompletionRequest {
            model: "claude-test".into(),
            messages: vec![Message {
                role: Role::User,
                content: MessageContent::Blocks(vec![text_block("hi")]),
            }],
            tools: vec![],
            max_tokens: 256,
            temperature: 0.0,
            system: Some("You are helpful".into()),
            thinking: None,
            tool_choice: None,
            cache_hints: crate::llm_driver::CacheHints::default(),
        }
    }

    fn fresh_cache() -> Arc<MokaCache> {
        Arc::new(MokaCache::new(MokaCacheConfig {
            max_capacity: 32,
            ttl: Duration::from_secs(60),
            tti: Duration::from_secs(60),
        }))
    }

    #[tokio::test]
    async fn identical_requests_hit_cache_after_first_call() {
        let driver = CountingDriver::new("hello");
        let cached = CachedLlmDriver::new(driver.clone(), fresh_cache());

        let _ = cached.complete(base_request()).await.unwrap();
        let _ = cached.complete(base_request()).await.unwrap();
        let _ = cached.complete(base_request()).await.unwrap();

        assert_eq!(driver.call_count(), 1);
    }

    #[tokio::test]
    async fn different_messages_produce_different_keys() {
        let driver = CountingDriver::new("hello");
        let cached = CachedLlmDriver::new(driver.clone(), fresh_cache());

        let mut req_a = base_request();
        let mut req_b = base_request();
        req_b.messages[0].content = MessageContent::Blocks(vec![text_block("ciao")]);

        let _ = cached.complete(req_a.clone()).await.unwrap();
        let _ = cached.complete(req_b).await.unwrap();
        // Same content again = hit
        req_a.messages[0].content = MessageContent::Blocks(vec![text_block("hi")]);
        let _ = cached.complete(req_a).await.unwrap();

        assert_eq!(driver.call_count(), 2);
    }

    #[tokio::test]
    async fn high_temperature_bypasses_cache() {
        let driver = CountingDriver::new("hello");
        let cached = CachedLlmDriver::new(driver.clone(), fresh_cache());

        let mut req = base_request();
        req.temperature = 0.7;
        for _ in 0..3 {
            let _ = cached.complete(req.clone()).await.unwrap();
        }
        assert_eq!(driver.call_count(), 3);
    }

    #[tokio::test]
    async fn boundary_temperature_0_2_still_caches() {
        let driver = CountingDriver::new("hello");
        let cached = CachedLlmDriver::new(driver.clone(), fresh_cache());

        let mut req = base_request();
        req.temperature = 0.2;
        let _ = cached.complete(req.clone()).await.unwrap();
        let _ = cached.complete(req).await.unwrap();
        assert_eq!(driver.call_count(), 1);
    }

    #[tokio::test]
    async fn hash_is_stable_for_equivalent_requests() {
        let a = CachedLlmDriver::hash_request(&base_request());
        let b = CachedLlmDriver::hash_request(&base_request());
        assert_eq!(a, b);
        assert_eq!(a.len(), 64, "sha256 hex is 64 chars");
    }
}
