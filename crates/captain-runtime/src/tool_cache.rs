//! Per-tool result cache (v3.10f).
//!
//! Wraps a [`Cache`] backend and a table of per-tool TTLs so that
//! replaying the same `(tool_name, args)` within the TTL window returns
//! the previous result without re-executing. Tools that *write* state
//! — `file_write`, `memory_store`, `shell_exec`, … — bypass the cache
//! *and* invalidate the readers that might now be stale.
//!
//! Defaults mirror the values listed in `docs/v3.10-cache-efficiency.md`:
//!
//! | tool              | ttl   | notes                                 |
//! |-------------------|-------|---------------------------------------|
//! | `file_read`       | 60s   | local disk snapshot, bust on write    |
//! | `web_search`      | 300s  | SERP changes slowly, good dedupe ROI  |
//! | `web_fetch`       | 600s  | static pages; dynamic ones are stale  |
//! | `knowledge_query` | 60s   | index mutates via memory_store        |
//! | `memory_recall`   | 30s   | tighter — memories age fast           |
//! | everything else   | 0s    | no-cache by default                   |
//!
//! Integration with the agent loop is handled one layer up; this module
//! provides the pure-data primitives (key derivation, TTL lookup,
//! invalidation fan-out) so the integration is a tiny wrapper.

use crate::cache::{Cache, MokaCache, MokaCacheConfig};
use dashmap::DashMap;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::sync::{Arc, OnceLock};
use std::time::Duration;

/// Serializable envelope stored in the cache.
///
/// We keep `is_error` alongside the payload so a cached error doesn't
/// get replayed as if it were a successful response. Callers can
/// decide to skip caching errors by branching on it.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CachedToolResult {
    pub output: String,
    pub is_error: bool,
}

/// Tool classification: affects cacheability and invalidation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToolKind {
    /// Pure read — safe to cache, no side-effects.
    Read,
    /// Mutates state — never cached, triggers invalidation of readers.
    Write,
}

/// Per-tool policy used by the cache.
#[derive(Debug, Clone)]
pub struct ToolPolicy {
    pub ttl: Duration,
    pub kind: ToolKind,
    /// When this tool is a writer, which *readers* should be
    /// invalidated. Matching is on `tool_name` only: each reader namespace has
    /// a monotonically increasing version, so writes conservatively invalidate
    /// every cached read for that reader without needing to know the original
    /// read args.
    pub invalidates: Vec<String>,
}

impl ToolPolicy {
    fn read(ttl_secs: u64) -> Self {
        Self {
            ttl: Duration::from_secs(ttl_secs),
            kind: ToolKind::Read,
            invalidates: vec![],
        }
    }

    fn write(invalidates: impl IntoIterator<Item = &'static str>) -> Self {
        Self {
            ttl: Duration::ZERO,
            kind: ToolKind::Write,
            invalidates: invalidates.into_iter().map(str::to_string).collect(),
        }
    }
}

/// Shared policy table — the "what can I cache and for how long" map.
#[derive(Clone)]
pub struct ToolResultCache {
    cache: Arc<dyn Cache>,
    policies: HashMap<String, ToolPolicy>,
    key_prefix: String,
    versions: Arc<DashMap<String, u64>>,
}

impl ToolResultCache {
    pub fn new(cache: Arc<dyn Cache>) -> Self {
        Self {
            cache,
            policies: default_policies(),
            key_prefix: "tool:v1:".to_string(),
            versions: Arc::new(DashMap::new()),
        }
    }

    pub fn with_prefix(mut self, prefix: impl Into<String>) -> Self {
        self.key_prefix = prefix.into();
        self
    }

    /// Override or extend the default policy table. Unknown tools keep
    /// defaulting to `ToolKind::Read` with `ttl = 0s` (effectively
    /// no-cache) so accidentally-uncovered tools never silently stale.
    pub fn set_policy(&mut self, tool_name: impl Into<String>, policy: ToolPolicy) {
        self.policies.insert(tool_name.into(), policy);
    }

    /// Lookup the policy for a tool. Returns `None` for unknown tools —
    /// the wrapper should treat that as "don't cache".
    pub fn policy(&self, tool_name: &str) -> Option<&ToolPolicy> {
        self.policies.get(tool_name)
    }

    /// True when the tool is a read with a non-zero TTL (i.e. worth
    /// caching at all).
    pub fn is_cacheable(&self, tool_name: &str) -> bool {
        self.policies
            .get(tool_name)
            .is_some_and(|p| p.kind == ToolKind::Read && !p.ttl.is_zero())
    }

    /// Deterministic cache key for a `(tool_name, args)` pair.
    pub fn key_for(&self, tool_name: &str, args: &serde_json::Value) -> String {
        let bytes = serde_json::to_vec(args).unwrap_or_default();
        let digest = Sha256::digest(&bytes);
        let version = self.version_for(tool_name);
        format!(
            "{}{}:v{}:{}",
            self.key_prefix,
            tool_name,
            version,
            hex::encode(digest)
        )
    }

    fn version_for(&self, tool_name: &str) -> u64 {
        self.versions
            .get(tool_name)
            .map(|v| *v.value())
            .unwrap_or(0)
    }

    fn bump_version(&self, tool_name: &str) {
        self.versions
            .entry(tool_name.to_string())
            .and_modify(|v| *v = v.saturating_add(1))
            .or_insert(1);
    }

    /// Try to fetch a cached result. Returns `Ok(None)` on miss or when
    /// the tool is not cacheable.
    pub async fn get(
        &self,
        tool_name: &str,
        args: &serde_json::Value,
    ) -> anyhow::Result<Option<CachedToolResult>> {
        if !self.is_cacheable(tool_name) {
            return Ok(None);
        }
        let key = self.key_for(tool_name, args);
        let Some(raw) = self.cache.get(&key).await? else {
            return Ok(None);
        };
        match serde_json::from_str::<CachedToolResult>(&raw) {
            Ok(v) => Ok(Some(v)),
            Err(err) => {
                tracing::warn!("tool cache value malformed (invalidating): {err}");
                let _ = self.cache.invalidate(&key).await;
                Ok(None)
            }
        }
    }

    /// Store a result, but only when the tool is cacheable and the
    /// result is not an error (caching errors would defeat retries).
    pub async fn store(
        &self,
        tool_name: &str,
        args: &serde_json::Value,
        result: &CachedToolResult,
    ) -> anyhow::Result<()> {
        if result.is_error {
            return Ok(());
        }
        if !self.is_cacheable(tool_name) {
            return Ok(());
        }
        let key = self.key_for(tool_name, args);
        let payload = serde_json::to_string(result)
            .map_err(|e| anyhow::anyhow!("tool cache serialize: {e}"))?;
        self.cache.insert(key, payload).await
    }

    /// Called when a writer tool runs: invalidate downstream reader namespaces.
    ///
    /// `args_scope` is intentionally ignored for correctness. Writer args do
    /// not generally match reader args (`file_write` includes content,
    /// `memory_store` differs from `memory_recall`), so versioning the reader
    /// namespace is cheaper and safer than trying to delete exact stale keys.
    pub async fn invalidate_after_write(
        &self,
        writer_tool: &str,
        _args_scope: &serde_json::Value,
    ) -> anyhow::Result<()> {
        let Some(policy) = self.policies.get(writer_tool) else {
            return Ok(());
        };
        for reader in &policy.invalidates {
            self.bump_version(reader);
        }
        Ok(())
    }
}

/// Process-wide [`ToolResultCache`] singleton, gated on the
/// `CAPTAIN_TOOL_CACHE` environment variable. Returns `None` when
/// disabled so callers can skip the lookup entirely at zero cost.
static GLOBAL_TOOL_CACHE: OnceLock<Option<Arc<ToolResultCache>>> = OnceLock::new();

pub fn global_cache() -> Option<Arc<ToolResultCache>> {
    GLOBAL_TOOL_CACHE
        .get_or_init(|| {
            let enabled = std::env::var("CAPTAIN_TOOL_CACHE")
                .ok()
                .map(|v| {
                    let lc = v.trim().to_ascii_lowercase();
                    matches!(lc.as_str(), "1" | "true" | "yes" | "on")
                })
                .unwrap_or(false);
            if !enabled {
                return None;
            }
            let backend: Arc<dyn Cache> = Arc::new(MokaCache::new(MokaCacheConfig {
                max_capacity: 4_000,
                ttl: Duration::from_secs(600),
                tti: Duration::from_secs(300),
            }));
            tracing::info!("v3.10f tool result cache enabled (Moka, cap=4000, ttl=600s)");
            Some(Arc::new(ToolResultCache::new(backend)))
        })
        .clone()
}

fn default_policies() -> HashMap<String, ToolPolicy> {
    let mut m = HashMap::new();
    // Readers
    m.insert("file_read".into(), ToolPolicy::read(60));
    m.insert("document_extract".into(), ToolPolicy::read(60));
    m.insert("web_search".into(), ToolPolicy::read(300));
    m.insert("web_fetch".into(), ToolPolicy::read(600));
    m.insert("knowledge_query".into(), ToolPolicy::read(60));
    m.insert("memory_recall".into(), ToolPolicy::read(30));

    // Writers — invalidate their obvious reader counterparts.
    m.insert(
        "file_write".into(),
        ToolPolicy::write(["file_read", "document_extract"]),
    );
    m.insert(
        "web_download".into(),
        ToolPolicy::write(["file_read", "document_extract"]),
    );
    m.insert(
        "document_create".into(),
        ToolPolicy::write(["file_read", "document_extract"]),
    );
    m.insert(
        "memory_store".into(),
        ToolPolicy::write(["memory_recall", "knowledge_query"]),
    );
    m.insert(
        "memory_forget".into(),
        ToolPolicy::write(["memory_recall", "knowledge_query"]),
    );
    // shell_exec is a writer-by-default: commands can mutate the
    // filesystem, so we never cache and we bust file_read broadly on
    // execution.
    m.insert(
        "shell_exec".into(),
        ToolPolicy::write(["file_read", "document_extract"]),
    );
    // Browser tools mutate a live page/session and must never replay from cache.
    for tool in [
        "browser_batch",
        "browser_navigate",
        "browser_click",
        "browser_type",
        "browser_keys",
        "browser_select",
        "browser_hover",
        "browser_scroll",
        "browser_wait",
        "browser_run_js",
        "browser_back",
        "browser_close",
    ] {
        m.insert(
            tool.into(),
            ToolPolicy::write(std::iter::empty::<&'static str>()),
        );
    }
    m
}

#[cfg(test)]
#[path = "tool_cache_tests.rs"]
mod tests;
