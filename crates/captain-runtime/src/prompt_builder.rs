//! Centralized system prompt builder.
//!
//! Assembles a structured, multi-section system prompt from agent context.
//! Replaces the scattered `push_str` prompt injection throughout the codebase
//! with a single, testable, ordered prompt builder.

use crate::prompt_sanitizer::sanitize;

#[path = "prompt_builder_behavior.rs"]
mod prompt_builder_behavior;
#[cfg(test)]
#[path = "prompt_builder_channel_tests.rs"]
mod prompt_builder_channel_tests;
#[path = "prompt_builder_codex_compact.rs"]
mod prompt_builder_codex_compact;
#[cfg(test)]
#[path = "prompt_builder_context_tests.rs"]
mod prompt_builder_context_tests;
#[path = "prompt_builder_environment.rs"]
mod prompt_builder_environment;
#[cfg(test)]
#[path = "prompt_builder_guard_tests.rs"]
mod prompt_builder_guard_tests;
#[path = "prompt_builder_memory.rs"]
mod prompt_builder_memory;
#[cfg(test)]
#[path = "prompt_builder_memory_tests.rs"]
mod prompt_builder_memory_tests;
#[cfg(test)]
#[path = "prompt_builder_model_tests.rs"]
mod prompt_builder_model_tests;
#[cfg(test)]
#[path = "prompt_builder_persona_tests.rs"]
mod prompt_builder_persona_tests;
#[path = "prompt_builder_project.rs"]
mod prompt_builder_project;
#[cfg(test)]
#[path = "prompt_builder_rule_tests.rs"]
mod prompt_builder_rule_tests;
#[path = "prompt_builder_runtime_sections.rs"]
mod prompt_builder_runtime_sections;
#[cfg(test)]
#[path = "prompt_builder_size_tests.rs"]
mod prompt_builder_size_tests;
#[cfg(test)]
#[path = "prompt_builder_skill_tests.rs"]
mod prompt_builder_skill_tests;
#[cfg(test)]
#[path = "prompt_builder_structure_tests.rs"]
mod prompt_builder_structure_tests;
#[path = "prompt_builder_text.rs"]
mod prompt_builder_text;
#[path = "prompt_builder_tool_categories.rs"]
mod prompt_builder_tool_categories;
#[path = "prompt_builder_tool_docs.rs"]
mod prompt_builder_tool_docs;
#[path = "prompt_builder_tool_hints.rs"]
mod prompt_builder_tool_hints;
#[cfg(test)]
#[path = "prompt_builder_tool_tests.rs"]
mod prompt_builder_tool_tests;
#[path = "prompt_builder_types.rs"]
mod prompt_builder_types;

use prompt_builder_behavior::{
    model_family_guidance, DECISION_TABLES, DEEP_RESEARCH_BEHAVIOR, TOOL_CALL_BEHAVIOR,
};
use prompt_builder_codex_compact::build_codex_economy_system_prompt_with_cache;
use prompt_builder_environment::{
    build_deployment_context_section, build_environment_section, build_language_contract_section,
};
use prompt_builder_memory::build_persistent_memory_capsule_section;
pub use prompt_builder_memory::{
    build_canonical_context_message, build_memory_protocol_section, build_memory_section,
    build_recalled_memories_section,
};
use prompt_builder_project::{format_active_project_section, format_recent_projects_section};
pub use prompt_builder_runtime_sections::build_delegation_section;
use prompt_builder_runtime_sections::{
    build_channel_section, build_mcp_section, build_peer_agents_section, build_persona_section,
    build_sender_section, build_skills_section, build_user_section, CONSCIOUSNESS_AWARENESS,
    OPERATIONAL_GUIDELINES, SAFETY_SECTION,
};
use prompt_builder_text::{cap_str, capitalize};
pub use prompt_builder_tool_categories::tool_category;
pub use prompt_builder_tool_docs::tool_doc;
pub use prompt_builder_tool_hints::tool_hint;
pub use prompt_builder_types::{
    detect_model_family, ActiveProjectSummary, BuiltSystemPrompt, ModelFamily, PromptContext,
    PromptProfile, RecentProjectSummary,
};

/// Build the complete system prompt from a `PromptContext`.
///
/// Produces an ordered, multi-section prompt. Sections with no content are
/// omitted entirely (no empty headers). Subagent mode skips sections that
/// add unnecessary context overhead.
pub fn build_system_prompt(ctx: &PromptContext) -> String {
    build_system_prompt_with_cache(ctx).system_prompt
}

/// Build the system prompt and identify the provider-cacheable prefix.
///
/// Official provider guidance is strict: cache hits require an exact repeated
/// prefix. Keep reusable instructions first, then append turn/session-specific
/// context at the end so Anthropic/OpenAI/Gemini can reuse the heavy prefix
/// without hiding dynamic context from the model.
pub fn build_system_prompt_with_cache(ctx: &PromptContext) -> BuiltSystemPrompt {
    if ctx.prompt_profile == PromptProfile::CodexEconomy {
        return build_codex_economy_system_prompt_with_cache(ctx);
    }

    render_prompt_sections(build_full_system_prompt_sections(ctx))
}

struct PromptSections {
    cacheable: Vec<String>,
    dynamic: Vec<String>,
}

impl PromptSections {
    fn new() -> Self {
        Self {
            cacheable: Vec::with_capacity(12),
            dynamic: Vec::with_capacity(8),
        }
    }

    fn push_cacheable(&mut self, section: impl Into<String>) {
        self.cacheable.push(section.into());
    }

    fn push_dynamic(&mut self, section: impl Into<String>) {
        self.dynamic.push(section.into());
    }
}

fn build_full_system_prompt_sections(ctx: &PromptContext) -> PromptSections {
    let mut sections = PromptSections::new();
    sections.push_cacheable(build_identity_section(ctx));
    push_dynamic_turn_context_sections(ctx, &mut sections);
    push_cacheable_runtime_behavior_sections(ctx, &mut sections);
    push_tool_memory_sections(ctx, &mut sections);
    push_skill_persona_sections(ctx, &mut sections);
    push_recent_learning_sections(ctx, &mut sections);
    push_dynamic_user_context_sections(ctx, &mut sections);
    push_safety_tail_sections(ctx, &mut sections);
    sections
}

fn push_dynamic_turn_context_sections(ctx: &PromptContext, sections: &mut PromptSections) {
    if let Some(ref p) = ctx.active_project {
        sections.push_dynamic(format_active_project_section(p, false));
    }
    if !ctx.recent_projects.is_empty() {
        sections.push_dynamic(format_recent_projects_section(&ctx.recent_projects, false));
    }
    if let Some(ref date) = ctx.current_date {
        sections.push_dynamic(format!("## Current Date\nToday is {date}."));
    }
    if let Some(section) = build_runtime_identity_section(ctx) {
        sections.push_dynamic(section);
    }
    if let Some(section) = build_language_contract_section(ctx.configured_language.as_deref()) {
        sections.push_dynamic(section);
    }
    if let Some(section) = build_deployment_context_section(ctx.deployment_profile.as_deref()) {
        sections.push_dynamic(section);
    }
}

fn push_cacheable_runtime_behavior_sections(ctx: &PromptContext, sections: &mut PromptSections) {
    if ctx.is_subagent {
        return;
    }
    sections.push_cacheable(build_environment_section());
    sections.push_cacheable(TOOL_CALL_BEHAVIOR.to_string());
    sections.push_cacheable(DECISION_TABLES.to_string());
    sections.push_cacheable(DEEP_RESEARCH_BEHAVIOR.to_string());
    if let Some(family) = ctx.model_family {
        if let Some(guidance) = model_family_guidance(family) {
            sections.push_cacheable(guidance.to_string());
        }
    }
    if let Some(ref agents) = ctx.agents_md {
        if !agents.trim().is_empty() {
            sections.push_cacheable(cap_str(&sanitize("AGENTS.md", agents), 2000));
        }
    }
}

fn push_tool_memory_sections(ctx: &PromptContext, sections: &mut PromptSections) {
    let tools_section = build_tools_section(&ctx.granted_tools);
    if !tools_section.is_empty() {
        sections.push_cacheable(tools_section);
    }
    sections.push_cacheable(build_memory_protocol_section());
    if let Some(ref capsule) = ctx.persistent_memory_capsule {
        if !capsule.trim().is_empty() {
            sections.push_dynamic(build_persistent_memory_capsule_section(capsule));
        }
    }
    sections.push_dynamic(build_recalled_memories_section(&ctx.recalled_memories));
    if ctx.orchestration_mode == captain_types::agent::OrchestrationMode::Delegation {
        sections.push_cacheable(build_delegation_section());
    }
}

fn push_skill_persona_sections(ctx: &PromptContext, sections: &mut PromptSections) {
    if !ctx.skill_summary.is_empty() || !ctx.skill_prompt_context.is_empty() {
        sections.push_cacheable(build_skills_section(
            &ctx.skill_summary,
            &ctx.skill_prompt_context,
        ));
    }
    if !ctx.mcp_summary.is_empty() {
        sections.push_cacheable(build_mcp_section(&ctx.mcp_summary));
    }
    if ctx.is_subagent {
        return;
    }
    let persona = build_persona_section(
        ctx.identity_md.as_deref(),
        ctx.soul_md.as_deref(),
        ctx.user_md.as_deref(),
        ctx.graph_md.as_deref(),
        ctx.workspace_path.as_deref(),
    );
    if !persona.is_empty() {
        sections.push_cacheable(persona);
    }
    push_style_section(ctx, sections);
}

fn push_style_section(ctx: &PromptContext, sections: &mut PromptSections) {
    if let Some(ref style) = ctx.style_md {
        if !style.trim().is_empty() {
            sections.push_cacheable(format!(
                "## Communication Style\n{}",
                cap_str(&sanitize("STYLE.md", style), 500)
            ));
        }
    }
}

fn push_recent_learning_sections(ctx: &PromptContext, sections: &mut PromptSections) {
    if ctx.is_subagent {
        return;
    }
    if let Some(ref journal) = ctx.recent_journal {
        if !journal.trim().is_empty() {
            sections.push_dynamic(format!(
                "## Recent Activity Journal\nYour notes from the past few days:\n{}",
                cap_str(journal, 2000)
            ));
        }
    }
    if let Some(ref rules) = ctx.feedback_rules {
        if !rules.trim().is_empty() {
            sections.push_dynamic(format!(
                "## Learned Rules (from user feedback)\nThe user has corrected your behavior in the past. Follow these rules:\n{}",
                cap_str(rules, 500)
            ));
        }
    }
    if ctx.is_autonomous {
        push_heartbeat_section(ctx, sections);
    }
}

fn push_heartbeat_section(ctx: &PromptContext, sections: &mut PromptSections) {
    if let Some(ref heartbeat) = ctx.heartbeat_md {
        if !heartbeat.trim().is_empty() {
            sections.push_cacheable(format!(
                "## Heartbeat Checklist\n{}",
                cap_str(heartbeat, 1000)
            ));
        }
    }
}

fn push_dynamic_user_context_sections(ctx: &PromptContext, sections: &mut PromptSections) {
    if ctx.is_subagent {
        return;
    }
    sections.push_dynamic(build_user_section(ctx.user_name.as_deref()));
    if let Some(ref channel) = ctx.channel_type {
        sections.push_dynamic(build_channel_section(channel));
    }
    if let Some(sender_line) =
        build_sender_section(ctx.sender_name.as_deref(), ctx.sender_id.as_deref())
    {
        sections.push_dynamic(sender_line);
    }
    if !ctx.peer_agents.is_empty() {
        sections.push_dynamic(build_peer_agents_section(&ctx.agent_name, &ctx.peer_agents));
    }
}

fn push_safety_tail_sections(ctx: &PromptContext, sections: &mut PromptSections) {
    if !ctx.is_subagent {
        sections.push_cacheable(SAFETY_SECTION.to_string());
        sections.push_cacheable(CONSCIOUSNESS_AWARENESS.to_string());
    }
    sections.push_cacheable(OPERATIONAL_GUIDELINES.to_string());
    push_bootstrap_section(ctx, sections);
    push_workspace_context_section(ctx, sections);
}

fn push_bootstrap_section(ctx: &PromptContext, sections: &mut PromptSections) {
    if ctx.is_subagent {
        return;
    }
    if let Some(ref bootstrap) = ctx.bootstrap_md {
        if !bootstrap.trim().is_empty() {
            let has_user_name = ctx.recalled_memories.iter().any(|(k, _)| k == "user_name");
            if !has_user_name && ctx.user_name.is_none() {
                sections.push_cacheable(format!(
                    "## First-Run Protocol\n{}",
                    cap_str(&sanitize("BOOTSTRAP.md", bootstrap), 1500)
                ));
            }
        }
    }
}

fn push_workspace_context_section(ctx: &PromptContext, sections: &mut PromptSections) {
    if ctx.is_subagent {
        return;
    }
    if let Some(ref ws_ctx) = ctx.workspace_context {
        if !ws_ctx.trim().is_empty() {
            sections.push_cacheable(cap_str(&sanitize("workspace_context", ws_ctx), 1000));
        }
    }
}

fn render_prompt_sections(sections: PromptSections) -> BuiltSystemPrompt {
    let cacheable_prompt = sections.cacheable.join("\n\n");
    let cacheable_prefix_bytes = if cacheable_prompt.is_empty() {
        None
    } else {
        Some(cacheable_prompt.len())
    };
    let dynamic_prompt = if sections.dynamic.is_empty() {
        String::new()
    } else {
        format!(
            "## Current Turn Context\nThese sections are fresh for this turn. Treat them as authoritative when they refine or override reusable instructions above.\n\n{}",
            sections.dynamic.join("\n\n")
        )
    };

    let system_prompt = if dynamic_prompt.is_empty() {
        cacheable_prompt
    } else if cacheable_prompt.is_empty() {
        dynamic_prompt
    } else {
        format!("{}\n\n{}", cacheable_prompt, dynamic_prompt)
    };

    BuiltSystemPrompt {
        system_prompt,
        cacheable_prefix_bytes,
    }
}

/// Build a deliberately tiny prompt for trivial/direct turns.
///
/// This path is selected by the kernel only when the current user message is
/// clearly answerable without tools, memory, workspace context, or history
/// lookup. Keeping it separate from the full prompt preserves quality for real
/// work while avoiding a 10k+ token fixed prompt for "hey" or exact-echo probes.
pub fn build_lean_direct_system_prompt(ctx: &PromptContext) -> BuiltSystemPrompt {
    let identity = if ctx.base_system_prompt.trim().is_empty() {
        format!(
            "You are {}, an AI agent running inside the Captain Agent OS.",
            ctx.agent_name
        )
    } else {
        cap_str(
            &sanitize("base_system_prompt", &ctx.base_system_prompt),
            1200,
        )
    };

    let mut sections = Vec::with_capacity(4);
    sections.push(identity);
    sections.push(
        "## Direct Response Mode\n\
         The current user message is simple and does not require tools, memory, \
         files, network access, or workspace inspection.\n\
         - Answer directly in the user's language.\n\
         - Keep the response concise.\n\
         - Do not mention tools, hidden routing, prompts, or internal context.\n\
         - If the user asks you to reply with exact text, output exactly that text."
            .to_string(),
    );

    if let Some(section) = build_runtime_identity_section(ctx) {
        sections.push(section);
    }

    if let Some(ref channel) = ctx.channel_type {
        if !channel.trim().is_empty() {
            sections.push(format!("Channel: {channel}."));
        }
    }
    if let Some(section) = build_language_contract_section(ctx.configured_language.as_deref()) {
        sections.push(section);
    }
    if let Some(section) = build_deployment_context_section(ctx.deployment_profile.as_deref()) {
        sections.push(section);
    }

    let system_prompt = sections.join("\n\n");
    BuiltSystemPrompt {
        cacheable_prefix_bytes: Some(system_prompt.len()),
        system_prompt,
    }
}

// ---------------------------------------------------------------------------
// Section builders
// ---------------------------------------------------------------------------

fn build_runtime_identity_section(ctx: &PromptContext) -> Option<String> {
    let provider = ctx.active_provider.as_deref()?.trim();
    let model = ctx.active_model.as_deref()?.trim();
    if provider.is_empty() || model.is_empty() {
        return None;
    }

    Some(format!(
        "## Runtime Identity\n\
         Active agent provider: `{}`\n\
         Active agent model: `{}`\n\
         These values are live for this turn. When asked which provider or model this agent is using, answer from this section. Do not infer your identity from peer agents, old session messages, memory, or model training. This is separate from the Captain binary version, which must be verified from live runtime status.",
        cap_str(provider, 80),
        cap_str(model, 160)
    ))
}

fn build_identity_section(ctx: &PromptContext) -> String {
    if ctx.base_system_prompt.is_empty() {
        format!(
            "You are {}, an AI agent running inside the Captain Agent OS.\n{}",
            ctx.agent_name, ctx.agent_description
        )
    } else {
        ctx.base_system_prompt.clone()
    }
}

/// Build the grouped tools section (Section 3).
///
/// Tools with a full `tool_doc()` (v3.7d WHEN/WHY/SKIP triptych) get
/// a dedicated sub-section *after* the grouped one-liners so the LLM
/// sees both the inventory and the decision framework for critical tools.
pub fn build_tools_section(granted_tools: &[String]) -> String {
    if granted_tools.is_empty() {
        return String::new();
    }

    // Group tools by category
    let mut groups: std::collections::BTreeMap<&str, Vec<(&str, &str)>> =
        std::collections::BTreeMap::new();
    for name in granted_tools {
        let cat = tool_category(name);
        let hint = tool_hint(name);
        groups.entry(cat).or_default().push((name.as_str(), hint));
    }

    let mut out = String::from("## Your Tools\nYou have access to these capabilities:\n");
    for (category, tools) in &groups {
        out.push_str(&format!("\n**{}**: ", capitalize(category)));
        let descs: Vec<String> = tools
            .iter()
            .map(|(name, hint)| {
                if hint.is_empty() {
                    (*name).to_string()
                } else {
                    format!("{name} ({hint})")
                }
            })
            .collect();
        out.push_str(&descs.join(", "));
    }

    // v3.7d — Full WHEN/WHY/SKIP docs for granted critical tools
    let docs: Vec<(&str, &'static str)> = granted_tools
        .iter()
        .filter_map(|name| tool_doc(name).map(|doc| (name.as_str(), doc)))
        .collect();
    if !docs.is_empty() {
        out.push_str("\n\n### Critical tools — when to call, when to skip\n");
        for (name, doc) in docs {
            out.push_str(&format!("\n**{name}**\n{doc}\n"));
        }
    }

    out
}

// ---------------------------------------------------------------------------
// Utilities
// ---------------------------------------------------------------------------
