//! Versioned Hub-to-Node protocol contract.
//!
//! This module contains wire types and deterministic validation only. Transport,
//! authentication, persistence, and execution live in their owning crates.

use captain_types::approval::{
    is_valid_approval_action_digest, normalize_approval_reason, ApprovalDecision, RiskLevel,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::BTreeSet;
use std::fmt;
use thiserror::Error;

/// First production Hub-to-Node protocol supported by Captain.
pub const HUB_NODE_PROTOCOL_VERSION: ProtocolVersion = ProtocolVersion { major: 1, minor: 0 };

const MAX_IDENTIFIER_LEN: usize = 128;
const MAX_LABEL_LEN: usize = 160;
const MAX_TOOL_FAMILIES: usize = 256;
const MAX_WORKSPACES: usize = 128;
const MAX_TOOL_INPUT_BYTES: usize = 1024 * 1024;
const MAX_RESULT_CONTENT_BYTES: usize = 1024 * 1024;
const MAX_ACTIVE_RUNS: usize = 256;
const MAX_PROGRESS_MESSAGE_BYTES: usize = 4096;
const MAX_PROTOCOL_ERROR_BYTES: usize = 2048;
const MAX_RUN_DECISION_MESSAGE_BYTES: usize = 2048;
const MAX_APPROVAL_SUMMARY_BYTES: usize = 512;

/// Additive protocol version. A major mismatch is always incompatible.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ProtocolVersion {
    pub major: u16,
    pub minor: u16,
}

impl ProtocolVersion {
    /// Negotiate the highest mutually understood additive minor version.
    pub fn negotiate(self, peer: Self) -> Result<Self, ProtocolContractError> {
        if self.major == 0 || peer.major == 0 || self.major != peer.major {
            return Err(ProtocolContractError::IncompatibleVersion { local: self, peer });
        }
        Ok(Self {
            major: self.major,
            minor: self.minor.min(peer.minor),
        })
    }
}

/// A paired device's role. A Hub is the authority and is not represented as a
/// remote execution device in this protocol.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DeviceRole {
    Client,
    Node,
}

/// Outbound transports a Node can use to reach its Hub on HTTPS port 443.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NodeTransport {
    WebSocket,
    HttpStream,
    LongPoll,
}

/// A logical workspace known by the Hub. The local filesystem path is
/// deliberately absent and must remain confined to the Node.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LogicalWorkspace {
    pub workspace_id: String,
    pub label: String,
    #[serde(default)]
    pub read_only: bool,
}

impl LogicalWorkspace {
    pub fn validate(&self) -> Result<(), ProtocolContractError> {
        validate_identifier("workspace_id", &self.workspace_id)?;
        validate_label("workspace label", &self.label)
    }
}

/// Capabilities announced by a device during the authenticated handshake.
/// A Client announces transport/UI support but no execution family or
/// workspace; a Node may announce both under explicit grants.
/// Unknown JSON fields are intentionally ignored for additive evolution.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CapabilityDescriptor {
    pub captain_version: String,
    pub platform: String,
    pub transports: Vec<NodeTransport>,
    #[serde(default)]
    pub tool_families: Vec<String>,
    #[serde(default)]
    pub workspaces: Vec<LogicalWorkspace>,
    #[serde(default)]
    pub supports_streaming_output: bool,
}

impl CapabilityDescriptor {
    pub fn validate(&self) -> Result<(), ProtocolContractError> {
        validate_label("captain_version", &self.captain_version)?;
        validate_identifier("platform", &self.platform)?;
        if self.transports.is_empty() {
            return Err(ProtocolContractError::MissingTransport);
        }
        if self.tool_families.len() > MAX_TOOL_FAMILIES {
            return Err(ProtocolContractError::LimitExceeded("tool_families"));
        }
        if self.workspaces.len() > MAX_WORKSPACES {
            return Err(ProtocolContractError::LimitExceeded("workspaces"));
        }
        reject_duplicates(
            "transport",
            self.transports.iter().map(|item| format!("{item:?}")),
        )?;
        for family in &self.tool_families {
            validate_identifier("tool family", family)?;
        }
        reject_duplicates("tool family", self.tool_families.iter().cloned())?;
        for workspace in &self.workspaces {
            workspace.validate()?;
        }
        reject_duplicates(
            "workspace",
            self.workspaces
                .iter()
                .map(|workspace| workspace.workspace_id.clone()),
        )
    }
}

/// Explicit execution authority granted by the Hub owner to one device.
/// It is always empty for a lightweight Client.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeviceGrant {
    #[serde(default)]
    pub workspace_ids: Vec<String>,
    #[serde(default)]
    pub tool_families: Vec<String>,
    #[serde(default)]
    pub allow_mutation: bool,
}

impl DeviceGrant {
    pub fn validate_shape(&self) -> Result<(), ProtocolContractError> {
        if self.workspace_ids.len() > MAX_WORKSPACES {
            return Err(ProtocolContractError::LimitExceeded("granted workspaces"));
        }
        if self.tool_families.len() > MAX_TOOL_FAMILIES {
            return Err(ProtocolContractError::LimitExceeded(
                "granted tool families",
            ));
        }
        reject_duplicates("granted workspace", self.workspace_ids.iter().cloned())?;
        reject_duplicates("granted tool family", self.tool_families.iter().cloned())?;
        for workspace_id in &self.workspace_ids {
            validate_identifier("granted workspace", workspace_id)?;
        }
        for family in &self.tool_families {
            validate_identifier("granted tool family", family)?;
        }
        Ok(())
    }

    pub fn validate_subset_of(&self, requested: &Self) -> Result<(), ProtocolContractError> {
        self.validate_shape()?;
        requested.validate_shape()?;
        let requested_workspaces = requested.workspace_ids.iter().collect::<BTreeSet<_>>();
        let requested_families = requested.tool_families.iter().collect::<BTreeSet<_>>();
        let exceeds_request = self
            .workspace_ids
            .iter()
            .any(|workspace| !requested_workspaces.contains(workspace))
            || self
                .tool_families
                .iter()
                .any(|family| !requested_families.contains(family))
            || (self.allow_mutation && !requested.allow_mutation);
        if exceeds_request {
            return Err(ProtocolContractError::GrantExceedsRequest);
        }
        Ok(())
    }

    pub fn validate_against(
        &self,
        capabilities: &CapabilityDescriptor,
    ) -> Result<(), ProtocolContractError> {
        capabilities.validate()?;
        self.validate_shape()?;

        let available_workspaces = capabilities
            .workspaces
            .iter()
            .map(|workspace| workspace.workspace_id.as_str())
            .collect::<BTreeSet<_>>();
        for workspace_id in &self.workspace_ids {
            if !available_workspaces.contains(workspace_id.as_str()) {
                return Err(ProtocolContractError::CapabilityNotAdvertised(
                    workspace_id.clone(),
                ));
            }
        }

        let available_families = capabilities
            .tool_families
            .iter()
            .map(String::as_str)
            .collect::<BTreeSet<_>>();
        for family in &self.tool_families {
            if !available_families.contains(family.as_str()) {
                return Err(ProtocolContractError::CapabilityNotAdvertised(
                    family.clone(),
                ));
            }
        }
        Ok(())
    }
}

/// Public pairing claim submitted by a Client or Node. It carries only a
/// digest of the credential generated locally by that device; the credential itself is
/// never sent in this claim or persisted by the Hub.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DevicePairingClaim {
    pub display_name: String,
    pub role: DeviceRole,
    pub platform: String,
    pub protocol_version: ProtocolVersion,
    pub credential_sha256: String,
    pub capabilities: CapabilityDescriptor,
    #[serde(default)]
    pub requested_grants: DeviceGrant,
}

impl DevicePairingClaim {
    pub fn validate(&self) -> Result<(), ProtocolContractError> {
        validate_label("display_name", &self.display_name)?;
        validate_identifier("platform", &self.platform)?;
        HUB_NODE_PROTOCOL_VERSION.negotiate(self.protocol_version)?;
        validate_sha256("credential_sha256", &self.credential_sha256)?;
        self.capabilities.validate()?;
        if self.platform != self.capabilities.platform {
            return Err(ProtocolContractError::DeviceMetadataMismatch("platform"));
        }
        if self.role == DeviceRole::Client
            && (!self.capabilities.workspaces.is_empty()
                || !self.capabilities.tool_families.is_empty())
        {
            return Err(ProtocolContractError::ClientAdvertisedExecution);
        }
        self.requested_grants.validate_against(&self.capabilities)?;
        Ok(())
    }
}

/// The side-effect class controls safe retry behavior after a transport loss.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RunEffect {
    ReadOnly,
    LocalMutation,
    ExternalEffect,
}

/// A leased unit of tool work offered by the Hub to one Node.
#[derive(Clone, PartialEq, Serialize, Deserialize)]
pub struct RunLease {
    pub run_id: String,
    pub attempt: u32,
    pub idempotency_key: String,
    pub workspace_id: String,
    pub tool_name: String,
    pub input: serde_json::Value,
    pub effect: RunEffect,
    pub lease_expires_at_ms: i64,
}

impl RunLease {
    pub fn validate(&self) -> Result<(), ProtocolContractError> {
        validate_identifier("run_id", &self.run_id)?;
        validate_identifier("idempotency_key", &self.idempotency_key)?;
        validate_identifier("workspace_id", &self.workspace_id)?;
        validate_identifier("tool_name", &self.tool_name)?;
        if self.attempt == 0 {
            return Err(ProtocolContractError::InvalidAttempt);
        }
        if !self.input.is_object() {
            return Err(ProtocolContractError::InvalidToolInput);
        }
        let input_bytes =
            serde_json::to_vec(&self.input).map_err(|_| ProtocolContractError::InvalidToolInput)?;
        if input_bytes.len() > MAX_TOOL_INPUT_BYTES {
            return Err(ProtocolContractError::LimitExceeded("tool input"));
        }
        if self.lease_expires_at_ms <= 0 {
            return Err(ProtocolContractError::InvalidLeaseExpiry);
        }
        Ok(())
    }
}

impl fmt::Debug for RunLease {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("RunLease")
            .field("run_id", &self.run_id)
            .field("attempt", &self.attempt)
            .field("idempotency_key", &self.idempotency_key)
            .field("workspace_id", &self.workspace_id)
            .field("tool_name", &self.tool_name)
            .field("input", &"[REDACTED]")
            .field("effect", &self.effect)
            .field("lease_expires_at_ms", &self.lease_expires_at_ms)
            .finish()
    }
}

/// Sanitized reason why a Node refused an offered run before any effect began.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RunRejection {
    pub run_id: String,
    pub attempt: u32,
    pub code: String,
    pub message: String,
    pub retryable: bool,
    pub path_policy_applied: bool,
}

impl RunRejection {
    pub fn validate(&self) -> Result<(), ProtocolContractError> {
        validate_run_attempt(&self.run_id, self.attempt)?;
        validate_identifier("run rejection code", &self.code)?;
        validate_text(
            "run rejection message",
            &self.message,
            MAX_RUN_DECISION_MESSAGE_BYTES,
        )?;
        require_path_policy(self.path_policy_applied)
    }
}

impl fmt::Debug for RunRejection {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("RunRejection")
            .field("run_id", &self.run_id)
            .field("attempt", &self.attempt)
            .field("code", &self.code)
            .field("message", &"[REDACTED]")
            .field("retryable", &self.retryable)
            .field("path_policy_applied", &self.path_policy_applied)
            .finish()
    }
}

/// A local guard has paused an offered run until the operator decides on the
/// exact action digest. Raw tool input and local filesystem paths are absent.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RunApprovalRequest {
    pub run_id: String,
    pub attempt: u32,
    pub approval_id: String,
    pub action_digest: String,
    pub action_summary: String,
    pub risk_level: RiskLevel,
    pub expires_at_ms: i64,
    pub path_policy_applied: bool,
}

impl RunApprovalRequest {
    pub fn validate(&self) -> Result<(), ProtocolContractError> {
        validate_run_attempt(&self.run_id, self.attempt)?;
        validate_identifier("approval_id", &self.approval_id)?;
        if !is_valid_approval_action_digest(&self.action_digest) {
            return Err(ProtocolContractError::InvalidDigest("action digest"));
        }
        validate_text(
            "approval action summary",
            &self.action_summary,
            MAX_APPROVAL_SUMMARY_BYTES,
        )?;
        if self.expires_at_ms <= 0 {
            return Err(ProtocolContractError::InvalidTimestamp);
        }
        require_path_policy(self.path_policy_applied)
    }
}

impl fmt::Debug for RunApprovalRequest {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("RunApprovalRequest")
            .field("run_id", &self.run_id)
            .field("attempt", &self.attempt)
            .field("approval_id", &self.approval_id)
            .field("action_digest", &self.action_digest)
            .field("action_summary", &"[REDACTED]")
            .field("risk_level", &self.risk_level)
            .field("expires_at_ms", &self.expires_at_ms)
            .field("path_policy_applied", &self.path_policy_applied)
            .finish()
    }
}

/// Operator decision sent by the Hub back to the Node that raised the exact
/// approval request. The Node remains the final policy enforcement boundary.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RunApprovalDecision {
    pub run_id: String,
    pub attempt: u32,
    pub approval_id: String,
    pub action_digest: String,
    pub decision: ApprovalDecision,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    pub decided_at_ms: i64,
}

impl RunApprovalDecision {
    pub fn validate(&self) -> Result<(), ProtocolContractError> {
        validate_run_attempt(&self.run_id, self.attempt)?;
        validate_identifier("approval_id", &self.approval_id)?;
        if !is_valid_approval_action_digest(&self.action_digest) {
            return Err(ProtocolContractError::InvalidDigest("action digest"));
        }
        if self.decided_at_ms <= 0 {
            return Err(ProtocolContractError::InvalidTimestamp);
        }
        let normalized = normalize_approval_reason(self.reason.as_deref())
            .map_err(|_| ProtocolContractError::InvalidLabel("approval reason"))?;
        if normalized != self.reason {
            return Err(ProtocolContractError::InvalidLabel("approval reason"));
        }
        Ok(())
    }
}

impl fmt::Debug for RunApprovalDecision {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("RunApprovalDecision")
            .field("run_id", &self.run_id)
            .field("attempt", &self.attempt)
            .field("approval_id", &self.approval_id)
            .field("action_digest", &self.action_digest)
            .field("decision", &self.decision)
            .field("reason", &self.reason.as_ref().map(|_| "[REDACTED]"))
            .field("decided_at_ms", &self.decided_at_ms)
            .finish()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RunTerminalStatus {
    Succeeded,
    Failed,
    Cancelled,
    Uncertain,
}

/// Sanitized terminal evidence returned by a Node. Absolute local paths must
/// already be virtualized as `workspace://<id>/...` before this is emitted.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RunCompletion {
    pub run_id: String,
    pub attempt: u32,
    pub status: RunTerminalStatus,
    pub result_content: String,
    pub result_sha256: String,
    pub total_output_bytes: u64,
    pub stored_output_bytes: u64,
    #[serde(default)]
    pub capped: bool,
    #[serde(default)]
    pub redacted: bool,
    pub path_policy_applied: bool,
}

impl RunCompletion {
    pub fn validate(&self) -> Result<(), ProtocolContractError> {
        validate_identifier("run_id", &self.run_id)?;
        validate_sha256("result_sha256", &self.result_sha256)?;
        if self.attempt == 0 {
            return Err(ProtocolContractError::InvalidAttempt);
        }
        if self.stored_output_bytes > self.total_output_bytes {
            return Err(ProtocolContractError::InvalidOutputSize);
        }
        if self.stored_output_bytes != self.result_content.len() as u64 {
            return Err(ProtocolContractError::InvalidOutputSize);
        }
        if self.result_content.len() > MAX_RESULT_CONTENT_BYTES {
            return Err(ProtocolContractError::LimitExceeded("result content"));
        }
        if sha256_hex(self.result_content.as_bytes()) != self.result_sha256 {
            return Err(ProtocolContractError::InvalidDigest("result_sha256"));
        }
        if !self.path_policy_applied {
            return Err(ProtocolContractError::PathPolicyMissing);
        }
        Ok(())
    }
}

fn sha256_hex(value: &[u8]) -> String {
    hex::encode(Sha256::digest(value))
}

impl fmt::Debug for RunCompletion {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("RunCompletion")
            .field("run_id", &self.run_id)
            .field("attempt", &self.attempt)
            .field("status", &self.status)
            .field("result_content", &"[REDACTED]")
            .field("result_sha256", &self.result_sha256)
            .field("total_output_bytes", &self.total_output_bytes)
            .field("stored_output_bytes", &self.stored_output_bytes)
            .field("capped", &self.capped)
            .field("redacted", &self.redacted)
            .field("path_policy_applied", &self.path_policy_applied)
            .finish()
    }
}

/// One monotonically sequenced message on the authenticated Hub/Node channel.
#[derive(Clone, PartialEq, Serialize, Deserialize)]
pub struct HubNodeEnvelope {
    pub protocol_version: ProtocolVersion,
    pub device_id: String,
    pub connection_id: String,
    pub sequence: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ack_sequence: Option<u64>,
    pub sent_at_ms: i64,
    #[serde(flatten)]
    pub message: HubNodeMessage,
}

impl HubNodeEnvelope {
    pub fn validate(&self) -> Result<(), ProtocolContractError> {
        HUB_NODE_PROTOCOL_VERSION.negotiate(self.protocol_version)?;
        validate_identifier("device_id", &self.device_id)?;
        validate_identifier("connection_id", &self.connection_id)?;
        if self.sequence == 0 {
            return Err(ProtocolContractError::InvalidSequence);
        }
        if self.sent_at_ms <= 0 {
            return Err(ProtocolContractError::InvalidTimestamp);
        }
        if self.ack_sequence == Some(0) {
            return Err(ProtocolContractError::InvalidSequence);
        }
        self.message.validate()
    }
}

impl fmt::Debug for HubNodeEnvelope {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("HubNodeEnvelope")
            .field("protocol_version", &self.protocol_version)
            .field("device_id", &self.device_id)
            .field("connection_id", &self.connection_id)
            .field("sequence", &self.sequence)
            .field("ack_sequence", &self.ack_sequence)
            .field("sent_at_ms", &self.sent_at_ms)
            .field("message_type", &self.message.kind_name())
            .finish()
    }
}

/// Protocol payloads. Fields may be added within major version 1; consumers
/// must ignore unknown object fields.
#[derive(Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", content = "payload", rename_all = "snake_case")]
pub enum HubNodeMessage {
    Hello {
        role: DeviceRole,
        capabilities: CapabilityDescriptor,
        #[serde(default)]
        resume_after_sequence: u64,
        #[serde(default)]
        active_run_ids: Vec<String>,
    },
    Welcome {
        negotiated_version: ProtocolVersion,
        transport: NodeTransport,
        heartbeat_interval_ms: u64,
        lease_duration_ms: u64,
    },
    /// Explicit Hub-side tombstone for one unacknowledged outbound sequence
    /// replaced during reconnect. The original payload remains in Hub audit
    /// storage; only its kind and digest cross the wire.
    Superseded {
        original_message_kind: String,
        original_message_sha256: String,
    },
    Heartbeat {
        #[serde(default)]
        active_run_ids: Vec<String>,
    },
    RunOffer(RunLease),
    RunAccepted {
        run_id: String,
        attempt: u32,
    },
    RunApprovalRequired(RunApprovalRequest),
    RunApprovalDecision(RunApprovalDecision),
    RunRejected(RunRejection),
    RunProgress {
        run_id: String,
        attempt: u32,
        progress_sequence: u64,
        message: String,
        path_policy_applied: bool,
    },
    RunCompleted(RunCompletion),
    CancelRun {
        run_id: String,
        attempt: u32,
        reason: String,
    },
    AckOnly,
    ProtocolError {
        code: String,
        message: String,
        retryable: bool,
        path_policy_applied: bool,
    },
}

impl HubNodeMessage {
    fn kind_name(&self) -> &'static str {
        match self {
            Self::Hello { .. } => "hello",
            Self::Welcome { .. } => "welcome",
            Self::Superseded { .. } => "superseded",
            Self::Heartbeat { .. } => "heartbeat",
            Self::RunOffer(_) => "run_offer",
            Self::RunAccepted { .. } => "run_accepted",
            Self::RunApprovalRequired(_) => "run_approval_required",
            Self::RunApprovalDecision(_) => "run_approval_decision",
            Self::RunRejected(_) => "run_rejected",
            Self::RunProgress { .. } => "run_progress",
            Self::RunCompleted(_) => "run_completed",
            Self::CancelRun { .. } => "cancel_run",
            Self::AckOnly => "ack_only",
            Self::ProtocolError { .. } => "protocol_error",
        }
    }

    fn validate(&self) -> Result<(), ProtocolContractError> {
        match self {
            Self::Hello {
                role,
                capabilities,
                active_run_ids,
                ..
            } => {
                if *role != DeviceRole::Node {
                    return Err(ProtocolContractError::ClientAdvertisedExecution);
                }
                capabilities.validate()?;
                validate_active_runs(active_run_ids)
            }
            Self::Welcome {
                negotiated_version,
                heartbeat_interval_ms,
                lease_duration_ms,
                ..
            } => {
                HUB_NODE_PROTOCOL_VERSION.negotiate(*negotiated_version)?;
                if *heartbeat_interval_ms == 0 || *lease_duration_ms <= *heartbeat_interval_ms {
                    return Err(ProtocolContractError::InvalidTiming);
                }
                Ok(())
            }
            Self::Superseded {
                original_message_kind,
                original_message_sha256,
            } => {
                validate_identifier("original message kind", original_message_kind)?;
                validate_sha256("original message digest", original_message_sha256)
            }
            Self::Heartbeat { active_run_ids } => validate_active_runs(active_run_ids),
            Self::RunOffer(lease) => lease.validate(),
            Self::RunAccepted { run_id, attempt } => validate_run_attempt(run_id, *attempt),
            Self::RunApprovalRequired(request) => request.validate(),
            Self::RunApprovalDecision(decision) => decision.validate(),
            Self::RunRejected(rejection) => rejection.validate(),
            Self::RunProgress {
                run_id,
                attempt,
                progress_sequence,
                message,
                path_policy_applied,
            } => {
                validate_run_attempt(run_id, *attempt)?;
                if *progress_sequence == 0 {
                    return Err(ProtocolContractError::InvalidSequence);
                }
                validate_text("progress message", message, MAX_PROGRESS_MESSAGE_BYTES)?;
                require_path_policy(*path_policy_applied)
            }
            Self::RunCompleted(completion) => completion.validate(),
            Self::CancelRun {
                run_id,
                attempt,
                reason,
            } => {
                validate_run_attempt(run_id, *attempt)?;
                validate_label("cancel reason", reason)
            }
            Self::AckOnly => Ok(()),
            Self::ProtocolError {
                code,
                message,
                path_policy_applied,
                ..
            } => {
                validate_identifier("error code", code)?;
                validate_text("error message", message, MAX_PROTOCOL_ERROR_BYTES)?;
                require_path_policy(*path_policy_applied)
            }
        }
    }
}

impl fmt::Debug for HubNodeMessage {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("HubNodeMessage")
            .field("type", &self.kind_name())
            .finish()
    }
}

#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum ProtocolContractError {
    #[error("incompatible protocol versions: local {local:?}, peer {peer:?}")]
    IncompatibleVersion {
        local: ProtocolVersion,
        peer: ProtocolVersion,
    },
    #[error("{0} is invalid")]
    InvalidIdentifier(&'static str),
    #[error("{0} is invalid")]
    InvalidLabel(&'static str),
    #[error("{0} must be a lowercase SHA-256 digest")]
    InvalidDigest(&'static str),
    #[error("at least one outbound transport is required")]
    MissingTransport,
    #[error("{0} exceeds the protocol limit")]
    LimitExceeded(&'static str),
    #[error("duplicate {0}")]
    Duplicate(&'static str),
    #[error("capability was not advertised: {0}")]
    CapabilityNotAdvertised(String),
    #[error("approved grants exceed the device request")]
    GrantExceedsRequest,
    #[error("a client cannot advertise execution capabilities")]
    ClientAdvertisedExecution,
    #[error("device metadata does not match its capability descriptor: {0}")]
    DeviceMetadataMismatch(&'static str),
    #[error("run attempt must start at one")]
    InvalidAttempt,
    #[error("tool input must be a JSON object")]
    InvalidToolInput,
    #[error("lease expiry is invalid")]
    InvalidLeaseExpiry,
    #[error("stored output exceeds total output")]
    InvalidOutputSize,
    #[error("node result did not apply the local path policy")]
    PathPolicyMissing,
    #[error("sequence numbers start at one")]
    InvalidSequence,
    #[error("timestamp is invalid")]
    InvalidTimestamp,
    #[error("heartbeat and lease timings are invalid")]
    InvalidTiming,
}

fn validate_run_attempt(run_id: &str, attempt: u32) -> Result<(), ProtocolContractError> {
    validate_identifier("run_id", run_id)?;
    if attempt == 0 {
        return Err(ProtocolContractError::InvalidAttempt);
    }
    Ok(())
}

fn require_path_policy(applied: bool) -> Result<(), ProtocolContractError> {
    if applied {
        Ok(())
    } else {
        Err(ProtocolContractError::PathPolicyMissing)
    }
}

fn validate_active_runs(active_run_ids: &[String]) -> Result<(), ProtocolContractError> {
    if active_run_ids.len() > MAX_ACTIVE_RUNS {
        return Err(ProtocolContractError::LimitExceeded("active runs"));
    }
    for run_id in active_run_ids {
        validate_identifier("active run", run_id)?;
    }
    reject_duplicates("active run", active_run_ids.iter().cloned())
}

pub(crate) fn validate_identifier(
    field: &'static str,
    value: &str,
) -> Result<(), ProtocolContractError> {
    let valid = !value.is_empty()
        && value.len() <= MAX_IDENTIFIER_LEN
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-' | b':'));
    if valid {
        Ok(())
    } else {
        Err(ProtocolContractError::InvalidIdentifier(field))
    }
}

fn validate_label(field: &'static str, value: &str) -> Result<(), ProtocolContractError> {
    validate_text(field, value, MAX_LABEL_LEN)
}

fn validate_text(
    field: &'static str,
    value: &str,
    max_bytes: usize,
) -> Result<(), ProtocolContractError> {
    let trimmed = value.trim();
    let valid = !trimmed.is_empty()
        && trimmed.len() <= max_bytes
        && !trimmed.chars().any(|character| character.is_control());
    if valid {
        Ok(())
    } else {
        Err(ProtocolContractError::InvalidLabel(field))
    }
}

fn validate_sha256(field: &'static str, value: &str) -> Result<(), ProtocolContractError> {
    if value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        Ok(())
    } else {
        Err(ProtocolContractError::InvalidDigest(field))
    }
}

fn reject_duplicates(
    field: &'static str,
    values: impl IntoIterator<Item = String>,
) -> Result<(), ProtocolContractError> {
    let mut seen = BTreeSet::new();
    for value in values {
        if !seen.insert(value) {
            return Err(ProtocolContractError::Duplicate(field));
        }
    }
    Ok(())
}

#[cfg(test)]
#[path = "hub_protocol_tests.rs"]
mod tests;
