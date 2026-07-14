use crate::embedding::EmbeddingDriver;
use captain_memory::MemorySubstrate;
use captain_types::agent::AgentId;
use captain_types::memory::{Memory, MemoryFilter, MemoryFragment};
use tracing::{debug, warn};

const AGENT_RECALL_LIMIT: usize = 3;
const SHARED_RECALL_LIMIT: usize = 2;

pub(crate) async fn recall_turn_memories(
    user_message: &str,
    agent_id: AgentId,
    memory: &MemorySubstrate,
    embedding_driver: Option<&(dyn EmbeddingDriver + Send + Sync)>,
    lean_direct_turn: bool,
    streaming: bool,
) -> Vec<MemoryFragment> {
    if lean_direct_turn {
        return Vec::new();
    }

    if let Some(emb) = embedding_driver {
        match emb.embed_one(user_message).await {
            Ok(query_vec) => {
                if streaming {
                    debug!("Using vector recall (streaming, dims={})", query_vec.len());
                } else {
                    debug!("Using vector recall (dims={})", query_vec.len());
                }
                return recall_with_vector(user_message, agent_id, memory, &query_vec, streaming)
                    .await;
            }
            Err(e) => {
                if streaming {
                    warn!("Embedding failed (streaming), falling back to text search: {e}");
                } else {
                    warn!("Embedding failed, falling back to text search: {e}");
                }
            }
        }
    }

    recall_with_text(user_message, agent_id, memory, streaming).await
}

async fn recall_with_vector(
    user_message: &str,
    agent_id: AgentId,
    memory: &MemorySubstrate,
    query_vec: &[f32],
    streaming: bool,
) -> Vec<MemoryFragment> {
    let agent_memories = memory
        .recall_with_embedding_async(
            user_message,
            AGENT_RECALL_LIMIT,
            Some(agent_memory_filter(agent_id)),
            Some(query_vec),
        )
        .await
        .unwrap_or_else(|e| {
            warn_recall_failure("Vector", "agent", streaming, &e.to_string());
            vec![]
        });
    let shared_memories = memory
        .recall_with_embedding_async(
            user_message,
            SHARED_RECALL_LIMIT,
            Some(shared_memory_filter()),
            Some(query_vec),
        )
        .await
        .unwrap_or_else(|e| {
            warn_recall_failure("Vector", "shared", streaming, &e.to_string());
            vec![]
        });
    combine_recalled(agent_memories, shared_memories)
}

async fn recall_with_text(
    user_message: &str,
    agent_id: AgentId,
    memory: &MemorySubstrate,
    streaming: bool,
) -> Vec<MemoryFragment> {
    let agent_memories = memory
        .recall(
            user_message,
            AGENT_RECALL_LIMIT,
            Some(agent_memory_filter(agent_id)),
        )
        .await
        .unwrap_or_else(|e| {
            warn_recall_failure("Text", "agent", streaming, &e.to_string());
            vec![]
        });
    let shared_memories = memory
        .recall(
            user_message,
            SHARED_RECALL_LIMIT,
            Some(shared_memory_filter()),
        )
        .await
        .unwrap_or_else(|e| {
            warn_recall_failure("Text", "shared", streaming, &e.to_string());
            vec![]
        });
    combine_recalled(agent_memories, shared_memories)
}

fn agent_memory_filter(agent_id: AgentId) -> MemoryFilter {
    MemoryFilter {
        agent_id: Some(agent_id),
        ..Default::default()
    }
}

fn shared_memory_filter() -> MemoryFilter {
    MemoryFilter {
        scope: Some("explicit".to_string()),
        ..Default::default()
    }
}

fn combine_recalled(
    mut agent_memories: Vec<MemoryFragment>,
    shared_memories: Vec<MemoryFragment>,
) -> Vec<MemoryFragment> {
    agent_memories.extend(shared_memories);
    agent_memories
}

fn warn_recall_failure(kind: &str, scope: &str, streaming: bool, error: &str) {
    if streaming {
        warn!("{kind} recall ({scope}, streaming) failed: {error}");
    } else {
        warn!("{kind} recall ({scope}) failed: {error}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::embedding::EmbeddingError;
    use async_trait::async_trait;
    use captain_types::memory::MemorySource;
    use std::collections::HashMap;

    #[tokio::test]
    async fn lean_direct_turn_skips_memory_recall() {
        let memory = MemorySubstrate::open_in_memory(0.01).unwrap();
        let agent_id = AgentId::new();
        memory
            .remember(
                agent_id,
                "alpha agent memory",
                MemorySource::Conversation,
                "episodic",
                HashMap::new(),
            )
            .await
            .unwrap();

        let recalled = recall_turn_memories("alpha", agent_id, &memory, None, true, false).await;

        assert!(recalled.is_empty());
    }

    #[tokio::test]
    async fn text_recall_combines_agent_and_explicit_shared_memories() {
        let memory = MemorySubstrate::open_in_memory(0.01).unwrap();
        let agent_id = AgentId::new();
        memory
            .remember(
                agent_id,
                "alpha agent memory",
                MemorySource::Conversation,
                "episodic",
                HashMap::new(),
            )
            .await
            .unwrap();
        memory
            .remember(
                AgentId::new(),
                "alpha shared memory",
                MemorySource::UserProvided,
                "explicit",
                HashMap::new(),
            )
            .await
            .unwrap();

        let recalled = recall_turn_memories("alpha", agent_id, &memory, None, false, true).await;
        let contents = recalled
            .iter()
            .map(|m| m.content.as_str())
            .collect::<Vec<_>>();

        assert!(contents.contains(&"alpha agent memory"));
        assert!(contents.contains(&"alpha shared memory"));
    }

    struct StaticEmbeddingDriver;

    #[async_trait]
    impl EmbeddingDriver for StaticEmbeddingDriver {
        async fn embed(&self, texts: &[&str]) -> Result<Vec<Vec<f32>>, EmbeddingError> {
            Ok(texts.iter().map(|_| vec![1.0, 0.0]).collect())
        }

        fn dimensions(&self) -> usize {
            2
        }
    }

    #[tokio::test]
    async fn vector_recall_uses_embedding_driver_when_available() {
        let memory = MemorySubstrate::open_in_memory(0.01).unwrap();
        let agent_id = AgentId::new();
        memory
            .remember_with_embedding_async(
                agent_id,
                "beta vector memory",
                MemorySource::Conversation,
                "episodic",
                HashMap::new(),
                Some(&[1.0, 0.0]),
            )
            .await
            .unwrap();
        let driver = StaticEmbeddingDriver;

        let recalled =
            recall_turn_memories("beta", agent_id, &memory, Some(&driver), false, false).await;

        assert!(recalled.iter().any(|m| m.content == "beta vector memory"));
    }
}
