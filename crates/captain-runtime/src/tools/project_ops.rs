use crate::kernel_handle::KernelHandle;
use crate::tools::project_input::{
    normalize_project_tool_checkpoint_summary, normalize_project_tool_goal,
    normalize_project_tool_id, normalize_project_tool_milestone_name, normalize_project_tool_name,
    normalize_project_tool_optional_id, normalize_project_tool_slug,
    normalize_project_tool_task_description, normalize_project_tool_task_status,
    normalize_project_tool_task_title,
};
use crate::tools::{ensure_no_secret_literal, require_kernel};
use std::sync::Arc;

pub(crate) fn tool_project_create(
    input: &serde_json::Value,
    kernel: Option<&Arc<dyn KernelHandle>>,
) -> Result<String, String> {
    let kh = require_kernel(kernel)?;
    let name = normalize_project_tool_name(input["name"].as_str().ok_or("Missing 'name'")?)
        .map_err(str::to_string)?;
    let slug = normalize_project_tool_slug(input["slug"].as_str().ok_or("Missing 'slug'")?)
        .map_err(str::to_string)?;
    let goal = normalize_project_tool_goal(input["goal"].as_str().unwrap_or(""))
        .map_err(str::to_string)?;
    let deadline = input["deadline"].as_i64();
    let project = kh.project_create(&name, &slug, &goal, deadline)?;
    Ok(serde_json::to_string_pretty(&project).unwrap_or_else(|_| project.to_string()))
}

pub(crate) fn tool_project_list(
    input: &serde_json::Value,
    kernel: Option<&Arc<dyn KernelHandle>>,
) -> Result<String, String> {
    let kh = require_kernel(kernel)?;
    let include_archived = input["include_archived"].as_bool().unwrap_or(false);
    let query = input["query"]
        .as_str()
        .map(str::trim)
        .filter(|s| !s.is_empty());
    let list = kh.project_list(include_archived)?;
    let compact = compact_project_list_for_agent(&list, include_archived, query);
    Ok(serde_json::to_string_pretty(&compact).unwrap_or_else(|_| compact.to_string()))
}

/// Cap applied to the `timeline` (runtime events) section when
/// `include_events` is requested — projects that ran many orchestration
/// phases accumulate hundreds of timeline entries, each carrying a
/// sizeable `data` payload. Returning only the most recent ones keeps the
/// opt-in payload bounded; the total count is always included.
const PROJECT_GET_EVENTS_LIMIT: usize = 50;

/// `project_get` — the raw dump used to reach ~126k chars on loaded
/// projects (unbounded `metadata.runtime.timeline` / `worker_results`),
/// which the generic context compactor then mangled at random. Default
/// response is a compact, deterministic summary; heavy sections are
/// opt-in via `include_events` / `include_worker_results` / `include_tasks`.
pub(crate) fn tool_project_get(
    input: &serde_json::Value,
    kernel: Option<&Arc<dyn KernelHandle>>,
) -> Result<String, String> {
    let kh = require_kernel(kernel)?;
    let slug = normalize_project_tool_slug(input["slug"].as_str().ok_or("Missing 'slug'")?)
        .map_err(str::to_string)?;
    let project = match kh.project_find_by_slug(&slug)? {
        Some(p) => p,
        None => return Ok(format!("Project '{slug}' not found.")),
    };

    let include_events = input["include_events"].as_bool().unwrap_or(false);
    let include_worker_results = input["include_worker_results"].as_bool().unwrap_or(false);
    let include_tasks = input["include_tasks"].as_bool().unwrap_or(false);

    // project_resume() is the existing SSOT for "checkpoint + task graph"
    // — reuse it instead of re-deriving those queries here. Existence was
    // already confirmed above, so an error here is unexpected; degrade
    // gracefully (summary without checkpoint/tasks) rather than fail the
    // whole call.
    let resume = kh.project_resume(&slug).unwrap_or(serde_json::Value::Null);

    let summary = build_project_get_summary(
        &project,
        &resume,
        include_events,
        include_worker_results,
        include_tasks,
    );
    Ok(serde_json::to_string_pretty(&summary).unwrap_or_else(|_| summary.to_string()))
}

/// Builds the compact `project_get` payload from the raw project row (as
/// returned by `project_find_by_slug`, including its `metadata.runtime`
/// blob) and the `project_resume` result (checkpoint + task graph).
fn build_project_get_summary(
    project: &serde_json::Value,
    resume: &serde_json::Value,
    include_events: bool,
    include_worker_results: bool,
    include_tasks: bool,
) -> serde_json::Value {
    let null = serde_json::Value::Null;
    let metadata = project.get("metadata").unwrap_or(&null);
    let runtime = metadata.get("runtime").unwrap_or(&null);

    let events = runtime.get("timeline").and_then(|v| v.as_array());
    let events_total = events.map(|a| a.len()).unwrap_or(0);
    let mut events_section =
        serde_json::json!({ "count": events_total, "included": include_events });
    if include_events {
        let items = events
            .map(|a| {
                let start = a.len().saturating_sub(PROJECT_GET_EVENTS_LIMIT);
                a[start..].to_vec()
            })
            .unwrap_or_default();
        events_section["items"] = serde_json::json!(items);
    } else if events_total > 0 {
        events_section["hint"] = serde_json::json!(format!(
            "events: {events_total} (omitted, pass include_events:true to fetch the last {PROJECT_GET_EVENTS_LIMIT})"
        ));
    }

    let worker_results = runtime.get("worker_results").and_then(|v| v.as_object());
    let worker_results_total = worker_results.map(|o| o.len()).unwrap_or(0);
    let mut worker_results_section =
        serde_json::json!({ "count": worker_results_total, "included": include_worker_results });
    if include_worker_results {
        worker_results_section["items"] = runtime
            .get("worker_results")
            .cloned()
            .unwrap_or_else(|| serde_json::json!({}));
    } else if worker_results_total > 0 {
        worker_results_section["hint"] = serde_json::json!(format!(
            "worker_results: {worker_results_total} phase(s) (omitted, pass include_worker_results:true)"
        ));
    }

    let all_tasks: Vec<serde_json::Value> = resume
        .get("tasks")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();
    let open_tasks: Vec<&serde_json::Value> = all_tasks
        .iter()
        .filter(|t| {
            !matches!(
                t.get("status").and_then(|s| s.as_str()),
                Some("done") | Some("cancelled")
            )
        })
        .collect();
    let next_actions: Vec<serde_json::Value> = open_tasks
        .iter()
        .take(10)
        .map(|t| {
            serde_json::json!({
                "id": t.get("id").cloned().unwrap_or(serde_json::Value::Null),
                "title": t.get("title").cloned().unwrap_or(serde_json::Value::Null),
                "status": t.get("status").cloned().unwrap_or(serde_json::Value::Null),
            })
        })
        .collect();

    let mut tasks_section = serde_json::json!({
        "count": all_tasks.len(),
        "open_count": open_tasks.len(),
        "included": include_tasks,
    });
    if include_tasks {
        tasks_section["items"] = serde_json::json!(all_tasks);
    } else if !all_tasks.is_empty() {
        tasks_section["hint"] = serde_json::json!(format!(
            "tasks: {} (omitted, pass include_tasks:true for the full list)",
            all_tasks.len()
        ));
    }

    // Latest checkpoint: summary only — never the raw `state` blob.
    let checkpoint = resume
        .get("checkpoint")
        .filter(|c| !c.is_null())
        .map(|c| {
            serde_json::json!({
                "summary": c.get("summary").cloned().unwrap_or(serde_json::Value::Null),
                "created_at": c.get("created_at").cloned().unwrap_or(serde_json::Value::Null),
                "session_id": c.get("session_id").cloned().unwrap_or(serde_json::Value::Null),
            })
        })
        .unwrap_or(serde_json::Value::Null);

    serde_json::json!({
        "id": project.get("id").cloned().unwrap_or(serde_json::Value::Null),
        "name": project.get("name").cloned().unwrap_or(serde_json::Value::Null),
        "slug": project.get("slug").cloned().unwrap_or(serde_json::Value::Null),
        "goal": project.get("goal").cloned().unwrap_or(serde_json::Value::Null),
        "status": project.get("status").cloned().unwrap_or(serde_json::Value::Null),
        "deadline": project.get("deadline").cloned().unwrap_or(serde_json::Value::Null),
        "created_at": project.get("created_at").cloned().unwrap_or(serde_json::Value::Null),
        "updated_at": project.get("updated_at").cloned().unwrap_or(serde_json::Value::Null),
        "runtime": {
            "status": runtime.get("status").cloned().unwrap_or(serde_json::Value::Null),
            "current_phase": runtime.get("current_phase").cloned().unwrap_or(serde_json::Value::Null),
            "progress": runtime.get("progress").cloned().unwrap_or(serde_json::Value::Null),
            "updated_at": runtime.get("updated_at").cloned().unwrap_or(serde_json::Value::Null),
        },
        "next_actions": next_actions,
        "checkpoint": checkpoint,
        "sections": {
            "events": events_section,
            "worker_results": worker_results_section,
            "tasks": tasks_section,
        },
    })
}

pub(crate) fn tool_project_archive(
    input: &serde_json::Value,
    kernel: Option<&Arc<dyn KernelHandle>>,
) -> Result<String, String> {
    let kh = require_kernel(kernel)?;
    let id = normalize_project_tool_id(input["id"].as_str().ok_or("Missing 'id'")?)
        .map_err(str::to_string)?;
    match kh.project_archive(&id)? {
        Some(p) => Ok(format!(
            "Project archived: {}",
            p.get("slug").and_then(|v| v.as_str()).unwrap_or(&id)
        )),
        None => Ok("Project not found.".to_string()),
    }
}

pub(crate) fn tool_project_delete(
    input: &serde_json::Value,
    kernel: Option<&Arc<dyn KernelHandle>>,
) -> Result<String, String> {
    let kh = require_kernel(kernel)?;
    let id = normalize_project_tool_id(input["id"].as_str().ok_or("Missing 'id'")?)
        .map_err(str::to_string)?;
    if kh.project_delete(&id)? {
        Ok(format!("Project deleted: {id}"))
    } else {
        Ok("Project not found.".to_string())
    }
}

pub(crate) fn tool_project_resume(
    input: &serde_json::Value,
    kernel: Option<&Arc<dyn KernelHandle>>,
) -> Result<String, String> {
    let kh = require_kernel(kernel)?;
    let slug = normalize_project_tool_slug(input["slug"].as_str().ok_or("Missing 'slug'")?)
        .map_err(str::to_string)?;
    let state = kh.project_resume(&slug)?;
    Ok(serde_json::to_string_pretty(&state).unwrap_or_else(|_| state.to_string()))
}

pub(crate) fn tool_project_task_create(
    input: &serde_json::Value,
    kernel: Option<&Arc<dyn KernelHandle>>,
) -> Result<String, String> {
    let kh = require_kernel(kernel)?;
    let project_id =
        normalize_project_tool_id(input["project_id"].as_str().ok_or("Missing 'project_id'")?)
            .map_err(str::to_string)?;
    let title =
        normalize_project_tool_task_title(input["title"].as_str().ok_or("Missing 'title'")?)
            .map_err(str::to_string)?;
    let description =
        normalize_project_tool_task_description(input["description"].as_str().unwrap_or(""))
            .map_err(str::to_string)?;
    let parent_id =
        normalize_project_tool_optional_id(input["parent_id"].as_str()).map_err(str::to_string)?;
    let task = kh.project_task_create(&project_id, &title, &description, parent_id.as_deref())?;
    Ok(serde_json::to_string_pretty(&task).unwrap_or_else(|_| task.to_string()))
}

pub(crate) fn tool_project_task_list(
    input: &serde_json::Value,
    kernel: Option<&Arc<dyn KernelHandle>>,
) -> Result<String, String> {
    let kh = require_kernel(kernel)?;
    let project_id =
        normalize_project_tool_id(input["project_id"].as_str().ok_or("Missing 'project_id'")?)
            .map_err(str::to_string)?;
    let rows = kh.project_task_list(&project_id)?;
    Ok(serde_json::to_string_pretty(&rows).unwrap_or_else(|_| rows.to_string()))
}

pub(crate) fn tool_project_task_update(
    input: &serde_json::Value,
    kernel: Option<&Arc<dyn KernelHandle>>,
) -> Result<String, String> {
    let kh = require_kernel(kernel)?;
    let id = normalize_project_tool_id(input["id"].as_str().ok_or("Missing 'id'")?)
        .map_err(str::to_string)?;
    let status =
        normalize_project_tool_task_status(input["status"].as_str().ok_or("Missing 'status'")?)
            .map_err(str::to_string)?;
    match kh.project_task_update_status(&id, &status)? {
        Some(t) => Ok(serde_json::to_string_pretty(&t).unwrap_or_else(|_| t.to_string())),
        None => Ok("Task not found.".to_string()),
    }
}

pub(crate) fn tool_milestone_create(
    input: &serde_json::Value,
    kernel: Option<&Arc<dyn KernelHandle>>,
) -> Result<String, String> {
    let kh = require_kernel(kernel)?;
    let project_id =
        normalize_project_tool_id(input["project_id"].as_str().ok_or("Missing 'project_id'")?)
            .map_err(str::to_string)?;
    let name =
        normalize_project_tool_milestone_name(input["name"].as_str().ok_or("Missing 'name'")?)
            .map_err(str::to_string)?;
    let due_date = input["due_date"].as_i64();
    let m = kh.milestone_create(&project_id, &name, due_date)?;
    Ok(serde_json::to_string_pretty(&m).unwrap_or_else(|_| m.to_string()))
}

pub(crate) fn tool_milestone_list(
    input: &serde_json::Value,
    kernel: Option<&Arc<dyn KernelHandle>>,
) -> Result<String, String> {
    let kh = require_kernel(kernel)?;
    let project_id =
        normalize_project_tool_id(input["project_id"].as_str().ok_or("Missing 'project_id'")?)
            .map_err(str::to_string)?;
    let rows = kh.milestone_list(&project_id)?;
    Ok(serde_json::to_string_pretty(&rows).unwrap_or_else(|_| rows.to_string()))
}

pub(crate) fn tool_milestone_complete(
    input: &serde_json::Value,
    kernel: Option<&Arc<dyn KernelHandle>>,
) -> Result<String, String> {
    let kh = require_kernel(kernel)?;
    let id = normalize_project_tool_id(input["id"].as_str().ok_or("Missing 'id'")?)
        .map_err(str::to_string)?;
    match kh.milestone_complete(&id)? {
        Some(m) => Ok(serde_json::to_string_pretty(&m).unwrap_or_else(|_| m.to_string())),
        None => Ok("Milestone not found.".to_string()),
    }
}

pub(crate) fn tool_milestone_progress(
    input: &serde_json::Value,
    kernel: Option<&Arc<dyn KernelHandle>>,
) -> Result<String, String> {
    let kh = require_kernel(kernel)?;
    let project_id =
        normalize_project_tool_id(input["project_id"].as_str().ok_or("Missing 'project_id'")?)
            .map_err(str::to_string)?;
    let p = kh.milestone_progress(&project_id)?;
    Ok(serde_json::to_string_pretty(&p).unwrap_or_else(|_| p.to_string()))
}

pub(crate) fn tool_checkpoint_save(
    input: &serde_json::Value,
    kernel: Option<&Arc<dyn KernelHandle>>,
    caller_agent_id: Option<&str>,
) -> Result<String, String> {
    let kh = require_kernel(kernel)?;
    let project_id =
        normalize_project_tool_id(input["project_id"].as_str().ok_or("Missing 'project_id'")?)
            .map_err(str::to_string)?;
    let summary = normalize_project_tool_checkpoint_summary(
        input["summary"].as_str().ok_or("Missing 'summary'")?,
    )
    .map_err(str::to_string)?;
    ensure_no_secret_literal("checkpoint_save", "summary", &summary)?;
    let state = input
        .get("state")
        .cloned()
        .unwrap_or(serde_json::Value::Null);
    ensure_no_secret_literal("checkpoint_save", "state", &state.to_string())?;
    let cp = kh.checkpoint_save(&project_id, &summary, state, caller_agent_id)?;
    Ok(serde_json::to_string_pretty(&cp).unwrap_or_else(|_| cp.to_string()))
}

fn compact_project_list_for_agent(
    list: &serde_json::Value,
    include_archived: bool,
    query: Option<&str>,
) -> serde_json::Value {
    let query_terms = project_query_terms(query);
    let rows = list
        .as_array()
        .into_iter()
        .flatten()
        .filter(|project| project_matches_query(project, &query_terms))
        .take(25)
        .map(compact_project_for_agent)
        .collect::<Vec<_>>();

    serde_json::json!({
        "count": rows.len(),
        "include_archived": include_archived,
        "query": query,
        "projects": rows,
        "operator_hint": "Match user references against project slug/name before interpreting a number as a menu choice; e.g. 'projet1' can mean slug/name projet1, not option 1."
    })
}

fn compact_project_for_agent(project: &serde_json::Value) -> serde_json::Value {
    let metadata = project.get("metadata").unwrap_or(&serde_json::Value::Null);
    let runtime = metadata.get("runtime").unwrap_or(&serde_json::Value::Null);
    let workers = compact_project_workers(runtime.get("workers"));
    let runtime_status = runtime
        .get("status")
        .and_then(serde_json::Value::as_str)
        .unwrap_or("ready");
    let runtime_phase = runtime
        .get("current_phase")
        .and_then(serde_json::Value::as_str)
        .unwrap_or("observe");
    let slug = project
        .get("slug")
        .and_then(serde_json::Value::as_str)
        .unwrap_or("");

    serde_json::json!({
        "id": project.get("id").and_then(serde_json::Value::as_str).unwrap_or(""),
        "slug": slug,
        "name": project.get("name").and_then(serde_json::Value::as_str).unwrap_or(""),
        "goal": bounded_project_text(project.get("goal").and_then(serde_json::Value::as_str).unwrap_or(""), 240),
        "status": project.get("status").and_then(serde_json::Value::as_str).unwrap_or("planning"),
        "updated_at": project.get("updated_at").and_then(serde_json::Value::as_i64).unwrap_or(0),
        "runtime": {
            "status": runtime_status,
            "phase": runtime_phase,
            "progress": runtime.get("progress").and_then(serde_json::Value::as_u64).unwrap_or(0).min(100),
            "workers": workers
        },
        "next_actions": project_next_actions(slug, runtime_status)
    })
}

fn compact_project_workers(workers: Option<&serde_json::Value>) -> serde_json::Value {
    let mut by_status = std::collections::BTreeMap::<String, usize>::new();
    let total = workers
        .and_then(serde_json::Value::as_array)
        .map(|items| {
            for worker in items {
                let status = worker
                    .get("status")
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or("planned")
                    .to_string();
                *by_status.entry(status).or_default() += 1;
            }
            items.len()
        })
        .unwrap_or(0);
    serde_json::json!({
        "total": total,
        "by_status": by_status
    })
}

fn project_next_actions(slug: &str, runtime_status: &str) -> Vec<String> {
    let mut actions = vec![format!("project_get {{\"slug\":\"{slug}\"}}")];
    match runtime_status {
        "paused" | "blocked" | "failed" => {
            actions.push(format!("project_resume {{\"slug\":\"{slug}\"}}"));
        }
        "ready" => {
            actions.push(format!("captain project start {slug}"));
        }
        "running" => {
            actions.push(format!("captain project status {slug}"));
        }
        _ => {}
    }
    actions
}

fn project_query_terms(query: Option<&str>) -> Vec<String> {
    query
        .unwrap_or("")
        .split(|ch: char| !ch.is_alphanumeric())
        .filter(|term| term.len() >= 2)
        .map(|term| term.to_lowercase())
        .collect()
}

fn project_matches_query(project: &serde_json::Value, terms: &[String]) -> bool {
    if terms.is_empty() {
        return true;
    }
    let haystack = [
        project
            .get("slug")
            .and_then(serde_json::Value::as_str)
            .unwrap_or(""),
        project
            .get("name")
            .and_then(serde_json::Value::as_str)
            .unwrap_or(""),
        project
            .get("goal")
            .and_then(serde_json::Value::as_str)
            .unwrap_or(""),
    ]
    .join(" ")
    .to_lowercase();
    terms.iter().all(|term| haystack.contains(term))
}

fn bounded_project_text(text: &str, limit: usize) -> String {
    let cleaned = text.trim();
    if cleaned.chars().count() <= limit {
        return cleaned.to_string();
    }
    let keep = limit.saturating_sub(3);
    let mut out = cleaned.chars().take(keep).collect::<String>();
    out.push_str("...");
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// Fixture: a loaded project whose `metadata.runtime.timeline` carries
    /// `event_count` sizeable events — reproduces the ~126k char raw dump.
    fn loaded_project_fixture(event_count: usize) -> (serde_json::Value, serde_json::Value) {
        let padding = "x".repeat(500);
        let timeline: Vec<serde_json::Value> = (0..event_count)
            .map(|i| {
                json!({
                    "id": format!("evt-{i}"),
                    "kind": "worker.completed",
                    "detail": padding,
                    "data": { "run_id": "run-1" },
                })
            })
            .collect();

        let project = json!({
            "id": "proj-1",
            "name": "Loaded Project",
            "slug": "loaded-project",
            "goal": "Ship the thing",
            "status": "active",
            "metadata": {
                "runtime": {
                    "status": "running",
                    "current_phase": "build",
                    "progress": 42,
                    "timeline": timeline,
                    "worker_results": {
                        "observe": { "summary": padding, "status": "done" },
                        "build": { "summary": padding, "status": "running" },
                    },
                }
            },
        });
        let resume = json!({
            "checkpoint": {
                "summary": "Made good progress on the build phase.",
                "state": { "open_tasks": ["t1"] },
                "created_at": 1_700_000_050_000i64,
                "session_id": "sess-1",
            },
            "tasks": [
                { "id": "t1", "title": "Wire up the pipeline", "status": "doing" },
                { "id": "t2", "title": "Old task", "status": "done" },
            ],
        });
        (project, resume)
    }

    #[test]
    fn project_get_summary_is_compact_and_counts_heavy_sections() {
        let (project, resume) = loaded_project_fixture(240);

        let summary = build_project_get_summary(&project, &resume, false, false, false);
        let rendered = serde_json::to_string_pretty(&summary).unwrap();

        assert!(
            rendered.len() < 8_000,
            "default summary must stay compact, got {} chars",
            rendered.len()
        );
        assert_eq!(summary["slug"], "loaded-project");
        assert_eq!(summary["runtime"]["current_phase"], "build");
        assert_eq!(summary["sections"]["events"]["count"], 240);
        assert!(summary["sections"]["events"].get("items").is_none());
        assert!(summary["sections"]["worker_results"].get("items").is_none());
        // Raw checkpoint `state` blob must never leak into the summary.
        assert!(summary["checkpoint"].get("state").is_none());
        // next_actions = open tasks only (t2 is done, excluded).
        let next_actions = summary["next_actions"].as_array().unwrap();
        assert_eq!(next_actions.len(), 1);
        assert_eq!(next_actions[0]["id"], "t1");
    }

    #[test]
    fn project_get_include_events_caps_at_the_last_50() {
        let (project, resume) = loaded_project_fixture(240);

        let summary = build_project_get_summary(&project, &resume, true, false, false);

        assert_eq!(summary["sections"]["events"]["count"], 240);
        let items = summary["sections"]["events"]["items"].as_array().unwrap();
        assert_eq!(items.len(), 50);
        assert_eq!(items.last().unwrap()["id"], "evt-239");
    }

    #[test]
    fn project_get_opt_in_sections_reintegrate_content() {
        let (project, resume) = loaded_project_fixture(5);

        let summary = build_project_get_summary(&project, &resume, false, true, true);

        assert!(
            summary["sections"]["worker_results"]["items"]["build"]["summary"]
                .as_str()
                .unwrap()
                .len()
                > 100
        );
        assert_eq!(summary["sections"]["tasks"]["count"], 2);
        assert_eq!(summary["sections"]["tasks"]["open_count"], 1);
        assert_eq!(
            summary["sections"]["tasks"]["items"]
                .as_array()
                .unwrap()
                .len(),
            2
        );
    }

    #[test]
    fn compact_project_list_keeps_identity_and_runtime_state() {
        let list = json!([
            {
                "id": "cab968e7-d3c8-47cb-9420-b7dd8b4c050f",
                "name": "Projet1 — Gestion documents couple",
                "slug": "projet1-documents-couple",
                "goal": "Développer une petite application locale de gestion de documents personnels pour un couple.",
                "status": "planning",
                "updated_at": 1782855873736i64,
                "metadata": {
                    "runtime": {
                        "status": "ready",
                        "current_phase": "observe",
                        "progress": 10,
                        "workers": [
                            {"phase": "observe", "status": "ready", "prompt": "private worker prompt"},
                            {"phase": "think", "status": "planned", "prompt": "private worker prompt"},
                            {"phase": "plan", "status": "planned", "prompt": "private worker prompt"}
                        ],
                        "timeline": [
                            {"kind": "project.ready", "data": {"raw": "large-private-payload"}}
                        ]
                    }
                }
            }
        ]);

        let compact = compact_project_list_for_agent(&list, false, None);
        let project = &compact["projects"][0];

        assert_eq!(compact["count"], json!(1));
        assert_eq!(project["slug"], "projet1-documents-couple");
        assert_eq!(project["runtime"]["status"], "ready");
        assert_eq!(project["runtime"]["phase"], "observe");
        assert_eq!(project["runtime"]["progress"], 10);
        assert_eq!(project["runtime"]["workers"]["total"], 3);
        assert_eq!(project["runtime"]["workers"]["by_status"]["planned"], 2);
        assert_eq!(project["runtime"]["workers"]["by_status"]["ready"], 1);
        assert!(project["next_actions"]
            .as_array()
            .unwrap()
            .iter()
            .any(|action| action == "captain project start projet1-documents-couple"));
        assert!(project.get("metadata").is_none());
        assert!(!compact.to_string().contains("large-private-payload"));
        assert!(!compact.to_string().contains("private worker prompt"));
    }

    #[test]
    fn compact_project_list_query_matches_project_alias_before_menu_choice() {
        let list = json!([
            {
                "id": "old",
                "name": "Release smoke calculator",
                "slug": "release-smoke-calculator",
                "goal": "Créer une calculatrice CLI minimale.",
                "status": "done",
                "updated_at": 1,
                "metadata": {}
            },
            {
                "id": "doc",
                "name": "Projet1 — Gestion documents couple",
                "slug": "projet1-documents-couple",
                "goal": "Application de gestion documentaire du couple.",
                "status": "planning",
                "updated_at": 2,
                "metadata": {
                    "runtime": {"status": "ready", "current_phase": "observe", "progress": 10}
                }
            }
        ]);

        let by_slug_alias = compact_project_list_for_agent(&list, false, Some("projet1"));
        assert_eq!(by_slug_alias["count"], json!(1));
        assert_eq!(
            by_slug_alias["projects"][0]["slug"],
            "projet1-documents-couple"
        );

        let by_semantic_terms = compact_project_list_for_agent(&list, false, Some("doc couple"));
        assert_eq!(by_semantic_terms["count"], json!(1));
        assert_eq!(
            by_semantic_terms["projects"][0]["slug"],
            "projet1-documents-couple"
        );
        assert!(by_slug_alias["operator_hint"]
            .as_str()
            .unwrap()
            .contains("not option 1"));
    }
}
