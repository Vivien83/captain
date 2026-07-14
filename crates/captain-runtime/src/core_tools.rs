//! CORE tools — the tools always visible to the LLM.
//!
//! All other builtin tools are *deferred*: the LLM must call `tool_search`
//! to discover them on demand. This mirrors Claude Code's tool surface
//! design (Option C, validated 2026-04-29).
//!
//! Selection criterion: universal foundation only. Expensive domain tools
//! (file, shell, SSH, browser, media, scheduling, config, etc.) are deferred
//! behind `capability_search`/`tool_search` and dynamically surfaced when the
//! agent proves it needs them. This keeps cheap conversational turns cheap
//! without removing any actual capability.

/// Tools that stay in the agent's prompt on every turn, regardless of any
/// semantic ranking or tool-allowlist filtering at the runtime layer.
///
/// Order is grouped by family for readability. The exact ordering is not
/// load-bearing — `is_core_tool` does a contains check.
pub const CORE_TOOLS: &[&str] = &[
    // Discovery / recovery. These are the product safety net: at the first
    // meaningful doubt about capability availability, the agent searches
    // instead of guessing or asking the user prematurely.
    "capability_search",
    "skill_search",
    "skill_view",
    "tool_search",
    "captain_docs",
    "ask_user",
    // Memory. Kept visible because personal-assistant quality depends on
    // remembering and recalling facts without an extra discovery hop.
    "memory_context_batch",
    "memory_save",
    "memory_recall",
    // Lightweight context / clock.
    "session_recall",
    // Projects. Read-only project state is core continuity context; mutations
    // remain deferred so normal chat does not expose the whole project surface.
    "project_list",
    "project_get",
    "system_time",
    // Native voice. Installed by default so voice turns do not waste a turn
    // rediscovering STT/TTS every time, especially on Telegram.
    "speech_to_text",
    "text_to_speech",
    "channel_send",
];

/// Tools every sub-agent must keep, even with a narrow execution allowlist.
/// They are discovery/control-plane tools, not broad execution privileges.
pub const SUBAGENT_DEFAULT_TOOLS: &[&str] = &[
    "capability_search",
    "skill_search",
    "skill_view",
    "tool_search",
    "captain_docs",
    "system_time",
];

/// `true` iff `name` is in the CORE set (always visible to the LLM).
pub fn is_core_tool(name: &str) -> bool {
    CORE_TOOLS.contains(&name)
}

/// `true` iff `name` is a builtin tool that is *not* in CORE — i.e. it must
/// be retrieved on demand via `tool_search`.
///
/// Note: this returns `true` for any string that isn't in CORE, including
/// non-builtin names. Callers that want to enforce "deferred *and* builtin"
/// should intersect with `builtin_tool_definitions()`.
pub fn is_deferred_tool_name(name: &str) -> bool {
    !is_core_tool(name)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    #[test]
    fn core_count_is_16() {
        assert_eq!(
            CORE_TOOLS.len(),
            16,
            "CORE list must stay small; deferred tools/skills are surfaced by discovery tools"
        );
    }

    #[test]
    fn no_duplicates_in_core() {
        let set: HashSet<&&str> = CORE_TOOLS.iter().collect();
        assert_eq!(
            set.len(),
            CORE_TOOLS.len(),
            "duplicate tool name in CORE_TOOLS"
        );
    }

    #[test]
    fn is_core_for_known_names() {
        assert!(is_core_tool("capability_search"));
        assert!(is_core_tool("skill_search"));
        assert!(is_core_tool("skill_view"));
        assert!(is_core_tool("tool_search"));
        assert!(is_core_tool("captain_docs"));
        assert!(is_core_tool("memory_context_batch"));
        assert!(is_core_tool("memory_save"));
        assert!(is_core_tool("memory_recall"));
        assert!(is_core_tool("session_recall"));
        assert!(is_core_tool("project_list"));
        assert!(is_core_tool("project_get"));
        assert!(is_core_tool("system_time"));
        assert!(is_core_tool("speech_to_text"));
        assert!(is_core_tool("text_to_speech"));
        assert!(is_core_tool("channel_send"));
        assert!(!is_core_tool("file_read"));
        assert!(!is_core_tool("ssh_exec"));
        assert!(!is_core_tool("self_improvement_review"));
        assert!(!is_core_tool("memory_store"));
        assert!(!is_core_tool("browser_navigate"));
        assert!(!is_core_tool("nonexistent_tool"));
    }

    #[test]
    fn tool_search_must_be_in_core() {
        // tool_search is the entry point for deferred tools — without it
        // the LLM cannot discover anything beyond CORE. Pinning it via
        // a dedicated test prevents accidental removal.
        assert!(
            CORE_TOOLS.contains(&"tool_search"),
            "tool_search MUST be in CORE — it's the entry point for deferred tools"
        );
    }

    #[test]
    fn skill_search_must_be_in_core() {
        assert!(
            CORE_TOOLS.contains(&"skill_search"),
            "skill_search MUST be in CORE — it routes procedural skill discovery"
        );
    }

    #[test]
    fn skill_view_must_be_in_core() {
        assert!(
            CORE_TOOLS.contains(&"skill_view"),
            "skill_view MUST be in CORE — it loads exact procedural guidance after skill_search"
        );
    }

    #[test]
    fn capability_search_must_be_in_core() {
        // capability_search is the unified resolver across builtin tools,
        // skills, Hands, MCP, and docs. Without it, the agent falls back to
        // asking the user or guessing which discovery surface applies.
        assert!(
            CORE_TOOLS.contains(&"capability_search"),
            "capability_search MUST be in CORE — it routes capability discovery"
        );
    }

    #[test]
    fn subagent_default_tools_are_core_discovery_tools() {
        assert!(SUBAGENT_DEFAULT_TOOLS.contains(&"capability_search"));
        assert!(SUBAGENT_DEFAULT_TOOLS.contains(&"skill_search"));
        assert!(SUBAGENT_DEFAULT_TOOLS.contains(&"skill_view"));
        assert!(SUBAGENT_DEFAULT_TOOLS.contains(&"tool_search"));
        for name in SUBAGENT_DEFAULT_TOOLS {
            assert!(
                CORE_TOOLS.contains(name),
                "sub-agent default tool '{name}' must remain in CORE"
            );
        }
    }

    #[test]
    fn captain_docs_must_be_in_core() {
        // RTFM gate: agent must be able to read its own manual without
        // an extra discovery hop.
        assert!(CORE_TOOLS.contains(&"captain_docs"));
    }

    #[test]
    fn ask_user_must_be_in_core() {
        // ask_user is intercepted specially in agent_loop; if it's not
        // visible, the agent loses its only escape hatch.
        assert!(CORE_TOOLS.contains(&"ask_user"));
    }

    #[test]
    fn is_deferred_is_inverse_of_core() {
        for name in CORE_TOOLS {
            assert!(!is_deferred_tool_name(name), "{name} is core, not deferred");
        }
        assert!(is_deferred_tool_name("browser_navigate"));
        assert!(!is_deferred_tool_name("text_to_speech"));
    }
}
