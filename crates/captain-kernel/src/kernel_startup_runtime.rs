use super::CaptainKernel;
use captain_types::config::MemoryBackend;
use captain_types::memory::Memory;
use captain_types::model_catalog::ProviderInfo;
use chrono::Timelike;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tracing::{debug, info, warn};

impl CaptainKernel {
    pub(super) fn spawn_local_provider_probe(self: &Arc<Self>) {
        let kernel = Arc::clone(self);
        tokio::spawn(async move {
            let local_providers = {
                let catalog = kernel
                    .model_catalog
                    .read()
                    .unwrap_or_else(|e| e.into_inner());
                local_provider_probe_targets(catalog.list_providers())
            };

            for (provider_id, base_url) in &local_providers {
                let result =
                    captain_runtime::provider_health::probe_provider(provider_id, base_url).await;
                if result.reachable {
                    info!(
                        provider = %provider_id,
                        models = result.discovered_models.len(),
                        latency_ms = result.latency_ms,
                        "Local provider online"
                    );
                    if !result.discovered_models.is_empty() {
                        if let Ok(mut catalog) = kernel.model_catalog.write() {
                            catalog.merge_discovered_models(provider_id, &result.discovered_models);
                        }
                    }
                } else {
                    debug!(
                        provider = %provider_id,
                        error = result.error.as_deref().unwrap_or("unknown"),
                        "Local provider offline"
                    );
                }
            }
        });
    }

    pub(super) fn spawn_metering_cleanup_loop(self: &Arc<Self>) {
        let kernel = Arc::clone(self);
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(std::time::Duration::from_secs(24 * 3600));
            interval.tick().await;
            loop {
                interval.tick().await;
                if kernel.supervisor.is_shutting_down() {
                    break;
                }
                match kernel.metering.cleanup(90) {
                    Ok(removed) if removed > 0 => {
                        info!("Metering cleanup: removed {removed} old usage records");
                    }
                    Err(e) => {
                        warn!("Metering cleanup failed: {e}");
                    }
                    _ => {}
                }
            }
        });
    }

    pub(super) fn spawn_memory_consolidation_loop(self: &Arc<Self>) {
        let interval_hours = self.config.memory.consolidation_interval_hours;
        if interval_hours == 0 {
            return;
        }

        let kernel = Arc::clone(self);
        tokio::spawn(async move {
            let mut interval =
                tokio::time::interval(std::time::Duration::from_secs(interval_hours * 3600));
            interval.tick().await;
            loop {
                interval.tick().await;
                if kernel.supervisor.is_shutting_down() {
                    break;
                }
                match kernel.memory.consolidate().await {
                    Ok(report) => {
                        if report.memories_decayed > 0 || report.memories_merged > 0 {
                            info!(
                                merged = report.memories_merged,
                                decayed = report.memories_decayed,
                                duration_ms = report.duration_ms,
                                "Memory consolidation completed"
                            );
                        }
                    }
                    Err(e) => {
                        warn!("Memory consolidation failed: {e}");
                    }
                }
            }
        });
        info!("Memory consolidation scheduled every {interval_hours} hour(s)");
    }

    pub(super) fn spawn_graph_dream_cycle(self: &Arc<Self>) {
        let kernel = Arc::clone(self);
        let dream_interval_hours = self.config.dream_interval_hours;
        tokio::spawn(async move {
            let initial_delay = std::time::Duration::from_secs(30 * 60);
            let cycle_interval = std::time::Duration::from_secs(dream_interval_hours * 3600);
            tokio::time::sleep(initial_delay).await;
            loop {
                if kernel.supervisor.is_shutting_down() {
                    break;
                }
                kernel.graph_memory.auto_verify_predictions();

                match kernel.graph_memory.dream_with_insights() {
                    Ok((stats, insights)) => {
                        let created = stats["facts_created"].as_u64().unwrap_or(0);
                        let linked = stats["links_created"].as_u64().unwrap_or(0);
                        let dark = stats["dark_nodes_marked"].as_u64().unwrap_or(0);
                        let replayed = stats["episodes_replayed"].as_u64().unwrap_or(0);
                        info!(
                            replayed,
                            facts_created = created,
                            links = linked,
                            dark_nodes = dark,
                            insights = insights.len(),
                            "Graph dream cycle completed"
                        );
                        for insight in &insights {
                            info!(insight = %insight.insight, "Dream insight");
                        }
                        let _ = kernel.graph_memory.save();

                        if kernel.config.memory.backend == MemoryBackend::Mempalace
                            && !insights.is_empty()
                        {
                            let mut conns = kernel.mcp_connections.lock().await;
                            if let Some(conn) = conns.iter_mut().find(|c| c.name() == "mempalace") {
                                let summary = insights
                                    .iter()
                                    .map(|i| {
                                        format!(
                                            "- {} (confidence: {:.0}%)",
                                            i.insight,
                                            i.confidence * 100.0
                                        )
                                    })
                                    .collect::<Vec<_>>()
                                    .join("\n");
                                let diary_input = serde_json::json!({
                                    "agent_name": "system",
                                    "entry": format!("Dream cycle: {} facts, {} links, {} dark nodes\n{}", created, linked, dark, summary),
                                    "topic": "dream_cycle",
                                });
                                if let Err(e) = conn
                                    .call_tool("mcp_mempalace_mempalace_diary_write", &diary_input)
                                    .await
                                {
                                    warn!("MemPalace dream diary_write failed: {e}");
                                } else {
                                    info!(
                                        insights = insights.len(),
                                        "Dream insights mirrored to MemPalace diary"
                                    );
                                }
                            }
                        }
                    }
                    Err(e) => {
                        warn!("Graph dream cycle failed: {e}");
                    }
                }
                tokio::time::sleep(cycle_interval).await;
            }
        });
        info!(
            "Graph dream cycle scheduled (initial: 30min, then every {}h)",
            dream_interval_hours
        );
    }

    pub(super) fn spawn_neural_heartbeat(self: &Arc<Self>) {
        let kernel = Arc::clone(self);
        let pulse_interval_secs = self.config.pulse_interval_secs;
        tokio::spawn(async move {
            let pulse_interval = std::time::Duration::from_secs(pulse_interval_secs);
            tokio::time::sleep(std::time::Duration::from_secs(120)).await;
            loop {
                if kernel.supervisor.is_shutting_down() {
                    break;
                }
                kernel.graph_memory.recompute_neuromodulators();
                let threshold = kernel.graph_memory.adjusted_saillance_threshold();
                let raw_thoughts = kernel.graph_memory.neural_pulse(threshold);
                kernel.graph_memory.drain_recent_entities();
                let surfaced = kernel.graph_memory.filter_thoughts(raw_thoughts);
                for thought in &surfaced {
                    info!(
                        thought_type = ?thought.thought_type,
                        score = thought.activation_score,
                        summary = %thought.summary,
                        "Surfaced thought (Act)"
                    );
                    let _ = kernel.graph_memory.record_event(
                        "_sys::thought",
                        &thought.summary,
                        vec![
                            ("thought_type", &format!("{:?}", thought.thought_type)),
                            ("score", &format!("{:.3}", thought.activation_score)),
                            ("decision", "act"),
                        ],
                        None,
                    );
                    kernel.graph_memory.buffer_thought_for_digest(thought);
                }
                let queued = kernel.graph_memory.queued_thought_count();
                if queued > 0 {
                    debug!(queued, "Thoughts queued for next interaction");
                }

                let predictions = kernel.graph_memory.auto_predict();
                if !predictions.is_empty() {
                    debug!(count = predictions.len(), "Auto-predictions generated");
                }

                kernel.graph_memory.emotional_boost();
                kernel.generate_graph_snapshot().await;

                tokio::time::sleep(pulse_interval).await;
            }
        });
        info!(
            "Neural heartbeat started (pulse every {}s, filter active)",
            pulse_interval_secs
        );
    }

    pub(super) fn spawn_telegram_consciousness_digest(self: &Arc<Self>) {
        if !self.config.digest_enabled {
            info!("Telegram consciousness digest disabled (digest_enabled=false)");
            return;
        }

        let kernel = Arc::clone(self);
        let tz_name = kernel.config.timezone.clone();
        let digest_min_interval_secs = self.config.digest_min_interval_hours * 3600;
        tokio::spawn(async move {
            let check_interval = std::time::Duration::from_secs(900);
            let min_interval_secs: i64 = digest_min_interval_secs as i64;
            let mut last_sent_ts: i64 = 0;

            tokio::time::sleep(std::time::Duration::from_secs(digest_min_interval_secs)).await;

            loop {
                if kernel.supervisor.is_shutting_down() {
                    break;
                }

                let now_utc = chrono::Utc::now();
                let local_hour = tz_name
                    .parse::<chrono_tz::Tz>()
                    .ok()
                    .map(|tz| now_utc.with_timezone(&tz).hour())
                    .unwrap_or(now_utc.hour());
                let now_ts = now_utc.timestamp();

                if !telegram_digest_should_attempt(
                    kernel.config.channels.silent_mode,
                    local_hour,
                    now_ts,
                    last_sent_ts,
                    min_interval_secs,
                ) {
                    tokio::time::sleep(check_interval).await;
                    continue;
                }

                if let Some(digest_msg) = kernel.graph_memory.flush_telegram_digest() {
                    if let Some(tg) = kernel.channel_adapters.get("telegram") {
                        let chat_id = kernel
                            .config
                            .channels
                            .telegram
                            .as_ref()
                            .and_then(|t| t.default_chat_id.clone())
                            .unwrap_or_default();
                        if !chat_id.is_empty() {
                            let user = captain_channels::types::ChannelUser {
                                platform_id: chat_id,
                                display_name: "system".to_string(),
                                captain_user: None,
                            };
                            let content = captain_channels::types::ChannelContent::Text(digest_msg);
                            let adapter = tg.clone();
                            let _ = adapter.send(&user, content).await;
                            last_sent_ts = now_ts;
                            info!("Telegram consciousness digest sent");
                        }
                    }
                }
                tokio::time::sleep(check_interval).await;
            }
        });
        info!(
            "Telegram digest scheduled (8h-23h, min {}h interval, smart filter)",
            self.config.digest_min_interval_hours
        );
    }

    pub(super) fn spawn_mcp_connection_if_configured(self: &Arc<Self>) {
        let has_mcp = self
            .effective_mcp_servers
            .read()
            .map(|servers| !servers.is_empty())
            .unwrap_or(false);
        if has_mcp {
            let kernel = Arc::clone(self);
            tokio::spawn(async move {
                kernel.connect_mcp_servers().await;
            });
        }
    }

    pub(super) fn spawn_extension_health_monitor(self: &Arc<Self>) {
        let kernel = Arc::clone(self);
        tokio::spawn(async move {
            kernel.run_extension_health_loop().await;
        });
    }

    pub(super) fn spawn_workflow_autoload(self: &Arc<Self>) {
        let workflow_dir =
            workflow_autoload_dir(&self.config.home_dir, self.config.workflows_dir.as_deref());
        if workflow_dir.exists() {
            let kernel = Arc::clone(self);
            tokio::spawn(async move {
                let count = kernel.load_workflows_from_dir(&workflow_dir).await;
                if count > 0 {
                    info!(
                        "Auto-loaded {count} workflow(s) from {}",
                        workflow_dir.display()
                    );
                }
            });
        }
    }
}

fn local_provider_probe_targets(providers: &[ProviderInfo]) -> Vec<(String, String)> {
    providers
        .iter()
        .filter(|provider| !provider.key_required && !provider.base_url.is_empty())
        .map(|provider| (provider.id.clone(), provider.base_url.clone()))
        .collect()
}

fn telegram_digest_should_attempt(
    silent_mode: bool,
    local_hour: u32,
    now_ts: i64,
    last_sent_ts: i64,
    min_interval_secs: i64,
) -> bool {
    !silent_mode && (8..23).contains(&local_hour) && now_ts - last_sent_ts >= min_interval_secs
}

fn workflow_autoload_dir(home_dir: &Path, configured: Option<&Path>) -> PathBuf {
    configured
        .map(Path::to_path_buf)
        .unwrap_or_else(|| home_dir.join("workflows"))
}

#[cfg(test)]
mod tests {
    use super::{
        local_provider_probe_targets, telegram_digest_should_attempt, workflow_autoload_dir,
    };
    use captain_types::model_catalog::ProviderInfo;
    use std::path::Path;

    #[test]
    fn startup_probe_targets_skip_keyed_or_blank_providers() {
        let providers = vec![
            ProviderInfo {
                id: "ollama".to_string(),
                base_url: "http://127.0.0.1:11434".to_string(),
                key_required: false,
                ..ProviderInfo::default()
            },
            ProviderInfo {
                id: "openai".to_string(),
                base_url: "https://api.openai.com/v1".to_string(),
                key_required: true,
                ..ProviderInfo::default()
            },
            ProviderInfo {
                id: "claude-code".to_string(),
                base_url: String::new(),
                key_required: false,
                ..ProviderInfo::default()
            },
        ];

        assert_eq!(
            local_provider_probe_targets(&providers),
            vec![("ollama".to_string(), "http://127.0.0.1:11434".to_string())]
        );
    }

    #[test]
    fn telegram_digest_attempt_respects_silent_hours_and_spacing() {
        assert!(telegram_digest_should_attempt(false, 8, 10_000, 0, 7_200));
        assert!(!telegram_digest_should_attempt(true, 12, 10_000, 0, 7_200));
        assert!(!telegram_digest_should_attempt(false, 7, 10_000, 0, 7_200));
        assert!(!telegram_digest_should_attempt(false, 23, 10_000, 0, 7_200));
        assert!(!telegram_digest_should_attempt(
            false, 12, 10_000, 4_000, 7_200
        ));
    }

    #[test]
    fn workflow_autoload_dir_defaults_under_home_or_uses_configured_path() {
        let home = Path::new("/tmp/captain-home");
        assert_eq!(workflow_autoload_dir(home, None), home.join("workflows"));
        assert_eq!(
            workflow_autoload_dir(home, Some(Path::new("/srv/workflows"))),
            Path::new("/srv/workflows")
        );
    }
}
