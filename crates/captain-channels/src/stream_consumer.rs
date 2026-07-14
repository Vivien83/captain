//! Live streaming for chat-like channels (HS.1).
//!
//! The consumer drains a stream of [`ChannelStreamEvent`]s coming from the
//! agent loop and translates them into a *succession* of channel messages:
//!
//! ```text
//!   text (edited live, cursor ▉) → tool bubble → text (live) → tool → final text
//! ```
//!
//! This feels much more human than buffering the whole reply and posting one
//! big message at the end. Captain ships native progressive delivery here.
//!
//! ## Decoupling from `captain-runtime`
//!
//! Channel adapters live in this crate and must NOT depend on the runtime
//! crate (it would create a layering circle and pull half the workspace
//! into every adapter). [`ChannelStreamEvent`] is therefore a *neutral*
//! re-statement of the event vocabulary; the mapping from
//! `captain_runtime::llm_driver::StreamEvent` to this enum lives one
//! layer up, in `captain-api` (HS.2 / HS.3).
//!
//! ## What HS.1 does NOT do
//!
//! - No Telegram wiring (HS.2).
//! - No MarkdownV2 escaping (HS.3 — kept plain-text for now).
//! - No 4096-char overflow split (HS.4).
//! - No flood-control adaptive interval (HS.2).

use async_trait::async_trait;
use std::collections::{HashMap, VecDeque};
use std::time::{Duration, Instant};

/// Default cadence at which we re-edit the live message. Telegram applies
/// aggressive per-chat edit flood control on long turns; keep this deliberately
/// below one edit per second so tool-heavy Codex streams have room for tool
/// bubbles and final delivery.
pub const DEFAULT_EDIT_INTERVAL_MS: u64 = 2_500;
pub const DEFAULT_TOOL_PROGRESS_EDIT_INTERVAL_MS: u64 = 1_000;

/// Minimum byte length the accumulator must reach before we edit early
/// (i.e. before the timer fires). Keep this low so short useful chunks appear.
pub const DEFAULT_BUFFER_THRESHOLD: usize = 40;

/// Cursor glyph appended to the live message while it is still streaming.
/// Removed on the final flush. U+2589 LEFT THREE QUARTERS BLOCK.
pub const STREAM_CURSOR: &str = " ▉";

/// Neutral re-statement of the runtime's `StreamEvent` for channel
/// consumption. Only the fields a chat surface actually needs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ChannelStreamEvent {
    /// Append text to the *current* live message.
    TextDelta(String),
    /// A tool call started. The current text message is finalised
    /// (cursor removed) and a tool bubble is emitted as a NEW message.
    ///
    /// `emoji` is a compact prefix (`💻`, `📖`, …) the upstream layer
    /// chose for the tool family. Empty string means "no preference",
    /// the renderer falls back to a generic arrow. HS.6.
    ToolStart {
        tool_use_id: String,
        emoji: String,
        name: String,
        input_preview: String,
    },
    /// A tool call finished. The matching tool bubble is updated with
    /// its result preview. After this, the next `TextDelta` opens a
    /// fresh live message — *that's* what makes a "semi-response"
    /// possible between tools without coupling to a Commentary event.
    ToolEnd {
        tool_use_id: String,
        emoji: String,
        name: String,
        result_preview: String,
        is_error: bool,
    },
    /// Semantic progress for a currently-running tool. Unlike generic
    /// commentary, this edits the existing tool bubble when possible,
    /// so Telegram does not receive a burst of standalone messages.
    ToolProgress { tool_use_id: String, chunk: String },
    /// An intermediate "semi-response" the agent emits proactively
    /// (Captain's `IntermediateMessage`). Opens a NEW live message immediately,
    /// so the user sees the thought before the next tool call starts.
    Commentary(String),
    /// Stream is over. Flush whatever's pending, drop the cursor.
    Done,
}

/// What a chat platform must expose so the consumer can drive it.
///
/// `send_new_message` and `edit_message` are the two ops that matter.
/// `supports_edits` lets adapters that can't edit (Email, SMS) opt out
/// of progressive rendering — the consumer falls back to "send one
/// message on Done".
#[async_trait]
pub trait StreamingChannelAdapter: Send + Sync {
    /// Post a brand-new message; return the platform's message id so
    /// subsequent `edit_message` calls can target it.
    async fn send_new_message(&self, text: &str) -> Result<String, String>;

    /// Replace the body of an existing message.
    async fn edit_message(&self, message_id: &str, text: &str) -> Result<(), String>;

    /// `false` for write-once channels (Email, SMS). The consumer
    /// degrades to a single `send_new_message` on `Done` in that case.
    fn supports_edits(&self) -> bool {
        true
    }

    /// HS.9 — hint: the next `send_new_message` should be treated as
    /// the "final reply" and may carry a `reply_to` quote of the
    /// user's prompt. Adapters that don't model quoted replies
    /// inherit the no-op default.
    fn arm_final_reply_quote(&self) {}
}

/// Telegram's hard ceiling per message body, kept here so the consumer
/// can split before the platform refuses an edit. HS.4.
pub const DEFAULT_MAX_MESSAGE_BYTES: usize = 4096;

/// Tunable behaviour.
#[derive(Debug, Clone)]
pub struct StreamConsumerConfig {
    pub edit_interval: Duration,
    pub buffer_threshold: usize,
    pub cursor: &'static str,
    /// HS.4 — when an edit would push the rendered body (text + cursor)
    /// past this many bytes, the consumer finalises the current message
    /// and rolls the overflow into a fresh one. 0 disables splitting
    /// (channels with no length cap, or tests).
    pub max_message_bytes: usize,
}

impl Default for StreamConsumerConfig {
    fn default() -> Self {
        Self {
            edit_interval: Duration::from_millis(DEFAULT_EDIT_INTERVAL_MS),
            buffer_threshold: DEFAULT_BUFFER_THRESHOLD,
            cursor: STREAM_CURSOR,
            max_message_bytes: DEFAULT_MAX_MESSAGE_BYTES,
        }
    }
}

/// HS.4 — Split `text` so the head fits within `max_bytes`, preferring
/// (in order) a paragraph break, a line break, a sentence boundary, a
/// word break, then a UTF-8 char boundary. Returns `(head, tail)` where
/// `head.len() <= max_bytes`. The split point is consumed: a `\n` cut
/// vanishes, a space cut vanishes, a sentence cut keeps the punctuation
/// in the head.
///
/// If `text` already fits, returns `(text, "")`.
pub fn split_at_boundary(text: &str, max_bytes: usize) -> (String, String) {
    if text.len() <= max_bytes {
        return (text.to_string(), String::new());
    }
    let cut_window = &text[..max_bytes];

    // 1. Last paragraph break (double newline).
    if let Some(idx) = cut_window.rfind("\n\n") {
        if idx == 0 {
            return split_at_char_boundary(text, max_bytes);
        }
        return (text[..idx].to_string(), text[idx + 2..].to_string());
    }
    // 2. Last single newline.
    if let Some(idx) = cut_window.rfind('\n') {
        if idx == 0 {
            return split_at_char_boundary(text, max_bytes);
        }
        return (text[..idx].to_string(), text[idx + 1..].to_string());
    }
    // 3. Last sentence boundary (". ", "! ", "? ") — keep the punctuation.
    for marker in [". ", "! ", "? "] {
        if let Some(idx) = cut_window.rfind(marker) {
            return (
                text[..idx + marker.len() - 1].to_string(),
                text[idx + marker.len()..].to_string(),
            );
        }
    }
    // 4. Last whitespace.
    if let Some(idx) = cut_window.rfind(' ') {
        if idx == 0 {
            return split_at_char_boundary(text, max_bytes);
        }
        return (text[..idx].to_string(), text[idx + 1..].to_string());
    }
    // 5. Last UTF-8 char boundary — guarantees no panic.
    split_at_char_boundary(text, max_bytes)
}

fn split_at_char_boundary(text: &str, max_bytes: usize) -> (String, String) {
    let mut cut = max_bytes;
    while cut > 0 && !text.is_char_boundary(cut) {
        cut -= 1;
    }
    if cut == 0 {
        cut = text.chars().next().map(|ch| ch.len_utf8()).unwrap_or(0);
    }
    (text[..cut].to_string(), text[cut..].to_string())
}

fn clip_bytes(text: &str, max_bytes: usize) -> String {
    if max_bytes == 0 || text.len() <= max_bytes {
        return text.to_string();
    }
    let mut cut = max_bytes.saturating_sub("…".len());
    while cut > 0 && !text.is_char_boundary(cut) {
        cut -= 1;
    }
    let mut out = text[..cut].to_string();
    out.push('…');
    out
}

/// State machine that drives a [`StreamingChannelAdapter`] from a stream
/// of [`ChannelStreamEvent`]s.
///
/// Lifecycle: build with [`StreamConsumer::new`], then call
/// [`StreamConsumer::handle_event`] for each event. The owner usually
/// drains a `tokio::sync::mpsc::Receiver` in a loop and forwards events
/// here. On end-of-stream, send `ChannelStreamEvent::Done` to flush.
pub struct StreamConsumer<A: StreamingChannelAdapter> {
    adapter: A,
    config: StreamConsumerConfig,
    /// Live text buffer for the *current* message (without cursor).
    accumulated: String,
    /// Id returned by `send_new_message` for the live message — we
    /// reach for it on every edit. `None` between segments (after a
    /// tool finalises) until the next `TextDelta`/`Commentary` opens
    /// a new live message.
    current_message_id: Option<String>,
    /// Timestamp of the last `edit_message` call we issued. Used to
    /// throttle to `edit_interval` cadence.
    last_edit_at: Option<Instant>,
    /// Timestamp of the last tool progress bubble edit. Browser batches
    /// can emit many semantic ticks quickly; keep Telegram readable and
    /// below flood-control while still storing every tick for the final
    /// tool result edit.
    last_tool_progress_edit_at: Option<Instant>,
    /// `true` once the live message has been opened; goes back to
    /// `false` after a tool segment closes it.
    has_live_message: bool,
    /// HS.9 — true once at least one tool bubble has been emitted in
    /// this stream. Used to decide whether the *next* live message
    /// to open is the "final reply" (post-tool) and should carry a
    /// reply-quote of the user's prompt.
    seen_tool: bool,
    /// HS.9 — true once `arm_final_reply_quote` has been called for
    /// this stream. Prevents quoting twice if multiple text segments
    /// follow tool bubbles.
    replied: bool,
    /// HS.10/HS.11 — pending tool bubbles keyed by tool_use_id when
    /// available, falling back to tool name for older event producers.
    /// `ToolProgress` edits the same bubble in place, which keeps
    /// Telegram readable during long browser runs.
    tool_pending: HashMap<String, VecDeque<PendingToolBubble>>,
}

#[derive(Debug, Clone)]
struct PendingToolBubble {
    message_id: String,
    start_body: String,
    body: String,
}

impl<A: StreamingChannelAdapter> StreamConsumer<A> {
    pub fn new(adapter: A, config: StreamConsumerConfig) -> Self {
        Self {
            adapter,
            config,
            accumulated: String::new(),
            current_message_id: None,
            last_edit_at: None,
            last_tool_progress_edit_at: None,
            has_live_message: false,
            seen_tool: false,
            replied: false,
            tool_pending: HashMap::new(),
        }
    }

    /// Process a single event. The decision tree follows progressive delivery:
    ///
    /// - `TextDelta` → append + maybe flush
    /// - `ToolStart` → finalise the live text (cursor off), emit tool bubble
    /// - `ToolProgress` → edit the matching tool bubble in place
    /// - `ToolEnd`   → emit tool result bubble; live message stays closed
    /// - `Commentary` → finalise + open a NEW live message with `text`
    /// - `Done`      → final flush, no cursor on the rendered text
    pub async fn handle_event(&mut self, event: ChannelStreamEvent) -> Result<(), String> {
        match event {
            ChannelStreamEvent::TextDelta(text) => self.on_text_delta(&text).await,
            ChannelStreamEvent::ToolStart {
                tool_use_id,
                emoji,
                name,
                input_preview,
            } => {
                self.finalise_live_text().await?;
                self.seen_tool = true;
                self.last_tool_progress_edit_at = None;
                let body = render_tool_start(&emoji, &name, &input_preview);
                let mid = self.adapter.send_new_message(&body).await?;
                // HS.10 — remember the (mid, body) so the matching
                // ToolProgress/ToolEnd can EDIT this same bubble.
                self.tool_pending
                    .entry(tool_pending_key(&tool_use_id, &name))
                    .or_default()
                    .push_back(PendingToolBubble {
                        message_id: mid,
                        start_body: body.clone(),
                        body,
                    });
                Ok(())
            }
            ChannelStreamEvent::ToolEnd {
                tool_use_id,
                emoji,
                name,
                result_preview,
                is_error,
            } => {
                self.seen_tool = true;
                // HS.10 — try to edit the matching ToolStart bubble in
                // place. Falls back to a fresh send_new_message for
                // tool ends that arrive without a prior start (an
                // edge case when the runtime emitted a result for a
                // tool the channel filter dropped).
                let pending = self.pop_pending_tool(&tool_use_id, &name);
                if let Some(pending) = pending {
                    let combined = format!(
                        "{}\n{}",
                        pending.body,
                        render_tool_result_suffix(&result_preview, is_error)
                    );
                    self.adapter
                        .edit_message(&pending.message_id, &combined)
                        .await?;
                    Ok(())
                } else {
                    self.adapter
                        .send_new_message(&render_tool_end(
                            &emoji,
                            &name,
                            &result_preview,
                            is_error,
                        ))
                        .await
                        .map(|_| ())
                }
            }
            ChannelStreamEvent::ToolProgress { tool_use_id, chunk } => {
                let max_message_bytes = self.config.max_message_bytes;
                if let Some((message_id, body)) =
                    self.update_pending_tool_progress(&tool_use_id, &chunk, max_message_bytes)
                {
                    if self.should_edit_tool_progress() {
                        self.adapter.edit_message(&message_id, &body).await?;
                        self.last_tool_progress_edit_at = Some(Instant::now());
                    }
                }
                Ok(())
            }
            ChannelStreamEvent::Commentary(text) => {
                self.finalise_live_text().await?;
                self.open_live_message(&text).await
            }
            ChannelStreamEvent::Done => self.finalise_live_text().await,
        }
    }

    fn pop_pending_tool(&mut self, tool_use_id: &str, name: &str) -> Option<PendingToolBubble> {
        let primary = tool_pending_key(tool_use_id, name);
        if let Some(pending) = self
            .tool_pending
            .get_mut(&primary)
            .and_then(|q| q.pop_front())
        {
            return Some(pending);
        }
        if tool_use_id.is_empty() {
            return None;
        }
        self.tool_pending
            .get_mut(&tool_name_pending_key(name))
            .and_then(|q| q.pop_front())
    }

    fn update_pending_tool_progress(
        &mut self,
        tool_use_id: &str,
        chunk: &str,
        max_message_bytes: usize,
    ) -> Option<(String, String)> {
        if tool_use_id.trim().is_empty() {
            return None;
        }
        let key = tool_id_pending_key(tool_use_id);
        let queue = self.tool_pending.get_mut(&key)?;
        let pending = queue.front_mut()?;
        pending.body =
            append_tool_progress_body(&pending.start_body, &pending.body, chunk, max_message_bytes);
        Some((pending.message_id.clone(), pending.body.clone()))
    }

    fn should_edit_tool_progress(&self) -> bool {
        let interval = if self.config.edit_interval.is_zero() {
            Duration::ZERO
        } else {
            self.config.edit_interval.min(Duration::from_millis(
                DEFAULT_TOOL_PROGRESS_EDIT_INTERVAL_MS,
            ))
        };
        self.last_tool_progress_edit_at
            .map(|last| last.elapsed() >= interval)
            .unwrap_or(true)
    }

    /// `true` when the live edit cadence allows another edit.
    ///
    /// Older logic used `buffer_threshold OR timer`, which meant every delta
    /// after ~40 bytes could trigger an edit. That is exactly the Telegram
    /// flood-control failure mode on long tool-heavy turns. The threshold now
    /// gates whether an edit is useful; the timer gates whether it is allowed.
    fn should_flush(&self) -> bool {
        if self.accumulated.is_empty() {
            return false;
        }
        match self.last_edit_at {
            None => true,
            Some(t) => {
                t.elapsed() >= self.config.edit_interval
                    && self.accumulated.len() >= self.config.buffer_threshold
            }
        }
    }

    async fn on_text_delta(&mut self, text: &str) -> Result<(), String> {
        self.accumulated.push_str(text);
        if !self.has_live_message {
            self.send_open_with_cursor().await?;
            return Ok(());
        }
        if !self.adapter.supports_edits() {
            return Ok(());
        }
        if self.should_flush() {
            self.edit_with_cursor().await?;
        }
        Ok(())
    }

    /// Open a brand-new live message holding the supplied seed text,
    /// then mark the message as live so subsequent deltas hit `edit`.
    async fn open_live_message(&mut self, seed: &str) -> Result<(), String> {
        self.accumulated.clear();
        self.accumulated.push_str(seed);
        self.send_open_with_cursor().await
    }

    async fn send_open_with_cursor(&mut self) -> Result<(), String> {
        // HS.11 — Codex can produce large first deltas/snapshots. If the
        // first live opener itself exceeds Telegram's cap, adapter-level
        // splitting would hide multiple message ids from this consumer and
        // the final edit would target only the last one with the full body.
        // Split closed chunks here before opening the live tail.
        if self.seen_tool && !self.replied {
            self.adapter.arm_final_reply_quote();
            self.replied = true;
        }
        self.drain_closed_chunks_before_live_open().await?;
        let body = if self.adapter.supports_edits() {
            self.with_cursor()
        } else {
            self.accumulated.clone()
        };
        // HS.9 — the FIRST live text bubble that opens AFTER a tool
        // is the agent's "final reply": ask the adapter to attach a
        // reply-quote of the user's prompt to it. If no tool ever
        // ran, we never quote — that's the trade-off (Telegram's
        // editMessageText doesn't accept reply_parameters, so we'd
        // need to know "this is final" *at send time*, which is
        // only knowable post-tool).
        let id = self.adapter.send_new_message(&body).await?;
        self.current_message_id = Some(id);
        self.has_live_message = true;
        self.last_edit_at = Some(Instant::now());
        Ok(())
    }

    async fn drain_closed_chunks_before_live_open(&mut self) -> Result<(), String> {
        if self.config.max_message_bytes == 0 {
            return Ok(());
        }
        let limit = self.live_text_limit();
        while self.accumulated.len() > limit {
            let (head, tail) = split_at_boundary(&self.accumulated, limit);
            if head.is_empty() {
                break;
            }
            self.adapter.send_new_message(&head).await?;
            self.accumulated = tail;
        }
        Ok(())
    }

    fn live_text_limit(&self) -> usize {
        if self.config.max_message_bytes == 0 {
            return usize::MAX;
        }
        if self.adapter.supports_edits() {
            self.config
                .max_message_bytes
                .saturating_sub(self.config.cursor.len())
                .max(1)
        } else {
            self.config.max_message_bytes
        }
    }

    async fn edit_with_cursor(&mut self) -> Result<(), String> {
        let Some(id) = self.current_message_id.clone() else {
            return Ok(());
        };
        // HS.4 — if the rendered body (text + cursor) would exceed the
        // platform cap, finalise the current message with the head chunk
        // (cursor stripped) and roll the tail into a fresh live message.
        if self.config.max_message_bytes > 0 {
            let safe_limit = self.live_text_limit();
            if self.accumulated.len() > safe_limit {
                let (head, tail) = split_at_boundary(&self.accumulated, safe_limit);
                self.adapter.edit_message(&id, &head).await?;
                self.accumulated = tail;
                self.current_message_id = None;
                self.has_live_message = false;
                self.last_edit_at = None;
                return self.send_open_with_cursor().await;
            }
        }
        let body = self.with_cursor();
        self.adapter.edit_message(&id, &body).await?;
        self.last_edit_at = Some(Instant::now());
        Ok(())
    }

    /// Final write of the live message: cursor stripped, edit issued
    /// only if the adapter supports edits AND something changed since
    /// the last edit. Then the live state is reset so the next opener
    /// (TextDelta after ToolEnd, or Commentary) starts fresh.
    async fn finalise_live_text(&mut self) -> Result<(), String> {
        if !self.has_live_message {
            self.accumulated.clear();
            return Ok(());
        }
        if self.adapter.supports_edits() {
            if let Some(id) = self.current_message_id.clone() {
                let body = self.accumulated.clone();
                if self.config.max_message_bytes > 0 && body.len() > self.config.max_message_bytes {
                    let (head, tail) = split_at_boundary(&body, self.config.max_message_bytes);
                    self.adapter.edit_message(&id, &head).await?;
                    self.send_closed_chunks(&tail).await?;
                } else {
                    self.adapter.edit_message(&id, &body).await?;
                }
            }
        } else {
            // Write-once channel — defer to send_new_message on Done.
            let body = self.accumulated.clone();
            self.send_closed_chunks(&body).await?;
        }
        self.accumulated.clear();
        self.current_message_id = None;
        self.has_live_message = false;
        self.last_edit_at = None;
        Ok(())
    }

    fn with_cursor(&self) -> String {
        let mut out = String::with_capacity(self.accumulated.len() + self.config.cursor.len());
        out.push_str(&self.accumulated);
        out.push_str(self.config.cursor);
        out
    }

    async fn send_closed_chunks(&self, text: &str) -> Result<(), String> {
        if text.is_empty() {
            return Ok(());
        }
        if self.config.max_message_bytes == 0 || text.len() <= self.config.max_message_bytes {
            self.adapter.send_new_message(text).await?;
            return Ok(());
        }

        let mut remaining = text.to_string();
        while !remaining.is_empty() {
            if remaining.len() <= self.config.max_message_bytes {
                self.adapter.send_new_message(&remaining).await?;
                break;
            }
            let (head, tail) = split_at_boundary(&remaining, self.config.max_message_bytes);
            if head.is_empty() {
                self.adapter.send_new_message(&remaining).await?;
                break;
            }
            self.adapter.send_new_message(&head).await?;
            remaining = tail;
        }
        Ok(())
    }
}

/// Render a "tool started" bubble — HS.6 with extra polish: a family emoji
/// + the tool name on its own line (HTML
///   `<b>`-bolded so the eye lands on it first), then the input preview
///   as a `<code>` block on the next line. Falls back to `→` if no
///   emoji is provided.
///
/// The HTML tags are honoured by Telegram (`parse_mode=HTML`,
/// `sanitize_telegram_html` already allows them) and stripped harmlessly
/// by the unit tests' MockAdapter — they're just bytes in the body.
pub fn render_tool_start(emoji: &str, name: &str, input_preview: &str) -> String {
    let prefix = if emoji.is_empty() { "→" } else { emoji };
    if input_preview.is_empty() {
        format!("{prefix} <b>{name}</b>")
    } else {
        format!("{prefix} <b>{name}</b>\n<code>{input_preview}</code>")
    }
}

fn tool_pending_key(tool_use_id: &str, name: &str) -> String {
    if tool_use_id.trim().is_empty() {
        tool_name_pending_key(name)
    } else {
        tool_id_pending_key(tool_use_id)
    }
}

fn tool_id_pending_key(tool_use_id: &str) -> String {
    format!("id:{}", tool_use_id.trim())
}

fn tool_name_pending_key(name: &str) -> String {
    format!("name:{name}")
}

const MAX_TOOL_PROGRESS_LINES: usize = 12;
const MAX_TOOL_PROGRESS_LINE_BYTES: usize = 240;

fn append_tool_progress_body(
    start_body: &str,
    current_body: &str,
    chunk: &str,
    max_message_bytes: usize,
) -> String {
    let mut progress_lines: VecDeque<String> = current_body
        .strip_prefix(start_body)
        .unwrap_or_default()
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(ToOwned::to_owned)
        .collect();

    for line in render_tool_progress_lines(chunk) {
        progress_lines.push_back(line);
    }
    while progress_lines.len() > MAX_TOOL_PROGRESS_LINES {
        progress_lines.pop_front();
    }

    loop {
        let body = if progress_lines.is_empty() {
            start_body.to_string()
        } else {
            format!(
                "{start_body}\n{}",
                progress_lines
                    .iter()
                    .cloned()
                    .collect::<Vec<_>>()
                    .join("\n")
            )
        };
        if max_message_bytes == 0 || body.len() <= max_message_bytes || progress_lines.is_empty() {
            return body;
        }
        progress_lines.pop_front();
    }
}

pub fn render_tool_progress_lines(chunk: &str) -> Vec<String> {
    chunk
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .take(MAX_TOOL_PROGRESS_LINES)
        .map(|line| {
            let clipped = clip_bytes(line, MAX_TOOL_PROGRESS_LINE_BYTES);
            format!("↳ {clipped}")
        })
        .collect()
}

/// HS.10 — Render JUST the result line that gets appended to a
/// ToolStart bubble when the tool finishes. Used by the in-place
/// edit path so a single Telegram message holds both the command
/// and its result, in chronological order.
pub fn render_tool_result_suffix(result_preview: &str, is_error: bool) -> String {
    let prefix = if is_error { "✗" } else { "✓" };
    if result_preview.is_empty() {
        prefix.to_string()
    } else {
        format!("{prefix} <code>{result_preview}</code>")
    }
}

/// Render a "tool ended" bubble as a STANDALONE message. Used as a
/// fallback when no matching ToolStart bubble was tracked (HS.10
/// graceful degradation). Compact on success, expanded on error.
pub fn render_tool_end(_emoji: &str, name: &str, result_preview: &str, is_error: bool) -> String {
    if is_error {
        if result_preview.is_empty() {
            format!("✗ <b>{name}</b>")
        } else {
            format!("✗ <b>{name}</b>\n<code>{result_preview}</code>")
        }
    } else if result_preview.is_empty() {
        "✓".to_string()
    } else {
        format!("✓ <code>{result_preview}</code>")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};

    #[derive(Debug, Clone, PartialEq, Eq)]
    enum MockCall {
        SendNew(String),
        Edit(String, String),
    }

    /// Adapter that records every call. Each `send_new_message` returns
    /// `mid_<n>` so tests can assert on the (id, text) edit pairs.
    #[derive(Clone, Default)]
    struct MockAdapter {
        calls: Arc<Mutex<Vec<MockCall>>>,
        next_id: Arc<Mutex<usize>>,
        supports_edits: bool,
    }

    impl MockAdapter {
        fn new() -> Self {
            Self {
                calls: Arc::new(Mutex::new(Vec::new())),
                next_id: Arc::new(Mutex::new(0)),
                supports_edits: true,
            }
        }

        fn write_once() -> Self {
            Self {
                calls: Arc::new(Mutex::new(Vec::new())),
                next_id: Arc::new(Mutex::new(0)),
                supports_edits: false,
            }
        }

        fn calls(&self) -> Vec<MockCall> {
            self.calls.lock().unwrap().clone()
        }
    }

    #[async_trait]
    impl StreamingChannelAdapter for MockAdapter {
        async fn send_new_message(&self, text: &str) -> Result<String, String> {
            let mut id = self.next_id.lock().unwrap();
            let mid = format!("mid_{id}");
            *id += 1;
            self.calls
                .lock()
                .unwrap()
                .push(MockCall::SendNew(text.to_string()));
            Ok(mid)
        }

        async fn edit_message(&self, message_id: &str, text: &str) -> Result<(), String> {
            self.calls
                .lock()
                .unwrap()
                .push(MockCall::Edit(message_id.to_string(), text.to_string()));
            Ok(())
        }

        fn supports_edits(&self) -> bool {
            self.supports_edits
        }
    }

    fn cfg_no_throttle() -> StreamConsumerConfig {
        // Threshold 1 byte + zero edit_interval → every delta forces a flush.
        // Test helper to make ordering assertions deterministic without sleeps.
        // max_message_bytes=0 disables splitting (HS.4) for tests focused
        // on cursor / segment boundaries.
        StreamConsumerConfig {
            edit_interval: Duration::from_secs(0),
            buffer_threshold: 1,
            cursor: STREAM_CURSOR,
            max_message_bytes: 0,
        }
    }

    fn cfg_buffer_only(threshold: usize) -> StreamConsumerConfig {
        StreamConsumerConfig {
            edit_interval: Duration::from_secs(3600),
            buffer_threshold: threshold,
            cursor: STREAM_CURSOR,
            max_message_bytes: 0,
        }
    }

    #[tokio::test]
    async fn first_text_delta_opens_a_new_message_with_cursor() {
        let mock = MockAdapter::new();
        let mut consumer = StreamConsumer::new(mock.clone(), cfg_no_throttle());

        consumer
            .handle_event(ChannelStreamEvent::TextDelta("Hello".into()))
            .await
            .unwrap();

        let calls = mock.calls();
        assert_eq!(calls.len(), 1, "expected exactly one send_new_message");
        assert_eq!(
            calls[0],
            MockCall::SendNew(format!("Hello{STREAM_CURSOR}")),
            "the live opener must carry the cursor"
        );
    }

    #[tokio::test]
    async fn subsequent_deltas_edit_the_same_message() {
        let mock = MockAdapter::new();
        let mut consumer = StreamConsumer::new(mock.clone(), cfg_no_throttle());

        consumer
            .handle_event(ChannelStreamEvent::TextDelta("Hi".into()))
            .await
            .unwrap();
        consumer
            .handle_event(ChannelStreamEvent::TextDelta(", world!".into()))
            .await
            .unwrap();

        let calls = mock.calls();
        assert_eq!(calls.len(), 2);
        assert_eq!(calls[0], MockCall::SendNew(format!("Hi{STREAM_CURSOR}")));
        assert_eq!(
            calls[1],
            MockCall::Edit("mid_0".into(), format!("Hi, world!{STREAM_CURSOR}"))
        );
    }

    #[tokio::test]
    async fn done_finalises_with_cursor_stripped() {
        let mock = MockAdapter::new();
        let mut consumer = StreamConsumer::new(mock.clone(), cfg_no_throttle());

        consumer
            .handle_event(ChannelStreamEvent::TextDelta("ok".into()))
            .await
            .unwrap();
        consumer
            .handle_event(ChannelStreamEvent::Done)
            .await
            .unwrap();

        let calls = mock.calls();
        let final_call = calls.last().expect("at least one call");
        match final_call {
            MockCall::Edit(_id, body) => assert_eq!(
                body, "ok",
                "the final edit must drop the cursor — got {body:?}"
            ),
            other => panic!("expected final Edit, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn tool_start_finalises_text_then_posts_tool_bubble() {
        let mock = MockAdapter::new();
        let mut consumer = StreamConsumer::new(mock.clone(), cfg_no_throttle());

        consumer
            .handle_event(ChannelStreamEvent::TextDelta("checking…".into()))
            .await
            .unwrap();
        consumer
            .handle_event(ChannelStreamEvent::ToolStart {
                tool_use_id: "t1".into(),
                emoji: String::new(),
                name: "web_search".into(),
                input_preview: "rust async".into(),
            })
            .await
            .unwrap();

        let calls = mock.calls();
        // 1: send_new("checking… ▉"), 2: edit("checking…"), 3: send_new(tool bubble)
        assert_eq!(calls.len(), 3, "got {calls:?}");
        match &calls[2] {
            MockCall::SendNew(body) => {
                assert!(
                    body.starts_with("→ <b>web_search</b>"),
                    "tool bubble: {body}"
                );
                assert!(body.contains("rust async"));
            }
            other => panic!("expected tool bubble as a NEW message, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn text_after_tool_opens_a_brand_new_live_message() {
        // The "semi-response between tools" UX: after a tool finalises,
        // the next TextDelta must open a NEW message, not edit the one
        // before the tool. Otherwise the post-tool reply would silently
        // overwrite the pre-tool reply.
        let mock = MockAdapter::new();
        let mut consumer = StreamConsumer::new(mock.clone(), cfg_no_throttle());

        consumer
            .handle_event(ChannelStreamEvent::TextDelta("first".into()))
            .await
            .unwrap();
        consumer
            .handle_event(ChannelStreamEvent::ToolStart {
                tool_use_id: "t1".into(),
                emoji: String::new(),
                name: "web_search".into(),
                input_preview: "x".into(),
            })
            .await
            .unwrap();
        consumer
            .handle_event(ChannelStreamEvent::ToolEnd {
                tool_use_id: "t1".into(),
                emoji: String::new(),
                name: "web_search".into(),
                result_preview: "5 hits".into(),
                is_error: false,
            })
            .await
            .unwrap();
        consumer
            .handle_event(ChannelStreamEvent::TextDelta("second".into()))
            .await
            .unwrap();

        let calls = mock.calls();
        // Expected: send("first ▉"), edit("first"), send(tool start), send(tool end), send("second ▉")
        assert_eq!(calls.len(), 5, "got {calls:?}");
        assert!(
            matches!(&calls[4], MockCall::SendNew(b) if b == &format!("second{STREAM_CURSOR}"))
        );
    }

    #[tokio::test]
    async fn commentary_finalises_and_opens_new_message() {
        let mock = MockAdapter::new();
        let mut consumer = StreamConsumer::new(mock.clone(), cfg_no_throttle());

        consumer
            .handle_event(ChannelStreamEvent::TextDelta("intro".into()))
            .await
            .unwrap();
        consumer
            .handle_event(ChannelStreamEvent::Commentary("aside thought".into()))
            .await
            .unwrap();

        let calls = mock.calls();
        // send("intro ▉"), edit("intro"), send("aside thought ▉")
        assert_eq!(calls.len(), 3);
        assert!(matches!(&calls[1], MockCall::Edit(id, body) if id == "mid_0" && body == "intro"));
        assert!(
            matches!(&calls[2], MockCall::SendNew(b) if b == &format!("aside thought{STREAM_CURSOR}"))
        );
    }

    #[tokio::test]
    async fn buffer_threshold_does_not_bypass_edit_cadence() {
        // With a high time interval and threshold=10, crossing the threshold
        // must still hold back edits. The first delta opens (forced); later
        // deltas wait for the cadence window to avoid Telegram flood-control.
        let mock = MockAdapter::new();
        let mut consumer = StreamConsumer::new(mock.clone(), cfg_buffer_only(10));

        // First delta of 3 bytes: opens the message (forced first edit).
        consumer
            .handle_event(ChannelStreamEvent::TextDelta("abc".into()))
            .await
            .unwrap();
        // Below threshold AND below time → no edit.
        consumer
            .handle_event(ChannelStreamEvent::TextDelta("de".into()))
            .await
            .unwrap();
        consumer
            .handle_event(ChannelStreamEvent::TextDelta("fg".into()))
            .await
            .unwrap();
        // 8 bytes accumulated, still under 10 → still no edit.
        let mid_calls = mock.calls();
        assert_eq!(
            mid_calls.len(),
            1,
            "no edit until threshold is crossed; got {mid_calls:?}"
        );
        // Cross the 10-byte threshold but keep the cadence window closed.
        consumer
            .handle_event(ChannelStreamEvent::TextDelta("hijklm".into()))
            .await
            .unwrap();
        let calls = mock.calls();
        assert_eq!(
            calls.len(),
            1,
            "threshold alone must not force an edit; got {calls:?}"
        );
    }

    #[tokio::test]
    async fn write_once_adapter_only_emits_at_finalise() {
        // A channel that doesn't support edits (Email, SMS) must NOT
        // receive intermediate edits. The whole reply lands as a single
        // send_new_message at finalise time.
        let mock = MockAdapter::write_once();
        let mut consumer = StreamConsumer::new(mock.clone(), cfg_no_throttle());

        consumer
            .handle_event(ChannelStreamEvent::TextDelta("part1".into()))
            .await
            .unwrap();
        consumer
            .handle_event(ChannelStreamEvent::TextDelta("part2".into()))
            .await
            .unwrap();
        consumer
            .handle_event(ChannelStreamEvent::Done)
            .await
            .unwrap();

        let calls = mock.calls();
        // 1 open send (no edits in between) + 1 final send_new on Done.
        // Note: write-once channels can't edit so we send a fresh msg
        // at finalise — the single open send carried *raw* text without cursor.
        assert!(calls.iter().all(|c| matches!(c, MockCall::SendNew(_))));
        let bodies: Vec<&str> = calls
            .iter()
            .filter_map(|c| match c {
                MockCall::SendNew(b) => Some(b.as_str()),
                _ => None,
            })
            .collect();
        assert_eq!(bodies.len(), 2, "got bodies {bodies:?}");
        assert!(
            !bodies[0].contains(STREAM_CURSOR),
            "no cursor on write-once"
        );
        assert_eq!(bodies[1], "part1part2", "final flush concatenates");
    }

    #[tokio::test]
    async fn tool_end_without_matching_start_falls_back_to_send_new() {
        // HS.10 graceful degradation: a ToolEnd that arrives without
        // a paired ToolStart (channel filter dropped the start, or
        // the runtime emitted only a result event) still produces a
        // visible bubble via send_new_message.
        let mock = MockAdapter::new();
        let mut consumer = StreamConsumer::new(mock.clone(), cfg_no_throttle());

        consumer
            .handle_event(ChannelStreamEvent::ToolEnd {
                tool_use_id: "missing".into(),
                emoji: "🌐".into(),
                name: "web_fetch".into(),
                result_preview: "404".into(),
                is_error: true,
            })
            .await
            .unwrap();

        let calls = mock.calls();
        assert_eq!(calls.len(), 1, "no matching start → fallback to send_new");
        match &calls[0] {
            MockCall::SendNew(body) => {
                assert!(body.starts_with("✗"), "error arrow expected, got {body}");
            }
            other => panic!("expected tool bubble, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn tool_start_then_end_edits_the_same_bubble() {
        // HS.10 happy path: ToolStart posts the bubble, ToolEnd EDITS
        // the same message_id with the original body + a result line.
        // No second send_new, no separate "result" message.
        let mock = MockAdapter::new();
        let mut consumer = StreamConsumer::new(mock.clone(), cfg_no_throttle());

        consumer
            .handle_event(ChannelStreamEvent::ToolStart {
                tool_use_id: "t1".into(),
                emoji: "💻".into(),
                name: "shell_exec".into(),
                input_preview: "uname -a".into(),
            })
            .await
            .unwrap();
        consumer
            .handle_event(ChannelStreamEvent::ToolEnd {
                tool_use_id: "t1".into(),
                emoji: "💻".into(),
                name: "shell_exec".into(),
                result_preview: "Darwin 25.3.0".into(),
                is_error: false,
            })
            .await
            .unwrap();

        let calls = mock.calls();
        assert_eq!(calls.len(), 2, "expect one send + one edit, got {calls:?}");
        match &calls[0] {
            MockCall::SendNew(body) => {
                assert!(body.contains("shell_exec"));
                assert!(body.contains("uname -a"));
            }
            other => panic!("expected ToolStart send, got {other:?}"),
        }
        match &calls[1] {
            MockCall::Edit(_id, body) => {
                // Same body as the start (preserved verbatim) +
                // a result suffix on a new line.
                assert!(body.contains("uname -a"), "edit must preserve start body");
                assert!(body.contains("Darwin"), "edit must append result");
                assert!(body.starts_with("💻"), "emoji must remain first");
                let result_line = body.lines().last().unwrap();
                assert!(
                    result_line.starts_with('✓'),
                    "result line must start with ✓: {result_line}"
                );
            }
            other => panic!("expected ToolEnd edit, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn tool_progress_edits_the_running_tool_bubble() {
        let mock = MockAdapter::new();
        let mut consumer = StreamConsumer::new(mock.clone(), cfg_no_throttle());

        consumer
            .handle_event(ChannelStreamEvent::ToolStart {
                tool_use_id: "browser-1".into(),
                emoji: "🌐".into(),
                name: "browser_batch".into(),
                input_preview: "open example.com".into(),
            })
            .await
            .unwrap();
        consumer
            .handle_event(ChannelStreamEvent::ToolProgress {
                tool_use_id: "browser-1".into(),
                chunk: "> 1/2 open https://example.com".into(),
            })
            .await
            .unwrap();
        consumer
            .handle_event(ChannelStreamEvent::ToolProgress {
                tool_use_id: "browser-1".into(),
                chunk: "✓ 1/2 open · title \"Example Domain\"".into(),
            })
            .await
            .unwrap();
        consumer
            .handle_event(ChannelStreamEvent::ToolEnd {
                tool_use_id: "browser-1".into(),
                emoji: "🌐".into(),
                name: "browser_batch".into(),
                result_preview: "success".into(),
                is_error: false,
            })
            .await
            .unwrap();

        let calls = mock.calls();
        assert_eq!(calls.len(), 4, "start + 2 progress edits + result edit");
        assert!(matches!(&calls[0], MockCall::SendNew(body) if body.contains("browser_batch")));
        assert!(
            matches!(&calls[1], MockCall::Edit(id, body) if id == "mid_0" && body.contains("↳ > 1/2 open"))
        );
        assert!(
            matches!(&calls[2], MockCall::Edit(id, body) if id == "mid_0" && body.contains("Example Domain"))
        );
        assert!(
            matches!(&calls[3], MockCall::Edit(id, body) if id == "mid_0" && body.contains("success") && body.contains("Example Domain"))
        );
    }

    #[tokio::test]
    async fn tool_progress_uses_tool_use_id_not_tool_name_fifo() {
        let mock = MockAdapter::new();
        let mut consumer = StreamConsumer::new(mock.clone(), cfg_no_throttle());

        for id in ["browser-a", "browser-b"] {
            consumer
                .handle_event(ChannelStreamEvent::ToolStart {
                    tool_use_id: id.into(),
                    emoji: "🌐".into(),
                    name: "browser_batch".into(),
                    input_preview: id.into(),
                })
                .await
                .unwrap();
        }
        consumer
            .handle_event(ChannelStreamEvent::ToolProgress {
                tool_use_id: "browser-b".into(),
                chunk: "✓ second bubble only".into(),
            })
            .await
            .unwrap();

        let calls = mock.calls();
        assert_eq!(calls.len(), 3, "two starts + one progress edit");
        assert!(
            matches!(&calls[2], MockCall::Edit(id, body) if id == "mid_1" && body.contains("second bubble only"))
        );
    }

    #[tokio::test]
    async fn tool_pairs_are_fifo_per_name() {
        // HS.10 — two consecutive shell_exec calls must edit the
        // bubbles in the order they were opened, not interleaved.
        let mock = MockAdapter::new();
        let mut consumer = StreamConsumer::new(mock.clone(), cfg_no_throttle());

        for cmd in ["alpha", "bravo"] {
            consumer
                .handle_event(ChannelStreamEvent::ToolStart {
                    tool_use_id: String::new(),
                    emoji: "💻".into(),
                    name: "shell_exec".into(),
                    input_preview: cmd.into(),
                })
                .await
                .unwrap();
        }
        for res in ["one", "two"] {
            consumer
                .handle_event(ChannelStreamEvent::ToolEnd {
                    tool_use_id: String::new(),
                    emoji: "💻".into(),
                    name: "shell_exec".into(),
                    result_preview: res.into(),
                    is_error: false,
                })
                .await
                .unwrap();
        }

        let calls = mock.calls();
        // 2 sends (alpha, bravo) + 2 edits (alpha→one, bravo→two).
        assert_eq!(calls.len(), 4, "got {calls:?}");
        let edits: Vec<&MockCall> = calls
            .iter()
            .filter(|c| matches!(c, MockCall::Edit(_, _)))
            .collect();
        assert_eq!(edits.len(), 2);
        let MockCall::Edit(id_a, body_a) = edits[0] else {
            panic!()
        };
        let MockCall::Edit(id_b, body_b) = edits[1] else {
            panic!()
        };
        assert_eq!(id_a, "mid_0", "first edit must target the first bubble");
        assert_eq!(id_b, "mid_1", "second edit must target the second bubble");
        assert!(body_a.contains("alpha") && body_a.contains("one"));
        assert!(body_b.contains("bravo") && body_b.contains("two"));
    }

    #[test]
    fn render_tool_result_suffix_compact_on_success() {
        assert_eq!(render_tool_result_suffix("ok", false), "✓ <code>ok</code>");
        assert_eq!(render_tool_result_suffix("", false), "✓");
        assert_eq!(
            render_tool_result_suffix("boom", true),
            "✗ <code>boom</code>"
        );
        assert_eq!(render_tool_result_suffix("", true), "✗");
    }

    #[test]
    fn render_tool_start_uses_emoji_and_html_bold() {
        // No emoji → fallback arrow.
        assert_eq!(render_tool_start("", "ls", ""), "→ <b>ls</b>");
        // With emoji + input → emoji + bold name + code block on next line.
        assert_eq!(
            render_tool_start("💻", "shell_exec", "curl -s example.com"),
            "💻 <b>shell_exec</b>\n<code>curl -s example.com</code>"
        );
    }

    #[test]
    fn render_tool_end_compact_on_success_explicit_on_error() {
        // Success + empty preview → just the checkmark.
        assert_eq!(render_tool_end("💻", "shell_exec", "", false), "✓");
        // Success + preview → "✓ <code>preview</code>", no name (the
        // start bubble carries the name; repeating it would be noise).
        assert_eq!(
            render_tool_end("💻", "shell_exec", "ok", false),
            "✓ <code>ok</code>"
        );
        // Error → ✗ + bold name + code block (always visible).
        assert_eq!(
            render_tool_end("💻", "shell_exec", "boom", true),
            "✗ <b>shell_exec</b>\n<code>boom</code>"
        );
        // Error + empty preview → ✗ + name, no empty code block.
        assert_eq!(
            render_tool_end("💻", "shell_exec", "", true),
            "✗ <b>shell_exec</b>"
        );
    }

    // -----------------------------------------------------------------
    // HS.4 — split_at_boundary + overflow handling in the consumer
    // -----------------------------------------------------------------

    #[test]
    fn split_short_text_returns_intact() {
        let (head, tail) = split_at_boundary("hello", 100);
        assert_eq!(head, "hello");
        assert!(tail.is_empty());
    }

    #[test]
    fn split_prefers_paragraph_break() {
        let s = "first paragraph\n\nsecond paragraph that overflows";
        let (head, tail) = split_at_boundary(s, 25);
        assert_eq!(head, "first paragraph");
        assert!(tail.starts_with("second"));
    }

    #[test]
    fn split_falls_back_to_single_newline() {
        let s = "line one\nline two that goes long";
        let (head, tail) = split_at_boundary(s, 20);
        assert_eq!(head, "line one");
        assert_eq!(tail, "line two that goes long");
    }

    #[test]
    fn split_falls_back_to_sentence_boundary() {
        let s = "First sentence. Second sentence overflows here.";
        let (head, tail) = split_at_boundary(s, 25);
        assert_eq!(head, "First sentence.");
        assert!(tail.starts_with("Second"));
    }

    #[test]
    fn split_falls_back_to_word_boundary() {
        let s = "alpha bravo charlie delta echo";
        let (head, tail) = split_at_boundary(s, 15);
        assert!(head.ends_with('o') || head.ends_with('a'));
        assert!(!tail.starts_with(' '), "leading space should be consumed");
        assert!(head.len() <= 15);
    }

    #[test]
    fn split_respects_utf8_when_no_separator() {
        // Long unbroken UTF-8 with no separators — must not panic and
        // must roll back to a char boundary.
        let s = "café".repeat(20);
        let (head, _tail) = split_at_boundary(&s, 10);
        assert!(s.is_char_boundary(head.len()));
        assert!(head.len() <= 10);
    }

    #[tokio::test]
    async fn consumer_splits_when_buffer_exceeds_cap() {
        // Build a tiny adapter cap so we can prove the split path
        // without writing 4 KB of test text.
        let mock = MockAdapter::new();
        let mut consumer = StreamConsumer::new(
            mock.clone(),
            StreamConsumerConfig {
                edit_interval: Duration::from_secs(0),
                buffer_threshold: 1,
                cursor: STREAM_CURSOR,
                max_message_bytes: 30, // tiny cap for the test
            },
        );

        // First delta opens the live message (small).
        consumer
            .handle_event(ChannelStreamEvent::TextDelta("alpha bravo ".into()))
            .await
            .unwrap();
        // Second delta pushes total past the 30-byte cap → split.
        consumer
            .handle_event(ChannelStreamEvent::TextDelta(
                "charlie delta echo foxtrot".into(),
            ))
            .await
            .unwrap();
        consumer
            .handle_event(ChannelStreamEvent::Done)
            .await
            .unwrap();

        let calls = mock.calls();
        // Must have at least: opener send, finalise edit (head), open new
        // (tail with cursor), final edit (cursor stripped).
        assert!(
            calls.len() >= 3,
            "split flow must produce ≥3 calls, got {}: {calls:?}",
            calls.len()
        );
        // No single rendered body may exceed the cap.
        for c in &calls {
            let body = match c {
                MockCall::SendNew(b) | MockCall::Edit(_, b) => b,
            };
            assert!(
                body.len() <= 30,
                "split must keep every body under cap; got {}: {body:?}",
                body.len()
            );
        }
        // The very last call must be a cursor-free finalise.
        match calls.last().unwrap() {
            MockCall::Edit(_, body) => assert!(!body.contains('▉')),
            other => panic!("expected final edit, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn consumer_splits_huge_first_delta_before_opening_live_tail() {
        // Codex sometimes emits a large first text snapshot. The
        // consumer must split before the first live send, otherwise
        // Telegram's adapter-level split returns only one id while the
        // final edit still contains the whole body.
        let mock = MockAdapter::new();
        let mut consumer = StreamConsumer::new(
            mock.clone(),
            StreamConsumerConfig {
                edit_interval: Duration::from_secs(0),
                buffer_threshold: 1,
                cursor: STREAM_CURSOR,
                max_message_bytes: 30,
            },
        );

        consumer
            .handle_event(ChannelStreamEvent::TextDelta(
                "alpha bravo charlie delta echo foxtrot golf".into(),
            ))
            .await
            .unwrap();
        consumer
            .handle_event(ChannelStreamEvent::Done)
            .await
            .unwrap();

        let calls = mock.calls();
        assert!(calls.len() >= 3, "expected pre-split opener: {calls:?}");
        for c in &calls {
            let body = match c {
                MockCall::SendNew(b) | MockCall::Edit(_, b) => b,
            };
            assert!(
                body.len() <= 30,
                "every Telegram body must fit cap; got {}: {body:?}",
                body.len()
            );
        }
        assert!(matches!(&calls[0], MockCall::SendNew(b) if !b.contains('▉')));
        assert!(matches!(calls.last().unwrap(), MockCall::Edit(_, b) if !b.contains('▉')));
    }

    #[tokio::test]
    async fn consumer_with_cap_zero_never_splits() {
        // `max_message_bytes = 0` is the documented "no cap" mode; the
        // split path must not engage even if the buffer balloons.
        let mock = MockAdapter::new();
        let mut consumer = StreamConsumer::new(
            mock.clone(),
            StreamConsumerConfig {
                edit_interval: Duration::from_secs(0),
                buffer_threshold: 1,
                cursor: STREAM_CURSOR,
                max_message_bytes: 0,
            },
        );

        consumer
            .handle_event(ChannelStreamEvent::TextDelta("a".repeat(500)))
            .await
            .unwrap();
        consumer
            .handle_event(ChannelStreamEvent::Done)
            .await
            .unwrap();

        let calls = mock.calls();
        // Exactly one open + one final edit — no split.
        assert_eq!(calls.len(), 2, "no-cap mode must not split: {calls:?}");
    }
}
