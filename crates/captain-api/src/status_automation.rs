//! Automation status helpers.

use captain_kernel::cron::JobMeta;
use chrono::{DateTime, Utc};

pub(crate) fn build_automation_delivery_status(
    metas: &[JobMeta],
    now: DateTime<Utc>,
) -> serde_json::Value {
    let failed_jobs = metas
        .iter()
        .filter(|meta| meta.last_delivery_error.is_some())
        .count();
    let redelivery_queued: usize = metas.iter().map(|meta| meta.redelivery_queue.len()).sum();
    let redelivery_due: usize = metas
        .iter()
        .flat_map(|meta| meta.redelivery_queue.iter())
        .filter(|entry| entry.next_attempt_at <= now)
        .count();
    let dead_letters: usize = metas.iter().map(|meta| meta.dead_letters.len()).sum();

    let mut last_errors: Vec<serde_json::Value> = metas
        .iter()
        .filter_map(|meta| {
            let error = meta.last_delivery_error.as_ref()?;
            let next_redelivery_at = meta
                .redelivery_queue
                .iter()
                .map(|entry| entry.next_attempt_at)
                .min()
                .map(|dt| dt.to_rfc3339());
            Some(serde_json::json!({
                "job_id": meta.job.id.to_string(),
                "job_name": meta.job.name,
                "last_run": meta.job.last_run.map(|dt| dt.to_rfc3339()),
                "last_status": meta.last_status,
                "error_kind": delivery_error_kind(error),
                "error_preview": safe_delivery_error_preview(error),
                "redelivery_queued": meta.redelivery_queue.len(),
                "dead_letters": meta.dead_letters.len(),
                "next_redelivery_at": next_redelivery_at,
            }))
        })
        .collect();
    last_errors.sort_by(|a, b| {
        b["last_run"]
            .as_str()
            .unwrap_or("")
            .cmp(a["last_run"].as_str().unwrap_or(""))
    });
    last_errors.truncate(5);

    let state = if dead_letters > 0 {
        "dead_letter"
    } else if failed_jobs > 0 || redelivery_due > 0 {
        "attention"
    } else if redelivery_queued > 0 {
        "retrying"
    } else {
        "ok"
    };

    serde_json::json!({
        "state": state,
        "failed_jobs": failed_jobs,
        "redelivery_queued": redelivery_queued,
        "redelivery_due": redelivery_due,
        "dead_letters": dead_letters,
        "last_errors": last_errors,
    })
}

fn delivery_error_kind(error: &str) -> &'static str {
    let lower = error.to_ascii_lowercase();
    if lower.contains("rate limit") || lower.contains("429") {
        "rate_limit"
    } else if lower.contains("timeout") || lower.contains("timed out") {
        "timeout"
    } else if lower.contains("http 5") || lower.contains(" 5") {
        "transient_http"
    } else if lower.contains("http 4")
        || lower.contains("unauthorized")
        || lower.contains("forbidden")
    {
        "target_or_auth"
    } else {
        "delivery_failed"
    }
}

fn safe_delivery_error_preview(error: &str) -> String {
    let trimmed = error.trim();
    if trimmed.contains("webhook:http://")
        || trimmed.contains("webhook:https://")
        || trimmed.contains("http://")
        || trimmed.contains("https://")
    {
        return format!(
            "{}; inspect cron detail for endpoint",
            delivery_error_kind(trimmed)
        );
    }
    truncate_chars(trimmed, 180)
}

fn truncate_chars(value: &str, max_chars: usize) -> String {
    let mut out = String::new();
    for (index, ch) in value.chars().enumerate() {
        if index >= max_chars {
            out.push_str("...");
            break;
        }
        out.push(ch);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use captain_kernel::cron::JobMeta;
    use captain_kernel::cron_delivery_queue::CronRedelivery;
    use captain_kernel::delivery_reliability::DeliveryDeadLetter;
    use captain_types::agent::AgentId;
    use captain_types::scheduler::{CronAction, CronDelivery, CronJob, CronJobId, CronSchedule};
    use chrono::Duration;

    fn meta_with_delivery_issue(now: DateTime<Utc>) -> JobMeta {
        let job = CronJob {
            id: CronJobId::new(),
            agent_id: AgentId::new(),
            name: "daily report".to_string(),
            enabled: true,
            schedule: CronSchedule::Every { every_secs: 3600 },
            action: CronAction::SystemEvent {
                text: "ping".to_string(),
            },
            delivery: CronDelivery::Webhook {
                url: "https://example.com/hook?token=secret".to_string(),
            },
            created_at: now,
            last_run: Some(now),
            next_run: None,
        };
        JobMeta {
            job: job.clone(),
            one_shot: false,
            last_status: Some("delivery_failed".to_string()),
            last_delivery_error: Some(
                "webhook:https://example.com/hook?token=secret after 5 attempt(s): HTTP 503"
                    .to_string(),
            ),
            consecutive_errors: 0,
            run_history: Vec::new(),
            dead_letters: vec![DeliveryDeadLetter {
                timestamp: now,
                target: "webhook:https://example.com/hook?token=secret".to_string(),
                error: "HTTP 503".to_string(),
                payload_preview: "body".to_string(),
                attempts: 5,
            }],
            redelivery_queue: vec![CronRedelivery {
                id: "redeliver-1".to_string(),
                job_id: job.id,
                agent_id: job.agent_id,
                target: "webhook:https://example.com/hook?token=secret".to_string(),
                delivery: job.delivery,
                payload_path: "/tmp/payload".to_string(),
                created_at: now,
                next_attempt_at: now - Duration::seconds(1),
                attempts: 1,
                max_attempts: 3,
                last_error: Some("HTTP 503".to_string()),
            }],
        }
    }

    #[test]
    fn delivery_status_counts_queued_due_and_dead_letters() {
        let now = Utc::now();
        let status = build_automation_delivery_status(&[meta_with_delivery_issue(now)], now);

        assert_eq!(status["state"], "dead_letter");
        assert_eq!(status["failed_jobs"], 1);
        assert_eq!(status["redelivery_queued"], 1);
        assert_eq!(status["redelivery_due"], 1);
        assert_eq!(status["dead_letters"], 1);
        assert_eq!(status["last_errors"][0]["error_kind"], "transient_http");
    }

    #[test]
    fn delivery_status_redacts_webhook_urls_from_preview() {
        let now = Utc::now();
        let status = build_automation_delivery_status(&[meta_with_delivery_issue(now)], now);
        let preview = status["last_errors"][0]["error_preview"].as_str().unwrap();

        assert!(preview.contains("inspect cron detail"));
        assert!(!preview.contains("token=secret"));
    }
}
