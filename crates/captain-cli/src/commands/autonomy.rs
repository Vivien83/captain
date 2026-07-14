use super::log_events::{
    format_unix_ms_utc, parse_log_since_ms, parse_utc_timestamp_ms, read_session_events,
    summarize_log_payload, CliLogEvent,
};
use crate::{daemon_client, daemon_json, require_daemon, truncate_display, ui};

struct AutonomySnapshots {
    status: serde_json::Value,
    agents: serde_json::Value,
    cron: serde_json::Value,
    triggers: serde_json::Value,
    workflows: serde_json::Value,
    approvals: serde_json::Value,
}

struct AutonomyRows {
    agents: Vec<serde_json::Value>,
    jobs: Vec<serde_json::Value>,
    triggers: Vec<serde_json::Value>,
    workflows: Vec<serde_json::Value>,
    approvals: Vec<serde_json::Value>,
}

pub(crate) fn cmd_autonomy_status(json: bool, lines: usize, since: Option<&str>) {
    let since_ms = parse_autonomy_since_or_exit(since);
    let lines = lines.max(1);
    let snapshots = fetch_autonomy_snapshots();
    let output = build_autonomy_status_output(&snapshots, since_ms, lines);

    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(&output).unwrap_or_default()
        );
    } else {
        print_autonomy_status(&output);
    }
}

fn parse_autonomy_since_or_exit(since: Option<&str>) -> Option<i64> {
    match since.map(parse_log_since_ms).transpose() {
        Ok(value) => value,
        Err(e) => {
            ui::error(&e);
            ui::hint("Use a duration like 1h, 24h, 7d or a UTC timestamp.");
            std::process::exit(2);
        }
    }
}

fn fetch_autonomy_snapshots() -> AutonomySnapshots {
    let base = require_daemon("autonomy status");
    let client = daemon_client();
    AutonomySnapshots {
        status: daemon_json(client.get(format!("{base}/api/status")).send()),
        agents: daemon_json(client.get(format!("{base}/api/agents")).send()),
        cron: daemon_json(client.get(format!("{base}/api/cron/jobs")).send()),
        triggers: daemon_json(client.get(format!("{base}/api/triggers")).send()),
        workflows: daemon_json(client.get(format!("{base}/api/workflows")).send()),
        approvals: daemon_json(client.get(format!("{base}/api/approvals")).send()),
    }
}

fn build_autonomy_status_output(
    snapshots: &AutonomySnapshots,
    since_ms: Option<i64>,
    lines: usize,
) -> serde_json::Value {
    let events = read_session_events(None, since_ms, lines.saturating_mul(80).clamp(100, 5_000))
        .unwrap_or_default();
    build_autonomy_status_output_from_events(snapshots, &events, lines)
}

fn build_autonomy_status_output_from_events(
    snapshots: &AutonomySnapshots,
    events: &[CliLogEvent],
    lines: usize,
) -> serde_json::Value {
    let rows = collect_autonomy_rows(snapshots);
    let job_issues = autonomy_job_issues(&rows.jobs);
    let next_jobs = next_enabled_jobs(&rows.jobs, 5);
    let (recent_actions, recent_errors) = split_recent_autonomy_events(events, lines);

    serde_json::json!({
        "daemon": {
            "status": snapshots.status["status"].as_str().unwrap_or("?"),
            "uptime_seconds": snapshots.status["uptime_seconds"].as_u64().unwrap_or(0),
            "provider": snapshots.status["default_provider"].as_str().unwrap_or("?"),
            "model": snapshots.status["default_model"].as_str().unwrap_or("?"),
            "channels_configured": snapshots.status["channel_configured_count"].as_u64().unwrap_or(0),
        },
        "agents": {
            "running": count_running_agents(&rows.agents),
            "total": rows.agents.len(),
            "items": rows.agents,
        },
        "cron": {
            "enabled": count_enabled(&rows.jobs),
            "total": rows.jobs.len(),
            "next": next_jobs,
            "issues": job_issues,
        },
        "triggers": {
            "enabled": count_enabled(&rows.triggers),
            "total": rows.triggers.len(),
            "items": rows.triggers,
        },
        "workflows": {
            "total": rows.workflows.len(),
            "items": rows.workflows,
        },
        "approvals": {
            "pending": rows.approvals.len(),
            "items": rows.approvals,
        },
        "recent_actions": recent_actions,
        "recent_errors": recent_errors,
    })
}

fn collect_autonomy_rows(snapshots: &AutonomySnapshots) -> AutonomyRows {
    AutonomyRows {
        agents: value_array(&snapshots.agents, "agents"),
        jobs: value_array(&snapshots.cron, "jobs"),
        triggers: value_array(&snapshots.triggers, "triggers"),
        workflows: value_array(&snapshots.workflows, "workflows"),
        approvals: value_array(&snapshots.approvals, "approvals"),
    }
}

fn count_enabled(rows: &[serde_json::Value]) -> usize {
    rows.iter()
        .filter(|row| row["enabled"].as_bool().unwrap_or(false))
        .count()
}

fn count_running_agents(agent_rows: &[serde_json::Value]) -> usize {
    agent_rows
        .iter()
        .filter(|agent| agent["state"].as_str() == Some("Running"))
        .count()
}

fn autonomy_job_issues(job_rows: &[serde_json::Value]) -> Vec<serde_json::Value> {
    job_rows
        .iter()
        .filter(|job| {
            job["consecutive_errors"].as_u64().unwrap_or(0) > 0
                || job["last_status"].as_str() == Some("error")
        })
        .cloned()
        .collect()
}

fn split_recent_autonomy_events(
    events: &[CliLogEvent],
    lines: usize,
) -> (Vec<serde_json::Value>, Vec<serde_json::Value>) {
    let recent_errors = recent_event_json(
        events.iter().filter(|event| autonomy_event_is_error(event)),
        lines,
    );
    let recent_actions = recent_event_json(
        events
            .iter()
            .filter(|event| !autonomy_event_is_error(event)),
        lines,
    );
    (recent_actions, recent_errors)
}

fn print_autonomy_status(output: &serde_json::Value) {
    ui::section("Autonomy Status");
    print_autonomy_summary(output);

    ui::blank();
    ui::section("Next Jobs");
    print_autonomy_next_jobs(&output["cron"]["next"]);

    if output["cron"]["issues"]
        .as_array()
        .is_some_and(|items| !items.is_empty())
    {
        ui::blank();
        ui::section("Job Issues");
        print_autonomy_job_issues(&output["cron"]["issues"]);
    }

    print_autonomy_recent_sections(output);
}

fn print_autonomy_summary(output: &serde_json::Value) {
    ui::kv("Daemon", output["daemon"]["status"].as_str().unwrap_or("?"));
    ui::kv(
        "Model",
        &format!(
            "{}/{}",
            output["daemon"]["provider"].as_str().unwrap_or("?"),
            output["daemon"]["model"].as_str().unwrap_or("?")
        ),
    );
    ui::kv(
        "Agents",
        &format!(
            "{}/{} running",
            output["agents"]["running"].as_u64().unwrap_or(0),
            output["agents"]["total"].as_u64().unwrap_or(0)
        ),
    );
    ui::kv(
        "Channels",
        &format!(
            "{} configured",
            output["daemon"]["channels_configured"]
                .as_u64()
                .unwrap_or(0)
        ),
    );
    ui::kv(
        "Cron jobs",
        &format!(
            "{}/{} enabled",
            output["cron"]["enabled"].as_u64().unwrap_or(0),
            output["cron"]["total"].as_u64().unwrap_or(0)
        ),
    );
    ui::kv(
        "Triggers",
        &format!(
            "{}/{} enabled",
            output["triggers"]["enabled"].as_u64().unwrap_or(0),
            output["triggers"]["total"].as_u64().unwrap_or(0)
        ),
    );
    ui::kv(
        "Workflows",
        &output["workflows"]["total"]
            .as_u64()
            .unwrap_or(0)
            .to_string(),
    );
    ui::kv(
        "Approvals",
        &format!(
            "{} pending",
            output["approvals"]["pending"].as_u64().unwrap_or(0)
        ),
    );
}

fn print_autonomy_recent_sections(output: &serde_json::Value) {
    ui::blank();
    ui::section("Recent Actions");
    print_autonomy_events(&output["recent_actions"]);

    if output["recent_errors"]
        .as_array()
        .is_some_and(|items| !items.is_empty())
    {
        ui::blank();
        ui::section("Recent Errors");
        print_autonomy_events(&output["recent_errors"]);
    }
}

fn print_autonomy_next_jobs(next_jobs: &serde_json::Value) {
    if let Some(items) = next_jobs.as_array().filter(|items| !items.is_empty()) {
        for job in items {
            println!(
                "  {:<24} {:<22} {}",
                truncate_display(job["name"].as_str().unwrap_or("?"), 24),
                job["next_run"].as_str().unwrap_or("?"),
                job["last_status"].as_str().unwrap_or("-")
            );
        }
    } else {
        println!("  No enabled jobs with next_run.");
    }
}

fn print_autonomy_job_issues(job_issues: &serde_json::Value) {
    let Some(items) = job_issues.as_array() else {
        return;
    };
    for job in items {
        println!(
            "  {:<24} errors={} last={}",
            truncate_display(job["name"].as_str().unwrap_or("?"), 24),
            job["consecutive_errors"].as_u64().unwrap_or(0),
            job["last_status"].as_str().unwrap_or("?")
        );
    }
}

fn value_array(value: &serde_json::Value, key: &str) -> Vec<serde_json::Value> {
    value
        .as_array()
        .or_else(|| value.get(key).and_then(|v| v.as_array()))
        .cloned()
        .unwrap_or_default()
}

fn next_enabled_jobs(jobs: &[serde_json::Value], limit: usize) -> Vec<serde_json::Value> {
    let mut rows: Vec<serde_json::Value> = jobs
        .iter()
        .filter(|job| job["enabled"].as_bool().unwrap_or(false))
        .filter(|job| job["next_run"].as_str().is_some_and(|s| !s.is_empty()))
        .cloned()
        .collect();
    rows.sort_by_key(|job| {
        parse_utc_timestamp_ms(job["next_run"].as_str().unwrap_or("?")).unwrap_or(i64::MAX)
    });
    rows.truncate(limit.max(1));
    rows
}

fn recent_event_json<'a>(
    events: impl Iterator<Item = &'a CliLogEvent>,
    limit: usize,
) -> Vec<serde_json::Value> {
    let mut rows: Vec<serde_json::Value> = events
        .map(|event| {
            serde_json::json!({
                "id": event.id,
                "timestamp": format_unix_ms_utc(event.ts),
                "session_id": event.session_id,
                "event_type": event.event_type,
                "summary": summarize_log_payload(event),
            })
        })
        .collect();
    let start = rows.len().saturating_sub(limit.max(1));
    rows.split_off(start)
}

fn autonomy_event_is_error(event: &CliLogEvent) -> bool {
    let event_type = event.event_type.to_ascii_lowercase();
    let payload = event.payload.to_string().to_ascii_lowercase();
    event
        .payload
        .get("is_error")
        .and_then(|v| v.as_bool())
        .unwrap_or(false)
        || event
            .payload
            .get("status")
            .and_then(|v| v.as_str())
            .is_some_and(|status| status.eq_ignore_ascii_case("error"))
        || event_type.contains("error")
        || payload.contains("\"is_error\":true")
        || payload.contains("\"status\":\"error\"")
}

fn print_autonomy_events(events: &serde_json::Value) {
    let Some(items) = events.as_array() else {
        println!("  No recent events.");
        return;
    };
    if items.is_empty() {
        println!("  No recent events.");
        return;
    }
    for event in items {
        println!(
            "  {} {:<22} {}",
            event["timestamp"].as_str().unwrap_or("?"),
            event["event_type"].as_str().unwrap_or("?"),
            truncate_display(event["summary"].as_str().unwrap_or(""), 100)
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn event(id: i64, event_type: &str, payload: serde_json::Value) -> CliLogEvent {
        CliLogEvent {
            id,
            session_id: "session-1".to_string(),
            ts: id * 1000,
            event_type: event_type.to_string(),
            payload,
        }
    }

    #[test]
    fn autonomy_status_output_counts_resources_and_splits_recent_events() {
        let snapshots = AutonomySnapshots {
            status: json!({
                "status": "running",
                "uptime_seconds": 12,
                "default_provider": "codex",
                "default_model": "gpt-5.5",
                "channel_configured_count": 2
            }),
            agents: json!({
                "agents": [
                    {"name": "captain", "state": "Running"},
                    {"name": "worker", "state": "Stopped"}
                ]
            }),
            cron: json!({
                "jobs": [
                    {
                        "name": "later",
                        "enabled": true,
                        "next_run": "2026-06-20T11:00:00Z",
                        "last_status": "ok",
                        "consecutive_errors": 0
                    },
                    {
                        "name": "broken",
                        "enabled": true,
                        "next_run": "2026-06-20T10:00:00Z",
                        "last_status": "error",
                        "consecutive_errors": 2
                    },
                    {"name": "disabled", "enabled": false}
                ]
            }),
            triggers: json!({"triggers": [{"enabled": true}, {"enabled": false}]}),
            workflows: json!({"workflows": [{"id": "wf-1"}]}),
            approvals: json!({"approvals": [{"id": "approval-1"}]}),
        };
        let events = vec![
            event(1, "phase_change", json!({"phase": "thinking"})),
            event(2, "tool_execution_result", json!({"status": "error"})),
        ];

        let output = build_autonomy_status_output_from_events(&snapshots, &events, 10);

        assert_eq!(output["agents"]["running"], 1);
        assert_eq!(output["agents"]["total"], 2);
        assert_eq!(output["cron"]["enabled"], 2);
        assert_eq!(output["triggers"]["enabled"], 1);
        assert_eq!(output["cron"]["next"][0]["name"], "broken");
        assert_eq!(output["cron"]["issues"][0]["name"], "broken");
        assert_eq!(output["recent_actions"].as_array().unwrap().len(), 1);
        assert_eq!(output["recent_errors"].as_array().unwrap().len(), 1);
    }
}
