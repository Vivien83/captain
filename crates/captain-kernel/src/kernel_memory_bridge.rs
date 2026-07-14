use crate::event_bus::EventBus;
use captain_types::agent::AgentId;
use captain_types::event::{ChatStreamEvent, Event, EventPayload, EventTarget};
use std::path::Path;
use std::sync::Arc;
use tracing::{debug, info, warn};

/// Compact tool trace for the Haiku/Kimi learning pass at end of run.
/// This keeps successful capability/doc lookups reproducible without
/// sending raw command inputs or full tool payloads into the reflector.
pub(crate) fn learning_workflow_outcome(
    status: &str,
    tool_calls: &[captain_runtime::agent_loop::ToolCallRecord],
) -> String {
    if tool_calls.is_empty() {
        return status.to_string();
    }

    let errors = tool_calls.iter().filter(|tc| tc.is_error).count();
    let mut parts = Vec::new();
    for (idx, tc) in tool_calls.iter().take(8).enumerate() {
        let state = if tc.is_error { "error" } else { "ok" };
        let mut line = format!(
            "{}. {} {} {}ms",
            idx + 1,
            tc.tool_name,
            state,
            tc.duration_ms
        );

        if tc.is_error {
            let err = captain_types::truncate_str(&tc.output_summary, 140);
            if !err.is_empty() {
                line.push_str(&format!(" err={err}"));
            }
        } else if matches!(
            tc.tool_name.as_str(),
            "capability_search" | "tool_search" | "captain_docs"
        ) {
            let out = captain_types::truncate_str(&tc.output_summary, 180);
            if !out.is_empty() {
                line.push_str(&format!(" out={out}"));
            }
        }

        parts.push(line);
    }
    if tool_calls.len() > parts.len() {
        parts.push(format!("... {} more", tool_calls.len() - parts.len()));
    }

    format!(
        "{status}; tool_trace: {} calls, {} errors; {}",
        tool_calls.len(),
        errors,
        parts.join(" | ")
    )
}

/// Mirror conversation data to MemPalace MCP.
///
/// Since v3.12a, every KG triple goes through the write-through buffer
/// (`memory_writes` table). That makes the mirror resilient to a momentary
/// MemPalace outage without losing signal: the background resync worker
/// replays pending rows on reconnect.
///
/// The diary entry is still sent directly — it is a free-form narrative
/// block rather than a (subject, predicate, object) triple, so it does
/// not fit the write-through model.
///
/// Writes:
/// 1. Conversation turn -> diary entry (direct, best-effort)
/// 2. User preferences -> KG triple via write_through
/// 3. Personal info -> KG triple via write_through
/// 4. Tool results -> KG triple via write_through
pub(crate) async fn mirror_to_mempalace(
    mcp_conns: &tokio::sync::Mutex<Vec<captain_runtime::mcp::McpConnection>>,
    memory_writes_conn: Arc<std::sync::Mutex<rusqlite::Connection>>,
    agent_name: &str,
    user_msg: &str,
    assistant_msg: &str,
    tool_calls: &[captain_runtime::agent_loop::ToolCallRecord],
) {
    // 1. Diary: conversation summary — direct MCP call (not a triple).
    {
        let mut conns = mcp_conns.lock().await;
        if let Some(conn) = conns.iter_mut().find(|c| c.name() == "mempalace") {
            let diary_content = format!(
                "User: {}\nAssistant: {}",
                captain_types::truncate_str(user_msg, 300),
                captain_types::truncate_str(assistant_msg, 500),
            );
            let diary_input = serde_json::json!({
                "agent_name": agent_name,
                "entry": diary_content,
                "topic": "conversation",
            });
            if let Err(e) = conn
                .call_tool("mcp_mempalace_mempalace_diary_write", &diary_input)
                .await
            {
                warn!("MemPalace mirror diary_write failed: {e}");
            }
        } else {
            debug!("MemPalace mirror: no mempalace MCP server connected for diary");
        }
    }

    // Build a sender lazily for the triples below.
    let sender = captain_runtime::memory_writer::McpMemPalaceSender { mcp_conns };
    let sender_ref: &dyn captain_runtime::memory_writer::MemPalaceSender = &sender;

    // 2. User facts (preferences + personal info) — write-through.
    let facts = crate::graph_memory::extract_user_facts(user_msg);
    for fact in &facts {
        let record = captain_memory::memory_writer::NewMemoryWrite {
            subject: fact.subject.clone(),
            predicate: fact.predicate.clone(),
            object: fact.object.clone(),
            wing: None,
            room: None,
            source: "mirror.fact".to_string(),
        };
        if let Err(e) = captain_runtime::memory_writer::write_through(
            Arc::clone(&memory_writes_conn),
            Some(sender_ref),
            record,
        )
        .await
        {
            debug!("MemPalace mirror write_through(fact) failed: {e}");
        }
    }

    // 3. Tool results tracking — write-through.
    for tc in tool_calls {
        let predicate = if tc.is_error { "failed" } else { "succeeded" };
        let object = if tc.is_error {
            captain_types::truncate_str(&tc.output_summary, 200).to_string()
        } else {
            format!("{}ms", tc.duration_ms)
        };
        let record = captain_memory::memory_writer::NewMemoryWrite {
            subject: tc.tool_name.clone(),
            predicate: predicate.to_string(),
            object,
            wing: None,
            room: None,
            source: "mirror.tool_result".to_string(),
        };
        if let Err(e) = captain_runtime::memory_writer::write_through(
            Arc::clone(&memory_writes_conn),
            Some(sender_ref),
            record,
        )
        .await
        {
            debug!(
                "MemPalace mirror write_through(tool) failed for {}: {e}",
                tc.tool_name
            );
        }
    }

    if !facts.is_empty() || !tool_calls.is_empty() {
        info!(
            facts = facts.len(),
            tools = tool_calls.len(),
            "MemPalace mirror: synced turn + {} facts + {} tool results (write-through)",
            facts.len(),
            tool_calls.len(),
        );
    }
}

/// Append an assistant response summary to the daily memory log (best-effort, append-only).
/// Caps daily log at 1MB to prevent unbounded growth.
pub(crate) fn append_daily_memory_log(workspace: &Path, response: &str) {
    use std::io::Write;
    let trimmed = response.trim();
    if trimmed.is_empty() {
        return;
    }
    let today = chrono::Utc::now().format("%Y-%m-%d").to_string();
    let log_path = workspace.join("memory").join(format!("{today}.md"));
    // Security: cap total daily log to 1MB.
    if let Ok(metadata) = std::fs::metadata(&log_path) {
        if metadata.len() > 1_048_576 {
            return;
        }
    }
    // Truncate long responses for the log (UTF-8 safe).
    let summary = captain_types::truncate_str(trimmed, 500);
    let timestamp = chrono::Utc::now().format("%H:%M:%S").to_string();
    if let Ok(mut f) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&log_path)
    {
        let _ = writeln!(f, "\n## {timestamp}\n{summary}\n");
    }
}

/// Phase O.2: bridge that turns auto-memorize commits into broadcast
/// `ChatStreamEvent::MemoryStored` events so connected clients render a
/// learning line in the chat. Holds a cloned `EventBus`; fire-and-forget,
/// never blocks the committer.
pub(crate) struct KernelCommitNotifier {
    event_bus: EventBus,
}

impl KernelCommitNotifier {
    pub(crate) fn new(event_bus: EventBus) -> Self {
        Self { event_bus }
    }
}

#[async_trait::async_trait]
impl captain_runtime::memory_committer::CommitNotifier for KernelCommitNotifier {
    async fn on_queued(
        &self,
        review_id: &str,
        subject: &str,
        predicate: &str,
        object: &str,
        channel: Option<&str>,
        source: &str,
    ) {
        let payload = EventPayload::ChatStream(ChatStreamEvent::MemoryQueued {
            review_id: review_id.to_string(),
            subject: subject.to_string(),
            predicate: predicate.to_string(),
            object: object.to_string(),
            source: source.to_string(),
            channel: channel.map(|c| c.to_string()),
        });
        let event = Event::new(AgentId::default(), EventTarget::Broadcast, payload);
        self.event_bus.publish(event).await;
    }

    async fn on_committed(
        &self,
        committed: &captain_runtime::memory_committer::CommittedLearning,
        source: &str,
    ) {
        let payload = EventPayload::ChatStream(ChatStreamEvent::MemoryStored {
            subject: committed.subject.clone(),
            predicate: committed.predicate.clone(),
            object: committed.object.clone(),
            source: source.to_string(),
            wing: committed.wing.clone(),
            room: committed.room.clone(),
            // Commit-C: the channel propagated end-to-end from the
            // ConversationTurn signal lands here. Subscribers can now
            // route the learning notice back to the originating canal.
            channel: committed.channel.clone(),
            category: committed.category.clone(),
        });
        let event = Event::new(AgentId::default(), EventTarget::Broadcast, payload);
        self.event_bus.publish(event).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn learning_workflow_outcome_preserves_capability_trace() {
        let calls = vec![
            captain_runtime::agent_loop::ToolCallRecord {
                tool_name: "capability_search".into(),
                reason: "Find the right capability for the task.".into(),
                is_error: false,
                duration_ms: 12,
                input_summary: r#"{"query":"ssh server"}"#.into(),
                output_summary: r#"{"results":[{"name":"ssh_exec","source":"builtin"}]}"#.into(),
            },
            captain_runtime::agent_loop::ToolCallRecord {
                tool_name: "ssh_exec".into(),
                reason: "Run the requested remote command.".into(),
                is_error: true,
                duration_ms: 50,
                input_summary: "{}".into(),
                output_summary: "alias not found; use captain_docs for recovery".into(),
            },
        ];

        let outcome = learning_workflow_outcome("success", &calls);
        assert!(outcome.contains("tool_trace: 2 calls"));
        assert!(outcome.contains("capability_search ok"));
        assert!(outcome.contains("ssh_exec error"));
        assert!(outcome.contains("ssh_exec"));
        assert!(
            !outcome.contains(r#"{"query":"ssh server"}"#),
            "raw tool inputs should not be sent into the reflection prompt"
        );
    }

    #[test]
    fn learning_workflow_outcome_limits_trace_volume() {
        let calls = (0..10)
            .map(|idx| captain_runtime::agent_loop::ToolCallRecord {
                tool_name: format!("tool_{idx}"),
                reason: "Use this tool to continue the current task.".into(),
                is_error: false,
                duration_ms: idx,
                input_summary: "hidden".into(),
                output_summary: "summary".into(),
            })
            .collect::<Vec<_>>();

        let outcome = learning_workflow_outcome("success", &calls);
        assert!(outcome.contains("tool_trace: 10 calls"));
        assert!(outcome.contains("... 2 more"));
        assert!(!outcome.contains("tool_9"));
        assert!(!outcome.contains("hidden"));
    }

    #[test]
    fn daily_memory_log_appends_trimmed_summary() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("memory")).unwrap();

        append_daily_memory_log(dir.path(), "  hello memory  ");

        let entries = std::fs::read_dir(dir.path().join("memory"))
            .unwrap()
            .collect::<Result<Vec<_>, _>>()
            .unwrap();
        assert_eq!(entries.len(), 1);
        let content = std::fs::read_to_string(entries[0].path()).unwrap();
        assert!(content.contains("hello memory"));
    }

    #[test]
    fn daily_memory_log_skips_blank_and_oversized_logs() {
        let dir = tempfile::tempdir().unwrap();
        let memory_dir = dir.path().join("memory");
        std::fs::create_dir_all(&memory_dir).unwrap();

        append_daily_memory_log(dir.path(), "   ");
        assert_eq!(std::fs::read_dir(&memory_dir).unwrap().count(), 0);

        let today = chrono::Utc::now().format("%Y-%m-%d").to_string();
        let log_path = memory_dir.join(format!("{today}.md"));
        std::fs::write(&log_path, vec![b'x'; 1_048_577]).unwrap();
        append_daily_memory_log(dir.path(), "should not append");

        let content = std::fs::read_to_string(&log_path).unwrap();
        assert!(!content.contains("should not append"));
    }
}
