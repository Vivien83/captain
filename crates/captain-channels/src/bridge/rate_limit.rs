//! Per-channel inbound rate limiting.

use dashmap::DashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

/// Tracks recent message timestamps per `"{channel_type}:{platform_id}"`.
#[derive(Debug, Clone, Default)]
pub(crate) struct ChannelRateLimiter {
    buckets: Arc<DashMap<String, Vec<Instant>>>,
}

impl ChannelRateLimiter {
    /// Check if a user is rate-limited.
    ///
    /// Returns `Ok(())` if allowed and `Err(message)` if blocked.
    /// `max_per_minute == 0` means unlimited.
    pub(crate) fn check(
        &self,
        channel_type: &str,
        platform_id: &str,
        max_per_minute: u32,
    ) -> Result<(), String> {
        if max_per_minute == 0 {
            return Ok(());
        }

        let key = format!("{channel_type}:{platform_id}");
        let now = Instant::now();
        let window = Duration::from_secs(60);

        let mut entry = self.buckets.entry(key).or_default();
        entry.retain(|&ts| now.duration_since(ts) < window);

        if entry.len() >= max_per_minute as usize {
            return Err(format!(
                "Rate limit exceeded ({max_per_minute} messages/minute). Please wait."
            ));
        }

        entry.push(now);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rate_limiter_allows_within_limit() {
        let limiter = ChannelRateLimiter::default();

        assert!(limiter.check("telegram", "user1", 5).is_ok());
        assert!(limiter.check("telegram", "user1", 5).is_ok());
        assert!(limiter.check("telegram", "user1", 5).is_ok());
    }

    #[test]
    fn rate_limiter_blocks_over_limit() {
        let limiter = ChannelRateLimiter::default();
        for _ in 0..3 {
            limiter.check("telegram", "user1", 3).unwrap();
        }

        let result = limiter.check("telegram", "user1", 3);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("Rate limit exceeded"));
    }

    #[test]
    fn rate_limiter_zero_means_unlimited() {
        let limiter = ChannelRateLimiter::default();

        for _ in 0..100 {
            assert!(limiter.check("telegram", "user1", 0).is_ok());
        }
    }

    #[test]
    fn rate_limiter_separates_users() {
        let limiter = ChannelRateLimiter::default();
        for _ in 0..3 {
            limiter.check("telegram", "user1", 3).unwrap();
        }

        assert!(limiter.check("telegram", "user1", 3).is_err());
        assert!(limiter.check("telegram", "user2", 3).is_ok());
    }
}
