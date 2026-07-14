use captain_runtime::agent_loop::AGENT_LOOP_MAX_ITERATIONS_KEY;
use captain_runtime::core_tools::SUBAGENT_DEFAULT_TOOLS;
use captain_types::agent::{AgentEntry, AgentId, AgentManifest, ManifestCapabilities, ToolProfile};
use captain_types::config::KernelConfig;
use captain_types::tool_compat::normalize_tool_name;

const CODEX_BACKGROUND_PRIMARY: &str = "gpt-5.5";
const CODEX_BACKGROUND_FALLBACKS: &[&str] = &[];
const CODEX_BACKGROUND_INCOMPATIBLE_MODELS: &[&str] = &["gpt-5.3-codex", "gpt-5.3-codex-spark"];
pub(super) const DEFAULT_CONTEXT_WINDOW_TOKENS: usize = 200_000;
pub(super) const STREAMING_USER_INPUT_BUFFER: usize = 64;

fn strip_provider_prefix_for_background(model: &str) -> &str {
    model
        .trim()
        .split_once('/')
        .map(|(_, model)| model)
        .unwrap_or_else(|| model.trim())
}

fn codex_background_model_is_compatible(model: &str) -> bool {
    !CODEX_BACKGROUND_INCOMPATIBLE_MODELS
        .iter()
        .any(|blocked| model.eq_ignore_ascii_case(blocked))
}

fn normalize_codex_background_model(
    catalog: &captain_runtime::model_catalog::ModelCatalog,
    configured_model: &str,
) -> Option<String> {
    let bare = strip_provider_prefix_for_background(configured_model);
    if bare.is_empty() || !codex_background_model_is_compatible(bare) {
        return None;
    }

    let prefixed = format!("codex/{bare}");
    catalog
        .find_model(&prefixed)
        .or_else(|| catalog.find_model(configured_model.trim()))
        .filter(|entry| entry.provider == "codex")
        .map(|entry| strip_provider_prefix_for_background(&entry.id))
        .filter(|model| codex_background_model_is_compatible(model))
        .map(ToString::to_string)
}

pub(super) fn default_codex_background_model(
    catalog: &captain_runtime::model_catalog::ModelCatalog,
) -> String {
    normalize_codex_background_model(catalog, CODEX_BACKGROUND_PRIMARY).unwrap_or_else(|| {
        catalog
            .default_model_for_provider("codex")
            .and_then(|model| normalize_codex_background_model(catalog, &model))
            .unwrap_or_else(|| CODEX_BACKGROUND_PRIMARY.to_string())
    })
}

pub(super) fn normalize_background_model_for_provider(
    catalog: &captain_runtime::model_catalog::ModelCatalog,
    provider: &str,
    configured_model: &str,
) -> String {
    if provider.eq_ignore_ascii_case("codex") {
        normalize_codex_background_model(catalog, configured_model)
            .unwrap_or_else(|| default_codex_background_model(catalog))
    } else {
        configured_model.trim().to_string()
    }
}

pub(super) fn normalize_background_fallbacks_for_provider(
    catalog: &captain_runtime::model_catalog::ModelCatalog,
    provider: &str,
    primary_model: &str,
    configured_fallbacks: &[String],
) -> Vec<String> {
    if !provider.eq_ignore_ascii_case("codex") {
        return configured_fallbacks
            .iter()
            .map(|m| m.trim().to_string())
            .filter(|m| !m.is_empty())
            .collect();
    }

    let mut out = Vec::new();
    for model in configured_fallbacks {
        if let Some(normalized) = normalize_codex_background_model(catalog, model) {
            if normalized != primary_model && !out.contains(&normalized) {
                out.push(normalized);
            }
        }
    }
    for model in CODEX_BACKGROUND_FALLBACKS {
        if let Some(normalized) = normalize_codex_background_model(catalog, model) {
            if normalized != primary_model && !out.contains(&normalized) {
                out.push(normalized);
            }
        }
    }
    out
}

pub(super) fn is_lean_direct_turn(message: &str) -> bool {
    let normalized = message
        .trim()
        .to_ascii_lowercase()
        .replace(['’', '‘', '`'], "'");
    if normalized.is_empty() || normalized.chars().count() > 360 {
        return false;
    }

    let exact_reply_cues = [
        "reply exactly",
        "respond exactly",
        "réponds exactement",
        "reponds exactement",
        "répond exactement",
        "repond exactement",
    ];
    if exact_reply_cues.iter().any(|cue| normalized.contains(cue)) {
        return true;
    }

    let direct_response_cues = [
        "réponds en ",
        "reponds en ",
        "répond en ",
        "repond en ",
        "respond in ",
        "reply in ",
        "dis simplement",
        "dit simplement",
        "just say",
        "no tool",
        "aucun outil",
        "sans outil",
        "ne lance aucun outil",
    ];
    let has_direct_response_cue = direct_response_cues
        .iter()
        .any(|cue| normalized.contains(cue));

    let action_cues = [
        "analyse",
        "build",
        "check",
        "cherche",
        "corrige",
        "debug",
        "file",
        "fichier",
        "fix",
        "log",
        "relance",
        "recherche",
        "ssh",
        "test",
        "vps",
        "web",
        "url",
    ];
    if action_cues
        .iter()
        .any(|cue| normalized.split_whitespace().any(|word| word == *cue))
    {
        return false;
    }

    if has_direct_response_cue {
        return true;
    }

    matches!(
        normalized.as_str(),
        "hey"
            | "hello"
            | "hi"
            | "salut"
            | "bonjour"
            | "bonsoir"
            | "yo"
            | "coucou"
            | "merci"
            | "thanks"
            | "ok"
            | "okay"
            | "ça marche ?"
            | "ca marche ?"
            | "tu es la ?"
            | "tu es là ?"
            | "t'es la ?"
            | "t'es là ?"
            | "ping"
    )
}

fn compaction_config_for_provider(provider: &str) -> captain_runtime::compactor::CompactionConfig {
    captain_runtime::compactor::CompactionConfig::for_provider(provider)
}

pub(super) fn compaction_config_for_provider_with_context(
    provider: &str,
    context_window_tokens: Option<usize>,
) -> captain_runtime::compactor::CompactionConfig {
    let mut config = compaction_config_for_provider(provider);
    if let Some(window) = context_window_tokens.filter(|window| *window > 0) {
        config.context_window_tokens = window;
    }
    config
}

/// Compaction profile derived from the agent's manifest.
///
/// Background (autonomous) agents tick 24/7 with short turns; the
/// codex_economy message-count threshold (14) made them run an LLM
/// summarization every other tick — observed live on researcher-hand: one
/// compaction per 2 minutes, ~200k tokens/hour burned largely on
/// compacting its own ticks. Count-based compaction is spaced out for
/// them; the token threshold still bounds real context growth.
pub(super) fn compaction_config_for_manifest(
    manifest: &captain_types::agent::AgentManifest,
    context_window_tokens: Option<usize>,
) -> captain_runtime::compactor::CompactionConfig {
    let mut config = compaction_config_for_provider_with_context(
        &manifest.model.provider,
        context_window_tokens,
    );
    if manifest.autonomous.is_some() {
        config.threshold = config.threshold.max(48);
        config.keep_recent = config.keep_recent.max(8);
    }
    config
}

pub(super) fn context_window_for_model(
    catalog: &captain_runtime::model_catalog::ModelCatalog,
    provider: &str,
    configured_model: &str,
) -> Option<usize> {
    let configured = configured_model.trim();
    if configured.is_empty() {
        return None;
    }

    let prefixed = if configured.contains('/') {
        configured.to_string()
    } else {
        format!("{}/{configured}", provider.trim())
    };
    let bare = strip_provider_prefix_for_background(configured);

    catalog
        .find_model(configured)
        .or_else(|| catalog.find_model(&prefixed))
        .or_else(|| catalog.find_model(bare))
        .map(|entry| entry.context_window as usize)
        .filter(|window| *window > 0)
}

pub(super) fn prompt_profile_for_provider(
    provider: &str,
) -> captain_runtime::prompt_builder::PromptProfile {
    if provider.eq_ignore_ascii_case("codex") || provider.eq_ignore_ascii_case("openai-codex") {
        captain_runtime::prompt_builder::PromptProfile::CodexEconomy
    } else {
        captain_runtime::prompt_builder::PromptProfile::Full
    }
}

pub(super) fn apply_agent_loop_config(manifest: &mut AgentManifest, config: &KernelConfig) {
    manifest.metadata.insert(
        AGENT_LOOP_MAX_ITERATIONS_KEY.to_string(),
        serde_json::json!(config.agent_loop.effective_max_iterations()),
    );
}

pub(super) fn subagent_depth_from_manifest(manifest: &AgentManifest) -> u64 {
    manifest
        .metadata
        .get("subagent_depth")
        .and_then(|v| v.as_u64())
        .unwrap_or(0)
}

pub(super) fn apply_subagent_lineage_metadata(
    manifest: &mut AgentManifest,
    parent: Option<AgentId>,
    parent_entry: Option<&AgentEntry>,
) {
    let Some(parent_id) = parent else {
        return;
    };

    let parent_depth = parent_entry
        .map(|entry| subagent_depth_from_manifest(&entry.manifest))
        .unwrap_or(0);
    let root_agent_id = parent_entry
        .and_then(|entry| entry.manifest.metadata.get("root_agent_id"))
        .and_then(|v| v.as_str())
        .map(ToOwned::to_owned)
        .unwrap_or_else(|| parent_id.to_string());

    manifest
        .metadata
        .insert("is_subagent".to_string(), serde_json::json!(true));
    manifest.metadata.insert(
        "parent_agent_id".to_string(),
        serde_json::json!(parent_id.to_string()),
    );
    manifest.metadata.insert(
        "root_agent_id".to_string(),
        serde_json::json!(root_agent_id),
    );
    manifest.metadata.insert(
        "subagent_depth".to_string(),
        serde_json::json!(parent_depth.saturating_add(1)),
    );
}

fn push_unique_tool(tools: &mut Vec<String>, tool: &str) {
    let normalized = normalize_tool_name(tool);
    if !tools
        .iter()
        .any(|existing| normalize_tool_name(existing) == normalized)
    {
        tools.push(normalized.to_string());
    }
}

fn explicit_or_profile_tools_for_subagent(manifest: &AgentManifest) -> Vec<String> {
    let mut tools = Vec::new();

    if !manifest.tool_allowlist.is_empty() && !manifest.tool_allowlist.iter().any(|t| t == "*") {
        for tool in &manifest.tool_allowlist {
            push_unique_tool(&mut tools, tool);
        }
        return tools;
    }

    if !manifest.capabilities.tools.is_empty()
        && !manifest.capabilities.tools.iter().any(|t| t == "*")
    {
        for tool in &manifest.capabilities.tools {
            push_unique_tool(&mut tools, tool);
        }
        return tools;
    }

    if let Some(profile) = manifest.profile.as_ref() {
        if !matches!(profile, ToolProfile::Full | ToolProfile::Custom) {
            for tool in profile.tools() {
                push_unique_tool(&mut tools, &tool);
            }
            return tools;
        }
    }

    for tool in ToolProfile::Minimal.tools() {
        push_unique_tool(&mut tools, &tool);
    }
    tools
}

fn grant_capability_implications_for_tools(caps: &mut ManifestCapabilities, tools: &[String]) {
    let has_tool = |name: &str| tools.iter().any(|t| t == name || t == "*");
    if tools
        .iter()
        .any(|t| t == "*" || t.starts_with("web_") || t == "browser_batch")
        && !caps.network.iter().any(|host| host == "*")
    {
        caps.network.push("*".to_string());
    }
    if has_tool("shell_exec") && !caps.shell.iter().any(|scope| scope == "*") {
        caps.shell.push("*".to_string());
    }
    if tools.iter().any(|t| t == "*" || t.starts_with("agent_")) {
        caps.agent_spawn = caps.agent_spawn || has_tool("agent_spawn") || has_tool("*");
        if !caps.agent_message.iter().any(|scope| scope == "*") {
            caps.agent_message.push("*".to_string());
        }
    }
    if tools
        .iter()
        .any(|t| t == "*" || t == "memory_recall" || t == "memory_store")
        && caps.memory_read.is_empty()
    {
        caps.memory_read.push("self.*".to_string());
    }
    if tools
        .iter()
        .any(|t| t == "*" || t == "memory_save" || t == "memory_store")
        && caps.memory_write.is_empty()
    {
        caps.memory_write.push("self.*".to_string());
    }
}

pub(super) fn normalize_subagent_tool_scope(manifest: &mut AgentManifest) {
    let mut tools = explicit_or_profile_tools_for_subagent(manifest);
    for tool in SUBAGENT_DEFAULT_TOOLS {
        push_unique_tool(&mut tools, tool);
    }

    let mut caps = if manifest.capabilities.tools.is_empty() {
        manifest
            .profile
            .as_ref()
            .map(ToolProfile::implied_capabilities)
            .unwrap_or_else(|| manifest.capabilities.clone())
    } else {
        manifest.capabilities.clone()
    };
    grant_capability_implications_for_tools(&mut caps, &tools);
    caps.tools = tools.clone();
    manifest.capabilities = caps;
    manifest.tool_allowlist = tools;
}

#[cfg(test)]
mod compaction_profile_tests {
    use super::*;
    use captain_types::agent::{AgentManifest, AutonomousConfig};

    fn codex_manifest(autonomous: bool) -> AgentManifest {
        let mut manifest = AgentManifest::default();
        manifest.model.provider = "codex".to_string();
        if autonomous {
            manifest.autonomous = Some(AutonomousConfig::default());
        }
        manifest
    }

    /// Live: researcher-hand (autonomous, codex threshold 14) ran an LLM
    /// compaction every other tick. Background agents get a spaced-out
    /// count threshold; interactive agents keep the economy profile.
    #[test]
    fn autonomous_agents_get_spaced_count_compaction() {
        let interactive = compaction_config_for_manifest(&codex_manifest(false), Some(272_000));
        assert_eq!(interactive.threshold, 14);
        assert_eq!(interactive.keep_recent, 6);

        let background = compaction_config_for_manifest(&codex_manifest(true), Some(272_000));
        assert_eq!(background.threshold, 48);
        assert_eq!(background.keep_recent, 8);
        assert_eq!(background.context_window_tokens, 272_000);
    }
}
