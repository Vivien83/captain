//! Parallel execution primitives for tool calls (v3.10g).
//!
//! When the LLM emits multiple tool calls in the same assistant message,
//! the agent loop can execute them concurrently only when every call is a
//! known, side-effect-free read. Unknown builtins, MCP tools, skill tools,
//! and newly added tools fail closed to sequential execution until their
//! read-only contract is reviewed and added here.

use captain_types::tool::ToolCall;
use tracing::info;

/// Reviewed read-only tools that may share an execution group.
///
/// This is intentionally an allowlist rather than a denylist. Tool cache
/// policy answers a different question (whether a result is worth caching)
/// and cannot prove that an unknown tool is side-effect-free. Keeping this
/// list small means a newly registered builtin, MCP tool, or skill remains
/// sequential until its effect contract is explicit.
const PARALLEL_SAFE_TOOLS: &[&str] = &[
    "agent_find",
    "agent_list",
    "capability_search",
    "captain_docs",
    "document_extract",
    "file_list",
    "file_read",
    "file_search",
    "hand_list",
    "hand_status",
    "knowledge_query",
    "memory_context_batch",
    "memory_recall",
    "mcp_status",
    "project_get",
    "project_list",
    "session_recall",
    "session_tool_call_summary",
    "skill_search",
    "skill_view",
    "system_time",
    "tool_run_list",
    "tool_run_result",
    "tool_run_status",
    "tool_search",
    "web_fetch",
    "web_search",
];

/// True only when a tool has an explicit, reviewed read-only contract.
pub fn is_parallel_safe(tool_name: &str) -> bool {
    PARALLEL_SAFE_TOOLS.contains(&tool_name)
}

/// Conservative side-effect classification used by existing callers and
/// tests. Unknown tools are treated as side-effecting and run sequentially.
pub fn is_side_effect(tool_name: &str) -> bool {
    !is_parallel_safe(tool_name)
}

/// A group of tool calls that can run together.
///
/// `Parallel` groups are safe to `futures::future::join_all` across.
/// `Sequential` groups contain exactly one call and must run in the
/// order they appear.
#[derive(Debug, Clone)]
pub enum ExecutionGroup {
    Parallel(Vec<ToolCall>),
    Sequential(ToolCall),
}

/// Partition `tool_calls` into groups that preserve relative order.
///
/// Consecutive side-effect-free calls collapse into one `Parallel`
/// group; any side-effect call becomes its own `Sequential` group and
/// acts as a fence. The result has the invariant:
/// `flatten(groups) == tool_calls` (order-preserving), so replay is
/// deterministic regardless of parallelism speed-ups.
pub fn partition_parallel_groups(tool_calls: &[ToolCall]) -> Vec<ExecutionGroup> {
    let mut groups: Vec<ExecutionGroup> = Vec::new();
    let mut pending: Vec<ToolCall> = Vec::new();

    for call in tool_calls {
        if !is_parallel_safe(&call.name) {
            if !pending.is_empty() {
                groups.push(ExecutionGroup::Parallel(std::mem::take(&mut pending)));
            }
            groups.push(ExecutionGroup::Sequential(call.clone()));
        } else {
            pending.push(call.clone());
        }
    }
    if !pending.is_empty() {
        groups.push(ExecutionGroup::Parallel(pending));
    }
    groups
}

pub fn parallelizable_call_count(tool_calls: &[ToolCall]) -> usize {
    partition_parallel_groups(tool_calls)
        .iter()
        .map(|group| match group {
            ExecutionGroup::Parallel(calls) if calls.len() >= 2 => calls.len(),
            _ => 0,
        })
        .sum()
}

/// Emit an observability signal when the current batch contains a concurrent
/// read-only group.
pub fn log_parallel_opportunity(tool_calls: &[ToolCall]) {
    if tool_calls.len() < 2 {
        return;
    }

    let parallel_size = parallelizable_call_count(tool_calls);
    if parallel_size >= 2 {
        info!(
            tool_calls = tool_calls.len(),
            parallelizable = parallel_size,
            "tool batch contains reviewed read-only calls that will run concurrently",
        );
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn tc(name: &str) -> ToolCall {
        ToolCall {
            id: format!("id-{name}"),
            name: name.to_string(),
            input: serde_json::Value::Null,
        }
    }

    #[test]
    fn known_side_effect_tools_are_flagged() {
        assert!(is_side_effect("shell_exec"));
        assert!(is_side_effect("file_write"));
        assert!(is_side_effect("memory_store"));
        assert!(is_side_effect("browser_keys"));
        assert!(is_side_effect("browser_select"));
        assert!(is_side_effect("browser_hover"));
        assert!(is_side_effect("tool_run_start"));
        assert!(is_side_effect("tool_run_cancel"));
        assert!(is_side_effect("ask_user"));
        assert!(is_side_effect("document_create"));
        assert!(is_side_effect("memory_forget"));
        assert!(is_side_effect("browser_batch"));
        assert!(is_side_effect("browser_wait"));
        assert!(is_side_effect("browser_close"));
    }

    #[test]
    fn read_tools_are_not_side_effect() {
        assert!(!is_side_effect("file_read"));
        assert!(!is_side_effect("web_search"));
        assert!(!is_side_effect("memory_recall"));
        assert!(!is_side_effect("tool_run_status"));
        assert!(!is_side_effect("tool_run_result"));
    }

    #[test]
    fn unknown_tool_defaults_to_sequential() {
        assert!(is_side_effect("totally_invented"));
        assert!(!is_parallel_safe("mcp__unknown_server__unknown_tool"));
        assert!(!is_parallel_safe("skill_generated_mutation"));
    }

    #[test]
    fn empty_input_gives_empty_output() {
        assert!(partition_parallel_groups(&[]).is_empty());
    }

    #[test]
    fn all_reads_collapse_into_one_parallel_group() {
        let calls = vec![tc("file_read"), tc("web_search"), tc("memory_recall")];
        let groups = partition_parallel_groups(&calls);
        assert_eq!(groups.len(), 1);
        match &groups[0] {
            ExecutionGroup::Parallel(cs) => assert_eq!(cs.len(), 3),
            _ => panic!("expected a parallel group"),
        }
    }

    #[test]
    fn writes_fence_parallel_groups() {
        let calls = vec![
            tc("file_read"),
            tc("web_search"),
            tc("file_write"),
            tc("file_read"),
            tc("memory_recall"),
        ];
        let groups = partition_parallel_groups(&calls);
        assert_eq!(groups.len(), 3);

        match &groups[0] {
            ExecutionGroup::Parallel(cs) => assert_eq!(cs.len(), 2),
            _ => panic!("group 0 should be parallel"),
        }
        match &groups[1] {
            ExecutionGroup::Sequential(c) => assert_eq!(c.name, "file_write"),
            _ => panic!("group 1 should be sequential"),
        }
        match &groups[2] {
            ExecutionGroup::Parallel(cs) => assert_eq!(cs.len(), 2),
            _ => panic!("group 2 should be parallel"),
        }
    }

    #[test]
    fn ordering_is_preserved_when_flattened() {
        let calls = vec![
            tc("file_read"),
            tc("shell_exec"),
            tc("file_read"),
            tc("memory_store"),
            tc("web_search"),
        ];
        let groups = partition_parallel_groups(&calls);

        let mut flat: Vec<&ToolCall> = Vec::new();
        for g in &groups {
            match g {
                ExecutionGroup::Parallel(cs) => flat.extend(cs.iter()),
                ExecutionGroup::Sequential(c) => flat.push(c),
            }
        }
        let names: Vec<&str> = flat.iter().map(|c| c.name.as_str()).collect();
        assert_eq!(
            names,
            vec![
                "file_read",
                "shell_exec",
                "file_read",
                "memory_store",
                "web_search",
            ]
        );
    }

    #[test]
    fn single_write_tool_yields_one_sequential() {
        let groups = partition_parallel_groups(&[tc("file_write")]);
        assert_eq!(groups.len(), 1);
        assert!(matches!(&groups[0], ExecutionGroup::Sequential(_)));
    }

    #[test]
    fn single_read_tool_yields_one_parallel_of_one() {
        let groups = partition_parallel_groups(&[tc("file_read")]);
        assert_eq!(groups.len(), 1);
        match &groups[0] {
            ExecutionGroup::Parallel(cs) => assert_eq!(cs.len(), 1),
            _ => panic!("expected parallel"),
        }
    }

    #[test]
    fn parallelizable_count_ignores_singletons_and_side_effects() {
        let calls = [
            tc("file_read"),
            tc("web_search"),
            tc("shell_exec"),
            tc("memory_recall"),
        ];

        assert_eq!(parallelizable_call_count(&calls), 2);
    }

    #[test]
    fn parallelizable_count_accumulates_separate_read_batches() {
        let calls = [
            tc("file_read"),
            tc("web_search"),
            tc("file_write"),
            tc("memory_recall"),
            tc("knowledge_query"),
        ];

        assert_eq!(parallelizable_call_count(&calls), 4);
    }
}
