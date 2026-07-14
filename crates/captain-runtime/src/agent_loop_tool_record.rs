use crate::agent_loop_hooks::fire_after_tool_call_hook;
use crate::kernel_handle::KernelHandle;
use captain_types::agent::AgentManifest;
use captain_types::tool::{ToolCall, ToolResult};
use std::sync::Arc;

/// Record of a single tool call during agent execution.
#[derive(Debug, Clone, serde::Serialize)]
pub struct ToolCallRecord {
    pub tool_name: String,
    pub reason: String,
    pub is_error: bool,
    pub duration_ms: u64,
    pub input_summary: String,
    pub output_summary: String,
}

pub(crate) fn record_tool_call(
    recorded: &mut Vec<ToolCallRecord>,
    tool_call: &ToolCall,
    result: &ToolResult,
    duration_ms: u64,
) {
    recorded.push(ToolCallRecord {
        tool_name: tool_call.name.clone(),
        reason: tool_decision_reason(tool_call),
        is_error: result.is_error,
        duration_ms,
        input_summary: summarize_json_input(&tool_call.input),
        output_summary: summarize_text(&result.content),
    });
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn record_completed_tool_call(
    recorded: &mut Vec<ToolCallRecord>,
    kernel: Option<&Arc<dyn KernelHandle>>,
    hooks: Option<&crate::hooks::HookRegistry>,
    manifest: &AgentManifest,
    caller_id: &str,
    tool_call: &ToolCall,
    result: &ToolResult,
    duration_ms: u64,
) {
    record_tool_call(recorded, tool_call, result, duration_ms);

    if let Some(kh) = kernel {
        kh.record_temporal_action(&tool_call.name);
    }

    fire_after_tool_call_hook(hooks, manifest, caller_id, tool_call, result);
}

fn summarize_json_input(input: &serde_json::Value) -> String {
    let input_s = serde_json::to_string(input).unwrap_or_default();
    summarize_text(&input_s)
}

fn summarize_text(text: &str) -> String {
    summarize_text_with_limit(text, 200)
}

fn summarize_text_with_limit(text: &str, max_chars: usize) -> String {
    if text.chars().count() > max_chars {
        let keep = max_chars.saturating_sub(3);
        format!("{}\u{2026}", text.chars().take(keep).collect::<String>())
    } else {
        text.to_string()
    }
}

fn tool_decision_reason(tool_call: &ToolCall) -> String {
    explicit_tool_reason(&tool_call.input)
        .unwrap_or_else(|| fallback_tool_reason(&tool_call.name).to_string())
}

fn explicit_tool_reason(input: &serde_json::Value) -> Option<String> {
    ["reason", "why", "purpose", "rationale", "decision_reason"]
        .iter()
        .find_map(|key| input.get(*key).and_then(serde_json::Value::as_str))
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(|value| summarize_text_with_limit(value, 160))
}

fn fallback_tool_reason(tool_name: &str) -> &'static str {
    match tool_name {
        "shell_exec" | "shell_exec_critical" => "Run a shell command needed for the current task.",
        "apply_patch" | "file_write" | "file_edit" | "file_read" | "file_list" => {
            "Inspect or update workspace files for the current task."
        }
        "web_search" | "web_fetch" | "web_research" => {
            "Use web context needed to answer or verify the task."
        }
        "browser_open" | "browser_click" | "browser_type" | "browser_screenshot"
        | "browser_batch" => "Inspect or operate a browser page for the current task.",
        "ask_user" => "Ask the user for missing context before continuing.",
        "channel_send" => "Notify the user through an external channel.",
        "goal_create" | "goal_status" | "goal_pause" | "goal_resume" | "goal_delete" => {
            "Manage an autonomous goal requested by the user."
        }
        _ => "Use this tool to continue the current task.",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hooks::{HookContext, HookHandler, HookRegistry};
    use captain_types::agent::HookEvent;
    use std::sync::Mutex;

    fn tool_call(input: serde_json::Value) -> ToolCall {
        ToolCall {
            id: "call-1".to_string(),
            name: "shell_exec".to_string(),
            input,
        }
    }

    fn tool_result(content: String, is_error: bool) -> ToolResult {
        ToolResult {
            tool_use_id: "call-1".to_string(),
            content,
            is_error,
        }
    }

    struct CaptureHandler {
        calls: Mutex<Vec<serde_json::Value>>,
    }

    impl CaptureHandler {
        fn new() -> Self {
            Self {
                calls: Mutex::new(Vec::new()),
            }
        }

        fn calls(&self) -> Vec<serde_json::Value> {
            self.calls.lock().unwrap().clone()
        }
    }

    impl HookHandler for CaptureHandler {
        fn on_event(&self, ctx: &HookContext) -> Result<(), String> {
            self.calls.lock().unwrap().push(ctx.data.clone());
            Ok(())
        }
    }

    #[test]
    fn record_tool_call_preserves_short_fields() {
        let call = tool_call(serde_json::json!({"cmd": "pwd"}));
        let result = tool_result("ok".to_string(), false);
        let mut recorded = Vec::new();

        record_tool_call(&mut recorded, &call, &result, 42);

        assert_eq!(recorded.len(), 1);
        assert_eq!(recorded[0].tool_name, "shell_exec");
        assert_eq!(
            recorded[0].reason,
            "Run a shell command needed for the current task."
        );
        assert!(!recorded[0].is_error);
        assert_eq!(recorded[0].duration_ms, 42);
        assert_eq!(recorded[0].input_summary, r#"{"cmd":"pwd"}"#);
        assert_eq!(recorded[0].output_summary, "ok");
    }

    #[test]
    fn record_tool_call_prefers_explicit_reason() {
        let call = tool_call(serde_json::json!({
            "cmd": "cargo test",
            "reason": "Verify the edited runtime path before committing."
        }));
        let result = tool_result("ok".to_string(), false);
        let mut recorded = Vec::new();

        record_tool_call(&mut recorded, &call, &result, 42);

        assert_eq!(
            recorded[0].reason,
            "Verify the edited runtime path before committing."
        );
    }

    #[test]
    fn record_tool_call_truncates_long_input_and_output() {
        let call = tool_call(serde_json::json!({"cmd": "x".repeat(260)}));
        let result = tool_result("y".repeat(260), true);
        let mut recorded = Vec::new();

        record_tool_call(&mut recorded, &call, &result, 7);

        assert_eq!(recorded.len(), 1);
        assert!(recorded[0].is_error);
        assert_eq!(recorded[0].input_summary.chars().count(), 198);
        assert_eq!(recorded[0].output_summary.chars().count(), 198);
        assert!(recorded[0].input_summary.ends_with('\u{2026}'));
        assert!(recorded[0].output_summary.ends_with('\u{2026}'));
    }

    #[test]
    fn record_completed_tool_call_records_and_fires_after_hook() {
        let mut manifest = AgentManifest::default();
        manifest.name = "captain".to_string();
        let registry = HookRegistry::new();
        let capture = Arc::new(CaptureHandler::new());
        registry.register(HookEvent::AfterToolCall, capture.clone());
        let call = tool_call(serde_json::json!({"cmd": "pwd"}));
        let result = tool_result("done".to_string(), false);
        let mut recorded = Vec::new();

        record_completed_tool_call(
            &mut recorded,
            None,
            Some(&registry),
            &manifest,
            "agent-1",
            &call,
            &result,
            9,
        );

        assert_eq!(recorded.len(), 1);
        assert_eq!(recorded[0].tool_name, "shell_exec");
        assert_eq!(recorded[0].duration_ms, 9);
        let calls = capture.calls();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0]["tool_name"], "shell_exec");
        assert_eq!(calls[0]["result"], "done");
        assert_eq!(calls[0]["is_error"], false);
    }
}
