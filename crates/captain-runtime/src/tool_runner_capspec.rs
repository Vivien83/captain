use async_trait::async_trait;
use captain_capspec::{
    CapabilityExecutionAuthority, CapabilityExecutionContext, CapabilityInvocation,
    CapabilityInvocationResult, CapabilityToolInvoker, Effect, CAPABILITY_TOOL_PREFIX,
};

use super::{ToolDispatchOutcome, ToolDispatchRequest};

pub(super) async fn dispatch_capspec_tool(
    request: &ToolDispatchRequest<'_>,
) -> Option<ToolDispatchOutcome> {
    if !request.tool_name.starts_with(CAPABILITY_TOOL_PREFIX) {
        return None;
    }
    Some(ToolDispatchOutcome::Dispatched(
        execute_capspec_tool(request).await,
    ))
}

async fn execute_capspec_tool(request: &ToolDispatchRequest<'_>) -> Result<String, String> {
    let kernel = request
        .kernel
        .ok_or_else(|| "CapSpec tools require the Captain kernel".to_string())?;
    let executor = kernel
        .capspec_executor_for_workspace(request.workspace_root)?
        .ok_or_else(|| "CapSpec runtime is not available on this kernel".to_string())?;
    let required_tools = executor
        .required_tools(request.tool_name, request.workspace_root)
        .map_err(|error| error.to_string())?;
    let authority = execution_authority(request, kernel, &required_tools)?;
    let invoker = ToolRunnerCapabilityInvoker { request };
    let execution = executor
        .execute_tool(
            request.tool_name,
            request.input.clone(),
            CapabilityExecutionContext {
                caller_agent_id: request.caller_agent_id.map(str::to_string),
                workspace: request
                    .workspace_root
                    .map(|path| path.to_string_lossy().into_owned()),
                origin: crate::tool_runner::current_origin_channel()
                    .unwrap_or_else(|| "runtime".to_string()),
                authority: Some(authority),
            },
            &invoker,
        )
        .await
        .map_err(|error| error.to_string())?;
    serde_json::to_string(&execution)
        .map_err(|error| format!("serialize CapSpec execution result: {error}"))
}

fn execution_authority(
    request: &ToolDispatchRequest<'_>,
    kernel: &std::sync::Arc<dyn crate::kernel_handle::KernelHandle>,
    required_tools: &[String],
) -> Result<CapabilityExecutionAuthority, String> {
    let mut allowed_tools = Vec::with_capacity(required_tools.len());
    for tool in required_tools {
        let allowed_by_manifest = request
            .allowed_tools
            .is_none_or(|allowed| allowed.iter().any(|candidate| candidate == tool));
        if !allowed_by_manifest {
            return Err(format!(
                "caller allowed_tools deny primitive tool '{tool}' required by '{}'",
                request.tool_name
            ));
        }
        if kernel.tool_is_blocked_for_agent(request.caller_agent_id, tool) {
            return Err(format!(
                "caller tool_blocklist denies primitive tool '{tool}' required by '{}'",
                request.tool_name
            ));
        }
        allowed_tools.push(tool.clone());
    }
    allowed_tools.sort();
    allowed_tools.dedup();
    let mut allowed_env_vars = request.allowed_env_vars.map(<[_]>::to_vec);
    if let Some(values) = &mut allowed_env_vars {
        values.sort();
        values.dedup();
    }
    Ok(CapabilityExecutionAuthority {
        allowed_tools,
        allowed_env_vars,
        exec_policy: request.exec_policy.cloned(),
        subagent_depth: crate::tool_runner::current_agent_lineage_depth(),
    })
}

struct ToolRunnerCapabilityInvoker<'request, 'context> {
    request: &'request ToolDispatchRequest<'context>,
}

#[async_trait]
impl CapabilityToolInvoker for ToolRunnerCapabilityInvoker<'_, '_> {
    async fn invoke(&self, invocation: CapabilityInvocation) -> CapabilityInvocationResult {
        let result = crate::tool_runner::execute_tool(
            &invocation.tool_use_id,
            &invocation.tool_name,
            &invocation.input,
            self.request.kernel,
            self.request.allowed_tools,
            self.request.caller_agent_id,
            self.request.skill_registry,
            self.request.mcp_connections,
            self.request.web_ctx,
            self.request.browser_ctx,
            self.request.allowed_env_vars,
            self.request.workspace_root,
            self.request.media_engine,
            self.request.exec_policy,
            self.request.tts_engine,
            self.request.docker_config,
            self.request.process_manager,
        )
        .await;
        let result = crate::capspec_tool_result::normalize_capspec_tool_result(
            &invocation.tool_name,
            result,
        );
        CapabilityInvocationResult::from_tool_result(result)
    }

    fn reviewed_effect(&self, tool_name: &str) -> Effect {
        captain_capspec::reviewed_effect(tool_name)
    }
}
