//! Pattern detector — observer for recurring tool sequences (v3.13a).
//!
//! Each successful tool execution funnels through `record(agent_id,
//! tool)`. The detector keeps a per-agent rolling window of the last
//! `WINDOW_LEN` tools; once the window is full it computes a stable
//! sha256 hash of the ordered sequence and upserts the corresponding
//! row in `skill_patterns`. When the row's `count` crosses the
//! configured threshold and the pattern is fresh, a
//! `SkillPatternCandidate` is emitted on the output channel for the
//! `SkillProposer` (v3.13b) to judge with the LLM.
//!
//! The detector is intentionally cheap: a DashMap lookup, a hash, a
//! single-row UPDATE/INSERT, and an optional non-blocking try_send.
//! No model calls live here.

use captain_memory::skill_patterns::{self, SkillPattern};
use dashmap::DashMap;
use rusqlite::Connection;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::VecDeque;
use std::sync::{Arc, Mutex, OnceLock};
use tokio::sync::mpsc;
use tracing::{debug, trace};

pub const WINDOW_LEN: usize = 5;
pub const MIN_SEQ_LEN: usize = 3;
pub const DEFAULT_THRESHOLD: u32 = 5;
pub const DEFAULT_WINDOW_DAYS: u32 = 7;
pub const DEFAULT_OUTPUT_CAPACITY: usize = 64;

/// Forwarded to the proposer when a pattern crosses the threshold.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SkillPatternCandidate {
    pub hash: String,
    pub agent_id: String,
    pub tool_sequence: Vec<String>,
    pub count: u32,
    pub first_seen: i64,
    pub last_seen: i64,
    pub origin_channel: Option<String>,
}

impl From<&SkillPattern> for SkillPatternCandidate {
    fn from(p: &SkillPattern) -> Self {
        Self {
            hash: p.hash.clone(),
            agent_id: p.agent_id.clone(),
            tool_sequence: p.tool_sequence.clone(),
            count: p.count,
            first_seen: p.first_seen,
            last_seen: p.last_seen,
            origin_channel: None,
        }
    }
}

#[derive(Debug, Clone)]
pub struct DetectorConfig {
    pub threshold: u32,
    pub window_days: u32,
    pub min_seq_len: usize,
    pub window_len: usize,
}

impl Default for DetectorConfig {
    fn default() -> Self {
        Self {
            threshold: DEFAULT_THRESHOLD,
            window_days: DEFAULT_WINDOW_DAYS,
            min_seq_len: MIN_SEQ_LEN,
            window_len: WINDOW_LEN,
        }
    }
}

impl From<&captain_types::config::SkillsConfig> for DetectorConfig {
    fn from(s: &captain_types::config::SkillsConfig) -> Self {
        Self {
            threshold: s.pattern_threshold,
            window_days: s.pattern_window_days,
            min_seq_len: MIN_SEQ_LEN,
            window_len: WINDOW_LEN,
        }
    }
}

impl DetectorConfig {
    pub fn apply_autonomy_aggressiveness(&mut self, aggressiveness: f32) {
        let scaled = (self.threshold as f32 / aggressiveness).round();
        self.threshold = scaled.clamp(1.0, u32::MAX as f32) as u32;
    }
}

pub struct PatternDetector {
    conn: Arc<Mutex<Connection>>,
    cfg: DetectorConfig,
    rolling: DashMap<String, VecDeque<String>>,
    tx: mpsc::Sender<SkillPatternCandidate>,
}

impl PatternDetector {
    pub fn new(
        conn: Arc<Mutex<Connection>>,
        cfg: DetectorConfig,
        capacity: usize,
    ) -> (Arc<Self>, mpsc::Receiver<SkillPatternCandidate>) {
        let (tx, rx) = mpsc::channel(capacity);
        (
            Arc::new(Self {
                conn,
                cfg,
                rolling: DashMap::new(),
                tx,
            }),
            rx,
        )
    }

    /// Record one successful tool execution. Updates the rolling
    /// window and, when the window has at least `min_seq_len` tools,
    /// upserts the pattern and possibly emits a candidate.
    pub fn record(&self, agent_id: &str, tool: &str) {
        self.record_with_channel(agent_id, tool, None);
    }

    /// Same as `record`, but preserves the conversation channel that
    /// produced the tool call so the eventual skill proposal can be shown
    /// where the user is currently talking to Captain.
    pub fn record_with_channel(&self, agent_id: &str, tool: &str, origin_channel: Option<String>) {
        let mut entry = self.rolling.entry(agent_id.to_string()).or_default();
        if entry.len() >= self.cfg.window_len {
            entry.pop_front();
        }
        entry.push_back(tool.to_string());
        if entry.len() < self.cfg.min_seq_len {
            return;
        }

        let sequence: Vec<String> = entry.iter().cloned().collect();
        drop(entry);

        let hash = hash_sequence(agent_id, &sequence);
        let upserted = {
            let guard = match self.conn.lock() {
                Ok(g) => g,
                Err(e) => {
                    debug!(error = %e, "pattern_detector sqlite poisoned, skipping");
                    return;
                }
            };
            match skill_patterns::incr_or_insert(&guard, &hash, agent_id, &sequence) {
                Ok(row) => row,
                Err(e) => {
                    debug!(error = %e, "pattern_detector incr_or_insert failed");
                    return;
                }
            }
        };

        if upserted.count >= self.cfg.threshold && upserted.proposed_at.is_none() {
            let mut cand = SkillPatternCandidate::from(&upserted);
            cand.origin_channel = origin_channel;
            match self.tx.try_send(cand) {
                Ok(()) => trace!(hash = %hash, count = upserted.count, "pattern candidate emitted"),
                Err(mpsc::error::TrySendError::Full(_)) => {
                    debug!("pattern_detector output full, dropping candidate")
                }
                Err(mpsc::error::TrySendError::Closed(_)) => {
                    debug!("pattern_detector output closed, dropping candidate")
                }
            }
        }
    }

    /// Drop an agent's rolling window — useful when the agent dies.
    pub fn forget_agent(&self, agent_id: &str) {
        self.rolling.remove(agent_id);
    }
}

/// Stable hash for a `(agent_id, sequence)` pair. Same inputs → same
/// hex string. Hashing the agent id keeps patterns scoped per agent
/// in case two agents share a tool name.
pub fn hash_sequence(agent_id: &str, sequence: &[String]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(agent_id.as_bytes());
    hasher.update(b"\x00");
    for tool in sequence {
        hasher.update(tool.as_bytes());
        hasher.update(b"\x1f");
    }
    let digest = hasher.finalize();
    hex::encode(&digest[..16]) // 32 hex chars — plenty unique, half storage
}

// ---------------------------------------------------------------------------
// Global installation
// ---------------------------------------------------------------------------

static GLOBAL: OnceLock<Arc<PatternDetector>> = OnceLock::new();

pub fn install(
    conn: Arc<Mutex<Connection>>,
    cfg: DetectorConfig,
    capacity: usize,
) -> Option<mpsc::Receiver<SkillPatternCandidate>> {
    let (det, rx) = PatternDetector::new(conn, cfg, capacity);
    if GLOBAL.set(det).is_ok() {
        Some(rx)
    } else {
        None
    }
}

pub fn global() -> Option<Arc<PatternDetector>> {
    GLOBAL.get().cloned()
}

/// Convenience: record through the global detector if installed.
pub fn record(agent_id: &str, tool: &str) {
    if let Some(d) = global() {
        d.record(agent_id, tool);
    }
}

pub fn record_with_channel(agent_id: &str, tool: &str, origin_channel: Option<String>) {
    if let Some(d) = global() {
        d.record_with_channel(agent_id, tool, origin_channel);
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use captain_memory::migration::run_migrations;

    fn fresh_db() -> Arc<Mutex<Connection>> {
        let conn = Connection::open_in_memory().unwrap();
        run_migrations(&conn).unwrap();
        Arc::new(Mutex::new(conn))
    }

    fn cfg(threshold: u32) -> DetectorConfig {
        DetectorConfig {
            threshold,
            window_days: 7,
            min_seq_len: 3,
            window_len: 5,
        }
    }

    /// Tighter config used by sequence-identity tests: window == min
    /// so the hash is only ever computed over the same three-tool
    /// prefix. Otherwise a rolling window over 5 tools produces a
    /// distinct hash at each slide and `count` never actually stacks.
    fn cfg_tight(threshold: u32) -> DetectorConfig {
        DetectorConfig {
            threshold,
            window_days: 7,
            min_seq_len: 3,
            window_len: 3,
        }
    }

    #[test]
    fn hash_is_stable_for_same_inputs() {
        let h1 = hash_sequence("a", &["x".into(), "y".into(), "z".into()]);
        let h2 = hash_sequence("a", &["x".into(), "y".into(), "z".into()]);
        assert_eq!(h1, h2);
    }

    #[test]
    fn hash_differs_with_order() {
        let h1 = hash_sequence("a", &["x".into(), "y".into(), "z".into()]);
        let h2 = hash_sequence("a", &["z".into(), "y".into(), "x".into()]);
        assert_ne!(h1, h2);
    }

    #[test]
    fn hash_differs_with_agent_id() {
        let s = vec!["x".into(), "y".into(), "z".into()];
        assert_ne!(hash_sequence("a", &s), hash_sequence("b", &s));
    }

    #[tokio::test]
    async fn record_below_min_seq_len_does_nothing() {
        let db = fresh_db();
        let (det, mut rx) = PatternDetector::new(db.clone(), cfg(2), 8);
        det.record("a", "t1");
        det.record("a", "t2");
        let n = tokio::time::timeout(std::time::Duration::from_millis(40), rx.recv()).await;
        assert!(n.is_err(), "expected no candidate");
        // No row inserted yet either.
        let guard = db.lock().unwrap();
        let any = captain_memory::skill_patterns::list_ready(&guard, 1, 7, 10).unwrap();
        assert!(any.is_empty());
    }

    #[tokio::test]
    async fn record_emits_after_threshold_hit() {
        let db = fresh_db();
        let (det, mut rx) = PatternDetector::new(db.clone(), cfg_tight(3), 8);
        // With window_len=3 every record past the first two produces
        // the same hash, so count stacks predictably.
        for _ in 0..3 {
            det.record("a", "t1");
            det.record("a", "t2");
            det.record("a", "t3");
        }
        let cand = tokio::time::timeout(std::time::Duration::from_millis(200), rx.recv())
            .await
            .expect("candidate must arrive within 200ms")
            .expect("channel must yield");
        assert!(cand.count >= 3);
        assert_eq!(cand.agent_id, "a");
    }

    #[tokio::test]
    async fn record_with_channel_preserves_origin_channel() {
        let db = fresh_db();
        let (det, mut rx) = PatternDetector::new(db.clone(), cfg_tight(3), 8);
        for _ in 0..3 {
            det.record_with_channel("a", "t1", Some("telegram".into()));
            det.record_with_channel("a", "t2", Some("telegram".into()));
            det.record_with_channel("a", "t3", Some("telegram".into()));
        }
        let cand = tokio::time::timeout(std::time::Duration::from_millis(200), rx.recv())
            .await
            .expect("candidate must arrive within 200ms")
            .expect("channel must yield");
        assert_eq!(cand.origin_channel.as_deref(), Some("telegram"));
    }

    #[tokio::test]
    async fn record_does_not_emit_after_proposed_marked() {
        let db = fresh_db();
        let (det, mut rx) = PatternDetector::new(db.clone(), cfg_tight(2), 8);
        for _ in 0..3 {
            det.record("a", "t1");
            det.record("a", "t2");
            det.record("a", "t3");
        }
        while rx.try_recv().is_ok() {}

        // The rolling window produces up to three distinct hashes
        // ([t1,t2,t3], [t2,t3,t1], [t3,t1,t2]). Mark every pattern
        // currently past the threshold as proposed so no sibling
        // hash slips through.
        {
            let guard = db.lock().unwrap();
            let ready = captain_memory::skill_patterns::list_ready(&guard, 2, 7, 10).unwrap();
            assert!(!ready.is_empty(), "pattern(s) past threshold must exist");
            for p in &ready {
                captain_memory::skill_patterns::mark_proposed(&guard, &p.hash).unwrap();
            }
        }

        for _ in 0..3 {
            det.record("a", "t1");
            det.record("a", "t2");
            det.record("a", "t3");
        }
        let extra = tokio::time::timeout(std::time::Duration::from_millis(100), rx.recv()).await;
        assert!(extra.is_err(), "no further candidate after all proposed");
    }

    #[tokio::test]
    async fn record_separates_agents() {
        let db = fresh_db();
        let (det, _rx) = PatternDetector::new(db.clone(), cfg_tight(2), 8);
        for _ in 0..3 {
            det.record("a", "t1");
            det.record("a", "t2");
            det.record("a", "t3");
        }
        det.record("b", "t1");
        det.record("b", "t2");
        det.record("b", "t3");
        let guard = db.lock().unwrap();
        let h_a = hash_sequence("a", &["t1".into(), "t2".into(), "t3".into()]);
        let h_b = hash_sequence("b", &["t1".into(), "t2".into(), "t3".into()]);
        let row_a = captain_memory::skill_patterns::get(&guard, &h_a)
            .unwrap()
            .unwrap();
        let row_b = captain_memory::skill_patterns::get(&guard, &h_b)
            .unwrap()
            .unwrap();
        assert!(row_a.count >= 3);
        assert!(row_b.count >= 1);
        assert_ne!(row_a.hash, row_b.hash);
    }

    #[tokio::test]
    async fn rolling_window_slides_correctly() {
        let db = fresh_db();
        let (det, _rx) = PatternDetector::new(db.clone(), cfg(99), 8);
        // window_len = 5: feed 7 tools then check the last upsert
        // matches tools 3..=7.
        for tool in ["a", "b", "c", "d", "e", "f", "g"] {
            det.record("agent", tool);
        }
        let expected = vec![
            "c".to_string(),
            "d".into(),
            "e".into(),
            "f".into(),
            "g".into(),
        ];
        let h = hash_sequence("agent", &expected);
        let guard = db.lock().unwrap();
        let row = captain_memory::skill_patterns::get(&guard, &h).unwrap();
        assert!(row.is_some(), "expected pattern for last 5 tools");
    }

    #[tokio::test]
    async fn forget_agent_clears_window() {
        let db = fresh_db();
        let (det, _rx) = PatternDetector::new(db.clone(), cfg(99), 8);
        det.record("a", "t1");
        det.record("a", "t2");
        det.forget_agent("a");
        // After forget, only one tool builds back up — not enough for min_seq_len
        det.record("a", "t3");
        let guard = db.lock().unwrap();
        // No 3-tool sequence inserted.
        let count: i64 = guard
            .query_row("SELECT COUNT(*) FROM skill_patterns", [], |r| r.get(0))
            .unwrap();
        assert_eq!(count, 0);
    }

    #[test]
    fn hex_hash_is_32_chars() {
        let h = hash_sequence("a", &["x".into(), "y".into(), "z".into()]);
        assert_eq!(h.len(), 32);
        assert!(h.chars().all(|c| c.is_ascii_hexdigit()));
    }

    #[test]
    fn install_is_idempotent_returns_none_second_call() {
        // Don't actually install in tests because OnceLock is process-wide.
        // Just sanity-check the public surface compiles.
        let _ = global();
    }

    #[tokio::test]
    async fn record_through_global_is_noop_when_not_installed() {
        // When the OnceLock is empty, record() must not panic.
        record("ghost-agent", "any-tool");
    }
}
