//! MemorySubstrate: unified implementation of the `Memory` trait.
//!
//! Composes the structured store, semantic store, knowledge store,
//! session store, and consolidation engine behind a single async API.

use crate::consolidation::ConsolidationEngine;
use crate::knowledge::KnowledgeStore;
use crate::migration::run_migrations;
use crate::semantic::SemanticStore;
use crate::session::{Session, SessionStore};
use crate::structured::StructuredStore;
use crate::usage::UsageStore;

use async_trait::async_trait;
use captain_types::agent::{AgentEntry, AgentId, SessionId};
use captain_types::error::{CaptainError, CaptainResult};
use captain_types::memory::{
    ConsolidationReport, Entity, ExportFormat, GraphMatch, GraphPattern, ImportReport, Memory,
    MemoryFilter, MemoryFragment, MemoryId, MemorySource, Relation,
};
use rusqlite::Connection;
use std::collections::HashMap;
use std::path::Path;
use std::sync::{Arc, Mutex};

/// The unified memory substrate. Implements the `Memory` trait by delegating
/// to specialized stores backed by a shared SQLite connection.
pub struct MemorySubstrate {
    conn: Arc<Mutex<Connection>>,
    structured: StructuredStore,
    pub semantic: SemanticStore,
    knowledge: KnowledgeStore,
    sessions: SessionStore,
    consolidation: ConsolidationEngine,
    usage: UsageStore,
}

impl MemorySubstrate {
    /// Open or create a memory substrate at the given database path.
    pub fn open(db_path: &Path, decay_rate: f32) -> CaptainResult<Self> {
        let conn = Connection::open(db_path).map_err(|e| CaptainError::Memory(e.to_string()))?;
        conn.execute_batch("PRAGMA journal_mode=WAL; PRAGMA busy_timeout=5000;")
            .map_err(|e| CaptainError::Memory(e.to_string()))?;
        run_migrations(&conn).map_err(|e| CaptainError::Memory(e.to_string()))?;
        let shared = Arc::new(Mutex::new(conn));

        Ok(Self {
            conn: Arc::clone(&shared),
            structured: StructuredStore::new(Arc::clone(&shared)),
            semantic: SemanticStore::new(Arc::clone(&shared)),
            knowledge: KnowledgeStore::new(Arc::clone(&shared)),
            sessions: SessionStore::new(Arc::clone(&shared)),
            usage: UsageStore::new(Arc::clone(&shared)),
            consolidation: ConsolidationEngine::new(shared, decay_rate),
        })
    }

    /// Create an in-memory substrate (for testing).
    pub fn open_in_memory(decay_rate: f32) -> CaptainResult<Self> {
        let conn = Connection::open_in_memory().map_err(|e| CaptainError::Memory(e.to_string()))?;
        run_migrations(&conn).map_err(|e| CaptainError::Memory(e.to_string()))?;
        let shared = Arc::new(Mutex::new(conn));

        Ok(Self {
            conn: Arc::clone(&shared),
            structured: StructuredStore::new(Arc::clone(&shared)),
            semantic: SemanticStore::new(Arc::clone(&shared)),
            knowledge: KnowledgeStore::new(Arc::clone(&shared)),
            sessions: SessionStore::new(Arc::clone(&shared)),
            usage: UsageStore::new(Arc::clone(&shared)),
            consolidation: ConsolidationEngine::new(shared, decay_rate),
        })
    }

    /// Get a reference to the usage store.
    pub fn usage(&self) -> &UsageStore {
        &self.usage
    }

    /// Get the shared database connection (for constructing stores from outside).
    pub fn usage_conn(&self) -> Arc<Mutex<Connection>> {
        Arc::clone(&self.conn)
    }

    /// Append one row to `sessions_events` for timeline replay (v3.9f).
    ///
    /// The call is synchronous but cheap — a single prepared INSERT under
    /// the shared connection's mutex. Errors propagate so callers can
    /// decide between logging-and-continuing (the agent loop's stance:
    /// observability must never crash the turn) and failing loudly
    /// (tests, admin tools).
    pub fn append_session_event(
        &self,
        session_id: &str,
        event_type: &str,
        payload: &serde_json::Value,
    ) -> CaptainResult<i64> {
        let guard = self
            .conn
            .lock()
            .map_err(|e| CaptainError::Memory(format!("event log lock: {e}")))?;
        crate::event_log::append(&guard, session_id, event_type, payload)
            .map_err(|e| CaptainError::Memory(format!("event log append: {e}")))
    }

    /// Read a window of session events back. See [`event_log::range`].
    pub fn read_session_events(
        &self,
        query: &crate::event_log::RangeQuery,
    ) -> CaptainResult<Vec<crate::event_log::SessionEvent>> {
        let guard = self
            .conn
            .lock()
            .map_err(|e| CaptainError::Memory(format!("event log lock: {e}")))?;
        crate::event_log::range(&guard, query)
            .map_err(|e| CaptainError::Memory(format!("event log range: {e}")))
    }

    /// Read the newest session events back, returned oldest-to-newest.
    pub fn read_session_events_tail(
        &self,
        query: &crate::event_log::RangeQuery,
    ) -> CaptainResult<Vec<crate::event_log::SessionEvent>> {
        let guard = self
            .conn
            .lock()
            .map_err(|e| CaptainError::Memory(format!("event log lock: {e}")))?;
        crate::event_log::tail(&guard, query)
            .map_err(|e| CaptainError::Memory(format!("event log tail: {e}")))
    }

    /// Read newest session events for one event type, returned oldest-to-newest.
    pub fn read_session_events_tail_by_type(
        &self,
        query: &crate::event_log::RangeQuery,
        event_type: &str,
    ) -> CaptainResult<Vec<crate::event_log::SessionEvent>> {
        let guard = self
            .conn
            .lock()
            .map_err(|e| CaptainError::Memory(format!("event log lock: {e}")))?;
        crate::event_log::tail_by_type(&guard, query, event_type)
            .map_err(|e| CaptainError::Memory(format!("event log tail by type: {e}")))
    }

    /// Count events stored for a session. O(log n) with the composite index.
    pub fn count_session_events(&self, session_id: &str) -> CaptainResult<u64> {
        let guard = self
            .conn
            .lock()
            .map_err(|e| CaptainError::Memory(format!("event log lock: {e}")))?;
        crate::event_log::count(&guard, session_id)
            .map_err(|e| CaptainError::Memory(format!("event log count: {e}")))
    }

    /// Count events stored for a session and event type.
    pub fn count_session_events_by_type(
        &self,
        session_id: &str,
        event_type: &str,
    ) -> CaptainResult<u64> {
        let guard = self
            .conn
            .lock()
            .map_err(|e| CaptainError::Memory(format!("event log lock: {e}")))?;
        crate::event_log::count_by_type(&guard, session_id, event_type)
            .map_err(|e| CaptainError::Memory(format!("event log count by type: {e}")))
    }

    // -----------------------------------------------------------------
    // Project entity (v3.11a)
    // -----------------------------------------------------------------

    /// Create a new project. Errors if the slug is already taken.
    pub fn project_create(
        &self,
        input: crate::project::NewProject,
    ) -> CaptainResult<crate::project::Project> {
        let guard = self
            .conn
            .lock()
            .map_err(|e| CaptainError::Memory(format!("project lock: {e}")))?;
        crate::project::create(&guard, input)
            .map_err(|e| CaptainError::Memory(format!("project create: {e}")))
    }

    pub fn project_get(&self, id: &str) -> CaptainResult<Option<crate::project::Project>> {
        let guard = self
            .conn
            .lock()
            .map_err(|e| CaptainError::Memory(format!("project lock: {e}")))?;
        crate::project::get(&guard, id)
            .map_err(|e| CaptainError::Memory(format!("project get: {e}")))
    }

    pub fn project_find_by_slug(
        &self,
        slug: &str,
    ) -> CaptainResult<Option<crate::project::Project>> {
        let guard = self
            .conn
            .lock()
            .map_err(|e| CaptainError::Memory(format!("project lock: {e}")))?;
        crate::project::find_by_slug(&guard, slug)
            .map_err(|e| CaptainError::Memory(format!("project find_by_slug: {e}")))
    }

    pub fn project_list(
        &self,
        include_archived: bool,
    ) -> CaptainResult<Vec<crate::project::Project>> {
        let guard = self
            .conn
            .lock()
            .map_err(|e| CaptainError::Memory(format!("project lock: {e}")))?;
        crate::project::list(&guard, include_archived)
            .map_err(|e| CaptainError::Memory(format!("project list: {e}")))
    }

    pub fn project_update(
        &self,
        id: &str,
        patch: crate::project::ProjectPatch,
    ) -> CaptainResult<Option<crate::project::Project>> {
        let guard = self
            .conn
            .lock()
            .map_err(|e| CaptainError::Memory(format!("project lock: {e}")))?;
        crate::project::update(&guard, id, patch)
            .map_err(|e| CaptainError::Memory(format!("project update: {e}")))
    }

    pub fn project_archive(&self, id: &str) -> CaptainResult<Option<crate::project::Project>> {
        let guard = self
            .conn
            .lock()
            .map_err(|e| CaptainError::Memory(format!("project lock: {e}")))?;
        crate::project::archive(&guard, id)
            .map_err(|e| CaptainError::Memory(format!("project archive: {e}")))
    }

    pub fn project_delete(&self, id: &str) -> CaptainResult<bool> {
        let guard = self
            .conn
            .lock()
            .map_err(|e| CaptainError::Memory(format!("project lock: {e}")))?;
        crate::project::delete(&guard, id)
            .map_err(|e| CaptainError::Memory(format!("project delete: {e}")))
    }

    // -----------------------------------------------------------------
    // Project tasks (v3.11b)
    // -----------------------------------------------------------------

    pub fn task_create(
        &self,
        input: crate::project_task::NewProjectTask,
    ) -> CaptainResult<crate::project_task::ProjectTask> {
        let guard = self
            .conn
            .lock()
            .map_err(|e| CaptainError::Memory(format!("task lock: {e}")))?;
        crate::project_task::create(&guard, input)
            .map_err(|e| CaptainError::Memory(format!("task create: {e}")))
    }

    pub fn task_get(&self, id: &str) -> CaptainResult<Option<crate::project_task::ProjectTask>> {
        let guard = self
            .conn
            .lock()
            .map_err(|e| CaptainError::Memory(format!("task lock: {e}")))?;
        crate::project_task::get(&guard, id)
            .map_err(|e| CaptainError::Memory(format!("task get: {e}")))
    }

    pub fn task_list_for_project(
        &self,
        project_id: &str,
    ) -> CaptainResult<Vec<crate::project_task::ProjectTask>> {
        let guard = self
            .conn
            .lock()
            .map_err(|e| CaptainError::Memory(format!("task lock: {e}")))?;
        crate::project_task::list_for_project(&guard, project_id)
            .map_err(|e| CaptainError::Memory(format!("task list: {e}")))
    }

    pub fn task_update(
        &self,
        id: &str,
        patch: crate::project_task::TaskPatch,
    ) -> CaptainResult<Option<crate::project_task::ProjectTask>> {
        let guard = self
            .conn
            .lock()
            .map_err(|e| CaptainError::Memory(format!("task lock: {e}")))?;
        crate::project_task::update(&guard, id, patch)
            .map_err(|e| CaptainError::Memory(format!("task update: {e}")))
    }

    pub fn task_delete(&self, id: &str) -> CaptainResult<bool> {
        let guard = self
            .conn
            .lock()
            .map_err(|e| CaptainError::Memory(format!("task lock: {e}")))?;
        crate::project_task::delete(&guard, id)
            .map_err(|e| CaptainError::Memory(format!("task delete: {e}")))
    }

    // -----------------------------------------------------------------
    // Cross-session todos (v3.12g)
    // -----------------------------------------------------------------

    pub fn todo_create(&self, input: crate::todo::NewTodo) -> CaptainResult<crate::todo::Todo> {
        let guard = self
            .conn
            .lock()
            .map_err(|e| CaptainError::Memory(format!("todo lock: {e}")))?;
        crate::todo::create(&guard, input)
            .map_err(|e| CaptainError::Memory(format!("todo create: {e}")))
    }

    pub fn todo_list(
        &self,
        filter: crate::todo::TodoFilter,
        limit: Option<u32>,
    ) -> CaptainResult<Vec<crate::todo::Todo>> {
        let guard = self
            .conn
            .lock()
            .map_err(|e| CaptainError::Memory(format!("todo lock: {e}")))?;
        crate::todo::list(&guard, filter, limit)
            .map_err(|e| CaptainError::Memory(format!("todo list: {e}")))
    }

    pub fn todo_complete(&self, id: &str) -> CaptainResult<Option<crate::todo::Todo>> {
        let guard = self
            .conn
            .lock()
            .map_err(|e| CaptainError::Memory(format!("todo lock: {e}")))?;
        crate::todo::complete(&guard, id)
            .map_err(|e| CaptainError::Memory(format!("todo complete: {e}")))
    }

    pub fn todo_reopen(&self, id: &str) -> CaptainResult<Option<crate::todo::Todo>> {
        let guard = self
            .conn
            .lock()
            .map_err(|e| CaptainError::Memory(format!("todo lock: {e}")))?;
        crate::todo::reopen(&guard, id)
            .map_err(|e| CaptainError::Memory(format!("todo reopen: {e}")))
    }

    pub fn todo_delete(&self, id: &str) -> CaptainResult<bool> {
        let guard = self
            .conn
            .lock()
            .map_err(|e| CaptainError::Memory(format!("todo lock: {e}")))?;
        crate::todo::delete(&guard, id)
            .map_err(|e| CaptainError::Memory(format!("todo delete: {e}")))
    }

    // -----------------------------------------------------------------
    // Milestones (v3.11c)
    // -----------------------------------------------------------------

    pub fn milestone_create(
        &self,
        input: crate::milestone::NewMilestone,
    ) -> CaptainResult<crate::milestone::Milestone> {
        let guard = self
            .conn
            .lock()
            .map_err(|e| CaptainError::Memory(format!("milestone lock: {e}")))?;
        crate::milestone::create(&guard, input)
            .map_err(|e| CaptainError::Memory(format!("milestone create: {e}")))
    }

    pub fn milestone_list_for_project(
        &self,
        project_id: &str,
    ) -> CaptainResult<Vec<crate::milestone::Milestone>> {
        let guard = self
            .conn
            .lock()
            .map_err(|e| CaptainError::Memory(format!("milestone lock: {e}")))?;
        crate::milestone::list_for_project(&guard, project_id)
            .map_err(|e| CaptainError::Memory(format!("milestone list: {e}")))
    }

    pub fn milestone_complete(
        &self,
        id: &str,
    ) -> CaptainResult<Option<crate::milestone::Milestone>> {
        let guard = self
            .conn
            .lock()
            .map_err(|e| CaptainError::Memory(format!("milestone lock: {e}")))?;
        crate::milestone::complete(&guard, id)
            .map_err(|e| CaptainError::Memory(format!("milestone complete: {e}")))
    }

    pub fn milestone_progress(
        &self,
        project_id: &str,
        now_unix_ms: i64,
    ) -> CaptainResult<crate::milestone::MilestoneProgress> {
        let guard = self
            .conn
            .lock()
            .map_err(|e| CaptainError::Memory(format!("milestone lock: {e}")))?;
        crate::milestone::progress(&guard, project_id, now_unix_ms)
            .map_err(|e| CaptainError::Memory(format!("milestone progress: {e}")))
    }

    // -----------------------------------------------------------------
    // Project checkpoints (v3.11g)
    // -----------------------------------------------------------------

    pub fn checkpoint_append(
        &self,
        input: crate::project_checkpoint::NewCheckpoint,
    ) -> CaptainResult<crate::project_checkpoint::Checkpoint> {
        let guard = self
            .conn
            .lock()
            .map_err(|e| CaptainError::Memory(format!("checkpoint lock: {e}")))?;
        crate::project_checkpoint::append(&guard, input)
            .map_err(|e| CaptainError::Memory(format!("checkpoint append: {e}")))
    }

    pub fn checkpoint_latest(
        &self,
        project_id: &str,
    ) -> CaptainResult<Option<crate::project_checkpoint::Checkpoint>> {
        let guard = self
            .conn
            .lock()
            .map_err(|e| CaptainError::Memory(format!("checkpoint lock: {e}")))?;
        crate::project_checkpoint::latest(&guard, project_id)
            .map_err(|e| CaptainError::Memory(format!("checkpoint latest: {e}")))
    }

    pub fn checkpoint_history(
        &self,
        project_id: &str,
        limit: usize,
    ) -> CaptainResult<Vec<crate::project_checkpoint::Checkpoint>> {
        let guard = self
            .conn
            .lock()
            .map_err(|e| CaptainError::Memory(format!("checkpoint lock: {e}")))?;
        crate::project_checkpoint::history(&guard, project_id, limit)
            .map_err(|e| CaptainError::Memory(format!("checkpoint history: {e}")))
    }

    /// Save an agent entry to persistent storage.
    pub fn save_agent(&self, entry: &AgentEntry) -> CaptainResult<()> {
        self.structured.save_agent(entry)
    }

    /// Load an agent entry from persistent storage.
    pub fn load_agent(&self, agent_id: AgentId) -> CaptainResult<Option<AgentEntry>> {
        self.structured.load_agent(agent_id)
    }

    /// Remove an agent from persistent storage and cascade-delete sessions.
    pub fn remove_agent(&self, agent_id: AgentId) -> CaptainResult<()> {
        // Delete associated sessions first
        let _ = self.sessions.delete_agent_sessions(agent_id);
        self.structured.remove_agent(agent_id)
    }

    /// Load all agent entries from persistent storage.
    pub fn load_all_agents(&self) -> CaptainResult<Vec<AgentEntry>> {
        self.structured.load_all_agents()
    }

    /// List all saved agents.
    pub fn list_agents(&self) -> CaptainResult<Vec<(String, String, String)>> {
        self.structured.list_agents()
    }

    /// Synchronous get from the structured store (for kernel handle use).
    pub fn structured_get(
        &self,
        agent_id: AgentId,
        key: &str,
    ) -> CaptainResult<Option<serde_json::Value>> {
        self.structured.get(agent_id, key)
    }

    /// List all KV pairs for an agent.
    pub fn list_kv(&self, agent_id: AgentId) -> CaptainResult<Vec<(String, serde_json::Value)>> {
        self.structured.list_kv(agent_id)
    }

    /// Delete a KV entry for an agent.
    pub fn structured_delete(&self, agent_id: AgentId, key: &str) -> CaptainResult<()> {
        self.structured.delete(agent_id, key)
    }

    /// Synchronous set in the structured store (for kernel handle use).
    pub fn structured_set(
        &self,
        agent_id: AgentId,
        key: &str,
        value: serde_json::Value,
    ) -> CaptainResult<()> {
        self.structured.set(agent_id, key, value)
    }

    /// Get a session by ID.
    pub fn get_session(&self, session_id: SessionId) -> CaptainResult<Option<Session>> {
        self.sessions.get_session(session_id)
    }

    /// Save a session.
    pub fn save_session(&self, session: &Session) -> CaptainResult<()> {
        self.sessions.save_session(session)
    }

    /// Import a historical session without overwriting a row that already
    /// exists under the same stable ID.
    pub fn import_session_if_absent(
        &self,
        session: &Session,
        created_at_secs: u64,
        updated_at_secs: u64,
    ) -> CaptainResult<bool> {
        self.sessions
            .import_session_if_absent(session, created_at_secs, updated_at_secs)
    }

    /// Save a session asynchronously — runs the SQLite write in a blocking
    /// thread so the tokio runtime stays responsive.
    pub async fn save_session_async(&self, session: &Session) -> CaptainResult<()> {
        let sessions = self.sessions.clone();
        let session = session.clone();
        tokio::task::spawn_blocking(move || sessions.save_session(&session))
            .await
            .map_err(|e| CaptainError::Internal(e.to_string()))?
    }

    /// Create a new empty session for an agent.
    pub fn create_session(&self, agent_id: AgentId) -> CaptainResult<Session> {
        self.sessions.create_session(agent_id)
    }

    /// List all sessions with metadata.
    pub fn list_sessions(&self) -> CaptainResult<Vec<serde_json::Value>> {
        self.sessions.list_sessions()
    }

    /// Delete a session by ID.
    pub fn delete_session(&self, session_id: SessionId) -> CaptainResult<()> {
        self.sessions.delete_session(session_id)
    }

    /// Delete all sessions belonging to an agent.
    pub fn delete_agent_sessions(&self, agent_id: AgentId) -> CaptainResult<()> {
        self.sessions.delete_agent_sessions(agent_id)
    }

    /// Delete the canonical (cross-channel) session for an agent.
    pub fn delete_canonical_session(&self, agent_id: AgentId) -> CaptainResult<()> {
        self.sessions.delete_canonical_session(agent_id)
    }

    /// Set or clear a session label.
    pub fn set_session_label(
        &self,
        session_id: SessionId,
        label: Option<&str>,
    ) -> CaptainResult<()> {
        self.sessions.set_session_label(session_id, label)
    }

    /// Find a session by label for a given agent.
    pub fn find_session_by_label(
        &self,
        agent_id: AgentId,
        label: &str,
    ) -> CaptainResult<Option<Session>> {
        self.sessions.find_session_by_label(agent_id, label)
    }

    /// List all sessions for a specific agent.
    pub fn list_agent_sessions(&self, agent_id: AgentId) -> CaptainResult<Vec<serde_json::Value>> {
        self.sessions.list_agent_sessions(agent_id)
    }

    /// Create a new session with an optional label.
    pub fn create_session_with_label(
        &self,
        agent_id: AgentId,
        label: Option<&str>,
    ) -> CaptainResult<Session> {
        self.sessions.create_session_with_label(agent_id, label)
    }

    /// Load canonical session context for cross-channel memory.
    ///
    /// Returns the compacted summary (if any) and recent messages from the
    /// agent's persistent canonical session.
    pub fn canonical_context(
        &self,
        agent_id: AgentId,
        window_size: Option<usize>,
    ) -> CaptainResult<(Option<String>, Vec<captain_types::message::Message>)> {
        self.sessions.canonical_context(agent_id, window_size)
    }

    /// Store an LLM-generated summary, replacing older messages with the kept subset.
    ///
    /// Used by the compactor to replace text-truncation compaction with an
    /// LLM-generated summary of older conversation history.
    pub fn store_llm_summary(
        &self,
        agent_id: AgentId,
        summary: &str,
        kept_messages: Vec<captain_types::message::Message>,
    ) -> CaptainResult<()> {
        self.sessions
            .store_llm_summary(agent_id, summary, kept_messages)
    }

    /// Write a human-readable JSONL mirror of a session to disk.
    ///
    /// Best-effort — errors are returned but should be logged,
    /// never affecting the primary SQLite store.
    pub fn write_jsonl_mirror(
        &self,
        session: &Session,
        sessions_dir: &Path,
    ) -> Result<(), std::io::Error> {
        self.sessions.write_jsonl_mirror(session, sessions_dir)
    }

    /// Append messages to the agent's canonical session for cross-channel persistence.
    pub fn append_canonical(
        &self,
        agent_id: AgentId,
        messages: &[captain_types::message::Message],
        compaction_threshold: Option<usize>,
    ) -> CaptainResult<()> {
        self.sessions
            .append_canonical(agent_id, messages, compaction_threshold)?;
        Ok(())
    }

    // -----------------------------------------------------------------
    // Paired devices persistence
    // -----------------------------------------------------------------

    /// Load all paired devices from the database.
    pub fn load_paired_devices(&self) -> CaptainResult<Vec<serde_json::Value>> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| CaptainError::Memory(e.to_string()))?;
        let mut stmt = conn.prepare(
            "SELECT device_id, display_name, platform, paired_at, last_seen, push_token FROM paired_devices"
        ).map_err(|e| CaptainError::Memory(e.to_string()))?;
        let rows = stmt
            .query_map([], |row| {
                Ok(serde_json::json!({
                    "device_id": row.get::<_, String>(0)?,
                    "display_name": row.get::<_, String>(1)?,
                    "platform": row.get::<_, String>(2)?,
                    "paired_at": row.get::<_, String>(3)?,
                    "last_seen": row.get::<_, String>(4)?,
                    "push_token": row.get::<_, Option<String>>(5)?,
                }))
            })
            .map_err(|e| CaptainError::Memory(e.to_string()))?;
        let mut devices = Vec::new();
        for row in rows {
            devices.push(row.map_err(|e| CaptainError::Memory(e.to_string()))?);
        }
        Ok(devices)
    }

    /// Save a paired device to the database (insert or replace).
    pub fn save_paired_device(
        &self,
        device_id: &str,
        display_name: &str,
        platform: &str,
        paired_at: &str,
        last_seen: &str,
        push_token: Option<&str>,
    ) -> CaptainResult<()> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| CaptainError::Memory(e.to_string()))?;
        conn.execute(
            "INSERT OR REPLACE INTO paired_devices (device_id, display_name, platform, paired_at, last_seen, push_token) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            rusqlite::params![device_id, display_name, platform, paired_at, last_seen, push_token],
        ).map_err(|e| CaptainError::Memory(e.to_string()))?;
        Ok(())
    }

    /// Remove a paired device from the database.
    pub fn remove_paired_device(&self, device_id: &str) -> CaptainResult<()> {
        let conn = self
            .conn
            .lock()
            .map_err(|e| CaptainError::Memory(e.to_string()))?;
        conn.execute(
            "DELETE FROM paired_devices WHERE device_id = ?1",
            rusqlite::params![device_id],
        )
        .map_err(|e| CaptainError::Memory(e.to_string()))?;
        Ok(())
    }

    // -----------------------------------------------------------------
    // Embedding-aware memory operations
    // -----------------------------------------------------------------

    /// Store a memory with an embedding vector.
    pub fn remember_with_embedding(
        &self,
        agent_id: AgentId,
        content: &str,
        source: MemorySource,
        scope: &str,
        metadata: HashMap<String, serde_json::Value>,
        embedding: Option<&[f32]>,
    ) -> CaptainResult<MemoryId> {
        self.semantic
            .remember_with_embedding(agent_id, content, source, scope, metadata, embedding)
    }

    /// Recall memories using vector similarity when a query embedding is provided.
    pub fn recall_with_embedding(
        &self,
        query: &str,
        limit: usize,
        filter: Option<MemoryFilter>,
        query_embedding: Option<&[f32]>,
    ) -> CaptainResult<Vec<MemoryFragment>> {
        self.semantic
            .recall_with_embedding(query, limit, filter, query_embedding)
    }

    /// Update the embedding for an existing memory.
    pub fn update_embedding(&self, id: MemoryId, embedding: &[f32]) -> CaptainResult<()> {
        self.semantic.update_embedding(id, embedding)
    }

    /// Async wrapper for `recall_with_embedding` — runs in a blocking thread.
    pub async fn recall_with_embedding_async(
        &self,
        query: &str,
        limit: usize,
        filter: Option<MemoryFilter>,
        query_embedding: Option<&[f32]>,
    ) -> CaptainResult<Vec<MemoryFragment>> {
        let store = self.semantic.clone();
        let query = query.to_string();
        let embedding_owned = query_embedding.map(|e| e.to_vec());
        tokio::task::spawn_blocking(move || {
            store.recall_with_embedding(&query, limit, filter, embedding_owned.as_deref())
        })
        .await
        .map_err(|e| CaptainError::Internal(e.to_string()))?
    }

    /// Async wrapper for `remember_with_embedding` — runs in a blocking thread.
    pub async fn remember_with_embedding_async(
        &self,
        agent_id: AgentId,
        content: &str,
        source: MemorySource,
        scope: &str,
        metadata: HashMap<String, serde_json::Value>,
        embedding: Option<&[f32]>,
    ) -> CaptainResult<MemoryId> {
        let store = self.semantic.clone();
        let content = content.to_string();
        let scope = scope.to_string();
        let embedding_owned = embedding.map(|e| e.to_vec());
        tokio::task::spawn_blocking(move || {
            store.remember_with_embedding(
                agent_id,
                &content,
                source,
                &scope,
                metadata,
                embedding_owned.as_deref(),
            )
        })
        .await
        .map_err(|e| CaptainError::Internal(e.to_string()))?
    }

    // -----------------------------------------------------------------
    // Task queue operations
    // -----------------------------------------------------------------

    /// Post a new task to the shared queue. Returns the task ID.
    pub async fn task_post(
        &self,
        title: &str,
        description: &str,
        assigned_to: Option<&str>,
        created_by: Option<&str>,
    ) -> CaptainResult<String> {
        let conn = Arc::clone(&self.conn);
        let title = title.to_string();
        let description = description.to_string();
        let assigned_to = assigned_to.unwrap_or("").to_string();
        let created_by = created_by.unwrap_or("").to_string();

        tokio::task::spawn_blocking(move || {
            let id = uuid::Uuid::new_v4().to_string();
            let now = chrono::Utc::now().to_rfc3339();
            let db = conn.lock().map_err(|e| CaptainError::Internal(e.to_string()))?;
            db.execute(
                "INSERT INTO task_queue (id, agent_id, task_type, payload, status, priority, created_at, title, description, assigned_to, created_by)
                 VALUES (?1, ?2, ?3, ?4, 'pending', 0, ?5, ?6, ?7, ?8, ?9)",
                rusqlite::params![id, &created_by, &title, b"", now, title, description, assigned_to, created_by],
            )
            .map_err(|e| CaptainError::Memory(e.to_string()))?;
            Ok(id)
        })
        .await
        .map_err(|e| CaptainError::Internal(e.to_string()))?
    }

    /// Claim the next pending task (optionally for a specific assignee). Returns task JSON or None.
    pub async fn task_claim(&self, agent_id: &str) -> CaptainResult<Option<serde_json::Value>> {
        let conn = Arc::clone(&self.conn);
        let agent_id = agent_id.to_string();

        tokio::task::spawn_blocking(move || {
            let db = conn.lock().map_err(|e| CaptainError::Internal(e.to_string()))?;
            // Find first pending task assigned to this agent, or any unassigned pending task
            let mut stmt = db.prepare(
                "SELECT id, title, description, assigned_to, created_by, created_at
                 FROM task_queue
                 WHERE status = 'pending' AND (assigned_to = ?1 OR assigned_to = '')
                 ORDER BY priority DESC, created_at ASC
                 LIMIT 1"
            ).map_err(|e| CaptainError::Memory(e.to_string()))?;

            let result = stmt.query_row(rusqlite::params![agent_id], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, String>(4)?,
                    row.get::<_, String>(5)?,
                ))
            });

            match result {
                Ok((id, title, description, assigned, created_by, created_at)) => {
                    // Update status to in_progress
                    db.execute(
                        "UPDATE task_queue SET status = 'in_progress', assigned_to = ?2 WHERE id = ?1",
                        rusqlite::params![id, agent_id],
                    ).map_err(|e| CaptainError::Memory(e.to_string()))?;

                    Ok(Some(serde_json::json!({
                        "id": id,
                        "title": title,
                        "description": description,
                        "status": "in_progress",
                        "assigned_to": if assigned.is_empty() { &agent_id } else { &assigned },
                        "created_by": created_by,
                        "created_at": created_at,
                    })))
                }
                Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
                Err(e) => Err(CaptainError::Memory(e.to_string())),
            }
        })
        .await
        .map_err(|e| CaptainError::Internal(e.to_string()))?
    }

    /// Mark a task as completed with a result string.
    pub async fn task_complete(&self, task_id: &str, result: &str) -> CaptainResult<()> {
        let conn = Arc::clone(&self.conn);
        let task_id = task_id.to_string();
        let result = result.to_string();

        tokio::task::spawn_blocking(move || {
            let now = chrono::Utc::now().to_rfc3339();
            let db = conn.lock().map_err(|e| CaptainError::Internal(e.to_string()))?;
            let rows = db.execute(
                "UPDATE task_queue SET status = 'completed', result = ?2, completed_at = ?3 WHERE id = ?1",
                rusqlite::params![task_id, result, now],
            ).map_err(|e| CaptainError::Memory(e.to_string()))?;
            if rows == 0 {
                return Err(CaptainError::Internal(format!("Task not found: {task_id}")));
            }
            Ok(())
        })
        .await
        .map_err(|e| CaptainError::Internal(e.to_string()))?
    }

    /// List tasks, optionally filtered by status.
    pub async fn task_list(&self, status: Option<&str>) -> CaptainResult<Vec<serde_json::Value>> {
        let conn = Arc::clone(&self.conn);
        let status = status.map(|s| s.to_string());

        tokio::task::spawn_blocking(move || {
            let db = conn.lock().map_err(|e| CaptainError::Internal(e.to_string()))?;
            let (sql, params): (&str, Vec<Box<dyn rusqlite::types::ToSql>>) = match &status {
                Some(s) => (
                    "SELECT id, title, description, status, assigned_to, created_by, created_at, completed_at, result FROM task_queue WHERE status = ?1 ORDER BY created_at DESC",
                    vec![Box::new(s.clone())],
                ),
                None => (
                    "SELECT id, title, description, status, assigned_to, created_by, created_at, completed_at, result FROM task_queue ORDER BY created_at DESC",
                    vec![],
                ),
            };

            let mut stmt = db.prepare(sql).map_err(|e| CaptainError::Memory(e.to_string()))?;
            let params_refs: Vec<&dyn rusqlite::types::ToSql> = params.iter().map(|p| p.as_ref()).collect();
            let rows = stmt.query_map(params_refs.as_slice(), |row| {
                Ok(serde_json::json!({
                    "id": row.get::<_, String>(0)?,
                    "title": row.get::<_, String>(1).unwrap_or_default(),
                    "description": row.get::<_, String>(2).unwrap_or_default(),
                    "status": row.get::<_, String>(3)?,
                    "assigned_to": row.get::<_, String>(4).unwrap_or_default(),
                    "created_by": row.get::<_, String>(5).unwrap_or_default(),
                    "created_at": row.get::<_, String>(6).unwrap_or_default(),
                    "completed_at": row.get::<_, Option<String>>(7).unwrap_or(None),
                    "result": row.get::<_, Option<String>>(8).unwrap_or(None),
                }))
            }).map_err(|e| CaptainError::Memory(e.to_string()))?;

            let mut tasks = Vec::new();
            for row in rows {
                tasks.push(row.map_err(|e| CaptainError::Memory(e.to_string()))?);
            }
            Ok(tasks)
        })
        .await
        .map_err(|e| CaptainError::Internal(e.to_string()))?
    }
}

#[async_trait]
impl Memory for MemorySubstrate {
    async fn get(&self, agent_id: AgentId, key: &str) -> CaptainResult<Option<serde_json::Value>> {
        let store = self.structured.clone();
        let key = key.to_string();
        tokio::task::spawn_blocking(move || store.get(agent_id, &key))
            .await
            .map_err(|e| CaptainError::Internal(e.to_string()))?
    }

    async fn set(
        &self,
        agent_id: AgentId,
        key: &str,
        value: serde_json::Value,
    ) -> CaptainResult<()> {
        let store = self.structured.clone();
        let key = key.to_string();
        tokio::task::spawn_blocking(move || store.set(agent_id, &key, value))
            .await
            .map_err(|e| CaptainError::Internal(e.to_string()))?
    }

    async fn delete(&self, agent_id: AgentId, key: &str) -> CaptainResult<()> {
        let store = self.structured.clone();
        let key = key.to_string();
        tokio::task::spawn_blocking(move || store.delete(agent_id, &key))
            .await
            .map_err(|e| CaptainError::Internal(e.to_string()))?
    }

    async fn remember(
        &self,
        agent_id: AgentId,
        content: &str,
        source: MemorySource,
        scope: &str,
        metadata: HashMap<String, serde_json::Value>,
    ) -> CaptainResult<MemoryId> {
        let store = self.semantic.clone();
        let content = content.to_string();
        let scope = scope.to_string();
        tokio::task::spawn_blocking(move || {
            store.remember(agent_id, &content, source, &scope, metadata)
        })
        .await
        .map_err(|e| CaptainError::Internal(e.to_string()))?
    }

    async fn recall(
        &self,
        query: &str,
        limit: usize,
        filter: Option<MemoryFilter>,
    ) -> CaptainResult<Vec<MemoryFragment>> {
        let store = self.semantic.clone();
        let query = query.to_string();
        tokio::task::spawn_blocking(move || store.recall(&query, limit, filter))
            .await
            .map_err(|e| CaptainError::Internal(e.to_string()))?
    }

    async fn forget(&self, id: MemoryId) -> CaptainResult<()> {
        let store = self.semantic.clone();
        tokio::task::spawn_blocking(move || store.forget(id))
            .await
            .map_err(|e| CaptainError::Internal(e.to_string()))?
    }

    async fn add_entity(&self, entity: Entity) -> CaptainResult<String> {
        let store = self.knowledge.clone();
        tokio::task::spawn_blocking(move || store.add_entity(entity))
            .await
            .map_err(|e| CaptainError::Internal(e.to_string()))?
    }

    async fn add_relation(&self, relation: Relation) -> CaptainResult<String> {
        let store = self.knowledge.clone();
        tokio::task::spawn_blocking(move || store.add_relation(relation))
            .await
            .map_err(|e| CaptainError::Internal(e.to_string()))?
    }

    async fn query_graph(&self, pattern: GraphPattern) -> CaptainResult<Vec<GraphMatch>> {
        let store = self.knowledge.clone();
        tokio::task::spawn_blocking(move || store.query_graph(pattern))
            .await
            .map_err(|e| CaptainError::Internal(e.to_string()))?
    }

    async fn consolidate(&self) -> CaptainResult<ConsolidationReport> {
        let engine = self.consolidation.clone();
        tokio::task::spawn_blocking(move || engine.consolidate())
            .await
            .map_err(|e| CaptainError::Internal(e.to_string()))?
    }

    async fn export(&self, format: ExportFormat) -> CaptainResult<Vec<u8>> {
        let _ = format;
        Ok(Vec::new())
    }

    async fn import(&self, _data: &[u8], _format: ExportFormat) -> CaptainResult<ImportReport> {
        Ok(ImportReport {
            entities_imported: 0,
            relations_imported: 0,
            memories_imported: 0,
            errors: vec!["Import not yet implemented in Phase 1".to_string()],
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_substrate_kv() {
        let substrate = MemorySubstrate::open_in_memory(0.1).unwrap();
        let agent_id = AgentId::new();
        substrate
            .set(agent_id, "key", serde_json::json!("value"))
            .await
            .unwrap();
        let val = substrate.get(agent_id, "key").await.unwrap();
        assert_eq!(val, Some(serde_json::json!("value")));
    }

    #[tokio::test]
    async fn test_substrate_remember_recall() {
        let substrate = MemorySubstrate::open_in_memory(0.1).unwrap();
        let agent_id = AgentId::new();
        substrate
            .remember(
                agent_id,
                "Rust is a great language",
                MemorySource::Conversation,
                "episodic",
                HashMap::new(),
            )
            .await
            .unwrap();
        let results = substrate.recall("Rust", 10, None).await.unwrap();
        assert_eq!(results.len(), 1);
    }

    #[tokio::test]
    async fn test_task_post_and_list() {
        let substrate = MemorySubstrate::open_in_memory(0.1).unwrap();
        let id = substrate
            .task_post(
                "Review code",
                "Check the auth module for issues",
                Some("auditor"),
                Some("orchestrator"),
            )
            .await
            .unwrap();
        assert!(!id.is_empty());

        let tasks = substrate.task_list(Some("pending")).await.unwrap();
        assert_eq!(tasks.len(), 1);
        assert_eq!(tasks[0]["title"], "Review code");
        assert_eq!(tasks[0]["assigned_to"], "auditor");
        assert_eq!(tasks[0]["status"], "pending");
    }

    #[tokio::test]
    async fn test_task_claim_and_complete() {
        let substrate = MemorySubstrate::open_in_memory(0.1).unwrap();
        let task_id = substrate
            .task_post(
                "Audit endpoint",
                "Security audit the /api/login endpoint",
                Some("auditor"),
                None,
            )
            .await
            .unwrap();

        // Claim the task
        let claimed = substrate.task_claim("auditor").await.unwrap();
        assert!(claimed.is_some());
        let claimed = claimed.unwrap();
        assert_eq!(claimed["id"], task_id);
        assert_eq!(claimed["status"], "in_progress");

        // Complete the task
        substrate
            .task_complete(&task_id, "No vulnerabilities found")
            .await
            .unwrap();

        // Verify it shows as completed
        let tasks = substrate.task_list(Some("completed")).await.unwrap();
        assert_eq!(tasks.len(), 1);
        assert_eq!(tasks[0]["result"], "No vulnerabilities found");
    }

    #[tokio::test]
    async fn test_task_claim_empty() {
        let substrate = MemorySubstrate::open_in_memory(0.1).unwrap();
        let claimed = substrate.task_claim("nobody").await.unwrap();
        assert!(claimed.is_none());
    }
}
