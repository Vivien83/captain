//! Chat screen: scrollable message history, streaming output, tool spinners, input.

#[cfg(test)]
use super::chat_tool_message::render_tool_message;
use super::{
    chat_input_layout::locate_cursor,
    chat_keymap::{
        global_key_action_for_key, input_key_action_for_key, streaming_key_action_for_key,
        GlobalKeyAction, InputKeyAction, StreamingKeyAction,
    },
    chat_markdown_export, chat_model_label,
    chat_model_picker::{model_picker_key_action_for_key, ModelPickerKeyAction},
    chat_quick_action_prompt::{
        approval_quick_action_choice_for_key, ask_user_quick_action_choice_for_key,
        model_switch_quick_action_for_key, ModelSwitchQuickActionKey, MODEL_SWITCH_INVALID_REPLY,
    },
    chat_screen_render::draw_chat_screen,
    chat_session_picker::{session_picker_key_action_for_key, SessionPickerKeyAction},
    chat_session_replay,
    chat_slash_picker::{self, slash_picker_key_action_for_key, SlashPickerKeyAction},
    chat_tool_message::should_render_tool_expanded,
};
use crate::tui::theme;
use ratatui::crossterm::event::KeyEvent;
#[cfg(test)]
use ratatui::crossterm::event::{KeyCode, KeyModifiers};
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
#[cfg(test)]
use ratatui::text::Line;
use ratatui::Frame;

const HISTORY_LINE_SCROLL: u16 = 1;
#[allow(dead_code)]
const HISTORY_WHEEL_SCROLL: u16 = 5;
const HISTORY_PAGE_SCROLL: u16 = 12;

/// Phase-i.8: a file already uploaded to the daemon, waiting to be attached
/// to the next message. The CLI keeps the original filename so it can be
/// shown in the staged-attachment hint.
#[derive(Clone)]
pub struct PendingAttachment {
    pub file_id: String,
    pub filename: String,
    pub content_type: String,
    /// TUI polish — the original on-disk path that produced the upload.
    /// Kept locally so the chat can render an inline preview via
    /// `tui::image_preview` without round-tripping through the daemon.
    /// `None` when the attachment came in via a path-less code path
    /// (paste / drag / programmatic).
    pub local_path: Option<std::path::PathBuf>,
}

/// Model entry for the picker.
#[derive(Clone)]
pub struct ModelEntry {
    pub id: String,
    pub display_name: String,
    pub provider: String,
    pub tier: String,
}

/// Tool call metadata for rich rendering.
#[derive(Clone)]
pub struct ToolInfo {
    pub id: String,
    pub name: String,
    pub input: String,
    pub result: String,
    pub stdout: String,
    pub stderr: String,
    pub is_error: bool,
    pub status: ToolStatus,
    pub started_at: Option<std::time::Instant>,
    pub completed_at: Option<std::time::Instant>,
    pub duration_ms: Option<u64>,
    pub expanded: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ToolStatus {
    Running,
    Success,
    Error,
}

#[derive(Clone, Copy, Debug)]
pub struct ToolClickZone {
    pub x_start: u16,
    pub x_end: u16,
    pub y: u16,
    pub message_idx: usize,
    pub action: ToolClickAction,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ToolClickAction {
    Toggle,
    CopyCommand,
}

#[derive(Clone, Debug)]
pub struct PendingModelSwitch {
    pub model_id: String,
    pub current_provider: String,
    pub current_model: String,
    pub target_provider: String,
    pub target_model: String,
    pub risk: String,
    pub recommended_session_strategy: String,
    pub active_message_count: usize,
    pub canonical_summary_present: bool,
}

/// An `ask_user` tool call with predefined options, surfaced as a modal
/// popup (same pattern as `pending_approval`) instead of plain chat text —
/// only set when `options` is non-empty; a question without options stays
/// a regular chat message so free-text keyboard input keeps working
/// unchanged.
#[derive(Clone, Debug)]
pub struct PendingAskUser {
    pub question: String,
    pub options: Vec<String>,
}

#[derive(Clone, Copy, Debug)]
pub struct QuickActionClickZone {
    pub x_start: u16,
    pub x_end: u16,
    pub y: u16,
    pub choice: QuickActionChoiceId,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ModelSwitchChoice {
    NewSession,
    CompactSession,
    Cancel,
}

impl ModelSwitchChoice {
    fn strategy(self) -> Option<&'static str> {
        match self {
            Self::NewSession => Some("new_session"),
            Self::CompactSession => Some("compact_session"),
            Self::Cancel => None,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum QuickActionChoiceId {
    ModelSwitchNewSession,
    ModelSwitchCompactSession,
    ModelSwitchCancel,
    ApprovalOnce,
    ApprovalSession,
    ApprovalAlways,
    ApprovalReject,
    /// Index into `PendingAskUser::options`.
    AskUserOption(usize),
}

impl QuickActionChoiceId {
    pub(super) fn from_model_switch(choice: ModelSwitchChoice) -> Self {
        match choice {
            ModelSwitchChoice::NewSession => Self::ModelSwitchNewSession,
            ModelSwitchChoice::CompactSession => Self::ModelSwitchCompactSession,
            ModelSwitchChoice::Cancel => Self::ModelSwitchCancel,
        }
    }

    fn as_model_switch(self) -> Option<ModelSwitchChoice> {
        match self {
            Self::ModelSwitchNewSession => Some(ModelSwitchChoice::NewSession),
            Self::ModelSwitchCompactSession => Some(ModelSwitchChoice::CompactSession),
            Self::ModelSwitchCancel => Some(ModelSwitchChoice::Cancel),
            _ => None,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ChatMouseAction {
    ToolToggled,
    CopyCommand(String),
    ApplyModelSwitch {
        model_id: String,
        session_strategy: String,
    },
    ModelSwitchCancelled,
    ApproveRequest(String),
    ApproveSessionRequest(String),
    ApproveAlwaysRequest(String),
    RejectRequest(String),
}

/// A single message in the chat history.
#[derive(Clone)]
pub struct ChatMessage {
    pub role: Role,
    pub text: String,
    pub tool: Option<ToolInfo>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Role {
    User,
    Agent,
    System,
    Tool,
}

pub struct ChatState {
    /// Agent display name.
    pub agent_name: String,
    /// Provider/model for the title bar.
    pub model_label: String,
    /// Connection mode label.
    pub mode_label: String,
    /// Full chat history.
    pub messages: Vec<ChatMessage>,
    /// Operator-only live notices. These are rendered but never persisted in
    /// chat history, so one-shot tokens can be shown without entering replay.
    pub operator_notices: Vec<String>,
    /// Current streaming text being accumulated.
    pub streaming_text: String,
    /// Whether we are currently streaming.
    pub is_streaming: bool,
    /// Waiting for first token (shows "thinking..." spinner).
    pub thinking: bool,
    /// Current tool being executed (spinner).
    pub active_tool: Option<String>,
    /// Spinner frame index.
    pub spinner_frame: usize,
    /// Input line buffer.
    pub input: String,
    /// Byte index of the editing cursor inside `input`. Always lands on a
    /// UTF-8 char boundary. `0 ≤ input_cursor ≤ input.len()`.
    /// Default is the end of the buffer so legacy code paths that don't
    /// track the cursor stay backward-compatible.
    pub input_cursor: usize,
    /// Scroll offset (lines from the bottom).
    pub scroll_offset: u16,
    /// Screen-space click targets for rendered tool headers.
    pub tool_click_zones: Vec<ToolClickZone>,
    /// Token usage from last response.
    pub last_tokens: Option<(u64, u64)>,
    /// Cached input tokens reported by the provider for the last response.
    pub last_cached_input_tokens: u64,
    /// Cache creation/write tokens reported by the provider for the last response.
    pub last_cache_creation_tokens: u64,
    /// Cost in USD from last response.
    pub last_cost_usd: Option<f64>,
    /// Phase-g.2: wall-clock start of the current chat session, set on the
    /// first user message so the status bar can display elapsed duration.
    pub session_start: Option<std::time::Instant>,
    /// Phase-i.7: separate accumulator for ThinkingDelta tokens so they don't
    /// pollute the agent's actual response. Rendered as a collapsible block.
    pub thinking_text: String,
    /// Phase-i.7: when true, the thinking block is shown expanded; collapsed
    /// otherwise (default).
    pub thinking_expanded: bool,
    /// Phase-i.8: file_ids of attachments uploaded via /image, attached to the
    /// next outgoing message and cleared on send.
    pub pending_attachments: Vec<PendingAttachment>,
    /// Phase-i.6: an approval request the agent is waiting on, surfaced as a
    /// modal popup over the chat. None when no decision is pending.
    pub pending_approval: Option<crate::tui::screens::approvals::ApprovalRequest>,
    /// A safe model/provider switch that needs an explicit user session choice.
    pub pending_model_switch: Option<PendingModelSwitch>,
    /// An `ask_user` question with options, surfaced as a modal popup. None
    /// when no question is pending, or when the current question has no
    /// options (those stay a plain chat message instead).
    pub pending_ask_user: Option<PendingAskUser>,
    /// Screen-space click targets for the active quick-action prompt.
    pub quick_action_click_zones: Vec<QuickActionClickZone>,
    /// Mirrors the app-level terminal mouse capture state. When false,
    /// clickable affordances are hidden so native terminal selection/copy
    /// remains honest and no dead "buttons" are shown.
    pub mouse_capture_enabled: bool,
    /// Characters received during current stream (~4 chars ≈ 1 token).
    pub streaming_chars: usize,
    /// Status message (errors, etc.)
    pub status_msg: Option<String>,
    /// Messages staged while the agent is streaming — sent automatically when done.
    pub staged_messages: Vec<String>,
    /// Accumulates ToolInputDelta text for the current tool call.
    pub tool_input_buf: String,
    /// Model picker overlay state.
    pub show_model_picker: bool,
    /// Available models for the picker.
    pub model_picker_models: Vec<ModelEntry>,
    /// Filter text for model search.
    pub model_picker_filter: String,
    /// Selected index in the filtered model list.
    pub model_picker_idx: usize,
    /// Phase L.3: clé filesystem-safe pour sauvegarder cette session.
    /// Set par enter_chat_*; vide → pas de persistance.
    pub session_key: String,
    /// Authoritative session shared by every Captain surface.
    pub authoritative_session_id: Option<String>,
    /// Owner of the authoritative session.
    pub authoritative_agent_id: Option<String>,
    /// Phase L.3: chemin du fichier de session courant. None tant qu'on
    /// n'a pas encore écrit la première sauvegarde.
    pub session_path: Option<std::path::PathBuf>,
    /// Phase L.3: timestamp UNIX à la création de la session courante.
    pub session_created_at: u64,
    /// Phase L.3: cumul tokens input depuis le début de la session.
    pub session_input_tokens: u64,
    /// Phase L.3: cumul tokens output depuis le début de la session.
    pub session_output_tokens: u64,
    /// Cumul input tokens served from provider prompt cache.
    pub session_cached_input_tokens: u64,
    /// Cumul provider cache creation/write tokens.
    pub session_cache_creation_tokens: u64,
    /// Phase L.3: cumul coût USD depuis le début de la session.
    pub session_cost_usd: f64,
    /// Phase L.4: overlay session picker (Ctrl+O).
    pub show_session_picker: bool,
    /// Phase L.4: liste des sessions disponibles, peuplée à l'ouverture du picker.
    pub session_picker_items: Vec<crate::tui::session_store::SessionSummary>,
    /// Phase L.4: index sélectionné dans la liste du picker.
    pub session_picker_idx: usize,
    /// Phase N.2: index sélectionné dans le slash command picker live.
    /// Reset à 0 dès qu'on tape ou efface une lettre dans l'input.
    pub slash_picker_idx: usize,
    /// Sub-agents/detached tool_runs currently in flight, fed by
    /// AgentLifecycle/ToolRunStatus events over the memory-events SSE/
    /// in-process bridge. Rendered as a persistent status-line badge so the
    /// user can see Captain is waiting on background work even after it
    /// leaves the current turn's transcript.
    pub background_activity: Vec<BackgroundActivityEntry>,
}

/// One in-flight background item (spawned sub-agent or detached tool_run).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BackgroundActivityEntry {
    /// agent_id or run_id — used to remove the entry once it completes.
    pub key: String,
    /// Human-readable label shown in the badge (agent name or tool name).
    pub label: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ChatAction {
    Continue,
    SendMessage(String),
    Back,
    SlashCommand(String),
    /// Open the model picker (fetch models first).
    OpenModelPicker,
    /// Open the session picker and refresh it from the authoritative store.
    OpenSessionPicker,
    /// Switch to a specific model by id.
    SwitchModel(String),
    /// Apply a pending safe model switch with an explicit session strategy.
    ApplyModelSwitch {
        model_id: String,
        session_strategy: String,
    },
    /// Phase-i.6: approve a pending approval request by id (one-shot).
    ApproveRequest(String),
    /// Q.11.b.b: approve for the rest of the session (cache short-circuit).
    ApproveSessionRequest(String),
    /// Q.11.b.b: approve always (persisted into allow_always policy).
    ApproveAlwaysRequest(String),
    /// Phase-i.6: reject a pending approval request by id.
    RejectRequest(String),
    /// Answer a pending ask_user question with the chosen option's text.
    AnswerAskUser(String),
    /// Load one authoritative persisted session by UUID.
    ResumeSession(String),
}

impl ChatState {
    pub fn new() -> Self {
        Self {
            agent_name: String::new(),
            model_label: String::new(),
            mode_label: String::new(),
            messages: Vec::new(),
            operator_notices: Vec::new(),
            streaming_text: String::new(),
            is_streaming: false,
            thinking: false,
            active_tool: None,
            spinner_frame: 0,
            input: String::new(),
            input_cursor: 0,
            scroll_offset: 0,
            tool_click_zones: Vec::new(),
            last_tokens: None,
            last_cached_input_tokens: 0,
            last_cache_creation_tokens: 0,
            last_cost_usd: None,
            session_start: None,
            thinking_text: String::new(),
            thinking_expanded: false,
            pending_attachments: Vec::new(),
            pending_approval: None,
            pending_model_switch: None,
            pending_ask_user: None,
            quick_action_click_zones: Vec::new(),
            mouse_capture_enabled: false,
            streaming_chars: 0,
            status_msg: None,
            staged_messages: Vec::new(),
            tool_input_buf: String::new(),
            show_model_picker: false,
            model_picker_models: Vec::new(),
            model_picker_filter: String::new(),
            model_picker_idx: 0,
            session_key: String::new(),
            authoritative_session_id: None,
            authoritative_agent_id: None,
            session_path: None,
            session_created_at: 0,
            session_input_tokens: 0,
            session_output_tokens: 0,
            session_cached_input_tokens: 0,
            session_cache_creation_tokens: 0,
            session_cost_usd: 0.0,
            show_session_picker: false,
            session_picker_items: Vec::new(),
            session_picker_idx: 0,
            slash_picker_idx: 0,
            background_activity: Vec::new(),
        }
    }

    /// Record a sub-agent/tool_run as started (or refresh its label if the
    /// same key is already tracked — e.g. a status change before completion).
    pub fn track_background_activity(&mut self, key: String, label: String) {
        if let Some(entry) = self.background_activity.iter_mut().find(|e| e.key == key) {
            entry.label = label;
        } else {
            self.background_activity
                .push(BackgroundActivityEntry { key, label });
        }
    }

    /// Remove a sub-agent/tool_run once it reaches a terminal state.
    pub fn clear_background_activity(&mut self, key: &str) {
        self.background_activity.retain(|e| e.key != key);
    }

    pub fn reset(&mut self) {
        self.messages.clear();
        self.operator_notices.clear();
        self.streaming_text.clear();
        self.is_streaming = false;
        self.thinking = false;
        self.active_tool = None;
        self.spinner_frame = 0;
        self.input.clear();
        self.input_cursor = 0;
        self.scroll_offset = 0;
        self.tool_click_zones.clear();
        self.last_tokens = None;
        self.last_cached_input_tokens = 0;
        self.last_cache_creation_tokens = 0;
        self.last_cost_usd = None;
        self.session_start = None;
        self.thinking_text.clear();
        self.thinking_expanded = false;
        self.pending_attachments.clear();
        self.pending_approval = None;
        self.pending_model_switch = None;
        self.pending_ask_user = None;
        self.quick_action_click_zones.clear();
        self.streaming_chars = 0;
        self.status_msg = None;
        self.staged_messages.clear();
        self.tool_input_buf.clear();
        self.show_model_picker = false;
        self.model_picker_filter.clear();
        self.model_picker_idx = 0;
        self.session_key.clear();
        self.authoritative_session_id = None;
        self.authoritative_agent_id = None;
        self.session_path = None;
        self.session_created_at = 0;
        self.session_input_tokens = 0;
        self.session_output_tokens = 0;
        self.session_cached_input_tokens = 0;
        self.session_cache_creation_tokens = 0;
        self.session_cost_usd = 0.0;
        self.show_session_picker = false;
        self.session_picker_items.clear();
        self.session_picker_idx = 0;
        self.slash_picker_idx = 0;
    }

    /// Push a completed message into history.
    pub fn push_message(&mut self, role: Role, text: String) {
        if role == Role::User {
            if self.session_start.is_none() {
                self.session_start = Some(std::time::Instant::now());
            }
            // Phase-i.7: a new user turn starts a fresh thinking buffer so the
            // collapsible block doesn't show reasoning from a previous turn.
            self.thinking_text.clear();
            self.thinking_expanded = false;
        }
        self.messages.push(ChatMessage {
            role,
            text,
            tool: None,
        });
        // A local user turn means the operator is taking back the live view.
        // Agent/tool updates keep the viewport stable when the user scrolled
        // up to read history.
        if role == Role::User {
            self.scroll_to_bottom();
        }
    }

    pub fn push_operator_notice(&mut self, lines: Vec<String>) {
        self.operator_notices.extend(
            lines
                .into_iter()
                .map(|line| line.trim_end().to_string())
                .filter(|line| !line.is_empty()),
        );
    }

    pub fn scroll_history_up(&mut self, lines: u16) {
        self.scroll_offset = self.scroll_offset.saturating_add(lines);
    }

    pub fn scroll_history_down(&mut self, lines: u16) {
        self.scroll_offset = self.scroll_offset.saturating_sub(lines);
    }

    pub fn scroll_line_up(&mut self) {
        self.scroll_history_up(HISTORY_LINE_SCROLL);
    }

    pub fn scroll_line_down(&mut self) {
        self.scroll_history_down(HISTORY_LINE_SCROLL);
    }

    #[allow(dead_code)]
    pub fn scroll_wheel_up(&mut self) {
        self.scroll_history_up(HISTORY_WHEEL_SCROLL);
    }

    #[allow(dead_code)]
    pub fn scroll_wheel_down(&mut self) {
        self.scroll_history_down(HISTORY_WHEEL_SCROLL);
    }

    pub fn scroll_page_up(&mut self) {
        self.scroll_history_up(HISTORY_PAGE_SCROLL);
    }

    pub fn scroll_page_down(&mut self) {
        self.scroll_history_down(HISTORY_PAGE_SCROLL);
    }

    #[allow(dead_code)]
    pub fn scroll_to_top(&mut self) {
        self.scroll_offset = u16::MAX;
    }

    pub fn scroll_to_bottom(&mut self) {
        self.scroll_offset = 0;
    }

    pub fn request_model_switch_choice(&mut self, prompt: PendingModelSwitch) {
        self.pending_model_switch = Some(prompt);
        self.quick_action_click_zones.clear();
        self.show_model_picker = false;
        self.show_session_picker = false;
        self.status_msg = Some(
            "Choisis comment migrer la session: bouton, touche 1/2, ou reponse naturelle."
                .to_string(),
        );
    }

    /// Append streaming text delta.
    pub fn append_stream(&mut self, text: &str) {
        self.thinking = false;
        self.streaming_text.push_str(text);
        self.streaming_chars += text.len();
    }

    /// Phase-i.7: append a ThinkingDelta token to the dedicated buffer.
    /// Distinct from append_stream: thinking text is the agent's reasoning,
    /// not its visible answer, and lives in its own collapsible block.
    pub fn append_thinking(&mut self, text: &str) {
        self.thinking = true;
        self.thinking_text.push_str(text);
    }

    /// Phase-i.7: toggle the visibility of the thinking block.
    pub fn toggle_thinking(&mut self) {
        if !self.thinking_text.is_empty() {
            self.thinking_expanded = !self.thinking_expanded;
        }
    }

    pub fn handle_mouse_click(&mut self, x: u16, y: u16) -> Option<ChatMouseAction> {
        if self.has_quick_action_prompt() {
            let zone = self
                .quick_action_click_zones
                .iter()
                .find(|zone| y == zone.y && x >= zone.x_start && x <= zone.x_end)
                .copied();
            if let Some(zone) = zone {
                return self
                    .resolve_quick_action_choice(zone.choice)
                    .map(chat_action_to_mouse_action);
            }
            return None;
        }

        let zone = self
            .tool_click_zones
            .iter()
            .find(|zone| y == zone.y && x >= zone.x_start && x <= zone.x_end)
            .copied()?;

        if matches!(zone.action, ToolClickAction::CopyCommand) {
            let command = self
                .messages
                .get(zone.message_idx)
                .and_then(|msg| msg.tool.as_ref())
                .and_then(copyable_tool_command)?;
            return Some(ChatMouseAction::CopyCommand(command));
        }

        let info = self
            .messages
            .get_mut(zone.message_idx)
            .and_then(|msg| msg.tool.as_mut())?;
        if info.status == ToolStatus::Running {
            return None;
        }
        let currently_open = should_render_tool_expanded(info);
        info.expanded = !currently_open;
        if currently_open {
            info.completed_at = None;
        }
        Some(ChatMouseAction::ToolToggled)
    }

    fn resolve_model_switch_choice(&mut self, choice: ModelSwitchChoice) -> Option<ChatAction> {
        if choice == ModelSwitchChoice::Cancel {
            self.pending_model_switch = None;
            self.quick_action_click_zones.clear();
            self.input_clear();
            self.status_msg = Some("Changement de modele annule.".to_string());
            return Some(ChatAction::Continue);
        }

        let strategy = choice.strategy()?.to_string();
        let prompt = self.pending_model_switch.take()?;
        self.quick_action_click_zones.clear();
        self.input_clear();
        self.status_msg = None;
        Some(ChatAction::ApplyModelSwitch {
            model_id: prompt.model_id,
            session_strategy: strategy,
        })
    }

    pub(super) fn has_quick_action_prompt(&self) -> bool {
        self.pending_approval.is_some()
            || self.pending_model_switch.is_some()
            || self.pending_ask_user.is_some()
    }

    fn resolve_quick_action_choice(&mut self, choice: QuickActionChoiceId) -> Option<ChatAction> {
        if let Some(model_choice) = choice.as_model_switch() {
            return self.resolve_model_switch_choice(model_choice);
        }

        if let QuickActionChoiceId::AskUserOption(idx) = choice {
            let pending = self.pending_ask_user.take()?;
            self.quick_action_click_zones.clear();
            return pending
                .options
                .get(idx)
                .cloned()
                .map(ChatAction::AnswerAskUser);
        }

        let req = self.pending_approval.take()?;
        self.quick_action_click_zones.clear();
        match choice {
            QuickActionChoiceId::ApprovalOnce => Some(ChatAction::ApproveRequest(req.id)),
            QuickActionChoiceId::ApprovalSession => Some(ChatAction::ApproveSessionRequest(req.id)),
            QuickActionChoiceId::ApprovalAlways => Some(ChatAction::ApproveAlwaysRequest(req.id)),
            QuickActionChoiceId::ApprovalReject => Some(ChatAction::RejectRequest(req.id)),
            _ => None,
        }
    }

    pub fn last_command_to_copy(&self) -> Option<String> {
        self.messages
            .iter()
            .rev()
            .filter_map(|msg| msg.tool.as_ref())
            .find_map(copyable_tool_command)
    }

    pub fn toggle_latest_completed_tool(&mut self) -> bool {
        for msg in self.messages.iter_mut().rev() {
            let Some(info) = msg.tool.as_mut() else {
                continue;
            };
            if info.status == ToolStatus::Running {
                continue;
            }

            let currently_open = should_render_tool_expanded(info);
            info.expanded = !currently_open;
            if currently_open {
                info.completed_at = None;
            }
            self.status_msg = Some(if info.expanded {
                format!("Tool `{}` déplié.", info.name)
            } else {
                format!("Tool `{}` replié.", info.name)
            });
            return true;
        }

        self.status_msg = Some("Aucun tool call terminé à déplier.".to_string());
        false
    }

    /// Take the next staged message (if any) for auto-send after stream completes.
    pub fn take_staged(&mut self) -> Option<String> {
        if self.staged_messages.is_empty() {
            None
        } else {
            Some(self.staged_messages.remove(0))
        }
    }

    /// Finalize streaming: move accumulated text to history.
    pub fn finalize_stream(&mut self) {
        if !self.streaming_text.is_empty() {
            let text = sanitize_function_tags(&std::mem::take(&mut self.streaming_text));
            self.push_message(Role::Agent, text);
        }
        self.is_streaming = false;
        self.thinking = false;
        self.active_tool = None;
        self.streaming_chars = 0;
        self.tool_input_buf.clear();
    }

    /// Set a tool as active (spinner) and clear the input accumulator.
    pub fn tool_start(&mut self, _id: &str, name: &str) {
        self.active_tool = Some(name.to_string());
        self.tool_input_buf.clear();
        self.spinner_frame = 0;
    }

    /// A tool_use block is complete — push a "running" tool message with input.
    pub fn tool_use_end(&mut self, id: &str, name: &str, input: &str) {
        self.messages.push(ChatMessage {
            role: Role::Tool,
            text: name.to_string(),
            tool: Some(ToolInfo {
                id: id.to_string(),
                name: name.to_string(),
                input: input.to_string(),
                result: String::new(),
                stdout: String::new(),
                stderr: String::new(),
                is_error: false,
                status: ToolStatus::Running,
                started_at: Some(std::time::Instant::now()),
                completed_at: None,
                duration_ms: None,
                expanded: true,
            }),
        });
        self.active_tool = None;
    }

    /// Append live stdout/stderr output to the active tool block.
    pub fn tool_output_delta(&mut self, tool_use_id: &str, stream: &str, chunk: &str) {
        let target = self.messages.iter_mut().rev().find(|msg| {
            msg.role == Role::Tool
                && msg.tool.as_ref().is_some_and(|info| {
                    info.status == ToolStatus::Running
                        && (tool_use_id.is_empty() || info.id == tool_use_id)
                })
        });
        let Some(msg) = target else {
            return;
        };
        let Some(info) = msg.tool.as_mut() else {
            return;
        };
        if stream == "stderr" {
            info.stderr.push_str(chunk);
        } else {
            info.stdout.push_str(chunk);
        }
        info.expanded = true;
    }

    /// Fill in the result for the matching tool message.
    pub fn tool_result(&mut self, tool_use_id: &str, name: &str, result: &str, is_error: bool) {
        // Prefer the runtime id. Fallback by name keeps older stream events usable.
        for msg in self.messages.iter_mut().rev() {
            if msg.role == Role::Tool {
                if let Some(ref mut info) = msg.tool {
                    let matches_id = !tool_use_id.is_empty() && info.id == tool_use_id;
                    let matches_legacy = tool_use_id.is_empty() && info.name == name;
                    if info.status == ToolStatus::Running && (matches_id || matches_legacy) {
                        info.result = result.to_string();
                        info.is_error = is_error;
                        info.status = if is_error {
                            ToolStatus::Error
                        } else {
                            ToolStatus::Success
                        };
                        info.duration_ms = info.started_at.map(|t| t.elapsed().as_millis() as u64);
                        info.completed_at = Some(std::time::Instant::now());
                        info.expanded = is_error || keep_tool_expanded_on_success(&info.name);
                        break;
                    }
                }
            }
        }
        self.active_tool = None;
    }

    /// Advance the spinner frame (called on tick).
    pub fn tick(&mut self) {
        let has_running_tool = self.messages.iter().any(|msg| {
            msg.tool
                .as_ref()
                .is_some_and(|info| info.status == ToolStatus::Running)
        });
        if self.active_tool.is_some() || self.thinking || has_running_tool {
            self.spinner_frame = (self.spinner_frame + 1) % theme::SPINNER_FRAMES.len();
        }
    }

    /// Phase L.3 / O.4: démarre une session persistante pour cet agent.
    /// Comportement O.4: démarrage TOUJOURS vierge — on ne rejoue plus
    /// l'historique automatiquement. Voir `/reload` ou `/history` pour
    /// récupérer manuellement la session précédente. Cela évite de
    /// surprendre l'utilisateur avec un vieux message agent au boot.
    /// Le `key` reste stable pour qu'un nouveau write append au même
    /// fichier que la session précédente, mais on génère un nouveau
    /// `created_at` à chaque démarrage.
    pub fn start_session(&mut self, key: &str) {
        self.session_key = key.to_string();
        self.authoritative_session_id = None;
        self.authoritative_agent_id = None;
        self.session_path = None;
        self.session_created_at = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        self.session_input_tokens = 0;
        self.session_output_tokens = 0;
        self.session_cached_input_tokens = 0;
        self.session_cache_creation_tokens = 0;
        self.session_cost_usd = 0.0;
    }

    pub fn bind_authoritative_session(&mut self, agent_id: &str, session_id: &str) {
        self.authoritative_agent_id = Some(agent_id.to_string());
        self.authoritative_session_id = Some(session_id.to_string());
    }

    /// Phase L.3: sauvegarde immédiate de la session sur disque (best-effort).
    /// Silencieux en cas d'échec — la persistance ne doit jamais bloquer le chat.
    pub fn persist_session(&mut self) {
        if self.session_key.is_empty() {
            return;
        }
        use crate::tui::session_store as store;
        let messages = self
            .messages
            .iter()
            .map(|m| store::PersistedMessage {
                role: match m.role {
                    Role::User => "user",
                    Role::Agent => "agent",
                    Role::System => "system",
                    Role::Tool => "tool",
                }
                .to_string(),
                text: m.text.clone(),
                tool: m.tool.as_ref().map(|t| store::PersistedTool {
                    name: t.name.clone(),
                    input: t.input.clone(),
                    result: persisted_tool_result(t),
                    is_error: t.is_error,
                }),
            })
            .collect();
        let session = store::PersistedSession {
            session_id: self.authoritative_session_id.clone(),
            agent_id: self.authoritative_agent_id.clone(),
            agent_name: self.agent_name.clone(),
            model_label: chat_model_label::sanitize_model_label(&self.model_label),
            mode_label: self.mode_label.clone(),
            messages,
            session_input_tokens: self.session_input_tokens,
            session_output_tokens: self.session_output_tokens,
            session_cached_input_tokens: self.session_cached_input_tokens,
            session_cache_creation_tokens: self.session_cache_creation_tokens,
            session_cost_usd: self.session_cost_usd,
            created_at: self.session_created_at,
            updated_at: 0,
        };
        let key = self.session_key.clone();
        let path = self.session_path.clone();
        if let Some(written) = store::save_session(&key, path.as_deref(), &session) {
            self.session_path = Some(written);
        }
    }

    /// Phase L.4: charge explicitement une session depuis son chemin (utilisé
    /// par le session picker). Vide l'état chat puis rejoue les messages.
    pub fn replay_session_from(&mut self, key: &str, path: &std::path::Path) {
        chat_session_replay::replay_session_from(self, key, path);
    }

    /// Phase L.4: ouvre l'overlay session picker en peuplant la liste.
    pub fn open_session_picker(&mut self) {
        use crate::tui::session_store as store;
        self.session_picker_items = store::list_sessions();
        self.session_picker_idx = 0;
        self.show_session_picker = true;
    }

    pub fn set_authoritative_session_picker_items(
        &mut self,
        sessions: &[crate::tui::screens::sessions::SessionInfo],
    ) {
        use crate::tui::session_store::SessionSummary;
        self.session_picker_items = sessions
            .iter()
            .filter(|session| uuid::Uuid::parse_str(&session.id).is_ok())
            .map(|session| SessionSummary {
                agent_key: format!("authoritative-{}", session.agent_id),
                session_id: Some(session.id.clone()),
                label: session.label.clone(),
                agent_name: session.agent_name.clone(),
                model_label: String::new(),
                path: std::path::PathBuf::new(),
                updated_at: chrono::DateTime::parse_from_rfc3339(&session.created)
                    .ok()
                    .and_then(|timestamp| u64::try_from(timestamp.timestamp()).ok())
                    .unwrap_or_default(),
                message_count: session.message_count as usize,
                session_input_tokens: 0,
                session_output_tokens: 0,
            })
            .collect();
        self.session_picker_idx = 0;
        self.show_session_picker = !self.session_picker_items.is_empty();
    }

    /// Phase L.4: exporte la session courante en markdown sous
    /// `~/.captain/exports/`. Retourne le chemin écrit ou un message d'erreur.
    pub fn export_markdown(&self) -> Result<std::path::PathBuf, String> {
        chat_markdown_export::export_markdown(self)
    }

    /// Phase L.3: accumule tokens et coût d'un tour LLM dans les compteurs
    /// session. Appelé depuis handle_stream_done.
    pub fn record_usage(
        &mut self,
        input: u64,
        output: u64,
        cached_input: u64,
        cache_creation: u64,
        cost_usd: f64,
    ) {
        self.session_input_tokens = self.session_input_tokens.saturating_add(input);
        self.session_output_tokens = self.session_output_tokens.saturating_add(output);
        self.session_cached_input_tokens = self
            .session_cached_input_tokens
            .saturating_add(cached_input);
        self.session_cache_creation_tokens = self
            .session_cache_creation_tokens
            .saturating_add(cache_creation);
        if cost_usd.is_finite() && cost_usd > 0.0 {
            self.session_cost_usd += cost_usd;
        }
    }

    /// Phase N.2: true quand le slash picker live doit être affiché —
    /// input commence par '/', pas en streaming, aucun autre overlay actif.
    pub fn slash_picker_active(&self) -> bool {
        self.input.starts_with('/')
            && !self.is_streaming
            && !self.show_model_picker
            && !self.show_session_picker
            && self.pending_approval.is_none()
    }

    /// Phase N.2: liste filtrée des slash commands matchant le préfixe input.
    /// Retourne un Vec<&'static str> (les commandes sont en const).
    pub fn slash_filtered(&self) -> Vec<&'static str> {
        chat_slash_picker::slash_filtered(self.input.as_str())
    }

    /// Return filtered models based on the current picker filter.
    pub fn filtered_models(&self) -> Vec<&ModelEntry> {
        if self.model_picker_filter.is_empty() {
            return self.model_picker_models.iter().collect();
        }
        let f = self.model_picker_filter.to_lowercase();
        self.model_picker_models
            .iter()
            .filter(|m| {
                m.id.to_lowercase().contains(&f)
                    || m.display_name.to_lowercase().contains(&f)
                    || m.provider.to_lowercase().contains(&f)
            })
            .collect()
    }

    /// Handle a clipboard paste delivered by crossterm bracketed paste mode.
    ///
    /// The whole pasted blob is inserted at the cursor position so a paste
    /// in the middle of a draft keeps the surrounding text intact.
    pub fn handle_paste(&mut self, s: &str) {
        self.input_insert_str(&normalize_pasted_line_endings(s));
    }

    // ── Input cursor helpers ──────────────────────────────────────────────
    //
    // Every mutating operation on `input` goes through one of these so the
    // cursor stays valid (always on a UTF-8 boundary, always within bounds)
    // and the editing experience matches what a user expects from a real
    // text input — backspace deletes the char *before* the cursor, arrow
    // keys move *through* the buffer, Home/End jump to the extremes.

    /// Insert `c` at the cursor position and advance the cursor past it.
    pub fn input_insert_char(&mut self, c: char) {
        let pos = self.input_cursor.min(self.input.len());
        self.input.insert(pos, c);
        self.input_cursor = pos + c.len_utf8();
    }

    /// Insert `s` at the cursor position and advance the cursor past it.
    pub fn input_insert_str(&mut self, s: &str) {
        let pos = self.input_cursor.min(self.input.len());
        self.input.insert_str(pos, s);
        self.input_cursor = pos + s.len();
    }

    /// Delete the char immediately before the cursor (Backspace).
    /// No-op when the cursor is at 0.
    pub fn input_backspace(&mut self) {
        if self.input_cursor == 0 || self.input.is_empty() {
            return;
        }
        let new_cursor = prev_char_boundary(&self.input, self.input_cursor);
        self.input.replace_range(new_cursor..self.input_cursor, "");
        self.input_cursor = new_cursor;
    }

    /// Delete the char at the cursor (forward Delete). No-op at end.
    pub fn input_delete_forward(&mut self) {
        if self.input_cursor >= self.input.len() {
            return;
        }
        let next = next_char_boundary(&self.input, self.input_cursor);
        self.input.replace_range(self.input_cursor..next, "");
    }

    /// Reset the input buffer and cursor.
    pub fn input_clear(&mut self) {
        self.input.clear();
        self.input_cursor = 0;
    }

    /// Move the cursor one char left (no-op at 0).
    pub fn input_move_left(&mut self) {
        if self.input_cursor == 0 {
            return;
        }
        self.input_cursor = prev_char_boundary(&self.input, self.input_cursor);
    }

    /// Move the cursor one char right (no-op at end).
    pub fn input_move_right(&mut self) {
        if self.input_cursor >= self.input.len() {
            return;
        }
        self.input_cursor = next_char_boundary(&self.input, self.input_cursor);
    }

    /// Jump the cursor to the beginning of the buffer.
    pub fn input_move_home(&mut self) {
        self.input_cursor = 0;
    }

    /// Jump the cursor to the end of the buffer.
    pub fn input_move_end(&mut self) {
        self.input_cursor = self.input.len();
    }

    /// Move the cursor one logical line up, preserving the column when
    /// possible (clamped to the previous line's length). Returns `true`
    /// when the cursor actually moved — callers fall back to scrolling
    /// the message viewport when this returns `false` (single-line draft
    /// or already on the first line).
    pub fn input_move_up_line(&mut self) -> bool {
        let (line, col) = locate_cursor(&self.input, self.input_cursor);
        if line == 0 {
            return false;
        }
        let lines: Vec<&str> = self.input.split('\n').collect();
        let prev_line = lines[line - 1];
        let new_col = col.min(prev_line.len());
        // Byte offset of the start of `line - 1`.
        let mut offset = 0usize;
        for l in lines.iter().take(line - 1) {
            offset += l.len() + 1;
        }
        let mut target = offset + new_col;
        // Defensive: snap back to a UTF-8 boundary if the column landed
        // mid-char (the previous line ends in a multi-byte glyph at an
        // index ≤ col).
        while target > 0 && !self.input.is_char_boundary(target) {
            target -= 1;
        }
        self.input_cursor = target;
        true
    }

    /// Mirror of `input_move_up_line` going downwards.
    pub fn input_move_down_line(&mut self) -> bool {
        let (line, col) = locate_cursor(&self.input, self.input_cursor);
        let lines: Vec<&str> = self.input.split('\n').collect();
        if line + 1 >= lines.len() {
            return false;
        }
        let next_line = lines[line + 1];
        let new_col = col.min(next_line.len());
        let mut offset = 0usize;
        for l in lines.iter().take(line + 1) {
            offset += l.len() + 1;
        }
        let mut target = offset + new_col;
        while target > 0 && !self.input.is_char_boundary(target) {
            target -= 1;
        }
        self.input_cursor = target;
        true
    }

    #[allow(dead_code)]
    pub(crate) fn compose_model_label(provider: &str, model: &str) -> Option<String> {
        chat_model_label::compose_model_label(provider, model)
    }

    #[allow(dead_code)]
    pub(crate) fn model_label_from_agent_metadata(body: &serde_json::Value) -> Option<String> {
        chat_model_label::model_label_from_agent_metadata(body)
    }

    fn handle_global_key_action(&mut self, action: GlobalKeyAction) -> Option<ChatAction> {
        match action {
            GlobalKeyAction::Back => Some(ChatAction::Back),
            GlobalKeyAction::CloseModelPicker => {
                self.show_model_picker = false;
                Some(ChatAction::Continue)
            }
            GlobalKeyAction::ResetChat => {
                self.reset_preserving_chat_identity();
                Some(ChatAction::Continue)
            }
            GlobalKeyAction::ClearInput => {
                self.input_clear();
                Some(ChatAction::Continue)
            }
            GlobalKeyAction::DeleteWordBeforeCursor => {
                self.delete_word_before_cursor();
                Some(ChatAction::Continue)
            }
            GlobalKeyAction::CompleteSlash => {
                self.complete_slash_command_prefix();
                Some(ChatAction::Continue)
            }
            GlobalKeyAction::ToggleThinking => {
                self.toggle_thinking();
                Some(ChatAction::Continue)
            }
            GlobalKeyAction::ToggleLatestTool => {
                self.toggle_latest_completed_tool();
                Some(ChatAction::Continue)
            }
            GlobalKeyAction::OpenModelPicker => Some(ChatAction::OpenModelPicker),
            GlobalKeyAction::ToggleSessionPicker => {
                if self.show_session_picker {
                    self.show_session_picker = false;
                    Some(ChatAction::Continue)
                } else {
                    self.open_session_picker();
                    Some(ChatAction::OpenSessionPicker)
                }
            }
            GlobalKeyAction::Noop => Some(ChatAction::Continue),
            GlobalKeyAction::Continue => None,
        }
    }

    fn reset_preserving_chat_identity(&mut self) {
        let name = self.agent_name.clone();
        let model = self.model_label.clone();
        let mode = self.mode_label.clone();
        self.reset();
        self.agent_name = name;
        self.model_label = model;
        self.mode_label = mode;
    }

    fn delete_word_before_cursor(&mut self) {
        while self.input_cursor > 0
            && self.input[..self.input_cursor]
                .chars()
                .next_back()
                .is_some_and(|c| c.is_whitespace())
        {
            self.input_backspace();
        }
        while self.input_cursor > 0
            && self.input[..self.input_cursor]
                .chars()
                .next_back()
                .is_some_and(|c| !c.is_whitespace())
        {
            self.input_backspace();
        }
    }

    fn complete_slash_command_prefix(&mut self) {
        if !self.input.starts_with('/') {
            return;
        }
        let prefix = self.input.as_str();
        let matches = chat_slash_picker::slash_filtered(prefix);
        match matches.len() {
            0 => {}
            1 => {
                self.input = matches[0].to_string();
                self.input_cursor = self.input.len();
            }
            _ => {
                let common = chat_slash_picker::longest_common_prefix(&matches);
                if common.len() > prefix.len() {
                    self.input = common;
                    self.input_cursor = self.input.len();
                }
            }
        }
    }

    fn handle_session_picker_key(&mut self, key: KeyEvent) -> ChatAction {
        match session_picker_key_action_for_key(key) {
            SessionPickerKeyAction::Close => self.show_session_picker = false,
            SessionPickerKeyAction::Up => {
                self.session_picker_idx = self.session_picker_idx.saturating_sub(1);
            }
            SessionPickerKeyAction::Down => {
                let max = self.session_picker_items.len().saturating_sub(1);
                self.session_picker_idx = (self.session_picker_idx + 1).min(max);
            }
            SessionPickerKeyAction::Select => {
                if let Some(picked) = self
                    .session_picker_items
                    .get(self.session_picker_idx)
                    .cloned()
                {
                    self.show_session_picker = false;
                    if let Some(session_id) = picked.session_id {
                        return ChatAction::ResumeSession(session_id);
                    }
                    self.status_msg = Some(
                        "Cette session locale n'a pas encore été importée par le runtime."
                            .to_string(),
                    );
                }
            }
            SessionPickerKeyAction::Continue => {}
        }
        ChatAction::Continue
    }

    fn handle_model_picker_key(&mut self, key: KeyEvent) -> ChatAction {
        match model_picker_key_action_for_key(key) {
            ModelPickerKeyAction::Close => {
                self.show_model_picker = false;
            }
            ModelPickerKeyAction::Up => {
                self.model_picker_idx = self.model_picker_idx.saturating_sub(1);
            }
            ModelPickerKeyAction::Down => {
                let max = self.filtered_models().len().saturating_sub(1);
                if self.model_picker_idx < max {
                    self.model_picker_idx += 1;
                }
            }
            ModelPickerKeyAction::Select => {
                let filtered = self.filtered_models();
                if let Some(entry) = filtered.get(self.model_picker_idx) {
                    let model_id = entry.id.clone();
                    self.show_model_picker = false;
                    self.model_picker_filter.clear();
                    self.model_picker_idx = 0;
                    return ChatAction::SwitchModel(model_id);
                }
            }
            ModelPickerKeyAction::Backspace => {
                self.model_picker_filter.pop();
                self.model_picker_idx = 0;
            }
            ModelPickerKeyAction::Insert(c) => {
                self.model_picker_filter.push(c);
                self.model_picker_idx = 0;
            }
            ModelPickerKeyAction::Continue => {}
        }
        ChatAction::Continue
    }

    fn handle_streaming_key(&mut self, key: KeyEvent) -> ChatAction {
        let action = streaming_key_action_for_key(key);
        match action {
            StreamingKeyAction::Back => ChatAction::Back,
            StreamingKeyAction::StageInput => {
                self.stage_streaming_input();
                ChatAction::Continue
            }
            StreamingKeyAction::ScrollPageUp
            | StreamingKeyAction::ScrollPageDown
            | StreamingKeyAction::PageUp
            | StreamingKeyAction::PageDown => {
                self.apply_streaming_scroll(action);
                ChatAction::Continue
            }
            StreamingKeyAction::Insert(_)
            | StreamingKeyAction::Backspace
            | StreamingKeyAction::Delete => {
                self.apply_streaming_edit(action);
                ChatAction::Continue
            }
            StreamingKeyAction::Left
            | StreamingKeyAction::Right
            | StreamingKeyAction::Home
            | StreamingKeyAction::End
            | StreamingKeyAction::Up
            | StreamingKeyAction::Down => {
                self.apply_streaming_navigation(action);
                ChatAction::Continue
            }
            StreamingKeyAction::Continue => ChatAction::Continue,
        }
    }

    fn stage_streaming_input(&mut self) {
        let msg = self.input.trim().to_string();
        self.input_clear();
        if !msg.is_empty() && !msg.starts_with('/') {
            self.staged_messages.push(msg.clone());
            self.push_message(Role::User, msg);
        }
    }

    fn apply_streaming_scroll(&mut self, action: StreamingKeyAction) {
        match action {
            StreamingKeyAction::ScrollPageUp | StreamingKeyAction::PageUp => self.scroll_page_up(),
            StreamingKeyAction::ScrollPageDown | StreamingKeyAction::PageDown => {
                self.scroll_page_down();
            }
            _ => {}
        }
    }

    fn apply_streaming_edit(&mut self, action: StreamingKeyAction) {
        match action {
            StreamingKeyAction::Insert(c) => self.input_insert_char(c),
            StreamingKeyAction::Backspace => self.input_backspace(),
            StreamingKeyAction::Delete => self.input_delete_forward(),
            _ => {}
        }
    }

    fn apply_streaming_navigation(&mut self, action: StreamingKeyAction) {
        match action {
            StreamingKeyAction::Left => self.input_move_left(),
            StreamingKeyAction::Right => self.input_move_right(),
            StreamingKeyAction::Home => self.input_move_home(),
            StreamingKeyAction::End => self.input_move_end(),
            StreamingKeyAction::Up => {
                if !self.input_move_up_line() {
                    self.scroll_line_up();
                }
            }
            StreamingKeyAction::Down if !self.input_move_down_line() => {
                self.scroll_line_down();
            }
            _ => {}
        }
    }

    fn handle_slash_picker_key(&mut self, key: KeyEvent) -> Option<ChatAction> {
        match slash_picker_key_action_for_key(key) {
            SlashPickerKeyAction::Up => {
                self.slash_picker_idx = self.slash_picker_idx.saturating_sub(1);
                Some(ChatAction::Continue)
            }
            SlashPickerKeyAction::Down => {
                let max = self.slash_filtered().len().saturating_sub(1);
                self.slash_picker_idx = (self.slash_picker_idx + 1).min(max);
                Some(ChatAction::Continue)
            }
            SlashPickerKeyAction::Cancel => {
                self.input_clear();
                self.slash_picker_idx = 0;
                Some(ChatAction::Continue)
            }
            SlashPickerKeyAction::Select => {
                let picked = self
                    .slash_filtered()
                    .get(self.slash_picker_idx)
                    .copied()
                    .map(String::from);
                self.input_clear();
                self.slash_picker_idx = 0;
                Some(
                    picked
                        .map(ChatAction::SlashCommand)
                        .unwrap_or(ChatAction::Continue),
                )
            }
            SlashPickerKeyAction::Continue => None,
        }
    }

    fn handle_input_key(&mut self, key: KeyEvent) -> ChatAction {
        let action = input_key_action_for_key(key);
        match action {
            InputKeyAction::Back => ChatAction::Back,
            InputKeyAction::Submit => self.submit_input(),
            InputKeyAction::InsertNewline => {
                self.input_insert_char('\n');
                ChatAction::Continue
            }
            InputKeyAction::ScrollPageUp
            | InputKeyAction::ScrollPageDown
            | InputKeyAction::PageUp
            | InputKeyAction::PageDown => {
                self.apply_input_scroll(action);
                ChatAction::Continue
            }
            InputKeyAction::Insert(_) | InputKeyAction::Backspace | InputKeyAction::Delete => {
                self.apply_input_edit(action);
                ChatAction::Continue
            }
            InputKeyAction::Left
            | InputKeyAction::Right
            | InputKeyAction::Home
            | InputKeyAction::End
            | InputKeyAction::Up
            | InputKeyAction::Down => {
                self.apply_input_navigation(action);
                ChatAction::Continue
            }
            InputKeyAction::Continue => ChatAction::Continue,
        }
    }

    fn submit_input(&mut self) -> ChatAction {
        let msg = self.input.trim().to_string();
        self.input_clear();
        if msg.is_empty() {
            return ChatAction::Continue;
        }
        if msg.starts_with('/') {
            return ChatAction::SlashCommand(msg);
        }
        self.push_message(Role::User, msg.clone());
        ChatAction::SendMessage(msg)
    }

    fn apply_input_scroll(&mut self, action: InputKeyAction) {
        match action {
            InputKeyAction::ScrollPageUp | InputKeyAction::PageUp => self.scroll_page_up(),
            InputKeyAction::ScrollPageDown | InputKeyAction::PageDown => self.scroll_page_down(),
            _ => {}
        }
    }

    fn apply_input_edit(&mut self, action: InputKeyAction) {
        match action {
            InputKeyAction::Insert(c) => self.input_insert_char(c),
            InputKeyAction::Backspace => self.input_backspace(),
            InputKeyAction::Delete => self.input_delete_forward(),
            _ => {}
        }
        self.slash_picker_idx = 0;
    }

    fn apply_input_navigation(&mut self, action: InputKeyAction) {
        match action {
            InputKeyAction::Left => self.input_move_left(),
            InputKeyAction::Right => self.input_move_right(),
            InputKeyAction::Home => self.input_move_home(),
            InputKeyAction::End => self.input_move_end(),
            InputKeyAction::Up => {
                if !self.input_move_up_line() {
                    self.scroll_line_up();
                }
            }
            InputKeyAction::Down if !self.input_move_down_line() => {
                self.scroll_line_down();
            }
            _ => {}
        }
    }

    pub fn handle_key(&mut self, key: KeyEvent) -> ChatAction {
        if self.has_quick_action_prompt() {
            return self.handle_quick_action_key(key);
        }

        let global_action =
            global_key_action_for_key(key, self.show_model_picker, self.is_streaming);
        if let Some(action) = self.handle_global_key_action(global_action) {
            return action;
        }

        // Session picker mode: navigation + load on Enter.
        if self.show_session_picker {
            return self.handle_session_picker_key(key);
        }

        // Model picker mode: intercept all keys
        if self.show_model_picker {
            return self.handle_model_picker_key(key);
        }

        // When streaming, allow typing + staging messages, scrolling, and Esc
        if self.is_streaming {
            return self.handle_streaming_key(key);
        }

        // Phase N.2: slash picker live — quand l'input commence par '/',
        // ↑↓ navigue dans la liste filtrée, Enter applique la commande
        // sélectionnée (qui peut différer de ce que l'user a tapé), Esc
        // annule l'input slash.
        if self.slash_picker_active() {
            if let Some(action) = self.handle_slash_picker_key(key) {
                return action;
            }
        }

        self.handle_input_key(key)
    }

    fn handle_quick_action_key(&mut self, key: KeyEvent) -> ChatAction {
        // Phase-i.6 + Q.11.b.b: pending approval prompt — absorb keys until decided.
        // Four choices: [o]nce / [s]ession / [A]lways / [n]o (or [Esc]).
        // Legacy [y] and [d] kept for back-compat.
        if self.pending_approval.is_some() {
            return match approval_quick_action_choice_for_key(key) {
                Some(choice) => self
                    .resolve_quick_action_choice(choice)
                    .unwrap_or(ChatAction::Continue),
                None => ChatAction::Continue,
            };
        }

        // Safe model/provider switch prompt — keep the user in the current
        // chat and accept buttons, 1/2, or natural language typed in input.
        if self.pending_model_switch.is_some() {
            return self.handle_model_switch_quick_action_key(key);
        }

        // ask_user with options — [1]..[9] pick an option, same as the mouse
        // click zones drawn by build_ask_user_prompt.
        if let Some(pending) = self.pending_ask_user.as_ref() {
            let n_options = pending.options.len();
            return match ask_user_quick_action_choice_for_key(key, n_options) {
                Some(choice) => self
                    .resolve_quick_action_choice(choice)
                    .unwrap_or(ChatAction::Continue),
                None => ChatAction::Continue,
            };
        }

        ChatAction::Continue
    }

    fn handle_model_switch_quick_action_key(&mut self, key: KeyEvent) -> ChatAction {
        let recommended_strategy = self
            .pending_model_switch
            .as_ref()
            .map(|pending| pending.recommended_session_strategy.as_str());
        match model_switch_quick_action_for_key(key, self.input.trim(), recommended_strategy) {
            ModelSwitchQuickActionKey::Choice(choice) => self
                .resolve_quick_action_choice(choice)
                .unwrap_or(ChatAction::Continue),
            ModelSwitchQuickActionKey::InvalidAnswer => {
                self.status_msg = Some(MODEL_SWITCH_INVALID_REPLY.to_string());
                ChatAction::Continue
            }
            ModelSwitchQuickActionKey::Backspace => {
                self.input_backspace();
                ChatAction::Continue
            }
            ModelSwitchQuickActionKey::Delete => {
                self.input_delete_forward();
                ChatAction::Continue
            }
            ModelSwitchQuickActionKey::Left => {
                self.input_move_left();
                ChatAction::Continue
            }
            ModelSwitchQuickActionKey::Right => {
                self.input_move_right();
                ChatAction::Continue
            }
            ModelSwitchQuickActionKey::Home => {
                self.input_move_home();
                ChatAction::Continue
            }
            ModelSwitchQuickActionKey::End => {
                self.input_move_end();
                ChatAction::Continue
            }
            ModelSwitchQuickActionKey::Insert(c) => {
                self.input_insert_char(c);
                ChatAction::Continue
            }
            ModelSwitchQuickActionKey::Continue => ChatAction::Continue,
        }
    }
}

#[cfg(test)]
mod tests_background_activity {
    use super::*;

    #[test]
    fn tracking_then_clearing_removes_the_entry() {
        let mut state = ChatState::new();
        state.track_background_activity("agent-1".to_string(), "agent researcher".to_string());
        assert_eq!(state.background_activity.len(), 1);

        state.clear_background_activity("agent-1");
        assert!(state.background_activity.is_empty());
    }

    #[test]
    fn tracking_the_same_key_twice_refreshes_the_label_instead_of_duplicating() {
        let mut state = ChatState::new();
        state.track_background_activity("toolrun-1".to_string(), "tool_run shell_exec".to_string());
        state.track_background_activity(
            "toolrun-1".to_string(),
            "tool_run shell_exec (retry)".to_string(),
        );

        assert_eq!(state.background_activity.len(), 1);
        assert_eq!(
            state.background_activity[0].label,
            "tool_run shell_exec (retry)"
        );
    }

    #[test]
    fn clearing_an_unknown_key_is_a_no_op() {
        let mut state = ChatState::new();
        state.track_background_activity("agent-1".to_string(), "agent researcher".to_string());
        state.clear_background_activity("agent-does-not-exist");
        assert_eq!(state.background_activity.len(), 1);
    }
}

/// Walk back from `idx` to the previous UTF-8 char boundary inside `s`.
/// Used by `input_backspace` and `input_move_left` so we never split a
/// multi-byte char (é, emoji, …) when editing.
fn prev_char_boundary(s: &str, idx: usize) -> usize {
    let mut i = idx.min(s.len()).saturating_sub(1);
    while i > 0 && !s.is_char_boundary(i) {
        i -= 1;
    }
    i
}

/// Walk forward from `idx` to the next UTF-8 char boundary inside `s`.
fn next_char_boundary(s: &str, idx: usize) -> usize {
    let mut i = (idx + 1).min(s.len());
    while i < s.len() && !s.is_char_boundary(i) {
        i += 1;
    }
    i
}

pub fn draw(
    f: &mut Frame,
    area: Rect,
    state: &mut ChatState,
    image_cache: &mut crate::tui::image_preview::ImagePreviewCache,
) {
    draw_chat_screen(f, area, state, image_cache);
}

fn chat_action_to_mouse_action(action: ChatAction) -> ChatMouseAction {
    match action {
        ChatAction::ApplyModelSwitch {
            model_id,
            session_strategy,
        } => ChatMouseAction::ApplyModelSwitch {
            model_id,
            session_strategy,
        },
        ChatAction::ApproveRequest(id) => ChatMouseAction::ApproveRequest(id),
        ChatAction::ApproveSessionRequest(id) => ChatMouseAction::ApproveSessionRequest(id),
        ChatAction::ApproveAlwaysRequest(id) => ChatMouseAction::ApproveAlwaysRequest(id),
        ChatAction::RejectRequest(id) => ChatMouseAction::RejectRequest(id),
        _ => ChatMouseAction::ModelSwitchCancelled,
    }
}

pub(super) fn tool_status_parts(
    info: &ToolInfo,
    spinner_frame: Option<usize>,
) -> (&'static str, &'static str, Style) {
    match info.status {
        ToolStatus::Running => (
            spinner_frame
                .map(|i| theme::SPINNER_FRAMES[i % theme::SPINNER_FRAMES.len()])
                .unwrap_or("\u{25cf}"),
            "running",
            Style::default()
                .fg(theme::BLUE)
                .add_modifier(Modifier::BOLD),
        ),
        ToolStatus::Success => (
            "\u{2714}",
            "done",
            Style::default()
                .fg(theme::GREEN)
                .add_modifier(Modifier::BOLD),
        ),
        ToolStatus::Error => (
            "\u{2718}",
            "error",
            Style::default().fg(theme::RED).add_modifier(Modifier::BOLD),
        ),
    }
}

fn keep_tool_expanded_on_success(name: &str) -> bool {
    matches!(name, "apply_patch" | "edit_file" | "multi_edit")
}

fn persisted_tool_result(info: &ToolInfo) -> String {
    if !info.result.is_empty() {
        return info.result.clone();
    }
    let mut out = String::new();
    if !info.stdout.is_empty() {
        out.push_str("STDOUT:\n");
        out.push_str(&info.stdout);
    }
    if !info.stderr.is_empty() {
        if !out.is_empty() {
            out.push('\n');
        }
        out.push_str("STDERR:\n");
        out.push_str(&info.stderr);
    }
    out
}

pub(super) fn tool_output_summary(info: &ToolInfo) -> String {
    let stdout_lines = info.stdout.lines().count();
    let stderr_lines = info.stderr.lines().count();
    if stdout_lines > 0 || stderr_lines > 0 {
        let mut parts = Vec::new();
        if stdout_lines > 0 {
            parts.push(format!("stdout {stdout_lines} lines"));
        }
        if stderr_lines > 0 {
            parts.push(format!("stderr {stderr_lines} lines"));
        }
        if let Some(exit) = extract_exit_code(&info.result) {
            parts.push(format!("exit {exit}"));
        }
        return parts.join(" · ");
    }
    truncate_line(&info.result.replace('\n', " "), 120)
}

pub(super) fn tool_input_summary(info: &ToolInfo) -> String {
    if let Some(command) = tool_command(info) {
        return command;
    }
    if let Ok(v) = serde_json::from_str::<serde_json::Value>(&info.input) {
        match info.name.as_str() {
            "file_read" | "file_write" | "file_delete" => {
                if let Some(path) = json_str(&v, &["path", "file_path"]) {
                    return path.to_string();
                }
            }
            "web_fetch" => {
                if let Some(url) = json_str(&v, &["url"]) {
                    return url.to_string();
                }
            }
            "web_search" => {
                if let Some(query) = json_str(&v, &["query", "q"]) {
                    return query.to_string();
                }
            }
            "channel_send" => {
                let channel = json_str(&v, &["channel"]).unwrap_or("channel");
                let recipient = json_str(&v, &["recipient", "to"]).unwrap_or("default");
                return format!("{channel} → {recipient}");
            }
            "memory_store" | "memory_save" => {
                if let Some(subject) = json_str(&v, &["subject", "key"]) {
                    return subject.to_string();
                }
            }
            _ => {}
        }
        return compact_json(&v);
    }
    info.input.replace('\n', " ")
}

pub(super) fn tool_command(info: &ToolInfo) -> Option<String> {
    if let Some(command) = copyable_tool_command(info) {
        return Some(command);
    }
    let v = serde_json::from_str::<serde_json::Value>(&info.input).ok()?;
    match info.name.as_str() {
        "execute_code" => json_str(&v, &["language", "lang"])
            .map(|lang| format!("{lang} inline code"))
            .or_else(|| Some("inline code".to_string())),
        _ => None,
    }
}

pub(super) fn copyable_tool_command(info: &ToolInfo) -> Option<String> {
    let v = serde_json::from_str::<serde_json::Value>(&info.input).ok()?;
    match info.name.as_str() {
        "shell_exec" | "ssh_exec" | "docker_exec" => json_command(&v, &["command", "cmd"]),
        "process_start" => process_start_command(&v),
        _ => None,
    }
}

fn process_start_command(v: &serde_json::Value) -> Option<String> {
    let command = json_command(v, &["command", "cmd"])?;
    let args = match v.get("args").and_then(|value| value.as_array()) {
        Some(items) => items
            .iter()
            .map(|arg| arg.as_str().map(shell_quote_arg))
            .collect::<Option<Vec<_>>>(),
        None => Some(Vec::new()),
    }?;
    if args.is_empty() {
        Some(command)
    } else {
        let mut parts = Vec::with_capacity(args.len() + 1);
        parts.push(shell_quote_arg(&command));
        parts.extend(args);
        Some(parts.join(" "))
    }
}

fn json_command(v: &serde_json::Value, keys: &[&str]) -> Option<String> {
    keys.iter().find_map(|key| {
        let value = v.get(*key)?;
        if let Some(command) = value.as_str() {
            return Some(command.to_string());
        }
        let args = value.as_array()?;
        args.iter()
            .map(|arg| arg.as_str().map(shell_quote_arg))
            .collect::<Option<Vec<_>>>()
            .map(|parts| parts.join(" "))
    })
}

fn shell_quote_arg(arg: &str) -> String {
    if arg.is_empty() {
        return "''".to_string();
    }
    if arg.chars().all(|ch| {
        ch.is_ascii_alphanumeric() || matches!(ch, '_' | '-' | '.' | '/' | ':' | '=' | '+')
    }) {
        return arg.to_string();
    }
    format!("'{}'", arg.replace('\'', "'\\''"))
}

fn json_str<'a>(v: &'a serde_json::Value, keys: &[&str]) -> Option<&'a str> {
    keys.iter()
        .find_map(|key| v.get(*key).and_then(|val| val.as_str()))
}

fn compact_json(v: &serde_json::Value) -> String {
    serde_json::to_string(v).unwrap_or_else(|_| "{}".to_string())
}

pub(super) fn pretty_tool_input(input: &str) -> String {
    serde_json::from_str::<serde_json::Value>(input)
        .ok()
        .and_then(|v| serde_json::to_string_pretty(&v).ok())
        .unwrap_or_else(|| input.to_string())
}

pub(super) fn tail_lines(text: &str, max: usize) -> Vec<String> {
    let mut lines: Vec<String> = text.lines().map(ToString::to_string).collect();
    if lines.len() > max {
        lines = lines.split_off(lines.len() - max);
    }
    lines
}

pub(super) fn format_duration_ms(ms: u64) -> String {
    if ms < 1_000 {
        format!("{ms}ms")
    } else if ms < 60_000 {
        format!("{:.1}s", ms as f64 / 1000.0)
    } else {
        let secs = ms / 1000;
        format!("{}m{:02}s", secs / 60, secs % 60)
    }
}

fn extract_exit_code(result: &str) -> Option<String> {
    let lower = result.to_ascii_lowercase();
    let idx = lower.find("exit code")?;
    let tail = result[idx..].chars().take(24).collect::<String>();
    tail.split(|c: char| !c.is_ascii_digit() && c != '-')
        .find(|part| !part.is_empty())
        .map(ToString::to_string)
}

/// Normalize `\r\n` and lone `\r` line endings to `\n` in pasted text.
///
/// Some terminals/sources deliver bracketed-paste data with `\r` line
/// endings instead of `\n`. Every downstream consumer of `ChatState.input`
/// (`raw_input_lines`, `compute_input_visual_rows`, `wrap_text`) splits on
/// `'\n'` only, so a paste with `\r` endings was treated as a single giant
/// logical line — rendered as one wrapped block instead of the original
/// multi-line/multi-paragraph structure.
pub(super) fn normalize_pasted_line_endings(s: &str) -> String {
    if !s.contains('\r') {
        return s.to_string();
    }
    s.replace("\r\n", "\n").replace('\r', "\n")
}

pub(super) fn wrap_text(text: &str, max_width: usize) -> Vec<String> {
    if max_width == 0 {
        return vec![text.to_string()];
    }

    let mut result = Vec::new();
    for line in text.lines() {
        if line.is_empty() {
            result.push(String::new());
            continue;
        }

        let mut current = String::new();
        for word in line.split_whitespace() {
            if current.is_empty() {
                current = word.to_string();
            } else if current.len() + 1 + word.len() <= max_width {
                current.push(' ');
                current.push_str(word);
            } else {
                result.push(current);
                current = word.to_string();
            }
        }
        if !current.is_empty() {
            result.push(current);
        }
    }

    if result.is_empty() {
        result.push(String::new());
    }

    result
}

/// Strip leaked `<function>...</function>` tags from streaming text.
fn sanitize_function_tags(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(start) = rest.find("<function>") {
        out.push_str(&rest[..start]);
        if let Some(end) = rest[start..].find("</function>") {
            rest = &rest[start + end + "</function>".len()..];
        } else {
            // Unclosed tag — drop from <function> to end
            rest = "";
        }
    }
    out.push_str(rest);
    out
}

/// Truncate a string to `max_len` chars, appending `…` if truncated.
pub(super) fn truncate_line(s: &str, max_len: usize) -> String {
    if s.len() <= max_len {
        s.to_string()
    } else {
        format!(
            "{}\u{2026}",
            captain_types::truncate_str(s, max_len.saturating_sub(1))
        )
    }
}

#[cfg(test)]
mod tests_q11b_modal {
    use super::*;
    use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

    fn state_with_pending() -> ChatState {
        let mut s = ChatState::new();
        s.pending_approval = Some(crate::tui::screens::approvals::ApprovalRequest {
            id: "req-modal-42".into(),
            agent_name: "captain".into(),
            tool_name: "shell_exec".into(),
            description: "rm -rf /tmp".into(),
            action: "rm -rf /tmp".into(),
            risk_level: "high".into(),
            created_at: 0,
        });
        s
    }

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }
    fn shift_key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::SHIFT)
    }

    #[test]
    fn tool_result_matches_repeated_tool_calls_by_id() {
        let mut s = ChatState::new();
        s.tool_use_end("call-a", "captain_docs", r#"{"family":"shell-process"}"#);
        s.tool_use_end("call-b", "captain_docs", r#"{"family":"ssh"}"#);

        s.tool_result(
            "call-a",
            "captain_docs",
            r#"{"family":"shell-process"}"#,
            false,
        );
        s.tool_result("call-b", "captain_docs", r#"{"family":"ssh"}"#, false);

        let tool_results: Vec<_> = s
            .messages
            .iter()
            .filter_map(|msg| msg.tool.as_ref())
            .map(|tool| (tool.id.as_str(), tool.result.as_str(), tool.status))
            .collect();

        assert_eq!(
            tool_results,
            vec![
                (
                    "call-a",
                    r#"{"family":"shell-process"}"#,
                    ToolStatus::Success
                ),
                ("call-b", r#"{"family":"ssh"}"#, ToolStatus::Success),
            ]
        );
    }

    #[test]
    fn q11bb_modal_o_returns_approve_once() {
        let mut s = state_with_pending();
        match s.handle_key(key(KeyCode::Char('o'))) {
            ChatAction::ApproveRequest(id) => assert_eq!(id, "req-modal-42"),
            other => panic!(
                "expected ApproveRequest, got: {:?}",
                std::mem::discriminant(&other)
            ),
        }
        assert!(
            s.pending_approval.is_none(),
            "modal must close after decision"
        );
    }

    #[test]
    fn q11bb_modal_s_returns_approve_session() {
        let mut s = state_with_pending();
        match s.handle_key(key(KeyCode::Char('s'))) {
            ChatAction::ApproveSessionRequest(id) => assert_eq!(id, "req-modal-42"),
            other => panic!(
                "expected ApproveSessionRequest, got: {:?}",
                std::mem::discriminant(&other)
            ),
        }
        assert!(s.pending_approval.is_none());
    }

    #[test]
    fn q11bb_modal_uppercase_a_returns_approve_always() {
        let mut s = state_with_pending();
        match s.handle_key(shift_key(KeyCode::Char('A'))) {
            ChatAction::ApproveAlwaysRequest(id) => assert_eq!(id, "req-modal-42"),
            other => panic!(
                "expected ApproveAlwaysRequest, got: {:?}",
                std::mem::discriminant(&other)
            ),
        }
        assert!(s.pending_approval.is_none());
    }

    #[test]
    fn q11bb_modal_n_or_esc_still_rejects() {
        let mut s = state_with_pending();
        assert!(matches!(
            s.handle_key(key(KeyCode::Char('n'))),
            ChatAction::RejectRequest(_)
        ));
        assert!(s.pending_approval.is_none());

        let mut s2 = state_with_pending();
        assert!(matches!(
            s2.handle_key(key(KeyCode::Esc)),
            ChatAction::RejectRequest(_)
        ));
    }

    #[test]
    fn q11bb_modal_unknown_key_does_not_close() {
        let mut s = state_with_pending();
        assert!(matches!(
            s.handle_key(key(KeyCode::Char('z'))),
            ChatAction::Continue
        ));
        assert!(
            s.pending_approval.is_some(),
            "unknown key must keep the modal open"
        );
    }

    #[test]
    fn q11bb_modal_mouse_choice_returns_action() {
        let mut s = state_with_pending();
        s.quick_action_click_zones.push(QuickActionClickZone {
            x_start: 10,
            x_end: 24,
            y: 5,
            choice: QuickActionChoiceId::ApprovalSession,
        });

        assert_eq!(
            s.handle_mouse_click(12, 5),
            Some(ChatMouseAction::ApproveSessionRequest(
                "req-modal-42".to_string()
            ))
        );
        assert!(s.pending_approval.is_none());
    }
}

#[cfg(test)]
mod tests_global_key_application {
    use super::*;
    use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

    fn ctrl_key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::CONTROL)
    }

    #[test]
    fn ctrl_m_while_streaming_is_handled_noop_not_input() {
        let mut state = ChatState::new();
        state.is_streaming = true;
        state.input_insert_str("draft");

        let action = state.handle_key(ctrl_key(KeyCode::Char('m')));

        assert!(matches!(action, ChatAction::Continue));
        assert_eq!(state.input, "draft");
        assert!(!state.show_model_picker);
    }

    #[test]
    fn ctrl_l_resets_history_but_preserves_chat_identity() {
        let mut state = ChatState::new();
        state.agent_name = "captain-test".to_string();
        state.model_label = "codex/gpt-test".to_string();
        state.mode_label = "auto".to_string();
        state.push_message(Role::User, "hello".to_string());

        let action = state.handle_key(ctrl_key(KeyCode::Char('l')));

        assert!(matches!(action, ChatAction::Continue));
        assert!(state.messages.is_empty());
        assert_eq!(state.agent_name, "captain-test");
        assert_eq!(state.model_label, "codex/gpt-test");
        assert_eq!(state.mode_label, "auto");
    }

    #[test]
    fn ctrl_w_deletes_word_before_cursor() {
        let mut state = ChatState::new();
        state.input_insert_str("alpha beta  gamma");

        let action = state.handle_key(ctrl_key(KeyCode::Char('w')));

        assert!(matches!(action, ChatAction::Continue));
        assert_eq!(state.input, "alpha beta  ");
        assert_eq!(state.input_cursor, state.input.len());
    }
}

#[cfg(test)]
mod tests_picker_key_application {
    use super::*;
    use crate::tui::session_store::SessionSummary;
    use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    use std::path::PathBuf;

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    fn summary(agent_key: &str) -> SessionSummary {
        SessionSummary {
            agent_key: agent_key.to_string(),
            session_id: Some("aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa".to_string()),
            label: agent_key.to_string(),
            agent_name: agent_key.to_string(),
            model_label: "codex/gpt-test".to_string(),
            path: PathBuf::from("/tmp/missing-captain-session.json"),
            updated_at: 10,
            message_count: 1,
            session_input_tokens: 2,
            session_output_tokens: 3,
        }
    }

    fn model(id: &str) -> ModelEntry {
        ModelEntry {
            id: id.to_string(),
            display_name: id.to_string(),
            provider: "codex".to_string(),
            tier: "frontier".to_string(),
        }
    }

    #[test]
    fn session_picker_keys_navigate_and_close() {
        let mut state = ChatState::new();
        state.show_session_picker = true;
        state.session_picker_items = vec![summary("alpha"), summary("beta")];
        state.session_picker_idx = 1;

        assert!(matches!(
            state.handle_key(key(KeyCode::Up)),
            ChatAction::Continue
        ));
        assert_eq!(state.session_picker_idx, 0);

        assert!(matches!(
            state.handle_key(key(KeyCode::Down)),
            ChatAction::Continue
        ));
        assert_eq!(state.session_picker_idx, 1);

        assert!(matches!(
            state.handle_key(key(KeyCode::Esc)),
            ChatAction::Continue
        ));
        assert!(!state.show_session_picker);
    }

    #[test]
    fn session_picker_stays_open_while_canonical_history_loads() {
        let mut state = ChatState::new();

        state.open_session_picker();

        assert!(state.show_session_picker);
    }

    #[test]
    fn session_picker_selection_requests_authoritative_restore() {
        let mut state = ChatState::new();
        state.show_session_picker = true;
        state.session_picker_items = vec![summary("alpha")];

        match state.handle_key(key(KeyCode::Enter)) {
            ChatAction::ResumeSession(session_id) => {
                assert_eq!(session_id, "aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa");
            }
            other => panic!(
                "expected ResumeSession, got {:?}",
                std::mem::discriminant(&other)
            ),
        }
        assert!(!state.show_session_picker);
    }

    #[test]
    fn model_picker_keys_filter_navigate_and_select() {
        let mut state = ChatState::new();
        state.show_model_picker = true;
        state.model_picker_models = vec![model("codex/alpha"), model("codex/beta")];

        assert!(matches!(
            state.handle_key(key(KeyCode::Char('b'))),
            ChatAction::Continue
        ));
        assert_eq!(state.model_picker_filter, "b");
        assert_eq!(state.model_picker_idx, 0);

        match state.handle_key(key(KeyCode::Enter)) {
            ChatAction::SwitchModel(model_id) => assert_eq!(model_id, "codex/beta"),
            other => panic!(
                "expected SwitchModel, got {:?}",
                std::mem::discriminant(&other)
            ),
        }
        assert!(!state.show_model_picker);
        assert!(state.model_picker_filter.is_empty());
        assert_eq!(state.model_picker_idx, 0);
    }
}

#[cfg(test)]
mod tests_streaming_key_application {
    use super::*;
    use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    #[test]
    fn streaming_enter_stages_non_slash_input() {
        let mut state = ChatState::new();
        state.is_streaming = true;
        state.input_insert_str("  queued follow-up  ");

        let action = state.handle_key(key(KeyCode::Enter));

        assert!(matches!(action, ChatAction::Continue));
        assert!(state.input.is_empty());
        assert_eq!(state.staged_messages, vec!["queued follow-up".to_string()]);
        assert_eq!(state.messages.len(), 1);
        assert_eq!(state.messages[0].text, "queued follow-up");
    }

    #[test]
    fn streaming_enter_clears_but_does_not_stage_slash_command() {
        let mut state = ChatState::new();
        state.is_streaming = true;
        state.input_insert_str("/status");

        let action = state.handle_key(key(KeyCode::Enter));

        assert!(matches!(action, ChatAction::Continue));
        assert!(state.input.is_empty());
        assert!(state.staged_messages.is_empty());
        assert!(state.messages.is_empty());
    }

    #[test]
    fn streaming_escape_returns_back() {
        let mut state = ChatState::new();
        state.is_streaming = true;

        assert!(matches!(
            state.handle_key(key(KeyCode::Esc)),
            ChatAction::Back
        ));
    }
}

#[cfg(test)]
mod tests_slash_input_key_application {
    use super::*;
    use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    fn modified_key(code: KeyCode, modifiers: KeyModifiers) -> KeyEvent {
        KeyEvent::new(code, modifiers)
    }

    #[test]
    fn slash_picker_enter_selects_filtered_command_and_clears_input() {
        let mut state = ChatState::new();
        state.input_insert_str("/stat");

        match state.handle_key(key(KeyCode::Enter)) {
            ChatAction::SlashCommand(cmd) => assert_eq!(cmd, "/status"),
            other => panic!(
                "expected SlashCommand, got {:?}",
                std::mem::discriminant(&other)
            ),
        }
        assert!(state.input.is_empty());
        assert_eq!(state.slash_picker_idx, 0);
    }

    #[test]
    fn slash_picker_escape_clears_input_and_selection() {
        let mut state = ChatState::new();
        state.input_insert_str("/s");
        state.slash_picker_idx = 3;

        let action = state.handle_key(key(KeyCode::Esc));

        assert!(matches!(action, ChatAction::Continue));
        assert!(state.input.is_empty());
        assert_eq!(state.slash_picker_idx, 0);
    }

    #[test]
    fn normal_input_enter_sends_trimmed_user_message() {
        let mut state = ChatState::new();
        state.input_insert_str("  hello captain  ");

        match state.handle_key(key(KeyCode::Enter)) {
            ChatAction::SendMessage(msg) => assert_eq!(msg, "hello captain"),
            other => panic!(
                "expected SendMessage, got {:?}",
                std::mem::discriminant(&other)
            ),
        }
        assert!(state.input.is_empty());
        assert_eq!(state.messages.len(), 1);
        assert_eq!(state.messages[0].text, "hello captain");
    }

    #[test]
    fn normal_input_shift_enter_inserts_newline_without_sending() {
        let mut state = ChatState::new();
        state.input_insert_str("hello");

        let action = state.handle_key(modified_key(KeyCode::Enter, KeyModifiers::SHIFT));

        assert!(matches!(action, ChatAction::Continue));
        assert_eq!(state.input, "hello\n");
        assert!(state.messages.is_empty());
    }

    #[test]
    fn normal_input_editing_resets_slash_picker_selection() {
        let mut state = ChatState::new();
        state.input_insert_str("/");
        state.slash_picker_idx = 4;

        let action = state.handle_key(key(KeyCode::Char('s')));

        assert!(matches!(action, ChatAction::Continue));
        assert_eq!(state.input, "/s");
        assert_eq!(state.slash_picker_idx, 0);
    }
}

#[cfg(test)]
mod tests_model_switch_prompt {
    use super::*;
    use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    fn prompt() -> PendingModelSwitch {
        PendingModelSwitch {
            model_id: "openai/gpt-5.4".into(),
            current_provider: "anthropic".into(),
            current_model: "claude-sonnet-4-6".into(),
            target_provider: "openai".into(),
            target_model: "gpt-5.4".into(),
            risk: "high".into(),
            recommended_session_strategy: "compact_session".into(),
            active_message_count: 12,
            canonical_summary_present: true,
        }
    }

    #[test]
    fn model_switch_prompt_accepts_keyboard_choice() {
        let mut s = ChatState::new();
        s.request_model_switch_choice(prompt());

        match s.handle_key(key(KeyCode::Char('2'))) {
            ChatAction::ApplyModelSwitch {
                model_id,
                session_strategy,
            } => {
                assert_eq!(model_id, "openai/gpt-5.4");
                assert_eq!(session_strategy, "compact_session");
            }
            other => panic!(
                "expected ApplyModelSwitch, got: {:?}",
                std::mem::discriminant(&other)
            ),
        }
        assert!(s.pending_model_switch.is_none());
    }

    #[test]
    fn model_switch_prompt_accepts_natural_language() {
        let mut s = ChatState::new();
        s.request_model_switch_choice(prompt());
        s.input_insert_str("garde le contexte");

        match s.handle_key(key(KeyCode::Enter)) {
            ChatAction::ApplyModelSwitch {
                model_id,
                session_strategy,
            } => {
                assert_eq!(model_id, "openai/gpt-5.4");
                assert_eq!(session_strategy, "compact_session");
            }
            other => panic!(
                "expected ApplyModelSwitch, got: {:?}",
                std::mem::discriminant(&other)
            ),
        }
        assert!(s.input.is_empty());
    }

    #[test]
    fn model_switch_prompt_mouse_choice_returns_action() {
        let mut s = ChatState::new();
        s.request_model_switch_choice(prompt());
        s.quick_action_click_zones.push(QuickActionClickZone {
            x_start: 10,
            x_end: 30,
            y: 5,
            choice: QuickActionChoiceId::ModelSwitchNewSession,
        });

        assert_eq!(
            s.handle_mouse_click(12, 5),
            Some(ChatMouseAction::ApplyModelSwitch {
                model_id: "openai/gpt-5.4".into(),
                session_strategy: "new_session".into(),
            })
        );
        assert!(s.pending_model_switch.is_none());
    }
}

#[cfg(test)]
mod tests_command_copy {
    use super::*;
    use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

    fn rendered_text(lines: &[Line<'static>]) -> String {
        lines
            .iter()
            .flat_map(|line| line.spans.iter())
            .map(|span| span.content.as_ref())
            .collect::<Vec<_>>()
            .join("")
    }

    #[test]
    fn last_command_to_copy_uses_original_tool_input() {
        let mut s = ChatState::new();
        s.tool_use_end(
            "call-1",
            "shell_exec",
            r#"{"command":"cargo test --package captain-cli command_copy"}"#,
        );

        assert_eq!(
            s.last_command_to_copy().as_deref(),
            Some("cargo test --package captain-cli command_copy")
        );
    }

    #[test]
    fn command_copy_click_returns_command_without_toggling() {
        let mut s = ChatState::new();
        s.tool_use_end("call-1", "shell_exec", r#"{"command":"echo precise"}"#);
        s.tool_click_zones.push(ToolClickZone {
            x_start: 2,
            x_end: 7,
            y: 4,
            message_idx: 0,
            action: ToolClickAction::CopyCommand,
        });

        assert_eq!(
            s.handle_mouse_click(4, 4),
            Some(ChatMouseAction::CopyCommand("echo precise".to_string()))
        );
        assert!(
            s.messages[0]
                .tool
                .as_ref()
                .is_some_and(|tool| tool.expanded),
            "copy click must not collapse or expand the tool card"
        );
    }

    #[test]
    fn tool_header_click_toggles_completed_tool() {
        let mut s = ChatState::new();
        s.tool_use_end("call-1", "shell_exec", r#"{"command":"echo precise"}"#);
        s.tool_result("call-1", "shell_exec", "ok", false);
        s.messages[0].tool.as_mut().unwrap().completed_at = None;
        s.tool_click_zones.push(ToolClickZone {
            x_start: 0,
            x_end: 14,
            y: 4,
            message_idx: 0,
            action: ToolClickAction::Toggle,
        });

        assert!(!should_render_tool_expanded(
            s.messages[0].tool.as_ref().unwrap()
        ));
        assert_eq!(
            s.handle_mouse_click(4, 4),
            Some(ChatMouseAction::ToolToggled)
        );
        assert!(s.messages[0].tool.as_ref().unwrap().expanded);
        assert_eq!(
            s.handle_mouse_click(4, 4),
            Some(ChatMouseAction::ToolToggled)
        );
        assert!(!s.messages[0].tool.as_ref().unwrap().expanded);
    }

    #[test]
    fn rendered_copy_badge_respects_mouse_capture() {
        let mut s = ChatState::new();
        s.tool_use_end("call-1", "shell_exec", r#"{"command":"echo precise"}"#);
        s.tool_result("call-1", "shell_exec", "ok", false);
        let info = s.messages[0].tool.as_ref().expect("tool message");

        let mut native_selection_lines = Vec::new();
        render_tool_message(&mut native_selection_lines, info, 96, 0, false);
        assert!(
            !rendered_text(&native_selection_lines).contains("[copy]"),
            "copy affordance must be hidden when mouse capture is disabled"
        );

        let mut mouse_lines = Vec::new();
        render_tool_message(&mut mouse_lines, info, 96, 0, true);
        assert!(
            rendered_text(&mouse_lines).contains("[copy]"),
            "copy affordance should be visible only when it can be clicked"
        );
    }

    #[test]
    fn ctrl_e_toggles_latest_completed_tool() {
        let mut s = ChatState::new();
        s.tool_use_end("call-1", "shell_exec", r#"{"command":"echo precise"}"#);
        s.tool_result("call-1", "shell_exec", "ok", false);
        s.messages[0].tool.as_mut().unwrap().completed_at = None;

        assert!(!should_render_tool_expanded(
            s.messages[0].tool.as_ref().unwrap()
        ));

        let action = s.handle_key(KeyEvent::new(KeyCode::Char('e'), KeyModifiers::CONTROL));
        assert!(matches!(action, ChatAction::Continue));
        assert!(s.messages[0].tool.as_ref().unwrap().expanded);

        let action = s.handle_key(KeyEvent::new(KeyCode::Char('e'), KeyModifiers::CONTROL));
        assert!(matches!(action, ChatAction::Continue));
        assert!(!s.messages[0].tool.as_ref().unwrap().expanded);
    }

    #[test]
    fn execute_code_summary_is_not_a_copyable_command() {
        let mut s = ChatState::new();
        s.tool_use_end(
            "call-1",
            "execute_code",
            r#"{"language":"python","code":"print('hello')"}"#,
        );

        assert_eq!(s.last_command_to_copy(), None);
    }

    #[test]
    fn process_start_command_includes_shell_quoted_args() {
        let mut s = ChatState::new();
        s.tool_use_end(
            "call-1",
            "process_start",
            r#"{"command":"python","args":["-iu","script with spaces.py"]}"#,
        );

        assert_eq!(
            s.last_command_to_copy().as_deref(),
            Some("python -iu 'script with spaces.py'")
        );
    }
}

#[cfg(test)]
mod tests_paste {
    use super::*;

    #[test]
    fn paste_appends_into_empty_input() {
        let mut s = ChatState::new();
        s.handle_paste("Hello world");
        assert_eq!(s.input, "Hello world");
    }

    #[test]
    fn paste_appends_after_existing_input() {
        let mut s = ChatState::new();
        // After direct assignment we must align the cursor so handle_paste
        // (cursor-aware) inserts at the same logical end-of-buffer position
        // a real keystroke pipeline would produce.
        s.input = "Hello ".into();
        s.input_cursor = s.input.len();
        s.handle_paste("from clipboard");
        assert_eq!(s.input, "Hello from clipboard");
    }

    #[test]
    fn paste_with_newlines_keeps_them_as_content_no_submit() {
        // Core ghost-failure this fixes: terminals without bracketed paste
        // turn a multi-line clipboard into N successive Enter presses, each
        // of which submits a partial message. handle_paste must keep the
        // newlines as plain content inside the buffer.
        let mut s = ChatState::new();
        s.handle_paste("line1\nline2\nline3");
        assert_eq!(s.input, "line1\nline2\nline3");
        assert!(
            s.input.contains('\n'),
            "newlines must survive in the input buffer"
        );
        assert_eq!(
            s.input.matches('\n').count(),
            2,
            "exactly the pasted newlines should be present"
        );
    }

    #[test]
    fn paste_with_cr_line_endings_is_normalized_to_lf() {
        // Observed live: a multi-line prompt pasted into the TUI showed up
        // as one flat visual block. Root cause: some sources deliver
        // bracketed-paste data with `\r`/`\r\n` endings, which every
        // downstream consumer of `input` (split on '\n' only) treats as a
        // single giant logical line.
        let mut s = ChatState::new();
        s.handle_paste("line1\r\nline2\rline3");
        assert_eq!(s.input, "line1\nline2\nline3");
        assert_eq!(s.input.matches('\n').count(), 2);
        assert!(!s.input.contains('\r'));
    }

    #[test]
    fn normalize_pasted_line_endings_is_noop_without_cr() {
        assert_eq!(normalize_pasted_line_endings("plain text"), "plain text");
        assert_eq!(
            normalize_pasted_line_endings("line1\nline2"),
            "line1\nline2"
        );
    }

    #[test]
    fn paste_during_streaming_buffers_into_input() {
        let mut s = ChatState::new();
        s.is_streaming = true;
        s.handle_paste("queued");
        assert_eq!(s.input, "queued");
    }
}

#[cfg(test)]
mod cursor_tests {
    use super::*;

    fn s() -> ChatState {
        ChatState::new()
    }

    #[test]
    fn insert_advances_cursor() {
        let mut st = s();
        st.input_insert_char('a');
        st.input_insert_char('b');
        st.input_insert_char('c');
        assert_eq!(st.input, "abc");
        assert_eq!(st.input_cursor, 3);
    }

    #[test]
    fn left_then_insert_lands_in_the_middle() {
        let mut st = s();
        st.input_insert_str("ac");
        st.input_move_left();
        st.input_insert_char('b');
        assert_eq!(st.input, "abc");
        assert_eq!(st.input_cursor, 2);
    }

    #[test]
    fn backspace_deletes_char_before_cursor() {
        let mut st = s();
        st.input_insert_str("abcd");
        st.input_move_left(); // cursor between c and d
        st.input_backspace(); // removes c
        assert_eq!(st.input, "abd");
        assert_eq!(st.input_cursor, 2);
    }

    #[test]
    fn delete_forward_removes_char_at_cursor() {
        let mut st = s();
        st.input_insert_str("abcd");
        st.input_move_home(); // cursor at 0
        st.input_delete_forward();
        assert_eq!(st.input, "bcd");
        assert_eq!(st.input_cursor, 0);
    }

    #[test]
    fn home_and_end_jump_extremes() {
        let mut st = s();
        st.input_insert_str("hello");
        st.input_move_home();
        assert_eq!(st.input_cursor, 0);
        st.input_move_end();
        assert_eq!(st.input_cursor, 5);
    }

    #[test]
    fn left_at_zero_is_noop() {
        let mut st = s();
        st.input_insert_char('x');
        st.input_move_left();
        assert_eq!(st.input_cursor, 0);
        st.input_move_left();
        assert_eq!(st.input_cursor, 0);
    }

    #[test]
    fn right_at_end_is_noop() {
        let mut st = s();
        st.input_insert_str("xy");
        st.input_move_right();
        assert_eq!(st.input_cursor, 2);
    }

    /// Multi-byte UTF-8: the cursor must always land on a char boundary.
    /// `é` is 2 bytes, an emoji typically 4. A naive byte-index cursor
    /// would split mid-char and panic on `String::insert`.
    #[test]
    fn cursor_respects_utf8_boundaries() {
        let mut st = s();
        st.input_insert_str("café"); // 5 bytes
        assert_eq!(st.input_cursor, 5);
        st.input_move_left(); // skip past 'é' (2 bytes)
        assert_eq!(st.input_cursor, 3);
        st.input_backspace(); // removes 'f'
        assert_eq!(st.input, "caé");
    }

    #[test]
    fn insert_str_at_middle_keeps_surrounding() {
        let mut st = s();
        st.input_insert_str("ad");
        st.input_move_left();
        st.input_insert_str("bc"); // between a and d
        assert_eq!(st.input, "abcd");
        assert_eq!(st.input_cursor, 3);
    }

    #[test]
    fn input_clear_resets_cursor() {
        let mut st = s();
        st.input_insert_str("hello world");
        st.input_clear();
        assert_eq!(st.input, "");
        assert_eq!(st.input_cursor, 0);
    }

    /// Up/Down on a single-line draft must report "didn't move" so the
    /// caller falls back to scrolling the message history.
    #[test]
    fn up_down_line_noop_on_single_line() {
        let mut st = s();
        st.input_insert_str("hello");
        assert!(!st.input_move_up_line());
        assert!(!st.input_move_down_line());
        // Cursor stays put.
        assert_eq!(st.input_cursor, 5);
    }

    #[test]
    fn user_message_returns_scrollback_to_live_bottom() {
        let mut st = s();
        st.scroll_page_up();
        assert!(st.scroll_offset > 0);
        st.push_message(Role::User, "reprends la main".into());
        assert_eq!(st.scroll_offset, 0);
    }

    #[test]
    fn system_message_preserves_manual_scrollback_position() {
        let mut st = s();
        st.scroll_page_up();
        let before = st.scroll_offset;
        st.push_message(Role::System, "notification".into());
        assert_eq!(st.scroll_offset, before);
    }

    #[test]
    fn ctrl_b_and_ctrl_f_scroll_history_by_page() {
        let mut st = s();
        let up = KeyEvent::new(KeyCode::Char('b'), KeyModifiers::CONTROL);
        let down = KeyEvent::new(KeyCode::Char('f'), KeyModifiers::CONTROL);

        st.handle_key(up);
        assert_eq!(st.scroll_offset, super::HISTORY_PAGE_SCROLL);

        st.handle_key(down);
        assert_eq!(st.scroll_offset, 0);
    }

    /// Multi-line: Up moves to the previous line at the same column.
    #[test]
    fn up_line_moves_to_previous_with_column_preserved() {
        let mut st = s();
        st.input_insert_str("first\nsecond\nthird");
        // Cursor at end of "third" → line=2, col=5.
        assert!(st.input_move_up_line());
        // Now at line=1 ("second"), col clamped to 5 (within "second").
        let (l, c) = super::locate_cursor(&st.input, st.input_cursor);
        assert_eq!((l, c), (1, 5));
        assert!(st.input_move_up_line());
        let (l, c) = super::locate_cursor(&st.input, st.input_cursor);
        assert_eq!((l, c), (0, 5));
        // Already on the first line — Up returns false.
        assert!(!st.input_move_up_line());
    }

    /// Down must clamp to the next line's length when the previous column
    /// would overflow it.
    #[test]
    fn down_line_clamps_column_to_shorter_target() {
        let mut st = s();
        st.input_insert_str("longer line\nshort");
        st.input_move_home(); // line 0, col 0
                              // Move forward to col 9 ("longer li|ne"), still on line 0.
        for _ in 0..9 {
            st.input_move_right();
        }
        let (l, c) = super::locate_cursor(&st.input, st.input_cursor);
        assert_eq!((l, c), (0, 9));
        assert!(st.input_move_down_line());
        // "short" is 5 chars, so the column is clamped to 5.
        let (l, c) = super::locate_cursor(&st.input, st.input_cursor);
        assert_eq!((l, c), (1, 5));
    }
}

// ---------------------------------------------------------------------------
// #182 — Boot session resume must survive the inevitable `chat.reset()` that
// happens when the daemon-detect path drops the user into `enter_chat_*`.
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests_boot_resume_182 {
    use super::*;
    use crate::tui::session_store::{PersistedMessage, PersistedSession};

    /// Write a fixture session to a temp file and return its path.
    fn write_fixture(messages: Vec<(&str, &str)>) -> std::path::PathBuf {
        let tmp = std::env::temp_dir().join(format!(
            "captain_test_182_{}_{}.json",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0)
        ));
        let session = PersistedSession {
            session_id: None,
            agent_id: None,
            agent_name: "captain".into(),
            model_label: "anthropic/claude-sonnet-4".into(),
            mode_label: "in-process".into(),
            messages: messages
                .into_iter()
                .map(|(role, text)| PersistedMessage {
                    role: role.into(),
                    text: text.into(),
                    tool: None,
                })
                .collect(),
            session_input_tokens: 100,
            session_output_tokens: 50,
            session_cached_input_tokens: 0,
            session_cache_creation_tokens: 0,
            session_cost_usd: 0.001,
            created_at: 1_700_000_000,
            updated_at: 0,
        };
        std::fs::write(&tmp, serde_json::to_string_pretty(&session).unwrap()).unwrap();
        tmp
    }

    /// Reproduce the bug's required ordering: a chat that just had its
    /// state replayed gets `reset()`'d (because daemon detect routes
    /// through `enter_chat_*`), and the rehydration MUST be re-applied
    /// afterwards. This test pins the post-reset path the fix relies on.
    #[test]
    fn replay_after_reset_restores_visible_history() {
        let path = write_fixture(vec![
            ("user", "salut"),
            ("agent", "yo"),
            ("user", "tu peux faire X ?"),
        ]);

        let mut chat = ChatState::new();
        // Simulate the pre-reset state: messages from a prior agent + dirty input.
        chat.push_message(Role::Agent, "stale content".into());
        chat.input.push_str("draft");

        // enter_chat_* always resets first.
        chat.reset();
        assert!(chat.messages.is_empty(), "reset must clear messages");
        assert!(chat.input.is_empty(), "reset must clear input");
        assert!(chat.session_path.is_none(), "reset must drop session_path");

        // The fix: re-apply the persisted session AFTER reset.
        chat.replay_session_from("test-agent", &path);

        assert_eq!(
            chat.messages.len(),
            3,
            "all 3 history turns must be visible"
        );
        assert_eq!(chat.messages[0].text, "salut");
        assert_eq!(chat.messages[1].text, "yo");
        assert_eq!(chat.messages[2].text, "tu peux faire X ?");
        assert_eq!(
            chat.session_path,
            Some(path.clone()),
            "the resumed session_path must rebind so subsequent persists \
             keep writing to the same on-disk file"
        );
        assert_eq!(chat.session_input_tokens, 100);
        assert_eq!(chat.session_output_tokens, 50);

        let _ = std::fs::remove_file(&path);
    }

    /// Defensive: a corrupted / missing path must NOT leave the chat in a
    /// half-rehydrated state. The bug here would be losing every message
    /// the caller had set up *before* the failed replay attempt.
    #[test]
    fn replay_from_missing_path_is_a_noop() {
        let mut chat = ChatState::new();
        chat.push_message(Role::Agent, "kept".into());

        let bogus = std::env::temp_dir().join("captain_test_182_does_not_exist_12345.json");
        chat.replay_session_from("test-agent", &bogus);

        assert_eq!(
            chat.messages.len(),
            1,
            "missing-path replay must not wipe state"
        );
        assert_eq!(chat.messages[0].text, "kept");
    }
}
