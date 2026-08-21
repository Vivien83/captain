mod model;
mod remote;
mod view;

use self::{
    model::{ChatMessage, MessageRole, SessionInfo, StreamSignal},
    remote::{
        answer_question, bootstrap, create_session, load_session, spawn_message_stream,
        ChatBootstrap, ConsoleRemoteError,
    },
};
use crate::{
    manager::ConsoleConnection, ConsoleManager, ConsoleManagerError, ConsoleProfileSummary,
};
use captain_node::ClientAccessTransport;
use ratatui::crossterm::{
    event::{
        self, DisableBracketedPaste, EnableBracketedPaste, Event, KeyCode, KeyEvent, KeyEventKind,
        KeyModifiers,
    },
    execute,
};
use std::{
    io,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    time::Duration,
};
use tokio::{sync::mpsc, task::JoinHandle};

const MAX_TRANSCRIPT_MESSAGES: usize = 400;
const MAX_STREAMED_ASSISTANT_BYTES: usize = 1024 * 1024;
const MAX_INPUT_BYTES: usize = 64 * 1024;

pub async fn run_tui(profile: Option<&str>) -> Result<(), ConsoleTuiError> {
    install_panic_hook();
    let manager = ConsoleManager::open_default()?;
    let profiles = manager
        .list()?
        .into_iter()
        .filter(|profile| profile.configured)
        .collect::<Vec<_>>();
    if profiles.is_empty() {
        return Err(ConsoleTuiError::NoPairedCaptain);
    }
    let selector = match profile {
        Some(selector) => selector.to_string(),
        None if profiles.len() == 1 => profiles[0].id.clone(),
        None => choose_profile(&profiles)?,
    };
    let mut connection = manager.connect(&selector)?;
    let initial = bootstrap(&connection)
        .await
        .map_err(ConsoleTuiError::from_remote)?;
    connection.profile = manager.activate(&connection.profile.id)?;
    run_chat(connection, initial).await
}

fn choose_profile(profiles: &[ConsoleProfileSummary]) -> Result<String, ConsoleTuiError> {
    let mut selected = profiles
        .iter()
        .position(|profile| profile.active)
        .unwrap_or(0);
    let mut terminal = ratatui::init();
    let _terminal_guard = TerminalRestoreGuard::plain();
    loop {
        terminal.draw(|frame| view::draw_profile_picker(frame, profiles, selected))?;
        if !event::poll(Duration::from_millis(100))? {
            continue;
        }
        let Event::Key(key) = event::read()? else {
            continue;
        };
        if key.kind != KeyEventKind::Press {
            continue;
        }
        match key.code {
            KeyCode::Up | KeyCode::Char('k') => selected = selected.saturating_sub(1),
            KeyCode::Down | KeyCode::Char('j') => {
                selected = (selected + 1).min(profiles.len().saturating_sub(1));
            }
            KeyCode::Enter => {
                return Ok(profiles[selected].id.clone());
            }
            KeyCode::Esc => {
                return Err(ConsoleTuiError::SelectionCancelled);
            }
            _ => {}
        }
    }
}

async fn run_chat(
    connection: ConsoleConnection,
    initial: ChatBootstrap,
) -> Result<(), ConsoleTuiError> {
    let mut terminal = ratatui::init();
    let _terminal_guard = TerminalRestoreGuard::bracketed()?;
    let mut inputs = TerminalEvents::spawn()?;
    let (stream_tx, mut stream_rx) = mpsc::unbounded_channel();
    let (operation_tx, mut operation_rx) = mpsc::unbounded_channel();
    let mut app = ConsoleApp::new(connection, initial);

    let result = loop {
        terminal.draw(|frame| view::draw_console(frame, &app))?;
        tokio::select! {
            input = inputs.recv() => {
                let Some(input) = input else {
                    break Ok(());
                };
                match app.handle_input(input, &stream_tx, &operation_tx) {
                    AppControl::Continue => {}
                    AppControl::Quit => break Ok(()),
                }
            }
            stream = stream_rx.recv() => {
                if let Some(stream) = stream {
                    app.handle_stream(stream);
                }
            }
            operation = operation_rx.recv() => {
                if let Some(operation) = operation {
                    app.handle_operation(operation);
                }
            }
        }
    };

    app.abort_local_stream();
    inputs.stop();
    result
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Focus {
    Sessions,
    Input,
}

pub(super) struct ConsoleApp {
    profile: ConsoleProfileSummary,
    transport: Arc<ClientAccessTransport>,
    agent_id: String,
    agent_name: String,
    model: String,
    sessions: Vec<SessionInfo>,
    selected_session: usize,
    loaded_session_id: String,
    messages: Vec<ChatMessage>,
    stream_buffer: String,
    input: String,
    focus: Focus,
    scroll: u16,
    status: String,
    busy: bool,
    streaming: bool,
    pending_question: bool,
    stream_handle: Option<JoinHandle<()>>,
}

impl ConsoleApp {
    fn new(connection: ConsoleConnection, initial: ChatBootstrap) -> Self {
        let loaded_session_id = initial.sessions[initial.selected_session].id.clone();
        Self {
            profile: connection.profile,
            transport: connection.transport,
            agent_id: initial.agent.id,
            agent_name: initial.agent.name,
            model: initial.agent.model,
            sessions: initial.sessions,
            selected_session: initial.selected_session,
            loaded_session_id,
            messages: initial.messages,
            stream_buffer: String::new(),
            input: String::new(),
            focus: Focus::Input,
            scroll: 0,
            status: "Ready".to_string(),
            busy: false,
            streaming: false,
            pending_question: false,
            stream_handle: None,
        }
    }

    fn handle_input(
        &mut self,
        input: TerminalInput,
        stream_tx: &mpsc::UnboundedSender<Result<StreamSignal, ConsoleRemoteError>>,
        operation_tx: &mpsc::UnboundedSender<AppOperation>,
    ) -> AppControl {
        match input {
            TerminalInput::Paste(text) => {
                if self.focus == Focus::Input {
                    self.append_input(&text);
                }
            }
            TerminalInput::Key(key) => return self.handle_key(key, stream_tx, operation_tx),
            TerminalInput::Resize => {}
        }
        AppControl::Continue
    }

    fn handle_key(
        &mut self,
        key: KeyEvent,
        stream_tx: &mpsc::UnboundedSender<Result<StreamSignal, ConsoleRemoteError>>,
        operation_tx: &mpsc::UnboundedSender<AppOperation>,
    ) -> AppControl {
        if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('c') {
            return AppControl::Quit;
        }
        if key.code == KeyCode::Tab {
            self.focus = match self.focus {
                Focus::Sessions => Focus::Input,
                Focus::Input => Focus::Sessions,
            };
            return AppControl::Continue;
        }
        if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('n') {
            self.begin_new_session(operation_tx);
            return AppControl::Continue;
        }
        if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('r') {
            self.begin_load_selected(operation_tx);
            return AppControl::Continue;
        }
        match key.code {
            KeyCode::PageUp => self.scroll = self.scroll.saturating_add(12),
            KeyCode::PageDown => self.scroll = self.scroll.saturating_sub(12),
            _ if self.focus == Focus::Sessions => self.handle_session_key(key, operation_tx),
            _ => self.handle_input_key(key, stream_tx, operation_tx),
        }
        AppControl::Continue
    }

    fn handle_session_key(
        &mut self,
        key: KeyEvent,
        operation_tx: &mpsc::UnboundedSender<AppOperation>,
    ) {
        match key.code {
            KeyCode::Up | KeyCode::Char('k') => {
                self.selected_session = self.selected_session.saturating_sub(1);
            }
            KeyCode::Down | KeyCode::Char('j') => {
                self.selected_session =
                    (self.selected_session + 1).min(self.sessions.len().saturating_sub(1));
            }
            KeyCode::Enter => self.begin_load_selected(operation_tx),
            _ => {}
        }
    }

    fn handle_input_key(
        &mut self,
        key: KeyEvent,
        stream_tx: &mpsc::UnboundedSender<Result<StreamSignal, ConsoleRemoteError>>,
        operation_tx: &mpsc::UnboundedSender<AppOperation>,
    ) {
        match key.code {
            KeyCode::Enter if key.modifiers.contains(KeyModifiers::SHIFT) => {
                self.append_input("\n");
            }
            KeyCode::Enter => self.submit_input(stream_tx, operation_tx),
            KeyCode::Backspace => {
                self.input.pop();
            }
            KeyCode::Esc => self.input.clear(),
            KeyCode::Char(character)
                if !key
                    .modifiers
                    .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT) =>
            {
                self.append_input(&character.to_string());
            }
            _ => {}
        }
    }

    fn append_input(&mut self, value: &str) {
        let value = value
            .chars()
            .map(|character| {
                if matches!(character, '\n' | '\t') || !character.is_control() {
                    character
                } else {
                    ' '
                }
            })
            .collect::<String>();
        let remaining = MAX_INPUT_BYTES.saturating_sub(self.input.len());
        if remaining == 0 {
            self.status = "Input limit reached (64 KiB)".to_string();
            return;
        }
        let end = floor_char_boundary(&value, remaining.min(value.len()));
        self.input.push_str(&value[..end]);
        if end < value.len() {
            self.status = "Input truncated at 64 KiB".to_string();
        }
    }

    fn submit_input(
        &mut self,
        stream_tx: &mpsc::UnboundedSender<Result<StreamSignal, ConsoleRemoteError>>,
        operation_tx: &mpsc::UnboundedSender<AppOperation>,
    ) {
        let message = self.input.trim().to_string();
        if message.is_empty() || self.busy {
            return;
        }
        if self.pending_question {
            self.busy = true;
            self.status = "Sending answer...".to_string();
            let transport = Arc::clone(&self.transport);
            let agent_id = self.agent_id.clone();
            let session_id = self.loaded_session_id.clone();
            let sender = operation_tx.clone();
            tokio::spawn(async move {
                let result = answer_question(&transport, &agent_id, &session_id, message.clone())
                    .await
                    .map(|()| message);
                let _ = sender.send(AppOperation::QuestionAnswered(result));
            });
            return;
        }
        if self.streaming {
            self.status = "Captain is still working".to_string();
            return;
        }
        self.input.clear();
        self.push_message(ChatMessage {
            role: MessageRole::User,
            content: message.clone(),
        });
        self.stream_buffer.clear();
        self.streaming = true;
        self.status = "Captain is working...".to_string();
        self.scroll = 0;
        match spawn_message_stream(
            Arc::clone(&self.transport),
            self.agent_id.clone(),
            self.loaded_session_id.clone(),
            message,
            stream_tx.clone(),
        ) {
            Ok(handle) => self.stream_handle = Some(handle),
            Err(error) => self.finish_stream_error(error),
        }
    }

    fn begin_load_selected(&mut self, operation_tx: &mpsc::UnboundedSender<AppOperation>) {
        if self.busy || self.streaming || self.sessions.is_empty() {
            self.status = if self.streaming {
                "Wait for the current Captain turn before changing session".to_string()
            } else {
                self.status.clone()
            };
            return;
        }
        let session_id = self.sessions[self.selected_session].id.clone();
        self.busy = true;
        self.status = "Restoring session...".to_string();
        let transport = Arc::clone(&self.transport);
        let sender = operation_tx.clone();
        tokio::spawn(async move {
            let result = load_session(&transport, &session_id).await;
            let _ = sender.send(AppOperation::SessionLoaded { session_id, result });
        });
    }

    fn begin_new_session(&mut self, operation_tx: &mpsc::UnboundedSender<AppOperation>) {
        if self.busy || self.streaming {
            self.status = "Wait for the current Captain turn before creating a session".to_string();
            return;
        }
        self.busy = true;
        self.status = "Creating a detached session...".to_string();
        let transport = Arc::clone(&self.transport);
        let agent_id = self.agent_id.clone();
        let sender = operation_tx.clone();
        tokio::spawn(async move {
            let result = create_session(&transport, &agent_id).await;
            let _ = sender.send(AppOperation::SessionCreated(result));
        });
    }

    fn handle_stream(&mut self, event: Result<StreamSignal, ConsoleRemoteError>) {
        match event {
            Ok(StreamSignal::Text(text)) => {
                let remaining =
                    MAX_STREAMED_ASSISTANT_BYTES.saturating_sub(self.stream_buffer.len());
                let end = floor_char_boundary(&text, remaining.min(text.len()));
                self.stream_buffer.push_str(&text[..end]);
                if end < text.len() {
                    self.status = "Assistant output capped at 1 MiB".to_string();
                }
                self.scroll = 0;
            }
            Ok(StreamSignal::ToolStarted(name)) => {
                self.push_activity(format!("Tool started: {name}"));
                self.status = format!("Running {name}...");
            }
            Ok(StreamSignal::ToolFinished { name, failed }) => {
                self.push_activity(format!(
                    "Tool {}: {name}",
                    if failed { "failed" } else { "completed" }
                ));
                self.status = if failed {
                    format!("{name} failed")
                } else {
                    format!("{name} completed")
                };
            }
            Ok(StreamSignal::Phase(phase)) => self.status = phase,
            Ok(StreamSignal::AskUser { question, options }) => {
                let choices = if options.is_empty() {
                    String::new()
                } else {
                    format!("\n{}", options.join("  |  "))
                };
                self.push_activity(format!("Captain asks: {question}{choices}"));
                self.pending_question = true;
                self.status = "Answer Captain in the input field".to_string();
            }
            Ok(StreamSignal::Done) => self.finish_stream(),
            Err(error) => self.finish_stream_error(error),
        }
    }

    fn handle_operation(&mut self, operation: AppOperation) {
        self.busy = false;
        match operation {
            AppOperation::SessionLoaded { session_id, result } => match result {
                Ok(messages) => {
                    self.loaded_session_id = session_id.clone();
                    if let Some(index) = self
                        .sessions
                        .iter()
                        .position(|session| session.id == session_id)
                    {
                        self.selected_session = index;
                    }
                    self.messages = messages;
                    self.stream_buffer.clear();
                    self.pending_question = false;
                    self.scroll = 0;
                    self.status = "Session restored".to_string();
                }
                Err(error) => self.status = error.to_string(),
            },
            AppOperation::SessionCreated(result) => match result {
                Ok(session) => {
                    self.loaded_session_id = session.id.clone();
                    self.sessions.insert(0, session);
                    self.selected_session = 0;
                    self.messages.clear();
                    self.stream_buffer.clear();
                    self.pending_question = false;
                    self.scroll = 0;
                    self.status = "New detached session ready".to_string();
                }
                Err(error) => self.status = error.to_string(),
            },
            AppOperation::QuestionAnswered(result) => match result {
                Ok(answer) => {
                    self.input.clear();
                    self.pending_question = false;
                    self.push_message(ChatMessage {
                        role: MessageRole::User,
                        content: answer,
                    });
                    self.status = "Answer delivered; Captain resumed".to_string();
                }
                Err(error) => self.status = error.to_string(),
            },
        }
    }

    fn finish_stream(&mut self) {
        if !self.stream_buffer.is_empty() {
            let content = std::mem::take(&mut self.stream_buffer);
            self.push_message(ChatMessage {
                role: MessageRole::Assistant,
                content,
            });
        }
        self.streaming = false;
        self.pending_question = false;
        self.stream_handle = None;
        self.status = "Ready".to_string();
        self.scroll = 0;
    }

    fn finish_stream_error(&mut self, error: ConsoleRemoteError) {
        if !self.stream_buffer.is_empty() {
            let content = std::mem::take(&mut self.stream_buffer);
            self.push_message(ChatMessage {
                role: MessageRole::Assistant,
                content,
            });
        }
        self.streaming = false;
        self.pending_question = false;
        self.stream_handle = None;
        self.status = error.to_string();
    }

    fn push_activity(&mut self, content: String) {
        self.push_message(ChatMessage {
            role: MessageRole::System,
            content,
        });
        self.scroll = 0;
    }

    fn push_message(&mut self, message: ChatMessage) {
        self.messages.push(message);
        let excess = self.messages.len().saturating_sub(MAX_TRANSCRIPT_MESSAGES);
        if excess > 0 {
            self.messages.drain(..excess);
        }
    }

    fn abort_local_stream(&mut self) {
        if let Some(handle) = self.stream_handle.take() {
            handle.abort();
        }
    }
}

enum AppOperation {
    SessionLoaded {
        session_id: String,
        result: Result<Vec<ChatMessage>, ConsoleRemoteError>,
    },
    SessionCreated(Result<SessionInfo, ConsoleRemoteError>),
    QuestionAnswered(Result<String, ConsoleRemoteError>),
}

#[derive(Debug)]
enum TerminalInput {
    Key(KeyEvent),
    Paste(String),
    Resize,
}

struct TerminalEvents {
    receiver: mpsc::UnboundedReceiver<TerminalInput>,
    stop: Arc<AtomicBool>,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl TerminalEvents {
    fn spawn() -> Result<Self, ConsoleTuiError> {
        let (sender, receiver) = mpsc::unbounded_channel();
        let stop = Arc::new(AtomicBool::new(false));
        let thread_stop = Arc::clone(&stop);
        let thread = std::thread::Builder::new()
            .name("captain-console-input".to_string())
            .spawn(move || {
                while !thread_stop.load(Ordering::Relaxed) {
                    match event::poll(Duration::from_millis(50)) {
                        Ok(true) => match event::read() {
                            Ok(Event::Key(key)) if key.kind == KeyEventKind::Press => {
                                if sender.send(TerminalInput::Key(key)).is_err() {
                                    break;
                                }
                            }
                            Ok(Event::Paste(value)) => {
                                if sender.send(TerminalInput::Paste(value)).is_err() {
                                    break;
                                }
                            }
                            Ok(Event::Resize(_, _)) => {
                                if sender.send(TerminalInput::Resize).is_err() {
                                    break;
                                }
                            }
                            Ok(_) | Err(_) => {}
                        },
                        Ok(false) => {}
                        Err(_) => break,
                    }
                }
            })
            .map_err(|_| ConsoleTuiError::InputUnavailable)?;
        Ok(Self {
            receiver,
            stop,
            thread: Some(thread),
        })
    }

    async fn recv(&mut self) -> Option<TerminalInput> {
        self.receiver.recv().await
    }

    fn stop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

impl Drop for TerminalEvents {
    fn drop(&mut self) {
        self.stop();
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AppControl {
    Continue,
    Quit,
}

fn floor_char_boundary(value: &str, mut index: usize) -> usize {
    while index > 0 && !value.is_char_boundary(index) {
        index -= 1;
    }
    index
}

fn install_panic_hook() {
    let original = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let _ = execute!(io::stdout(), DisableBracketedPaste);
        ratatui::restore();
        original(info);
    }));
}

struct TerminalRestoreGuard {
    bracketed: bool,
}

impl TerminalRestoreGuard {
    fn plain() -> Self {
        Self { bracketed: false }
    }

    fn bracketed() -> Result<Self, ConsoleTuiError> {
        if let Err(error) = execute!(io::stdout(), EnableBracketedPaste) {
            ratatui::restore();
            return Err(error.into());
        }
        Ok(Self { bracketed: true })
    }
}

impl Drop for TerminalRestoreGuard {
    fn drop(&mut self) {
        if self.bracketed {
            let _ = execute!(io::stdout(), DisableBracketedPaste);
        }
        ratatui::restore();
    }
}

#[derive(Debug, thiserror::Error)]
pub enum ConsoleTuiError {
    #[error(transparent)]
    Manager(#[from] ConsoleManagerError),
    #[error("no paired Captain is configured; run `captain-console pair` first")]
    NoPairedCaptain,
    #[error("Captain selection was cancelled")]
    SelectionCancelled,
    #[error("the terminal input worker is unavailable")]
    InputUnavailable,
    #[error("Captain Console terminal I/O failed")]
    TerminalIo(#[from] io::Error),
    #[error("{0}")]
    Remote(String),
}

impl ConsoleTuiError {
    fn from_remote(error: ConsoleRemoteError) -> Self {
        Self::Remote(error.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn utf8_input_cap_never_splits_a_character() {
        assert_eq!(floor_char_boundary("abé", 3), 2);
        assert_eq!(floor_char_boundary("abé", 4), 4);
    }
}
