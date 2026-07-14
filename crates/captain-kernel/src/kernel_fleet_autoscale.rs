use captain_types::agent::{
    AgentEntry, AgentId, AgentManifest, AgentState, AutoScaleConfig, ModelConfig,
};
use std::collections::HashSet;

use super::CaptainKernel;

impl CaptainKernel {
    /// Count fleet workers by rough activity (active = Running + last_active < 60s).
    pub(crate) fn count_workers_by_state(&self, children: &[AgentId]) -> (u32, u32) {
        let now = chrono::Utc::now();
        let mut active = 0u32;
        let mut idle = 0u32;
        for cid in children {
            if let Some(entry) = self.registry.get(*cid) {
                if matches!(entry.state, AgentState::Running) {
                    let age = (now - entry.last_active).num_seconds();
                    if age < 60 {
                        active += 1;
                    } else {
                        idle += 1;
                    }
                }
            }
        }
        (active, idle)
    }

    /// Compute current LoadMetrics for a fleet manager.
    pub(crate) fn fleet_load_metrics(
        &self,
        manager_id: AgentId,
    ) -> Option<crate::fleet_autoscale::LoadMetrics> {
        let entry = self.registry.get(manager_id)?;
        let (active, idle) = self.count_workers_by_state(&entry.children);
        let queue_depth = entry
            .children
            .iter()
            .map(|cid| self.pending_tasks_for(cid))
            .sum::<u32>();
        let tokens = self
            .scheduler
            .get_agent_usage(manager_id)
            .map(|u| u.input_tokens + u.output_tokens)
            .unwrap_or(0);
        Some(crate::fleet_autoscale::LoadMetrics {
            manager_id,
            active_workers: active,
            idle_workers: idle,
            queue_depth,
            tokens_used_last_window: tokens,
            last_scale_event: entry.last_scale_event,
        })
    }

    fn pending_tasks_for(&self, _worker_id: &AgentId) -> u32 {
        // Best-effort: count tasks assigned to this agent that are not completed.
        // Task queue API is async; keep this approximation for the autoscale tick.
        0
    }

    pub(crate) async fn count_pending_tasks_for_fleet(
        &self,
        manager_id: AgentId,
        children: &[AgentId],
    ) -> u32 {
        let pending = match self.memory.task_list(Some("pending")).await {
            Ok(v) => v,
            Err(_) => return 0,
        };
        let mgr_str = manager_id.to_string();
        let child_strs: HashSet<String> = children.iter().map(|c| c.to_string()).collect();
        pending
            .iter()
            .filter(|t| {
                let assignee = t.get("assigned_to").and_then(|v| v.as_str()).unwrap_or("");
                assignee == mgr_str || child_strs.contains(assignee)
            })
            .count() as u32
    }

    pub async fn autoscale_tick(&self) {
        use crate::fleet_autoscale::{decide, LoadMetrics, ScaleDecision};

        let now = chrono::Utc::now();
        let managers: Vec<_> = self
            .registry
            .list()
            .into_iter()
            .filter(|e| e.tags.iter().any(|t| t == "manager"))
            .filter(|e| e.autoscale.as_ref().is_some_and(|c| c.enabled))
            .collect();

        for manager in managers {
            let Some(cfg) = manager.autoscale.clone() else {
                continue;
            };
            let (active, idle) = self.count_workers_by_state(&manager.children);
            let queue_depth = self
                .count_pending_tasks_for_fleet(manager.id, &manager.children)
                .await;
            let tokens = self
                .scheduler
                .get_agent_usage(manager.id)
                .map(|u| u.input_tokens + u.output_tokens)
                .unwrap_or(0);
            let metrics = LoadMetrics {
                manager_id: manager.id,
                active_workers: active,
                idle_workers: idle,
                queue_depth,
                tokens_used_last_window: tokens,
                last_scale_event: manager.last_scale_event,
            };
            let decision = decide(&metrics, &cfg, now);
            match decision {
                ScaleDecision::NoChange => {}
                ScaleDecision::Spawn => {
                    if let Err(e) = self.autoscale_spawn_worker(&manager, &cfg) {
                        tracing::warn!(manager = %manager.name, "Autoscale spawn failed: {e}");
                    } else {
                        tracing::info!(
                            manager = %manager.name,
                            queue_depth,
                            total = active + idle + 1,
                            "Autoscale: spawned worker"
                        );
                        let _ = self.registry.stamp_scale_event(manager.id);
                        if let Some(entry) = self.registry.get(manager.id) {
                            let _ = self.memory.save_agent(&entry);
                        }
                    }
                }
                ScaleDecision::Kill => {
                    if let Some(victim) = self.pick_idle_worker(&manager.children) {
                        match CaptainKernel::kill_agent(self, victim) {
                            Ok(_) => {
                                tracing::info!(
                                    manager = %manager.name,
                                    victim = %victim,
                                    "Autoscale: killed idle worker"
                                );
                                let _ = self.registry.stamp_scale_event(manager.id);
                                if let Some(entry) = self.registry.get(manager.id) {
                                    let _ = self.memory.save_agent(&entry);
                                }
                            }
                            Err(e) => {
                                tracing::warn!(manager = %manager.name, "Autoscale kill failed: {e}");
                            }
                        }
                    }
                }
            }
        }
    }

    fn pick_idle_worker(&self, children: &[AgentId]) -> Option<AgentId> {
        let now = chrono::Utc::now();
        children
            .iter()
            .filter_map(|cid| {
                self.registry.get(*cid).and_then(|e| {
                    if matches!(e.state, AgentState::Running)
                        && (now - e.last_active).num_seconds() >= 60
                    {
                        Some((*cid, e.last_active))
                    } else {
                        None
                    }
                })
            })
            .min_by_key(|(_, t)| *t)
            .map(|(cid, _)| cid)
    }

    fn autoscale_spawn_worker(
        &self,
        manager: &AgentEntry,
        cfg: &AutoScaleConfig,
    ) -> Result<AgentId, String> {
        let toml_str = cfg.worker_template.clone().unwrap_or_else(|| {
            default_worker_manifest_for_domain(
                &manager.name,
                &manager.manifest.description,
                &manager.manifest.model,
            )
        });
        let manifest: AgentManifest =
            toml::from_str(&toml_str).map_err(|e| format!("Invalid worker template: {e}"))?;
        self.spawn_agent_with_parent(manifest, Some(manager.id), None)
            .map_err(|e| format!("spawn failed: {e}"))
    }
}

fn default_worker_manifest_for_domain(
    manager_name: &str,
    domain: &str,
    model: &ModelConfig,
) -> String {
    let worker_name = format!(
        "{}-worker-{}",
        manager_name,
        &uuid::Uuid::new_v4().to_string()[..8]
    );
    let tools = worker_tools_for_domain(manager_name, domain);
    let tools_toml = tools
        .iter()
        .map(|t| format!("\"{t}\""))
        .collect::<Vec<_>>()
        .join(", ");
    format!(
        r#"name = "{worker_name}"
description = "Worker auto-scaled under {manager_name}"
version = "1.0.0"
author = "autoscale"
module = "llm"
tags = ["worker", "fleet:{manager_name}"]

[model]
provider = "{provider}"
model = "{model_id}"

[capabilities]
tools = [{tools_toml}]
"#,
        provider = model.provider,
        model_id = model.model,
    )
}

pub(crate) fn worker_tools_for_domain(manager_name: &str, domain: &str) -> Vec<&'static str> {
    let base: &[&'static str] = &[
        "memory_store",
        "memory_recall",
        "task_claim",
        "task_complete",
        "channel_send",
    ];
    let name = manager_name.to_lowercase();
    let dom = domain.to_lowercase();
    let extra: &[&'static str] =
        if name.contains("research") || dom.contains("research") || dom.contains("recherche") {
            &["web_search", "web_fetch", "knowledge_add"]
        } else if name.contains("trading") || name.contains("finance") || dom.contains("trading") {
            &["web_search", "web_fetch", "http_get"]
        } else if name.contains("ops") || name.contains("devops") || dom.contains("opération") {
            &["shell_exec", "file_read", "file_list"]
        } else if name.contains("content") || name.contains("redac") || dom.contains("contenu") {
            &["web_search", "web_fetch", "knowledge_query"]
        } else if name.contains("security") || dom.contains("sécurité") {
            &["shell_exec", "http_get", "file_read"]
        } else {
            &["web_search", "web_fetch"]
        };
    let mut out: Vec<&'static str> = base.to_vec();
    out.extend_from_slice(extra);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_worker_manifest_carries_manager_lineage_and_model() {
        let model = ModelConfig {
            provider: "codex".to_string(),
            model: "gpt-5.5".to_string(),
            ..Default::default()
        };

        let manifest_toml = default_worker_manifest_for_domain("research", "veille", &model);
        let parsed: AgentManifest = toml::from_str(&manifest_toml).expect("worker manifest");

        assert!(parsed.name.starts_with("research-worker-"));
        assert_eq!(parsed.author, "autoscale");
        assert!(parsed.tags.contains(&"worker".to_string()));
        assert!(parsed.tags.contains(&"fleet:research".to_string()));
        assert_eq!(parsed.model.provider, "codex");
        assert_eq!(parsed.model.model, "gpt-5.5");
        assert!(parsed
            .capabilities
            .tools
            .contains(&"web_search".to_string()));
        assert!(parsed
            .capabilities
            .tools
            .contains(&"knowledge_add".to_string()));
    }
}
