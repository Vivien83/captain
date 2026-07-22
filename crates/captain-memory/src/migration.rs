//! SQLite schema creation and migration.
//!
//! Creates all tables needed by the memory substrate on first boot.

use rusqlite::Connection;

/// Current schema version.
const SCHEMA_VERSION: u32 = 32;

/// Run all migrations to bring the database up to date.
pub fn run_migrations(conn: &Connection) -> Result<(), rusqlite::Error> {
    let current_version = get_schema_version(conn);

    if current_version < 1 {
        migrate_v1(conn)?;
    }

    if current_version < 2 {
        migrate_v2(conn)?;
    }

    if current_version < 3 {
        migrate_v3(conn)?;
    }

    if current_version < 4 {
        migrate_v4(conn)?;
    }

    if current_version < 5 {
        migrate_v5(conn)?;
    }

    if current_version < 6 {
        migrate_v6(conn)?;
    }

    if current_version < 7 {
        migrate_v7(conn)?;
    }

    if current_version < 8 {
        migrate_v8(conn)?;
    }

    if current_version < 9 {
        migrate_v9(conn)?;
    }

    if current_version < 10 {
        migrate_v10(conn)?;
    }

    if current_version < 11 {
        migrate_v11(conn)?;
    }

    if current_version < 12 {
        migrate_v12(conn)?;
    }

    if current_version < 13 {
        migrate_v13(conn)?;
    }

    if current_version < 14 {
        migrate_v14(conn)?;
    }

    if current_version < 15 {
        migrate_v15(conn)?;
    }

    if current_version < 16 {
        migrate_v16(conn)?;
    }

    if current_version < 17 {
        migrate_v17(conn)?;
    }

    if current_version < 18 {
        migrate_v18(conn)?;
    }

    if current_version < 19 {
        migrate_v19(conn)?;
    }

    if current_version < 20 {
        migrate_v20(conn)?;
    }

    if current_version < 21 {
        migrate_v21(conn)?;
    }

    if current_version < 22 {
        migrate_v22(conn)?;
    }

    if current_version < 23 {
        migrate_v23(conn)?;
    }

    if current_version < 24 {
        migrate_v24(conn)?;
    }

    if current_version < 25 {
        migrate_v25(conn)?;
    }

    if current_version < 26 {
        migrate_v26(conn)?;
    }

    if current_version < 27 {
        migrate_v27(conn)?;
    }

    if current_version < 28 {
        migrate_v28(conn)?;
    }

    if current_version < 29 {
        migrate_v29(conn)?;
    }

    if current_version < 30 {
        migrate_v30(conn)?;
    }

    if current_version < 31 {
        migrate_v31(conn)?;
    }

    if current_version < 32 {
        migrate_v32(conn)?;
    }

    set_schema_version(conn, SCHEMA_VERSION)?;
    Ok(())
}

/// Get the current schema version from the database.
fn get_schema_version(conn: &Connection) -> u32 {
    conn.pragma_query_value(None, "user_version", |row| row.get(0))
        .unwrap_or(0)
}

/// Check if a column exists in a table (SQLite has no ADD COLUMN IF NOT EXISTS).
fn column_exists(conn: &Connection, table: &str, column: &str) -> bool {
    let sql = format!("PRAGMA table_info({})", table);
    let Ok(mut stmt) = conn.prepare(&sql) else {
        return false;
    };
    let Ok(rows) = stmt.query_map([], |row| row.get::<_, String>(1)) else {
        return false;
    };
    let names: Vec<String> = rows.filter_map(|r| r.ok()).collect();
    names.iter().any(|n| n == column)
}

fn table_exists(conn: &Connection, table: &str) -> Result<bool, rusqlite::Error> {
    conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = ?1)",
        [table],
        |row| row.get(0),
    )
}

/// Set the schema version in the database.
fn set_schema_version(conn: &Connection, version: u32) -> Result<(), rusqlite::Error> {
    conn.pragma_update(None, "user_version", version)
}

/// Version 1: Create all core tables.
fn migrate_v1(conn: &Connection) -> Result<(), rusqlite::Error> {
    conn.execute_batch(
        "
        -- Agent registry
        CREATE TABLE IF NOT EXISTS agents (
            id TEXT PRIMARY KEY,
            name TEXT NOT NULL,
            manifest BLOB NOT NULL,
            state TEXT NOT NULL,
            created_at TEXT NOT NULL,
            updated_at TEXT NOT NULL
        );

        -- Session history
        CREATE TABLE IF NOT EXISTS sessions (
            id TEXT PRIMARY KEY,
            agent_id TEXT NOT NULL,
            messages BLOB NOT NULL,
            context_window_tokens INTEGER DEFAULT 0,
            created_at TEXT NOT NULL,
            updated_at TEXT NOT NULL
        );

        -- Event log
        CREATE TABLE IF NOT EXISTS events (
            id TEXT PRIMARY KEY,
            source_agent TEXT NOT NULL,
            target TEXT NOT NULL,
            payload BLOB NOT NULL,
            timestamp TEXT NOT NULL
        );
        CREATE INDEX IF NOT EXISTS idx_events_timestamp ON events(timestamp);
        CREATE INDEX IF NOT EXISTS idx_events_source ON events(source_agent);

        -- Key-value store (per-agent)
        CREATE TABLE IF NOT EXISTS kv_store (
            agent_id TEXT NOT NULL,
            key TEXT NOT NULL,
            value BLOB NOT NULL,
            version INTEGER NOT NULL DEFAULT 1,
            updated_at TEXT NOT NULL,
            PRIMARY KEY (agent_id, key)
        );

        -- Task queue
        CREATE TABLE IF NOT EXISTS task_queue (
            id TEXT PRIMARY KEY,
            agent_id TEXT NOT NULL,
            task_type TEXT NOT NULL,
            payload BLOB NOT NULL,
            status TEXT NOT NULL DEFAULT 'pending',
            priority INTEGER NOT NULL DEFAULT 0,
            scheduled_at TEXT,
            created_at TEXT NOT NULL,
            completed_at TEXT
        );
        CREATE INDEX IF NOT EXISTS idx_task_status_priority ON task_queue(status, priority DESC);

        -- Semantic memories
        CREATE TABLE IF NOT EXISTS memories (
            id TEXT PRIMARY KEY,
            agent_id TEXT NOT NULL,
            content TEXT NOT NULL,
            source TEXT NOT NULL,
            scope TEXT NOT NULL DEFAULT 'episodic',
            confidence REAL NOT NULL DEFAULT 1.0,
            metadata TEXT NOT NULL DEFAULT '{}',
            created_at TEXT NOT NULL,
            accessed_at TEXT NOT NULL,
            access_count INTEGER NOT NULL DEFAULT 0,
            deleted INTEGER NOT NULL DEFAULT 0
        );
        CREATE INDEX IF NOT EXISTS idx_memories_agent ON memories(agent_id);
        CREATE INDEX IF NOT EXISTS idx_memories_scope ON memories(scope);

        -- Knowledge graph entities
        CREATE TABLE IF NOT EXISTS entities (
            id TEXT PRIMARY KEY,
            entity_type TEXT NOT NULL,
            name TEXT NOT NULL,
            properties TEXT NOT NULL DEFAULT '{}',
            created_at TEXT NOT NULL,
            updated_at TEXT NOT NULL
        );

        -- Knowledge graph relations
        CREATE TABLE IF NOT EXISTS relations (
            id TEXT PRIMARY KEY,
            source_entity TEXT NOT NULL,
            relation_type TEXT NOT NULL,
            target_entity TEXT NOT NULL,
            properties TEXT NOT NULL DEFAULT '{}',
            confidence REAL NOT NULL DEFAULT 1.0,
            created_at TEXT NOT NULL
        );
        CREATE INDEX IF NOT EXISTS idx_relations_source ON relations(source_entity);
        CREATE INDEX IF NOT EXISTS idx_relations_target ON relations(target_entity);
        CREATE INDEX IF NOT EXISTS idx_relations_type ON relations(relation_type);

        -- Migration tracking
        CREATE TABLE IF NOT EXISTS migrations (
            version INTEGER PRIMARY KEY,
            applied_at TEXT NOT NULL,
            description TEXT
        );

        INSERT OR IGNORE INTO migrations (version, applied_at, description)
        VALUES (1, datetime('now'), 'Initial schema');
        ",
    )?;
    Ok(())
}

/// Version 2: Add collaboration columns to task_queue for agent task delegation.
fn migrate_v2(conn: &Connection) -> Result<(), rusqlite::Error> {
    // SQLite requires one ALTER TABLE per statement; check before adding
    let cols = [
        ("title", "TEXT DEFAULT ''"),
        ("description", "TEXT DEFAULT ''"),
        ("assigned_to", "TEXT DEFAULT ''"),
        ("created_by", "TEXT DEFAULT ''"),
        ("result", "TEXT DEFAULT ''"),
    ];
    for (name, typedef) in &cols {
        if !column_exists(conn, "task_queue", name) {
            conn.execute(
                &format!("ALTER TABLE task_queue ADD COLUMN {} {}", name, typedef),
                [],
            )?;
        }
    }

    conn.execute(
        "INSERT OR IGNORE INTO migrations (version, applied_at, description) VALUES (2, datetime('now'), 'Add collaboration columns to task_queue')",
        [],
    )?;

    Ok(())
}

/// Version 3: Add embedding column to memories table for vector search.
fn migrate_v3(conn: &Connection) -> Result<(), rusqlite::Error> {
    if !column_exists(conn, "memories", "embedding") {
        conn.execute(
            "ALTER TABLE memories ADD COLUMN embedding BLOB DEFAULT NULL",
            [],
        )?;
    }
    conn.execute(
        "INSERT OR IGNORE INTO migrations (version, applied_at, description) VALUES (3, datetime('now'), 'Add embedding column to memories')",
        [],
    )?;
    Ok(())
}

/// Version 4: Add usage_events table for cost tracking and metering.
fn migrate_v4(conn: &Connection) -> Result<(), rusqlite::Error> {
    conn.execute_batch(
        "
        CREATE TABLE IF NOT EXISTS usage_events (
            id TEXT PRIMARY KEY,
            agent_id TEXT NOT NULL,
            timestamp TEXT NOT NULL,
            model TEXT NOT NULL,
            input_tokens INTEGER NOT NULL DEFAULT 0,
            output_tokens INTEGER NOT NULL DEFAULT 0,
            cached_input_tokens INTEGER NOT NULL DEFAULT 0,
            cache_creation_tokens INTEGER NOT NULL DEFAULT 0,
            cost_usd REAL NOT NULL DEFAULT 0.0,
            tool_calls INTEGER NOT NULL DEFAULT 0
        );
        CREATE INDEX IF NOT EXISTS idx_usage_agent_time ON usage_events(agent_id, timestamp);
        CREATE INDEX IF NOT EXISTS idx_usage_timestamp ON usage_events(timestamp);

        INSERT OR IGNORE INTO migrations (version, applied_at, description)
        VALUES (4, datetime('now'), 'Add usage_events table for cost tracking');
        ",
    )?;
    Ok(())
}

/// Version 5: Add canonical_sessions table for cross-channel persistent memory.
fn migrate_v5(conn: &Connection) -> Result<(), rusqlite::Error> {
    conn.execute_batch(
        "
        CREATE TABLE IF NOT EXISTS canonical_sessions (
            agent_id TEXT PRIMARY KEY,
            messages BLOB NOT NULL,
            compaction_cursor INTEGER NOT NULL DEFAULT 0,
            compacted_summary TEXT,
            updated_at TEXT NOT NULL
        );

        INSERT OR IGNORE INTO migrations (version, applied_at, description)
        VALUES (5, datetime('now'), 'Add canonical_sessions for cross-channel memory');
        ",
    )?;
    Ok(())
}

/// Version 6: Add label column to sessions table.
fn migrate_v6(conn: &Connection) -> Result<(), rusqlite::Error> {
    // Check if column already exists before ALTER (SQLite has no ADD COLUMN IF NOT EXISTS)
    if !column_exists(conn, "sessions", "label") {
        conn.execute("ALTER TABLE sessions ADD COLUMN label TEXT", [])?;
    }
    conn.execute(
        "INSERT OR IGNORE INTO migrations (version, applied_at, description) VALUES (6, datetime('now'), 'Add label column to sessions for human-readable labels')",
        [],
    )?;
    Ok(())
}

/// Version 7: Add paired_devices table for device pairing persistence.
fn migrate_v7(conn: &Connection) -> Result<(), rusqlite::Error> {
    conn.execute_batch(
        "
        CREATE TABLE IF NOT EXISTS paired_devices (
            device_id TEXT PRIMARY KEY,
            display_name TEXT NOT NULL,
            platform TEXT NOT NULL,
            paired_at TEXT NOT NULL,
            last_seen TEXT NOT NULL,
            push_token TEXT
        );

        INSERT OR IGNORE INTO migrations (version, applied_at, description)
        VALUES (7, datetime('now'), 'Add paired_devices table for device pairing');
        ",
    )?;
    Ok(())
}

/// Version 8: Add audit_entries table for persistent Merkle audit trail.
fn migrate_v8(conn: &Connection) -> Result<(), rusqlite::Error> {
    conn.execute_batch(
        "
        CREATE TABLE IF NOT EXISTS audit_entries (
            seq INTEGER PRIMARY KEY,
            timestamp TEXT NOT NULL,
            agent_id TEXT NOT NULL,
            action TEXT NOT NULL,
            detail TEXT NOT NULL,
            outcome TEXT NOT NULL,
            prev_hash TEXT NOT NULL,
            hash TEXT NOT NULL
        );
        CREATE INDEX IF NOT EXISTS idx_audit_agent ON audit_entries(agent_id);
        CREATE INDEX IF NOT EXISTS idx_audit_timestamp ON audit_entries(timestamp);
        CREATE INDEX IF NOT EXISTS idx_audit_action ON audit_entries(action);

        INSERT OR IGNORE INTO migrations (version, applied_at, description)
        VALUES (8, datetime('now'), 'Add audit_entries table for persistent Merkle audit trail');
        ",
    )?;
    Ok(())
}

/// Version 9: Add sessions_events table for timeline replay (v3.9f).
fn migrate_v9(conn: &Connection) -> Result<(), rusqlite::Error> {
    conn.execute_batch(
        "
        CREATE TABLE IF NOT EXISTS sessions_events (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            session_id TEXT NOT NULL,
            ts INTEGER NOT NULL,
            event_type TEXT NOT NULL,
            payload TEXT NOT NULL
        );
        CREATE INDEX IF NOT EXISTS idx_sessions_events_session_ts
            ON sessions_events(session_id, ts);
        CREATE INDEX IF NOT EXISTS idx_sessions_events_ts
            ON sessions_events(ts);

        INSERT OR IGNORE INTO migrations (version, applied_at, description)
        VALUES (9, datetime('now'), 'Add sessions_events table for timeline replay');
        ",
    )?;
    Ok(())
}

/// Version 10: Add projects table for v3.11a (project entity + CRUD).
fn migrate_v10(conn: &Connection) -> Result<(), rusqlite::Error> {
    conn.execute_batch(
        "
        CREATE TABLE IF NOT EXISTS projects (
            id TEXT PRIMARY KEY,
            name TEXT NOT NULL,
            slug TEXT NOT NULL UNIQUE,
            goal TEXT NOT NULL DEFAULT '',
            status TEXT NOT NULL DEFAULT 'planning',
            deadline INTEGER,
            created_at INTEGER NOT NULL,
            updated_at INTEGER NOT NULL,
            metadata_json TEXT NOT NULL DEFAULT '{}'
        );
        CREATE INDEX IF NOT EXISTS idx_projects_status ON projects(status);
        CREATE INDEX IF NOT EXISTS idx_projects_updated_at ON projects(updated_at);

        INSERT OR IGNORE INTO migrations (version, applied_at, description)
        VALUES (10, datetime('now'), 'Add projects table for v3.11 projects & memory');
        ",
    )?;
    Ok(())
}

/// Version 11: Add project_tasks table for v3.11b (task graph per project).
///
/// Named `project_tasks` (not `tasks`) to avoid conflicting with the
/// v1 `task_queue` table which serves a different purpose (background
/// work queue for agents). `parent_id` is nullable to model sub-task
/// DAGs without requiring a forest root.
fn migrate_v11(conn: &Connection) -> Result<(), rusqlite::Error> {
    conn.execute_batch(
        "
        CREATE TABLE IF NOT EXISTS project_tasks (
            id TEXT PRIMARY KEY,
            project_id TEXT NOT NULL,
            parent_id TEXT,
            title TEXT NOT NULL,
            description TEXT NOT NULL DEFAULT '',
            status TEXT NOT NULL DEFAULT 'todo',
            assignee_agent_id TEXT,
            priority INTEGER NOT NULL DEFAULT 0,
            deadline INTEGER,
            created_at INTEGER NOT NULL,
            updated_at INTEGER NOT NULL,
            completed_at INTEGER,
            FOREIGN KEY (project_id) REFERENCES projects(id) ON DELETE CASCADE,
            FOREIGN KEY (parent_id) REFERENCES project_tasks(id) ON DELETE SET NULL
        );
        CREATE INDEX IF NOT EXISTS idx_project_tasks_project ON project_tasks(project_id);
        CREATE INDEX IF NOT EXISTS idx_project_tasks_parent ON project_tasks(parent_id);
        CREATE INDEX IF NOT EXISTS idx_project_tasks_status ON project_tasks(status);

        INSERT OR IGNORE INTO migrations (version, applied_at, description)
        VALUES (11, datetime('now'), 'Add project_tasks table for v3.11b task graph');
        ",
    )?;
    Ok(())
}

/// Version 12: Add milestones table for v3.11c.
fn migrate_v12(conn: &Connection) -> Result<(), rusqlite::Error> {
    conn.execute_batch(
        "
        CREATE TABLE IF NOT EXISTS milestones (
            id TEXT PRIMARY KEY,
            project_id TEXT NOT NULL,
            name TEXT NOT NULL,
            due_date INTEGER,
            status TEXT NOT NULL DEFAULT 'upcoming',
            deliverables_json TEXT NOT NULL DEFAULT '[]',
            completed_at INTEGER,
            created_at INTEGER NOT NULL,
            updated_at INTEGER NOT NULL,
            FOREIGN KEY (project_id) REFERENCES projects(id) ON DELETE CASCADE
        );
        CREATE INDEX IF NOT EXISTS idx_milestones_project ON milestones(project_id);
        CREATE INDEX IF NOT EXISTS idx_milestones_due_date ON milestones(due_date);

        INSERT OR IGNORE INTO migrations (version, applied_at, description)
        VALUES (12, datetime('now'), 'Add milestones table for v3.11c');
        ",
    )?;
    Ok(())
}

/// Version 13: Add project_checkpoints table for v3.11g handoff protocol.
fn migrate_v13(conn: &Connection) -> Result<(), rusqlite::Error> {
    conn.execute_batch(
        "
        CREATE TABLE IF NOT EXISTS project_checkpoints (
            id TEXT PRIMARY KEY,
            project_id TEXT NOT NULL,
            session_id TEXT,
            summary TEXT NOT NULL,
            state_json TEXT NOT NULL DEFAULT '{}',
            created_at INTEGER NOT NULL,
            FOREIGN KEY (project_id) REFERENCES projects(id) ON DELETE CASCADE
        );
        CREATE INDEX IF NOT EXISTS idx_checkpoints_project_created
            ON project_checkpoints(project_id, created_at DESC);

        INSERT OR IGNORE INTO migrations (version, applied_at, description)
        VALUES (13, datetime('now'), 'Add project_checkpoints for v3.11g handoff');
        ",
    )?;
    Ok(())
}

/// Version 14: Add memory_writes table for v3.12a write-through memory_writer.
///
/// Captures every memory write (from `memory_store` tool, `mirror_to_mempalace`,
/// or the future LearningCommitter) so it can be replayed to MemPalace if
/// that backend is momentarily down. Migration 23 promotes this table into
/// Captain's durable continuity journal while MemPalace remains the semantic
/// index derived from it.
///
/// `sync_status`: 'pending' (awaiting MemPalace), 'synced' (confirmed),
/// 'error' (degraded after repeated failures; migration 23 keeps it retryable).
fn migrate_v14(conn: &Connection) -> Result<(), rusqlite::Error> {
    conn.execute_batch(
        "
        CREATE TABLE IF NOT EXISTS memory_writes (
            id TEXT PRIMARY KEY,
            subject TEXT NOT NULL,
            predicate TEXT NOT NULL,
            object TEXT NOT NULL,
            wing TEXT,
            room TEXT,
            source TEXT NOT NULL,
            sync_status TEXT NOT NULL DEFAULT 'pending',
            sync_attempts INTEGER NOT NULL DEFAULT 0,
            created_at INTEGER NOT NULL,
            synced_at INTEGER,
            last_error TEXT
        );
        CREATE INDEX IF NOT EXISTS idx_memory_writes_sync_status
            ON memory_writes(sync_status);
        CREATE INDEX IF NOT EXISTS idx_memory_writes_created_at
            ON memory_writes(created_at);

        INSERT OR IGNORE INTO migrations (version, applied_at, description)
        VALUES (14, datetime('now'), 'Add memory_writes for v3.12a write-through');
        ",
    )?;
    Ok(())
}

/// Version 15: Add learning_review_queue for v3.12g approval mode.
///
/// Holds MemoryCandidate rows that await human approval before being
/// committed to MemPalace. `decision` is NULL while pending; becomes
/// 'approved' or 'denied' on decide. Approved items are additionally
/// written through via memory_writer and the `written_write_id`
/// column points to the `memory_writes` row for audit.
fn migrate_v15(conn: &Connection) -> Result<(), rusqlite::Error> {
    conn.execute_batch(
        "
        CREATE TABLE IF NOT EXISTS learning_review_queue (
            id TEXT PRIMARY KEY,
            outcome TEXT NOT NULL,
            agent_id TEXT NOT NULL,
            wing TEXT NOT NULL,
            room TEXT NOT NULL,
            subject TEXT NOT NULL,
            predicate TEXT NOT NULL,
            object TEXT NOT NULL,
            confidence REAL NOT NULL,
            source TEXT NOT NULL,
            created_at INTEGER NOT NULL,
            decided_at INTEGER,
            decided_by TEXT,
            decision TEXT,
            written_write_id TEXT
        );
        CREATE INDEX IF NOT EXISTS idx_review_queue_decision
            ON learning_review_queue(decision);
        CREATE INDEX IF NOT EXISTS idx_review_queue_created_at
            ON learning_review_queue(created_at);

        INSERT OR IGNORE INTO migrations (version, applied_at, description)
        VALUES (15, datetime('now'), 'Add learning_review_queue for v3.12g approval mode');
        ",
    )?;
    Ok(())
}

/// Version 16: Add skill_patterns for v3.13a SkillSynthesizer.
///
/// Tracks recurring tool sequences observed per agent. The
/// `pattern_detector` increments the count for a `(agent_id, tool
/// sequence)` pair; once `count` crosses the configured threshold the
/// row is forwarded to the `SkillProposer` (LLM judge). `proposed_at`
/// is stamped after the first proposal so the same pattern is not
/// re-proposed indefinitely.
fn migrate_v16(conn: &Connection) -> Result<(), rusqlite::Error> {
    conn.execute_batch(
        "
        CREATE TABLE IF NOT EXISTS skill_patterns (
            hash TEXT PRIMARY KEY,
            agent_id TEXT NOT NULL,
            tool_sequence_json TEXT NOT NULL,
            first_seen INTEGER NOT NULL,
            last_seen INTEGER NOT NULL,
            count INTEGER NOT NULL DEFAULT 1,
            proposed_at INTEGER
        );
        CREATE INDEX IF NOT EXISTS idx_skill_patterns_agent
            ON skill_patterns(agent_id);
        CREATE INDEX IF NOT EXISTS idx_skill_patterns_count
            ON skill_patterns(count DESC);
        CREATE INDEX IF NOT EXISTS idx_skill_patterns_last_seen
            ON skill_patterns(last_seen DESC);

        INSERT OR IGNORE INTO migrations (version, applied_at, description)
        VALUES (16, datetime('now'), 'Add skill_patterns for v3.13 SkillSynthesizer');
        ",
    )?;
    Ok(())
}

/// Version 17: Add skill_proposals for v3.13c review queue.
///
/// Holds drafted skill proposals from the SkillProposer (v3.13b)
/// awaiting human approval. `status` is NULL while pending, becomes
/// 'approved' or 'denied' on decide. `written_path` records where the
/// SkillWriter (v3.13d) deposited the generated `.md` file once
/// approved — kept for audit and to allow reverts.
fn migrate_v17(conn: &Connection) -> Result<(), rusqlite::Error> {
    conn.execute_batch(
        "
        CREATE TABLE IF NOT EXISTS skill_proposals (
            id TEXT PRIMARY KEY,
            pattern_hash TEXT NOT NULL,
            name TEXT NOT NULL,
            description TEXT NOT NULL,
            trigger_hint TEXT NOT NULL DEFAULT '',
            tool_sequence_json TEXT NOT NULL,
            arg_schema_hint TEXT NOT NULL DEFAULT '',
            confidence REAL NOT NULL,
            source_agent_id TEXT NOT NULL,
            status TEXT,
            created_at INTEGER NOT NULL,
            decided_at INTEGER,
            decided_by TEXT,
            written_path TEXT
        );
        CREATE INDEX IF NOT EXISTS idx_skill_proposals_status
            ON skill_proposals(status);
        CREATE INDEX IF NOT EXISTS idx_skill_proposals_pattern_hash
            ON skill_proposals(pattern_hash);
        CREATE INDEX IF NOT EXISTS idx_skill_proposals_created_at
            ON skill_proposals(created_at);

        INSERT OR IGNORE INTO migrations (version, applied_at, description)
        VALUES (17, datetime('now'), 'Add skill_proposals for v3.13c review queue');
        ",
    )?;
    Ok(())
}

/// Version 18: remember the origin channel of generated skill proposals.
///
/// The SkillSynthesizer runs asynchronously after tool-heavy turns. Without
/// the origin channel, a queued proposal can only appear in the dashboard and
/// not in the conversation that triggered it. This column lets CLI/Telegram
/// receive the same visible approval prompt as memory learning.
fn migrate_v18(conn: &Connection) -> Result<(), rusqlite::Error> {
    conn.execute_batch(
        "
        ALTER TABLE skill_proposals ADD COLUMN origin_channel TEXT;

        INSERT OR IGNORE INTO migrations (version, applied_at, description)
        VALUES (18, datetime('now'), 'Add origin_channel to skill_proposals');
        ",
    )
    .or_else(|e| {
        if e.to_string().contains("duplicate column name") {
            conn.execute(
                "INSERT OR IGNORE INTO migrations (version, applied_at, description)
                 VALUES (18, datetime('now'), 'Add origin_channel to skill_proposals')",
                [],
            )?;
            Ok(())
        } else {
            Err(e)
        }
    })
}

/// Version 19: Add cross-session `todos` table.
///
/// Global capture surface (no project FK, no agent FK), distinct from
/// `project_tasks` (project DAG) and `goals` (autopilot loops). One row =
/// one durable todo item that survives daemon restarts and conversation
/// compactions.
fn migrate_v19(conn: &Connection) -> Result<(), rusqlite::Error> {
    conn.execute_batch(
        "
        CREATE TABLE IF NOT EXISTS todos (
            id TEXT PRIMARY KEY,
            title TEXT NOT NULL,
            body TEXT NOT NULL DEFAULT '',
            done INTEGER NOT NULL DEFAULT 0,
            created_at INTEGER NOT NULL,
            completed_at INTEGER
        );
        CREATE INDEX IF NOT EXISTS idx_todos_done ON todos(done);
        CREATE INDEX IF NOT EXISTS idx_todos_created_at ON todos(created_at);

        INSERT OR IGNORE INTO migrations (version, applied_at, description)
        VALUES (19, datetime('now'), 'Add cross-session todos table');
        ",
    )?;
    Ok(())
}

/// Version 20: Add prompt-cache telemetry to usage events.
fn migrate_v20(conn: &Connection) -> Result<(), rusqlite::Error> {
    if !column_exists(conn, "usage_events", "cached_input_tokens") {
        conn.execute(
            "ALTER TABLE usage_events ADD COLUMN cached_input_tokens INTEGER NOT NULL DEFAULT 0",
            [],
        )?;
    }
    if !column_exists(conn, "usage_events", "cache_creation_tokens") {
        conn.execute(
            "ALTER TABLE usage_events ADD COLUMN cache_creation_tokens INTEGER NOT NULL DEFAULT 0",
            [],
        )?;
    }
    conn.execute(
        "INSERT OR IGNORE INTO migrations (version, applied_at, description)
         VALUES (20, datetime('now'), 'Add prompt-cache telemetry to usage_events')",
        [],
    )?;
    Ok(())
}

/// Version 21: Add discovery family metadata to generated skill proposals.
fn migrate_v21(conn: &Connection) -> Result<(), rusqlite::Error> {
    if !column_exists(conn, "skill_proposals", "family") {
        conn.execute(
            "ALTER TABLE skill_proposals ADD COLUMN family TEXT NOT NULL DEFAULT 'general-automation'",
            [],
        )?;
    }
    conn.execute(
        "INSERT OR IGNORE INTO migrations (version, applied_at, description)
         VALUES (21, datetime('now'), 'Add family to skill_proposals')",
        [],
    )?;
    Ok(())
}

/// Version 22: Add detached_tool_runs so long-running detached tool runs
/// (tool_run_start) survive a Captain restart instead of vanishing from
/// the in-memory registry (crates/captain-runtime/src/tool_runs.rs).
fn migrate_v22(conn: &Connection) -> Result<(), rusqlite::Error> {
    conn.execute_batch(
        "
        CREATE TABLE IF NOT EXISTS detached_tool_runs (
            run_id TEXT PRIMARY KEY,
            tool_name TEXT NOT NULL,
            status TEXT NOT NULL,
            caller_agent_id TEXT,
            origin_tool_use_id TEXT,
            started_at INTEGER NOT NULL,
            finished_at INTEGER,
            is_error INTEGER,
            result TEXT,
            result_truncated INTEGER NOT NULL DEFAULT 0
        );
        CREATE INDEX IF NOT EXISTS idx_detached_tool_runs_status ON detached_tool_runs(status);
        CREATE INDEX IF NOT EXISTS idx_detached_tool_runs_started_at ON detached_tool_runs(started_at);

        INSERT OR IGNORE INTO migrations (version, applied_at, description)
        VALUES (22, datetime('now'), 'Add detached_tool_runs table');
        ",
    )?;
    Ok(())
}

/// Version 23: Make the local memory journal durably retryable.
///
/// MemPalace is the semantic index, while `memory_writes` is Captain's local
/// continuity journal. Retry metadata must therefore survive restarts and an
/// exhausted retry budget must never make a fact disappear permanently.
/// `operation` and `retracted_at` prepare the same journal for durable
/// invalidations without changing existing add rows.
fn migrate_v23(conn: &Connection) -> Result<(), rusqlite::Error> {
    if !column_exists(conn, "memory_writes", "operation") {
        conn.execute(
            "ALTER TABLE memory_writes ADD COLUMN operation TEXT NOT NULL DEFAULT 'add'",
            [],
        )?;
    }
    if !column_exists(conn, "memory_writes", "last_attempt_at") {
        conn.execute(
            "ALTER TABLE memory_writes ADD COLUMN last_attempt_at INTEGER",
            [],
        )?;
    }
    if !column_exists(conn, "memory_writes", "next_retry_at") {
        conn.execute(
            "ALTER TABLE memory_writes ADD COLUMN next_retry_at INTEGER",
            [],
        )?;
    }
    if !column_exists(conn, "memory_writes", "retracted_at") {
        conn.execute(
            "ALTER TABLE memory_writes ADD COLUMN retracted_at INTEGER",
            [],
        )?;
    }
    conn.execute_batch(
        "
        CREATE INDEX IF NOT EXISTS idx_memory_writes_retry
            ON memory_writes(sync_status, next_retry_at, created_at);
        CREATE INDEX IF NOT EXISTS idx_memory_writes_active
            ON memory_writes(retracted_at, created_at);

        INSERT OR IGNORE INTO migrations (version, applied_at, description)
        VALUES (23, datetime('now'), 'Add durable retry metadata to memory_writes');
        ",
    )?;
    Ok(())
}

/// Version 24: Persist provider-owned subscription quota observations.
fn migrate_v24(conn: &Connection) -> Result<(), rusqlite::Error> {
    conn.execute_batch(
        "
        CREATE TABLE IF NOT EXISTS provider_quota_snapshots (
            provider TEXT NOT NULL,
            limit_id TEXT NOT NULL,
            snapshot_json TEXT NOT NULL,
            alert_level TEXT NOT NULL,
            observed_at TEXT NOT NULL,
            updated_at TEXT NOT NULL,
            PRIMARY KEY (provider, limit_id)
        );
        CREATE INDEX IF NOT EXISTS idx_provider_quota_snapshots_observed
            ON provider_quota_snapshots(observed_at DESC);

        CREATE TABLE IF NOT EXISTS provider_quota_events (
            id TEXT PRIMARY KEY,
            provider TEXT NOT NULL,
            limit_id TEXT NOT NULL,
            change_kind TEXT NOT NULL,
            alert_level TEXT NOT NULL,
            snapshot_json TEXT NOT NULL,
            observed_at TEXT NOT NULL
        );
        CREATE INDEX IF NOT EXISTS idx_provider_quota_events_observed
            ON provider_quota_events(observed_at DESC);

        INSERT OR IGNORE INTO migrations (version, applied_at, description)
        VALUES (24, datetime('now'), 'Add provider subscription quota snapshots and events');
        ",
    )?;
    Ok(())
}

/// Version 25: Persist workflow episodes and their tool attempts.
fn migrate_v25(conn: &Connection) -> Result<(), rusqlite::Error> {
    conn.execute_batch(
        "
        CREATE TABLE IF NOT EXISTS workflow_episodes (
            id TEXT PRIMARY KEY,
            session_id TEXT NOT NULL,
            turn_id TEXT NOT NULL,
            agent_id TEXT NOT NULL,
            origin_channel TEXT,
            project_id TEXT,
            workspace_scope TEXT,
            intent_redacted TEXT NOT NULL,
            intent_fingerprint TEXT NOT NULL,
            status TEXT NOT NULL DEFAULT 'running',
            explicit_reuse_request INTEGER NOT NULL DEFAULT 0,
            tool_attempt_count INTEGER NOT NULL DEFAULT 0,
            success_count INTEGER NOT NULL DEFAULT 0,
            failure_count INTEGER NOT NULL DEFAULT 0,
            has_secret_input INTEGER NOT NULL DEFAULT 0,
            has_unverified_mutation INTEGER NOT NULL DEFAULT 0,
            failure_reason TEXT,
            started_at INTEGER NOT NULL,
            completed_at INTEGER,
            analysis_status TEXT NOT NULL DEFAULT 'pending',
            analysis_reason TEXT,
            analyzed_at INTEGER,
            created_at INTEGER NOT NULL,
            updated_at INTEGER NOT NULL,
            UNIQUE(agent_id, session_id, turn_id),
            CHECK(status IN ('running', 'succeeded', 'failed', 'stopped', 'uncertain')),
            CHECK(analysis_status IN ('pending', 'claimed', 'processed', 'rejected'))
        );
        CREATE INDEX IF NOT EXISTS idx_workflow_episodes_analysis
            ON workflow_episodes(analysis_status, status, completed_at);
        CREATE INDEX IF NOT EXISTS idx_workflow_episodes_session
            ON workflow_episodes(session_id, started_at DESC);
        CREATE INDEX IF NOT EXISTS idx_workflow_episodes_intent
            ON workflow_episodes(intent_fingerprint, completed_at DESC);

        CREATE TABLE IF NOT EXISTS workflow_episode_steps (
            episode_id TEXT NOT NULL,
            tool_use_id TEXT NOT NULL,
            ordinal INTEGER NOT NULL,
            tool_name TEXT NOT NULL,
            dependency_ids_json TEXT NOT NULL DEFAULT '[]',
            input_shape_json TEXT NOT NULL,
            input_fingerprint TEXT NOT NULL,
            effect_class TEXT NOT NULL,
            status TEXT NOT NULL DEFAULT 'running',
            retry_count INTEGER NOT NULL DEFAULT 0,
            output_class TEXT,
            verification_marker TEXT,
            secret_detected INTEGER NOT NULL DEFAULT 0,
            started_at INTEGER NOT NULL,
            completed_at INTEGER,
            duration_ms INTEGER,
            PRIMARY KEY (episode_id, tool_use_id),
            FOREIGN KEY (episode_id) REFERENCES workflow_episodes(id) ON DELETE CASCADE,
            CHECK(effect_class IN ('read', 'write', 'external', 'destructive', 'unknown')),
            CHECK(status IN ('running', 'succeeded', 'failed', 'interrupted', 'uncertain'))
        );
        CREATE INDEX IF NOT EXISTS idx_workflow_episode_steps_order
            ON workflow_episode_steps(episode_id, ordinal, started_at);
        CREATE INDEX IF NOT EXISTS idx_workflow_episode_steps_tool
            ON workflow_episode_steps(tool_name, status, completed_at);

        INSERT OR IGNORE INTO migrations (version, applied_at, description)
        VALUES (25, datetime('now'), 'Add durable workflow episodes and tool attempts');
        ",
    )?;
    Ok(())
}

/// Version 26: Add the crash-safe Skill Learning V2 control plane.
fn migrate_v26(conn: &Connection) -> Result<(), rusqlite::Error> {
    conn.execute_batch(
        "
        CREATE TABLE IF NOT EXISTS workflow_learning_proposals (
            id TEXT PRIMARY KEY,
            idempotency_key TEXT NOT NULL UNIQUE,
            workflow_signature TEXT NOT NULL,
            state TEXT NOT NULL,
            state_version INTEGER NOT NULL DEFAULT 0,
            revision_sha256 TEXT,
            operator_token TEXT,
            artifact_sha256 TEXT,
            staging_job_id TEXT,
            kind TEXT,
            name TEXT,
            source_agent_id TEXT NOT NULL,
            origin_channel TEXT,
            evidence_json TEXT NOT NULL,
            validation_json TEXT,
            snoozed_until INTEGER,
            last_error_code TEXT,
            last_error_message TEXT,
            created_at INTEGER NOT NULL,
            updated_at INTEGER NOT NULL,
            CHECK(state IN (
                'observed', 'eligible', 'drafting', 'validating', 'proposed',
                'dismissed', 'snoozed', 'superseded',
                'approved_pending_install', 'active_canary', 'active',
                'rejected', 'install_failed', 'rolled_back'
            )),
            CHECK(kind IS NULL OR kind IN ('skill', 'capspec', 'automation', 'refinement')),
            CHECK(revision_sha256 IS NULL OR length(revision_sha256) = 64),
            CHECK(operator_token IS NULL OR (
                length(operator_token) = 20
                AND operator_token NOT GLOB '*[^0-9a-f]*'
            )),
            CHECK(artifact_sha256 IS NULL OR length(artifact_sha256) = 64)
        );
        CREATE INDEX IF NOT EXISTS idx_workflow_learning_proposals_state
            ON workflow_learning_proposals(state, updated_at, id);
        CREATE INDEX IF NOT EXISTS idx_workflow_learning_proposals_signature
            ON workflow_learning_proposals(workflow_signature, created_at DESC);
        CREATE UNIQUE INDEX IF NOT EXISTS idx_workflow_learning_proposals_revision
            ON workflow_learning_proposals(revision_sha256)
            WHERE revision_sha256 IS NOT NULL;
        CREATE UNIQUE INDEX IF NOT EXISTS idx_workflow_learning_proposals_operator_token
            ON workflow_learning_proposals(operator_token)
            WHERE operator_token IS NOT NULL;

        CREATE TABLE IF NOT EXISTS workflow_learning_proposal_events (
            sequence INTEGER PRIMARY KEY AUTOINCREMENT,
            idempotency_key TEXT NOT NULL UNIQUE,
            proposal_id TEXT NOT NULL,
            from_state TEXT,
            to_state TEXT NOT NULL,
            resulting_version INTEGER NOT NULL,
            revision_sha256 TEXT,
            actor TEXT NOT NULL,
            reason TEXT NOT NULL,
            created_at INTEGER NOT NULL,
            FOREIGN KEY (proposal_id) REFERENCES workflow_learning_proposals(id) ON DELETE RESTRICT
        );
        CREATE INDEX IF NOT EXISTS idx_workflow_learning_events_proposal
            ON workflow_learning_proposal_events(proposal_id, sequence);

        CREATE TABLE IF NOT EXISTS workflow_learning_jobs (
            id TEXT PRIMARY KEY,
            idempotency_key TEXT NOT NULL UNIQUE,
            proposal_id TEXT NOT NULL,
            revision_sha256 TEXT,
            kind TEXT NOT NULL,
            status TEXT NOT NULL DEFAULT 'pending',
            payload_json TEXT NOT NULL,
            attempt_count INTEGER NOT NULL DEFAULT 0,
            max_attempts INTEGER NOT NULL DEFAULT 3,
            run_after INTEGER NOT NULL,
            lease_owner TEXT,
            lease_expires_at INTEGER,
            effect_state TEXT NOT NULL DEFAULT 'none',
            result_json TEXT,
            error_code TEXT,
            error_message TEXT,
            created_at INTEGER NOT NULL,
            updated_at INTEGER NOT NULL,
            FOREIGN KEY (proposal_id) REFERENCES workflow_learning_proposals(id) ON DELETE RESTRICT,
            CHECK(kind IN ('analyze', 'draft', 'validate', 'install', 'canary', 'rollback')),
            CHECK(status IN ('pending', 'running', 'retry_wait', 'succeeded', 'uncertain', 'dead')),
            CHECK(effect_state IN ('none', 'started', 'completed')),
            CHECK(attempt_count >= 0),
            CHECK(max_attempts BETWEEN 1 AND 20)
        );
        CREATE INDEX IF NOT EXISTS idx_workflow_learning_jobs_due
            ON workflow_learning_jobs(status, run_after, created_at);
        CREATE INDEX IF NOT EXISTS idx_workflow_learning_jobs_proposal
            ON workflow_learning_jobs(proposal_id, created_at);

        CREATE TABLE IF NOT EXISTS workflow_learning_outbox (
            id TEXT PRIMARY KEY,
            idempotency_key TEXT NOT NULL UNIQUE,
            proposal_id TEXT NOT NULL,
            revision_sha256 TEXT,
            topic TEXT NOT NULL,
            payload_json TEXT NOT NULL,
            status TEXT NOT NULL DEFAULT 'pending',
            attempt_count INTEGER NOT NULL DEFAULT 0,
            max_attempts INTEGER NOT NULL DEFAULT 8,
            run_after INTEGER NOT NULL,
            lease_owner TEXT,
            lease_expires_at INTEGER,
            delivery_result_json TEXT,
            last_error TEXT,
            delivered_at INTEGER,
            created_at INTEGER NOT NULL,
            updated_at INTEGER NOT NULL,
            FOREIGN KEY (proposal_id) REFERENCES workflow_learning_proposals(id) ON DELETE RESTRICT,
            CHECK(status IN ('pending', 'delivering', 'retry_wait', 'delivered', 'dead')),
            CHECK(attempt_count >= 0),
            CHECK(max_attempts BETWEEN 1 AND 20)
        );
        CREATE INDEX IF NOT EXISTS idx_workflow_learning_outbox_due
            ON workflow_learning_outbox(status, run_after, created_at);
        CREATE INDEX IF NOT EXISTS idx_workflow_learning_outbox_proposal
            ON workflow_learning_outbox(proposal_id, created_at);

        CREATE TABLE IF NOT EXISTS workflow_learning_installations (
            proposal_id TEXT NOT NULL,
            revision_sha256 TEXT NOT NULL,
            kind TEXT NOT NULL,
            phase TEXT NOT NULL,
            target_locator TEXT NOT NULL,
            backup_locator TEXT,
            backup_sha256 TEXT,
            installed_sha256 TEXT NOT NULL,
            last_error TEXT,
            prepared_at INTEGER NOT NULL,
            promoted_at INTEGER,
            verified_at INTEGER,
            rolled_back_at INTEGER,
            updated_at INTEGER NOT NULL,
            PRIMARY KEY (proposal_id, revision_sha256),
            FOREIGN KEY (proposal_id) REFERENCES workflow_learning_proposals(id) ON DELETE RESTRICT,
            CHECK(kind IN ('skill', 'capspec', 'automation', 'refinement')),
            CHECK(phase IN (
                'prepared', 'promoted', 'verified', 'active',
                'rollback_pending', 'rolled_back', 'quarantined', 'failed'
            )),
            CHECK(length(revision_sha256) = 64),
            CHECK(length(installed_sha256) = 64)
        );

        INSERT OR IGNORE INTO migrations (version, applied_at, description)
        VALUES (26, datetime('now'), 'Add durable workflow-learning proposals, jobs, outbox, and installations');
        ",
    )?;
    Ok(())
}

fn migrate_v27(conn: &Connection) -> Result<(), rusqlite::Error> {
    if !column_exists(conn, "workflow_learning_installations", "phase_version") {
        conn.execute(
            "ALTER TABLE workflow_learning_installations
             ADD COLUMN phase_version INTEGER NOT NULL DEFAULT 0",
            [],
        )?;
    }
    conn.execute_batch(
        "
        CREATE TABLE IF NOT EXISTS workflow_learning_installation_events (
            sequence INTEGER PRIMARY KEY AUTOINCREMENT,
            idempotency_key TEXT NOT NULL UNIQUE,
            proposal_id TEXT NOT NULL,
            revision_sha256 TEXT NOT NULL,
            from_phase TEXT,
            to_phase TEXT NOT NULL,
            resulting_version INTEGER NOT NULL,
            last_error TEXT,
            actor TEXT NOT NULL,
            reason TEXT NOT NULL,
            created_at INTEGER NOT NULL,
            FOREIGN KEY (proposal_id, revision_sha256)
                REFERENCES workflow_learning_installations(proposal_id, revision_sha256)
                ON DELETE RESTRICT,
            CHECK(from_phase IS NULL OR from_phase IN (
                'prepared', 'promoted', 'verified', 'active',
                'rollback_pending', 'rolled_back', 'quarantined', 'failed'
            )),
            CHECK(to_phase IN (
                'prepared', 'promoted', 'verified', 'active',
                'rollback_pending', 'rolled_back', 'quarantined', 'failed'
            )),
            CHECK(length(revision_sha256) = 64),
            CHECK(resulting_version >= 0)
        );
        CREATE INDEX IF NOT EXISTS idx_workflow_learning_installation_events_revision
            ON workflow_learning_installation_events(
                proposal_id, revision_sha256, sequence
            );

        INSERT OR IGNORE INTO migrations (version, applied_at, description)
        VALUES (27, datetime('now'), 'Add CAS audit events to workflow-learning installations');
        ",
    )?;
    if !column_exists(conn, "workflow_learning_installation_events", "last_error") {
        conn.execute(
            "ALTER TABLE workflow_learning_installation_events ADD COLUMN last_error TEXT",
            [],
        )?;
    }
    Ok(())
}

fn migrate_v28(conn: &Connection) -> Result<(), rusqlite::Error> {
    for (column, definition) in [
        ("analysis_result_json", "TEXT"),
        ("analysis_proposal_id", "TEXT"),
        ("analysis_updated_at", "INTEGER"),
    ] {
        if !column_exists(conn, "workflow_episodes", column) {
            conn.execute(
                &format!("ALTER TABLE workflow_episodes ADD COLUMN {column} {definition}"),
                [],
            )?;
        }
    }
    conn.execute_batch(
        "INSERT OR IGNORE INTO migrations (version, applied_at, description)
         VALUES (28, datetime('now'), 'Audit workflow episode analysis outcomes');",
    )?;
    Ok(())
}

/// Version 29: Persist the compact operator lookup token for exact callbacks.
fn migrate_v29(conn: &Connection) -> Result<(), rusqlite::Error> {
    let tx = conn.unchecked_transaction()?;
    if !column_exists(&tx, "workflow_learning_proposals", "operator_token") {
        tx.execute(
            "ALTER TABLE workflow_learning_proposals
             ADD COLUMN operator_token TEXT
             CHECK(operator_token IS NULL OR (
                 length(operator_token) = 20
                 AND operator_token NOT GLOB '*[^0-9a-f]*'
             ))",
            [],
        )?;
    }
    tx.execute(
        "UPDATE workflow_learning_proposals
         SET operator_token = lower(substr(revision_sha256, 1, 20))
         WHERE revision_sha256 IS NOT NULL AND operator_token IS NULL",
        [],
    )?;
    tx.execute_batch(
        "CREATE UNIQUE INDEX IF NOT EXISTS idx_workflow_learning_proposals_operator_token
             ON workflow_learning_proposals(operator_token)
             WHERE operator_token IS NOT NULL;
         INSERT OR IGNORE INTO migrations (version, applied_at, description)
         VALUES (29, datetime('now'), 'Persist unique workflow proposal operator tokens');",
    )?;
    tx.commit()?;
    Ok(())
}

/// Version 30: Persist exact, expiring operator refinement bindings.
fn migrate_v30(conn: &Connection) -> Result<(), rusqlite::Error> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS workflow_learning_refinements (
             id TEXT PRIMARY KEY,
             idempotency_key TEXT NOT NULL UNIQUE,
             proposal_id TEXT NOT NULL,
             revision_sha256 TEXT NOT NULL,
             expected_proposal_version INTEGER NOT NULL,
             actor TEXT NOT NULL,
             surface TEXT NOT NULL,
             conversation_key TEXT NOT NULL,
             source_message_id TEXT,
             language TEXT NOT NULL,
             state TEXT NOT NULL DEFAULT 'awaiting_input',
             state_version INTEGER NOT NULL DEFAULT 0,
             instruction TEXT,
             captured_message_id TEXT,
             child_proposal_id TEXT,
             draft_job_id TEXT,
             last_error TEXT,
             expires_at INTEGER NOT NULL,
             created_at INTEGER NOT NULL,
             updated_at INTEGER NOT NULL,
             FOREIGN KEY (proposal_id) REFERENCES workflow_learning_proposals(id) ON DELETE RESTRICT,
             FOREIGN KEY (child_proposal_id) REFERENCES workflow_learning_proposals(id) ON DELETE RESTRICT,
             CHECK(state IN (
                 'awaiting_input', 'queued', 'completed', 'failed', 'cancelled', 'expired'
             )),
             CHECK(length(revision_sha256) = 64),
             CHECK(expected_proposal_version >= 0),
             CHECK(state_version >= 0)
         );
         CREATE UNIQUE INDEX IF NOT EXISTS idx_workflow_refinements_active_binding
             ON workflow_learning_refinements(surface, conversation_key, actor)
             WHERE state = 'awaiting_input';
         CREATE UNIQUE INDEX IF NOT EXISTS idx_workflow_refinements_active_revision
             ON workflow_learning_refinements(proposal_id, revision_sha256)
             WHERE state IN ('awaiting_input', 'queued');
         CREATE INDEX IF NOT EXISTS idx_workflow_refinements_due
             ON workflow_learning_refinements(state, expires_at, id);

         CREATE TABLE IF NOT EXISTS workflow_learning_refinement_events (
             sequence INTEGER PRIMARY KEY AUTOINCREMENT,
             idempotency_key TEXT NOT NULL UNIQUE,
             request_id TEXT NOT NULL,
             from_state TEXT,
             to_state TEXT NOT NULL,
             resulting_version INTEGER NOT NULL,
             actor TEXT NOT NULL,
             reason TEXT NOT NULL,
             created_at INTEGER NOT NULL,
             FOREIGN KEY (request_id) REFERENCES workflow_learning_refinements(id) ON DELETE RESTRICT
         );
         CREATE INDEX IF NOT EXISTS idx_workflow_refinement_events_request
             ON workflow_learning_refinement_events(request_id, sequence);

         INSERT OR IGNORE INTO migrations (version, applied_at, description)
         VALUES (30, datetime('now'), 'Persist workflow proposal refinement bindings');",
    )?;
    Ok(())
}

/// Version 31: Persist isolated tests independently from active installations.
fn migrate_v31(conn: &Connection) -> Result<(), rusqlite::Error> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS workflow_learning_tests (
             sequence INTEGER PRIMARY KEY AUTOINCREMENT,
             id TEXT NOT NULL UNIQUE,
             idempotency_key TEXT NOT NULL UNIQUE,
             proposal_id TEXT NOT NULL,
             revision_sha256 TEXT NOT NULL,
             job_id TEXT NOT NULL UNIQUE,
             status TEXT NOT NULL DEFAULT 'queued',
             requested_by TEXT NOT NULL,
             result_json TEXT,
             requested_at INTEGER NOT NULL,
             completed_at INTEGER,
             updated_at INTEGER NOT NULL,
             FOREIGN KEY (proposal_id) REFERENCES workflow_learning_proposals(id) ON DELETE RESTRICT,
             FOREIGN KEY (job_id) REFERENCES workflow_learning_jobs(id) ON DELETE RESTRICT,
             CHECK(status IN ('queued', 'passed', 'failed')),
             CHECK(length(revision_sha256) = 64),
             CHECK(
                 (status = 'queued' AND result_json IS NULL AND completed_at IS NULL)
                 OR
                 (status IN ('passed', 'failed') AND result_json IS NOT NULL AND completed_at IS NOT NULL)
             )
         );
         CREATE INDEX IF NOT EXISTS idx_workflow_learning_tests_revision
             ON workflow_learning_tests(
                 proposal_id, revision_sha256, requested_at DESC, sequence DESC
             );

         INSERT OR IGNORE INTO migrations (version, applied_at, description)
         VALUES (31, datetime('now'), 'Persist isolated workflow-learning test evidence');",
    )?;
    Ok(())
}

/// Version 32: Archive and retire the v3.13 SkillSynthesizer substrate.
///
/// Legacy rows do not contain immutable staging, validation evidence, or a
/// recoverable activation contract, so they must never be promoted into
/// Skill Learning V2 proposals. This migration preserves an exact audit copy,
/// retires pending work, and makes the source tables read-only. Dropping and
/// recreating the guards inside the transaction keeps the migration replayable
/// if the process stops after commit but before `user_version` is advanced.
fn migrate_v32(conn: &Connection) -> Result<(), rusqlite::Error> {
    let has_skill_patterns = table_exists(conn, "skill_patterns")?;
    let has_skill_proposals = table_exists(conn, "skill_proposals")?;
    let tx = conn.unchecked_transaction()?;
    tx.execute_batch(
        "DROP TRIGGER IF EXISTS guard_legacy_skill_patterns_insert;
         DROP TRIGGER IF EXISTS guard_legacy_skill_patterns_update;
         DROP TRIGGER IF EXISTS guard_legacy_skill_patterns_delete;
         DROP TRIGGER IF EXISTS guard_legacy_skill_proposals_insert;
         DROP TRIGGER IF EXISTS guard_legacy_skill_proposals_update;
         DROP TRIGGER IF EXISTS guard_legacy_skill_proposals_delete;

         CREATE TABLE IF NOT EXISTS legacy_skill_patterns_archive (
             hash TEXT PRIMARY KEY,
             agent_id TEXT NOT NULL,
             tool_sequence_json TEXT NOT NULL,
             first_seen INTEGER NOT NULL,
             last_seen INTEGER NOT NULL,
             count INTEGER NOT NULL,
             proposed_at INTEGER,
             archived_at INTEGER NOT NULL,
             archive_reason TEXT NOT NULL
         );

         CREATE TABLE IF NOT EXISTS legacy_skill_proposals_archive (
             id TEXT PRIMARY KEY,
             pattern_hash TEXT NOT NULL,
             name TEXT NOT NULL,
             description TEXT NOT NULL,
             trigger_hint TEXT NOT NULL,
             tool_sequence_json TEXT NOT NULL,
             arg_schema_hint TEXT NOT NULL,
             confidence REAL NOT NULL,
             family TEXT NOT NULL,
             source_agent_id TEXT NOT NULL,
             origin_channel TEXT,
             status TEXT,
             created_at INTEGER NOT NULL,
             decided_at INTEGER,
             decided_by TEXT,
             written_path TEXT,
             original_state TEXT NOT NULL,
             archived_at INTEGER NOT NULL,
             archive_reason TEXT NOT NULL
         );

         INSERT OR IGNORE INTO migrations (version, applied_at, description)
         VALUES (32, datetime('now'), 'Archive and retire the v3.13 SkillSynthesizer');",
    )?;

    if has_skill_patterns {
        tx.execute_batch(
            "INSERT OR IGNORE INTO legacy_skill_patterns_archive (
                 hash, agent_id, tool_sequence_json, first_seen, last_seen, count,
                 proposed_at, archived_at, archive_reason
             )
             SELECT hash, agent_id, tool_sequence_json, first_seen, last_seen, count,
                    proposed_at, CAST(strftime('%s', 'now') AS INTEGER) * 1000,
                    'v3.13 SkillSynthesizer retired in favor of durable workflow learning'
             FROM skill_patterns;

             UPDATE skill_patterns
             SET proposed_at = COALESCE(
                 proposed_at,
                 CAST(strftime('%s', 'now') AS INTEGER) * 1000
             );

             CREATE TRIGGER guard_legacy_skill_patterns_insert
             BEFORE INSERT ON skill_patterns BEGIN
                 SELECT RAISE(ABORT, 'legacy SkillSynthesizer is retired; use workflow learning');
             END;
             CREATE TRIGGER guard_legacy_skill_patterns_update
             BEFORE UPDATE ON skill_patterns BEGIN
                 SELECT RAISE(ABORT, 'legacy SkillSynthesizer is retired; use workflow learning');
             END;
             CREATE TRIGGER guard_legacy_skill_patterns_delete
             BEFORE DELETE ON skill_patterns BEGIN
                 SELECT RAISE(ABORT, 'legacy SkillSynthesizer is retired; use workflow learning');
             END;",
        )?;
    }

    if has_skill_proposals {
        tx.execute_batch(
            "INSERT OR IGNORE INTO legacy_skill_proposals_archive (
                 id, pattern_hash, name, description, trigger_hint,
                 tool_sequence_json, arg_schema_hint, confidence, family,
                 source_agent_id, origin_channel, status, created_at, decided_at,
                 decided_by, written_path, original_state, archived_at,
                 archive_reason
             )
             SELECT id, pattern_hash, name, description, trigger_hint,
                    tool_sequence_json, arg_schema_hint, confidence, family,
                    source_agent_id, origin_channel, status, created_at, decided_at,
                    decided_by, written_path,
                    CASE WHEN status IS NULL THEN 'pending' ELSE status END,
                    CAST(strftime('%s', 'now') AS INTEGER) * 1000,
                    'v3.13 SkillSynthesizer retired in favor of durable workflow learning'
             FROM skill_proposals;

             UPDATE skill_proposals
             SET status = 'denied',
                 decided_at = COALESCE(
                     decided_at,
                     CAST(strftime('%s', 'now') AS INTEGER) * 1000
                 ),
                 decided_by = COALESCE(decided_by, 'system:skill2-v32-retirement')
             WHERE status IS NULL;

             CREATE TRIGGER guard_legacy_skill_proposals_insert
             BEFORE INSERT ON skill_proposals BEGIN
                 SELECT RAISE(ABORT, 'legacy SkillSynthesizer is retired; use workflow learning');
             END;
             CREATE TRIGGER guard_legacy_skill_proposals_update
             BEFORE UPDATE ON skill_proposals BEGIN
                 SELECT RAISE(ABORT, 'legacy SkillSynthesizer is retired; use workflow learning');
             END;
             CREATE TRIGGER guard_legacy_skill_proposals_delete
             BEFORE DELETE ON skill_proposals BEGIN
                 SELECT RAISE(ABORT, 'legacy SkillSynthesizer is retired; use workflow learning');
             END;",
        )?;
    }
    tx.commit()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn reset_to_v31_fixture(conn: &Connection) {
        conn.execute_batch(
            "DROP TRIGGER IF EXISTS guard_legacy_skill_patterns_insert;
             DROP TRIGGER IF EXISTS guard_legacy_skill_patterns_update;
             DROP TRIGGER IF EXISTS guard_legacy_skill_patterns_delete;
             DROP TRIGGER IF EXISTS guard_legacy_skill_proposals_insert;
             DROP TRIGGER IF EXISTS guard_legacy_skill_proposals_update;
             DROP TRIGGER IF EXISTS guard_legacy_skill_proposals_delete;
             DROP TABLE IF EXISTS legacy_skill_patterns_archive;
             DROP TABLE IF EXISTS legacy_skill_proposals_archive;
             DELETE FROM migrations WHERE version = 32;
             PRAGMA user_version = 31;",
        )
        .unwrap();
    }

    fn seed_legacy_skill_synthesizer(conn: &Connection) {
        conn.execute_batch(
            "INSERT INTO skill_patterns (
                 hash, agent_id, tool_sequence_json, first_seen, last_seen,
                 count, proposed_at
             ) VALUES ('legacy-pattern', 'captain', '[\"shell_exec\",\"file_write\"]',
                       10, 20, 4, NULL);

             INSERT INTO skill_proposals (
                 id, pattern_hash, name, description, trigger_hint,
                 tool_sequence_json, arg_schema_hint, confidence, family,
                 source_agent_id, origin_channel, status, created_at,
                 decided_at, decided_by, written_path
             ) VALUES
                 ('legacy-pending', 'legacy-pattern', 'pending-skill', 'pending',
                  'when pending', '[\"shell_exec\"]', '{}', 0.8,
                  'general-automation', 'captain', 'telegram', NULL, 30,
                  NULL, NULL, NULL),
                 ('legacy-approved', 'legacy-pattern', 'approved-skill', 'approved',
                  'when approved', '[\"shell_exec\"]', '{}', 0.9,
                  'general-automation', 'captain', 'cli', 'approved', 31,
                  41, 'operator', '/legacy/skill.md'),
                 ('legacy-denied', 'legacy-pattern', 'denied-skill', 'denied',
                  'when denied', '[\"shell_exec\"]', '{}', 0.7,
                  'general-automation', 'captain', 'web', 'denied', 32,
                  42, 'operator', NULL);",
        )
        .unwrap();
    }

    #[test]
    fn test_migration_creates_tables() {
        let conn = Connection::open_in_memory().unwrap();
        run_migrations(&conn).unwrap();

        // Verify tables exist
        let tables: Vec<String> = conn
            .prepare("SELECT name FROM sqlite_master WHERE type='table' ORDER BY name")
            .unwrap()
            .query_map([], |row| row.get(0))
            .unwrap()
            .filter_map(|r| r.ok())
            .collect();

        assert!(tables.contains(&"agents".to_string()));
        assert!(tables.contains(&"sessions".to_string()));
        assert!(tables.contains(&"kv_store".to_string()));
        assert!(tables.contains(&"memories".to_string()));
        assert!(tables.contains(&"entities".to_string()));
        assert!(tables.contains(&"relations".to_string()));
        assert!(tables.contains(&"sessions_events".to_string()));
        assert!(tables.contains(&"projects".to_string()));
        assert!(tables.contains(&"project_tasks".to_string()));
        assert!(tables.contains(&"milestones".to_string()));
        assert!(tables.contains(&"project_checkpoints".to_string()));
        assert!(tables.contains(&"memory_writes".to_string()));
        assert!(tables.contains(&"learning_review_queue".to_string()));
        assert!(tables.contains(&"skill_patterns".to_string()));
        assert!(tables.contains(&"skill_proposals".to_string()));
        assert!(tables.contains(&"legacy_skill_patterns_archive".to_string()));
        assert!(tables.contains(&"legacy_skill_proposals_archive".to_string()));
        assert!(tables.contains(&"todos".to_string()));
        assert!(tables.contains(&"detached_tool_runs".to_string()));
        assert!(tables.contains(&"provider_quota_snapshots".to_string()));
        assert!(tables.contains(&"provider_quota_events".to_string()));
        assert!(tables.contains(&"workflow_episodes".to_string()));
        assert!(tables.contains(&"workflow_episode_steps".to_string()));
        assert!(tables.contains(&"workflow_learning_proposals".to_string()));
        assert!(tables.contains(&"workflow_learning_proposal_events".to_string()));
        assert!(tables.contains(&"workflow_learning_jobs".to_string()));
        assert!(tables.contains(&"workflow_learning_outbox".to_string()));
        assert!(tables.contains(&"workflow_learning_installations".to_string()));
        assert!(tables.contains(&"workflow_learning_installation_events".to_string()));
        assert!(tables.contains(&"workflow_learning_refinements".to_string()));
        assert!(tables.contains(&"workflow_learning_refinement_events".to_string()));
        assert!(tables.contains(&"workflow_learning_tests".to_string()));
        assert!(column_exists(
            &conn,
            "workflow_learning_installations",
            "phase_version"
        ));
        assert!(column_exists(
            &conn,
            "workflow_learning_installation_events",
            "last_error"
        ));
        assert!(column_exists(
            &conn,
            "workflow_episodes",
            "analysis_result_json"
        ));
        assert!(column_exists(
            &conn,
            "workflow_episodes",
            "analysis_proposal_id"
        ));
        assert!(column_exists(
            &conn,
            "workflow_episodes",
            "analysis_updated_at"
        ));
        assert!(column_exists(
            &conn,
            "workflow_learning_proposals",
            "operator_token"
        ));
        assert_eq!(get_schema_version(&conn), 32);
    }

    #[test]
    fn test_migration_idempotent() {
        let conn = Connection::open_in_memory().unwrap();
        run_migrations(&conn).unwrap();
        run_migrations(&conn).unwrap(); // Should not error
    }

    #[test]
    fn v23_upgrades_existing_memory_journal_without_losing_rows() {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE migrations (
                version INTEGER PRIMARY KEY,
                applied_at TEXT NOT NULL,
                description TEXT
            );
            CREATE TABLE memory_writes (
                id TEXT PRIMARY KEY,
                subject TEXT NOT NULL,
                predicate TEXT NOT NULL,
                object TEXT NOT NULL,
                wing TEXT,
                room TEXT,
                source TEXT NOT NULL,
                sync_status TEXT NOT NULL DEFAULT 'pending',
                sync_attempts INTEGER NOT NULL DEFAULT 0,
                created_at INTEGER NOT NULL,
                synced_at INTEGER,
                last_error TEXT
            );
            INSERT INTO memory_writes
                (id, subject, predicate, object, source, created_at)
                VALUES ('legacy', 'user', 'prefers', 'concise', 'test', 1);
            PRAGMA user_version = 22;",
        )
        .unwrap();

        run_migrations(&conn).unwrap();
        assert!(column_exists(&conn, "memory_writes", "operation"));
        assert!(column_exists(&conn, "memory_writes", "next_retry_at"));
        assert!(column_exists(&conn, "memory_writes", "retracted_at"));
        let (count, operation): (i64, String) = conn
            .query_row(
                "SELECT COUNT(*), operation FROM memory_writes WHERE id = 'legacy'",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        assert_eq!(count, 1);
        assert_eq!(operation, "add");
        assert_eq!(get_schema_version(&conn), 32);
    }

    #[test]
    fn v29_backfills_operator_tokens_for_published_proposals() {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE migrations (
                version INTEGER PRIMARY KEY,
                applied_at TEXT NOT NULL,
                description TEXT
            );
            CREATE TABLE workflow_learning_proposals (
                id TEXT PRIMARY KEY,
                revision_sha256 TEXT
            );
            INSERT INTO workflow_learning_proposals (id, revision_sha256)
            VALUES (
                'published',
                'AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA'
            );
            PRAGMA user_version = 28;",
        )
        .unwrap();

        run_migrations(&conn).unwrap();

        let token: String = conn
            .query_row(
                "SELECT operator_token FROM workflow_learning_proposals WHERE id = 'published'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(token, "aaaaaaaaaaaaaaaaaaaa");
        assert_eq!(get_schema_version(&conn), 32);
    }

    #[test]
    fn v30_adds_refinement_bindings_without_changing_existing_proposals() {
        let conn = Connection::open_in_memory().unwrap();
        run_migrations(&conn).unwrap();
        conn.execute(
            "INSERT INTO workflow_learning_proposals (
                 id, idempotency_key, workflow_signature, state, state_version,
                 source_agent_id, evidence_json, created_at, updated_at
             ) VALUES ('existing', 'existing:key', ?1, 'observed', 0,
                       'captain', '{}', 1, 1)",
            ["a".repeat(64)],
        )
        .unwrap();
        conn.pragma_update(None, "user_version", 29).unwrap();
        conn.execute("DELETE FROM migrations WHERE version = 30", [])
            .unwrap();

        run_migrations(&conn).unwrap();

        let count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM workflow_learning_proposals WHERE id = 'existing'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(count, 1);
        assert!(column_exists(
            &conn,
            "workflow_learning_refinements",
            "conversation_key"
        ));
        assert_eq!(get_schema_version(&conn), 32);
    }

    #[test]
    fn v31_adds_isolated_test_history_without_changing_existing_proposals() {
        let conn = Connection::open_in_memory().unwrap();
        run_migrations(&conn).unwrap();
        conn.execute(
            "INSERT INTO workflow_learning_proposals (
                 id, idempotency_key, workflow_signature, state, state_version,
                 source_agent_id, evidence_json, created_at, updated_at
             ) VALUES ('existing-v31', 'existing-v31:key', ?1, 'observed', 0,
                       'captain', '{}', 1, 1)",
            ["b".repeat(64)],
        )
        .unwrap();
        conn.pragma_update(None, "user_version", 30).unwrap();
        conn.execute("DELETE FROM migrations WHERE version = 31", [])
            .unwrap();

        run_migrations(&conn).unwrap();

        let count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM workflow_learning_proposals WHERE id = 'existing-v31'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(count, 1);
        assert!(column_exists(
            &conn,
            "workflow_learning_tests",
            "revision_sha256"
        ));
        assert_eq!(get_schema_version(&conn), 32);
    }

    #[test]
    fn v32_archives_every_legacy_state_without_fabricating_v2_work() {
        let conn = Connection::open_in_memory().unwrap();
        run_migrations(&conn).unwrap();
        reset_to_v31_fixture(&conn);
        seed_legacy_skill_synthesizer(&conn);

        run_migrations(&conn).unwrap();

        let pattern_count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM legacy_skill_patterns_archive",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(pattern_count, 1);

        let archived_states: Vec<String> = conn
            .prepare(
                "SELECT original_state FROM legacy_skill_proposals_archive
                 ORDER BY original_state",
            )
            .unwrap()
            .query_map([], |row| row.get(0))
            .unwrap()
            .collect::<Result<Vec<_>, _>>()
            .unwrap();
        assert_eq!(archived_states, vec!["approved", "denied", "pending"]);

        let (status, decided_by): (String, String) = conn
            .query_row(
                "SELECT status, decided_by FROM skill_proposals
                 WHERE id = 'legacy-pending'",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        assert_eq!(status, "denied");
        assert_eq!(decided_by, "system:skill2-v32-retirement");

        let v2_count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM workflow_learning_proposals",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(v2_count, 0);

        let error = conn
            .execute(
                "INSERT INTO skill_patterns (
                     hash, agent_id, tool_sequence_json, first_seen, last_seen, count
                 ) VALUES ('new-legacy', 'captain', '[]', 1, 1, 1)",
                [],
            )
            .unwrap_err();
        assert!(error.to_string().contains("SkillSynthesizer is retired"));
        assert_eq!(get_schema_version(&conn), 32);
    }

    #[test]
    fn v32_replays_after_reopen_without_duplicate_archive_rows() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("memory.sqlite");
        {
            let conn = Connection::open(&path).unwrap();
            run_migrations(&conn).unwrap();
            reset_to_v31_fixture(&conn);
            seed_legacy_skill_synthesizer(&conn);
        }
        {
            let conn = Connection::open(&path).unwrap();
            run_migrations(&conn).unwrap();
            conn.pragma_update(None, "user_version", 31).unwrap();
            conn.execute("DELETE FROM migrations WHERE version = 32", [])
                .unwrap();
        }
        {
            let conn = Connection::open(&path).unwrap();
            run_migrations(&conn).unwrap();
            let proposals: i64 = conn
                .query_row(
                    "SELECT COUNT(*) FROM legacy_skill_proposals_archive",
                    [],
                    |row| row.get(0),
                )
                .unwrap();
            let patterns: i64 = conn
                .query_row(
                    "SELECT COUNT(*) FROM legacy_skill_patterns_archive",
                    [],
                    |row| row.get(0),
                )
                .unwrap();
            assert_eq!((proposals, patterns), (3, 1));
            assert_eq!(get_schema_version(&conn), 32);
        }
    }
}
