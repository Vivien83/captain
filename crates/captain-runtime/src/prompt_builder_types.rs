/// LLM model families with distinct known failure modes (v3.7f).
///
/// Used to conditionally inject family-specific guidance into the system
/// prompt. Keeps the prompt dense with relevance — each non-applicable
/// instruction dilutes the applicable ones.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ModelFamily {
    /// OpenAI GPT/Codex — tend to narrate before acting, need act_dont_ask nudge.
    OpenAI,
    /// Google Gemini/Gemma — tend to refuse ambiguously or over-apologize.
    Google,
    /// Anthropic Claude — strong tool-caller, needs permission-scope reminder.
    Anthropic,
    /// Xiaomi Mimo — fine-tuned for tool calling, needs minimal nudging.
    Mimo,
    /// Any other family — no family-specific guidance.
    Other,
}

/// Prompt compilation profile.
///
/// `Full` preserves the existing rich prompt used by Claude and other models.
/// `CodexEconomy` keeps the same product contracts, but compiles them into a
/// smaller cache-friendly prompt and relies on Captain's discovery/retrieval
/// tools to rehydrate detail on demand.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum PromptProfile {
    #[default]
    Full,
    CodexEconomy,
}

/// Detect the model family from a provider-prefixed or plain model ID.
///
/// Matches on common substring markers. Falls back to `Other` when no known
/// family is recognized — safer than guessing wrong guidance.
pub fn detect_model_family(model_id: &str) -> ModelFamily {
    let m = model_id.to_ascii_lowercase();
    if m.contains("gpt") || m.contains("codex") || m.contains("o1-") || m.contains("o3") {
        ModelFamily::OpenAI
    } else if m.contains("gemini") || m.contains("gemma") {
        ModelFamily::Google
    } else if m.contains("claude") {
        ModelFamily::Anthropic
    } else if m.contains("mimo") {
        ModelFamily::Mimo
    } else {
        ModelFamily::Other
    }
}

/// All the context needed to build a system prompt for an agent.
#[derive(Debug, Clone, Default)]
pub struct PromptContext {
    /// Agent name (from manifest).
    pub agent_name: String,
    /// Agent description (from manifest).
    pub agent_description: String,
    /// Provider selected for this agent turn. This is live runtime identity,
    /// not a catalog recommendation or a peer-agent attribute.
    pub active_provider: Option<String>,
    /// Model selected for this agent turn. Prompt builders expose it in a
    /// dynamic section so restored history cannot override current identity.
    pub active_model: Option<String>,
    /// Base system prompt authored in the agent manifest.
    pub base_system_prompt: String,
    /// Tool names this agent has access to.
    pub granted_tools: Vec<String>,
    /// Recalled memories as (key, content) pairs.
    pub recalled_memories: Vec<(String, String)>,
    /// Skill summary text (from kernel.build_skill_summary()).
    pub skill_summary: String,
    /// Prompt context from prompt-only skills.
    pub skill_prompt_context: String,
    /// MCP server/tool summary text.
    pub mcp_summary: String,
    /// Agent workspace path.
    pub workspace_path: Option<String>,
    /// SOUL.md content (persona).
    pub soul_md: Option<String>,
    /// Global USER.md profile content.
    pub user_md: Option<String>,
    /// Configured default user language (e.g. "fr"). Used as a first-turn
    /// fallback before enough conversational language evidence exists.
    pub configured_language: Option<String>,
    /// Installation/deployment profile from config (`vps`, `desktop`, `core`,
    /// ...). Used to disambiguate local-vs-remote actions on hosted installs.
    pub deployment_profile: Option<String>,
    /// Retired legacy MEMORY.md content. Kept for compatibility with older
    /// callers, but never prompt-injected; durable memory comes from
    /// recalled_memories, canonical_context, graph_md and MemPalace tools.
    pub memory_md: Option<String>,
    /// Short declarative fact capsule derived from committed persistent memory.
    /// This is the Captain-native always-on memory layer: compact facts only,
    /// never workflows or imperative instructions.
    pub persistent_memory_capsule: Option<String>,
    /// Cross-channel canonical context summary.
    pub canonical_context: Option<String>,
    /// Known user name (from shared memory).
    pub user_name: Option<String>,
    /// Channel type (telegram, discord, web, etc.).
    pub channel_type: Option<String>,
    /// Whether this agent was spawned as a subagent.
    pub is_subagent: bool,
    /// Whether this agent has autonomous config.
    pub is_autonomous: bool,
    /// AGENTS.md content (behavioral guidance).
    pub agents_md: Option<String>,
    /// BOOTSTRAP.md content (first-run ritual).
    pub bootstrap_md: Option<String>,
    /// Workspace context section (project type, context files).
    pub workspace_context: Option<String>,
    /// IDENTITY.md content (visual identity + personality frontmatter).
    pub identity_md: Option<String>,
    /// HEARTBEAT.md content (autonomous agent checklist).
    pub heartbeat_md: Option<String>,
    /// Peer agents visible to this agent: (name, state, model).
    pub peer_agents: Vec<(String, String, String)>,
    /// Current date/time string for temporal awareness.
    pub current_date: Option<String>,
    /// Sender identity (e.g. WhatsApp phone number, Telegram user ID).
    pub sender_id: Option<String>,
    /// Sender display name.
    pub sender_name: Option<String>,
    /// Learned rules from user feedback (FEEDBACK.jsonl).
    pub feedback_rules: Option<String>,
    /// GRAPH.md content (auto-generated graph snapshot).
    pub graph_md: Option<String>,
    /// STYLE.md content (tone, format, per-channel conventions).
    pub style_md: Option<String>,
    /// Recent journal entries from memory/ dir (last 3 days, capped).
    pub recent_journal: Option<String>,
    /// Orchestration mode — controls whether to inject delegation instructions.
    pub orchestration_mode: captain_types::agent::OrchestrationMode,
    /// LLM model family — controls family-specific guidance injection (v3.7f).
    pub model_family: Option<ModelFamily>,
    /// v3.11d — active project context for this agent. Resolved from
    /// `active_project::global().get(agent_id)` when the kernel builds
    /// the context. When `Some`, a `## Active Project` section is
    /// inserted so the LLM scopes reasoning (tasks, milestones, memory)
    /// to this project.
    pub active_project: Option<ActiveProjectSummary>,
    /// Compact list of recent non-terminal projects. This is not a replacement
    /// for `project_list`; it keeps project names/slugs visible so the agent
    /// can resolve user references before falling back to memory search.
    pub recent_projects: Vec<RecentProjectSummary>,
    /// Context compilation profile. Defaults to the full prompt so existing
    /// providers keep their behavior unless the kernel opts in explicitly.
    pub prompt_profile: PromptProfile,
}

/// Compact snapshot of the user's active project, injected into the
/// system prompt when one is set.
#[derive(Debug, Clone)]
pub struct ActiveProjectSummary {
    pub id: String,
    pub slug: String,
    pub name: String,
    pub goal: String,
    pub status: String,
    pub source_type: Option<String>,
    pub workspace_path: Option<String>,
    pub repository: Option<String>,
    pub latest_checkpoint: Option<String>,
    pub active_tasks: Vec<String>,
    pub blocked_tasks: Vec<String>,
    pub next_actions: Vec<String>,
    pub milestone_status: Option<String>,
    pub project_goals: Vec<String>,
    pub project_rules: Option<String>,
}

/// Minimal project row injected for continuity when no project is explicitly
/// active for the agent.
#[derive(Debug, Clone)]
pub struct RecentProjectSummary {
    pub slug: String,
    pub name: String,
    pub goal: String,
    pub status: String,
    pub runtime_status: String,
    pub runtime_phase: String,
    pub progress: u64,
    pub next_actions: Vec<String>,
}

/// System prompt plus the byte boundary of its stable/cacheable prefix.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BuiltSystemPrompt {
    pub system_prompt: String,
    pub cacheable_prefix_bytes: Option<usize>,
}
