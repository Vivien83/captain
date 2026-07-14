use crate::error::{KernelError, KernelResult};
use captain_memory::session::Session;
use captain_runtime::agent_loop::{
    run_agent_loop, run_agent_loop_streaming, AgentLoopResult, PhaseCallback,
};
use captain_runtime::kernel_handle::KernelHandle;
use captain_runtime::llm_driver::{LlmDriver, StreamEvent};
use captain_skills::registry::SkillRegistry;
use captain_types::agent::{AgentId, AgentManifest};
use captain_types::media::LinkConfig;
use captain_types::message::ContentBlock;
use captain_types::tool::ToolDefinition;
use std::collections::HashSet;
use std::sync::Arc;
use tracing::warn;

use super::kernel_agent_runtime::STREAMING_USER_INPUT_BUFFER;
use super::kernel_llm_runtime::{LlmPreLoopCompactionDecision, LlmPreLoopCompactionStage};
use super::kernel_running_tasks::RunningTaskCleanup;
use super::CaptainKernel;

type StreamingLlmLoopResult = (
    tokio::sync::mpsc::Receiver<StreamEvent>,
    tokio::task::JoinHandle<KernelResult<AgentLoopResult>>,
    tokio::sync::mpsc::Sender<String>,
);

struct StreamingLoopChannels {
    tx: tokio::sync::mpsc::Sender<StreamEvent>,
    rx: tokio::sync::mpsc::Receiver<StreamEvent>,
    user_input_tx: tokio::sync::mpsc::Sender<String>,
    user_input_rx: Arc<tokio::sync::Mutex<tokio::sync::mpsc::Receiver<String>>>,
}

struct StreamingLlmTask {
    request: StreamingLlmLoopRequest,
    tx: tokio::sync::mpsc::Sender<StreamEvent>,
    user_input_rx: Arc<tokio::sync::Mutex<tokio::sync::mpsc::Receiver<String>>>,
    message_with_links: String,
    run_id: uuid::Uuid,
}

struct StreamingTaskParts {
    agent_id: AgentId,
    session: Session,
    manifest: AgentManifest,
    tools: Vec<ToolDefinition>,
    driver: Arc<dyn LlmDriver>,
    kernel_handle: Option<Arc<dyn KernelHandle>>,
    lean_direct: bool,
    effective_ctx_window: usize,
    ctx_window: Option<usize>,
    pre_loop_compaction: LlmPreLoopCompactionDecision,
    content_blocks: Option<Vec<ContentBlock>>,
    channel_type: Option<String>,
    tx: tokio::sync::mpsc::Sender<StreamEvent>,
    user_input_rx: Arc<tokio::sync::Mutex<tokio::sync::mpsc::Receiver<String>>>,
    message_with_links: String,
    run_id: uuid::Uuid,
}

pub(super) struct NonStreamingLlmLoopRequest<'a> {
    pub agent_id: AgentId,
    pub message: &'a str,
    pub session: &'a mut Session,
    pub manifest: &'a mut AgentManifest,
    pub tools: &'a [ToolDefinition],
    pub driver: Arc<dyn LlmDriver>,
    pub kernel_handle: Option<Arc<dyn KernelHandle>>,
    pub lean_direct: bool,
    pub ctx_window: Option<usize>,
    pub content_blocks: Option<Vec<ContentBlock>>,
    pub channel_type: Option<String>,
}

pub(super) struct StreamingLlmLoopRequest {
    pub agent_id: AgentId,
    pub message: String,
    pub session: Session,
    pub manifest: AgentManifest,
    pub tools: Vec<ToolDefinition>,
    pub driver: Arc<dyn LlmDriver>,
    pub kernel_handle: Option<Arc<dyn KernelHandle>>,
    pub lean_direct: bool,
    pub effective_ctx_window: usize,
    pub ctx_window: Option<usize>,
    pub pre_loop_compaction: LlmPreLoopCompactionDecision,
    pub content_blocks: Option<Vec<ContentBlock>>,
    pub channel_type: Option<String>,
}

impl CaptainKernel {
    pub(super) async fn run_non_streaming_llm_loop(
        &self,
        request: NonStreamingLlmLoopRequest<'_>,
    ) -> KernelResult<AgentLoopResult> {
        let NonStreamingLlmLoopRequest {
            agent_id,
            message,
            session,
            manifest,
            tools,
            driver,
            kernel_handle,
            lean_direct,
            ctx_window,
            content_blocks,
            channel_type,
        } = request;

        let skill_snapshot =
            self.prepare_llm_skill_snapshot(agent_id, manifest, lean_direct, false);
        let message_with_links = llm_message_with_link_context(message, &self.config.links);
        let tts_engine = if self.tts_engine.config_snapshot().enabled {
            Some(&self.tts_engine)
        } else {
            None
        };
        let docker_config = if self.config.docker.enabled {
            Some(&self.config.docker)
        } else {
            None
        };

        run_agent_loop(
            manifest,
            &message_with_links,
            session,
            &self.memory,
            driver,
            tools,
            kernel_handle,
            Some(&skill_snapshot),
            Some(&self.mcp_connections),
            Some(&self.web_ctx),
            Some(&self.browser_ctx),
            self.embedding_driver.as_deref(),
            manifest.workspace.as_deref(),
            None,
            Some(&self.media_engine),
            tts_engine,
            docker_config,
            Some(&self.hooks),
            ctx_window,
            Some(&self.process_manager),
            content_blocks,
            channel_type,
        )
        .await
        .map_err(KernelError::Captain)
    }

    pub(super) fn stream_llm_agent_loop(
        self: &Arc<Self>,
        request: StreamingLlmLoopRequest,
    ) -> KernelResult<StreamingLlmLoopResult> {
        let StreamingLlmLoopRequest {
            agent_id,
            message,
            session,
            manifest,
            tools,
            driver,
            kernel_handle,
            lean_direct,
            effective_ctx_window,
            ctx_window,
            pre_loop_compaction,
            content_blocks,
            channel_type,
        } = request;

        let channels = streaming_loop_channels();
        let message_with_links = llm_message_with_link_context(&message, &self.config.links);
        let run_id = uuid::Uuid::new_v4();
        let handle = self.spawn_streaming_llm_task(StreamingLlmTask {
            request: StreamingLlmLoopRequest {
                agent_id,
                message,
                session,
                manifest,
                tools,
                driver,
                kernel_handle,
                lean_direct,
                effective_ctx_window,
                ctx_window,
                pre_loop_compaction,
                content_blocks,
                channel_type,
            },
            tx: channels.tx,
            user_input_rx: channels.user_input_rx,
            message_with_links,
            run_id,
        });

        self.track_running_task(agent_id, run_id, handle.abort_handle());
        if handle.is_finished() {
            self.clear_running_task(agent_id, run_id);
        }

        Ok((channels.rx, handle, channels.user_input_tx))
    }

    fn spawn_streaming_llm_task(
        self: &Arc<Self>,
        task: StreamingLlmTask,
    ) -> tokio::task::JoinHandle<KernelResult<AgentLoopResult>> {
        let kernel_clone = Arc::clone(self);
        self.spawn_supervised_agent_task(task.request.agent_id, async move {
            run_streaming_llm_task(kernel_clone, task).await
        })
    }

    pub(super) fn prepare_llm_skill_snapshot(
        &self,
        agent_id: AgentId,
        manifest: &mut AgentManifest,
        lean_direct: bool,
        streaming: bool,
    ) -> SkillRegistry {
        let mut skill_snapshot = self
            .skill_registry
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .snapshot();

        if lean_direct {
            return skill_snapshot;
        }

        if let Some(ref workspace) = manifest.workspace {
            let ws_skills = workspace.join("skills");
            if ws_skills.exists() {
                if let Err(e) = skill_snapshot.load_workspace_skills(&ws_skills) {
                    if streaming {
                        warn!(agent_id = %agent_id, "Failed to load workspace skills (streaming): {e}");
                    } else {
                        warn!(agent_id = %agent_id, "Failed to load workspace skills: {e}");
                    }
                }
            }
        }

        let global_names: HashSet<String> = self
            .skill_registry
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .list()
            .into_iter()
            .map(|s| s.manifest.skill.name.clone())
            .collect();
        append_workspace_skill_prompt_context(manifest, &skill_snapshot, &global_names);
        skill_snapshot
    }
}

async fn run_streaming_llm_task(
    kernel_clone: Arc<CaptainKernel>,
    task: StreamingLlmTask,
) -> KernelResult<AgentLoopResult> {
    let mut task = unpack_streaming_task(task);

    let _running_task_cleanup =
        RunningTaskCleanup::new(Arc::clone(&kernel_clone), task.agent_id, task.run_id);
    apply_streaming_pre_loop_compaction(
        &kernel_clone,
        task.agent_id,
        &mut task.session,
        task.effective_ctx_window,
        task.pre_loop_compaction,
    )
    .await;

    let (messages_before, skill_snapshot, phase_cb) =
        prepare_streaming_task_state(&kernel_clone, &mut task);

    let result = execute_streaming_agent_loop(
        &kernel_clone,
        &task.message_with_links,
        &mut task.session,
        &task.manifest,
        &task.tools,
        task.driver,
        task.kernel_handle,
        task.tx,
        &skill_snapshot,
        &phase_cb,
        task.ctx_window,
        task.content_blocks,
        Some(task.user_input_rx),
        task.channel_type.clone(),
    )
    .await;

    // Release the sender held by the phase callback before post-processing so
    // WS/SSE consumers can observe stream closure promptly.
    drop(phase_cb);
    finish_streaming_task_result(
        &kernel_clone,
        task.agent_id,
        &task.session,
        messages_before,
        &task.manifest,
        &task.tools,
        task.ctx_window,
        result,
    )
}

fn unpack_streaming_task(task: StreamingLlmTask) -> StreamingTaskParts {
    let StreamingLlmTask {
        request,
        tx,
        user_input_rx,
        message_with_links,
        run_id,
    } = task;
    let StreamingLlmLoopRequest {
        agent_id,
        session,
        manifest,
        tools,
        driver,
        kernel_handle,
        lean_direct,
        effective_ctx_window,
        ctx_window,
        pre_loop_compaction,
        content_blocks,
        channel_type,
        ..
    } = request;

    StreamingTaskParts {
        agent_id,
        session,
        manifest,
        tools,
        driver,
        kernel_handle,
        lean_direct,
        effective_ctx_window,
        ctx_window,
        pre_loop_compaction,
        content_blocks,
        channel_type,
        tx,
        user_input_rx,
        message_with_links,
        run_id,
    }
}

async fn apply_streaming_pre_loop_compaction(
    kernel: &CaptainKernel,
    agent_id: AgentId,
    session: &mut Session,
    effective_ctx_window: usize,
    pre_loop_compaction: LlmPreLoopCompactionDecision,
) {
    kernel
        .execute_llm_session_compaction_plan(
            agent_id,
            session,
            effective_ctx_window,
            LlmPreLoopCompactionStage::StreamingAuto,
            pre_loop_compaction,
        )
        .await;
}

fn prepare_streaming_task_state(
    kernel: &Arc<CaptainKernel>,
    task: &mut StreamingTaskParts,
) -> (usize, SkillRegistry, PhaseCallback) {
    let messages_before = task.session.messages.len();
    let skill_snapshot = kernel.prepare_llm_skill_snapshot(
        task.agent_id,
        &mut task.manifest,
        task.lean_direct,
        true,
    );
    let phase_cb = kernel.streaming_phase_callback(task.tx.clone());
    (messages_before, skill_snapshot, phase_cb)
}

#[allow(clippy::too_many_arguments)]
async fn execute_streaming_agent_loop(
    kernel: &Arc<CaptainKernel>,
    message_with_links: &str,
    session: &mut Session,
    manifest: &AgentManifest,
    tools: &[ToolDefinition],
    driver: Arc<dyn LlmDriver>,
    kernel_handle: Option<Arc<dyn KernelHandle>>,
    tx: tokio::sync::mpsc::Sender<StreamEvent>,
    skill_snapshot: &SkillRegistry,
    phase_cb: &PhaseCallback,
    ctx_window: Option<usize>,
    content_blocks: Option<Vec<ContentBlock>>,
    user_input_rx: Option<Arc<tokio::sync::Mutex<tokio::sync::mpsc::Receiver<String>>>>,
    channel_type: Option<String>,
) -> Result<AgentLoopResult, captain_types::error::CaptainError> {
    // Tool RAG stays out of this path; visible tools are already filtered by
    // the caller, and broader discovery goes through tool_search.
    run_agent_loop_streaming(
        manifest,
        message_with_links,
        session,
        &kernel.memory,
        driver,
        tools,
        kernel_handle,
        tx,
        Some(skill_snapshot),
        Some(&kernel.mcp_connections),
        Some(&kernel.web_ctx),
        Some(&kernel.browser_ctx),
        kernel.embedding_driver.as_deref(),
        manifest.workspace.as_deref(),
        Some(phase_cb),
        Some(&kernel.media_engine),
        if kernel.tts_engine.config_snapshot().enabled {
            Some(&kernel.tts_engine)
        } else {
            None
        },
        if kernel.config.docker.enabled {
            Some(&kernel.config.docker)
        } else {
            None
        },
        Some(&kernel.hooks),
        ctx_window,
        Some(&kernel.process_manager),
        content_blocks,
        user_input_rx,
        channel_type,
    )
    .await
}

#[allow(clippy::too_many_arguments)]
fn finish_streaming_task_result(
    kernel: &Arc<CaptainKernel>,
    agent_id: AgentId,
    session: &Session,
    messages_before: usize,
    manifest: &AgentManifest,
    tools: &[ToolDefinition],
    ctx_window: Option<usize>,
    result: Result<AgentLoopResult, captain_types::error::CaptainError>,
) -> KernelResult<AgentLoopResult> {
    match result {
        Ok(result) => {
            kernel.finish_streaming_llm_success(
                agent_id,
                &kernel.memory,
                session,
                messages_before,
                manifest,
                tools,
                ctx_window,
                &result,
            );
            Ok(result)
        }
        Err(e) => {
            kernel.record_streaming_llm_failure(agent_id, &e);
            Err(KernelError::Captain(e))
        }
    }
}

fn streaming_loop_channels() -> StreamingLoopChannels {
    let (tx, rx) = tokio::sync::mpsc::channel::<StreamEvent>(64);
    let (user_input_tx, user_input_rx) =
        tokio::sync::mpsc::channel::<String>(STREAMING_USER_INPUT_BUFFER);
    StreamingLoopChannels {
        tx,
        rx,
        user_input_tx,
        user_input_rx: Arc::new(tokio::sync::Mutex::new(user_input_rx)),
    }
}

fn append_workspace_skill_prompt_context(
    manifest: &mut AgentManifest,
    skill_snapshot: &SkillRegistry,
    global_names: &HashSet<String>,
) {
    let ws_skills_list: Vec<_> = skill_snapshot
        .list()
        .into_iter()
        .filter(|s| s.enabled && !global_names.contains(&s.manifest.skill.name))
        .collect();
    if ws_skills_list.is_empty() {
        return;
    }

    let mut addon = String::from("\n\n--- Custom Skills ---\n");
    for s in &ws_skills_list {
        addon.push_str(&format!(
            "- {}: {}\n",
            s.manifest.skill.name, s.manifest.skill.description
        ));
        let skillmd = s.path.join("SKILL.md");
        if let Ok(content) = std::fs::read_to_string(&skillmd) {
            if let Some(body) = skill_markdown_prompt_body(&content) {
                addon.push_str(body.trim());
                addon.push('\n');
            }
        }
    }
    manifest.model.system_prompt.push_str(&addon);
}

fn skill_markdown_prompt_body(content: &str) -> Option<&str> {
    content.find("\n---\n").map(|end| {
        let body = &content[end + 5..];
        let cap = body.len().min(1000);
        &body[..cap]
    })
}

fn llm_message_with_link_context(message: &str, config: &LinkConfig) -> String {
    if let Some(link_ctx) = captain_runtime::link_understanding::build_link_context(message, config)
    {
        format!("{message}{link_ctx}")
    } else {
        message.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn skill_markdown_prompt_body_reads_capped_body_after_frontmatter() {
        let body = format!("{}{}", "x".repeat(1005), "\nignored");
        let content = format!("---\nname: custom\n---\n{body}");
        let extracted = skill_markdown_prompt_body(&content).expect("frontmatter body");

        assert_eq!(extracted.len(), 1000);
        assert!(extracted.chars().all(|c| c == 'x'));
        assert!(skill_markdown_prompt_body("no frontmatter").is_none());
    }

    #[test]
    fn llm_message_with_link_context_appends_detected_urls_only_when_enabled() {
        let message = "Read https://example.com/report";
        let disabled = llm_message_with_link_context(message, &LinkConfig::default());
        assert_eq!(disabled, message);

        let enabled = llm_message_with_link_context(
            message,
            &LinkConfig {
                enabled: true,
                max_links: 1,
                ..LinkConfig::default()
            },
        );
        assert!(enabled.starts_with(message));
        assert!(enabled.contains("[Link Context - URLs detected in message]"));
        assert!(enabled.contains("https://example.com/report"));
    }

    #[test]
    fn streaming_user_input_channel_uses_configured_buffer() {
        let channels = streaming_loop_channels();

        for idx in 0..STREAMING_USER_INPUT_BUFFER {
            channels
                .user_input_tx
                .try_send(format!("message-{idx}"))
                .expect("buffer slot should be available");
        }
        assert!(channels
            .user_input_tx
            .try_send("overflow".to_string())
            .is_err());
    }
}
