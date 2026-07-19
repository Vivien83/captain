//! Live provider subscription quota monitor and persistence bridge.

use crate::CaptainKernel;
use captain_memory::provider_quota::ProviderQuotaStore;
use captain_runtime::provider_quota::ProviderQuotaObserver;
use captain_types::quota::{ProviderQuotaSnapshot, QuotaAlertLevel};
use std::sync::Arc;
use std::time::Duration;
use tracing::{info, warn};

const REFRESH_INTERVAL: Duration = Duration::from_secs(300);

#[derive(Clone, Debug, PartialEq, Eq)]
struct CodexQuotaTarget {
    name: String,
    provider: String,
    base_url: Option<String>,
}

/// Build the callback attached to live Codex model responses.
pub(crate) fn provider_quota_observer(store: ProviderQuotaStore) -> ProviderQuotaObserver {
    Arc::new(move |snapshot| persist_provider_quota_snapshot(&store, &snapshot))
}

/// Refresh Codex's official account usage endpoint immediately and every 5 minutes.
pub(crate) fn spawn_codex_provider_quota_monitor(kernel: Arc<CaptainKernel>) {
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(REFRESH_INTERVAL);
        interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        let mut last_error: Option<String> = None;
        loop {
            interval.tick().await;
            let Some(target) = codex_quota_target(&kernel) else {
                last_error = None;
                continue;
            };
            let Some(token) = captain_runtime::model_catalog::read_codex_credential_with_refresh()
            else {
                log_refresh_error_once(
                    &mut last_error,
                    "Codex session unavailable for subscription quota refresh".to_string(),
                );
                continue;
            };
            let base_url = target
                .base_url
                .clone()
                .or_else(|| kernel.lookup_provider_url(&target.provider))
                .unwrap_or_else(|| captain_types::model_catalog::CODEX_BASE_URL.to_string());
            match captain_runtime::provider_quota::fetch_codex_subscription_quotas(
                &token, &base_url,
            )
            .await
            {
                Ok(snapshots) => {
                    if last_error.take().is_some() {
                        info!(provider = "codex", "Provider quota refresh recovered");
                    }
                    for snapshot in snapshots {
                        persist_provider_quota_snapshot(kernel.memory.provider_quotas(), &snapshot);
                    }
                }
                Err(error) => log_refresh_error_once(&mut last_error, error.to_string()),
            }
        }
    });
}

fn codex_quota_target(kernel: &CaptainKernel) -> Option<CodexQuotaTarget> {
    let default = kernel.effective_default_model();
    let default = CodexQuotaTarget {
        name: "captain-default".to_string(),
        provider: default.provider,
        base_url: default.base_url,
    };
    let agents = kernel
        .registry
        .list()
        .into_iter()
        .map(|entry| CodexQuotaTarget {
            name: entry.name,
            provider: entry.manifest.model.provider,
            base_url: entry.manifest.model.base_url,
        })
        .collect();
    select_codex_quota_target(default, agents)
}

fn select_codex_quota_target(
    default: CodexQuotaTarget,
    mut agents: Vec<CodexQuotaTarget>,
) -> Option<CodexQuotaTarget> {
    if is_codex_provider(&default.provider) {
        return Some(default);
    }
    agents.retain(|agent| is_codex_provider(&agent.provider));
    agents.sort_by(|left, right| {
        (left.name != "captain", left.name.as_str())
            .cmp(&(right.name != "captain", right.name.as_str()))
    });
    agents.into_iter().next()
}

fn is_codex_provider(provider: &str) -> bool {
    provider.eq_ignore_ascii_case("codex") || provider.eq_ignore_ascii_case("openai-codex")
}

fn persist_provider_quota_snapshot(store: &ProviderQuotaStore, snapshot: &ProviderQuotaSnapshot) {
    match store.record(snapshot) {
        Ok(change) if change.should_announce() => {
            let primary_used = snapshot.primary.as_ref().map(|window| window.used_percent);
            let secondary_used = snapshot
                .secondary
                .as_ref()
                .map(|window| window.used_percent);
            let primary_reset = snapshot
                .primary
                .as_ref()
                .and_then(|window| window.resets_at);
            let secondary_reset = snapshot
                .secondary
                .as_ref()
                .and_then(|window| window.resets_at);
            match change.current_alert {
                QuotaAlertLevel::Normal => info!(
                    provider = %snapshot.provider,
                    limit_id = %snapshot.limit_id,
                    limit_name = snapshot.limit_name.as_deref().unwrap_or("unknown"),
                    alert = %change.current_alert,
                    previous_alert = ?change.previous_alert,
                    primary_used_percent = ?primary_used,
                    secondary_used_percent = ?secondary_used,
                    primary_resets_at = ?primary_reset,
                    secondary_resets_at = ?secondary_reset,
                    source = ?snapshot.source,
                    "Provider subscription quota observed"
                ),
                _ => warn!(
                    provider = %snapshot.provider,
                    limit_id = %snapshot.limit_id,
                    limit_name = snapshot.limit_name.as_deref().unwrap_or("unknown"),
                    alert = %change.current_alert,
                    previous_alert = ?change.previous_alert,
                    primary_used_percent = ?primary_used,
                    secondary_used_percent = ?secondary_used,
                    primary_resets_at = ?primary_reset,
                    secondary_resets_at = ?secondary_reset,
                    source = ?snapshot.source,
                    "Provider subscription quota alert"
                ),
            }
        }
        Ok(_) => {}
        Err(error) => warn!(
            provider = %snapshot.provider,
            limit_id = %snapshot.limit_id,
            error = %error,
            "Failed to persist provider subscription quota"
        ),
    }
}

fn log_refresh_error_once(last_error: &mut Option<String>, error: String) {
    if last_error.as_deref() != Some(error.as_str()) {
        warn!(provider = "codex", error = %error, "Provider quota refresh failed");
        *last_error = Some(error);
    }
}

#[cfg(test)]
mod tests {
    use super::{select_codex_quota_target, CodexQuotaTarget};

    fn target(name: &str, provider: &str, base_url: Option<&str>) -> CodexQuotaTarget {
        CodexQuotaTarget {
            name: name.to_string(),
            provider: provider.to_string(),
            base_url: base_url.map(str::to_string),
        }
    }

    #[test]
    fn default_codex_target_has_priority() {
        let selected = select_codex_quota_target(
            target("captain-default", "codex", Some("https://default.test")),
            vec![target("worker", "codex", Some("https://worker.test"))],
        )
        .unwrap();

        assert_eq!(selected.name, "captain-default");
        assert_eq!(selected.base_url.as_deref(), Some("https://default.test"));
    }

    #[test]
    fn registered_codex_agent_enables_refresh_under_another_default() {
        let selected = select_codex_quota_target(
            target("captain-default", "anthropic", None),
            vec![
                target("z-worker", "openai-codex", Some("https://z.test")),
                target("captain", "codex", Some("https://captain.test")),
            ],
        )
        .unwrap();

        assert_eq!(selected.name, "captain");
        assert_eq!(selected.provider, "codex");
        assert_eq!(selected.base_url.as_deref(), Some("https://captain.test"));
    }

    #[test]
    fn no_codex_target_disables_account_refresh() {
        let selected = select_codex_quota_target(
            target("captain-default", "anthropic", None),
            vec![target("worker", "mistral", None)],
        );

        assert!(selected.is_none());
    }
}
