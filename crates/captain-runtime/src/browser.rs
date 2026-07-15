//! Native browser automation via Chrome DevTools Protocol (CDP).
//!
//! Direct WebSocket connection to Chromium. No Python, no Playwright.
//! Launches a Chromium process, connects over CDP WebSocket, and sends
//! JSON-RPC commands for navigation, interaction, screenshots, etc.
//!
//! # Security
//! - SSRF check runs in Rust before navigate commands
//! - All page content wrapped with `wrap_external_content()` markers
//! - Session limits: max concurrent, idle timeout, 1 per agent
//! - No subprocess bridge, no env leakage, no Python code execution

use captain_types::config::BrowserConfig;
use captain_types::message::ContentBlock;
use dashmap::DashMap;
use futures::stream::{SplitSink, SplitStream};
use futures::{SinkExt, StreamExt};
use serde::{Deserialize, Serialize};
use std::collections::VecDeque;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::io::AsyncBufReadExt;
use tokio::sync::{oneshot, Mutex};
use tokio_tungstenite::tungstenite::Message as WsMessage;
use tracing::{debug, info, warn};

mod browser_batch;
mod browser_events;
mod browser_keys;
mod browser_launch;
mod browser_observe;
mod browser_profile;
mod browser_visual;

use browser_batch::{parse_browser_batch_op, BrowserBatchOp};
use browser_events::{
    parse_console_event, parse_network_event, BrowserConsoleEvent, BrowserNetworkEvent,
};
use browser_keys::browser_key_spec;
use browser_launch::{apply_chromium_env, cdp_list_url, chromium_launch_args};
use browser_observe::observe_page_js;
use browser_profile::{
    reclaim_captain_profile_lock, user_data_dir_for_agent, user_data_dir_path_for_agent,
};
use browser_visual::{optional_visual_prompt, screenshot_payload};

type WsStream =
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;

// ── Constants ──────────────────────────────────────────────────────────────

const CDP_CONNECT_TIMEOUT_SECS: u64 = 15;
const CDP_COMMAND_TIMEOUT_SECS: u64 = 30;
const PAGE_LOAD_POLL_INTERVAL_MS: u64 = 200;
const PAGE_LOAD_MAX_POLLS: u32 = 150; // 30 seconds
const MAX_NETWORK_EVENTS: usize = 200;
const MAX_CONSOLE_EVENTS: usize = 200;
const MAX_BATCH_STEPS: usize = 20;
const DEFAULT_OBSERVE_ELEMENTS: usize = 60;
const MAX_OBSERVE_ELEMENTS: usize = 120;
#[allow(dead_code)]
const MAX_CONTENT_CHARS: usize = 50_000;

// ── Public types ───────────────────────────────────────────────────────────

/// Browser tool output plus request-only multimodal content for the active LLM.
pub(crate) struct BrowserToolResult {
    pub(crate) content: String,
    pub(crate) transient_content: Vec<ContentBlock>,
}

impl BrowserToolResult {
    pub(crate) fn text(content: String) -> Self {
        Self {
            content,
            transient_content: Vec::new(),
        }
    }
}

/// Command sent to the browser.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "action")]
pub enum BrowserCommand {
    Navigate { url: String },
    Click { selector: String },
    Type { selector: String, text: String },
    Keys { keys: String },
    Select { selector: String, value: String },
    Hover { selector: String },
    Screenshot,
    ReadPage,
    Close,
    Scroll { direction: String, amount: i32 },
    Wait { selector: String, timeout_ms: u64 },
    RunJs { expression: String },
    Back,
    Observe { max_elements: usize },
}

/// Response from a browser command.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BrowserResponse {
    pub success: bool,
    pub data: Option<serde_json::Value>,
    pub error: Option<String>,
}

impl BrowserResponse {
    fn ok(data: serde_json::Value) -> Self {
        Self {
            success: true,
            data: Some(data),
            error: None,
        }
    }
    fn err(msg: impl Into<String>) -> Self {
        Self {
            success: false,
            data: None,
            error: Some(msg.into()),
        }
    }
}

// ── CDP connection ─────────────────────────────────────────────────────────

/// Low-level Chrome DevTools Protocol connection over WebSocket.
struct CdpConnection {
    write: Arc<Mutex<SplitSink<WsStream, WsMessage>>>,
    pending: Arc<DashMap<u64, oneshot::Sender<Result<serde_json::Value, String>>>>,
    network_events: Arc<Mutex<VecDeque<BrowserNetworkEvent>>>,
    console_events: Arc<Mutex<VecDeque<BrowserConsoleEvent>>>,
    next_network_event_id: Arc<AtomicU64>,
    next_console_event_id: Arc<AtomicU64>,
    next_id: AtomicU64,
    _reader_handle: tokio::task::JoinHandle<()>,
}

impl CdpConnection {
    /// Connect to a CDP WebSocket endpoint.
    async fn connect(ws_url: &str) -> Result<Self, String> {
        let (stream, _) = tokio::time::timeout(
            Duration::from_secs(CDP_CONNECT_TIMEOUT_SECS),
            tokio_tungstenite::connect_async(ws_url),
        )
        .await
        .map_err(|_| format!("CDP WebSocket connect timed out: {ws_url}"))?
        .map_err(|e| format!("CDP WebSocket connect failed: {e}"))?;

        let (write, read) = stream.split();
        let write = Arc::new(Mutex::new(write));
        let pending: Arc<DashMap<u64, oneshot::Sender<Result<serde_json::Value, String>>>> =
            Arc::new(DashMap::new());
        let network_events = Arc::new(Mutex::new(VecDeque::with_capacity(MAX_NETWORK_EVENTS)));
        let console_events = Arc::new(Mutex::new(VecDeque::with_capacity(MAX_CONSOLE_EVENTS)));
        let next_network_event_id = Arc::new(AtomicU64::new(1));
        let next_console_event_id = Arc::new(AtomicU64::new(1));

        let reader_pending = Arc::clone(&pending);
        let reader_network_events = Arc::clone(&network_events);
        let reader_console_events = Arc::clone(&console_events);
        let reader_next_network_event_id = Arc::clone(&next_network_event_id);
        let reader_next_console_event_id = Arc::clone(&next_console_event_id);
        let reader_handle = tokio::spawn(Self::reader_loop(
            read,
            reader_pending,
            reader_network_events,
            reader_console_events,
            reader_next_network_event_id,
            reader_next_console_event_id,
        ));

        Ok(Self {
            write,
            pending,
            network_events,
            console_events,
            next_network_event_id,
            next_console_event_id,
            next_id: AtomicU64::new(1),
            _reader_handle: reader_handle,
        })
    }

    /// Background task: read WebSocket messages and route responses.
    async fn reader_loop(
        mut read: SplitStream<WsStream>,
        pending: Arc<DashMap<u64, oneshot::Sender<Result<serde_json::Value, String>>>>,
        network_events: Arc<Mutex<VecDeque<BrowserNetworkEvent>>>,
        console_events: Arc<Mutex<VecDeque<BrowserConsoleEvent>>>,
        next_network_event_id: Arc<AtomicU64>,
        next_console_event_id: Arc<AtomicU64>,
    ) {
        while let Some(msg) = read.next().await {
            let text = match msg {
                Ok(WsMessage::Text(t)) => t.to_string(),
                Ok(WsMessage::Close(_)) => break,
                Err(e) => {
                    debug!("CDP WebSocket read error: {e}");
                    break;
                }
                _ => continue,
            };

            let json: serde_json::Value = match serde_json::from_str(&text) {
                Ok(v) => v,
                Err(_) => continue,
            };

            // Route response to waiting caller by id
            if let Some(id) = json.get("id").and_then(|v| v.as_u64()) {
                if let Some((_, sender)) = pending.remove(&id) {
                    if let Some(error) = json.get("error") {
                        let msg = error["message"].as_str().unwrap_or("CDP error").to_string();
                        let _ = sender.send(Err(msg));
                    } else {
                        let result = json
                            .get("result")
                            .cloned()
                            .unwrap_or(serde_json::Value::Null);
                        let _ = sender.send(Ok(result));
                    }
                }
                continue;
            }

            if let Some(event) =
                parse_network_event(&json, next_network_event_id.fetch_add(1, Ordering::Relaxed))
            {
                let mut events = network_events.lock().await;
                events.push_back(event);
                while events.len() > MAX_NETWORK_EVENTS {
                    events.pop_front();
                }
            }

            if let Some(event) =
                parse_console_event(&json, next_console_event_id.fetch_add(1, Ordering::Relaxed))
            {
                let mut events = console_events.lock().await;
                events.push_back(event);
                while events.len() > MAX_CONSOLE_EVENTS {
                    events.pop_front();
                }
            }
        }
    }

    /// Send a CDP command and wait for the response.
    async fn send(
        &self,
        method: &str,
        params: serde_json::Value,
    ) -> Result<serde_json::Value, String> {
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let (tx, rx) = oneshot::channel();
        self.pending.insert(id, tx);

        let msg = serde_json::json!({ "id": id, "method": method, "params": params });
        self.write
            .lock()
            .await
            .send(WsMessage::Text(msg.to_string()))
            .await
            .map_err(|e| format!("CDP send failed: {e}"))?;

        match tokio::time::timeout(Duration::from_secs(CDP_COMMAND_TIMEOUT_SECS), rx).await {
            Ok(Ok(result)) => result,
            Ok(Err(_)) => Err("CDP response channel closed".to_string()),
            Err(_) => {
                self.pending.remove(&id);
                Err("CDP command timed out".to_string())
            }
        }
    }

    /// Evaluate JavaScript in the browser page and return the value.
    async fn run_js(&self, expression: &str) -> Result<serde_json::Value, String> {
        let result = self
            .send(
                "Runtime.evaluate",
                serde_json::json!({
                    "expression": expression,
                    "returnByValue": true,
                    "awaitPromise": true,
                }),
            )
            .await?;

        // Check for JS exceptions
        if let Some(desc) = result
            .get("exceptionDetails")
            .and_then(|e| e.get("text"))
            .and_then(|t| t.as_str())
        {
            return Err(format!("JS error: {desc}"));
        }

        Ok(result
            .get("result")
            .and_then(|r| r.get("value"))
            .cloned()
            .unwrap_or(serde_json::Value::Null))
    }

    async fn network_events(&self, limit: usize, clear: bool) -> Vec<BrowserNetworkEvent> {
        let limit = limit.clamp(1, MAX_NETWORK_EVENTS);
        let mut events = self.network_events.lock().await;
        let start = events.len().saturating_sub(limit);
        let out = events.iter().skip(start).cloned().collect();
        if clear {
            events.clear();
            self.next_network_event_id.store(1, Ordering::Relaxed);
        }
        out
    }

    async fn console_events(&self, limit: usize, clear: bool) -> Vec<BrowserConsoleEvent> {
        let limit = limit.clamp(1, MAX_CONSOLE_EVENTS);
        let mut events = self.console_events.lock().await;
        let start = events.len().saturating_sub(limit);
        let out = events.iter().skip(start).cloned().collect();
        if clear {
            events.clear();
            self.next_console_event_id.store(1, Ordering::Relaxed);
        }
        out
    }
}

impl Drop for CdpConnection {
    fn drop(&mut self) {
        self._reader_handle.abort();
    }
}

// ── Browser session ────────────────────────────────────────────────────────

/// A live browser session: one Chromium process + one CDP connection per agent.
struct BrowserSession {
    process: tokio::process::Child,
    cdp: CdpConnection,
    #[allow(dead_code)]
    last_active: Instant,
}

impl BrowserSession {
    /// Launch Chromium and establish a CDP connection.
    ///
    /// `agent_id` selects (and creates if missing) a persistent
    /// `--user-data-dir` under `~/.captain/browser-profiles/<agent_id>/`.
    /// Without that argument every agent shared the host's default
    /// Chrome profile — cookies, localStorage and saved logins leaked
    /// across agents and clobbered the user's own browser. B.7 isolates
    /// each agent's session to its own directory so logins stay scoped
    /// and the user's profile is never touched.
    async fn launch(config: &BrowserConfig, agent_id: &str) -> Result<Self, String> {
        let chrome_path = find_chromium(config)?;
        debug!(path = %chrome_path.display(), "Launching Chromium");

        let user_data_dir = user_data_dir_for_agent(agent_id)?;
        reclaim_captain_profile_lock(&user_data_dir).await?;

        let args = chromium_launch_args(config, &user_data_dir, is_running_as_root());

        let mut cmd = tokio::process::Command::new(&chrome_path);
        cmd.args(&args);
        cmd.kill_on_drop(true);
        cmd.stderr(std::process::Stdio::piped());
        cmd.stdout(std::process::Stdio::null());
        cmd.stdin(std::process::Stdio::null());
        apply_chromium_env(&mut cmd);

        let mut child = cmd.spawn().map_err(|e| {
            format!(
                "Failed to launch Chromium at {}: {e}",
                chrome_path.display()
            )
        })?;

        // Parse stderr for the DevTools WebSocket URL
        let stderr = child.stderr.take().ok_or("No stderr from Chromium")?;
        let ws_url = Self::read_devtools_url(stderr).await?;
        debug!(ws_url = %ws_url, "Got CDP WebSocket URL");

        let list_url = cdp_list_url(&ws_url)?;
        let page_ws = Self::find_page_ws(&list_url).await?;
        debug!(page_ws = %page_ws, "Connecting to page");

        let cdp = CdpConnection::connect(&page_ws).await?;

        // Enable required domains
        let _ = cdp.send("Page.enable", serde_json::json!({})).await;
        let _ = cdp.send("Runtime.enable", serde_json::json!({})).await;
        let _ = cdp.send("Log.enable", serde_json::json!({})).await;
        let _ = cdp
            .send(
                "Network.enable",
                serde_json::json!({
                    "maxTotalBufferSize": 1048576,
                    "maxResourceBufferSize": 262144
                }),
            )
            .await;

        Ok(Self {
            process: child,
            cdp,
            last_active: Instant::now(),
        })
    }

    /// Read stderr until we find "DevTools listening on ws://...".
    async fn read_devtools_url(stderr: tokio::process::ChildStderr) -> Result<String, String> {
        let reader = tokio::io::BufReader::new(stderr);
        let mut lines = reader.lines();
        let deadline = tokio::time::Instant::now() + Duration::from_secs(CDP_CONNECT_TIMEOUT_SECS);

        loop {
            let line = tokio::time::timeout_at(deadline, lines.next_line())
                .await
                .map_err(|_| {
                    "Timed out waiting for Chromium to start. Is Chrome/Chromium installed?"
                        .to_string()
                })?
                .map_err(|e| format!("Failed to read Chromium stderr: {e}"))?;

            match line {
                Some(l) if l.contains("DevTools listening on") => {
                    let url = l
                        .split("DevTools listening on ")
                        .nth(1)
                        .ok_or("Malformed DevTools URL line")?
                        .trim()
                        .to_string();
                    return Ok(url);
                }
                Some(_) => continue,
                None => {
                    return Err(
                        "Chromium exited before printing DevTools URL. Is Chrome installed?"
                            .to_string(),
                    );
                }
            }
        }
    }

    /// Fetch /json/list and find the page WebSocket URL.
    async fn find_page_ws(list_url: &str) -> Result<String, String> {
        for attempt in 0..10 {
            if attempt > 0 {
                tokio::time::sleep(Duration::from_millis(300)).await;
            }
            let resp = match reqwest::get(list_url).await {
                Ok(r) => r,
                Err(_) => continue,
            };
            let targets: Vec<serde_json::Value> = match resp.json().await {
                Ok(t) => t,
                Err(_) => continue,
            };
            for target in &targets {
                if target["type"].as_str() == Some("page") {
                    if let Some(ws) = target["webSocketDebuggerUrl"].as_str() {
                        return Ok(ws.to_string());
                    }
                }
            }
        }
        Err("No page target found in Chromium".to_string())
    }

    /// Execute a browser command via CDP.
    async fn execute(&mut self, cmd: BrowserCommand) -> BrowserResponse {
        self.last_active = Instant::now();
        match cmd {
            BrowserCommand::Navigate { url } => self.cmd_navigate(&url).await,
            BrowserCommand::Click { selector } => self.cmd_click(&selector).await,
            BrowserCommand::Type { selector, text } => self.cmd_type(&selector, &text).await,
            BrowserCommand::Keys { keys } => self.cmd_keys(&keys).await,
            BrowserCommand::Select { selector, value } => self.cmd_select(&selector, &value).await,
            BrowserCommand::Hover { selector } => self.cmd_hover(&selector).await,
            BrowserCommand::Screenshot => self.cmd_screenshot().await,
            BrowserCommand::ReadPage => self.cmd_read_page().await,
            BrowserCommand::Close => BrowserResponse::ok(serde_json::json!({"closed": true})),
            BrowserCommand::Scroll { direction, amount } => {
                self.cmd_scroll(&direction, amount).await
            }
            BrowserCommand::Wait {
                selector,
                timeout_ms,
            } => self.cmd_wait(&selector, timeout_ms).await,
            BrowserCommand::RunJs { expression } => self.cmd_run_js(&expression).await,
            BrowserCommand::Back => self.cmd_back().await,
            BrowserCommand::Observe { max_elements } => self.cmd_observe(max_elements).await,
        }
    }

    async fn status(&self) -> serde_json::Value {
        let page = self
            .cdp
            .run_js(
                r#"JSON.stringify({
                    title: document.title || '',
                    url: location.href || '',
                    readyState: document.readyState || '',
                    viewport: { width: window.innerWidth, height: window.innerHeight },
                    scroll: { x: window.scrollX, y: window.scrollY }
                })"#,
            )
            .await
            .ok()
            .and_then(|v| {
                v.as_str()
                    .and_then(|s| serde_json::from_str::<serde_json::Value>(s).ok())
            });

        serde_json::json!({
            "pid": self.process.id(),
            "last_active_secs_ago": self.last_active.elapsed().as_secs(),
            "page": page,
        })
    }

    // ── Command implementations ────────────────────────────────────────

    async fn cmd_navigate(&self, url: &str) -> BrowserResponse {
        let result = self
            .cdp
            .send("Page.navigate", serde_json::json!({ "url": url }))
            .await;

        if let Err(e) = result {
            return BrowserResponse::err(format!("Navigate failed: {e}"));
        }

        // Wait for page load
        self.wait_for_load().await;

        match self.page_info().await {
            Ok(info) => BrowserResponse::ok(info),
            Err(e) => BrowserResponse::err(format!("Navigate succeeded but page info failed: {e}")),
        }
    }

    async fn cmd_click(&self, selector: &str) -> BrowserResponse {
        let sel_json = serde_json::to_string(selector).unwrap_or_default();
        let js = format!(
            r#"(() => {{
    let sel = {sel_json};
    function byCaptainRef(s) {{
        if (typeof s === 'string' && /^@e\d+$/.test(s.trim())) {{
            return document.querySelector('[data-captain-ref="' + s.trim().slice(1).replace(/"/g, '\\"') + '"]');
        }}
        return null;
    }}
    function safeQuery(s) {{
        try {{ return document.querySelector(s); }} catch (_) {{ return null; }}
    }}
    let el = byCaptainRef(sel) || safeQuery(sel);
    if (!el) {{
        const all = document.querySelectorAll('a, button, [role="button"], input[type="submit"], [onclick]');
        const lower = sel.toLowerCase();
        for (const e of all) {{
            if (e.textContent.trim().toLowerCase().includes(lower)) {{ el = e; break; }}
        }}
    }}
    if (!el) return JSON.stringify({{success: false, error: 'Element not found: ' + sel}});
    el.scrollIntoView({{block: 'center'}});
    el.click();
    return JSON.stringify({{success: true, tag: el.tagName, text: el.textContent.substring(0, 100).trim()}});
}})()"#
        );

        match self.cdp.run_js(&js).await {
            Ok(val) => {
                let parsed: serde_json::Value = val
                    .as_str()
                    .and_then(|s| serde_json::from_str(s).ok())
                    .unwrap_or(val);
                if parsed["success"].as_bool() == Some(false) {
                    return BrowserResponse::err(
                        parsed["error"]
                            .as_str()
                            .unwrap_or("Click failed")
                            .to_string(),
                    );
                }
                // Wait briefly for any navigation triggered by click
                tokio::time::sleep(Duration::from_millis(500)).await;
                self.wait_for_load().await;
                match self.page_info().await {
                    Ok(info) => BrowserResponse::ok(info),
                    Err(_) => BrowserResponse::ok(parsed),
                }
            }
            Err(e) => BrowserResponse::err(format!("Click failed: {e}")),
        }
    }

    async fn cmd_type(&self, selector: &str, text: &str) -> BrowserResponse {
        let sel_json = serde_json::to_string(selector).unwrap_or_default();
        let text_json = serde_json::to_string(text).unwrap_or_default();
        let js = format!(
            r#"(() => {{
    let sel = {sel_json};
    let txt = {text_json};
    function byCaptainRef(s) {{
        if (typeof s === 'string' && /^@e\d+$/.test(s.trim())) {{
            return document.querySelector('[data-captain-ref="' + s.trim().slice(1).replace(/"/g, '\\"') + '"]');
        }}
        return null;
    }}
    function safeQuery(s) {{
        try {{ return document.querySelector(s); }} catch (_) {{ return null; }}
    }}
    let el = byCaptainRef(sel) || safeQuery(sel);
    if (!el) return JSON.stringify({{success: false, error: 'Input not found: ' + sel}});
    el.focus();
    el.value = txt;
    el.dispatchEvent(new Event('input', {{bubbles: true}}));
    el.dispatchEvent(new Event('change', {{bubbles: true}}));
    return JSON.stringify({{success: true, selector: sel, typed: txt.length + ' chars'}});
}})()"#
        );

        match self.cdp.run_js(&js).await {
            Ok(val) => {
                let parsed: serde_json::Value = val
                    .as_str()
                    .and_then(|s| serde_json::from_str(s).ok())
                    .unwrap_or(val);
                if parsed["success"].as_bool() == Some(false) {
                    BrowserResponse::err(parsed["error"].as_str().unwrap_or("Type failed"))
                } else {
                    BrowserResponse::ok(parsed)
                }
            }
            Err(e) => BrowserResponse::err(format!("Type failed: {e}")),
        }
    }

    async fn cmd_keys(&self, keys: &str) -> BrowserResponse {
        let keys = keys.trim();
        if keys.is_empty() {
            return BrowserResponse::err("Keys failed: 'keys' cannot be empty");
        }

        let spec = browser_key_spec(keys);
        if let Some(text) = spec.insert_text {
            match self
                .cdp
                .send("Input.insertText", serde_json::json!({ "text": text }))
                .await
            {
                Ok(_) => {
                    return BrowserResponse::ok(serde_json::json!({
                        "keys": keys,
                        "mode": "insert_text",
                    }))
                }
                Err(e) => return BrowserResponse::err(format!("Keys failed: {e}")),
            }
        }

        let mut down = serde_json::json!({
            "type": "keyDown",
            "key": spec.key.clone(),
            "modifiers": spec.modifiers,
        });
        let mut up = serde_json::json!({
            "type": "keyUp",
            "key": spec.key.clone(),
            "modifiers": spec.modifiers,
        });
        if let Some(code) = spec.code {
            down["code"] = serde_json::json!(code.clone());
            up["code"] = serde_json::json!(code);
        }
        if let Some(key_code) = spec.key_code {
            down["windowsVirtualKeyCode"] = serde_json::json!(key_code);
            down["nativeVirtualKeyCode"] = serde_json::json!(key_code);
            up["windowsVirtualKeyCode"] = serde_json::json!(key_code);
            up["nativeVirtualKeyCode"] = serde_json::json!(key_code);
        }
        if let Some(text) = spec.text {
            down["text"] = serde_json::json!(text.clone());
            down["unmodifiedText"] = serde_json::json!(text);
        }

        if let Err(e) = self.cdp.send("Input.dispatchKeyEvent", down).await {
            return BrowserResponse::err(format!("Keys failed: {e}"));
        }
        if let Err(e) = self.cdp.send("Input.dispatchKeyEvent", up).await {
            return BrowserResponse::err(format!("Keys failed: {e}"));
        }

        tokio::time::sleep(Duration::from_millis(100)).await;
        BrowserResponse::ok(serde_json::json!({
            "keys": keys,
            "mode": "key_event",
        }))
    }

    async fn cmd_select(&self, selector: &str, value: &str) -> BrowserResponse {
        let sel_json = serde_json::to_string(selector).unwrap_or_default();
        let value_json = serde_json::to_string(value).unwrap_or_default();
        let js = format!(
            r#"(() => {{
    let sel = {sel_json};
    let wanted = {value_json};
    function byCaptainRef(s) {{
        if (typeof s === 'string' && /^@e\d+$/.test(s.trim())) {{
            return document.querySelector('[data-captain-ref="' + s.trim().slice(1).replace(/"/g, '\\"') + '"]');
        }}
        return null;
    }}
    function safeQuery(s) {{
        try {{ return document.querySelector(s); }} catch (_) {{ return null; }}
    }}
    let el = byCaptainRef(sel) || safeQuery(sel);
    if (!el) return JSON.stringify({{success: false, error: 'Select not found: ' + sel}});
    if (el.tagName !== 'SELECT') {{
        return JSON.stringify({{success: false, error: 'Element is not a <select>: ' + sel}});
    }}
    let chosen = null;
    for (const opt of Array.from(el.options || [])) {{
        const text = (opt.textContent || '').trim();
        if (opt.value === wanted || text === wanted || opt.label === wanted) {{
            chosen = opt;
            break;
        }}
    }}
    if (!chosen) {{
        return JSON.stringify({{success: false, error: 'Option not found in ' + sel + ': ' + wanted}});
    }}
    el.focus();
    el.value = chosen.value;
    chosen.selected = true;
    el.dispatchEvent(new Event('input', {{bubbles: true}}));
    el.dispatchEvent(new Event('change', {{bubbles: true}}));
    return JSON.stringify({{
        success: true,
        selector: sel,
        value: el.value,
        text: (chosen.textContent || '').trim()
    }});
}})()"#
        );

        match self.cdp.run_js(&js).await {
            Ok(val) => {
                let parsed: serde_json::Value = val
                    .as_str()
                    .and_then(|s| serde_json::from_str(s).ok())
                    .unwrap_or(val);
                if parsed["success"].as_bool() == Some(false) {
                    BrowserResponse::err(parsed["error"].as_str().unwrap_or("Select failed"))
                } else {
                    BrowserResponse::ok(parsed)
                }
            }
            Err(e) => BrowserResponse::err(format!("Select failed: {e}")),
        }
    }

    async fn cmd_hover(&self, selector: &str) -> BrowserResponse {
        let sel_json = serde_json::to_string(selector).unwrap_or_default();
        let js = format!(
            r#"(() => {{
    let sel = {sel_json};
    function byCaptainRef(s) {{
        if (typeof s === 'string' && /^@e\d+$/.test(s.trim())) {{
            return document.querySelector('[data-captain-ref="' + s.trim().slice(1).replace(/"/g, '\\"') + '"]');
        }}
        return null;
    }}
    function safeQuery(s) {{
        try {{ return document.querySelector(s); }} catch (_) {{ return null; }}
    }}
    let el = byCaptainRef(sel) || safeQuery(sel);
    if (!el) {{
        const all = document.querySelectorAll('a, button, [role="button"], [role="menuitem"], input, textarea, select, [title], [aria-label]');
        const lower = sel.toLowerCase();
        for (const e of all) {{
            const label = ((e.textContent || '') + ' ' + (e.getAttribute('aria-label') || '') + ' ' + (e.getAttribute('title') || '')).trim().toLowerCase();
            if (label.includes(lower)) {{ el = e; break; }}
        }}
    }}
    if (!el) return JSON.stringify({{success: false, error: 'Element not found: ' + sel}});
    el.scrollIntoView({{block: 'center', inline: 'center'}});
    const rect = el.getBoundingClientRect();
    if (!rect || rect.width <= 0 || rect.height <= 0) {{
        return JSON.stringify({{success: false, error: 'Element has no visible box: ' + sel}});
    }}
    return JSON.stringify({{
        success: true,
        x: rect.left + rect.width / 2,
        y: rect.top + rect.height / 2,
        tag: el.tagName,
        text: (el.textContent || el.getAttribute('aria-label') || el.getAttribute('title') || '').trim().substring(0, 100)
    }});
}})()"#
        );

        let parsed = match self.cdp.run_js(&js).await {
            Ok(val) => val
                .as_str()
                .and_then(|s| serde_json::from_str::<serde_json::Value>(s).ok())
                .unwrap_or(val),
            Err(e) => return BrowserResponse::err(format!("Hover failed: {e}")),
        };
        if parsed["success"].as_bool() == Some(false) {
            return BrowserResponse::err(parsed["error"].as_str().unwrap_or("Hover failed"));
        }

        let x = parsed["x"].as_f64().unwrap_or(0.0);
        let y = parsed["y"].as_f64().unwrap_or(0.0);
        match self
            .cdp
            .send(
                "Input.dispatchMouseEvent",
                serde_json::json!({
                    "type": "mouseMoved",
                    "x": x,
                    "y": y,
                    "button": "none",
                }),
            )
            .await
        {
            Ok(_) => BrowserResponse::ok(parsed),
            Err(e) => BrowserResponse::err(format!("Hover failed: {e}")),
        }
    }

    async fn cmd_screenshot(&self) -> BrowserResponse {
        match self
            .cdp
            .send(
                "Page.captureScreenshot",
                serde_json::json!({ "format": "png" }),
            )
            .await
        {
            Ok(result) => {
                let b64 = result["data"].as_str().unwrap_or("");
                let url = self
                    .cdp
                    .run_js("location.href")
                    .await
                    .ok()
                    .and_then(|v| v.as_str().map(String::from))
                    .unwrap_or_default();
                BrowserResponse::ok(
                    serde_json::json!({"image_base64": b64, "url": url, "format": "png"}),
                )
            }
            Err(e) => BrowserResponse::err(format!("Screenshot failed: {e}")),
        }
    }

    async fn cmd_read_page(&self) -> BrowserResponse {
        match self.cdp.run_js(EXTRACT_CONTENT_JS).await {
            Ok(val) => {
                let parsed: serde_json::Value = val
                    .as_str()
                    .and_then(|s| serde_json::from_str(s).ok())
                    .unwrap_or(val);
                BrowserResponse::ok(parsed)
            }
            Err(e) => BrowserResponse::err(format!("ReadPage failed: {e}")),
        }
    }

    async fn cmd_scroll(&self, direction: &str, amount: i32) -> BrowserResponse {
        let (dx, dy) = match direction {
            "up" => (0, -amount),
            "down" => (0, amount),
            "left" => (-amount, 0),
            "right" => (amount, 0),
            _ => (0, amount),
        };
        let js = format!("window.scrollBy({dx}, {dy}); JSON.stringify({{scrollX: window.scrollX, scrollY: window.scrollY}})");
        match self.cdp.run_js(&js).await {
            Ok(val) => {
                let parsed: serde_json::Value = val
                    .as_str()
                    .and_then(|s| serde_json::from_str(s).ok())
                    .unwrap_or(val);
                BrowserResponse::ok(parsed)
            }
            Err(e) => BrowserResponse::err(format!("Scroll failed: {e}")),
        }
    }

    async fn cmd_wait(&self, selector: &str, timeout_ms: u64) -> BrowserResponse {
        let sel_json = serde_json::to_string(selector).unwrap_or_default();
        let max_ms = timeout_ms.min(30_000);
        let polls = (max_ms / PAGE_LOAD_POLL_INTERVAL_MS).max(1);

        for _ in 0..polls {
            let js = format!(
                r#"(() => {{
    const sel = {sel_json};
    if (typeof sel === 'string' && /^@e\d+$/.test(sel.trim())) {{
        return document.querySelector('[data-captain-ref="' + sel.trim().slice(1).replace(/"/g, '\\"') + '"]') ? 'found' : null;
    }}
    try {{
        return document.querySelector(sel) ? 'found' : null;
    }} catch (_) {{
        return null;
    }}
}})()"#
            );
            if let Ok(val) = self.cdp.run_js(&js).await {
                if val.as_str() == Some("found") {
                    return BrowserResponse::ok(
                        serde_json::json!({"found": true, "selector": selector}),
                    );
                }
            }
            tokio::time::sleep(Duration::from_millis(PAGE_LOAD_POLL_INTERVAL_MS)).await;
        }

        BrowserResponse::err(format!(
            "Timed out waiting for selector: {selector} ({max_ms}ms)"
        ))
    }

    async fn cmd_run_js(&self, expression: &str) -> BrowserResponse {
        match self.cdp.run_js(expression).await {
            Ok(val) => BrowserResponse::ok(serde_json::json!({"result": val})),
            Err(e) => BrowserResponse::err(format!("JS execution failed: {e}")),
        }
    }

    async fn cmd_back(&self) -> BrowserResponse {
        match self.cdp.run_js("history.back(); 'ok'").await {
            Ok(_) => {
                tokio::time::sleep(Duration::from_millis(500)).await;
                self.wait_for_load().await;
                match self.page_info().await {
                    Ok(info) => BrowserResponse::ok(info),
                    Err(e) => {
                        BrowserResponse::err(format!("Back succeeded but page info failed: {e}"))
                    }
                }
            }
            Err(e) => BrowserResponse::err(format!("Back failed: {e}")),
        }
    }

    async fn cmd_observe(&self, max_elements: usize) -> BrowserResponse {
        let max_elements = max_elements.clamp(1, MAX_OBSERVE_ELEMENTS);
        match self.cdp.run_js(&observe_page_js(max_elements)).await {
            Ok(val) => {
                let parsed: serde_json::Value = val
                    .as_str()
                    .and_then(|s| serde_json::from_str(s).ok())
                    .unwrap_or(val);
                BrowserResponse::ok(parsed)
            }
            Err(e) => BrowserResponse::err(format!("Observe failed: {e}")),
        }
    }

    // ── Helpers ────────────────────────────────────────────────────────

    /// Poll until document.readyState is 'complete' or 'interactive'.
    async fn wait_for_load(&self) {
        for _ in 0..PAGE_LOAD_MAX_POLLS {
            if let Ok(val) = self.cdp.run_js("document.readyState").await {
                let state = val.as_str().unwrap_or("");
                if state == "complete" || state == "interactive" {
                    return;
                }
            }
            tokio::time::sleep(Duration::from_millis(PAGE_LOAD_POLL_INTERVAL_MS)).await;
        }
    }

    /// Get current page title, URL, and readable content.
    async fn page_info(&self) -> Result<serde_json::Value, String> {
        let info = self
            .cdp
            .run_js("JSON.stringify({title: document.title, url: location.href})")
            .await?;
        let parsed: serde_json::Value = info
            .as_str()
            .and_then(|s| serde_json::from_str(s).ok())
            .unwrap_or(info);

        let content_val = self
            .cdp
            .run_js(EXTRACT_CONTENT_JS)
            .await
            .unwrap_or_default();
        let content_obj: serde_json::Value = content_val
            .as_str()
            .and_then(|s| serde_json::from_str(s).ok())
            .unwrap_or(content_val);
        let content_text = content_obj["content"].as_str().unwrap_or("");

        Ok(serde_json::json!({
            "title": parsed["title"],
            "url": parsed["url"],
            "content": content_text,
        }))
    }
}

impl Drop for BrowserSession {
    fn drop(&mut self) {
        let _ = self.process.start_kill();
    }
}

// ── Chromium discovery ─────────────────────────────────────────────────────

/// Find a Chromium-based browser binary on this system.
fn find_chromium(config: &BrowserConfig) -> Result<PathBuf, String> {
    // 1. User-configured path
    if let Some(ref path) = config.chromium_path {
        if !path.is_empty() {
            let p = PathBuf::from(path);
            if p.exists() {
                return Ok(p);
            }
            return Err(format!("Configured chromium_path not found: {path}"));
        }
    }

    // 2. CHROME_PATH env var
    if let Ok(path) = std::env::var("CHROME_PATH") {
        let p = PathBuf::from(&path);
        if p.exists() {
            return Ok(p);
        }
    }

    // 3. Platform-specific search
    let candidates = chromium_candidates();
    for candidate in &candidates {
        let p = PathBuf::from(candidate);
        if p.exists() {
            return Ok(p);
        }
    }

    // 4. Try PATH lookup
    for name in &[
        "google-chrome",
        "google-chrome-stable",
        "chromium",
        "chromium-browser",
        "chrome",
    ] {
        if let Ok(output) = std::process::Command::new("which").arg(name).output() {
            if output.status.success() {
                let path = String::from_utf8_lossy(&output.stdout).trim().to_string();
                if !path.is_empty() {
                    return Ok(PathBuf::from(path));
                }
            }
        }
        // Windows: use where.exe
        #[cfg(windows)]
        if let Ok(output) = std::process::Command::new("where.exe").arg(name).output() {
            if output.status.success() {
                let path = String::from_utf8_lossy(&output.stdout)
                    .lines()
                    .next()
                    .unwrap_or("")
                    .trim()
                    .to_string();
                if !path.is_empty() {
                    return Ok(PathBuf::from(path));
                }
            }
        }
    }

    Err(
        "Chromium/Chrome not found. Install Chrome or set CHROME_PATH. \
         Checked: Chrome, Chromium, Edge, Brave in standard locations."
            .to_string(),
    )
}

/// Platform-specific candidate paths for Chromium-based browsers.
fn chromium_candidates() -> Vec<String> {
    let mut paths = Vec::new();

    #[cfg(windows)]
    {
        let program_files = std::env::var("ProgramFiles").unwrap_or_default();
        let program_files_x86 = std::env::var("ProgramFiles(x86)").unwrap_or_default();
        let local_app = std::env::var("LOCALAPPDATA").unwrap_or_default();

        for pf in &[&program_files, &program_files_x86] {
            if pf.is_empty() {
                continue;
            }
            paths.push(format!("{pf}\\Google\\Chrome\\Application\\chrome.exe"));
            paths.push(format!("{pf}\\Microsoft\\Edge\\Application\\msedge.exe"));
            paths.push(format!(
                "{pf}\\BraveSoftware\\Brave-Browser\\Application\\brave.exe"
            ));
        }
        if !local_app.is_empty() {
            paths.push(format!(
                "{local_app}\\Google\\Chrome\\Application\\chrome.exe"
            ));
            paths.push(format!(
                "{local_app}\\Microsoft\\Edge\\Application\\msedge.exe"
            ));
        }
    }

    #[cfg(target_os = "macos")]
    {
        paths.push("/Applications/Google Chrome.app/Contents/MacOS/Google Chrome".into());
        paths.push("/Applications/Chromium.app/Contents/MacOS/Chromium".into());
        paths.push("/Applications/Microsoft Edge.app/Contents/MacOS/Microsoft Edge".into());
        paths.push("/Applications/Brave Browser.app/Contents/MacOS/Brave Browser".into());
    }

    #[cfg(target_os = "linux")]
    {
        paths.push("/usr/bin/google-chrome".into());
        paths.push("/usr/bin/google-chrome-stable".into());
        paths.push("/usr/bin/chromium".into());
        paths.push("/usr/bin/chromium-browser".into());
        paths.push("/snap/bin/chromium".into());
        paths.push("/usr/bin/microsoft-edge".into());
        paths.push("/usr/bin/brave-browser".into());
    }

    paths
}

// ── Browser manager ────────────────────────────────────────────────────────

/// Manages browser sessions for all agents.
pub struct BrowserManager {
    sessions: DashMap<String, Arc<Mutex<BrowserSession>>>,
    config: BrowserConfig,
}

impl BrowserManager {
    /// Create a new BrowserManager with the given configuration.
    pub fn new(config: BrowserConfig) -> Self {
        Self {
            sessions: DashMap::new(),
            config,
        }
    }

    /// Check whether an agent has an active browser session.
    pub fn has_session(&self, agent_id: &str) -> bool {
        self.sessions.contains_key(agent_id)
    }

    /// Inspect browser configuration and the current agent session without creating one.
    pub async fn status(&self, agent_id: &str) -> serde_json::Value {
        let session = self
            .sessions
            .get(agent_id)
            .map(|entry| Arc::clone(entry.value()));
        let session_status = match session {
            Some(session) => Some(session.lock().await.status().await),
            None => None,
        };
        let profile_dir = user_data_dir_path_for_agent(agent_id)
            .map(|p| p.display().to_string())
            .unwrap_or_default();
        let chromium = match find_chromium(&self.config) {
            Ok(path) => serde_json::json!({"available": true, "path": path.display().to_string()}),
            Err(error) => serde_json::json!({"available": false, "error": error}),
        };

        serde_json::json!({
            "agent_id": agent_id,
            "active": session_status.is_some(),
            "active_sessions": self.sessions.len(),
            "max_sessions": self.config.max_sessions,
            "profile_dir": profile_dir,
            "headless": self.config.headless,
            "viewport": {
                "width": self.config.viewport_width,
                "height": self.config.viewport_height,
            },
            "idle_timeout_secs": self.config.idle_timeout_secs,
            "chromium": chromium,
            "session": session_status,
        })
    }

    /// Return recent network events for the active agent session without creating one.
    pub async fn network_log(
        &self,
        agent_id: &str,
        limit: usize,
        clear: bool,
    ) -> serde_json::Value {
        let session = self
            .sessions
            .get(agent_id)
            .map(|entry| Arc::clone(entry.value()));
        let Some(session) = session else {
            return serde_json::json!({
                "active": false,
                "events": [],
                "message": "No active browser session. Call browser_navigate first.",
            });
        };
        let events = session.lock().await.cdp.network_events(limit, clear).await;
        serde_json::json!({
            "active": true,
            "limit": limit.clamp(1, MAX_NETWORK_EVENTS),
            "cleared": clear,
            "events": events,
        })
    }

    /// Return recent console / page error events for the active session.
    pub async fn console_log(
        &self,
        agent_id: &str,
        limit: usize,
        clear: bool,
    ) -> serde_json::Value {
        let session = self
            .sessions
            .get(agent_id)
            .map(|entry| Arc::clone(entry.value()));
        let Some(session) = session else {
            return serde_json::json!({
                "active": false,
                "events": [],
                "message": "No active browser session. Call browser_navigate first.",
            });
        };
        let events = session.lock().await.cdp.console_events(limit, clear).await;
        serde_json::json!({
            "active": true,
            "limit": limit.clamp(1, MAX_CONSOLE_EVENTS),
            "cleared": clear,
            "events": events,
        })
    }

    /// Observe the active page with a compact element list.
    pub async fn observe(&self, agent_id: &str, max_elements: usize) -> serde_json::Value {
        let session = self
            .sessions
            .get(agent_id)
            .map(|entry| Arc::clone(entry.value()));
        let Some(session) = session else {
            return serde_json::json!({
                "active": false,
                "message": "No active browser session. Call browser_navigate first.",
                "elements": [],
            });
        };
        let resp = session
            .lock()
            .await
            .execute(BrowserCommand::Observe { max_elements })
            .await;
        if resp.success {
            let mut data = resp.data.unwrap_or_default();
            if let Some(obj) = data.as_object_mut() {
                obj.insert("active".to_string(), serde_json::Value::Bool(true));
            }
            data
        } else {
            serde_json::json!({
                "active": true,
                "success": false,
                "error": resp.error.unwrap_or_else(|| "Observe failed".to_string()),
            })
        }
    }

    /// Combined status + observation + recent network/console diagnostics.
    pub async fn diagnostics(
        &self,
        agent_id: &str,
        limit: usize,
        clear: bool,
        max_elements: usize,
    ) -> serde_json::Value {
        serde_json::json!({
            "status": self.status(agent_id).await,
            "observation": self.observe(agent_id, max_elements).await,
            "network": self.network_log(agent_id, limit, clear).await,
            "console": self.console_log(agent_id, limit, clear).await,
        })
    }

    /// Send a command to an agent's browser session (creating one if needed).
    pub async fn send_command(
        &self,
        agent_id: &str,
        cmd: BrowserCommand,
    ) -> Result<BrowserResponse, String> {
        let session = self.get_or_create(agent_id).await?;
        let mut guard = session.lock().await;
        let resp = guard.execute(cmd).await;

        if !resp.success {
            if let Some(ref err) = resp.error {
                warn!(agent_id, error = %err, "Browser command failed");
            }
        }

        Ok(resp)
    }

    /// Close an agent's browser session.
    pub async fn close_session(&self, agent_id: &str) {
        if let Some((_, session)) = self.sessions.remove(agent_id) {
            drop(session);
            info!(agent_id, "Browser session closed");
        }
    }

    /// Clean up an agent's browser session (called after agent loop ends).
    pub async fn cleanup_agent(&self, agent_id: &str) {
        self.close_session(agent_id).await;
    }

    /// Get existing session or create a new one.
    async fn get_or_create(&self, agent_id: &str) -> Result<Arc<Mutex<BrowserSession>>, String> {
        if let Some(entry) = self.sessions.get(agent_id) {
            return Ok(Arc::clone(entry.value()));
        }

        if self.sessions.len() >= self.config.max_sessions {
            return Err(format!(
                "Maximum browser sessions reached ({}). Close an existing session first.",
                self.config.max_sessions
            ));
        }

        let session = BrowserSession::launch(&self.config, agent_id).await?;
        let arc = Arc::new(Mutex::new(session));
        self.sessions.insert(agent_id.to_string(), Arc::clone(&arc));
        info!(agent_id, "Browser session created (native CDP)");
        Ok(arc)
    }
}

// B.7 — Resolve the per-agent `--user-data-dir`.
//
// The directory lives under `~/.captain/browser-profiles/<sanitized-agent-id>/`
// so cookies/sessions persist across daemon restarts but never cross
// the boundary between two agents (or between Captain and the user's
// own Chrome profile, which would otherwise be clobbered by sharing the
// default profile path).
//
// ── Tool handler functions ─────────────────────────────────────────────────

/// browser_navigate: Navigate to a URL. SSRF-checked before sending.
pub async fn tool_browser_navigate(
    input: &serde_json::Value,
    mgr: &BrowserManager,
    agent_id: &str,
) -> Result<String, String> {
    let url = input["url"].as_str().ok_or("Missing 'url' parameter")?;
    crate::web_fetch::check_ssrf(url)?;

    let resp = mgr
        .send_command(
            agent_id,
            BrowserCommand::Navigate {
                url: url.to_string(),
            },
        )
        .await?;
    if !resp.success {
        return Err(resp.error.unwrap_or_else(|| "Navigate failed".to_string()));
    }

    let data = resp.data.unwrap_or_default();
    let title = data["title"].as_str().unwrap_or("(no title)");
    let page_url = data["url"].as_str().unwrap_or(url);
    let content = data["content"].as_str().unwrap_or("");
    let wrapped = crate::web_content::wrap_external_content(page_url, content);

    Ok(format!(
        "Navigated to: {page_url}\nTitle: {title}\n\n{wrapped}"
    ))
}

/// browser_click: Click an element by CSS selector or visible text.
pub async fn tool_browser_click(
    input: &serde_json::Value,
    mgr: &BrowserManager,
    agent_id: &str,
) -> Result<String, String> {
    let selector = input["selector"]
        .as_str()
        .ok_or("Missing 'selector' parameter")?;

    let resp = mgr
        .send_command(
            agent_id,
            BrowserCommand::Click {
                selector: selector.to_string(),
            },
        )
        .await?;
    if !resp.success {
        return Err(resp.error.unwrap_or_else(|| "Click failed".to_string()));
    }

    let data = resp.data.unwrap_or_default();
    let title = data["title"].as_str().unwrap_or("(no title)");
    let url = data["url"].as_str().unwrap_or("");
    Ok(format!("Clicked: {selector}\nPage: {title}\nURL: {url}"))
}

/// browser_type: Type text into an input field.
pub async fn tool_browser_type(
    input: &serde_json::Value,
    mgr: &BrowserManager,
    agent_id: &str,
) -> Result<String, String> {
    let selector = input["selector"]
        .as_str()
        .ok_or("Missing 'selector' parameter")?;
    let text = input["text"].as_str().ok_or("Missing 'text' parameter")?;

    let resp = mgr
        .send_command(
            agent_id,
            BrowserCommand::Type {
                selector: selector.to_string(),
                text: text.to_string(),
            },
        )
        .await?;
    if !resp.success {
        return Err(resp.error.unwrap_or_else(|| "Type failed".to_string()));
    }
    Ok(format!("Typed into {selector}: {text}"))
}

/// browser_keys: Send keyboard input to the focused element/page.
pub async fn tool_browser_keys(
    input: &serde_json::Value,
    mgr: &BrowserManager,
    agent_id: &str,
) -> Result<String, String> {
    let keys = input["keys"].as_str().ok_or("Missing 'keys' parameter")?;

    let resp = mgr
        .send_command(
            agent_id,
            BrowserCommand::Keys {
                keys: keys.to_string(),
            },
        )
        .await?;
    if !resp.success {
        return Err(resp.error.unwrap_or_else(|| "Keys failed".to_string()));
    }
    Ok(format!("Sent keys: {keys}"))
}

/// browser_select: Select an option in a native <select> field.
pub async fn tool_browser_select(
    input: &serde_json::Value,
    mgr: &BrowserManager,
    agent_id: &str,
) -> Result<String, String> {
    let selector = input["selector"]
        .as_str()
        .ok_or("Missing 'selector' parameter")?;
    let value = input["value"].as_str().ok_or("Missing 'value' parameter")?;

    let resp = mgr
        .send_command(
            agent_id,
            BrowserCommand::Select {
                selector: selector.to_string(),
                value: value.to_string(),
            },
        )
        .await?;
    if !resp.success {
        return Err(resp.error.unwrap_or_else(|| "Select failed".to_string()));
    }
    let data = resp.data.unwrap_or_default();
    Ok(format!(
        "Selected in {selector}: value={} text={}",
        data["value"].as_str().unwrap_or(value),
        data["text"].as_str().unwrap_or("")
    ))
}

/// browser_hover: Move the pointer over an element to reveal hover UI.
pub async fn tool_browser_hover(
    input: &serde_json::Value,
    mgr: &BrowserManager,
    agent_id: &str,
) -> Result<String, String> {
    let selector = input["selector"]
        .as_str()
        .ok_or("Missing 'selector' parameter")?;

    let resp = mgr
        .send_command(
            agent_id,
            BrowserCommand::Hover {
                selector: selector.to_string(),
            },
        )
        .await?;
    if !resp.success {
        return Err(resp.error.unwrap_or_else(|| "Hover failed".to_string()));
    }
    let data = resp.data.unwrap_or_default();
    Ok(format!(
        "Hovered: {selector} ({})",
        data["text"].as_str().unwrap_or("")
    ))
}

/// browser_screenshot: Take a screenshot of the current page.
pub(crate) async fn tool_browser_screenshot(
    input: &serde_json::Value,
    mgr: &BrowserManager,
    agent_id: &str,
) -> Result<BrowserToolResult, String> {
    let prompt = optional_visual_prompt(input)?;
    let resp = mgr
        .send_command(agent_id, BrowserCommand::Screenshot)
        .await?;
    if !resp.success {
        return Err(resp
            .error
            .unwrap_or_else(|| "Screenshot failed".to_string()));
    }

    let data = resp.data.unwrap_or_default();
    let payload = screenshot_payload(&data, prompt.as_deref())?;
    let content = serde_json::to_string_pretty(&payload.metadata)
        .map_err(|error| format!("Screenshot result serialization failed: {error}"))?;
    Ok(BrowserToolResult {
        content,
        transient_content: payload.transient_content,
    })
}

/// browser_read_page: Read current page content as markdown.
pub async fn tool_browser_read_page(
    _input: &serde_json::Value,
    mgr: &BrowserManager,
    agent_id: &str,
) -> Result<String, String> {
    let resp = mgr.send_command(agent_id, BrowserCommand::ReadPage).await?;
    if !resp.success {
        return Err(resp.error.unwrap_or_else(|| "ReadPage failed".to_string()));
    }

    let data = resp.data.unwrap_or_default();
    let title = data["title"].as_str().unwrap_or("(no title)");
    let url = data["url"].as_str().unwrap_or("");
    let content = data["content"].as_str().unwrap_or("");
    let wrapped = crate::web_content::wrap_external_content(url, content);

    Ok(format!("Page: {title}\nURL: {url}\n\n{wrapped}"))
}

/// browser_close: Close the browser session.
pub async fn tool_browser_close(
    _input: &serde_json::Value,
    mgr: &BrowserManager,
    agent_id: &str,
) -> Result<String, String> {
    mgr.close_session(agent_id).await;
    Ok("Browser session closed.".to_string())
}

/// browser_scroll: Scroll the page in a direction.
pub async fn tool_browser_scroll(
    input: &serde_json::Value,
    mgr: &BrowserManager,
    agent_id: &str,
) -> Result<String, String> {
    let direction = input["direction"].as_str().unwrap_or("down").to_string();
    let amount = input["amount"].as_i64().unwrap_or(600) as i32;

    let resp = mgr
        .send_command(agent_id, BrowserCommand::Scroll { direction, amount })
        .await?;
    if !resp.success {
        return Err(resp.error.unwrap_or_else(|| "Scroll failed".to_string()));
    }
    let data = resp.data.unwrap_or_default();
    Ok(format!(
        "Scrolled. Position: scrollX={}, scrollY={}",
        data["scrollX"], data["scrollY"]
    ))
}

/// browser_wait: Wait for a CSS selector to appear on the page.
pub async fn tool_browser_wait(
    input: &serde_json::Value,
    mgr: &BrowserManager,
    agent_id: &str,
) -> Result<String, String> {
    let selector = input["selector"]
        .as_str()
        .ok_or("Missing 'selector' parameter")?;
    let timeout_ms = input["timeout_ms"].as_u64().unwrap_or(5000);

    let resp = mgr
        .send_command(
            agent_id,
            BrowserCommand::Wait {
                selector: selector.to_string(),
                timeout_ms,
            },
        )
        .await?;
    if !resp.success {
        return Err(resp.error.unwrap_or_else(|| "Wait timed out".to_string()));
    }
    Ok(format!("Element found: {selector}"))
}

/// browser_run_js: Run JavaScript on the current page.
pub async fn tool_browser_run_js(
    input: &serde_json::Value,
    mgr: &BrowserManager,
    agent_id: &str,
) -> Result<String, String> {
    let expression = input["expression"]
        .as_str()
        .ok_or("Missing 'expression' parameter")?;

    let resp = mgr
        .send_command(
            agent_id,
            BrowserCommand::RunJs {
                expression: expression.to_string(),
            },
        )
        .await?;
    if !resp.success {
        return Err(resp
            .error
            .unwrap_or_else(|| "JS execution failed".to_string()));
    }
    let data = resp.data.unwrap_or_default();
    Ok(serde_json::to_string_pretty(&data["result"]).unwrap_or_else(|_| "null".to_string()))
}

/// browser_back: Go back in browser history.
pub async fn tool_browser_back(
    _input: &serde_json::Value,
    mgr: &BrowserManager,
    agent_id: &str,
) -> Result<String, String> {
    let resp = mgr.send_command(agent_id, BrowserCommand::Back).await?;
    if !resp.success {
        return Err(resp.error.unwrap_or_else(|| "Back failed".to_string()));
    }
    let data = resp.data.unwrap_or_default();
    let title = data["title"].as_str().unwrap_or("(no title)");
    let url = data["url"].as_str().unwrap_or("");
    Ok(format!("Went back.\nPage: {title}\nURL: {url}"))
}

/// browser_status: inspect browser configuration and the current agent session.
pub async fn tool_browser_status(
    _input: &serde_json::Value,
    mgr: &BrowserManager,
    agent_id: &str,
) -> Result<String, String> {
    serde_json::to_string_pretty(&mgr.status(agent_id).await)
        .map_err(|e| format!("Browser status serialization failed: {e}"))
}

/// browser_network_log: return recent CDP network events for the active page.
pub async fn tool_browser_network_log(
    input: &serde_json::Value,
    mgr: &BrowserManager,
    agent_id: &str,
) -> Result<String, String> {
    let limit = input["limit"]
        .as_u64()
        .unwrap_or(50)
        .clamp(1, MAX_NETWORK_EVENTS as u64) as usize;
    let clear = input["clear"].as_bool().unwrap_or(false);
    serde_json::to_string_pretty(&mgr.network_log(agent_id, limit, clear).await)
        .map_err(|e| format!("Browser network log serialization failed: {e}"))
}

/// browser_observe: compact page observation with stable refs for click/type.
pub async fn tool_browser_observe(
    input: &serde_json::Value,
    mgr: &BrowserManager,
    agent_id: &str,
) -> Result<String, String> {
    let max_elements = input["max_elements"]
        .as_u64()
        .unwrap_or(DEFAULT_OBSERVE_ELEMENTS as u64)
        .clamp(1, MAX_OBSERVE_ELEMENTS as u64) as usize;
    serde_json::to_string_pretty(&mgr.observe(agent_id, max_elements).await)
        .map_err(|e| format!("Browser observation serialization failed: {e}"))
}

/// browser_diagnostics: status + observation + network + console in one call.
pub async fn tool_browser_diagnostics(
    input: &serde_json::Value,
    mgr: &BrowserManager,
    agent_id: &str,
) -> Result<String, String> {
    let limit = input["limit"]
        .as_u64()
        .unwrap_or(50)
        .clamp(1, MAX_NETWORK_EVENTS as u64) as usize;
    let clear = input["clear"].as_bool().unwrap_or(false);
    let max_elements = input["max_elements"]
        .as_u64()
        .unwrap_or(DEFAULT_OBSERVE_ELEMENTS as u64)
        .clamp(1, MAX_OBSERVE_ELEMENTS as u64) as usize;
    serde_json::to_string_pretty(&mgr.diagnostics(agent_id, limit, clear, max_elements).await)
        .map_err(|e| format!("Browser diagnostics serialization failed: {e}"))
}

/// browser_batch: execute a bounded multi-step browser scenario atomically.
pub(crate) async fn tool_browser_batch(
    input: &serde_json::Value,
    mgr: &BrowserManager,
    agent_id: &str,
) -> Result<BrowserToolResult, String> {
    let steps = browser_batch_steps(input)?;
    let options = browser_batch_options(input);
    let mut entries = Vec::with_capacity(steps.len());
    let mut ok = true;
    let mut stopped_at = serde_json::Value::Null;
    let mut transient_content = Vec::new();

    browser_batch_emit_progress(format!(
        "Browser activity · {} action{}",
        steps.len(),
        if steps.len() == 1 { "" } else { "s" }
    ));

    for (idx, step) in steps.iter().enumerate() {
        let (action, op) = match parse_browser_batch_op(step) {
            Ok(parsed) => parsed,
            Err(error) => {
                ok = false;
                stopped_at = serde_json::json!(idx);
                entries.push(browser_batch_invalid_step_entry(idx, steps.len(), error));
                if options.stop_on_error {
                    break;
                }
                continue;
            }
        };

        browser_batch_emit_progress(browser_batch_step_start_message(idx, steps.len(), &op));
        let step_result =
            browser_batch_step_entry(idx, action, op, mgr, agent_id, &options).await?;
        let entry = step_result.entry;
        transient_content.extend(step_result.transient_content);
        browser_batch_emit_progress(browser_batch_step_done_message(idx, steps.len(), &entry));

        if entry["success"].as_bool() == Some(false) {
            ok = false;
            stopped_at = serde_json::json!(idx);
            entries.push(entry);
            if options.stop_on_error {
                break;
            }
        } else {
            entries.push(entry);
        }
    }

    browser_batch_emit_progress(format!(
        "o final observation · {}",
        browser_final_observation_label(&options.final_observation)
    ));
    let final_state = browser_batch_final_state(mgr, agent_id, &options).await?;

    browser_batch_emit_progress(format!(
        "{} browser_batch complete · {} action{} executed",
        if ok { "✓" } else { "x" },
        entries.len(),
        if entries.len() == 1 { "" } else { "s" }
    ));

    Ok(BrowserToolResult {
        content: browser_batch_response(ok, entries, stopped_at, final_state)?,
        transient_content,
    })
}

struct BrowserBatchOptions {
    stop_on_error: bool,
    include_data: bool,
    final_observation: String,
    max_elements: usize,
}

struct BrowserBatchStepResult {
    entry: serde_json::Value,
    transient_content: Vec<ContentBlock>,
}

impl BrowserBatchStepResult {
    fn text(entry: serde_json::Value) -> Self {
        Self {
            entry,
            transient_content: Vec::new(),
        }
    }
}

fn browser_batch_steps(input: &serde_json::Value) -> Result<&Vec<serde_json::Value>, String> {
    let steps = input["steps"]
        .as_array()
        .ok_or("Missing 'steps' array parameter")?;
    if steps.is_empty() {
        return Err("'steps' must contain at least one browser action".to_string());
    }
    if steps.len() > MAX_BATCH_STEPS {
        return Err(format!(
            "'steps' accepts at most {MAX_BATCH_STEPS} actions per browser_batch call"
        ));
    }
    Ok(steps)
}

fn browser_batch_options(input: &serde_json::Value) -> BrowserBatchOptions {
    BrowserBatchOptions {
        stop_on_error: input["stop_on_error"].as_bool().unwrap_or(true),
        include_data: input["include_data"].as_bool().unwrap_or(false),
        final_observation: input["final_observation"]
            .as_str()
            .unwrap_or("observe")
            .to_string(),
        max_elements: input["max_elements"]
            .as_u64()
            .unwrap_or(DEFAULT_OBSERVE_ELEMENTS as u64)
            .clamp(1, MAX_OBSERVE_ELEMENTS as u64) as usize,
    }
}

fn browser_batch_invalid_step_entry(
    index: usize,
    total: usize,
    error: String,
) -> serde_json::Value {
    browser_batch_emit_progress(format!(
        "x {}/{} invalid step · {}",
        index + 1,
        total,
        captain_types::truncate_str(&error, 160)
    ));
    serde_json::json!({
        "index": index,
        "success": false,
        "error": error,
    })
}

async fn browser_batch_step_entry(
    index: usize,
    action: String,
    op: BrowserBatchOp,
    mgr: &BrowserManager,
    agent_id: &str,
    options: &BrowserBatchOptions,
) -> Result<BrowserBatchStepResult, String> {
    Ok(match op {
        BrowserBatchOp::Command(command) => {
            let resp = mgr.send_command(agent_id, command).await?;
            BrowserBatchStepResult::text(browser_batch_command_entry(
                index,
                &action,
                resp,
                options.include_data,
            ))
        }
        BrowserBatchOp::Screenshot { prompt } => {
            let resp = mgr
                .send_command(agent_id, BrowserCommand::Screenshot)
                .await?;
            if !resp.success {
                BrowserBatchStepResult::text(serde_json::json!({
                    "index": index,
                    "action": action,
                    "success": false,
                    "error": resp.error.unwrap_or_else(|| "Screenshot failed".to_string()),
                }))
            } else {
                let data = resp.data.unwrap_or_default();
                let payload = screenshot_payload(&data, prompt.as_deref())?;
                let mut entry = serde_json::json!({
                    "index": index,
                    "action": action,
                    "success": true,
                    "summary": payload.metadata,
                });
                if options.include_data {
                    entry["data"] = entry["summary"].clone();
                }
                BrowserBatchStepResult {
                    entry,
                    transient_content: payload.transient_content,
                }
            }
        }
        BrowserBatchOp::Status => BrowserBatchStepResult::text(serde_json::json!({
            "index": index,
            "action": action,
            "success": true,
            "data": mgr.status(agent_id).await,
        })),
        BrowserBatchOp::NetworkLog { limit, clear } => {
            BrowserBatchStepResult::text(serde_json::json!({
                "index": index,
                "action": action,
                "success": true,
                "data": mgr.network_log(agent_id, limit, clear).await,
            }))
        }
        BrowserBatchOp::Diagnostics { limit, clear } => {
            BrowserBatchStepResult::text(serde_json::json!({
                "index": index,
                "action": action,
                "success": true,
                "data": mgr.diagnostics(agent_id, limit, clear, options.max_elements).await,
            }))
        }
        BrowserBatchOp::Close => {
            mgr.close_session(agent_id).await;
            BrowserBatchStepResult::text(serde_json::json!({
                "index": index,
                "action": action,
                "success": true,
                "summary": {"closed": true},
            }))
        }
    })
}

async fn browser_batch_final_state(
    mgr: &BrowserManager,
    agent_id: &str,
    options: &BrowserBatchOptions,
) -> Result<serde_json::Value, String> {
    match options.final_observation.as_str() {
        "none" => Ok(serde_json::Value::Null),
        "status" => Ok(mgr.status(agent_id).await),
        "diagnostics" => Ok(mgr.diagnostics(agent_id, 50, false, options.max_elements).await),
        "read_page" => browser_final_read_page(mgr, agent_id).await,
        "observe" | "" => Ok(mgr.observe(agent_id, options.max_elements).await),
        other => Err(format!(
            "Unknown final_observation '{other}'. Use observe, read_page, status, diagnostics, or none."
        )),
    }
}

fn browser_batch_response(
    ok: bool,
    entries: Vec<serde_json::Value>,
    stopped_at: serde_json::Value,
    final_state: serde_json::Value,
) -> Result<String, String> {
    serde_json::to_string_pretty(&serde_json::json!({
        "success": ok,
        "steps_executed": entries.len(),
        "stopped_at": stopped_at,
        "final_observation": final_state,
        "results": entries,
    }))
    .map_err(|e| format!("Browser batch serialization failed: {e}"))
}

fn browser_batch_emit_progress(message: impl AsRef<str>) {
    let message = message.as_ref().trim();
    if message.is_empty() {
        return;
    }
    crate::tool_runner::emit_tool_chunk("progress", &format!("{message}\n"));
}

fn browser_batch_step_start_message(index: usize, total: usize, op: &BrowserBatchOp) -> String {
    format!("> {}/{} {}", index + 1, total, browser_batch_op_label(op))
}

fn browser_batch_step_done_message(
    index: usize,
    total: usize,
    entry: &serde_json::Value,
) -> String {
    let action = entry["action"].as_str().unwrap_or("step");
    if entry["success"].as_bool() == Some(false) {
        let error = entry["error"].as_str().unwrap_or("Browser command failed");
        return format!(
            "x {}/{} {} failed · {}",
            index + 1,
            total,
            browser_action_label(action),
            captain_types::truncate_str(error, 140)
        );
    }

    let summary = &entry["summary"];
    let detail = browser_progress_summary(action, summary);
    if detail.is_empty() {
        format!("✓ {}/{} {}", index + 1, total, browser_action_label(action))
    } else {
        format!(
            "✓ {}/{} {} · {}",
            index + 1,
            total,
            browser_action_label(action),
            detail
        )
    }
}

fn browser_batch_op_label(op: &BrowserBatchOp) -> String {
    match op {
        BrowserBatchOp::Command(BrowserCommand::Navigate { url }) => {
            format!("open {}", browser_display_url(url))
        }
        BrowserBatchOp::Command(BrowserCommand::Click { selector }) => {
            format!("click {}", browser_display_selector(selector))
        }
        BrowserBatchOp::Command(BrowserCommand::Type { selector, text }) => {
            format!(
                "type {} into {}",
                browser_display_typed_text(selector, text),
                browser_display_selector(selector)
            )
        }
        BrowserBatchOp::Command(BrowserCommand::Keys { keys }) => {
            format!("press {}", browser_display_value(keys, 48))
        }
        BrowserBatchOp::Command(BrowserCommand::Select { selector, value }) => {
            format!(
                "select {} in {}",
                browser_display_value(value, 48),
                browser_display_selector(selector)
            )
        }
        BrowserBatchOp::Command(BrowserCommand::Hover { selector }) => {
            format!("hover {}", browser_display_selector(selector))
        }
        BrowserBatchOp::Screenshot { prompt: Some(_) } => {
            "capture and analyze screenshot".to_string()
        }
        BrowserBatchOp::Screenshot { prompt: None }
        | BrowserBatchOp::Command(BrowserCommand::Screenshot) => "capture screenshot".to_string(),
        BrowserBatchOp::Command(BrowserCommand::ReadPage) => "read page".to_string(),
        BrowserBatchOp::Command(BrowserCommand::Close) | BrowserBatchOp::Close => {
            "close browser".to_string()
        }
        BrowserBatchOp::Command(BrowserCommand::Scroll { direction, amount }) => {
            format!("scroll {direction} {amount}px")
        }
        BrowserBatchOp::Command(BrowserCommand::Wait {
            selector,
            timeout_ms,
        }) => format!(
            "wait for {} up to {}ms",
            browser_display_selector(selector),
            timeout_ms
        ),
        BrowserBatchOp::Command(BrowserCommand::RunJs { expression }) => {
            format!("run JavaScript {}", browser_display_value(expression, 70))
        }
        BrowserBatchOp::Command(BrowserCommand::Back) => "go back".to_string(),
        BrowserBatchOp::Command(BrowserCommand::Observe { max_elements }) => {
            format!("observe page controls (max {max_elements})")
        }
        BrowserBatchOp::Status => "inspect browser status".to_string(),
        BrowserBatchOp::NetworkLog { limit, clear } => {
            format!("read network log (limit {limit}, clear {clear})")
        }
        BrowserBatchOp::Diagnostics { limit, clear } => {
            format!("run diagnostics (limit {limit}, clear {clear})")
        }
    }
}

fn browser_progress_summary(action: &str, summary: &serde_json::Value) -> String {
    let mut parts = Vec::new();
    if let Some(note) = browser_antibot_note(summary) {
        parts.push(note.to_string());
    }
    if let Some(url) = summary.get("url").and_then(|v| v.as_str()) {
        if !url.is_empty() {
            parts.push(browser_display_url(url));
        }
    }
    if let Some(title) = summary.get("title").and_then(|v| v.as_str()) {
        if !title.is_empty() {
            parts.push(format!("title {}", browser_display_value(title, 80)));
        }
    }
    if let Some(chars) = summary.get("content_chars").and_then(|v| v.as_u64()) {
        if chars > 0 {
            parts.push(format!("{chars} chars"));
        }
    }
    if action.contains("screenshot") {
        let count = summary
            .get("image_urls")
            .and_then(|v| v.as_array())
            .map(|v| v.len())
            .unwrap_or(0);
        parts.push(format!(
            "{count} image{}",
            if count == 1 { "" } else { "s" }
        ));
        if let Some(status) = summary
            .pointer("/visual_analysis/status")
            .and_then(|value| value.as_str())
        {
            parts.push(format!("visual {status}"));
        }
    }
    if parts.is_empty() {
        summary
            .get("data_preview")
            .and_then(|v| v.as_str())
            .or_else(|| summary.get("result_preview").and_then(|v| v.as_str()))
            .map(|s| browser_display_value(s, 120))
            .unwrap_or_default()
    } else {
        parts.join(" · ")
    }
}

fn browser_antibot_note(summary: &serde_json::Value) -> Option<&'static str> {
    let mut haystack = String::new();
    for key in [
        "url",
        "title",
        "content_preview",
        "data_preview",
        "result_preview",
    ] {
        if let Some(value) = summary.get(key).and_then(|v| v.as_str()) {
            haystack.push_str(value);
            haystack.push('\n');
        }
    }
    let lower = haystack.to_ascii_lowercase();
    if lower.contains("google.com/sorry")
        || lower.contains("unusual traffic")
        || lower.contains("trafic exceptionnel")
        || lower.contains("captcha")
        || lower.contains("not a robot")
        || lower.contains("automated queries")
    {
        Some("anti-bot/CAPTCHA detected")
    } else {
        None
    }
}

fn browser_final_observation_label(value: &str) -> &'static str {
    match value {
        "none" => "none",
        "status" => "status",
        "diagnostics" => "diagnostics",
        "read_page" => "read page",
        "observe" | "" => "observe",
        _ => "custom",
    }
}

fn browser_action_label(action: &str) -> &'static str {
    match action {
        "navigate" | "browser_navigate" => "open",
        "click" | "browser_click" => "click",
        "type" | "browser_type" => "type",
        "keys" | "press" | "browser_keys" => "keys",
        "select" | "browser_select" => "select",
        "hover" | "browser_hover" => "hover",
        "scroll" | "browser_scroll" => "scroll",
        "wait" | "browser_wait" => "wait",
        "run_js" | "js" | "browser_run_js" => "JavaScript",
        "back" | "browser_back" => "back",
        "read_page" | "read" | "browser_read_page" => "read",
        "screenshot" | "browser_screenshot" => "screenshot",
        "observe" | "browser_observe" => "observe",
        "status" | "browser_status" => "status",
        "network_log" | "browser_network_log" => "network",
        "diagnostics" | "browser_diagnostics" => "diagnostics",
        "close" | "browser_close" => "close",
        _ => "step",
    }
}

fn browser_display_url(url: &str) -> String {
    let redacted = redact_url_for_display(url);
    browser_display_value(&redacted, 120)
}

fn redact_url_for_display(url: &str) -> String {
    let mut out = url.to_string();
    for marker in ["api_key=", "apikey=", "token=", "secret=", "password="] {
        if let Some(pos) = out.to_ascii_lowercase().find(marker) {
            let value_start = pos + marker.len();
            let value_end = out[value_start..]
                .find(['&', '#'])
                .map(|offset| value_start + offset)
                .unwrap_or(out.len());
            out.replace_range(value_start..value_end, "[redacted]");
        }
    }
    out
}

fn browser_display_selector(selector: &str) -> String {
    browser_display_value(selector, 72)
}

fn browser_display_typed_text(selector: &str, text: &str) -> String {
    let lower = selector.to_ascii_lowercase();
    let sensitive = ["password", "token", "secret", "api", "key", "auth"]
        .iter()
        .any(|needle| lower.contains(needle));
    if sensitive {
        format!("[masked, {} chars]", text.chars().count())
    } else {
        browser_display_value(text, 80)
    }
}

fn browser_display_value(value: &str, max_chars: usize) -> String {
    let clean = value.split_whitespace().collect::<Vec<_>>().join(" ");
    let clipped = captain_types::truncate_str(&clean, max_chars).to_string();
    if clipped.is_empty() {
        "\"\"".to_string()
    } else {
        format!("\"{clipped}\"")
    }
}

fn browser_batch_command_entry(
    index: usize,
    action: &str,
    resp: BrowserResponse,
    include_data: bool,
) -> serde_json::Value {
    if !resp.success {
        return serde_json::json!({
            "index": index,
            "action": action,
            "success": false,
            "error": resp.error.unwrap_or_else(|| "Browser command failed".to_string()),
        });
    }

    let data = resp.data.unwrap_or_default();
    let summary = browser_command_summary(&data);
    let mut entry = serde_json::json!({
        "index": index,
        "action": action,
        "success": true,
        "summary": summary,
    });
    if include_data {
        entry["data"] = redact_large_browser_data(data);
    }
    entry
}

fn browser_command_summary(data: &serde_json::Value) -> serde_json::Value {
    let title = data["title"].as_str().unwrap_or("");
    let url = data["url"].as_str().unwrap_or("");
    let content = data["content"].as_str().unwrap_or("");
    if !title.is_empty() || !url.is_empty() || !content.is_empty() {
        return serde_json::json!({
            "title": title,
            "url": url,
            "content_chars": content.chars().count(),
            "content_preview": captain_types::truncate_str(content, 900),
        });
    }

    if let Some(result) = data.get("result") {
        let s = serde_json::to_string(result).unwrap_or_default();
        return serde_json::json!({
            "result_preview": captain_types::truncate_str(&s, 900),
        });
    }

    let s = serde_json::to_string(data).unwrap_or_default();
    serde_json::json!({
        "data_preview": captain_types::truncate_str(&s, 900),
    })
}

fn redact_large_browser_data(mut data: serde_json::Value) -> serde_json::Value {
    if let Some(content) = data
        .get("content")
        .and_then(|v| v.as_str())
        .map(|s| captain_types::truncate_str(s, 8_000).to_string())
    {
        data["content"] = serde_json::Value::String(content);
    }
    data
}

async fn browser_final_read_page(
    mgr: &BrowserManager,
    agent_id: &str,
) -> Result<serde_json::Value, String> {
    let resp = mgr.send_command(agent_id, BrowserCommand::ReadPage).await?;
    if !resp.success {
        return Ok(serde_json::json!({
            "success": false,
            "error": resp.error.unwrap_or_else(|| "ReadPage failed".to_string()),
        }));
    }

    let data = resp.data.unwrap_or_default();
    let title = data["title"].as_str().unwrap_or("(no title)");
    let url = data["url"].as_str().unwrap_or("");
    let content = data["content"].as_str().unwrap_or("");
    Ok(serde_json::json!({
        "success": true,
        "title": title,
        "url": url,
        "content": crate::web_content::wrap_external_content(url, content),
    }))
}

/// JavaScript to extract readable page content as markdown.
const EXTRACT_CONTENT_JS: &str = r#"(() => {
    const title = document.title || '';
    const url = location.href || '';
    const body = document.body;
    if (!body) return JSON.stringify({title, url, content: ''});

    const clone = body.cloneNode(true);
    const remove = ['script','style','nav','footer','header','aside','iframe','noscript','svg','canvas'];
    remove.forEach(tag => clone.querySelectorAll(tag).forEach(el => el.remove()));

    let root = clone.querySelector('main, article, [role="main"], .content, #content');
    if (!root) root = clone;

    const lines = [];
    function walk(node) {
        if (node.nodeType === 3) {
            const t = node.textContent.trim();
            if (t) lines.push(t);
            return;
        }
        if (node.nodeType !== 1) return;
        const tag = node.tagName.toLowerCase();
        if (['h1','h2','h3','h4','h5','h6'].includes(tag)) {
            const level = '#'.repeat(parseInt(tag[1]));
            lines.push('\n' + level + ' ' + node.textContent.trim());
            return;
        }
        if (tag === 'a' && node.href && node.textContent.trim()) {
            lines.push('[' + node.textContent.trim() + '](' + node.href + ')');
            return;
        }
        if (tag === 'li') {
            lines.push('- ' + node.textContent.trim());
            return;
        }
        if (tag === 'br') { lines.push(''); return; }
        if (['p','div','section','tr'].includes(tag)) lines.push('');
        for (const child of node.childNodes) walk(child);
        if (['p','div','section','tr'].includes(tag)) lines.push('');
    }
    walk(root);

    let content = lines.join('\n').replace(/\n{3,}/g, '\n\n').trim();
    if (content.length > 50000) content = content.substring(0, 50000) + '\n... (truncated)';
    return JSON.stringify({title, url, content});
})()"#;

// ── Root detection ─────────────────────────────────────────────────────────

/// Returns true if the current process is running as root (UID 0).
///
/// On Linux, reads `/proc/self/status` to get the effective UID without
/// requiring a `libc` dependency. Falls back to checking the `HOME` env var
/// on systems where `/proc` is not available.
fn is_running_as_root() -> bool {
    #[cfg(unix)]
    {
        // Primary: read effective UID from /proc/self/status (Linux)
        if let Ok(status) = std::fs::read_to_string("/proc/self/status") {
            for line in status.lines() {
                if let Some(rest) = line.strip_prefix("Uid:") {
                    // Format: "Uid:	<real> <effective> <saved> <fs>"
                    if let Some(euid_str) = rest.split_whitespace().nth(1) {
                        return euid_str == "0";
                    }
                }
            }
        }
        // Fallback: HOME=/root is a reliable indicator on most Unix systems
        std::env::var("HOME").map(|h| h == "/root").unwrap_or(false)
    }
    #[cfg(not(unix))]
    {
        false
    }
}

// ── Tests ──────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_browser_config_defaults() {
        let config = BrowserConfig::default();
        assert!(config.headless);
        assert_eq!(config.viewport_width, 1280);
        assert_eq!(config.viewport_height, 720);
        assert_eq!(config.timeout_secs, 30);
        assert_eq!(config.idle_timeout_secs, 300);
        assert_eq!(config.max_sessions, 5);
        assert!(config.chromium_path.is_none());
    }

    #[test]
    fn test_browser_command_serialize_navigate() {
        let cmd = BrowserCommand::Navigate {
            url: "https://example.com".to_string(),
        };
        let json = serde_json::to_string(&cmd).unwrap();
        assert!(json.contains("\"action\":\"Navigate\""));
        assert!(json.contains("\"url\":\"https://example.com\""));
    }

    #[test]
    fn test_browser_command_serialize_click() {
        let cmd = BrowserCommand::Click {
            selector: "#submit-btn".to_string(),
        };
        let json = serde_json::to_string(&cmd).unwrap();
        assert!(json.contains("\"action\":\"Click\""));
        assert!(json.contains("\"selector\":\"#submit-btn\""));
    }

    #[test]
    fn test_browser_command_serialize_type() {
        let cmd = BrowserCommand::Type {
            selector: "input[name='email']".to_string(),
            text: "test@example.com".to_string(),
        };
        let json = serde_json::to_string(&cmd).unwrap();
        assert!(json.contains("\"action\":\"Type\""));
        assert!(json.contains("test@example.com"));
    }

    #[test]
    fn test_browser_command_serialize_keys() {
        let cmd = BrowserCommand::Keys {
            keys: "Enter".to_string(),
        };
        let json = serde_json::to_string(&cmd).unwrap();
        assert!(json.contains("\"action\":\"Keys\""));
        assert!(json.contains("\"keys\":\"Enter\""));
    }

    #[test]
    fn test_browser_command_serialize_select() {
        let cmd = BrowserCommand::Select {
            selector: "select[name='country']".to_string(),
            value: "France".to_string(),
        };
        let json = serde_json::to_string(&cmd).unwrap();
        assert!(json.contains("\"action\":\"Select\""));
        assert!(json.contains("France"));
    }

    #[test]
    fn test_browser_command_serialize_hover() {
        let cmd = BrowserCommand::Hover {
            selector: "@e2".to_string(),
        };
        let json = serde_json::to_string(&cmd).unwrap();
        assert!(json.contains("\"action\":\"Hover\""));
        assert!(json.contains("\"selector\":\"@e2\""));
    }

    #[test]
    fn test_browser_command_serialize_screenshot() {
        let cmd = BrowserCommand::Screenshot;
        let json = serde_json::to_string(&cmd).unwrap();
        assert!(json.contains("\"action\":\"Screenshot\""));
    }

    #[test]
    fn test_browser_command_serialize_read_page() {
        let cmd = BrowserCommand::ReadPage;
        let json = serde_json::to_string(&cmd).unwrap();
        assert!(json.contains("\"action\":\"ReadPage\""));
    }

    #[test]
    fn test_browser_command_serialize_close() {
        let cmd = BrowserCommand::Close;
        let json = serde_json::to_string(&cmd).unwrap();
        assert!(json.contains("\"action\":\"Close\""));
    }

    #[test]
    fn test_browser_command_serialize_scroll() {
        let cmd = BrowserCommand::Scroll {
            direction: "down".to_string(),
            amount: 500,
        };
        let json = serde_json::to_string(&cmd).unwrap();
        assert!(json.contains("\"action\":\"Scroll\""));
        assert!(json.contains("\"amount\":500"));
    }

    #[test]
    fn test_browser_command_serialize_run_js() {
        let cmd = BrowserCommand::RunJs {
            expression: "document.title".to_string(),
        };
        let json = serde_json::to_string(&cmd).unwrap();
        assert!(json.contains("\"action\":\"RunJs\""));
    }

    #[test]
    fn test_browser_command_serialize_back() {
        let cmd = BrowserCommand::Back;
        let json = serde_json::to_string(&cmd).unwrap();
        assert!(json.contains("\"action\":\"Back\""));
    }

    #[test]
    fn test_browser_command_serialize_wait() {
        let cmd = BrowserCommand::Wait {
            selector: "#loaded".to_string(),
            timeout_ms: 3000,
        };
        let json = serde_json::to_string(&cmd).unwrap();
        assert!(json.contains("\"action\":\"Wait\""));
        assert!(json.contains("\"timeout_ms\":3000"));
    }

    #[test]
    fn test_browser_command_serialize_observe() {
        let cmd = BrowserCommand::Observe { max_elements: 42 };
        let json = serde_json::to_string(&cmd).unwrap();
        assert!(json.contains("\"action\":\"Observe\""));
        assert!(json.contains("\"max_elements\":42"));
    }

    #[test]
    fn browser_key_spec_supports_common_shortcuts() {
        let enter = browser_key_spec("Enter");
        assert_eq!(enter.key, "Enter");
        assert_eq!(enter.key_code, Some(13));

        let select_all = browser_key_spec("Control+a");
        assert_eq!(select_all.modifiers, 2);
        assert_eq!(select_all.key, "a");

        let text = browser_key_spec("hello");
        assert_eq!(text.insert_text.as_deref(), Some("hello"));
    }

    #[test]
    fn test_browser_response_deserialize() {
        let json =
            r#"{"success": true, "data": {"title": "Example", "url": "https://example.com"}}"#;
        let resp: BrowserResponse = serde_json::from_str(json).unwrap();
        assert!(resp.success);
        assert!(resp.data.is_some());
        assert!(resp.error.is_none());
        let data = resp.data.unwrap();
        assert_eq!(data["title"], "Example");
    }

    #[test]
    fn test_browser_response_error_deserialize() {
        let json = r#"{"success": false, "error": "Element not found"}"#;
        let resp: BrowserResponse = serde_json::from_str(json).unwrap();
        assert!(!resp.success);
        assert!(resp.data.is_none());
        assert_eq!(resp.error.unwrap(), "Element not found");
    }

    #[test]
    fn test_browser_manager_new() {
        let config = BrowserConfig::default();
        let mgr = BrowserManager::new(config);
        assert!(mgr.sessions.is_empty());
    }

    #[test]
    fn test_is_running_as_root_returns_bool() {
        // Just verify it doesn't panic and returns a bool.
        let _ = is_running_as_root();
    }

    #[test]
    fn test_chromium_candidates_not_empty() {
        let paths = chromium_candidates();
        assert!(
            !paths.is_empty(),
            "Should have platform-specific candidates"
        );
    }

    #[test]
    fn test_response_helpers() {
        let ok = BrowserResponse::ok(serde_json::json!({"a": 1}));
        assert!(ok.success);
        assert!(ok.error.is_none());

        let err = BrowserResponse::err("bad");
        assert!(!err.success);
        assert_eq!(err.error.unwrap(), "bad");
    }
}
