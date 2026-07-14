//! Content-addressed cache for vision descriptions (V.8g, #184).
//!
//! Re-analysing the same video should be nearly free. Without this cache, the
//! frame-by-frame video pipeline re-uploads every frame to the vision provider
//! on every call — even when the user just asks a different question about
//! the same source clip. The cache key is the BLAKE3 hash of the actual
//! image bytes plus a small set of dimensions that change the response shape:
//!
//!   `provider : model : mime : hint_hash : bytes_hash`
//!
//! Why content-hashing instead of path/mtime: real workflows route uploads
//! through `inbound/telegram/<uuid>.mp4`, so the path is fresh on every
//! download but the bytes are identical. Path-based keying would never hit;
//! content-based keying hits every time.
//!
//! Storage: in-process `moka` cache (already a workspace dep). 7-day TTL,
//! 5_000-entry cap. Persistence across daemon restarts is a future ticket
//! (likely redb, mirroring the LLM cache shape) — out of scope for V.8g.

use captain_types::media::MediaUnderstanding;
use moka::future::Cache;
use std::time::Duration;

/// Default TTL for cached vision descriptions. Long enough that re-running
/// the same workflow within a week is free; short enough that bug fixes in
/// the vision pipeline naturally take effect.
const DEFAULT_TTL_SECS: u64 = 7 * 24 * 3600;

/// Max number of cached entries. At ~1-2 KB per `MediaUnderstanding` JSON
/// blob this is ~10 MB worst-case — negligible for a daemon.
const DEFAULT_MAX_ENTRIES: u64 = 5_000;

/// Content-addressed cache for vision descriptions.
///
/// Keyed by `BLAKE3(provider || model || mime || hint || image_bytes)`,
/// flattened into a colon-separated string for moka's `K: Hash`. None of
/// the components contain a colon, so the join is unambiguous.
#[derive(Clone)]
pub struct VisionCache {
    inner: Cache<String, String>,
}

impl VisionCache {
    /// Build a cache with default TTL + capacity.
    pub fn new() -> Self {
        Self {
            inner: Cache::builder()
                .max_capacity(DEFAULT_MAX_ENTRIES)
                .time_to_live(Duration::from_secs(DEFAULT_TTL_SECS))
                .build(),
        }
    }

    /// Build the canonical cache key for a vision request.
    ///
    /// Hashes the image bytes once (BLAKE3 is fast — well over 1 GB/s on
    /// modern CPUs) and the hint separately so a long user prompt doesn't
    /// pollute key-comparison time.
    pub fn make_key(provider: &str, model: &str, mime: &str, hint: &str, bytes: &[u8]) -> String {
        let bytes_hash = blake3::hash(bytes).to_hex();
        let hint_hash = if hint.is_empty() {
            String::new()
        } else {
            blake3::hash(hint.as_bytes()).to_hex().to_string()
        };
        format!("{provider}:{model}:{mime}:{hint_hash}:{bytes_hash}")
    }

    /// Fetch a cached `MediaUnderstanding` if present.
    ///
    /// Returns `None` on miss, on JSON deserialisation failure (treats a
    /// poisoned entry as a miss rather than crashing the request), or on
    /// expired entry.
    pub async fn get(&self, key: &str) -> Option<MediaUnderstanding> {
        let raw = self.inner.get(key).await?;
        match serde_json::from_str::<MediaUnderstanding>(&raw) {
            Ok(v) => Some(v),
            Err(_) => {
                // Defensive: if a future schema change makes a stored value
                // un-deserialisable, drop it and treat as a miss. Better to
                // re-fetch than to surface a crash to the agent.
                self.inner.invalidate(key).await;
                None
            }
        }
    }

    /// Store a `MediaUnderstanding` under `key`.
    ///
    /// Best-effort — JSON serialisation can't fail in practice for the
    /// `MediaUnderstanding` shape (only String fields), but we silently
    /// skip storage if it ever does. A failed store just means the next
    /// request goes to the provider, which is exactly the pre-cache
    /// behaviour.
    pub async fn put(&self, key: String, value: &MediaUnderstanding) {
        if let Ok(raw) = serde_json::to_string(value) {
            self.inner.insert(key, raw).await;
        }
    }
}

impl Default for VisionCache {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use captain_types::media::MediaType;

    fn sample_understanding() -> MediaUnderstanding {
        MediaUnderstanding {
            media_type: MediaType::Image,
            description: "A black square on white background".to_string(),
            provider: "anthropic".to_string(),
            model: "claude-haiku-4-5-20251001".to_string(),
        }
    }

    #[tokio::test]
    async fn put_then_get_roundtrips() {
        let cache = VisionCache::new();
        let key = VisionCache::make_key(
            "anthropic",
            "claude-haiku-4-5-20251001",
            "image/jpeg",
            "",
            &[0xDE, 0xAD, 0xBE, 0xEF],
        );
        let val = sample_understanding();
        cache.put(key.clone(), &val).await;

        let fetched = cache.get(&key).await.expect("must hit");
        assert_eq!(fetched.description, val.description);
        assert_eq!(fetched.provider, val.provider);
        assert_eq!(fetched.model, val.model);
    }

    #[tokio::test]
    async fn miss_returns_none() {
        let cache = VisionCache::new();
        let key =
            VisionCache::make_key("anthropic", "claude-sonnet-4-6", "image/jpeg", "", &[0x01]);
        assert!(cache.get(&key).await.is_none());
    }

    /// Content-addressing: same bytes, same hint, same provider, same model →
    /// same key. The whole point of the cache.
    #[test]
    fn key_is_deterministic_per_content() {
        let bytes = [0x01, 0x02, 0x03];
        let k1 = VisionCache::make_key("anthropic", "claude-sonnet-4-6", "image/jpeg", "", &bytes);
        let k2 = VisionCache::make_key("anthropic", "claude-sonnet-4-6", "image/jpeg", "", &bytes);
        assert_eq!(k1, k2);
    }

    /// Mime change (e.g. caller swaps PNG for JPEG of the same pixels) MUST
    /// produce a different key — providers care about media_type in the
    /// upload payload, so the response shape is mime-dependent.
    #[test]
    fn key_differs_when_mime_differs() {
        let bytes = [0x01, 0x02];
        let k_png =
            VisionCache::make_key("anthropic", "claude-sonnet-4-6", "image/png", "", &bytes);
        let k_jpg =
            VisionCache::make_key("anthropic", "claude-sonnet-4-6", "image/jpeg", "", &bytes);
        assert_ne!(k_png, k_jpg);
    }

    /// Hint change MUST produce a different key — same image with a
    /// different question to the model is a different request.
    #[test]
    fn key_differs_when_hint_differs() {
        let bytes = [0x01, 0x02];
        let k_a = VisionCache::make_key(
            "anthropic",
            "claude-sonnet-4-6",
            "image/jpeg",
            "Décris l'action principale",
            &bytes,
        );
        let k_b = VisionCache::make_key(
            "anthropic",
            "claude-sonnet-4-6",
            "image/jpeg",
            "Combien de personnes ?",
            &bytes,
        );
        assert_ne!(k_a, k_b);
    }

    /// Model change MUST produce a different key — Haiku and Sonnet produce
    /// genuinely different descriptions, so cache-mixing them is wrong.
    #[test]
    fn key_differs_when_model_differs() {
        let bytes = [0x01, 0x02];
        let k_haiku = VisionCache::make_key(
            "anthropic",
            "claude-haiku-4-5-20251001",
            "image/jpeg",
            "",
            &bytes,
        );
        let k_sonnet =
            VisionCache::make_key("anthropic", "claude-sonnet-4-6", "image/jpeg", "", &bytes);
        assert_ne!(k_haiku, k_sonnet);
    }

    /// Cached value must roundtrip byte-for-byte (no truncation in JSON
    /// serialisation, no field loss). Long descriptions matter for video
    /// frame analysis where each frame produces ~600 chars.
    #[tokio::test]
    async fn cached_value_matches_byte_for_byte() {
        let cache = VisionCache::new();
        let mut val = sample_understanding();
        val.description = "x".repeat(2000); // realistic frame description size
        let key =
            VisionCache::make_key("anthropic", "claude-sonnet-4-6", "image/jpeg", "", &[0xFF]);
        cache.put(key.clone(), &val).await;
        let fetched = cache.get(&key).await.expect("must hit");
        assert_eq!(fetched.description, val.description);
        assert_eq!(fetched.description.len(), 2000);
    }
}
