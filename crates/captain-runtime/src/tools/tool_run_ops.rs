//! Tool run supervision operations.

use crate::kernel_handle::KernelHandle;
use crate::tool_runs::{global_registry, parse_status_filter, ToolRunSnapshot, ToolRunStatus};
use captain_types::config::ExecPolicy;
use captain_types::event::{EventPayload, ToolRunEvent};
use captain_types::tool::ToolResult;
use std::path::PathBuf;
use std::sync::Arc;

use super::{
    dispatch_package_tool, dispatch_shell_exec, dispatch_ssh_tool, tool_execute_code,
    ShellDispatchOutcome,
};

const DETACHABLE_TOOLS: &[&str] = &[
    "shell_exec",
    "ssh_exec",
    "ssh_health_check",
    "execute_code",
    "cargo",
    "npm",
    "pip",
];

#[derive(Clone)]
pub(crate) struct ToolRunStartContext {
    pub(crate) kernel: Option<Arc<dyn KernelHandle>>,
    pub(crate) allowed_tools: Option<Vec<String>>,
    pub(crate) caller_agent_id: Option<String>,
    pub(crate) allowed_env_vars: Vec<String>,
    pub(crate) workspace_root: Option<PathBuf>,
    pub(crate) exec_policy: Option<ExecPolicy>,
}

pub(crate) async fn tool_run_start(
    input: &serde_json::Value,
    ctx: ToolRunStartContext,
) -> Result<String, String> {
    let target_tool = input["tool_name"]
        .as_str()
        .map(captain_types::tool_compat::normalize_tool_name)
        .ok_or("Missing 'tool_name' parameter")?;
    let target_input = input
        .get("input")
        .cloned()
        .ok_or("Missing 'input' parameter")?;
    let reason = input["reason"]
        .as_str()
        .map(str::trim)
        .filter(|s| !s.is_empty());
    let depends_on = parse_depends_on(input)?;

    assert_detachable_tool_allowed(target_tool, ctx.allowed_tools.as_deref())?;
    if let Some(blocked) = blocking_dependency_response(target_tool, &depends_on)? {
        return Ok(blocked);
    }

    let registry = global_registry();
    let run_id = registry.start(
        target_tool.to_string(),
        ctx.caller_agent_id.clone(),
        None,
        true,
    );
    let task_run_id = run_id.clone();
    let task_registry = registry.clone();
    let task_tool = target_tool.to_string();
    // Captured before `ctx` moves into the detached execution below — used
    // afterwards to surface completion on the event bus and, if the caller
    // isn't already mid-turn, wake it up rather than leaving it to remember
    // to poll tool_run_status.
    let notify_kernel = ctx.kernel.clone();
    let notify_caller_agent_id = ctx.caller_agent_id.clone();
    let handle = tokio::spawn(async move {
        let result =
            execute_detached_tool_with_chunk_capture(&task_run_id, &task_tool, target_input, ctx)
                .await;
        task_registry.finish(&task_run_id, &result);
        notify_tool_run_completion(
            notify_kernel,
            notify_caller_agent_id,
            &task_run_id,
            &task_tool,
            if result.is_error {
                ToolRunStatus::Failed
            } else {
                ToolRunStatus::Completed
            },
        )
        .await;
    });
    registry.attach_abort_handle(&run_id, handle.abort_handle());

    Ok(serde_json::json!({
        "run_id": run_id,
        "status": ToolRunStatus::Running.as_str(),
        "tool_name": target_tool,
        "detached": true,
        "cancellable": true,
        "reason": reason,
        "depends_on": depends_on,
        "next_actions": [
            "Use tool_run_status with this run_id to check progress.",
            "Use tool_run_result when status is completed, failed, or cancelled.",
            "Use tool_run_cancel if the run should be stopped."
        ],
    })
    .to_string())
}

/// Surface a detached tool_run's completion: publish it on the event bus
/// (TUI/SSE/agent-API webhook visibility) and, if the caller agent isn't
/// already mid-turn, inject a system message waking it up so it doesn't
/// have to remember to poll tool_run_status/tool_run_result.
async fn notify_tool_run_completion(
    kernel: Option<Arc<dyn KernelHandle>>,
    caller_agent_id: Option<String>,
    run_id: &str,
    tool_name: &str,
    status: ToolRunStatus,
) {
    let Some(kernel) = kernel else { return };
    kernel
        .publish_typed_event(EventPayload::ToolRun(ToolRunEvent {
            run_id: run_id.to_string(),
            tool_name: tool_name.to_string(),
            status: status.as_str().to_string(),
            caller_agent_id: caller_agent_id.clone(),
        }))
        .await;

    let Some(caller_agent_id) = caller_agent_id else {
        return;
    };
    if kernel.agent_is_busy(&caller_agent_id) {
        return;
    }
    let message = format!(
        "Le tool_run {run_id} ({tool_name}) est terminé ({}). \
         Utilise tool_run_result pour voir le résultat.",
        status.as_str()
    );
    if let Err(e) = kernel
        .inject_system_message(&caller_agent_id, &message)
        .await
    {
        tracing::warn!(
            agent = %caller_agent_id,
            run_id,
            error = %e,
            "Failed to wake caller agent after detached tool_run completion"
        );
    }
}

fn parse_depends_on(input: &serde_json::Value) -> Result<Vec<String>, String> {
    let Some(value) = input.get("depends_on") else {
        return Ok(Vec::new());
    };
    let Some(items) = value.as_array() else {
        return Err("'depends_on' must be an array of run ids".to_string());
    };
    let mut run_ids = Vec::new();
    for item in items {
        let run_id = item
            .as_str()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .ok_or("'depends_on' must contain only non-empty string run ids")?;
        run_ids.push(run_id.to_string());
    }
    Ok(run_ids)
}

fn blocking_dependency_response(
    target_tool: &str,
    depends_on: &[String],
) -> Result<Option<String>, String> {
    if depends_on.is_empty() {
        return Ok(None);
    }
    let registry = global_registry();
    let mut dependencies = Vec::new();
    let mut blocking = Vec::new();
    for run_id in depends_on {
        let snapshot = registry
            .snapshot(run_id)
            .ok_or_else(|| format!("Unknown dependency tool run id: {run_id}"))?;
        if snapshot.status != ToolRunStatus::Completed || snapshot.is_error == Some(true) {
            blocking.push(snapshot.run_id.clone());
        }
        dependencies.push(snapshot_json(&snapshot));
    }
    if blocking.is_empty() {
        return Ok(None);
    }
    Ok(Some(
        serde_json::json!({
            "status": "blocked_dependency",
            "detached": false,
            "tool_name": target_tool,
            "blocking_run_ids": blocking,
            "dependencies": dependencies,
            "next_actions": [
                "Use tool_run_status/tool_run_result on the blocking run ids.",
                "Only start this dependent tool after required runs are completed successfully."
            ],
        })
        .to_string(),
    ))
}

fn snapshot_json(snapshot: &ToolRunSnapshot) -> serde_json::Value {
    serde_json::to_value(snapshot).unwrap_or_else(|_| serde_json::json!({}))
}

async fn execute_detached_tool_with_chunk_capture(
    run_id: &str,
    target_tool: &str,
    target_input: serde_json::Value,
    ctx: ToolRunStartContext,
) -> ToolResult {
    let (stream_tx, mut stream_rx) = tokio::sync::mpsc::channel(64);
    let registry = global_registry();
    let capture_run_id = run_id.to_string();
    let capture_task = tokio::spawn(async move {
        while let Some(event) = stream_rx.recv().await {
            if let crate::llm_driver::StreamEvent::ToolOutputDelta { stream, chunk, .. } = event {
                registry.append_chunk(&capture_run_id, stream, &chunk);
            }
        }
    });
    let stream_ctx = Some(crate::tool_runner::ToolStreamCtx {
        tool_use_id: run_id.to_string(),
        tx: stream_tx,
    });
    let result = crate::tool_runner::TOOL_STREAM
        .scope(
            stream_ctx,
            execute_detached_tool(run_id, target_tool, target_input, ctx),
        )
        .await;
    let _ = capture_task.await;
    result
}

async fn execute_detached_tool(
    run_id: &str,
    target_tool: &str,
    target_input: serde_json::Value,
    ctx: ToolRunStartContext,
) -> ToolResult {
    let result = match target_tool {
        "shell_exec" => {
            match dispatch_shell_exec(
                run_id,
                &target_input,
                ctx.kernel.as_ref(),
                ctx.caller_agent_id.as_deref(),
                Some(&ctx.allowed_env_vars),
                ctx.workspace_root.as_deref(),
                ctx.exec_policy.as_ref(),
            )
            .await
            {
                ShellDispatchOutcome::Blocked(result) => return result,
                ShellDispatchOutcome::Result(result) => result,
            }
        }
        "ssh_exec" | "ssh_health_check" => {
            dispatch_ssh_tool(target_tool, &target_input, ctx.exec_policy.as_ref()).await
        }
        "execute_code" => tool_execute_code(&target_input, ctx.workspace_root.as_deref()).await,
        "cargo" | "npm" | "pip" => {
            dispatch_package_tool(
                target_tool,
                &target_input,
                Some(&ctx.allowed_env_vars),
                ctx.workspace_root.as_deref(),
                ctx.exec_policy.as_ref(),
            )
            .await
        }
        other => Err(format!("Tool `{other}` is not detachable.")),
    };
    tool_result_from_result(run_id, result)
}

fn tool_result_from_result(tool_use_id: &str, result: Result<String, String>) -> ToolResult {
    match result {
        Ok(content) => ToolResult {
            tool_use_id: tool_use_id.to_string(),
            content,
            is_error: false,
            transient_content: Vec::new(),
        },
        Err(error) => ToolResult {
            tool_use_id: tool_use_id.to_string(),
            content: error,
            is_error: true,
            transient_content: Vec::new(),
        },
    }
}

fn assert_detachable_tool_allowed(
    target_tool: &str,
    allowed_tools: Option<&[String]>,
) -> Result<(), String> {
    if !DETACHABLE_TOOLS.contains(&target_tool) {
        return Err(format!(
            "Tool `{target_tool}` is not detachable yet. Supported detached tools: {}.",
            DETACHABLE_TOOLS.join(", ")
        ));
    }
    if target_tool.starts_with("tool_run_") {
        return Err("tool_run tools cannot be launched recursively.".to_string());
    }
    if let Some(allowed_tools) = allowed_tools {
        let allowed = allowed_tools.iter().any(|tool| {
            tool == target_tool
                || captain_types::tool_compat::normalize_tool_name(tool) == target_tool
        });
        if !allowed {
            return Err(format!(
                "tool_run_start blocked: target tool `{target_tool}` is not in this agent's allowed tool list."
            ));
        }
    }
    Ok(())
}

pub(crate) fn tool_run_status(input: &serde_json::Value) -> Result<String, String> {
    let run_id = input["run_id"]
        .as_str()
        .ok_or("Missing 'run_id' parameter")?;
    let snapshot = global_registry()
        .snapshot(run_id)
        .ok_or_else(|| format!("Unknown tool run id: {run_id}"))?;
    serde_json::to_string(&snapshot).map_err(|err| err.to_string())
}

pub(crate) fn tool_run_result(input: &serde_json::Value) -> Result<String, String> {
    let run_id = input["run_id"]
        .as_str()
        .ok_or("Missing 'run_id' parameter")?;
    let snapshot = global_registry()
        .result(run_id)
        .ok_or_else(|| format!("Unknown tool run id: {run_id}"))?;
    serde_json::to_string(&snapshot).map_err(|err| err.to_string())
}

pub(crate) fn tool_run_cancel(input: &serde_json::Value) -> Result<String, String> {
    let run_id = input["run_id"]
        .as_str()
        .ok_or("Missing 'run_id' parameter")?;
    let snapshot = global_registry().cancel(run_id)?;
    serde_json::to_string(&snapshot).map_err(|err| err.to_string())
}

pub(crate) fn tool_run_list(input: &serde_json::Value) -> Result<String, String> {
    let status = parse_status_filter(input["status"].as_str())?;
    let limit = input["limit"].as_u64().unwrap_or(20).clamp(1, 50) as usize;
    let runs = global_registry().list(status, limit);
    Ok(serde_json::json!({
        "runs": runs,
        "count": runs.len(),
    })
    .to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_non_detachable_tool() {
        let err = assert_detachable_tool_allowed("browser_click", None).unwrap_err();
        assert!(err.contains("not detachable"));
    }

    #[test]
    fn rejects_target_outside_agent_allowlist() {
        let allowed = vec!["tool_run_start".to_string()];
        let err = assert_detachable_tool_allowed("shell_exec", Some(&allowed)).unwrap_err();
        assert!(err.contains("allowed tool list"));
    }

    #[test]
    fn allows_target_inside_agent_allowlist() {
        let allowed = vec!["tool_run_start".to_string(), "shell_exec".to_string()];
        assert_detachable_tool_allowed("shell_exec", Some(&allowed)).unwrap();
    }

    #[test]
    fn tool_run_list_filters_status() {
        let registry = global_registry();
        let run_id = registry.start("shell_exec", None, None, true);
        let out = tool_run_list(&serde_json::json!({"status": "running", "limit": 1})).unwrap();
        assert!(out.contains(&run_id));
        registry.finish_with_content(&run_id, ToolRunStatus::Cancelled, true, "cleanup".into());
    }

    #[test]
    fn dependency_response_blocks_until_required_run_completes() {
        let registry = global_registry();
        let run_id = registry.start("shell_exec", None, None, true);

        let blocked = blocking_dependency_response("ssh_exec", std::slice::from_ref(&run_id))
            .unwrap()
            .expect("running dependency should block");
        let parsed: serde_json::Value = serde_json::from_str(&blocked).unwrap();
        assert_eq!(parsed["status"], "blocked_dependency");
        assert_eq!(parsed["blocking_run_ids"][0], run_id);

        registry.finish_with_content(&run_id, ToolRunStatus::Completed, false, "ok".into());
        assert!(blocking_dependency_response("ssh_exec", &[run_id])
            .unwrap()
            .is_none());
    }

    #[test]
    fn parse_depends_on_rejects_non_string_ids() {
        let err = parse_depends_on(&serde_json::json!({"depends_on": [42]})).unwrap_err();
        assert!(err.contains("non-empty string"));
    }

    #[tokio::test]
    async fn tool_run_start_launches_detached_shell_result() {
        let out = tool_run_start(
            &serde_json::json!({
                "tool_name": "shell_exec",
                "input": {
                    "command": "sh -c 'printf detached-tool-run'",
                    "timeout_seconds": 1
                },
                "reason": "test detached execution"
            }),
            ToolRunStartContext {
                kernel: None,
                allowed_tools: Some(vec![
                    "tool_run_start".to_string(),
                    "tool_run_status".to_string(),
                    "tool_run_result".to_string(),
                    "shell_exec".to_string(),
                ]),
                caller_agent_id: Some("agent-test".to_string()),
                allowed_env_vars: Vec::new(),
                workspace_root: None,
                exec_policy: None,
            },
        )
        .await
        .expect("detached shell should start");
        let parsed: serde_json::Value = serde_json::from_str(&out).unwrap();
        let run_id = parsed["run_id"].as_str().unwrap();

        let mut last_status = String::new();
        for _ in 0..20 {
            let status_json = tool_run_status(&serde_json::json!({ "run_id": run_id })).unwrap();
            let status: serde_json::Value = serde_json::from_str(&status_json).unwrap();
            last_status = status["status"].as_str().unwrap().to_string();
            if last_status != "running" {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(25)).await;
        }

        assert_eq!(last_status, "completed");
        let result_json = tool_run_result(&serde_json::json!({ "run_id": run_id })).unwrap();
        let result: serde_json::Value = serde_json::from_str(&result_json).unwrap();
        assert_eq!(result["status"], "completed");
        assert!(result["result"]
            .as_str()
            .unwrap()
            .contains("detached-tool-run"));
    }

    /// Captures wake-up calls made after a detached tool_run completes, so
    /// tests can assert on them without a real kernel/LLM.
    struct WakeUpCapturingKernel {
        published: std::sync::Mutex<Vec<EventPayload>>,
        injected_messages: std::sync::Mutex<Vec<(String, String)>>,
    }

    #[async_trait::async_trait]
    impl KernelHandle for WakeUpCapturingKernel {
        async fn spawn_agent(
            &self,
            _manifest: &str,
            _parent: Option<&str>,
        ) -> Result<(String, String), String> {
            Err("stub".into())
        }
        async fn send_to_agent(&self, _id: &str, _msg: &str) -> Result<String, String> {
            Err("stub".into())
        }
        fn list_agents(&self) -> Vec<crate::kernel_handle::AgentInfo> {
            Vec::new()
        }
        fn kill_agent(&self, _id: &str) -> Result<(), String> {
            Ok(())
        }
        fn memory_store(&self, _key: &str, _value: serde_json::Value) -> Result<(), String> {
            Ok(())
        }
        fn memory_recall(&self, _key: &str) -> Result<Option<serde_json::Value>, String> {
            Ok(None)
        }
        fn find_agents(&self, _q: &str) -> Vec<crate::kernel_handle::AgentInfo> {
            Vec::new()
        }
        async fn task_post(
            &self,
            _t: &str,
            _d: &str,
            _a: Option<&str>,
            _c: Option<&str>,
        ) -> Result<String, String> {
            Err("stub".into())
        }
        async fn task_claim(&self, _id: &str) -> Result<Option<serde_json::Value>, String> {
            Ok(None)
        }
        async fn task_complete(&self, _id: &str, _r: &str) -> Result<(), String> {
            Ok(())
        }
        async fn publish_typed_event(&self, payload: EventPayload) {
            self.published.lock().unwrap().push(payload);
        }
        async fn inject_system_message(&self, agent_id: &str, message: &str) -> Result<(), String> {
            self.injected_messages
                .lock()
                .unwrap()
                .push((agent_id.to_string(), message.to_string()));
            Ok(())
        }
    }

    #[tokio::test]
    async fn tool_run_completion_publishes_event_and_wakes_idle_caller() {
        let kernel = Arc::new(WakeUpCapturingKernel {
            published: std::sync::Mutex::new(Vec::new()),
            injected_messages: std::sync::Mutex::new(Vec::new()),
        });

        let out = tool_run_start(
            &serde_json::json!({
                "tool_name": "shell_exec",
                "input": {
                    "command": "sh -c 'printf ok'",
                    "timeout_seconds": 1
                }
            }),
            ToolRunStartContext {
                kernel: Some(kernel.clone() as Arc<dyn KernelHandle>),
                allowed_tools: Some(vec!["shell_exec".to_string()]),
                caller_agent_id: Some("caller-agent".to_string()),
                allowed_env_vars: Vec::new(),
                workspace_root: None,
                exec_policy: None,
            },
        )
        .await
        .expect("detached shell should start");
        let run_id = serde_json::from_str::<serde_json::Value>(&out).unwrap()["run_id"]
            .as_str()
            .unwrap()
            .to_string();

        for _ in 0..50 {
            if !kernel.injected_messages.lock().unwrap().is_empty() {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }

        let published = kernel.published.lock().unwrap();
        assert_eq!(published.len(), 1, "exactly one ToolRun event expected");
        match &published[0] {
            EventPayload::ToolRun(ev) => {
                assert_eq!(ev.run_id, run_id);
                assert_eq!(ev.tool_name, "shell_exec");
                assert_eq!(ev.status, "completed");
                assert_eq!(ev.caller_agent_id.as_deref(), Some("caller-agent"));
            }
            other => panic!("expected EventPayload::ToolRun, got {other:?}"),
        }

        let injected = kernel.injected_messages.lock().unwrap();
        assert_eq!(injected.len(), 1);
        assert_eq!(injected[0].0, "caller-agent");
        assert!(injected[0].1.contains(&run_id));
        assert!(injected[0].1.contains("tool_run_result"));
    }
}
