use crate::llm_driver::CacheHints;
use captain_memory::session::Session;
use captain_types::agent::AgentManifest;
use captain_types::config::AGENT_LOOP_MAX_ITERATIONS_HARD_CAP;

/// Maximum iterations in the agent loop before giving up (v3.8g: bumped 50→90).
/// Conservative default; autonomous.max_iterations still overrides.
const MAX_ITERATIONS: u32 = 90;
pub const AGENT_LOOP_MAX_ITERATIONS_KEY: &str = "agent_loop_max_iterations";

const SYSTEM_CACHE_PREFIX_BYTES_KEY: &str = "system_cache_prefix_bytes";
const LEAN_DIRECT_TURN_KEY: &str = "lean_direct_turn";

pub(crate) fn manifest_lean_direct_turn(manifest: &AgentManifest) -> bool {
    manifest
        .metadata
        .get(LEAN_DIRECT_TURN_KEY)
        .and_then(|v| v.as_bool())
        .unwrap_or(false)
}

pub(crate) fn max_iterations_for_manifest(manifest: &AgentManifest) -> u32 {
    let configured = manifest
        .autonomous
        .as_ref()
        .map(|a| a.max_iterations)
        .or_else(|| {
            manifest
                .metadata
                .get(AGENT_LOOP_MAX_ITERATIONS_KEY)
                .and_then(|v| v.as_u64())
                .and_then(|n| u32::try_from(n).ok())
        })
        .unwrap_or(MAX_ITERATIONS);

    configured.clamp(1, AGENT_LOOP_MAX_ITERATIONS_HARD_CAP)
}

fn cache_hints_for_manifest(manifest: &AgentManifest) -> CacheHints {
    let cacheable_system_prefix_bytes = manifest
        .metadata
        .get(SYSTEM_CACHE_PREFIX_BYTES_KEY)
        .and_then(|v| v.as_u64())
        .and_then(|n| usize::try_from(n).ok());

    CacheHints::for_provider(&manifest.model.provider)
        .with_system_prefix_bytes(cacheable_system_prefix_bytes)
}

pub(crate) fn cache_hints_for_session(manifest: &AgentManifest, session: &Session) -> CacheHints {
    cache_hints_for_manifest(manifest)
        .with_prompt_cache_key(Some(format!("captain-session-{}", session.id.0)))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_session() -> Session {
        Session {
            id: captain_types::agent::SessionId::new(),
            agent_id: captain_types::agent::AgentId::new(),
            messages: Vec::new(),
            context_window_tokens: 0,
            label: None,
        }
    }

    #[test]
    fn default_max_iterations_stays_hermes_90() {
        assert_eq!(MAX_ITERATIONS, 90);
        assert_eq!(max_iterations_for_manifest(&AgentManifest::default()), 90);
    }

    #[test]
    fn max_iterations_can_be_overridden_by_manifest_metadata() {
        let mut manifest = AgentManifest::default();
        manifest.metadata.insert(
            AGENT_LOOP_MAX_ITERATIONS_KEY.to_string(),
            serde_json::json!(180),
        );
        assert_eq!(max_iterations_for_manifest(&manifest), 180);
    }

    #[test]
    fn max_iterations_prefers_autonomous_agent_override() {
        let mut manifest = AgentManifest::default();
        manifest.metadata.insert(
            AGENT_LOOP_MAX_ITERATIONS_KEY.to_string(),
            serde_json::json!(180),
        );
        manifest.autonomous = Some(captain_types::agent::AutonomousConfig {
            max_iterations: 240,
            ..Default::default()
        });
        assert_eq!(max_iterations_for_manifest(&manifest), 240);
    }

    #[test]
    fn max_iterations_is_clamped_to_runtime_guardrail() {
        let mut manifest = AgentManifest::default();
        manifest.metadata.insert(
            AGENT_LOOP_MAX_ITERATIONS_KEY.to_string(),
            serde_json::json!(2000),
        );
        assert_eq!(
            max_iterations_for_manifest(&manifest),
            AGENT_LOOP_MAX_ITERATIONS_HARD_CAP
        );
    }

    #[test]
    fn max_iterations_clamps_zero_to_one() {
        let mut manifest = AgentManifest::default();
        manifest.metadata.insert(
            AGENT_LOOP_MAX_ITERATIONS_KEY.to_string(),
            serde_json::json!(0),
        );
        assert_eq!(max_iterations_for_manifest(&manifest), 1);
    }

    #[test]
    fn lean_direct_turn_defaults_false_and_reads_boolean_metadata() {
        let mut manifest = AgentManifest::default();
        assert!(!manifest_lean_direct_turn(&manifest));

        manifest
            .metadata
            .insert(LEAN_DIRECT_TURN_KEY.to_string(), serde_json::json!(true));
        assert!(manifest_lean_direct_turn(&manifest));
    }

    #[test]
    fn cache_hints_add_session_key_and_optional_system_prefix() {
        let mut manifest = AgentManifest::default();
        manifest.model.provider = "anthropic".to_string();
        manifest.metadata.insert(
            SYSTEM_CACHE_PREFIX_BYTES_KEY.to_string(),
            serde_json::json!(1234),
        );
        let session = test_session();

        let hints = cache_hints_for_session(&manifest, &session);

        assert!(hints.cache_system);
        assert!(hints.cache_tools);
        assert_eq!(hints.cacheable_system_prefix_bytes, Some(1234));
        assert_eq!(
            hints.prompt_cache_key,
            Some(format!("captain-session-{}", session.id.0))
        );
    }

    #[test]
    fn cache_hints_keep_non_anthropic_breakpoints_disabled() {
        let mut manifest = AgentManifest::default();
        manifest.model.provider = "codex".to_string();
        let hints = cache_hints_for_session(&manifest, &test_session());

        assert!(!hints.cache_system);
        assert!(!hints.cache_tools);
        assert!(hints.prompt_cache_key.is_some());
    }
}
