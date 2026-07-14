//! Project milestones with deadline tracking (v3.11c).
//!
//! Milestones are named checkpoints on a project's timeline, each with
//! an optional due date and a free-form deliverables list. The `status`
//! enum covers the lifecycle (`upcoming → in_progress → completed`); a
//! separate computed `is_missed()` flags anything past its deadline
//! that has not reached `completed` so dashboards can surface urgency
//! without a background job rewriting rows.

use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum MilestoneStatus {
    Upcoming,
    InProgress,
    Completed,
}

impl MilestoneStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Upcoming => "upcoming",
            Self::InProgress => "in_progress",
            Self::Completed => "completed",
        }
    }

    #[allow(clippy::should_implement_trait)]
    pub fn from_str(s: &str) -> Option<Self> {
        match s {
            "upcoming" => Some(Self::Upcoming),
            "in_progress" => Some(Self::InProgress),
            "completed" => Some(Self::Completed),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Milestone {
    pub id: String,
    pub project_id: String,
    pub name: String,
    pub due_date: Option<i64>,
    pub status: MilestoneStatus,
    pub deliverables: Vec<String>,
    pub completed_at: Option<i64>,
    pub created_at: i64,
    pub updated_at: i64,
}

impl Milestone {
    /// True when the milestone has a due date in the past AND has not
    /// been completed. Computed — not stored — so dashboards stay
    /// accurate without a background sweeper.
    pub fn is_missed(&self, now_unix_ms: i64) -> bool {
        self.status != MilestoneStatus::Completed
            && self.due_date.map(|d| d < now_unix_ms).unwrap_or(false)
    }
}

#[derive(Debug, Clone)]
pub struct NewMilestone {
    pub project_id: String,
    pub name: String,
    pub due_date: Option<i64>,
    pub deliverables: Vec<String>,
}

#[derive(Debug, Clone, Default)]
pub struct MilestonePatch {
    pub name: Option<String>,
    pub due_date: Option<Option<i64>>,
    pub status: Option<MilestoneStatus>,
    pub deliverables: Option<Vec<String>>,
}

/// Aggregate returned by [`progress`]. `pct` is in `[0.0, 1.0]`, or
/// `0.0` when the project has no milestones yet (avoids NaN).
#[derive(Debug, Clone, Copy, Serialize, PartialEq)]
pub struct MilestoneProgress {
    pub total: u32,
    pub completed: u32,
    pub missed: u32,
    pub pct: f32,
}

pub fn create(conn: &Connection, input: NewMilestone) -> Result<Milestone, rusqlite::Error> {
    let id = uuid::Uuid::new_v4().to_string();
    let now = now_unix_ms();
    let deliverables_json =
        serde_json::to_string(&input.deliverables).unwrap_or_else(|_| "[]".into());
    conn.execute(
        "INSERT INTO milestones (id, project_id, name, due_date, status, deliverables_json, completed_at, created_at, updated_at)
         VALUES (?1, ?2, ?3, ?4, 'upcoming', ?5, NULL, ?6, ?6)",
        params![id, input.project_id, input.name, input.due_date, deliverables_json, now],
    )?;
    get(conn, &id)?.ok_or(rusqlite::Error::QueryReturnedNoRows)
}

pub fn get(conn: &Connection, id: &str) -> Result<Option<Milestone>, rusqlite::Error> {
    let mut stmt = conn.prepare(
        "SELECT id, project_id, name, due_date, status, deliverables_json,
                completed_at, created_at, updated_at
         FROM milestones WHERE id = ?1",
    )?;
    let mut rows = stmt.query(params![id])?;
    if let Some(row) = rows.next()? {
        Ok(Some(row_to_milestone(row)?))
    } else {
        Ok(None)
    }
}

pub fn list_for_project(
    conn: &Connection,
    project_id: &str,
) -> Result<Vec<Milestone>, rusqlite::Error> {
    let mut stmt = conn.prepare(
        "SELECT id, project_id, name, due_date, status, deliverables_json,
                completed_at, created_at, updated_at
         FROM milestones WHERE project_id = ?1
         ORDER BY due_date ASC NULLS LAST, created_at ASC",
    )?;
    let rows = stmt.query_map(params![project_id], row_to_milestone)?;
    let mut out = Vec::new();
    for row in rows {
        out.push(row?);
    }
    Ok(out)
}

pub fn update(
    conn: &Connection,
    id: &str,
    patch: MilestonePatch,
) -> Result<Option<Milestone>, rusqlite::Error> {
    if get(conn, id)?.is_none() {
        return Ok(None);
    }
    let now = now_unix_ms();
    if let Some(name) = patch.name {
        conn.execute(
            "UPDATE milestones SET name = ?1, updated_at = ?2 WHERE id = ?3",
            params![name, now, id],
        )?;
    }
    if let Some(due_date) = patch.due_date {
        conn.execute(
            "UPDATE milestones SET due_date = ?1, updated_at = ?2 WHERE id = ?3",
            params![due_date, now, id],
        )?;
    }
    if let Some(status) = patch.status {
        let completed_at = if status == MilestoneStatus::Completed {
            Some(now)
        } else {
            None
        };
        conn.execute(
            "UPDATE milestones SET status = ?1, completed_at = ?2, updated_at = ?3 WHERE id = ?4",
            params![status.as_str(), completed_at, now, id],
        )?;
    }
    if let Some(deliverables) = patch.deliverables {
        let json = serde_json::to_string(&deliverables).unwrap_or_else(|_| "[]".into());
        conn.execute(
            "UPDATE milestones SET deliverables_json = ?1, updated_at = ?2 WHERE id = ?3",
            params![json, now, id],
        )?;
    }
    get(conn, id)
}

/// Shortcut for marking a milestone as completed.
pub fn complete(conn: &Connection, id: &str) -> Result<Option<Milestone>, rusqlite::Error> {
    update(
        conn,
        id,
        MilestonePatch {
            status: Some(MilestoneStatus::Completed),
            ..Default::default()
        },
    )
}

/// Aggregate progress for a project. `missed` is computed against `now`.
pub fn progress(
    conn: &Connection,
    project_id: &str,
    now_unix_ms: i64,
) -> Result<MilestoneProgress, rusqlite::Error> {
    let rows = list_for_project(conn, project_id)?;
    let total = rows.len() as u32;
    let completed = rows
        .iter()
        .filter(|m| m.status == MilestoneStatus::Completed)
        .count() as u32;
    let missed = rows.iter().filter(|m| m.is_missed(now_unix_ms)).count() as u32;
    let pct = if total == 0 {
        0.0
    } else {
        completed as f32 / total as f32
    };
    Ok(MilestoneProgress {
        total,
        completed,
        missed,
        pct,
    })
}

fn row_to_milestone(row: &rusqlite::Row<'_>) -> Result<Milestone, rusqlite::Error> {
    let status_str: String = row.get(4)?;
    let deliverables_str: String = row.get(5)?;
    let deliverables: Vec<String> = serde_json::from_str(&deliverables_str).unwrap_or_default();
    Ok(Milestone {
        id: row.get(0)?,
        project_id: row.get(1)?,
        name: row.get(2)?,
        due_date: row.get(3)?,
        status: MilestoneStatus::from_str(&status_str).unwrap_or(MilestoneStatus::Upcoming),
        deliverables,
        completed_at: row.get(6)?,
        created_at: row.get(7)?,
        updated_at: row.get(8)?,
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

    #[test]
    fn create_then_list_roundtrip() {
        let conn = fresh_db();
        let pid = make_project(&conn, "alpha");
        let m = create(
            &conn,
            NewMilestone {
                project_id: pid.clone(),
                name: "v1 beta".into(),
                due_date: Some(1_800_000_000_000),
                deliverables: vec!["docs".into(), "tests".into()],
            },
        )
        .unwrap();
        assert_eq!(m.status, MilestoneStatus::Upcoming);
        assert_eq!(m.deliverables, vec!["docs", "tests"]);

        let all = list_for_project(&conn, &pid).unwrap();
        assert_eq!(all.len(), 1);
        assert_eq!(all[0].id, m.id);
    }

    #[test]
    fn is_missed_is_computed_not_stored() {
        let past = 1_000_000_000_000_i64;
        let future = 9_000_000_000_000_i64;
        let now = 1_500_000_000_000_i64;

        let m = Milestone {
            id: "x".into(),
            project_id: "p".into(),
            name: "n".into(),
            due_date: Some(past),
            status: MilestoneStatus::Upcoming,
            deliverables: vec![],
            completed_at: None,
            created_at: 0,
            updated_at: 0,
        };
        assert!(m.is_missed(now));

        let done = Milestone {
            status: MilestoneStatus::Completed,
            ..m.clone()
        };
        assert!(
            !done.is_missed(now),
            "completed past milestone is not missed"
        );

        let soon = Milestone {
            due_date: Some(future),
            ..m
        };
        assert!(!soon.is_missed(now));
    }

    #[test]
    fn complete_stamps_completed_at() {
        let conn = fresh_db();
        let pid = make_project(&conn, "alpha");
        let m = create(
            &conn,
            NewMilestone {
                project_id: pid,
                name: "launch".into(),
                due_date: None,
                deliverables: vec![],
            },
        )
        .unwrap();
        assert!(m.completed_at.is_none());

        let done = complete(&conn, &m.id).unwrap().unwrap();
        assert_eq!(done.status, MilestoneStatus::Completed);
        assert!(done.completed_at.is_some());
    }

    #[test]
    fn progress_percentages_are_sane() {
        let conn = fresh_db();
        let pid = make_project(&conn, "alpha");
        let mut ids = Vec::new();
        for i in 0..4 {
            let m = create(
                &conn,
                NewMilestone {
                    project_id: pid.clone(),
                    name: format!("m{i}"),
                    due_date: Some(1_000_000_000_000 + i * 1000),
                    deliverables: vec![],
                },
            )
            .unwrap();
            ids.push(m.id);
        }
        // Complete 2 of 4
        complete(&conn, &ids[0]).unwrap();
        complete(&conn, &ids[1]).unwrap();

        let now = 1_500_000_000_000_i64; // after all due dates
        let p = progress(&conn, &pid, now).unwrap();
        assert_eq!(p.total, 4);
        assert_eq!(p.completed, 2);
        assert_eq!(p.missed, 2); // the 2 non-completed with past due_date
        assert!((p.pct - 0.5).abs() < 1e-6);
    }

    #[test]
    fn progress_returns_zero_pct_on_empty_project() {
        let conn = fresh_db();
        let pid = make_project(&conn, "empty");
        let p = progress(&conn, &pid, now_unix_ms()).unwrap();
        assert_eq!(p.total, 0);
        assert_eq!(p.pct, 0.0);
    }

    #[test]
    fn update_deliverables_roundtrip() {
        let conn = fresh_db();
        let pid = make_project(&conn, "alpha");
        let m = create(
            &conn,
            NewMilestone {
                project_id: pid,
                name: "x".into(),
                due_date: None,
                deliverables: vec!["a".into()],
            },
        )
        .unwrap();

        let patched = update(
            &conn,
            &m.id,
            MilestonePatch {
                deliverables: Some(vec!["a".into(), "b".into(), "c".into()]),
                ..Default::default()
            },
        )
        .unwrap()
        .unwrap();
        assert_eq!(patched.deliverables, vec!["a", "b", "c"]);
    }

    #[test]
    fn project_cascade_removes_milestones() {
        let conn = fresh_db();
        conn.execute_batch("PRAGMA foreign_keys = ON;").unwrap();
        let pid = make_project(&conn, "alpha");
        create(
            &conn,
            NewMilestone {
                project_id: pid.clone(),
                name: "x".into(),
                due_date: None,
                deliverables: vec![],
            },
        )
        .unwrap();
        assert_eq!(list_for_project(&conn, &pid).unwrap().len(), 1);

        conn.execute("DELETE FROM projects WHERE id = ?1", params![pid])
            .unwrap();
        assert_eq!(list_for_project(&conn, &pid).unwrap().len(), 0);
    }

    #[test]
    fn status_roundtrip() {
        for s in [
            MilestoneStatus::Upcoming,
            MilestoneStatus::InProgress,
            MilestoneStatus::Completed,
        ] {
            assert_eq!(MilestoneStatus::from_str(s.as_str()), Some(s));
        }
    }
}
