//! Authority-preserving ToolRunner bridge for resumed CapSpec runs.

use super::CaptainKernel;
use async_trait::async_trait;
use captain_capspec::{
    CapabilityExecutionAuthority, CapabilityInvocation, CapabilityInvocationResult,
    CapabilityResumeContext, CapabilityToolInvoker, Effect,
};
use captain_runtime::kernel_handle::KernelHandle;
use captain_types::agent::{AgentEntry, AgentId, AgentMode, AgentState};
use captain_types::config::ExecPolicy;
use std::collections::BTreeSet;
use std::path::PathBuf;
use std::sync::Arc;

pub(super) struct KernelCapabilityResumeInvoker {
    kernel: Arc<CaptainKernel>,
    kernel_handle: Arc<dyn KernelHandle>,
    caller_id: AgentId,
    caller_id_text: String,
    workspace: Option<PathBuf>,
    origin: String,
    authority: CapabilityExecutionAuthority,
    required_tools: BTreeSet<String>,
}

impl KernelCapabilityResumeInvoker {
    pub(super) fn prepare(
        kernel: Arc<CaptainKernel>,
        context: CapabilityResumeContext,
    ) -> Result<Self, String> {
        let caller_id_text = context.execution.caller_agent_id.ok_or_else(|| {
            "CapSpec run has no caller identity; only mark_failed is safe".to_string()
        })?;
        let caller_id = caller_id_text.parse::<AgentId>().map_err(|_| {
            "CapSpec run caller identity is invalid; only mark_failed is safe".to_string()
        })?;
        let authority = context.execution.authority.ok_or_else(|| {
            "CapSpec run predates the resumable authority snapshot; only mark_failed is safe"
                .to_string()
        })?;
        let entry = running_entry(&kernel, caller_id)?;
        for tool in &context.required_tools {
            if !authority
                .allowed_tools
                .iter()
                .any(|allowed| allowed == tool)
            {
                return Err(format!(
                    "CapSpec run authority snapshot denies required tool '{tool}'"
                ));
            }
            if !current_tool_allowed(&kernel, &entry, tool) {
                return Err(format!(
                    "current caller authority revoked required tool '{tool}'; only mark_failed is safe"
                ));
            }
        }
        let kernel_handle: Arc<dyn KernelHandle> = kernel.clone();
        Ok(Self {
            kernel,
            kernel_handle,
            caller_id,
            caller_id_text,
            workspace: context.execution.workspace.map(PathBuf::from),
            origin: context.execution.origin,
            authority,
            required_tools: context.required_tools.into_iter().collect(),
        })
    }

    fn current_entry(&self) -> Result<AgentEntry, String> {
        running_entry(&self.kernel, self.caller_id)
    }
}

#[async_trait]
impl CapabilityToolInvoker for KernelCapabilityResumeInvoker {
    async fn invoke(&self, invocation: CapabilityInvocation) -> CapabilityInvocationResult {
        let entry = match self.current_entry() {
            Ok(entry) => entry,
            Err(error) => return CapabilityInvocationResult::error(error),
        };
        if !self.required_tools.contains(&invocation.tool_name)
            || !self
                .authority
                .allowed_tools
                .iter()
                .any(|tool| tool == &invocation.tool_name)
        {
            return CapabilityInvocationResult::error(format!(
                "pinned CapSpec authority denies '{}'",
                invocation.tool_name
            ));
        }
        if !current_tool_allowed(&self.kernel, &entry, &invocation.tool_name) {
            return CapabilityInvocationResult::error(format!(
                "current caller authority revoked '{}'",
                invocation.tool_name
            ));
        }

        let allowed_tools = vec![invocation.tool_name.clone()];
        let current_env = manifest_allowed_env(&entry);
        let allowed_env = intersect_optional_lists(
            self.authority.allowed_env_vars.as_deref(),
            current_env.as_deref(),
        );
        let exec_policy = intersect_exec_policies(
            self.authority.exec_policy.as_ref(),
            entry.manifest.exec_policy.as_ref(),
        );
        let current_depth =
            super::kernel_agent_runtime::subagent_depth_from_manifest(&entry.manifest);
        let lineage_depth = self
            .authority
            .subagent_depth
            .max(u32::try_from(current_depth).unwrap_or(u32::MAX));
        let skill_snapshot = self
            .kernel
            .skill_registry
            .read()
            .unwrap_or_else(|error| error.into_inner())
            .snapshot();
        let docker_config = self
            .kernel
            .config
            .docker
            .enabled
            .then_some(&self.kernel.config.docker);
        let tts_engine = self
            .kernel
            .tts_engine
            .config_snapshot()
            .enabled
            .then_some(&self.kernel.tts_engine);
        let dispatch = captain_runtime::tool_runner::execute_tool(
            &invocation.tool_use_id,
            &invocation.tool_name,
            &invocation.input,
            Some(&self.kernel_handle),
            Some(&allowed_tools),
            Some(&self.caller_id_text),
            Some(&skill_snapshot),
            Some(self.kernel.mcp_connections.as_ref()),
            Some(&self.kernel.web_ctx),
            Some(&self.kernel.browser_ctx),
            allowed_env.as_deref(),
            self.workspace.as_deref(),
            Some(&self.kernel.media_engine),
            exec_policy.as_ref(),
            tts_engine,
            docker_config,
            Some(self.kernel.process_manager.as_ref()),
        );
        let dispatch =
            captain_runtime::tool_runner::with_origin_channel(Some(self.origin.clone()), dispatch);
        let result =
            captain_runtime::tool_runner::with_agent_lineage_depth(lineage_depth, dispatch).await;
        let result = captain_runtime::capspec_tool_result::normalize_capspec_tool_result(
            &invocation.tool_name,
            result,
        );
        CapabilityInvocationResult::from_tool_result(result)
    }

    fn reviewed_effect(&self, tool_name: &str) -> Effect {
        captain_capspec::reviewed_effect(tool_name)
    }
}

fn running_entry(kernel: &CaptainKernel, caller_id: AgentId) -> Result<AgentEntry, String> {
    let entry = kernel
        .registry
        .get(caller_id)
        .ok_or_else(|| "CapSpec caller no longer exists; only mark_failed is safe".to_string())?;
    if entry.state != AgentState::Running {
        return Err(format!(
            "CapSpec caller is {:?}; resume requires a running caller",
            entry.state
        ));
    }
    Ok(entry)
}

fn current_tool_allowed(kernel: &CaptainKernel, entry: &AgentEntry, tool: &str) -> bool {
    if entry.mode == AgentMode::Observe
        || (entry.mode == AgentMode::Assist
            && captain_capspec::reviewed_effect(tool) != Effect::Read)
    {
        return false;
    }
    let allowlist = &entry.manifest.tool_allowlist;
    if !allowlist.is_empty()
        && !allowlist.iter().any(|candidate| candidate == "*")
        && !allowlist.iter().any(|candidate| candidate == tool)
    {
        return false;
    }
    let declared = &entry.manifest.capabilities.tools;
    if allowlist.is_empty()
        && !declared.is_empty()
        && !declared.iter().any(|candidate| candidate == "*")
        && !declared.iter().any(|candidate| candidate == tool)
    {
        return false;
    }
    !kernel.tool_is_blocked_for_agent(Some(&entry.id.to_string()), tool)
}

fn manifest_allowed_env(entry: &AgentEntry) -> Option<Vec<String>> {
    let mut values = entry
        .manifest
        .metadata
        .get("hand_allowed_env")
        .and_then(serde_json::Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(serde_json::Value::as_str)
                .map(str::to_string)
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    values.sort();
    values.dedup();
    (!values.is_empty()).then_some(values)
}

fn intersect_optional_lists(
    original: Option<&[String]>,
    current: Option<&[String]>,
) -> Option<Vec<String>> {
    match (original, current) {
        (None, None) => None,
        (Some(values), None) | (None, Some(values)) => Some(values.to_vec()),
        (Some(original), Some(current)) => Some(
            original
                .iter()
                .filter(|value| current.contains(value))
                .cloned()
                .collect(),
        ),
    }
}

fn intersect_exec_policies(
    original: Option<&ExecPolicy>,
    current: Option<&ExecPolicy>,
) -> Option<ExecPolicy> {
    match (original, current) {
        (None, None) => None,
        (Some(policy), None) | (None, Some(policy)) => Some(policy.clone()),
        (Some(original), Some(current)) => Some(original.intersect(current)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use captain_types::config::{CriticalMode, ExecSecurityMode};

    #[test]
    fn authority_lists_only_shrink() {
        assert_eq!(
            intersect_optional_lists(
                Some(&["PATH".to_string(), "HOME".to_string()]),
                Some(&["PATH".to_string(), "TOKEN".to_string()]),
            ),
            Some(vec!["PATH".to_string()])
        );
        assert_eq!(
            intersect_optional_lists(Some(&["PATH".to_string()]), None),
            Some(vec!["PATH".to_string()])
        );
    }

    #[test]
    fn resumed_exec_policy_uses_the_stricter_contract() {
        let mut original = ExecPolicy {
            mode: ExecSecurityMode::Full,
            timeout_secs: 60,
            max_output_bytes: 2000,
            no_output_timeout_secs: 0,
            critical_mode: CriticalMode::Open,
            ..ExecPolicy::default()
        };
        original.blocked_commands = vec!["old-block".to_string()];
        let current = ExecPolicy {
            profile: captain_types::config::ExecutionProfile::RemoteOperator,
            mode: ExecSecurityMode::Allowlist,
            safe_bins: vec!["echo".to_string()],
            allowed_commands: vec!["cargo test".to_string()],
            blocked_commands: vec!["new-block".to_string()],
            timeout_secs: 20,
            max_output_bytes: 1000,
            no_output_timeout_secs: 10,
            critical_mode: CriticalMode::Safe,
        };
        let merged = intersect_exec_policies(Some(&original), Some(&current)).unwrap();
        assert_eq!(
            merged.profile,
            captain_types::config::ExecutionProfile::RemoteOperator
        );
        assert_eq!(merged.mode, ExecSecurityMode::Allowlist);
        assert_eq!(merged.allowed_commands, ["cargo test"]);
        assert_eq!(merged.timeout_secs, 20);
        assert_eq!(merged.no_output_timeout_secs, 10);
        assert_eq!(merged.max_output_bytes, 1000);
        assert_eq!(merged.critical_mode, CriticalMode::Safe);
        assert_eq!(merged.blocked_commands, ["new-block", "old-block"]);
    }
}
