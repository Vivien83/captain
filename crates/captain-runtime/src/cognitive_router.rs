//! Cognitive learning router.
//!
//! Used to split reflection output into declarative memory vs. "procedural"
//! candidates auto-converted into skill proposals. That split was removed
//! (2026-07-04): `is_procedural_candidate` fired on a bare keyword match
//! ("workflow", "skill", "recipe"...) anywhere in a candidate's subject or
//! predicate, with no requirement that an actual tool-usage pattern was
//! ever observed — any conversation merely *mentioning* one of those words
//! could produce a skill proposal with an empty tool trace and a
//! confidence copied straight from reflection_job's fixed 0.5/0.7/0.9
//! bucket scale. Every proposal from this path looked identical (~70%,
//! no tools listed) and many were unrelated to anything Captain actually
//! did repeatedly.
//!
//! `pattern_detector.rs` → `skill_proposer.rs` remains the only source of
//! skill proposals: it only fires on an actually-repeated, observed tool
//! call sequence, and asks a dedicated LLM judge for a real per-case
//! confidence. All reflection candidates now flow to declarative memory
//! unconditionally; this module still exists as the wiring point kernel
//! startup spawns, kept as a thin pass-through so callers don't change.

use rusqlite::Connection;
use std::sync::{Arc, Mutex as StdMutex};
use tokio::sync::mpsc;
use tracing::debug;

use crate::proposal_policy::ProposalPolicy;
use crate::reflection_job::ReflectionBatch;
use crate::skill_proposer::SkillProposal;

/// No longer splits anything — every candidate flows to declarative
/// memory. Kept for the `spawn_router` call site; always returns an
/// empty proposal list.
pub fn split_batch(
    batch: ReflectionBatch,
    _language: &str,
) -> (Option<ReflectionBatch>, Vec<SkillProposal>) {
    if batch.candidates.is_empty() {
        (None, Vec::new())
    } else {
        (Some(batch), Vec::new())
    }
}

pub fn spawn_router(
    mut rx: mpsc::Receiver<ReflectionBatch>,
    _policy: Option<Arc<ProposalPolicy>>,
    _conn: Arc<StdMutex<Connection>>,
    language: String,
    output_capacity: usize,
) -> (
    tokio::task::JoinHandle<()>,
    mpsc::Receiver<ReflectionBatch>,
    mpsc::Receiver<captain_memory::skill_proposals::Proposal>,
) {
    let (memory_tx, memory_rx) = mpsc::channel(output_capacity);
    // Never sent on — split_batch no longer produces proposals — but kept
    // so the kernel wiring that reads from this channel doesn't change.
    let (_proposal_tx, proposal_rx) = mpsc::channel(output_capacity);

    let handle = tokio::spawn(async move {
        while let Some(batch) = rx.recv().await {
            let (memory_batch, _proposals) = split_batch(batch, &language);
            if let Some(batch) = memory_batch {
                if let Err(e) = memory_tx.try_send(batch) {
                    debug!(error = %e, "cognitive_router: memory output full");
                }
            }
        }
    });

    (handle, memory_rx, proposal_rx)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::outcome_detector::Outcome;
    use crate::reflection_job::MemoryCandidate;

    fn cand(category: Option<&str>) -> MemoryCandidate {
        MemoryCandidate {
            wing: "learnings".into(),
            room: "general".into(),
            subject: "deploy workflow".into(),
            predicate: "uses".into(),
            object: "Build, smoke test, then publish a release note".into(),
            confidence: 0.9,
            category: category.map(str::to_string),
        }
    }

    fn batch(candidates: Vec<MemoryCandidate>) -> ReflectionBatch {
        ReflectionBatch {
            outcome: Outcome::ConversationIdle,
            agent_id: "captain".into(),
            candidates,
            channel: Some("telegram".into()),
        }
    }

    /// Regression: a candidate whose subject/predicate merely mentions
    /// "workflow" used to be misclassified as procedural and turned into
    /// a skill proposal with no tool trace. It must now just be memory.
    #[test]
    fn keyword_bearing_candidates_no_longer_become_skill_proposals() {
        let (memory, proposals) = split_batch(batch(vec![cand(Some("skill"))]), "fr");
        assert!(proposals.is_empty());
        assert_eq!(memory.unwrap().candidates.len(), 1);
    }

    #[test]
    fn declarative_candidates_continue_to_memory() {
        let mut c = cand(Some("info"));
        c.subject = "user".into();
        c.predicate = "prefers".into();
        let (memory, proposals) = split_batch(batch(vec![c]), "fr");
        assert!(proposals.is_empty());
        assert_eq!(memory.unwrap().candidates.len(), 1);
    }

    #[test]
    fn empty_batch_produces_no_memory_output() {
        let (memory, proposals) = split_batch(batch(Vec::new()), "fr");
        assert!(memory.is_none());
        assert!(proposals.is_empty());
    }

    #[tokio::test]
    async fn router_never_enqueues_a_skill_proposal() {
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        captain_memory::migration::run_migrations(&conn).unwrap();
        let conn = Arc::new(StdMutex::new(conn));
        let policy = Arc::new(ProposalPolicy::new(crate::proposal_policy::PolicyConfig {
            max_per_day: 10,
            min_confidence: 0.5,
        }));
        let (tx, rx) = mpsc::channel(4);
        let (_h, mut mem_rx, mut prop_rx) =
            spawn_router(rx, Some(policy), conn, "fr".to_string(), 4);

        let mut fact = cand(Some("info"));
        fact.subject = "user".into();
        fact.predicate = "prefers".into();
        tx.send(batch(vec![cand(Some("solution")), fact]))
            .await
            .unwrap();
        drop(tx);

        let mem = tokio::time::timeout(std::time::Duration::from_millis(200), mem_rx.recv())
            .await
            .expect("memory batch should arrive")
            .expect("memory batch");
        assert_eq!(mem.candidates.len(), 2);

        // No proposal ever arrives; the channel just closes with the router.
        assert!(prop_rx.recv().await.is_none());
    }
}
