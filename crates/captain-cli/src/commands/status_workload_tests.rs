use super::*;

#[test]
fn status_json_u64_reads_nested_counts() {
    let value = serde_json::json!({
        "workload": {
            "automation": {
                "cron_jobs": 3
            }
        }
    });
    assert_eq!(
        status_json_u64(&value, &["workload", "automation", "cron_jobs"]),
        3
    );
    assert_eq!(status_json_u64(&value, &["workload", "missing"]), 0);
}

#[test]
fn format_enabled_total_handles_empty_groups() {
    assert_eq!(format_enabled_total(0, 0), "0");
    assert_eq!(format_enabled_total(2, 5), "2/5 enabled");
}

#[test]
fn format_duration_keeps_status_rows_short() {
    assert_eq!(format_duration(9), "9s");
    assert_eq!(format_duration(65), "1m05s");
    assert_eq!(format_duration(3661), "1h01m");
}

#[test]
fn channel_queue_summary_includes_pending_messages() {
    assert_eq!(
        channel_queue_summary(2, 1, 3, 4, 5, 6),
        "2 active session(s), 1 pending session(s), 3 pending message(s), 4 inflight, 5 dead-letter, 6 interjected"
    );
}

#[test]
fn process_status_marker_distinguishes_recovered_processes() {
    assert_eq!(process_status_marker(true, true), "alive");
    assert_eq!(process_status_marker(true, false), "recovered");
    assert_eq!(process_status_marker(false, false), "exited");
}

#[test]
fn sort_project_attention_prioritizes_actionable_waits_then_recency() {
    let mut items = vec![
        serde_json::json!({"state": "failed", "updated_at": 30, "project_slug": "failed"}),
        serde_json::json!({"state": "waiting_for_user", "updated_at": 10, "project_slug": "ask"}),
        serde_json::json!({"state": "resume_ready", "updated_at": 20, "project_slug": "resume-old"}),
        serde_json::json!({"state": "resume_ready", "updated_at": 40, "project_slug": "resume-new"}),
    ];

    sort_project_attention_items(&mut items);

    let slugs: Vec<&str> = items
        .iter()
        .filter_map(|item| item["project_slug"].as_str())
        .collect();
    assert_eq!(slugs, vec!["ask", "resume-new", "resume-old", "failed"]);
}

#[test]
fn project_attention_count_can_remain_larger_than_visible_rows() {
    let mut items = (0..10)
        .map(|idx| {
            serde_json::json!({
                "state": "blocked",
                "updated_at": idx,
                "project_slug": format!("blocked-{idx}")
            })
        })
        .collect::<Vec<_>>();

    sort_project_attention_items(&mut items);
    let total = items.len();
    items.truncate(8);

    assert_eq!(total, 10);
    assert_eq!(items.len(), 8);
}

#[test]
fn automation_delivery_issue_summary_is_hidden_when_clean() {
    let delivery = serde_json::json!({
        "failed_jobs": 0,
        "redelivery_queued": 0,
        "redelivery_due": 0,
        "dead_letters": 0
    });
    assert!(automation_delivery_issue_summary(&delivery).is_none());
}

#[test]
fn automation_delivery_issue_summary_reports_retry_and_dead_letters() {
    let delivery = serde_json::json!({
        "failed_jobs": 1,
        "redelivery_queued": 2,
        "redelivery_due": 1,
        "dead_letters": 3
    });
    assert_eq!(
        automation_delivery_issue_summary(&delivery).unwrap(),
        "1 failed job(s), 2 queued retry, 1 due, 3 dead letter(s)"
    );
}
