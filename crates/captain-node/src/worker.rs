//! Durable local worker that binds Node inbox decisions to runtime execution.

use crate::{
    AuthorizedNodeRun, NodeExecutionAuthorization, NodeExecutionPolicy, NodeRailError,
    NodeRailStore, NodeReviewedTool, NodeRunDisposition,
};
use captain_types::approval::RiskLevel;
use captain_wire::{
    hub_protocol::{RunApprovalRequest, RunRejection, RunTerminalStatus},
    HubNodeMessage, RunCompletion, RunEffect, RunLease,
};
use futures::future::BoxFuture;
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    fmt,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    time::Duration,
};
use thiserror::Error;
use tokio::sync::Notify;

const MAX_RUNNING_NODE_RUNS: usize = 4;
const MAX_CLAIMABLE_PAGE: usize = 64;
const MAX_APPROVAL_WINDOW_MS: i64 = 15 * 60 * 1_000;
const MAX_RESULT_CONTENT_BYTES: usize = 1_048_576;
const CANCELLATION_GRACE: Duration = Duration::from_secs(2);

#[derive(Clone, PartialEq, Eq)]
pub struct NodeToolReview {
    reviewed: NodeReviewedTool,
    action_digest: String,
    approval_required: bool,
    risk_level: RiskLevel,
    action_summary: String,
}

impl NodeToolReview {
    pub fn new(
        reviewed: NodeReviewedTool,
        action_digest: impl Into<String>,
        approval_required: bool,
        risk_level: RiskLevel,
        action_summary: impl Into<String>,
    ) -> Result<Self, NodeWorkerError> {
        let action_digest = action_digest.into();
        let action_summary = action_summary.into();
        if action_digest.len() != 64
            || !action_digest.bytes().all(|byte| byte.is_ascii_hexdigit())
            || action_summary.is_empty()
            || action_summary.len() > 1_024
            || action_summary
                .chars()
                .any(|ch| ch.is_control() && !matches!(ch, '\n' | '\r' | '\t'))
            || looks_like_raw_local_path(&action_summary)
        {
            return Err(NodeWorkerError::DriverContract);
        }
        Ok(Self {
            reviewed,
            action_digest,
            approval_required,
            risk_level,
            action_summary,
        })
    }

    pub fn reviewed(&self) -> &NodeReviewedTool {
        &self.reviewed
    }

    pub fn action_digest(&self) -> &str {
        &self.action_digest
    }

    pub const fn approval_required(&self) -> bool {
        self.approval_required
    }
}

impl fmt::Debug for NodeToolReview {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("NodeToolReview")
            .field("reviewed", &self.reviewed)
            .field("action_digest", &self.action_digest)
            .field("approval_required", &self.approval_required)
            .field("risk_level", &self.risk_level)
            .field("action_summary", &"[REDACTED]")
            .finish()
    }
}

#[derive(Clone, PartialEq, Eq)]
pub struct NodeToolExecutionOutput {
    succeeded: bool,
    content: String,
    total_output_bytes: u64,
    capped: bool,
    redacted: bool,
}

impl NodeToolExecutionOutput {
    pub fn new(
        succeeded: bool,
        content: impl Into<String>,
        total_output_bytes: u64,
        capped: bool,
        redacted: bool,
    ) -> Result<Self, NodeWorkerError> {
        let content = content.into();
        if content.len() > MAX_RESULT_CONTENT_BYTES
            || total_output_bytes < content.len() as u64
            || (capped && total_output_bytes <= content.len() as u64)
            || (!capped && !redacted && total_output_bytes != content.len() as u64)
            || content
                .chars()
                .any(|ch| ch.is_control() && !matches!(ch, '\n' | '\r' | '\t'))
            || looks_like_raw_local_path(&content)
        {
            return Err(NodeWorkerError::DriverContract);
        }
        Ok(Self {
            succeeded,
            content,
            total_output_bytes,
            capped,
            redacted,
        })
    }

    pub const fn succeeded(&self) -> bool {
        self.succeeded
    }

    pub fn content(&self) -> &str {
        &self.content
    }

    pub const fn total_output_bytes(&self) -> u64 {
        self.total_output_bytes
    }

    pub const fn capped(&self) -> bool {
        self.capped
    }

    pub const fn redacted(&self) -> bool {
        self.redacted
    }

    fn fixed_failure(content: &'static str) -> Self {
        Self {
            succeeded: false,
            content: content.to_string(),
            total_output_bytes: content.len() as u64,
            capped: false,
            redacted: false,
        }
    }
}

impl fmt::Debug for NodeToolExecutionOutput {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("NodeToolExecutionOutput")
            .field("succeeded", &self.succeeded)
            .field("content", &"[REDACTED]")
            .field("total_output_bytes", &self.total_output_bytes)
            .field("stored_output_bytes", &self.content.len())
            .field("capped", &self.capped)
            .field("redacted", &self.redacted)
            .finish()
    }
}

#[derive(Clone, Default)]
pub struct NodeRunCancellation {
    inner: Arc<NodeRunCancellationInner>,
}

#[derive(Default)]
struct NodeRunCancellationInner {
    requested: AtomicBool,
    notify: Notify,
}

impl NodeRunCancellation {
    pub fn is_requested(&self) -> bool {
        self.inner.requested.load(Ordering::SeqCst)
    }

    pub async fn requested(&self) {
        loop {
            let mut notified = Box::pin(self.inner.notify.notified());
            notified.as_mut().enable();
            if self.is_requested() {
                return;
            }
            notified.await;
        }
    }

    fn request(&self) {
        if !self.inner.requested.swap(true, Ordering::SeqCst) {
            self.inner.notify.notify_waiters();
        }
    }
}

impl fmt::Debug for NodeRunCancellation {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("NodeRunCancellation")
            .field("requested", &self.is_requested())
            .finish()
    }
}

pub trait NodeToolDriver: Send + Sync + 'static {
    fn review(&self, lease: &RunLease) -> Result<NodeToolReview, RunRejection>;

    fn execute(
        self: Arc<Self>,
        run: AuthorizedNodeRun,
        approved_action_digest: Option<String>,
        cancellation: NodeRunCancellation,
    ) -> BoxFuture<'static, NodeToolExecutionOutput>;
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct NodeWorkerCycle {
    pub applied_inbound: usize,
    pub launched: usize,
    pub completed: usize,
    pub cancelled: usize,
    pub rejected_before_effect: usize,
}

impl NodeWorkerCycle {
    fn merge(&mut self, other: Self) {
        self.applied_inbound += other.applied_inbound;
        self.launched += other.launched;
        self.completed += other.completed;
        self.cancelled += other.cancelled;
        self.rejected_before_effect += other.rejected_before_effect;
    }
}

struct RunningNodeRun {
    claim_id: String,
    lease: RunLease,
    cancellation: NodeRunCancellation,
    task: tokio::task::JoinHandle<NodeToolExecutionOutput>,
}

impl fmt::Debug for RunningNodeRun {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("RunningNodeRun")
            .field("run_id", &self.lease.run_id)
            .field("attempt", &self.lease.attempt)
            .field("effect", &self.lease.effect)
            .field("claim_id", &"[REDACTED]")
            .field("cancellation", &self.cancellation)
            .field("finished", &self.task.is_finished())
            .finish()
    }
}

pub struct NodeWorker<D: NodeToolDriver + ?Sized> {
    rail: NodeRailStore,
    policy: NodeExecutionPolicy,
    driver: Arc<D>,
    running: BTreeMap<(String, u32), RunningNodeRun>,
}

impl<D: NodeToolDriver + ?Sized> NodeWorker<D> {
    pub fn new(rail: NodeRailStore, policy: NodeExecutionPolicy, driver: Arc<D>) -> Self {
        Self {
            rail,
            policy,
            driver,
            running: BTreeMap::new(),
        }
    }

    pub fn rail(&self) -> &NodeRailStore {
        &self.rail
    }

    pub fn running_len(&self) -> usize {
        self.running.len()
    }

    /// Applies durable Hub input first, then commits finished local work, and
    /// only then claims newly ready work. This ordering makes a queued cancel
    /// authoritative over an uncommitted local result.
    pub async fn advance(&mut self, now_ms: i64) -> Result<NodeWorkerCycle, NodeWorkerError> {
        let mut cycle = self.apply_pending_inbound(now_ms).await?;
        cycle.merge(self.collect_finished(now_ms).await?);
        cycle.merge(self.launch_claimable(now_ms)?);
        Ok(cycle)
    }

    async fn apply_pending_inbound(
        &mut self,
        now_ms: i64,
    ) -> Result<NodeWorkerCycle, NodeWorkerError> {
        let mut cycle = NodeWorkerCycle::default();
        loop {
            let Some(record) = self.rail.pending_inbound(1)?.into_iter().next() else {
                break;
            };
            let sequence = record.envelope.sequence;
            match record.envelope.message {
                HubNodeMessage::RunOffer(lease) => {
                    let disposition = self.disposition_for_offer(&lease, now_ms);
                    self.rail.apply_run_offer(sequence, &disposition, now_ms)?;
                }
                HubNodeMessage::RunApprovalDecision(_) => {
                    self.rail.apply_run_approval_decision(sequence, now_ms)?;
                }
                HubNodeMessage::CancelRun {
                    ref run_id,
                    attempt,
                    ..
                } => {
                    let outcome = self.rail.apply_cancel_run(sequence, now_ms)?;
                    if outcome.signal_runner {
                        self.cancel_running(run_id, attempt, now_ms).await?;
                        cycle.cancelled += 1;
                        cycle.completed += 1;
                    }
                }
                _ => return Err(NodeWorkerError::UnexpectedInbound),
            }
            cycle.applied_inbound += 1;
        }
        Ok(cycle)
    }

    fn disposition_for_offer(&self, lease: &RunLease, now_ms: i64) -> NodeRunDisposition {
        let review = match self.driver.review(lease) {
            Ok(review) => review,
            Err(rejection) => {
                return NodeRunDisposition::Reject(valid_rejection_or_fallback(lease, rejection))
            }
        };
        let authorized = match self.policy.authorize(lease, review.reviewed()) {
            NodeExecutionAuthorization::Authorized(run) => run,
            NodeExecutionAuthorization::Rejected(rejection) => {
                return NodeRunDisposition::Reject(rejection)
            }
        };
        if authorized.lease() != lease {
            return NodeRunDisposition::Reject(fallback_rejection(
                lease,
                "authorization_contract_mismatch",
                false,
            ));
        }
        if !requires_approval(&review, lease.effect) {
            return NodeRunDisposition::Accept;
        }
        let expires_at_ms = lease
            .lease_expires_at_ms
            .min(now_ms.saturating_add(MAX_APPROVAL_WINDOW_MS));
        let request = RunApprovalRequest {
            run_id: lease.run_id.clone(),
            attempt: lease.attempt,
            approval_id: uuid::Uuid::new_v4().hyphenated().to_string(),
            action_digest: review.action_digest,
            action_summary: review.action_summary,
            risk_level: review.risk_level,
            expires_at_ms,
            path_policy_applied: true,
        };
        if expires_at_ms <= now_ms || request.validate().is_err() {
            NodeRunDisposition::Reject(fallback_rejection(
                lease,
                "approval_contract_invalid",
                false,
            ))
        } else {
            NodeRunDisposition::RequireApproval(request)
        }
    }

    fn launch_claimable(&mut self, now_ms: i64) -> Result<NodeWorkerCycle, NodeWorkerError> {
        let mut cycle = NodeWorkerCycle::default();
        if self.running.len() >= MAX_RUNNING_NODE_RUNS {
            return Ok(cycle);
        }
        for run in self.rail.claimable_runs(MAX_CLAIMABLE_PAGE)? {
            if self.running.len() >= MAX_RUNNING_NODE_RUNS {
                break;
            }
            let key = (run.lease.run_id.clone(), run.lease.attempt);
            if self.running.contains_key(&key) {
                continue;
            }
            let (authorized, approved_digest) = match self.authorize_before_claim(&run.lease) {
                Ok(authorized) => authorized,
                Err(rejection) => {
                    self.rail.reject_run_before_effect(
                        &run.lease.run_id,
                        run.lease.attempt,
                        &rejection,
                        now_ms,
                    )?;
                    cycle.rejected_before_effect += 1;
                    continue;
                }
            };
            let claim = match self
                .rail
                .claim_run(&run.lease.run_id, run.lease.attempt, now_ms)
            {
                Ok(claim) => claim,
                Err(NodeRailError::RunNotReady | NodeRailError::RunCancellationPending) => continue,
                Err(error) => return Err(error.into()),
            };
            let cancellation = NodeRunCancellation::default();
            let task = tokio::spawn(Arc::clone(&self.driver).execute(
                authorized,
                approved_digest,
                cancellation.clone(),
            ));
            self.running.insert(
                key,
                RunningNodeRun {
                    claim_id: claim.claim_id,
                    lease: claim.run.lease,
                    cancellation,
                    task,
                },
            );
            cycle.launched += 1;
        }
        Ok(cycle)
    }

    fn authorize_before_claim(
        &self,
        lease: &RunLease,
    ) -> Result<(AuthorizedNodeRun, Option<String>), RunRejection> {
        let review = self
            .driver
            .review(lease)
            .map_err(|rejection| valid_rejection_or_fallback(lease, rejection))?;
        let authorized = match self.policy.authorize(lease, review.reviewed()) {
            NodeExecutionAuthorization::Authorized(run) => run,
            NodeExecutionAuthorization::Rejected(rejection) => return Err(rejection),
        };
        let approved_digest = self
            .rail
            .approved_action_digest(&lease.run_id, lease.attempt)
            .map_err(|_| fallback_rejection(lease, "approval_state_unavailable", true))?;
        if requires_approval(&review, lease.effect) {
            if approved_digest.as_deref() != Some(review.action_digest()) {
                return Err(fallback_rejection(lease, "approval_digest_mismatch", false));
            }
        } else if approved_digest.is_some() {
            return Err(fallback_rejection(
                lease,
                "approval_contract_mismatch",
                false,
            ));
        }
        Ok((authorized, approved_digest))
    }

    async fn collect_finished(&mut self, now_ms: i64) -> Result<NodeWorkerCycle, NodeWorkerError> {
        let finished = self
            .running
            .iter()
            .filter_map(|(key, running)| running.task.is_finished().then_some(key.clone()))
            .collect::<Vec<_>>();
        let mut cycle = NodeWorkerCycle::default();
        for key in finished {
            let RunningNodeRun {
                claim_id,
                lease,
                cancellation: _,
                task,
            } = self
                .running
                .remove(&key)
                .ok_or(NodeWorkerError::RunnerStateConflict)?;
            let output = match task.await {
                Ok(output) => output,
                Err(_) if lease.effect == RunEffect::ReadOnly => {
                    NodeToolExecutionOutput::fixed_failure(
                        "Local read execution stopped unexpectedly.",
                    )
                }
                Err(_) => {
                    self.complete_fixed(
                        &claim_id,
                        &lease,
                        RunTerminalStatus::Uncertain,
                        "Local execution stopped after its effect claim; the outcome is uncertain.",
                        now_ms,
                    )?;
                    cycle.completed += 1;
                    continue;
                }
            };
            self.complete_output(&claim_id, &lease, output, now_ms)?;
            cycle.completed += 1;
        }
        Ok(cycle)
    }

    async fn cancel_running(
        &mut self,
        run_id: &str,
        attempt: u32,
        now_ms: i64,
    ) -> Result<(), NodeWorkerError> {
        let key = (run_id.to_string(), attempt);
        let RunningNodeRun {
            claim_id,
            lease,
            cancellation,
            mut task,
        } = self
            .running
            .remove(&key)
            .ok_or(NodeWorkerError::RunnerStateConflict)?;
        cancellation.request();
        if tokio::time::timeout(CANCELLATION_GRACE, &mut task)
            .await
            .is_err()
        {
            task.abort();
            let _ = task.await;
        }
        let (status, content) = if lease.effect == RunEffect::ReadOnly {
            (
                RunTerminalStatus::Cancelled,
                "Local read execution was cancelled by the operator.",
            )
        } else {
            (
                RunTerminalStatus::Uncertain,
                "Cancellation arrived after the local effect claim; the outcome is uncertain.",
            )
        };
        self.complete_fixed(&claim_id, &lease, status, content, now_ms)
    }

    fn complete_output(
        &self,
        claim_id: &str,
        lease: &RunLease,
        output: NodeToolExecutionOutput,
        now_ms: i64,
    ) -> Result<(), NodeWorkerError> {
        let status = if output.succeeded {
            RunTerminalStatus::Succeeded
        } else {
            RunTerminalStatus::Failed
        };
        let completion = completion_for_output(lease, status, output);
        self.rail.complete_run(claim_id, &completion, now_ms)?;
        Ok(())
    }

    fn complete_fixed(
        &self,
        claim_id: &str,
        lease: &RunLease,
        status: RunTerminalStatus,
        content: &'static str,
        now_ms: i64,
    ) -> Result<(), NodeWorkerError> {
        let output = NodeToolExecutionOutput::fixed_failure(content);
        let completion = completion_for_output(lease, status, output);
        self.rail.complete_run(claim_id, &completion, now_ms)?;
        Ok(())
    }
}

impl<D: NodeToolDriver + ?Sized> Drop for NodeWorker<D> {
    fn drop(&mut self) {
        for running in self.running.values() {
            running.cancellation.request();
            running.task.abort();
        }
    }
}

impl<D: NodeToolDriver + ?Sized> fmt::Debug for NodeWorker<D> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("NodeWorker")
            .field("rail", &self.rail)
            .field("policy", &self.policy)
            .field("driver", &"[REDACTED]")
            .field("running", &self.running)
            .finish()
    }
}

fn completion_for_output(
    lease: &RunLease,
    status: RunTerminalStatus,
    output: NodeToolExecutionOutput,
) -> RunCompletion {
    let stored_output_bytes = output.content.len() as u64;
    RunCompletion {
        run_id: lease.run_id.clone(),
        attempt: lease.attempt,
        status,
        result_sha256: hex::encode(Sha256::digest(output.content.as_bytes())),
        result_content: output.content,
        total_output_bytes: output.total_output_bytes,
        stored_output_bytes,
        capped: output.capped,
        redacted: output.redacted,
        path_policy_applied: true,
    }
}

fn requires_approval(review: &NodeToolReview, effect: RunEffect) -> bool {
    review.approval_required || effect == RunEffect::ExternalEffect
}

fn valid_rejection_or_fallback(lease: &RunLease, rejection: RunRejection) -> RunRejection {
    if rejection.run_id == lease.run_id
        && rejection.attempt == lease.attempt
        && rejection.validate().is_ok()
        && !looks_like_raw_local_path(&rejection.message)
    {
        rejection
    } else {
        fallback_rejection(lease, "driver_review_failed", false)
    }
}

fn fallback_rejection(lease: &RunLease, code: &str, retryable: bool) -> RunRejection {
    RunRejection {
        run_id: lease.run_id.clone(),
        attempt: lease.attempt,
        code: code.to_string(),
        message: "The local Node could not authorize the exact offered run".to_string(),
        retryable,
        path_policy_applied: true,
    }
}

fn looks_like_raw_local_path(value: &str) -> bool {
    value
        .split_whitespace()
        .flat_map(|token| {
            token.split(|ch: char| {
                matches!(
                    ch,
                    '"' | '\'' | '`' | '(' | ')' | '[' | ']' | '{' | '}' | ',' | ';' | '='
                )
            })
        })
        .map(|token| token.trim_matches(|ch: char| matches!(ch, '.' | '!')))
        .any(|token| {
            token.starts_with('/')
                || token.starts_with("~/")
                || token.starts_with("~\\")
                || token.starts_with("\\\\")
                || (token.len() >= 3
                    && token.as_bytes()[0].is_ascii_alphabetic()
                    && token.as_bytes()[1] == b':'
                    && matches!(token.as_bytes()[2], b'/' | b'\\'))
        })
}

#[derive(Debug, Error)]
pub enum NodeWorkerError {
    #[error("Node durable worker state is unavailable")]
    Rail(#[from] NodeRailError),
    #[error("Node tool driver violated its sanitized contract")]
    DriverContract,
    #[error("Node runner state conflicts with the durable ledger")]
    RunnerStateConflict,
    #[error("Node received an unexpected Hub message on the execution inbox")]
    UnexpectedInbound,
}

#[cfg(test)]
#[path = "worker_tests.rs"]
mod tests;
