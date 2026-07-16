use crate::embedding::EmbeddingDriver;
use crate::kernel_handle::KernelHandle;
use captain_memory::MemorySubstrate;
use captain_types::agent::AgentId;
use captain_types::memory::{Memory, MemoryFilter, MemoryFragment, MemoryId, MemorySource};
use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use tracing::{debug, warn};

const AGENT_RECALL_LIMIT: usize = 3;
const SHARED_RECALL_LIMIT: usize = 2;
const LOCAL_JOURNAL_RECALL_LIMIT: usize = 3;
const TOTAL_RECALL_LIMIT: usize = 5;

pub(crate) async fn recall_turn_memories(
    user_message: &str,
    agent_id: AgentId,
    memory: &MemorySubstrate,
    kernel: Option<&Arc<dyn KernelHandle>>,
    memory_retractions: &[crate::memory_retractions::MemoryRetraction],
    embedding_driver: Option<&(dyn EmbeddingDriver + Send + Sync)>,
    lean_direct_turn: bool,
    streaming: bool,
) -> Vec<MemoryFragment> {
    if lean_direct_turn {
        return Vec::new();
    }

    let recalled = if let Some(emb) = embedding_driver {
        match emb.embed_one(user_message).await {
            Ok(query_vec) => {
                if streaming {
                    debug!("Using vector recall (streaming, dims={})", query_vec.len());
                } else {
                    debug!("Using vector recall (dims={})", query_vec.len());
                }
                recall_with_vector(user_message, agent_id, memory, &query_vec, streaming).await
            }
            Err(e) => {
                if streaming {
                    warn!("Embedding failed (streaming), falling back to text search: {e}");
                } else {
                    warn!("Embedding failed, falling back to text search: {e}");
                }
                recall_with_text(user_message, agent_id, memory, streaming).await
            }
        }
    } else {
        recall_with_text(user_message, agent_id, memory, streaming).await
    };

    let local = recall_local_journal(
        user_message,
        agent_id,
        kernel,
        memory_retractions,
        streaming,
    );
    combine_prioritized(local, recalled)
}

fn recall_local_journal(
    user_message: &str,
    agent_id: AgentId,
    kernel: Option<&Arc<dyn KernelHandle>>,
    memory_retractions: &[crate::memory_retractions::MemoryRetraction],
    streaming: bool,
) -> Vec<MemoryFragment> {
    match crate::tools::memory_context::recall_local_memory_write_rows(
        user_message,
        kernel,
        LOCAL_JOURNAL_RECALL_LIMIT,
        memory_retractions,
    ) {
        Ok(rows) => rows
            .into_iter()
            .map(|row| memory_write_fragment(row, agent_id))
            .collect(),
        Err(error) => {
            warn_recall_failure("Local journal", "shared", streaming, &error);
            Vec::new()
        }
    }
}

fn memory_write_fragment(
    row: captain_memory::memory_writer::MemoryWrite,
    agent_id: AgentId,
) -> MemoryFragment {
    let created_at = chrono::DateTime::<chrono::Utc>::from_timestamp_millis(row.created_at)
        .unwrap_or_else(chrono::Utc::now);
    let mut metadata = HashMap::new();
    metadata.insert(
        "memory_write_id".to_string(),
        serde_json::Value::String(row.id.clone()),
    );
    metadata.insert(
        "memory_write_source".to_string(),
        serde_json::Value::String(row.source),
    );
    metadata.insert(
        "sync_status".to_string(),
        serde_json::Value::String(row.sync_status.as_str().to_string()),
    );
    MemoryFragment {
        id: MemoryId(uuid::Uuid::parse_str(&row.id).unwrap_or_else(|_| uuid::Uuid::new_v4())),
        agent_id,
        content: format!(
            "Durable fact [{}]: {} {} {}",
            created_at.to_rfc3339(),
            row.subject,
            row.predicate,
            row.object
        ),
        embedding: None,
        metadata,
        source: MemorySource::UserProvided,
        confidence: 1.0,
        created_at,
        accessed_at: created_at,
        access_count: 1,
        scope: "explicit".to_string(),
    }
}

fn combine_prioritized(
    local_memories: Vec<MemoryFragment>,
    recalled_memories: Vec<MemoryFragment>,
) -> Vec<MemoryFragment> {
    let mut seen = HashSet::new();
    local_memories
        .into_iter()
        .chain(recalled_memories)
        .filter(|memory| seen.insert(memory.content.trim().to_lowercase()))
        .take(TOTAL_RECALL_LIMIT)
        .collect()
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
    use rusqlite::Connection;
    use std::sync::Mutex;

    struct MemoryKernelStub {
        conn: Arc<Mutex<Connection>>,
    }

    #[async_trait]
    impl KernelHandle for MemoryKernelStub {
        async fn spawn_agent(
            &self,
            _manifest: &str,
            _parent: Option<&str>,
        ) -> Result<(String, String), String> {
            Err("stub".into())
        }

        async fn send_to_agent(&self, _id: &str, _msg: &str) -> Result<String, String> {
            Err("stub".into())
        }

        fn list_agents(&self) -> Vec<crate::kernel_handle::AgentInfo> {
            Vec::new()
        }

        fn kill_agent(&self, _id: &str) -> Result<(), String> {
            Ok(())
        }

        fn memory_store(&self, _key: &str, _value: serde_json::Value) -> Result<(), String> {
            Ok(())
        }

        fn memory_recall(&self, _key: &str) -> Result<Option<serde_json::Value>, String> {
            Ok(None)
        }

        fn memory_writes_conn(&self) -> Option<Arc<Mutex<Connection>>> {
            Some(Arc::clone(&self.conn))
        }

        fn find_agents(&self, _query: &str) -> Vec<crate::kernel_handle::AgentInfo> {
            Vec::new()
        }

        async fn task_post(
            &self,
            _title: &str,
            _description: &str,
            _assignee: Option<&str>,
            _created_by: Option<&str>,
        ) -> Result<String, String> {
            Err("stub".into())
        }

        async fn task_claim(&self, _id: &str) -> Result<Option<serde_json::Value>, String> {
            Ok(None)
        }

        async fn task_complete(&self, _id: &str, _result: &str) -> Result<(), String> {
            Ok(())
        }
    }

    fn memory_kernel() -> Arc<dyn KernelHandle> {
        let conn = Connection::open_in_memory().unwrap();
        captain_memory::migration::run_migrations(&conn).unwrap();
        Arc::new(MemoryKernelStub {
            conn: Arc::new(Mutex::new(conn)),
        })
    }

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

        let recalled =
            recall_turn_memories("alpha", agent_id, &memory, None, &[], None, true, false).await;

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

        let recalled =
            recall_turn_memories("alpha", agent_id, &memory, None, &[], None, false, true).await;
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

        let recalled = recall_turn_memories(
            "beta",
            agent_id,
            &memory,
            None,
            &[],
            Some(&driver),
            false,
            false,
        )
        .await;

        assert!(recalled.iter().any(|m| m.content == "beta vector memory"));
    }

    #[tokio::test]
    async fn local_journal_recall_finds_old_durable_fact_and_prioritizes_it() {
        let memory = MemorySubstrate::open_in_memory(0.01).unwrap();
        let agent_id = AgentId::new();
        memory
            .remember(
                agent_id,
                "PUBLIC2 certification semantic note",
                MemorySource::Conversation,
                "episodic",
                HashMap::new(),
            )
            .await
            .unwrap();
        let kernel = memory_kernel();
        let conn = kernel.memory_writes_conn().unwrap();
        let durable = captain_memory::memory_writer::append(
            &conn.lock().unwrap(),
            captain_memory::memory_writer::NewMemoryWrite {
                subject: "user:vivien".into(),
                predicate: "prefers_certification_label".into(),
                object: "PUBLIC2 certification stages are called jalons ambre".into(),
                wing: Some("preferences".into()),
                room: Some("naming".into()),
                source: "memory_save:preference".into(),
            },
        )
        .unwrap();
        for index in 0..200 {
            let mut noise = captain_memory::memory_writer::NewMemoryWrite {
                subject: "runtime:event".into(),
                predicate: "observed".into(),
                object: format!("unrelated recent event {index}"),
                wing: None,
                room: None,
                source: "mirror".into(),
            };
            noise.subject.push_str(&index.to_string());
            captain_memory::memory_writer::append(&conn.lock().unwrap(), noise).unwrap();
        }

        let recalled = recall_turn_memories(
            "Quelle est ma convention PUBLIC2 pour les étapes de certification ?",
            agent_id,
            &memory,
            Some(&kernel),
            &[],
            None,
            false,
            false,
        )
        .await;

        assert_eq!(
            recalled[0].metadata["memory_write_id"],
            serde_json::Value::String(durable.id)
        );
        assert!(recalled[0].content.contains("jalons ambre"));
    }

    #[tokio::test]
    async fn local_journal_recall_never_reinjects_retracted_fact() {
        let memory = MemorySubstrate::open_in_memory(0.01).unwrap();
        let agent_id = AgentId::new();
        let kernel = memory_kernel();
        let conn = kernel.memory_writes_conn().unwrap();
        captain_memory::memory_writer::append(
            &conn.lock().unwrap(),
            captain_memory::memory_writer::NewMemoryWrite {
                subject: "user:vivien".into(),
                predicate: "prefers_certification_label".into(),
                object: "PUBLIC2 certification stages are called balises cuivre".into(),
                wing: None,
                room: None,
                source: "memory_save:preference".into(),
            },
        )
        .unwrap();
        captain_memory::memory_writer::retract_by_match(
            &conn.lock().unwrap(),
            Some("user:vivien"),
            Some("prefers_certification_label"),
            Some("%balises cuivre%"),
            "memory_forget",
        )
        .unwrap();

        let recalled = recall_turn_memories(
            "Quelle est ma convention PUBLIC2 pour les étapes de certification ?",
            agent_id,
            &memory,
            Some(&kernel),
            &[],
            None,
            false,
            false,
        )
        .await;

        assert!(recalled
            .iter()
            .all(|memory| !memory.content.contains("balises cuivre")));
    }

    #[tokio::test]
    async fn local_journal_active_replacement_survives_fuzzy_archive_guard() {
        let memory = MemorySubstrate::open_in_memory(0.01).unwrap();
        let agent_id = AgentId::new();
        let kernel = memory_kernel();
        let conn = kernel.memory_writes_conn().unwrap();
        captain_memory::memory_writer::append(
            &conn.lock().unwrap(),
            captain_memory::memory_writer::NewMemoryWrite {
                subject: "Vivien".into(),
                predicate: "preferred_name".into(),
                object: "revues temporaires".into(),
                wing: None,
                room: None,
                source: "memory_save:preference".into(),
            },
        )
        .unwrap();
        captain_memory::memory_writer::retract_by_match(
            &conn.lock().unwrap(),
            Some("Vivien"),
            Some("preferred_name"),
            Some("revues temporaires"),
            "memory_forget",
        )
        .unwrap();
        let active = captain_memory::memory_writer::append(
            &conn.lock().unwrap(),
            captain_memory::memory_writer::NewMemoryWrite {
                subject: "Vivien PUBLIC2-MEMCERT-0715B".into(),
                predicate: "préfère appeler les revues temporaires".into(),
                object: "cycles azur".into(),
                wing: None,
                room: None,
                source: "memory_save:preference".into(),
            },
        )
        .unwrap();
        let archive_guard = crate::memory_retractions::MemoryRetraction::from_filters(
            Some("Vivien"),
            Some("preferred_name"),
            Some("revues temporaires"),
        )
        .unwrap();

        let recalled = recall_turn_memories(
            "Quel nom PUBLIC2-MEMCERT-0715B pour les revues temporaires ?",
            agent_id,
            &memory,
            Some(&kernel),
            &[archive_guard],
            None,
            false,
            false,
        )
        .await;

        assert_eq!(
            recalled[0].metadata["memory_write_id"],
            serde_json::Value::String(active.id)
        );
        assert!(recalled[0].content.contains("cycles azur"));
    }

    #[tokio::test]
    async fn local_journal_recall_prioritizes_correction_then_latest_fact() {
        let memory = MemorySubstrate::open_in_memory(0.01).unwrap();
        let agent_id = AgentId::new();
        let kernel = memory_kernel();
        let conn = kernel.memory_writes_conn().unwrap();
        captain_memory::memory_writer::append(
            &conn.lock().unwrap(),
            captain_memory::memory_writer::NewMemoryWrite {
                subject: "Vivien".into(),
                predicate: "préfère".into(),
                object: "PUBLIC2 certification stages: balises cuivre".into(),
                wing: None,
                room: None,
                source: "memory_save:preference".into(),
            },
        )
        .unwrap();
        std::thread::sleep(std::time::Duration::from_millis(2));
        let latest = captain_memory::memory_writer::append(
            &conn.lock().unwrap(),
            captain_memory::memory_writer::NewMemoryWrite {
                subject: "Vivien".into(),
                predicate: "préfère".into(),
                object: "PUBLIC2 certification stages: jalons ambre".into(),
                wing: None,
                room: None,
                source: "memory_save:preference".into(),
            },
        )
        .unwrap();
        std::thread::sleep(std::time::Duration::from_millis(2));
        let correction = captain_memory::memory_writer::append(
            &conn.lock().unwrap(),
            captain_memory::memory_writer::NewMemoryWrite {
                subject: "Vivien".into(),
                predicate: "corrigé".into(),
                object: "Préférence PUBLIC2 corrigée: balises cuivre est obsolète, remplacer par jalons ambre".into(),
                wing: None,
                room: None,
                source: "memory_save:correction".into(),
            },
        )
        .unwrap();

        let recalled = recall_turn_memories(
            "Quelle préférence PUBLIC2 appliquer pour la certification ?",
            agent_id,
            &memory,
            Some(&kernel),
            &[],
            None,
            false,
            false,
        )
        .await;

        assert_eq!(
            recalled[0].metadata["memory_write_id"],
            serde_json::Value::String(correction.id)
        );
        assert_eq!(
            recalled[1].metadata["memory_write_id"],
            serde_json::Value::String(latest.id)
        );
    }

    #[tokio::test]
    async fn local_journal_precise_marker_beats_generic_older_correction() {
        let memory = MemorySubstrate::open_in_memory(0.01).unwrap();
        let agent_id = AgentId::new();
        let kernel = memory_kernel();
        let conn = kernel.memory_writes_conn().unwrap();
        captain_memory::memory_writer::append(
            &conn.lock().unwrap(),
            captain_memory::memory_writer::NewMemoryWrite {
                subject: "Vivien".into(),
                predicate: "corrigé".into(),
                object: "Préférence PUBLIC2 corrigée: balises cuivre est obsolète, remplacer par jalons ambre".into(),
                wing: None,
                room: None,
                source: "memory_save:correction".into(),
            },
        )
        .unwrap();
        let targeted = captain_memory::memory_writer::append(
            &conn.lock().unwrap(),
            captain_memory::memory_writer::NewMemoryWrite {
                subject: "Vivien — certification synthétique PUBLIC2-MEMCERT-0715B".into(),
                predicate: "préfère appeler les revues temporaires".into(),
                object: "cycles azur".into(),
                wing: None,
                room: None,
                source: "memory_save:preference".into(),
            },
        )
        .unwrap();

        let recalled = recall_turn_memories(
            "Correction PUBLIC2-MEMCERT-0715B: cycles azur est obsolète pour les revues temporaires",
            agent_id,
            &memory,
            Some(&kernel),
            &[],
            None,
            false,
            false,
        )
        .await;

        assert_eq!(
            recalled[0].metadata["memory_write_id"],
            serde_json::Value::String(targeted.id)
        );
        assert!(recalled[0].content.contains("cycles azur"));
    }
}
