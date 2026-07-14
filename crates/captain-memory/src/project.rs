//! Project entity — CRUD over the `projects` table (v3.11a).
//!
//! A project groups related work (tasks, milestones, memory, crons) under
//! a single slug. Subsequent v3.11 sub-phases (task graph, milestones,
//! memory wing, context switch, weekly reflection, handoff) all key off
//! `project_id`, so this module is the foundation.

use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};
use std::time::{SystemTime, UNIX_EPOCH};

/// Lifecycle status of a project.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum ProjectStatus {
    Planning,
    Active,
    Paused,
    Done,
    Archived,
}

impl ProjectStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Planning => "planning",
            Self::Active => "active",
            Self::Paused => "paused",
            Self::Done => "done",
            Self::Archived => "archived",
        }
    }

    #[allow(clippy::should_implement_trait)]
    pub fn from_str(s: &str) -> Option<Self> {
        match s {
            "planning" => Some(Self::Planning),
            "active" => Some(Self::Active),
            "paused" => Some(Self::Paused),
            "done" => Some(Self::Done),
            "archived" => Some(Self::Archived),
            _ => None,
        }
    }
}

/// One row from the `projects` table.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Project {
    pub id: String,
    pub name: String,
    pub slug: String,
    pub goal: String,
    pub status: ProjectStatus,
    pub deadline: Option<i64>,
    pub created_at: i64,
    pub updated_at: i64,
    pub metadata: serde_json::Value,
}

/// Fields required to create a new project. Everything else defaults.
#[derive(Debug, Clone)]
pub struct NewProject {
    pub name: String,
    pub slug: String,
    pub goal: String,
    pub deadline: Option<i64>,
}

/// Patch applied by [`update`]. Only the `Some` fields are written.
#[derive(Debug, Clone, Default)]
pub struct ProjectPatch {
    pub name: Option<String>,
    pub goal: Option<String>,
    pub status: Option<ProjectStatus>,
    pub deadline: Option<Option<i64>>,
    pub metadata: Option<serde_json::Value>,
}

/// Create a new project. Returns the created row with its generated id.
///
/// The `slug` must be unique across active rows — the underlying UNIQUE
/// constraint surfaces a friendly error when violated.
pub fn create(conn: &Connection, input: NewProject) -> Result<Project, rusqlite::Error> {
    let id = uuid::Uuid::new_v4().to_string();
    let now = now_unix_ms();
    conn.execute(
        "INSERT INTO projects (id, name, slug, goal, status, deadline, created_at, updated_at, metadata_json)
         VALUES (?1, ?2, ?3, ?4, 'planning', ?5, ?6, ?6, '{}')",
        params![id, input.name, input.slug, input.goal, input.deadline, now],
    )?;
    get(conn, &id)?.ok_or_else(|| rusqlite::Error::QueryReturnedNoRows)
}

/// Fetch one project by id.
pub fn get(conn: &Connection, id: &str) -> Result<Option<Project>, rusqlite::Error> {
    let mut stmt = conn.prepare(
        "SELECT id, name, slug, goal, status, deadline, created_at, updated_at, metadata_json
         FROM projects WHERE id = ?1",
    )?;
    let mut rows = stmt.query(params![id])?;
    if let Some(row) = rows.next()? {
        Ok(Some(row_to_project(row)?))
    } else {
        Ok(None)
    }
}

/// Lookup a project by slug (unique). Convenience for `/project <slug>`.
pub fn find_by_slug(conn: &Connection, slug: &str) -> Result<Option<Project>, rusqlite::Error> {
    let mut stmt = conn.prepare(
        "SELECT id, name, slug, goal, status, deadline, created_at, updated_at, metadata_json
         FROM projects WHERE slug = ?1",
    )?;
    let mut rows = stmt.query(params![slug])?;
    if let Some(row) = rows.next()? {
        Ok(Some(row_to_project(row)?))
    } else {
        Ok(None)
    }
}

/// List projects, ordered by most recently updated. When `include_archived`
/// is false, `archived` rows are skipped — the common dashboard case.
pub fn list(conn: &Connection, include_archived: bool) -> Result<Vec<Project>, rusqlite::Error> {
    let mut stmt = if include_archived {
        conn.prepare(
            "SELECT id, name, slug, goal, status, deadline, created_at, updated_at, metadata_json
             FROM projects ORDER BY updated_at DESC",
        )?
    } else {
        conn.prepare(
            "SELECT id, name, slug, goal, status, deadline, created_at, updated_at, metadata_json
             FROM projects WHERE status != 'archived' ORDER BY updated_at DESC",
        )?
    };
    let rows = stmt.query_map([], row_to_project)?;
    let mut out = Vec::new();
    for row in rows {
        out.push(row?);
    }
    Ok(out)
}

/// Apply a partial update. Returns the refreshed row or `None` if the
/// project does not exist.
pub fn update(
    conn: &Connection,
    id: &str,
    patch: ProjectPatch,
) -> Result<Option<Project>, rusqlite::Error> {
    if get(conn, id)?.is_none() {
        return Ok(None);
    }
    let now = now_unix_ms();
    if let Some(name) = patch.name {
        conn.execute(
            "UPDATE projects SET name = ?1, updated_at = ?2 WHERE id = ?3",
            params![name, now, id],
        )?;
    }
    if let Some(goal) = patch.goal {
        conn.execute(
            "UPDATE projects SET goal = ?1, updated_at = ?2 WHERE id = ?3",
            params![goal, now, id],
        )?;
    }
    if let Some(status) = patch.status {
        conn.execute(
            "UPDATE projects SET status = ?1, updated_at = ?2 WHERE id = ?3",
            params![status.as_str(), now, id],
        )?;
    }
    if let Some(deadline) = patch.deadline {
        conn.execute(
            "UPDATE projects SET deadline = ?1, updated_at = ?2 WHERE id = ?3",
            params![deadline, now, id],
        )?;
    }
    if let Some(metadata) = patch.metadata {
        let json = serde_json::to_string(&metadata).unwrap_or_else(|_| "{}".to_string());
        conn.execute(
            "UPDATE projects SET metadata_json = ?1, updated_at = ?2 WHERE id = ?3",
            params![json, now, id],
        )?;
    }
    get(conn, id)
}

/// Shortcut for `update(status = Archived)`.
pub fn archive(conn: &Connection, id: &str) -> Result<Option<Project>, rusqlite::Error> {
    update(
        conn,
        id,
        ProjectPatch {
            status: Some(ProjectStatus::Archived),
            ..Default::default()
        },
    )
}

/// Permanently remove a project and its project-scoped records.
///
/// SQLite foreign keys are not enabled on every historical connection, so this
/// does the dependent deletes explicitly before deleting the project row.
pub fn delete(conn: &Connection, id: &str) -> Result<bool, rusqlite::Error> {
    conn.execute(
        "DELETE FROM project_checkpoints WHERE project_id = ?1",
        params![id],
    )?;
    conn.execute("DELETE FROM milestones WHERE project_id = ?1", params![id])?;
    conn.execute(
        "DELETE FROM project_tasks WHERE project_id = ?1",
        params![id],
    )?;
    let n = conn.execute("DELETE FROM projects WHERE id = ?1", params![id])?;
    Ok(n > 0)
}

fn row_to_project(row: &rusqlite::Row<'_>) -> Result<Project, rusqlite::Error> {
    let status_str: String = row.get(4)?;
    let metadata_str: String = row.get(8)?;
    let metadata = serde_json::from_str(&metadata_str)
        .unwrap_or(serde_json::Value::Object(Default::default()));
    Ok(Project {
        id: row.get(0)?,
        name: row.get(1)?,
        slug: row.get(2)?,
        goal: row.get(3)?,
        status: ProjectStatus::from_str(&status_str).unwrap_or(ProjectStatus::Planning),
        deadline: row.get(5)?,
        created_at: row.get(6)?,
        updated_at: row.get(7)?,
        metadata,
    })
}

fn now_unix_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::migration::run_migrations;

    fn fresh_db() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        run_migrations(&conn).unwrap();
        conn
    }

    fn base(slug: &str) -> NewProject {
        NewProject {
            name: format!("project {slug}"),
            slug: slug.to_string(),
            goal: "test goal".into(),
            deadline: None,
        }
    }

    #[test]
    fn create_then_get_roundtrip() {
        let conn = fresh_db();
        let p = create(&conn, base("alpha")).unwrap();
        assert_eq!(p.slug, "alpha");
        assert_eq!(p.status, ProjectStatus::Planning);
        assert_eq!(p.created_at, p.updated_at);

        let fetched = get(&conn, &p.id).unwrap().unwrap();
        assert_eq!(fetched, p);
    }

    #[test]
    fn find_by_slug_matches_exactly() {
        let conn = fresh_db();
        let p = create(&conn, base("beta")).unwrap();
        assert_eq!(find_by_slug(&conn, "beta").unwrap().unwrap().id, p.id);
        assert!(find_by_slug(&conn, "gamma").unwrap().is_none());
    }

    #[test]
    fn unique_slug_constraint_rejects_duplicates() {
        let conn = fresh_db();
        create(&conn, base("dupe")).unwrap();
        let err = create(&conn, base("dupe")).unwrap_err();
        let msg = err.to_string().to_lowercase();
        assert!(msg.contains("unique") || msg.contains("constraint"));
    }

    #[test]
    fn list_excludes_archived_by_default() {
        let conn = fresh_db();
        let a = create(&conn, base("keep")).unwrap();
        let b = create(&conn, base("bye")).unwrap();
        archive(&conn, &b.id).unwrap();

        let default_list = list(&conn, false).unwrap();
        assert_eq!(default_list.len(), 1);
        assert_eq!(default_list[0].id, a.id);

        let full_list = list(&conn, true).unwrap();
        assert_eq!(full_list.len(), 2);
    }

    #[test]
    fn update_patches_selected_fields_and_bumps_updated_at() {
        let conn = fresh_db();
        let p = create(&conn, base("patchme")).unwrap();
        std::thread::sleep(std::time::Duration::from_millis(5));

        let patched = update(
            &conn,
            &p.id,
            ProjectPatch {
                name: Some("Renamed".into()),
                status: Some(ProjectStatus::Active),
                deadline: Some(Some(1_700_000_000_000)),
                metadata: Some(serde_json::json!({ "owner": "captain" })),
                ..Default::default()
            },
        )
        .unwrap()
        .unwrap();

        assert_eq!(patched.name, "Renamed");
        assert_eq!(patched.status, ProjectStatus::Active);
        assert_eq!(patched.deadline, Some(1_700_000_000_000));
        assert_eq!(patched.metadata["owner"], "captain");
        assert!(patched.updated_at > p.updated_at);
        // goal untouched
        assert_eq!(patched.goal, p.goal);
    }

    #[test]
    fn update_returns_none_for_unknown_id() {
        let conn = fresh_db();
        let out = update(&conn, "ghost-id", ProjectPatch::default()).unwrap();
        assert!(out.is_none());
    }

    #[test]
    fn archive_transitions_status() {
        let conn = fresh_db();
        let p = create(&conn, base("gone")).unwrap();
        let archived = archive(&conn, &p.id).unwrap().unwrap();
        assert_eq!(archived.status, ProjectStatus::Archived);
    }

    #[test]
    fn delete_removes_project_and_project_scoped_rows() {
        let conn = fresh_db();
        let p = create(&conn, base("delete-me")).unwrap();
        conn.execute(
            "INSERT INTO project_tasks (id, project_id, parent_id, title, description, status, assignee_agent_id, priority, deadline, created_at, updated_at, completed_at)
             VALUES ('task-1', ?1, NULL, 'task', '', 'todo', NULL, 0, NULL, 1, 1, NULL)",
            params![p.id],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO milestones (id, project_id, name, due_date, status, deliverables_json, completed_at, created_at, updated_at)
             VALUES ('milestone-1', ?1, 'milestone', NULL, 'upcoming', '[]', NULL, 1, 1)",
            params![p.id],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO project_checkpoints (id, project_id, session_id, summary, state_json, created_at)
             VALUES ('checkpoint-1', ?1, NULL, 'checkpoint', '{}', 1)",
            params![p.id],
        )
        .unwrap();

        assert!(delete(&conn, &p.id).unwrap());
        assert!(get(&conn, &p.id).unwrap().is_none());
        for table in ["project_tasks", "milestones", "project_checkpoints"] {
            let count: i64 = conn
                .query_row(
                    &format!("SELECT COUNT(*) FROM {table} WHERE project_id = ?1"),
                    params![p.id],
                    |row| row.get(0),
                )
                .unwrap();
            assert_eq!(count, 0, "{table} should be cleaned up");
        }
    }

    #[test]
    fn status_roundtrip_stringifies() {
        for s in [
            ProjectStatus::Planning,
            ProjectStatus::Active,
            ProjectStatus::Paused,
            ProjectStatus::Done,
            ProjectStatus::Archived,
        ] {
            assert_eq!(ProjectStatus::from_str(s.as_str()), Some(s));
        }
        assert_eq!(ProjectStatus::from_str("nope"), None);
    }
}
