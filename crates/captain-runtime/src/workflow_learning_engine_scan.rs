use std::collections::HashMap;

use captain_memory::workflow_learning::{WorkflowAnalysisOutcome, WorkflowAnalysisOutcomeStatus};
use captain_memory::workflow_learning_control::NewWorkflowProposal;
use captain_memory::workflow_learning_queue::WorkflowJobKind;

use crate::workflow_learning_analysis::{analyze_workflow_evidence, WorkflowRejectionReason};
use crate::workflow_learning_engine::{
    WorkflowLearningEngine, WorkflowLearningEngineError, WorkflowScanSummary,
};
use crate::workflow_learning_engine_support::{
    analysis_catalog, bounded_json, first_group_source, new_group_job, proposal_id_for,
    should_defer,
};

impl WorkflowLearningEngine {
    pub fn scan_once(
        &self,
        now_unix_ms: i64,
    ) -> Result<WorkflowScanSummary, WorkflowLearningEngineError> {
        let evidence = self
            .episodes
            .list_pending_evidence(self.config.scan_limit)?;
        let mut summary = WorkflowScanSummary {
            episodes_seen: evidence.len(),
            ..Default::default()
        };
        if evidence.is_empty() {
            return Ok(summary);
        }
        let proposals = self.control.list(None, 1_000)?;
        let (catalog, proposal_by_signature) = analysis_catalog(&proposals);
        let evidence_sources = evidence
            .iter()
            .map(|item| {
                (
                    item.episode.id.clone(),
                    (
                        item.episode.agent_id.clone(),
                        item.episode.origin_channel.clone(),
                    ),
                )
            })
            .collect::<HashMap<_, _>>();
        let batch = analyze_workflow_evidence(evidence, &catalog);

        for rejected in batch.rejected_episodes {
            let result_json = bounded_json(&rejected)?;
            self.record_rejected(vec![rejected.episode_id], result_json, now_unix_ms)?;
            summary.rejected += 1;
        }

        let day_start = now_unix_ms.saturating_sub(86_400_000);
        let mut proposals_today = proposals
            .iter()
            .filter(|proposal| proposal.created_at_unix_ms >= day_start)
            .count() as u32;
        for group in batch.groups {
            if should_defer(&group) {
                summary.deferred += group.episode_ids.len();
                continue;
            }
            let group_json = bounded_json(&group)?;
            if group
                .reasons
                .contains(&WorkflowRejectionReason::DuplicatePending)
            {
                if let Some(existing) = proposal_by_signature.get(&group.signature) {
                    self.record_processed(
                        group.episode_ids,
                        group_json,
                        &existing.id,
                        now_unix_ms,
                    )?;
                    summary.linked_existing += group.occurrence_count;
                } else {
                    self.record_rejected(group.episode_ids, group_json, now_unix_ms)?;
                    summary.rejected += group.occurrence_count;
                }
                continue;
            }
            if !group.eligible {
                self.record_rejected(group.episode_ids, group_json, now_unix_ms)?;
                summary.rejected += group.occurrence_count;
                continue;
            }
            if proposals_today >= self.config.daily_proposal_limit {
                summary.deferred += group.occurrence_count;
                continue;
            }
            let source = first_group_source(&group, &evidence_sources)?;
            let proposal_id = proposal_id_for(&group);
            let proposal = NewWorkflowProposal {
                id: proposal_id.clone(),
                idempotency_key: format!("{proposal_id}:observed"),
                workflow_signature: group.signature.clone(),
                source_agent_id: source.0.clone(),
                origin_channel: source.1.clone(),
                evidence_json: group_json.clone(),
                created_at_unix_ms: now_unix_ms,
            };
            let analyze_job = new_group_job(
                &format!("{proposal_id}-analyze"),
                &proposal_id,
                WorkflowJobKind::Analyze,
                &group,
                now_unix_ms,
            )?;
            self.control
                .observe_and_enqueue_analysis(&proposal, &analyze_job)?;
            self.record_processed(group.episode_ids, group_json, &proposal_id, now_unix_ms)?;
            proposals_today += 1;
            summary.proposals_created += 1;
        }
        Ok(summary)
    }

    fn record_processed(
        &self,
        episode_ids: Vec<String>,
        result_json: String,
        proposal_id: &str,
        now_unix_ms: i64,
    ) -> Result<(), WorkflowLearningEngineError> {
        self.episodes
            .record_analysis_outcome(&WorkflowAnalysisOutcome {
                episode_ids,
                status: WorkflowAnalysisOutcomeStatus::Processed,
                result_json,
                proposal_id: Some(proposal_id.to_string()),
                recorded_at_unix_ms: now_unix_ms,
            })?;
        Ok(())
    }

    fn record_rejected(
        &self,
        episode_ids: Vec<String>,
        result_json: String,
        now_unix_ms: i64,
    ) -> Result<(), WorkflowLearningEngineError> {
        self.episodes
            .record_analysis_outcome(&WorkflowAnalysisOutcome {
                episode_ids,
                status: WorkflowAnalysisOutcomeStatus::Rejected,
                result_json,
                proposal_id: None,
                recorded_at_unix_ms: now_unix_ms,
            })?;
        Ok(())
    }
}
