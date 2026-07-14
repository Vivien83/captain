use crate::agent_loop_tool_record::ToolCallRecord;
use captain_types::message::{ReplyDirectives, TokenUsage};

/// Result of an agent loop execution.
#[derive(Debug)]
pub struct AgentLoopResult {
    /// The final text response from the agent.
    pub response: String,
    /// Total token usage across all LLM calls.
    pub total_usage: TokenUsage,
    /// Number of iterations the loop ran.
    pub iterations: u32,
    /// Estimated cost in USD (populated by the kernel after the loop returns).
    pub cost_usd: Option<f64>,
    /// True when the agent intentionally chose not to reply (NO_REPLY token or [[silent]]).
    pub silent: bool,
    /// Reply directives extracted from the agent's response.
    pub directives: ReplyDirectives,
    /// All tool calls made during this agent turn (for graph instrumentation).
    pub tool_calls: Vec<ToolCallRecord>,
}

impl AgentLoopResult {
    /// True iff at least one successful tool call already pushed content to
    /// a channel during this turn (currently `channel_send`). Cron paths
    /// use this to avoid re-delivering `response` on top of a message the
    /// agent has already sent through the tool: the canonical double-message
    /// fix on `AgentTurn` and `InlineWorkflow`.
    pub fn delivered_via_channel_tool(&self) -> bool {
        self.tool_calls
            .iter()
            .any(|tc| !tc.is_error && tc.tool_name == "channel_send")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn record(tool_name: &str, is_error: bool) -> ToolCallRecord {
        ToolCallRecord {
            tool_name: tool_name.to_string(),
            reason: "Use this tool to continue the current task.".to_string(),
            is_error,
            duration_ms: 12,
            input_summary: "input".to_string(),
            output_summary: "output".to_string(),
        }
    }

    fn result(tool_calls: Vec<ToolCallRecord>) -> AgentLoopResult {
        AgentLoopResult {
            response: "ok".to_string(),
            total_usage: TokenUsage::default(),
            iterations: 1,
            cost_usd: None,
            silent: false,
            directives: ReplyDirectives::default(),
            tool_calls,
        }
    }

    #[test]
    fn delivered_via_channel_tool_detects_successful_channel_send() {
        let result = result(vec![
            record("web_search", false),
            record("channel_send", false),
        ]);

        assert!(result.delivered_via_channel_tool());
    }

    #[test]
    fn delivered_via_channel_tool_ignores_failed_channel_send() {
        let result = result(vec![record("channel_send", true)]);

        assert!(!result.delivered_via_channel_tool());
    }

    #[test]
    fn delivered_via_channel_tool_ignores_other_successful_tools() {
        let result = result(vec![
            record("file_write", false),
            record("web_fetch", false),
        ]);

        assert!(!result.delivered_via_channel_tool());
    }
}
