//! Captain Wire Protocol (OFP) — agent-to-agent networking.
//!
//! Provides cross-machine agent discovery, authentication, and communication
//! over TCP connections using a JSON-RPC framed protocol.
//!
//! ## Architecture
//!
//! - **PeerNode**: Local network endpoint that listens for incoming connections
//! - **PeerRegistry**: Tracks known peers and their agents
//! - **WireMessage**: JSON-framed protocol messages
//! - **PeerHandle**: Trait for routing remote messages through the kernel

pub mod desktop_client_access;
pub mod execution_target;
pub mod hub_pairing;
pub mod hub_protocol;
pub mod hub_transport;
pub mod message;
pub mod peer;
pub mod registry;

pub use desktop_client_access::{
    client_api_path_is_authorizable, client_relay_path_is_canonical, desktop_client_route_allows,
    ClientHttpMethod, DESKTOP_CLIENT_POLICY_VERSION,
};
pub use execution_target::{ExecutionTarget, ExecutionTargetContractError};
pub use hub_pairing::{
    DeviceAccessToken, DeviceCredentialExchange, PairingChallenge, PairingContractError,
    PairingPollRequest, PairingPollResponse, PairingState, DEVICE_TOKEN_PATH, PAIRING_CLAIM_PATH,
    PAIRING_POLL_PATH,
};
pub use hub_protocol::{
    CapabilityDescriptor, DeviceGrant, DevicePairingClaim, DeviceRole, HubNodeEnvelope,
    HubNodeMessage, LogicalWorkspace, NodeTransport, ProtocolContractError, ProtocolVersion,
    RunCompletion, RunEffect, RunLease, RunTerminalStatus, HUB_NODE_PROTOCOL_VERSION,
};
pub use hub_transport::{
    HubNodeCloseRequest, HubNodeConnectRequest, HubNodeDeliveryBatch, HubNodeIngressRequest,
    HubNodePullRequest, HubNodeStreamRequest, HubNodeWebSocketFrame, HubTransportContractError,
    HUB_NODE_CLOSE_PATH, HUB_NODE_CONNECT_PATH, HUB_NODE_ENVELOPE_PATH, HUB_NODE_PULL_PATH,
    HUB_NODE_STREAM_PATH, HUB_NODE_WEBSOCKET_PATH, MAX_HUB_NODE_BATCH_MESSAGES,
    MAX_HUB_NODE_FRAME_BYTES, MAX_HUB_NODE_LONG_POLL_WAIT_MS,
};
pub use message::{WireMessage, WireRequest, WireResponse};
pub use peer::{PeerConfig, PeerNode};
pub use registry::{PeerEntry, PeerRegistry, RemoteAgent};
