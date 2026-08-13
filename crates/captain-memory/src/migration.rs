//! SQLite schema creation and migration.
//!
//! Creates all tables needed by the memory substrate on first boot.

use rusqlite::Connection;

/// Current schema version.
const SCHEMA_VERSION: u32 = 51;

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

    if current_version < 33 {
        migrate_v33(conn)?;
    }

    if current_version < 34 {
        migrate_v34(conn)?;
    }

    if current_version < 35 {
        migrate_v35(conn)?;
    }

    if current_version < 36 {
        migrate_v36(conn)?;
    }

    if current_version < 37 {
        migrate_v37(conn)?;
    }

    if current_version < 38 {
        migrate_v38(conn)?;
    }

    if current_version < 39 {
        migrate_v39(conn)?;
    }

    if current_version < 40 {
        migrate_v40(conn)?;
    }

    if current_version < 41 {
        migrate_v41(conn)?;
    }

    if current_version < 42 {
        migrate_v42(conn)?;
    }

    if current_version < 43 {
        migrate_v43(conn)?;
    }

    if current_version < 44 {
        migrate_v44(conn)?;
    }

    if current_version < 45 {
        migrate_v45(conn)?;
    }

    if current_version < 46 {
        migrate_v46(conn)?;
    }

    if current_version < 47 {
        migrate_v47(conn)?;
    }

    if current_version < 48 {
        migrate_v48(conn)?;
    }

    if current_version < 49 {
        migrate_v49(conn)?;
    }

    if current_version < 50 {
        migrate_v50(conn)?;
    }

    if current_version < 51 {
        migrate_v51(conn)?;
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

/// Version 8: Add audit_entries table for the persistent audit hash chain.
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
        VALUES (8, datetime('now'), 'Add audit_entries table for persistent audit hash chain');
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

/// Version 33: durable, dependency-aware sub-agent delegation jobs.
fn migrate_v33(conn: &Connection) -> Result<(), rusqlite::Error> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS agent_delegation_jobs (
             id TEXT PRIMARY KEY,
             idempotency_key TEXT NOT NULL UNIQUE,
             caller_agent_id TEXT NOT NULL,
             target_agent_id TEXT NOT NULL,
             title TEXT NOT NULL,
             task TEXT NOT NULL,
             max_tokens INTEGER NOT NULL,
             status TEXT NOT NULL,
             state_version INTEGER NOT NULL DEFAULT 0,
             attempt_count INTEGER NOT NULL DEFAULT 0,
             lease_owner TEXT,
             lease_expires_at INTEGER,
             effect_state TEXT NOT NULL DEFAULT 'not_started',
             result TEXT,
             result_truncated INTEGER NOT NULL DEFAULT 0,
             used_tokens INTEGER,
             error_code TEXT,
             error_message TEXT,
             cancel_requested_at INTEGER,
             started_at INTEGER,
             completed_at INTEGER,
             created_at INTEGER NOT NULL,
             updated_at INTEGER NOT NULL,
             CHECK(status IN (
                 'blocked', 'queued', 'running', 'cancel_requested',
                 'succeeded', 'failed', 'cancelled', 'uncertain',
                 'dependency_failed'
             )),
             CHECK(effect_state IN ('not_started', 'started', 'completed')),
             CHECK(state_version >= 0),
             CHECK(attempt_count >= 0 AND attempt_count <= 20),
             CHECK(max_tokens BETWEEN 1 AND 500000),
             CHECK(result_truncated IN (0, 1)),
             CHECK(used_tokens IS NULL OR used_tokens >= 0)
         );
         CREATE INDEX IF NOT EXISTS idx_agent_delegation_jobs_due
             ON agent_delegation_jobs(status, created_at, id);
         CREATE INDEX IF NOT EXISTS idx_agent_delegation_jobs_caller
             ON agent_delegation_jobs(caller_agent_id, updated_at DESC, id);
         CREATE INDEX IF NOT EXISTS idx_agent_delegation_jobs_target
             ON agent_delegation_jobs(target_agent_id, status, updated_at DESC);

         CREATE TABLE IF NOT EXISTS agent_delegation_dependencies (
             job_id TEXT NOT NULL,
             depends_on_job_id TEXT NOT NULL,
             created_at INTEGER NOT NULL,
             PRIMARY KEY (job_id, depends_on_job_id),
             FOREIGN KEY (job_id) REFERENCES agent_delegation_jobs(id) ON DELETE CASCADE,
             FOREIGN KEY (depends_on_job_id) REFERENCES agent_delegation_jobs(id) ON DELETE RESTRICT,
             CHECK(job_id <> depends_on_job_id)
         );
         CREATE INDEX IF NOT EXISTS idx_agent_delegation_dependencies_parent
             ON agent_delegation_dependencies(depends_on_job_id, job_id);

         INSERT OR IGNORE INTO migrations (version, applied_at, description)
         VALUES (33, datetime('now'), 'Add durable dependency-aware sub-agent delegation jobs');",
    )?;
    Ok(())
}

/// Version 34: durable operational heartbeat for Skill Learning V2.
fn migrate_v34(conn: &Connection) -> Result<(), rusqlite::Error> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS workflow_learning_runtime (
             singleton INTEGER PRIMARY KEY CHECK(singleton = 1),
             worker_id TEXT NOT NULL,
             phase TEXT NOT NULL,
             provider TEXT,
             model TEXT,
             started_at INTEGER NOT NULL,
             heartbeat_at INTEGER NOT NULL,
             last_scan_at INTEGER,
             last_progress_at INTEGER,
             last_error_scope TEXT,
             CHECK(phase IN ('starting', 'running', 'degraded')),
             CHECK((provider IS NULL) = (model IS NULL)),
             CHECK(started_at >= 0 AND heartbeat_at >= started_at),
             CHECK(last_scan_at IS NULL OR (last_scan_at >= 0 AND last_scan_at <= heartbeat_at)),
             CHECK(last_progress_at IS NULL OR (last_progress_at >= 0 AND last_progress_at <= heartbeat_at))
         );

         INSERT OR IGNORE INTO migrations (version, applied_at, description)
         VALUES (34, datetime('now'), 'Add durable workflow-learning runtime heartbeat');",
    )?;
    Ok(())
}

/// Version 35: active compaction registry for exact restart reconciliation.
fn migrate_v35(conn: &Connection) -> Result<(), rusqlite::Error> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS compaction_active_operations (
             operation_id TEXT PRIMARY KEY,
             runtime_instance_id TEXT NOT NULL,
             agent_id TEXT NOT NULL,
             session_id TEXT NOT NULL,
             payload TEXT NOT NULL,
             started_at INTEGER NOT NULL,
             updated_at INTEGER NOT NULL,
             CHECK(length(operation_id) BETWEEN 1 AND 128),
             CHECK(length(runtime_instance_id) BETWEEN 1 AND 128),
             CHECK(length(agent_id) BETWEEN 1 AND 128),
             CHECK(length(session_id) BETWEEN 1 AND 128),
             CHECK(length(payload) BETWEEN 2 AND 65536),
             CHECK(started_at >= 0 AND updated_at >= started_at)
         );
         CREATE INDEX IF NOT EXISTS idx_compaction_active_runtime
             ON compaction_active_operations(runtime_instance_id, started_at, operation_id);

         INSERT OR IGNORE INTO migrations (version, applied_at, description)
         VALUES (35, datetime('now'), 'Add active compaction restart reconciliation');",
    )?;
    Ok(())
}

/// Version 36: version audit hashes and add immutable recovery epochs.
fn migrate_v36(conn: &Connection) -> Result<(), rusqlite::Error> {
    if !column_exists(conn, "audit_entries", "epoch") {
        conn.execute(
            "ALTER TABLE audit_entries ADD COLUMN epoch INTEGER NOT NULL DEFAULT 0",
            [],
        )?;
    }
    if !column_exists(conn, "audit_entries", "hash_version") {
        conn.execute(
            "ALTER TABLE audit_entries ADD COLUMN hash_version INTEGER NOT NULL DEFAULT 1",
            [],
        )?;
    }

    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS audit_epochs (
             epoch INTEGER PRIMARY KEY,
             start_seq INTEGER NOT NULL,
             started_at TEXT NOT NULL,
             predecessor_tip_hash TEXT NOT NULL,
             status TEXT NOT NULL,
             terminal_hash TEXT,
             sealed_at TEXT,
             invalid_reason TEXT,
             CHECK(epoch >= 0),
             CHECK(start_seq >= 0),
             CHECK(status IN ('active', 'invalid')),
             CHECK(length(predecessor_tip_hash) = 64),
             CHECK(terminal_hash IS NULL OR length(terminal_hash) = 64),
             CHECK(
                 (status = 'active' AND terminal_hash IS NULL AND sealed_at IS NULL)
                 OR
                 (status = 'invalid' AND terminal_hash IS NOT NULL AND sealed_at IS NOT NULL)
             )
         );
         CREATE UNIQUE INDEX IF NOT EXISTS idx_audit_epochs_one_active
             ON audit_epochs(status) WHERE status = 'active';
         CREATE INDEX IF NOT EXISTS idx_audit_entries_epoch_seq
             ON audit_entries(epoch, seq);

         INSERT OR IGNORE INTO audit_epochs (
             epoch, start_seq, started_at, predecessor_tip_hash, status,
             terminal_hash, sealed_at, invalid_reason
         )
         SELECT
             0,
             COALESCE(MIN(seq), 0),
             COALESCE(MIN(timestamp), datetime('now')),
             '0000000000000000000000000000000000000000000000000000000000000000',
             'active',
             NULL,
             NULL,
             NULL
         FROM audit_entries;

         INSERT OR IGNORE INTO migrations (version, applied_at, description)
         VALUES (36, datetime('now'), 'Version audit hashes and add immutable recovery epochs');",
    )?;
    Ok(())
}

/// Version 37: durable parent lineage, nesting depth and reserved token budget
/// for detached sub-agent delegations.
fn migrate_v37(conn: &Connection) -> Result<(), rusqlite::Error> {
    if !column_exists(conn, "agent_delegation_jobs", "root_job_id") {
        conn.execute(
            "ALTER TABLE agent_delegation_jobs
             ADD COLUMN root_job_id TEXT NOT NULL DEFAULT ''",
            [],
        )?;
    }
    if !column_exists(conn, "agent_delegation_jobs", "parent_job_id") {
        conn.execute(
            "ALTER TABLE agent_delegation_jobs ADD COLUMN parent_job_id TEXT",
            [],
        )?;
    }
    if !column_exists(conn, "agent_delegation_jobs", "depth") {
        conn.execute(
            "ALTER TABLE agent_delegation_jobs
             ADD COLUMN depth INTEGER NOT NULL DEFAULT 1",
            [],
        )?;
    }

    conn.execute_batch(
        "UPDATE agent_delegation_jobs
         SET root_job_id = id, parent_job_id = NULL, depth = 1
         WHERE root_job_id = '';

         CREATE TABLE IF NOT EXISTS agent_delegation_lineages (
             root_job_id TEXT PRIMARY KEY,
             reserved_tokens INTEGER NOT NULL,
             created_at INTEGER NOT NULL,
             updated_at INTEGER NOT NULL,
             CHECK(length(root_job_id) BETWEEN 1 AND 96),
             CHECK(reserved_tokens BETWEEN 1 AND 500000),
             CHECK(updated_at >= created_at)
         );

         INSERT OR IGNORE INTO agent_delegation_lineages (
             root_job_id, reserved_tokens, created_at, updated_at
         )
         SELECT root_job_id, SUM(max_tokens), MIN(created_at), MAX(updated_at)
         FROM agent_delegation_jobs
         GROUP BY root_job_id;

         UPDATE agent_delegation_lineages
         SET reserved_tokens = MAX(
                 reserved_tokens,
                 COALESCE((
                     SELECT SUM(jobs.max_tokens)
                     FROM agent_delegation_jobs jobs
                     WHERE jobs.root_job_id =
                           agent_delegation_lineages.root_job_id
                 ), 0)
             ),
             updated_at = MAX(
                 updated_at,
                 COALESCE((
                     SELECT MAX(jobs.updated_at)
                     FROM agent_delegation_jobs jobs
                     WHERE jobs.root_job_id =
                           agent_delegation_lineages.root_job_id
                 ), updated_at)
             );

         CREATE INDEX IF NOT EXISTS idx_agent_delegation_jobs_root
             ON agent_delegation_jobs(root_job_id, created_at, id);
         CREATE INDEX IF NOT EXISTS idx_agent_delegation_jobs_parent
             ON agent_delegation_jobs(parent_job_id, created_at, id);
         CREATE INDEX IF NOT EXISTS idx_agent_delegation_jobs_depth
             ON agent_delegation_jobs(root_job_id, depth, created_at, id);

         DROP TRIGGER IF EXISTS guard_agent_delegation_lineage_insert;
         CREATE TRIGGER guard_agent_delegation_lineage_insert
         BEFORE INSERT ON agent_delegation_jobs
         WHEN NEW.root_job_id = ''
           OR NEW.depth NOT BETWEEN 1 AND 10
           OR (
               NEW.parent_job_id IS NULL
               AND (NEW.root_job_id <> NEW.id OR NEW.depth <> 1)
           )
           OR (
               NEW.parent_job_id IS NOT NULL
               AND (NEW.parent_job_id = '' OR NEW.parent_job_id = NEW.id OR NEW.depth <= 1)
           )
         BEGIN
             SELECT RAISE(ABORT, 'invalid agent delegation lineage');
         END;

         DROP TRIGGER IF EXISTS guard_agent_delegation_lineage_update;
         CREATE TRIGGER guard_agent_delegation_lineage_update
         BEFORE UPDATE OF id, root_job_id, parent_job_id, depth
         ON agent_delegation_jobs
         WHEN NEW.root_job_id = ''
           OR NEW.depth NOT BETWEEN 1 AND 10
           OR (
               NEW.parent_job_id IS NULL
               AND (NEW.root_job_id <> NEW.id OR NEW.depth <> 1)
           )
           OR (
               NEW.parent_job_id IS NOT NULL
               AND (NEW.parent_job_id = '' OR NEW.parent_job_id = NEW.id OR NEW.depth <= 1)
           )
         BEGIN
             SELECT RAISE(ABORT, 'invalid agent delegation lineage');
         END;

         INSERT OR IGNORE INTO migrations (version, applied_at, description)
         VALUES (
             37,
             datetime('now'),
             'Add durable lineage, depth and reserved delegation budgets'
         );",
    )?;
    Ok(())
}

/// Version 38: native multi-account Gmail registry. OAuth credentials and
/// tokens are referenced by opaque vault keys and never stored in SQLite.
fn migrate_v38(conn: &Connection) -> Result<(), rusqlite::Error> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS gmail_accounts (
             alias TEXT PRIMARY KEY,
             email_address TEXT NOT NULL,
             access_profile TEXT NOT NULL,
             granted_scopes_json TEXT NOT NULL,
             token_vault_key TEXT NOT NULL UNIQUE,
             client_vault_key TEXT NOT NULL,
             history_id TEXT,
             status TEXT NOT NULL DEFAULT 'ready',
             enabled INTEGER NOT NULL DEFAULT 1,
             is_default INTEGER NOT NULL DEFAULT 0,
             last_sync_at INTEGER,
             last_error_code TEXT,
             created_at INTEGER NOT NULL,
             updated_at INTEGER NOT NULL,
             CHECK(length(alias) BETWEEN 1 AND 48),
             CHECK(alias = lower(alias)),
             CHECK(alias NOT GLOB '*[^a-z0-9._-]*'),
             CHECK(substr(alias, 1, 1) GLOB '[a-z0-9]'),
             CHECK(length(email_address) BETWEEN 3 AND 320),
             CHECK(instr(email_address, char(10)) = 0),
             CHECK(instr(email_address, char(13)) = 0),
             CHECK(access_profile IN ('send', 'read', 'assistant')),
             CHECK(length(granted_scopes_json) BETWEEN 2 AND 4096),
             CHECK(length(token_vault_key) BETWEEN 1 AND 128),
             CHECK(length(client_vault_key) BETWEEN 1 AND 128),
             CHECK(token_vault_key NOT GLOB '*[^A-Z0-9_]*'),
             CHECK(client_vault_key NOT GLOB '*[^A-Z0-9_]*'),
             CHECK(token_vault_key <> client_vault_key),
             CHECK(history_id IS NULL OR length(history_id) BETWEEN 1 AND 128),
             CHECK(status IN ('ready', 'reauth_required', 'disabled')),
             CHECK(enabled IN (0, 1)),
             CHECK(is_default IN (0, 1)),
             CHECK(last_error_code IS NULL OR length(last_error_code) BETWEEN 1 AND 64),
             CHECK(created_at >= 0 AND updated_at >= created_at),
             CHECK(last_sync_at IS NULL OR last_sync_at >= created_at)
         );
         CREATE UNIQUE INDEX IF NOT EXISTS idx_gmail_accounts_email
             ON gmail_accounts(email_address COLLATE NOCASE);
         CREATE UNIQUE INDEX IF NOT EXISTS idx_gmail_accounts_one_default
             ON gmail_accounts(is_default) WHERE is_default = 1;
         CREATE INDEX IF NOT EXISTS idx_gmail_accounts_status
             ON gmail_accounts(enabled, status, alias);

         INSERT OR IGNORE INTO migrations (version, applied_at, description)
         VALUES (38, datetime('now'), 'Add native multi-account Gmail registry');",
    )?;
    Ok(())
}

/// Version 39: deterministic Gmail rules, immutable match audit, and a
/// crash-safe delivery outbox. A delivery interrupted after dispatch becomes
/// uncertain and cannot be replayed without an explicit operator decision.
fn migrate_v39(conn: &Connection) -> Result<(), rusqlite::Error> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS gmail_automation_rules (
             id TEXT PRIMARY KEY,
             account_alias TEXT NOT NULL,
             name TEXT NOT NULL,
             condition_json TEXT NOT NULL,
             action_json TEXT NOT NULL,
             enabled INTEGER NOT NULL DEFAULT 1,
             max_fires_per_hour INTEGER NOT NULL DEFAULT 30,
             state_version INTEGER NOT NULL DEFAULT 1,
             created_at INTEGER NOT NULL,
             updated_at INTEGER NOT NULL,
             CHECK(length(id) BETWEEN 1 AND 96),
             CHECK(id NOT GLOB '*[^A-Za-z0-9._:-]*'),
             CHECK(length(account_alias) BETWEEN 1 AND 48),
             CHECK(length(name) BETWEEN 1 AND 160),
             CHECK(length(condition_json) BETWEEN 2 AND 16384),
             CHECK(length(action_json) BETWEEN 2 AND 32768),
             CHECK(enabled IN (0, 1)),
             CHECK(max_fires_per_hour BETWEEN 1 AND 1000),
             CHECK(state_version >= 1),
             CHECK(created_at >= 0 AND updated_at >= created_at)
         );
         CREATE UNIQUE INDEX IF NOT EXISTS idx_gmail_automation_rule_name
             ON gmail_automation_rules(account_alias, name COLLATE NOCASE);
         CREATE INDEX IF NOT EXISTS idx_gmail_automation_rules_enabled
             ON gmail_automation_rules(enabled, account_alias, id);

         CREATE TABLE IF NOT EXISTS gmail_automation_events (
             id TEXT PRIMARY KEY,
             idempotency_key TEXT NOT NULL UNIQUE,
             rule_id TEXT NOT NULL REFERENCES gmail_automation_rules(id) ON DELETE RESTRICT,
             rule_version INTEGER NOT NULL,
             rule_snapshot_json TEXT NOT NULL,
             account_alias TEXT NOT NULL,
             message_id TEXT NOT NULL,
             history_id TEXT NOT NULL,
             metadata_json TEXT NOT NULL,
             decision TEXT NOT NULL,
             created_at INTEGER NOT NULL,
             CHECK(length(id) BETWEEN 1 AND 96),
             CHECK(length(idempotency_key) BETWEEN 1 AND 192),
             CHECK(rule_version >= 1),
             CHECK(length(rule_snapshot_json) BETWEEN 2 AND 65536),
             CHECK(length(message_id) BETWEEN 1 AND 256),
             CHECK(length(history_id) BETWEEN 1 AND 128),
             CHECK(history_id NOT GLOB '*[^0-9]*'),
             CHECK(length(metadata_json) BETWEEN 2 AND 65536),
             CHECK(decision IN ('queued', 'suppressed_rate_limit')),
             CHECK(created_at >= 0),
             UNIQUE(rule_id, account_alias, message_id)
         );
         CREATE INDEX IF NOT EXISTS idx_gmail_automation_events_rate
             ON gmail_automation_events(rule_id, decision, created_at);
         CREATE INDEX IF NOT EXISTS idx_gmail_automation_events_account
             ON gmail_automation_events(account_alias, created_at DESC);

         CREATE TABLE IF NOT EXISTS gmail_automation_outbox (
             id TEXT PRIMARY KEY,
             idempotency_key TEXT NOT NULL UNIQUE,
             event_id TEXT NOT NULL UNIQUE
                 REFERENCES gmail_automation_events(id) ON DELETE RESTRICT,
             target_agent_id TEXT NOT NULL,
             payload_json TEXT NOT NULL,
             status TEXT NOT NULL DEFAULT 'pending',
             attempt_count INTEGER NOT NULL DEFAULT 0,
             max_attempts INTEGER NOT NULL DEFAULT 3,
             run_after INTEGER NOT NULL,
             lease_owner TEXT,
             lease_expires_at INTEGER,
             delivery_result_json TEXT,
             last_error TEXT,
             delivered_at INTEGER,
             created_at INTEGER NOT NULL,
             updated_at INTEGER NOT NULL,
             CHECK(length(id) BETWEEN 1 AND 96),
             CHECK(length(idempotency_key) BETWEEN 1 AND 192),
             CHECK(length(target_agent_id) = 36),
             CHECK(length(payload_json) BETWEEN 2 AND 98304),
             CHECK(status IN (
                 'pending', 'delivering', 'retry_wait', 'delivered', 'dead', 'uncertain'
             )),
             CHECK(attempt_count >= 0 AND attempt_count <= max_attempts),
             CHECK(max_attempts BETWEEN 1 AND 10),
             CHECK(run_after >= 0),
             CHECK((lease_owner IS NULL) = (lease_expires_at IS NULL)),
             CHECK(status = 'delivering' OR lease_owner IS NULL),
             CHECK(lease_owner IS NULL OR length(lease_owner) BETWEEN 1 AND 96),
             CHECK(delivery_result_json IS NULL OR length(delivery_result_json) <= 32768),
             CHECK(last_error IS NULL OR length(last_error) <= 2048),
             CHECK(delivered_at IS NULL OR delivered_at >= created_at),
             CHECK(created_at >= 0 AND updated_at >= created_at)
         );
         CREATE INDEX IF NOT EXISTS idx_gmail_automation_outbox_due
             ON gmail_automation_outbox(status, run_after, created_at);
         CREATE INDEX IF NOT EXISTS idx_gmail_automation_outbox_lease
             ON gmail_automation_outbox(status, lease_expires_at);

         INSERT OR IGNORE INTO migrations (version, applied_at, description)
         VALUES (
             39,
             datetime('now'),
             'Add deterministic Gmail rules and crash-safe delivery outbox'
         );",
    )?;
    Ok(())
}

/// Version 40: resumable page checkpoint for incremental and recovery Gmail
/// synchronization. OAuth token refreshes no longer serve as the sync cursor.
fn migrate_v40(conn: &Connection) -> Result<(), rusqlite::Error> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS gmail_sync_checkpoints (
             account_alias TEXT PRIMARY KEY,
             mode TEXT NOT NULL,
             start_history_id TEXT NOT NULL,
             target_history_id TEXT NOT NULL,
             page_token TEXT,
             pages_processed INTEGER NOT NULL DEFAULT 0,
             messages_processed INTEGER NOT NULL DEFAULT 0,
             last_error_code TEXT,
             started_at INTEGER NOT NULL,
             updated_at INTEGER NOT NULL,
             CHECK(mode IN ('incremental', 'recovery')),
             CHECK(length(start_history_id) BETWEEN 1 AND 128),
             CHECK(start_history_id NOT GLOB '*[^0-9]*'),
             CHECK(length(target_history_id) BETWEEN 1 AND 128),
             CHECK(target_history_id NOT GLOB '*[^0-9]*'),
             CHECK(page_token IS NULL OR length(page_token) BETWEEN 1 AND 2048),
             CHECK(pages_processed >= 0),
             CHECK(messages_processed >= 0),
             CHECK(last_error_code IS NULL OR length(last_error_code) BETWEEN 1 AND 64),
             CHECK(started_at >= 0 AND updated_at >= started_at)
         );
         CREATE INDEX IF NOT EXISTS idx_gmail_sync_checkpoints_updated
             ON gmail_sync_checkpoints(updated_at, account_alias);

         INSERT OR IGNORE INTO migrations (version, applied_at, description)
         VALUES (40, datetime('now'), 'Add resumable Gmail synchronization checkpoints');",
    )?;
    Ok(())
}

/// Version 41: crash-safe outbox for provider-confirmed quota-reset cards.
fn migrate_v41(conn: &Connection) -> Result<(), rusqlite::Error> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS provider_quota_reset_outbox (
             id TEXT PRIMARY KEY,
             provider TEXT NOT NULL,
             limit_id TEXT NOT NULL,
             payload_json TEXT NOT NULL,
             status TEXT NOT NULL DEFAULT 'pending',
             attempt_count INTEGER NOT NULL DEFAULT 0,
             max_attempts INTEGER NOT NULL DEFAULT 24,
             run_after INTEGER NOT NULL,
             lease_owner TEXT,
             lease_expires_at INTEGER,
             external_message_id TEXT,
             last_error TEXT,
             delivered_at INTEGER,
             created_at INTEGER NOT NULL,
             updated_at INTEGER NOT NULL,
             CHECK(length(id) = 36),
             CHECK(length(payload_json) BETWEEN 2 AND 65536),
             CHECK(status IN (
                 'pending', 'delivering', 'retry_wait', 'delivered',
                 'suppressed', 'dead', 'uncertain'
             )),
             CHECK(attempt_count >= 0 AND attempt_count <= max_attempts),
             CHECK(max_attempts BETWEEN 1 AND 100),
             CHECK(run_after >= 0),
             CHECK((lease_owner IS NULL) = (lease_expires_at IS NULL)),
             CHECK(status = 'delivering' OR lease_owner IS NULL),
             CHECK(lease_owner IS NULL OR length(lease_owner) BETWEEN 1 AND 96),
             CHECK(external_message_id IS NULL OR length(external_message_id) <= 256),
             CHECK(last_error IS NULL OR length(last_error) <= 2048),
             CHECK(delivered_at IS NULL OR delivered_at >= created_at),
             CHECK(created_at >= 0 AND updated_at >= created_at)
         );
         CREATE INDEX IF NOT EXISTS idx_provider_quota_reset_outbox_due
             ON provider_quota_reset_outbox(status, run_after, created_at);
         CREATE INDEX IF NOT EXISTS idx_provider_quota_reset_outbox_lease
             ON provider_quota_reset_outbox(status, lease_expires_at);

         INSERT OR IGNORE INTO migrations (version, applied_at, description)
         VALUES (41, datetime('now'), 'Add crash-safe provider quota reset outbox');",
    )?;
    Ok(())
}

/// Version 42: promote the historical detached-run table into the durable
/// metadata ledger for every tool run and attach owner-only output evidence.
/// The table name remains unchanged to keep existing installations and backup
/// tooling compatible.
fn migrate_v42(conn: &Connection) -> Result<(), rusqlite::Error> {
    if !column_exists(conn, "detached_tool_runs", "detached") {
        conn.execute_batch(
            "ALTER TABLE detached_tool_runs
                 ADD COLUMN detached INTEGER NOT NULL DEFAULT 1;",
        )?;
    }
    if !column_exists(conn, "detached_tool_runs", "input_sha256") {
        conn.execute_batch("ALTER TABLE detached_tool_runs ADD COLUMN input_sha256 TEXT;")?;
    }
    if !column_exists(conn, "detached_tool_runs", "output_file_name") {
        conn.execute_batch("ALTER TABLE detached_tool_runs ADD COLUMN output_file_name TEXT;")?;
    }
    if !column_exists(conn, "detached_tool_runs", "output_stored_bytes") {
        conn.execute_batch(
            "ALTER TABLE detached_tool_runs ADD COLUMN output_stored_bytes INTEGER;",
        )?;
    }
    if !column_exists(conn, "detached_tool_runs", "output_total_bytes") {
        conn.execute_batch(
            "ALTER TABLE detached_tool_runs ADD COLUMN output_total_bytes INTEGER;",
        )?;
    }
    if !column_exists(conn, "detached_tool_runs", "output_sha256") {
        conn.execute_batch("ALTER TABLE detached_tool_runs ADD COLUMN output_sha256 TEXT;")?;
    }
    if !column_exists(conn, "detached_tool_runs", "output_capped") {
        conn.execute_batch(
            "ALTER TABLE detached_tool_runs
                 ADD COLUMN output_capped INTEGER NOT NULL DEFAULT 0;",
        )?;
    }
    if !column_exists(conn, "detached_tool_runs", "output_redacted") {
        conn.execute_batch(
            "ALTER TABLE detached_tool_runs
                 ADD COLUMN output_redacted INTEGER NOT NULL DEFAULT 0;",
        )?;
    }
    conn.execute(
        "INSERT OR IGNORE INTO migrations (version, applied_at, description)
         VALUES (42, datetime('now'), 'Promote tool runs and add durable output evidence')",
        [],
    )?;
    Ok(())
}

/// Version 43: preserve explicit retry lineage without persisting raw tool
/// input. `retry_of_run_id` points to the immediately preceding run; the input
/// digest remains the authority for exact-retry validation.
fn migrate_v43(conn: &Connection) -> Result<(), rusqlite::Error> {
    if !column_exists(conn, "detached_tool_runs", "retry_of_run_id") {
        conn.execute_batch("ALTER TABLE detached_tool_runs ADD COLUMN retry_of_run_id TEXT;")?;
    }
    if !column_exists(conn, "detached_tool_runs", "retry_attempt") {
        conn.execute_batch(
            "ALTER TABLE detached_tool_runs
                 ADD COLUMN retry_attempt INTEGER NOT NULL DEFAULT 0;",
        )?;
    }
    conn.execute_batch(
        "CREATE INDEX IF NOT EXISTS idx_detached_tool_runs_retry_of
             ON detached_tool_runs(retry_of_run_id);
         INSERT OR IGNORE INTO migrations (version, applied_at, description)
         VALUES (43, datetime('now'), 'Add explicit tool run retry lineage');",
    )?;
    Ok(())
}

/// Version 44: active work-verification registry for exact restart
/// reconciliation. Full history remains in the append-only session event log;
/// this table contains only operations that have no terminal event yet.
fn migrate_v44(conn: &Connection) -> Result<(), rusqlite::Error> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS work_verification_active_operations (
             operation_id TEXT PRIMARY KEY,
             runtime_instance_id TEXT NOT NULL,
             agent_id TEXT NOT NULL,
             session_id TEXT NOT NULL,
             payload TEXT NOT NULL,
             started_at INTEGER NOT NULL,
             updated_at INTEGER NOT NULL,
             CHECK(length(operation_id) BETWEEN 1 AND 128),
             CHECK(length(runtime_instance_id) BETWEEN 1 AND 128),
             CHECK(length(agent_id) BETWEEN 1 AND 128),
             CHECK(length(session_id) BETWEEN 1 AND 128),
             CHECK(length(payload) BETWEEN 2 AND 262144)
         );
         CREATE INDEX IF NOT EXISTS idx_work_verification_active_runtime
             ON work_verification_active_operations(
                 runtime_instance_id, started_at, operation_id
             );
         CREATE INDEX IF NOT EXISTS idx_work_verification_active_session
             ON work_verification_active_operations(
                 session_id, started_at, operation_id
             );
         INSERT OR IGNORE INTO migrations (version, applied_at, description)
         VALUES (44, datetime('now'), 'Add active work verification registry');",
    )?;
    Ok(())
}

/// Version 45: authoritative device registry and crash-safe pairing claims.
/// Legacy mobile pairing rows remain intact and are imported as
/// `reauth_required`; no historical plaintext push token is copied.
fn migrate_v45(conn: &Connection) -> Result<(), rusqlite::Error> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS captain_devices (
             device_id TEXT PRIMARY KEY,
             display_name TEXT NOT NULL,
             role TEXT NOT NULL,
             platform TEXT NOT NULL,
             captain_version TEXT NOT NULL,
             protocol_major INTEGER NOT NULL,
             protocol_minor INTEGER NOT NULL,
             credential_sha256 TEXT UNIQUE,
             capabilities_json TEXT NOT NULL,
             grants_json TEXT NOT NULL,
             status TEXT NOT NULL,
             paired_at_ms INTEGER NOT NULL,
             last_seen_ms INTEGER NOT NULL,
             updated_at_ms INTEGER NOT NULL,
             last_transport TEXT,
             last_error_code TEXT,
             revoked_at_ms INTEGER,
             CHECK(length(device_id) BETWEEN 1 AND 128),
             CHECK(device_id NOT GLOB '*[^A-Za-z0-9._:-]*'),
             CHECK(length(display_name) BETWEEN 1 AND 160),
             CHECK(instr(display_name, char(10)) = 0),
             CHECK(instr(display_name, char(13)) = 0),
             CHECK(role IN ('client', 'node')),
             CHECK(length(platform) BETWEEN 1 AND 128),
             CHECK(platform NOT GLOB '*[^A-Za-z0-9._:-]*'),
             CHECK(length(captain_version) BETWEEN 1 AND 160),
             CHECK(protocol_major BETWEEN 0 AND 65535),
             CHECK(protocol_minor BETWEEN 0 AND 65535),
             CHECK(
                 credential_sha256 IS NULL OR (
                     length(credential_sha256) = 64
                     AND credential_sha256 NOT GLOB '*[^0-9a-f]*'
                 )
             ),
             CHECK(length(capabilities_json) BETWEEN 2 AND 262144),
             CHECK(json_valid(capabilities_json)),
             CHECK(length(grants_json) BETWEEN 2 AND 65536),
             CHECK(json_valid(grants_json)),
             CHECK(status IN ('active', 'reauth_required', 'revoked')),
             CHECK(paired_at_ms >= 0),
             CHECK(last_seen_ms >= paired_at_ms),
             CHECK(updated_at_ms >= paired_at_ms),
             CHECK(last_transport IS NULL OR last_transport IN (
                 'web_socket', 'http_stream', 'long_poll', 'local'
             )),
             CHECK(last_error_code IS NULL OR length(last_error_code) BETWEEN 1 AND 96),
             CHECK(revoked_at_ms IS NULL OR revoked_at_ms >= paired_at_ms),
             CHECK((status = 'revoked') = (revoked_at_ms IS NOT NULL)),
             CHECK(status <> 'active' OR credential_sha256 IS NOT NULL)
         );
         CREATE INDEX IF NOT EXISTS idx_captain_devices_status
             ON captain_devices(status, role, display_name);
         CREATE INDEX IF NOT EXISTS idx_captain_devices_seen
             ON captain_devices(last_seen_ms DESC, device_id);

         CREATE TABLE IF NOT EXISTS device_pairing_requests (
             request_id TEXT PRIMARY KEY,
             display_code_sha256 TEXT NOT NULL UNIQUE,
             polling_secret_sha256 TEXT NOT NULL UNIQUE,
             credential_sha256 TEXT NOT NULL UNIQUE,
             display_name TEXT NOT NULL,
             role TEXT NOT NULL,
             platform TEXT NOT NULL,
             captain_version TEXT NOT NULL,
             protocol_major INTEGER NOT NULL,
             protocol_minor INTEGER NOT NULL,
             capabilities_json TEXT NOT NULL,
             requested_grants_json TEXT NOT NULL,
             status TEXT NOT NULL DEFAULT 'pending',
             created_at_ms INTEGER NOT NULL,
             expires_at_ms INTEGER NOT NULL,
             decided_at_ms INTEGER,
             approved_device_id TEXT REFERENCES captain_devices(device_id) ON DELETE RESTRICT,
             CHECK(length(request_id) = 36),
             CHECK(request_id NOT GLOB '*[^0-9a-f-]*'),
             CHECK(length(display_code_sha256) = 64),
             CHECK(display_code_sha256 NOT GLOB '*[^0-9a-f]*'),
             CHECK(length(polling_secret_sha256) = 64),
             CHECK(polling_secret_sha256 NOT GLOB '*[^0-9a-f]*'),
             CHECK(length(credential_sha256) = 64),
             CHECK(credential_sha256 NOT GLOB '*[^0-9a-f]*'),
             CHECK(length(display_name) BETWEEN 1 AND 160),
             CHECK(instr(display_name, char(10)) = 0),
             CHECK(instr(display_name, char(13)) = 0),
             CHECK(role IN ('client', 'node')),
             CHECK(length(platform) BETWEEN 1 AND 128),
             CHECK(platform NOT GLOB '*[^A-Za-z0-9._:-]*'),
             CHECK(length(captain_version) BETWEEN 1 AND 160),
             CHECK(protocol_major BETWEEN 1 AND 65535),
             CHECK(protocol_minor BETWEEN 0 AND 65535),
             CHECK(length(capabilities_json) BETWEEN 2 AND 262144),
             CHECK(json_valid(capabilities_json)),
             CHECK(length(requested_grants_json) BETWEEN 2 AND 65536),
             CHECK(json_valid(requested_grants_json)),
             CHECK(status IN ('pending', 'approved', 'denied', 'expired')),
             CHECK(created_at_ms >= 0),
             CHECK(expires_at_ms > created_at_ms),
             CHECK(decided_at_ms IS NULL OR decided_at_ms >= created_at_ms),
             CHECK((status = 'pending') = (decided_at_ms IS NULL)),
             CHECK((status = 'approved') = (approved_device_id IS NOT NULL))
         );
         CREATE INDEX IF NOT EXISTS idx_device_pairing_pending
             ON device_pairing_requests(status, expires_at_ms, created_at_ms);
         ",
    )?;

    if table_exists(conn, "paired_devices")? {
        conn.execute_batch(
            "INSERT OR IGNORE INTO captain_devices (
             device_id, display_name, role, platform, captain_version,
             protocol_major, protocol_minor, credential_sha256,
             capabilities_json, grants_json, status, paired_at_ms,
             last_seen_ms, updated_at_ms, last_transport,
             last_error_code, revoked_at_ms
         )
         SELECT
             device_id, display_name, 'client', platform, 'legacy',
             0, 0, NULL, '{}', '{}', 'reauth_required',
             CAST(COALESCE(strftime('%s', paired_at), '0') AS INTEGER) * 1000,
             MAX(
                 CAST(COALESCE(strftime('%s', paired_at), '0') AS INTEGER) * 1000,
                 CAST(COALESCE(strftime('%s', last_seen), '0') AS INTEGER) * 1000
             ),
             MAX(
                 CAST(COALESCE(strftime('%s', paired_at), '0') AS INTEGER) * 1000,
                 CAST(COALESCE(strftime('%s', last_seen), '0') AS INTEGER) * 1000
             ),
             NULL, 'legacy_pairing_requires_reauth', NULL
         FROM paired_devices
         WHERE length(device_id) BETWEEN 1 AND 128
           AND device_id NOT GLOB '*[^A-Za-z0-9._:-]*'
           AND length(display_name) BETWEEN 1 AND 160
           AND instr(display_name, char(10)) = 0
           AND instr(display_name, char(13)) = 0
           AND length(platform) BETWEEN 1 AND 128
           AND platform NOT GLOB '*[^A-Za-z0-9._:-]*';",
        )?;
    }

    conn.execute(
        "INSERT OR IGNORE INTO migrations (version, applied_at, description)
         VALUES (45, datetime('now'), 'Add authoritative devices and durable pairing claims')",
        [],
    )?;
    Ok(())
}

/// Version 46: durable Hub-to-Node execution rail. Messages are persisted as
/// validated JSON payloads before delivery; cursors make both directions
/// replayable without re-running an acknowledged side effect.
fn migrate_v46(conn: &Connection) -> Result<(), rusqlite::Error> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS hub_node_runs (
             run_id TEXT PRIMARY KEY,
             device_id TEXT NOT NULL REFERENCES captain_devices(device_id) ON DELETE RESTRICT,
             idempotency_key TEXT NOT NULL,
             workspace_id TEXT NOT NULL,
             tool_name TEXT NOT NULL,
             input_json TEXT NOT NULL,
             effect TEXT NOT NULL,
             status TEXT NOT NULL DEFAULT 'queued',
             attempt INTEGER NOT NULL DEFAULT 0,
             lease_owner TEXT,
             lease_expires_at_ms INTEGER,
             effect_state TEXT NOT NULL DEFAULT 'not_started',
             progress_sequence INTEGER NOT NULL DEFAULT 0,
             progress_message TEXT,
             completion_json TEXT,
             completion_sha256 TEXT,
             error_code TEXT,
             cancel_requested_at_ms INTEGER,
             created_at_ms INTEGER NOT NULL,
             updated_at_ms INTEGER NOT NULL,
             terminal_at_ms INTEGER,
             UNIQUE(device_id, idempotency_key),
             CHECK(length(run_id) BETWEEN 1 AND 128),
             CHECK(run_id NOT GLOB '*[^A-Za-z0-9._:-]*'),
             CHECK(length(device_id) BETWEEN 1 AND 128),
             CHECK(length(idempotency_key) BETWEEN 1 AND 128),
             CHECK(idempotency_key NOT GLOB '*[^A-Za-z0-9._:-]*'),
             CHECK(length(workspace_id) BETWEEN 1 AND 128),
             CHECK(workspace_id NOT GLOB '*[^A-Za-z0-9._:-]*'),
             CHECK(length(tool_name) BETWEEN 1 AND 128),
             CHECK(tool_name NOT GLOB '*[^A-Za-z0-9._:-]*'),
             CHECK(length(input_json) BETWEEN 2 AND 1048576),
             CHECK(json_valid(input_json)),
             CHECK(effect IN ('read_only', 'local_mutation', 'external_effect')),
             CHECK(status IN (
                 'queued', 'leased', 'accepted', 'cancel_requested',
                 'succeeded', 'failed', 'cancelled', 'uncertain'
             )),
             CHECK(attempt BETWEEN 0 AND 4294967295),
             CHECK(effect_state IN ('not_started', 'started', 'completed')),
             CHECK(progress_sequence >= 0),
             CHECK(progress_message IS NULL OR length(progress_message) <= 4096),
             CHECK(completion_json IS NULL OR (
                 length(completion_json) BETWEEN 2 AND 8388608
                 AND json_valid(completion_json)
             )),
             CHECK(completion_sha256 IS NULL OR (
                 length(completion_sha256) = 64
                 AND completion_sha256 NOT GLOB '*[^0-9a-f]*'
             )),
             CHECK(error_code IS NULL OR length(error_code) BETWEEN 1 AND 96),
             CHECK(created_at_ms >= 0),
             CHECK(updated_at_ms >= created_at_ms),
             CHECK(terminal_at_ms IS NULL OR terminal_at_ms >= created_at_ms),
             CHECK(
                 (status IN ('leased', 'accepted', 'cancel_requested')) =
                 (lease_owner IS NOT NULL AND lease_expires_at_ms IS NOT NULL)
             ),
             CHECK(
                 (status IN ('succeeded', 'failed', 'cancelled', 'uncertain')) =
                 (terminal_at_ms IS NOT NULL)
             )
         );
         CREATE INDEX IF NOT EXISTS idx_hub_node_runs_dispatch
             ON hub_node_runs(device_id, status, created_at_ms, run_id);
         CREATE INDEX IF NOT EXISTS idx_hub_node_runs_lease
             ON hub_node_runs(status, lease_expires_at_ms);

         CREATE TABLE IF NOT EXISTS hub_node_cursors (
             device_id TEXT PRIMARY KEY REFERENCES captain_devices(device_id) ON DELETE RESTRICT,
             next_hub_sequence INTEGER NOT NULL DEFAULT 1,
             last_node_sequence INTEGER NOT NULL DEFAULT 0,
             last_hub_ack_sequence INTEGER NOT NULL DEFAULT 0,
             updated_at_ms INTEGER NOT NULL,
             CHECK(next_hub_sequence >= 1),
             CHECK(last_node_sequence >= 0),
             CHECK(last_hub_ack_sequence >= 0),
             CHECK(last_hub_ack_sequence < next_hub_sequence),
             CHECK(updated_at_ms >= 0)
         );

         CREATE TABLE IF NOT EXISTS hub_node_outbox (
             device_id TEXT NOT NULL REFERENCES captain_devices(device_id) ON DELETE RESTRICT,
             sequence INTEGER NOT NULL,
             message_kind TEXT NOT NULL,
             message_json TEXT NOT NULL,
             message_sha256 TEXT NOT NULL,
             run_id TEXT,
             created_at_ms INTEGER NOT NULL,
             acked_at_ms INTEGER,
             PRIMARY KEY(device_id, sequence),
             CHECK(sequence >= 1),
             CHECK(length(message_kind) BETWEEN 1 AND 64),
             CHECK(message_kind NOT GLOB '*[^a-z0-9_]*'),
             CHECK(length(message_json) BETWEEN 2 AND 8388608),
             CHECK(json_valid(message_json)),
             CHECK(length(message_sha256) = 64),
             CHECK(message_sha256 NOT GLOB '*[^0-9a-f]*'),
             CHECK(created_at_ms >= 0),
             CHECK(acked_at_ms IS NULL OR acked_at_ms >= created_at_ms)
         );
         CREATE INDEX IF NOT EXISTS idx_hub_node_outbox_pending
             ON hub_node_outbox(device_id, acked_at_ms, sequence);

         CREATE TABLE IF NOT EXISTS hub_node_inbox (
             device_id TEXT NOT NULL REFERENCES captain_devices(device_id) ON DELETE RESTRICT,
             sequence INTEGER NOT NULL,
             connection_id TEXT NOT NULL,
             message_kind TEXT NOT NULL,
             message_sha256 TEXT NOT NULL,
             received_at_ms INTEGER NOT NULL,
             PRIMARY KEY(device_id, sequence),
             CHECK(sequence >= 1),
             CHECK(length(connection_id) BETWEEN 1 AND 128),
             CHECK(connection_id NOT GLOB '*[^A-Za-z0-9._:-]*'),
             CHECK(length(message_kind) BETWEEN 1 AND 64),
             CHECK(message_kind NOT GLOB '*[^a-z0-9_]*'),
             CHECK(length(message_sha256) = 64),
             CHECK(message_sha256 NOT GLOB '*[^0-9a-f]*'),
             CHECK(received_at_ms >= 0)
         );

         INSERT OR IGNORE INTO migrations (version, applied_at, description)
         VALUES (46, datetime('now'), 'Add durable Hub Node execution rail');",
    )?;
    Ok(())
}

/// Version 47: one durable presence row per paired Node. A process restart
/// marks active rows offline before any new transport is accepted.
fn migrate_v47(conn: &Connection) -> Result<(), rusqlite::Error> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS hub_node_connections (
             device_id TEXT PRIMARY KEY REFERENCES captain_devices(device_id) ON DELETE RESTRICT,
             connection_id TEXT NOT NULL UNIQUE,
             transport TEXT NOT NULL,
             protocol_major INTEGER NOT NULL,
             protocol_minor INTEGER NOT NULL,
             status TEXT NOT NULL,
             connected_at_ms INTEGER NOT NULL,
             last_seen_ms INTEGER NOT NULL,
             updated_at_ms INTEGER NOT NULL,
             disconnected_at_ms INTEGER,
             last_error_code TEXT,
             CHECK(length(device_id) BETWEEN 1 AND 128),
             CHECK(length(connection_id) BETWEEN 1 AND 128),
             CHECK(connection_id NOT GLOB '*[^A-Za-z0-9._:-]*'),
             CHECK(transport IN ('web_socket', 'http_stream', 'long_poll')),
             CHECK(protocol_major BETWEEN 1 AND 65535),
             CHECK(protocol_minor BETWEEN 0 AND 65535),
             CHECK(status IN ('active', 'offline')),
             CHECK(connected_at_ms >= 0),
             CHECK(last_seen_ms >= connected_at_ms),
             CHECK(updated_at_ms >= connected_at_ms),
             CHECK(disconnected_at_ms IS NULL OR disconnected_at_ms >= connected_at_ms),
             CHECK((status = 'active') = (disconnected_at_ms IS NULL)),
             CHECK(last_error_code IS NULL OR length(last_error_code) BETWEEN 1 AND 96)
         );
         CREATE INDEX IF NOT EXISTS idx_hub_node_connections_presence
             ON hub_node_connections(status, last_seen_ms DESC, device_id);
         INSERT OR IGNORE INTO migrations (version, applied_at, description)
         VALUES (47, datetime('now'), 'Add durable Hub Node connection presence');",
    )?;
    Ok(())
}

/// Version 48: distinguish a real Node acknowledgement from an outbox row
/// superseded and resequenced for a newer authenticated connection.
fn migrate_v48(conn: &Connection) -> Result<(), rusqlite::Error> {
    if !column_exists(conn, "hub_node_outbox", "superseded_at_ms") {
        conn.execute(
            "ALTER TABLE hub_node_outbox ADD COLUMN superseded_at_ms INTEGER",
            [],
        )?;
    }
    conn.execute_batch(
        "CREATE INDEX IF NOT EXISTS idx_hub_node_outbox_delivery
             ON hub_node_outbox(device_id, acked_at_ms, superseded_at_ms, sequence);
         INSERT OR IGNORE INTO migrations (version, applied_at, description)
         VALUES (48, datetime('now'), 'Track resequenced Hub Node outbox rows');",
    )?;
    Ok(())
}

/// Version 49: correlate local Node policy rejection and exact-action
/// approvals with one leased run without persisting raw tool input twice.
fn migrate_v49(conn: &Connection) -> Result<(), rusqlite::Error> {
    if !column_exists(conn, "hub_node_runs", "rejection_json") {
        conn.execute(
            "ALTER TABLE hub_node_runs ADD COLUMN rejection_json TEXT",
            [],
        )?;
    }
    if !column_exists(conn, "hub_node_runs", "rejection_sha256") {
        conn.execute(
            "ALTER TABLE hub_node_runs ADD COLUMN rejection_sha256 TEXT",
            [],
        )?;
    }
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS hub_node_run_approvals (
             approval_id TEXT PRIMARY KEY,
             run_id TEXT NOT NULL REFERENCES hub_node_runs(run_id) ON DELETE RESTRICT,
             attempt INTEGER NOT NULL,
             action_digest TEXT NOT NULL,
             action_summary TEXT NOT NULL,
             risk_level TEXT NOT NULL,
             status TEXT NOT NULL DEFAULT 'pending',
             decision TEXT,
             reason TEXT,
             requested_at_ms INTEGER NOT NULL,
             expires_at_ms INTEGER NOT NULL,
             decided_at_ms INTEGER,
             UNIQUE(run_id, attempt),
             CHECK(length(approval_id) BETWEEN 1 AND 128),
             CHECK(approval_id NOT GLOB '*[^A-Za-z0-9._:-]*'),
             CHECK(attempt BETWEEN 1 AND 4294967295),
             CHECK(length(action_digest) = 64),
             CHECK(action_digest NOT GLOB '*[^0-9a-f]*'),
             CHECK(length(action_summary) BETWEEN 1 AND 512),
             CHECK(risk_level IN ('low', 'medium', 'high', 'critical')),
             CHECK(status IN ('pending', 'approved', 'denied', 'timed_out')),
             CHECK(decision IS NULL OR decision IN (
                 'approved', 'approved_session', 'approved_always',
                 'denied', 'denied_session', 'denied_always', 'timed_out'
             )),
             CHECK(reason IS NULL OR length(reason) BETWEEN 1 AND 280),
             CHECK(requested_at_ms >= 0),
             CHECK(expires_at_ms > requested_at_ms),
             CHECK(decided_at_ms IS NULL OR decided_at_ms >= requested_at_ms),
             CHECK(
                 (status = 'pending' AND decision IS NULL AND decided_at_ms IS NULL)
                 OR
                 (status <> 'pending' AND decision IS NOT NULL AND decided_at_ms IS NOT NULL)
             )
         );
         CREATE INDEX IF NOT EXISTS idx_hub_node_run_approvals_pending
             ON hub_node_run_approvals(status, expires_at_ms, run_id);
         INSERT OR IGNORE INTO migrations (version, applied_at, description)
         VALUES (49, datetime('now'), 'Add exact Hub Node run approval and rejection evidence');",
    )?;
    Ok(())
}

/// Version 50: persist only short-lived access-token digests so paired
/// Clients and Nodes remain authenticated across a Hub restart without ever
/// writing a bearer token itself to disk.
fn migrate_v50(conn: &Connection) -> Result<(), rusqlite::Error> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS device_access_tokens (
             token_sha256 TEXT PRIMARY KEY,
             device_id TEXT NOT NULL REFERENCES captain_devices(device_id) ON DELETE CASCADE,
             issued_at_ms INTEGER NOT NULL,
             expires_at_ms INTEGER NOT NULL,
             CHECK(length(token_sha256) = 64),
             CHECK(token_sha256 NOT GLOB '*[^0-9a-f]*'),
             CHECK(issued_at_ms >= 0),
             CHECK(expires_at_ms > issued_at_ms)
         );
         CREATE INDEX IF NOT EXISTS idx_device_access_tokens_device
             ON device_access_tokens(device_id, issued_at_ms, token_sha256);
         CREATE INDEX IF NOT EXISTS idx_device_access_tokens_expiry
             ON device_access_tokens(expires_at_ms, token_sha256);
         INSERT OR IGNORE INTO migrations (version, applied_at, description)
         VALUES (50, datetime('now'), 'Persist short-lived device access token digests');",
    )?;
    Ok(())
}

/// Version 51: persist the logical execution target pinned to a session or
/// project. Node filesystem paths are intentionally absent.
fn migrate_v51(conn: &Connection) -> Result<(), rusqlite::Error> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS execution_target_bindings (
             scope_kind TEXT NOT NULL,
             scope_id TEXT NOT NULL,
             target_kind TEXT NOT NULL,
             device_id TEXT,
             workspace_id TEXT,
             updated_at_ms INTEGER NOT NULL,
             PRIMARY KEY(scope_kind, scope_id),
             CHECK(scope_kind IN ('session', 'project')),
             CHECK(length(scope_id) BETWEEN 1 AND 128),
             CHECK(scope_id NOT GLOB '*[^A-Za-z0-9._:-]*'),
             CHECK(target_kind IN ('auto', 'hub', 'node')),
             CHECK(updated_at_ms >= 0),
             CHECK(
                 (target_kind IN ('auto', 'hub')
                     AND device_id IS NULL AND workspace_id IS NULL)
                 OR
                 (target_kind = 'node'
                     AND device_id IS NOT NULL
                     AND workspace_id IS NOT NULL
                     AND length(device_id) = 41
                     AND device_id LIKE 'node-%'
                     AND length(workspace_id) BETWEEN 1 AND 128
                     AND workspace_id NOT GLOB '*[^A-Za-z0-9._:-]*')
             )
         );
         CREATE INDEX IF NOT EXISTS idx_execution_target_bindings_node
             ON execution_target_bindings(device_id, workspace_id, scope_kind, scope_id)
             WHERE target_kind = 'node';
         CREATE TRIGGER IF NOT EXISTS cleanup_session_execution_target
             AFTER DELETE ON sessions
             BEGIN
                 DELETE FROM execution_target_bindings
                 WHERE scope_kind = 'session' AND scope_id = OLD.id;
             END;
         CREATE TRIGGER IF NOT EXISTS cleanup_project_execution_target
             AFTER DELETE ON projects
             BEGIN
                 DELETE FROM execution_target_bindings
                 WHERE scope_kind = 'project' AND scope_id = OLD.id;
             END;
         INSERT OR IGNORE INTO migrations (version, applied_at, description)
         VALUES (51, datetime('now'), 'Persist logical session and project execution targets');",
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn create_legacy_audit_table(conn: &Connection) {
        conn.execute_batch(
            "CREATE TABLE audit_entries (
                 seq INTEGER PRIMARY KEY,
                 timestamp TEXT NOT NULL,
                 agent_id TEXT NOT NULL,
                 action TEXT NOT NULL,
                 detail TEXT NOT NULL,
                 outcome TEXT NOT NULL,
                 prev_hash TEXT NOT NULL,
                 hash TEXT NOT NULL
             );",
        )
        .unwrap();
    }

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
        assert!(tables.contains(&"work_verification_active_operations".to_string()));
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
        assert!(tables.contains(&"captain_devices".to_string()));
        assert!(tables.contains(&"device_pairing_requests".to_string()));
        assert!(tables.contains(&"device_access_tokens".to_string()));
        assert!(tables.contains(&"hub_node_runs".to_string()));
        assert!(tables.contains(&"hub_node_cursors".to_string()));
        assert!(tables.contains(&"hub_node_outbox".to_string()));
        assert!(tables.contains(&"hub_node_inbox".to_string()));
        assert!(tables.contains(&"hub_node_connections".to_string()));
        assert!(tables.contains(&"provider_quota_snapshots".to_string()));
        assert!(tables.contains(&"provider_quota_events".to_string()));
        assert!(tables.contains(&"provider_quota_reset_outbox".to_string()));
        assert!(tables.contains(&"gmail_accounts".to_string()));
        assert!(tables.contains(&"gmail_automation_rules".to_string()));
        assert!(tables.contains(&"gmail_automation_events".to_string()));
        assert!(tables.contains(&"gmail_automation_outbox".to_string()));
        assert!(tables.contains(&"gmail_sync_checkpoints".to_string()));
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
        assert!(tables.contains(&"workflow_learning_runtime".to_string()));
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
        assert!(tables.contains(&"audit_epochs".to_string()));
        assert!(column_exists(&conn, "audit_entries", "epoch"));
        assert!(column_exists(&conn, "audit_entries", "hash_version"));
        assert_eq!(get_schema_version(&conn), SCHEMA_VERSION);
    }

    #[test]
    fn audit_epoch_migration_preserves_legacy_rows_without_rehashing() {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE migrations (
                 version INTEGER PRIMARY KEY,
                 applied_at TEXT NOT NULL,
                 description TEXT NOT NULL
             );
             CREATE TABLE audit_entries (
                 seq INTEGER PRIMARY KEY,
                 timestamp TEXT NOT NULL,
                 agent_id TEXT NOT NULL,
                 action TEXT NOT NULL,
                 detail TEXT NOT NULL,
                 outcome TEXT NOT NULL,
                 prev_hash TEXT NOT NULL,
                 hash TEXT NOT NULL
             );
             INSERT INTO audit_entries (
                 seq, timestamp, agent_id, action, detail, outcome, prev_hash, hash
             ) VALUES (
                 0, '2026-07-29T00:00:00Z', 'captain', 'ToolInvoke',
                 'legacy', 'ok',
                 '0000000000000000000000000000000000000000000000000000000000000000',
                 'aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa'
             );
             PRAGMA user_version = 35;",
        )
        .unwrap();

        // This fixture intentionally contains only the v35 audit surface.
        // Exercise v36 directly so later unrelated migrations do not require
        // fabricating tables that a real v35 database would already contain.
        migrate_v36(&conn).unwrap();

        let row: (i64, i64, String) = conn
            .query_row(
                "SELECT epoch, hash_version, hash FROM audit_entries WHERE seq = 0",
                [],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .unwrap();
        assert_eq!(row.0, 0);
        assert_eq!(row.1, 1);
        assert_eq!(
            row.2,
            "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
        );
        let epoch: (i64, i64, String) = conn
            .query_row(
                "SELECT epoch, start_seq, status FROM audit_epochs",
                [],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .unwrap();
        assert_eq!(epoch, (0, 0, "active".to_string()));
    }

    #[test]
    fn audit_epoch_migration_refuses_a_missing_history_table() {
        let conn = Connection::open_in_memory().unwrap();
        conn.pragma_update(None, "user_version", 35).unwrap();

        let error = run_migrations(&conn).unwrap_err();

        assert!(error.to_string().contains("no such table: audit_entries"));
        assert_eq!(get_schema_version(&conn), 35);
    }

    #[test]
    fn test_migration_idempotent() {
        let conn = Connection::open_in_memory().unwrap();
        run_migrations(&conn).unwrap();
        run_migrations(&conn).unwrap(); // Should not error
    }

    #[test]
    fn v39_replays_cleanly_from_a_v38_database() {
        let conn = Connection::open_in_memory().unwrap();
        run_migrations(&conn).unwrap();
        conn.execute_batch(
            "DROP TABLE gmail_automation_outbox;
             DROP TABLE gmail_automation_events;
             DROP TABLE gmail_automation_rules;
             DELETE FROM migrations WHERE version = 39;
             PRAGMA user_version = 38;",
        )
        .unwrap();

        run_migrations(&conn).unwrap();

        assert!(table_exists(&conn, "gmail_automation_rules").unwrap());
        assert!(table_exists(&conn, "gmail_automation_events").unwrap());
        assert!(table_exists(&conn, "gmail_automation_outbox").unwrap());
        assert_eq!(get_schema_version(&conn), SCHEMA_VERSION);
        let migration_count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM migrations WHERE version = 39",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(migration_count, 1);
    }

    #[test]
    fn v40_replays_cleanly_from_a_v39_database() {
        let conn = Connection::open_in_memory().unwrap();
        run_migrations(&conn).unwrap();
        conn.execute_batch(
            "DROP TABLE gmail_sync_checkpoints;
             DELETE FROM migrations WHERE version = 40;
             PRAGMA user_version = 39;",
        )
        .unwrap();

        run_migrations(&conn).unwrap();

        assert!(table_exists(&conn, "gmail_sync_checkpoints").unwrap());
        assert_eq!(get_schema_version(&conn), SCHEMA_VERSION);
        let migration_count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM migrations WHERE version = 40",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(migration_count, 1);
    }

    #[test]
    fn v41_replays_cleanly_from_a_v40_database() {
        let conn = Connection::open_in_memory().unwrap();
        run_migrations(&conn).unwrap();
        conn.execute_batch(
            "DROP TABLE provider_quota_reset_outbox;
             DELETE FROM migrations WHERE version = 41;
             PRAGMA user_version = 40;",
        )
        .unwrap();

        run_migrations(&conn).unwrap();

        assert!(table_exists(&conn, "provider_quota_reset_outbox").unwrap());
        assert_eq!(get_schema_version(&conn), SCHEMA_VERSION);
        let migration_count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM migrations WHERE version = 41",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(migration_count, 1);
    }

    #[test]
    fn v42_promotes_existing_detached_runs_without_losing_rows() {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE migrations (
                 version INTEGER PRIMARY KEY,
                 applied_at TEXT NOT NULL,
                 description TEXT
             );
             CREATE TABLE detached_tool_runs (
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
             INSERT INTO detached_tool_runs
                 (run_id, tool_name, status, started_at, result_truncated)
             VALUES ('toolrun-existing', 'shell_exec', 'completed', 10, 0);
             PRAGMA user_version = 41;",
        )
        .unwrap();

        run_migrations(&conn).unwrap();

        let row: (String, i64, Option<String>) = conn
            .query_row(
                "SELECT run_id, detached, output_file_name
                 FROM detached_tool_runs WHERE run_id = 'toolrun-existing'",
                [],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .unwrap();
        assert_eq!(row, ("toolrun-existing".to_string(), 1, None));
        assert_eq!(get_schema_version(&conn), SCHEMA_VERSION);
    }

    #[test]
    fn v43_adds_retry_lineage_without_rewriting_existing_runs() {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE migrations (
                 version INTEGER PRIMARY KEY,
                 applied_at TEXT NOT NULL,
                 description TEXT
             );
             CREATE TABLE detached_tool_runs (
                 run_id TEXT PRIMARY KEY,
                 tool_name TEXT NOT NULL,
                 status TEXT NOT NULL,
                 caller_agent_id TEXT,
                 origin_tool_use_id TEXT,
                 started_at INTEGER NOT NULL,
                 finished_at INTEGER,
                 is_error INTEGER,
                 result TEXT,
                 result_truncated INTEGER NOT NULL DEFAULT 0,
                 detached INTEGER NOT NULL DEFAULT 1,
                 input_sha256 TEXT,
                 output_file_name TEXT,
                 output_stored_bytes INTEGER,
                 output_total_bytes INTEGER,
                 output_sha256 TEXT,
                 output_capped INTEGER NOT NULL DEFAULT 0,
                 output_redacted INTEGER NOT NULL DEFAULT 0
             );
             INSERT INTO detached_tool_runs
                 (run_id, tool_name, status, started_at, result_truncated, detached)
             VALUES ('toolrun-before-retry', 'shell_exec', 'failed', 10, 0, 1);
             PRAGMA user_version = 42;",
        )
        .unwrap();

        run_migrations(&conn).unwrap();

        let row: (String, Option<String>, i64) = conn
            .query_row(
                "SELECT run_id, retry_of_run_id, retry_attempt FROM detached_tool_runs
                 WHERE run_id = 'toolrun-before-retry'",
                [],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .unwrap();
        assert_eq!(row, ("toolrun-before-retry".to_string(), None, 0));
        assert_eq!(get_schema_version(&conn), SCHEMA_VERSION);
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
        create_legacy_audit_table(&conn);
        migrate_v22(&conn).unwrap();

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
        assert_eq!(get_schema_version(&conn), SCHEMA_VERSION);
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
        create_legacy_audit_table(&conn);
        migrate_v22(&conn).unwrap();

        run_migrations(&conn).unwrap();

        let token: String = conn
            .query_row(
                "SELECT operator_token FROM workflow_learning_proposals WHERE id = 'published'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(token, "aaaaaaaaaaaaaaaaaaaa");
        assert_eq!(get_schema_version(&conn), SCHEMA_VERSION);
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
        assert_eq!(get_schema_version(&conn), SCHEMA_VERSION);
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
        assert_eq!(get_schema_version(&conn), SCHEMA_VERSION);
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
        assert_eq!(get_schema_version(&conn), SCHEMA_VERSION);
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
            assert_eq!(get_schema_version(&conn), SCHEMA_VERSION);
        }
    }

    #[test]
    fn v33_adds_durable_delegation_jobs_and_replays_without_data_loss() {
        let conn = Connection::open_in_memory().unwrap();
        run_migrations(&conn).unwrap();

        conn.execute(
            "INSERT INTO agent_delegation_jobs (
                 id, idempotency_key, caller_agent_id, target_agent_id,
                 title, task, max_tokens, status, created_at, updated_at,
                 root_job_id, depth
             ) VALUES ('job-v33', 'idem:job-v33', 'caller', 'worker',
                       'proof', 'produce evidence', 5000, 'queued', 1, 1,
                       'job-v33', 1)",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO agent_delegation_dependencies
                 (job_id, depends_on_job_id, created_at)
             VALUES ('job-v33', 'job-v33', 1)",
            [],
        )
        .unwrap_err();

        conn.pragma_update(None, "user_version", 32).unwrap();
        conn.execute("DELETE FROM migrations WHERE version = 33", [])
            .unwrap();
        run_migrations(&conn).unwrap();

        let row: (String, String, i64) = conn
            .query_row(
                "SELECT status, effect_state, max_tokens
                 FROM agent_delegation_jobs WHERE id = 'job-v33'",
                [],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .unwrap();
        assert_eq!(row, ("queued".to_string(), "not_started".to_string(), 5000));
        assert!(table_exists(&conn, "agent_delegation_dependencies").unwrap());
        assert_eq!(get_schema_version(&conn), SCHEMA_VERSION);
    }

    #[test]
    fn v34_adds_and_replays_workflow_learning_runtime_without_data_loss() {
        let conn = Connection::open_in_memory().unwrap();
        run_migrations(&conn).unwrap();
        conn.execute(
            "INSERT INTO workflow_learning_runtime (
                 singleton, worker_id, phase, provider, model,
                 started_at, heartbeat_at
             ) VALUES (1, 'worker-v34', 'running', 'codex', 'gpt-5.6-sol', 1, 2)",
            [],
        )
        .unwrap();

        conn.pragma_update(None, "user_version", 33).unwrap();
        conn.execute("DELETE FROM migrations WHERE version = 34", [])
            .unwrap();
        run_migrations(&conn).unwrap();

        assert!(table_exists(&conn, "workflow_learning_runtime").unwrap());
        let row: (String, String, String, i64) = conn
            .query_row(
                "SELECT worker_id, provider, model, heartbeat_at
                 FROM workflow_learning_runtime WHERE singleton = 1",
                [],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
            )
            .unwrap();
        assert_eq!(
            row,
            (
                "worker-v34".to_string(),
                "codex".to_string(),
                "gpt-5.6-sol".to_string(),
                2,
            )
        );
        assert_eq!(get_schema_version(&conn), SCHEMA_VERSION);
    }

    #[test]
    fn v35_adds_and_replays_active_compactions_without_data_loss() {
        let conn = Connection::open_in_memory().unwrap();
        run_migrations(&conn).unwrap();
        conn.execute(
            "INSERT INTO compaction_active_operations (
                 operation_id, runtime_instance_id, agent_id, session_id,
                 payload, started_at, updated_at
             ) VALUES ('operation-v35', 'runtime-v35', 'agent-v35',
                       'session-v35', '{}', 1, 2)",
            [],
        )
        .unwrap();

        conn.pragma_update(None, "user_version", 34).unwrap();
        conn.execute("DELETE FROM migrations WHERE version = 35", [])
            .unwrap();
        run_migrations(&conn).unwrap();

        let row: (String, String, i64) = conn
            .query_row(
                "SELECT runtime_instance_id, session_id, updated_at
                 FROM compaction_active_operations WHERE operation_id = 'operation-v35'",
                [],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .unwrap();
        assert_eq!(
            row,
            ("runtime-v35".to_string(), "session-v35".to_string(), 2)
        );
        assert_eq!(get_schema_version(&conn), SCHEMA_VERSION);
    }

    #[test]
    fn v37_backfills_delegation_lineage_and_replays_without_double_reservation() {
        let conn = Connection::open_in_memory().unwrap();
        run_migrations(&conn).unwrap();
        conn.execute(
            "INSERT INTO agent_delegation_lineages (
                 root_job_id, reserved_tokens, created_at, updated_at
             ) VALUES ('job-v37', 5000, 1, 1)",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO agent_delegation_jobs (
                 id, idempotency_key, caller_agent_id, target_agent_id,
                 title, task, max_tokens, status, created_at, updated_at,
                 root_job_id, depth
             ) VALUES ('job-v37', 'idem:job-v37', 'caller', 'worker',
                       'proof', 'produce evidence', 5000, 'queued', 1, 1,
                       'job-v37', 1)",
            [],
        )
        .unwrap();

        conn.execute_batch(
            "DROP TRIGGER guard_agent_delegation_lineage_insert;
             DROP TRIGGER guard_agent_delegation_lineage_update;
             DROP TABLE agent_delegation_lineages;
             UPDATE agent_delegation_jobs
             SET root_job_id = '', parent_job_id = NULL, depth = 1
             WHERE id = 'job-v37';
             DELETE FROM migrations WHERE version = 37;
             PRAGMA user_version = 36;",
        )
        .unwrap();
        run_migrations(&conn).unwrap();

        let row: (String, Option<String>, i64, i64) = conn
            .query_row(
                "SELECT jobs.root_job_id, jobs.parent_job_id, jobs.depth,
                        lineages.reserved_tokens
                 FROM agent_delegation_jobs jobs
                 JOIN agent_delegation_lineages lineages
                   ON lineages.root_job_id = jobs.root_job_id
                 WHERE jobs.id = 'job-v37'",
                [],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
            )
            .unwrap();
        assert_eq!(row, ("job-v37".to_string(), None, 1, 5000));

        conn.pragma_update(None, "user_version", 36).unwrap();
        conn.execute("DELETE FROM migrations WHERE version = 37", [])
            .unwrap();
        run_migrations(&conn).unwrap();
        let reserved: i64 = conn
            .query_row(
                "SELECT reserved_tokens FROM agent_delegation_lineages
                 WHERE root_job_id = 'job-v37'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(reserved, 5000);
        assert_eq!(get_schema_version(&conn), SCHEMA_VERSION);
    }

    #[test]
    fn v38_adds_gmail_registry_and_replays_without_losing_accounts() {
        let conn = Connection::open_in_memory().unwrap();
        run_migrations(&conn).unwrap();
        conn.execute(
            "INSERT INTO gmail_accounts (
                 alias, email_address, access_profile, granted_scopes_json,
                 token_vault_key, client_vault_key, history_id, status,
                 enabled, is_default, created_at, updated_at
             ) VALUES (
                 'personal', 'person@gmail.com', 'assistant',
                 '[\"https://www.googleapis.com/auth/gmail.modify\"]',
                 'CAPTAIN_GMAIL_PERSONAL_TOKEN',
                 'CAPTAIN_GMAIL_PERSONAL_CLIENT', '12345', 'ready',
                 1, 1, 1, 1
             )",
            [],
        )
        .unwrap();

        conn.pragma_update(None, "user_version", 37).unwrap();
        conn.execute("DELETE FROM migrations WHERE version = 38", [])
            .unwrap();
        run_migrations(&conn).unwrap();

        let row: (String, String, String, i64) = conn
            .query_row(
                "SELECT email_address, access_profile, token_vault_key, is_default
                 FROM gmail_accounts WHERE alias = 'personal'",
                [],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
            )
            .unwrap();
        assert_eq!(
            row,
            (
                "person@gmail.com".to_string(),
                "assistant".to_string(),
                "CAPTAIN_GMAIL_PERSONAL_TOKEN".to_string(),
                1,
            )
        );
        assert_eq!(get_schema_version(&conn), SCHEMA_VERSION);

        assert!(conn
            .execute(
                "INSERT INTO gmail_accounts (
                     alias, email_address, access_profile, granted_scopes_json,
                     token_vault_key, client_vault_key, status,
                     enabled, is_default, created_at, updated_at
                 ) VALUES (
                     'bad', 'bad@gmail.com', 'send', '[]',
                     'lowercase_token', 'CAPTAIN_GMAIL_BAD_CLIENT', 'ready',
                     1, 0, 1, 1
                 )",
                [],
            )
            .is_err());
    }

    #[test]
    fn v45_imports_legacy_devices_without_push_tokens_and_replays_safely() {
        let conn = Connection::open_in_memory().unwrap();
        run_migrations(&conn).unwrap();
        conn.execute(
            "INSERT INTO paired_devices (
                 device_id, display_name, platform, paired_at, last_seen, push_token
             ) VALUES (
                 'legacy-phone', 'Legacy Phone', 'ios',
                 '2026-08-01T10:00:00Z', '2026-08-02T12:00:00Z',
                 'push-token-must-not-migrate'
             )",
            [],
        )
        .unwrap();
        conn.execute_batch(
            "DROP TABLE device_pairing_requests;
             DROP TABLE captain_devices;
             DELETE FROM migrations WHERE version = 45;
             PRAGMA user_version = 44;",
        )
        .unwrap();

        run_migrations(&conn).unwrap();
        let imported: (String, String, Option<String>, String, i64, i64) = conn
            .query_row(
                "SELECT role, status, credential_sha256, last_error_code,
                        paired_at_ms, last_seen_ms
                 FROM captain_devices WHERE device_id = 'legacy-phone'",
                [],
                |row| {
                    Ok((
                        row.get(0)?,
                        row.get(1)?,
                        row.get(2)?,
                        row.get(3)?,
                        row.get(4)?,
                        row.get(5)?,
                    ))
                },
            )
            .unwrap();
        assert_eq!(imported.0, "client");
        assert_eq!(imported.1, "reauth_required");
        assert_eq!(imported.2, None);
        assert_eq!(imported.3, "legacy_pairing_requires_reauth");
        assert!(imported.4 > 0);
        assert!(imported.5 >= imported.4);

        let columns: Vec<String> = conn
            .prepare("PRAGMA table_info(captain_devices)")
            .unwrap()
            .query_map([], |row| row.get(1))
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap();
        assert!(!columns.iter().any(|column| column == "push_token"));

        conn.execute("DELETE FROM migrations WHERE version = 45", [])
            .unwrap();
        conn.pragma_update(None, "user_version", 44).unwrap();
        run_migrations(&conn).unwrap();
        let count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM captain_devices WHERE device_id = 'legacy-phone'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(count, 1);
        assert_eq!(get_schema_version(&conn), SCHEMA_VERSION);
    }

    #[test]
    fn v48_adds_resequence_audit_without_rewriting_outbox_rows() {
        let conn = Connection::open_in_memory().unwrap();
        run_migrations(&conn).unwrap();
        conn.execute(
            "INSERT INTO captain_devices (
                 device_id, display_name, role, platform, captain_version,
                 protocol_major, protocol_minor, credential_sha256,
                 capabilities_json, grants_json, status, paired_at_ms,
                 last_seen_ms, updated_at_ms
             ) VALUES ('node-v48', 'Node', 'node', 'linux', 'alpha.14',
                       1, 0, ?1, '{}', '{}', 'active', 1, 1, 1)",
            ["a".repeat(64)],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO hub_node_outbox (
                 device_id, sequence, message_kind, message_json,
                 message_sha256, created_at_ms
             ) VALUES ('node-v48', 1, 'welcome', '{}', ?1, 2)",
            ["b".repeat(64)],
        )
        .unwrap();

        conn.execute("DROP INDEX idx_hub_node_outbox_delivery", [])
            .unwrap();
        conn.execute(
            "ALTER TABLE hub_node_outbox DROP COLUMN superseded_at_ms",
            [],
        )
        .unwrap();
        conn.execute("DELETE FROM migrations WHERE version = 48", [])
            .unwrap();
        conn.pragma_update(None, "user_version", 47).unwrap();

        run_migrations(&conn).unwrap();
        assert!(column_exists(&conn, "hub_node_outbox", "superseded_at_ms"));
        let retained: (String, Option<i64>) = conn
            .query_row(
                "SELECT message_kind, superseded_at_ms
                 FROM hub_node_outbox WHERE device_id = 'node-v48'",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        assert_eq!(retained, ("welcome".to_string(), None));

        conn.execute("DELETE FROM migrations WHERE version = 48", [])
            .unwrap();
        conn.pragma_update(None, "user_version", 47).unwrap();
        run_migrations(&conn).unwrap();
        assert_eq!(get_schema_version(&conn), SCHEMA_VERSION);
    }

    #[test]
    fn v49_adds_exact_node_run_decisions_and_replays_safely() {
        let conn = Connection::open_in_memory().unwrap();
        run_migrations(&conn).unwrap();
        conn.execute_batch(
            "DROP TABLE hub_node_run_approvals;
             ALTER TABLE hub_node_runs DROP COLUMN rejection_sha256;
             ALTER TABLE hub_node_runs DROP COLUMN rejection_json;
             DELETE FROM migrations WHERE version = 49;
             PRAGMA user_version = 48;",
        )
        .unwrap();

        run_migrations(&conn).unwrap();
        assert!(column_exists(&conn, "hub_node_runs", "rejection_json"));
        assert!(column_exists(&conn, "hub_node_runs", "rejection_sha256"));
        assert!(table_exists(&conn, "hub_node_run_approvals").unwrap());

        conn.execute("DELETE FROM migrations WHERE version = 49", [])
            .unwrap();
        conn.pragma_update(None, "user_version", 48).unwrap();
        run_migrations(&conn).unwrap();
        assert_eq!(get_schema_version(&conn), SCHEMA_VERSION);
    }

    #[test]
    fn v50_adds_durable_device_access_token_digests_idempotently() {
        let conn = Connection::open_in_memory().unwrap();
        run_migrations(&conn).unwrap();
        conn.execute_batch(
            "DROP TABLE device_access_tokens;
             DELETE FROM migrations WHERE version = 50;
             PRAGMA user_version = 49;",
        )
        .unwrap();

        run_migrations(&conn).unwrap();
        assert!(table_exists(&conn, "device_access_tokens").unwrap());

        conn.execute("DELETE FROM migrations WHERE version = 50", [])
            .unwrap();
        conn.pragma_update(None, "user_version", 49).unwrap();
        run_migrations(&conn).unwrap();
        assert_eq!(get_schema_version(&conn), SCHEMA_VERSION);
    }

    #[test]
    fn v51_adds_logical_execution_targets_idempotently() {
        let conn = Connection::open_in_memory().unwrap();
        run_migrations(&conn).unwrap();
        conn.execute_batch(
            "DROP TABLE execution_target_bindings;
             DELETE FROM migrations WHERE version = 51;
             PRAGMA user_version = 50;",
        )
        .unwrap();

        run_migrations(&conn).unwrap();
        assert!(table_exists(&conn, "execution_target_bindings").unwrap());
        assert!(table_exists(&conn, "sessions").unwrap());
        let trigger_count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master
                 WHERE type = 'trigger'
                   AND name IN ('cleanup_session_execution_target',
                                'cleanup_project_execution_target')",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(trigger_count, 2);
        conn.execute(
            "INSERT INTO execution_target_bindings
                (scope_kind, scope_id, target_kind, device_id, workspace_id, updated_at_ms)
             VALUES ('session', 'session-1', 'hub', NULL, NULL, 1)",
            [],
        )
        .unwrap();

        conn.execute("DELETE FROM migrations WHERE version = 51", [])
            .unwrap();
        conn.pragma_update(None, "user_version", 50).unwrap();
        run_migrations(&conn).unwrap();
        let retained: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM execution_target_bindings
                 WHERE scope_kind = 'session' AND scope_id = 'session-1'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(retained, 1);
        assert_eq!(get_schema_version(&conn), SCHEMA_VERSION);
    }
}
