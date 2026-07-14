use crate::agent_loop_codex_request::is_codex_provider;
use crate::agent_loop_context::{
    append_recalled_memory_context, append_runtime_awareness_context,
    append_runtime_config_context, append_runtime_context,
};
use crate::kernel_handle::KernelHandle;
use crate::memory_retractions::MemoryRetraction;
use captain_types::agent::AgentManifest;
use captain_types::memory::MemoryFragment;
use std::sync::Arc;

pub(crate) async fn build_turn_system_prompt(
    manifest: &AgentManifest,
    kernel: Option<&Arc<dyn KernelHandle>>,
    user_message: &str,
    memories: &[MemoryFragment],
    memory_retractions: &[MemoryRetraction],
    lean_direct_turn: bool,
) -> String {
    let mut system_prompt = manifest.model.system_prompt.clone();
    let compact_codex_context = is_codex_provider(&manifest.model.provider) && !lean_direct_turn;

    if compact_codex_context {
        append_runtime_context(
            &mut system_prompt,
            manifest,
            kernel,
            user_message,
            memory_retractions,
            true,
        )
        .await;
        append_recalled_memory_context(&mut system_prompt, memories, memory_retractions, true);
        return system_prompt;
    }

    if !lean_direct_turn {
        append_runtime_config_context(&mut system_prompt, kernel).await;
    }

    append_recalled_memory_context(&mut system_prompt, memories, memory_retractions, false);

    if !lean_direct_turn {
        append_runtime_awareness_context(
            &mut system_prompt,
            manifest,
            kernel,
            user_message,
            memory_retractions,
        );
    }

    system_prompt
}

#[cfg(test)]
mod tests {
    use super::*;
    use captain_types::agent::AgentId;
    use captain_types::memory::{MemoryId, MemorySource};
    use chrono::Utc;
    use std::collections::HashMap;

    fn test_manifest(provider: &str) -> AgentManifest {
        let mut manifest = AgentManifest::default();
        manifest.name = "captain".to_string();
        manifest.model.provider = provider.to_string();
        manifest.model.system_prompt = "base prompt".to_string();
        manifest
    }

    fn memory(content: &str) -> MemoryFragment {
        MemoryFragment {
            id: MemoryId::new(),
            agent_id: AgentId::new(),
            content: content.to_string(),
            embedding: None,
            metadata: HashMap::new(),
            source: MemorySource::Conversation,
            confidence: 1.0,
            created_at: Utc::now(),
            accessed_at: Utc::now(),
            access_count: 0,
            scope: "test".to_string(),
        }
    }

    #[tokio::test]
    async fn lean_direct_turn_keeps_base_prompt_without_dynamic_inputs() {
        let prompt =
            build_turn_system_prompt(&test_manifest("anthropic"), None, "hello", &[], &[], true)
                .await;

        assert_eq!(prompt, "base prompt");
    }

    #[tokio::test]
    async fn non_codex_turn_adds_runtime_clock_before_recalled_memory() {
        let memories = vec![memory("project fact")];

        let prompt = build_turn_system_prompt(
            &test_manifest("anthropic"),
            None,
            "hello",
            &memories,
            &[],
            false,
        )
        .await;

        let clock_pos = prompt.find("Current time:").unwrap();
        let memory_pos = prompt.find("### Recalled memories").unwrap();
        assert!(clock_pos < memory_pos);
        assert!(prompt.contains("project fact"));
    }

    #[tokio::test]
    async fn codex_turn_uses_compact_runtime_capsule_before_memory_capsule() {
        let memories = vec![memory("compact fact")];

        let prompt = build_turn_system_prompt(
            &test_manifest("codex"),
            None,
            "hello",
            &memories,
            &[],
            false,
        )
        .await;

        let runtime_pos = prompt.find("## Runtime Context Capsule").unwrap();
        let memory_pos = prompt.find("## Retrieved Memory Capsule").unwrap();
        assert!(runtime_pos < memory_pos);
        assert!(prompt.contains("now:"));
        assert!(!prompt.contains("Current time:"));
    }
}
