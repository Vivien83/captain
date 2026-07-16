use super::*;
use crate::tools::tool_memory_recall_mempalace;

#[test]
fn memory_forget_in_tool_registry() {
    let tools = builtin_tool_definitions();
    let names: Vec<&str> = tools.iter().map(|t| t.name.as_str()).collect();
    assert!(names.contains(&"memory_forget"));
    let def = tools
        .iter()
        .find(|t| t.name == "memory_forget")
        .expect("memory_forget tool must exist");
    assert!(def.description.contains("RÉTRACTATION DURABLE"));
    assert!(def.description.contains("SPONTANÉMENT"));
}

#[tokio::test]
async fn memory_forget_rejects_empty_filter_set() {
    let kh: Arc<dyn KernelHandle> = Arc::new(MemSaveStubKernel);
    let res = tool_memory_forget(&serde_json::json!({}), None, Some(&kh)).await;
    assert!(res.is_err());
    assert!(res.unwrap_err().contains("at least subject"));
}

struct MemoryForgetStubKernel {
    conn: std::sync::Arc<std::sync::Mutex<rusqlite::Connection>>,
    kv: std::sync::Mutex<Option<serde_json::Value>>,
}

#[async_trait::async_trait]
impl KernelHandle for MemoryForgetStubKernel {
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
    fn memory_kv_store(&self, _key: &str, value: serde_json::Value) -> Result<(), String> {
        *self.kv.lock().unwrap() = Some(value);
        Ok(())
    }
    fn memory_kv_recall(&self, _key: &str) -> Result<Option<serde_json::Value>, String> {
        Ok(self.kv.lock().unwrap().clone())
    }
    fn memory_writes_conn(&self) -> Option<std::sync::Arc<std::sync::Mutex<rusqlite::Connection>>> {
        Some(std::sync::Arc::clone(&self.conn))
    }
    fn find_agents(&self, _q: &str) -> Vec<crate::kernel_handle::AgentInfo> {
        Vec::new()
    }
    async fn task_post(
        &self,
        _t: &str,
        _d: &str,
        _a: Option<&str>,
        _c: Option<&str>,
    ) -> Result<String, String> {
        Err("stub".into())
    }
    async fn task_claim(&self, _id: &str) -> Result<Option<serde_json::Value>, String> {
        Ok(None)
    }
    async fn task_complete(&self, _id: &str, _r: &str) -> Result<(), String> {
        Ok(())
    }
}

#[tokio::test]
async fn memory_forget_records_active_context_suppression() {
    let conn = rusqlite::Connection::open_in_memory().unwrap();
    captain_memory::migration::run_migrations(&conn).unwrap();
    captain_memory::memory_writer::append(
        &conn,
        captain_memory::memory_writer::NewMemoryWrite {
            subject: "user".into(),
            predicate: "has_pet".into(),
            object: "ancienne_valeur".into(),
            wing: None,
            room: None,
            source: "test".into(),
        },
    )
    .unwrap();

    let kh = std::sync::Arc::new(MemoryForgetStubKernel {
        conn: std::sync::Arc::new(std::sync::Mutex::new(conn)),
        kv: std::sync::Mutex::new(None),
    });
    let dyn_kh: Arc<dyn KernelHandle> = kh.clone();
    let res = tool_memory_forget(
        &serde_json::json!({"object":"%ancienne_valeur%"}),
        None,
        Some(&dyn_kh),
    )
    .await
    .unwrap();
    let payload: serde_json::Value = serde_json::from_str(&res).unwrap();
    assert_eq!(payload["removed"], 1);
    assert_eq!(payload["invalidations_queued"], 1);
    assert_eq!(payload["remote_pending"], 1);
    assert_eq!(payload["active_context_suppressed"], true);

    let stored = kh.kv.lock().unwrap().clone();
    let retractions = crate::memory_retractions::load_retractions(stored);
    assert_eq!(retractions.len(), 1);
    assert_eq!(retractions[0].terms, vec!["ancienne", "valeur"]);

    let conn = kh.conn.lock().unwrap();
    let audit = captain_memory::memory_writer::list_recent(&conn, None, 10).unwrap();
    assert_eq!(audit.len(), 2);
    assert!(audit.iter().any(|row| {
        row.operation == captain_memory::memory_writer::MemoryOperation::Invalidate
    }));
    assert!(audit.iter().any(|row| {
        row.operation == captain_memory::memory_writer::MemoryOperation::Add
            && row.retracted_at.is_some()
    }));
}

#[test]
fn memory_recall_part_filters_retracted_mempalace_diary() {
    let retraction =
        crate::memory_retractions::MemoryRetraction::from_filters(None, None, Some("%rocky%"))
            .unwrap();
    let stale_diary = r#"{
      "results": [{
        "text": "User: J'ai un animal de compagnie ? Assistant: Oui, un chien Rocky",
        "wing": "wing_captain",
        "room": "diary"
      }]
    }"#;

    assert!(
        memory_recall_part("MemPalace", stale_diary, std::slice::from_ref(&retraction)).is_none(),
        "retracted MemPalace diary hits must not be exposed to the model"
    );
    assert_eq!(
        memory_recall_part("MemPalace", r#"{"results":[]}"#, &[retraction]).as_deref(),
        Some(r#"[MemPalace] {"results":[]}"#)
    );
}

#[tokio::test]
async fn memory_recall_prefers_active_local_fact_over_fuzzy_archive_guard() {
    let conn = rusqlite::Connection::open_in_memory().unwrap();
    captain_memory::migration::run_migrations(&conn).unwrap();
    captain_memory::memory_writer::append(
        &conn,
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
        &conn,
        Some("Vivien"),
        Some("preferred_name"),
        Some("revues temporaires"),
        "memory_forget",
    )
    .unwrap();
    let active = captain_memory::memory_writer::append(
        &conn,
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
    let retraction = crate::memory_retractions::MemoryRetraction::from_filters(
        Some("Vivien"),
        Some("preferred_name"),
        Some("revues temporaires"),
    )
    .unwrap();
    let kh: Arc<dyn KernelHandle> = Arc::new(MemoryForgetStubKernel {
        conn: std::sync::Arc::new(std::sync::Mutex::new(conn)),
        kv: std::sync::Mutex::new(Some(crate::memory_retractions::retractions_to_value(
            std::slice::from_ref(&retraction),
        ))),
    });

    let recalled = tool_memory_recall_mempalace(
        &serde_json::json!({
            "key": "PUBLIC2-MEMCERT-0715B cycles azur revues temporaires"
        }),
        None,
        Some(&kh),
    )
    .await
    .unwrap();

    assert!(recalled.contains("Local journal — active authoritative facts"));
    assert!(recalled.contains(&active.id));
    assert!(recalled.contains("cycles azur"));
    assert!(!recalled.contains("No active memory found"));
}

#[test]
fn memory_context_tokens_keep_compact_product_markers() {
    assert_eq!(
        memory_context_tokens("Captain P0/P1 grouped rails"),
        vec!["captain", "p0", "p1", "grouped", "rails"]
    );
    assert_eq!(
        memory_context_tokens("service:inventory-api"),
        vec!["inventory", "api"]
    );
}

#[test]
fn memory_context_filters_low_confidence_mempalace_noise() {
    let raw = serde_json::json!({
        "query": "service:inventory-api",
        "results": [
            {
                "text": "User: Run backup-check.sh and send the result to Telegram.",
                "wing": "wing_captain",
                "room": "diary",
                "similarity": -0.51
            },
            {
                "text": "service inventory-api runs on edge-prod as inventory-api.service in /srv/inventory-api.",
                "wing": "wing_infrastructure",
                "room": "services",
                "similarity": 0.42
            }
        ]
    })
    .to_string();

    let compact = compact_mempalace_search_result(
        "service:inventory-api",
        &raw,
        5,
        500,
        DEFAULT_MEMORY_CONTEXT_MIN_SIMILARITY,
        true,
        &[],
    );
    assert_eq!(compact["match_count"], 1);
    assert_eq!(compact["filtered"], 1);
    assert!(compact["matches"][0]["preview"]
        .as_str()
        .unwrap()
        .contains("inventory-api"));
    assert!(!compact["matches"][0]["preview"]
        .as_str()
        .unwrap()
        .contains("backup-check"));
}

#[test]
fn memory_context_rejects_negative_similarity_even_with_lexical_overlap() {
    let raw = serde_json::json!({
        "results": [
            {
                "text": "Captain memory source truth MemPalace retired marker inside a stale diary transcript.",
                "wing": "wing_captain",
                "room": "diary",
                "similarity": -0.006
            },
            {
                "text": "Captain memory source of truth is MemPalace; workspace memory markdown is retired.",
                "wing": "wing_captain",
                "room": "architecture",
                "similarity": 0.42
            }
        ]
    })
    .to_string();

    let compact = compact_mempalace_search_result(
        "Captain memory source truth MemPalace retired",
        &raw,
        5,
        500,
        DEFAULT_MEMORY_CONTEXT_MIN_SIMILARITY,
        true,
        &[],
    );

    assert_eq!(compact["match_count"], 1);
    assert_eq!(compact["filtered"], 1);
    assert!(compact["matches"][0]["preview"]
        .as_str()
        .unwrap()
        .contains("source of truth"));
    assert!(!compact["matches"][0]["preview"]
        .as_str()
        .unwrap()
        .contains("stale diary"));
}

#[tokio::test]
async fn memory_context_batch_reads_local_memory_writes_high_confidence() {
    let conn = rusqlite::Connection::open_in_memory().unwrap();
    captain_memory::migration::run_migrations(&conn).unwrap();
    captain_memory::memory_writer::append(
        &conn,
        captain_memory::memory_writer::NewMemoryWrite {
            subject: "service:inventory-api".into(),
            predicate: "runs_on".into(),
            object: "edge-prod as inventory-api.service".into(),
            wing: Some("infrastructure".into()),
            room: Some("services".into()),
            source: "memory_save:info".into(),
        },
    )
    .unwrap();
    captain_memory::memory_writer::append(
        &conn,
        captain_memory::memory_writer::NewMemoryWrite {
            subject: "workflow:backup".into(),
            predicate: "uses".into(),
            object: "backup-check.sh".into(),
            wing: None,
            room: None,
            source: "test".into(),
        },
    )
    .unwrap();

    let kh = Arc::new(MemoryForgetStubKernel {
        conn: Arc::new(std::sync::Mutex::new(conn)),
        kv: std::sync::Mutex::new(None),
    });
    let dyn_kh: Arc<dyn KernelHandle> = kh;
    let compact = compact_memory_context_result(
        "service:inventory-api",
        None,
        Some(&dyn_kh),
        captain_types::config::MemoryBackend::Graph,
        3,
        700,
        DEFAULT_MEMORY_CONTEXT_MIN_SIMILARITY,
        true,
    )
    .await;

    assert_eq!(compact["match_count"], 1);
    let local = compact["sources"]
        .as_array()
        .unwrap()
        .iter()
        .find(|source| source["source"] == "memory_writes")
        .expect("memory_writes source must be present");
    assert_eq!(local["match_count"], 1);
    assert!(local["matches"][0]["preview"]
        .as_str()
        .unwrap()
        .contains("inventory-api"));
    assert!(!local["matches"][0]["preview"]
        .as_str()
        .unwrap()
        .contains("backup-check"));
}

#[test]
fn memory_context_accepts_strong_similarity_without_lexical_overlap() {
    let raw = serde_json::json!({
        "results": [
            {
                "text": "The user prefers concise technical answers.",
                "similarity": 0.91
            }
        ]
    })
    .to_string();

    let compact = compact_mempalace_search_result(
        "style de réponse",
        &raw,
        5,
        500,
        DEFAULT_MEMORY_CONTEXT_MIN_SIMILARITY,
        true,
        &[],
    );
    assert_eq!(compact["match_count"], 1);
    assert_eq!(compact["matches"][0]["similarity"], 0.91);
}
