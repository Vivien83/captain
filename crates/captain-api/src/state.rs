//! Shared API application state.

use captain_kernel::CaptainKernel;
use dashmap::DashMap;
use std::sync::Arc;
use std::time::Instant;

/// Shared application state.
///
/// The kernel is wrapped in Arc so it can serve as both the main kernel
/// and the KernelHandle for inter-agent tool access.
pub struct AppState {
    pub kernel: Arc<CaptainKernel>,
    pub started_at: Instant,
    /// Optional peer registry for OFP mesh networking status.
    pub peer_registry: Option<Arc<captain_wire::registry::PeerRegistry>>,
    /// Channel bridge manager held behind a mutex for hot-reload swaps.
    pub bridge_manager: tokio::sync::Mutex<Option<captain_channels::bridge::BridgeManager>>,
    /// Live channel config updated on hot-reload so list_channels() reflects reality.
    pub channels_config: tokio::sync::RwLock<captain_types::config::ChannelsConfig>,
    /// Notify handle to trigger graceful HTTP server shutdown from the API.
    pub shutdown_notify: Arc<tokio::sync::Notify>,
    /// ClawHub response cache: cache key -> (fetched_at, response_json).
    pub clawhub_cache: DashMap<String, (Instant, serde_json::Value)>,
    /// Answer channel for an agent's currently-pending ask_user question,
    /// keyed by agent and optional persisted session. Populated by
    /// send_message_stream while its SSE stream is open, drained by POST
    /// /api/agents/:id/message/answer, and removed once the stream ends.
    pub ask_user_channels: DashMap<String, tokio::sync::mpsc::Sender<String>>,
    /// Probe cache for local provider health checks.
    pub provider_probe_cache: captain_runtime::provider_health::ProbeCache,
}
