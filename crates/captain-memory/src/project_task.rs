//! Task graph per project (v3.11b).
//!
//! Tasks belong to a single project and optionally nest via `parent_id`
//! to form a DAG of sub-tasks. Status transitions match the roadmap:
//! `todo → doing → (blocked | review) → done | cancelled`.
//!
//! Cycle detection is enforced on insert / update: a task cannot
//! become its own ancestor. Orphan tasks (pointing to a non-existent
//! parent in the same project) are rejected at the same gate.

use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum TaskStatus {
    Todo,
    Doing,
    Blocked,
    Review,
    Done,
    Cancelled,
}

impl TaskStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Todo => "todo",
            Self::Doing => "doing",
            Self::Blocked => "blocked",
            Self::Review => "review",
            Self::Done => "done",
            Self::Cancelled => "cancelled",
        }
    }

    #[allow(clippy::should_implement_trait)]
    pub fn from_str(s: &str) -> Option<Self> {
        match s {
            "todo" => Some(Self::Todo),
            "doing" => Some(Self::Doing),
            "blocked" => Some(Self::Blocked),
            "review" => Some(Self::Review),
            "done" => Some(Self::Done),
            "cancelled" => Some(Self::Cancelled),
            _ => None,
        }
    }

    pub fn is_terminal(&self) -> bool {
        matches!(self, Self::Done | Self::Cancelled)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ProjectTask {
    pub id: String,
    pub project_id: String,
    pub parent_id: Option<String>,
    pub title: String,
    pub description: String,
    pub status: TaskStatus,
    pub assignee_agent_id: Option<String>,
    pub priority: i32,
    pub deadline: Option<i64>,
    pub created_at: i64,
    pub updated_at: i64,
    pub completed_at: Option<i64>,
}

#[derive(Debug, Clone)]
pub struct NewProjectTask {
    pub project_id: String,
    pub parent_id: Option<String>,
    pub title: String,
    pub description: String,
    pub priority: i32,
    pub deadline: Option<i64>,
    pub assignee_agent_id: Option<String>,
}

#[derive(Debug, Clone, Default)]
pub struct TaskPatch {
    pub title: Option<String>,
    pub description: Option<String>,
    pub status: Option<TaskStatus>,
    pub parent_id: Option<Option<String>>,
    pub priority: Option<i32>,
    pub deadline: Option<Option<i64>>,
    pub assignee_agent_id: Option<Option<String>>,
}

/// Error surfaced at the DAG-invariant gate.
#[derive(Debug, thiserror::Error)]
pub enum TaskError {
    #[error("parent task {0} does not belong to the same project")]
    OrphanParent(String),
    #[error("parent task {0} does not exist")]
    UnknownParent(String),
    #[error("cycle detected: task {0} would become its own ancestor via {1}")]
    Cycle(String, String),
    #[error(transparent)]
    Sql(#[from] rusqlite::Error),
}

pub fn create(conn: &Connection, input: NewProjectTask) -> Result<ProjectTask, TaskError> {
    if let Some(pid) = &input.parent_id {
        validate_parent(conn, pid, &input.project_id, None)?;
    }
    let id = uuid::Uuid::new_v4().to_string();
    let now = now_unix_ms();
    conn.execute(
        "INSERT INTO project_tasks (id, project_id, parent_id, title, description, status, assignee_agent_id, priority, deadline, created_at, updated_at, completed_at)
         VALUES (?1, ?2, ?3, ?4, ?5, 'todo', ?6, ?7, ?8, ?9, ?9, NULL)",
        params![
            id,
            input.project_id,
            input.parent_id,
            input.title,
            input.description,
            input.assignee_agent_id,
            input.priority,
            input.deadline,
            now,
        ],
    )?;
    get(conn, &id)?.ok_or(TaskError::Sql(rusqlite::Error::QueryReturnedNoRows))
}

pub fn get(conn: &Connection, id: &str) -> Result<Option<ProjectTask>, rusqlite::Error> {
    let mut stmt = conn.prepare(
        "SELECT id, project_id, parent_id, title, description, status, assignee_agent_id,
                priority, deadline, created_at, updated_at, completed_at
         FROM project_tasks WHERE id = ?1",
    )?;
    let mut rows = stmt.query(params![id])?;
    if let Some(row) = rows.next()? {
        Ok(Some(row_to_task(row)?))
    } else {
        Ok(None)
    }
}

pub fn list_for_project(
    conn: &Connection,
    project_id: &str,
) -> Result<Vec<ProjectTask>, rusqlite::Error> {
    let mut stmt = conn.prepare(
        "SELECT id, project_id, parent_id, title, description, status, assignee_agent_id,
                priority, deadline, created_at, updated_at, completed_at
         FROM project_tasks WHERE project_id = ?1
         ORDER BY priority DESC, created_at ASC",
    )?;
    let rows = stmt.query_map(params![project_id], row_to_task)?;
    let mut out = Vec::new();
    for row in rows {
        out.push(row?);
    }
    Ok(out)
}

pub fn update(
    conn: &Connection,
    id: &str,
    patch: TaskPatch,
) -> Result<Option<ProjectTask>, TaskError> {
    let Some(existing) = get(conn, id)? else {
        return Ok(None);
    };
    if let Some(Some(pid)) = &patch.parent_id {
        validate_parent(conn, pid, &existing.project_id, Some(id))?;
    }
    let now = now_unix_ms();
    if let Some(title) = patch.title {
        conn.execute(
            "UPDATE project_tasks SET title = ?1, updated_at = ?2 WHERE id = ?3",
            params![title, now, id],
        )?;
    }
    if let Some(description) = patch.description {
        conn.execute(
            "UPDATE project_tasks SET description = ?1, updated_at = ?2 WHERE id = ?3",
            params![description, now, id],
        )?;
    }
    if let Some(status) = patch.status {
        let completed_at = if status == TaskStatus::Done {
            Some(now)
        } else {
            None
        };
        conn.execute(
            "UPDATE project_tasks SET status = ?1, updated_at = ?2, completed_at = ?3 WHERE id = ?4",
            params![status.as_str(), now, completed_at, id],
        )?;
    }
    if let Some(parent_id) = patch.parent_id {
        conn.execute(
            "UPDATE project_tasks SET parent_id = ?1, updated_at = ?2 WHERE id = ?3",
            params![parent_id, now, id],
        )?;
    }
    if let Some(priority) = patch.priority {
        conn.execute(
            "UPDATE project_tasks SET priority = ?1, updated_at = ?2 WHERE id = ?3",
            params![priority, now, id],
        )?;
    }
    if let Some(deadline) = patch.deadline {
        conn.execute(
            "UPDATE project_tasks SET deadline = ?1, updated_at = ?2 WHERE id = ?3",
            params![deadline, now, id],
        )?;
    }
    if let Some(assignee) = patch.assignee_agent_id {
        conn.execute(
            "UPDATE project_tasks SET assignee_agent_id = ?1, updated_at = ?2 WHERE id = ?3",
            params![assignee, now, id],
        )?;
    }
    Ok(get(conn, id)?)
}

pub fn delete(conn: &Connection, id: &str) -> Result<bool, rusqlite::Error> {
    let n = conn.execute("DELETE FROM project_tasks WHERE id = ?1", params![id])?;
    Ok(n > 0)
}

/// Validate that `parent_id` belongs to the same project and does not
/// create a cycle with `self_id` (if updating). Used at create and
/// update gates.
fn validate_parent(
    conn: &Connection,
    parent_id: &str,
    project_id: &str,
    self_id: Option<&str>,
) -> Result<(), TaskError> {
    let parent =
        get(conn, parent_id)?.ok_or_else(|| TaskError::UnknownParent(parent_id.to_string()))?;
    if parent.project_id != project_id {
        return Err(TaskError::OrphanParent(parent_id.to_string()));
    }
    // Walk ancestors upward. If we meet self_id, it's a cycle.
    if let Some(self_id) = self_id {
        let mut cursor = Some(parent_id.to_string());
        let mut seen: HashSet<String> = HashSet::new();
        while let Some(pid) = cursor {
            if pid == self_id {
                return Err(TaskError::Cycle(self_id.to_string(), parent_id.to_string()));
            }
            if !seen.insert(pid.clone()) {
                // Defensive: existing cycle in stored data — bail.
                break;
            }
            let row = get(conn, &pid)?;
            cursor = row.and_then(|t| t.parent_id);
        }
    }
    Ok(())
}

fn row_to_task(row: &rusqlite::Row<'_>) -> Result<ProjectTask, rusqlite::Error> {
    let status_str: String = row.get(5)?;
    Ok(ProjectTask {
        id: row.get(0)?,
        project_id: row.get(1)?,
        parent_id: row.get(2)?,
        title: row.get(3)?,
        description: row.get(4)?,
        status: TaskStatus::from_str(&status_str).unwrap_or(TaskStatus::Todo),
        assignee_agent_id: row.get(6)?,
        priority: row.get(7)?,
        deadline: row.get(8)?,
        created_at: row.get(9)?,
        updated_at: row.get(10)?,
        completed_at: row.get(11)?,
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
    use crate::project::{create as create_project, NewProject};

    fn fresh_db() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        run_migrations(&conn).unwrap();
        conn
    }

    fn make_project(conn: &Connection, slug: &str) -> String {
        create_project(
            conn,
            NewProject {
                name: format!("p-{slug}"),
                slug: slug.to_string(),
                goal: String::new(),
                deadline: None,
            },
        )
        .unwrap()
        .id
    }

    fn base(project_id: &str, title: &str) -> NewProjectTask {
        NewProjectTask {
            project_id: project_id.to_string(),
            parent_id: None,
            title: title.to_string(),
            description: String::new(),
            priority: 0,
            deadline: None,
            assignee_agent_id: None,
        }
    }

    #[test]
    fn create_then_list_roundtrip() {
        let conn = fresh_db();
        let pid = make_project(&conn, "alpha");
        let t = create(&conn, base(&pid, "draft design")).unwrap();
        assert_eq!(t.status, TaskStatus::Todo);
        assert_eq!(t.project_id, pid);

        let all = list_for_project(&conn, &pid).unwrap();
        assert_eq!(all.len(), 1);
        assert_eq!(all[0].title, "draft design");
    }

    #[test]
    fn orphan_parent_is_rejected() {
        let conn = fresh_db();
        let pid = make_project(&conn, "alpha");
        let mut nt = base(&pid, "child");
        nt.parent_id = Some("does-not-exist".into());
        let err = create(&conn, nt).unwrap_err();
        assert!(matches!(err, TaskError::UnknownParent(_)));
    }

    #[test]
    fn parent_from_other_project_is_rejected() {
        let conn = fresh_db();
        let p_a = make_project(&conn, "alpha");
        let p_b = make_project(&conn, "beta");
        let parent_in_a = create(&conn, base(&p_a, "root-a")).unwrap();

        let mut nt = base(&p_b, "child-b");
        nt.parent_id = Some(parent_in_a.id);
        let err = create(&conn, nt).unwrap_err();
        assert!(matches!(err, TaskError::OrphanParent(_)));
    }

    #[test]
    fn cycle_detection_blocks_self_ancestry() {
        let conn = fresh_db();
        let pid = make_project(&conn, "alpha");
        let a = create(&conn, base(&pid, "a")).unwrap();
        let b = create(
            &conn,
            NewProjectTask {
                parent_id: Some(a.id.clone()),
                ..base(&pid, "b")
            },
        )
        .unwrap();

        // Attempt to make A's parent become B (B -> A -> B cycle)
        let patched = update(
            &conn,
            &a.id,
            TaskPatch {
                parent_id: Some(Some(b.id.clone())),
                ..Default::default()
            },
        );
        assert!(matches!(patched, Err(TaskError::Cycle(_, _))));
    }

    #[test]
    fn status_done_stamps_completed_at() {
        let conn = fresh_db();
        let pid = make_project(&conn, "alpha");
        let t = create(&conn, base(&pid, "ship")).unwrap();
        assert!(t.completed_at.is_none());
        std::thread::sleep(std::time::Duration::from_millis(3));

        let done = update(
            &conn,
            &t.id,
            TaskPatch {
                status: Some(TaskStatus::Done),
                ..Default::default()
            },
        )
        .unwrap()
        .unwrap();
        assert!(done.completed_at.is_some());
        assert!(done.updated_at > t.updated_at);
    }

    #[test]
    fn delete_removes_task() {
        let conn = fresh_db();
        let pid = make_project(&conn, "alpha");
        let t = create(&conn, base(&pid, "tmp")).unwrap();
        assert!(delete(&conn, &t.id).unwrap());
        assert!(get(&conn, &t.id).unwrap().is_none());
    }

    #[test]
    fn cascade_delete_when_project_is_removed() {
        let conn = fresh_db();
        let pid = make_project(&conn, "alpha");
        create(&conn, base(&pid, "a")).unwrap();
        create(&conn, base(&pid, "b")).unwrap();
        assert_eq!(list_for_project(&conn, &pid).unwrap().len(), 2);

        conn.execute("DELETE FROM projects WHERE id = ?1", params![pid])
            .unwrap();
        assert_eq!(list_for_project(&conn, &pid).unwrap().len(), 0);
    }

    #[test]
    fn status_roundtrip_and_terminal_flag() {
        for s in [
            TaskStatus::Todo,
            TaskStatus::Doing,
            TaskStatus::Blocked,
            TaskStatus::Review,
            TaskStatus::Done,
            TaskStatus::Cancelled,
        ] {
            assert_eq!(TaskStatus::from_str(s.as_str()), Some(s));
        }
        assert!(TaskStatus::Done.is_terminal());
        assert!(TaskStatus::Cancelled.is_terminal());
        assert!(!TaskStatus::Todo.is_terminal());
    }
}
