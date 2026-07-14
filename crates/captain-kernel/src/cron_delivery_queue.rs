//! Persistent cron delivery redelivery queue.
//!
//! Metadata lives in the cron job store; payload bodies live in separate files
//! so `cron_jobs.json` stays inspectable.

use captain_types::agent::AgentId;
use captain_types::scheduler::{CronDelivery, CronJob, CronJobId};
use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use uuid::Uuid;

use crate::delivery_reliability::{
    channel_target, jittered_backoff_delay_ms, webhook_target, DeliveryFailure,
    DEFAULT_MAX_DELIVERY_ATTEMPTS,
};

pub const MAX_REDELIVERY_QUEUE: usize = 20;
pub const MAX_REDELIVERY_ROUNDS: usize = 3;
pub const PAYLOAD_DIR_NAME: &str = "cron_delivery_queue";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CronRedelivery {
    pub id: String,
    pub job_id: CronJobId,
    pub agent_id: AgentId,
    pub target: String,
    pub delivery: CronDelivery,
    pub payload_path: String,
    pub created_at: DateTime<Utc>,
    pub next_attempt_at: DateTime<Utc>,
    pub attempts: usize,
    pub max_attempts: usize,
    pub last_error: Option<String>,
}

impl CronRedelivery {
    pub fn new(
        job: &CronJob,
        delivery: CronDelivery,
        failure: &DeliveryFailure,
        payload_path: String,
        now: DateTime<Utc>,
    ) -> Self {
        Self {
            id: Uuid::new_v4().to_string(),
            job_id: job.id,
            agent_id: job.agent_id,
            target: delivery_target(&delivery, failure),
            delivery,
            payload_path,
            created_at: now,
            next_attempt_at: next_attempt_after(failure.attempts, now),
            attempts: 0,
            max_attempts: MAX_REDELIVERY_ROUNDS,
            last_error: Some(failure.to_string()),
        }
    }

    pub fn schedule_failure(&mut self, failure: &DeliveryFailure, now: DateTime<Utc>) -> bool {
        self.attempts += 1;
        self.last_error = Some(failure.to_string());
        if self.attempts >= self.max_attempts {
            return false;
        }
        self.next_attempt_at =
            next_attempt_after(self.attempts + DEFAULT_MAX_DELIVERY_ATTEMPTS, now);
        true
    }
}

pub fn delivery_target(delivery: &CronDelivery, fallback: &DeliveryFailure) -> String {
    match delivery {
        CronDelivery::Channel { channel, to } => channel_target(channel, to),
        CronDelivery::Webhook { url } => webhook_target(url),
        CronDelivery::LastChannel | CronDelivery::None => fallback.target.clone(),
    }
}

pub fn payload_dir(home_dir: &Path) -> PathBuf {
    home_dir.join(PAYLOAD_DIR_NAME)
}

pub fn write_payload_file(
    home_dir: &Path,
    job_id: CronJobId,
    payload: &str,
    now: DateTime<Utc>,
) -> Result<String, String> {
    let dir = payload_dir(home_dir);
    std::fs::create_dir_all(&dir)
        .map_err(|e| format!("failed to create redelivery queue dir: {e}"))?;
    secure_dir(&dir);

    let file_name = format!(
        "{}-{}-{}.txt",
        job_id,
        now.timestamp_millis(),
        Uuid::new_v4().simple()
    );
    let final_path = dir.join(file_name);
    let tmp_path = final_path.with_extension("tmp");
    std::fs::write(&tmp_path, payload.as_bytes())
        .map_err(|e| format!("failed to write redelivery payload: {e}"))?;
    std::fs::rename(&tmp_path, &final_path)
        .map_err(|e| format!("failed to commit redelivery payload: {e}"))?;
    secure_file(&final_path);
    Ok(final_path.to_string_lossy().to_string())
}

pub fn read_payload_file(path: &str) -> Result<String, String> {
    std::fs::read_to_string(path).map_err(|e| format!("failed to read redelivery payload: {e}"))
}

pub fn remove_payload_file(path: &str) {
    let _ = std::fs::remove_file(path);
}

pub fn push_redelivery(queue: &mut Vec<CronRedelivery>, entry: CronRedelivery) -> Vec<String> {
    queue.push(entry);
    if queue.len() <= MAX_REDELIVERY_QUEUE {
        return Vec::new();
    }
    let overflow = queue.len() - MAX_REDELIVERY_QUEUE;
    queue
        .drain(0..overflow)
        .map(|entry| entry.payload_path)
        .collect()
}

fn next_attempt_after(attempt_seed: usize, now: DateTime<Utc>) -> DateTime<Utc> {
    now + Duration::milliseconds(jittered_backoff_delay_ms(attempt_seed) as i64)
}

fn secure_dir(path: &Path) {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700));
    }
    #[cfg(not(unix))]
    {
        let _ = path;
    }
}

fn secure_file(path: &Path) {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600));
    }
    #[cfg(not(unix))]
    {
        let _ = path;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use captain_types::scheduler::{CronAction, CronSchedule};

    fn make_job() -> CronJob {
        CronJob {
            id: CronJobId::new(),
            agent_id: AgentId::new(),
            name: "delivery".to_string(),
            enabled: true,
            schedule: CronSchedule::Every { every_secs: 3600 },
            action: CronAction::SystemEvent {
                text: "ping".to_string(),
            },
            delivery: CronDelivery::None,
            created_at: Utc::now(),
            last_run: None,
            next_run: None,
        }
    }

    #[test]
    fn schedules_redelivery_after_failure() {
        let job = make_job();
        let now = Utc::now();
        let failure = DeliveryFailure::new("channel:telegram:42", "HTTP 503", 5);
        let entry = CronRedelivery::new(
            &job,
            CronDelivery::Channel {
                channel: "telegram".into(),
                to: "42".into(),
            },
            &failure,
            "/tmp/payload.txt".into(),
            now,
        );

        assert_eq!(entry.job_id, job.id);
        assert_eq!(entry.target, "channel:telegram:42");
        assert!(entry.next_attempt_at > now);
        assert_eq!(entry.attempts, 0);
    }

    #[test]
    fn push_redelivery_is_bounded_and_returns_dropped_payloads() {
        let job = make_job();
        let failure = DeliveryFailure::new("webhook:https://example.com", "HTTP 500", 5);
        let mut queue = Vec::new();
        let mut dropped = Vec::new();
        for i in 0..(MAX_REDELIVERY_QUEUE + 2) {
            let mut entry = CronRedelivery::new(
                &job,
                CronDelivery::Webhook {
                    url: "https://example.com".into(),
                },
                &failure,
                format!("/tmp/{i}.txt"),
                Utc::now(),
            );
            entry.id = i.to_string();
            dropped.extend(push_redelivery(&mut queue, entry));
        }

        assert_eq!(queue.len(), MAX_REDELIVERY_QUEUE);
        assert_eq!(queue[0].id, "2");
        assert_eq!(dropped, vec!["/tmp/0.txt", "/tmp/1.txt"]);
    }
}
