use crate::cron::CronJobPatch;
use crate::triggers::{FileChangeTrigger, TriggerId, DEFAULT_FILE_WATCH_DEBOUNCE_MS};
use captain_types::agent::AgentId;
use captain_types::event::FileEventKind;
use captain_types::scheduler::{CronAction, CronDelivery, CronJob, CronJobId, CronSchedule};
use serde::de::DeserializeOwned;
use std::path::PathBuf;

use super::CaptainKernel;

impl CaptainKernel {
    pub(super) fn handle_cron_create(
        &self,
        agent_id: &str,
        job_json: serde_json::Value,
    ) -> Result<String, String> {
        let name = job_json["name"]
            .as_str()
            .ok_or("Missing 'name' field")?
            .to_string();
        let schedule: CronSchedule = serde_json::from_value(job_json["schedule"].clone())
            .map_err(|e| format!("Invalid schedule: {e}"))?;
        let action: CronAction = serde_json::from_value(job_json["action"].clone())
            .map_err(|e| format!("Invalid action: {e}"))?;
        let delivery = cron_create_delivery(&job_json)?;
        let one_shot = job_json["one_shot"].as_bool().unwrap_or(false);
        let aid = parse_agent_id(agent_id, "Invalid agent ID")?;

        let job = CronJob {
            id: CronJobId::new(),
            agent_id: aid,
            name,
            schedule,
            action,
            delivery,
            enabled: true,
            created_at: chrono::Utc::now(),
            next_run: None,
            last_run: None,
        };

        let id = self
            .cron_scheduler
            .add_job(job, one_shot)
            .map_err(|e| format!("{e}"))?;

        if let Err(e) = self.cron_scheduler.persist() {
            tracing::warn!("Failed to persist cron jobs: {e}");
        }

        Ok(serde_json::json!({
            "job_id": id.to_string(),
            "status": "created"
        })
        .to_string())
    }

    pub(super) fn handle_cron_list(
        &self,
        agent_id: &str,
    ) -> Result<Vec<serde_json::Value>, String> {
        let aid = parse_agent_id(agent_id, "Invalid agent ID")?;
        let jobs = self.cron_scheduler.list_jobs(aid);
        let json_jobs = jobs
            .into_iter()
            .map(|j| serde_json::to_value(&j).unwrap_or_default())
            .collect();
        Ok(json_jobs)
    }

    pub(super) fn handle_cron_update(
        &self,
        agent_id: &str,
        job_json: serde_json::Value,
    ) -> Result<String, String> {
        let aid = parse_agent_id(agent_id, "Invalid agent ID")?;
        let requested = requested_cron_job_id(&job_json)?;
        let job_id = self.resolve_cron_job_id_for_agent(aid, requested)?;
        let patch = cron_patch_from_json(&job_json)?;

        let updated = self
            .cron_scheduler
            .update_job(job_id, patch)
            .map_err(|e| format!("{e}"))?;

        if let Err(e) = self.cron_scheduler.persist() {
            tracing::warn!("Failed to persist cron jobs: {e}");
        }

        Ok(serde_json::json!({
            "job_id": job_id.to_string(),
            "status": "updated",
            "job": updated
        })
        .to_string())
    }

    pub(super) fn handle_cron_cancel(&self, job_id: &str) -> Result<(), String> {
        let id: CronJobId = job_id.parse().map_err(|e| format!("Invalid job ID: {e}"))?;
        self.cron_scheduler
            .remove_job(id)
            .map_err(|e| format!("{e}"))?;

        if let Err(e) = self.cron_scheduler.persist() {
            tracing::warn!("Failed to persist cron jobs: {e}");
        }

        Ok(())
    }

    pub(super) fn handle_file_trigger_register(
        &self,
        agent_id: &str,
        input: serde_json::Value,
    ) -> Result<String, String> {
        let agent_id = parse_agent_id(agent_id, "Invalid agent_id")?;
        let trigger = FileChangeTrigger {
            id: TriggerId::new(),
            paths: parse_file_trigger_paths(&input)?,
            recursive: input
                .get("recursive")
                .and_then(|v| v.as_bool())
                .unwrap_or(true),
            events: parse_file_trigger_events(&input)?,
            agent_id,
            prompt_template: input
                .get("prompt_template")
                .and_then(|v| v.as_str())
                .unwrap_or("File {kind}: {path}")
                .to_string(),
            debounce_ms: input
                .get("debounce_ms")
                .and_then(|v| v.as_u64())
                .unwrap_or(DEFAULT_FILE_WATCH_DEBOUNCE_MS),
            enabled: input
                .get("enabled")
                .and_then(|v| v.as_bool())
                .unwrap_or(true),
        };

        let id = self
            .register_file_change_trigger(trigger)
            .map_err(|e| e.to_string())?;
        Ok(id.to_string())
    }

    pub(super) fn handle_file_trigger_list(
        &self,
        agent_id: Option<&str>,
    ) -> Result<Vec<serde_json::Value>, String> {
        let filter = match agent_id {
            Some(s) => Some(parse_agent_id(s, "Invalid agent_id")?),
            None => None,
        };
        let list = self
            .list_file_change_triggers(filter)
            .iter()
            .map(|t| {
                serde_json::json!({
                    "id": t.id.to_string(),
                    "agent_id": t.agent_id.to_string(),
                    "paths": t.paths.iter().map(|p| p.display().to_string()).collect::<Vec<_>>(),
                    "recursive": t.recursive,
                    "events": t.events.iter().map(|k| k.as_str()).collect::<Vec<_>>(),
                    "prompt_template": t.prompt_template,
                    "debounce_ms": t.debounce_ms,
                    "enabled": t.enabled,
                })
            })
            .collect();
        Ok(list)
    }

    pub(super) fn handle_file_trigger_set_enabled(
        &self,
        trigger_id: &str,
        enabled: bool,
    ) -> Result<bool, String> {
        let id: TriggerId = trigger_id
            .parse()
            .map_err(|e| format!("Invalid trigger_id: {e}"))?;
        self.set_file_change_trigger_enabled(id, enabled)
            .map_err(|e| e.to_string())
    }

    pub(super) fn handle_file_trigger_remove(&self, trigger_id: &str) -> Result<bool, String> {
        let id: TriggerId = trigger_id
            .parse()
            .map_err(|e| format!("Invalid trigger_id: {e}"))?;
        self.remove_file_change_trigger(id)
            .map_err(|e| e.to_string())
    }

    fn resolve_cron_job_id_for_agent(
        &self,
        agent_id: AgentId,
        requested: &str,
    ) -> Result<CronJobId, String> {
        let matches: Vec<_> = self
            .cron_scheduler
            .list_jobs(agent_id)
            .into_iter()
            .filter(|j| j.id.to_string().starts_with(requested))
            .collect();
        match matches.as_slice() {
            [] => Err(format!("Cron job '{requested}' not found for this agent")),
            [job] => Ok(job.id),
            many => {
                let ids: Vec<String> = many.iter().map(|j| j.id.to_string()).collect();
                Err(format!(
                    "Cron job id prefix '{requested}' is ambiguous: {}",
                    ids.join(", ")
                ))
            }
        }
    }
}

fn parse_agent_id(agent_id: &str, label: &str) -> Result<AgentId, String> {
    agent_id
        .parse::<AgentId>()
        .map_err(|e| format!("{label}: {e}"))
}

fn cron_create_delivery(job_json: &serde_json::Value) -> Result<CronDelivery, String> {
    if job_json["delivery"].is_object() {
        serde_json::from_value(job_json["delivery"].clone())
            .map_err(|e| format!("Invalid delivery: {e}"))
    } else {
        Ok(CronDelivery::None)
    }
}

fn requested_cron_job_id(job_json: &serde_json::Value) -> Result<&str, String> {
    job_json["job_id"]
        .as_str()
        .or_else(|| job_json["id"].as_str())
        .ok_or_else(|| "Missing 'job_id' field".to_string())
}

fn cron_patch_from_json(job_json: &serde_json::Value) -> Result<CronJobPatch, String> {
    let patch = CronJobPatch {
        name: job_json["name"].as_str().map(ToString::to_string),
        schedule: optional_cron_patch_value(job_json, "schedule", "Invalid schedule")?,
        action: optional_cron_patch_value(job_json, "action", "Invalid action")?,
        delivery: optional_cron_patch_value(job_json, "delivery", "Invalid delivery")?,
        enabled: optional_bool_patch(job_json, "enabled", "Invalid enabled: expected boolean")?,
        one_shot: optional_bool_patch(job_json, "one_shot", "Invalid one_shot: expected boolean")?,
    };

    if patch.name.is_none()
        && patch.schedule.is_none()
        && patch.action.is_none()
        && patch.delivery.is_none()
        && patch.enabled.is_none()
        && patch.one_shot.is_none()
    {
        return Err("cron_update requires at least one patch field".to_string());
    }

    Ok(patch)
}

fn optional_cron_patch_value<T>(
    job_json: &serde_json::Value,
    field: &str,
    error_prefix: &str,
) -> Result<Option<T>, String>
where
    T: DeserializeOwned,
{
    match job_json.get(field) {
        Some(v) if !v.is_null() => serde_json::from_value(v.clone())
            .map(Some)
            .map_err(|e| format!("{error_prefix}: {e}")),
        _ => Ok(None),
    }
}

fn optional_bool_patch(
    job_json: &serde_json::Value,
    field: &str,
    error: &str,
) -> Result<Option<bool>, String> {
    match job_json.get(field) {
        Some(v) => Some(v.as_bool().ok_or(error))
            .transpose()
            .map_err(str::to_string),
        None => Ok(None),
    }
}

fn parse_file_trigger_paths(input: &serde_json::Value) -> Result<Vec<PathBuf>, String> {
    let raw_paths = input
        .get("paths")
        .and_then(|v| v.as_array())
        .ok_or_else(|| "Missing 'paths' array".to_string())?;
    if raw_paths.is_empty() {
        return Err("'paths' must contain at least one path".to_string());
    }
    let mut paths = Vec::with_capacity(raw_paths.len());
    for value in raw_paths {
        let s = value
            .as_str()
            .ok_or_else(|| "'paths' entries must be strings".to_string())?;
        paths.push(PathBuf::from(s));
    }
    Ok(paths)
}

fn parse_file_trigger_events(input: &serde_json::Value) -> Result<Vec<FileEventKind>, String> {
    match input.get("events").and_then(|v| v.as_array()) {
        Some(values) if !values.is_empty() => {
            let mut out = Vec::with_capacity(values.len());
            for v in values {
                let s = v
                    .as_str()
                    .ok_or_else(|| "'events' entries must be strings".to_string())?;
                out.push(s.parse::<FileEventKind>()?);
            }
            Ok(out)
        }
        _ => Ok(vec![FileEventKind::Any]),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cron_patch_rejects_empty_patch() {
        let err = cron_patch_from_json(&serde_json::json!({"job_id": "abc"})).unwrap_err();
        assert_eq!(err, "cron_update requires at least one patch field");
    }

    #[test]
    fn cron_patch_rejects_non_bool_enabled() {
        let err = cron_patch_from_json(&serde_json::json!({
            "job_id": "abc",
            "enabled": "yes"
        }))
        .unwrap_err();
        assert_eq!(err, "Invalid enabled: expected boolean");
    }

    #[test]
    fn cron_patch_accepts_name_and_one_shot() {
        let patch = cron_patch_from_json(&serde_json::json!({
            "job_id": "abc",
            "name": "Nightly",
            "one_shot": true
        }))
        .unwrap();

        assert_eq!(patch.name.as_deref(), Some("Nightly"));
        assert_eq!(patch.one_shot, Some(true));
        assert_eq!(patch.enabled, None);
    }

    #[test]
    fn file_trigger_paths_require_non_empty_string_array() {
        let err = parse_file_trigger_paths(&serde_json::json!({})).unwrap_err();
        assert_eq!(err, "Missing 'paths' array");

        let err = parse_file_trigger_paths(&serde_json::json!({"paths": []})).unwrap_err();
        assert_eq!(err, "'paths' must contain at least one path");

        let err = parse_file_trigger_paths(&serde_json::json!({"paths": [1]})).unwrap_err();
        assert_eq!(err, "'paths' entries must be strings");
    }

    #[test]
    fn file_trigger_events_default_to_any() {
        let events = parse_file_trigger_events(&serde_json::json!({"events": []})).unwrap();
        assert_eq!(events, vec![FileEventKind::Any]);
    }

    #[test]
    fn file_trigger_events_parse_aliases() {
        let events =
            parse_file_trigger_events(&serde_json::json!({"events": ["write", "deleted"]}))
                .unwrap();
        assert_eq!(events, vec![FileEventKind::Modify, FileEventKind::Remove]);
    }
}
