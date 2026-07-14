//! Discord Gateway adapter for the Captain channel bridge.
//!
//! Uses Discord Gateway WebSocket (v10) for receiving messages and the REST API
//! for sending responses. No external Discord crate — just `tokio-tungstenite` + `reqwest`.

use crate::rbac;
use crate::types::{
    split_message, ChannelAdapter, ChannelContent, ChannelMessage, ChannelType, ChannelUser,
};
use async_trait::async_trait;
use futures::{SinkExt, Stream, StreamExt};
use std::collections::HashMap;
use std::pin::Pin;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{mpsc, watch, RwLock};
use tracing::{debug, error, info, warn};
use zeroize::Zeroizing;

const DISCORD_API_BASE: &str = "https://discord.com/api/v10";
const MAX_BACKOFF: Duration = Duration::from_secs(60);
const INITIAL_BACKOFF: Duration = Duration::from_secs(1);
const DISCORD_MSG_LIMIT: usize = 2000;

type DiscordWsStream =
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;
type DiscordWsSink =
    futures::stream::SplitSink<DiscordWsStream, tokio_tungstenite::tungstenite::Message>;
type DiscordWsRead = futures::stream::SplitStream<DiscordWsStream>;

/// Discord Gateway opcodes.
mod opcode {
    pub const DISPATCH: u64 = 0;
    pub const HEARTBEAT: u64 = 1;
    pub const IDENTIFY: u64 = 2;
    pub const RESUME: u64 = 6;
    pub const RECONNECT: u64 = 7;
    pub const INVALID_SESSION: u64 = 9;
    pub const HELLO: u64 = 10;
    pub const HEARTBEAT_ACK: u64 = 11;
}

/// Discord Gateway adapter using WebSocket.
pub struct DiscordAdapter {
    /// SECURITY: Bot token is zeroized on drop to prevent memory disclosure.
    token: Zeroizing<String>,
    client: reqwest::Client,
    allowed_guilds: Vec<String>,
    allowed_users: Vec<String>,
    ignore_bots: bool,
    intents: u64,
    shutdown_tx: Arc<watch::Sender<bool>>,
    shutdown_rx: watch::Receiver<bool>,
    /// Bot's own user ID (populated after READY event).
    bot_user_id: Arc<RwLock<Option<String>>>,
    /// Session ID for resume (populated after READY event).
    session_id: Arc<RwLock<Option<String>>>,
    /// Resume gateway URL.
    resume_gateway_url: Arc<RwLock<Option<String>>>,
}

struct DiscordGatewayContext {
    token: Zeroizing<String>,
    intents: u64,
    allowed_guilds: Vec<String>,
    allowed_users: Vec<String>,
    ignore_bots: bool,
    bot_user_id: Arc<RwLock<Option<String>>>,
    session_id_store: Arc<RwLock<Option<String>>>,
    resume_url_store: Arc<RwLock<Option<String>>>,
    shutdown: watch::Receiver<bool>,
    tx: mpsc::Sender<ChannelMessage>,
}

enum DiscordConnectionOutcome {
    Reconnect,
    Stop,
}

enum DiscordGatewayAction {
    Continue,
    Reconnect,
    Stop,
}

impl DiscordAdapter {
    pub fn new(
        token: String,
        allowed_guilds: Vec<String>,
        allowed_users: Vec<String>,
        ignore_bots: bool,
        intents: u64,
    ) -> Self {
        let (shutdown_tx, shutdown_rx) = watch::channel(false);
        Self {
            token: Zeroizing::new(token),
            client: reqwest::Client::new(),
            allowed_guilds,
            allowed_users,
            ignore_bots,
            intents,
            shutdown_tx: Arc::new(shutdown_tx),
            shutdown_rx,
            bot_user_id: Arc::new(RwLock::new(None)),
            session_id: Arc::new(RwLock::new(None)),
            resume_gateway_url: Arc::new(RwLock::new(None)),
        }
    }

    /// Get the WebSocket gateway URL from the Discord API.
    async fn get_gateway_url(&self) -> Result<String, Box<dyn std::error::Error>> {
        let url = format!("{DISCORD_API_BASE}/gateway/bot");
        let resp: serde_json::Value = self
            .client
            .get(&url)
            .header("Authorization", format!("Bot {}", self.token.as_str()))
            .send()
            .await?
            .json()
            .await?;

        let ws_url = resp["url"]
            .as_str()
            .ok_or("Missing 'url' in gateway response")?;

        Ok(format!("{ws_url}/?v=10&encoding=json"))
    }

    /// Send a message to a Discord channel via REST API.
    async fn api_send_message(
        &self,
        channel_id: &str,
        text: &str,
    ) -> Result<(), Box<dyn std::error::Error>> {
        let url = format!("{DISCORD_API_BASE}/channels/{channel_id}/messages");
        let chunks = split_message(text, DISCORD_MSG_LIMIT);

        for chunk in chunks {
            let body = crate::discord_message::discord_message_payload(chunk);
            let resp = self
                .client
                .post(&url)
                .header("Authorization", format!("Bot {}", self.token.as_str()))
                .json(&body)
                .send()
                .await?;

            if !resp.status().is_success() {
                let body_text = resp.text().await.unwrap_or_default();
                warn!("Discord sendMessage failed: {body_text}");
            }
        }
        Ok(())
    }

    /// Send typing indicator to a Discord channel.
    async fn api_send_typing(&self, channel_id: &str) -> Result<(), Box<dyn std::error::Error>> {
        let url = format!("{DISCORD_API_BASE}/channels/{channel_id}/typing");
        let _ = self
            .client
            .post(&url)
            .header("Authorization", format!("Bot {}", self.token.as_str()))
            .send()
            .await?;
        Ok(())
    }
}

#[async_trait]
impl ChannelAdapter for DiscordAdapter {
    fn name(&self) -> &str {
        "discord"
    }

    fn channel_type(&self) -> ChannelType {
        ChannelType::Discord
    }

    async fn start(
        &self,
    ) -> Result<Pin<Box<dyn Stream<Item = ChannelMessage> + Send>>, Box<dyn std::error::Error>>
    {
        let gateway_url = self.get_gateway_url().await?;
        info!("Discord gateway URL obtained");

        let (tx, rx) = mpsc::channel::<ChannelMessage>(256);

        let gateway_context = DiscordGatewayContext {
            token: self.token.clone(),
            intents: self.intents,
            allowed_guilds: self.allowed_guilds.clone(),
            allowed_users: self.allowed_users.clone(),
            ignore_bots: self.ignore_bots,
            bot_user_id: self.bot_user_id.clone(),
            session_id_store: self.session_id.clone(),
            resume_url_store: self.resume_gateway_url.clone(),
            shutdown: self.shutdown_rx.clone(),
            tx,
        };
        tokio::spawn(async move {
            run_discord_gateway_loop(gateway_context, gateway_url).await;
        });

        let stream = tokio_stream::wrappers::ReceiverStream::new(rx);
        Ok(Box::pin(stream))
    }

    async fn send(
        &self,
        user: &ChannelUser,
        content: ChannelContent,
    ) -> Result<(), Box<dyn std::error::Error>> {
        // platform_id is the channel_id for Discord
        let channel_id = &user.platform_id;
        match content {
            ChannelContent::Text(text) => {
                self.api_send_message(channel_id, &text).await?;
            }
            _ => {
                self.api_send_message(channel_id, "(Unsupported content type)")
                    .await?;
            }
        }
        Ok(())
    }

    async fn send_typing(&self, user: &ChannelUser) -> Result<(), Box<dyn std::error::Error>> {
        self.api_send_typing(&user.platform_id).await
    }

    async fn stop(&self) -> Result<(), Box<dyn std::error::Error>> {
        let _ = self.shutdown_tx.send(true);
        Ok(())
    }
}

async fn run_discord_gateway_loop(mut ctx: DiscordGatewayContext, gateway_url: String) {
    let mut backoff = INITIAL_BACKOFF;
    let mut connect_url = gateway_url;
    // Sequence persists across reconnections for RESUME.
    let sequence: Arc<RwLock<Option<u64>>> = Arc::new(RwLock::new(None));

    loop {
        if *ctx.shutdown.borrow() {
            break;
        }

        info!("Connecting to Discord gateway...");
        let ws_result = tokio_tungstenite::connect_async(&connect_url).await;
        let ws_stream = match ws_result {
            Ok((stream, _)) => stream,
            Err(e) => {
                warn!("Discord gateway connection failed: {e}, retrying in {backoff:?}");
                tokio::time::sleep(backoff).await;
                backoff = calculate_discord_backoff(backoff);
                continue;
            }
        };

        backoff = INITIAL_BACKOFF;
        info!("Discord gateway connected");

        let (mut ws_tx, mut ws_rx) = ws_stream.split();
        match run_discord_gateway_connection(&mut ctx, &mut ws_tx, &mut ws_rx, &sequence).await {
            DiscordConnectionOutcome::Stop => break,
            DiscordConnectionOutcome::Reconnect => {}
        }

        if *ctx.shutdown.borrow() {
            break;
        }

        if let Some(ref url) = *ctx.resume_url_store.read().await {
            connect_url = format!("{url}/?v=10&encoding=json");
        }

        warn!("Discord: reconnecting in {backoff:?}");
        tokio::time::sleep(backoff).await;
        backoff = calculate_discord_backoff(backoff);
    }

    info!("Discord gateway loop stopped");
}

async fn run_discord_gateway_connection(
    ctx: &mut DiscordGatewayContext,
    ws_tx: &mut DiscordWsSink,
    ws_rx: &mut DiscordWsRead,
    sequence: &Arc<RwLock<Option<u64>>>,
) -> DiscordConnectionOutcome {
    // Set once HELLO reports heartbeat_interval. Discord requires the
    // client to heartbeat proactively on this cadence and closes the
    // connection (code 4009, session timeout) if it doesn't — this loop
    // used to parse the interval into a throwaway variable and never
    // acted on it, so every connection got disconnected roughly every
    // heartbeat_interval and reconnected in a loop (observed live as
    // repeated "Discord: reconnecting in 1s").
    let mut heartbeat_timer: Option<tokio::time::Interval> = None;

    loop {
        tokio::select! {
            msg = ws_rx.next() => {
                let msg = match msg {
                    Some(Ok(message)) => message,
                    Some(Err(e)) => {
                        warn!("Discord WebSocket error: {e}");
                        return DiscordConnectionOutcome::Reconnect;
                    }
                    None => {
                        info!("Discord WebSocket closed");
                        return DiscordConnectionOutcome::Reconnect;
                    }
                };

                let Some(payload) = discord_gateway_payload_from_message(msg) else {
                    continue;
                };
                if let Some(s) = payload["s"].as_u64() {
                    *sequence.write().await = Some(s);
                }

                let is_hello = payload["op"].as_u64() == Some(opcode::HELLO);
                match handle_discord_gateway_payload(ctx, ws_tx, sequence, &payload).await {
                    DiscordGatewayAction::Continue => {}
                    DiscordGatewayAction::Reconnect => return DiscordConnectionOutcome::Reconnect,
                    DiscordGatewayAction::Stop => return DiscordConnectionOutcome::Stop,
                }

                if is_hello {
                    if let Some(interval_ms) = payload["d"]["heartbeat_interval"].as_u64() {
                        heartbeat_timer = Some(discord_heartbeat_timer(interval_ms));
                    }
                }
            }
            _ = ctx.shutdown.changed() => {
                if *ctx.shutdown.borrow() {
                    info!("Discord shutdown requested");
                    let _ = ws_tx.close().await;
                    return DiscordConnectionOutcome::Stop;
                }
            }
            _ = discord_heartbeat_tick(&mut heartbeat_timer), if heartbeat_timer.is_some() => {
                send_discord_heartbeat(ws_tx, sequence).await;
            }
        }
    }
}

/// Interval that fires every `interval_ms`, first tick delayed by a full
/// period (IDENTIFY/RESUME was just sent in `handle_discord_hello`, an
/// immediate heartbeat isn't needed — `tokio::time::interval` otherwise
/// completes its first tick right away).
fn discord_heartbeat_timer(interval_ms: u64) -> tokio::time::Interval {
    let mut interval = tokio::time::interval(Duration::from_millis(interval_ms));
    interval.reset();
    interval
}

async fn discord_heartbeat_tick(timer: &mut Option<tokio::time::Interval>) {
    match timer {
        Some(timer) => {
            timer.tick().await;
        }
        None => std::future::pending().await,
    }
}

fn discord_gateway_payload_from_message(
    msg: tokio_tungstenite::tungstenite::Message,
) -> Option<serde_json::Value> {
    let text = match msg {
        tokio_tungstenite::tungstenite::Message::Text(text) => text,
        tokio_tungstenite::tungstenite::Message::Close(_) => {
            info!("Discord gateway closed by server");
            return None;
        }
        _ => return None,
    };

    match serde_json::from_str(&text) {
        Ok(value) => Some(value),
        Err(e) => {
            warn!("Discord: failed to parse gateway message: {e}");
            None
        }
    }
}

async fn handle_discord_gateway_payload(
    ctx: &DiscordGatewayContext,
    ws_tx: &mut DiscordWsSink,
    sequence: &Arc<RwLock<Option<u64>>>,
    payload: &serde_json::Value,
) -> DiscordGatewayAction {
    match payload["op"].as_u64().unwrap_or(999) {
        opcode::HELLO => handle_discord_hello(ctx, ws_tx, sequence, payload).await,
        opcode::DISPATCH => handle_discord_dispatch(ctx, payload).await,
        opcode::HEARTBEAT => {
            send_discord_heartbeat(ws_tx, sequence).await;
            DiscordGatewayAction::Continue
        }
        opcode::HEARTBEAT_ACK => {
            debug!("Discord heartbeat ACK received");
            DiscordGatewayAction::Continue
        }
        opcode::RECONNECT => {
            info!("Discord: server requested reconnect");
            DiscordGatewayAction::Reconnect
        }
        opcode::INVALID_SESSION => handle_discord_invalid_session(ctx, sequence, payload).await,
        op => {
            debug!("Discord: unknown opcode {op}");
            DiscordGatewayAction::Continue
        }
    }
}

async fn handle_discord_hello(
    ctx: &DiscordGatewayContext,
    ws_tx: &mut DiscordWsSink,
    sequence: &Arc<RwLock<Option<u64>>>,
    payload: &serde_json::Value,
) -> DiscordGatewayAction {
    let interval = payload["d"]["heartbeat_interval"].as_u64().unwrap_or(45000);
    debug!("Discord HELLO: heartbeat_interval={interval}ms");

    let gateway_msg = build_discord_resume_or_identify(ctx, sequence).await;
    if let Err(e) = send_discord_gateway_json(ws_tx, &gateway_msg).await {
        error!("Discord: failed to send IDENTIFY/RESUME: {e}");
        return DiscordGatewayAction::Reconnect;
    }
    DiscordGatewayAction::Continue
}

async fn build_discord_resume_or_identify(
    ctx: &DiscordGatewayContext,
    sequence: &Arc<RwLock<Option<u64>>>,
) -> serde_json::Value {
    let session_id = ctx.session_id_store.read().await.clone();
    let seq = *sequence.read().await;

    if let (Some(sid), Some(seq)) = (session_id, seq) {
        info!("Discord: sending RESUME (session={sid})");
        return serde_json::json!({
            "op": opcode::RESUME,
            "d": {
                "token": ctx.token.as_str(),
                "session_id": sid,
                "seq": seq
            }
        });
    }

    info!("Discord: sending IDENTIFY");
    serde_json::json!({
        "op": opcode::IDENTIFY,
        "d": {
            "token": ctx.token.as_str(),
            "intents": ctx.intents,
            "properties": {
                "os": "linux",
                "browser": "captain",
                "device": "captain"
            }
        }
    })
}

async fn handle_discord_dispatch(
    ctx: &DiscordGatewayContext,
    payload: &serde_json::Value,
) -> DiscordGatewayAction {
    let event_name = payload["t"].as_str().unwrap_or("");
    let d = &payload["d"];

    match event_name {
        "READY" => {
            handle_discord_ready(ctx, d).await;
            DiscordGatewayAction::Continue
        }
        "MESSAGE_CREATE" | "MESSAGE_UPDATE" => {
            handle_discord_message_event(ctx, event_name, d).await
        }
        "RESUMED" => {
            info!("Discord session resumed successfully");
            DiscordGatewayAction::Continue
        }
        _ => {
            debug!("Discord event: {event_name}");
            DiscordGatewayAction::Continue
        }
    }
}

async fn handle_discord_ready(ctx: &DiscordGatewayContext, d: &serde_json::Value) {
    let user_id = d["user"]["id"].as_str().unwrap_or("").to_string();
    let username = d["user"]["username"].as_str().unwrap_or("unknown");
    let sid = d["session_id"].as_str().unwrap_or("").to_string();
    let resume_url = d["resume_gateway_url"].as_str().unwrap_or("").to_string();

    *ctx.bot_user_id.write().await = Some(user_id.clone());
    *ctx.session_id_store.write().await = Some(sid);
    if !resume_url.is_empty() {
        *ctx.resume_url_store.write().await = Some(resume_url);
    }

    info!("Discord bot ready: {username} ({user_id})");
}

async fn handle_discord_message_event(
    ctx: &DiscordGatewayContext,
    event_name: &str,
    d: &serde_json::Value,
) -> DiscordGatewayAction {
    let Some(msg) = parse_discord_message(
        d,
        &ctx.bot_user_id,
        &ctx.allowed_guilds,
        &ctx.allowed_users,
        ctx.ignore_bots,
    )
    .await
    else {
        return DiscordGatewayAction::Continue;
    };

    debug!(
        "Discord {event_name} from {}: {:?}",
        msg.sender.display_name, msg.content
    );

    if ctx.tx.send(msg).await.is_err() {
        return DiscordGatewayAction::Stop;
    }

    DiscordGatewayAction::Continue
}

async fn send_discord_heartbeat(ws_tx: &mut DiscordWsSink, sequence: &Arc<RwLock<Option<u64>>>) {
    let seq = *sequence.read().await;
    let hb = serde_json::json!({ "op": opcode::HEARTBEAT, "d": seq });
    let _ = send_discord_gateway_json(ws_tx, &hb).await;
}

async fn handle_discord_invalid_session(
    ctx: &DiscordGatewayContext,
    sequence: &Arc<RwLock<Option<u64>>>,
    payload: &serde_json::Value,
) -> DiscordGatewayAction {
    let resumable = payload["d"].as_bool().unwrap_or(false);
    if resumable {
        info!("Discord: invalid session (resumable)");
    } else {
        info!("Discord: invalid session (not resumable), clearing session");
        *ctx.session_id_store.write().await = None;
        *sequence.write().await = None;
    }
    DiscordGatewayAction::Reconnect
}

async fn send_discord_gateway_json(
    ws_tx: &mut DiscordWsSink,
    value: &serde_json::Value,
) -> Result<(), tokio_tungstenite::tungstenite::Error> {
    ws_tx
        .send(tokio_tungstenite::tungstenite::Message::Text(
            serde_json::to_string(value).unwrap(),
        ))
        .await
}

fn calculate_discord_backoff(current: Duration) -> Duration {
    (current * 2).min(MAX_BACKOFF)
}

/// Parse a Discord MESSAGE_CREATE or MESSAGE_UPDATE payload into a `ChannelMessage`.
async fn parse_discord_message(
    d: &serde_json::Value,
    bot_user_id: &Arc<RwLock<Option<String>>>,
    allowed_guilds: &[String],
    allowed_users: &[String],
    ignore_bots: bool,
) -> Option<ChannelMessage> {
    let author = d.get("author")?;
    let author_id = author["id"].as_str()?;

    if discord_author_is_filtered(author, author_id, bot_user_id, allowed_users, ignore_bots).await
    {
        return None;
    }

    if !discord_guild_is_allowed(d, allowed_guilds) {
        return None;
    }

    let content_text = d["content"].as_str().unwrap_or("");
    if content_text.is_empty() {
        return None;
    }

    let channel_id = d["channel_id"].as_str()?;
    let message_id = d["id"].as_str().unwrap_or("0");
    let is_group = d["guild_id"].as_str().is_some();
    let was_mentioned = discord_message_was_mentioned(d, content_text, bot_user_id).await;

    Some(ChannelMessage {
        channel: ChannelType::Discord,
        platform_message_id: message_id.to_string(),
        sender: ChannelUser {
            platform_id: channel_id.to_string(),
            display_name: discord_display_name(author),
            captain_user: None,
        },
        content: discord_content_from_text(content_text),
        target_agent: None,
        timestamp: discord_message_timestamp(d),
        is_group,
        thread_id: None,
        metadata: discord_message_metadata(was_mentioned),
    })
}

async fn discord_author_is_filtered(
    author: &serde_json::Value,
    author_id: &str,
    bot_user_id: &Arc<RwLock<Option<String>>>,
    allowed_users: &[String],
    ignore_bots: bool,
) -> bool {
    if let Some(ref bid) = *bot_user_id.read().await {
        if author_id == bid {
            return true;
        }
    }

    if ignore_bots && author["bot"].as_bool() == Some(true) {
        return true;
    }

    if !rbac::is_authorized(allowed_users, author_id) {
        debug!("Discord: ignoring message from unlisted user {author_id}");
        return true;
    }

    false
}

fn discord_guild_is_allowed(d: &serde_json::Value, allowed_guilds: &[String]) -> bool {
    if allowed_guilds.is_empty() {
        return true;
    }

    d["guild_id"]
        .as_str()
        .is_none_or(|guild_id| allowed_guilds.iter().any(|g| g == guild_id))
}

fn discord_display_name(author: &serde_json::Value) -> String {
    let username = author["username"].as_str().unwrap_or("Unknown");
    let discriminator = author["discriminator"].as_str().unwrap_or("0000");
    if discriminator == "0" {
        username.to_string()
    } else {
        format!("{username}#{discriminator}")
    }
}

fn discord_content_from_text(content_text: &str) -> ChannelContent {
    if !content_text.starts_with('/') {
        return ChannelContent::Text(content_text.to_string());
    }

    let parts: Vec<&str> = content_text.splitn(2, ' ').collect();
    let args = if parts.len() > 1 {
        parts[1].split_whitespace().map(String::from).collect()
    } else {
        vec![]
    };
    ChannelContent::Command {
        name: parts[0][1..].to_string(),
        args,
    }
}

fn discord_message_timestamp(d: &serde_json::Value) -> chrono::DateTime<chrono::Utc> {
    d["timestamp"]
        .as_str()
        .and_then(|ts| chrono::DateTime::parse_from_rfc3339(ts).ok())
        .map(|dt| dt.with_timezone(&chrono::Utc))
        .unwrap_or_else(chrono::Utc::now)
}

async fn discord_message_was_mentioned(
    d: &serde_json::Value,
    content_text: &str,
    bot_user_id: &Arc<RwLock<Option<String>>>,
) -> bool {
    let Some(ref bid) = *bot_user_id.read().await else {
        return false;
    };

    let mentioned_in_array = d["mentions"]
        .as_array()
        .map(|arr| arr.iter().any(|m| m["id"].as_str() == Some(bid.as_str())))
        .unwrap_or(false);
    let mentioned_in_content =
        content_text.contains(&format!("<@{bid}>")) || content_text.contains(&format!("<@!{bid}>"));
    mentioned_in_array || mentioned_in_content
}

fn discord_message_metadata(was_mentioned: bool) -> HashMap<String, serde_json::Value> {
    let mut metadata = HashMap::new();
    if was_mentioned {
        metadata.insert("was_mentioned".to_string(), serde_json::json!(true));
    }
    metadata
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn discord_heartbeat_timer_uses_hello_interval() {
        let timer = discord_heartbeat_timer(41_250);
        assert_eq!(timer.period(), Duration::from_millis(41_250));
    }

    /// Live bug: HELLO's heartbeat_interval was parsed and discarded, so
    /// no heartbeat was ever sent proactively and Discord closed the
    /// connection every ~41-45s. With virtual time, the timer must fire
    /// once a full period has elapsed and not before.
    #[tokio::test(start_paused = true)]
    async fn discord_heartbeat_tick_fires_after_full_period_not_before() {
        let mut timer = Some(discord_heartbeat_timer(1_000));

        // Comfortable margin on both sides of the deadline, so the two
        // timers involved (this timeout and the interval's own tick)
        // never land close enough to race each other under the paused
        // clock's auto-advance.
        tokio::time::advance(Duration::from_millis(500)).await;
        assert!(
            tokio::time::timeout(
                Duration::from_millis(100),
                discord_heartbeat_tick(&mut timer)
            )
            .await
            .is_err(),
            "must not fire before a full period has elapsed"
        );

        tokio::time::advance(Duration::from_millis(600)).await;
        tokio::time::timeout(
            Duration::from_millis(100),
            discord_heartbeat_tick(&mut timer),
        )
        .await
        .expect("must fire once the period has elapsed");
    }

    #[tokio::test]
    async fn discord_heartbeat_tick_never_fires_without_a_timer() {
        let mut timer: Option<tokio::time::Interval> = None;
        assert!(
            tokio::time::timeout(
                Duration::from_millis(50),
                discord_heartbeat_tick(&mut timer)
            )
            .await
            .is_err(),
            "no HELLO received yet — must stay pending forever, not fire spuriously"
        );
    }

    async fn parse_discord_message_allow_all(
        d: &serde_json::Value,
        bot_user_id: &Arc<RwLock<Option<String>>>,
        allowed_guilds: &[String],
        ignore_bots: bool,
    ) -> Option<ChannelMessage> {
        let allowed_users = vec!["*".to_string()];
        parse_discord_message(d, bot_user_id, allowed_guilds, &allowed_users, ignore_bots).await
    }

    #[tokio::test]
    async fn test_parse_discord_message_basic() {
        let bot_id = Arc::new(RwLock::new(Some("bot123".to_string())));
        let d = serde_json::json!({
            "id": "msg1",
            "channel_id": "ch1",
            "content": "Hello agent!",
            "author": {
                "id": "user456",
                "username": "alice",
                "discriminator": "0",
                "bot": false
            },
            "timestamp": "2024-01-01T00:00:00+00:00"
        });

        let msg = parse_discord_message_allow_all(&d, &bot_id, &[], true)
            .await
            .unwrap();
        assert_eq!(msg.channel, ChannelType::Discord);
        assert_eq!(msg.sender.display_name, "alice");
        assert_eq!(msg.sender.platform_id, "ch1");
        assert!(matches!(msg.content, ChannelContent::Text(ref t) if t == "Hello agent!"));
    }

    #[tokio::test]
    async fn test_parse_discord_message_filters_bot() {
        let bot_id = Arc::new(RwLock::new(Some("bot123".to_string())));
        let d = serde_json::json!({
            "id": "msg1",
            "channel_id": "ch1",
            "content": "My own message",
            "author": {
                "id": "bot123",
                "username": "captain",
                "discriminator": "0"
            },
            "timestamp": "2024-01-01T00:00:00+00:00"
        });

        let msg = parse_discord_message_allow_all(&d, &bot_id, &[], true).await;
        assert!(msg.is_none());
    }

    #[tokio::test]
    async fn test_parse_discord_message_filters_other_bots() {
        let bot_id = Arc::new(RwLock::new(Some("bot123".to_string())));
        let d = serde_json::json!({
            "id": "msg1",
            "channel_id": "ch1",
            "content": "Bot message",
            "author": {
                "id": "other_bot",
                "username": "somebot",
                "discriminator": "0",
                "bot": true
            },
            "timestamp": "2024-01-01T00:00:00+00:00"
        });

        let msg = parse_discord_message_allow_all(&d, &bot_id, &[], true).await;
        assert!(msg.is_none());
    }

    #[tokio::test]
    async fn test_parse_discord_ignore_bots_false_allows_other_bots() {
        let bot_id = Arc::new(RwLock::new(Some("bot123".to_string())));
        let d = serde_json::json!({
            "id": "msg1",
            "channel_id": "ch1",
            "content": "Bot message",
            "author": {
                "id": "other_bot",
                "username": "somebot",
                "discriminator": "0",
                "bot": true
            },
            "timestamp": "2024-01-01T00:00:00+00:00"
        });

        // With ignore_bots=false, other bots' messages should be allowed
        let msg = parse_discord_message_allow_all(&d, &bot_id, &[], false).await;
        assert!(msg.is_some());
        let msg = msg.unwrap();
        assert_eq!(msg.sender.display_name, "somebot");
        assert!(matches!(msg.content, ChannelContent::Text(ref t) if t == "Bot message"));
    }

    #[tokio::test]
    async fn test_parse_discord_ignore_bots_false_still_filters_self() {
        let bot_id = Arc::new(RwLock::new(Some("bot123".to_string())));
        let d = serde_json::json!({
            "id": "msg1",
            "channel_id": "ch1",
            "content": "My own message",
            "author": {
                "id": "bot123",
                "username": "captain",
                "discriminator": "0",
                "bot": true
            },
            "timestamp": "2024-01-01T00:00:00+00:00"
        });

        // Even with ignore_bots=false, the bot's own messages must still be filtered
        let msg = parse_discord_message_allow_all(&d, &bot_id, &[], false).await;
        assert!(msg.is_none());
    }

    #[tokio::test]
    async fn test_parse_discord_message_guild_filter() {
        let bot_id = Arc::new(RwLock::new(Some("bot123".to_string())));
        let d = serde_json::json!({
            "id": "msg1",
            "channel_id": "ch1",
            "guild_id": "999",
            "content": "Hello",
            "author": {
                "id": "user1",
                "username": "bob",
                "discriminator": "0"
            },
            "timestamp": "2024-01-01T00:00:00+00:00"
        });

        // Not in allowed guilds
        let msg =
            parse_discord_message_allow_all(&d, &bot_id, &["111".into(), "222".into()], true).await;
        assert!(msg.is_none());

        // In allowed guilds
        let msg = parse_discord_message_allow_all(&d, &bot_id, &["999".into()], true).await;
        assert!(msg.is_some());
    }

    #[tokio::test]
    async fn test_parse_discord_command() {
        let bot_id = Arc::new(RwLock::new(None));
        let d = serde_json::json!({
            "id": "msg1",
            "channel_id": "ch1",
            "content": "/agent hello-world",
            "author": {
                "id": "user1",
                "username": "alice",
                "discriminator": "0"
            },
            "timestamp": "2024-01-01T00:00:00+00:00"
        });

        let msg = parse_discord_message_allow_all(&d, &bot_id, &[], true)
            .await
            .unwrap();
        match &msg.content {
            ChannelContent::Command { name, args } => {
                assert_eq!(name, "agent");
                assert_eq!(args, &["hello-world"]);
            }
            other => panic!("Expected Command, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn test_parse_discord_empty_content() {
        let bot_id = Arc::new(RwLock::new(None));
        let d = serde_json::json!({
            "id": "msg1",
            "channel_id": "ch1",
            "content": "",
            "author": {
                "id": "user1",
                "username": "alice",
                "discriminator": "0"
            },
            "timestamp": "2024-01-01T00:00:00+00:00"
        });

        let msg = parse_discord_message_allow_all(&d, &bot_id, &[], true).await;
        assert!(msg.is_none());
    }

    #[tokio::test]
    async fn test_parse_discord_discriminator() {
        let bot_id = Arc::new(RwLock::new(None));
        let d = serde_json::json!({
            "id": "msg1",
            "channel_id": "ch1",
            "content": "Hi",
            "author": {
                "id": "user1",
                "username": "alice",
                "discriminator": "1234"
            },
            "timestamp": "2024-01-01T00:00:00+00:00"
        });

        let msg = parse_discord_message_allow_all(&d, &bot_id, &[], true)
            .await
            .unwrap();
        assert_eq!(msg.sender.display_name, "alice#1234");
    }

    #[tokio::test]
    async fn test_parse_discord_message_update() {
        let bot_id = Arc::new(RwLock::new(Some("bot123".to_string())));
        let d = serde_json::json!({
            "id": "msg1",
            "channel_id": "ch1",
            "content": "Edited message content",
            "author": {
                "id": "user456",
                "username": "alice",
                "discriminator": "0",
                "bot": false
            },
            "timestamp": "2024-01-01T00:00:00+00:00",
            "edited_timestamp": "2024-01-01T00:01:00+00:00"
        });

        // MESSAGE_UPDATE uses the same parse function as MESSAGE_CREATE
        let msg = parse_discord_message_allow_all(&d, &bot_id, &[], true)
            .await
            .unwrap();
        assert_eq!(msg.channel, ChannelType::Discord);
        assert!(
            matches!(msg.content, ChannelContent::Text(ref t) if t == "Edited message content")
        );
    }

    #[tokio::test]
    async fn test_parse_discord_allowed_users_filter() {
        let bot_id = Arc::new(RwLock::new(Some("bot123".to_string())));
        let d = serde_json::json!({
            "id": "msg1",
            "channel_id": "ch1",
            "content": "Hello",
            "author": {
                "id": "user999",
                "username": "bob",
                "discriminator": "0"
            },
            "timestamp": "2024-01-01T00:00:00+00:00"
        });

        // Not in allowed users
        let msg = parse_discord_message(
            &d,
            &bot_id,
            &[],
            &["user111".into(), "user222".into()],
            true,
        )
        .await;
        assert!(msg.is_none());

        // In allowed users
        let msg = parse_discord_message(&d, &bot_id, &[], &["user999".into()], true).await;
        assert!(msg.is_some());

        // Empty allowed_users = deny all
        let msg = parse_discord_message(&d, &bot_id, &[], &[], true).await;
        assert!(msg.is_none());

        // Wildcard explicitly allows all users
        let msg = parse_discord_message(&d, &bot_id, &[], &["*".into()], true).await;
        assert!(msg.is_some());
    }

    #[tokio::test]
    async fn test_parse_discord_mention_detection() {
        let bot_id = Arc::new(RwLock::new(Some("bot123".to_string())));

        // Message with bot mentioned in mentions array
        let d = serde_json::json!({
            "id": "msg1",
            "channel_id": "ch1",
            "guild_id": "guild1",
            "content": "Hey <@bot123> help me",
            "mentions": [{"id": "bot123", "username": "captain"}],
            "author": {
                "id": "user1",
                "username": "alice",
                "discriminator": "0"
            },
            "timestamp": "2024-01-01T00:00:00+00:00"
        });

        let msg = parse_discord_message_allow_all(&d, &bot_id, &[], true)
            .await
            .unwrap();
        assert!(msg.is_group);
        assert_eq!(
            msg.metadata.get("was_mentioned").and_then(|v| v.as_bool()),
            Some(true)
        );

        // Message without mention in group
        let d2 = serde_json::json!({
            "id": "msg2",
            "channel_id": "ch1",
            "guild_id": "guild1",
            "content": "Just chatting",
            "author": {
                "id": "user1",
                "username": "alice",
                "discriminator": "0"
            },
            "timestamp": "2024-01-01T00:00:00+00:00"
        });

        let msg2 = parse_discord_message_allow_all(&d2, &bot_id, &[], true)
            .await
            .unwrap();
        assert!(msg2.is_group);
        assert!(!msg2.metadata.contains_key("was_mentioned"));
    }

    #[tokio::test]
    async fn test_parse_discord_dm_not_group() {
        let bot_id = Arc::new(RwLock::new(None));
        let d = serde_json::json!({
            "id": "msg1",
            "channel_id": "dm-ch1",
            "content": "Hello",
            "author": {
                "id": "user1",
                "username": "alice",
                "discriminator": "0"
            },
            "timestamp": "2024-01-01T00:00:00+00:00"
        });

        let msg = parse_discord_message_allow_all(&d, &bot_id, &[], true)
            .await
            .unwrap();
        assert!(!msg.is_group);
    }

    #[test]
    fn test_discord_adapter_creation() {
        let adapter = DiscordAdapter::new(
            "test-token".to_string(),
            vec!["123".to_string(), "456".to_string()],
            vec![],
            true,
            37376,
        );
        assert_eq!(adapter.name(), "discord");
        assert_eq!(adapter.channel_type(), ChannelType::Discord);
    }

    #[test]
    fn discord_backoff_doubles_until_cap() {
        assert_eq!(
            calculate_discord_backoff(Duration::from_secs(1)),
            Duration::from_secs(2)
        );
        assert_eq!(
            calculate_discord_backoff(Duration::from_secs(45)),
            MAX_BACKOFF
        );
    }

    #[tokio::test]
    async fn discord_gateway_identify_and_resume_payloads_are_stable() {
        let (_shutdown_tx, shutdown) = watch::channel(false);
        let (tx, _rx) = mpsc::channel(1);
        let session_id_store = Arc::new(RwLock::new(None));
        let ctx = DiscordGatewayContext {
            token: Zeroizing::new("test-token".to_string()),
            intents: 42,
            allowed_guilds: Vec::new(),
            allowed_users: vec!["*".to_string()],
            ignore_bots: true,
            bot_user_id: Arc::new(RwLock::new(None)),
            session_id_store: session_id_store.clone(),
            resume_url_store: Arc::new(RwLock::new(None)),
            shutdown,
            tx,
        };
        let sequence = Arc::new(RwLock::new(None));

        let identify = build_discord_resume_or_identify(&ctx, &sequence).await;
        assert_eq!(identify["op"].as_u64(), Some(opcode::IDENTIFY));
        assert_eq!(identify["d"]["token"].as_str(), Some("test-token"));
        assert_eq!(identify["d"]["intents"].as_u64(), Some(42));

        *session_id_store.write().await = Some("session-1".to_string());
        *sequence.write().await = Some(99);
        let resume = build_discord_resume_or_identify(&ctx, &sequence).await;
        assert_eq!(resume["op"].as_u64(), Some(opcode::RESUME));
        assert_eq!(resume["d"]["session_id"].as_str(), Some("session-1"));
        assert_eq!(resume["d"]["seq"].as_u64(), Some(99));
    }
}
