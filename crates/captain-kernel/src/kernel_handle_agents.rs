use captain_runtime::agent_loop::with_turn_token_budget;
use captain_runtime::kernel_handle;
use captain_types::agent::{AgentEntry, AgentId, AgentManifest, AutoScaleConfig};
use serde_json::Value;

use super::kernel_agent_workspace::manager_domain_template;
use super::CaptainKernel;

impl CaptainKernel {
    /// Resolve an agent reference that may be a UUID or a human-readable
    /// name (e.g. "captain"), matching the fallback `agent_send` already
    /// used. Read-only introspection tools (`agent_status`, `agent_caps`,
    /// `session_tool_call_summary`) used to require a strict UUID only,
    /// which failed live ("Invalid agent ID") when an agent referred to
    /// itself by name instead of looking its own UUID up first.
    fn resolve_agent_id(&self, agent_ref: &str) -> Result<AgentId, String> {
        match agent_ref.parse() {
            Ok(id) => Ok(id),
            Err(_) => self
                .registry
                .find_by_name(agent_ref)
                .map(|e| e.id)
                .ok_or_else(|| format!("Agent not found: {agent_ref}")),
        }
    }

    pub(super) async fn handle_spawn_agent(
        &self,
        manifest_toml: &str,
        parent_id: Option<&str>,
    ) -> Result<(String, String), String> {
        let content_hash = captain_types::manifest_signing::hash_manifest(manifest_toml);
        tracing::debug!(hash = %content_hash, "Manifest SHA-256 computed for integrity tracking");

        let manifest: AgentManifest = match toml::from_str(manifest_toml) {
            Ok(manifest) => manifest,
            Err(e) => match captain_types::agent::repair_flat_model_fields(manifest_toml)
                .and_then(|repaired| toml::from_str(&repaired).ok())
            {
                Some(repaired_manifest) => repaired_manifest,
                None => {
                    return Err(captain_types::agent::format_agent_manifest_parse_error(
                        &e,
                        manifest_toml,
                    ))
                }
            },
        };
        let name = manifest.name.clone();
        let parent = parent_id.and_then(|pid| pid.parse::<AgentId>().ok());
        let id = match self.spawn_agent_with_parent(manifest, parent, None) {
            Ok(id) => id,
            Err(crate::error::KernelError::Captain(
                captain_types::error::CaptainError::AgentAlreadyExists(existing_name),
            )) => {
                return Err(self.agent_already_exists_recovery_message(&existing_name));
            }
            Err(e) => return Err(format!("Spawn failed: {e}")),
        };
        Ok((id.to_string(), name))
    }

    /// Build an actionable error for a name collision on spawn — the model
    /// hit this live and searched captain_docs for "agent_spawn error
    /// recovery" with zero results, then blindly retried with a slightly
    /// different name 30+ times instead of recovering sensibly. Naming the
    /// existing agent's id/state and the three real recovery paths directly
    /// in the error means no doc lookup is needed to know what to do next.
    fn agent_already_exists_recovery_message(&self, existing_name: &str) -> String {
        match self.registry.find_by_name(existing_name) {
            Some(entry) => format!(
                "Spawn failed: an agent named '{existing_name}' already exists \
                 (id: {}, state: {:?}). Pick one: (1) message it directly with \
                 send_to_agent/agent_send using id {} instead of spawning a new \
                 one, (2) call agent_kill on id {} first if you want to replace \
                 it, or (3) choose a genuinely different name only if this is \
                 meant to be a separate, independent agent.",
                entry.id, entry.state, entry.id, entry.id
            ),
            None => format!(
                "Spawn failed: an agent named '{existing_name}' already exists, \
                 but it could not be looked up by name (may have just been \
                 killed). Retry agent_list to find its current id, or pick a \
                 genuinely different name."
            ),
        }
    }

    pub(super) async fn handle_send_to_agent(
        &self,
        agent_id: &str,
        message: &str,
    ) -> Result<String, String> {
        let id = self.resolve_agent_id(agent_id)?;
        let result = self
            .send_message(id, message)
            .await
            .map_err(|e| format!("Send failed: {e}"))?;
        Ok(result.response)
    }

    pub(super) fn handle_list_agents(&self) -> Vec<kernel_handle::AgentInfo> {
        self.registry
            .list()
            .into_iter()
            .map(agent_info_from_entry)
            .collect()
    }

    pub(super) fn handle_kill_agent(&self, agent_id: &str) -> Result<(), String> {
        let id: AgentId = agent_id
            .parse()
            .map_err(|_| "Invalid agent ID".to_string())?;
        CaptainKernel::kill_agent(self, id).map_err(|e| format!("Kill failed: {e}"))
    }

    pub(super) async fn handle_create_manager(
        &self,
        name: &str,
        domain: &str,
        model: Option<&str>,
        budget_tokens: u64,
    ) -> Result<(String, String), String> {
        let model_str = model.unwrap_or(&self.config.default_model.model);
        let provider = &self.config.default_model.provider;
        let manifest_toml = manager_manifest_toml(name, domain, provider, model_str);

        let manifest: AgentManifest =
            toml::from_str(&manifest_toml).map_err(|e| format!("Invalid manifest: {e}"))?;
        let spawned_name = manifest.name.clone();
        let aid = self
            .spawn_agent_with_parent(manifest, None, None)
            .map_err(|e| format!("Spawn failed: {e}"))?;
        let id = aid.to_string();

        if budget_tokens > 0 {
            self.scheduler.set_hourly_quota(aid, budget_tokens);
        }

        let manager_prompt = manager_system_prompt(name, domain, budget_tokens);
        let _ = self
            .handle_inject_system_message(&id, &manager_prompt)
            .await;

        Ok((id, spawned_name))
    }

    pub(super) fn handle_list_managers(&self) -> Vec<Value> {
        self.registry
            .list()
            .into_iter()
            .filter(|e| e.tags.iter().any(|t| t == "manager"))
            .map(|e| {
                let worker_count = e.children.len();
                let usage = self.scheduler.get_agent_usage(e.id);
                let tokens = usage.map(|u| u.input_tokens + u.output_tokens).unwrap_or(0);
                serde_json::json!({
                    "id": e.id.to_string(),
                    "name": e.name,
                    "state": format!("{:?}", e.state),
                    "domain": e.manifest.description,
                    "model": format!("{}:{}", e.manifest.model.provider, e.manifest.model.model),
                    "workers": worker_count,
                    "tokens_used": tokens,
                    "last_active": e.last_active.to_rfc3339(),
                })
            })
            .collect()
    }

    pub(super) async fn handle_close_manager(&self, manager_id: &str) -> Result<u32, String> {
        let aid: AgentId = manager_id.parse().map_err(|_| "Invalid manager ID")?;
        let entry = self.registry.get(aid).ok_or("Manager not found")?;

        if !entry.tags.iter().any(|t| t == "manager") {
            return Err("Agent is not a manager".into());
        }

        let mut killed = 0u32;
        for child_id in &entry.children {
            let child_str = child_id.to_string();
            if CaptainKernel::kill_agent(self, *child_id).is_ok() {
                tracing::info!(child = %child_str, "Killed fleet worker");
                killed += 1;
            }
        }

        CaptainKernel::kill_agent(self, aid).map_err(|e| format!("Failed to kill manager: {e}"))?;
        killed += 1;

        Ok(killed)
    }

    pub(super) fn handle_set_manager_mission(
        &self,
        manager_id: &str,
        mission: Option<&str>,
    ) -> Result<(), String> {
        let aid: AgentId = manager_id.parse().map_err(|_| "Invalid manager ID")?;
        self.registry
            .set_mission(aid, mission.map(|s| s.to_string()))
            .map_err(|e| format!("{e}"))?;
        if let Some(entry) = self.registry.get(aid) {
            if let Err(e) = self.memory.save_agent(&entry) {
                tracing::warn!(agent = %entry.name, "Mission persist failed: {e}");
            }
        }
        Ok(())
    }

    pub(super) fn handle_configure_autoscale(
        &self,
        manager_id: &str,
        config: AutoScaleConfig,
    ) -> Result<(), String> {
        let aid: AgentId = manager_id.parse().map_err(|_| "Invalid manager ID")?;
        let entry_check = self.registry.get(aid).ok_or("Manager not found")?;
        if !entry_check.tags.iter().any(|t| t == "manager") {
            return Err("Agent is not a manager".into());
        }
        if config.max_workers < config.min_workers {
            return Err("max_workers must be >= min_workers".into());
        }
        if config.kill_threshold >= config.spawn_threshold {
            return Err("kill_threshold must be < spawn_threshold".into());
        }
        self.registry
            .set_autoscale(aid, Some(config))
            .map_err(|e| format!("{e}"))?;
        if let Some(entry) = self.registry.get(aid) {
            if let Err(e) = self.memory.save_agent(&entry) {
                tracing::warn!(agent = %entry.name, "Autoscale persist failed: {e}");
            }
        }
        Ok(())
    }

    pub(super) fn handle_fleet_metrics(&self, manager_id: &str) -> Result<Value, String> {
        let aid: AgentId = manager_id.parse().map_err(|_| "Invalid manager ID")?;
        let entry = self.registry.get(aid).ok_or("Manager not found")?;
        let metrics = self
            .fleet_load_metrics(aid)
            .ok_or("Unable to compute metrics")?;
        Ok(serde_json::json!({
            "manager_id": aid.to_string(),
            "name": entry.name,
            "active_workers": metrics.active_workers,
            "idle_workers": metrics.idle_workers,
            "total_workers": entry.children.len(),
            "queue_depth": metrics.queue_depth,
            "mission": entry.mission,
            "mission_set_at": entry.mission_set_at.map(|t| t.to_rfc3339()),
            "autoscale": entry.autoscale,
            "last_scale_event": entry.last_scale_event.map(|t| t.to_rfc3339()),
            "tokens_used_last_window": metrics.tokens_used_last_window,
        }))
    }

    pub(super) fn handle_check_agent_quota(&self, agent_name: &str) -> Result<(), String> {
        let entry = self
            .registry
            .list()
            .into_iter()
            .find(|e| e.name == agent_name);
        if let Some(entry) = entry {
            self.scheduler
                .check_quota(entry.id)
                .map_err(|e| format!("{e}"))
        } else {
            Ok(())
        }
    }

    pub(super) fn handle_agent_status_info(&self, agent_id: &str) -> Result<Value, String> {
        let aid = self.resolve_agent_id(agent_id)?;
        let entry = self.registry.get(aid).ok_or("Agent not found")?;
        let usage = self.scheduler.get_agent_usage(aid);
        let total_tokens = usage
            .as_ref()
            .map(|u| u.input_tokens + u.output_tokens)
            .unwrap_or(0);
        Ok(serde_json::json!({
            "name": entry.name,
            "state": format!("{:?}", entry.state),
            "model": format!("{}:{}", entry.manifest.model.provider, entry.manifest.model.model),
            "tokens_total": total_tokens,
            "last_active": entry.last_active.to_rfc3339(),
        }))
    }

    pub(super) async fn handle_agent_events(
        &self,
        agent_id: &str,
        limit: usize,
    ) -> Result<Vec<Value>, String> {
        let aid: AgentId = agent_id.parse().map_err(|_| "Invalid agent ID")?;
        let history = self.event_bus.history(limit).await;
        let filtered: Vec<Value> = history
            .into_iter()
            .filter(|e| e.source == aid)
            .map(|e| {
                serde_json::json!({
                    "id": e.id.to_string(),
                    "timestamp": e.timestamp.to_rfc3339(),
                    "payload": format!("{:?}", e.payload),
                })
            })
            .collect();
        Ok(filtered)
    }

    /// Full capability + budget report for an agent, combining what
    /// `captain agent caps` already computes over HTTP
    /// (`agent_lifecycle_routes::get_agent` + `usage_budget_routes::agent_budget_status`)
    /// into one in-process call. Added so an agent can introspect its own or
    /// another agent's real capabilities/budget without shelling out to the
    /// `captain` CLI binary, which isn't reachable from `shell_exec`'s
    /// sandbox (observed live: an agent asked to check `agent caps` looped
    /// through multiple failing `shell_exec` variants trying to invoke it).
    pub(super) fn handle_agent_capability_report(&self, agent_id: &str) -> Result<Value, String> {
        let aid = self.resolve_agent_id(agent_id)?;
        let entry = self
            .registry
            .get(aid)
            .ok_or_else(|| "Agent not found".to_string())?;

        let effective = captain_types::agent::effective_manifest_capabilities(&entry.manifest);
        let quota = &entry.manifest.resources;

        let usage_store = captain_memory::usage::UsageStore::new(self.memory.usage_conn());
        let hourly = usage_store.query_hourly(aid).unwrap_or(0.0);
        let daily = usage_store.query_daily(aid).unwrap_or(0.0);
        let monthly = usage_store.query_monthly(aid).unwrap_or(0.0);
        let tokens_used = self
            .scheduler
            .get_usage(aid)
            .map(|(tokens, _)| tokens)
            .unwrap_or(0);
        let cost_ratio = |spend: f64, limit: f64| if limit > 0.0 { spend / limit } else { 0.0 };

        Ok(serde_json::json!({
            "agent_id": agent_id,
            "agent_name": entry.name,
            "state": format!("{:?}", entry.state),
            "model": format!("{}:{}", entry.manifest.model.provider, entry.manifest.model.model),
            "capabilities_declared": entry.manifest.capabilities,
            "capabilities_effective": effective,
            "resources": quota,
            "budget": {
                "hourly": {
                    "spend": hourly,
                    "limit": quota.max_cost_per_hour_usd,
                    "pct": cost_ratio(hourly, quota.max_cost_per_hour_usd),
                },
                "daily": {
                    "spend": daily,
                    "limit": quota.max_cost_per_day_usd,
                    "pct": cost_ratio(daily, quota.max_cost_per_day_usd),
                },
                "monthly": {
                    "spend": monthly,
                    "limit": quota.max_cost_per_month_usd,
                    "pct": cost_ratio(monthly, quota.max_cost_per_month_usd),
                },
                "tokens": {
                    "used": tokens_used,
                    "limit": quota.max_llm_tokens_per_hour,
                    "pct": if quota.max_llm_tokens_per_hour > 0 {
                        tokens_used as f64 / quota.max_llm_tokens_per_hour as f64
                    } else {
                        0.0
                    },
                },
            },
        }))
    }

    /// Summarize which tools an agent's own current session actually called,
    /// sourced from the persisted session event log (the same substrate
    /// `captain replay`/`/api/sessions/{id}/events` reads). Lets an agent
    /// verify its own claims (e.g. before writing a self-test report)
    /// instead of asserting a capability was exercised without evidence.
    pub(super) fn handle_session_tool_call_summary(
        &self,
        agent_id: &str,
        limit: usize,
    ) -> Result<Value, String> {
        let aid = self.resolve_agent_id(agent_id)?;
        if self.registry.get(aid).is_none() {
            return Err("Agent not found".to_string());
        }
        // The durable event log keys entries by agent_id (see
        // `timeline.rs`/`kernel_compaction_runtime.rs`: `session_id =
        // agent_id.to_string()`), not by `AgentEntry.session_id` (a
        // different concept — the LLM conversation session). Using the
        // wrong key here silently returned `events_scanned: 0` on every
        // call, making the whole verification tool useless.
        let session_id = aid.to_string();

        let query = captain_memory::event_log::RangeQuery {
            session_id: session_id.clone(),
            from_ts: None,
            to_ts: None,
            limit: Some(limit.clamp(1, 2000)),
        };
        let events = self
            .memory
            .read_session_events_tail(&query)
            .map_err(|e| e.to_string())?;

        let mut summary = summarize_tool_call_events(&session_id, agent_id, &events);
        let recent_calls = {
            let mut history = RECENT_TOOL_CALL_SUMMARY_CALLS
                .entry(agent_id.to_string())
                .or_default();
            record_and_count_recent_calls(
                &mut history,
                chrono::Utc::now().timestamp_millis(),
                TOOL_CALL_SUMMARY_BURST_WINDOW_MS,
            )
        };
        if recent_calls > TOOL_CALL_SUMMARY_BURST_THRESHOLD {
            summary["note"] = serde_json::json!(format!(
                "Tu as appelé cet outil {recent_calls} fois dans la dernière minute. \
                 Appelle-le une fois par étape de test, ou juste avant de finaliser ton \
                 rapport — pas après chaque action."
            ));
        }
        Ok(summary)
    }

    pub(super) async fn handle_inject_system_message(
        &self,
        agent_id: &str,
        message: &str,
    ) -> Result<(), String> {
        let aid: AgentId = agent_id.parse().map_err(|_| "Invalid agent ID")?;
        self.handle_send_to_agent(&aid.to_string(), &format!("[SYSTEM CORRECTION] {message}"))
            .await?;
        Ok(())
    }

    pub(super) async fn handle_delegate_task(
        &self,
        agent_id: &str,
        task: &str,
        max_tokens: u64,
    ) -> Result<String, String> {
        let aid: AgentId = agent_id.parse().map_err(|_| "Invalid agent ID")?;
        let task_id = self
            .handle_task_post(task, task, Some(agent_id), None)
            .await?;
        let budget = (max_tokens > 0).then_some(max_tokens);
        let result = with_turn_token_budget(budget, self.send_message(aid, task))
            .await
            .map_err(|e| format!("Delegated send failed: {e}"))?;
        let response = result.response.clone();
        let used_tokens = result.total_usage.total();
        let budget_exceeded = max_tokens > 0 && used_tokens > max_tokens;
        let status = delegate_status(max_tokens, used_tokens);
        self.handle_task_complete(&task_id, &response).await?;
        Ok(serde_json::to_string_pretty(&serde_json::json!({
            "task_id": task_id.clone(),
            "agent_id": agent_id,
            "status": status,
            "budget_tokens": max_tokens,
            "used_tokens": used_tokens,
            "budget_exceeded": budget_exceeded,
            "result": response,
            "note": "agent_delegate is synchronous today. The run budget is scoped to this delegation and can stop further tool steps after the budget is reached; a single LLM call can still exceed the target before Captain can interrupt it."
        }))
        .unwrap_or_else(|_| format!("Task {task_id} completed.")))
    }

    pub(super) fn handle_find_agents(&self, query: &str) -> Vec<kernel_handle::AgentInfo> {
        let q = query.to_lowercase();
        self.registry
            .list()
            .into_iter()
            .filter(|entry| entry_matches_find_query(entry, &q))
            .map(agent_info_from_entry)
            .collect()
    }
}

fn agent_info_from_entry(entry: AgentEntry) -> kernel_handle::AgentInfo {
    kernel_handle::AgentInfo {
        id: entry.id.to_string(),
        name: entry.name.clone(),
        state: format!("{:?}", entry.state),
        model_provider: entry.manifest.model.provider.clone(),
        model_name: entry.manifest.model.model.clone(),
        description: entry.manifest.description.clone(),
        tags: entry.tags.clone(),
        tools: entry.manifest.capabilities.tools.clone(),
    }
}

/// Multi-term queries ("captain researcher-hand") match agents on ANY term
/// (OR) — a strict substring match on the whole string returned
/// "No agents found" for every multi-name query.
fn entry_matches_find_query(entry: &AgentEntry, query_lower: &str) -> bool {
    let terms: Vec<&str> = query_lower
        .split([' ', ','])
        .filter(|term| !term.is_empty())
        .collect();
    if terms.is_empty() {
        return agent_matches_query(entry, query_lower);
    }
    terms.iter().any(|term| agent_matches_query(entry, term))
}

fn agent_matches_query(entry: &AgentEntry, query_lower: &str) -> bool {
    let name_match = entry.name.to_lowercase().contains(query_lower);
    let tag_match = entry
        .tags
        .iter()
        .any(|tag| tag.to_lowercase().contains(query_lower));
    let tool_match = entry
        .manifest
        .capabilities
        .tools
        .iter()
        .any(|tool| tool.to_lowercase().contains(query_lower));
    let desc_match = entry
        .manifest
        .description
        .to_lowercase()
        .contains(query_lower);
    name_match || tag_match || tool_match || desc_match
}

fn manager_manifest_toml(name: &str, domain: &str, provider: &str, model: &str) -> String {
    format!(
        r#"name = "{name}"
description = "Fleet Manager — {domain}"
version = "1.0.0"
author = "captain"
module = "llm"
tags = ["manager", "fleet:{name}"]

[model]
provider = "{provider}"
model = "{model}"

[capabilities]
tools = ["agent_spawn", "agent_send", "agent_list", "agent_kill", "agent_status", "agent_watch", "agent_delegate", "agent_correct", "fleet_set_mission", "fleet_configure_autoscale", "fleet_metrics", "memory_store", "memory_recall", "task_post", "task_complete", "channel_send", "ask_user"]
"#
    )
}

fn manager_system_prompt(name: &str, domain: &str, budget_tokens: u64) -> String {
    let domain_guide = manager_domain_template(name, domain);
    format!(
        "[SYSTEM CORRECTION] Tu es le Manager de la flotte '{name}'.\n\
         Domaine : {domain}\n\
         Budget : {budget_tokens} tokens/heure\n\n\
         RÈGLES GÉNÉRALES :\n\
         - Tu es autonome. Captain te donne des missions, tu les exécutes.\n\
         - Spawn des workers avec agent_spawn pour les sous-tâches.\n\
         - Surveille avec agent_status et agent_watch.\n\
         - Corrige avec agent_correct si besoin.\n\
         - Kill les workers quand ils ont fini (agent_kill).\n\
         - Quand ta mission est finie, rapporte avec ce format :\n\
           [RAPPORT] Flotte: {name}\n\
           Status: terminé/en_cours/échec\n\
           Résultat: (résumé)\n\
           Workers: N spawned, N terminés\n\
           Tokens: N utilisés / {budget_tokens} budget\n\n\
         CHOIX DES MODÈLES WORKERS :\n\
         - Tâche simple (résumé, formatage, tri) → modèle rapide/pas cher\n\
         - Tâche complexe (analyse, raisonnement, code) → modèle intelligent\n\
         - Gros volume (batch, scraping) → modèle rapide haute capacité\n\
         Dans le manifest TOML du worker, mets [model] provider et model adaptés.\n\n\
         BUDGET :\n\
         - Ton budget total : {budget_tokens} tokens/heure.\n\
         - Répartis entre tes workers via agent_delegate(max_tokens=N).\n\
         - Si budget serré, fais toi-même au lieu de spawner.\n\n\
         {domain_guide}"
    )
}

fn delegate_status(max_tokens: u64, used_tokens: u64) -> &'static str {
    if max_tokens > 0 && used_tokens > max_tokens {
        "budget_exceeded"
    } else {
        "completed"
    }
}

/// Recent call timestamps (ms) per agent, used to detect and gently push
/// back on compulsive `session_tool_call_summary` re-checking — observed
/// live: 35 calls in under 6 minutes during a single test run instead of
/// once per test step.
static RECENT_TOOL_CALL_SUMMARY_CALLS: std::sync::LazyLock<dashmap::DashMap<String, Vec<i64>>> =
    std::sync::LazyLock::new(dashmap::DashMap::new);

const TOOL_CALL_SUMMARY_BURST_WINDOW_MS: i64 = 60_000;
const TOOL_CALL_SUMMARY_BURST_THRESHOLD: usize = 3;

/// Pure: prune timestamps outside the burst window, record `now_ms`, and
/// return the resulting count (including this call).
fn record_and_count_recent_calls(history: &mut Vec<i64>, now_ms: i64, window_ms: i64) -> usize {
    history.retain(|&ts| now_ms - ts <= window_ms);
    history.push(now_ms);
    history.len()
}

/// Pure aggregation used by `handle_session_tool_call_summary`. Only
/// `tool_execution_result` events count as "tested": a tool that was merely
/// selected (`tool_use_start`/`tool_use_end`) but never completed must not
/// be reported as exercised.
fn summarize_tool_call_events(
    session_id: &str,
    agent_id: &str,
    events: &[captain_memory::event_log::SessionEvent],
) -> Value {
    let mut call_counts: std::collections::BTreeMap<String, u32> =
        std::collections::BTreeMap::new();
    let mut calls: Vec<Value> = Vec::new();
    for event in events {
        if event.event_type != "tool_execution_result" {
            continue;
        }
        let Some(tool_name) = event
            .payload
            .get("name")
            .or_else(|| event.payload.get("tool"))
            .or_else(|| event.payload.get("tool_name"))
            .and_then(|v| v.as_str())
        else {
            continue;
        };
        *call_counts.entry(tool_name.to_string()).or_insert(0) += 1;
        calls.push(serde_json::json!({
            "tool_name": tool_name,
            "ts": event.ts,
            "is_error": event.payload.get("is_error").cloned().unwrap_or(Value::Bool(false)),
        }));
    }

    serde_json::json!({
        "session_id": session_id,
        "agent_id": agent_id,
        "events_scanned": events.len(),
        "distinct_tools_called": call_counts.keys().cloned().collect::<Vec<_>>(),
        "call_counts": call_counts,
        "calls": calls,
    })
}

#[cfg(test)]
mod tests {
    use captain_types::agent::{
        AgentEntry, AgentId, AgentMode, AgentState, ManifestCapabilities, SessionId,
    };

    use super::{
        agent_matches_query, delegate_status, entry_matches_find_query, manager_manifest_toml,
        record_and_count_recent_calls, summarize_tool_call_events,
    };
    use captain_memory::event_log::SessionEvent;

    fn test_entry() -> AgentEntry {
        let mut manifest = captain_types::agent::AgentManifest {
            name: "auditor".to_string(),
            description: "Security auditor".to_string(),
            ..Default::default()
        };
        manifest.capabilities = ManifestCapabilities {
            tools: vec!["shell_exec".to_string(), "memory_recall".to_string()],
            ..Default::default()
        };
        AgentEntry {
            id: AgentId::new(),
            name: "auditor".to_string(),
            manifest,
            state: AgentState::Running,
            mode: AgentMode::default(),
            created_at: chrono::Utc::now(),
            last_active: chrono::Utc::now(),
            parent: None,
            children: Vec::new(),
            session_id: SessionId::new(),
            tags: vec!["security".to_string(), "audit".to_string()],
            identity: Default::default(),
            onboarding_completed: false,
            onboarding_completed_at: None,
            mission: None,
            mission_set_at: None,
            autoscale: None,
            last_scale_event: None,
        }
    }

    #[tokio::test]
    async fn spawn_name_collision_error_names_the_existing_agent_and_recovery_options() {
        let tmp = tempfile::tempdir().unwrap();
        let home_dir = tmp.path().join("kernel-handle-agents-spawn-collision-test");
        let config = captain_types::config::KernelConfig {
            home_dir: home_dir.clone(),
            data_dir: home_dir.join("data"),
            ..captain_types::config::KernelConfig::default()
        };
        let kernel = crate::kernel::CaptainKernel::boot_with_config(config).expect("kernel boot");

        let mut manifest = captain_types::agent::AgentManifest {
            name: "dup-test".to_string(),
            description: "test agent".to_string(),
            module: "builtin:chat".to_string(),
            ..Default::default()
        };
        let manifest_toml = toml::to_string(&manifest).expect("manifest serializes to TOML");

        let (first_id, _) = kernel
            .handle_spawn_agent(&manifest_toml, None)
            .await
            .expect("first spawn with this name should succeed");

        manifest.description = "a different description, same name".to_string();
        let second_manifest_toml = toml::to_string(&manifest).unwrap();
        let err = kernel
            .handle_spawn_agent(&second_manifest_toml, None)
            .await
            .expect_err("second spawn with the same name must be rejected");

        assert!(
            err.contains(&first_id),
            "error should name the existing agent's id: {err}"
        );
        assert!(
            err.contains("send_to_agent") || err.contains("agent_send"),
            "got: {err}"
        );
        assert!(err.contains("agent_kill"), "got: {err}");
        assert!(
            !err.contains("agent_spawn error recovery"),
            "must not send the model back to a docs lookup that returns nothing: {err}"
        );
    }

    #[test]
    fn agent_query_matches_name_tags_tools_and_description() {
        let entry = test_entry();

        assert!(agent_matches_query(&entry, "audit"));
        assert!(agent_matches_query(&entry, "security"));
        assert!(agent_matches_query(&entry, "shell"));
        assert!(agent_matches_query(&entry, "auditor"));
        assert!(!agent_matches_query(&entry, "calendar"));
    }

    /// The live bug: agent_find("captain researcher-hand agent-as-service")
    /// returned "No agents found" while each name alone matched. Multi-term
    /// queries must OR their terms.
    #[test]
    fn multi_term_find_query_matches_any_term() {
        let entry = test_entry();

        assert!(entry_matches_find_query(&entry, "auditor other-agent"));
        assert!(entry_matches_find_query(&entry, "other-agent auditor"));
        assert!(entry_matches_find_query(&entry, "calendar,auditor"));
        assert!(!entry_matches_find_query(&entry, "calendar scheduler"));
        // Single term behaves exactly as before.
        assert!(entry_matches_find_query(&entry, "auditor"));
        assert!(!entry_matches_find_query(&entry, "calendar"));
        // Empty query keeps the legacy match-all behavior.
        assert!(entry_matches_find_query(&entry, ""));
    }

    #[test]
    fn manager_manifest_keeps_required_fleet_tools_and_model() {
        let manifest = manager_manifest_toml("ops", "operations", "codex", "gpt-5");

        assert!(manifest.contains("name = \"ops\""));
        assert!(manifest.contains("provider = \"codex\""));
        assert!(manifest.contains("model = \"gpt-5\""));
        assert!(manifest.contains("fleet_configure_autoscale"));
        assert!(manifest.contains("agent_delegate"));
    }

    #[test]
    fn delegate_status_reports_budget_exceeded_only_after_limit() {
        assert_eq!(delegate_status(0, 10_000), "completed");
        assert_eq!(delegate_status(100, 100), "completed");
        assert_eq!(delegate_status(100, 101), "budget_exceeded");
    }

    fn event(id: i64, event_type: &str, payload: serde_json::Value) -> SessionEvent {
        SessionEvent {
            id,
            session_id: "sess-1".to_string(),
            ts: 1_000 + id,
            event_type: event_type.to_string(),
            payload,
        }
    }

    #[test]
    fn summarize_tool_call_events_only_counts_completed_calls() {
        let events = vec![
            // Selected but never completed: must NOT count as tested.
            event(
                1,
                "tool_use_end",
                serde_json::json!({"name": "speech_to_text"}),
            ),
            event(
                2,
                "tool_execution_result",
                serde_json::json!({"name": "file_write", "is_error": false}),
            ),
            event(
                3,
                "tool_execution_result",
                serde_json::json!({"name": "file_write", "is_error": false}),
            ),
        ];

        let summary = summarize_tool_call_events("sess-1", "agent-1", &events);

        assert_eq!(
            summary["distinct_tools_called"],
            serde_json::json!(["file_write"])
        );
        assert_eq!(summary["call_counts"]["file_write"], 2);
        assert_eq!(summary["events_scanned"], 3);
        assert!(!summary["distinct_tools_called"]
            .as_array()
            .unwrap()
            .iter()
            .any(|v| v == "speech_to_text"));
    }

    #[test]
    fn summarize_tool_call_events_empty_session_reports_no_claims() {
        let summary = summarize_tool_call_events("sess-2", "agent-2", &[]);

        assert_eq!(summary["distinct_tools_called"], serde_json::json!([]));
        assert_eq!(summary["events_scanned"], 0);
    }

    #[test]
    fn record_and_count_recent_calls_prunes_outside_window() {
        let mut history = vec![0, 10_000, 40_000];
        // now=65_000, window=60_000: only 10_000 and 40_000 survive (>=5_000),
        // plus this new call.
        let count = record_and_count_recent_calls(&mut history, 65_000, 60_000);
        assert_eq!(count, 3);
        assert_eq!(history, vec![10_000, 40_000, 65_000]);
    }

    #[test]
    fn record_and_count_recent_calls_starts_from_empty() {
        let mut history = Vec::new();
        assert_eq!(
            record_and_count_recent_calls(&mut history, 1_000, 60_000),
            1
        );
        assert_eq!(
            record_and_count_recent_calls(&mut history, 2_000, 60_000),
            2
        );
        assert_eq!(
            record_and_count_recent_calls(&mut history, 2_500, 60_000),
            3
        );
    }
}
