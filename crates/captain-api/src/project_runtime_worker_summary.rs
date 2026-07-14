use crate::project_runtime_checkpoints::trim_runtime_text;
use captain_runtime::agent_loop::ToolCallRecord;
use std::collections::HashSet;

pub(crate) fn runtime_worker_summary(
    role: &str,
    phase: &str,
    response: &str,
    tool_calls: &[ToolCallRecord],
) -> String {
    if let Some(block) = extract_runtime_status_block(response) {
        return block;
    }

    let cleaned = trim_runtime_text(response, 2200);
    let meaningful = cleaned.trim();
    if !meaningful.is_empty() && !looks_like_tool_transcript(meaningful) {
        return meaningful.to_string();
    }

    runtime_tool_call_summary(role, phase, tool_calls)
}

fn extract_runtime_status_block(response: &str) -> Option<String> {
    let cleaned = trim_runtime_text(response, 4000);
    let lower = cleaned.to_ascii_lowercase();
    let idx = lower.rfind("status:")?;
    let block = cleaned[idx..].trim();
    if block.len() < "status: complete".len() {
        return None;
    }
    Some(trim_runtime_text(block, 2200))
}

fn looks_like_tool_transcript(text: &str) -> bool {
    let trimmed = text.trim_start();
    let lower = trimmed.to_ascii_lowercase();
    trimmed.starts_with('{')
        || trimmed.starts_with("to=")
        || lower.contains("to=shell_exec")
        || lower.contains("to=file_")
        || lower.contains("to=apply_patch")
        || lower.matches("to=").count() >= 2
}

fn runtime_tool_call_summary(role: &str, phase: &str, tool_calls: &[ToolCallRecord]) -> String {
    let total = tool_calls.len();
    let failures = tool_calls.iter().filter(|call| call.is_error).count();
    let mut seen = HashSet::new();
    let tools = tool_calls
        .iter()
        .filter_map(|call| {
            if seen.insert(call.tool_name.as_str()) {
                Some(call.tool_name.as_str())
            } else {
                None
            }
        })
        .take(6)
        .collect::<Vec<_>>()
        .join(", ");
    let tool_phrase = if total == 0 {
        "no tool calls".to_string()
    } else if tools.is_empty() {
        format!("{total} tool calls")
    } else {
        format!("{total} tool calls ({tools})")
    };
    let verify = if failures == 0 {
        "No tool errors were reported by the worker loop."
    } else {
        "One or more tool calls reported errors; inspect the worker session for details."
    };
    format!(
        "STATUS: complete\n\
         SUMMARY: The {role} phase completed for {phase}. The provider returned a tool transcript instead of a clean final handoff, so Captain generated this readable summary from execution metadata. The worker used {tool_phrase}.\n\
         CHANGED_FILES: see worker session transcript\n\
         VERIFY: {verify}\n\
         NEXT: continue to the next gated project phase"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn runtime_worker_summary_prefers_final_status_block() {
        let response = "{\"path\":\"/tmp/project\"}to=file_list\nSTATUS: complete\nSUMMARY: Repo inspected.\nVERIFY: none";
        let summary = runtime_worker_summary("observer", "observe", response, &[]);
        assert!(summary.starts_with("STATUS: complete"));
        assert!(summary.contains("SUMMARY: Repo inspected."));
        assert!(!summary.contains("to=file_list"));
    }

    #[test]
    fn runtime_worker_summary_synthesizes_when_provider_returns_tool_transcript() {
        let calls = vec![ToolCallRecord {
            tool_name: "shell_exec".to_string(),
            reason: "Run the build command.".to_string(),
            is_error: false,
            duration_ms: 10,
            input_summary: "ls".to_string(),
            output_summary: "ok".to_string(),
        }];
        let summary = runtime_worker_summary(
            "builder",
            "build",
            "{\"command\":\"ls\"}to=shell_exec code",
            &calls,
        );
        assert!(summary.starts_with("STATUS: complete"));
        assert!(summary.contains("builder phase completed"));
        assert!(summary.contains("shell_exec"));
        assert!(!summary.contains("{\"command\""));
    }
}
