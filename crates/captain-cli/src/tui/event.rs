//! Event system: crossterm polling, tick timer, streaming bridges.

use crate::agent_api_sheet::AgentApiSpawnSheet;
use captain_kernel::CaptainKernel;
use captain_runtime::agent_loop::AgentLoopResult;
use captain_runtime::llm_driver::StreamEvent;
use captain_types::agent::AgentId;
use ratatui::crossterm::event::{
    self, Event as CtEvent, KeyEvent, KeyEventKind, MouseButton, MouseEventKind,
};
use std::sync::{mpsc, Arc};
use std::time::Duration;

use super::event_memory::memory_event_from_json;
use super::event_stream::{daemon_stream_events_from_sse_line, DaemonStreamState};
use super::screens::{
    audit::AuditEntry,
    channels::{ChannelInfo, ChannelStatus},
    comms::{CommsEdge, CommsEventItem, CommsNode},
    dashboard::{AuditRow, StatusSnapshot},
    extensions::{ExtensionHealthInfo, ExtensionInfo},
    graph::{GraphEntity, GraphFact, GraphStats},
    hands::{HandInfo, HandInstanceInfo},
    learning::{CommittedRow, LearningMetrics, ReviewItem},
    logs::LogEntry,
    memory::{AgentEntry, KvPair},
    peers::PeerInfo,
    projects::{
        ProjectDetail, ProjectGoal, ProjectInfo, ProjectRuntimeEvent, ProjectRuntimeWorker,
        ProjectTask,
    },
    security::SecurityFeature,
    sessions::SessionInfo,
    settings::{ModelInfo, ProviderInfo, TestResult, ToolInfo},
    skills::{ClawHubResult, McpServerInfo, SkillInfo},
    skills_proposed::{Pattern, Proposal, SkillsMetrics},
    templates::ProviderAuth,
    triggers::TriggerInfo,
    usage::{AgentUsage, ModelUsage, UsageSummary},
    workflows::{WorkflowInfo, WorkflowRun},
};
use super::session_runtime::{self, LoadedSession};

fn env_flag_enabled(value: &str) -> bool {
    matches!(
        value.trim().to_ascii_lowercase().as_str(),
        "1" | "true" | "yes" | "on"
    )
}

fn mouse_capture_default_from_env(
    tui_mouse: Option<&str>,
    web_terminal: Option<&str>,
    prefer_enabled: bool,
) -> bool {
    if let Some(value) = tui_mouse {
        return env_flag_enabled(value);
    }
    prefer_enabled || web_terminal.map(env_flag_enabled).unwrap_or(false)
}

pub(super) fn mouse_capture_default() -> bool {
    // Was gated behind SSH/web-terminal detection, so mouse scroll silently
    // did nothing on a plain local terminal (keyboard arrows still worked,
    // masking the gap). Default to enabled everywhere, matching standalone
    // chat — `/mouse off` (or `CAPTAIN_TUI_MOUSE=0`) restores native
    // terminal text selection for anyone who prefers that trade-off.
    let tui_mouse = std::env::var("CAPTAIN_TUI_MOUSE").ok();
    let web_terminal = std::env::var("CAPTAIN_WEB_TERMINAL").ok();
    mouse_capture_default_from_env(tui_mouse.as_deref(), web_terminal.as_deref(), true)
}

pub(super) fn standalone_chat_mouse_capture_default() -> bool {
    let tui_mouse = std::env::var("CAPTAIN_TUI_MOUSE").ok();
    let web_terminal = std::env::var("CAPTAIN_WEB_TERMINAL").ok();
    mouse_capture_default_from_env(tui_mouse.as_deref(), web_terminal.as_deref(), true)
}

pub(super) fn set_mouse_capture(enabled: bool) -> Result<(), String> {
    use ratatui::crossterm::event::{DisableMouseCapture, EnableMouseCapture};
    use ratatui::crossterm::execute;

    let mut stdout = std::io::stdout();
    if enabled {
        execute!(stdout, EnableMouseCapture).map_err(|e| e.to_string())
    } else {
        execute!(stdout, DisableMouseCapture).map_err(|e| e.to_string())
    }
}

#[cfg(test)]
mod mouse_capture_tests {
    use super::*;

    #[test]
    fn web_terminal_enables_mouse_capture_by_default() {
        assert!(mouse_capture_default_from_env(None, Some("1"), false));
        assert!(mouse_capture_default_from_env(None, Some("true"), false));
    }

    #[test]
    fn native_terminal_keeps_mouse_capture_disabled_by_default() {
        assert!(!mouse_capture_default_from_env(None, None, false));
        assert!(!mouse_capture_default_from_env(None, Some("0"), false));
    }

    #[test]
    fn explicit_tui_mouse_overrides_web_terminal_default() {
        assert!(!mouse_capture_default_from_env(
            Some("off"),
            Some("1"),
            true
        ));
        assert!(mouse_capture_default_from_env(Some("on"), None, false));
    }

    #[test]
    fn standalone_chat_can_prefer_mouse_capture_for_tui_scrollback() {
        assert!(mouse_capture_default_from_env(None, None, true));
        assert!(!mouse_capture_default_from_env(Some("0"), None, true));
    }
}

fn channel_status_from_api(ch: &serde_json::Value) -> ChannelStatus {
    let configured = ch["configured"].as_bool().unwrap_or(false);
    let has_token = ch["has_token"].as_bool().unwrap_or(true);
    let ready = ch["ready"].as_bool();
    match ch["status"].as_str() {
        Some("ready") => ChannelStatus::Ready,
        Some("missing_env") => ChannelStatus::MissingEnv,
        Some(_) => ChannelStatus::NotConfigured,
        None if ready == Some(true) => ChannelStatus::Ready,
        None if ready == Some(false) && configured => ChannelStatus::MissingEnv,
        None if configured && has_token => ChannelStatus::Ready,
        None if configured => ChannelStatus::MissingEnv,
        None => ChannelStatus::NotConfigured,
    }
}

#[cfg(test)]
mod channel_status_tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn explicit_status_takes_precedence() {
        assert!(
            channel_status_from_api(&json!({"status": "ready", "ready": false}))
                == ChannelStatus::Ready
        );
        assert!(
            channel_status_from_api(&json!({"status": "missing_env", "ready": true}))
                == ChannelStatus::MissingEnv
        );
        assert!(
            channel_status_from_api(&json!({"status": "frozen", "configured": true}))
                == ChannelStatus::NotConfigured
        );
    }

    #[test]
    fn ready_field_is_used_when_legacy_status_is_absent() {
        assert!(channel_status_from_api(&json!({"ready": true})) == ChannelStatus::Ready);
        assert!(
            channel_status_from_api(&json!({"ready": false, "configured": true}))
                == ChannelStatus::MissingEnv
        );
        assert!(
            channel_status_from_api(&json!({"ready": false, "configured": false}))
                == ChannelStatus::NotConfigured
        );
    }

    #[test]
    fn legacy_configured_token_fields_still_work() {
        assert!(
            channel_status_from_api(&json!({"configured": true, "has_token": true}))
                == ChannelStatus::Ready
        );
        assert!(
            channel_status_from_api(&json!({"configured": true, "has_token": false}))
                == ChannelStatus::MissingEnv
        );
        assert!(
            channel_status_from_api(&json!({"configured": false})) == ChannelStatus::NotConfigured
        );
    }
}

// ── BackendRef ──────────────────────────────────────────────────────────────

/// Lightweight reference to the active backend, for passing to spawn functions.
#[derive(Clone)]
pub enum BackendRef {
    Daemon(String),
    InProcess(Arc<CaptainKernel>),
}

// ── AppEvent ────────────────────────────────────────────────────────────────

/// Unified application event.
pub enum AppEvent {
    /// A crossterm key press event (filtered to Press only).
    Key(KeyEvent),
    /// A crossterm bracketed paste event — the full pasted blob, including
    /// embedded newlines. Routed to the active screen so multi-line content
    /// stays in the input buffer instead of being interpreted as N Enter keys.
    Paste(String),
    /// Mouse scroll wheel event (only emitted when EnableMouseCapture is on).
    /// `up = true` means the user scrolled up (= move viewport up = older
    /// messages). The active screen translates this into its own scroll model.
    Scroll { up: bool },
    /// Mouse click event in terminal coordinates.
    MouseClick { x: u16, y: u16 },
    /// Periodic tick for animations (spinners, etc.).
    Tick,
    /// A streaming event from the LLM (daemon SSE or kernel mpsc).
    Stream(StreamEvent),
    /// The streaming agent loop finished.
    StreamDone(Result<AgentLoopResult, String>),
    /// IJ.4 — fired right after `send_message_streaming` returns the live
    /// `user_input_tx`. Lets the main loop forward keystrokes typed during
    /// the stream into the running agent loop (parity with Telegram's
    /// `IJ.2` interjection). `None` for daemon mode where the tx lives
    /// server-side and is reachable only through the SSE endpoint.
    StreamStarted {
        interject_tx: tokio::sync::mpsc::Sender<String>,
    },
    /// The kernel finished booting in the background.
    KernelReady(Arc<CaptainKernel>),
    /// The kernel failed to boot.
    KernelError(String),
    /// An agent was successfully spawned (daemon mode).
    AgentSpawned {
        id: String,
        name: String,
        api_sheet: Option<AgentApiSpawnSheet>,
    },
    /// Agent spawn failed.
    AgentSpawnError(String),
    /// Daemon detection result from background thread.
    DaemonDetected {
        url: Option<String>,
        agent_count: u64,
    },

    // ── New tab events ──────────────────────────────────────────────────────
    /// Dashboard data loaded.
    DashboardData {
        agent_count: u64,
        uptime_secs: u64,
        version: String,
        provider: String,
        model: String,
        status: StatusSnapshot,
    },
    /// Audit trail loaded.
    AuditLoaded(Vec<AuditRow>),
    /// Channel list loaded.
    ChannelListLoaded(Vec<ChannelInfo>),
    /// Channel test result.
    ChannelTestResult { success: bool, message: String },
    /// Workflow list loaded.
    WorkflowListLoaded(Vec<WorkflowInfo>),
    /// Workflow runs loaded for a specific workflow.
    WorkflowRunsLoaded(Vec<WorkflowRun>),
    /// Workflow run completed.
    WorkflowRunResult(String),
    /// Workflow created successfully.
    WorkflowCreated(String),
    /// Development projects loaded.
    ProjectListLoaded(Vec<ProjectInfo>),
    /// One development project detail loaded.
    ProjectDetailLoaded(ProjectDetail),
    /// Project mutation completed.
    ProjectMutated(String),
    /// Trigger list loaded.
    TriggerListLoaded(Vec<TriggerInfo>),
    /// Trigger created.
    TriggerCreated(String),
    /// Trigger deleted.
    TriggerDeleted(String),
    /// Phase-i.5: a trigger had its enabled flag toggled.
    TriggerToggled { id: String, enabled: bool },
    /// Agent killed successfully.
    AgentKilled { id: String },
    /// Agent kill failed.
    AgentKillError(String),
    /// Generic fetch error for any tab.
    FetchError(String),

    /// Phase O.2: a fact was committed to MemPalace by the auto-memorize
    /// pipeline. Surfaced as a discreet 🧠 line in the chat so the user
    /// sees what the reflection model captured. Fired by the kernel
    /// in-process subscription bridge installed at boot.
    MemoryStored {
        subject: String,
        predicate: String,
        object: String,
        source: String,
    },
    /// A learning candidate was queued for approval. This is still surfaced
    /// in the current chat so the user sees what Captain tried to learn even
    /// before accepting or rejecting it.
    MemoryQueued {
        review_id: String,
        subject: String,
        predicate: String,
        object: String,
        source: String,
    },
    /// A reusable workflow was drafted as a skill proposal. Critical
    /// self-improvement stays approval-only, but the proposal must appear in
    /// the current chat instead of hiding in the Skills* tab.
    SkillProposalQueued {
        proposal_id: String,
        name: String,
        description: String,
        trigger_hint: String,
        confidence: f32,
        family: Option<String>,
    },
    /// A sub-agent spawned/terminated/crashed — feeds the "background
    /// activity" badge in the chat status line so the user can see Captain
    /// is waiting on work happening off-screen.
    AgentLifecycle {
        kind: String,
        agent_id: String,
        name: Option<String>,
        detail: Option<String>,
    },
    /// A detached tool_run (execute_code/shell_exec/...) changed status —
    /// feeds the same "background activity" badge.
    ToolRunStatus {
        run_id: String,
        tool_name: String,
        status: String,
        caller_agent_id: Option<String>,
    },

    // ── New screen events ──────────────────────────────────────────────────
    /// Sessions loaded.
    SessionsLoaded(Vec<SessionInfo>),
    /// One persisted session loaded and ownership-resolved for chat restore.
    SessionLoaded(LoadedSession),
    /// Session deleted.
    SessionDeleted(String),
    /// Phase-h.1: Learning data loaded (pending + committed + metrics).
    LearningLoaded {
        pending: Vec<crate::tui::screens::learning::ReviewItem>,
        committed: Vec<crate::tui::screens::learning::CommittedRow>,
        metrics: Option<crate::tui::screens::learning::LearningMetrics>,
    },
    /// Phase-h.1: a learning review item was approved or denied.
    LearningDecided { id: String, approved: bool },
    /// Phase-h.2: Skills Proposed data loaded (proposals + patterns + metrics).
    SkillsProposedLoaded {
        proposals: Vec<crate::tui::screens::skills_proposed::Proposal>,
        patterns: Vec<crate::tui::screens::skills_proposed::Pattern>,
        metrics: Option<crate::tui::screens::skills_proposed::SkillsMetrics>,
    },
    /// Phase-h.2: a skill proposal was approved or denied.
    SkillProposalDecided { id: String, approved: bool },
    /// Phase-h.3: cron jobs list loaded.
    CronJobsLoaded(Vec<crate::tui::screens::cron::CronJob>),
    /// Phase-h.3: a cron job mutation completed (toggle/run/delete).
    CronJobMutated { id: String, what: &'static str },
    /// Phase-h.4: approvals list loaded.
    ApprovalsLoaded(Vec<crate::tui::screens::approvals::ApprovalRequest>),
    /// Phase-h.4: an approval was resolved.
    ApprovalDecided { id: String, approved: bool },
    /// Phase-i.6: a fresh approval request found for the chat agent — used to
    /// trigger the in-chat modal popup. None means the poll found nothing.
    ChatApprovalDetected(Option<crate::tui::screens::approvals::ApprovalRequest>),
    /// Phase-j.4: a /voice recording finished. Ok(path) → ready to upload,
    /// Err(msg) → tool missing or recording failed.
    VoiceRecorded(Result<std::path::PathBuf, String>),
    /// Phase-h.5: budget data loaded (global + per-agent).
    BudgetLoaded {
        global: Option<crate::tui::screens::budget::BudgetGlobal>,
        agents: Vec<crate::tui::screens::budget::AgentSpend>,
    },
    /// Phase-h.6: knowledge graph data loaded (stats + entities + facts).
    GraphLoaded {
        stats: Option<crate::tui::screens::graph::GraphStats>,
        entities: Vec<crate::tui::screens::graph::GraphEntity>,
        facts: Vec<crate::tui::screens::graph::GraphFact>,
    },
    /// Memory agents loaded (for agent selector).
    MemoryAgentsLoaded(Vec<AgentEntry>),
    /// Memory KV pairs loaded.
    MemoryKvLoaded(Vec<KvPair>),
    /// Memory KV saved.
    MemoryKvSaved { key: String },
    /// Memory KV deleted.
    MemoryKvDeleted(String),
    /// Skills loaded.
    SkillsLoaded(Vec<SkillInfo>),
    /// ClawHub results loaded.
    ClawHubLoaded(Vec<ClawHubResult>),
    /// Skill installed.
    SkillInstalled(String),
    /// Skill uninstalled.
    SkillUninstalled(String),
    /// MCP servers loaded.
    McpServersLoaded(Vec<McpServerInfo>),
    /// Templates providers loaded (auth status).
    TemplateProvidersLoaded(Vec<ProviderAuth>),
    /// Security features loaded.
    SecurityLoaded(Vec<SecurityFeature>),
    /// Security chain verification result.
    SecurityChainVerified { valid: bool, message: String },
    /// Audit entries loaded (full audit screen).
    AuditEntriesLoaded(Vec<AuditEntry>),
    /// Audit chain verified.
    AuditChainVerified(bool),
    /// Usage summary loaded.
    UsageSummaryLoaded(UsageSummary),
    /// Usage by model loaded.
    UsageByModelLoaded(Vec<ModelUsage>),
    /// Usage by agent loaded.
    UsageByAgentLoaded(Vec<AgentUsage>),
    /// Settings providers loaded.
    SettingsProvidersLoaded(Vec<ProviderInfo>),
    /// Settings models loaded.
    SettingsModelsLoaded(Vec<ModelInfo>),
    /// Settings tools loaded.
    SettingsToolsLoaded(Vec<ToolInfo>),
    /// Provider key saved.
    ProviderKeySaved(String),
    /// Provider key deleted.
    ProviderKeyDeleted(String),
    /// Provider test result.
    ProviderTestResult(TestResult),
    /// Peers loaded.
    PeersLoaded(Vec<PeerInfo>),
    /// Log entries loaded.
    LogsLoaded(Vec<LogEntry>),
    /// Hand definitions loaded (marketplace).
    HandsLoaded(Vec<HandInfo>),
    /// Active hand instances loaded.
    ActiveHandsLoaded(Vec<HandInstanceInfo>),
    /// Hand activated.
    HandActivated(String),
    /// Hand deactivated.
    HandDeactivated(String),
    /// Hand paused.
    HandPaused(String),
    /// Hand resumed.
    HandResumed(String),
    /// Extensions loaded (available + installed).
    ExtensionsLoaded(Vec<ExtensionInfo>),
    /// Extension health loaded.
    ExtensionHealthLoaded(Vec<ExtensionHealthInfo>),
    /// Extension installed.
    ExtensionInstalled(String),
    /// Extension removed.
    ExtensionRemoved(String),
    /// Extension reconnected.
    ExtensionReconnected(String, usize),
    /// Agent skills loaded (for edit screen).
    AgentSkillsLoaded {
        assigned: Vec<String>,
        available: Vec<String>,
    },
    /// Agent MCP servers loaded (for edit screen).
    AgentMcpServersLoaded {
        assigned: Vec<String>,
        available: Vec<String>,
    },
    /// Agent skills updated.
    AgentSkillsUpdated(String),
    /// Agent MCP servers updated.
    AgentMcpServersUpdated(String),
    /// Comms topology loaded.
    CommsTopologyLoaded {
        nodes: Vec<super::screens::comms::CommsNode>,
        edges: Vec<super::screens::comms::CommsEdge>,
    },
    /// Comms events loaded.
    CommsEventsLoaded(Vec<super::screens::comms::CommsEventItem>),
    /// Comms send result.
    CommsSendResult(String),
    /// Comms task post result.
    CommsTaskResult(String),
}

/// Spawn the crossterm polling + tick thread. Returns sender + receiver.
pub fn spawn_event_thread(
    tick_rate: Duration,
) -> (mpsc::Sender<AppEvent>, mpsc::Receiver<AppEvent>) {
    let (tx, rx) = mpsc::channel();
    let poll_tx = tx.clone();

    std::thread::spawn(move || {
        loop {
            if event::poll(tick_rate).unwrap_or(false) {
                if let Ok(ev) = event::read() {
                    let sent = match ev {
                        // CRITICAL: only forward Press events — Windows sends
                        // Release and Repeat too, which causes double/triple input
                        CtEvent::Key(key) if key.kind == KeyEventKind::Press => {
                            poll_tx.send(AppEvent::Key(key))
                        }
                        // Bracketed paste: full clipboard blob with newlines.
                        // Only emitted when EnableBracketedPaste is active.
                        CtEvent::Paste(data) => poll_tx.send(AppEvent::Paste(data)),
                        // Mouse scroll and clicks are only emitted when mouse
                        // capture is active. Standalone chat enables it by
                        // default because alternate-screen TUIs do not expose
                        // native terminal scrollback; `/mouse off` restores
                        // native selection/copy.
                        CtEvent::Mouse(m) => match m.kind {
                            MouseEventKind::ScrollUp => poll_tx.send(AppEvent::Scroll { up: true }),
                            MouseEventKind::ScrollDown => {
                                poll_tx.send(AppEvent::Scroll { up: false })
                            }
                            MouseEventKind::Down(MouseButton::Left) => {
                                poll_tx.send(AppEvent::MouseClick {
                                    x: m.column,
                                    y: m.row,
                                })
                            }
                            _ => Ok(()),
                        },
                        _ => Ok(()),
                    };
                    if sent.is_err() {
                        break;
                    }
                }
            } else {
                // No event within tick_rate → send tick for spinner animations
                if poll_tx.send(AppEvent::Tick).is_err() {
                    break;
                }
            }
        }
    });

    (tx, rx)
}

// ── Original spawn functions ────────────────────────────────────────────────

/// Detect daemon in a background thread (non-blocking).
pub fn spawn_daemon_detect(tx: mpsc::Sender<AppEvent>) {
    std::thread::spawn(move || {
        let url = crate::find_daemon();
        let mut agent_count = 0u64;

        if let Some(ref u) = url {
            if let Ok(client) = reqwest::blocking::Client::builder()
                .timeout(Duration::from_secs(2))
                .default_headers(crate::daemon_auth_headers())
                .build()
            {
                if let Ok(resp) = client.get(format!("{u}/api/status")).send() {
                    if let Ok(body) = resp.json::<serde_json::Value>() {
                        agent_count = body["agent_count"].as_u64().unwrap_or(0);
                    }
                }
            }
        }

        let _ = tx.send(AppEvent::DaemonDetected { url, agent_count });
    });
}

/// Spawn a background thread that boots the kernel.
/// Phase O.3: subscribe to the daemon's `/api/memory/events` SSE
/// endpoint and forward each memory event into the chat. Mirror of
/// `spawn_memory_subscriber` but for the daemon backend. Stops silently
/// on connection close — caller re-spawns on reconnect if needed.
pub fn spawn_daemon_memory_subscriber(base_url: String, tx: mpsc::Sender<AppEvent>) {
    std::thread::spawn(move || {
        use std::io::{BufRead, BufReader, Read};

        let client = match reqwest::blocking::Client::builder()
            .timeout(Duration::from_secs(0)) // no read timeout — long-lived SSE
            .default_headers(crate::daemon_auth_headers())
            .build()
        {
            Ok(c) => c,
            Err(_) => return,
        };

        let url = format!("{base_url}/api/memory/events");
        let resp = match client
            .get(&url)
            .header("Accept", "text/event-stream")
            .send()
        {
            Ok(r) if r.status().is_success() => r,
            _ => return,
        };

        struct RespReader(reqwest::blocking::Response);
        impl Read for RespReader {
            fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
                self.0.read(buf)
            }
        }

        let reader = BufReader::new(RespReader(resp));
        let mut event_kind: String = String::new();
        for line in reader.lines() {
            let Ok(line) = line else { return };
            if line.is_empty() {
                event_kind.clear();
                continue;
            }
            if let Some(rest) = line.strip_prefix("event:") {
                event_kind = rest.trim().to_string();
                continue;
            }
            if let Some(data) = line.strip_prefix("data: ") {
                let Ok(v) = serde_json::from_str::<serde_json::Value>(data) else {
                    continue;
                };
                let Some(ev) = memory_event_from_json(&event_kind, &v) else {
                    continue;
                };
                if tx.send(ev).is_err() {
                    return;
                }
            }
        }
    });
}

/// Phase O.2: subscribe to the in-process kernel's broadcast event bus
/// and forward every memory feedback event to the TUI. Spawned once at
/// `handle_kernel_ready`.
/// Silent on Lagged() — under heavy load we may drop a few notifications,
/// the underlying memory_writes table is the source of truth anyway.
pub fn spawn_memory_subscriber(
    kernel: std::sync::Arc<captain_kernel::CaptainKernel>,
    tx: mpsc::Sender<AppEvent>,
) {
    use captain_types::event::{ChatStreamEvent, EventPayload, LifecycleEvent};
    // The TUI run loop is plain std::thread (no ambient Tokio runtime),
    // so we spin up a dedicated current-thread runtime here. broadcast::Receiver::recv
    // is async and needs a reactor; without it the call panics with
    // "there is no reactor running".
    std::thread::spawn(move || {
        let rt = match tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
        {
            Ok(rt) => rt,
            Err(_) => return,
        };
        rt.block_on(async move {
            let mut rx = kernel.event_bus.subscribe_all();
            loop {
                match rx.recv().await {
                    Ok(event) => {
                        let app_ev = match event.payload {
                            EventPayload::ChatStream(stream_ev) => match stream_ev {
                                ChatStreamEvent::MemoryStored {
                                    subject,
                                    predicate,
                                    object,
                                    source,
                                    ..
                                } => AppEvent::MemoryStored {
                                    subject,
                                    predicate,
                                    object,
                                    source,
                                },
                                ChatStreamEvent::MemoryQueued {
                                    review_id,
                                    subject,
                                    predicate,
                                    object,
                                    source,
                                    ..
                                } => AppEvent::MemoryQueued {
                                    review_id,
                                    subject,
                                    predicate,
                                    object,
                                    source,
                                },
                                ChatStreamEvent::SkillProposalQueued {
                                    proposal_id,
                                    name,
                                    description,
                                    trigger_hint,
                                    confidence,
                                    family,
                                    ..
                                } => AppEvent::SkillProposalQueued {
                                    proposal_id,
                                    name,
                                    description,
                                    trigger_hint,
                                    confidence,
                                    family,
                                },
                                _ => continue,
                            },
                            EventPayload::Lifecycle(lifecycle_ev) => {
                                let (kind, agent_id, name, detail) = match lifecycle_ev {
                                    LifecycleEvent::Spawned { agent_id, name } => {
                                        ("spawned", agent_id, Some(name), None)
                                    }
                                    LifecycleEvent::Terminated { agent_id, reason } => {
                                        ("terminated", agent_id, None, Some(reason))
                                    }
                                    LifecycleEvent::Crashed { agent_id, error } => {
                                        ("crashed", agent_id, None, Some(error))
                                    }
                                    LifecycleEvent::Started { .. }
                                    | LifecycleEvent::Suspended { .. }
                                    | LifecycleEvent::Resumed { .. } => continue,
                                };
                                AppEvent::AgentLifecycle {
                                    kind: kind.to_string(),
                                    agent_id: agent_id.to_string(),
                                    name,
                                    detail,
                                }
                            }
                            EventPayload::ToolRun(run_ev) => AppEvent::ToolRunStatus {
                                run_id: run_ev.run_id,
                                tool_name: run_ev.tool_name,
                                status: run_ev.status,
                                caller_agent_id: run_ev.caller_agent_id,
                            },
                            _ => continue,
                        };
                        if tx.send(app_ev).is_err() {
                            return;
                        }
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => return,
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
                }
            }
        });
    });
}

pub fn spawn_kernel_boot(config: Option<std::path::PathBuf>, tx: mpsc::Sender<AppEvent>) {
    std::thread::spawn(move || {
        // Create a tokio runtime context so any tokio::spawn calls during
        // boot (e.g. publish_event via set_self_handle) find the reactor.
        let rt = tokio::runtime::Runtime::new().unwrap();
        let _guard = rt.enter();

        match CaptainKernel::boot(config.as_deref()) {
            Ok(k) => {
                let k = Arc::new(k);
                k.set_self_handle();
                let _ = tx.send(AppEvent::KernelReady(k));
            }
            Err(e) => {
                let _ = tx.send(AppEvent::KernelError(format!("{e}")));
            }
        }
    });
}

/// Spawn a background thread for in-process streaming.
pub fn spawn_inprocess_stream(
    kernel: Arc<CaptainKernel>,
    agent_id: AgentId,
    session_id: Option<captain_types::agent::SessionId>,
    message: String,
    tx: mpsc::Sender<AppEvent>,
) {
    std::thread::spawn(move || {
        let rt = match tokio::runtime::Runtime::new() {
            Ok(rt) => rt,
            Err(e) => {
                let _ = tx.send(AppEvent::StreamDone(Err(format!("Runtime error: {e}"))));
                return;
            }
        };

        // Enter the runtime context so tokio::spawn inside
        // send_message_streaming() finds the reactor.
        let _guard = rt.enter();

        match kernel.send_message_streaming_in_session(
            agent_id,
            &message,
            None,
            None,
            None,
            None,
            Some("cli".to_string()),
            session_id,
        ) {
            Ok((mut rx, handle, user_input_tx)) => {
                let _ = tx.send(AppEvent::StreamStarted {
                    interject_tx: user_input_tx,
                });
                rt.block_on(async {
                    while let Some(ev) = rx.recv().await {
                        if tx.send(AppEvent::Stream(ev)).is_err() {
                            return;
                        }
                    }
                    let result = handle
                        .await
                        .map_err(|e| e.to_string())
                        .and_then(|r| r.map_err(|e| e.to_string()));
                    let _ = tx.send(AppEvent::StreamDone(result));
                });
            }
            Err(e) => {
                let _ = tx.send(AppEvent::StreamDone(Err(format!("{e}"))));
            }
        }
    });
}

/// Spawn a background thread for daemon SSE streaming.
pub fn spawn_daemon_stream(
    base_url: String,
    agent_id: String,
    session_id: Option<String>,
    message: String,
    attachments: Vec<crate::tui::screens::chat::PendingAttachment>,
    tx: mpsc::Sender<AppEvent>,
) {
    std::thread::spawn(move || {
        use std::io::{BufRead, BufReader, Read};

        let client = reqwest::blocking::Client::builder()
            .timeout(Duration::from_secs(300))
            .default_headers(crate::daemon_auth_headers())
            .build()
            .unwrap();

        let url = format!("{base_url}/api/agents/{agent_id}/message/stream");
        let scoped_session = session_id.clone();
        let attachments_json: Vec<serde_json::Value> = attachments
            .iter()
            .map(|a| {
                serde_json::json!({
                    "file_id": a.file_id,
                    "filename": a.filename,
                    "content_type": a.content_type,
                })
            })
            .collect();
        let body = if attachments_json.is_empty() {
            serde_json::json!({
                "message": message,
                "session_id": session_id.clone(),
                "channel_type": "cli"
            })
        } else {
            serde_json::json!({
                "message": message,
                "session_id": session_id,
                "channel_type": "cli",
                "attachments": attachments_json,
            })
        };
        let resp = client.post(&url).json(&body).send();

        let resp = match resp {
            Ok(r) if r.status().is_success() => r,
            Ok(response) if scoped_session.is_some() => {
                let _ = tx.send(AppEvent::StreamDone(Err(format!(
                    "Session persistée refusée par le daemon: HTTP {}",
                    response.status()
                ))));
                return;
            }
            Ok(_) => {
                let fallback = daemon_fallback(&base_url, &agent_id, &message);
                let _ = tx.send(AppEvent::StreamDone(fallback));
                return;
            }
            Err(e) => {
                let detail = if e.is_connect() {
                    format!(
                        "Daemon injoignable à {base_url} ({e}). Lance `captain start` ou ouvre Tab Agents pour switcher en in-process."
                    )
                } else if e.is_timeout() {
                    format!(
                        "Timeout sur {url} ({e}). Le daemon est peut-être surchargé — réessaie ou redémarre-le."
                    )
                } else {
                    format!("Erreur réseau vers {url}: {e}")
                };
                let _ = tx.send(AppEvent::StreamDone(Err(detail)));
                return;
            }
        };

        struct RespReader(reqwest::blocking::Response);
        impl Read for RespReader {
            fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
                self.0.read(buf)
            }
        }

        let mut stream_state = DaemonStreamState::default();

        let reader = BufReader::new(RespReader(resp));
        for line in reader.lines() {
            let line = match line {
                Ok(l) => l,
                Err(_) => break,
            };
            for event in daemon_stream_events_from_sse_line(&line, &mut stream_state) {
                if tx.send(AppEvent::Stream(event)).is_err() {
                    return;
                }
            }
        }

        // Connection closed — agent loop is truly done.
        let _ = tx.send(AppEvent::StreamDone(Ok(AgentLoopResult {
            response: String::new(),
            total_usage: stream_state.total_usage(),
            iterations: 0,
            cost_usd: None,
            silent: false,
            directives: Default::default(),
            tool_calls: vec![],
        })));
    });
}

/// Blocking fallback for daemon chat (non-streaming).
fn daemon_fallback(
    base_url: &str,
    agent_id: &str,
    message: &str,
) -> Result<AgentLoopResult, String> {
    let client = reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(120))
        .default_headers(crate::daemon_auth_headers())
        .build()
        .map_err(|e| e.to_string())?;

    let resp = client
        .post(format!("{base_url}/api/agents/{agent_id}/message"))
        .json(&serde_json::json!({"message": message}))
        .send()
        .map_err(|e| e.to_string())?;

    let body: serde_json::Value = resp.json().map_err(|e| e.to_string())?;

    if let Some(response) = body.get("response").and_then(|r| r.as_str()) {
        let input_tokens = body["input_tokens"].as_u64().unwrap_or(0);
        let output_tokens = body["output_tokens"].as_u64().unwrap_or(0);
        Ok(AgentLoopResult {
            response: response.to_string(),
            total_usage: captain_types::message::TokenUsage {
                input_tokens,
                output_tokens,
                ..Default::default()
            },
            iterations: body["iterations"].as_u64().unwrap_or(0) as u32,
            cost_usd: body["cost_usd"].as_f64(),
            silent: false,
            directives: Default::default(),
            tool_calls: vec![],
        })
    } else {
        Err(body["error"]
            .as_str()
            .unwrap_or("Unknown error")
            .to_string())
    }
}

/// Spawn a background thread that spawns an agent on the daemon.
pub fn spawn_daemon_agent(base_url: String, toml_content: String, tx: mpsc::Sender<AppEvent>) {
    std::thread::spawn(move || {
        let client = reqwest::blocking::Client::builder()
            .timeout(Duration::from_secs(30))
            .default_headers(crate::daemon_auth_headers())
            .build()
            .unwrap();

        let resp = client
            .post(format!("{base_url}/api/agents"))
            .json(&serde_json::json!({"manifest_toml": toml_content}))
            .send();

        match resp {
            Ok(r) => {
                let body: serde_json::Value = r.json().unwrap_or_default();
                if let Some(id) = body.get("agent_id").and_then(|v| v.as_str()) {
                    let name = body["name"].as_str().unwrap_or("agent").to_string();
                    let api_sheet = AgentApiSpawnSheet::from_spawn_body(&body);
                    let _ = tx.send(AppEvent::AgentSpawned {
                        id: id.to_string(),
                        name,
                        api_sheet,
                    });
                } else {
                    let _ = tx.send(AppEvent::AgentSpawnError(
                        body["error"]
                            .as_str()
                            .unwrap_or("Failed to spawn agent")
                            .to_string(),
                    ));
                }
            }
            Err(e) => {
                let _ = tx.send(AppEvent::AgentSpawnError(format!("{e}")));
            }
        }
    });
}

// ── New spawn functions for tabs ────────────────────────────────────────────

/// Fetch dashboard data in background.
pub fn spawn_fetch_dashboard(backend: BackendRef, tx: mpsc::Sender<AppEvent>) {
    std::thread::spawn(move || match backend {
        BackendRef::Daemon(base_url) => {
            let client = reqwest::blocking::Client::builder()
                .timeout(Duration::from_secs(5))
                .default_headers(crate::daemon_auth_headers())
                .build()
                .unwrap_or_else(|_| reqwest::blocking::Client::new());

            if let Ok(resp) = client.get(format!("{base_url}/api/status")).send() {
                if let Ok(body) = resp.json::<serde_json::Value>() {
                    let status = StatusSnapshot::from_json(&body);
                    let _ = tx.send(AppEvent::DashboardData {
                        agent_count: status.agent_count,
                        uptime_secs: status.uptime_secs,
                        version: status.version.clone(),
                        provider: status.provider.clone(),
                        model: status.model.clone(),
                        status,
                    });
                }
            }

            // Try to fetch audit trail
            if let Ok(resp) = client.get(format!("{base_url}/api/audit/recent")).send() {
                if let Ok(body) = resp.json::<serde_json::Value>() {
                    let rows: Vec<AuditRow> = body
                        .as_array()
                        .or_else(|| body.get("entries").and_then(|entries| entries.as_array()))
                        .map(|arr| {
                            arr.iter()
                                .map(|r| AuditRow {
                                    timestamp: r["timestamp"].as_str().unwrap_or("").to_string(),
                                    agent: str_any(r, &["agent", "agent_id"])
                                        .unwrap_or("")
                                        .to_string(),
                                    action: r["action"].as_str().unwrap_or("").to_string(),
                                    detail: r["detail"].as_str().unwrap_or("").to_string(),
                                })
                                .collect()
                        })
                        .unwrap_or_default();
                    let _ = tx.send(AppEvent::AuditLoaded(rows));
                }
            }
        }
        BackendRef::InProcess(kernel) => {
            let count = kernel.registry.count() as u64;
            let status = StatusSnapshot::in_process(count, crate::cli_runtime::captain_version());
            let _ = tx.send(AppEvent::DashboardData {
                agent_count: count,
                uptime_secs: 0,
                version: status.version.clone(),
                provider: String::new(),
                model: String::new(),
                status,
            });
            // In-process mode doesn't have a REST audit endpoint yet
            let _ = tx.send(AppEvent::AuditLoaded(Vec::new()));
        }
    });
}

/// Fetch channel list in background.
pub fn spawn_fetch_channels(backend: BackendRef, tx: mpsc::Sender<AppEvent>) {
    std::thread::spawn(move || match backend {
        BackendRef::Daemon(base_url) => {
            let client = reqwest::blocking::Client::builder()
                .timeout(Duration::from_secs(5))
                .default_headers(crate::daemon_auth_headers())
                .build()
                .unwrap_or_else(|_| reqwest::blocking::Client::new());

            if let Ok(resp) = client.get(format!("{base_url}/api/channels")).send() {
                if let Ok(body) = resp.json::<serde_json::Value>() {
                    let channels: Vec<ChannelInfo> = body
                        .as_array()
                        .or_else(|| {
                            body.get("channels")
                                .and_then(|channels| channels.as_array())
                        })
                        .map(|arr| {
                            arr.iter()
                                .map(|ch| {
                                    let configured = ch["configured"].as_bool().unwrap_or(false);
                                    let status = channel_status_from_api(ch);
                                    ChannelInfo {
                                        name: ch["name"].as_str().unwrap_or("?").to_string(),
                                        display_name: ch["display_name"]
                                            .as_str()
                                            .unwrap_or(ch["name"].as_str().unwrap_or("?"))
                                            .to_string(),
                                        category: ch["category"]
                                            .as_str()
                                            .unwrap_or("messaging")
                                            .to_string(),
                                        status,
                                        env_vars: Vec::new(),
                                        enabled: ch["enabled"].as_bool().unwrap_or(configured),
                                    }
                                })
                                .collect()
                        })
                        .unwrap_or_default();
                    let _ = tx.send(AppEvent::ChannelListLoaded(channels));
                }
            }
        }
        BackendRef::InProcess(_kernel) => {
            // In-process: fall back to default channel detection
            let _ = tx.send(AppEvent::ChannelListLoaded(Vec::new()));
        }
    });
}

/// Test a channel in background.
pub fn spawn_test_channel(backend: BackendRef, channel: String, tx: mpsc::Sender<AppEvent>) {
    std::thread::spawn(move || match backend {
        BackendRef::Daemon(base_url) => {
            let client = reqwest::blocking::Client::builder()
                .timeout(Duration::from_secs(10))
                .default_headers(crate::daemon_auth_headers())
                .build()
                .unwrap_or_else(|_| reqwest::blocking::Client::new());

            match client
                .post(format!("{base_url}/api/channels/{channel}/test"))
                .send()
            {
                Ok(resp) => {
                    let success = resp.status().is_success();
                    let msg = resp
                        .json::<serde_json::Value>()
                        .ok()
                        .and_then(|b| b["message"].as_str().map(String::from))
                        .unwrap_or_else(|| {
                            if success {
                                "Test passed".to_string()
                            } else {
                                "Test failed".to_string()
                            }
                        });
                    let _ = tx.send(AppEvent::ChannelTestResult {
                        success,
                        message: msg,
                    });
                }
                Err(e) => {
                    let _ = tx.send(AppEvent::ChannelTestResult {
                        success: false,
                        message: format!("{e}"),
                    });
                }
            }
        }
        BackendRef::InProcess(_) => {
            let _ = tx.send(AppEvent::ChannelTestResult {
                success: false,
                message: "Channel test not available in in-process mode".to_string(),
            });
        }
    });
}

/// Fetch workflow list in background.
pub fn spawn_fetch_workflows(backend: BackendRef, tx: mpsc::Sender<AppEvent>) {
    std::thread::spawn(move || match backend {
        BackendRef::Daemon(base_url) => {
            let client = reqwest::blocking::Client::builder()
                .timeout(Duration::from_secs(5))
                .default_headers(crate::daemon_auth_headers())
                .build()
                .unwrap_or_else(|_| reqwest::blocking::Client::new());

            if let Ok(resp) = client.get(format!("{base_url}/api/workflows")).send() {
                if let Ok(body) = resp.json::<serde_json::Value>() {
                    let workflows: Vec<WorkflowInfo> = body
                        .as_array()
                        .map(|arr| {
                            arr.iter()
                                .map(|wf| WorkflowInfo {
                                    id: wf["id"].as_str().unwrap_or("?").to_string(),
                                    name: wf["name"].as_str().unwrap_or("?").to_string(),
                                    steps: wf["steps"].as_u64().unwrap_or(0) as usize,
                                    created: str_any(wf, &["created", "created_at"])
                                        .unwrap_or("")
                                        .to_string(),
                                })
                                .collect()
                        })
                        .unwrap_or_default();
                    let _ = tx.send(AppEvent::WorkflowListLoaded(workflows));
                }
            }
        }
        BackendRef::InProcess(_kernel) => {
            // Workflows in in-process mode - return empty for now
            let _ = tx.send(AppEvent::WorkflowListLoaded(Vec::new()));
        }
    });
}

/// Fetch workflow runs in background.
pub fn spawn_fetch_workflow_runs(
    backend: BackendRef,
    workflow_id: String,
    tx: mpsc::Sender<AppEvent>,
) {
    std::thread::spawn(move || match backend {
        BackendRef::Daemon(base_url) => {
            let client = reqwest::blocking::Client::builder()
                .timeout(Duration::from_secs(5))
                .default_headers(crate::daemon_auth_headers())
                .build()
                .unwrap_or_else(|_| reqwest::blocking::Client::new());

            if let Ok(resp) = client
                .get(format!("{base_url}/api/workflows/{workflow_id}/runs"))
                .send()
            {
                if let Ok(body) = resp.json::<serde_json::Value>() {
                    let runs: Vec<WorkflowRun> = body
                        .as_array()
                        .map(|arr| {
                            arr.iter()
                                .map(|r| WorkflowRun {
                                    id: r["id"].as_str().unwrap_or("?").to_string(),
                                    state: r["state"].as_str().unwrap_or("?").to_string(),
                                    duration: r["duration"].as_str().unwrap_or("").to_string(),
                                    output_preview: r["output"].as_str().unwrap_or("").to_string(),
                                })
                                .collect()
                        })
                        .unwrap_or_default();
                    let _ = tx.send(AppEvent::WorkflowRunsLoaded(runs));
                }
            }
        }
        BackendRef::InProcess(_) => {
            let _ = tx.send(AppEvent::WorkflowRunsLoaded(Vec::new()));
        }
    });
}

/// Run a workflow in background.
pub fn spawn_run_workflow(
    backend: BackendRef,
    workflow_id: String,
    input: String,
    tx: mpsc::Sender<AppEvent>,
) {
    std::thread::spawn(move || match backend {
        BackendRef::Daemon(base_url) => {
            let client = reqwest::blocking::Client::builder()
                .timeout(Duration::from_secs(60))
                .default_headers(crate::daemon_auth_headers())
                .build()
                .unwrap_or_else(|_| reqwest::blocking::Client::new());

            match client
                .post(format!("{base_url}/api/workflows/{workflow_id}/run"))
                .json(&serde_json::json!({"input": input}))
                .send()
            {
                Ok(resp) => {
                    let body: serde_json::Value = resp.json().unwrap_or_default();
                    let result = body["output"]
                        .as_str()
                        .unwrap_or("Workflow completed")
                        .to_string();
                    let _ = tx.send(AppEvent::WorkflowRunResult(result));
                }
                Err(e) => {
                    let _ = tx.send(AppEvent::WorkflowRunResult(format!("Error: {e}")));
                }
            }
        }
        BackendRef::InProcess(_) => {
            let _ = tx.send(AppEvent::WorkflowRunResult(
                "Workflow execution not available in in-process mode".to_string(),
            ));
        }
    });
}

/// Create a workflow in background.
pub fn spawn_create_workflow(
    backend: BackendRef,
    name: String,
    description: String,
    steps_json: String,
    tx: mpsc::Sender<AppEvent>,
) {
    std::thread::spawn(move || match backend {
        BackendRef::Daemon(base_url) => {
            let client = reqwest::blocking::Client::builder()
                .timeout(Duration::from_secs(10))
                .default_headers(crate::daemon_auth_headers())
                .build()
                .unwrap_or_else(|_| reqwest::blocking::Client::new());

            match client
                .post(format!("{base_url}/api/workflows"))
                .json(&serde_json::json!({
                    "name": name,
                    "description": description,
                    "steps": steps_json,
                }))
                .send()
            {
                Ok(resp) => {
                    let body: serde_json::Value = resp.json().unwrap_or_default();
                    let id = body["id"].as_str().unwrap_or("created").to_string();
                    let _ = tx.send(AppEvent::WorkflowCreated(id));
                }
                Err(e) => {
                    let _ = tx.send(AppEvent::FetchError(format!("Create workflow: {e}")));
                }
            }
        }
        BackendRef::InProcess(_) => {
            let _ = tx.send(AppEvent::FetchError(
                "Workflow creation not available in in-process mode".to_string(),
            ));
        }
    });
}

/// Fetch development project list in background.
pub fn spawn_fetch_projects(backend: BackendRef, tx: mpsc::Sender<AppEvent>) {
    std::thread::spawn(move || match backend {
        BackendRef::Daemon(base_url) => {
            let client = reqwest::blocking::Client::builder()
                .timeout(Duration::from_secs(10))
                .default_headers(crate::daemon_auth_headers())
                .build()
                .unwrap_or_else(|_| reqwest::blocking::Client::new());
            match client
                .get(format!("{base_url}/api/projects?include_archived=true"))
                .send()
            {
                Ok(resp) => match resp.json::<serde_json::Value>() {
                    Ok(body) => {
                        let projects = body["projects"]
                            .as_array()
                            .map(|arr| arr.iter().map(project_info_from_json).collect())
                            .unwrap_or_default();
                        let _ = tx.send(AppEvent::ProjectListLoaded(projects));
                    }
                    Err(e) => {
                        let _ = tx.send(AppEvent::FetchError(format!("Projects parse: {e}")));
                    }
                },
                Err(e) => {
                    let _ = tx.send(AppEvent::FetchError(format!("Projects load: {e}")));
                }
            }
        }
        BackendRef::InProcess(kernel) => {
            let projects = kernel
                .memory
                .project_list(true)
                .map(|rows| rows.iter().map(project_info_from_memory).collect())
                .unwrap_or_default();
            let _ = tx.send(AppEvent::ProjectListLoaded(projects));
        }
    });
}

pub fn spawn_resume_project(backend: BackendRef, id_or_slug: String, tx: mpsc::Sender<AppEvent>) {
    std::thread::spawn(move || match backend {
        BackendRef::Daemon(base_url) => {
            let client = reqwest::blocking::Client::builder()
                .timeout(Duration::from_secs(10))
                .default_headers(crate::daemon_auth_headers())
                .build()
                .unwrap_or_else(|_| reqwest::blocking::Client::new());
            match client
                .get(format!(
                    "{base_url}/api/projects/{}/resume",
                    path_escape(&id_or_slug)
                ))
                .send()
            {
                Ok(resp) => match resp.json::<serde_json::Value>() {
                    Ok(body) => {
                        let _ = tx.send(AppEvent::ProjectDetailLoaded(project_detail_from_json(
                            &body,
                        )));
                    }
                    Err(e) => {
                        let _ = tx.send(AppEvent::FetchError(format!("Project parse: {e}")));
                    }
                },
                Err(e) => {
                    let _ = tx.send(AppEvent::FetchError(format!("Project resume: {e}")));
                }
            }
        }
        BackendRef::InProcess(kernel) => {
            let project = kernel
                .memory
                .project_find_by_slug(&id_or_slug)
                .ok()
                .flatten()
                .or_else(|| kernel.memory.project_get(&id_or_slug).ok().flatten());
            if let Some(project) = project {
                let tasks = kernel
                    .memory
                    .task_list_for_project(&project.id)
                    .unwrap_or_default()
                    .iter()
                    .map(project_task_from_memory)
                    .collect();
                let goals = kernel
                    .goal_store
                    .list_for_project(&project.id, &project.slug)
                    .iter()
                    .map(project_goal_from_memory)
                    .collect();
                let checkpoint = kernel
                    .memory
                    .checkpoint_latest(&project.id)
                    .ok()
                    .flatten()
                    .map(|cp| cp.summary);
                let _ = tx.send(AppEvent::ProjectDetailLoaded(ProjectDetail {
                    project: project_info_from_memory(&project),
                    tasks,
                    goals,
                    checkpoint,
                    runtime_workers: runtime_workers_from_json(
                        project
                            .metadata
                            .pointer("/runtime")
                            .unwrap_or(&serde_json::Value::Null),
                    ),
                    runtime_events: runtime_events_from_json(
                        project
                            .metadata
                            .pointer("/runtime")
                            .unwrap_or(&serde_json::Value::Null),
                    ),
                }));
            }
        }
    });
}

#[allow(clippy::too_many_arguments)]
pub fn spawn_launch_project(
    backend: BackendRef,
    name: String,
    slug: String,
    goal: String,
    source_type: String,
    local_path: String,
    github_full_name: String,
    branch: String,
    tx: mpsc::Sender<AppEvent>,
) {
    std::thread::spawn(move || match backend {
        BackendRef::Daemon(base_url) => {
            let client = reqwest::blocking::Client::builder()
                .timeout(Duration::from_secs(180))
                .default_headers(crate::daemon_auth_headers())
                .build()
                .unwrap_or_else(|_| reqwest::blocking::Client::new());
            let payload = serde_json::json!({
                "name": if name.trim().is_empty() { serde_json::Value::Null } else { serde_json::json!(name) },
                "slug": if slug.trim().is_empty() { serde_json::Value::Null } else { serde_json::json!(slug) },
                "goal": goal,
                "source_type": source_type,
                "local_path": if local_path.trim().is_empty() { serde_json::Value::Null } else { serde_json::json!(local_path) },
                "github_full_name": if github_full_name.trim().is_empty() { serde_json::Value::Null } else { serde_json::json!(github_full_name) },
                "github_branch": if branch.trim().is_empty() { serde_json::Value::Null } else { serde_json::json!(branch) },
                "branch": if branch.trim().is_empty() { serde_json::Value::Null } else { serde_json::json!(branch) },
                "create_folder": true,
            });
            match client
                .post(format!("{base_url}/api/projects/launch"))
                .json(&payload)
                .send()
            {
                Ok(resp) => {
                    let body: serde_json::Value = resp.json().unwrap_or_default();
                    if let Some(err) = body["error"].as_str() {
                        let _ = tx.send(AppEvent::FetchError(format!("Project launch: {err}")));
                        return;
                    }
                    let _ = tx.send(AppEvent::ProjectDetailLoaded(project_detail_from_json(
                        &body,
                    )));
                    let _ = tx.send(AppEvent::ProjectMutated("project launched".to_string()));
                }
                Err(e) => {
                    let _ = tx.send(AppEvent::FetchError(format!("Project launch: {e}")));
                }
            }
        }
        BackendRef::InProcess(_) => {
            let _ = tx.send(AppEvent::FetchError(
                "Project launch requires the daemon API for workspace setup".to_string(),
            ));
        }
    });
}

pub fn spawn_project_simple_action(
    backend: BackendRef,
    method: &'static str,
    path: String,
    body: Option<serde_json::Value>,
    tx: mpsc::Sender<AppEvent>,
) {
    std::thread::spawn(move || match backend {
        BackendRef::Daemon(base_url) => {
            let client = reqwest::blocking::Client::builder()
                .timeout(Duration::from_secs(30))
                .default_headers(crate::daemon_auth_headers())
                .build()
                .unwrap_or_else(|_| reqwest::blocking::Client::new());
            let url = format!("{base_url}{path}");
            let req = match method {
                "DELETE" => client.delete(url),
                "PATCH" => client.patch(url),
                "POST" => client.post(url),
                "PUT" => client.put(url),
                _ => client.get(url),
            };
            let req = if let Some(body) = body {
                req.json(&body)
            } else {
                req
            };
            match req.send() {
                Ok(resp) => {
                    if resp.status().is_success() {
                        let _ = tx.send(AppEvent::ProjectMutated("project updated".to_string()));
                    } else {
                        let body: serde_json::Value = resp.json().unwrap_or_default();
                        let _ = tx.send(AppEvent::FetchError(format!(
                            "Project action: {}",
                            body["error"].as_str().unwrap_or("request failed")
                        )));
                    }
                }
                Err(e) => {
                    let _ = tx.send(AppEvent::FetchError(format!("Project action: {e}")));
                }
            }
        }
        BackendRef::InProcess(_) => {
            let _ = tx.send(AppEvent::FetchError(
                "Project action requires daemon mode".to_string(),
            ));
        }
    });
}

fn project_detail_from_json(body: &serde_json::Value) -> ProjectDetail {
    let project = project_info_from_json(&body["project"]);
    let runtime = body.get("runtime").or_else(|| {
        body.get("project")
            .and_then(|project| project.get("runtime"))
    });
    let tasks = body["tasks"]
        .as_array()
        .map(|arr| arr.iter().map(project_task_from_json).collect())
        .unwrap_or_default();
    let goals = body["goals"]
        .as_array()
        .map(|arr| arr.iter().map(project_goal_from_json).collect())
        .unwrap_or_default();
    let checkpoint = body
        .get("latest_checkpoint")
        .or_else(|| body.get("checkpoint"))
        .and_then(|v| {
            v.get("summary")
                .and_then(|s| s.as_str())
                .or_else(|| v.as_str())
        })
        .map(str::to_string);
    ProjectDetail {
        project,
        tasks,
        goals,
        checkpoint,
        runtime_workers: runtime.map(runtime_workers_from_json).unwrap_or_default(),
        runtime_events: runtime.map(runtime_events_from_json).unwrap_or_default(),
    }
}

fn path_escape(value: &str) -> String {
    let mut out = String::new();
    for byte in value.bytes() {
        let ch = byte as char;
        if ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_' | '.' | '~') {
            out.push(ch);
        } else {
            out.push_str(&format!("%{byte:02X}"));
        }
    }
    out
}

fn project_info_from_json(v: &serde_json::Value) -> ProjectInfo {
    ProjectInfo {
        id: v["id"].as_str().unwrap_or("").to_string(),
        name: v["name"].as_str().unwrap_or("").to_string(),
        slug: v["slug"].as_str().unwrap_or("").to_string(),
        goal: v["goal"].as_str().unwrap_or("").to_string(),
        status: v["status"].as_str().unwrap_or("").to_string(),
        lifecycle_phase: v["lifecycle_phase"]
            .as_str()
            .unwrap_or("observe")
            .to_string(),
        goal_count: v["goal_count"].as_u64().unwrap_or(0) as usize,
        active_goal_count: v["active_goal_count"].as_u64().unwrap_or(0) as usize,
        source_type: v["source_type"]
            .as_str()
            .or_else(|| v.pointer("/source/type").and_then(|x| x.as_str()))
            .unwrap_or("legacy")
            .to_string(),
        workspace_path: v["workspace_path"]
            .as_str()
            .or_else(|| v.pointer("/workspace/path").and_then(|x| x.as_str()))
            .or_else(|| v.pointer("/source/local_path").and_then(|x| x.as_str()))
            .or_else(|| v.pointer("/source/path").and_then(|x| x.as_str()))
            .unwrap_or("")
            .to_string(),
        repository: v
            .pointer("/source/full_name")
            .and_then(|x| x.as_str())
            .unwrap_or("")
            .to_string(),
        runtime_status: v
            .pointer("/runtime/status")
            .and_then(|x| x.as_str())
            .unwrap_or("ready")
            .to_string(),
        runtime_progress: v
            .pointer("/runtime/progress")
            .and_then(|x| x.as_u64())
            .unwrap_or(0),
        runtime_worker_count: v
            .pointer("/runtime/workers")
            .and_then(|x| x.as_array())
            .map(|arr| arr.len())
            .unwrap_or(0),
    }
}

fn project_task_from_json(v: &serde_json::Value) -> ProjectTask {
    ProjectTask {
        id: v["id"].as_str().unwrap_or("").to_string(),
        title: v["title"].as_str().unwrap_or("").to_string(),
        status: v["status"].as_str().unwrap_or("").to_string(),
        description: v["description"].as_str().unwrap_or("").to_string(),
    }
}

fn project_goal_from_json(v: &serde_json::Value) -> ProjectGoal {
    ProjectGoal {
        id: v["id"].as_str().unwrap_or("").to_string(),
        name: v["name"].as_str().unwrap_or("").to_string(),
        status: v["status"].as_str().unwrap_or("").to_string(),
        check_command: v["check_command"].as_str().unwrap_or("").to_string(),
        description: v["description"].as_str().unwrap_or("").to_string(),
    }
}

fn runtime_workers_from_json(v: &serde_json::Value) -> Vec<ProjectRuntimeWorker> {
    v.get("workers")
        .and_then(|workers| workers.as_array())
        .map(|workers| {
            workers
                .iter()
                .map(|worker| ProjectRuntimeWorker {
                    role: worker["role"].as_str().unwrap_or("").to_string(),
                    phase: worker["phase"].as_str().unwrap_or("").to_string(),
                    status: worker["status"].as_str().unwrap_or("").to_string(),
                    agent_id: worker["agent_id"].as_str().unwrap_or("").to_string(),
                    summary: worker["summary"].as_str().unwrap_or("").to_string(),
                })
                .collect()
        })
        .unwrap_or_default()
}

fn runtime_events_from_json(v: &serde_json::Value) -> Vec<ProjectRuntimeEvent> {
    v.get("timeline")
        .and_then(|events| events.as_array())
        .map(|events| {
            events
                .iter()
                .map(|event| ProjectRuntimeEvent {
                    title: event["title"].as_str().unwrap_or("").to_string(),
                    phase: event["phase"].as_str().unwrap_or("").to_string(),
                    status: event["status"].as_str().unwrap_or("").to_string(),
                    detail: event["detail"].as_str().unwrap_or("").to_string(),
                    actor: event["actor"].as_str().unwrap_or("").to_string(),
                })
                .collect()
        })
        .unwrap_or_default()
}

fn project_info_from_memory(p: &captain_memory::project::Project) -> ProjectInfo {
    let source = p
        .metadata
        .pointer("/launch/source")
        .or_else(|| p.metadata.get("source"));
    ProjectInfo {
        id: p.id.clone(),
        name: p.name.clone(),
        slug: p.slug.clone(),
        goal: p.goal.clone(),
        status: p.status.as_str().to_string(),
        lifecycle_phase: p
            .metadata
            .pointer("/lifecycle/current_phase")
            .or_else(|| p.metadata.pointer("/launch/lifecycle/current_phase"))
            .and_then(|v| v.as_str())
            .unwrap_or("observe")
            .to_string(),
        goal_count: 0,
        active_goal_count: 0,
        source_type: source
            .and_then(|s| s.get("type"))
            .and_then(|v| v.as_str())
            .unwrap_or("legacy")
            .to_string(),
        workspace_path: p
            .metadata
            .pointer("/launch/workspace/path")
            .and_then(|v| v.as_str())
            .or_else(|| {
                source
                    .and_then(|s| s.get("local_path"))
                    .and_then(|v| v.as_str())
            })
            .or_else(|| source.and_then(|s| s.get("path")).and_then(|v| v.as_str()))
            .unwrap_or("")
            .to_string(),
        repository: source
            .and_then(|s| s.get("full_name"))
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string(),
        runtime_status: p
            .metadata
            .pointer("/runtime/status")
            .and_then(|v| v.as_str())
            .unwrap_or("ready")
            .to_string(),
        runtime_progress: p
            .metadata
            .pointer("/runtime/progress")
            .and_then(|v| v.as_u64())
            .unwrap_or(0),
        runtime_worker_count: p
            .metadata
            .pointer("/runtime/workers")
            .and_then(|v| v.as_array())
            .map(|arr| arr.len())
            .unwrap_or(0),
    }
}

fn project_task_from_memory(t: &captain_memory::project_task::ProjectTask) -> ProjectTask {
    ProjectTask {
        id: t.id.clone(),
        title: t.title.clone(),
        status: t.status.as_str().to_string(),
        description: t.description.clone(),
    }
}

fn project_goal_from_memory(g: &captain_kernel::goals::Goal) -> ProjectGoal {
    ProjectGoal {
        id: g.id.clone(),
        name: g.name.clone(),
        status: match g.status {
            captain_kernel::goals::GoalStatus::Active => "active",
            captain_kernel::goals::GoalStatus::Paused => "paused",
            captain_kernel::goals::GoalStatus::Escalated => "escalated",
        }
        .to_string(),
        check_command: g.check_command.clone(),
        description: g.description.clone(),
    }
}

/// Fetch triggers in background.
pub fn spawn_fetch_triggers(backend: BackendRef, tx: mpsc::Sender<AppEvent>) {
    std::thread::spawn(move || match backend {
        BackendRef::Daemon(base_url) => {
            let client = reqwest::blocking::Client::builder()
                .timeout(Duration::from_secs(5))
                .default_headers(crate::daemon_auth_headers())
                .build()
                .unwrap_or_else(|_| reqwest::blocking::Client::new());

            if let Ok(resp) = client.get(format!("{base_url}/api/triggers")).send() {
                if let Ok(body) = resp.json::<serde_json::Value>() {
                    let triggers: Vec<TriggerInfo> = body
                        .as_array()
                        .map(|arr| {
                            arr.iter()
                                .map(|tr| TriggerInfo {
                                    id: tr["id"].as_str().unwrap_or("?").to_string(),
                                    agent_id: tr["agent_id"].as_str().unwrap_or("?").to_string(),
                                    pattern: compact_value(&tr["pattern"]),
                                    fires: u64_any(tr, &["fires", "fire_count"]).unwrap_or(0),
                                    enabled: tr["enabled"].as_bool().unwrap_or(true),
                                })
                                .collect()
                        })
                        .unwrap_or_default();
                    let _ = tx.send(AppEvent::TriggerListLoaded(triggers));
                }
            }
        }
        BackendRef::InProcess(_) => {
            let _ = tx.send(AppEvent::TriggerListLoaded(Vec::new()));
        }
    });
}

/// Create a trigger in background.
pub fn spawn_create_trigger(
    backend: BackendRef,
    agent_id: String,
    pattern_type: String,
    pattern_param: String,
    prompt: String,
    max_fires: u64,
    tx: mpsc::Sender<AppEvent>,
) {
    std::thread::spawn(move || match backend {
        BackendRef::Daemon(base_url) => {
            let client = reqwest::blocking::Client::builder()
                .timeout(Duration::from_secs(10))
                .default_headers(crate::daemon_auth_headers())
                .build()
                .unwrap_or_else(|_| reqwest::blocking::Client::new());

            match client
                .post(format!("{base_url}/api/triggers"))
                .json(&serde_json::json!({
                    "agent_id": agent_id,
                    "pattern_type": pattern_type,
                    "pattern_param": pattern_param,
                    "prompt": prompt,
                    "max_fires": max_fires,
                }))
                .send()
            {
                Ok(resp) => {
                    let body: serde_json::Value = resp.json().unwrap_or_default();
                    let id = body["id"].as_str().unwrap_or("created").to_string();
                    let _ = tx.send(AppEvent::TriggerCreated(id));
                }
                Err(e) => {
                    let _ = tx.send(AppEvent::FetchError(format!("Create trigger: {e}")));
                }
            }
        }
        BackendRef::InProcess(_) => {
            let _ = tx.send(AppEvent::FetchError(
                "Trigger creation not available in in-process mode".to_string(),
            ));
        }
    });
}

/// Phase-i.5: PUT /api/triggers/:id { enabled } — toggle enabled flag.
pub fn spawn_toggle_trigger(
    backend: BackendRef,
    trigger_id: String,
    enabled: bool,
    tx: mpsc::Sender<AppEvent>,
) {
    std::thread::spawn(move || match backend {
        BackendRef::Daemon(base_url) => {
            let client = daemon_client();
            let body = serde_json::json!({ "enabled": enabled });
            match client
                .put(format!("{base_url}/api/triggers/{trigger_id}"))
                .json(&body)
                .send()
            {
                Ok(r) if r.status().is_success() => {
                    let _ = tx.send(AppEvent::TriggerToggled {
                        id: trigger_id,
                        enabled,
                    });
                }
                _ => {
                    let _ = tx.send(AppEvent::FetchError(format!(
                        "Failed to toggle trigger {trigger_id}"
                    )));
                }
            }
        }
        BackendRef::InProcess(_) => {
            let _ = tx.send(AppEvent::FetchError(
                "Triggers require daemon mode".to_string(),
            ));
        }
    });
}

/// Delete a trigger in background.
pub fn spawn_delete_trigger(backend: BackendRef, trigger_id: String, tx: mpsc::Sender<AppEvent>) {
    std::thread::spawn(move || match backend {
        BackendRef::Daemon(base_url) => {
            let client = reqwest::blocking::Client::builder()
                .timeout(Duration::from_secs(5))
                .default_headers(crate::daemon_auth_headers())
                .build()
                .unwrap_or_else(|_| reqwest::blocking::Client::new());

            match client
                .delete(format!("{base_url}/api/triggers/{trigger_id}"))
                .send()
            {
                Ok(resp) if resp.status().is_success() => {
                    let _ = tx.send(AppEvent::TriggerDeleted(trigger_id));
                }
                _ => {
                    let _ = tx.send(AppEvent::FetchError(format!(
                        "Failed to delete trigger {trigger_id}"
                    )));
                }
            }
        }
        BackendRef::InProcess(_) => {
            let _ = tx.send(AppEvent::FetchError(
                "Trigger deletion not available in in-process mode".to_string(),
            ));
        }
    });
}

/// Kill an agent in background (for detail view action).
pub fn spawn_kill_agent(backend: BackendRef, agent_id: String, tx: mpsc::Sender<AppEvent>) {
    std::thread::spawn(move || match backend {
        BackendRef::Daemon(base_url) => {
            let client = reqwest::blocking::Client::builder()
                .timeout(Duration::from_secs(5))
                .default_headers(crate::daemon_auth_headers())
                .build()
                .unwrap_or_else(|_| reqwest::blocking::Client::new());

            match client
                .delete(format!("{base_url}/api/agents/{agent_id}"))
                .send()
            {
                Ok(resp) if resp.status().is_success() => {
                    let _ = tx.send(AppEvent::AgentKilled { id: agent_id });
                }
                _ => {
                    let _ = tx.send(AppEvent::AgentKillError(format!(
                        "Failed to kill agent {agent_id}"
                    )));
                }
            }
        }
        BackendRef::InProcess(kernel) => {
            // Try to parse as UUID-based AgentId
            if let Ok(uuid) = uuid::Uuid::parse_str(&agent_id) {
                let aid = AgentId(uuid);
                match kernel.kill_agent(aid) {
                    Ok(()) => {
                        let _ = tx.send(AppEvent::AgentKilled { id: agent_id });
                    }
                    Err(e) => {
                        let _ = tx.send(AppEvent::AgentKillError(format!("{e}")));
                    }
                }
            } else {
                let _ = tx.send(AppEvent::AgentKillError(format!(
                    "Invalid agent ID: {agent_id}"
                )));
            }
        }
    });
}

/// Fetch skill assignment for an agent.
pub fn spawn_fetch_agent_skills(backend: BackendRef, agent_id: String, tx: mpsc::Sender<AppEvent>) {
    std::thread::spawn(move || match backend {
        BackendRef::Daemon(base_url) => {
            let client = reqwest::blocking::Client::builder()
                .timeout(Duration::from_secs(5))
                .default_headers(crate::daemon_auth_headers())
                .build()
                .unwrap_or_else(|_| reqwest::blocking::Client::new());
            if let Ok(resp) = client
                .get(format!("{base_url}/api/agents/{agent_id}/skills"))
                .send()
            {
                if let Ok(body) = resp.json::<serde_json::Value>() {
                    let assigned: Vec<String> = body["assigned"]
                        .as_array()
                        .map(|a| {
                            a.iter()
                                .filter_map(|v| v.as_str().map(String::from))
                                .collect()
                        })
                        .unwrap_or_default();
                    let available: Vec<String> = body["available"]
                        .as_array()
                        .map(|a| {
                            a.iter()
                                .filter_map(|v| v.as_str().map(String::from))
                                .collect()
                        })
                        .unwrap_or_default();
                    let _ = tx.send(AppEvent::AgentSkillsLoaded {
                        assigned,
                        available,
                    });
                    return;
                }
            }
            let _ = tx.send(AppEvent::FetchError("Failed to fetch skills".to_string()));
        }
        BackendRef::InProcess(kernel) => {
            if let Ok(uuid) = uuid::Uuid::parse_str(&agent_id) {
                let aid = captain_types::agent::AgentId(uuid);
                let assigned = kernel
                    .registry
                    .get(aid)
                    .map(|e| e.manifest.skills.clone())
                    .unwrap_or_default();
                let available = kernel
                    .skill_registry
                    .read()
                    .unwrap_or_else(|e| e.into_inner())
                    .skill_names();
                let _ = tx.send(AppEvent::AgentSkillsLoaded {
                    assigned,
                    available,
                });
            }
        }
    });
}

/// Fetch MCP server assignment for an agent.
pub fn spawn_fetch_agent_mcp_servers(
    backend: BackendRef,
    agent_id: String,
    tx: mpsc::Sender<AppEvent>,
) {
    std::thread::spawn(move || match backend {
        BackendRef::Daemon(base_url) => {
            let client = reqwest::blocking::Client::builder()
                .timeout(Duration::from_secs(5))
                .default_headers(crate::daemon_auth_headers())
                .build()
                .unwrap_or_else(|_| reqwest::blocking::Client::new());
            if let Ok(resp) = client
                .get(format!("{base_url}/api/agents/{agent_id}/mcp_servers"))
                .send()
            {
                if let Ok(body) = resp.json::<serde_json::Value>() {
                    let assigned: Vec<String> = body["assigned"]
                        .as_array()
                        .map(|a| {
                            a.iter()
                                .filter_map(|v| v.as_str().map(String::from))
                                .collect()
                        })
                        .unwrap_or_default();
                    let available: Vec<String> = body["available"]
                        .as_array()
                        .map(|a| {
                            a.iter()
                                .filter_map(|v| v.as_str().map(String::from))
                                .collect()
                        })
                        .unwrap_or_default();
                    let _ = tx.send(AppEvent::AgentMcpServersLoaded {
                        assigned,
                        available,
                    });
                    return;
                }
            }
            let _ = tx.send(AppEvent::FetchError(
                "Failed to fetch MCP servers".to_string(),
            ));
        }
        BackendRef::InProcess(kernel) => {
            if let Ok(uuid) = uuid::Uuid::parse_str(&agent_id) {
                let aid = captain_types::agent::AgentId(uuid);
                let assigned = kernel
                    .registry
                    .get(aid)
                    .map(|e| e.manifest.mcp_servers.clone())
                    .unwrap_or_default();
                let mut available = Vec::new();
                if let Ok(mcp_tools) = kernel.mcp_tools.lock() {
                    let mut seen = std::collections::HashSet::new();
                    for tool in mcp_tools.iter() {
                        if let Some(server) = captain_runtime::mcp::extract_mcp_server(&tool.name) {
                            if seen.insert(server.to_string()) {
                                available.push(server.to_string());
                            }
                        }
                    }
                }
                let _ = tx.send(AppEvent::AgentMcpServersLoaded {
                    assigned,
                    available,
                });
            }
        }
    });
}

/// Update an agent's skills.
pub fn spawn_update_agent_skills(
    backend: BackendRef,
    agent_id: String,
    skills: Vec<String>,
    tx: mpsc::Sender<AppEvent>,
) {
    std::thread::spawn(move || match backend {
        BackendRef::Daemon(base_url) => {
            let client = reqwest::blocking::Client::builder()
                .timeout(Duration::from_secs(5))
                .default_headers(crate::daemon_auth_headers())
                .build()
                .unwrap_or_else(|_| reqwest::blocking::Client::new());
            match client
                .put(format!("{base_url}/api/agents/{agent_id}/skills"))
                .json(&serde_json::json!({"skills": skills}))
                .send()
            {
                Ok(resp) if resp.status().is_success() => {
                    let _ = tx.send(AppEvent::AgentSkillsUpdated(agent_id));
                }
                _ => {
                    let _ = tx.send(AppEvent::FetchError("Failed to update skills".to_string()));
                }
            }
        }
        BackendRef::InProcess(kernel) => {
            if let Ok(uuid) = uuid::Uuid::parse_str(&agent_id) {
                let aid = captain_types::agent::AgentId(uuid);
                match kernel.set_agent_skills(aid, skills) {
                    Ok(()) => {
                        let _ = tx.send(AppEvent::AgentSkillsUpdated(agent_id));
                    }
                    Err(e) => {
                        let _ = tx.send(AppEvent::FetchError(format!("Skills update: {e}")));
                    }
                }
            }
        }
    });
}

/// Update an agent's MCP servers.
pub fn spawn_update_agent_mcp_servers(
    backend: BackendRef,
    agent_id: String,
    servers: Vec<String>,
    tx: mpsc::Sender<AppEvent>,
) {
    std::thread::spawn(move || match backend {
        BackendRef::Daemon(base_url) => {
            let client = reqwest::blocking::Client::builder()
                .timeout(Duration::from_secs(5))
                .default_headers(crate::daemon_auth_headers())
                .build()
                .unwrap_or_else(|_| reqwest::blocking::Client::new());
            match client
                .put(format!("{base_url}/api/agents/{agent_id}/mcp_servers"))
                .json(&serde_json::json!({"mcp_servers": servers}))
                .send()
            {
                Ok(resp) if resp.status().is_success() => {
                    let _ = tx.send(AppEvent::AgentMcpServersUpdated(agent_id));
                }
                _ => {
                    let _ = tx.send(AppEvent::FetchError(
                        "Failed to update MCP servers".to_string(),
                    ));
                }
            }
        }
        BackendRef::InProcess(kernel) => {
            if let Ok(uuid) = uuid::Uuid::parse_str(&agent_id) {
                let aid = captain_types::agent::AgentId(uuid);
                match kernel.set_agent_mcp_servers(aid, servers) {
                    Ok(()) => {
                        let _ = tx.send(AppEvent::AgentMcpServersUpdated(agent_id));
                    }
                    Err(e) => {
                        let _ = tx.send(AppEvent::FetchError(format!("MCP update: {e}")));
                    }
                }
            }
        }
    });
}

// ── New screen spawn functions ───────────────────────────────────────────────

fn daemon_client() -> reqwest::blocking::Client {
    reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(5))
        .default_headers(crate::daemon_auth_headers())
        .build()
        .unwrap_or_else(|_| reqwest::blocking::Client::new())
}

fn array_payload<'a>(body: &'a serde_json::Value, key: &str) -> Option<&'a Vec<serde_json::Value>> {
    body.as_array()
        .or_else(|| body.get(key).and_then(|value| value.as_array()))
}

fn array_payload_any<'a>(
    body: &'a serde_json::Value,
    keys: &[&str],
) -> Option<&'a Vec<serde_json::Value>> {
    body.as_array().or_else(|| {
        keys.iter()
            .find_map(|key| body.get(*key).and_then(|value| value.as_array()))
    })
}

fn str_any<'a>(value: &'a serde_json::Value, keys: &[&str]) -> Option<&'a str> {
    keys.iter()
        .find_map(|key| value.get(*key).and_then(|item| item.as_str()))
}

fn u64_any(value: &serde_json::Value, keys: &[&str]) -> Option<u64> {
    keys.iter()
        .find_map(|key| value.get(*key).and_then(|item| item.as_u64()))
}

fn f64_any(value: &serde_json::Value, keys: &[&str]) -> Option<f64> {
    keys.iter()
        .find_map(|key| value.get(*key).and_then(|item| item.as_f64()))
}

fn compact_value(value: &serde_json::Value) -> String {
    value
        .as_str()
        .map(ToString::to_string)
        .unwrap_or_else(|| serde_json::to_string(value).unwrap_or_default())
}

fn source_label(value: &serde_json::Value) -> String {
    value
        .as_str()
        .map(ToString::to_string)
        .unwrap_or_else(|| str_any(value, &["type", "slug"]).unwrap_or("").to_string())
}

fn cron_schedule_label(value: &serde_json::Value) -> String {
    if let Some(text) = value.as_str() {
        return text.to_string();
    }
    if let Some(expr) = str_any(value, &["expr"]) {
        return expr.to_string();
    }
    if let Some(cron) = value.get("Cron") {
        let expr = str_any(cron, &["expr"]).unwrap_or("");
        let tz = str_any(cron, &["tz"]).unwrap_or("UTC");
        return if tz.is_empty() || tz == "UTC" {
            expr.to_string()
        } else {
            format!("{expr} ({tz})")
        };
    }
    if let Some(every) = value
        .get("Every")
        .and_then(|v| v.get("every_secs"))
        .and_then(|v| v.as_u64())
        .or_else(|| value.get("every_secs").and_then(|v| v.as_u64()))
    {
        return format!("every {every}s");
    }
    if let Some(at) = value
        .get("At")
        .and_then(|v| str_any(v, &["at"]))
        .or_else(|| str_any(value, &["at"]))
    {
        return at.to_string();
    }
    compact_value(value)
}

fn fetch_agent_name_map(
    base_url: &str,
    client: &reqwest::blocking::Client,
) -> std::collections::HashMap<String, String> {
    let mut names = std::collections::HashMap::new();
    let Ok(resp) = client.get(format!("{base_url}/api/agents")).send() else {
        return names;
    };
    let Ok(body) = resp.json::<serde_json::Value>() else {
        return names;
    };
    let Some(items) = array_payload(&body, "agents") else {
        return names;
    };
    for agent in items {
        if let (Some(id), Some(name)) = (str_any(agent, &["id"]), str_any(agent, &["name"])) {
            names.insert(id.to_string(), name.to_string());
        }
    }
    names
}

/// Fetch sessions list.
pub fn spawn_fetch_sessions(backend: BackendRef, tx: mpsc::Sender<AppEvent>) {
    std::thread::spawn(move || match list_sessions_for_backend(&backend) {
        Ok((values, agent_names)) => {
            let sessions = values
                .iter()
                .map(|value| session_info_from_value(value, &agent_names))
                .collect();
            let _ = tx.send(AppEvent::SessionsLoaded(sessions));
        }
        Err(error) => {
            let _ = tx.send(AppEvent::FetchError(error));
        }
    });
}

/// Resolve a session by full UUID, unique UUID prefix or title, then load its
/// transcript without changing the agent's global active session.
pub fn spawn_resolve_session(backend: BackendRef, selector: String, tx: mpsc::Sender<AppEvent>) {
    std::thread::spawn(move || {
        let result = list_sessions_for_backend(&backend)
            .and_then(|(sessions, _)| {
                session_runtime::resolve_session_selector(&sessions, &selector)
            })
            .and_then(|session_id| load_session_for_backend(&backend, &session_id));
        match result {
            Ok(session) => {
                let _ = tx.send(AppEvent::SessionLoaded(session));
            }
            Err(error) => {
                let _ = tx.send(AppEvent::FetchError(error));
            }
        }
    });
}

pub fn spawn_load_session(backend: BackendRef, session_id: String, tx: mpsc::Sender<AppEvent>) {
    std::thread::spawn(
        move || match load_session_for_backend(&backend, &session_id) {
            Ok(session) => {
                let _ = tx.send(AppEvent::SessionLoaded(session));
            }
            Err(error) => {
                let _ = tx.send(AppEvent::FetchError(error));
            }
        },
    );
}

fn list_sessions_for_backend(
    backend: &BackendRef,
) -> Result<
    (
        Vec<serde_json::Value>,
        std::collections::HashMap<String, String>,
    ),
    String,
> {
    match backend {
        BackendRef::Daemon(base_url) => {
            let client = daemon_client();
            let response = client
                .get(format!("{base_url}/api/sessions"))
                .send()
                .map_err(|error| error.to_string())?;
            if !response.status().is_success() {
                return Err(format!("Session list failed: HTTP {}", response.status()));
            }
            let body = response
                .json::<serde_json::Value>()
                .map_err(|error| error.to_string())?;
            Ok((
                session_runtime::session_values(&body),
                fetch_agent_name_map(base_url, &client),
            ))
        }
        BackendRef::InProcess(kernel) => {
            let sessions = kernel
                .memory
                .list_sessions()
                .map_err(|error| error.to_string())?;
            let names = kernel
                .registry
                .list()
                .into_iter()
                .map(|entry| (entry.id.to_string(), entry.name))
                .collect();
            Ok((sessions, names))
        }
    }
}

fn session_info_from_value(
    value: &serde_json::Value,
    agent_names: &std::collections::HashMap<String, String>,
) -> SessionInfo {
    let id = str_any(value, &["id", "session_id"])
        .unwrap_or_default()
        .to_string();
    let agent_id = str_any(value, &["agent_id"])
        .unwrap_or_default()
        .to_string();
    let label = str_any(value, &["label"])
        .filter(|label| !label.trim().is_empty())
        .map(ToString::to_string)
        .unwrap_or_else(|| format!("Session {}", id.get(..8).unwrap_or(&id)));
    SessionInfo {
        id,
        label,
        agent_name: str_any(value, &["agent_name"])
            .map(ToString::to_string)
            .or_else(|| agent_names.get(&agent_id).cloned())
            .unwrap_or_else(|| agent_id.clone()),
        agent_id,
        message_count: u64_any(value, &["message_count", "messages"]).unwrap_or(0),
        created: str_any(
            value,
            &["updated_at", "last_active", "created", "created_at"],
        )
        .unwrap_or_default()
        .to_string(),
    }
}

fn load_session_for_backend(
    backend: &BackendRef,
    session_id: &str,
) -> Result<LoadedSession, String> {
    let (detail, agent_name) = match backend {
        BackendRef::Daemon(base_url) => {
            let client = daemon_client();
            let response = client
                .get(format!("{base_url}/api/sessions/{session_id}"))
                .send()
                .map_err(|error| error.to_string())?;
            if !response.status().is_success() {
                return Err(format!(
                    "Impossible de relire la session {session_id}: HTTP {}",
                    response.status()
                ));
            }
            let detail = response
                .json::<serde_json::Value>()
                .map_err(|error| error.to_string())?;
            let owner = detail
                .get("agent_id")
                .and_then(serde_json::Value::as_str)
                .unwrap_or_default();
            let name = fetch_agent_name_map(base_url, &client)
                .remove(owner)
                .unwrap_or_else(|| owner.to_string());
            (detail, name)
        }
        BackendRef::InProcess(kernel) => {
            let id = session_id
                .parse::<uuid::Uuid>()
                .map(captain_types::agent::SessionId)
                .map_err(|_| "Invalid session ID".to_string())?;
            let session = kernel
                .memory
                .get_session(id)
                .map_err(|error| error.to_string())?
                .ok_or_else(|| format!("Session introuvable : {session_id}"))?;
            let name = kernel
                .registry
                .get(session.agent_id)
                .map(|entry| entry.name)
                .unwrap_or_else(|| session.agent_id.to_string());
            (session_runtime::native_session_detail(&session), name)
        }
    };
    session_runtime::loaded_session_from_detail(detail, agent_name)
}

/// Phase-h.1: fetch Learning data (committed + review + metrics) in parallel
/// into a single LearningLoaded event. Daemon-only for now.
fn committed_row_from_json(value: &serde_json::Value) -> CommittedRow {
    CommittedRow {
        id: value["id"].as_str().unwrap_or_default().to_string(),
        subject: value["subject"].as_str().unwrap_or_default().to_string(),
        predicate: value["predicate"].as_str().unwrap_or_default().to_string(),
        object: value["object"].as_str().unwrap_or_default().to_string(),
        source: value["source"].as_str().unwrap_or_default().to_string(),
        sync_status: value["sync_status"]
            .as_str()
            .unwrap_or_default()
            .to_string(),
        created_at: value["created_at"].as_i64().unwrap_or(0),
    }
}

fn committed_rows_from_payload(body: &serde_json::Value) -> Vec<CommittedRow> {
    array_payload(body, "committed")
        .map(|items| items.iter().map(committed_row_from_json).collect())
        .unwrap_or_default()
}

fn review_item_from_json(value: &serde_json::Value) -> ReviewItem {
    ReviewItem {
        id: value["id"].as_str().unwrap_or_default().to_string(),
        subject: value["subject"].as_str().unwrap_or_default().to_string(),
        predicate: value["predicate"].as_str().unwrap_or_default().to_string(),
        object: value["object"].as_str().unwrap_or_default().to_string(),
        confidence: value["confidence"].as_f64().unwrap_or(0.0),
        source: value["source"].as_str().unwrap_or_default().to_string(),
        created_at: value["created_at"].as_i64().unwrap_or(0),
    }
}

fn review_items_from_payload(body: &serde_json::Value) -> Vec<ReviewItem> {
    array_payload(body, "pending")
        .map(|items| items.iter().map(review_item_from_json).collect())
        .unwrap_or_default()
}

fn learning_metrics_from_json(value: &serde_json::Value) -> LearningMetrics {
    LearningMetrics {
        synced: value["memory_writes"]["synced"].as_u64().unwrap_or(0),
        pending: value["memory_writes"]["pending"].as_u64().unwrap_or(0),
        error: value["memory_writes"]["error"].as_u64().unwrap_or(0),
        review_queue_pending: value["review_queue_pending"].as_u64().unwrap_or(0),
        mode: value["learning_mode"].as_str().unwrap_or("").to_string(),
        enabled: value["learning_enabled"].as_bool().unwrap_or(false),
    }
}

pub fn spawn_fetch_learning(backend: BackendRef, tx: mpsc::Sender<AppEvent>) {
    std::thread::spawn(move || match backend {
        BackendRef::Daemon(base_url) => {
            let client = daemon_client();
            let committed: Vec<CommittedRow> = client
                .get(format!("{base_url}/api/learning/committed?limit=100"))
                .send()
                .ok()
                .and_then(|r| r.json::<serde_json::Value>().ok())
                .map(|body| committed_rows_from_payload(&body))
                .unwrap_or_default();
            let pending: Vec<ReviewItem> = client
                .get(format!("{base_url}/api/learning/review?limit=100"))
                .send()
                .ok()
                .and_then(|r| r.json::<serde_json::Value>().ok())
                .map(|body| review_items_from_payload(&body))
                .unwrap_or_default();
            let metrics = client
                .get(format!("{base_url}/api/learning/metrics"))
                .send()
                .ok()
                .and_then(|r| r.json::<serde_json::Value>().ok())
                .map(|body| learning_metrics_from_json(&body));
            let _ = tx.send(AppEvent::LearningLoaded {
                pending,
                committed,
                metrics,
            });
        }
        BackendRef::InProcess(_) => {
            let _ = tx.send(AppEvent::FetchError(
                "Learning data is only available in daemon mode".to_string(),
            ));
        }
    });
}

#[cfg(test)]
mod learning_fetch_tests {
    use super::*;

    #[test]
    fn learning_mapping_accepts_wrapped_payloads_and_metrics() {
        let committed = committed_rows_from_payload(&serde_json::json!({
            "committed": [
                {
                    "id": "write-1",
                    "subject": "Captain",
                    "predicate": "learned",
                    "object": "operator preference",
                    "source": "learning.session",
                    "sync_status": "synced",
                    "created_at": 42
                }
            ]
        }));
        let pending = review_items_from_payload(&serde_json::json!({
            "pending": [
                {
                    "id": "review-1",
                    "subject": "Captain",
                    "predicate": "uses",
                    "object": "approval",
                    "confidence": 0.9,
                    "source": "learning.review",
                    "created_at": 43
                }
            ]
        }));
        let metrics = learning_metrics_from_json(&serde_json::json!({
            "memory_writes": {
                "synced": 7,
                "pending": 2,
                "error": 1
            },
            "review_queue_pending": 3,
            "learning_mode": "approval",
            "learning_enabled": true
        }));

        assert_eq!(committed.len(), 1);
        assert_eq!(committed[0].id, "write-1");
        assert_eq!(committed[0].sync_status, "synced");
        assert_eq!(pending.len(), 1);
        assert_eq!(pending[0].id, "review-1");
        assert!((pending[0].confidence - 0.9).abs() < f64::EPSILON);
        assert_eq!(metrics.synced, 7);
        assert_eq!(metrics.pending, 2);
        assert_eq!(metrics.review_queue_pending, 3);
        assert_eq!(metrics.mode, "approval");
        assert!(metrics.enabled);
    }

    #[test]
    fn learning_mapping_accepts_legacy_arrays_and_missing_fields() {
        let committed = committed_rows_from_payload(&serde_json::json!([
            {
                "id": "write-1",
                "created_at": 10
            },
            {}
        ]));
        let pending = review_items_from_payload(&serde_json::json!([{}]));
        let metrics = learning_metrics_from_json(&serde_json::json!({}));

        assert_eq!(committed.len(), 2);
        assert_eq!(committed[0].created_at, 10);
        assert_eq!(committed[1].id, "");
        assert_eq!(pending.len(), 1);
        assert_eq!(pending[0].confidence, 0.0);
        assert_eq!(metrics.error, 0);
        assert_eq!(metrics.mode, "");
        assert!(!metrics.enabled);
    }
}

/// Phase-h.1: POST approve/deny decision for a review item.
pub fn spawn_decide_learning(
    backend: BackendRef,
    id: String,
    approve: bool,
    tx: mpsc::Sender<AppEvent>,
) {
    std::thread::spawn(move || match backend {
        BackendRef::Daemon(base_url) => {
            let client = daemon_client();
            let url = format!("{base_url}/api/learning/review/{id}/decide");
            let body = serde_json::json!({ "approve": approve });
            match client.post(&url).json(&body).send() {
                Ok(r) if r.status().is_success() => {
                    let _ = tx.send(AppEvent::LearningDecided {
                        id,
                        approved: approve,
                    });
                }
                Ok(r) => {
                    let _ = tx.send(AppEvent::FetchError(format!(
                        "Learning decide failed ({})",
                        r.status()
                    )));
                }
                Err(e) => {
                    let _ = tx.send(AppEvent::FetchError(format!("Learning decide: {e}")));
                }
            }
        }
        BackendRef::InProcess(_) => {
            let _ = tx.send(AppEvent::FetchError(
                "Learning decisions require daemon mode".to_string(),
            ));
        }
    });
}

/// Phase-h.2: fetch Skills Proposed data (proposals + patterns + metrics).
fn string_array_field(value: &serde_json::Value, key: &str) -> Vec<String> {
    value[key]
        .as_array()
        .map(|items| {
            items
                .iter()
                .filter_map(|item| item.as_str().map(String::from))
                .collect()
        })
        .unwrap_or_default()
}

fn skill_proposal_from_json(value: &serde_json::Value) -> Proposal {
    Proposal {
        id: value["id"].as_str().unwrap_or_default().to_string(),
        name: value["name"].as_str().unwrap_or_default().to_string(),
        description: value["description"]
            .as_str()
            .unwrap_or_default()
            .to_string(),
        trigger_hint: value["trigger_hint"]
            .as_str()
            .unwrap_or_default()
            .to_string(),
        tool_sequence: string_array_field(value, "tool_sequence"),
        confidence: value["confidence"].as_f64().unwrap_or(0.0),
        created_at: value["created_at"].as_i64().unwrap_or(0),
    }
}

fn skill_proposals_from_payload(body: &serde_json::Value) -> Vec<Proposal> {
    array_payload(body, "pending")
        .map(|items| items.iter().map(skill_proposal_from_json).collect())
        .unwrap_or_default()
}

fn skill_pattern_from_json(value: &serde_json::Value) -> Pattern {
    Pattern {
        hash: value["hash"].as_str().unwrap_or_default().to_string(),
        agent_id: value["agent_id"].as_str().unwrap_or_default().to_string(),
        tool_sequence: string_array_field(value, "tool_sequence"),
        count: value["count"].as_u64().unwrap_or(0),
        last_seen: value["last_seen"].as_i64().unwrap_or(0),
    }
}

fn skill_patterns_from_payload(body: &serde_json::Value) -> Vec<Pattern> {
    array_payload(body, "patterns")
        .map(|items| items.iter().map(skill_pattern_from_json).collect())
        .unwrap_or_default()
}

fn skills_metrics_from_json(value: &serde_json::Value) -> SkillsMetrics {
    SkillsMetrics {
        pending: value["pending"].as_u64().unwrap_or(0),
        patterns_hot: value["patterns_hot"].as_u64().unwrap_or(0),
        total_patterns: value["total_patterns"].as_u64().unwrap_or(0),
        approved: value["approved"].as_u64().unwrap_or(0),
        denied: value["denied"].as_u64().unwrap_or(0),
        mode: value["skills_mode"]
            .as_str()
            .unwrap_or_default()
            .to_string(),
        enabled: value["skills_enabled"].as_bool().unwrap_or(false),
    }
}

pub fn spawn_fetch_skills_proposed(backend: BackendRef, tx: mpsc::Sender<AppEvent>) {
    std::thread::spawn(move || match backend {
        BackendRef::Daemon(base_url) => {
            let client = daemon_client();
            let proposals: Vec<Proposal> = client
                .get(format!("{base_url}/api/skills/proposals?limit=100"))
                .send()
                .ok()
                .and_then(|r| r.json::<serde_json::Value>().ok())
                .map(|body| skill_proposals_from_payload(&body))
                .unwrap_or_default();
            let patterns: Vec<Pattern> = client
                .get(format!(
                    "{base_url}/api/skills/patterns?threshold=1&window_days=30&limit=50"
                ))
                .send()
                .ok()
                .and_then(|r| r.json::<serde_json::Value>().ok())
                .map(|body| skill_patterns_from_payload(&body))
                .unwrap_or_default();
            let metrics = client
                .get(format!("{base_url}/api/skills/metrics"))
                .send()
                .ok()
                .and_then(|r| r.json::<serde_json::Value>().ok())
                .map(|body| skills_metrics_from_json(&body));
            let _ = tx.send(AppEvent::SkillsProposedLoaded {
                proposals,
                patterns,
                metrics,
            });
        }
        BackendRef::InProcess(_) => {
            let _ = tx.send(AppEvent::FetchError(
                "Skills proposals are only available in daemon mode".to_string(),
            ));
        }
    });
}

#[cfg(test)]
mod skills_proposed_fetch_tests {
    use super::*;

    #[test]
    fn skills_proposed_mapping_accepts_wrapped_payloads() {
        let proposals = skill_proposals_from_payload(&serde_json::json!({
            "pending": [
                {
                    "id": "proposal-1",
                    "name": "Summarize release notes",
                    "description": "Create a reusable release workflow",
                    "trigger_hint": "When changelog changes",
                    "tool_sequence": ["rg", "git"],
                    "confidence": 0.875,
                    "created_at": 42
                }
            ]
        }));
        let patterns = skill_patterns_from_payload(&serde_json::json!({
            "patterns": [
                {
                    "hash": "abc123",
                    "agent_id": "agent-core",
                    "tool_sequence": ["rg", "cargo"],
                    "count": 3,
                    "last_seen": 99
                }
            ]
        }));
        let metrics = skills_metrics_from_json(&serde_json::json!({
            "pending": 2,
            "patterns_hot": 4,
            "total_patterns": 8,
            "approved": 5,
            "denied": 1,
            "skills_mode": "review",
            "skills_enabled": true
        }));

        assert_eq!(proposals.len(), 1);
        assert_eq!(proposals[0].id, "proposal-1");
        assert_eq!(proposals[0].tool_sequence, vec!["rg", "git"]);
        assert!((proposals[0].confidence - 0.875).abs() < f64::EPSILON);
        assert_eq!(patterns.len(), 1);
        assert_eq!(patterns[0].hash, "abc123");
        assert_eq!(patterns[0].tool_sequence, vec!["rg", "cargo"]);
        assert_eq!(patterns[0].count, 3);
        assert_eq!(metrics.pending, 2);
        assert_eq!(metrics.mode, "review");
        assert!(metrics.enabled);
    }

    #[test]
    fn skills_proposed_mapping_accepts_legacy_arrays_and_missing_fields() {
        let proposals = skill_proposals_from_payload(&serde_json::json!([
            {
                "id": "proposal-1",
                "tool_sequence": ["shell", 42, "git"],
                "confidence": 0.5
            },
            {}
        ]));
        let patterns = skill_patterns_from_payload(&serde_json::json!([{}]));
        let metrics = skills_metrics_from_json(&serde_json::json!({}));

        assert_eq!(proposals.len(), 2);
        assert_eq!(proposals[0].tool_sequence, vec!["shell", "git"]);
        assert_eq!(proposals[0].created_at, 0);
        assert_eq!(proposals[1].id, "");
        assert_eq!(patterns.len(), 1);
        assert_eq!(patterns[0].count, 0);
        assert_eq!(metrics.total_patterns, 0);
        assert_eq!(metrics.mode, "");
        assert!(!metrics.enabled);
    }
}

/// Phase-h.2: POST approve/deny decision for a skill proposal.
pub fn spawn_decide_skill_proposal(
    backend: BackendRef,
    id: String,
    approve: bool,
    tx: mpsc::Sender<AppEvent>,
) {
    std::thread::spawn(move || match backend {
        BackendRef::Daemon(base_url) => {
            let client = daemon_client();
            let url = format!("{base_url}/api/skills/proposals/{id}/decide");
            let body = serde_json::json!({ "approve": approve });
            match client.post(&url).json(&body).send() {
                Ok(r) if r.status().is_success() => {
                    let _ = tx.send(AppEvent::SkillProposalDecided {
                        id,
                        approved: approve,
                    });
                }
                Ok(r) => {
                    let _ = tx.send(AppEvent::FetchError(format!(
                        "Skill decide failed ({})",
                        r.status()
                    )));
                }
                Err(e) => {
                    let _ = tx.send(AppEvent::FetchError(format!("Skill decide: {e}")));
                }
            }
        }
        BackendRef::InProcess(_) => {
            let _ = tx.send(AppEvent::FetchError(
                "Skill decisions require daemon mode".to_string(),
            ));
        }
    });
}

/// Phase-h.3: fetch cron jobs list.
pub fn spawn_fetch_cron(backend: BackendRef, tx: mpsc::Sender<AppEvent>) {
    use crate::tui::screens::cron::CronJob;
    std::thread::spawn(move || match backend {
        BackendRef::Daemon(base_url) => {
            let client = daemon_client();
            let jobs: Vec<CronJob> = client
                .get(format!("{base_url}/api/cron/jobs"))
                .send()
                .ok()
                .and_then(|r| r.json::<serde_json::Value>().ok())
                .and_then(|v| {
                    v.get("jobs").and_then(|a| a.as_array()).map(|arr| {
                        arr.iter()
                            .map(|j| CronJob {
                                id: j["id"].as_str().unwrap_or_default().to_string(),
                                name: j["name"].as_str().unwrap_or_default().to_string(),
                                schedule: cron_schedule_label(&j["schedule"]),
                                enabled: j["enabled"].as_bool().unwrap_or(false),
                                last_status: j["last_status"].as_str().unwrap_or("").to_string(),
                                agent_id: j["agent_id"].as_str().unwrap_or_default().to_string(),
                                consecutive_errors: j["consecutive_errors"].as_u64().unwrap_or(0),
                            })
                            .collect()
                    })
                })
                .unwrap_or_default();
            let _ = tx.send(AppEvent::CronJobsLoaded(jobs));
        }
        BackendRef::InProcess(_) => {
            let _ = tx.send(AppEvent::FetchError(
                "Cron jobs are only managed in daemon mode".to_string(),
            ));
        }
    });
}

/// Phase-h.3: toggle enabled flag (PUT /api/cron/jobs/{id}/enable).
pub fn spawn_cron_toggle(
    backend: BackendRef,
    id: String,
    enabled: bool,
    tx: mpsc::Sender<AppEvent>,
) {
    std::thread::spawn(move || match backend {
        BackendRef::Daemon(base_url) => {
            let client = daemon_client();
            let url = format!("{base_url}/api/cron/jobs/{id}/enable");
            let body = serde_json::json!({ "enabled": enabled });
            match client.put(&url).json(&body).send() {
                Ok(r) if r.status().is_success() => {
                    let _ = tx.send(AppEvent::CronJobMutated { id, what: "toggle" });
                }
                _ => {
                    let _ = tx.send(AppEvent::FetchError("Cron toggle failed".to_string()));
                }
            }
        }
        BackendRef::InProcess(_) => {
            let _ = tx.send(AppEvent::FetchError("Cron requires daemon".to_string()));
        }
    });
}

/// Phase-h.3: fire a cron job immediately (POST /api/cron/jobs/{id}/run).
pub fn spawn_cron_run_now(backend: BackendRef, id: String, tx: mpsc::Sender<AppEvent>) {
    std::thread::spawn(move || match backend {
        BackendRef::Daemon(base_url) => {
            let client = daemon_client();
            let url = format!("{base_url}/api/cron/jobs/{id}/run");
            match client.post(&url).send() {
                Ok(r) if r.status().is_success() => {
                    let _ = tx.send(AppEvent::CronJobMutated { id, what: "run" });
                }
                _ => {
                    let _ = tx.send(AppEvent::FetchError("Cron run failed".to_string()));
                }
            }
        }
        BackendRef::InProcess(_) => {
            let _ = tx.send(AppEvent::FetchError("Cron requires daemon".to_string()));
        }
    });
}

/// Phase-h.3: delete a cron job.
pub fn spawn_cron_delete(backend: BackendRef, id: String, tx: mpsc::Sender<AppEvent>) {
    std::thread::spawn(move || match backend {
        BackendRef::Daemon(base_url) => {
            let client = daemon_client();
            let url = format!("{base_url}/api/cron/jobs/{id}");
            match client.delete(&url).send() {
                Ok(r) if r.status().is_success() => {
                    let _ = tx.send(AppEvent::CronJobMutated { id, what: "delete" });
                }
                _ => {
                    let _ = tx.send(AppEvent::FetchError("Cron delete failed".to_string()));
                }
            }
        }
        BackendRef::InProcess(_) => {
            let _ = tx.send(AppEvent::FetchError("Cron requires daemon".to_string()));
        }
    });
}

/// Phase-h.4: fetch pending approvals.
pub fn spawn_fetch_approvals(backend: BackendRef, tx: mpsc::Sender<AppEvent>) {
    use crate::tui::screens::approvals::ApprovalRequest;
    std::thread::spawn(move || match backend {
        BackendRef::Daemon(base_url) => {
            let client = daemon_client();
            let items: Vec<ApprovalRequest> = client
                .get(format!("{base_url}/api/approvals"))
                .send()
                .ok()
                .and_then(|r| r.json::<serde_json::Value>().ok())
                .and_then(|v| {
                    v.get("approvals").and_then(|a| a.as_array()).map(|arr| {
                        arr.iter()
                            .map(|a| ApprovalRequest {
                                id: a["id"].as_str().unwrap_or_default().to_string(),
                                agent_name: a["agent_name"]
                                    .as_str()
                                    .unwrap_or_default()
                                    .to_string(),
                                tool_name: a["tool_name"].as_str().unwrap_or_default().to_string(),
                                description: a["description"]
                                    .as_str()
                                    .unwrap_or_default()
                                    .to_string(),
                                action: a["action"].as_str().unwrap_or_default().to_string(),
                                risk_level: a["risk_level"].as_str().unwrap_or("low").to_string(),
                                created_at: a["created_at"].as_i64().unwrap_or(0),
                            })
                            .collect()
                    })
                })
                .unwrap_or_default();
            let _ = tx.send(AppEvent::ApprovalsLoaded(items));
        }
        BackendRef::InProcess(_) => {
            let _ = tx.send(AppEvent::FetchError(
                "Approvals require daemon mode".to_string(),
            ));
        }
    });
}

/// Phase-h.4 + Q.11.b — POST a decision to one of the 4 approval routes.
/// `decision_path` must be one of: `approve`, `approve_session`,
/// `approve_always`, `reject`. The function emits `ApprovalDecided` with
/// `approved=true` for the three approval variants and `false` for reject.
pub fn spawn_decide_approval(
    backend: BackendRef,
    id: String,
    decision_path: &'static str,
    tx: mpsc::Sender<AppEvent>,
) {
    std::thread::spawn(move || match backend {
        BackendRef::Daemon(base_url) => {
            let client = daemon_client();
            let url = format!("{base_url}/api/approvals/{id}/{decision_path}");
            let approved = decision_path != "reject";
            match client.post(&url).send() {
                Ok(r) if r.status().is_success() => {
                    let _ = tx.send(AppEvent::ApprovalDecided { id, approved });
                }
                _ => {
                    let _ = tx.send(AppEvent::FetchError(format!(
                        "Approval {decision_path} failed"
                    )));
                }
            }
        }
        BackendRef::InProcess(_) => {
            let _ = tx.send(AppEvent::FetchError("Approvals require daemon".to_string()));
        }
    });
}

/// Answer a pending ask_user question. Daemon mode POSTs to
/// `/api/agents/:id/message/answer` (T1); in-process mode has no HTTP
/// surface to hit, so it's reported as unsupported through this path — the
/// in-process case is instead handled directly via `current_stream_input_tx`
/// in `handle_chat_action`, without going through a spawned thread, since
/// the channel is a plain in-memory `mpsc::Sender` already held by the app.
pub fn spawn_answer_ask_user(
    backend: BackendRef,
    agent_id: String,
    session_id: Option<String>,
    content: String,
    tx: mpsc::Sender<AppEvent>,
) {
    std::thread::spawn(move || match backend {
        BackendRef::Daemon(base_url) => {
            let client = daemon_client();
            let url = format!("{base_url}/api/agents/{agent_id}/message/answer");
            match client
                .post(&url)
                .json(&serde_json::json!({
                    "content": content,
                    "session_id": session_id,
                }))
                .send()
            {
                Ok(r) if r.status().is_success() => {}
                _ => {
                    let _ = tx.send(AppEvent::FetchError(
                        "Reponse a la question impossible a envoyer".to_string(),
                    ));
                }
            }
        }
        BackendRef::InProcess(_) => {
            let _ = tx.send(AppEvent::FetchError(
                "ask_user (in-process) doit passer par current_stream_input_tx".to_string(),
            ));
        }
    });
}

/// Phase-j.4: spawn a background `rec` (sox) recording for `secs` seconds,
/// then send the resulting WAV path back via VoiceRecorded. Errors out cleanly
/// if `rec` isn't on PATH so the user gets a clear message instead of a hang.
pub fn spawn_record_voice(secs: u64, tx: mpsc::Sender<AppEvent>) {
    std::thread::spawn(move || {
        // 1. Locate `rec` (or fallback to `sox` with -d -t input).
        let bin = if which("rec").is_some() {
            "rec".to_string()
        } else if which("sox").is_some() {
            "sox".to_string()
        } else {
            let _ = tx.send(AppEvent::VoiceRecorded(Err(
                "sox/rec introuvable. Installer sox: `brew install sox` (macOS) ou \
                 `apt install sox` (Linux)."
                    .to_string(),
            )));
            return;
        };

        // 2. Build a unique temp file path.
        let id = uuid::Uuid::new_v4();
        let tmp = std::env::temp_dir().join(format!("captain_voice_{id}.wav"));

        // 3. Run blocking, with a hard cap so a hung mic can't hold the thread
        //    forever (rec's `trim 0 N` already caps, but defensive).
        let secs = secs.clamp(1, 60);
        let trim_arg = secs.to_string();
        let result = std::process::Command::new(&bin)
            .args(if bin == "sox" {
                vec![
                    "-d".to_string(),
                    tmp.display().to_string(),
                    "trim".to_string(),
                    "0".to_string(),
                    trim_arg.clone(),
                ]
            } else {
                vec![
                    tmp.display().to_string(),
                    "trim".to_string(),
                    "0".to_string(),
                    trim_arg.clone(),
                ]
            })
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::piped())
            .output();

        let payload = match result {
            Ok(out) if out.status.success() && tmp.exists() => Ok(tmp),
            Ok(out) => {
                let stderr = String::from_utf8_lossy(&out.stderr);
                Err(format!(
                    "Enregistrement échoué (code {:?}): {}",
                    out.status.code(),
                    stderr.lines().next().unwrap_or("")
                ))
            }
            Err(e) => Err(format!("Lancement {bin} échoué: {e}")),
        };
        let _ = tx.send(AppEvent::VoiceRecorded(payload));
    });
}

/// Cross-platform PATH lookup for a binary name.
fn which(name: &str) -> Option<std::path::PathBuf> {
    let path_var = std::env::var("PATH").ok()?;
    let separator = if cfg!(windows) { ';' } else { ':' };
    for dir in path_var.split(separator) {
        let candidate = std::path::PathBuf::from(dir).join(name);
        if candidate.exists() {
            return Some(candidate);
        }
    }
    None
}

/// Phase-i.6: light poll for a single approval matching the given agent name.
/// Used by the chat screen to surface an in-flight approval as a modal.
pub fn spawn_poll_chat_approval(
    backend: BackendRef,
    agent_name: String,
    tx: mpsc::Sender<AppEvent>,
) {
    use crate::tui::screens::approvals::ApprovalRequest;
    std::thread::spawn(move || match backend {
        BackendRef::Daemon(base_url) => {
            let client = daemon_client();
            let found = client
                .get(format!("{base_url}/api/approvals"))
                .send()
                .ok()
                .and_then(|r| r.json::<serde_json::Value>().ok())
                .and_then(|v| {
                    v.get("approvals")
                        .and_then(|a| a.as_array())
                        .and_then(|arr| {
                            arr.iter()
                                .find(|a| a["agent_name"].as_str() == Some(&agent_name))
                                .map(|a| ApprovalRequest {
                                    id: a["id"].as_str().unwrap_or_default().to_string(),
                                    agent_name: a["agent_name"]
                                        .as_str()
                                        .unwrap_or_default()
                                        .to_string(),
                                    tool_name: a["tool_name"]
                                        .as_str()
                                        .unwrap_or_default()
                                        .to_string(),
                                    description: a["description"]
                                        .as_str()
                                        .unwrap_or_default()
                                        .to_string(),
                                    action: a["action"].as_str().unwrap_or_default().to_string(),
                                    risk_level: a["risk_level"]
                                        .as_str()
                                        .unwrap_or("low")
                                        .to_string(),
                                    created_at: a["created_at"].as_i64().unwrap_or(0),
                                })
                        })
                });
            let _ = tx.send(AppEvent::ChatApprovalDetected(found));
        }
        BackendRef::InProcess(_) => {
            // Approvals only flow through the daemon today.
            let _ = tx.send(AppEvent::ChatApprovalDetected(None));
        }
    });
}

/// Phase-h.5: fetch budget snapshot (global + per-agent ranking).
pub fn spawn_fetch_budget(backend: BackendRef, tx: mpsc::Sender<AppEvent>) {
    use crate::tui::screens::budget::{AgentSpend, BudgetGlobal};
    std::thread::spawn(move || match backend {
        BackendRef::Daemon(base_url) => {
            let client = daemon_client();
            let global = client
                .get(format!("{base_url}/api/budget"))
                .send()
                .ok()
                .and_then(|r| r.json::<serde_json::Value>().ok())
                .map(|v| BudgetGlobal {
                    hourly_usd: v["hourly_spend"].as_f64().unwrap_or(0.0),
                    daily_usd: v["daily_spend"].as_f64().unwrap_or(0.0),
                    monthly_usd: v["monthly_spend"].as_f64().unwrap_or(0.0),
                    hourly_limit: v["hourly_limit"].as_f64().unwrap_or(0.0),
                    daily_limit: v["daily_limit"].as_f64().unwrap_or(0.0),
                    monthly_limit: v["monthly_limit"].as_f64().unwrap_or(0.0),
                    alert_threshold: v["alert_threshold"].as_f64().unwrap_or(0.8),
                });
            let agents: Vec<AgentSpend> = client
                .get(format!("{base_url}/api/budget/agents"))
                .send()
                .ok()
                .and_then(|r| r.json::<serde_json::Value>().ok())
                .and_then(|v| {
                    v.get("agents").and_then(|a| a.as_array()).map(|arr| {
                        arr.iter()
                            .map(|a| AgentSpend {
                                agent_id: a["agent_id"].as_str().unwrap_or_default().to_string(),
                                agent_name: a["agent_name"]
                                    .as_str()
                                    .or_else(|| a["name"].as_str())
                                    .unwrap_or_default()
                                    .to_string(),
                                hourly_usd: a["hourly_usd"].as_f64().unwrap_or(0.0),
                                daily_usd: a["daily_usd"].as_f64().unwrap_or(0.0),
                                monthly_usd: a["monthly_usd"].as_f64().unwrap_or(0.0),
                            })
                            .collect()
                    })
                })
                .unwrap_or_default();
            let _ = tx.send(AppEvent::BudgetLoaded { global, agents });
        }
        BackendRef::InProcess(_) => {
            let _ = tx.send(AppEvent::FetchError(
                "Budget requires daemon mode".to_string(),
            ));
        }
    });
}

/// Phase-i.4: BM25 + hybrid search over the graph; replaces the entity list
/// with the matched results. Uses the same GraphLoaded event to refresh state.
fn graph_id_string(value: &serde_json::Value, keys: &[&str]) -> String {
    keys.iter()
        .find_map(|key| {
            value
                .get(*key)
                .and_then(|item| item.as_str().map(String::from))
                .or_else(|| {
                    value
                        .get(*key)
                        .and_then(|item| item.as_i64().map(|id| id.to_string()))
                })
        })
        .unwrap_or_default()
}

fn graph_endpoint_id(value: &serde_json::Value, key: &str) -> String {
    value[key].as_str().map(String::from).unwrap_or_else(|| {
        value[key]
            .as_i64()
            .map(|id| format!("#{id}"))
            .unwrap_or_default()
    })
}

fn graph_stats_from_json(value: &serde_json::Value) -> GraphStats {
    GraphStats {
        entities: value["entities"].as_u64().unwrap_or(0),
        facts: value["facts"].as_u64().unwrap_or(0),
        episodes: value["episodes"].as_u64().unwrap_or(0),
    }
}

fn graph_entity_from_json(value: &serde_json::Value) -> GraphEntity {
    GraphEntity {
        id: graph_id_string(value, &["id", "entity_id"]),
        name: value["name"].as_str().unwrap_or_default().to_string(),
        kind: value["entity_type"]
            .as_str()
            .or_else(|| value["kind"].as_str())
            .or_else(|| value["type"].as_str())
            .unwrap_or_default()
            .to_string(),
        fact_count: value["fact_count"].as_u64().unwrap_or(0),
    }
}

fn graph_entities_from_payload(body: &serde_json::Value) -> Vec<GraphEntity> {
    array_payload_any(body, &["entities", "results", "hits"])
        .map(|items| items.iter().map(graph_entity_from_json).collect())
        .unwrap_or_default()
}

fn graph_fact_from_json(value: &serde_json::Value) -> GraphFact {
    GraphFact {
        subject: graph_endpoint_id(value, "source"),
        predicate: value["relation_type"]
            .as_str()
            .or_else(|| value["predicate"].as_str())
            .unwrap_or_default()
            .to_string(),
        object: value["description"]
            .as_str()
            .map(String::from)
            .filter(|description| !description.is_empty())
            .unwrap_or_else(|| graph_endpoint_id(value, "target")),
        confidence: value["confidence"].as_f64().unwrap_or(0.0),
    }
}

fn graph_facts_from_payload(body: &serde_json::Value) -> Vec<GraphFact> {
    array_payload(body, "facts")
        .map(|items| items.iter().map(graph_fact_from_json).collect())
        .unwrap_or_default()
}

pub fn spawn_search_graph(backend: BackendRef, query: String, tx: mpsc::Sender<AppEvent>) {
    std::thread::spawn(move || match backend {
        BackendRef::Daemon(base_url) => {
            let client = daemon_client();
            let entities: Vec<GraphEntity> = client
                .get(format!("{base_url}/api/graph/search"))
                .query(&[("q", query.as_str()), ("limit", "100")])
                .send()
                .ok()
                .and_then(|r| r.json::<serde_json::Value>().ok())
                .map(|body| graph_entities_from_payload(&body))
                .unwrap_or_default();
            // Reuse GraphLoaded with empty facts and a stats stub showing
            // result count, so the screen state updates atomically.
            let stats = Some(GraphStats {
                entities: entities.len() as u64,
                facts: 0,
                episodes: 0,
            });
            let _ = tx.send(AppEvent::GraphLoaded {
                stats,
                entities,
                facts: Vec::new(),
            });
        }
        BackendRef::InProcess(_) => {
            let _ = tx.send(AppEvent::FetchError(
                "Graph search requires daemon mode".to_string(),
            ));
        }
    });
}

/// Phase-h.6: fetch graph stats + entities + facts.
pub fn spawn_fetch_graph(backend: BackendRef, tx: mpsc::Sender<AppEvent>) {
    std::thread::spawn(move || match backend {
        BackendRef::Daemon(base_url) => {
            let client = daemon_client();
            let stats = client
                .get(format!("{base_url}/api/graph/stats"))
                .send()
                .ok()
                .and_then(|r| r.json::<serde_json::Value>().ok())
                .map(|body| graph_stats_from_json(&body));
            let entities: Vec<GraphEntity> = client
                .get(format!("{base_url}/api/graph/entities?limit=100"))
                .send()
                .ok()
                .and_then(|r| r.json::<serde_json::Value>().ok())
                .map(|body| graph_entities_from_payload(&body))
                .unwrap_or_default();
            let facts: Vec<GraphFact> = client
                .get(format!("{base_url}/api/graph/facts?limit=100"))
                .send()
                .ok()
                .and_then(|r| r.json::<serde_json::Value>().ok())
                .map(|body| graph_facts_from_payload(&body))
                .unwrap_or_default();
            let _ = tx.send(AppEvent::GraphLoaded {
                stats,
                entities,
                facts,
            });
        }
        BackendRef::InProcess(_) => {
            let _ = tx.send(AppEvent::FetchError(
                "Graph view requires daemon mode".to_string(),
            ));
        }
    });
}

#[cfg(test)]
mod graph_fetch_tests {
    use super::*;

    #[test]
    fn graph_fetch_mapping_accepts_stats_entities_and_facts() {
        let stats = graph_stats_from_json(&serde_json::json!({
            "entities": 7,
            "facts": 11,
            "episodes": 3
        }));
        let entities = graph_entities_from_payload(&serde_json::json!({
            "entities": [
                {
                    "id": 42,
                    "name": "Captain",
                    "entity_type": "agent",
                    "fact_count": 5
                }
            ]
        }));
        let facts = graph_facts_from_payload(&serde_json::json!({
            "facts": [
                {
                    "source": 42,
                    "relation_type": "knows",
                    "description": "Captain knows the operator",
                    "confidence": 0.75
                }
            ]
        }));

        assert_eq!(stats.entities, 7);
        assert_eq!(entities.len(), 1);
        assert_eq!(entities[0].id, "42");
        assert_eq!(entities[0].kind, "agent");
        assert_eq!(entities[0].fact_count, 5);
        assert_eq!(facts.len(), 1);
        assert_eq!(facts[0].subject, "#42");
        assert_eq!(facts[0].predicate, "knows");
        assert_eq!(facts[0].object, "Captain knows the operator");
        assert!((facts[0].confidence - 0.75).abs() < f64::EPSILON);
    }

    #[test]
    fn graph_mapping_accepts_search_hits_and_fact_target_fallback() {
        let entities = graph_entities_from_payload(&serde_json::json!({
            "hits": [
                {
                    "entity_id": 9,
                    "name": "runtime",
                    "entity_type": "system",
                    "score": 0.95
                }
            ]
        }));
        let facts = graph_facts_from_payload(&serde_json::json!([
            {
                "source": "agent",
                "target": 9,
                "predicate": "mentions",
                "description": "",
                "confidence": 0.5
            }
        ]));

        assert_eq!(entities.len(), 1);
        assert_eq!(entities[0].id, "9");
        assert_eq!(entities[0].kind, "system");
        assert_eq!(entities[0].fact_count, 0);
        assert_eq!(facts.len(), 1);
        assert_eq!(facts[0].subject, "agent");
        assert_eq!(facts[0].predicate, "mentions");
        assert_eq!(facts[0].object, "#9");
    }
}

/// Delete a session.
pub fn spawn_delete_session(backend: BackendRef, session_id: String, tx: mpsc::Sender<AppEvent>) {
    std::thread::spawn(move || match backend {
        BackendRef::Daemon(base_url) => {
            let client = daemon_client();
            match client
                .delete(format!("{base_url}/api/sessions/{session_id}"))
                .send()
            {
                Ok(resp) if resp.status().is_success() => {
                    let _ = tx.send(AppEvent::SessionDeleted(session_id));
                }
                _ => {
                    let _ = tx.send(AppEvent::FetchError(format!(
                        "Failed to delete session {session_id}"
                    )));
                }
            }
        }
        BackendRef::InProcess(_) => {
            let _ = tx.send(AppEvent::FetchError(
                "Session management not available in in-process mode".to_string(),
            ));
        }
    });
}

/// Fetch agents for memory screen agent selector.
pub fn spawn_fetch_memory_agents(backend: BackendRef, tx: mpsc::Sender<AppEvent>) {
    std::thread::spawn(move || match backend {
        BackendRef::Daemon(base_url) => {
            let client = daemon_client();
            if let Ok(resp) = client.get(format!("{base_url}/api/agents")).send() {
                if let Ok(body) = resp.json::<serde_json::Value>() {
                    let agents: Vec<AgentEntry> = body
                        .as_array()
                        .map(|arr| {
                            arr.iter()
                                .map(|a| AgentEntry {
                                    id: a["id"].as_str().unwrap_or("").to_string(),
                                    name: a["name"].as_str().unwrap_or("").to_string(),
                                })
                                .collect()
                        })
                        .unwrap_or_default();
                    let _ = tx.send(AppEvent::MemoryAgentsLoaded(agents));
                }
            }
        }
        BackendRef::InProcess(kernel) => {
            let agents: Vec<AgentEntry> = kernel
                .registry
                .list()
                .iter()
                .map(|e| AgentEntry {
                    id: format!("{}", e.id),
                    name: e.name.clone(),
                })
                .collect();
            let _ = tx.send(AppEvent::MemoryAgentsLoaded(agents));
        }
    });
}

/// Fetch KV pairs for an agent.
pub fn spawn_fetch_memory_kv(backend: BackendRef, agent_id: String, tx: mpsc::Sender<AppEvent>) {
    std::thread::spawn(move || match backend {
        BackendRef::Daemon(base_url) => {
            let client = daemon_client();
            if let Ok(resp) = client
                .get(format!("{base_url}/api/memory/agents/{agent_id}/kv"))
                .send()
            {
                if let Ok(body) = resp.json::<serde_json::Value>() {
                    let pairs: Vec<KvPair> = if let Some(obj) = body.as_object() {
                        obj.iter()
                            .map(|(k, v)| KvPair {
                                key: k.clone(),
                                value: v.as_str().unwrap_or(&v.to_string()).to_string(),
                            })
                            .collect()
                    } else {
                        Vec::new()
                    };
                    let _ = tx.send(AppEvent::MemoryKvLoaded(pairs));
                }
            }
        }
        BackendRef::InProcess(_) => {
            let _ = tx.send(AppEvent::MemoryKvLoaded(Vec::new()));
        }
    });
}

/// Save a KV pair.
pub fn spawn_save_memory_kv(
    backend: BackendRef,
    agent_id: String,
    key: String,
    value: String,
    tx: mpsc::Sender<AppEvent>,
) {
    std::thread::spawn(move || match backend {
        BackendRef::Daemon(base_url) => {
            let client = daemon_client();
            match client
                .put(format!("{base_url}/api/memory/agents/{agent_id}/kv/{key}"))
                .json(&serde_json::json!({"value": value}))
                .send()
            {
                Ok(resp) if resp.status().is_success() => {
                    let _ = tx.send(AppEvent::MemoryKvSaved { key });
                }
                _ => {
                    let _ = tx.send(AppEvent::FetchError("Failed to save KV pair".to_string()));
                }
            }
        }
        BackendRef::InProcess(_) => {
            let _ = tx.send(AppEvent::FetchError(
                "Memory KV not available in in-process mode".to_string(),
            ));
        }
    });
}

/// Delete a KV pair.
pub fn spawn_delete_memory_kv(
    backend: BackendRef,
    agent_id: String,
    key: String,
    tx: mpsc::Sender<AppEvent>,
) {
    std::thread::spawn(move || match backend {
        BackendRef::Daemon(base_url) => {
            let client = daemon_client();
            match client
                .delete(format!("{base_url}/api/memory/agents/{agent_id}/kv/{key}"))
                .send()
            {
                Ok(resp) if resp.status().is_success() => {
                    let _ = tx.send(AppEvent::MemoryKvDeleted(key));
                }
                _ => {
                    let _ = tx.send(AppEvent::FetchError("Failed to delete KV pair".to_string()));
                }
            }
        }
        BackendRef::InProcess(_) => {
            let _ = tx.send(AppEvent::FetchError(
                "Memory KV not available in in-process mode".to_string(),
            ));
        }
    });
}

/// Fetch installed skills.
pub fn spawn_fetch_skills(backend: BackendRef, tx: mpsc::Sender<AppEvent>) {
    std::thread::spawn(move || match backend {
        BackendRef::Daemon(base_url) => {
            let client = daemon_client();
            if let Ok(resp) = client.get(format!("{base_url}/api/skills")).send() {
                if let Ok(body) = resp.json::<serde_json::Value>() {
                    let skills: Vec<SkillInfo> = body
                        .as_array()
                        .map(|arr| {
                            arr.iter()
                                .map(|s| SkillInfo {
                                    name: s["name"].as_str().unwrap_or("").to_string(),
                                    runtime: s["runtime"].as_str().unwrap_or("").to_string(),
                                    source: source_label(&s["source"]),
                                    description: s["description"]
                                        .as_str()
                                        .unwrap_or("")
                                        .to_string(),
                                })
                                .collect()
                        })
                        .unwrap_or_default();
                    let _ = tx.send(AppEvent::SkillsLoaded(skills));
                }
            }
        }
        BackendRef::InProcess(_) => {
            let _ = tx.send(AppEvent::SkillsLoaded(Vec::new()));
        }
    });
}

/// Search ClawHub marketplace.
pub fn spawn_search_clawhub(backend: BackendRef, query: String, tx: mpsc::Sender<AppEvent>) {
    std::thread::spawn(move || match backend {
        BackendRef::Daemon(base_url) => {
            let client = daemon_client();
            let encoded: String = query
                .chars()
                .map(|c| {
                    if c.is_alphanumeric() || c == '-' || c == '_' || c == '.' || c == '~' {
                        c.to_string()
                    } else {
                        format!("%{:02X}", c as u32)
                    }
                })
                .collect();
            let url = format!("{base_url}/api/clawhub/search?q={encoded}");
            if let Ok(resp) = client.get(&url).send() {
                if let Ok(body) = resp.json::<serde_json::Value>() {
                    let results = parse_clawhub_results(&body);
                    let _ = tx.send(AppEvent::ClawHubLoaded(results));
                }
            }
        }
        BackendRef::InProcess(_) => {
            let _ = tx.send(AppEvent::ClawHubLoaded(Vec::new()));
        }
    });
}

/// Browse ClawHub marketplace.
pub fn spawn_browse_clawhub(backend: BackendRef, sort: String, tx: mpsc::Sender<AppEvent>) {
    std::thread::spawn(move || match backend {
        BackendRef::Daemon(base_url) => {
            let client = daemon_client();
            let url = format!("{base_url}/api/clawhub/browse?sort={sort}");
            if let Ok(resp) = client.get(&url).send() {
                if let Ok(body) = resp.json::<serde_json::Value>() {
                    let results = parse_clawhub_results(&body);
                    let _ = tx.send(AppEvent::ClawHubLoaded(results));
                }
            }
        }
        BackendRef::InProcess(_) => {
            let _ = tx.send(AppEvent::ClawHubLoaded(Vec::new()));
        }
    });
}

fn parse_clawhub_results(body: &serde_json::Value) -> Vec<ClawHubResult> {
    // API returns {"items": [...]} wrapper, fall back to bare array for compat
    let items = body
        .get("items")
        .and_then(|v| v.as_array())
        .or_else(|| body.as_array());

    items
        .map(|arr| {
            arr.iter()
                .map(|r| ClawHubResult {
                    name: r["name"].as_str().unwrap_or("").to_string(),
                    slug: r["slug"].as_str().unwrap_or("").to_string(),
                    description: r["description"].as_str().unwrap_or("").to_string(),
                    downloads: r["downloads"].as_u64().unwrap_or(0),
                    runtime: r["runtime"].as_str().unwrap_or("").to_string(),
                })
                .collect()
        })
        .unwrap_or_default()
}

/// Install a skill from ClawHub.
pub fn spawn_install_skill(backend: BackendRef, slug: String, tx: mpsc::Sender<AppEvent>) {
    std::thread::spawn(move || match backend {
        BackendRef::Daemon(base_url) => {
            let client = daemon_client();
            match client
                .post(format!("{base_url}/api/clawhub/install"))
                .json(&serde_json::json!({"slug": slug}))
                .send()
            {
                Ok(resp) if resp.status().is_success() => {
                    let _ = tx.send(AppEvent::SkillInstalled(slug));
                }
                _ => {
                    let _ = tx.send(AppEvent::FetchError(format!("Failed to install {slug}")));
                }
            }
        }
        BackendRef::InProcess(_) => {
            let _ = tx.send(AppEvent::FetchError(
                "Skill installation not available in in-process mode".to_string(),
            ));
        }
    });
}

/// Uninstall a skill.
pub fn spawn_uninstall_skill(backend: BackendRef, name: String, tx: mpsc::Sender<AppEvent>) {
    std::thread::spawn(move || match backend {
        BackendRef::Daemon(base_url) => {
            let client = daemon_client();
            match client
                .post(format!("{base_url}/api/skills/uninstall"))
                .json(&serde_json::json!({"name": name}))
                .send()
            {
                Ok(resp) if resp.status().is_success() => {
                    let _ = tx.send(AppEvent::SkillUninstalled(name));
                }
                _ => {
                    let _ = tx.send(AppEvent::FetchError(format!("Failed to uninstall {name}")));
                }
            }
        }
        BackendRef::InProcess(_) => {
            let _ = tx.send(AppEvent::FetchError(
                "Skill uninstall not available in in-process mode".to_string(),
            ));
        }
    });
}

/// Fetch MCP servers.
pub fn spawn_fetch_mcp_servers(backend: BackendRef, tx: mpsc::Sender<AppEvent>) {
    std::thread::spawn(move || match backend {
        BackendRef::Daemon(base_url) => {
            let client = daemon_client();
            if let Ok(resp) = client.get(format!("{base_url}/api/mcp/servers")).send() {
                if let Ok(body) = resp.json::<serde_json::Value>() {
                    let servers: Vec<McpServerInfo> = body
                        .as_array()
                        .map(|arr| {
                            arr.iter()
                                .map(|s| McpServerInfo {
                                    name: s["name"].as_str().unwrap_or("").to_string(),
                                    connected: s["connected"].as_bool().unwrap_or(false),
                                    tool_count: s["tool_count"].as_u64().unwrap_or(0) as usize,
                                })
                                .collect()
                        })
                        .unwrap_or_default();
                    let _ = tx.send(AppEvent::McpServersLoaded(servers));
                }
            }
        }
        BackendRef::InProcess(_) => {
            let _ = tx.send(AppEvent::McpServersLoaded(Vec::new()));
        }
    });
}

/// Fetch provider auth status for templates screen.
pub fn spawn_fetch_template_providers(backend: BackendRef, tx: mpsc::Sender<AppEvent>) {
    std::thread::spawn(move || match backend {
        BackendRef::Daemon(base_url) => {
            let client = daemon_client();
            if let Ok(resp) = client.get(format!("{base_url}/api/providers")).send() {
                if let Ok(body) = resp.json::<serde_json::Value>() {
                    // API returns { "providers": [...], "total": N }
                    let arr = body["providers"].as_array();
                    let providers: Vec<ProviderAuth> = arr
                        .map(|arr| {
                            arr.iter()
                                .map(|p| {
                                    let auth = p["auth_status"].as_str().unwrap_or("missing");
                                    ProviderAuth {
                                        name: p["id"].as_str().unwrap_or("").to_string(),
                                        configured: auth == "configured" || auth == "not_required",
                                    }
                                })
                                .collect()
                        })
                        .unwrap_or_default();
                    let _ = tx.send(AppEvent::TemplateProvidersLoaded(providers));
                }
            }
        }
        BackendRef::InProcess(_) => {
            let _ = tx.send(AppEvent::TemplateProvidersLoaded(Vec::new()));
        }
    });
}

/// Fetch security status.
pub fn spawn_fetch_security(backend: BackendRef, tx: mpsc::Sender<AppEvent>) {
    std::thread::spawn(move || match backend {
        BackendRef::Daemon(base_url) => {
            let client = daemon_client();
            if let Ok(resp) = client.get(format!("{base_url}/api/security")).send() {
                if let Ok(body) = resp.json::<serde_json::Value>() {
                    let features: Vec<SecurityFeature> = body
                        .as_array()
                        .map(|arr| {
                            arr.iter()
                                .map(|f| {
                                    use super::screens::security::SecuritySection;
                                    let section = match f["section"].as_str().unwrap_or("core") {
                                        "configurable" => SecuritySection::Configurable,
                                        "monitoring" => SecuritySection::Monitoring,
                                        _ => SecuritySection::Core,
                                    };
                                    SecurityFeature {
                                        name: f["name"].as_str().unwrap_or("").to_string(),
                                        active: f["active"].as_bool().unwrap_or(true),
                                        description: f["description"]
                                            .as_str()
                                            .unwrap_or("")
                                            .to_string(),
                                        section,
                                    }
                                })
                                .collect()
                        })
                        .unwrap_or_default();
                    if !features.is_empty() {
                        let _ = tx.send(AppEvent::SecurityLoaded(features));
                    }
                }
            }
        }
        BackendRef::InProcess(_) => {
            // Use builtin defaults (already loaded in SecurityState::new())
        }
    });
}

/// Verify audit chain.
pub fn spawn_verify_chain(backend: BackendRef, tx: mpsc::Sender<AppEvent>) {
    std::thread::spawn(move || match backend {
        BackendRef::Daemon(base_url) => {
            let client = daemon_client();
            match client.get(format!("{base_url}/api/audit/verify")).send() {
                Ok(resp) => {
                    let body: serde_json::Value = resp.json().unwrap_or_default();
                    let valid = body["valid"].as_bool().unwrap_or(false);
                    let message = body["message"]
                        .as_str()
                        .unwrap_or("Verification complete")
                        .to_string();
                    let _ = tx.send(AppEvent::SecurityChainVerified { valid, message });
                    let _ = tx.send(AppEvent::AuditChainVerified(valid));
                }
                Err(e) => {
                    let _ = tx.send(AppEvent::SecurityChainVerified {
                        valid: false,
                        message: format!("{e}"),
                    });
                }
            }
        }
        BackendRef::InProcess(_) => {
            let _ = tx.send(AppEvent::SecurityChainVerified {
                valid: true,
                message: "In-process mode: chain not applicable".to_string(),
            });
        }
    });
}

/// Fetch audit entries (for dedicated audit screen).
pub fn spawn_fetch_audit(backend: BackendRef, tx: mpsc::Sender<AppEvent>) {
    std::thread::spawn(move || match backend {
        BackendRef::Daemon(base_url) => {
            let client = daemon_client();
            if let Ok(resp) = client
                .get(format!("{base_url}/api/audit/recent?n=200"))
                .send()
            {
                if let Ok(body) = resp.json::<serde_json::Value>() {
                    let tip_hash = body["tip_hash"].as_str().unwrap_or("").to_string();
                    let entries: Vec<AuditEntry> = body
                        .as_array()
                        .or_else(|| body.get("entries").and_then(|entries| entries.as_array()))
                        .map(|arr| {
                            arr.iter()
                                .map(|e| AuditEntry {
                                    timestamp: e["timestamp"].as_str().unwrap_or("").to_string(),
                                    action: e["action"].as_str().unwrap_or("").to_string(),
                                    agent: str_any(e, &["agent", "agent_id"])
                                        .unwrap_or("")
                                        .to_string(),
                                    detail: e["detail"].as_str().unwrap_or("").to_string(),
                                    tip_hash: str_any(e, &["tip_hash", "hash"])
                                        .unwrap_or(&tip_hash)
                                        .to_string(),
                                })
                                .collect()
                        })
                        .unwrap_or_default();
                    let _ = tx.send(AppEvent::AuditEntriesLoaded(entries));
                }
            }
        }
        BackendRef::InProcess(_) => {
            let _ = tx.send(AppEvent::AuditEntriesLoaded(Vec::new()));
        }
    });
}

/// Fetch usage summary.
fn usage_summary_from_json(body: &serde_json::Value) -> UsageSummary {
    UsageSummary {
        total_input_tokens: body["total_input_tokens"].as_u64().unwrap_or(0),
        total_output_tokens: body["total_output_tokens"].as_u64().unwrap_or(0),
        total_cost_usd: body["total_cost_usd"].as_f64().unwrap_or(0.0),
        total_calls: u64_any(body, &["total_calls", "call_count"]).unwrap_or(0),
    }
}

fn model_usage_from_json(value: &serde_json::Value) -> ModelUsage {
    ModelUsage {
        model_id: str_any(value, &["model_id", "model"])
            .unwrap_or("")
            .to_string(),
        input_tokens: u64_any(value, &["input_tokens", "total_input_tokens"]).unwrap_or(0),
        output_tokens: u64_any(value, &["output_tokens", "total_output_tokens"]).unwrap_or(0),
        cost_usd: f64_any(value, &["cost_usd", "total_cost_usd"]).unwrap_or(0.0),
        calls: u64_any(value, &["calls", "call_count"]).unwrap_or(0),
    }
}

fn model_usage_from_payload(body: &serde_json::Value) -> Vec<ModelUsage> {
    array_payload(body, "models")
        .map(|items| items.iter().map(model_usage_from_json).collect())
        .unwrap_or_default()
}

fn agent_usage_from_json(value: &serde_json::Value) -> AgentUsage {
    AgentUsage {
        agent_name: str_any(value, &["agent_name", "name"])
            .unwrap_or("")
            .to_string(),
        agent_id: value["agent_id"].as_str().unwrap_or("").to_string(),
        total_tokens: u64_any(value, &["total_tokens", "tokens"]).unwrap_or(0),
        cost_usd: value["cost_usd"].as_f64().unwrap_or(0.0),
        tool_calls: value["tool_calls"].as_u64().unwrap_or(0),
    }
}

fn agent_usage_from_payload(body: &serde_json::Value) -> Vec<AgentUsage> {
    array_payload(body, "agents")
        .map(|items| items.iter().map(agent_usage_from_json).collect())
        .unwrap_or_default()
}

pub fn spawn_fetch_usage(backend: BackendRef, tx: mpsc::Sender<AppEvent>) {
    std::thread::spawn(move || match backend {
        BackendRef::Daemon(base_url) => {
            let client = daemon_client();
            // Summary
            if let Ok(resp) = client.get(format!("{base_url}/api/usage/summary")).send() {
                if let Ok(body) = resp.json::<serde_json::Value>() {
                    let _ = tx.send(AppEvent::UsageSummaryLoaded(usage_summary_from_json(&body)));
                }
            }
            // By model
            if let Ok(resp) = client.get(format!("{base_url}/api/usage/by-model")).send() {
                if let Ok(body) = resp.json::<serde_json::Value>() {
                    let _ = tx.send(AppEvent::UsageByModelLoaded(model_usage_from_payload(
                        &body,
                    )));
                }
            }
            // By agent
            if let Ok(resp) = client.get(format!("{base_url}/api/usage")).send() {
                if let Ok(body) = resp.json::<serde_json::Value>() {
                    let _ = tx.send(AppEvent::UsageByAgentLoaded(agent_usage_from_payload(
                        &body,
                    )));
                }
            }
        }
        BackendRef::InProcess(_) => {
            let _ = tx.send(AppEvent::UsageSummaryLoaded(UsageSummary::default()));
            let _ = tx.send(AppEvent::UsageByModelLoaded(Vec::new()));
            let _ = tx.send(AppEvent::UsageByAgentLoaded(Vec::new()));
        }
    });
}

#[cfg(test)]
mod usage_fetch_tests {
    use super::*;

    #[test]
    fn usage_mapping_accepts_summary_and_wrapped_collections() {
        let summary = usage_summary_from_json(&serde_json::json!({
            "total_input_tokens": 10,
            "total_output_tokens": 20,
            "total_cost_usd": 0.42,
            "call_count": 3
        }));
        let models = model_usage_from_payload(&serde_json::json!({
            "models": [
                {
                    "model": "gpt-5",
                    "total_input_tokens": 11,
                    "total_output_tokens": 22,
                    "total_cost_usd": 0.5,
                    "call_count": 4
                }
            ]
        }));
        let agents = agent_usage_from_payload(&serde_json::json!({
            "agents": [
                {
                    "agent_id": "agent-1",
                    "name": "Core",
                    "total_tokens": 33,
                    "cost_usd": 0.7,
                    "tool_calls": 5
                }
            ]
        }));

        assert_eq!(summary.total_input_tokens, 10);
        assert_eq!(summary.total_calls, 3);
        assert_eq!(models.len(), 1);
        assert_eq!(models[0].model_id, "gpt-5");
        assert_eq!(models[0].input_tokens, 11);
        assert_eq!(models[0].calls, 4);
        assert_eq!(agents.len(), 1);
        assert_eq!(agents[0].agent_name, "Core");
        assert_eq!(agents[0].total_tokens, 33);
        assert_eq!(agents[0].tool_calls, 5);
    }

    #[test]
    fn usage_mapping_accepts_legacy_arrays_and_missing_fields() {
        let summary = usage_summary_from_json(&serde_json::json!({
            "total_calls": 2
        }));
        let models = model_usage_from_payload(&serde_json::json!([
            {
                "model_id": "local",
                "input_tokens": 7,
                "output_tokens": 8,
                "cost_usd": 0.0,
                "calls": 1
            },
            {}
        ]));
        let agents = agent_usage_from_payload(&serde_json::json!([{}]));

        assert_eq!(summary.total_calls, 2);
        assert_eq!(summary.total_cost_usd, 0.0);
        assert_eq!(models.len(), 2);
        assert_eq!(models[0].model_id, "local");
        assert_eq!(models[0].output_tokens, 8);
        assert_eq!(models[1].model_id, "");
        assert_eq!(agents.len(), 1);
        assert_eq!(agents[0].agent_id, "");
        assert_eq!(agents[0].cost_usd, 0.0);
    }
}

/// Fetch settings providers.
pub fn spawn_fetch_providers(backend: BackendRef, tx: mpsc::Sender<AppEvent>) {
    std::thread::spawn(move || match backend {
        BackendRef::Daemon(base_url) => {
            let client = daemon_client();
            if let Ok(resp) = client.get(format!("{base_url}/api/providers")).send() {
                if let Ok(body) = resp.json::<serde_json::Value>() {
                    // API returns { "providers": [...], "total": N }
                    let arr = body["providers"].as_array();
                    let providers: Vec<ProviderInfo> = arr
                        .map(|arr| {
                            arr.iter()
                                .map(|p| {
                                    let auth = p["auth_status"].as_str().unwrap_or("missing");
                                    let key_required = p["key_required"].as_bool().unwrap_or(true);
                                    let configured = auth == "configured" || auth == "not_required";
                                    let is_local =
                                        p["is_local"].as_bool().unwrap_or(false) || !key_required;
                                    ProviderInfo {
                                        name: p["id"].as_str().unwrap_or("").to_string(),
                                        configured,
                                        env_var: p["api_key_env"]
                                            .as_str()
                                            .unwrap_or("")
                                            .to_string(),
                                        is_local,
                                        reachable: if is_local {
                                            p["reachable"].as_bool()
                                        } else {
                                            None
                                        },
                                        latency_ms: if is_local {
                                            p["latency_ms"].as_u64()
                                        } else {
                                            None
                                        },
                                    }
                                })
                                .collect()
                        })
                        .unwrap_or_default();
                    let _ = tx.send(AppEvent::SettingsProvidersLoaded(providers));
                }
            }
        }
        BackendRef::InProcess(_) => {
            let _ = tx.send(AppEvent::SettingsProvidersLoaded(Vec::new()));
        }
    });
}

/// Fetch settings models.
pub fn spawn_fetch_models(backend: BackendRef, tx: mpsc::Sender<AppEvent>) {
    std::thread::spawn(move || match backend {
        BackendRef::Daemon(base_url) => {
            let client = daemon_client();
            if let Ok(resp) = client.get(format!("{base_url}/api/models")).send() {
                if let Ok(body) = resp.json::<serde_json::Value>() {
                    let models: Vec<ModelInfo> = body
                        .as_array()
                        .map(|arr| {
                            arr.iter()
                                .map(|m| ModelInfo {
                                    id: m["id"].as_str().unwrap_or("").to_string(),
                                    provider: m["provider"].as_str().unwrap_or("").to_string(),
                                    tier: m["tier"].as_str().unwrap_or("").to_string(),
                                    context_window: m["context_window"].as_u64().unwrap_or(0),
                                    cost_input: m["cost_input"].as_f64().unwrap_or(0.0),
                                    cost_output: m["cost_output"].as_f64().unwrap_or(0.0),
                                })
                                .collect()
                        })
                        .unwrap_or_default();
                    let _ = tx.send(AppEvent::SettingsModelsLoaded(models));
                }
            }
        }
        BackendRef::InProcess(_) => {
            let _ = tx.send(AppEvent::SettingsModelsLoaded(Vec::new()));
        }
    });
}

/// Fetch settings tools.
pub fn spawn_fetch_tools(backend: BackendRef, tx: mpsc::Sender<AppEvent>) {
    std::thread::spawn(move || match backend {
        BackendRef::Daemon(base_url) => {
            let client = daemon_client();
            if let Ok(resp) = client.get(format!("{base_url}/api/tools")).send() {
                if let Ok(body) = resp.json::<serde_json::Value>() {
                    let tools: Vec<ToolInfo> = body
                        .as_array()
                        .map(|arr| {
                            arr.iter()
                                .map(|t| ToolInfo {
                                    name: t["name"].as_str().unwrap_or("").to_string(),
                                    description: t["description"]
                                        .as_str()
                                        .unwrap_or("")
                                        .to_string(),
                                })
                                .collect()
                        })
                        .unwrap_or_default();
                    let _ = tx.send(AppEvent::SettingsToolsLoaded(tools));
                }
            }
        }
        BackendRef::InProcess(_) => {
            let _ = tx.send(AppEvent::SettingsToolsLoaded(Vec::new()));
        }
    });
}

/// Save a provider API key.
pub fn spawn_save_provider_key(
    backend: BackendRef,
    name: String,
    api_key: String,
    tx: mpsc::Sender<AppEvent>,
) {
    std::thread::spawn(move || match backend {
        BackendRef::Daemon(base_url) => {
            let client = daemon_client();
            match client
                .post(format!("{base_url}/api/providers/{name}/key"))
                .json(&serde_json::json!({"key": api_key}))
                .send()
            {
                Ok(resp) if resp.status().is_success() => {
                    let _ = tx.send(AppEvent::ProviderKeySaved(name));
                }
                _ => {
                    let _ = tx.send(AppEvent::FetchError(format!(
                        "Failed to save key for {name}"
                    )));
                }
            }
        }
        BackendRef::InProcess(_) => {
            let _ = tx.send(AppEvent::FetchError(
                "Provider key management not available in in-process mode".to_string(),
            ));
        }
    });
}

/// Delete a provider API key.
pub fn spawn_delete_provider_key(backend: BackendRef, name: String, tx: mpsc::Sender<AppEvent>) {
    std::thread::spawn(move || match backend {
        BackendRef::Daemon(base_url) => {
            let client = daemon_client();
            match client
                .delete(format!("{base_url}/api/providers/{name}/key"))
                .send()
            {
                Ok(resp) if resp.status().is_success() => {
                    let _ = tx.send(AppEvent::ProviderKeyDeleted(name));
                }
                _ => {
                    let _ = tx.send(AppEvent::FetchError(format!(
                        "Failed to delete key for {name}"
                    )));
                }
            }
        }
        BackendRef::InProcess(_) => {
            let _ = tx.send(AppEvent::FetchError(
                "Provider key management not available in in-process mode".to_string(),
            ));
        }
    });
}

/// Test a provider connection.
pub fn spawn_test_provider(backend: BackendRef, name: String, tx: mpsc::Sender<AppEvent>) {
    std::thread::spawn(move || match backend {
        BackendRef::Daemon(base_url) => {
            let client = reqwest::blocking::Client::builder()
                .timeout(Duration::from_secs(15))
                .default_headers(crate::daemon_auth_headers())
                .build()
                .unwrap_or_else(|_| reqwest::blocking::Client::new());
            let start = std::time::Instant::now();
            match client
                .post(format!("{base_url}/api/providers/{name}/test"))
                .send()
            {
                Ok(resp) => {
                    let latency = start.elapsed().as_millis() as u64;
                    let success = resp.status().is_success();
                    let body: serde_json::Value = resp.json().unwrap_or_default();
                    let message = body["message"]
                        .as_str()
                        .unwrap_or(if success {
                            "Connection OK"
                        } else {
                            "Test failed"
                        })
                        .to_string();
                    let _ = tx.send(AppEvent::ProviderTestResult(TestResult {
                        provider: name,
                        success,
                        latency_ms: latency,
                        message,
                    }));
                }
                Err(e) => {
                    let _ = tx.send(AppEvent::ProviderTestResult(TestResult {
                        provider: name,
                        success: false,
                        latency_ms: 0,
                        message: format!("{e}"),
                    }));
                }
            }
        }
        BackendRef::InProcess(_) => {
            let _ = tx.send(AppEvent::ProviderTestResult(TestResult {
                provider: name,
                success: false,
                latency_ms: 0,
                message: "Provider test not available in in-process mode".to_string(),
            }));
        }
    });
}

/// Fetch peers.
pub fn spawn_fetch_peers(backend: BackendRef, tx: mpsc::Sender<AppEvent>) {
    std::thread::spawn(move || match backend {
        BackendRef::Daemon(base_url) => {
            let client = daemon_client();
            if let Ok(resp) = client.get(format!("{base_url}/api/peers")).send() {
                if let Ok(body) = resp.json::<serde_json::Value>() {
                    let peers: Vec<PeerInfo> = body
                        .as_array()
                        .or_else(|| body.get("peers").and_then(|peers| peers.as_array()))
                        .map(|arr| {
                            arr.iter()
                                .map(|p| PeerInfo {
                                    node_id: p["node_id"].as_str().unwrap_or("").to_string(),
                                    node_name: p["node_name"].as_str().unwrap_or("").to_string(),
                                    address: p["address"].as_str().unwrap_or("").to_string(),
                                    state: p["state"].as_str().unwrap_or("").to_string(),
                                    agent_count: p["agent_count"].as_u64().unwrap_or_else(|| {
                                        p["agents"].as_array().map(|a| a.len() as u64).unwrap_or(0)
                                    }),
                                    protocol_version: p["protocol_version"]
                                        .as_str()
                                        .unwrap_or("")
                                        .to_string(),
                                })
                                .collect()
                        })
                        .unwrap_or_default();
                    let _ = tx.send(AppEvent::PeersLoaded(peers));
                }
            }
        }
        BackendRef::InProcess(_) => {
            let _ = tx.send(AppEvent::PeersLoaded(Vec::new()));
        }
    });
}

/// Fetch log entries (uses audit endpoint, polled frequently).
pub fn spawn_fetch_logs(backend: BackendRef, tx: mpsc::Sender<AppEvent>) {
    std::thread::spawn(move || match backend {
        BackendRef::Daemon(base_url) => {
            let client = daemon_client();
            if let Ok(resp) = client
                .get(format!("{base_url}/api/audit/recent?n=200"))
                .send()
            {
                if let Ok(body) = resp.json::<serde_json::Value>() {
                    let entries: Vec<LogEntry> = body
                        .as_array()
                        .or_else(|| body.get("entries").and_then(|entries| entries.as_array()))
                        .map(|arr| {
                            arr.iter()
                                .map(|e| {
                                    let action = e["action"].as_str().unwrap_or("").to_string();
                                    let detail = e["detail"].as_str().unwrap_or("").to_string();
                                    let level =
                                        super::screens::logs::classify_level(&action, &detail);
                                    LogEntry {
                                        timestamp: e["timestamp"]
                                            .as_str()
                                            .unwrap_or("")
                                            .to_string(),
                                        level,
                                        action,
                                        detail,
                                        agent: str_any(e, &["agent", "agent_id"])
                                            .unwrap_or("")
                                            .to_string(),
                                    }
                                })
                                .collect()
                        })
                        .unwrap_or_default();
                    let _ = tx.send(AppEvent::LogsLoaded(entries));
                }
            }
        }
        BackendRef::InProcess(_) => {
            let _ = tx.send(AppEvent::LogsLoaded(Vec::new()));
        }
    });
}

// ── Hands events ────────────────────────────────────────────────────────────

/// Fetch hand definitions (marketplace).
pub fn spawn_fetch_hands(backend: BackendRef, tx: mpsc::Sender<AppEvent>) {
    std::thread::spawn(move || match backend {
        BackendRef::Daemon(base_url) => {
            let client = daemon_client();
            if let Ok(resp) = client.get(format!("{base_url}/api/hands")).send() {
                if let Ok(body) = resp.json::<serde_json::Value>() {
                    let hands: Vec<HandInfo> = body["hands"]
                        .as_array()
                        .map(|arr| {
                            arr.iter()
                                .map(|h| HandInfo {
                                    id: h["id"].as_str().unwrap_or("").to_string(),
                                    name: h["name"].as_str().unwrap_or("").to_string(),
                                    description: h["description"]
                                        .as_str()
                                        .unwrap_or("")
                                        .to_string(),
                                    category: h["category"].as_str().unwrap_or("").to_string(),
                                    icon: h["icon"].as_str().unwrap_or("").to_string(),
                                    requirements_met: h["requirements_met"]
                                        .as_bool()
                                        .unwrap_or(false),
                                })
                                .collect()
                        })
                        .unwrap_or_default();
                    let _ = tx.send(AppEvent::HandsLoaded(hands));
                }
            }
        }
        BackendRef::InProcess(kernel) => {
            let defs = kernel.hand_registry.list_definitions();
            let hands: Vec<HandInfo> = defs
                .iter()
                .map(|d| {
                    let reqs_met = kernel
                        .hand_registry
                        .check_requirements(&d.id)
                        .map(|r| r.iter().all(|(_, ok)| *ok))
                        .unwrap_or(false);
                    HandInfo {
                        id: d.id.clone(),
                        name: d.name.clone(),
                        description: d.description.clone(),
                        category: d.category.to_string(),
                        icon: d.icon.clone(),
                        requirements_met: reqs_met,
                    }
                })
                .collect();
            let _ = tx.send(AppEvent::HandsLoaded(hands));
        }
    });
}

/// Fetch active hand instances.
pub fn spawn_fetch_active_hands(backend: BackendRef, tx: mpsc::Sender<AppEvent>) {
    std::thread::spawn(move || match backend {
        BackendRef::Daemon(base_url) => {
            let client = daemon_client();
            if let Ok(resp) = client.get(format!("{base_url}/api/hands/active")).send() {
                if let Ok(body) = resp.json::<serde_json::Value>() {
                    let instances: Vec<HandInstanceInfo> = body["instances"]
                        .as_array()
                        .map(|arr| {
                            arr.iter()
                                .map(|i| HandInstanceInfo {
                                    instance_id: i["instance_id"]
                                        .as_str()
                                        .unwrap_or("")
                                        .to_string(),
                                    hand_id: i["hand_id"].as_str().unwrap_or("").to_string(),
                                    status: i["status"].as_str().unwrap_or("").to_string(),
                                    agent_name: i["agent_name"].as_str().unwrap_or("").to_string(),
                                    agent_id: i["agent_id"].as_str().unwrap_or("").to_string(),
                                    activated_at: i["activated_at"]
                                        .as_str()
                                        .unwrap_or("")
                                        .to_string(),
                                })
                                .collect()
                        })
                        .unwrap_or_default();
                    let _ = tx.send(AppEvent::ActiveHandsLoaded(instances));
                }
            }
        }
        BackendRef::InProcess(kernel) => {
            let instances: Vec<HandInstanceInfo> = kernel
                .hand_registry
                .list_instances()
                .iter()
                .map(|i| HandInstanceInfo {
                    instance_id: i.instance_id.to_string(),
                    hand_id: i.hand_id.clone(),
                    status: i.status.to_string(),
                    agent_name: i.agent_name.clone(),
                    agent_id: i.agent_id.map(|a| a.to_string()).unwrap_or_default(),
                    activated_at: i.activated_at.to_rfc3339(),
                })
                .collect();
            let _ = tx.send(AppEvent::ActiveHandsLoaded(instances));
        }
    });
}

/// Activate a hand.
pub fn spawn_activate_hand(backend: BackendRef, hand_id: String, tx: mpsc::Sender<AppEvent>) {
    std::thread::spawn(move || match backend {
        BackendRef::Daemon(base_url) => {
            let client = daemon_client();
            match client
                .post(format!("{base_url}/api/hands/{hand_id}/activate"))
                .json(&serde_json::json!({}))
                .send()
            {
                Ok(resp) if resp.status().is_success() => {
                    let _ = tx.send(AppEvent::HandActivated(hand_id));
                }
                Ok(resp) => {
                    let msg = resp
                        .json::<serde_json::Value>()
                        .ok()
                        .and_then(|b| b["error"].as_str().map(|s| s.to_string()))
                        .unwrap_or_else(|| "Activation failed".to_string());
                    let _ = tx.send(AppEvent::FetchError(msg));
                }
                Err(e) => {
                    let _ = tx.send(AppEvent::FetchError(format!("Failed to activate: {e}")));
                }
            }
        }
        BackendRef::InProcess(kernel) => {
            match kernel.activate_hand(&hand_id, std::collections::HashMap::new()) {
                Ok(_) => {
                    let _ = tx.send(AppEvent::HandActivated(hand_id));
                }
                Err(e) => {
                    let _ = tx.send(AppEvent::FetchError(format!("Activation failed: {e}")));
                }
            }
        }
    });
}

/// Deactivate a hand instance.
pub fn spawn_deactivate_hand(backend: BackendRef, instance_id: String, tx: mpsc::Sender<AppEvent>) {
    std::thread::spawn(move || match backend {
        BackendRef::Daemon(base_url) => {
            let client = daemon_client();
            match client
                .delete(format!("{base_url}/api/hands/instances/{instance_id}"))
                .send()
            {
                Ok(resp) if resp.status().is_success() => {
                    let _ = tx.send(AppEvent::HandDeactivated(instance_id));
                }
                _ => {
                    let _ = tx.send(AppEvent::FetchError(format!(
                        "Failed to deactivate {instance_id}"
                    )));
                }
            }
        }
        BackendRef::InProcess(kernel) => match uuid::Uuid::parse_str(&instance_id) {
            Ok(uuid) => match kernel.deactivate_hand(uuid) {
                Ok(()) => {
                    let _ = tx.send(AppEvent::HandDeactivated(instance_id));
                }
                Err(e) => {
                    let _ = tx.send(AppEvent::FetchError(format!("Deactivate failed: {e}")));
                }
            },
            Err(e) => {
                let _ = tx.send(AppEvent::FetchError(format!("Invalid instance ID: {e}")));
            }
        },
    });
}

/// Pause a hand instance.
pub fn spawn_pause_hand(backend: BackendRef, instance_id: String, tx: mpsc::Sender<AppEvent>) {
    std::thread::spawn(move || match backend {
        BackendRef::Daemon(base_url) => {
            let client = daemon_client();
            match client
                .post(format!(
                    "{base_url}/api/hands/instances/{instance_id}/pause"
                ))
                .send()
            {
                Ok(resp) if resp.status().is_success() => {
                    let _ = tx.send(AppEvent::HandPaused(instance_id));
                }
                _ => {
                    let _ = tx.send(AppEvent::FetchError(format!(
                        "Failed to pause {instance_id}"
                    )));
                }
            }
        }
        BackendRef::InProcess(kernel) => match uuid::Uuid::parse_str(&instance_id) {
            Ok(uuid) => match kernel.pause_hand(uuid) {
                Ok(()) => {
                    let _ = tx.send(AppEvent::HandPaused(instance_id));
                }
                Err(e) => {
                    let _ = tx.send(AppEvent::FetchError(format!("Pause failed: {e}")));
                }
            },
            Err(e) => {
                let _ = tx.send(AppEvent::FetchError(format!("Invalid instance ID: {e}")));
            }
        },
    });
}

/// Resume a hand instance.
pub fn spawn_resume_hand(backend: BackendRef, instance_id: String, tx: mpsc::Sender<AppEvent>) {
    std::thread::spawn(move || match backend {
        BackendRef::Daemon(base_url) => {
            let client = daemon_client();
            match client
                .post(format!(
                    "{base_url}/api/hands/instances/{instance_id}/resume"
                ))
                .send()
            {
                Ok(resp) if resp.status().is_success() => {
                    let _ = tx.send(AppEvent::HandResumed(instance_id));
                }
                _ => {
                    let _ = tx.send(AppEvent::FetchError(format!(
                        "Failed to resume {instance_id}"
                    )));
                }
            }
        }
        BackendRef::InProcess(kernel) => match uuid::Uuid::parse_str(&instance_id) {
            Ok(uuid) => match kernel.resume_hand(uuid) {
                Ok(()) => {
                    let _ = tx.send(AppEvent::HandResumed(instance_id));
                }
                Err(e) => {
                    let _ = tx.send(AppEvent::FetchError(format!("Resume failed: {e}")));
                }
            },
            Err(e) => {
                let _ = tx.send(AppEvent::FetchError(format!("Invalid instance ID: {e}")));
            }
        },
    });
}

// ── Extension spawn functions ───────────────────────────────────────────────

fn installed_extension_ids(body: &serde_json::Value) -> Vec<String> {
    body["installed"]
        .as_array()
        .map(|arr| {
            arr.iter()
                .filter_map(|i| i["id"].as_str().map(String::from))
                .collect()
        })
        .unwrap_or_default()
}

fn extension_info_from_api(entry: &serde_json::Value, installed_ids: &[String]) -> ExtensionInfo {
    let id = entry["id"].as_str().unwrap_or("").to_string();
    let installed = installed_ids.contains(&id);
    ExtensionInfo {
        id: id.clone(),
        name: entry["name"].as_str().unwrap_or("").to_string(),
        description: entry["description"].as_str().unwrap_or("").to_string(),
        icon: entry["icon"].as_str().unwrap_or("").to_string(),
        category: entry["category"].as_str().unwrap_or("").to_string(),
        installed,
        status: extension_install_status(installed),
        tags: entry["tags"]
            .as_array()
            .map(|t| {
                t.iter()
                    .filter_map(|v| v.as_str().map(String::from))
                    .collect()
            })
            .unwrap_or_default(),
        has_oauth: entry["has_oauth"].as_bool().unwrap_or(false),
    }
}

fn extension_infos_from_api(
    body: &serde_json::Value,
    installed_ids: &[String],
) -> Vec<ExtensionInfo> {
    body["integrations"]
        .as_array()
        .map(|arr| {
            arr.iter()
                .map(|entry| extension_info_from_api(entry, installed_ids))
                .collect()
        })
        .unwrap_or_default()
}

fn extension_info_from_template(
    template: &captain_extensions::IntegrationTemplate,
    installed: bool,
) -> ExtensionInfo {
    ExtensionInfo {
        id: template.id.clone(),
        name: template.name.clone(),
        description: template.description.clone(),
        icon: template.icon.clone(),
        category: template.category.to_string(),
        installed,
        status: extension_install_status(installed),
        tags: template.tags.clone(),
        has_oauth: template.oauth.is_some(),
    }
}

fn extension_install_status(installed: bool) -> String {
    if installed {
        "installed".to_string()
    } else {
        "available".to_string()
    }
}

/// Fetch all extensions (available + installed).
pub fn spawn_fetch_extensions(backend: BackendRef, tx: mpsc::Sender<AppEvent>) {
    std::thread::spawn(move || match backend {
        BackendRef::Daemon(base_url) => {
            let client = daemon_client();
            if let Ok(resp) = client
                .get(format!("{base_url}/api/integrations/available"))
                .send()
            {
                if let Ok(body) = resp.json::<serde_json::Value>() {
                    // Also fetch installed to merge status
                    let installed_ids = client
                        .get(format!("{base_url}/api/integrations"))
                        .send()
                        .ok()
                        .and_then(|r| r.json::<serde_json::Value>().ok())
                        .map(|body| installed_extension_ids(&body))
                        .unwrap_or_default();

                    let extensions = extension_infos_from_api(&body, &installed_ids);
                    let _ = tx.send(AppEvent::ExtensionsLoaded(extensions));
                }
            }
        }
        BackendRef::InProcess(kernel) => {
            let registry = kernel
                .extension_registry
                .read()
                .unwrap_or_else(|e| e.into_inner());
            let extensions: Vec<ExtensionInfo> = registry
                .list_templates()
                .iter()
                .map(|t| {
                    let installed = registry.is_installed(&t.id);
                    extension_info_from_template(t, installed)
                })
                .collect();
            let _ = tx.send(AppEvent::ExtensionsLoaded(extensions));
        }
    });
}

#[cfg(test)]
mod extension_fetch_tests {
    use super::*;

    #[test]
    fn extension_api_mapping_merges_installed_status_and_tags() {
        let body = serde_json::json!({
            "integrations": [
                {
                    "id": "github",
                    "name": "GitHub",
                    "description": "Issues and PRs",
                    "icon": "gh",
                    "category": "dev",
                    "tags": ["code", "review"],
                    "has_oauth": true
                },
                {
                    "id": "notion",
                    "name": "Notion"
                }
            ]
        });
        let installed = vec!["github".to_string()];

        let extensions = extension_infos_from_api(&body, &installed);

        assert_eq!(extensions.len(), 2);
        assert_eq!(extensions[0].id, "github");
        assert!(extensions[0].installed);
        assert_eq!(extensions[0].status, "installed");
        assert_eq!(extensions[0].tags, vec!["code", "review"]);
        assert!(extensions[0].has_oauth);
        assert_eq!(extensions[1].id, "notion");
        assert!(!extensions[1].installed);
        assert_eq!(extensions[1].status, "available");
    }

    #[test]
    fn installed_extension_ids_accepts_missing_or_wrapped_payloads() {
        let body = serde_json::json!({
            "installed": [
                {"id": "github"},
                {"id": "gmail"},
                {"name": "missing-id"}
            ]
        });

        assert_eq!(
            installed_extension_ids(&body),
            vec!["github".to_string(), "gmail".to_string()]
        );
        assert!(installed_extension_ids(&serde_json::json!({})).is_empty());
    }
}

/// Fetch extension health data.
pub fn spawn_fetch_extension_health(backend: BackendRef, tx: mpsc::Sender<AppEvent>) {
    std::thread::spawn(move || match backend {
        BackendRef::Daemon(base_url) => {
            let client = daemon_client();
            if let Ok(resp) = client
                .get(format!("{base_url}/api/integrations/health"))
                .send()
            {
                if let Ok(body) = resp.json::<serde_json::Value>() {
                    let entries: Vec<ExtensionHealthInfo> = body["health"]
                        .as_array()
                        .map(|arr| {
                            arr.iter()
                                .map(|h| ExtensionHealthInfo {
                                    id: h["id"].as_str().unwrap_or("").to_string(),
                                    status: h["status"].as_str().unwrap_or("").to_string(),
                                    tool_count: h["tool_count"].as_u64().unwrap_or(0) as usize,
                                    last_ok: h["last_ok"].as_str().unwrap_or("").to_string(),
                                    last_error: h["last_error"].as_str().unwrap_or("").to_string(),
                                    consecutive_failures: h["consecutive_failures"]
                                        .as_u64()
                                        .unwrap_or(0)
                                        as u32,
                                    reconnecting: h["reconnecting"].as_bool().unwrap_or(false),
                                    connected_since: h["connected_since"]
                                        .as_str()
                                        .unwrap_or("")
                                        .to_string(),
                                })
                                .collect()
                        })
                        .unwrap_or_default();
                    let _ = tx.send(AppEvent::ExtensionHealthLoaded(entries));
                }
            }
        }
        BackendRef::InProcess(kernel) => {
            let health = kernel.extension_health.all_health();
            let entries: Vec<ExtensionHealthInfo> = health
                .iter()
                .map(|h| ExtensionHealthInfo {
                    id: h.id.clone(),
                    status: h.status.to_string(),
                    tool_count: h.tool_count,
                    last_ok: h.last_ok.map(|t| t.to_rfc3339()).unwrap_or_default(),
                    last_error: h.last_error.clone().unwrap_or_default(),
                    consecutive_failures: h.consecutive_failures,
                    reconnecting: h.reconnecting,
                    connected_since: h
                        .connected_since
                        .map(|t| t.to_rfc3339())
                        .unwrap_or_default(),
                })
                .collect();
            let _ = tx.send(AppEvent::ExtensionHealthLoaded(entries));
        }
    });
}

/// Install an extension.
pub fn spawn_install_extension(backend: BackendRef, id: String, tx: mpsc::Sender<AppEvent>) {
    std::thread::spawn(move || match backend {
        BackendRef::Daemon(base_url) => {
            let client = daemon_client();
            match client
                .post(format!("{base_url}/api/integrations/add"))
                .json(&serde_json::json!({"id": id}))
                .send()
            {
                Ok(resp) if resp.status().is_success() => {
                    let _ = tx.send(AppEvent::ExtensionInstalled(id));
                }
                Ok(resp) => {
                    let body = resp.json::<serde_json::Value>().ok();
                    let err = body
                        .and_then(|b| b["error"].as_str().map(String::from))
                        .unwrap_or_else(|| format!("Failed to install {id}"));
                    let _ = tx.send(AppEvent::FetchError(err));
                }
                Err(e) => {
                    let _ = tx.send(AppEvent::FetchError(format!("Install failed: {e}")));
                }
            }
        }
        BackendRef::InProcess(_) => {
            let _ = tx.send(AppEvent::FetchError(
                "Install via in-process mode not supported — use CLI".to_string(),
            ));
        }
    });
}

/// Remove an extension.
pub fn spawn_remove_extension(backend: BackendRef, id: String, tx: mpsc::Sender<AppEvent>) {
    std::thread::spawn(move || match backend {
        BackendRef::Daemon(base_url) => {
            let client = daemon_client();
            match client
                .delete(format!("{base_url}/api/integrations/{id}"))
                .send()
            {
                Ok(resp) if resp.status().is_success() => {
                    let _ = tx.send(AppEvent::ExtensionRemoved(id));
                }
                _ => {
                    let _ = tx.send(AppEvent::FetchError(format!("Failed to remove {id}")));
                }
            }
        }
        BackendRef::InProcess(_) => {
            let _ = tx.send(AppEvent::FetchError(
                "Remove via in-process mode not supported — use CLI".to_string(),
            ));
        }
    });
}

/// Reconnect an extension's MCP server.
pub fn spawn_reconnect_extension(backend: BackendRef, id: String, tx: mpsc::Sender<AppEvent>) {
    std::thread::spawn(move || match backend {
        BackendRef::Daemon(base_url) => {
            let client = daemon_client();
            match client
                .post(format!("{base_url}/api/integrations/{id}/reconnect"))
                .send()
            {
                Ok(resp) if resp.status().is_success() => {
                    let tool_count = resp
                        .json::<serde_json::Value>()
                        .ok()
                        .and_then(|b| b["tool_count"].as_u64())
                        .unwrap_or(0) as usize;
                    let _ = tx.send(AppEvent::ExtensionReconnected(id, tool_count));
                }
                _ => {
                    let _ = tx.send(AppEvent::FetchError(format!("Failed to reconnect {id}")));
                }
            }
        }
        BackendRef::InProcess(_) => {
            let _ = tx.send(AppEvent::FetchError(
                "Reconnect via in-process mode not supported".to_string(),
            ));
        }
    });
}

/// Fetch comms topology + events.
fn comms_node_from_json(value: &serde_json::Value) -> CommsNode {
    CommsNode {
        id: value["id"].as_str().unwrap_or("").to_string(),
        name: value["name"].as_str().unwrap_or("").to_string(),
        state: value["state"].as_str().unwrap_or("").to_string(),
        model: value["model"].as_str().unwrap_or("").to_string(),
    }
}

fn comms_nodes_from_payload(body: &serde_json::Value) -> Vec<CommsNode> {
    array_payload(body, "nodes")
        .map(|items| items.iter().map(comms_node_from_json).collect())
        .unwrap_or_default()
}

fn comms_edge_from_json(value: &serde_json::Value) -> CommsEdge {
    CommsEdge {
        from: value["from"].as_str().unwrap_or("").to_string(),
        to: value["to"].as_str().unwrap_or("").to_string(),
        kind: value["kind"].as_str().unwrap_or("").to_string(),
    }
}

fn comms_edges_from_payload(body: &serde_json::Value) -> Vec<CommsEdge> {
    array_payload(body, "edges")
        .map(|items| items.iter().map(comms_edge_from_json).collect())
        .unwrap_or_default()
}

fn comms_topology_from_json(body: &serde_json::Value) -> (Vec<CommsNode>, Vec<CommsEdge>) {
    (
        comms_nodes_from_payload(body),
        comms_edges_from_payload(body),
    )
}

fn comms_event_from_json(value: &serde_json::Value) -> CommsEventItem {
    CommsEventItem {
        id: value["id"].as_str().unwrap_or("").to_string(),
        timestamp: value["timestamp"].as_str().unwrap_or("").to_string(),
        kind: value["kind"].as_str().unwrap_or("").to_string(),
        source_name: value["source_name"].as_str().unwrap_or("").to_string(),
        target_name: value["target_name"].as_str().unwrap_or("").to_string(),
        detail: value["detail"].as_str().unwrap_or("").to_string(),
    }
}

fn comms_events_from_payload(body: &serde_json::Value) -> Vec<CommsEventItem> {
    array_payload_any(body, &["events"])
        .map(|items| items.iter().map(comms_event_from_json).collect())
        .unwrap_or_default()
}

pub fn spawn_fetch_comms(backend: BackendRef, tx: mpsc::Sender<AppEvent>) {
    std::thread::spawn(move || match backend {
        BackendRef::Daemon(base_url) => {
            let client = daemon_client();
            // Fetch topology
            if let Ok(resp) = client.get(format!("{base_url}/api/comms/topology")).send() {
                if let Ok(body) = resp.json::<serde_json::Value>() {
                    let (nodes, edges) = comms_topology_from_json(&body);
                    let _ = tx.send(AppEvent::CommsTopologyLoaded { nodes, edges });
                }
            }
            // Fetch events
            if let Ok(resp) = client
                .get(format!("{base_url}/api/comms/events?limit=100"))
                .send()
            {
                if let Ok(body) = resp.json::<serde_json::Value>() {
                    let _ = tx.send(AppEvent::CommsEventsLoaded(comms_events_from_payload(
                        &body,
                    )));
                }
            }
        }
        BackendRef::InProcess(_) => {
            let _ = tx.send(AppEvent::CommsTopologyLoaded {
                nodes: Vec::new(),
                edges: Vec::new(),
            });
            let _ = tx.send(AppEvent::CommsEventsLoaded(Vec::new()));
        }
    });
}

#[cfg(test)]
mod comms_fetch_tests {
    use super::*;

    #[test]
    fn comms_mapping_accepts_topology_and_direct_events() {
        let (nodes, edges) = comms_topology_from_json(&serde_json::json!({
            "nodes": [
                {
                    "id": "agent-1",
                    "name": "Core",
                    "state": "Running",
                    "model": "gpt-5"
                }
            ],
            "edges": [
                {
                    "from": "agent-1",
                    "to": "agent-2",
                    "kind": "peer"
                }
            ]
        }));
        let events = comms_events_from_payload(&serde_json::json!([
            {
                "id": "event-1",
                "timestamp": "2026-06-16T12:00:00Z",
                "kind": "agent_message",
                "source_name": "Core",
                "target_name": "Worker",
                "detail": "hello"
            }
        ]));

        assert_eq!(nodes.len(), 1);
        assert_eq!(nodes[0].name, "Core");
        assert_eq!(edges.len(), 1);
        assert_eq!(edges[0].kind, "peer");
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].source_name, "Core");
        assert_eq!(events[0].detail, "hello");
    }

    #[test]
    fn comms_mapping_accepts_wrapped_events_and_missing_fields() {
        let (nodes, edges) = comms_topology_from_json(&serde_json::json!({}));
        let events = comms_events_from_payload(&serde_json::json!({
            "events": [
                {
                    "id": "event-1",
                    "kind": "task_posted"
                },
                {}
            ]
        }));

        assert!(nodes.is_empty());
        assert!(edges.is_empty());
        assert_eq!(events.len(), 2);
        assert_eq!(events[0].kind, "task_posted");
        assert_eq!(events[0].timestamp, "");
        assert_eq!(events[1].id, "");
    }
}

/// Send a message between agents via comms endpoint.
pub fn spawn_comms_send(
    backend: BackendRef,
    from: String,
    to: String,
    msg: String,
    tx: mpsc::Sender<AppEvent>,
) {
    std::thread::spawn(move || match backend {
        BackendRef::Daemon(base_url) => {
            let client = daemon_client();
            let body = serde_json::json!({
                "from_agent_id": from,
                "to_agent_id": to,
                "message": msg,
            });
            match client
                .post(format!("{base_url}/api/comms/send"))
                .json(&body)
                .send()
            {
                Ok(resp) => {
                    if resp.status().is_success() {
                        let _ = tx.send(AppEvent::CommsSendResult("Message sent".to_string()));
                    } else {
                        let err = resp
                            .json::<serde_json::Value>()
                            .ok()
                            .and_then(|v| v["error"].as_str().map(String::from))
                            .unwrap_or_else(|| "Send failed".to_string());
                        let _ = tx.send(AppEvent::CommsSendResult(err));
                    }
                }
                Err(e) => {
                    let _ = tx.send(AppEvent::CommsSendResult(format!("Error: {e}")));
                }
            }
        }
        BackendRef::InProcess(_) => {
            let _ = tx.send(AppEvent::CommsSendResult(
                "Send not supported in-process".to_string(),
            ));
        }
    });
}

/// Post a task via comms endpoint.
pub fn spawn_comms_task(
    backend: BackendRef,
    title: String,
    desc: String,
    assign: String,
    tx: mpsc::Sender<AppEvent>,
) {
    std::thread::spawn(move || match backend {
        BackendRef::Daemon(base_url) => {
            let client = daemon_client();
            let mut body = serde_json::json!({
                "title": title,
                "description": desc,
            });
            if !assign.is_empty() {
                body["assigned_to"] = serde_json::Value::String(assign);
            }
            match client
                .post(format!("{base_url}/api/comms/task"))
                .json(&body)
                .send()
            {
                Ok(resp) => {
                    if resp.status().is_success() {
                        let _ = tx.send(AppEvent::CommsTaskResult("Task posted".to_string()));
                    } else {
                        let err = resp
                            .json::<serde_json::Value>()
                            .ok()
                            .and_then(|v| v["error"].as_str().map(String::from))
                            .unwrap_or_else(|| "Post failed".to_string());
                        let _ = tx.send(AppEvent::CommsTaskResult(err));
                    }
                }
                Err(e) => {
                    let _ = tx.send(AppEvent::CommsTaskResult(format!("Error: {e}")));
                }
            }
        }
        BackendRef::InProcess(_) => {
            let _ = tx.send(AppEvent::CommsTaskResult(
                "Task post not supported in-process".to_string(),
            ));
        }
    });
}

#[cfg(test)]
mod tests_api_payload_compat {
    use super::*;

    #[test]
    fn array_payload_accepts_legacy_array_and_wrapped_collection() {
        let legacy = serde_json::json!([{"id": "a"}]);
        let wrapped = serde_json::json!({"sessions": [{"id": "b"}]});

        assert_eq!(array_payload(&legacy, "sessions").unwrap().len(), 1);
        assert_eq!(array_payload(&wrapped, "sessions").unwrap().len(), 1);
    }

    #[test]
    fn cron_schedule_label_accepts_current_serialized_shapes() {
        assert_eq!(
            cron_schedule_label(
                &serde_json::json!({"Cron": {"expr": "0 9 * * *", "tz": "Europe/Paris"}})
            ),
            "0 9 * * * (Europe/Paris)"
        );
        assert_eq!(
            cron_schedule_label(&serde_json::json!({"Every": {"every_secs": 300}})),
            "every 300s"
        );
        assert_eq!(
            cron_schedule_label(&serde_json::json!({"expr": "*/5 * * * *"})),
            "*/5 * * * *"
        );
    }

    #[test]
    fn source_label_accepts_object_sources() {
        assert_eq!(
            source_label(&serde_json::json!({"type": "clawhub", "slug": "demo"})),
            "clawhub"
        );
    }
}
