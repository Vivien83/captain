use crate::project_runtime_runner::project_runtime_is_running;
use crate::project_runtime_status::{
    limit_project_runtime_attention_items, project_runtime_needs_operator_attention,
    project_runtime_operator_status,
};
use crate::state::AppState;
use crate::status_automation::build_automation_delivery_status;
use captain_kernel::goals::GoalStatus;
use captain_memory::project;
use captain_runtime::native_embeddings::NativeEmbeddingsStatus;
use captain_runtime::native_voice::NativeVoiceStatus;
use captain_types::agent::ResourceQuota;
use captain_types::config::{KernelConfig, TtsConfig};
use chrono::{DateTime, Utc};
use std::path::PathBuf;

pub(crate) struct StatusWorkloadSnapshot {
    pub(crate) workload: serde_json::Value,
    pub(crate) all_project_attention: Vec<serde_json::Value>,
    pub(crate) goal_active: usize,
    pub(crate) goal_escalated: usize,
}

pub(crate) struct StatusAuthSnapshot {
    pub(crate) auth_enabled: bool,
    pub(crate) auth_mode: &'static str,
    pub(crate) api_key_configured: bool,
    pub(crate) session_auth_enabled: bool,
}

pub(crate) struct StatusMediaSnapshot {
    pub(crate) tts: serde_json::Value,
    pub(crate) media: serde_json::Value,
    pub(crate) native_voice: NativeVoiceStatus,
    pub(crate) native_embeddings: NativeEmbeddingsStatus,
}

#[derive(Debug, Default, PartialEq, Eq)]
struct ProjectStatusCounts {
    planning: usize,
    active: usize,
    paused: usize,
    done: usize,
}

pub(crate) fn build_status_agents(state: &AppState) -> Vec<serde_json::Value> {
    let mut agents: Vec<serde_json::Value> = state
        .kernel
        .registry
        .list()
        .iter()
        .map(|entry| {
            serde_json::json!({
                "id": entry.id.to_string(),
                "name": &entry.name,
                "state": format!("{:?}", entry.state),
                "mode": &entry.mode,
                "created_at": entry.created_at.to_rfc3339(),
                "model_provider": &entry.manifest.model.provider,
                "model_name": &entry.manifest.model.model,
                "profile": &entry.manifest.profile,
            })
        })
        .collect();
    agents.sort_by(|a, b| {
        let a_name = a["name"].as_str().unwrap_or("").to_ascii_lowercase();
        let b_name = b["name"].as_str().unwrap_or("").to_ascii_lowercase();
        let a_rank = if a_name == "captain" { 0 } else { 1 };
        let b_rank = if b_name == "captain" { 0 } else { 1 };
        a_rank.cmp(&b_rank).then_with(|| a_name.cmp(&b_name))
    });
    agents
}

pub(crate) fn build_status_budget(state: &AppState) -> serde_json::Value {
    let global = serde_json::to_value(
        state
            .kernel
            .metering
            .budget_status(&state.kernel.config.budget),
    )
    .unwrap_or_default();
    let mut total_tokens_used = 0u64;
    let mut limited_agents = 0usize;
    let mut agents = Vec::new();
    let provider_subscriptions = crate::provider_quota_status::build_provider_subscription_status(
        state.kernel.memory.provider_quotas(),
    );

    for entry in state.kernel.registry.list() {
        let hourly_usage = state.kernel.scheduler.get_hourly_usage(entry.id);
        let tokens_used = hourly_usage
            .as_ref()
            .map(|usage| usage.total_tokens)
            .unwrap_or(0);
        let tool_calls = hourly_usage
            .as_ref()
            .map(|usage| usage.tool_calls)
            .unwrap_or(0);
        let resets_at = hourly_usage.and_then(|usage| usage.resets_at);
        total_tokens_used = total_tokens_used.saturating_add(tokens_used);
        if entry.manifest.resources.max_llm_tokens_per_hour > 0 {
            limited_agents += 1;
        }
        agents.push(agent_budget_status_entry(
            &entry.id.to_string(),
            &entry.name,
            &entry.manifest.resources,
            tokens_used,
            tool_calls,
            resets_at,
        ));
    }

    let operator_actions = budget_operator_actions(&global, &agents, &provider_subscriptions);

    serde_json::json!({
        "global": global,
        "agents": agents,
        "total_tokens_used": total_tokens_used,
        "limited_agents": limited_agents,
        "provider_subscriptions": provider_subscriptions,
        "operator_actions": operator_actions,
    })
}

fn agent_budget_status_entry(
    agent_id: &str,
    name: &str,
    quota: &ResourceQuota,
    tokens_used: u64,
    tool_calls: u64,
    resets_at: Option<DateTime<Utc>>,
) -> serde_json::Value {
    serde_json::json!({
        "agent_id": agent_id,
        "name": name,
        "tokens": {
            "used": tokens_used,
            "limit": quota.max_llm_tokens_per_hour,
            "window_seconds": 3600,
            "resets_at": resets_at,
            "pct": ratio_u64(tokens_used, quota.max_llm_tokens_per_hour),
        },
        "tool_calls": {
            "used": tool_calls,
            "limit_per_minute": quota.max_tool_calls_per_minute,
        },
        "resources": {
            "max_memory_bytes": quota.max_memory_bytes,
            "max_cpu_time_ms": quota.max_cpu_time_ms,
            "max_network_bytes_per_hour": quota.max_network_bytes_per_hour,
            "max_cost_per_hour_usd": quota.max_cost_per_hour_usd,
            "max_cost_per_day_usd": quota.max_cost_per_day_usd,
            "max_cost_per_month_usd": quota.max_cost_per_month_usd,
        },
    })
}

fn ratio_u64(used: u64, limit: u64) -> f64 {
    if limit > 0 {
        used as f64 / limit as f64
    } else {
        0.0
    }
}

fn budget_operator_actions(
    global: &serde_json::Value,
    agents: &[serde_json::Value],
    provider_subscriptions: &serde_json::Value,
) -> Vec<String> {
    let threshold = global["alert_threshold"].as_f64().unwrap_or(0.8);
    let mut actions = Vec::new();
    for (label, pct_key, limit_key) in [
        ("hourly", "hourly_pct", "hourly_limit"),
        ("daily", "daily_pct", "daily_limit"),
        ("monthly", "monthly_pct", "monthly_limit"),
    ] {
        let pct = global[pct_key].as_f64().unwrap_or(0.0);
        let limit = global[limit_key].as_f64().unwrap_or(0.0);
        if limit > 0.0 && pct >= threshold {
            actions.push(format!(
                "Global {label} budget is at {:.1}%; inspect `captain status --verbose` and reduce active runs or raise the limit.",
                pct * 100.0
            ));
        }
    }
    for agent in agents {
        let pct = agent["tokens"]["pct"].as_f64().unwrap_or(0.0);
        let limit = agent["tokens"]["limit"].as_u64().unwrap_or(0);
        if limit > 0 && pct >= threshold {
            let id = agent["agent_id"].as_str().unwrap_or("?");
            let name = agent["name"].as_str().unwrap_or("agent");
            actions.push(format!(
                "Agent {name} token budget is at {:.1}%; run `captain agent caps {id}` before delegating more work.",
                pct * 100.0
            ));
        }
    }
    match provider_subscriptions["state"].as_str().unwrap_or("unavailable") {
        "exhausted" => {
            let item = most_severe_provider_quota(provider_subscriptions);
            let name = item
                .and_then(|value| value["limit_name"].as_str())
                .or_else(|| item.and_then(|value| value["limit_id"].as_str()))
                .unwrap_or("provider");
            let reset = item
                .and_then(provider_quota_reset)
                .map(|value| format!(" Retry after {value}."))
                .unwrap_or_default();
            actions.push(format!(
                "Provider subscription quota {name} is exhausted.{reset} Captain cannot reset this provider-owned allowance."
            ));
        }
        "critical" | "warning" => {
            if let Some(item) = most_severe_provider_quota(provider_subscriptions) {
                let name = item["limit_name"]
                    .as_str()
                    .or_else(|| item["limit_id"].as_str())
                    .unwrap_or("provider");
                let used = provider_quota_max_used(item);
                actions.push(format!(
                    "Provider subscription quota {name} is at {used:.1}%; inspect the provider-reported reset before starting long work."
                ));
            }
        }
        "stale" => actions.push(
            "Provider subscription quota data is stale; verify the Codex session and network before relying on the displayed allowance."
                .to_string(),
        ),
        _ => {}
    }
    actions
}

fn most_severe_provider_quota(status: &serde_json::Value) -> Option<&serde_json::Value> {
    status["items"].as_array()?.iter().max_by_key(|item| {
        match item["alert_level"].as_str().unwrap_or("normal") {
            "exhausted" => 3,
            "critical" => 2,
            "warning" => 1,
            _ => 0,
        }
    })
}

fn provider_quota_max_used(item: &serde_json::Value) -> f64 {
    ["primary", "secondary"]
        .iter()
        .filter_map(|window| item[*window]["used_percent"].as_f64())
        .fold(0.0, f64::max)
}

fn provider_quota_reset(item: &serde_json::Value) -> Option<&str> {
    ["primary", "secondary"]
        .iter()
        .filter(|window| item[**window]["used_percent"].as_f64().unwrap_or(0.0) >= 100.0)
        .filter_map(|window| item[*window]["resets_at"].as_str())
        .min()
}

pub(crate) fn build_active_runs(state: &AppState, now: DateTime<Utc>) -> Vec<serde_json::Value> {
    let registry_entries = state.kernel.registry.list();
    let mut active_runs: Vec<serde_json::Value> = state
        .kernel
        .running_tasks
        .iter()
        .map(|entry| {
            let agent_id = *entry.key();
            let task = entry.value();
            let agent = registry_entries
                .iter()
                .find(|registered| registered.id == agent_id);
            let profile = agent
                .and_then(|entry| entry.manifest.profile.as_ref())
                .map(|profile| format!("{profile:?}"))
                .unwrap_or_else(|| "?".to_string());
            let age_seconds = now
                .signed_duration_since(task.started_at)
                .num_seconds()
                .max(0);
            serde_json::json!({
                "agent_id": agent_id.to_string(),
                "agent_name": agent.map(|entry| entry.name.as_str()).unwrap_or("?"),
                "run_id": task.run_id.to_string(),
                "started_at": task.started_at.to_rfc3339(),
                "age_seconds": age_seconds,
                "state": "running",
                "model_provider": agent.map(|entry| entry.manifest.model.provider.as_str()).unwrap_or("?"),
                "model_name": agent.map(|entry| entry.manifest.model.model.as_str()).unwrap_or("?"),
                "profile": profile,
            })
        })
        .collect();
    active_runs.sort_by(|a, b| {
        b["age_seconds"]
            .as_i64()
            .unwrap_or(0)
            .cmp(&a["age_seconds"].as_i64().unwrap_or(0))
    });
    active_runs
}

pub(crate) fn status_workspace_dirs(config: &KernelConfig) -> (PathBuf, PathBuf) {
    let workspaces_dir = config
        .workspaces_dir
        .clone()
        .unwrap_or_else(|| config.home_dir.join("workspaces"));
    let workflows_dir = config
        .workflows_dir
        .clone()
        .unwrap_or_else(|| config.home_dir.join("workflows"));
    (workspaces_dir, workflows_dir)
}

pub(crate) fn build_status_auth(config: &KernelConfig) -> StatusAuthSnapshot {
    let api_key_configured = !config.api_key.trim().is_empty();
    let session_auth_enabled = config.auth.enabled;
    let auth_mode = match (api_key_configured, session_auth_enabled) {
        (true, true) => "api_key+session",
        (true, false) => "api_key",
        (false, true) => "session",
        (false, false) => "none",
    };
    StatusAuthSnapshot {
        auth_enabled: api_key_configured || session_auth_enabled,
        auth_mode,
        api_key_configured,
        session_auth_enabled,
    }
}

pub(crate) fn build_deployment_status(config: &KernelConfig) -> serde_json::Value {
    serde_json::json!({
        "profile": config.deployment.profile,
        "public_url": config.deployment.public_url,
        "https": config.deployment.https,
        "reverse_proxy": config.deployment.reverse_proxy,
    })
}

pub(crate) fn build_status_media(
    config: &KernelConfig,
    tts_config: TtsConfig,
) -> StatusMediaSnapshot {
    let tts_provider = tts_config.provider.clone().unwrap_or_else(|| {
        if tts_config.enabled {
            "auto".to_string()
        } else {
            "off".to_string()
        }
    });
    let tts_voice = match tts_provider.to_ascii_lowercase().as_str() {
        "elevenlabs" => tts_config.elevenlabs.voice_id.clone(),
        "openai" => tts_config.openai.voice.clone(),
        "local-native" => captain_runtime::native_voice::status()
            .tts_engine
            .unwrap_or("pending")
            .to_string(),
        _ => String::new(),
    };
    let native_voice = captain_runtime::native_voice::status();
    let native_embeddings = captain_runtime::native_embeddings::status();

    StatusMediaSnapshot {
        tts: serde_json::json!({
            "enabled": tts_config.enabled,
            "provider": tts_provider,
            "voice": tts_voice,
            "local_native": {
                "preferred_engine": &tts_config.local_native.preferred_engine,
                "fallback_engine": &tts_config.local_native.fallback_engine,
                "language": &tts_config.local_native.language,
            },
            "max_text_length": tts_config.max_text_length,
        }),
        media: serde_json::json!({
            "image_description": config.media.image_description,
            "audio_transcription": config.media.audio_transcription,
            "video_description": config.media.video_description,
            "image_provider": config.media.image_provider,
            "audio_provider": config.media.audio_provider,
            "audio_model": config.media.audio_model,
            "audio_effective_provider": if config.media.audio_provider.is_some() {
                config.media.audio_provider.clone()
            } else if native_voice.stt_ready {
                Some(captain_runtime::native_voice::WHISPER_PROVIDER.to_string())
            } else {
                None
            },
        }),
        native_voice,
        native_embeddings,
    }
}

pub(crate) fn build_status_workload(
    state: &AppState,
    now: DateTime<Utc>,
) -> StatusWorkloadSnapshot {
    let projects = state.kernel.memory.project_list(false).unwrap_or_default();
    let counts = project_status_counts(&projects);
    let latest_projects = latest_project_status_items(projects.clone());
    let mut project_attention = project_attention_statuses(&projects);
    let all_project_attention = project_attention.clone();
    let project_attention_count = limit_project_runtime_attention_items(&mut project_attention);

    let goals = state.kernel.goal_store.list();
    let goal_total = goals.len();
    let goal_active = state.kernel.goal_store.list_active().len();
    let goal_paused = goals
        .iter()
        .filter(|goal| matches!(goal.status, GoalStatus::Paused))
        .count();
    let goal_escalated = goals
        .iter()
        .filter(|goal| matches!(goal.status, GoalStatus::Escalated))
        .count();

    StatusWorkloadSnapshot {
        workload: serde_json::json!({
            "projects": {
                "total": projects.len(),
                "planning": counts.planning,
                "active": counts.active,
                "paused": counts.paused,
                "done": counts.done,
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
            "automation": build_status_automation(state, now),
        }),
        all_project_attention,
        goal_active,
        goal_escalated,
    }
}

fn build_status_automation(state: &AppState, now: DateTime<Utc>) -> serde_json::Value {
    let cron_metas = state.kernel.cron_scheduler.list_all_jobs_with_meta();
    let cron_enabled = cron_metas.iter().filter(|meta| meta.job.enabled).count();
    let cron_due = cron_metas
        .iter()
        .filter(|meta| {
            meta.job.enabled
                && meta
                    .job
                    .next_run
                    .map(|next_run| next_run <= now)
                    .unwrap_or(false)
        })
        .count();
    let triggers = state.kernel.list_triggers(None);
    let trigger_enabled = triggers.iter().filter(|trigger| trigger.enabled).count();
    let file_triggers = state.kernel.list_file_change_triggers(None);
    let file_trigger_enabled = file_triggers
        .iter()
        .filter(|trigger| trigger.enabled)
        .count();

    serde_json::json!({
        "cron_jobs": cron_metas.len(),
        "cron_enabled": cron_enabled,
        "cron_due": cron_due,
        "delivery": build_automation_delivery_status(&cron_metas, now),
        "triggers": triggers.len(),
        "triggers_enabled": trigger_enabled,
        "file_triggers": file_triggers.len(),
        "file_triggers_enabled": file_trigger_enabled,
    })
}

fn project_status_counts(projects: &[project::Project]) -> ProjectStatusCounts {
    let mut counts = ProjectStatusCounts::default();
    for project in projects {
        match project.status {
            project::ProjectStatus::Planning => counts.planning += 1,
            project::ProjectStatus::Active => counts.active += 1,
            project::ProjectStatus::Paused => counts.paused += 1,
            project::ProjectStatus::Done => counts.done += 1,
            project::ProjectStatus::Archived => {}
        }
    }
    counts
}

fn latest_project_status_items(mut projects: Vec<project::Project>) -> Vec<serde_json::Value> {
    projects.sort_by_key(|p| std::cmp::Reverse(p.updated_at));
    projects
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
        .collect()
}

fn project_attention_statuses(projects: &[project::Project]) -> Vec<serde_json::Value> {
    projects
        .iter()
        .filter_map(|project| {
            let runtime = project
                .metadata
                .get("runtime")
                .filter(|value| value.is_object())?;
            let status = project_runtime_operator_status(
                project,
                runtime,
                project_runtime_is_running(&project.id),
            );
            project_runtime_needs_operator_attention(&status).then_some(status)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn project_with_status(status: project::ProjectStatus) -> project::Project {
        project::Project {
            id: format!("project-{}", status.as_str()),
            name: "Demo".to_string(),
            slug: format!("demo-{}", status.as_str()),
            goal: "Keep status counts stable".to_string(),
            status,
            deadline: None,
            created_at: 1,
            updated_at: 1,
            metadata: serde_json::json!({}),
        }
    }

    #[test]
    fn project_status_counts_ignores_archived_projects() {
        let projects = vec![
            project_with_status(project::ProjectStatus::Planning),
            project_with_status(project::ProjectStatus::Active),
            project_with_status(project::ProjectStatus::Paused),
            project_with_status(project::ProjectStatus::Done),
            project_with_status(project::ProjectStatus::Archived),
        ];

        assert_eq!(
            project_status_counts(&projects),
            ProjectStatusCounts {
                planning: 1,
                active: 1,
                paused: 1,
                done: 1,
            }
        );
    }

    #[test]
    fn agent_budget_status_entry_reports_live_token_ratio() {
        let quota = ResourceQuota {
            max_llm_tokens_per_hour: 1_000,
            max_tool_calls_per_minute: 25,
            ..Default::default()
        };

        let reset = Utc::now();
        let entry = agent_budget_status_entry("agent-1", "worker", &quota, 250, 7, Some(reset));

        assert_eq!(entry["tokens"]["used"], serde_json::json!(250));
        assert_eq!(entry["tokens"]["limit"], serde_json::json!(1_000));
        assert_eq!(entry["tokens"]["pct"], serde_json::json!(0.25));
        assert_eq!(entry["tokens"]["window_seconds"], serde_json::json!(3600));
        assert_eq!(entry["tokens"]["resets_at"], serde_json::json!(reset));
        assert_eq!(entry["tool_calls"]["used"], serde_json::json!(7));
        assert_eq!(
            entry["tool_calls"]["limit_per_minute"],
            serde_json::json!(25)
        );
    }

    #[test]
    fn budget_operator_actions_point_to_agent_caps_for_hot_agents() {
        let quota = ResourceQuota {
            max_llm_tokens_per_hour: 100,
            ..Default::default()
        };
        let agent = agent_budget_status_entry("agent-1", "worker", &quota, 95, 0, None);
        let global = serde_json::json!({
            "hourly_pct": 0.0,
            "hourly_limit": 0.0,
            "daily_pct": 0.0,
            "daily_limit": 0.0,
            "monthly_pct": 0.0,
            "monthly_limit": 0.0,
            "alert_threshold": 0.8
        });

        let provider = serde_json::json!({"state": "unavailable", "items": []});
        let actions = budget_operator_actions(&global, &[agent], &provider);

        assert_eq!(actions.len(), 1);
        assert!(actions[0].contains("captain agent caps agent-1"));
    }

    #[test]
    fn budget_operator_actions_report_provider_exhaustion_and_reset() {
        let global = serde_json::json!({"alert_threshold": 0.8});
        let provider = serde_json::json!({
            "state": "exhausted",
            "items": [{
                "limit_id": "codex",
                "limit_name": "Codex",
                "alert_level": "exhausted",
                "primary": {"used_percent": 100.0, "resets_at": "2026-07-18T18:00:00Z"}
            }]
        });

        let actions = budget_operator_actions(&global, &[], &provider);

        assert_eq!(actions.len(), 1);
        assert!(actions[0].contains("Codex is exhausted"));
        assert!(actions[0].contains("2026-07-18T18:00:00Z"));
        assert!(actions[0].contains("cannot reset"));
    }
}
