use captain_memory::session::Session;
use captain_runtime::memory_retractions::MemoryRetraction;
use captain_runtime::prompt_builder::{BuiltSystemPrompt, PromptContext};
use captain_types::agent::{AgentId, AgentManifest};
use captain_types::tool::ToolDefinition;

use super::kernel_agent_runtime::prompt_profile_for_provider;
use super::kernel_first_use_text::read_global_user_profile;
use super::kernel_project_prompt::{
    read_recent_journal, resolve_active_project, resolve_recent_projects,
};
use super::kernel_prompt_context::{
    append_turn_diagnostic_context, assistant_style_context, build_persistent_memory_capsule,
    extract_feedback_rules, filter_prompt_memory_context, read_identity_file,
    read_workspace_prompt_file, system_prompt_with_runtime_update,
};
use super::kernel_workspace_security::shared_memory_agent_id;
use super::CaptainKernel;

pub(super) struct LlmPromptRequest<'a> {
    pub agent_id: AgentId,
    pub message: &'a str,
    pub manifest: &'a mut AgentManifest,
    pub session: &'a Session,
    pub tools: &'a [ToolDefinition],
    pub lean_direct: bool,
    pub sender_id: Option<String>,
    pub sender_name: Option<String>,
    pub channel_type: Option<String>,
    pub include_graph_recall: bool,
}

struct PromptRuntimeSnapshot {
    mcp_tool_count: usize,
    user_name: Option<String>,
    memory_retractions: Vec<MemoryRetraction>,
    runtime_update_notice: Option<String>,
    peer_agents: Vec<(String, String, String)>,
}

struct FullPromptContextRequest<'a> {
    agent_id: AgentId,
    message: &'a str,
    manifest: &'a AgentManifest,
    tools: &'a [ToolDefinition],
    sender_id: Option<String>,
    sender_name: Option<String>,
    channel_type: Option<String>,
    include_graph_recall: bool,
}

impl CaptainKernel {
    pub(super) fn prepare_llm_prompt(&self, request: LlmPromptRequest<'_>) {
        let LlmPromptRequest {
            agent_id,
            message,
            manifest,
            session,
            tools,
            lean_direct,
            sender_id,
            sender_name,
            channel_type,
            include_graph_recall,
        } = request;

        if lean_direct {
            self.prepare_lean_direct_prompt(manifest, channel_type);
            return;
        }

        let snapshot = self.prompt_runtime_snapshot();
        let prompt_ctx = self.full_prompt_context(
            FullPromptContextRequest {
                agent_id,
                message,
                manifest,
                tools,
                sender_id,
                sender_name,
                channel_type,
                include_graph_recall,
            },
            &snapshot,
        );
        apply_full_prompt(manifest, &prompt_ctx, session, message);
    }

    fn prepare_lean_direct_prompt(
        &self,
        manifest: &mut AgentManifest,
        channel_type: Option<String>,
    ) {
        let prompt_ctx = self.lean_direct_prompt_context(manifest, channel_type);
        let built_prompt =
            captain_runtime::prompt_builder::build_lean_direct_system_prompt(&prompt_ctx);
        apply_lean_direct_prompt(manifest, built_prompt);
    }

    fn lean_direct_prompt_context(
        &self,
        manifest: &AgentManifest,
        channel_type: Option<String>,
    ) -> PromptContext {
        PromptContext {
            agent_name: manifest.name.clone(),
            agent_description: manifest.description.clone(),
            active_provider: Some(manifest.model.provider.clone()),
            active_model: Some(manifest.model.model.clone()),
            base_system_prompt: manifest.model.system_prompt.clone(),
            configured_language: Some(self.config.language.clone()),
            deployment_profile: Some(self.config.deployment.profile.clone()),
            channel_type,
            is_subagent: manifest_is_subagent(manifest),
            model_family: Some(prompt_model_family(manifest)),
            ..Default::default()
        }
    }

    fn prompt_runtime_snapshot(&self) -> PromptRuntimeSnapshot {
        let mcp_tool_count = self.mcp_tools.lock().map(|t| t.len()).unwrap_or(0);
        let shared_id = shared_memory_agent_id();
        let user_name = self
            .memory
            .structured_get(shared_id, "user_name")
            .ok()
            .flatten()
            .and_then(|v| v.as_str().map(String::from));
        let memory_retractions = self.memory_retractions_for_prompt();
        let runtime_update_notice = self.runtime_update_notice();

        // State labels carry the last-activity age: an injected "Running"
        // goes stale during long turns and after compaction (the demo run
        // trusted a context that said Running while the agent had crashed).
        let now = chrono::Utc::now();
        let peer_agents: Vec<(String, String, String)> = self
            .registry
            .list()
            .iter()
            .map(|agent| {
                (
                    agent.name.clone(),
                    format!(
                        "{:?}, active {}",
                        agent.state,
                        humanize_age(now.signed_duration_since(agent.last_active))
                    ),
                    agent.manifest.model.model.clone(),
                )
            })
            .collect();

        PromptRuntimeSnapshot {
            mcp_tool_count,
            user_name,
            memory_retractions,
            runtime_update_notice,
            peer_agents,
        }
    }

    fn full_prompt_context(
        &self,
        request: FullPromptContextRequest<'_>,
        snapshot: &PromptRuntimeSnapshot,
    ) -> PromptContext {
        let mut ctx = self.base_full_prompt_context(&request, snapshot);
        self.apply_full_memory_context(&mut ctx, &request, snapshot);
        self.apply_full_workspace_context(&mut ctx, request.manifest, &snapshot.memory_retractions);
        ctx
    }

    fn base_full_prompt_context(
        &self,
        request: &FullPromptContextRequest<'_>,
        snapshot: &PromptRuntimeSnapshot,
    ) -> PromptContext {
        let manifest = request.manifest;
        let active_project =
            resolve_active_project(&self.memory, &self.goal_store, request.agent_id);
        let active_project_slug = active_project.as_ref().map(|project| project.slug.as_str());
        PromptContext {
            agent_name: manifest.name.clone(),
            agent_description: manifest.description.clone(),
            active_provider: Some(manifest.model.provider.clone()),
            active_model: Some(manifest.model.model.clone()),
            base_system_prompt: system_prompt_with_runtime_update(
                &manifest.model.system_prompt,
                snapshot.runtime_update_notice.clone(),
            ),
            granted_tools: request.tools.iter().map(|tool| tool.name.clone()).collect(),
            skill_summary: self.build_skill_summary(&manifest.skills),
            skill_prompt_context: self.collect_prompt_context(&manifest.skills),
            mcp_summary: if snapshot.mcp_tool_count > 0 {
                self.build_mcp_summary(&manifest.mcp_servers)
            } else {
                String::new()
            },
            configured_language: Some(self.config.language.clone()),
            deployment_profile: Some(self.config.deployment.profile.clone()),
            memory_md: None,
            user_name: snapshot.user_name.clone(),
            channel_type: request.channel_type.clone(),
            is_subagent: manifest_is_subagent(manifest),
            is_autonomous: manifest.autonomous.is_some(),
            bootstrap_md: None,
            peer_agents: snapshot.peer_agents.clone(),
            current_date: Some(current_prompt_date()),
            sender_id: request.sender_id.clone(),
            sender_name: request.sender_name.clone(),
            orchestration_mode: manifest.orchestration_mode,
            model_family: Some(prompt_model_family(manifest)),
            recent_projects: resolve_recent_projects(&self.memory, active_project_slug),
            active_project,
            prompt_profile: prompt_profile_for_provider(&manifest.model.provider),
            ..Default::default()
        }
    }

    fn apply_full_memory_context(
        &self,
        ctx: &mut PromptContext,
        request: &FullPromptContextRequest<'_>,
        snapshot: &PromptRuntimeSnapshot,
    ) {
        let retractions = &snapshot.memory_retractions;
        ctx.recalled_memories = self.recalled_memories_for_prompt(
            request.message,
            request.include_graph_recall,
            retractions,
        );
        ctx.user_md = filter_prompt_memory_context(
            read_global_user_profile(&self.config.home_dir),
            retractions,
        );
        ctx.persistent_memory_capsule = build_persistent_memory_capsule(&self.memory, retractions);
        ctx.canonical_context = filter_prompt_memory_context(
            self.memory
                .canonical_context(request.agent_id, None)
                .ok()
                .and_then(|(context, _)| context),
            retractions,
        );
        ctx.graph_md = filter_prompt_memory_context(
            read_identity_file(&self.config.home_dir, "GRAPH.md"),
            retractions,
        );
    }

    fn recalled_memories_for_prompt(
        &self,
        message: &str,
        include_graph_recall: bool,
        retractions: &[MemoryRetraction],
    ) -> Vec<(String, String)> {
        if !include_graph_recall {
            return Vec::new();
        }
        self.graph_memory
            .recall(message, 3)
            .into_iter()
            .filter_map(|turn| {
                captain_runtime::memory_retractions::filter_retracted_lines(
                    &turn.content,
                    retractions,
                )
                .map(|content| (format!("{}@{}", turn.role, turn.agent), content))
            })
            .collect()
    }

    fn apply_full_workspace_context(
        &self,
        ctx: &mut PromptContext,
        manifest: &AgentManifest,
        retractions: &[MemoryRetraction],
    ) {
        let style_file = manifest
            .workspace
            .as_ref()
            .and_then(|workspace| read_identity_file(workspace, "STYLE.md"));
        ctx.style_md = assistant_style_context(&self.config.assistant, style_file);

        let Some(workspace) = manifest.workspace.as_ref() else {
            return;
        };
        ctx.workspace_path = Some(workspace.display().to_string());
        ctx.soul_md = read_identity_file(workspace, "SOUL.md");
        ctx.agents_md = read_workspace_prompt_file(workspace, "AGENTS.md");
        ctx.workspace_context = Some(workspace_context_section(workspace));
        ctx.identity_md = read_identity_file(workspace, "IDENTITY.md");
        ctx.heartbeat_md = if manifest.autonomous.is_some() {
            read_identity_file(workspace, "HEARTBEAT.md")
        } else {
            None
        };
        ctx.feedback_rules = extract_feedback_rules(&workspace.join("FEEDBACK.jsonl"));
        ctx.recent_journal = filter_prompt_memory_context(
            read_recent_journal(&workspace.join("memory"), 3),
            retractions,
        );
    }
}

fn apply_lean_direct_prompt(manifest: &mut AgentManifest, built_prompt: BuiltSystemPrompt) {
    apply_built_prompt(manifest, built_prompt);
    manifest.metadata.remove("canonical_context_msg");
    manifest
        .metadata
        .insert("lean_direct_turn".to_string(), serde_json::json!(true));
}

fn apply_full_prompt(
    manifest: &mut AgentManifest,
    prompt_ctx: &PromptContext,
    session: &Session,
    message: &str,
) {
    let built_prompt = captain_runtime::prompt_builder::build_system_prompt_with_cache(prompt_ctx);
    apply_built_prompt(manifest, built_prompt);
    let canonical_context =
        captain_runtime::prompt_builder::build_canonical_context_message(prompt_ctx);
    if let Some(canonical_context) =
        append_turn_diagnostic_context(canonical_context, session, message)
    {
        manifest.metadata.insert(
            "canonical_context_msg".to_string(),
            serde_json::Value::String(canonical_context),
        );
    }
    manifest.metadata.remove("lean_direct_turn");
}

fn apply_built_prompt(manifest: &mut AgentManifest, built_prompt: BuiltSystemPrompt) {
    manifest.model.system_prompt = built_prompt.system_prompt;
    if let Some(bytes) = built_prompt.cacheable_prefix_bytes {
        manifest.metadata.insert(
            "system_cache_prefix_bytes".to_string(),
            serde_json::json!(bytes),
        );
    } else {
        manifest.metadata.remove("system_cache_prefix_bytes");
    }
}

fn manifest_is_subagent(manifest: &AgentManifest) -> bool {
    manifest
        .metadata
        .get("is_subagent")
        .and_then(|value| value.as_bool())
        .unwrap_or(false)
}

fn prompt_model_family(manifest: &AgentManifest) -> captain_runtime::prompt_builder::ModelFamily {
    captain_runtime::prompt_builder::detect_model_family(&manifest.model.model)
}

fn current_prompt_date() -> String {
    chrono::Local::now()
        .format("%A, %B %d, %Y (%Y-%m-%d)")
        .to_string()
}

/// Compact age label for peer-agent activity ("just now", "3m ago", "2h ago").
fn humanize_age(age: chrono::Duration) -> String {
    let secs = age.num_seconds().max(0);
    if secs < 60 {
        "just now".to_string()
    } else if secs < 3600 {
        format!("{}m ago", secs / 60)
    } else if secs < 86_400 {
        format!("{}h ago", secs / 3600)
    } else {
        format!("{}d ago", secs / 86_400)
    }
}

fn workspace_context_section(workspace: &std::path::Path) -> String {
    let mut workspace_context =
        captain_runtime::workspace_context::WorkspaceContext::detect(workspace);
    workspace_context.build_context_section()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn humanize_age_buckets_are_stable() {
        assert_eq!(humanize_age(chrono::Duration::seconds(5)), "just now");
        assert_eq!(humanize_age(chrono::Duration::seconds(200)), "3m ago");
        assert_eq!(humanize_age(chrono::Duration::hours(2)), "2h ago");
        assert_eq!(humanize_age(chrono::Duration::days(3)), "3d ago");
        assert_eq!(humanize_age(chrono::Duration::seconds(-10)), "just now");
    }

    #[test]
    fn apply_built_prompt_updates_cache_metadata() {
        let mut manifest = AgentManifest::default();
        apply_built_prompt(
            &mut manifest,
            BuiltSystemPrompt {
                system_prompt: "cached prompt".to_string(),
                cacheable_prefix_bytes: Some(7),
            },
        );

        assert_eq!(manifest.model.system_prompt, "cached prompt");
        assert_eq!(
            manifest
                .metadata
                .get("system_cache_prefix_bytes")
                .and_then(|value| value.as_u64()),
            Some(7)
        );
    }

    #[test]
    fn apply_built_prompt_removes_stale_cache_metadata() {
        let mut manifest = AgentManifest::default();
        manifest.metadata.insert(
            "system_cache_prefix_bytes".to_string(),
            serde_json::json!(123),
        );

        apply_built_prompt(
            &mut manifest,
            BuiltSystemPrompt {
                system_prompt: "uncached prompt".to_string(),
                cacheable_prefix_bytes: None,
            },
        );

        assert_eq!(manifest.model.system_prompt, "uncached prompt");
        assert!(!manifest.metadata.contains_key("system_cache_prefix_bytes"));
    }

    #[test]
    fn apply_lean_direct_prompt_marks_turn_and_clears_context() {
        let mut manifest = AgentManifest::default();
        manifest.metadata.insert(
            "canonical_context_msg".to_string(),
            serde_json::json!("stale context"),
        );

        apply_lean_direct_prompt(
            &mut manifest,
            BuiltSystemPrompt {
                system_prompt: "lean prompt".to_string(),
                cacheable_prefix_bytes: Some(11),
            },
        );

        assert_eq!(manifest.model.system_prompt, "lean prompt");
        assert_eq!(
            manifest
                .metadata
                .get("system_cache_prefix_bytes")
                .and_then(|value| value.as_u64()),
            Some(11)
        );
        assert!(!manifest.metadata.contains_key("canonical_context_msg"));
        assert_eq!(
            manifest
                .metadata
                .get("lean_direct_turn")
                .and_then(|value| value.as_bool()),
            Some(true)
        );
    }

    #[test]
    fn manifest_prompt_helpers_read_explicit_metadata_and_model_family() {
        let mut manifest = AgentManifest::default();
        assert!(!manifest_is_subagent(&manifest));

        manifest
            .metadata
            .insert("is_subagent".to_string(), serde_json::json!("true"));
        assert!(!manifest_is_subagent(&manifest));

        manifest
            .metadata
            .insert("is_subagent".to_string(), serde_json::json!(true));
        manifest.model.model = "claude-sonnet-4-20250514".to_string();

        assert!(manifest_is_subagent(&manifest));
        assert_eq!(
            prompt_model_family(&manifest),
            captain_runtime::prompt_builder::ModelFamily::Anthropic
        );
    }
}
