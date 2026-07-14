use super::status_project_attention::{
    print_project_attention_rows, project_attention_from_metadata,
};
use crate::{truncate_display, ui};

pub(super) fn print_status_workload(body: &serde_json::Value, verbose: bool) {
    let workload = &body["workload"];
    if workload.is_null() {
        return;
    }

    let project_total = status_json_u64(workload, &["projects", "total"]);
    let project_active = status_json_u64(workload, &["projects", "active"]);
    let project_planning = status_json_u64(workload, &["projects", "planning"]);
    let project_paused = status_json_u64(workload, &["projects", "paused"]);
    let goal_total = status_json_u64(workload, &["goals", "total"]);
    let goal_active = status_json_u64(workload, &["goals", "active"]);
    let goal_escalated = status_json_u64(workload, &["goals", "escalated"]);
    let active_runs = body["active_run_count"].as_u64().unwrap_or(0);

    ui::blank();
    ui::section("Workload");
    ui::kv("Active runs", &active_runs.to_string());
    ui::kv(
        "Projects",
        &format!(
            "{project_total} total ({project_active} active, {project_planning} planning, {project_paused} paused)"
        ),
    );
    let goals_summary = if goal_escalated > 0 {
        format!("{goal_active}/{goal_total} active, {goal_escalated} escalated")
    } else {
        format!("{goal_active}/{goal_total} active")
    };
    ui::kv("Goals", &goals_summary);
    print_project_attention_rows(workload, verbose);
    print_active_run_rows(body, verbose);
    print_active_process_rows(body, verbose);
    print_channel_queue_rows(body, verbose);

    let cron_total = status_json_u64(workload, &["automation", "cron_jobs"]);
    let cron_enabled = status_json_u64(workload, &["automation", "cron_enabled"]);
    let cron_due = status_json_u64(workload, &["automation", "cron_due"]);
    let trigger_total = status_json_u64(workload, &["automation", "triggers"]);
    let trigger_enabled = status_json_u64(workload, &["automation", "triggers_enabled"]);
    let file_trigger_total = status_json_u64(workload, &["automation", "file_triggers"]);
    let file_trigger_enabled = status_json_u64(workload, &["automation", "file_triggers_enabled"]);

    ui::blank();
    ui::section("Automation");
    let cron_summary = if cron_due > 0 {
        format!(
            "{}, {cron_due} due",
            format_enabled_total(cron_enabled, cron_total)
        )
    } else {
        format_enabled_total(cron_enabled, cron_total)
    };
    ui::kv("Cron", &cron_summary);
    ui::kv(
        "Triggers",
        &format_enabled_total(trigger_enabled, trigger_total),
    );
    ui::kv(
        "File triggers",
        &format_enabled_total(file_trigger_enabled, file_trigger_total),
    );
    print_automation_delivery_rows(workload, verbose);

    if verbose {
        if let Some(projects) = workload["projects"]["latest"].as_array() {
            if !projects.is_empty() {
                ui::blank();
                ui::section("Recent Projects");
                for project in projects {
                    let slug = project["slug"].as_str().unwrap_or("?");
                    let status = project["status"].as_str().unwrap_or("?");
                    let goal = truncate_display(project["goal"].as_str().unwrap_or(""), 72);
                    println!("    {slug} -- {status} -- {goal}");
                }
            }
        }
    }
}

fn print_active_run_rows(body: &serde_json::Value, verbose: bool) {
    let Some(active_runs) = body["active_runs"].as_array() else {
        return;
    };
    if active_runs.is_empty() {
        return;
    }

    ui::blank();
    ui::section("Running Work");
    for run in active_runs.iter().take(8) {
        let agent = run["agent_name"].as_str().unwrap_or("?");
        let model = run["model_name"].as_str().unwrap_or("?");
        let age = format_duration(run["age_seconds"].as_u64().unwrap_or(0));
        let run_id = run["run_id"].as_str().unwrap_or("?");
        let run_id = if verbose {
            run_id.to_string()
        } else {
            truncate_display(run_id, 12)
        };
        println!("    {agent} -- {age} -- {model} -- run {run_id}");
    }
    if active_runs.len() > 8 {
        println!("    ... and {} more", active_runs.len() - 8);
    }
}

fn print_active_process_rows(body: &serde_json::Value, verbose: bool) {
    let Some(processes) = body["active_processes"].as_array() else {
        return;
    };
    if processes.is_empty() {
        return;
    }

    ui::blank();
    ui::section("Background Processes");
    let mut recovered_alive = false;
    for process in processes.iter().take(8) {
        let id = process["id"].as_str().unwrap_or("?");
        let agent = process["agent_name"].as_str().unwrap_or("?");
        let uptime = format_duration(process["uptime_seconds"].as_u64().unwrap_or(0));
        let idle = format_duration(process["idle_seconds"].as_u64().unwrap_or(0));
        let command = truncate_display(process["command"].as_str().unwrap_or(""), 80);
        let alive = process["alive"].as_bool().unwrap_or(false);
        let attached = process["attached"].as_bool().unwrap_or(true);
        let marker = process_status_marker(alive, attached);
        recovered_alive |= alive && !attached;
        let id = if verbose {
            id.to_string()
        } else {
            truncate_display(id, 16)
        };
        println!("    {id} -- {marker} -- up {uptime} -- idle {idle} -- {agent} -- {command}");
    }
    if processes.len() > 8 {
        println!("    ... and {} more", processes.len() - 8);
    }
    if recovered_alive {
        ui::hint(
            "Recovered processes are detached after restart; inspect externally or stop intentionally with `captain process kill <process_id>`.",
        );
    }
}

fn process_status_marker(alive: bool, attached: bool) -> &'static str {
    match (alive, attached) {
        (true, true) => "alive",
        (true, false) => "recovered",
        (false, _) => "exited",
    }
}

fn sort_project_attention_items(items: &mut [serde_json::Value]) {
    items.sort_by(|a, b| {
        project_attention_priority(a)
            .cmp(&project_attention_priority(b))
            .then_with(|| {
                b["updated_at"]
                    .as_i64()
                    .unwrap_or(0)
                    .cmp(&a["updated_at"].as_i64().unwrap_or(0))
            })
    });
}

fn project_attention_priority(item: &serde_json::Value) -> u8 {
    match item["state"]
        .as_str()
        .or_else(|| item["operator_state"].as_str())
    {
        Some("waiting_for_user") => 0,
        Some("tool_request_pending") => 1,
        Some("resume_ready") => 2,
        Some("stale_active") => 3,
        Some("tool_request_denied") => 4,
        Some("failed") => 5,
        Some("blocked") => 6,
        _ => 9,
    }
}

fn print_channel_queue_rows(body: &serde_json::Value, verbose: bool) {
    let queue = &body["channels"]["inbound_queue"];
    if queue.is_null() {
        return;
    }
    let active = status_json_u64(body, &["channels", "inbound_queue", "active_sessions"]);
    let pending_sessions =
        status_json_u64(body, &["channels", "inbound_queue", "pending_sessions"]);
    let pending_messages =
        status_json_u64(body, &["channels", "inbound_queue", "pending_messages"]);
    let inflight_messages =
        status_json_u64(body, &["channels", "inbound_queue", "inflight_messages"]);
    let dead_letter_messages =
        status_json_u64(body, &["channels", "inbound_queue", "dead_letter_messages"]);
    let dead_letter_oldest_age = status_json_u64(
        body,
        &["channels", "inbound_queue", "dead_letter_oldest_age_secs"],
    );
    let interjected_messages =
        status_json_u64(body, &["channels", "inbound_queue", "interjected_messages"]);
    if active == 0
        && pending_sessions == 0
        && pending_messages == 0
        && inflight_messages == 0
        && dead_letter_messages == 0
        && interjected_messages == 0
    {
        return;
    }

    ui::blank();
    ui::section("Channel Queue");
    let summary = channel_queue_summary(
        active,
        pending_sessions,
        pending_messages,
        inflight_messages,
        dead_letter_messages,
        interjected_messages,
    );
    if pending_messages > 0 || inflight_messages > 0 || dead_letter_messages > 0 {
        ui::kv_warn("Inbound", &summary);
    } else {
        ui::kv("Inbound", &summary);
    }
    if dead_letter_messages > 0 {
        let age = format_duration(dead_letter_oldest_age);
        ui::hint(&format!(
            "Oldest inbound dead letter age: {age}. Review channel logs, then ask the affected user to resend."
        ));
    } else if inflight_messages > 0 {
        ui::hint(
            "Recovered follow-up is retrying; repeated unfinished recovery moves to dead letter.",
        );
    }

    if !verbose {
        return;
    }
    let Some(channels) = queue["channels"].as_array() else {
        return;
    };
    for channel in channels {
        let active = channel["active_sessions"].as_u64().unwrap_or(0);
        let pending = channel["pending_messages"].as_u64().unwrap_or(0);
        let inflight = channel["inflight_messages"].as_u64().unwrap_or(0);
        let dead_letters = channel["dead_letter_messages"].as_u64().unwrap_or(0);
        let interjected = channel["interjected_messages"].as_u64().unwrap_or(0);
        if active == 0 && pending == 0 && inflight == 0 && dead_letters == 0 && interjected == 0 {
            continue;
        }
        let name = channel["channel"].as_str().unwrap_or("?");
        let pending_sessions = channel["pending_sessions"].as_u64().unwrap_or(0);
        let inflight_sessions = channel["inflight_sessions"].as_u64().unwrap_or(0);
        let dead_letter_sessions = channel["dead_letter_sessions"].as_u64().unwrap_or(0);
        println!(
            "    {name} -- {active} active -- {pending_sessions} pending -- {pending} queued -- {inflight_sessions} inflight -- {inflight} retrying -- {dead_letter_sessions} dead -- {dead_letters} dead-letter -- {interjected} interjected"
        );
    }
}

fn channel_queue_summary(
    active: u64,
    pending_sessions: u64,
    pending_messages: u64,
    inflight_messages: u64,
    dead_letter_messages: u64,
    interjected_messages: u64,
) -> String {
    format!(
        "{active} active session(s), {pending_sessions} pending session(s), {pending_messages} pending message(s), {inflight_messages} inflight, {dead_letter_messages} dead-letter, {interjected_messages} interjected"
    )
}

fn print_automation_delivery_rows(workload: &serde_json::Value, verbose: bool) {
    let delivery = &workload["automation"]["delivery"];
    let Some(summary) = automation_delivery_issue_summary(delivery) else {
        return;
    };

    ui::kv_warn("Delivery", &summary);
    ui::hint("Inspect the cron detail before recreating or editing the job.");
    if !verbose {
        return;
    }

    if let Some(errors) = delivery["last_errors"]
        .as_array()
        .filter(|items| !items.is_empty())
    {
        ui::blank();
        ui::section("Cron Delivery Errors");
        for item in errors.iter().take(5) {
            let name = item["job_name"].as_str().unwrap_or("?");
            let kind = item["error_kind"].as_str().unwrap_or("delivery_failed");
            let preview = truncate_display(item["error_preview"].as_str().unwrap_or(""), 96);
            println!("    {name} -- {kind} -- {preview}");
        }
    }
}

fn automation_delivery_issue_summary(delivery: &serde_json::Value) -> Option<String> {
    if delivery.is_null() {
        return None;
    }
    let failed = delivery["failed_jobs"].as_u64().unwrap_or(0);
    let queued = delivery["redelivery_queued"].as_u64().unwrap_or(0);
    let due = delivery["redelivery_due"].as_u64().unwrap_or(0);
    let dead = delivery["dead_letters"].as_u64().unwrap_or(0);
    if failed == 0 && queued == 0 && due == 0 && dead == 0 {
        return None;
    }

    Some(format!(
        "{failed} failed job(s), {queued} queued retry, {due} due, {dead} dead letter(s)"
    ))
}

fn status_json_u64(value: &serde_json::Value, path: &[&str]) -> u64 {
    let mut cursor = value;
    for key in path {
        cursor = &cursor[*key];
    }
    cursor.as_u64().unwrap_or(0)
}

fn format_enabled_total(enabled: u64, total: u64) -> String {
    if total == 0 {
        "0".to_string()
    } else {
        format!("{enabled}/{total} enabled")
    }
}

fn format_duration(total_secs: u64) -> String {
    let hours = total_secs / 3600;
    let minutes = (total_secs % 3600) / 60;
    let seconds = total_secs % 60;
    if hours > 0 {
        format!("{hours}h{minutes:02}m")
    } else if minutes > 0 {
        format!("{minutes}m{seconds:02}s")
    } else {
        format!("{seconds}s")
    }
}

pub(super) fn kernel_status_workload(kernel: &captain_kernel::CaptainKernel) -> serde_json::Value {
    let projects = kernel.memory.project_list(false).unwrap_or_default();
    let mut project_planning = 0usize;
    let mut project_active = 0usize;
    let mut project_paused = 0usize;
    let mut project_done = 0usize;
    for project in &projects {
        match project.status.as_str() {
            "planning" => project_planning += 1,
            "active" => project_active += 1,
            "paused" => project_paused += 1,
            "done" => project_done += 1,
            _ => {}
        }
    }
    let mut latest_projects = projects.clone();
    latest_projects.sort_by_key(|p| std::cmp::Reverse(p.updated_at));
    let latest_projects: Vec<serde_json::Value> = latest_projects
        .into_iter()
        .take(5)
        .map(|project| {
            serde_json::json!({
                "id": project.id,
                "name": project.name,
                "slug": project.slug,
                "goal": project.goal,
                "status": project.status.as_str(),
                "updated_at": project.updated_at,
            })
        })
        .collect();
    let mut project_attention: Vec<serde_json::Value> = projects
        .iter()
        .filter_map(project_attention_from_metadata)
        .collect();
    sort_project_attention_items(&mut project_attention);
    let project_attention_count = project_attention.len();
    project_attention.truncate(8);
    let goals = kernel.goal_store.list();
    let goal_total = goals.len();
    let goal_active = kernel.goal_store.list_active().len();
    let goal_paused = goals
        .iter()
        .filter(|goal| matches!(goal.status, captain_kernel::goals::GoalStatus::Paused))
        .count();
    let goal_escalated = goals
        .iter()
        .filter(|goal| matches!(goal.status, captain_kernel::goals::GoalStatus::Escalated))
        .count();
    let cron_metas = kernel.cron_scheduler.list_all_jobs_with_meta();
    let cron_enabled = cron_metas.iter().filter(|meta| meta.job.enabled).count();
    let cron_due = 0usize;
    let triggers = kernel.list_triggers(None);
    let trigger_enabled = triggers.iter().filter(|trigger| trigger.enabled).count();
    let file_triggers = kernel.list_file_change_triggers(None);
    let file_trigger_enabled = file_triggers
        .iter()
        .filter(|trigger| trigger.enabled)
        .count();

    serde_json::json!({
        "projects": {
            "total": projects.len(),
            "planning": project_planning,
            "active": project_active,
            "paused": project_paused,
            "done": project_done,
            "latest": latest_projects,
            "attention_count": project_attention_count,
            "attention": project_attention,
        },
        "goals": {
            "total": goal_total,
            "active": goal_active,
            "paused": goal_paused,
            "escalated": goal_escalated,
        },
        "automation": {
            "cron_jobs": cron_metas.len(),
            "cron_enabled": cron_enabled,
            "cron_due": cron_due,
            "delivery": kernel_cron_delivery_status(&cron_metas),
            "triggers": triggers.len(),
            "triggers_enabled": trigger_enabled,
            "file_triggers": file_triggers.len(),
            "file_triggers_enabled": file_trigger_enabled,
        },
    })
}

fn kernel_cron_delivery_status(metas: &[captain_kernel::cron::JobMeta]) -> serde_json::Value {
    let failed_jobs = metas
        .iter()
        .filter(|meta| meta.last_delivery_error.is_some())
        .count();
    let redelivery_queued: usize = metas.iter().map(|meta| meta.redelivery_queue.len()).sum();
    let dead_letters: usize = metas.iter().map(|meta| meta.dead_letters.len()).sum();

    serde_json::json!({
        "failed_jobs": failed_jobs,
        "redelivery_queued": redelivery_queued,
        "redelivery_due": 0,
        "dead_letters": dead_letters,
        "last_errors": [],
    })
}

#[cfg(test)]
#[path = "status_workload_tests.rs"]
mod status_workload_tests;
