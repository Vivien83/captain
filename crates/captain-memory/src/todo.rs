//! Cross-session todo store.
//!
//! A deliberately minimal "things to do" surface that survives restarts and
//! compactions, sitting alongside the heavier `project_task_*` (project DAG)
//! and `goal_*` (autopilot loops) systems. Captain reaches for this when the
//! user wants to capture a quick item without spinning up a project or a
//! reflection budget.
//!
//! Schema invariants:
//! - one global table, no project FK, no agent FK — todos are global,
//!   matching the cross-session use case;
//! - `done` is a boolean flag, not a status enum — sub-states (blocked /
//!   review / cancelled) are project_task territory;
//! - `completed_at` is set the moment `done` flips to 1 and cleared if it
//!   flips back to 0 (resurrected todo).

use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};
use std::time::{SystemTime, UNIX_EPOCH};

/// A persisted todo item.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Todo {
    pub id: String,
    pub title: String,
    pub body: String,
    pub done: bool,
    /// Unix epoch milliseconds when the todo was created.
    pub created_at: i64,
    /// Unix epoch milliseconds when `done` last became `true`. `None` while
    /// the todo is still open.
    pub completed_at: Option<i64>,
}

/// Insert payload — the rest of the row is generated server-side.
#[derive(Debug, Clone)]
pub struct NewTodo {
    pub title: String,
    pub body: String,
}

/// Filter for [`list`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TodoFilter {
    /// Only `done = 0` rows.
    Open,
    /// Only `done = 1` rows.
    Done,
    /// Every row.
    All,
}

/// Default page size when the caller does not specify a limit. Big enough
/// that interactive usage never feels truncated, small enough that an
/// accidental `todo_list({})` after years of capture cannot stuff thousands
/// of rows into the LLM's context window.
pub const DEFAULT_TODO_LIST_LIMIT: u32 = 200;
/// Hard upper bound on `todo_list` page size — even when an operator
/// explicitly asks for more, we cap here to protect the response.
pub const MAX_TODO_LIST_LIMIT: u32 = 1_000;

/// Clamp a caller-supplied limit to the supported range. `None` returns the
/// default page size; `Some(0)` is treated as "default" (a 0-row response is
/// useless and almost always a typo).
pub fn clamp_todo_list_limit(value: Option<u32>) -> u32 {
    match value {
        None | Some(0) => DEFAULT_TODO_LIST_LIMIT,
        Some(n) => n.min(MAX_TODO_LIST_LIMIT),
    }
}

/// Insert a new todo. Returns the freshly-stored row.
pub fn create(conn: &Connection, input: NewTodo) -> Result<Todo, rusqlite::Error> {
    let id = uuid::Uuid::new_v4().to_string();
    let now = now_unix_ms();
    conn.execute(
        "INSERT INTO todos (id, title, body, done, created_at, completed_at)
         VALUES (?1, ?2, ?3, 0, ?4, NULL)",
        params![id, input.title, input.body, now],
    )?;
    get(conn, &id)?.ok_or(rusqlite::Error::QueryReturnedNoRows)
}

/// Fetch one todo by id.
pub fn get(conn: &Connection, id: &str) -> Result<Option<Todo>, rusqlite::Error> {
    let mut stmt = conn.prepare(
        "SELECT id, title, body, done, created_at, completed_at FROM todos WHERE id = ?1",
    )?;
    let mut rows = stmt.query(params![id])?;
    if let Some(row) = rows.next()? {
        Ok(Some(row_to_todo(row)?))
    } else {
        Ok(None)
    }
}

/// List todos, ordered open-first then by recency. The newest row is on top
/// inside each bucket so the most recent capture is always visible.
///
/// `limit` is clamped via [`clamp_todo_list_limit`]: `None` falls back to
/// [`DEFAULT_TODO_LIST_LIMIT`], anything above [`MAX_TODO_LIST_LIMIT`] is
/// capped. Pagination beyond the page size is intentionally not supported —
/// callers needing more should drive the query themselves via
/// [`crate::todo`] helpers or rely on `todo_complete` / `todo_delete` to
/// keep the working set bounded.
pub fn list(
    conn: &Connection,
    filter: TodoFilter,
    limit: Option<u32>,
) -> Result<Vec<Todo>, rusqlite::Error> {
    let cap = clamp_todo_list_limit(limit);
    let sql = match filter {
        TodoFilter::Open => {
            "SELECT id, title, body, done, created_at, completed_at FROM todos
             WHERE done = 0 ORDER BY created_at DESC LIMIT ?1"
        }
        TodoFilter::Done => {
            "SELECT id, title, body, done, created_at, completed_at FROM todos
             WHERE done = 1 ORDER BY completed_at DESC LIMIT ?1"
        }
        TodoFilter::All => {
            "SELECT id, title, body, done, created_at, completed_at FROM todos
             ORDER BY done ASC, created_at DESC LIMIT ?1"
        }
    };
    let mut stmt = conn.prepare(sql)?;
    let rows = stmt.query_map(params![cap], row_to_todo)?;
    let mut out = Vec::new();
    for row in rows {
        out.push(row?);
    }
    Ok(out)
}

/// Mark a todo as done. Returns `Ok(None)` if the id does not exist.
pub fn complete(conn: &Connection, id: &str) -> Result<Option<Todo>, rusqlite::Error> {
    if get(conn, id)?.is_none() {
        return Ok(None);
    }
    let now = now_unix_ms();
    conn.execute(
        "UPDATE todos SET done = 1, completed_at = ?1 WHERE id = ?2",
        params![now, id],
    )?;
    get(conn, id)
}

/// Reopen a previously completed todo. Returns `Ok(None)` if the id does not
/// exist. Idempotent on already-open todos.
pub fn reopen(conn: &Connection, id: &str) -> Result<Option<Todo>, rusqlite::Error> {
    if get(conn, id)?.is_none() {
        return Ok(None);
    }
    conn.execute(
        "UPDATE todos SET done = 0, completed_at = NULL WHERE id = ?1",
        params![id],
    )?;
    get(conn, id)
}

/// Remove a todo. Returns `true` when a row was actually deleted.
pub fn delete(conn: &Connection, id: &str) -> Result<bool, rusqlite::Error> {
    let n = conn.execute("DELETE FROM todos WHERE id = ?1", params![id])?;
    Ok(n > 0)
}

fn row_to_todo(row: &rusqlite::Row<'_>) -> Result<Todo, rusqlite::Error> {
    Ok(Todo {
        id: row.get(0)?,
        title: row.get(1)?,
        body: row.get(2)?,
        done: row.get::<_, i64>(3)? != 0,
        created_at: row.get(4)?,
        completed_at: row.get(5)?,
    })
}

fn now_unix_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::migration::run_migrations;

    fn fresh_db() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        run_migrations(&conn).unwrap();
        conn
    }

    #[test]
    fn create_then_get_round_trips() {
        let conn = fresh_db();
        let made = create(
            &conn,
            NewTodo {
                title: "buy milk".into(),
                body: "two litres, lactose-free".into(),
            },
        )
        .unwrap();
        let fetched = get(&conn, &made.id).unwrap().unwrap();
        assert_eq!(made, fetched);
        assert_eq!(made.title, "buy milk");
        assert_eq!(made.body, "two litres, lactose-free");
        assert!(!made.done);
        assert!(made.completed_at.is_none());
    }

    #[test]
    fn complete_sets_done_and_completed_at() {
        let conn = fresh_db();
        let made = create(
            &conn,
            NewTodo {
                title: "ship release".into(),
                body: String::new(),
            },
        )
        .unwrap();
        let after = complete(&conn, &made.id).unwrap().unwrap();
        assert!(after.done);
        assert!(after.completed_at.is_some());
    }

    #[test]
    fn reopen_clears_completed_at() {
        let conn = fresh_db();
        let made = create(
            &conn,
            NewTodo {
                title: "x".into(),
                body: String::new(),
            },
        )
        .unwrap();
        complete(&conn, &made.id).unwrap();
        let reopened = reopen(&conn, &made.id).unwrap().unwrap();
        assert!(!reopened.done);
        assert!(reopened.completed_at.is_none());
    }

    #[test]
    fn list_filters_open_done_all() {
        let conn = fresh_db();
        let a = create(
            &conn,
            NewTodo {
                title: "a".into(),
                body: String::new(),
            },
        )
        .unwrap();
        let b = create(
            &conn,
            NewTodo {
                title: "b".into(),
                body: String::new(),
            },
        )
        .unwrap();
        let _c = create(
            &conn,
            NewTodo {
                title: "c".into(),
                body: String::new(),
            },
        )
        .unwrap();
        complete(&conn, &a.id).unwrap();
        complete(&conn, &b.id).unwrap();

        assert_eq!(list(&conn, TodoFilter::Open, None).unwrap().len(), 1);
        assert_eq!(list(&conn, TodoFilter::Done, None).unwrap().len(), 2);
        assert_eq!(list(&conn, TodoFilter::All, None).unwrap().len(), 3);
    }

    #[test]
    fn list_caps_at_default_limit() {
        let conn = fresh_db();
        for i in 0..(DEFAULT_TODO_LIST_LIMIT as usize + 5) {
            create(
                &conn,
                NewTodo {
                    title: format!("t{i}"),
                    body: String::new(),
                },
            )
            .unwrap();
        }
        // None  → DEFAULT_TODO_LIST_LIMIT
        let page = list(&conn, TodoFilter::Open, None).unwrap();
        assert_eq!(page.len(), DEFAULT_TODO_LIST_LIMIT as usize);
        // Some(0) → DEFAULT (treat 0 as typo)
        let page0 = list(&conn, TodoFilter::Open, Some(0)).unwrap();
        assert_eq!(page0.len(), DEFAULT_TODO_LIST_LIMIT as usize);
        // Some(2) → 2
        assert_eq!(list(&conn, TodoFilter::Open, Some(2)).unwrap().len(), 2);
    }

    #[test]
    fn list_clamps_above_max_limit() {
        let conn = fresh_db();
        for i in 0..5 {
            create(
                &conn,
                NewTodo {
                    title: format!("t{i}"),
                    body: String::new(),
                },
            )
            .unwrap();
        }
        // The clamp helper itself is the source of truth for the cap.
        assert_eq!(clamp_todo_list_limit(None), DEFAULT_TODO_LIST_LIMIT);
        assert_eq!(clamp_todo_list_limit(Some(0)), DEFAULT_TODO_LIST_LIMIT);
        assert_eq!(
            clamp_todo_list_limit(Some(MAX_TODO_LIST_LIMIT + 1)),
            MAX_TODO_LIST_LIMIT
        );
    }

    #[test]
    fn delete_removes_row() {
        let conn = fresh_db();
        let made = create(
            &conn,
            NewTodo {
                title: "x".into(),
                body: String::new(),
            },
        )
        .unwrap();
        assert!(delete(&conn, &made.id).unwrap());
        assert!(get(&conn, &made.id).unwrap().is_none());
        assert!(
            !delete(&conn, &made.id).unwrap(),
            "second delete is a no-op"
        );
    }

    #[test]
    fn complete_unknown_id_returns_none() {
        let conn = fresh_db();
        assert!(complete(&conn, "00000000-0000-0000-0000-000000000000")
            .unwrap()
            .is_none());
    }
}
