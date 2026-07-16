use std::collections::{HashMap, VecDeque};
use std::time::{Duration, Instant};

const MAX_BOARD_ENTRIES: usize = 8;
const MAX_PROGRESS_LINES: usize = 6;
const MAX_STORED_PROGRESS_BYTES: usize = 480;
const MAX_ESCAPED_NAME_BYTES: usize = 160;
const MAX_ESCAPED_EMOJI_BYTES: usize = 48;
const MAX_ESCAPED_INPUT_BYTES: usize = 400;
const MAX_ESCAPED_PROGRESS_BYTES: usize = 280;
const MAX_ESCAPED_RESULT_BYTES: usize = 560;

#[derive(Debug, Clone)]
pub(crate) struct ToolActivityRender {
    pub(crate) board_index: usize,
    pub(crate) message_id: Option<String>,
    pub(crate) body: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct ToolLocation {
    board_index: usize,
    entry_index: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ToolStatus {
    Running,
    Succeeded,
    Failed,
}

#[derive(Debug, Clone)]
struct ToolActivityEntry {
    emoji: String,
    name: String,
    input_preview: String,
    progress: VecDeque<String>,
    result_preview: String,
    status: ToolStatus,
    started_at: Instant,
    elapsed: Option<Duration>,
}

impl ToolActivityEntry {
    fn running(emoji: &str, name: &str, input_preview: &str) -> Self {
        Self {
            emoji: emoji.to_string(),
            name: name.to_string(),
            input_preview: input_preview.to_string(),
            progress: VecDeque::new(),
            result_preview: String::new(),
            status: ToolStatus::Running,
            started_at: Instant::now(),
            elapsed: None,
        }
    }

    fn is_running(&self) -> bool {
        self.status == ToolStatus::Running
    }
}

#[derive(Debug, Clone)]
struct ToolActivityBoard {
    message_id: Option<String>,
    entries: Vec<ToolActivityEntry>,
}

#[derive(Debug, Default)]
pub(crate) struct ToolActivityTracker {
    boards: Vec<ToolActivityBoard>,
    active_board: Option<usize>,
    pending: HashMap<String, VecDeque<ToolLocation>>,
}

impl ToolActivityTracker {
    pub(crate) fn start_tool(
        &mut self,
        tool_use_id: &str,
        emoji: &str,
        name: &str,
        input_preview: &str,
    ) -> ToolActivityRender {
        let board_index = self
            .active_board
            .filter(|index| self.boards[*index].entries.len() < MAX_BOARD_ENTRIES)
            .unwrap_or_else(|| {
                self.boards.push(ToolActivityBoard {
                    message_id: None,
                    entries: Vec::new(),
                });
                let index = self.boards.len() - 1;
                self.active_board = Some(index);
                index
            });

        let entry_index = self.boards[board_index].entries.len();
        self.boards[board_index]
            .entries
            .push(ToolActivityEntry::running(emoji, name, input_preview));
        let location = ToolLocation {
            board_index,
            entry_index,
        };
        self.register_pending(tool_use_id, name, location);
        self.render(board_index)
    }

    pub(crate) fn bind_message_id(&mut self, board_index: usize, message_id: String) {
        if let Some(board) = self.boards.get_mut(board_index) {
            board.message_id = Some(message_id);
        }
    }

    pub(crate) fn finish_tool(
        &mut self,
        tool_use_id: &str,
        name: &str,
        result_preview: &str,
        is_error: bool,
    ) -> Option<ToolActivityRender> {
        self.close_wave();
        let location = self.take_pending_location(tool_use_id, name)?;
        let entry = self.entry_mut(location)?;
        entry.status = if is_error {
            ToolStatus::Failed
        } else {
            ToolStatus::Succeeded
        };
        entry.result_preview = result_preview.to_string();
        entry.elapsed = Some(entry.started_at.elapsed());
        Some(self.render(location.board_index))
    }

    pub(crate) fn progress_tool(
        &mut self,
        tool_use_id: &str,
        chunk: &str,
    ) -> Option<ToolActivityRender> {
        self.close_wave();
        if tool_use_id.trim().is_empty() {
            return None;
        }
        let location = self.peek_pending_location(&tool_id_key(tool_use_id))?;
        let entry = self.entry_mut(location)?;
        for line in progress_lines(chunk) {
            entry.progress.push_back(line);
        }
        while entry.progress.len() > MAX_PROGRESS_LINES {
            entry.progress.pop_front();
        }
        Some(self.render(location.board_index))
    }

    pub(crate) fn close_wave(&mut self) {
        self.active_board = None;
    }

    fn register_pending(&mut self, tool_use_id: &str, name: &str, location: ToolLocation) {
        self.pending
            .entry(tool_name_key(name))
            .or_default()
            .push_back(location);
        if !tool_use_id.trim().is_empty() {
            self.pending
                .entry(tool_id_key(tool_use_id))
                .or_default()
                .push_back(location);
        }
    }

    fn take_pending_location(&mut self, tool_use_id: &str, name: &str) -> Option<ToolLocation> {
        if !tool_use_id.trim().is_empty() {
            return self.pop_running_location(&tool_id_key(tool_use_id));
        }
        self.pop_running_location(&tool_name_key(name))
    }

    fn pop_running_location(&mut self, key: &str) -> Option<ToolLocation> {
        loop {
            let location = self.pending.get_mut(key)?.pop_front()?;
            if self.location_is_running(location) {
                return Some(location);
            }
        }
    }

    fn peek_pending_location(&mut self, key: &str) -> Option<ToolLocation> {
        loop {
            let location = self.pending.get(key)?.front().copied()?;
            if self.location_is_running(location) {
                return Some(location);
            }
            self.pending.get_mut(key)?.pop_front();
        }
    }

    fn location_is_running(&self, location: ToolLocation) -> bool {
        self.boards
            .get(location.board_index)
            .and_then(|board| board.entries.get(location.entry_index))
            .map(ToolActivityEntry::is_running)
            .unwrap_or(false)
    }

    fn entry_mut(&mut self, location: ToolLocation) -> Option<&mut ToolActivityEntry> {
        self.boards
            .get_mut(location.board_index)?
            .entries
            .get_mut(location.entry_index)
    }

    fn render(&self, board_index: usize) -> ToolActivityRender {
        let board = &self.boards[board_index];
        ToolActivityRender {
            board_index,
            message_id: board.message_id.clone(),
            body: render_board(board),
        }
    }
}

pub(crate) fn render_standalone_tool_result(
    emoji: &str,
    name: &str,
    result_preview: &str,
    is_error: bool,
) -> String {
    let mut entry = ToolActivityEntry::running(emoji, name, "");
    entry.status = if is_error {
        ToolStatus::Failed
    } else {
        ToolStatus::Succeeded
    };
    entry.result_preview = result_preview.to_string();
    entry.elapsed = Some(Duration::ZERO);
    format!("### ⚙️ Captain\n\n{}", render_entry(&entry))
}

fn render_board(board: &ToolActivityBoard) -> String {
    let entries = board
        .entries
        .iter()
        .map(render_entry)
        .collect::<Vec<_>>()
        .join("\n\n");
    format!("### ⚙️ Captain\n\n{entries}")
}

fn render_entry(entry: &ToolActivityEntry) -> String {
    let (status, open) = match entry.status {
        ToolStatus::Running => ("⏳", " open"),
        ToolStatus::Succeeded => ("✓", ""),
        ToolStatus::Failed => ("✗", " open"),
    };
    let emoji = escape_html_clipped(&entry.emoji, MAX_ESCAPED_EMOJI_BYTES);
    let name = escape_html_clipped(&entry.name, MAX_ESCAPED_NAME_BYTES);
    let elapsed = entry
        .elapsed
        .map(format_duration)
        .map(|duration| format!(" · {duration}"))
        .unwrap_or_default();
    let spacer = if emoji.is_empty() { "" } else { " " };
    let mut body = format!(
        "<details{open}>\n<summary>{status}{spacer}{emoji} <b>{name}</b>{elapsed}</summary>"
    );

    if !entry.input_preview.trim().is_empty() {
        body.push_str("\n\n<b>Input</b>\n<pre>");
        body.push_str(&escape_html_clipped(
            &entry.input_preview,
            MAX_ESCAPED_INPUT_BYTES,
        ));
        body.push_str("</pre>");
    }
    if !entry.progress.is_empty() {
        body.push_str("\n\n<b>Progress</b>\n<blockquote>");
        let progress = entry
            .progress
            .iter()
            .map(|line| {
                format!(
                    "↳ {}",
                    escape_html_clipped(line, MAX_ESCAPED_PROGRESS_BYTES)
                )
            })
            .collect::<Vec<_>>()
            .join("\n");
        body.push_str(&progress);
        body.push_str("</blockquote>");
    }
    if !entry.result_preview.trim().is_empty() {
        body.push_str("\n\n<b>Result</b>\n<pre>");
        body.push_str(&escape_html_clipped(
            &entry.result_preview,
            MAX_ESCAPED_RESULT_BYTES,
        ));
        body.push_str("</pre>");
    }
    body.push_str("\n</details>");
    body
}

fn progress_lines(chunk: &str) -> Vec<String> {
    chunk
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .take(MAX_PROGRESS_LINES)
        .map(|line| clip_utf8(line, MAX_STORED_PROGRESS_BYTES))
        .collect()
}

fn tool_id_key(tool_use_id: &str) -> String {
    format!("id:{}", tool_use_id.trim())
}

fn tool_name_key(name: &str) -> String {
    format!("name:{name}")
}

fn format_duration(duration: Duration) -> String {
    if duration < Duration::from_secs(1) {
        format!("{} ms", duration.as_millis())
    } else if duration < Duration::from_secs(60) {
        format!("{:.1} s", duration.as_secs_f64())
    } else {
        format!(
            "{} min {:02} s",
            duration.as_secs() / 60,
            duration.as_secs() % 60
        )
    }
}

fn escape_html_clipped(text: &str, max_bytes: usize) -> String {
    const ELLIPSIS: &str = "…";
    let mut out = String::new();
    let mut truncated = false;
    for ch in text.chars() {
        let mut encoded_char = [0_u8; 4];
        let encoded = match ch {
            '&' => "&amp;",
            '<' => "&lt;",
            '>' => "&gt;",
            '"' => "&quot;",
            '\'' => "&#39;",
            _ => ch.encode_utf8(&mut encoded_char),
        };
        if out.len() + encoded.len() + ELLIPSIS.len() > max_bytes {
            truncated = true;
            break;
        }
        out.push_str(encoded);
    }
    if truncated {
        out.push_str(ELLIPSIS);
    }
    out
}

fn clip_utf8(text: &str, max_bytes: usize) -> String {
    if text.len() <= max_bytes {
        return text.to_string();
    }
    let mut cut = max_bytes.saturating_sub("…".len());
    while cut > 0 && !text.is_char_boundary(cut) {
        cut -= 1;
    }
    format!("{}…", &text[..cut])
}

#[cfg(test)]
mod tests {
    use super::*;

    fn start(tracker: &mut ToolActivityTracker, id: &str, name: &str) -> ToolActivityRender {
        tracker.start_tool(id, "💻", name, &format!("input-{name}"))
    }

    #[test]
    fn consecutive_starts_share_one_parallel_activity_board() {
        let mut tracker = ToolActivityTracker::default();
        let first = start(&mut tracker, "a", "alpha");
        assert!(first.message_id.is_none());
        tracker.bind_message_id(first.board_index, "mid_0".to_string());

        let second = start(&mut tracker, "b", "bravo");
        assert_eq!(second.board_index, first.board_index);
        assert_eq!(second.message_id.as_deref(), Some("mid_0"));
        assert_eq!(second.body.matches("<details open>").count(), 2);
        assert!(second.body.contains("<b>alpha</b>"));
        assert!(second.body.contains("<b>bravo</b>"));
    }

    #[test]
    fn results_can_finish_out_of_order_without_crossing_entries() {
        let mut tracker = ToolActivityTracker::default();
        let first = start(&mut tracker, "a", "alpha");
        tracker.bind_message_id(first.board_index, "mid_0".to_string());
        start(&mut tracker, "b", "bravo");

        let bravo = tracker
            .finish_tool("b", "bravo", "result-bravo", false)
            .unwrap();
        let alpha_pos = bravo.body.find("<b>alpha</b>").unwrap();
        let bravo_pos = bravo.body.find("<b>bravo</b>").unwrap();
        let result_pos = bravo.body.find("result-bravo").unwrap();
        assert!(alpha_pos < bravo_pos && bravo_pos < result_pos);
        assert!(bravo.body[..bravo_pos].find("result-bravo").is_none());

        let alpha = tracker
            .finish_tool("a", "alpha", "result-alpha", false)
            .unwrap();
        assert!(alpha.body.contains("result-alpha"));
        assert!(alpha.body.contains("result-bravo"));
    }

    #[test]
    fn progress_targets_the_matching_parallel_entry() {
        let mut tracker = ToolActivityTracker::default();
        let first = start(&mut tracker, "a", "alpha");
        tracker.bind_message_id(first.board_index, "mid_0".to_string());
        start(&mut tracker, "b", "bravo");

        let update = tracker.progress_tool("b", "bravo-only").unwrap();
        let bravo_pos = update.body.find("<b>bravo</b>").unwrap();
        let progress_pos = update.body.find("bravo-only").unwrap();
        assert!(bravo_pos < progress_pos);
        assert!(update.body[..bravo_pos].find("bravo-only").is_none());
    }

    #[test]
    fn completion_closes_the_wave_for_dependent_tools() {
        let mut tracker = ToolActivityTracker::default();
        let first = start(&mut tracker, "a", "alpha");
        tracker.bind_message_id(first.board_index, "mid_0".to_string());
        tracker.finish_tool("a", "alpha", "ok", false).unwrap();

        let dependent = start(&mut tracker, "b", "bravo");
        assert_ne!(dependent.board_index, first.board_index);
        assert!(dependent.message_id.is_none());
    }

    #[test]
    fn success_is_collapsed_and_failure_stays_open() {
        let mut success = ToolActivityTracker::default();
        let first = start(&mut success, "a", "alpha");
        success.bind_message_id(first.board_index, "mid_0".to_string());
        let success_body = success.finish_tool("a", "alpha", "ok", false).unwrap().body;
        assert!(success_body.contains("<details>\n<summary>✓"));
        assert!(!success_body.contains("<details open>\n<summary>✓"));

        let mut failure = ToolActivityTracker::default();
        let first = start(&mut failure, "a", "alpha");
        failure.bind_message_id(first.board_index, "mid_0".to_string());
        let failure_body = failure
            .finish_tool("a", "alpha", "boom", true)
            .unwrap()
            .body;
        assert!(failure_body.contains("<details open>\n<summary>✗"));
    }

    #[test]
    fn tool_content_is_escaped_before_entering_rich_markup() {
        let mut tracker = ToolActivityTracker::default();
        let started = tracker.start_tool(
            "x",
            "<svg>",
            "x</summary><script>alert(1)</script>",
            "</pre><b>owned</b>",
        );
        tracker.bind_message_id(started.board_index, "mid_0".to_string());
        let finished = tracker
            .finish_tool("x", "ignored", "</details><script>bad</script>", true)
            .unwrap();
        assert!(!finished.body.contains("<script>"));
        assert!(!finished.body.contains("<svg>"));
        assert!(finished.body.contains("&lt;script&gt;"));
        assert!(finished.body.contains("&lt;/details&gt;"));
    }

    #[test]
    fn missing_ids_fall_back_to_fifo_by_tool_name() {
        let mut tracker = ToolActivityTracker::default();
        let first = start(&mut tracker, "", "shell_exec");
        tracker.bind_message_id(first.board_index, "mid_0".to_string());
        start(&mut tracker, "", "shell_exec");
        let one = tracker.finish_tool("", "shell_exec", "one", false).unwrap();
        let two = tracker.finish_tool("", "shell_exec", "two", false).unwrap();
        let first_result = one.body.find("one").unwrap();
        let second_entry = one.body.rfind("<b>shell_exec</b>").unwrap();
        assert!(first_result < second_entry);
        assert!(two.body.contains("one") && two.body.contains("two"));
    }

    #[test]
    fn duplicate_known_id_never_completes_another_same_name_tool() {
        let mut tracker = ToolActivityTracker::default();
        let first = start(&mut tracker, "a", "shell_exec");
        tracker.bind_message_id(first.board_index, "mid_0".to_string());
        start(&mut tracker, "b", "shell_exec");
        tracker
            .finish_tool("a", "shell_exec", "one", false)
            .unwrap();

        assert!(tracker
            .finish_tool("a", "shell_exec", "duplicate", false)
            .is_none());
        let second = tracker
            .finish_tool("b", "shell_exec", "two", false)
            .unwrap();
        assert!(!second.body.contains("duplicate"));
        assert!(second.body.contains("one") && second.body.contains("two"));
    }

    #[test]
    fn worst_case_parallel_board_stays_under_telegram_rich_limit() {
        let mut tracker = ToolActivityTracker::default();
        let hostile = "<&\"'>".repeat(400);
        for index in 0..MAX_BOARD_ENTRIES {
            let id = format!("tool-{index}");
            let started = tracker.start_tool(&id, "💻", &hostile, &hostile);
            if index == 0 {
                tracker.bind_message_id(started.board_index, "mid_0".to_string());
            }
        }

        let progress = (0..MAX_PROGRESS_LINES)
            .map(|_| hostile.as_str())
            .collect::<Vec<_>>()
            .join("\n");
        let mut final_body = String::new();
        for index in 0..MAX_BOARD_ENTRIES {
            let id = format!("tool-{index}");
            tracker.progress_tool(&id, &progress).unwrap();
            final_body = tracker
                .finish_tool(&id, &hostile, &hostile, false)
                .unwrap()
                .body;
        }

        assert!(
            final_body.len() <= 32_768,
            "activity board is {} bytes",
            final_body.len()
        );
    }
}
