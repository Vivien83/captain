//! Lightweight outbound-only execution node for a Captain Hub.
//!
//! This crate deliberately contains no agent loop, provider, or Captain
//! memory database. It owns the local Node transport and, in later modules,
//! the crash-safe execution outbox required by the Hub/Node protocol.

pub mod client_access;
pub mod client_config;
pub mod execution_policy;
pub mod link;
pub mod local_config;
pub mod network;
pub mod pairing;
pub mod rail;
pub mod runtime_status;
pub mod worker;

pub use client_access::{ClientAccessCredential, ClientAccessError, ClientAccessTransport};
pub use client_config::{ClientLocalConfig, ClientLocalConfigError, ClientLocalConfigStore};
pub use execution_policy::{
    AuthorizedNodeRun, NodeExecutionAuthorization, NodeExecutionPolicy, NodeExecutionPolicyError,
    NodeReviewedTool, NodeWorkspaceBinding,
};
pub use local_config::{
    NodeLocalConfig, NodeLocalConfigError, NodeLocalConfigStore, NodeLocalWorkspace,
};

pub use network::{
    HubNodeEndpoints, NodeHttpClient, NodeHttpStream, NodeNetworkConfig, NodeNetworkError,
    NodeProxyMode, NodeWebSocket, ResolvedProxyPassword,
};
pub use pairing::{
    ClientAccessSession, ClientAccessToken, ClientPairingClient, ClientPairingError,
    ClientPairingProfile, ClientPairingProgress, ClientPairingStore, NodeAccessToken,
    NodePairingClient, NodePairingError, NodePairingProfile, NodePairingProgress, NodePairingStore,
};
pub use rail::{
    NodeBootstrap, NodeBootstrapCapabilityState, NodeDeliveryOutcome, NodeInboundRecord,
    NodeRailError, NodeRailSnapshot, NodeRailStore, NodeRunApprovalOutcome, NodeRunCancelOutcome,
    NodeRunClaimOutcome, NodeRunCompletionOutcome, NodeRunDisposition, NodeRunIntakeOutcome,
    NodeRunPreflightRejectionOutcome, NodeRunRecord, NodeRunStatus,
};
pub use runtime_status::{
    NodeRuntimeState, NodeRuntimeStatus, NodeRuntimeStatusError, NodeRuntimeStatusStore,
};

#[cfg(test)]
#[path = "distributed_runtime_tests.rs"]
mod distributed_runtime_tests;
#[cfg(test)]
#[path = "rail_tests.rs"]
mod rail_tests;
pub use link::{
    NodeHeartbeatPolicy, NodeLinkError, NodeRailLink, NodeTransportFailure, NodeTransportFallback,
};
pub use worker::{
    NodeRunCancellation, NodeToolDriver, NodeToolExecutionOutput, NodeToolReview, NodeWorker,
    NodeWorkerCycle, NodeWorkerError,
};
