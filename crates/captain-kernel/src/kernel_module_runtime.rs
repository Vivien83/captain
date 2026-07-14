use crate::error::{KernelError, KernelResult};
use captain_runtime::agent_loop::AgentLoopResult;
use captain_runtime::kernel_handle::KernelHandle;
use captain_runtime::python_runtime::{self, PythonConfig};
use captain_runtime::sandbox::SandboxConfig;
use captain_types::agent::{AgentEntry, AgentId};
use captain_types::error::CaptainError;
use std::path::Path;
use std::sync::Arc;
use tracing::info;

use super::kernel_model_support::manifest_to_capabilities;
use super::CaptainKernel;

impl CaptainKernel {
    /// Execute a WASM module agent.
    ///
    /// Loads the `.wasm` or `.wat` file, maps manifest capabilities into
    /// `SandboxConfig`, and runs through the `WasmSandbox` engine.
    pub(super) async fn execute_wasm_agent(
        &self,
        entry: &AgentEntry,
        message: &str,
        kernel_handle: Option<Arc<dyn KernelHandle>>,
    ) -> KernelResult<AgentLoopResult> {
        let module_path = entry.manifest.module.strip_prefix("wasm:").unwrap_or("");
        let wasm_path = self.resolve_module_path(module_path);

        info!(agent = %entry.name, path = %wasm_path.display(), "Executing WASM agent");

        let wasm_bytes = std::fs::read(&wasm_path).map_err(|e| {
            KernelError::Captain(CaptainError::Internal(format!(
                "Failed to read WASM module '{}': {e}",
                wasm_path.display()
            )))
        })?;

        let sandbox_config = wasm_sandbox_config(entry);
        let input = wasm_agent_input(entry, message);

        let result = self
            .wasm_sandbox
            .execute(
                &wasm_bytes,
                input,
                sandbox_config,
                kernel_handle,
                &entry.id.to_string(),
            )
            .await
            .map_err(|e| {
                KernelError::Captain(CaptainError::Internal(format!(
                    "WASM execution failed: {e}"
                )))
            })?;

        info!(
            agent = %entry.name,
            fuel_consumed = result.fuel_consumed,
            "WASM agent execution complete"
        );

        Ok(static_agent_result(wasm_output_response(&result.output)))
    }

    /// Execute a Python script agent.
    ///
    /// Delegates to `python_runtime::run_python_agent()` via subprocess.
    pub(super) async fn execute_python_agent(
        &self,
        entry: &AgentEntry,
        agent_id: AgentId,
        message: &str,
    ) -> KernelResult<AgentLoopResult> {
        let script_path = entry.manifest.module.strip_prefix("python:").unwrap_or("");
        let resolved_path = self.resolve_module_path(script_path);

        info!(agent = %entry.name, path = %resolved_path.display(), "Executing Python agent");

        let config = python_agent_config(&resolved_path, entry.manifest.resources.max_cpu_time_ms);
        let context = python_agent_context(entry);

        let result = python_runtime::run_python_agent(
            &resolved_path.to_string_lossy(),
            &agent_id.to_string(),
            message,
            &context,
            &config,
        )
        .await
        .map_err(|e| {
            KernelError::Captain(CaptainError::Internal(format!(
                "Python execution failed: {e}"
            )))
        })?;

        info!(agent = %entry.name, "Python agent execution complete");

        Ok(static_agent_result(result.response))
    }
}

fn wasm_sandbox_config(entry: &AgentEntry) -> SandboxConfig {
    SandboxConfig {
        fuel_limit: entry.manifest.resources.max_cpu_time_ms * 100_000,
        max_memory_bytes: entry.manifest.resources.max_memory_bytes as usize,
        capabilities: manifest_to_capabilities(&entry.manifest),
        timeout_secs: Some(30),
    }
}

fn wasm_agent_input(entry: &AgentEntry, message: &str) -> serde_json::Value {
    serde_json::json!({
        "message": message,
        "agent_id": entry.id.to_string(),
        "agent_name": entry.name,
    })
}

fn wasm_output_response(output: &serde_json::Value) -> String {
    output
        .get("response")
        .and_then(|v| v.as_str())
        .or_else(|| output.get("text").and_then(|v| v.as_str()))
        .or_else(|| output.as_str())
        .map(|s| s.to_string())
        .unwrap_or_else(|| serde_json::to_string(output).unwrap_or_default())
}

fn python_agent_config(resolved_path: &Path, max_cpu_time_ms: u64) -> PythonConfig {
    PythonConfig {
        timeout_secs: (max_cpu_time_ms / 1000).max(30),
        working_dir: Some(
            resolved_path
                .parent()
                .unwrap_or(Path::new("."))
                .to_string_lossy()
                .to_string(),
        ),
        ..PythonConfig::default()
    }
}

fn python_agent_context(entry: &AgentEntry) -> serde_json::Value {
    serde_json::json!({
        "agent_name": entry.name,
        "system_prompt": entry.manifest.model.system_prompt,
    })
}

fn static_agent_result(response: String) -> AgentLoopResult {
    AgentLoopResult {
        response,
        total_usage: captain_types::message::TokenUsage {
            input_tokens: 0,
            output_tokens: 0,
            ..Default::default()
        },
        cost_usd: None,
        iterations: 1,
        silent: false,
        directives: Default::default(),
        tool_calls: vec![],
    }
}

#[cfg(test)]
mod tests {
    use super::{python_agent_config, wasm_output_response};
    use std::path::PathBuf;

    #[test]
    fn wasm_output_response_prefers_response_then_text_then_json() {
        assert_eq!(
            wasm_output_response(&serde_json::json!({"response": "primary", "text": "fallback"})),
            "primary"
        );
        assert_eq!(
            wasm_output_response(&serde_json::json!({"text": "fallback"})),
            "fallback"
        );
        assert_eq!(wasm_output_response(&serde_json::json!("raw")), "raw");
        assert_eq!(
            wasm_output_response(&serde_json::json!({"nested": true})),
            r#"{"nested":true}"#
        );
    }

    #[test]
    fn python_agent_config_clamps_timeout_and_uses_script_parent() {
        let script = PathBuf::from("/tmp/captain-agent/main.py");
        let config = python_agent_config(&script, 5_000);
        assert_eq!(config.timeout_secs, 30);
        assert_eq!(config.working_dir.as_deref(), Some("/tmp/captain-agent"));

        let longer = python_agent_config(&script, 90_000);
        assert_eq!(longer.timeout_secs, 90);
    }
}
