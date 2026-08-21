//! Bounded, non-secret live inventory for configured Captain authorities.

use crate::{
    gateway::{load_transport, GatewayUnavailableReason},
    ConsoleProfileSummary,
};
use bytes::Bytes;
use futures::{stream, StreamExt};
use reqwest::{header::HeaderMap, Method, StatusCode};
use serde::Serialize;
use serde_json::Value;
use std::{path::Path, sync::Arc, time::Duration};

const PROFILE_PROBE_CONCURRENCY: usize = 8;
const PROFILE_PROBE_TIMEOUT: Duration = Duration::from_secs(5);
const MAX_STATUS_BYTES: usize = 768 * 1024;
const MAX_SESSIONS_BYTES: usize = 2 * 1024 * 1024;
const MAX_QUOTAS: usize = 8;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ConsoleAuthorityAvailability {
    NotChecked,
    Online,
    Offline,
    SetupRequired,
    PairingRequired,
    CredentialUnavailable,
    InvalidResponse,
}

impl ConsoleAuthorityAvailability {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::NotChecked => "not_checked",
            Self::Online => "online",
            Self::Offline => "offline",
            Self::SetupRequired => "setup_required",
            Self::PairingRequired => "pairing_required",
            Self::CredentialUnavailable => "credential_unavailable",
            Self::InvalidResponse => "invalid_response",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ConsoleQuotaObservation {
    pub id: String,
    pub name: String,
    pub window: String,
    pub remaining_percent: Option<f64>,
    pub resets_at: Option<String>,
    pub alert_level: Option<String>,
    pub stale: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ConsoleAuthorityObservation {
    #[serde(flatten)]
    pub profile: ConsoleProfileSummary,
    pub availability: ConsoleAuthorityAvailability,
    pub observed_at_ms: Option<i64>,
    pub version: Option<String>,
    pub provider: Option<String>,
    pub model: Option<String>,
    pub health: Option<String>,
    pub alert_count: Option<u64>,
    pub session_count: Option<usize>,
    pub active_project_count: Option<u64>,
    pub quotas: Vec<ConsoleQuotaObservation>,
}

impl ConsoleAuthorityObservation {
    pub fn local(profile: ConsoleProfileSummary) -> Self {
        let availability = if profile.configured {
            ConsoleAuthorityAvailability::NotChecked
        } else {
            ConsoleAuthorityAvailability::SetupRequired
        };
        Self::empty(profile, availability)
    }

    fn empty(profile: ConsoleProfileSummary, availability: ConsoleAuthorityAvailability) -> Self {
        Self {
            profile,
            availability,
            observed_at_ms: None,
            version: None,
            provider: None,
            model: None,
            health: None,
            alert_count: None,
            session_count: None,
            active_project_count: None,
            quotas: Vec::new(),
        }
    }
}

pub(crate) async fn observe_profiles(
    home: &Path,
    profiles: Vec<ConsoleProfileSummary>,
) -> Vec<ConsoleAuthorityObservation> {
    let home = Arc::new(home.to_path_buf());
    let mut observed = stream::iter(profiles.into_iter().enumerate().map(|(index, profile)| {
        let home = Arc::clone(&home);
        async move { (index, observe_profile(&home, profile).await) }
    }))
    .buffer_unordered(PROFILE_PROBE_CONCURRENCY)
    .collect::<Vec<_>>()
    .await;
    observed.sort_by_key(|(index, _)| *index);
    observed.into_iter().map(|(_, item)| item).collect()
}

async fn observe_profile(
    home: &Path,
    profile: ConsoleProfileSummary,
) -> ConsoleAuthorityObservation {
    if !profile.configured {
        return ConsoleAuthorityObservation::local(profile);
    }
    let (transport, reason, selected_profile_id) = load_transport(home, Some(&profile.id));
    if selected_profile_id.as_deref() != Some(profile.id.as_str()) {
        return ConsoleAuthorityObservation::empty(
            profile,
            ConsoleAuthorityAvailability::CredentialUnavailable,
        );
    }
    let Some(transport) = transport else {
        return ConsoleAuthorityObservation::empty(profile, availability_for(reason));
    };
    let status = match request_json(&transport, "/api/status", MAX_STATUS_BYTES).await {
        Ok(status) => status,
        Err(error) => {
            return ConsoleAuthorityObservation::empty(profile, error.availability());
        }
    };
    let session_count = request_json(&transport, "/api/sessions", MAX_SESSIONS_BYTES)
        .await
        .ok()
        .and_then(|body| body.get("sessions")?.as_array().map(Vec::len));
    observation_from_status(profile, &status, session_count, current_time_ms())
}

async fn request_json(
    transport: &captain_node::ClientAccessTransport,
    path: &str,
    max_bytes: usize,
) -> Result<Value, ProbeError> {
    tokio::time::timeout(
        PROFILE_PROBE_TIMEOUT,
        request_json_within_deadline(transport, path, max_bytes),
    )
    .await
    .map_err(|_| ProbeError::Offline)?
}

async fn request_json_within_deadline(
    transport: &captain_node::ClientAccessTransport,
    path: &str,
    max_bytes: usize,
) -> Result<Value, ProbeError> {
    let response = transport
        .execute(Method::GET, path, &HeaderMap::new(), Bytes::new())
        .await
        .map_err(|_| ProbeError::Offline)?;
    if matches!(
        response.status(),
        StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN
    ) {
        return Err(ProbeError::PairingRequired);
    }
    if !response.status().is_success() {
        return Err(ProbeError::Offline);
    }
    if response
        .content_length()
        .is_some_and(|length| length > max_bytes as u64)
    {
        return Err(ProbeError::InvalidResponse);
    }
    let mut body = Vec::new();
    let mut chunks = response.bytes_stream();
    while let Some(chunk) = chunks.next().await {
        let chunk = chunk.map_err(|_| ProbeError::Offline)?;
        if body.len().saturating_add(chunk.len()) > max_bytes {
            return Err(ProbeError::InvalidResponse);
        }
        body.extend_from_slice(&chunk);
    }
    serde_json::from_slice(&body).map_err(|_| ProbeError::InvalidResponse)
}

fn observation_from_status(
    profile: ConsoleProfileSummary,
    status: &Value,
    session_count: Option<usize>,
    observed_at_ms: Option<i64>,
) -> ConsoleAuthorityObservation {
    ConsoleAuthorityObservation {
        profile,
        availability: ConsoleAuthorityAvailability::Online,
        observed_at_ms,
        version: safe_text(status.get("version"), 64),
        provider: safe_text(status.get("default_provider"), 80),
        model: safe_text(status.get("default_model"), 160),
        health: safe_text(status.pointer("/runtime_health/state"), 40),
        alert_count: status
            .pointer("/runtime_health/issue_count")
            .and_then(Value::as_u64),
        session_count,
        active_project_count: status
            .pointer("/workload/projects/active")
            .and_then(Value::as_u64),
        quotas: quota_observations(status),
    }
}

fn quota_observations(status: &Value) -> Vec<ConsoleQuotaObservation> {
    let mut observations = Vec::new();
    for item in status
        .pointer("/budget/provider_subscriptions/items")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        let Some(id) = safe_text(item.get("limit_id"), 80) else {
            continue;
        };
        let name = safe_text(item.get("limit_name"), 120).unwrap_or_else(|| id.clone());
        let alert_level = safe_text(item.get("alert_level"), 40);
        let stale = item.get("stale").and_then(Value::as_bool).unwrap_or(false);
        for (window_name, window) in [
            ("primary", item.get("primary")),
            ("secondary", item.get("secondary")),
            ("spend", item.pointer("/spend_control/individual_limit")),
        ] {
            let Some(window) = window else {
                continue;
            };
            let remaining_percent = finite_percent(window.get("remaining_percent"));
            let resets_at = safe_text(window.get("resets_at"), 80);
            if remaining_percent.is_none() && resets_at.is_none() {
                continue;
            }
            observations.push(ConsoleQuotaObservation {
                id: id.clone(),
                name: name.clone(),
                window: window_name.to_string(),
                remaining_percent,
                resets_at,
                alert_level: alert_level.clone(),
                stale,
            });
            if observations.len() == MAX_QUOTAS {
                return observations;
            }
        }
    }
    observations
}

fn finite_percent(value: Option<&Value>) -> Option<f64> {
    let value = value?.as_f64()?;
    value.is_finite().then(|| value.clamp(0.0, 100.0))
}

fn safe_text(value: Option<&Value>, max_bytes: usize) -> Option<String> {
    let text = value?.as_str()?.trim();
    if text.is_empty() || text.len() > max_bytes || text.chars().any(char::is_control) {
        return None;
    }
    Some(text.to_string())
}

fn availability_for(reason: Option<GatewayUnavailableReason>) -> ConsoleAuthorityAvailability {
    match reason {
        Some(GatewayUnavailableReason::Unconfigured) => ConsoleAuthorityAvailability::SetupRequired,
        Some(GatewayUnavailableReason::PairingIncomplete) => {
            ConsoleAuthorityAvailability::PairingRequired
        }
        Some(GatewayUnavailableReason::ProxyCredentialUnavailable)
        | Some(GatewayUnavailableReason::ConfigurationUnavailable)
        | Some(GatewayUnavailableReason::ProfileUnavailable)
        | None => ConsoleAuthorityAvailability::CredentialUnavailable,
    }
}

fn current_time_ms() -> Option<i64> {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .ok()
        .and_then(|elapsed| i64::try_from(elapsed.as_millis()).ok())
}

#[derive(Debug, Clone, Copy)]
enum ProbeError {
    Offline,
    PairingRequired,
    InvalidResponse,
}

impl ProbeError {
    fn availability(self) -> ConsoleAuthorityAvailability {
        match self {
            Self::Offline => ConsoleAuthorityAvailability::Offline,
            Self::PairingRequired => ConsoleAuthorityAvailability::PairingRequired,
            Self::InvalidResponse => ConsoleAuthorityAvailability::InvalidResponse,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn profile() -> ConsoleProfileSummary {
        ConsoleProfileSummary {
            id: "00000000-0000-4000-8000-000000000001".to_string(),
            label: "Production".to_string(),
            active: true,
            configured: true,
        }
    }

    #[test]
    fn status_projection_keeps_only_bounded_operator_facts() {
        let status = serde_json::json!({
            "version": "0.1.0-alpha.15",
            "default_provider": "codex",
            "default_model": "gpt-5.6-sol",
            "home_dir": "/private/hub/home",
            "runtime_health": {"state": "watch", "issue_count": 2, "issues": [{"summary": "private"}]},
            "workload": {"projects": {"active": 3}},
            "budget": {"provider_subscriptions": {"items": [{
                "limit_id": "weekly",
                "limit_name": "Weekly",
                "alert_level": "normal",
                "stale": true,
                "primary": {"remaining_percent": 72.5, "resets_at": "2026-08-24T10:00:00Z"},
                "secondary": {"remaining_percent": 51.0, "resets_at": "2026-08-30T10:00:00Z"}
            }]}}
        });
        let observed = observation_from_status(profile(), &status, Some(14), Some(42));
        assert_eq!(observed.availability, ConsoleAuthorityAvailability::Online);
        assert_eq!(observed.session_count, Some(14));
        assert_eq!(observed.active_project_count, Some(3));
        assert_eq!(observed.quotas[0].remaining_percent, Some(72.5));
        assert_eq!(observed.quotas[0].window, "primary");
        assert!(observed.quotas[0].stale);
        assert_eq!(observed.quotas[1].remaining_percent, Some(51.0));
        assert_eq!(observed.quotas[1].window, "secondary");
        let rendered = serde_json::to_string(&observed).unwrap();
        assert!(!rendered.contains("/private/hub/home"));
        assert!(!rendered.contains("issues"));
    }

    #[test]
    fn quota_and_text_projection_rejects_unbounded_or_control_data() {
        let items = (0..20)
            .map(|index| {
                serde_json::json!({
                    "limit_id": format!("quota-{index}"),
                    "limit_name": "Weekly\nsecret",
                    "primary": {"remaining_percent": 500.0}
                })
            })
            .collect::<Vec<_>>();
        let status = serde_json::json!({
            "version": "bad\nversion",
            "budget": {"provider_subscriptions": {"items": items}}
        });
        let observed = observation_from_status(profile(), &status, None, None);
        assert_eq!(observed.version, None);
        assert_eq!(observed.quotas.len(), MAX_QUOTAS);
        assert_eq!(observed.quotas[0].name, "quota-0");
        assert_eq!(observed.quotas[0].remaining_percent, Some(100.0));
    }

    #[test]
    fn local_inventory_distinguishes_unchecked_from_setup_required() {
        let configured = ConsoleAuthorityObservation::local(profile());
        assert_eq!(
            configured.availability,
            ConsoleAuthorityAvailability::NotChecked
        );
        let mut unconfigured = profile();
        unconfigured.configured = false;
        assert_eq!(
            ConsoleAuthorityObservation::local(unconfigured).availability,
            ConsoleAuthorityAvailability::SetupRequired
        );
    }
}
