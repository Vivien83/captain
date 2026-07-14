//! Generic async cache primitives (v3.10).
//!
//! Defines a minimal [`Cache`] trait so LLM responses, tool results, and
//! other expensive computations can share a common caching contract.
//!
//! ## Layers
//!
//! - [`MokaCache`] — hot in-memory layer backed by `moka::future::Cache`,
//!   microsecond-scale reads with capacity eviction + TTL + time-to-idle.
//! - [`RedbCache`] *(v3.10b)* — persistent layer surviving daemon restart.
//! - [`HybridCache`] *(v3.10c)* — Moka-then-Redb read-through.
//! - [`RedisCache`] *(v3.10d)* — optional distributed backend behind
//!   the `redis-cache` feature flag.
//!
//! The trait is intentionally string-keyed + string-valued: callers are
//! expected to serialize structured data (`serde_json`) before inserting.
//! This keeps the storage backends decoupled from caller-specific schemas
//! and makes `RedbCache` / `RedisCache` trivial to implement.

use async_trait::async_trait;
use moka::future::Cache as MokaFutureCache;
use redb::{Database, ReadableTable, ReadableTableMetadata, TableDefinition};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tokio::task;

// ---------------------------------------------------------------------------
// Trait
// ---------------------------------------------------------------------------

/// Generic async cache with string keys and string values.
///
/// Implementations must be `Send + Sync` so they can be shared across
/// runtime tasks. Errors are returned as `anyhow::Error` to keep the
/// contract backend-agnostic (each backend has its own error types).
#[async_trait]
pub trait Cache: Send + Sync {
    /// Fetch a value by key. Returns `Ok(None)` on miss or expiry.
    async fn get(&self, key: &str) -> anyhow::Result<Option<String>>;

    /// Insert a value, overwriting any previous value for the same key.
    async fn insert(&self, key: String, value: String) -> anyhow::Result<()>;

    /// Remove a key. No-op if absent.
    async fn invalidate(&self, key: &str) -> anyhow::Result<()>;

    /// Remove every entry in the cache.
    async fn clear(&self) -> anyhow::Result<()>;
}

// ---------------------------------------------------------------------------
// Metrics
// ---------------------------------------------------------------------------

/// Atomic counters for cache observability.
///
/// Exposed through [`MokaCache::metrics`] so tests and dashboards can
/// assert hit ratio without reaching into backend internals.
#[derive(Debug, Default)]
pub struct CacheMetrics {
    hits: AtomicU64,
    misses: AtomicU64,
    inserts: AtomicU64,
    invalidations: AtomicU64,
}

impl CacheMetrics {
    /// Snapshot of the counters at call time.
    pub fn snapshot(&self) -> CacheMetricsSnapshot {
        CacheMetricsSnapshot {
            hits: self.hits.load(Ordering::Relaxed),
            misses: self.misses.load(Ordering::Relaxed),
            inserts: self.inserts.load(Ordering::Relaxed),
            invalidations: self.invalidations.load(Ordering::Relaxed),
        }
    }

    fn record_hit(&self) {
        self.hits.fetch_add(1, Ordering::Relaxed);
    }

    fn record_miss(&self) {
        self.misses.fetch_add(1, Ordering::Relaxed);
    }

    fn record_insert(&self) {
        self.inserts.fetch_add(1, Ordering::Relaxed);
    }

    fn record_invalidation(&self) {
        self.invalidations.fetch_add(1, Ordering::Relaxed);
    }
}

/// Immutable snapshot returned by [`CacheMetrics::snapshot`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CacheMetricsSnapshot {
    pub hits: u64,
    pub misses: u64,
    pub inserts: u64,
    pub invalidations: u64,
}

impl CacheMetricsSnapshot {
    /// Ratio of hits over total reads (hits + misses). Returns 0.0 if no
    /// reads have been recorded yet (avoids NaN from 0/0).
    pub fn hit_ratio(&self) -> f64 {
        let total = self.hits + self.misses;
        if total == 0 {
            0.0
        } else {
            self.hits as f64 / total as f64
        }
    }
}

// ---------------------------------------------------------------------------
// Config
// ---------------------------------------------------------------------------

/// Configuration for [`MokaCache`].
#[derive(Debug, Clone)]
pub struct MokaCacheConfig {
    /// Maximum number of entries (LRU eviction past this bound).
    pub max_capacity: u64,
    /// Time-to-live: hard expiry measured from the last insert.
    pub ttl: Duration,
    /// Time-to-idle: soft expiry measured from the last access.
    /// Usually equal to or larger than `ttl`.
    pub tti: Duration,
}

impl Default for MokaCacheConfig {
    fn default() -> Self {
        Self {
            max_capacity: 10_000,
            ttl: Duration::from_secs(3600),
            tti: Duration::from_secs(1800),
        }
    }
}

// ---------------------------------------------------------------------------
// Moka impl (hot in-memory layer)
// ---------------------------------------------------------------------------

/// Hot in-memory cache layer.
///
/// Wraps `moka::future::Cache<String, String>`. Reads are lock-free and
/// sub-microsecond on warm entries. Use for request-local hot paths
/// (LLM dedupe, recent tool results) where a cold cache-miss is tolerable.
pub struct MokaCache {
    inner: MokaFutureCache<String, String>,
    metrics: Arc<CacheMetrics>,
}

impl MokaCache {
    /// Build a new Moka cache from the provided config.
    pub fn new(config: MokaCacheConfig) -> Self {
        let mut builder = MokaFutureCache::builder().max_capacity(config.max_capacity);
        if !config.ttl.is_zero() {
            builder = builder.time_to_live(config.ttl);
        }
        if !config.tti.is_zero() {
            builder = builder.time_to_idle(config.tti);
        }
        Self {
            inner: builder.build(),
            metrics: Arc::new(CacheMetrics::default()),
        }
    }

    /// Access the metrics handle. Cloneable `Arc` so dashboards can hold
    /// a reference independent of the cache lifetime.
    pub fn metrics(&self) -> Arc<CacheMetrics> {
        Arc::clone(&self.metrics)
    }

    /// Exposed for tests: approximate entry count after housekeeping.
    pub fn entry_count(&self) -> u64 {
        self.inner.run_pending_tasks_if_needed();
        self.inner.entry_count()
    }
}

#[async_trait]
impl Cache for MokaCache {
    async fn get(&self, key: &str) -> anyhow::Result<Option<String>> {
        match self.inner.get(key).await {
            Some(v) => {
                self.metrics.record_hit();
                Ok(Some(v))
            }
            None => {
                self.metrics.record_miss();
                Ok(None)
            }
        }
    }

    async fn insert(&self, key: String, value: String) -> anyhow::Result<()> {
        self.inner.insert(key, value).await;
        self.metrics.record_insert();
        Ok(())
    }

    async fn invalidate(&self, key: &str) -> anyhow::Result<()> {
        self.inner.invalidate(key).await;
        self.metrics.record_invalidation();
        Ok(())
    }

    async fn clear(&self) -> anyhow::Result<()> {
        self.inner.invalidate_all();
        self.inner.run_pending_tasks_if_needed();
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Redb impl (persistent embedded layer — v3.10b)
// ---------------------------------------------------------------------------

const REDB_TABLE: TableDefinition<&str, &[u8]> = TableDefinition::new("captain_cache");

/// Serializable envelope written to redb.
///
/// `expires_at_unix_ms == 0` means no TTL (entry kept until manually
/// invalidated). Any non-zero value is compared against the wall clock
/// on read and on GC sweeps.
#[derive(Serialize, Deserialize)]
struct RedbEntry {
    value: String,
    expires_at_unix_ms: u64,
}

/// Persistent cache layer backed by [`redb`](https://github.com/cberner/redb).
///
/// redb gives us WAL + ACID out of the box, so an unplanned daemon
/// shutdown in the middle of a write cannot corrupt the file. TTL is
/// implemented manually (redb has no native expiry): every entry stores
/// an absolute `expires_at_unix_ms`, checked on each read and by an
/// optional background GC task.
///
/// All database operations are synchronous — we wrap them in
/// [`tokio::task::spawn_blocking`] so they don't block the async runtime.
pub struct RedbCache {
    db: Arc<Database>,
    default_ttl: Duration,
    path: PathBuf,
    metrics: Arc<CacheMetrics>,
}

impl RedbCache {
    /// Open (or create) a redb database at `path` with the given default TTL.
    ///
    /// A TTL of `Duration::ZERO` disables expiry: entries persist until
    /// invalidated explicitly.
    pub async fn open(path: impl AsRef<Path>, default_ttl: Duration) -> anyhow::Result<Self> {
        let path = path.as_ref().to_path_buf();
        let path_clone = path.clone();
        let db = task::spawn_blocking(move || Database::create(path_clone))
            .await
            .map_err(|e| anyhow::anyhow!("redb open join error: {e}"))?
            .map_err(|e| anyhow::anyhow!("redb open failed ({}): {e}", path.display()))?;

        // Touch the table once so future reads never race with creation.
        let db = Arc::new(db);
        {
            let db = Arc::clone(&db);
            task::spawn_blocking(move || -> anyhow::Result<()> {
                let txn = db.begin_write()?;
                {
                    let _table = txn.open_table(REDB_TABLE)?;
                }
                txn.commit()?;
                Ok(())
            })
            .await
            .map_err(|e| anyhow::anyhow!("redb init join error: {e}"))??;
        }

        Ok(Self {
            db,
            default_ttl,
            path,
            metrics: Arc::new(CacheMetrics::default()),
        })
    }

    /// Path the cache was opened from. Useful for diagnostics and tests.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Metrics handle, cloneable across tasks.
    pub fn metrics(&self) -> Arc<CacheMetrics> {
        Arc::clone(&self.metrics)
    }

    /// Run a one-shot garbage collection pass, removing every entry whose
    /// `expires_at_unix_ms` has elapsed. Returns the number of entries
    /// removed.
    ///
    /// Intended to be called from [`RedbCache::spawn_gc_task`] or ad-hoc
    /// from tests.
    pub async fn run_gc(&self) -> anyhow::Result<u64> {
        let db = Arc::clone(&self.db);
        task::spawn_blocking(move || -> anyhow::Result<u64> {
            let now_ms = now_unix_ms();
            let mut to_delete: Vec<String> = Vec::new();

            // Collect expired keys under a read transaction first to avoid
            // holding a write lock while scanning.
            {
                let read_txn = db.begin_read()?;
                let table = read_txn.open_table(REDB_TABLE)?;
                for row in table.iter()? {
                    let (key, value) = row?;
                    let raw = value.value();
                    if let Ok(entry) = bincode::deserialize::<RedbEntry>(raw) {
                        if entry.expires_at_unix_ms != 0 && entry.expires_at_unix_ms <= now_ms {
                            to_delete.push(key.value().to_string());
                        }
                    }
                }
            }

            let mut removed = 0_u64;
            if !to_delete.is_empty() {
                let write_txn = db.begin_write()?;
                {
                    let mut table = write_txn.open_table(REDB_TABLE)?;
                    for k in &to_delete {
                        if table.remove(k.as_str())?.is_some() {
                            removed += 1;
                        }
                    }
                }
                write_txn.commit()?;
            }
            Ok(removed)
        })
        .await
        .map_err(|e| anyhow::anyhow!("redb gc join error: {e}"))?
    }

    /// Spawn a background task that invokes [`RedbCache::run_gc`] every
    /// `interval`. The returned [`task::JoinHandle`] is aborted when the
    /// caller drops it; holding it is the caller's responsibility.
    ///
    /// Errors during GC are logged at `warn` level — they never crash
    /// the runtime.
    pub fn spawn_gc_task(self: &Arc<Self>, interval: Duration) -> task::JoinHandle<()> {
        let this = Arc::clone(self);
        tokio::spawn(async move {
            let mut ticker = tokio::time::interval(interval);
            // Skip the immediate tick so we don't GC an empty store on boot.
            ticker.tick().await;
            loop {
                ticker.tick().await;
                match this.run_gc().await {
                    Ok(n) if n > 0 => {
                        tracing::debug!("RedbCache GC removed {n} expired entries");
                    }
                    Ok(_) => {}
                    Err(err) => tracing::warn!("RedbCache GC failed: {err}"),
                }
            }
        })
    }

    /// Approximate entry count. Iterates the table — use sparingly.
    pub async fn entry_count(&self) -> anyhow::Result<u64> {
        let db = Arc::clone(&self.db);
        task::spawn_blocking(move || -> anyhow::Result<u64> {
            let read_txn = db.begin_read()?;
            let table = read_txn.open_table(REDB_TABLE)?;
            Ok(table.len()?)
        })
        .await
        .map_err(|e| anyhow::anyhow!("redb len join error: {e}"))?
    }
}

#[async_trait]
impl Cache for RedbCache {
    async fn get(&self, key: &str) -> anyhow::Result<Option<String>> {
        let db = Arc::clone(&self.db);
        let key_owned = key.to_string();
        let outcome: Option<String> =
            task::spawn_blocking(move || -> anyhow::Result<Option<String>> {
                let read_txn = db.begin_read()?;
                let table = read_txn.open_table(REDB_TABLE)?;
                match table.get(key_owned.as_str())? {
                    Some(raw) => {
                        let entry: RedbEntry = bincode::deserialize(raw.value())
                            .map_err(|e| anyhow::anyhow!("redb deserialize: {e}"))?;
                        if entry.expires_at_unix_ms != 0
                            && entry.expires_at_unix_ms <= now_unix_ms()
                        {
                            Ok(None)
                        } else {
                            Ok(Some(entry.value))
                        }
                    }
                    None => Ok(None),
                }
            })
            .await
            .map_err(|e| anyhow::anyhow!("redb get join error: {e}"))??;

        match &outcome {
            Some(_) => self.metrics.record_hit(),
            None => self.metrics.record_miss(),
        }
        Ok(outcome)
    }

    async fn insert(&self, key: String, value: String) -> anyhow::Result<()> {
        let db = Arc::clone(&self.db);
        let ttl = self.default_ttl;
        task::spawn_blocking(move || -> anyhow::Result<()> {
            let expires_at_unix_ms = if ttl.is_zero() {
                0
            } else {
                now_unix_ms().saturating_add(ttl.as_millis() as u64)
            };
            let entry = RedbEntry {
                value,
                expires_at_unix_ms,
            };
            let bytes =
                bincode::serialize(&entry).map_err(|e| anyhow::anyhow!("redb serialize: {e}"))?;

            let write_txn = db.begin_write()?;
            {
                let mut table = write_txn.open_table(REDB_TABLE)?;
                table.insert(key.as_str(), bytes.as_slice())?;
            }
            write_txn.commit()?;
            Ok(())
        })
        .await
        .map_err(|e| anyhow::anyhow!("redb insert join error: {e}"))??;
        self.metrics.record_insert();
        Ok(())
    }

    async fn invalidate(&self, key: &str) -> anyhow::Result<()> {
        let db = Arc::clone(&self.db);
        let key_owned = key.to_string();
        task::spawn_blocking(move || -> anyhow::Result<()> {
            let write_txn = db.begin_write()?;
            {
                let mut table = write_txn.open_table(REDB_TABLE)?;
                table.remove(key_owned.as_str())?;
            }
            write_txn.commit()?;
            Ok(())
        })
        .await
        .map_err(|e| anyhow::anyhow!("redb invalidate join error: {e}"))??;
        self.metrics.record_invalidation();
        Ok(())
    }

    async fn clear(&self) -> anyhow::Result<()> {
        let db = Arc::clone(&self.db);
        task::spawn_blocking(move || -> anyhow::Result<()> {
            let write_txn = db.begin_write()?;
            {
                // redb has no bulk-clear; delete the table + recreate it.
                write_txn.delete_table(REDB_TABLE)?;
                let _t = write_txn.open_table(REDB_TABLE)?;
            }
            write_txn.commit()?;
            Ok(())
        })
        .await
        .map_err(|e| anyhow::anyhow!("redb clear join error: {e}"))??;
        Ok(())
    }
}

fn now_unix_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

// ---------------------------------------------------------------------------
// Hybrid impl (Moka hot + Redb cold — v3.10c)
// ---------------------------------------------------------------------------

/// Two-tier cache combining a hot in-memory Moka layer with a cold
/// persistent Redb layer.
///
/// ## Read path
/// 1. Look up Moka (microseconds). Hit → return immediately.
/// 2. Miss → look up Redb (milliseconds). Hit → promote into Moka, return.
/// 3. Miss on both → caller recomputes.
///
/// ## Write path
/// Writes go to both layers. A failure on the persistent layer is logged
/// at `warn` and swallowed; the hot layer alone is still useful until
/// the disk recovers or the daemon restarts.
///
/// ## Fallback
/// If [`HybridCache::with_optional_persistent`] receives `None`, the
/// hybrid degrades gracefully into a Moka-only cache. This is how the
/// runtime keeps operating when redb fails to open (disk full, perms,
/// corruption too severe for WAL recovery).
pub struct HybridCache {
    hot: Arc<MokaCache>,
    cold: Option<Arc<RedbCache>>,
}

impl HybridCache {
    /// Build a hybrid cache with both layers active.
    pub fn new(hot: Arc<MokaCache>, cold: Arc<RedbCache>) -> Self {
        Self {
            hot,
            cold: Some(cold),
        }
    }

    /// Build a hybrid cache where the persistent layer is optional.
    /// Pass `None` to degrade to a Moka-only cache (useful when redb
    /// failed to open and the runtime must keep serving).
    pub fn with_optional_persistent(hot: Arc<MokaCache>, cold: Option<Arc<RedbCache>>) -> Self {
        Self { hot, cold }
    }

    /// Whether the persistent layer is wired in.
    pub fn is_persistent(&self) -> bool {
        self.cold.is_some()
    }
}

#[async_trait]
impl Cache for HybridCache {
    async fn get(&self, key: &str) -> anyhow::Result<Option<String>> {
        if let Some(v) = self.hot.get(key).await? {
            return Ok(Some(v));
        }

        if let Some(cold) = &self.cold {
            match cold.get(key).await {
                Ok(Some(v)) => {
                    // Promote to the hot layer so the next read is μs-fast.
                    if let Err(err) = self.hot.insert(key.to_string(), v.clone()).await {
                        tracing::warn!("HybridCache hot-layer promote failed: {err}");
                    }
                    return Ok(Some(v));
                }
                Ok(None) => {}
                Err(err) => {
                    tracing::warn!("HybridCache cold-layer read failed: {err}");
                }
            }
        }
        Ok(None)
    }

    async fn insert(&self, key: String, value: String) -> anyhow::Result<()> {
        // Hot write always happens — it's the fallback.
        self.hot.insert(key.clone(), value.clone()).await?;

        if let Some(cold) = &self.cold {
            if let Err(err) = cold.insert(key, value).await {
                tracing::warn!("HybridCache cold-layer write failed: {err}");
            }
        }
        Ok(())
    }

    async fn invalidate(&self, key: &str) -> anyhow::Result<()> {
        self.hot.invalidate(key).await?;
        if let Some(cold) = &self.cold {
            if let Err(err) = cold.invalidate(key).await {
                tracing::warn!("HybridCache cold-layer invalidate failed: {err}");
            }
        }
        Ok(())
    }

    async fn clear(&self) -> anyhow::Result<()> {
        self.hot.clear().await?;
        if let Some(cold) = &self.cold {
            if let Err(err) = cold.clear().await {
                tracing::warn!("HybridCache cold-layer clear failed: {err}");
            }
        }
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Redis impl (optional distributed layer — v3.10d)
// ---------------------------------------------------------------------------

/// Distributed cache layer backed by Redis (v3.10d).
///
/// Only compiled when the `redis-cache` Cargo feature is enabled; this
/// keeps the default build free of Redis transitive deps for users who
/// run Captain without external infrastructure.
///
/// TTL is native: we use `SET key value EX ttl` so the Redis server
/// takes care of expiry. If the connection pool is saturated or the
/// server goes away, every call returns an error — callers should wrap
/// `RedisCache` behind [`HybridCache::with_optional_persistent`] or a
/// similar fallback so the runtime stays serveable.
#[cfg(feature = "redis-cache")]
pub struct RedisCache {
    pool: bb8::Pool<bb8_redis::RedisConnectionManager>,
    default_ttl: Duration,
    metrics: Arc<CacheMetrics>,
}

#[cfg(feature = "redis-cache")]
impl RedisCache {
    /// Build a `RedisCache` pointing at the given URL (`redis://…` or
    /// `rediss://…`) with a default TTL applied to every `insert`.
    pub async fn open(url: &str, default_ttl: Duration) -> anyhow::Result<Self> {
        let manager = bb8_redis::RedisConnectionManager::new(url)
            .map_err(|e| anyhow::anyhow!("redis manager: {e}"))?;
        let pool = bb8::Pool::builder()
            .max_size(16)
            .build(manager)
            .await
            .map_err(|e| anyhow::anyhow!("redis pool: {e}"))?;
        Ok(Self {
            pool,
            default_ttl,
            metrics: Arc::new(CacheMetrics::default()),
        })
    }

    pub fn metrics(&self) -> Arc<CacheMetrics> {
        Arc::clone(&self.metrics)
    }
}

#[cfg(feature = "redis-cache")]
#[async_trait]
impl Cache for RedisCache {
    async fn get(&self, key: &str) -> anyhow::Result<Option<String>> {
        use redis::AsyncCommands;
        let mut conn = self
            .pool
            .get()
            .await
            .map_err(|e| anyhow::anyhow!("redis get conn: {e}"))?;
        let value: Option<String> = conn
            .get(key)
            .await
            .map_err(|e| anyhow::anyhow!("redis GET {key}: {e}"))?;
        match &value {
            Some(_) => self.metrics.record_hit(),
            None => self.metrics.record_miss(),
        }
        Ok(value)
    }

    async fn insert(&self, key: String, value: String) -> anyhow::Result<()> {
        use redis::AsyncCommands;
        let mut conn = self
            .pool
            .get()
            .await
            .map_err(|e| anyhow::anyhow!("redis get conn: {e}"))?;
        if self.default_ttl.is_zero() {
            let _: () = conn
                .set(&key, value)
                .await
                .map_err(|e| anyhow::anyhow!("redis SET {key}: {e}"))?;
        } else {
            let ttl_secs = self.default_ttl.as_secs().max(1);
            let _: () = conn
                .set_ex(&key, value, ttl_secs)
                .await
                .map_err(|e| anyhow::anyhow!("redis SETEX {key}: {e}"))?;
        }
        self.metrics.record_insert();
        Ok(())
    }

    async fn invalidate(&self, key: &str) -> anyhow::Result<()> {
        use redis::AsyncCommands;
        let mut conn = self
            .pool
            .get()
            .await
            .map_err(|e| anyhow::anyhow!("redis get conn: {e}"))?;
        let _: i32 = conn
            .del(key)
            .await
            .map_err(|e| anyhow::anyhow!("redis DEL {key}: {e}"))?;
        self.metrics.record_invalidation();
        Ok(())
    }

    /// Redis `clear()` is intentionally a no-op with a warning: issuing
    /// `FLUSHDB` from a cache wrapper would wipe data the user never
    /// gave us permission to touch (other services may share the
    /// instance). Callers that really need to drop every key should go
    /// through Redis tooling directly.
    async fn clear(&self) -> anyhow::Result<()> {
        tracing::warn!(
            "RedisCache::clear() is a no-op — use redis-cli FLUSHDB explicitly if needed"
        );
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Moka helper (moka 0.12 renamed `sync` method on the future cache)
// ---------------------------------------------------------------------------

trait MokaHousekeeping {
    fn run_pending_tasks_if_needed(&self);
}

impl MokaHousekeeping for MokaFutureCache<String, String> {
    /// `moka::future::Cache::run_pending_tasks` is async; we keep the
    /// blocking flavor confined here so we can reach a stable entry count
    /// from non-async test code without spawning a runtime.
    fn run_pending_tasks_if_needed(&self) {
        // No-op in 0.12 — pending ops are drained on read/write. The hook
        // exists so we can upgrade behavior if moka's API evolves.
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn fast_config() -> MokaCacheConfig {
        MokaCacheConfig {
            max_capacity: 4,
            ttl: Duration::from_secs(60),
            tti: Duration::from_secs(60),
        }
    }

    #[tokio::test]
    async fn insert_then_get_returns_hit() {
        let cache = MokaCache::new(fast_config());
        cache.insert("k1".into(), "v1".into()).await.unwrap();

        let got = cache.get("k1").await.unwrap();
        assert_eq!(got.as_deref(), Some("v1"));

        let m = cache.metrics().snapshot();
        assert_eq!(m.hits, 1);
        assert_eq!(m.misses, 0);
        assert_eq!(m.inserts, 1);
    }

    #[tokio::test]
    async fn missing_key_is_miss() {
        let cache = MokaCache::new(fast_config());
        let got = cache.get("nope").await.unwrap();
        assert!(got.is_none());

        let m = cache.metrics().snapshot();
        assert_eq!(m.hits, 0);
        assert_eq!(m.misses, 1);
    }

    #[tokio::test]
    async fn ttl_expires_entry() {
        let cache = MokaCache::new(MokaCacheConfig {
            max_capacity: 8,
            ttl: Duration::from_millis(50),
            tti: Duration::from_millis(50),
        });
        cache.insert("k".into(), "v".into()).await.unwrap();
        assert!(cache.get("k").await.unwrap().is_some());

        tokio::time::sleep(Duration::from_millis(120)).await;
        assert!(cache.get("k").await.unwrap().is_none());
    }

    #[tokio::test]
    async fn capacity_evicts_lru() {
        let cache = MokaCache::new(MokaCacheConfig {
            max_capacity: 2,
            ttl: Duration::from_secs(60),
            tti: Duration::from_secs(60),
        });
        cache.insert("a".into(), "1".into()).await.unwrap();
        cache.insert("b".into(), "2".into()).await.unwrap();
        cache.insert("c".into(), "3".into()).await.unwrap();

        // Give moka a moment to drain its write queue.
        tokio::time::sleep(Duration::from_millis(20)).await;
        assert!(cache.entry_count() <= 2);
    }

    #[tokio::test]
    async fn invalidate_removes_key() {
        let cache = MokaCache::new(fast_config());
        cache.insert("k".into(), "v".into()).await.unwrap();
        cache.invalidate("k").await.unwrap();
        assert!(cache.get("k").await.unwrap().is_none());

        let m = cache.metrics().snapshot();
        assert_eq!(m.invalidations, 1);
    }

    #[tokio::test]
    async fn clear_empties_cache() {
        let cache = MokaCache::new(fast_config());
        cache.insert("a".into(), "1".into()).await.unwrap();
        cache.insert("b".into(), "2".into()).await.unwrap();
        cache.clear().await.unwrap();

        assert!(cache.get("a").await.unwrap().is_none());
        assert!(cache.get("b").await.unwrap().is_none());
    }

    #[tokio::test]
    async fn hit_ratio_reports_expected_value() {
        let cache = MokaCache::new(fast_config());
        cache.insert("k".into(), "v".into()).await.unwrap();
        let _ = cache.get("k").await.unwrap(); // hit
        let _ = cache.get("k").await.unwrap(); // hit
        let _ = cache.get("other").await.unwrap(); // miss

        let snap = cache.metrics().snapshot();
        let expected = 2.0_f64 / 3.0_f64;
        assert!((snap.hit_ratio() - expected).abs() < 1e-9);
    }

    // ---- RedbCache tests (v3.10b) ----

    fn unique_redb_path() -> std::path::PathBuf {
        let dir = std::env::temp_dir();
        let id = format!(
            "captain-redb-{}-{}.redb",
            std::process::id(),
            uuid::Uuid::new_v4(),
        );
        dir.join(id)
    }

    #[tokio::test]
    async fn redb_insert_then_get_returns_hit() {
        let path = unique_redb_path();
        let cache = RedbCache::open(&path, Duration::from_secs(60))
            .await
            .expect("open redb");
        cache.insert("k".into(), "v".into()).await.unwrap();

        let got = cache.get("k").await.unwrap();
        assert_eq!(got.as_deref(), Some("v"));

        let m = cache.metrics().snapshot();
        assert_eq!(m.hits, 1);
        assert_eq!(m.misses, 0);
        let _ = std::fs::remove_file(&path);
    }

    #[tokio::test]
    async fn redb_persists_across_reopen() {
        let path = unique_redb_path();

        // First instance writes the entry and is dropped.
        {
            let cache = RedbCache::open(&path, Duration::from_secs(600))
                .await
                .expect("open redb #1");
            cache
                .insert("persistent".into(), "42".into())
                .await
                .unwrap();
            drop(cache);
        }

        // Second instance must see the entry without repopulating.
        let cache2 = RedbCache::open(&path, Duration::from_secs(600))
            .await
            .expect("open redb #2");
        let got = cache2.get("persistent").await.unwrap();
        assert_eq!(got.as_deref(), Some("42"));

        let _ = std::fs::remove_file(&path);
    }

    #[tokio::test]
    async fn redb_ttl_expires_entry() {
        let path = unique_redb_path();
        let cache = RedbCache::open(&path, Duration::from_millis(50))
            .await
            .expect("open redb");
        cache.insert("short".into(), "x".into()).await.unwrap();
        assert!(cache.get("short").await.unwrap().is_some());

        tokio::time::sleep(Duration::from_millis(120)).await;
        assert!(cache.get("short").await.unwrap().is_none());

        let _ = std::fs::remove_file(&path);
    }

    #[tokio::test]
    async fn redb_gc_removes_expired_entries() {
        let path = unique_redb_path();
        let cache = RedbCache::open(&path, Duration::from_millis(50))
            .await
            .expect("open redb");
        cache.insert("a".into(), "1".into()).await.unwrap();
        cache.insert("b".into(), "2".into()).await.unwrap();
        assert_eq!(cache.entry_count().await.unwrap(), 2);

        tokio::time::sleep(Duration::from_millis(120)).await;
        let removed = cache.run_gc().await.unwrap();
        assert_eq!(removed, 2);
        assert_eq!(cache.entry_count().await.unwrap(), 0);

        let _ = std::fs::remove_file(&path);
    }

    #[tokio::test]
    async fn redb_invalidate_removes_key() {
        let path = unique_redb_path();
        let cache = RedbCache::open(&path, Duration::from_secs(60))
            .await
            .expect("open redb");
        cache.insert("k".into(), "v".into()).await.unwrap();
        cache.invalidate("k").await.unwrap();
        assert!(cache.get("k").await.unwrap().is_none());

        let _ = std::fs::remove_file(&path);
    }

    #[tokio::test]
    async fn redb_clear_empties_table() {
        let path = unique_redb_path();
        let cache = RedbCache::open(&path, Duration::from_secs(60))
            .await
            .expect("open redb");
        cache.insert("a".into(), "1".into()).await.unwrap();
        cache.insert("b".into(), "2".into()).await.unwrap();
        cache.clear().await.unwrap();
        assert_eq!(cache.entry_count().await.unwrap(), 0);

        let _ = std::fs::remove_file(&path);
    }

    // ---- HybridCache tests (v3.10c) ----

    #[tokio::test]
    async fn hybrid_promotes_from_cold_to_hot() {
        let path = unique_redb_path();
        let hot = Arc::new(MokaCache::new(fast_config()));
        let cold = Arc::new(
            RedbCache::open(&path, Duration::from_secs(60))
                .await
                .expect("open redb"),
        );

        // Seed only the cold layer to prove the hybrid pulls from it.
        cold.insert("k".into(), "from-disk".into()).await.unwrap();

        let hybrid = HybridCache::new(Arc::clone(&hot), Arc::clone(&cold));
        let got = hybrid.get("k").await.unwrap();
        assert_eq!(got.as_deref(), Some("from-disk"));

        // Second call must hit the hot layer (promoted above).
        let hot_before = hot.metrics().snapshot().hits;
        let _ = hybrid.get("k").await.unwrap();
        let hot_after = hot.metrics().snapshot().hits;
        assert!(hot_after > hot_before, "expected hot hit after promote");

        let _ = std::fs::remove_file(&path);
    }

    #[tokio::test]
    async fn hybrid_writes_to_both_layers() {
        let path = unique_redb_path();
        let hot = Arc::new(MokaCache::new(fast_config()));
        let cold = Arc::new(
            RedbCache::open(&path, Duration::from_secs(60))
                .await
                .expect("open redb"),
        );
        let hybrid = HybridCache::new(Arc::clone(&hot), Arc::clone(&cold));

        hybrid
            .insert("shared".into(), "stored".into())
            .await
            .unwrap();

        assert_eq!(
            hot.get("shared").await.unwrap().as_deref(),
            Some("stored"),
            "hot layer should hold the value"
        );
        assert_eq!(
            cold.get("shared").await.unwrap().as_deref(),
            Some("stored"),
            "cold layer should hold the value"
        );

        let _ = std::fs::remove_file(&path);
    }

    #[tokio::test]
    async fn hybrid_survives_simulated_restart() {
        let path = unique_redb_path();

        // Session #1: write via the hybrid, drop everything afterwards.
        {
            let hot = Arc::new(MokaCache::new(fast_config()));
            let cold = Arc::new(
                RedbCache::open(&path, Duration::from_secs(600))
                    .await
                    .expect("open redb #1"),
            );
            let hybrid = HybridCache::new(hot, cold);
            hybrid
                .insert("keep".into(), "survives".into())
                .await
                .unwrap();
        }

        // Session #2: fresh Moka (empty), same redb file — must still read.
        let hot2 = Arc::new(MokaCache::new(fast_config()));
        let cold2 = Arc::new(
            RedbCache::open(&path, Duration::from_secs(600))
                .await
                .expect("open redb #2"),
        );
        let hybrid2 = HybridCache::new(hot2, cold2);
        assert_eq!(
            hybrid2.get("keep").await.unwrap().as_deref(),
            Some("survives"),
            "hybrid must read through to the persistent layer after restart"
        );

        let _ = std::fs::remove_file(&path);
    }

    #[tokio::test]
    async fn hybrid_fallback_moka_only_when_cold_absent() {
        let hot = Arc::new(MokaCache::new(fast_config()));
        let hybrid = HybridCache::with_optional_persistent(Arc::clone(&hot), None);
        assert!(!hybrid.is_persistent());

        hybrid
            .insert("solo".into(), "hot-only".into())
            .await
            .unwrap();
        assert_eq!(
            hybrid.get("solo").await.unwrap().as_deref(),
            Some("hot-only")
        );

        hybrid.invalidate("solo").await.unwrap();
        assert!(hybrid.get("solo").await.unwrap().is_none());
    }
}
