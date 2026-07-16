use crate::error::KernelError;
use captain_runtime::agent_loop::{AgentLoopResult, ToolCallRecord};
use captain_runtime::audit::AuditAction;
use captain_runtime::learning_bus::{emit, LearningSignal};
use captain_runtime::mcp::McpConnection;
use captain_runtime::outcome_detector::UserMessageKind;
use captain_types::agent::{AgentEntry, AgentId, AgentState};
use captain_types::config::MemoryBackend;
use captain_types::message::TokenUsage;
use rusqlite::Connection;
use std::sync::Arc;
use tracing::warn;

use super::kernel_memory_bridge::{learning_workflow_outcome, mirror_to_mempalace};
use super::CaptainKernel;

const SEND_MESSAGE_SOURCE: &str = "kernel.send_message_full";

impl CaptainKernel {
    pub(super) fn emit_user_message_learning_hint(
        &self,
        agent_id: AgentId,
        message: &str,
    ) -> Option<&'static str> {
        captain_runtime::outcome_detector::classify_user_message(message).map(|kind| {
            let (signal, hint) = user_message_learning_signal(agent_id, message, kind);
            let _ = emit(signal);
            hint
        })
    }

    pub(super) fn record_agent_turn_success(
        &self,
        agent_id: AgentId,
        entry: &AgentEntry,
        message: &str,
        channel_type: Option<String>,
        regex_hint: Option<&'static str>,
        result: &AgentLoopResult,
    ) {
        let semantic_memory_opt_out =
            captain_runtime::outcome_detector::memory_write_opt_out(message);
        self.record_successful_turn_state(agent_id, result);
        if !semantic_memory_opt_out {
            self.emit_successful_conversation_signal(
                agent_id,
                message,
                &result.response,
                channel_type,
                regex_hint,
            );
        }
        self.spawn_agent_turn_memory_job(agent_id, entry, message, result, semantic_memory_opt_out);
        self.record_successful_turn_audit(agent_id, result);

        if !semantic_memory_opt_out {
            let _ = emit(workflow_success_signal(agent_id, result));
        }
    }

    fn record_successful_turn_state(&self, agent_id: AgentId, result: &AgentLoopResult) {
        self.scheduler.record_usage(agent_id, &result.total_usage);

        let _ = self.registry.set_state(agent_id, AgentState::Running);
    }

    fn emit_successful_conversation_signal(
        &self,
        agent_id: AgentId,
        message: &str,
        response: &str,
        channel_type: Option<String>,
        regex_hint: Option<&'static str>,
    ) {
        let _ = emit(conversation_turn_signal(
            agent_id,
            message,
            response,
            channel_type,
            regex_hint,
        ));
    }

    fn spawn_agent_turn_memory_job(
        &self,
        agent_id: AgentId,
        entry: &AgentEntry,
        message: &str,
        result: &AgentLoopResult,
        semantic_memory_opt_out: bool,
    ) {
        let job = AgentTurnMemoryJob {
            graph: self.graph_memory.clone(),
            agent_name: entry.manifest.name.clone(),
            agent_id: agent_id.to_string(),
            user_msg: message.to_string(),
            assistant_msg: result.response.clone(),
            tool_calls: result.tool_calls.clone(),
            usage: result.total_usage,
            cost_usd: result.cost_usd,
            memory_backend: self.config.memory.backend,
            mcp_connections: Arc::clone(&self.mcp_connections),
            memory_writes_conn: self.memory.usage_conn(),
            semantic_memory_opt_out,
        };
        tokio::spawn(job.run());
    }

    fn record_successful_turn_audit(&self, agent_id: AgentId, result: &AgentLoopResult) {
        self.audit_log.record(
            agent_id.to_string(),
            AuditAction::AgentMessage,
            format!(
                "tokens_in={}, tokens_out={}",
                result.total_usage.input_tokens, result.total_usage.output_tokens
            ),
            "ok",
        );
    }

    pub(super) fn record_agent_turn_failure(
        &self,
        agent_id: AgentId,
        entry: &AgentEntry,
        error: &KernelError,
    ) {
        self.audit_log.record(
            agent_id.to_string(),
            AuditAction::AgentMessage,
            "agent loop failed",
            format!("error: {error}"),
        );

        let graph = self.graph_memory.clone();
        let agent_name = entry.manifest.name.clone();
        let err_msg = format!("{error}");
        let err_for_graph = err_msg.clone();
        tokio::spawn(async move {
            let ets = chrono::Utc::now().timestamp_millis();
            let _ = graph.record_event(
                "_sys::error",
                &format!("error:{}@{}", agent_name, ets),
                vec![
                    ("agent", agent_name.as_str()),
                    ("error", &err_for_graph),
                    ("severity", "critical"),
                ],
                None,
            );
            let _ = graph.save();
        });

        let _ = emit(workflow_failure_signal(agent_id, &err_msg));

        self.supervisor.record_failure();
        warn!(agent_id = %agent_id, error = %error, "Agent loop failed — recoverable failure recorded in supervisor");
    }
}

struct AgentTurnMemoryJob {
    graph: Arc<crate::graph_memory::GraphMemory>,
    agent_name: String,
    agent_id: String,
    user_msg: String,
    assistant_msg: String,
    tool_calls: Vec<ToolCallRecord>,
    usage: TokenUsage,
    cost_usd: Option<f64>,
    memory_backend: MemoryBackend,
    mcp_connections: Arc<tokio::sync::Mutex<Vec<McpConnection>>>,
    memory_writes_conn: Arc<std::sync::Mutex<Connection>>,
    semantic_memory_opt_out: bool,
}

impl AgentTurnMemoryJob {
    async fn run(self) {
        if !self.semantic_memory_opt_out {
            self.store_conversation_turns();
        }

        let agent_entity_id = self.graph.find_entity_by_name("agent", &self.agent_name);
        let timestamp_millis = chrono::Utc::now().timestamp_millis();
        self.record_tool_events(agent_entity_id, timestamp_millis);
        self.record_usage_event(agent_entity_id, timestamp_millis);
        if !self.semantic_memory_opt_out {
            self.record_graph_reflection();
        }
        let _ = self.graph.save();
        if !self.semantic_memory_opt_out {
            self.mirror_to_mempalace_if_configured().await;
        }
    }

    fn store_conversation_turns(&self) {
        let _ = self
            .graph
            .store_turn(&self.agent_name, "user", &self.user_msg);
        let _ = self
            .graph
            .store_turn(&self.agent_name, "assistant", &self.assistant_msg);
    }

    fn record_tool_events(&self, agent_entity_id: Option<u64>, timestamp_millis: i64) {
        for (index, tool_call) in self.tool_calls.iter().enumerate() {
            self.record_one_tool_event(agent_entity_id, timestamp_millis, index, tool_call);
        }
    }

    fn record_one_tool_event(
        &self,
        agent_entity_id: Option<u64>,
        timestamp_millis: i64,
        index: usize,
        tool_call: &ToolCallRecord,
    ) {
        let duration_ms = tool_call.duration_ms.to_string();
        let event_name = tool_event_name(&tool_call.tool_name, timestamp_millis, index);
        let props = vec![
            ("agent", self.agent_name.as_str()),
            ("agent_id", self.agent_id.as_str()),
            ("status", tool_call_status(tool_call)),
            ("duration_ms", duration_ms.as_str()),
            ("input", tool_call.input_summary.as_str()),
            ("output", tool_call.output_summary.as_str()),
            ("tool", tool_call.tool_name.as_str()),
        ];

        if let Ok(event_id) = self
            .graph
            .record_event("_sys::tool_call", &event_name, props, None)
        {
            self.link_tool_event_to_agent(event_id, agent_entity_id, tool_call);
        }
    }

    fn link_tool_event_to_agent(
        &self,
        event_id: u64,
        agent_entity_id: Option<u64>,
        tool_call: &ToolCallRecord,
    ) {
        if let Some(agent_entity_id) = agent_entity_id {
            let desc = format!("{} → {}", tool_call.tool_name, self.agent_name);
            let _ = self
                .graph
                .add_doc_fact(event_id, agent_entity_id, "exécuté_par", &desc);
        }
    }

    fn record_usage_event(&self, agent_entity_id: Option<u64>, timestamp_millis: i64) {
        let cost_str = format_cost_usd(self.cost_usd);
        let input_tokens = self.usage.input_tokens.to_string();
        let output_tokens = self.usage.output_tokens.to_string();
        let tool_count = self.tool_calls.len().to_string();
        let event_name = format!("usage:{}@{}", self.agent_name, timestamp_millis);

        let _ = self.graph.record_event(
            "_sys::usage",
            &event_name,
            vec![
                ("agent", self.agent_name.as_str()),
                ("input_tokens", input_tokens.as_str()),
                ("output_tokens", output_tokens.as_str()),
                ("cost_usd", cost_str.as_str()),
                ("tool_count", tool_count.as_str()),
            ],
            agent_entity_id.map(|id| (id, "consommé_par", self.agent_name.as_str())),
        );
    }

    fn record_graph_reflection(&self) {
        let tools_for_reflect = tool_names_for_reflection(&self.tool_calls);
        let successful = !has_tool_errors(&self.tool_calls);
        let _ = self.graph.reflect(
            &self.agent_name,
            &tools_for_reflect,
            successful,
            self.tool_calls.len() as u32,
        );

        self.graph.update_mood(successful);
        let _ = self.graph.narrate(
            &self.agent_name,
            &self.user_msg,
            &tools_for_reflect,
            successful,
        );
    }

    async fn mirror_to_mempalace_if_configured(&self) {
        if self.memory_backend != MemoryBackend::Mempalace {
            return;
        }

        mirror_to_mempalace(
            &self.mcp_connections,
            Arc::clone(&self.memory_writes_conn),
            &self.agent_name,
            &self.user_msg,
            &self.assistant_msg,
            &self.tool_calls,
        )
        .await;
    }
}

fn tool_event_name(tool_name: &str, timestamp_millis: i64, index: usize) -> String {
    format!("{tool_name}@{timestamp_millis}_{index}")
}

fn tool_call_status(tool_call: &ToolCallRecord) -> &'static str {
    if tool_call.is_error {
        "error"
    } else {
        "ok"
    }
}

fn format_cost_usd(cost_usd: Option<f64>) -> String {
    cost_usd
        .map(|cost| format!("{cost:.6}"))
        .unwrap_or_default()
}

fn tool_names_for_reflection(tool_calls: &[ToolCallRecord]) -> Vec<&str> {
    tool_calls
        .iter()
        .map(|tool_call| tool_call.tool_name.as_str())
        .collect()
}

fn has_tool_errors(tool_calls: &[ToolCallRecord]) -> bool {
    tool_calls.iter().any(|tool_call| tool_call.is_error)
}

fn user_message_learning_signal(
    agent_id: AgentId,
    message: &str,
    kind: UserMessageKind,
) -> (LearningSignal, &'static str) {
    let agent_id = agent_id.to_string();
    let user_msg = message.to_string();
    match kind {
        UserMessageKind::Correction => (
            LearningSignal::UserCorrection {
                agent_id,
                user_msg,
                source: SEND_MESSAGE_SOURCE.to_string(),
            },
            "correction",
        ),
        UserMessageKind::Satisfaction => (
            LearningSignal::UserSatisfaction {
                agent_id,
                user_msg,
                source: SEND_MESSAGE_SOURCE.to_string(),
            },
            "satisfaction",
        ),
        UserMessageKind::ExplicitRemember => (
            LearningSignal::ExplicitRemember {
                agent_id,
                user_msg,
                source: SEND_MESSAGE_SOURCE.to_string(),
            },
            "explicit_remember",
        ),
    }
}

fn conversation_turn_signal(
    agent_id: AgentId,
    user_msg: &str,
    agent_response: &str,
    channel_type: Option<String>,
    regex_hint: Option<&'static str>,
) -> LearningSignal {
    LearningSignal::ConversationTurn {
        agent_id: agent_id.to_string(),
        user_msg: user_msg.to_string(),
        agent_response: agent_response.to_string(),
        channel: channel_type,
        regex_hint: regex_hint.map(String::from),
        source: SEND_MESSAGE_SOURCE.to_string(),
    }
}

fn workflow_success_signal(agent_id: AgentId, result: &AgentLoopResult) -> LearningSignal {
    LearningSignal::WorkflowRunComplete {
        agent_id: agent_id.to_string(),
        outcome: learning_workflow_outcome("success", &result.tool_calls),
        tool_calls: result.tool_calls.len() as u32,
        source: SEND_MESSAGE_SOURCE.to_string(),
    }
}

fn workflow_failure_signal(agent_id: AgentId, err_msg: &str) -> LearningSignal {
    LearningSignal::WorkflowRunComplete {
        agent_id: agent_id.to_string(),
        outcome: format!("failure: {err_msg}"),
        tool_calls: 0,
        source: SEND_MESSAGE_SOURCE.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::{
        conversation_turn_signal, format_cost_usd, has_tool_errors, tool_call_status,
        tool_event_name, tool_names_for_reflection, user_message_learning_signal,
        workflow_failure_signal, workflow_success_signal,
    };
    use crate::kernel::CaptainKernel;
    use async_trait::async_trait;
    use captain_runtime::agent_loop::{AgentLoopResult, ToolCallRecord};
    use captain_runtime::learning_bus::LearningSignal;
    use captain_runtime::llm_driver::{CompletionRequest, CompletionResponse, LlmDriver, LlmError};
    use captain_runtime::outcome_detector::UserMessageKind;
    use captain_types::agent::AgentId;
    use captain_types::config::{AssistantConfig, DefaultModelConfig, KernelConfig};
    use captain_types::message::{ContentBlock, ReplyDirectives, StopReason, TokenUsage};
    use std::sync::Arc;

    struct StaticDriver;

    #[async_trait]
    impl LlmDriver for StaticDriver {
        async fn complete(
            &self,
            _request: CompletionRequest,
        ) -> Result<CompletionResponse, LlmError> {
            Ok(CompletionResponse {
                content: vec![ContentBlock::Text {
                    text: "agent ok".to_string(),
                    provider_metadata: None,
                }],
                stop_reason: StopReason::EndTurn,
                tool_calls: Vec::new(),
                usage: TokenUsage {
                    input_tokens: 1,
                    output_tokens: 1,
                    ..Default::default()
                },
            })
        }
    }

    fn loop_result(response: &str) -> AgentLoopResult {
        AgentLoopResult {
            response: response.to_string(),
            total_usage: TokenUsage {
                input_tokens: 3,
                output_tokens: 4,
                ..Default::default()
            },
            iterations: 1,
            cost_usd: None,
            silent: false,
            directives: ReplyDirectives::default(),
            tool_calls: Vec::new(),
        }
    }

    fn tool_record(tool_name: &str, is_error: bool) -> ToolCallRecord {
        ToolCallRecord {
            tool_name: tool_name.to_string(),
            reason: "Use this tool to continue the current task.".to_string(),
            is_error,
            duration_ms: 42,
            input_summary: "input".to_string(),
            output_summary: "output".to_string(),
        }
    }

    fn boot_test_kernel() -> (tempfile::TempDir, Arc<CaptainKernel>) {
        let tmp = tempfile::tempdir().unwrap();
        let home_dir = tmp.path().join("agent-turn-observability");
        let config = KernelConfig {
            home_dir: home_dir.clone(),
            data_dir: home_dir.join("data"),
            default_model: DefaultModelConfig {
                provider: "static-test".to_string(),
                model: "static-test-model".to_string(),
                api_key_env: String::new(),
                base_url: None,
            },
            assistant: AssistantConfig {
                onboarding_completed: true,
                ..AssistantConfig::default()
            },
            ..KernelConfig::default()
        };
        let mut kernel = Arc::new(CaptainKernel::boot_with_config(config).expect("kernel boot"));
        Arc::get_mut(&mut kernel)
            .expect("kernel has no shared references yet")
            .default_driver = Arc::new(StaticDriver);
        kernel.set_self_handle();
        (tmp, kernel)
    }

    fn principal_agent_id(kernel: &CaptainKernel) -> AgentId {
        kernel
            .registry
            .list()
            .into_iter()
            .find(|entry| entry.name == "captain")
            .or_else(|| kernel.registry.list().into_iter().next())
            .expect("kernel should boot with at least one agent")
            .id
    }

    #[test]
    fn user_message_learning_signal_preserves_source_and_hint() {
        let agent_id = AgentId::new();
        let (signal, hint) = user_message_learning_signal(
            agent_id,
            "remember that x",
            UserMessageKind::ExplicitRemember,
        );

        assert_eq!(hint, "explicit_remember");
        assert_eq!(signal.source(), "kernel.send_message_full");
        assert_eq!(signal.agent_id(), agent_id.to_string());
        assert_eq!(signal.kind(), "explicit_remember");
    }

    #[test]
    fn conversation_turn_signal_keeps_channel_and_regex_hint() {
        let agent_id = AgentId::new();
        let signal = conversation_turn_signal(
            agent_id,
            "user",
            "assistant",
            Some("telegram".to_string()),
            Some("correction"),
        );

        match signal {
            LearningSignal::ConversationTurn {
                channel,
                regex_hint,
                source,
                ..
            } => {
                assert_eq!(channel.as_deref(), Some("telegram"));
                assert_eq!(regex_hint.as_deref(), Some("correction"));
                assert_eq!(source, "kernel.send_message_full");
            }
            other => panic!("unexpected signal: {other:?}"),
        }
    }

    #[test]
    fn workflow_signals_encode_success_and_failure() {
        let agent_id = AgentId::new();
        let success = workflow_success_signal(agent_id, &loop_result("ok"));
        let failure = workflow_failure_signal(agent_id, "boom");

        assert_eq!(success.kind(), "workflow_run_complete");
        match success {
            LearningSignal::WorkflowRunComplete {
                outcome,
                tool_calls,
                ..
            } => {
                assert_eq!(outcome, "success");
                assert_eq!(tool_calls, 0);
            }
            other => panic!("unexpected signal: {other:?}"),
        }
        match failure {
            LearningSignal::WorkflowRunComplete { outcome, .. } => {
                assert_eq!(outcome, "failure: boom");
            }
            other => panic!("unexpected signal: {other:?}"),
        }
    }

    #[test]
    fn tool_event_helpers_preserve_graph_contract() {
        let ok_record = tool_record("web_search", false);
        let err_record = tool_record("shell_exec", true);

        assert_eq!(tool_event_name("web_search", 1234, 2), "web_search@1234_2");
        assert_eq!(tool_call_status(&ok_record), "ok");
        assert_eq!(tool_call_status(&err_record), "error");
    }

    #[test]
    fn usage_and_reflection_helpers_keep_existing_format() {
        let records = vec![
            tool_record("capability_search", false),
            tool_record("shell", true),
        ];
        let tools = tool_names_for_reflection(&records);

        assert_eq!(format_cost_usd(Some(0.1234567)), "0.123457");
        assert_eq!(format_cost_usd(None), "");
        assert_eq!(tools, vec!["capability_search", "shell"]);
        assert!(has_tool_errors(&records));
    }

    #[tokio::test]
    async fn send_message_full_static_driver_records_usage_and_state() {
        let (_tmp, kernel) = boot_test_kernel();
        let agent_id = principal_agent_id(&kernel);

        let result = kernel
            .send_message_full(
                agent_id,
                "say ok",
                None,
                None,
                None,
                None,
                Some("system".to_string()),
            )
            .await
            .expect("static send_message_full should succeed");

        assert_eq!(result.response, "agent ok");
        let (tokens, _) = kernel
            .scheduler
            .get_usage(agent_id)
            .expect("scheduler usage should be recorded");
        assert_eq!(tokens, 2);
        assert!(matches!(
            kernel.registry.get(agent_id).map(|entry| entry.state),
            Some(captain_types::agent::AgentState::Running)
        ));

        kernel.shutdown();
    }
}
