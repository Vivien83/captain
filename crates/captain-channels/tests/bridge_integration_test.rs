//! Integration tests for the BridgeManager dispatch pipeline.
//!
//! These tests create a mock channel adapter (with injectable messages)
//! and a mock kernel handle, wire them through the real BridgeManager,
//! and verify the full dispatch pipeline works end-to-end.
//!
//! No external services are contacted — all communication is in-process
//! via real tokio channels and tasks.

use async_trait::async_trait;
use captain_channels::bridge::{BridgeManager, ChannelBridgeHandle};
use captain_channels::router::AgentRouter;
use captain_channels::types::{
    ChannelAdapter, ChannelContent, ChannelMessage, ChannelType, ChannelUser,
};
use captain_types::agent::AgentId;
use futures::Stream;
use std::collections::HashMap;
use std::pin::Pin;
use std::sync::{Arc, Mutex};
use tokio::sync::{mpsc, watch};

// ---------------------------------------------------------------------------
// Mock Adapter — injects test messages, captures sent responses
// ---------------------------------------------------------------------------

struct MockAdapter {
    name: String,
    channel_type: ChannelType,
    /// Receiver consumed by start() — wrapped as a Stream.
    rx: Mutex<Option<mpsc::Receiver<ChannelMessage>>>,
    /// Captures all messages sent via send().
    sent: Arc<Mutex<Vec<(String, String)>>>,
    shutdown_tx: watch::Sender<bool>,
    /// Set by stop() — the watch alone can't be observed once its receiver
    /// is dropped (send() fails without updating the value).
    stopped: std::sync::atomic::AtomicBool,
}

impl MockAdapter {
    /// Create a new mock adapter. Returns (adapter, sender) — use the sender
    /// to inject test messages into the adapter's stream.
    fn new(name: &str, channel_type: ChannelType) -> (Arc<Self>, mpsc::Sender<ChannelMessage>) {
        let (tx, rx) = mpsc::channel(256);
        let (shutdown_tx, _shutdown_rx) = watch::channel(false);

        let adapter = Arc::new(Self {
            name: name.to_string(),
            channel_type,
            rx: Mutex::new(Some(rx)),
            sent: Arc::new(Mutex::new(Vec::new())),
            shutdown_tx,
            stopped: std::sync::atomic::AtomicBool::new(false),
        });
        (adapter, tx)
    }

    /// Get a copy of all sent responses as (platform_id, text) pairs.
    fn get_sent(&self) -> Vec<(String, String)> {
        self.sent.lock().unwrap().clone()
    }
}

#[async_trait]
impl ChannelAdapter for MockAdapter {
    fn name(&self) -> &str {
        &self.name
    }

    fn channel_type(&self) -> ChannelType {
        self.channel_type.clone()
    }

    async fn start(
        &self,
    ) -> Result<Pin<Box<dyn Stream<Item = ChannelMessage> + Send>>, Box<dyn std::error::Error>>
    {
        let rx = self
            .rx
            .lock()
            .unwrap()
            .take()
            .expect("start() called more than once");
        let stream = tokio_stream::wrappers::ReceiverStream::new(rx);
        Ok(Box::pin(stream))
    }

    async fn send(
        &self,
        user: &ChannelUser,
        content: ChannelContent,
    ) -> Result<(), Box<dyn std::error::Error>> {
        if let ChannelContent::Text(text) = content {
            self.sent
                .lock()
                .unwrap()
                .push((user.platform_id.clone(), text));
        }
        Ok(())
    }

    async fn stop(&self) -> Result<(), Box<dyn std::error::Error>> {
        let _ = self.shutdown_tx.send(true);
        self.stopped
            .store(true, std::sync::atomic::Ordering::SeqCst);
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Mock Kernel Handle — echoes messages, serves agent lists
// ---------------------------------------------------------------------------

struct MockHandle {
    agents: Mutex<Vec<(AgentId, String)>>,
    /// Records all messages sent to agents: (agent_id, message).
    received: Arc<Mutex<Vec<(AgentId, String)>>>,
}

impl MockHandle {
    fn new(agents: Vec<(AgentId, String)>) -> Self {
        Self {
            agents: Mutex::new(agents),
            received: Arc::new(Mutex::new(Vec::new())),
        }
    }
}

#[async_trait]
impl ChannelBridgeHandle for MockHandle {
    async fn send_message(
        &self,
        agent_id: AgentId,
        message: &str,
        _channel_type: Option<&str>,
    ) -> Result<String, String> {
        self.received
            .lock()
            .unwrap()
            .push((agent_id, message.to_string()));
        Ok(format!("Echo: {message}"))
    }

    async fn find_agent_by_name(&self, name: &str) -> Result<Option<AgentId>, String> {
        let agents = self.agents.lock().unwrap();
        Ok(agents.iter().find(|(_, n)| n == name).map(|(id, _)| *id))
    }

    async fn list_agents(&self) -> Result<Vec<(AgentId, String)>, String> {
        Ok(self.agents.lock().unwrap().clone())
    }

    async fn spawn_agent_by_name(&self, _manifest_name: &str) -> Result<AgentId, String> {
        Err("mock: spawn not implemented".to_string())
    }
}

// ---------------------------------------------------------------------------
// Helper to create a ChannelMessage
// ---------------------------------------------------------------------------

fn make_text_msg(channel: ChannelType, user_id: &str, text: &str) -> ChannelMessage {
    ChannelMessage {
        channel,
        platform_message_id: "msg1".to_string(),
        sender: ChannelUser {
            platform_id: user_id.to_string(),
            display_name: "TestUser".to_string(),
            captain_user: None,
        },
        content: ChannelContent::Text(text.to_string()),
        target_agent: None,
        timestamp: chrono::Utc::now(),
        is_group: false,
        thread_id: None,
        metadata: HashMap::new(),
    }
}

fn make_command_msg(
    channel: ChannelType,
    user_id: &str,
    cmd: &str,
    args: Vec<&str>,
) -> ChannelMessage {
    ChannelMessage {
        channel,
        platform_message_id: "msg1".to_string(),
        sender: ChannelUser {
            platform_id: user_id.to_string(),
            display_name: "TestUser".to_string(),
            captain_user: None,
        },
        content: ChannelContent::Command {
            name: cmd.to_string(),
            args: args.into_iter().map(String::from).collect(),
        },
        target_agent: None,
        timestamp: chrono::Utc::now(),
        is_group: false,
        thread_id: None,
        metadata: HashMap::new(),
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

/// Test that text messages are dispatched to the correct agent and responses
/// are sent back through the adapter.
#[tokio::test]
async fn test_bridge_dispatch_text_message() {
    let agent_id = AgentId::new();
    let handle = Arc::new(MockHandle::new(vec![(agent_id, "coder".to_string())]));
    let router = Arc::new(AgentRouter::new());

    // Pre-route the user to the agent
    router.set_user_default("user1".to_string(), agent_id);

    let (adapter, tx) = MockAdapter::new("test-adapter", ChannelType::Telegram);
    let adapter_ref = adapter.clone();

    let mut manager = BridgeManager::new(handle.clone(), router);
    manager.start_adapter(adapter.clone()).await.unwrap();

    // Inject a text message
    tx.send(make_text_msg(
        ChannelType::Telegram,
        "user1",
        "Hello agent!",
    ))
    .await
    .unwrap();

    // Give the async dispatch loop time to process
    tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;

    // Verify: adapter received the echo response
    let sent = adapter_ref.get_sent();
    assert_eq!(sent.len(), 1, "Expected 1 response, got {}", sent.len());
    assert_eq!(sent[0].0, "user1");
    assert_eq!(sent[0].1, "Echo: Hello agent!");

    // Verify: handle received the message
    {
        let received = handle.received.lock().unwrap();
        assert_eq!(received.len(), 1);
        assert_eq!(received[0].0, agent_id);
        assert_eq!(received[0].1, "Hello agent!");
    }

    manager.stop().await;
}

/// Test that /agents command returns the list of running agents.
#[tokio::test]
async fn test_bridge_dispatch_agents_command() {
    let agent_id = AgentId::new();
    let handle = Arc::new(MockHandle::new(vec![
        (agent_id, "coder".to_string()),
        (AgentId::new(), "researcher".to_string()),
    ]));
    let router = Arc::new(AgentRouter::new());

    let (adapter, tx) = MockAdapter::new("test-adapter", ChannelType::Discord);
    let adapter_ref = adapter.clone();

    let mut manager = BridgeManager::new(handle.clone(), router);
    manager.start_adapter(adapter.clone()).await.unwrap();

    // Send /agents command as ChannelContent::Command
    tx.send(make_command_msg(
        ChannelType::Discord,
        "user1",
        "agents",
        vec![],
    ))
    .await
    .unwrap();

    tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;

    let sent = adapter_ref.get_sent();
    assert_eq!(sent.len(), 1);
    assert!(
        sent[0].1.contains("coder"),
        "Response should list 'coder', got: {}",
        sent[0].1
    );
    assert!(
        sent[0].1.contains("researcher"),
        "Response should list 'researcher', got: {}",
        sent[0].1
    );

    manager.stop().await;
}

/// Test the /help command returns help text.
#[tokio::test]
async fn test_bridge_dispatch_help_command() {
    let handle = Arc::new(MockHandle::new(vec![]));
    let router = Arc::new(AgentRouter::new());

    let (adapter, tx) = MockAdapter::new("test-adapter", ChannelType::Slack);
    let adapter_ref = adapter.clone();

    let mut manager = BridgeManager::new(handle, router);
    manager.start_adapter(adapter.clone()).await.unwrap();

    tx.send(make_command_msg(
        ChannelType::Slack,
        "user1",
        "help",
        vec![],
    ))
    .await
    .unwrap();

    tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;

    let sent = adapter_ref.get_sent();
    assert_eq!(sent.len(), 1);
    assert!(sent[0].1.contains("/agents"), "Help should mention /agents");
    assert!(sent[0].1.contains("/agent"), "Help should mention /agent");

    manager.stop().await;
}

/// Test /agent <name> command selects the agent and updates the router.
#[tokio::test]
async fn test_bridge_dispatch_agent_select_command() {
    let agent_id = AgentId::new();
    let handle = Arc::new(MockHandle::new(vec![(agent_id, "coder".to_string())]));
    let router = Arc::new(AgentRouter::new());

    let (adapter, tx) = MockAdapter::new("test-adapter", ChannelType::Telegram);
    let adapter_ref = adapter.clone();

    let mut manager = BridgeManager::new(handle, router.clone());
    manager.start_adapter(adapter.clone()).await.unwrap();

    // User selects "coder" agent
    tx.send(make_command_msg(
        ChannelType::Telegram,
        "user42",
        "agent",
        vec!["coder"],
    ))
    .await
    .unwrap();

    tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;

    let sent = adapter_ref.get_sent();
    assert_eq!(sent.len(), 1);
    assert!(
        sent[0].1.contains("Now talking to agent: coder"),
        "Expected selection confirmation, got: {}",
        sent[0].1
    );

    // Verify router was updated — user42 should now route to agent_id
    let resolved = router.resolve(&ChannelType::Telegram, "user42", None);
    assert_eq!(resolved, Some(agent_id));

    manager.stop().await;
}

/// Test that unrouted messages (no agent assigned) get a helpful error.
#[tokio::test]
async fn test_bridge_dispatch_no_agent_assigned() {
    let handle = Arc::new(MockHandle::new(vec![]));
    let router = Arc::new(AgentRouter::new());

    let (adapter, tx) = MockAdapter::new("test-adapter", ChannelType::Telegram);
    let adapter_ref = adapter.clone();

    let mut manager = BridgeManager::new(handle, router);
    manager.start_adapter(adapter.clone()).await.unwrap();

    // Send message with no agent routed
    tx.send(make_text_msg(ChannelType::Telegram, "user1", "hello"))
        .await
        .unwrap();

    tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;

    let sent = adapter_ref.get_sent();
    assert_eq!(sent.len(), 1);
    assert!(
        sent[0].1.contains("No agents available"),
        "Expected 'No agents available' message, got: {}",
        sent[0].1
    );

    manager.stop().await;
}

/// Test that slash commands embedded in text (/agents, /help) are handled as commands.
#[tokio::test]
async fn test_bridge_dispatch_slash_command_in_text() {
    let agent_id = AgentId::new();
    let handle = Arc::new(MockHandle::new(vec![(agent_id, "writer".to_string())]));
    let router = Arc::new(AgentRouter::new());

    let (adapter, tx) = MockAdapter::new("test-adapter", ChannelType::Telegram);
    let adapter_ref = adapter.clone();

    let mut manager = BridgeManager::new(handle, router);
    manager.start_adapter(adapter.clone()).await.unwrap();

    // Send "/agents" as plain text (not as a Command variant)
    tx.send(make_text_msg(ChannelType::Telegram, "user1", "/agents"))
        .await
        .unwrap();

    tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;

    let sent = adapter_ref.get_sent();
    assert_eq!(sent.len(), 1);
    assert!(
        sent[0].1.contains("writer"),
        "Should list the 'writer' agent, got: {}",
        sent[0].1
    );

    manager.stop().await;
}

/// Test /status command returns uptime info.
#[tokio::test]
async fn test_bridge_dispatch_status_command() {
    let handle = Arc::new(MockHandle::new(vec![
        (AgentId::new(), "a".to_string()),
        (AgentId::new(), "b".to_string()),
    ]));
    let router = Arc::new(AgentRouter::new());

    let (adapter, tx) = MockAdapter::new("test-adapter", ChannelType::Telegram);
    let adapter_ref = adapter.clone();

    let mut manager = BridgeManager::new(handle, router);
    manager.start_adapter(adapter.clone()).await.unwrap();

    tx.send(make_command_msg(
        ChannelType::Telegram,
        "user1",
        "status",
        vec![],
    ))
    .await
    .unwrap();

    tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;

    let sent = adapter_ref.get_sent();
    assert_eq!(sent.len(), 1);
    assert!(
        sent[0].1.contains("2 agent(s) running"),
        "Expected uptime info, got: {}",
        sent[0].1
    );

    manager.stop().await;
}

/// Test the full lifecycle: start adapter, send messages, stop adapter.
#[tokio::test]
async fn test_bridge_manager_lifecycle() {
    let agent_id = AgentId::new();
    let handle = Arc::new(MockHandle::new(vec![(agent_id, "bot".to_string())]));
    let router = Arc::new(AgentRouter::new());
    router.set_user_default("user1".to_string(), agent_id);

    let (adapter, tx) = MockAdapter::new("lifecycle-adapter", ChannelType::WebChat);
    let adapter_ref = adapter.clone();

    let mut manager = BridgeManager::new(handle, router);
    manager.start_adapter(adapter.clone()).await.unwrap();

    // Send multiple messages
    for i in 0..5 {
        tx.send(make_text_msg(
            ChannelType::WebChat,
            "user1",
            &format!("message {i}"),
        ))
        .await
        .unwrap();
    }

    tokio::time::sleep(tokio::time::Duration::from_millis(200)).await;

    let sent = adapter_ref.get_sent();
    assert_eq!(sent.len(), 5, "Expected 5 responses, got {}", sent.len());

    for (i, (_, text)) in sent.iter().enumerate() {
        assert_eq!(*text, format!("Echo: message {i}"));
    }

    // Stop — should complete without hanging
    manager.stop().await;
}

/// Test multiple adapters running simultaneously in the same BridgeManager.
#[tokio::test]
async fn test_bridge_multiple_adapters() {
    let agent_id = AgentId::new();
    let handle = Arc::new(MockHandle::new(vec![(agent_id, "multi".to_string())]));
    let router = Arc::new(AgentRouter::new());
    router.set_user_default("tg_user".to_string(), agent_id);
    router.set_user_default("dc_user".to_string(), agent_id);

    let (tg_adapter, tg_tx) = MockAdapter::new("telegram", ChannelType::Telegram);
    let (dc_adapter, dc_tx) = MockAdapter::new("discord", ChannelType::Discord);
    let tg_ref = tg_adapter.clone();
    let dc_ref = dc_adapter.clone();

    let mut manager = BridgeManager::new(handle, router);
    manager.start_adapter(tg_adapter).await.unwrap();
    manager.start_adapter(dc_adapter).await.unwrap();

    // Send to Telegram adapter
    tg_tx
        .send(make_text_msg(
            ChannelType::Telegram,
            "tg_user",
            "from telegram",
        ))
        .await
        .unwrap();

    // Send to Discord adapter
    dc_tx
        .send(make_text_msg(
            ChannelType::Discord,
            "dc_user",
            "from discord",
        ))
        .await
        .unwrap();

    tokio::time::sleep(tokio::time::Duration::from_millis(150)).await;

    let tg_sent = tg_ref.get_sent();
    assert_eq!(tg_sent.len(), 1);
    assert_eq!(tg_sent[0].1, "Echo: from telegram");

    let dc_sent = dc_ref.get_sent();
    assert_eq!(dc_sent.len(), 1);
    assert_eq!(dc_sent[0].1, "Echo: from discord");

    manager.stop().await;
}

/// A.1 — `reload_channel` swaps the running adapter for a single channel
/// (e.g. Telegram bot token rotation) without disturbing the others.
///
/// Why this matters: Captain edits its own `config.toml` to switch to a fresh
/// Telegram bot. Before A.1 the only way to re-spawn the adapter was to
/// rebuild the entire bridge, which briefly disconnects every other channel
/// for several seconds. After A.1 only the targeted adapter restarts.
#[tokio::test]
async fn test_bridge_reload_channel_replaces_targeted_adapter() {
    let agent_id = AgentId::new();
    let handle = Arc::new(MockHandle::new(vec![(agent_id, "bot".to_string())]));
    let router = Arc::new(AgentRouter::new());
    router.set_user_default("tg_user".to_string(), agent_id);
    router.set_user_default("dc_user".to_string(), agent_id);

    // Two adapters running side-by-side.
    let (tg_v1, tg_v1_tx) = MockAdapter::new("telegram", ChannelType::Telegram);
    let (dc, dc_tx) = MockAdapter::new("discord", ChannelType::Discord);
    let tg_v1_ref = tg_v1.clone();
    let dc_ref = dc.clone();

    let mut manager = BridgeManager::new(handle, router);
    manager.start_adapter(tg_v1).await.unwrap();
    manager.start_adapter(dc).await.unwrap();

    // Sanity: v1 dispatches normally.
    tg_v1_tx
        .send(make_text_msg(ChannelType::Telegram, "tg_user", "via v1"))
        .await
        .unwrap();
    tokio::time::sleep(tokio::time::Duration::from_millis(150)).await;
    assert_eq!(
        tg_v1_ref.get_sent().len(),
        1,
        "v1 should dispatch the first message"
    );

    // Hot-swap the telegram adapter (simulates new bot token rotation).
    let (tg_v2, tg_v2_tx) = MockAdapter::new("telegram", ChannelType::Telegram);
    let tg_v2_ref = tg_v2.clone();
    manager
        .reload_channel("telegram", tg_v2)
        .await
        .expect("reload_channel should succeed");

    // The new adapter must dispatch new messages.
    tg_v2_tx
        .send(make_text_msg(ChannelType::Telegram, "tg_user", "via v2"))
        .await
        .unwrap();
    tokio::time::sleep(tokio::time::Duration::from_millis(150)).await;
    let v2_sent = tg_v2_ref.get_sent();
    assert_eq!(v2_sent.len(), 1, "v2 must dispatch after reload");
    assert_eq!(v2_sent[0].1, "Echo: via v2");

    // The old adapter's dispatch loop must be gone — any message we still
    // try to push through its mpsc must NOT be echoed back to it.
    let _ = tg_v1_tx
        .send(make_text_msg(
            ChannelType::Telegram,
            "tg_user",
            "stale via v1",
        ))
        .await;
    tokio::time::sleep(tokio::time::Duration::from_millis(150)).await;
    assert_eq!(
        tg_v1_ref.get_sent().len(),
        1,
        "v1 must not dispatch any further messages after being replaced"
    );

    // Other adapters keep working.
    dc_tx
        .send(make_text_msg(
            ChannelType::Discord,
            "dc_user",
            "discord still alive",
        ))
        .await
        .unwrap();
    tokio::time::sleep(tokio::time::Duration::from_millis(150)).await;
    assert_eq!(dc_ref.get_sent().len(), 1, "discord must keep dispatching");

    manager.stop().await;
}

/// A.1 — `reload_channel` rejects a swap when the new adapter's name does not
/// match the targeted channel; otherwise we'd silently leak the old adapter.
#[tokio::test]
async fn test_bridge_reload_channel_rejects_name_mismatch() {
    let handle = Arc::new(MockHandle::new(vec![]));
    let router = Arc::new(AgentRouter::new());

    let (tg, _tg_tx) = MockAdapter::new("telegram", ChannelType::Telegram);
    let mut manager = BridgeManager::new(handle, router);
    manager.start_adapter(tg).await.unwrap();

    // New adapter named "discord" but reload requests "telegram" — this must fail.
    let (mismatched, _) = MockAdapter::new("discord", ChannelType::Discord);
    let res = manager.reload_channel("telegram", mismatched).await;
    assert!(res.is_err(), "name mismatch must be rejected");

    manager.stop().await;
}

/// A.1 — `reload_channel` on a name that was never started simply starts the
/// new adapter (idempotent first-install path).
#[tokio::test]
async fn test_bridge_reload_channel_starts_when_absent() {
    let handle = Arc::new(MockHandle::new(vec![]));
    let router = Arc::new(AgentRouter::new());

    let mut manager = BridgeManager::new(handle, router);
    let (tg, _tg_tx) = MockAdapter::new("telegram", ChannelType::Telegram);
    manager
        .reload_channel("telegram", tg)
        .await
        .expect("reload on absent channel should just start it");

    manager.stop().await;
}

/// Live bug: bridge stop()/reload only signalled the dispatch loop, never
/// `ChannelAdapter::stop()`, leaking the adapter's internal poller — the
/// Telegram getUpdates task kept running after a config hot-reload and
/// fought its replacement for updates (409 Conflict storm).
#[tokio::test]
async fn test_stop_and_reload_call_adapter_stop() {
    let handle = Arc::new(MockHandle::new(vec![]));
    let router = Arc::new(AgentRouter::new());

    let (adapter, _tx) = MockAdapter::new("test-adapter", ChannelType::Telegram);
    let mut manager = BridgeManager::new(handle, router);
    manager.start_adapter(adapter.clone()).await.unwrap();
    let stopped = |a: &MockAdapter| a.stopped.load(std::sync::atomic::Ordering::SeqCst);
    assert!(!stopped(&adapter));

    let (replacement, _tx2) = MockAdapter::new("test-adapter", ChannelType::Telegram);
    manager
        .reload_channel("test-adapter", replacement.clone())
        .await
        .unwrap();
    assert!(
        stopped(&adapter),
        "reload must stop the previous adapter's poller"
    );
    assert!(!stopped(&replacement));

    manager.stop().await;
    assert!(stopped(&replacement), "stop must stop the adapter's poller");
}
