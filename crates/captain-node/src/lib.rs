//! Lightweight outbound-only execution node for a Captain Hub.
//!
//! This crate deliberately contains no agent loop, provider, or Captain
//! memory database. It owns the local Node transport and, in later modules,
//! the crash-safe execution outbox required by the Hub/Node protocol.

pub mod client_access;
pub mod client_config;
pub mod client_profiles;
#[cfg(feature = "node-runtime")]
pub mod execution_policy;
#[cfg(feature = "node-runtime")]
pub mod link;
#[cfg(feature = "node-runtime")]
pub mod local_config;
#[cfg(feature = "node-runtime")]
mod local_tool_driver;
#[cfg(feature = "node-runtime")]
pub mod native_service;
#[path = "native_service_control/mod.rs"]
#[cfg(feature = "node-runtime")]
pub mod native_service_control;
pub mod network;
#[cfg(feature = "node-runtime")]
pub mod operator;
pub mod pairing;
pub mod proxy_secrets;
#[cfg(feature = "node-runtime")]
pub mod rail;
#[cfg(feature = "node-runtime")]
pub mod runtime_status;
#[cfg(feature = "node-runtime")]
pub mod shutdown;
#[cfg(feature = "node-runtime")]
pub mod worker;

pub use client_access::{ClientAccessCredential, ClientAccessError, ClientAccessTransport};
pub use client_config::{ClientLocalConfig, ClientLocalConfigError, ClientLocalConfigStore};
pub use client_profiles::{ClientProfileEntry, ClientProfileRegistry, ClientProfileRegistryError};
#[cfg(feature = "node-runtime")]
pub use execution_policy::{
    AuthorizedNodeRun, NodeExecutionAuthorization, NodeExecutionPolicy, NodeExecutionPolicyError,
    NodeReviewedTool, NodeWorkspaceBinding,
};
#[cfg(feature = "node-runtime")]
pub use local_config::{
    NodeLocalConfig, NodeLocalConfigError, NodeLocalConfigStore, NodeLocalWorkspace,
};
#[cfg(feature = "node-runtime")]
pub use local_tool_driver::NodeLocalToolDriver;
#[cfg(feature = "node-runtime")]
pub use native_service::{
    launchd_plist_content, node_service_log_path, systemd_user_unit_content,
    windows_service_bin_path, NodeServiceDefinitionError, NODE_LAUNCHD_LABEL, NODE_SYSTEMD_SERVICE,
    NODE_WINDOWS_DISPLAY_NAME, NODE_WINDOWS_SERVICE,
};
#[cfg(feature = "node-runtime")]
pub use native_service_control::{
    NativeNodeServiceController, NodeNativeServiceError, NodeNativeServiceState,
    NodeNativeServiceStatus,
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
pub use proxy_secrets::{NativeNodeProxySecrets, NodeProxySecretError};
#[cfg(feature = "node-runtime")]
pub use rail::{
    NodeBootstrap, NodeBootstrapCapabilityState, NodeDeliveryOutcome, NodeInboundRecord,
    NodeRailError, NodeRailSnapshot, NodeRailStore, NodeRunApprovalOutcome, NodeRunCancelOutcome,
    NodeRunClaimOutcome, NodeRunCompletionOutcome, NodeRunDisposition, NodeRunIntakeOutcome,
    NodeRunPreflightRejectionOutcome, NodeRunRecord, NodeRunStatus,
};
#[cfg(feature = "node-runtime")]
pub use runtime_status::{
    NodeRuntimeState, NodeRuntimeStatus, NodeRuntimeStatusError, NodeRuntimeStatusStore,
};
#[cfg(feature = "node-runtime")]
pub use shutdown::{node_shutdown_channel, NodeShutdown, NodeShutdownHandle};

#[cfg(all(test, feature = "node-runtime"))]
#[path = "distributed_runtime_tests.rs"]
mod distributed_runtime_tests;
#[cfg(all(test, feature = "node-runtime"))]
#[path = "rail_tests.rs"]
mod rail_tests;
#[cfg(feature = "node-runtime")]
pub use link::{
    NodeHeartbeatPolicy, NodeLinkError, NodeRailLink, NodeTransportFailure, NodeTransportFallback,
};
#[cfg(feature = "node-runtime")]
pub use worker::{
    NodeRunCancellation, NodeToolDriver, NodeToolExecutionOutput, NodeToolReview, NodeWorker,
    NodeWorkerCycle, NodeWorkerError,
};
