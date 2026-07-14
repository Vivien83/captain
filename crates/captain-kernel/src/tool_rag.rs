//! Tool RAG — Semantic tool retrieval via embedding similarity.
//!
//! At boot, all builtin tool descriptions are embedded in a background task.
//! Per request, the user query is embedded and compared to cached tool embeddings
//! via cosine similarity. Only the top-K most relevant tools are sent to the LLM.

use captain_runtime::embedding::{cosine_similarity, EmbeddingDriver};
use captain_types::tool::ToolDefinition;
use std::sync::Arc;
use tokio::sync::RwLock;
use tracing::{info, warn};

/// Default number of tools to return from semantic ranking.
pub const DEFAULT_TOP_K: usize = 8;

/// Minimum similarity threshold — tools below this are excluded even if in top-K.
pub const MIN_SIMILARITY_THRESHOLD: f32 = 0.15;

/// Tools that must always be included regardless of semantic ranking.
///
/// Captain's core toolkit needs to stay visible to the LLM on every turn —
/// otherwise an utterance like "mémorise X" or "le serveur va bien ?" gets
/// answered as if those capabilities did not exist (because `memory_save`
/// or `ssh_exec` were ranked out of the top-K). RAG only narrows situational
/// tools (web, image, mcp…); the agent's foundational verbs are pinned here.
const ALWAYS_INCLUDE: &[&str] = &[
    // Memory — declarative learning (Phase 1)
    "memory_save",
    "memory_recall",
    "memory_store",
    // Local system
    "shell_exec",
    "secret_read",
    "config_read",
    // Remote access
    "ssh_exec",
    "ssh_upload",
    "ssh_download",
    // User-facing communication
    "ask_user",
    "channel_send",
    // Multi-agent coordination
    "agent_send",
    "agent_list",
    // Scheduling
    "cron_create",
    "goal_create",
    // Knowledge graph
    "knowledge_query",
    // Self-extensibility: agent can ship new skills mid-session
    "scaffold_skill",
    // Captain-only: extend authorized workspace roots
    "workspace_add",
    // Cross-session recall over auto-generated checkpoint.md files
    "session_recall",
    // Retraction of incorrect facts that memory_save / reflection wrote
    "memory_forget",
];

struct ToolEmbeddingEntry {
    tool_name: String,
    embedding: Vec<f32>,
}

/// Cached tool embeddings for semantic retrieval.
pub struct ToolEmbeddingCache {
    entries: Vec<ToolEmbeddingEntry>,
}

impl ToolEmbeddingCache {
    /// Build cache by embedding tool name + description.
    /// Including the name ensures queries like "create a cron" match `cron_create`
    /// even when its description uses different phrasing.
    async fn build(tools: &[ToolDefinition], driver: &dyn EmbeddingDriver) -> Result<Self, String> {
        let texts: Vec<String> = tools
            .iter()
            .map(|t| format!("{}: {}", t.name, t.description))
            .collect();
        let text_refs: Vec<&str> = texts.iter().map(|s| s.as_str()).collect();
        let embeddings = driver.embed(&text_refs).await.map_err(|e| e.to_string())?;

        let entries = tools
            .iter()
            .zip(embeddings)
            .map(|(tool, emb)| ToolEmbeddingEntry {
                tool_name: tool.name.clone(),
                embedding: emb,
            })
            .collect();

        Ok(Self { entries })
    }

    /// Rank candidate tools by similarity to a query embedding.
    /// Returns top-K tool names sorted by descending similarity.
    fn rank(
        &self,
        query_embedding: &[f32],
        candidates: &[ToolDefinition],
        top_k: usize,
    ) -> Vec<String> {
        let mut scores: Vec<(&str, f32)> = candidates
            .iter()
            .filter_map(|tool| {
                self.entries
                    .iter()
                    .find(|e| e.tool_name == tool.name)
                    .map(|e| {
                        (
                            tool.name.as_str(),
                            cosine_similarity(&e.embedding, query_embedding),
                        )
                    })
            })
            .filter(|(_, score)| *score >= MIN_SIMILARITY_THRESHOLD)
            .collect();

        scores.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
        scores.truncate(top_k);

        scores
            .into_iter()
            .map(|(name, _)| name.to_string())
            .collect()
    }
}

/// Shared handle to the tool embedding cache.
pub type SharedToolCache = Arc<RwLock<Option<ToolEmbeddingCache>>>;

/// Spawn the background task that precomputes tool embeddings at boot.
pub fn spawn_tool_embedding_task(
    cache: SharedToolCache,
    driver: Arc<dyn EmbeddingDriver + Send + Sync>,
) {
    tokio::spawn(async move {
        let tools = captain_runtime::tool_runner::builtin_tool_definitions();
        match ToolEmbeddingCache::build(&tools, driver.as_ref()).await {
            Ok(built) => {
                info!(tool_count = built.entries.len(), "Tool RAG cache ready");
                *cache.write().await = Some(built);
            }
            Err(e) => {
                warn!(error = %e, "Tool RAG cache build failed — falling back to all tools");
            }
        }
    });
}

/// Select tools for a query using semantic ranking (Tool RAG).
///
/// 1. If cache not ready or embedding driver unavailable → return all candidates
/// 2. If candidates <= top_k → return all (no point ranking)
/// 3. Embed query, rank by cosine similarity, return top-K + always-include tools
/// 4. Non-builtin tools (skill/MCP) pass through unranked
pub async fn select_tools_for_query(
    candidates: Vec<ToolDefinition>,
    query: &str,
    top_k: usize,
    cache: &SharedToolCache,
    driver: &Option<Arc<dyn EmbeddingDriver + Send + Sync>>,
) -> Vec<ToolDefinition> {
    // Skip RAG for small tool sets
    if candidates.len() <= top_k {
        return candidates;
    }

    let cache_guard = cache.read().await;
    let cache_ref = match cache_guard.as_ref() {
        Some(c) => c,
        None => return candidates,
    };

    let driver_ref = match driver {
        Some(d) => d,
        None => return candidates,
    };

    let query_emb = match driver_ref.embed_one(query).await {
        Ok(emb) => emb,
        Err(e) => {
            warn!(error = %e, "Tool RAG query embedding failed — using all tools");
            return candidates;
        }
    };

    let selected_names = cache_ref.rank(&query_emb, &candidates, top_k);

    candidates
        .into_iter()
        .filter(|t| {
            // Keep ranked tools
            selected_names.contains(&t.name)
            // Always include communication tools
            || ALWAYS_INCLUDE.contains(&t.name.as_str())
            // Pass through non-builtin tools (skill/MCP — not in cache)
            || !cache_ref.entries.iter().any(|e| e.tool_name == t.name)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mock_tools() -> Vec<ToolDefinition> {
        vec![
            ToolDefinition {
                name: "web_search".into(),
                description: "Search the web".into(),
                input_schema: serde_json::json!({}),
            },
            ToolDefinition {
                name: "file_read".into(),
                description: "Read a file".into(),
                input_schema: serde_json::json!({}),
            },
            ToolDefinition {
                name: "shell_exec".into(),
                description: "Execute shell command".into(),
                input_schema: serde_json::json!({}),
            },
            ToolDefinition {
                name: "ask_user".into(),
                description: "Ask user a question".into(),
                input_schema: serde_json::json!({}),
            },
            ToolDefinition {
                name: "channel_send".into(),
                description: "Send channel message".into(),
                input_schema: serde_json::json!({}),
            },
        ]
    }

    fn mock_cache() -> ToolEmbeddingCache {
        ToolEmbeddingCache {
            entries: vec![
                ToolEmbeddingEntry {
                    tool_name: "web_search".into(),
                    embedding: vec![1.0, 0.0, 0.0],
                },
                ToolEmbeddingEntry {
                    tool_name: "file_read".into(),
                    embedding: vec![0.0, 1.0, 0.0],
                },
                ToolEmbeddingEntry {
                    tool_name: "shell_exec".into(),
                    embedding: vec![0.0, 0.0, 1.0],
                },
                ToolEmbeddingEntry {
                    tool_name: "ask_user".into(),
                    embedding: vec![0.5, 0.5, 0.0],
                },
                ToolEmbeddingEntry {
                    tool_name: "channel_send".into(),
                    embedding: vec![0.3, 0.3, 0.3],
                },
            ],
        }
    }

    #[test]
    fn test_rank_returns_top_k() {
        let cache = mock_cache();
        let tools = mock_tools();
        // Query close to web_search direction
        let query_emb = vec![0.9, 0.1, 0.0];
        let ranked = cache.rank(&query_emb, &tools, 2);
        assert_eq!(ranked.len(), 2);
        assert_eq!(ranked[0], "web_search");
    }

    #[test]
    fn test_rank_threshold_filters() {
        let cache = mock_cache();
        let tools = mock_tools();
        // Query orthogonal to shell_exec
        let query_emb = vec![1.0, 0.0, 0.0];
        let ranked = cache.rank(&query_emb, &tools, 5);
        // shell_exec has embedding [0,0,1] → cosine with [1,0,0] = 0.0 → below threshold
        assert!(!ranked.contains(&"shell_exec".to_string()));
    }

    #[test]
    fn test_always_include_tools() {
        let cache = mock_cache();
        let tools = mock_tools();
        let shared_cache = Arc::new(RwLock::new(Some(cache)));
        let rt = tokio::runtime::Runtime::new().unwrap();
        let result = rt.block_on(select_tools_for_query(
            tools,
            "test",
            1,
            &shared_cache,
            &None, // No driver → fallback to all candidates
        ));
        // No driver → returns all candidates (fallback)
        assert_eq!(result.len(), 5);
    }

    #[test]
    fn test_small_tool_set_skips_rag() {
        let tools = vec![
            ToolDefinition {
                name: "a".into(),
                description: "".into(),
                input_schema: serde_json::json!({}),
            },
            ToolDefinition {
                name: "b".into(),
                description: "".into(),
                input_schema: serde_json::json!({}),
            },
        ];
        let shared_cache: SharedToolCache = Arc::new(RwLock::new(None));
        let rt = tokio::runtime::Runtime::new().unwrap();
        let result = rt.block_on(select_tools_for_query(
            tools.clone(),
            "query",
            8,
            &shared_cache,
            &None,
        ));
        assert_eq!(result.len(), 2); // All returned, no filtering
    }

    #[test]
    fn test_always_include_contains_core_tools() {
        // Captain must keep its core toolkit visible to the LLM regardless of
        // the user query — RAG only narrows situational tools. Without these
        // pinned, "mémorise X" never surfaces memory_save, "le serveur" never
        // surfaces ssh_exec, and the agent fakes capability instead of acting.
        let expected = [
            "memory_save",
            "memory_recall",
            "memory_store",
            "shell_exec",
            "secret_read",
            "config_read",
            "ssh_exec",
            "ssh_upload",
            "ssh_download",
            "ask_user",
            "channel_send",
            "agent_send",
            "agent_list",
            "cron_create",
            "goal_create",
            "knowledge_query",
            "scaffold_skill",
            "workspace_add",
            "session_recall",
            "memory_forget",
        ];
        for tool in &expected {
            assert!(
                ALWAYS_INCLUDE.contains(tool),
                "ALWAYS_INCLUDE missing core tool: {tool}"
            );
        }
    }

    #[test]
    fn test_nonbuiltin_tools_pass_through() {
        let cache = mock_cache();
        let mut tools = mock_tools();
        // Add a skill tool not in cache
        tools.push(ToolDefinition {
            name: "my_skill_tool".into(),
            description: "Custom skill".into(),
            input_schema: serde_json::json!({}),
        });

        let shared_cache = Arc::new(RwLock::new(Some(cache)));

        // We can't easily test with a real driver, but verify the filter logic directly
        let cache_guard = shared_cache.try_read().unwrap();
        let cache_ref = cache_guard.as_ref().unwrap();
        let query_emb = vec![1.0, 0.0, 0.0];
        let selected = cache_ref.rank(&query_emb, &tools, 1);
        // my_skill_tool is not in cache entries, so rank ignores it
        assert!(!selected.contains(&"my_skill_tool".to_string()));
        // But in select_tools_for_query, non-cached tools pass through
        let skill_tool = &tools[5];
        let in_cache = cache_ref
            .entries
            .iter()
            .any(|e| e.tool_name == skill_tool.name);
        assert!(!in_cache); // Confirms it would pass through
    }
}
