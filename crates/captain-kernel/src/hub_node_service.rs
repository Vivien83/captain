//! Authenticated kernel boundary for outbound-only Hub/Node transports.

use crate::hub_pairing_service::{HubPairingService, PairingServiceError};
use captain_memory::hub_node_rail::{
    AppliedHubNodeEnvelope, CancelledHubNodeRun, DecidedHubNodeRunApproval,
    HubNodeConnectionRecord, HubNodeConnectionStatus, HubNodeDeliverySnapshot, HubNodeRailError,
    HubNodeRailStore, HubNodeRunApprovalRecord, HubNodeRunRecord, HubNodeRunStatus, NewHubNodeRun,
};
use captain_runtime::node_tool_runtime::{
    local_node_input_uses_workspace_relative_paths, local_node_tool_effect, local_node_tool_family,
    LocalNodeToolEffect,
};
use captain_wire::{
    hub_protocol::RunApprovalDecision, CapabilityDescriptor, DeviceGrant, DeviceRole,
    HubNodeDeliveryBatch, HubNodeEnvelope, HubNodeMessage, HubNodePullRequest,
    HubTransportContractError, NodeTransport, RunEffect, HUB_NODE_PROTOCOL_VERSION,
    MAX_HUB_NODE_BATCH_MESSAGES,
};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeSet,
    sync::{Arc, Mutex},
    time::Duration,
};
use thiserror::Error;
use tokio::sync::Notify;

pub const HUB_NODE_HEARTBEAT_INTERVAL_MS: u64 = 15_000;
pub const HUB_NODE_LEASE_DURATION_MS: u64 = 60_000;
pub const HUB_NODE_EMPTY_POLL_RETRY_MS: u64 = 1_000;

#[derive(Clone)]
pub struct HubNodeService {
    pairing: Arc<HubPairingService>,
    rail: HubNodeRailStore,
    active_transports: Arc<Mutex<BTreeSet<(String, NodeTransport)>>>,
    activity: Arc<Notify>,
}

impl HubNodeService {
    pub fn new(pairing: Arc<HubPairingService>, rail: HubNodeRailStore) -> Self {
        Self {
            pairing,
            rail,
            active_transports: Arc::new(Mutex::new(BTreeSet::new())),
            activity: Arc::new(Notify::new()),
        }
    }

    pub fn open_connection(
        &self,
        raw_access_token: &str,
        hello: &HubNodeEnvelope,
        transport: NodeTransport,
    ) -> Result<HubNodeDeliveryBatch, HubNodeServiceError> {
        self.authenticate_node(raw_access_token, &hello.device_id)?;
        let now_ms = chrono::Utc::now().timestamp_millis();
        self.rail.open_connection(
            hello,
            transport,
            HUB_NODE_HEARTBEAT_INTERVAL_MS,
            HUB_NODE_LEASE_DURATION_MS,
            now_ms,
        )?;
        let batch = self.delivery_batch(
            self.rail.delivery_snapshot(
                &hello.device_id,
                &hello.connection_id,
                MAX_HUB_NODE_BATCH_MESSAGES,
            )?,
            None,
        )?;
        self.activity.notify_waiters();
        Ok(batch)
    }

    pub fn apply_envelope(
        &self,
        raw_access_token: &str,
        envelope: &HubNodeEnvelope,
        transport: NodeTransport,
    ) -> Result<(AppliedHubNodeEnvelope, HubNodeDeliveryBatch), HubNodeServiceError> {
        self.authenticate_node(raw_access_token, &envelope.device_id)?;
        self.ensure_connection_transport(&envelope.device_id, &envelope.connection_id, transport)?;
        let applied = self.rail.apply_node_envelope(
            envelope,
            HUB_NODE_LEASE_DURATION_MS,
            chrono::Utc::now().timestamp_millis(),
        )?;
        self.activity.notify_waiters();
        let batch = self.delivery_batch(
            self.rail.delivery_snapshot(
                &envelope.device_id,
                &envelope.connection_id,
                MAX_HUB_NODE_BATCH_MESSAGES,
            )?,
            None,
        )?;
        Ok((applied, batch))
    }

    pub fn pull(
        &self,
        raw_access_token: &str,
        request: &HubNodePullRequest,
        transport: NodeTransport,
    ) -> Result<HubNodeDeliveryBatch, HubNodeServiceError> {
        request
            .validate()
            .map_err(|_| HubNodeServiceError::InvalidTransportRequest)?;
        self.authenticate_node(raw_access_token, &request.device_id)?;
        let snapshot = self.rail.delivery_snapshot(
            &request.device_id,
            &request.connection_id,
            usize::from(request.max_messages),
        )?;
        if snapshot.connection.protocol_version != request.protocol_version {
            return Err(HubNodeServiceError::InvalidTransportRequest);
        }
        if snapshot.connection.transport != transport {
            return Err(HubNodeServiceError::TransportMismatch);
        }
        let retry_after_ms = snapshot
            .messages
            .is_empty()
            .then_some(HUB_NODE_EMPTY_POLL_RETRY_MS);
        self.delivery_batch(snapshot, retry_after_ms)
    }

    pub fn close_connection(
        &self,
        raw_access_token: &str,
        device_id: &str,
        connection_id: &str,
        error_code: Option<&str>,
    ) -> Result<HubNodeConnectionRecord, HubNodeServiceError> {
        self.authenticate_node(raw_access_token, device_id)?;
        let now_ms = chrono::Utc::now().timestamp_millis();
        let connection = self
            .rail
            .close_connection(device_id, connection_id, error_code, now_ms)
            .map_err(HubNodeServiceError::from)?;
        self.rail.reconcile_after_disconnect(device_id, now_ms)?;
        self.activity.notify_waiters();
        Ok(connection)
    }

    /// Return the durable connection projection used by authenticated
    /// operator surfaces. Connection identifiers never leave the kernel/API
    /// boundary; callers must expose only presence and transport metadata.
    pub fn device_connection(
        &self,
        device_id: &str,
    ) -> Result<Option<HubNodeConnectionRecord>, HubNodeServiceError> {
        self.rail.connection(device_id).map_err(Into::into)
    }

    /// Validate current grants and atomically offer one durable tool run to
    /// the exact active Node connection. Replaying an active idempotency key
    /// returns the existing run; a recovered queued read may receive a new,
    /// explicitly incremented lease attempt.
    pub fn submit_run(
        &self,
        input: &NewHubNodeRun,
    ) -> Result<HubNodeRunRecord, HubNodeServiceError> {
        let tool_family = local_node_tool_family(&input.tool_name)
            .ok_or(HubNodeServiceError::ToolNotSupported)?;
        if !local_node_input_uses_workspace_relative_paths(&input.tool_name, &input.input) {
            return Err(HubNodeServiceError::PathPolicyViolation);
        }
        let derived_effect = match local_node_tool_effect(&input.tool_name, &input.input)
            .ok_or(HubNodeServiceError::ToolNotSupported)?
        {
            LocalNodeToolEffect::ReadOnly => RunEffect::ReadOnly,
            LocalNodeToolEffect::LocalMutation => RunEffect::LocalMutation,
            LocalNodeToolEffect::ExternalEffect => RunEffect::ExternalEffect,
        };
        if derived_effect != input.effect {
            return Err(HubNodeServiceError::EffectMismatch);
        }
        let connection = self.authorize_execution_target(
            &input.device_id,
            &input.workspace_id,
            tool_family,
            input.effect,
        )?;
        let run = self.rail.enqueue_run(input)?;
        let run = if run.status == HubNodeRunStatus::Queued {
            self.rail
                .lease_run(
                    &input.device_id,
                    &input.run_id,
                    &connection.connection_id,
                    chrono::Utc::now().timestamp_millis(),
                    i64::try_from(HUB_NODE_LEASE_DURATION_MS)
                        .map_err(|_| HubNodeServiceError::DeliveryInvariant)?,
                )?
                .map(|leased| leased.run)
                .unwrap_or_else(|| run.clone())
        } else {
            run
        };
        self.activity.notify_waiters();
        Ok(run)
    }

    pub fn get_run(&self, run_id: &str) -> Result<Option<HubNodeRunRecord>, HubNodeServiceError> {
        self.rail.get_run(run_id).map_err(Into::into)
    }

    pub fn get_run_approval(
        &self,
        run_id: &str,
    ) -> Result<Option<HubNodeRunApprovalRecord>, HubNodeServiceError> {
        self.rail.get_run_approval(run_id).map_err(Into::into)
    }

    pub fn decide_run_approval(
        &self,
        decision: &RunApprovalDecision,
    ) -> Result<DecidedHubNodeRunApproval, HubNodeServiceError> {
        let decided = self.rail.decide_run_approval(decision)?;
        self.activity.notify_waiters();
        Ok(decided)
    }

    pub fn request_run_cancel(
        &self,
        run_id: &str,
        reason: &str,
    ) -> Result<CancelledHubNodeRun, HubNodeServiceError> {
        let cancelled =
            self.rail
                .request_cancel(run_id, reason, chrono::Utc::now().timestamp_millis())?;
        self.activity.notify_waiters();
        Ok(cancelled)
    }

    /// Best-effort wake optimization for waiters. Durable state remains the
    /// authority, so callers always re-read the run after this returns.
    pub async fn wait_for_activity(&self, maximum_wait: Duration) {
        let _ = tokio::time::timeout(maximum_wait, self.activity.notified()).await;
    }

    fn authorize_execution_target(
        &self,
        device_id: &str,
        workspace_id: &str,
        tool_family: &str,
        effect: RunEffect,
    ) -> Result<HubNodeConnectionRecord, HubNodeServiceError> {
        let device = self
            .pairing
            .list_devices()
            .map_err(map_pairing_error)?
            .into_iter()
            .find(|device| device.device_id == device_id)
            .ok_or(HubNodeServiceError::NodeUnavailable)?;
        if device.role != "node" || device.status != "active" || device.revoked_at_ms.is_some() {
            return Err(HubNodeServiceError::NodeUnavailable);
        }
        HUB_NODE_PROTOCOL_VERSION
            .negotiate(captain_wire::ProtocolVersion {
                major: device.protocol_major,
                minor: device.protocol_minor,
            })
            .map_err(|_| HubNodeServiceError::NodeIncompatible)?;
        let capabilities: CapabilityDescriptor = serde_json::from_str(&device.capabilities_json)
            .map_err(|_| HubNodeServiceError::StorageUnavailable)?;
        let grants: DeviceGrant = serde_json::from_str(&device.grants_json)
            .map_err(|_| HubNodeServiceError::StorageUnavailable)?;
        grants
            .validate_against(&capabilities)
            .map_err(|_| HubNodeServiceError::StorageUnavailable)?;
        let workspace = capabilities
            .workspaces
            .iter()
            .find(|workspace| workspace.workspace_id == workspace_id)
            .filter(|_| {
                grants
                    .workspace_ids
                    .iter()
                    .any(|granted| granted == workspace_id)
            })
            .ok_or(HubNodeServiceError::WorkspaceNotGranted)?;
        if !capabilities
            .tool_families
            .iter()
            .any(|family| family == tool_family)
            || !grants
                .tool_families
                .iter()
                .any(|family| family == tool_family)
        {
            return Err(HubNodeServiceError::ToolFamilyNotGranted);
        }
        if effect != RunEffect::ReadOnly && (!grants.allow_mutation || workspace.read_only) {
            return Err(HubNodeServiceError::MutationNotGranted);
        }

        let now_ms = chrono::Utc::now().timestamp_millis();
        let connection = self
            .rail
            .connection(device_id)?
            .filter(|connection| {
                connection.status == HubNodeConnectionStatus::Active
                    && now_ms.saturating_sub(connection.last_seen_ms)
                        <= i64::try_from(HUB_NODE_LEASE_DURATION_MS).unwrap_or(i64::MAX)
            })
            .ok_or(HubNodeServiceError::NodeOffline)?;
        HUB_NODE_PROTOCOL_VERSION
            .negotiate(connection.protocol_version)
            .map_err(|_| HubNodeServiceError::NodeIncompatible)?;
        Ok(connection)
    }

    fn authenticate_node(
        &self,
        raw_access_token: &str,
        expected_device_id: &str,
    ) -> Result<(), HubNodeServiceError> {
        let device_id = self.authorize_transport_token(raw_access_token)?;
        if device_id != expected_device_id {
            return Err(HubNodeServiceError::DeviceIdentityMismatch);
        }
        Ok(())
    }

    /// Validate a short-lived device bearer before allocating a transport.
    /// The returned identifier is the only device metadata an API adapter
    /// needs; grants and persisted capability details remain inside the Hub.
    pub fn authorize_transport_token(
        &self,
        raw_access_token: &str,
    ) -> Result<String, HubNodeServiceError> {
        let identity = self
            .pairing
            .authenticate_access_token(raw_access_token)
            .map_err(map_pairing_error)?;
        if identity.role != DeviceRole::Node {
            return Err(HubNodeServiceError::NodeRoleRequired);
        }
        Ok(identity.device_id)
    }

    /// Reserve the single long-lived receive loop for a device and transport.
    /// The permit is process-local by design: durable connection ownership
    /// remains in SQLite, while this guard bounds concurrent HTTP/WS resources.
    pub fn acquire_transport_permit(
        &self,
        raw_access_token: &str,
        expected_device_id: &str,
        transport: NodeTransport,
    ) -> Result<HubNodeTransportPermit, HubNodeServiceError> {
        self.authenticate_node(raw_access_token, expected_device_id)?;
        let key = (expected_device_id.to_string(), transport);
        let mut active = self
            .active_transports
            .lock()
            .map_err(|_| HubNodeServiceError::StorageUnavailable)?;
        if !active.insert(key.clone()) {
            return Err(HubNodeServiceError::TransportBusy);
        }
        Ok(HubNodeTransportPermit {
            active_transports: Arc::clone(&self.active_transports),
            key: Some(key),
        })
    }

    /// Close a connection owned by a still-live transport permit. This is the
    /// cleanup path when the short bearer expires while an authenticated
    /// socket or stream is already running; the permit cannot cross devices
    /// or transport families.
    pub fn close_permitted_connection(
        &self,
        permit: &HubNodeTransportPermit,
        device_id: &str,
        connection_id: &str,
        transport: NodeTransport,
        error_code: Option<&str>,
    ) -> Result<HubNodeConnectionRecord, HubNodeServiceError> {
        if !permit.authorizes(device_id, transport) {
            return Err(HubNodeServiceError::DeviceIdentityMismatch);
        }
        self.ensure_connection_transport(device_id, connection_id, transport)?;
        let now_ms = chrono::Utc::now().timestamp_millis();
        let connection = self
            .rail
            .close_connection(device_id, connection_id, error_code, now_ms)
            .map_err(HubNodeServiceError::from)?;
        self.rail.reconcile_after_disconnect(device_id, now_ms)?;
        self.activity.notify_waiters();
        Ok(connection)
    }

    fn ensure_connection_transport(
        &self,
        device_id: &str,
        connection_id: &str,
        transport: NodeTransport,
    ) -> Result<(), HubNodeServiceError> {
        let connection = self
            .rail
            .connection(device_id)?
            .filter(|connection| connection.connection_id == connection_id)
            .ok_or_else(|| HubNodeServiceError::Rail(HubNodeRailError::ConnectionConflict))?;
        if connection.transport != transport {
            return Err(HubNodeServiceError::TransportMismatch);
        }
        Ok(())
    }

    fn delivery_batch(
        &self,
        snapshot: HubNodeDeliverySnapshot,
        retry_after_ms: Option<u64>,
    ) -> Result<HubNodeDeliveryBatch, HubNodeServiceError> {
        let protocol_version = snapshot.connection.protocol_version;
        let device_id = snapshot.connection.device_id;
        let connection_id = snapshot.connection.connection_id;
        let mut messages = Vec::with_capacity(snapshot.messages.len());
        for record in snapshot.messages {
            if record.device_id != device_id
                || record.message_sha256 != sha256_hex(record.message_json.as_bytes())
            {
                return Err(HubNodeServiceError::DeliveryInvariant);
            }
            let message = if record.superseded_at_ms.is_some() {
                HubNodeMessage::Superseded {
                    original_message_kind: record.message_kind.clone(),
                    original_message_sha256: record.message_sha256.clone(),
                }
            } else {
                serde_json::from_str(&record.message_json)
                    .map_err(|_| HubNodeServiceError::DeliveryInvariant)?
            };
            messages.push(HubNodeEnvelope {
                protocol_version,
                device_id: device_id.clone(),
                connection_id: connection_id.clone(),
                sequence: record.sequence,
                ack_sequence: None,
                sent_at_ms: record.created_at_ms,
                message,
            });
        }
        let batch = HubNodeDeliveryBatch {
            protocol_version,
            device_id,
            connection_id,
            acknowledged_node_sequence: snapshot.acknowledged_node_sequence,
            messages,
            retry_after_ms,
        };
        batch
            .validate()
            .map_err(|_| HubNodeServiceError::DeliveryInvariant)?;
        Ok(batch)
    }
}

pub struct HubNodeTransportPermit {
    active_transports: Arc<Mutex<BTreeSet<(String, NodeTransport)>>>,
    key: Option<(String, NodeTransport)>,
}

impl HubNodeTransportPermit {
    fn authorizes(&self, device_id: &str, transport: NodeTransport) -> bool {
        self.key
            .as_ref()
            .is_some_and(|key| key.0 == device_id && key.1 == transport)
    }
}

impl Drop for HubNodeTransportPermit {
    fn drop(&mut self) {
        let Some(key) = self.key.take() else {
            return;
        };
        let mut active = self
            .active_transports
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        active.remove(&key);
    }
}

#[derive(Debug, Error)]
pub enum HubNodeServiceError {
    #[error("Hub Node service is disabled")]
    Disabled,
    #[error("device authentication failed")]
    AuthenticationFailed,
    #[error("authenticated device is not an execution Node")]
    NodeRoleRequired,
    #[error("authenticated device identity does not match the request")]
    DeviceIdentityMismatch,
    #[error("paired execution Node is unavailable")]
    NodeUnavailable,
    #[error("paired execution Node is offline")]
    NodeOffline,
    #[error("paired execution Node protocol is incompatible")]
    NodeIncompatible,
    #[error("logical Node workspace is not granted")]
    WorkspaceNotGranted,
    #[error("Node tool family is not granted")]
    ToolFamilyNotGranted,
    #[error("tool is not supported by the local Node rail")]
    ToolNotSupported,
    #[error("Node tool paths must be relative to the logical workspace")]
    PathPolicyViolation,
    #[error("Hub Node run effect does not match the tool input")]
    EffectMismatch,
    #[error("Node workspace does not permit mutations")]
    MutationNotGranted,
    #[error("Hub Node transport request is invalid")]
    InvalidTransportRequest,
    #[error("Hub Node request transport does not match the active connection")]
    TransportMismatch,
    #[error("Hub Node transport already has an active receive loop")]
    TransportBusy,
    #[error("Hub Node durable delivery state is invalid")]
    DeliveryInvariant,
    #[error("Hub Node storage is temporarily unavailable")]
    StorageUnavailable,
    #[error("Hub Node rail rejected the operation")]
    Rail(#[source] HubNodeRailError),
}

impl From<HubNodeRailError> for HubNodeServiceError {
    fn from(error: HubNodeRailError) -> Self {
        Self::Rail(error)
    }
}

impl From<HubTransportContractError> for HubNodeServiceError {
    fn from(_: HubTransportContractError) -> Self {
        Self::InvalidTransportRequest
    }
}

fn map_pairing_error(error: PairingServiceError) -> HubNodeServiceError {
    match error {
        PairingServiceError::Disabled => HubNodeServiceError::Disabled,
        PairingServiceError::StorageUnavailable => HubNodeServiceError::StorageUnavailable,
        _ => HubNodeServiceError::AuthenticationFailed,
    }
}

fn sha256_hex(bytes: &[u8]) -> String {
    hex::encode(Sha256::digest(bytes))
}

#[cfg(test)]
#[path = "hub_node_service_tests.rs"]
mod tests;
