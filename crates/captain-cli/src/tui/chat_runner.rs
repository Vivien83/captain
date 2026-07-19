//! Standalone chat TUI for `captain chat`.
//!
//! Launches a focused ratatui chat screen — same beautiful rendering as the
//! full TUI's Chat tab, but without the 17-tab chrome. Reuses 100% of
//! `ChatState`, `chat::draw()`, event spawning, and the theme system.

use super::event::{self, AppEvent};
use super::screens::chat::{self, ChatAction, ChatMouseAction, ChatState, PendingAskUser, Role};
use super::session_runtime::{
    public_session_label, restore_public_session_messages, LoadedSession,
};
use super::slash_command;
use super::slash_daemon;
use super::slash_exit;
use super::slash_export;
use super::slash_fortune;
use super::slash_help;
use super::slash_info;
use super::slash_kill;
use super::slash_local;
use super::slash_model;
use super::slash_reload;
use super::slash_retry;
use super::slash_scroll;
use super::slash_session;
use super::slash_standalone;
use super::slash_think;
use super::theme;
use super::usage_slash_state::{
    cost_usage_message, token_usage_message, UsageSlashSnapshot, UsageSlashSurface,
};
use captain_kernel::CaptainKernel;
use captain_runtime::llm_driver::StreamEvent;
use captain_types::agent::AgentId;
use ratatui::layout::{Alignment, Constraint, Layout, Rect};
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
use std::path::PathBuf;
use std::sync::{mpsc, Arc};
use std::time::Duration;

fn web_resume_session_id() -> Option<String> {
    std::env::var("CAPTAIN_WEB_RESUME_SESSION_ID")
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| uuid::Uuid::parse_str(value).is_ok())
}

// ── Internal state ───────────────────────────────────────────────────────────

enum Backend {
    Daemon { base_url: String },
    InProcess { kernel: Arc<CaptainKernel> },
    None,
}

struct StandaloneChat {
    chat: ChatState,
    event_tx: mpsc::Sender<AppEvent>,
    backend: Backend,
    agent_id_daemon: Option<String>,
    daemon_session_id: Option<String>,
    agent_id_inprocess: Option<AgentId>,
    inprocess_session_id: Option<String>,
    agent_name: String,
    should_quit: bool,
    booting: bool,
    boot_error: Option<String>,
    spinner_frame: usize,
    /// Current terminal mouse-capture state. Standalone chat enables it by
    /// default unless CAPTAIN_TUI_MOUSE=0, so the TUI has real scrollback.
    mouse_capture_enabled: bool,
    /// In-process ask_user answer channel for the currently live stream —
    /// same role as `App::current_stream_input_tx` in `tui/mod.rs`, populated
    /// from `AppEvent::StreamStarted` and used to answer a pending
    /// `ask_user` without going through HTTP (in-process mode only).
    current_stream_input_tx: Option<tokio::sync::mpsc::Sender<String>>,
    provider_quota_watch_started: bool,
}

impl StandaloneChat {
    fn new(event_tx: mpsc::Sender<AppEvent>) -> Self {
        Self {
            chat: ChatState::new(),
            event_tx,
            backend: Backend::None,
            agent_id_daemon: None,
            daemon_session_id: None,
            agent_id_inprocess: None,
            inprocess_session_id: None,
            agent_name: String::new(),
            should_quit: false,
            booting: false,
            boot_error: None,
            spinner_frame: 0,
            mouse_capture_enabled: false,
            current_stream_input_tx: None,
            provider_quota_watch_started: false,
        }
    }

    // ── Event dispatch ───────────────────────────────────────────────────────

    fn handle_event(&mut self, ev: AppEvent) {
        match ev {
            AppEvent::Key(key) => self.handle_key(key),
            AppEvent::Paste(data) => self.chat.handle_paste(&data),
            AppEvent::Scroll { up } => {
                if up {
                    self.chat.scroll_wheel_up();
                } else {
                    self.chat.scroll_wheel_down();
                }
            }
            AppEvent::MouseClick { x, y } => {
                if let Some(action) = self.chat.handle_mouse_click(x, y) {
                    match action {
                        ChatMouseAction::CopyCommand(command) => {
                            self.copy_to_clipboard_status(command, "Command");
                        }
                        ChatMouseAction::ApplyModelSwitch {
                            model_id,
                            session_strategy,
                        } => self.switch_model(&model_id, Some(&session_strategy)),
                        ChatMouseAction::ApproveRequest(_)
                        | ChatMouseAction::ApproveSessionRequest(_)
                        | ChatMouseAction::ApproveAlwaysRequest(_)
                        | ChatMouseAction::RejectRequest(_)
                        | ChatMouseAction::ModelSwitchCancelled
                        | ChatMouseAction::ToolToggled => {}
                    }
                }
            }
            AppEvent::Tick => self.handle_tick(),
            AppEvent::Stream(stream_ev) => self.handle_stream(stream_ev),
            AppEvent::StreamDone(result) => self.handle_stream_done(result),
            AppEvent::KernelReady(kernel) => self.handle_kernel_ready(kernel),
            AppEvent::KernelError(err) => self.handle_kernel_error(err),
            AppEvent::AgentSpawned {
                id,
                name,
                api_sheet,
            } => self.handle_agent_spawned(id, name, api_sheet),
            AppEvent::AgentSpawnError(err) => self.handle_agent_spawn_error(err),
            AppEvent::StreamStarted { interject_tx } => {
                self.current_stream_input_tx = Some(interject_tx);
            }
            AppEvent::SessionLoaded(session) => self.handle_loaded_session(session),
            AppEvent::SessionsLoaded(sessions) => {
                if self.chat.show_session_picker {
                    self.chat.set_authoritative_session_picker_items(&sessions);
                    if !self.chat.show_session_picker {
                        self.chat.push_message(
                            Role::System,
                            slash_session::no_saved_history_message(crate::i18n::current())
                                .to_string(),
                        );
                    }
                }
            }
            AppEvent::ProviderQuotasLoaded(result) => match result {
                Ok(status) => self.chat.provider_quota_status = status,
                Err(error) => {
                    if !self.chat.provider_quota_status.has_observation() {
                        self.chat.provider_quota_status = Default::default();
                    }
                    tracing::debug!(error = %error, "TUI provider quota refresh unavailable");
                }
            },
            AppEvent::FetchError(error) => self.chat.status_msg = Some(error),
            // All other events (tab-specific data loads) are irrelevant in
            // standalone chat mode — silently ignore.
            _ => {}
        }
    }

    fn copy_to_clipboard_status(&mut self, text: String, label: &str) {
        let byte_len = text.len();
        let msg = match arboard::Clipboard::new().and_then(|mut cb| cb.set_text(text.clone())) {
            Ok(()) => slash_local::copy_success_message(
                slash_local::CopyStatusSurface::StandaloneChat,
                label,
                byte_len,
            ),
            Err(e) => {
                slash_local::copy_failure_message(slash_local::CopyStatusSurface::StandaloneChat, e)
            }
        };
        self.chat.push_message(Role::System, msg);
    }

    fn handle_key(&mut self, key: ratatui::crossterm::event::KeyEvent) {
        use ratatui::crossterm::event::{KeyCode, KeyModifiers};

        // Ctrl+Q / Ctrl+C always quit
        if key.modifiers.contains(KeyModifiers::CONTROL) {
            match key.code {
                KeyCode::Char('q') | KeyCode::Char('c') => {
                    self.should_quit = true;
                    return;
                }
                _ => {}
            }
        }

        // If still booting, only allow quit keys
        if self.booting || self.backend_is_none() {
            if key.code == KeyCode::Esc {
                self.should_quit = true;
            }
            return;
        }

        let action = self.chat.handle_key(key);
        self.handle_chat_action(action);
    }

    fn handle_tick(&mut self) {
        self.chat.tick();
        if self.booting {
            self.spinner_frame = (self.spinner_frame + 1) % theme::SPINNER_FRAMES.len();
        }
    }

    fn handle_stream(&mut self, ev: StreamEvent) {
        match ev {
            StreamEvent::TextDelta { text } => {
                self.chat.thinking = false;
                if self.chat.active_tool.is_some() {
                    self.chat.active_tool = None;
                }
                self.chat.append_stream(&text);
            }
            StreamEvent::ToolUseStart { id, name } => {
                if !self.chat.streaming_text.is_empty() {
                    let text = std::mem::take(&mut self.chat.streaming_text);
                    self.chat.push_message(Role::Agent, text);
                }
                self.chat.tool_start(&id, &name);
            }
            StreamEvent::ToolInputDelta { text } => {
                self.chat.tool_input_buf.push_str(&text);
            }
            StreamEvent::ToolUseEnd { id, name, input } => {
                let input_str = if !self.chat.tool_input_buf.is_empty() {
                    std::mem::take(&mut self.chat.tool_input_buf)
                } else {
                    serde_json::to_string(&input).unwrap_or_default()
                };
                self.chat.tool_use_end(&id, &name, &input_str);
            }
            StreamEvent::ContentComplete { usage, .. } => {
                self.chat
                    .record_context_usage(usage.input_tokens, usage.output_tokens);
                self.chat.last_tokens = Some((usage.input_tokens, usage.output_tokens));
                self.chat.last_cached_input_tokens = usage.cached_input_tokens;
                self.chat.last_cache_creation_tokens = usage.cache_creation_tokens;
            }
            StreamEvent::PhaseChange { phase, detail } => {
                if phase == "tool_use" {
                    if let Some(tool_name) = detail {
                        self.chat.tool_start("", &tool_name);
                    }
                } else if phase == "thinking" {
                    self.chat.thinking = true;
                } else if phase == "model_fallback" {
                    if let Some(text) = detail.filter(|value| !value.trim().is_empty()) {
                        self.chat.push_message(Role::Agent, text);
                    }
                }
            }
            StreamEvent::ThinkingDelta { text } => {
                self.chat.thinking = true;
                self.chat.append_stream(&text);
            }
            StreamEvent::ToolExecutionResult {
                tool_use_id,
                name,
                result_preview,
                is_error,
            } => {
                self.chat
                    .tool_result(&tool_use_id, &name, &result_preview, is_error);
            }
            StreamEvent::IntermediateMessage { content } => {
                if !self.chat.streaming_text.is_empty() {
                    let text = std::mem::take(&mut self.chat.streaming_text);
                    self.chat.push_message(Role::Agent, text);
                }
                self.chat.push_message(Role::Agent, content);
            }
            StreamEvent::AskUser { question, options } => {
                let options = options.unwrap_or_default();
                if options.is_empty() {
                    // No predefined choices — keep the existing plain-text
                    // behavior so the keyboard stays free for typing.
                    self.chat
                        .push_message(Role::Agent, format!("❓ {question}"));
                } else {
                    self.chat.pending_ask_user = Some(PendingAskUser { question, options });
                }
            }
            StreamEvent::UserResponse { .. } => {}
            StreamEvent::ToolOutputDelta {
                tool_use_id,
                stream,
                chunk,
            } => {
                self.chat.tool_output_delta(&tool_use_id, stream, &chunk);
            }
        }
    }

    fn handle_stream_done(
        &mut self,
        result: Result<captain_runtime::agent_loop::AgentLoopResult, String>,
    ) {
        self.chat.finalize_stream();
        match result {
            Ok(r) => {
                if !r.response.is_empty()
                    && self.chat.messages.last().map(|m| m.text.as_str()) != Some(&r.response)
                {
                    self.chat.push_message(Role::Agent, r.response);
                }
                if r.total_usage.input_tokens > 0 || r.total_usage.output_tokens > 0 {
                    if self.chat.context_stream_checkpoint_chars.is_none() {
                        self.chat.record_context_usage(
                            r.total_usage.input_tokens,
                            r.total_usage.output_tokens,
                        );
                    }
                    self.chat.last_tokens =
                        Some((r.total_usage.input_tokens, r.total_usage.output_tokens));
                    self.chat.last_cached_input_tokens = r.total_usage.cached_input_tokens;
                    self.chat.last_cache_creation_tokens = r.total_usage.cache_creation_tokens;
                    self.chat.record_usage(
                        r.total_usage.input_tokens,
                        r.total_usage.output_tokens,
                        r.total_usage.cached_input_tokens,
                        r.total_usage.cache_creation_tokens,
                        r.cost_usd.unwrap_or(0.0),
                    );
                }
                self.chat.last_cost_usd = r.cost_usd;
            }
            Err(e) => {
                self.chat.status_msg = Some(slash_standalone::stream_error_message(e));
            }
        }
        self.refresh_active_chat_metadata();
        self.chat.persist_session();
        // Auto-send the next staged message if any
        if let Some(msg) = self.chat.take_staged() {
            self.send_message(msg);
        }
    }

    fn refresh_active_chat_metadata(&mut self) {
        match &self.backend {
            Backend::Daemon { base_url } => {
                let Some(agent_id) = self.agent_id_daemon.as_deref() else {
                    return;
                };
                let client = crate::daemon_client();
                let Ok(resp) = client
                    .get(format!("{base_url}/api/agents/{agent_id}"))
                    .send()
                else {
                    return;
                };
                let Ok(body) = resp.json::<serde_json::Value>() else {
                    return;
                };
                self.chat.apply_agent_runtime_metadata(&body);
            }
            Backend::InProcess { kernel } => {
                let Some(agent_id) = self.agent_id_inprocess else {
                    return;
                };
                if let Some(entry) = kernel.registry.get(agent_id) {
                    self.chat.model_label = format!(
                        "{}/{}",
                        entry.manifest.model.provider, entry.manifest.model.model
                    );
                }
                if let Some(context_window) = kernel.effective_context_window_for_agent(agent_id) {
                    self.chat.set_context_window_tokens(context_window as u64);
                }
            }
            Backend::None => {}
        }
    }

    // ── Kernel lifecycle ─────────────────────────────────────────────────────

    fn handle_kernel_ready(&mut self, kernel: Arc<CaptainKernel>) {
        self.booting = false;
        self.boot_error = None;
        self.backend = Backend::InProcess { kernel };
        self.start_provider_quota_watch();
        // Spawn or find the agent
        self.resolve_inprocess_agent();
    }

    fn handle_kernel_error(&mut self, err: String) {
        self.booting = false;
        self.boot_error = Some(err);
    }

    fn handle_agent_spawned(
        &mut self,
        id: String,
        name: String,
        api_sheet: Option<crate::agent_api_sheet::AgentApiSpawnSheet>,
    ) {
        self.enter_chat_daemon(id, name);
        match api_sheet {
            Some(sheet) => self.chat.push_operator_notice(sheet.tui_notice_lines()),
            None => self.chat.push_operator_notice(vec![
                "Agent created.".to_string(),
                "Agent API sheet unavailable in this response.".to_string(),
                "Run `captain agent api <agent> --rotate-token` to inspect ingress/egress and get a fresh token.".to_string(),
            ]),
        }
    }

    fn handle_agent_spawn_error(&mut self, err: String) {
        self.chat.status_msg = Some(slash_standalone::daemon_agent_spawn_failed_message(err));
    }

    fn handle_loaded_session(&mut self, loaded: LoadedSession) {
        match &self.backend {
            Backend::Daemon { .. } => {
                self.agent_id_daemon = Some(loaded.agent_id.clone());
                self.daemon_session_id = Some(loaded.session_id.clone());
                self.agent_id_inprocess = None;
                self.inprocess_session_id = None;
            }
            Backend::InProcess { .. } => {
                let Ok(agent_id) = loaded.agent_id.parse::<AgentId>() else {
                    self.chat.status_msg = Some("Session owner is invalid".to_string());
                    return;
                };
                self.agent_id_inprocess = Some(agent_id);
                self.inprocess_session_id = Some(loaded.session_id.clone());
                self.agent_id_daemon = None;
                self.daemon_session_id = None;
            }
            Backend::None => {
                self.chat.status_msg = Some("No backend connected".to_string());
                return;
            }
        }

        self.agent_name = loaded.agent_name.clone();
        self.chat.reset();
        self.chat.agent_name = loaded.agent_name;
        self.chat.mode_label = match &self.backend {
            Backend::Daemon { .. } => "daemon",
            Backend::InProcess { .. } => "in-process",
            Backend::None => "disconnected",
        }
        .to_string();
        let short = loaded
            .session_id
            .get(..8)
            .unwrap_or(loaded.session_id.as_str());
        self.chat
            .start_session(&format!("session-{}-{short}", loaded.agent_id));
        self.chat
            .bind_authoritative_session(&loaded.agent_id, &loaded.session_id);
        let restored = restore_public_session_messages(&mut self.chat, &loaded.detail);
        self.refresh_active_chat_metadata();
        self.chat.push_message(
            Role::System,
            format!(
                "Session restaurée : {} ({restored} messages).",
                loaded.label
            ),
        );
    }

    // ── Chat action dispatch ─────────────────────────────────────────────────

    fn handle_chat_action(&mut self, action: ChatAction) {
        match action {
            ChatAction::Continue => {}
            ChatAction::Back => {
                self.should_quit = true;
            }
            ChatAction::SendMessage(msg) => self.send_message(msg),
            ChatAction::SlashCommand(cmd) => self.handle_slash_command(&cmd),
            ChatAction::ResumeSession(session_id) => {
                if let Some(backend) = self.backend_ref() {
                    event::spawn_load_session(backend, session_id, self.event_tx.clone());
                }
            }
            ChatAction::OpenSessionPicker => {
                if let Some(backend) = self.backend_ref() {
                    event::spawn_fetch_sessions(backend, self.event_tx.clone());
                }
            }
            ChatAction::OpenModelPicker => self.open_model_picker(),
            ChatAction::SwitchModel(model_id) => self.switch_model(&model_id, None),
            ChatAction::ApplyModelSwitch {
                model_id,
                session_strategy,
            } => self.switch_model(&model_id, Some(&session_strategy)),
            // Phase-i.6 + Q.11.b.b chat-runner standalone mode does not surface
            // approval modals (no chat_target). Drop silently — daemon side
            // still resolves via the polling /approvals tab if needed.
            ChatAction::ApproveRequest(_)
            | ChatAction::ApproveSessionRequest(_)
            | ChatAction::ApproveAlwaysRequest(_)
            | ChatAction::RejectRequest(_) => {}
            ChatAction::AnswerAskUser(content) => match &self.backend {
                Backend::InProcess { .. } => {
                    if let Some(tx) = self.current_stream_input_tx.as_ref() {
                        if tx.try_send(content).is_err() {
                            self.current_stream_input_tx = None;
                        }
                    }
                }
                Backend::Daemon { base_url } => {
                    if let Some(agent_id) = self.agent_id_daemon.clone() {
                        event::spawn_answer_ask_user(
                            event::BackendRef::Daemon(base_url.clone()),
                            agent_id,
                            self.daemon_session_id.clone(),
                            content,
                            self.event_tx.clone(),
                        );
                    }
                }
                Backend::None => {}
            },
        }
    }

    fn send_message(&mut self, message: String) {
        let session_id = match self.ensure_authoritative_session() {
            Ok(session_id) => session_id,
            Err(error) => {
                self.chat.status_msg = Some(format!("Session creation failed: {error}"));
                return;
            }
        };
        self.chat.is_streaming = true;
        self.chat.thinking = true;
        self.chat.streaming_chars = 0;
        self.chat.begin_context_stream();
        self.chat.last_tokens = None;
        self.chat.last_cached_input_tokens = 0;
        self.chat.last_cache_creation_tokens = 0;
        self.chat.last_cost_usd = None;
        self.chat.status_msg = None;

        match &self.backend {
            Backend::Daemon { base_url } if self.agent_id_daemon.is_some() => {
                event::spawn_daemon_stream(
                    base_url.clone(),
                    self.agent_id_daemon.as_ref().unwrap().clone(),
                    Some(session_id.clone()),
                    message,
                    Vec::new(), // chat_runner standalone has no /image yet
                    self.event_tx.clone(),
                );
            }
            Backend::InProcess { kernel } if self.agent_id_inprocess.is_some() => {
                event::spawn_inprocess_stream(
                    kernel.clone(),
                    self.agent_id_inprocess.unwrap(),
                    session_id
                        .parse::<uuid::Uuid>()
                        .ok()
                        .map(captain_types::agent::SessionId),
                    message,
                    self.event_tx.clone(),
                );
            }
            _ => {
                self.chat.is_streaming = false;
                self.chat.status_msg =
                    Some(slash_standalone::no_active_connection_message().to_string());
            }
        }
    }

    fn ensure_authoritative_session(&mut self) -> Result<String, String> {
        if let Some(session_id) = self
            .daemon_session_id
            .as_ref()
            .or(self.inprocess_session_id.as_ref())
        {
            return Ok(session_id.clone());
        }

        let (agent_id, session_id) = match &self.backend {
            Backend::Daemon { base_url } => {
                let agent_id = self.agent_id_daemon.clone().ok_or_else(|| {
                    slash_session::reset_daemon_agent_missing_message().to_string()
                })?;
                let response = crate::daemon_client()
                    .post(format!("{base_url}/api/agents/{agent_id}/sessions"))
                    .json(&serde_json::json!({"activate": false}))
                    .send()
                    .map_err(|error| error.to_string())?;
                if !response.status().is_success() {
                    return Err(format!("daemon returned HTTP {}", response.status()));
                }
                let body = response
                    .json::<serde_json::Value>()
                    .map_err(|error| error.to_string())?;
                let session_id = body
                    .get("session_id")
                    .and_then(serde_json::Value::as_str)
                    .filter(|value| uuid::Uuid::parse_str(value).is_ok())
                    .ok_or_else(|| "daemon returned no valid session ID".to_string())?
                    .to_string();
                self.daemon_session_id = Some(session_id.clone());
                (agent_id, session_id)
            }
            Backend::InProcess { kernel } => {
                let agent_id = self.agent_id_inprocess.ok_or_else(|| {
                    slash_session::reset_inprocess_agent_missing_message().to_string()
                })?;
                let created = kernel
                    .create_agent_session_detached(agent_id, None)
                    .map_err(|error| error.to_string())?;
                let session_id = created
                    .get("session_id")
                    .and_then(serde_json::Value::as_str)
                    .filter(|value| uuid::Uuid::parse_str(value).is_ok())
                    .ok_or_else(|| "kernel returned no valid session ID".to_string())?
                    .to_string();
                self.inprocess_session_id = Some(session_id.clone());
                (agent_id.to_string(), session_id)
            }
            Backend::None => return Err("No backend connected".to_string()),
        };
        self.chat.bind_authoritative_session(&agent_id, &session_id);
        Ok(session_id)
    }

    fn chat_session_prefix(&self) -> Option<String> {
        if let Some(id) = &self.agent_id_daemon {
            Some(format!("daemon-{id}"))
        } else {
            self.agent_id_inprocess
                .map(|id| format!("inprocess-{}", id.0))
        }
    }

    fn start_fresh_local_chat_session(&mut self) {
        let key = self.chat_session_prefix();
        self.chat.reset_preserving_chat_identity();
        if let Some(key) = key {
            let new_key = format!(
                "{key}-{}",
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map(|d| d.as_secs())
                    .unwrap_or(0)
            );
            self.chat.start_session(&new_key);
        }
    }

    fn reset_backend_session(&mut self) -> Result<(), String> {
        match &self.backend {
            Backend::Daemon { .. } => {
                self.agent_id_daemon.as_ref().ok_or_else(|| {
                    slash_session::reset_daemon_agent_missing_message().to_string()
                })?;
                self.daemon_session_id = None;
                Ok(())
            }
            Backend::InProcess { .. } => {
                self.agent_id_inprocess.ok_or_else(|| {
                    slash_session::reset_inprocess_agent_missing_message().to_string()
                })?;
                self.inprocess_session_id = None;
                Ok(())
            }
            Backend::None => Err(slash_session::reset_no_backend_connected_message().to_string()),
        }
    }

    // ── Slash commands (subset — no tab navigation) ──────────────────────────

    fn handle_slash_command(&mut self, cmd: &str) {
        let (raw_command, args) = slash_command::split_slash_command(cmd);
        let command = raw_command.to_ascii_lowercase();
        let canonical_command = slash_command::canonical_slash_command(&raw_command, args);
        let lang = crate::i18n::current();
        if self.handle_common_slash_command(&command, args, &canonical_command) {
            return;
        }
        match command.as_str() {
            "/help" => {
                self.chat
                    .push_message(Role::System, slash_help::standalone_help(lang).to_string());
            }
            "/status" => {
                self.handle_status_slash(lang);
            }
            "/clear" => {
                slash_local::clear_chat_preserving_identity(&mut self.chat);
                self.chat
                    .push_message(Role::System, slash_local::clear_message(lang).to_string());
            }
            "/sessions" | "/tasks" => {
                self.handle_sessions_slash(lang);
            }
            "/resume" => {
                if let Some(backend) = self.backend_ref() {
                    event::spawn_resolve_session(backend, args.to_string(), self.event_tx.clone());
                }
            }
            "/agents" => {
                self.handle_agents_slash(lang);
            }
            "/new" => self.handle_new_slash(lang),
            "/copy" => {
                self.handle_copy_slash(args, lang);
            }
            "/mouse" => {
                self.handle_mouse_slash(args, lang);
            }
            "/reload" => match slash_reload::reload_for(args) {
                slash_reload::SlashReload::ForwardDaemon => {
                    self.forward_daemon_slash_command(&canonical_command);
                }
                slash_reload::SlashReload::ReloadSession => {
                    self.handle_reload_session_slash(lang);
                }
            },
            "/tokens" => {
                self.handle_tokens_slash(lang);
            }
            "/cost" => {
                self.handle_cost_slash(lang);
            }
            "/history" => {
                self.handle_history_slash(lang);
            }
            "/export" => {
                self.handle_export_slash(lang);
            }
            "/retry" => match slash_retry::last_user_message(&self.chat.messages) {
                Some(msg) => self.send_message(msg),
                None => self.handle_retry_empty_slash(lang),
            },
            "/undo" => {
                self.handle_undo_slash(lang);
            }
            "/queue" => {
                self.handle_queue_slash(lang);
            }
            "/fortune" => {
                self.handle_fortune_slash(lang);
            }
            command if slash_standalone::is_attachment_or_voice_command(command) => {
                self.handle_attachments_voice_slash(lang);
            }
            command if slash_standalone::is_feedback_command(command) => {
                self.handle_feedback_unavailable_slash(lang);
            }
            command if slash_standalone::is_full_tui_navigation_command(command) => {
                self.handle_full_tui_navigation_slash(lang, &raw_command);
            }
            "/kill" => {
                self.handle_kill_slash(lang);
            }
            _ => {
                self.handle_unknown_slash(lang, &raw_command);
            }
        }
    }

    fn handle_common_slash_command(
        &mut self,
        command: &str,
        args: &str,
        canonical_command: &str,
    ) -> bool {
        if let Some(scroll) = slash_scroll::scroll_for(command) {
            match scroll {
                slash_scroll::SlashScroll::Top => self.chat.scroll_to_top(),
                slash_scroll::SlashScroll::Bottom => self.chat.scroll_to_bottom(),
            }
            return true;
        }
        if let Some(model) = slash_model::model_for(command, args) {
            match model {
                slash_model::SlashModel::OpenPicker => self.open_model_picker(),
                slash_model::SlashModel::Switch { model, strategy } => {
                    self.switch_model(model, strategy)
                }
            }
            return true;
        }
        if slash_think::is_think_command(command) {
            self.chat.toggle_thinking();
            return true;
        }
        if slash_daemon::is_daemon_forward_command(command) {
            self.forward_daemon_slash_command(canonical_command);
            return true;
        }
        if slash_exit::is_exit_command(command) {
            self.should_quit = true;
            return true;
        }
        false
    }

    fn handle_kill_slash(&mut self, lang: crate::i18n::Lang) {
        let name = self.agent_name.clone();
        if slash_kill::is_protected_agent(&name) {
            self.chat.push_message(
                Role::System,
                slash_kill::protected_agent_message(lang).to_string(),
            );
            return;
        }
        match &self.backend {
            Backend::Daemon { base_url } => {
                if let Some(ref id) = self.agent_id_daemon {
                    let client = crate::daemon_client();
                    let url = format!("{base_url}/api/agents/{id}");
                    match client.delete(&url).send() {
                        Ok(r) if r.status().is_success() => {
                            self.chat.push_message(
                                Role::System,
                                slash_kill::kill_success_message(lang, &name),
                            );
                            self.should_quit = true;
                        }
                        _ => {
                            self.chat.push_message(
                                Role::System,
                                slash_kill::kill_failed_message(lang, &name),
                            );
                        }
                    }
                }
            }
            Backend::InProcess { kernel } => {
                if let Some(id) = self.agent_id_inprocess {
                    match kernel.kill_agent(id) {
                        Ok(()) => {
                            self.chat.push_message(
                                Role::System,
                                slash_kill::kill_success_message(lang, &name),
                            );
                            self.should_quit = true;
                        }
                        Err(e) => {
                            self.chat.push_message(
                                Role::System,
                                slash_kill::kill_error_message(lang, e),
                            );
                        }
                    }
                }
            }
            Backend::None => {
                self.chat.push_message(
                    Role::System,
                    slash_kill::no_backend_message(lang).to_string(),
                );
            }
        }
    }

    fn handle_tokens_slash(&mut self, lang: crate::i18n::Lang) {
        let msg = token_usage_message(
            self.usage_slash_snapshot(),
            UsageSlashSurface::StandaloneChat,
            lang,
        );
        self.chat.push_message(Role::System, msg);
    }

    fn handle_cost_slash(&mut self, lang: crate::i18n::Lang) {
        let msg = cost_usage_message(
            self.usage_slash_snapshot(),
            UsageSlashSurface::StandaloneChat,
            lang,
        );
        self.chat.push_message(Role::System, msg);
    }

    fn handle_history_slash(&mut self, _lang: crate::i18n::Lang) {
        self.chat.open_session_picker();
        if let Some(backend) = self.backend_ref() {
            event::spawn_fetch_sessions(backend, self.event_tx.clone());
        }
    }

    fn handle_export_slash(&mut self, lang: crate::i18n::Lang) {
        let msg = match self.chat.export_markdown() {
            Ok(path) => slash_export::export_success_message(lang, &path),
            Err(e) => slash_export::export_failed_message(
                lang,
                slash_export::ExportSurface::StandaloneChat,
                e,
            ),
        };
        self.chat.push_message(Role::System, msg);
    }

    fn handle_retry_empty_slash(&mut self, lang: crate::i18n::Lang) {
        self.chat.push_message(
            Role::System,
            slash_retry::retry_nothing_message(lang).to_string(),
        );
    }

    fn handle_undo_slash(&mut self, lang: crate::i18n::Lang) {
        let dropped_user = slash_local::undo_last_exchange(&mut self.chat);
        self.chat.push_message(
            Role::System,
            slash_local::undo_result_message(dropped_user, lang).to_string(),
        );
    }

    fn handle_queue_slash(&mut self, lang: crate::i18n::Lang) {
        self.chat.push_message(
            Role::System,
            slash_local::queue_message_for_lang(&self.chat.staged_messages, lang),
        );
    }

    fn handle_fortune_slash(&mut self, lang: crate::i18n::Lang) {
        let secs = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        self.chat.push_message(
            Role::System,
            slash_fortune::fortune_message_for_timestamp_secs(secs, lang).to_string(),
        );
    }

    fn handle_attachments_voice_slash(&mut self, lang: crate::i18n::Lang) {
        self.chat.push_message(
            Role::System,
            slash_standalone::attachments_voice_message(lang).to_string(),
        );
    }

    fn handle_feedback_unavailable_slash(&mut self, lang: crate::i18n::Lang) {
        self.chat.push_message(
            Role::System,
            slash_standalone::feedback_unavailable_message(lang).to_string(),
        );
    }

    fn handle_full_tui_navigation_slash(&mut self, lang: crate::i18n::Lang, raw_command: &str) {
        self.chat.push_message(
            Role::System,
            slash_standalone::full_tui_navigation_message(lang, raw_command),
        );
    }

    fn handle_unknown_slash(&mut self, lang: crate::i18n::Lang, raw_command: &str) {
        self.chat.push_message(
            Role::System,
            slash_standalone::unknown_slash_message(lang, raw_command),
        );
    }

    fn handle_new_slash(&mut self, lang: crate::i18n::Lang) {
        match self.reset_backend_session() {
            Ok(()) => {
                self.start_fresh_local_chat_session();
                self.chat.push_message(
                    Role::System,
                    slash_session::new_session_started_message(lang).to_string(),
                );
            }
            Err(e) => {
                self.chat.push_message(
                    Role::System,
                    slash_session::reset_session_failed_message(lang, e),
                );
            }
        }
    }

    fn handle_copy_slash(&mut self, args: &str, lang: crate::i18n::Lang) {
        let (text, label, empty_msg) = match slash_local::copy_target(args) {
            Ok(target) => {
                let text = match target {
                    slash_local::CopyTarget::Command => self.chat.last_command_to_copy(),
                    slash_local::CopyTarget::Response => self
                        .chat
                        .messages
                        .iter()
                        .rev()
                        .find(|m| matches!(m.role, Role::Agent))
                        .map(|m| m.text.clone()),
                };
                let copy_text = slash_local::copy_target_text(target, lang);
                (text, copy_text.label, copy_text.empty_message)
            }
            Err(_) => {
                let msg = slash_local::copy_usage_message(
                    lang,
                    slash_local::CopyUsageSurface::StandaloneChat,
                );
                self.chat.push_message(Role::System, msg.to_string());
                return;
            }
        };
        if let Some(text) = text {
            self.copy_to_clipboard_status(text, label);
        } else {
            self.chat.push_message(Role::System, empty_msg.to_string());
        }
    }

    fn handle_mouse_slash(&mut self, args: &str, lang: crate::i18n::Lang) {
        let msg = match slash_local::mouse_capture_target(args, self.mouse_capture_enabled) {
            Some(enabled) => match event::set_mouse_capture(enabled) {
                Ok(()) => {
                    self.mouse_capture_enabled = enabled;
                    if enabled {
                        slash_local::mouse_enabled_message(
                            lang,
                            slash_local::MouseMessageSurface::StandaloneChat,
                        )
                        .to_string()
                    } else {
                        slash_local::mouse_disabled_message(lang).to_string()
                    }
                }
                Err(e) => slash_local::mouse_error_message(
                    lang,
                    slash_local::MouseMessageSurface::StandaloneChat,
                    e,
                ),
            },
            None => slash_local::mouse_usage_message(
                lang,
                slash_local::MouseMessageSurface::StandaloneChat,
            )
            .to_string(),
        };
        self.chat.push_message(Role::System, msg);
    }

    fn handle_reload_session_slash(&mut self, lang: crate::i18n::Lang) {
        let key = self.chat.session_key.clone();
        if key.is_empty() {
            self.chat.push_message(
                Role::System,
                slash_reload::no_active_session_message(lang).to_string(),
            );
            return;
        }

        use crate::tui::session_store as store;
        if let Some((_path, loaded)) = store::load_latest_session(&key) {
            if let (Some(session_id), Some(backend)) = (loaded.session_id, self.backend_ref()) {
                event::spawn_load_session(backend, session_id, self.event_tx.clone());
            } else {
                self.chat.push_message(
                    Role::System,
                    slash_reload::no_saved_session_message(lang).to_string(),
                );
            }
        } else {
            self.chat.push_message(
                Role::System,
                slash_reload::no_saved_session_message(lang).to_string(),
            );
        }
    }

    fn handle_status_slash(&mut self, lang: crate::i18n::Lang) {
        let snapshot = match &self.backend {
            Backend::Daemon { base_url } => slash_info::StatusSnapshot::Daemon {
                base_url,
                agent_name: Some(&self.agent_name),
            },
            Backend::InProcess { kernel } => slash_info::StatusSnapshot::InProcess {
                agent_count: kernel.registry.count(),
                agent_name: Some(&self.agent_name),
            },
            Backend::None => slash_info::StatusSnapshot::Disconnected,
        };
        self.chat
            .push_message(Role::System, slash_info::status_message(snapshot, lang));
    }

    fn handle_sessions_slash(&mut self, lang: crate::i18n::Lang) {
        let mut lines = Vec::new();
        match &self.backend {
            Backend::Daemon { base_url } => {
                let client = crate::daemon_client();
                if let Ok(resp) = client.get(format!("{base_url}/api/sessions")).send() {
                    if let Ok(body) = resp.json::<serde_json::Value>() {
                        lines.extend(slash_info::daemon_session_lines(&body, usize::MAX));
                    }
                }
            }
            Backend::InProcess { kernel } => {
                if let Ok(sessions) = kernel.memory.list_sessions() {
                    lines.extend(slash_info::daemon_session_lines(
                        &serde_json::json!({"sessions": sessions}),
                        usize::MAX,
                    ));
                }
            }
            Backend::None => {
                lines.push(slash_info::sessions_not_connected_message(lang).to_string())
            }
        }
        self.chat
            .push_message(Role::System, slash_info::sessions_list_message(lines, lang));
    }

    fn handle_agents_slash(&mut self, lang: crate::i18n::Lang) {
        let mut lines = Vec::new();
        match &self.backend {
            Backend::Daemon { base_url } => {
                let client = crate::daemon_client();
                if let Ok(resp) = client.get(format!("{base_url}/api/agents")).send() {
                    if let Ok(body) = resp.json::<serde_json::Value>() {
                        lines.extend(slash_info::daemon_agent_lines(&body));
                    }
                }
            }
            Backend::InProcess { kernel } => {
                for e in kernel.registry.list() {
                    let state = format!("{:?}", e.state);
                    lines.push(slash_info::inprocess_agent_line(
                        &e.name,
                        &state,
                        &e.manifest.model.provider,
                        &e.manifest.model.model,
                    ));
                }
            }
            Backend::None => {}
        }
        self.chat
            .push_message(Role::System, slash_info::agents_list_message(lines, lang));
    }

    fn forward_daemon_slash_command(&mut self, command: &str) {
        if matches!(self.backend, Backend::Daemon { .. }) {
            self.send_message(command.to_string());
        } else {
            let lang = crate::i18n::current();
            self.chat.push_message(
                Role::System,
                slash_daemon::unavailable_message(lang).to_string(),
            );
        }
    }

    fn usage_slash_snapshot(&self) -> UsageSlashSnapshot {
        UsageSlashSnapshot {
            session_input_tokens: self.chat.session_input_tokens,
            session_output_tokens: self.chat.session_output_tokens,
            session_cached_input_tokens: self.chat.session_cached_input_tokens,
            session_cache_creation_tokens: self.chat.session_cache_creation_tokens,
            last_tokens: self.chat.last_tokens,
            last_cached_input_tokens: self.chat.last_cached_input_tokens,
            session_cost_usd: self.chat.session_cost_usd,
            last_cost_usd: self.chat.last_cost_usd,
            message_count: self.chat.messages.len(),
        }
    }

    // ── Model picker helpers ──────────────────────────────────────────────────

    fn open_model_picker(&mut self) {
        use super::screens::chat::ModelEntry;

        let models = match &self.backend {
            Backend::Daemon { base_url } => {
                let client = crate::daemon_client();
                match client.get(format!("{base_url}/api/models")).send() {
                    Ok(resp) => match resp.json::<serde_json::Value>() {
                        Ok(body) => body["models"]
                            .as_array()
                            .map(|arr| {
                                arr.iter()
                                    .filter(|m| m["available"].as_bool().unwrap_or(false))
                                    .map(|m| ModelEntry {
                                        id: m["id"].as_str().unwrap_or("").to_string(),
                                        display_name: m["display_name"]
                                            .as_str()
                                            .unwrap_or("")
                                            .to_string(),
                                        provider: m["provider"].as_str().unwrap_or("").to_string(),
                                        tier: m["tier"].as_str().unwrap_or("Balanced").to_string(),
                                        context_window: m["context_window"]
                                            .as_u64()
                                            .unwrap_or_default(),
                                    })
                                    .collect()
                            })
                            .unwrap_or_default(),
                        Err(_) => Vec::new(),
                    },
                    Err(_) => Vec::new(),
                }
            }
            Backend::InProcess { kernel } => {
                let catalog = kernel.model_catalog.read().unwrap();
                catalog
                    .available_models()
                    .into_iter()
                    .map(|e| ModelEntry {
                        id: e.id.clone(),
                        display_name: e.display_name.clone(),
                        provider: e.provider.clone(),
                        tier: format!("{:?}", e.tier),
                        context_window: e.context_window,
                    })
                    .collect()
            }
            Backend::None => Vec::new(),
        };

        if models.is_empty() {
            self.chat.push_message(
                Role::System,
                slash_model::no_models_available_message().to_string(),
            );
            return;
        }

        self.chat.model_picker_models = models;
        self.chat.model_picker_filter.clear();
        self.chat.model_picker_idx = 0;
        self.chat.show_model_picker = true;
    }

    fn switch_model(&mut self, model_id: &str, session_strategy: Option<&str>) {
        // Skip if already on this model
        if self.chat.model_label.ends_with(model_id) {
            return;
        }

        match &self.backend {
            Backend::Daemon { base_url } => {
                self.switch_model_daemon(base_url.clone(), model_id, session_strategy);
            }
            Backend::InProcess { kernel } => {
                self.switch_model_inprocess(kernel.clone(), model_id, session_strategy);
            }
            Backend::None => {
                self.chat.push_message(
                    Role::System,
                    slash_model::no_backend_connected_message().to_string(),
                );
            }
        }
    }

    fn switch_model_daemon(
        &mut self,
        base_url: String,
        model_id: &str,
        session_strategy: Option<&str>,
    ) {
        let Some(ref agent_id) = self.agent_id_daemon else {
            return;
        };

        let client = crate::daemon_client();
        let plan_url = format!("{base_url}/api/agents/{agent_id}/model-switch/plan");
        let plan = match client
            .post(&plan_url)
            .json(&serde_json::json!({"model": model_id}))
            .send()
        {
            Ok(r) if r.status().is_success() => match r.json::<serde_json::Value>() {
                Ok(plan) => plan,
                Err(e) => {
                    self.chat.push_message(
                        Role::System,
                        slash_model::daemon_preflight_parse_failed_message(e),
                    );
                    return;
                }
            },
            Ok(r) => {
                self.chat.push_message(
                    Role::System,
                    slash_model::daemon_preflight_http_failed_message(r.status()),
                );
                return;
            }
            Err(e) => {
                self.chat
                    .push_message(Role::System, slash_model::daemon_preflight_error_message(e));
                return;
            }
        };

        if !plan["can_apply"].as_bool().unwrap_or(false) {
            let issues = slash_model::daemon_blocking_issues(&plan);
            self.chat.push_message(
                Role::System,
                slash_model::model_switch_blocked_message(&issues),
            );
            return;
        }

        let strategy =
            match slash_model::daemon_model_switch_decision(model_id, session_strategy, &plan) {
                slash_model::DaemonModelSwitchDecision::Apply(strategy) => strategy,
                slash_model::DaemonModelSwitchDecision::RequestChoice(prompt) => {
                    self.chat.request_model_switch_choice(prompt);
                    return;
                }
            };

        let apply_url = format!("{base_url}/api/agents/{agent_id}/model-switch/apply");
        match client
            .post(&apply_url)
            .json(&serde_json::json!({
                "model": model_id,
                "session_strategy": strategy,
            }))
            .send()
        {
            Ok(r) if r.status().is_success() => {
                if let Ok(body) = r.json::<serde_json::Value>() {
                    let (label, message) = slash_model::daemon_apply_success(&body, model_id);
                    if let Some(label) = label {
                        self.chat.model_label = label;
                    }
                    self.chat.apply_model_context_window(model_id);
                    self.chat.push_message(Role::System, message);
                    self.refresh_active_chat_metadata();
                }
            }
            Ok(r) => self.chat.push_message(
                Role::System,
                slash_model::safe_switch_http_failed_message(r.status()),
            ),
            Err(e) => self
                .chat
                .push_message(Role::System, slash_model::safe_switch_error_message(e)),
        }
    }

    fn switch_model_inprocess(
        &mut self,
        kernel: Arc<CaptainKernel>,
        model_id: &str,
        session_strategy: Option<&str>,
    ) {
        let Some(id) = self.agent_id_inprocess else {
            return;
        };

        let plan = match kernel.plan_model_switch(id, model_id, None) {
            Ok(plan) => plan,
            Err(e) => {
                self.chat.push_message(
                    Role::System,
                    slash_model::inprocess_preflight_failed_message(e),
                );
                return;
            }
        };
        if !plan.can_apply {
            self.chat.push_message(
                Role::System,
                slash_model::model_switch_blocked_message(&plan.blocking_issues.join("\n")),
            );
            return;
        }
        let strategy =
            match slash_model::inprocess_model_switch_decision(model_id, session_strategy, &plan) {
                slash_model::InProcessModelSwitchDecision::Apply(strategy) => strategy,
                slash_model::InProcessModelSwitchDecision::RequestChoice(prompt) => {
                    self.chat.request_model_switch_choice(prompt);
                    return;
                }
            };
        match kernel.apply_model_switch(id, model_id, None, strategy) {
            Ok(result) => {
                self.chat.model_label = format!(
                    "{}/{}",
                    result.plan.target_provider, result.plan.target_model
                );
                self.chat.apply_model_context_window(model_id);
                self.chat.push_message(Role::System, result.message);
                self.refresh_active_chat_metadata();
            }
            Err(e) => self
                .chat
                .push_message(Role::System, slash_model::switch_failed_message(e)),
        }
    }

    // ── Agent resolution helpers ─────────────────────────────────────────────

    fn enter_chat_daemon(&mut self, id: String, name: String) {
        self.agent_id_daemon = Some(id.clone());
        self.daemon_session_id = None;
        self.agent_id_inprocess = None;
        self.inprocess_session_id = None;
        self.agent_name = name.clone();
        self.chat.agent_name = name;
        self.chat.mode_label = "daemon".to_string();
        self.refresh_active_chat_metadata();

        let restored_web_session = match &self.backend {
            Backend::Daemon { base_url } => {
                let base_url = base_url.clone();
                self.try_restore_web_session(&base_url, &id)
            }
            _ => false,
        };
        if !restored_web_session {
            self.chat.start_session(&format!("daemon-{id}"));
        }

        self.chat.push_message(
            Role::System,
            slash_session::chat_session_help_message().to_string(),
        );
    }

    fn enter_chat_inprocess(&mut self, id: AgentId, name: String) {
        self.agent_id_inprocess = Some(id);
        self.inprocess_session_id = None;
        self.agent_id_daemon = None;
        self.daemon_session_id = None;
        self.agent_name = name.clone();
        self.chat.agent_name = name;
        self.chat.mode_label = "in-process".to_string();
        self.chat.start_session(&format!("inprocess-{}", id.0));

        if let Backend::InProcess { ref kernel } = self.backend {
            if let Some(entry) = kernel.registry.get(id) {
                self.chat.model_label = format!(
                    "{}/{}",
                    entry.manifest.model.provider, entry.manifest.model.model
                );
            }
        }

        self.chat.push_message(
            Role::System,
            slash_session::chat_session_help_message().to_string(),
        );
    }

    /// Resolve agent on daemon: find by name/id, or auto-spawn from template.
    fn resolve_daemon_agent(&mut self, base_url: &str, agent_name: Option<&str>) {
        let client = crate::daemon_client();
        let body = crate::daemon_json(client.get(format!("{base_url}/api/agents")).send());
        let agents = body.as_array();

        // Try to find by name/id
        let found = match agent_name {
            Some(name_or_id) => agents.and_then(|arr| {
                arr.iter().find(|a| {
                    a["name"].as_str() == Some(name_or_id) || a["id"].as_str() == Some(name_or_id)
                })
            }),
            None => agents.and_then(|arr| {
                arr.iter()
                    .find(|a| {
                        a["name"]
                            .as_str()
                            .map(|name| name.eq_ignore_ascii_case("captain"))
                            .unwrap_or(false)
                    })
                    .or_else(|| arr.first())
            }),
        };

        if let Some(agent) = found {
            let id = agent["id"].as_str().unwrap_or("").to_string();
            let name = agent["name"].as_str().unwrap_or("agent").to_string();
            self.backend = Backend::Daemon {
                base_url: base_url.to_string(),
            };
            self.start_provider_quota_watch();
            self.enter_chat_daemon(id, name);
            return;
        }

        // Auto-spawn from template
        let target_name = agent_name.unwrap_or("assistant");
        let all_templates = crate::templates::load_all_templates();
        let template = all_templates
            .iter()
            .find(|t| t.name == target_name)
            .or_else(|| all_templates.first());

        match template {
            Some(t) => {
                self.backend = Backend::Daemon {
                    base_url: base_url.to_string(),
                };
                self.start_provider_quota_watch();
                event::spawn_daemon_agent(
                    base_url.to_string(),
                    t.content.clone(),
                    self.event_tx.clone(),
                );
                self.chat.status_msg = Some(slash_standalone::spawning_agent_message(&t.name));
            }
            None => {
                self.boot_error = Some(slash_standalone::no_agent_templates_message().to_string());
            }
        }
    }

    fn try_restore_web_session(&mut self, base_url: &str, agent_id: &str) -> bool {
        let Some(session_id) = web_resume_session_id() else {
            return false;
        };

        let client = crate::daemon_client();
        let detail_resp = client
            .get(format!("{base_url}/api/sessions/{session_id}"))
            .send();
        let detail = match detail_resp {
            Ok(resp) if resp.status().is_success() => match resp.json::<serde_json::Value>() {
                Ok(body) => body,
                Err(e) => {
                    self.chat.push_message(
                        Role::System,
                        format!("Impossible de relire la session {session_id}: {e}"),
                    );
                    return false;
                }
            },
            Ok(resp) => {
                self.chat.push_message(
                    Role::System,
                    format!(
                        "Impossible de relire la session {session_id}: HTTP {}",
                        resp.status()
                    ),
                );
                return false;
            }
            Err(e) => {
                self.chat.push_message(
                    Role::System,
                    format!("Impossible de relire la session {session_id}: {e}"),
                );
                return false;
            }
        };

        let owner_id = match persisted_session_owner(&detail) {
            Ok(owner_id) => owner_id.to_string(),
            Err(error) => {
                self.chat.push_message(Role::System, error);
                return false;
            }
        };
        let owner_name = daemon_agent_name(&client, base_url, &owner_id).unwrap_or_else(|| {
            if owner_id == agent_id {
                self.agent_name.clone()
            } else {
                owner_id.clone()
            }
        });

        self.agent_id_daemon = Some(owner_id.clone());
        self.agent_name = owner_name.clone();
        self.chat.agent_name = owner_name;
        self.refresh_active_chat_metadata();

        self.daemon_session_id = Some(session_id.clone());
        let short = session_id.get(..8).unwrap_or(session_id.as_str());
        self.chat
            .start_session(&format!("daemon-{owner_id}-web-{short}"));
        self.chat.bind_authoritative_session(&owner_id, &session_id);
        let restored = restore_public_session_messages(&mut self.chat, &detail);
        let label = public_session_label(&detail);
        self.chat.push_message(
            Role::System,
            format!("Session restaurée: {label} ({restored} messages)."),
        );
        true
    }

    /// Resolve agent in-process: find existing or spawn from template.
    fn resolve_inprocess_agent(&mut self) {
        let kernel = match &self.backend {
            Backend::InProcess { kernel } => kernel.clone(),
            _ => return,
        };

        // Check for existing agents
        let existing = kernel.registry.list();
        if let Some(entry) = existing
            .iter()
            .find(|e| self.agent_name.is_empty() || e.name == self.agent_name)
        {
            self.enter_chat_inprocess(entry.id, entry.name.clone());
            return;
        }

        // Spawn from template
        let target_name = if self.agent_name.is_empty() {
            "assistant"
        } else {
            &self.agent_name
        };
        let all_templates = crate::templates::load_all_templates();
        let template = all_templates
            .iter()
            .find(|t| t.name == target_name)
            .or_else(|| all_templates.iter().find(|t| t.name == "assistant"))
            .or_else(|| all_templates.first());

        match template {
            Some(t) => {
                let manifest: captain_types::agent::AgentManifest = match toml::from_str(&t.content)
                {
                    Ok(m) => m,
                    Err(e) => {
                        self.chat.status_msg =
                            Some(slash_standalone::invalid_template_message(&t.name, e));
                        return;
                    }
                };
                let name = manifest.name.clone();
                match kernel.spawn_agent(manifest) {
                    Ok(id) => {
                        self.enter_chat_inprocess(id, name);
                    }
                    Err(e) => {
                        self.chat.status_msg =
                            Some(slash_standalone::inprocess_agent_spawn_failed_message(e));
                    }
                }
            }
            None => {
                self.chat.status_msg =
                    Some(slash_standalone::no_agent_templates_message().to_string());
            }
        }
    }

    fn backend_is_none(&self) -> bool {
        matches!(self.backend, Backend::None)
    }

    fn backend_ref(&self) -> Option<event::BackendRef> {
        match &self.backend {
            Backend::Daemon { base_url } => Some(event::BackendRef::Daemon(base_url.clone())),
            Backend::InProcess { kernel } => Some(event::BackendRef::InProcess(Arc::clone(kernel))),
            Backend::None => None,
        }
    }

    fn start_provider_quota_watch(&mut self) {
        if self.provider_quota_watch_started {
            return;
        }
        let Some(backend) = self.backend_ref() else {
            return;
        };
        self.provider_quota_watch_started = true;
        event::spawn_provider_quota_watch(backend, self.event_tx.clone());
    }

    // ── Drawing ──────────────────────────────────────────────────────────────

    fn draw(&mut self, frame: &mut ratatui::Frame) {
        let area = frame.area();

        if self.booting {
            self.draw_booting(frame, area);
        } else if let Some(ref err) = self.boot_error {
            self.draw_error(frame, area, err);
        } else {
            // Standalone chat has no /image command, so no image previews
            // are ever staged. We pass a fresh cache that will stay empty
            // (zero-cost) rather than wiring a long-lived field for a
            // feature this entry-point doesn't expose.
            let mut cache = crate::tui::image_preview::ImagePreviewCache::new();
            self.chat.mouse_capture_enabled = self.mouse_capture_enabled;
            chat::draw(frame, area, &mut self.chat, &mut cache);
        }
    }

    fn draw_booting(&self, frame: &mut ratatui::Frame, area: Rect) {
        let spinner = theme::SPINNER_FRAMES[self.spinner_frame];

        let chunks = Layout::vertical([
            Constraint::Percentage(40),
            Constraint::Length(3),
            Constraint::Min(0),
        ])
        .split(area);

        let lines = vec![
            Line::from(vec![
                Span::styled(format!(" {spinner} "), Style::default().fg(theme::ACCENT)),
                Span::styled(
                    "Booting kernel\u{2026}",
                    Style::default().fg(theme::TEXT_PRIMARY),
                ),
            ]),
            Line::from(""),
            Line::from(vec![Span::styled(
                "  This may take a moment while the kernel initializes.",
                theme::dim_style(),
            )]),
        ];

        let para = Paragraph::new(lines).alignment(Alignment::Center);
        frame.render_widget(para, chunks[1]);
    }

    fn draw_error(&self, frame: &mut ratatui::Frame, area: Rect, err: &str) {
        let chunks = Layout::vertical([
            Constraint::Percentage(35),
            Constraint::Length(5),
            Constraint::Min(0),
        ])
        .split(area);

        let lines = vec![
            Line::from(vec![
                Span::styled(" \u{2718} ", Style::default().fg(theme::RED)),
                Span::styled("Failed to start", Style::default().fg(theme::RED)),
            ]),
            Line::from(""),
            Line::from(vec![Span::styled(
                format!("  {err}"),
                Style::default().fg(theme::TEXT_SECONDARY),
            )]),
            Line::from(""),
            Line::from(vec![Span::styled(
                "  Press Esc to exit.",
                theme::hint_style(),
            )]),
        ];

        let para = Paragraph::new(lines).alignment(Alignment::Center);
        frame.render_widget(para, chunks[1]);
    }
}

fn persisted_session_owner(detail: &serde_json::Value) -> Result<&str, String> {
    detail
        .get("agent_id")
        .and_then(serde_json::Value::as_str)
        .filter(|owner| uuid::Uuid::parse_str(owner).is_ok())
        .ok_or_else(|| "Session persistée ignorée: propriétaire invalide.".to_string())
}

fn daemon_agent_name(
    client: &reqwest::blocking::Client,
    base_url: &str,
    agent_id: &str,
) -> Option<String> {
    let response = client.get(format!("{base_url}/api/agents")).send().ok()?;
    let body = response.json::<serde_json::Value>().ok()?;
    body.as_array()?
        .iter()
        .find(|agent| agent.get("id").and_then(serde_json::Value::as_str) == Some(agent_id))
        .and_then(|agent| agent.get("name").and_then(serde_json::Value::as_str))
        .map(str::to_string)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn standalone_chat() -> StandaloneChat {
        let (tx, _rx) = mpsc::channel();
        StandaloneChat::new(tx)
    }

    fn last_system_message_text(state: &StandaloneChat) -> &str {
        let msg = state.chat.messages.last().expect("system message pushed");
        assert!(matches!(msg.role, Role::System));
        &msg.text
    }

    #[test]
    fn provider_quota_event_updates_chat_and_keeps_last_good_observation() {
        let mut state = standalone_chat();
        let status = crate::tui::provider_quota::ProviderQuotaStatus {
            state: "warning".to_string(),
            reported_by_provider: true,
            quotas: vec![crate::tui::provider_quota::ProviderQuota {
                provider: "codex".to_string(),
                limit_id: "codex".to_string(),
                limit_name: "Codex".to_string(),
                ..Default::default()
            }],
        };

        state.handle_event(AppEvent::ProviderQuotasLoaded(Ok(status.clone())));
        assert_eq!(state.chat.provider_quota_status, status);

        state.handle_event(AppEvent::ProviderQuotasLoaded(Err(
            "daemon restarting".to_string()
        )));
        assert_eq!(state.chat.provider_quota_status, status);
    }

    #[test]
    fn ask_user_with_options_opens_modal_instead_of_a_message() {
        let mut state = standalone_chat();

        state.handle_stream(StreamEvent::AskUser {
            question: "Couleur ?".to_string(),
            options: Some(vec!["bleu".to_string(), "rouge".to_string()]),
        });

        let pending = state
            .chat
            .pending_ask_user
            .as_ref()
            .expect("pending ask_user");
        assert_eq!(pending.question, "Couleur ?");
        assert_eq!(pending.options, vec!["bleu", "rouge"]);
        assert!(state.chat.messages.is_empty());
    }

    #[test]
    fn ask_user_without_options_pushes_a_plain_message() {
        let mut state = standalone_chat();

        state.handle_stream(StreamEvent::AskUser {
            question: "Continue ?".to_string(),
            options: None,
        });

        assert!(state.chat.pending_ask_user.is_none());
        assert_eq!(
            state.chat.messages.last().map(|m| m.text.as_str()),
            Some("❓ Continue ?")
        );
    }

    #[test]
    fn answer_ask_user_is_a_no_op_without_a_connected_backend() {
        // Same rationale as tui/mod.rs's equivalent test: in-process and
        // daemon dispatch both need a live backend (verified manually via
        // tmux/curl in T4) — this covers the one branch testable in
        // isolation, Backend::None, which must never panic.
        let mut state = standalone_chat();
        assert!(matches!(state.backend, Backend::None));

        state.handle_chat_action(ChatAction::AnswerAskUser("bleu".to_string()));

        assert!(state.current_stream_input_tx.is_none());
    }

    fn spawn_sheet_with_token() -> crate::agent_api_sheet::AgentApiSpawnSheet {
        crate::agent_api_sheet::AgentApiSpawnSheet::from_spawn_body(&serde_json::json!({
            "agent_api_provisioning": {
                "protocol": "agent-as-service.v1",
                "status": "ingress_ready",
                "manifest_url": "/api/agents/a1/api/manifest",
                "audit_events_url": "/api/agents/a1/api/events",
                "ingress": {
                    "status": "ready",
                    "ingress_url": "/hooks/agents/a1/ingress",
                    "auth_scheme": "Authorization: Bearer $TOKEN",
                    "token_env": "CAPTAIN_AGENT_A1_TOKEN",
                    "token": "secret-token"
                },
                "egress": {
                    "status": "pending_callback_url",
                    "configure_url": "/api/agents/a1/api/egress/configure",
                    "test_url": "/api/agents/a1/api/egress/test",
                    "queue_status_url": "/api/agents/a1/api/egress/queue"
                },
                "operator_actions": [
                    "Ingress is ready, but Captain cannot infer the external callback URL."
                ]
            }
        }))
        .expect("sheet")
    }

    #[test]
    fn daemon_spawn_notice_is_visible_but_not_persisted_in_chat_history() {
        let mut state = standalone_chat();

        state.handle_agent_spawned(
            "a1".to_string(),
            "veille".to_string(),
            Some(spawn_sheet_with_token()),
        );

        assert!(state
            .chat
            .operator_notices
            .iter()
            .any(|line| line.contains("secret-token")));
        assert!(!state
            .chat
            .messages
            .iter()
            .any(|message| message.text.contains("secret-token")));
        assert_eq!(state.agent_name, "veille");
    }

    #[test]
    fn restore_public_session_messages_filters_empty_entries() {
        let mut state = standalone_chat();
        state.chat.push_message(Role::System, "old".to_string());
        state.chat.scroll_offset = 7;
        let detail = serde_json::json!({
            "label": " Web session ",
            "context_window_tokens": 272000,
            "estimated_context_tokens": 1234,
            "messages": [
                {"role": "user", "content": "hello"},
                {"role": "assistant", "content": "   "},
                {
                    "role": "assistant",
                    "content": "done",
                    "images": [{"media_type": "image/png"}]
                },
                {
                    "role": "tool",
                    "tools": [{"name": "read_file", "result": {"ok": true}}]
                }
            ]
        });

        let restored = restore_public_session_messages(&mut state.chat, &detail);

        assert_eq!(restored, 3);
        assert_eq!(state.chat.scroll_offset, 0);
        assert_eq!(state.chat.messages.len(), 3);
        assert!(matches!(state.chat.messages[0].role, Role::User));
        assert!(matches!(state.chat.messages[1].role, Role::Agent));
        assert!(matches!(state.chat.messages[2].role, Role::Tool));
        assert_eq!(state.chat.messages[0].text, "hello");
        assert_eq!(state.chat.context_window_tokens, 272_000);
        assert_eq!(state.chat.current_context_tokens, 1_234);
        assert!(state.chat.messages[1].text.contains("[Image: image/png]"));
        assert!(state.chat.messages[2].text.contains("[Tool: read_file]"));
        assert_eq!(public_session_label(&detail), " Web session ");

        let empty_label = serde_json::json!({"label": "  "});
        assert_eq!(public_session_label(&empty_label), "session persistée");
    }

    #[test]
    fn web_resume_is_session_scoped_instead_of_switching_the_agent() {
        let source = include_str!("chat_runner.rs");
        let global_switch_route = ["/sessions/", "{session_id}", "/switch"].concat();
        assert!(source.contains("self.daemon_session_id = Some(session_id.clone())"));
        assert!(source.contains("self.daemon_session_id.clone(),"));
        assert!(
            !source.contains(&global_switch_route),
            "restoring one Web tab must not globally switch the agent"
        );
    }

    #[test]
    fn web_resume_uses_the_persisted_session_owner() {
        let owner = uuid::Uuid::new_v4();
        let detail = serde_json::json!({"agent_id": owner.to_string()});

        assert_eq!(persisted_session_owner(&detail).unwrap(), owner.to_string());
        assert!(persisted_session_owner(&serde_json::json!({"agent_id": "captain"})).is_err());
    }

    #[test]
    fn canonical_history_opens_without_a_local_json_snapshot() {
        let mut state = standalone_chat();
        state.chat.show_session_picker = true;
        let session_id = uuid::Uuid::new_v4().to_string();

        state.handle_event(AppEvent::SessionsLoaded(vec![
            crate::tui::screens::sessions::SessionInfo {
                id: session_id.clone(),
                label: "Web-only session".to_string(),
                agent_name: "captain".to_string(),
                agent_id: uuid::Uuid::new_v4().to_string(),
                message_count: 2,
                created: "2026-07-13T10:00:00Z".to_string(),
            },
        ]));

        assert!(state.chat.show_session_picker);
        assert_eq!(
            state.chat.session_picker_items[0].session_id.as_deref(),
            Some(session_id.as_str())
        );
    }

    #[test]
    fn status_slash_reports_disconnected_without_backend() {
        let mut state = standalone_chat();

        state.handle_status_slash(crate::i18n::Lang::En);

        assert_eq!(last_system_message_text(&state), "Mode: disconnected");
    }

    #[test]
    fn sessions_slash_reports_not_connected_without_backend() {
        let mut state = standalone_chat();

        state.handle_sessions_slash(crate::i18n::Lang::En);

        assert_eq!(last_system_message_text(&state), "Not connected.");
    }

    #[test]
    fn agents_slash_reports_empty_without_backend() {
        let mut state = standalone_chat();

        state.handle_agents_slash(crate::i18n::Lang::En);

        assert_eq!(last_system_message_text(&state), "No agents running.");
    }

    #[test]
    fn new_slash_reports_reset_failure_without_backend() {
        let mut state = standalone_chat();

        state.handle_new_slash(crate::i18n::Lang::En);

        assert_eq!(
            last_system_message_text(&state),
            slash_session::reset_session_failed_message(
                crate::i18n::Lang::En,
                slash_session::reset_no_backend_connected_message()
            )
        );
    }

    #[test]
    fn copy_slash_reports_usage_for_unknown_target() {
        let mut state = standalone_chat();

        state.handle_copy_slash("clipboard", crate::i18n::Lang::En);

        assert_eq!(
            last_system_message_text(&state),
            slash_local::copy_usage_message(
                crate::i18n::Lang::En,
                slash_local::CopyUsageSurface::StandaloneChat,
            )
        );
    }

    #[test]
    fn mouse_slash_reports_usage_for_unknown_target() {
        let mut state = standalone_chat();

        state.handle_mouse_slash("maybe", crate::i18n::Lang::En);

        assert_eq!(
            last_system_message_text(&state),
            slash_local::mouse_usage_message(
                crate::i18n::Lang::En,
                slash_local::MouseMessageSurface::StandaloneChat,
            )
        );
    }

    #[test]
    fn reload_slash_reports_missing_session_without_active_key() {
        let mut state = standalone_chat();

        state.handle_reload_session_slash(crate::i18n::Lang::En);

        assert_eq!(
            last_system_message_text(&state),
            slash_reload::no_active_session_message(crate::i18n::Lang::En)
        );
    }

    #[test]
    fn retry_empty_slash_reports_no_target() {
        let mut state = standalone_chat();

        state.handle_retry_empty_slash(crate::i18n::Lang::En);

        assert_eq!(
            last_system_message_text(&state),
            slash_retry::retry_nothing_message(crate::i18n::Lang::En)
        );
    }

    #[test]
    fn undo_empty_slash_reports_no_user_message() {
        let mut state = standalone_chat();

        state.handle_undo_slash(crate::i18n::Lang::En);

        assert_eq!(
            last_system_message_text(&state),
            slash_local::undo_result_message(false, crate::i18n::Lang::En)
        );
    }

    #[test]
    fn queue_empty_slash_reports_empty_queue() {
        let mut state = standalone_chat();

        state.handle_queue_slash(crate::i18n::Lang::En);

        assert_eq!(
            last_system_message_text(&state),
            slash_local::queue_message_for_lang(&[], crate::i18n::Lang::En)
        );
    }

    #[test]
    fn standalone_unavailable_slashes_use_shared_messages() {
        let mut state = standalone_chat();

        state.handle_attachments_voice_slash(crate::i18n::Lang::En);
        assert_eq!(
            last_system_message_text(&state),
            slash_standalone::attachments_voice_message(crate::i18n::Lang::En)
        );

        state.handle_feedback_unavailable_slash(crate::i18n::Lang::En);
        assert_eq!(
            last_system_message_text(&state),
            slash_standalone::feedback_unavailable_message(crate::i18n::Lang::En)
        );
    }

    #[test]
    fn standalone_navigation_and_unknown_slashes_use_shared_messages() {
        let mut state = standalone_chat();

        state.handle_full_tui_navigation_slash(crate::i18n::Lang::En, "/projects");
        assert_eq!(
            last_system_message_text(&state),
            slash_standalone::full_tui_navigation_message(crate::i18n::Lang::En, "/projects")
        );

        state.handle_unknown_slash(crate::i18n::Lang::En, "/wat");
        assert_eq!(
            last_system_message_text(&state),
            slash_standalone::unknown_slash_message(crate::i18n::Lang::En, "/wat")
        );
    }

    #[test]
    fn kill_slash_protects_captain_agent() {
        let mut state = standalone_chat();
        state.agent_name = " Captain ".to_string();

        state.handle_kill_slash(crate::i18n::Lang::En);

        assert_eq!(
            last_system_message_text(&state),
            slash_kill::protected_agent_message(crate::i18n::Lang::En)
        );
        assert!(!state.should_quit);
    }

    #[test]
    fn kill_slash_reports_no_backend_for_unprotected_agent() {
        let mut state = standalone_chat();
        state.agent_name = "worker".to_string();

        state.handle_kill_slash(crate::i18n::Lang::En);

        assert_eq!(
            last_system_message_text(&state),
            slash_kill::no_backend_message(crate::i18n::Lang::En)
        );
        assert!(!state.should_quit);
    }

    #[test]
    fn switch_model_reports_no_backend_without_connection() {
        let mut state = standalone_chat();

        state.switch_model("codex/gpt-5.5", None);

        assert_eq!(
            last_system_message_text(&state),
            slash_model::no_backend_connected_message()
        );
    }

    #[test]
    fn common_slash_commands_are_handled_before_local_dispatch() {
        let mut state = standalone_chat();

        assert!(state.handle_common_slash_command("/exit", "", "/exit"));
        assert!(state.should_quit);
        assert!(!state.handle_common_slash_command("/help", "", "/help"));
    }
}

// ── Public entry point ───────────────────────────────────────────────────────

/// Launch the standalone chat TUI.
///
/// - If a daemon is running, connects to it and resolves the agent.
/// - Otherwise, boots the kernel in-process.
pub fn run_chat_tui(config: Option<PathBuf>, agent_name: Option<String>) {
    use ratatui::crossterm::event::{
        DisableBracketedPaste, DisableMouseCapture, EnableBracketedPaste,
    };
    use ratatui::crossterm::execute;

    let original_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let _ = execute!(std::io::stdout(), DisableMouseCapture);
        let _ = execute!(std::io::stdout(), DisableBracketedPaste);
        ratatui::restore();
        original_hook(info);
    }));

    let mut terminal = ratatui::init();
    let _ = execute!(std::io::stdout(), EnableBracketedPaste);
    let mouse_capture_enabled = event::standalone_chat_mouse_capture_default();
    let _ = event::set_mouse_capture(mouse_capture_enabled);

    let (tx, rx) = event::spawn_event_thread(Duration::from_millis(50));
    let mut state = StandaloneChat::new(tx.clone());
    state.mouse_capture_enabled = mouse_capture_enabled;

    // Store the requested agent name for later resolution
    if let Some(ref name) = agent_name {
        state.agent_name = name.clone();
    }

    // Boot sequence: check for daemon, or boot kernel in-process
    if let Some(base_url) = crate::find_daemon() {
        state.resolve_daemon_agent(&base_url, agent_name.as_deref());
    } else {
        state.booting = true;
        event::spawn_kernel_boot(config, tx);
    }

    // ── Main loop ────────────────────────────────────────────────────────────
    while !state.should_quit {
        terminal
            .draw(|frame| state.draw(frame))
            .expect("Failed to draw");

        match rx.recv_timeout(Duration::from_millis(33)) {
            Ok(ev) => state.handle_event(ev),
            Err(mpsc::RecvTimeoutError::Timeout) => {}
            Err(mpsc::RecvTimeoutError::Disconnected) => break,
        }
        // Drain queued events
        while let Ok(ev) = rx.try_recv() {
            state.handle_event(ev);
        }
    }

    let _ = event::set_mouse_capture(false);
    let _ = execute!(std::io::stdout(), DisableBracketedPaste);
    ratatui::restore();
}
