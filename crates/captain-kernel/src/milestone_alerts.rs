//! Deadline alerts for project milestones.
//!
//! Projects expose missed milestones synchronously, but near-deadline
//! alerts need a runtime loop so users get notified before a deadline is
//! missed. The loop records each sent alert in structured memory to stay
//! idempotent across daemon restarts.

use std::sync::Arc;
use std::time::Duration;

use captain_memory::milestone::{Milestone, MilestoneStatus};
use captain_memory::project::{Project, ProjectStatus};
use captain_runtime::kernel_handle::KernelHandle;
use serde_json::json;
use tracing::{debug, info, warn};

use crate::kernel::{shared_memory_agent_id, CaptainKernel};

pub const DEADLINE_ALERT_WINDOW_MS: i64 = 24 * 60 * 60 * 1000;
pub const ALERT_SCAN_INTERVAL_SECS: u64 = 60 * 60;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MilestoneDeadlineAlert {
    pub key: String,
    pub project_id: String,
    pub project_slug: String,
    pub project_name: String,
    pub milestone_id: String,
    pub milestone_name: String,
    pub due_date: i64,
    pub millis_until_due: i64,
}

pub fn spawn_deadline_alert_task(kernel: Arc<CaptainKernel>) {
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(Duration::from_secs(ALERT_SCAN_INTERVAL_SECS));
        loop {
            interval.tick().await;
            send_due_milestone_alerts(&kernel).await;
        }
    });
}

pub async fn send_due_milestone_alerts(kernel: &Arc<CaptainKernel>) {
    let Some(recipient) = default_telegram_recipient(kernel) else {
        debug!("milestone deadline alerts skipped: no telegram default_chat_id configured");
        return;
    };
    if !kernel.channel_adapters.contains_key("telegram") {
        debug!("milestone deadline alerts skipped: telegram adapter is not active");
        return;
    }

    let now_ms = chrono::Utc::now().timestamp_millis();
    let projects = match kernel.memory.project_list(false) {
        Ok(projects) => projects,
        Err(e) => {
            warn!(error = %e, "milestone deadline alerts: failed to list projects");
            return;
        }
    };

    for project in projects {
        let milestones = match kernel.memory.milestone_list_for_project(&project.id) {
            Ok(milestones) => milestones,
            Err(e) => {
                warn!(
                    project_id = %project.id,
                    error = %e,
                    "milestone deadline alerts: failed to list milestones"
                );
                continue;
            }
        };

        for milestone in milestones {
            let Some(alert) = build_alert(&project, &milestone, now_ms) else {
                continue;
            };
            if alert_was_sent(kernel, &alert.key) {
                continue;
            }

            let message = format_alert(&alert, &kernel.config.language);
            match <CaptainKernel as KernelHandle>::send_channel_message_from(
                kernel.as_ref(),
                "telegram",
                &recipient,
                &message,
                None,
                Some("captain"),
            )
            .await
            {
                Ok(_) => {
                    mark_alert_sent(kernel, &alert, now_ms);
                    info!(
                        project = %alert.project_slug,
                        milestone = %alert.milestone_id,
                        "milestone deadline alert sent"
                    );
                }
                Err(e) => {
                    warn!(
                        project = %alert.project_slug,
                        milestone = %alert.milestone_id,
                        error = %e,
                        "milestone deadline alert delivery failed"
                    );
                }
            }
        }
    }
}

pub fn build_alert(
    project: &Project,
    milestone: &Milestone,
    now_ms: i64,
) -> Option<MilestoneDeadlineAlert> {
    if matches!(
        project.status,
        ProjectStatus::Done | ProjectStatus::Archived
    ) {
        return None;
    }
    if milestone.status == MilestoneStatus::Completed {
        return None;
    }

    let due_date = milestone.due_date?;
    let millis_until_due = due_date - now_ms;
    if !(1..=DEADLINE_ALERT_WINDOW_MS).contains(&millis_until_due) {
        return None;
    }

    Some(MilestoneDeadlineAlert {
        key: alert_key(&project.id, milestone)?,
        project_id: project.id.clone(),
        project_slug: project.slug.clone(),
        project_name: project.name.clone(),
        milestone_id: milestone.id.clone(),
        milestone_name: milestone.name.clone(),
        due_date,
        millis_until_due,
    })
}

pub fn alert_key(project_id: &str, milestone: &Milestone) -> Option<String> {
    milestone.due_date.map(|due_date| {
        format!(
            "system.milestone_deadline_alert.sent:{project_id}:{}:{due_date}",
            milestone.id
        )
    })
}

pub fn format_alert(alert: &MilestoneDeadlineAlert, language: &str) -> String {
    let due = format_due_date(alert.due_date);
    let remaining = format_remaining(alert.millis_until_due, language);
    if language.to_ascii_lowercase().starts_with("fr") {
        format!(
            "Jalon projet a surveiller\n\
             Projet: {} ({})\n\
             Jalon: {}\n\
             Echeance: {}\n\
             Temps restant: {}\n\
             Captain garde ce jalon visible jusqu'a completion.",
            alert.project_name, alert.project_slug, alert.milestone_name, due, remaining
        )
    } else {
        format!(
            "Project milestone to watch\n\
             Project: {} ({})\n\
             Milestone: {}\n\
             Due: {}\n\
             Remaining: {}\n\
             Captain will keep this milestone visible until completion.",
            alert.project_name, alert.project_slug, alert.milestone_name, due, remaining
        )
    }
}

fn default_telegram_recipient(kernel: &CaptainKernel) -> Option<String> {
    kernel
        .config
        .channels
        .telegram
        .as_ref()?
        .default_chat_id
        .as_ref()
        .filter(|chat_id| !chat_id.trim().is_empty())
        .cloned()
}

fn alert_was_sent(kernel: &CaptainKernel, key: &str) -> bool {
    matches!(
        kernel.memory.structured_get(shared_memory_agent_id(), key),
        Ok(Some(_))
    )
}

fn mark_alert_sent(kernel: &CaptainKernel, alert: &MilestoneDeadlineAlert, now_ms: i64) {
    if let Err(e) = kernel.memory.structured_set(
        shared_memory_agent_id(),
        &alert.key,
        json!({
            "sent_at": now_ms,
            "project_id": alert.project_id.clone(),
            "project_slug": alert.project_slug.clone(),
            "milestone_id": alert.milestone_id.clone(),
            "due_date": alert.due_date,
        }),
    ) {
        warn!(
            key = %alert.key,
            error = %e,
            "milestone deadline alert sent but failed to record idempotency marker"
        );
    }
}

fn format_due_date(unix_ms: i64) -> String {
    chrono::DateTime::<chrono::Utc>::from_timestamp_millis(unix_ms)
        .map(|dt| dt.to_rfc3339())
        .unwrap_or_else(|| unix_ms.to_string())
}

fn format_remaining(millis: i64, language: &str) -> String {
    let total_minutes = (millis.max(0) + 59_999) / 60_000;
    let hours = total_minutes / 60;
    let minutes = total_minutes % 60;

    if language.to_ascii_lowercase().starts_with("fr") {
        if hours > 0 && minutes > 0 {
            format!("{hours} h {minutes} min")
        } else if hours > 0 {
            format!("{hours} h")
        } else {
            format!("{minutes} min")
        }
    } else if hours > 0 && minutes > 0 {
        format!("{hours}h {minutes}m")
    } else if hours > 0 {
        format!("{hours}h")
    } else {
        format!("{minutes}m")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn project(status: ProjectStatus) -> Project {
        Project {
            id: "project-1".into(),
            name: "Calculator".into(),
            slug: "calculator".into(),
            goal: "Build a CLI calculator".into(),
            status,
            deadline: None,
            created_at: 0,
            updated_at: 0,
            metadata: serde_json::json!({}),
        }
    }

    fn milestone(status: MilestoneStatus, due_date: Option<i64>) -> Milestone {
        Milestone {
            id: "milestone-1".into(),
            project_id: "project-1".into(),
            name: "Smoke test".into(),
            due_date,
            status,
            deliverables: vec![],
            completed_at: None,
            created_at: 0,
            updated_at: 0,
        }
    }

    #[test]
    fn build_alert_keeps_upcoming_milestones_inside_window() {
        let alert = build_alert(
            &project(ProjectStatus::Active),
            &milestone(
                MilestoneStatus::Upcoming,
                Some(DEADLINE_ALERT_WINDOW_MS - 1),
            ),
            0,
        )
        .expect("expected alert");

        assert_eq!(alert.project_slug, "calculator");
        assert_eq!(
            alert.key,
            "system.milestone_deadline_alert.sent:project-1:milestone-1:86399999"
        );
    }

    #[test]
    fn build_alert_skips_done_or_completed_work() {
        assert!(build_alert(
            &project(ProjectStatus::Done),
            &milestone(MilestoneStatus::Upcoming, Some(10_000)),
            0,
        )
        .is_none());
        assert!(build_alert(
            &project(ProjectStatus::Active),
            &milestone(MilestoneStatus::Completed, Some(10_000)),
            0,
        )
        .is_none());
    }

    #[test]
    fn build_alert_skips_missed_or_far_future_milestones() {
        assert!(build_alert(
            &project(ProjectStatus::Active),
            &milestone(MilestoneStatus::Upcoming, Some(0)),
            1,
        )
        .is_none());
        assert!(build_alert(
            &project(ProjectStatus::Active),
            &milestone(
                MilestoneStatus::Upcoming,
                Some(DEADLINE_ALERT_WINDOW_MS + 1)
            ),
            0,
        )
        .is_none());
    }

    #[test]
    fn format_alert_follows_user_language() {
        let alert = build_alert(
            &project(ProjectStatus::Active),
            &milestone(MilestoneStatus::Upcoming, Some(60_000)),
            0,
        )
        .unwrap();

        assert!(format_alert(&alert, "fr").contains("Jalon projet"));
        assert!(format_alert(&alert, "en").contains("Project milestone"));
    }
}
