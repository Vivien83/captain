//! Public-safe operator projection for durable tool runs.
//!
//! API and local operator surfaces share this type so neither can accidentally
//! serialize raw input, result previews, output filenames, or managed paths.

use crate::tool_run_output::ToolRunOutputPage;
use crate::tool_runs::{ToolRunSnapshot, ToolRunStatus};
use serde::{Deserialize, Serialize};

pub const OPERATOR_MAX_TAIL_LINES: usize = 200;
pub const OPERATOR_MAX_TAIL_BYTES: usize = 32 * 1024;
pub const OPERATOR_WITHHELD_TAIL: &str =
    "[tool run tail withheld because residual secret material was detected]";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OperatorToolRun {
    pub run_id: String,
    pub tool_name: String,
    pub status: ToolRunStatus,
    pub detached: bool,
    pub cancellable: bool,
    pub started_at_unix_ms: u128,
    pub finished_at_unix_ms: Option<u128>,
    pub elapsed_ms: u128,
    pub caller_agent_id: Option<String>,
    pub origin_tool_use_id: Option<String>,
    pub input_sha256: Option<String>,
    pub retry_of_run_id: Option<String>,
    pub retry_attempt: u32,
    pub is_error: Option<bool>,
    pub result_available: bool,
    pub result_truncated: bool,
    pub output_available: bool,
    pub output_stored_bytes: Option<u64>,
    pub output_total_bytes: Option<u64>,
    pub output_sha256: Option<String>,
    pub output_capped: bool,
    pub output_redacted: bool,
}

impl From<ToolRunSnapshot> for OperatorToolRun {
    fn from(snapshot: ToolRunSnapshot) -> Self {
        Self {
            run_id: snapshot.run_id,
            tool_name: snapshot.tool_name,
            status: snapshot.status,
            detached: snapshot.detached,
            cancellable: snapshot.cancellable,
            started_at_unix_ms: snapshot.started_at_unix_ms,
            finished_at_unix_ms: snapshot.finished_at_unix_ms,
            elapsed_ms: snapshot.elapsed_ms,
            caller_agent_id: snapshot.caller_agent_id,
            origin_tool_use_id: snapshot.origin_tool_use_id,
            input_sha256: snapshot.input_sha256,
            retry_of_run_id: snapshot.retry_of_run_id,
            retry_attempt: snapshot.retry_attempt,
            is_error: snapshot.is_error,
            result_available: snapshot.result_preview.is_some(),
            result_truncated: snapshot.result_truncated,
            output_available: snapshot.output_available,
            output_stored_bytes: snapshot.output_stored_bytes,
            output_total_bytes: snapshot.output_total_bytes,
            output_sha256: snapshot.output_sha256,
            output_capped: snapshot.output_capped,
            output_redacted: snapshot.output_redacted,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OperatorToolRunTail {
    pub run_id: String,
    pub status: ToolRunStatus,
    pub start_line: usize,
    pub end_line: usize,
    pub total_lines: usize,
    pub content: String,
    pub content_bytes: usize,
    pub content_truncated: bool,
    pub content_withheld: bool,
    pub sanitized: bool,
}

pub fn operator_tail(
    run_id: &str,
    status: ToolRunStatus,
    page: ToolRunOutputPage,
) -> OperatorToolRunTail {
    let mut content = page.content;
    let mut start_line = page.start_line;
    let mut content_truncated = false;
    if content.len() > OPERATOR_MAX_TAIL_BYTES {
        let start = utf8_suffix_start(&content, OPERATOR_MAX_TAIL_BYTES);
        start_line =
            start_line.saturating_add(content[..start].bytes().filter(|b| *b == b'\n').count());
        content = content[start..].to_string();
        content_truncated = true;
    }
    let content_withheld = crate::memory_policy::scan_for_secrets(&content).is_some();
    if content_withheld {
        content = OPERATOR_WITHHELD_TAIL.to_string();
        content_truncated = true;
    }
    let content_bytes = content.len();
    OperatorToolRunTail {
        run_id: run_id.to_string(),
        status,
        start_line,
        end_line: page.end_line,
        total_lines: page.total_lines,
        content,
        content_bytes,
        content_truncated,
        content_withheld,
        sanitized: true,
    }
}

fn utf8_suffix_start(value: &str, max_bytes: usize) -> usize {
    let mut start = value.len().saturating_sub(max_bytes);
    while start < value.len() && !value.is_char_boundary(start) {
        start += 1;
    }
    start
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn projection_never_serializes_raw_runtime_fields() {
        let registry = crate::tool_runs::ToolRunRegistry::default();
        let run_id = registry.start(
            "shell_exec",
            Some("captain".to_string()),
            Some("tool-use-1".to_string()),
            false,
            Some("a".repeat(64)),
        );
        registry.append_chunk(&run_id, "stdout", "password=do-not-expose\n");
        let value =
            serde_json::to_value(OperatorToolRun::from(registry.snapshot(&run_id).unwrap()))
                .unwrap();
        let object = value.as_object().unwrap();

        assert_eq!(object["input_sha256"], "a".repeat(64));
        for forbidden in ["result", "result_preview", "input", "path", "file_name"] {
            assert!(!object.contains_key(forbidden), "leaked field {forbidden}");
        }
        assert!(!value.to_string().contains("do-not-expose"));
    }

    #[test]
    fn tail_is_utf8_bounded_and_fails_closed_on_residual_secrets() {
        let long = format!("prefix\n{}", "é".repeat(OPERATOR_MAX_TAIL_BYTES));
        let tail = operator_tail(
            "toolrun-long",
            ToolRunStatus::Running,
            ToolRunOutputPage {
                start_line: 1,
                end_line: 2,
                total_lines: 2,
                content: long,
            },
        );
        assert!(tail.content_truncated);
        assert!(tail.content.len() <= OPERATOR_MAX_TAIL_BYTES);
        assert!(!tail.content_withheld);

        let withheld = operator_tail(
            "toolrun-secret",
            ToolRunStatus::Running,
            ToolRunOutputPage {
                start_line: 1,
                end_line: 1,
                total_lines: 1,
                content: "Bearer abcdefghijklmnopqrstuvwxyz123456".to_string(),
            },
        );
        assert!(withheld.content_withheld);
        assert_eq!(withheld.content, OPERATOR_WITHHELD_TAIL);
        assert!(!withheld.content.contains("abcdefghijklmnopqrstuvwxyz"));
    }
}
