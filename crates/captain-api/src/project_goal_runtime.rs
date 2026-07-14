use crate::project_naming::slugify_project_name;
use crate::routes::AppState;
use captain_kernel::goals::{EscalationTarget, Goal, GoalStatus};
use captain_memory::project;
use chrono::Utc;
use std::collections::VecDeque;
use std::sync::Arc;

#[allow(clippy::too_many_arguments)]
pub(crate) fn build_project_goal(
    state: &AppState,
    project: &project::Project,
    id: Option<String>,
    name: Option<String>,
    description: Option<String>,
    check_command: String,
    recovery_command: Option<String>,
    interval_secs: Option<u64>,
    escalation_threshold: Option<u32>,
    max_llm_calls_per_hour: Option<u32>,
    escalation_channel: Option<EscalationTarget>,
) -> Goal {
    let name = name
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| format!("{} goal", project.name));
    let id = id
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| unique_project_goal_id(state, &project.slug, &name));
    let now = Utc::now();
    Goal {
        id,
        name,
        description: description
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| project.goal.clone()),
        project_id: Some(project.id.clone()),
        project_slug: Some(project.slug.clone()),
        status: GoalStatus::Active,
        interval_secs: interval_secs.unwrap_or(300),
        check_command,
        recovery_command,
        escalation_threshold: escalation_threshold.unwrap_or(3),
        max_llm_calls_per_hour: max_llm_calls_per_hour.unwrap_or(20),
        escalation_channel,
        created_at: now,
        updated_at: now,
        last_check_ts: None,
        consecutive_fails: 0,
        escalated_at: None,
        recent_checks: VecDeque::new(),
        llm_call_log: Vec::new(),
        suggestions: Vec::new(),
    }
}

pub(crate) fn add_project_goal(state: &AppState, goal: Goal) -> Result<Goal, String> {
    goal.validate().map_err(|e| e.to_string())?;
    let id = goal.id.clone();
    state
        .kernel
        .goal_store
        .add(goal)
        .map_err(|e| e.to_string())?;
    spawn_project_goal_loop(state, id.clone());
    state
        .kernel
        .goal_store
        .get(&id)
        .ok_or_else(|| format!("goal '{id}' was not persisted"))
}

pub(crate) fn spawn_project_goal_loop(state: &AppState, id: String) {
    let kh: Arc<dyn captain_runtime::kernel_handle::KernelHandle> = state.kernel.clone();
    let ops: Arc<dyn captain_runtime::goal_loop::GoalLoopOps> =
        Arc::new(captain_runtime::goal_loop::KernelOps { kh });
    captain_runtime::goal_loop::spawn_goal_loop_for(id, ops);
}

fn unique_project_goal_id(state: &AppState, project_slug: &str, name: &str) -> String {
    let base_name = slugify_project_name(name).replace('-', "_");
    let project_prefix = project_slug.replace('-', "_");
    let mut base = format!("{project_prefix}_{base_name}");
    if base.len() > 58 {
        base.truncate(58);
        base = base.trim_end_matches('_').to_string();
    }
    if base.len() < 3 {
        base = format!("{project_prefix}_goal");
    }
    if state.kernel.goal_store.get(&base).is_none() {
        return base;
    }
    for idx in 2..100 {
        let suffix = format!("_{idx}");
        let mut candidate = base.clone();
        if candidate.len() + suffix.len() > 64 {
            candidate.truncate(64 - suffix.len());
            candidate = candidate.trim_end_matches('_').to_string();
        }
        candidate.push_str(&suffix);
        if state.kernel.goal_store.get(&candidate).is_none() {
            return candidate;
        }
    }
    format!("goal_{}", Utc::now().timestamp_millis())
}

#[cfg(test)]
mod tests {
    use super::*;
    use captain_kernel::CaptainKernel;
    use captain_memory::project::ProjectStatus;
    use captain_types::config::{DefaultModelConfig, KernelConfig};
    use std::time::Instant;

    fn test_state() -> (tempfile::TempDir, AppState) {
        let tmp = tempfile::tempdir().unwrap();
        let config = KernelConfig {
            home_dir: tmp.path().to_path_buf(),
            data_dir: tmp.path().join("data"),
            default_model: DefaultModelConfig {
                provider: "ollama".to_string(),
                model: "test-model".to_string(),
                api_key_env: "OLLAMA_API_KEY".to_string(),
                base_url: None,
            },
            ..KernelConfig::default()
        };
        let kernel = Arc::new(CaptainKernel::boot_with_config(config).unwrap());
        kernel.set_self_handle();
        let state = AppState {
            kernel,
            started_at: Instant::now(),
            peer_registry: None,
            bridge_manager: tokio::sync::Mutex::new(None),
            channels_config: tokio::sync::RwLock::new(Default::default()),
            shutdown_notify: Arc::new(tokio::sync::Notify::new()),
            clawhub_cache: dashmap::DashMap::new(),
            ask_user_channels: dashmap::DashMap::new(),
            provider_probe_cache: captain_runtime::provider_health::ProbeCache::new(),
        };
        (tmp, state)
    }

    fn project() -> project::Project {
        project::Project {
            id: "project-1".to_string(),
            name: "Demo Project".to_string(),
            slug: "demo-project".to_string(),
            goal: "Ship safely".to_string(),
            status: ProjectStatus::Active,
            deadline: None,
            created_at: 0,
            updated_at: 0,
            metadata: serde_json::json!({}),
        }
    }

    #[test]
    fn build_project_goal_applies_project_defaults() {
        let (_tmp, state) = test_state();
        let goal = build_project_goal(
            &state,
            &project(),
            None,
            Some(" Guard ".to_string()),
            Some(" ".to_string()),
            "cargo test".to_string(),
            None,
            None,
            None,
            None,
            None,
        );

        assert_eq!(goal.id, "demo_project_guard");
        assert_eq!(goal.name, "Guard");
        assert_eq!(goal.description, "Ship safely");
        assert_eq!(goal.project_id.as_deref(), Some("project-1"));
        assert_eq!(goal.project_slug.as_deref(), Some("demo-project"));
        assert_eq!(goal.status, GoalStatus::Active);
        assert_eq!(goal.interval_secs, 300);
        assert_eq!(goal.escalation_threshold, 3);
        assert_eq!(goal.max_llm_calls_per_hour, 20);
    }

    #[test]
    fn unique_project_goal_id_avoids_existing_ids() {
        let (_tmp, state) = test_state();
        let first = build_project_goal(
            &state,
            &project(),
            None,
            Some("Guard".to_string()),
            None,
            "cargo test".to_string(),
            None,
            None,
            None,
            None,
            None,
        );
        state.kernel.goal_store.add(first).unwrap();

        assert_eq!(
            unique_project_goal_id(&state, "demo-project", "Guard"),
            "demo_project_guard_2"
        );
    }
}
