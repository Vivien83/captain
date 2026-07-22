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
//! Skill Learning V2 now owns procedural workflow detection from durable tool
//! episodes. Reflection candidates flow to declarative memory unconditionally;
//! this module remains a thin, bounded channel between reflection and memory.

use tokio::sync::mpsc;
use tracing::debug;

use crate::reflection_job::ReflectionBatch;

/// Every reflection candidate remains declarative memory. Procedural learning
/// is derived independently from durable workflow episodes.
pub fn retain_declarative_batch(batch: ReflectionBatch) -> Option<ReflectionBatch> {
    if batch.candidates.is_empty() {
        None
    } else {
        Some(batch)
    }
}

pub fn spawn_router(
    mut rx: mpsc::Receiver<ReflectionBatch>,
    output_capacity: usize,
) -> (tokio::task::JoinHandle<()>, mpsc::Receiver<ReflectionBatch>) {
    let (memory_tx, memory_rx) = mpsc::channel(output_capacity);

    let handle = tokio::spawn(async move {
        while let Some(batch) = rx.recv().await {
            if let Some(batch) = retain_declarative_batch(batch) {
                if let Err(e) = memory_tx.try_send(batch) {
                    debug!(error = %e, "cognitive_router: memory output full");
                }
            }
        }
    });

    (handle, memory_rx)
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
        let memory = retain_declarative_batch(batch(vec![cand(Some("skill"))]));
        assert_eq!(memory.unwrap().candidates.len(), 1);
    }

    #[test]
    fn declarative_candidates_continue_to_memory() {
        let mut c = cand(Some("info"));
        c.subject = "user".into();
        c.predicate = "prefers".into();
        let memory = retain_declarative_batch(batch(vec![c]));
        assert_eq!(memory.unwrap().candidates.len(), 1);
    }

    #[test]
    fn empty_batch_produces_no_memory_output() {
        let memory = retain_declarative_batch(batch(Vec::new()));
        assert!(memory.is_none());
    }

    #[tokio::test]
    async fn router_never_enqueues_a_skill_proposal() {
        let (tx, rx) = mpsc::channel(4);
        let (_h, mut mem_rx) = spawn_router(rx, 4);

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
    }
}
