//! Crash-safe cached deployment readiness shared by status and doctor.

use std::collections::BTreeSet;
use std::io;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use captain_types::config::KernelConfig;
use chrono::{DateTime, Utc};
use futures::StreamExt;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use url::{Host, Url};

const SNAPSHOT_SCHEMA_VERSION: u32 = 1;
const SNAPSHOT_FILE_NAME: &str = "deployment-readiness.json";
const MAX_SNAPSHOT_BYTES: u64 = 256 * 1024;
const MAX_CHECKS: usize = 32;
const MAX_ACTIONS: usize = 16;
const MAX_TEXT_CHARS: usize = 512;
const MAX_HEALTH_BODY_BYTES: usize = 64 * 1024;
const PROBE_INITIAL_DELAY: Duration = Duration::from_secs(2);
const PROBE_INTERVAL: Duration = Duration::from_secs(5 * 60);
const DNS_TIMEOUT: Duration = Duration::from_secs(5);
const HTTP_CONNECT_TIMEOUT: Duration = Duration::from_secs(5);
const HTTP_TIMEOUT: Duration = Duration::from_secs(8);
const STALE_GRACE: chrono::Duration = chrono::Duration::minutes(1);
const MAX_SNAPSHOT_DURATION_MS: u64 = 60_000;
const MAX_SNAPSHOT_FUTURE_SKEW: chrono::Duration = chrono::Duration::minutes(5);
const MAX_SNAPSHOT_INTERVAL: chrono::Duration = chrono::Duration::hours(1);

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
struct HealthPayload {
    status: String,
    version: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum HealthProbeIssue {
    Timeout,
    Transport,
    HttpStatus(u16),
    BodyTooLarge,
    InvalidPayload,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct HealthObservation {
    reached: bool,
    payload: Option<HealthPayload>,
    issue: Option<HealthProbeIssue>,
}

#[derive(Debug)]
struct EndpointAssessment {
    checks: Vec<DeploymentReadinessCheck>,
    health: Option<HealthPayload>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum DeploymentReadinessState {
    NotConfigured,
    Pending,
    Ready,
    Degraded,
    Failed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum DeploymentCheckStatus {
    Skipped,
    Pending,
    Ok,
    Warning,
    Failed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct DeploymentReadinessCheck {
    pub id: String,
    pub status: DeploymentCheckStatus,
    pub summary: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub remediation: Option<String>,
}

impl DeploymentReadinessCheck {
    pub(crate) fn pending(id: &str, summary: &str) -> Self {
        Self::new(id, DeploymentCheckStatus::Pending, summary, None)
    }

    pub(crate) fn ok(id: &str, summary: &str) -> Self {
        Self::new(id, DeploymentCheckStatus::Ok, summary, None)
    }

    pub(crate) fn warning(id: &str, summary: &str, remediation: &str) -> Self {
        Self::new(
            id,
            DeploymentCheckStatus::Warning,
            summary,
            Some(remediation),
        )
    }

    pub(crate) fn failed(id: &str, summary: &str, remediation: &str) -> Self {
        Self::new(
            id,
            DeploymentCheckStatus::Failed,
            summary,
            Some(remediation),
        )
    }

    pub(crate) fn skipped(id: &str, summary: &str) -> Self {
        Self::new(id, DeploymentCheckStatus::Skipped, summary, None)
    }

    fn new(
        id: &str,
        status: DeploymentCheckStatus,
        summary: &str,
        remediation: Option<&str>,
    ) -> Self {
        Self {
            id: id.to_string(),
            status,
            summary: summary.to_string(),
            remediation: remediation.map(str::to_string),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct DeploymentReadinessSnapshot {
    schema_version: u32,
    config_fingerprint: String,
    pub state: DeploymentReadinessState,
    pub checked_at: Option<DateTime<Utc>>,
    pub duration_ms: Option<u64>,
    pub next_check_at: Option<DateTime<Utc>>,
    pub checks: Vec<DeploymentReadinessCheck>,
    pub operator_actions: Vec<String>,
}

impl DeploymentReadinessSnapshot {
    pub(crate) fn pending(config: &KernelConfig) -> Self {
        Self::from_checks(config, None, None, None, baseline_checks(config))
    }

    pub(crate) fn evaluated(
        config: &KernelConfig,
        checked_at: DateTime<Utc>,
        duration_ms: u64,
        next_check_at: DateTime<Utc>,
        probe_checks: Vec<DeploymentReadinessCheck>,
    ) -> Self {
        let mut checks = configuration_checks(config);
        checks.extend(probe_checks);
        Self::from_checks(
            config,
            Some(checked_at),
            Some(duration_ms),
            Some(next_check_at),
            checks,
        )
    }

    fn from_checks(
        config: &KernelConfig,
        checked_at: Option<DateTime<Utc>>,
        duration_ms: Option<u64>,
        next_check_at: Option<DateTime<Utc>>,
        checks: Vec<DeploymentReadinessCheck>,
    ) -> Self {
        let public_configured = !config.deployment.public_url.trim().is_empty();
        let state = readiness_state(public_configured, &checks);
        let operator_actions = remediation_actions(&checks);
        Self {
            schema_version: SNAPSHOT_SCHEMA_VERSION,
            config_fingerprint: config_fingerprint(config),
            state,
            checked_at,
            duration_ms,
            next_check_at,
            checks,
            operator_actions,
        }
    }
}

fn remediation_actions(checks: &[DeploymentReadinessCheck]) -> Vec<String> {
    checks
        .iter()
        .filter(|check| {
            matches!(
                check.status,
                DeploymentCheckStatus::Warning | DeploymentCheckStatus::Failed
            )
        })
        .filter_map(|check| check.remediation.clone())
        .collect::<BTreeSet<_>>()
        .into_iter()
        .take(MAX_ACTIONS)
        .collect()
}

fn refresh_derived_fields(config: &KernelConfig, snapshot: &mut DeploymentReadinessSnapshot) {
    snapshot.state = readiness_state(
        !config.deployment.public_url.trim().is_empty(),
        &snapshot.checks,
    );
    snapshot.operator_actions = remediation_actions(&snapshot.checks);
}

pub(crate) fn spawn(config: KernelConfig, listen_addr: SocketAddr) {
    tokio::spawn(async move {
        tokio::time::sleep(PROBE_INITIAL_DELAY).await;
        loop {
            let snapshot = probe_snapshot(&config, listen_addr).await;
            let write_config = config.clone();
            let write_snapshot = snapshot.clone();
            match tokio::task::spawn_blocking(move || save_snapshot(&write_config, &write_snapshot))
                .await
            {
                Ok(Ok(())) => tracing::debug!(
                    state = ?snapshot.state,
                    "deployment readiness snapshot refreshed"
                ),
                Ok(Err(error)) => tracing::warn!(
                    error_kind = ?error.kind(),
                    "deployment readiness snapshot could not be persisted"
                ),
                Err(_) => tracing::warn!("deployment readiness snapshot writer did not complete"),
            }
            tokio::time::sleep(PROBE_INTERVAL).await;
        }
    });
}

pub(crate) fn status_value(config: &KernelConfig) -> serde_json::Value {
    status_value_at(config, Utc::now())
}

fn status_value_at(config: &KernelConfig, now: DateTime<Utc>) -> serde_json::Value {
    let mut snapshot = load_or_pending(config);
    if snapshot
        .next_check_at
        .is_some_and(|next_check| next_check + STALE_GRACE < now)
    {
        snapshot.checks.retain(|check| check.id != "freshness");
        if snapshot.checks.len() >= MAX_CHECKS {
            snapshot.checks.truncate(MAX_CHECKS - 1);
        }
        snapshot.checks.push(DeploymentReadinessCheck::warning(
            "freshness",
            "The cached deployment check is stale",
            "Run captain doctor --full after confirming that the Captain daemon is running.",
        ));
        refresh_derived_fields(config, &mut snapshot);
    }

    serde_json::json!({
        "state": snapshot.state,
        "checked_at": snapshot.checked_at,
        "duration_ms": snapshot.duration_ms,
        "next_check_at": snapshot.next_check_at,
        "checks": snapshot.checks,
        "operator_actions": snapshot.operator_actions,
    })
}

pub(crate) fn load_or_pending(config: &KernelConfig) -> DeploymentReadinessSnapshot {
    match load_snapshot(config) {
        Ok(Some(snapshot)) => snapshot,
        Ok(None) => DeploymentReadinessSnapshot::pending(config),
        Err(error) => {
            tracing::warn!(error = %error, "deployment readiness snapshot unavailable");
            let mut checks = baseline_checks(config);
            checks.push(DeploymentReadinessCheck::warning(
                "snapshot",
                "The cached readiness snapshot could not be verified",
                "Run captain doctor --full after the daemon has completed a fresh readiness check.",
            ));
            DeploymentReadinessSnapshot::from_checks(config, None, None, None, checks)
        }
    }
}

pub(crate) fn save_snapshot(
    config: &KernelConfig,
    snapshot: &DeploymentReadinessSnapshot,
) -> io::Result<()> {
    validate_snapshot(config, snapshot)?;
    let bytes = serde_json::to_vec_pretty(snapshot)
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
    if bytes.len() as u64 > MAX_SNAPSHOT_BYTES {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "deployment readiness snapshot exceeds its size limit",
        ));
    }
    captain_types::durable_fs::atomic_write(&snapshot_path(config), &bytes)
}

fn load_snapshot(config: &KernelConfig) -> io::Result<Option<DeploymentReadinessSnapshot>> {
    let path = snapshot_path(config);
    let metadata = match std::fs::symlink_metadata(&path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error),
    };
    if metadata.file_type().is_symlink()
        || !metadata.is_file()
        || metadata.len() > MAX_SNAPSHOT_BYTES
    {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "deployment readiness snapshot has an invalid file shape",
        ));
    }
    let bytes = std::fs::read(path)?;
    let snapshot: DeploymentReadinessSnapshot = serde_json::from_slice(&bytes)
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
    if snapshot.config_fingerprint != config_fingerprint(config) {
        return Ok(None);
    }
    validate_snapshot(config, &snapshot)?;
    Ok(Some(snapshot))
}

fn validate_snapshot(
    config: &KernelConfig,
    snapshot: &DeploymentReadinessSnapshot,
) -> io::Result<()> {
    if snapshot.schema_version != SNAPSHOT_SCHEMA_VERSION
        || snapshot.config_fingerprint != config_fingerprint(config)
        || snapshot.checks.len() > MAX_CHECKS
        || snapshot.operator_actions.len() > MAX_ACTIONS
        || snapshot.config_fingerprint.len() != 64
    {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "deployment readiness snapshot failed structural validation",
        ));
    }
    let mut check_ids = BTreeSet::new();
    let text_valid = snapshot.checks.iter().all(|check| {
        valid_check_id(&check.id)
            && check_ids.insert(check.id.as_str())
            && bounded_text(&check.summary)
            && check.remediation.as_deref().is_none_or(bounded_text)
    }) && snapshot
        .operator_actions
        .iter()
        .all(|item| bounded_text(item));
    if !text_valid {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "deployment readiness snapshot contains invalid text",
        ));
    }
    let timing_valid = match (
        snapshot.checked_at,
        snapshot.duration_ms,
        snapshot.next_check_at,
    ) {
        (None, None, None) => true,
        (Some(checked_at), Some(duration_ms), Some(next_check_at)) => {
            duration_ms <= MAX_SNAPSHOT_DURATION_MS
                && checked_at <= Utc::now() + MAX_SNAPSHOT_FUTURE_SKEW
                && next_check_at > checked_at
                && next_check_at - checked_at <= MAX_SNAPSHOT_INTERVAL
        }
        _ => false,
    };
    let expected_actions = remediation_actions(&snapshot.checks);
    let expected_state = readiness_state(
        !config.deployment.public_url.trim().is_empty(),
        &snapshot.checks,
    );
    if !timing_valid
        || snapshot.operator_actions != expected_actions
        || snapshot.state != expected_state
    {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "deployment readiness snapshot failed semantic validation",
        ));
    }
    Ok(())
}

fn baseline_checks(config: &KernelConfig) -> Vec<DeploymentReadinessCheck> {
    let mut checks = configuration_checks(config);
    if config.deployment.public_url.trim().is_empty() {
        checks.extend([
            DeploymentReadinessCheck::skipped("local_port", "No public deployment is configured"),
            DeploymentReadinessCheck::skipped("local_health", "No public deployment is configured"),
            DeploymentReadinessCheck::skipped("dns", "No public domain is configured"),
            DeploymentReadinessCheck::skipped("tls", "No public HTTPS endpoint is configured"),
            DeploymentReadinessCheck::skipped("public_port", "No public endpoint is configured"),
            DeploymentReadinessCheck::skipped(
                "reverse_proxy_live",
                "No public reverse proxy is configured",
            ),
            DeploymentReadinessCheck::skipped("public_health", "No public endpoint is configured"),
            DeploymentReadinessCheck::skipped("version_parity", "No public endpoint is configured"),
        ]);
    } else {
        checks.extend([
            DeploymentReadinessCheck::pending("local_port", "Waiting for the local port probe"),
            DeploymentReadinessCheck::pending("local_health", "Waiting for the local health probe"),
            DeploymentReadinessCheck::pending("dns", "Waiting for the public DNS probe"),
            DeploymentReadinessCheck::pending("tls", "Waiting for the TLS handshake probe"),
            DeploymentReadinessCheck::pending("public_port", "Waiting for the public port probe"),
            DeploymentReadinessCheck::pending(
                "reverse_proxy_live",
                "Waiting for the public routing probe",
            ),
            DeploymentReadinessCheck::pending(
                "public_health",
                "Waiting for the public health probe",
            ),
            DeploymentReadinessCheck::pending(
                "version_parity",
                "Waiting for the version comparison",
            ),
        ]);
    }
    checks
}

async fn probe_snapshot(
    config: &KernelConfig,
    listen_addr: SocketAddr,
) -> DeploymentReadinessSnapshot {
    let started = Instant::now();
    let (local, public) = tokio::join!(
        probe_local_endpoint(listen_addr),
        probe_public_endpoint(config)
    );
    let mut checks = local.checks;
    checks.extend(public.checks);
    checks.push(version_parity_check(
        config,
        local.health.as_ref(),
        public.health.as_ref(),
    ));

    let checked_at = Utc::now();
    let duration_ms = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX);
    let interval =
        chrono::Duration::from_std(PROBE_INTERVAL).unwrap_or_else(|_| chrono::Duration::minutes(5));
    DeploymentReadinessSnapshot::evaluated(
        config,
        checked_at,
        duration_ms.min(MAX_SNAPSHOT_DURATION_MS),
        checked_at + interval,
        checks,
    )
}

async fn probe_local_endpoint(listen_addr: SocketAddr) -> EndpointAssessment {
    let loopback_addr = if listen_addr.ip().is_unspecified() {
        match listen_addr {
            SocketAddr::V4(address) => SocketAddr::new(
                std::net::IpAddr::V4(std::net::Ipv4Addr::LOCALHOST),
                address.port(),
            ),
            SocketAddr::V6(address) => SocketAddr::new(
                std::net::IpAddr::V6(std::net::Ipv6Addr::LOCALHOST),
                address.port(),
            ),
        }
    } else {
        listen_addr
    };
    let url = format!("http://{loopback_addr}/api/health");
    let observation = match health_client(None) {
        Ok(client) => observe_health(&client, &url).await,
        Err(()) => HealthObservation {
            reached: false,
            payload: None,
            issue: Some(HealthProbeIssue::Transport),
        },
    };

    let port_check = if observation.reached {
        DeploymentReadinessCheck::ok("local_port", "The local Captain API port is reachable")
    } else {
        DeploymentReadinessCheck::failed(
            "local_port",
            "The local Captain API port could not be reached",
            "Confirm that the daemon is running and that api_listen matches the active listener.",
        )
    };
    let health_check = health_check_from_observation(
        "local_health",
        "Local Captain health is healthy",
        "Local Captain health is degraded",
        "The local health endpoint could not be verified",
        "Run captain doctor --full and inspect database and audit integrity before exposing Captain.",
        &observation,
    );
    EndpointAssessment {
        checks: vec![port_check, health_check],
        health: observation.payload,
    }
}

async fn probe_public_endpoint(config: &KernelConfig) -> EndpointAssessment {
    let raw = config.deployment.public_url.trim();
    if raw.is_empty() {
        return EndpointAssessment {
            checks: skipped_public_probe_checks("No public endpoint is configured"),
            health: None,
        };
    }
    let Some(url) = validated_public_health_url(config) else {
        return EndpointAssessment {
            checks: skipped_public_probe_checks(
                "The public URL must be corrected before live probes can run",
            ),
            health: None,
        };
    };

    let pinned = match tokio::time::timeout(
        DNS_TIMEOUT,
        crate::ssrf_pin::resolve_pinned_socket_addr(url.as_str(), false),
    )
    .await
    {
        Ok(Ok(pinned)) => pinned,
        Ok(Err(_)) | Err(_) => {
            return EndpointAssessment {
                checks: vec![
                    DeploymentReadinessCheck::failed(
                        "dns",
                        "The public domain did not resolve to a safe public address",
                        "Point the domain to this host, wait for DNS propagation, then run captain doctor --full.",
                    ),
                    DeploymentReadinessCheck::skipped(
                        "tls",
                        "TLS was not attempted because DNS was not ready",
                    ),
                    DeploymentReadinessCheck::skipped(
                        "public_port",
                        "The public port was not probed because DNS was not ready",
                    ),
                    DeploymentReadinessCheck::skipped(
                        "reverse_proxy_live",
                        "Public routing was not probed because DNS was not ready",
                    ),
                    DeploymentReadinessCheck::skipped(
                        "public_health",
                        "Public health was not probed because DNS was not ready",
                    ),
                ],
                health: None,
            };
        }
    };

    let observation = match health_client(Some((&pinned.host, pinned.addr))) {
        Ok(client) => observe_health(&client, url.as_str()).await,
        Err(()) => HealthObservation {
            reached: false,
            payload: None,
            issue: Some(HealthProbeIssue::Transport),
        },
    };
    let tls_check = if url.scheme() != "https" {
        DeploymentReadinessCheck::warning(
            "tls",
            "The public endpoint does not use HTTPS",
            "Enable managed HTTPS before using this deployment in production.",
        )
    } else if observation.reached {
        DeploymentReadinessCheck::ok("tls", "The public TLS handshake succeeded")
    } else {
        DeploymentReadinessCheck::failed(
            "tls",
            "The public TLS handshake could not be verified",
            "Confirm DNS, firewall access, certificate issuance and reverse-proxy health.",
        )
    };
    let public_port_check = if observation.reached {
        DeploymentReadinessCheck::ok(
            "public_port",
            "The configured public port accepted an HTTP response",
        )
    } else {
        DeploymentReadinessCheck::failed(
            "public_port",
            "The configured public port could not be reached",
            "Allow inbound HTTPS traffic and confirm that the reverse proxy is listening.",
        )
    };
    let routing_check = if observation.payload.is_some() {
        DeploymentReadinessCheck::ok(
            "reverse_proxy_live",
            "The reverse proxy serves Captain's public health route",
        )
    } else if observation.reached {
        DeploymentReadinessCheck::warning(
            "reverse_proxy_live",
            "The public endpoint responded but Captain routing was not verified",
            "Check the reverse-proxy route for /api/health and preserve the original Host header.",
        )
    } else {
        DeploymentReadinessCheck::failed(
            "reverse_proxy_live",
            "The public reverse-proxy route could not be reached",
            "Validate the proxy configuration and restart or reload it only after validation succeeds.",
        )
    };
    let health_check = health_check_from_observation(
        "public_health",
        "Public Captain health is healthy",
        "Public Captain health is degraded",
        "The public health endpoint could not be verified",
        "Run captain doctor --full and verify the public /api/health response before production use.",
        &observation,
    );

    EndpointAssessment {
        checks: vec![
            DeploymentReadinessCheck::ok("dns", "The public domain resolves safely"),
            tls_check,
            public_port_check,
            routing_check,
            health_check,
        ],
        health: observation.payload,
    }
}

fn skipped_public_probe_checks(summary: &str) -> Vec<DeploymentReadinessCheck> {
    [
        "dns",
        "tls",
        "public_port",
        "reverse_proxy_live",
        "public_health",
    ]
    .into_iter()
    .map(|id| DeploymentReadinessCheck::skipped(id, summary))
    .collect()
}

fn validated_public_health_url(config: &KernelConfig) -> Option<Url> {
    if public_url_check(config).status != DeploymentCheckStatus::Ok {
        return None;
    }
    let mut url = Url::parse(config.deployment.public_url.trim()).ok()?;
    url.set_path("/api/health");
    url.set_query(None);
    url.set_fragment(None);
    Some(url)
}

fn health_client(pinned: Option<(&str, SocketAddr)>) -> Result<reqwest::Client, ()> {
    let mut builder = reqwest::Client::builder()
        .no_proxy()
        .redirect(reqwest::redirect::Policy::none())
        .connect_timeout(HTTP_CONNECT_TIMEOUT)
        .timeout(HTTP_TIMEOUT)
        .user_agent(format!(
            "captain-deployment-readiness/{}",
            captain_types::version::captain_version()
        ));
    if let Some((host, address)) = pinned {
        builder = builder.resolve(host, address);
    }
    builder.build().map_err(|_| ())
}

async fn observe_health(client: &reqwest::Client, url: &str) -> HealthObservation {
    let response = match tokio::time::timeout(HTTP_TIMEOUT, client.get(url).send()).await {
        Err(_) => return failed_observation(false, HealthProbeIssue::Timeout),
        Ok(Err(_)) => return failed_observation(false, HealthProbeIssue::Transport),
        Ok(Ok(response)) => response,
    };
    let status = response.status();
    if !status.is_success() {
        return failed_observation(true, HealthProbeIssue::HttpStatus(status.as_u16()));
    }
    if response
        .content_length()
        .is_some_and(|length| length > MAX_HEALTH_BODY_BYTES as u64)
    {
        return failed_observation(true, HealthProbeIssue::BodyTooLarge);
    }

    let mut body = Vec::new();
    let mut stream = response.bytes_stream();
    while let Some(chunk) = stream.next().await {
        let chunk = match chunk {
            Ok(chunk) => chunk,
            Err(_) => return failed_observation(true, HealthProbeIssue::Transport),
        };
        let Some(next_len) = body.len().checked_add(chunk.len()) else {
            return failed_observation(true, HealthProbeIssue::BodyTooLarge);
        };
        if next_len > MAX_HEALTH_BODY_BYTES {
            return failed_observation(true, HealthProbeIssue::BodyTooLarge);
        }
        body.extend_from_slice(&chunk);
    }

    match parse_health_payload(&body) {
        Some(payload) => HealthObservation {
            reached: true,
            payload: Some(payload),
            issue: None,
        },
        None => failed_observation(true, HealthProbeIssue::InvalidPayload),
    }
}

fn failed_observation(reached: bool, issue: HealthProbeIssue) -> HealthObservation {
    HealthObservation {
        reached,
        payload: None,
        issue: Some(issue),
    }
}

fn parse_health_payload(bytes: &[u8]) -> Option<HealthPayload> {
    if bytes.is_empty() || bytes.len() > MAX_HEALTH_BODY_BYTES {
        return None;
    }
    let payload: HealthPayload = serde_json::from_slice(bytes).ok()?;
    if !matches!(payload.status.as_str(), "ok" | "degraded")
        || payload.version.is_empty()
        || payload.version.chars().count() > 128
    {
        return None;
    }
    Some(payload)
}

fn health_check_from_observation(
    id: &str,
    healthy_summary: &str,
    degraded_summary: &str,
    failed_summary: &str,
    remediation: &str,
    observation: &HealthObservation,
) -> DeploymentReadinessCheck {
    match observation
        .payload
        .as_ref()
        .map(|payload| payload.status.as_str())
    {
        Some("ok") => DeploymentReadinessCheck::ok(id, healthy_summary),
        Some("degraded") => DeploymentReadinessCheck::warning(id, degraded_summary, remediation),
        _ => {
            let summary = match observation.issue {
                Some(HealthProbeIssue::Timeout) => "The health probe timed out",
                Some(HealthProbeIssue::HttpStatus(status)) => {
                    return DeploymentReadinessCheck::failed(
                        id,
                        &format!("The health endpoint returned HTTP {status}"),
                        remediation,
                    );
                }
                Some(HealthProbeIssue::BodyTooLarge) => {
                    "The health response exceeded the safe size limit"
                }
                Some(HealthProbeIssue::InvalidPayload) => {
                    "The health endpoint returned an invalid payload"
                }
                Some(HealthProbeIssue::Transport) | None => failed_summary,
            };
            DeploymentReadinessCheck::failed(id, summary, remediation)
        }
    }
}

fn version_parity_check(
    config: &KernelConfig,
    local: Option<&HealthPayload>,
    public: Option<&HealthPayload>,
) -> DeploymentReadinessCheck {
    let expected = captain_types::version::captain_version();
    let Some(local) = local else {
        return DeploymentReadinessCheck::skipped(
            "version_parity",
            "Version parity was not evaluated because local health was unavailable",
        );
    };
    if local.version != expected {
        return DeploymentReadinessCheck::failed(
            "version_parity",
            "The local health endpoint reports a different Captain version",
            "Restart the daemon from the installed Captain binary, then run captain doctor --full.",
        );
    }
    if config.deployment.public_url.trim().is_empty() {
        return DeploymentReadinessCheck::skipped(
            "version_parity",
            "No public endpoint is configured for version comparison",
        );
    }
    let Some(public) = public else {
        return DeploymentReadinessCheck::skipped(
            "version_parity",
            "Version parity was not evaluated because public health was unavailable",
        );
    };
    if public.version != expected {
        return DeploymentReadinessCheck::failed(
            "version_parity",
            "The public endpoint serves a different Captain version",
            "Reload the reverse proxy and restart the intended Captain daemon, then verify /api/health again.",
        );
    }
    DeploymentReadinessCheck::ok(
        "version_parity",
        "Local and public endpoints serve the running Captain version",
    )
}

fn configuration_checks(config: &KernelConfig) -> Vec<DeploymentReadinessCheck> {
    vec![
        api_binding_check(config),
        public_url_check(config),
        reverse_proxy_check(config),
    ]
}

fn api_binding_check(config: &KernelConfig) -> DeploymentReadinessCheck {
    match config.api_listen.parse::<SocketAddr>() {
        Ok(address) if address.ip().is_loopback() => {
            DeploymentReadinessCheck::ok("api_binding", "Captain API is bound to loopback")
        }
        Ok(_) => DeploymentReadinessCheck::warning(
            "api_binding",
            "Captain API is not bound to loopback",
            "Bind api_listen to 127.0.0.1 or ::1 when a reverse proxy terminates public traffic.",
        ),
        Err(_) => DeploymentReadinessCheck::failed(
            "api_binding",
            "Captain API listen address is invalid",
            "Set api_listen to a valid host:port value, for example 127.0.0.1:50051.",
        ),
    }
}

fn public_url_check(config: &KernelConfig) -> DeploymentReadinessCheck {
    let raw = config.deployment.public_url.trim();
    if raw.is_empty() {
        return DeploymentReadinessCheck::skipped(
            "public_url",
            "No public deployment URL is configured",
        );
    }
    let parsed =
        match Url::parse(raw) {
            Ok(parsed) => parsed,
            Err(_) => return DeploymentReadinessCheck::failed(
                "public_url",
                "Public deployment URL is invalid",
                "Set deployment.public_url to one HTTPS origin without a path, query or fragment.",
            ),
        };
    let domain_host = matches!(parsed.host(), Some(Host::Domain(_)));
    let clean_origin = matches!(parsed.path(), "" | "/")
        && parsed.query().is_none()
        && parsed.fragment().is_none()
        && parsed.username().is_empty()
        && parsed.password().is_none();
    let expected_scheme = if config.deployment.https {
        parsed.scheme() == "https"
    } else {
        matches!(parsed.scheme(), "http" | "https")
    };
    if !domain_host || !clean_origin || !expected_scheme {
        return DeploymentReadinessCheck::failed(
            "public_url",
            "Public deployment URL does not match the managed-domain contract",
            "Use one DNS hostname origin; managed HTTPS deployments require an https:// URL without credentials, path, query or fragment.",
        );
    }
    DeploymentReadinessCheck::ok("public_url", "Public deployment URL is structurally valid")
}

fn reverse_proxy_check(config: &KernelConfig) -> DeploymentReadinessCheck {
    if config.deployment.public_url.trim().is_empty() {
        return DeploymentReadinessCheck::skipped(
            "reverse_proxy",
            "No public reverse proxy is required",
        );
    }
    if config.deployment.reverse_proxy.trim().is_empty() {
        return DeploymentReadinessCheck::failed(
            "reverse_proxy",
            "No reverse proxy is declared for the public deployment",
            "Set deployment.reverse_proxy to the proxy serving the configured public URL.",
        );
    }
    DeploymentReadinessCheck::ok("reverse_proxy", "A public reverse proxy is declared")
}

fn readiness_state(
    public_configured: bool,
    checks: &[DeploymentReadinessCheck],
) -> DeploymentReadinessState {
    if checks
        .iter()
        .any(|check| check.status == DeploymentCheckStatus::Failed)
    {
        return DeploymentReadinessState::Failed;
    }
    if checks
        .iter()
        .any(|check| check.status == DeploymentCheckStatus::Warning)
    {
        return DeploymentReadinessState::Degraded;
    }
    if checks
        .iter()
        .any(|check| check.status == DeploymentCheckStatus::Pending)
    {
        return DeploymentReadinessState::Pending;
    }
    if public_configured {
        DeploymentReadinessState::Ready
    } else {
        DeploymentReadinessState::NotConfigured
    }
}

fn config_fingerprint(config: &KernelConfig) -> String {
    let mut digest = Sha256::new();
    let version = captain_types::version::captain_version();
    for value in [
        version.as_str(),
        config.api_listen.as_str(),
        config.deployment.profile.as_str(),
        config.deployment.public_url.as_str(),
        if config.deployment.https { "1" } else { "0" },
        config.deployment.reverse_proxy.as_str(),
    ] {
        digest.update((value.len() as u64).to_be_bytes());
        digest.update(value.as_bytes());
    }
    format!("{:x}", digest.finalize())
}

fn snapshot_path(config: &KernelConfig) -> PathBuf {
    config.data_dir.join("health").join(SNAPSHOT_FILE_NAME)
}

fn valid_check_id(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'_')
}

fn bounded_text(value: &str) -> bool {
    !value.is_empty() && value.chars().count() <= MAX_TEXT_CHARS
}

#[cfg(test)]
mod tests {
    use super::*;

    fn public_config(temp: &tempfile::TempDir) -> KernelConfig {
        let mut config = KernelConfig {
            home_dir: temp.path().join("home"),
            data_dir: temp.path().join("data"),
            ..KernelConfig::default()
        };
        config.deployment.profile = "vps".to_string();
        config.deployment.public_url = "https://agent.example.com".to_string();
        config.deployment.reverse_proxy = "caddy".to_string();
        config
    }

    #[test]
    fn public_baseline_is_pending_and_actionable_without_network_claims() {
        let temp = tempfile::tempdir().unwrap();
        let snapshot = DeploymentReadinessSnapshot::pending(&public_config(&temp));

        assert_eq!(snapshot.state, DeploymentReadinessState::Pending);
        assert!(snapshot.operator_actions.is_empty());
        assert_eq!(snapshot.config_fingerprint.len(), 64);
        assert!(snapshot
            .checks
            .iter()
            .any(|check| check.id == "dns" && check.status == DeploymentCheckStatus::Pending));
    }

    #[test]
    fn no_public_domain_is_explicitly_not_configured() {
        let temp = tempfile::tempdir().unwrap();
        let config = KernelConfig {
            home_dir: temp.path().join("home"),
            data_dir: temp.path().join("data"),
            ..KernelConfig::default()
        };
        let snapshot = DeploymentReadinessSnapshot::pending(&config);

        assert_eq!(snapshot.state, DeploymentReadinessState::NotConfigured);
        assert!(snapshot
            .checks
            .iter()
            .filter(|check| matches!(check.id.as_str(), "dns" | "tls" | "public_health"))
            .all(|check| check.status == DeploymentCheckStatus::Skipped));
    }

    #[test]
    fn invalid_public_origin_and_non_loopback_binding_degrade_fail_closed() {
        let temp = tempfile::tempdir().unwrap();
        let mut config = public_config(&temp);
        config.api_listen = "0.0.0.0:50051".to_string();
        config.deployment.public_url = "https://127.0.0.1/private?token=x".to_string();
        let snapshot = DeploymentReadinessSnapshot::pending(&config);

        assert_eq!(snapshot.state, DeploymentReadinessState::Failed);
        assert_eq!(snapshot.operator_actions.len(), 2);
        assert!(
            snapshot
                .checks
                .iter()
                .any(|check| check.id == "public_url"
                    && check.status == DeploymentCheckStatus::Failed)
        );
    }

    #[test]
    fn snapshot_round_trip_is_atomic_and_config_bound() {
        let temp = tempfile::tempdir().unwrap();
        let config = public_config(&temp);
        let snapshot = DeploymentReadinessSnapshot::pending(&config);
        save_snapshot(&config, &snapshot).unwrap();

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                std::fs::metadata(snapshot_path(&config))
                    .unwrap()
                    .permissions()
                    .mode()
                    & 0o777,
                0o600
            );
        }

        assert_eq!(load_snapshot(&config).unwrap(), Some(snapshot));
        let mut changed = config.clone();
        changed.deployment.public_url = "https://other.example.com".to_string();
        assert_eq!(load_snapshot(&changed).unwrap(), None);
    }

    #[test]
    fn malformed_snapshot_fails_closed_without_echoing_payload() {
        let temp = tempfile::tempdir().unwrap();
        let config = public_config(&temp);
        let path = snapshot_path(&config);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, br#"{"operator_actions":["secret-value"]}"#).unwrap();

        let fallback = load_or_pending(&config);
        assert_eq!(fallback.state, DeploymentReadinessState::Degraded);
        let encoded = serde_json::to_string(&fallback).unwrap();
        assert!(!encoded.contains("secret-value"));
        assert!(encoded.contains("cached readiness snapshot could not be verified"));
    }

    #[test]
    fn evaluated_snapshot_deduplicates_bounded_remediation() {
        let temp = tempfile::tempdir().unwrap();
        let config = public_config(&temp);
        let remediation = "Restart the reverse proxy after validating its configuration.";
        let now = Utc::now();
        let snapshot = DeploymentReadinessSnapshot::evaluated(
            &config,
            now,
            42,
            now + chrono::Duration::minutes(5),
            vec![
                DeploymentReadinessCheck::warning("tls", "TLS is pending", remediation),
                DeploymentReadinessCheck::failed(
                    "public_health",
                    "Public health is unavailable",
                    remediation,
                ),
            ],
        );

        assert_eq!(snapshot.state, DeploymentReadinessState::Failed);
        assert_eq!(snapshot.operator_actions, vec![remediation]);
        assert_eq!(snapshot.duration_ms, Some(42));
    }

    #[test]
    fn status_projection_omits_internal_schema_and_fingerprint() {
        let temp = tempfile::tempdir().unwrap();
        let config = public_config(&temp);

        let value = status_value_at(&config, Utc::now());

        assert_eq!(value["state"], serde_json::json!("pending"));
        assert!(value.get("checks").is_some());
        assert!(value.get("schema_version").is_none());
        assert!(value.get("config_fingerprint").is_none());
    }

    #[test]
    fn stale_snapshot_is_reported_as_degraded_without_mutating_disk() {
        let temp = tempfile::tempdir().unwrap();
        let config = public_config(&temp);
        let now = Utc::now();
        let checked_at = now - chrono::Duration::minutes(10);
        let snapshot = DeploymentReadinessSnapshot::evaluated(
            &config,
            checked_at,
            12,
            checked_at + chrono::Duration::minutes(5),
            vec![
                DeploymentReadinessCheck::ok("local_port", "Local port is ready"),
                DeploymentReadinessCheck::ok("local_health", "Local health is ready"),
                DeploymentReadinessCheck::ok("dns", "DNS is ready"),
                DeploymentReadinessCheck::ok("tls", "TLS is ready"),
                DeploymentReadinessCheck::ok("public_port", "Public port is ready"),
                DeploymentReadinessCheck::ok("reverse_proxy_live", "Routing is ready"),
                DeploymentReadinessCheck::ok("public_health", "Public health is ready"),
                DeploymentReadinessCheck::ok("version_parity", "Versions match"),
            ],
        );
        save_snapshot(&config, &snapshot).unwrap();

        let value = status_value_at(&config, now);

        assert_eq!(value["state"], serde_json::json!("degraded"));
        assert!(value["checks"]
            .as_array()
            .unwrap()
            .iter()
            .any(|check| check["id"] == "freshness" && check["status"] == "warning"));
        assert_eq!(load_snapshot(&config).unwrap(), Some(snapshot));
    }

    #[test]
    fn health_payload_parser_is_bounded_and_strict() {
        let healthy = parse_health_payload(br#"{"status":"ok","version":"captain-test"}"#)
            .expect("valid health payload");
        assert_eq!(healthy.status, "ok");
        assert_eq!(healthy.version, "captain-test");

        assert!(parse_health_payload(br#"{"status":"unknown","version":"x"}"#).is_none());
        assert!(parse_health_payload(br#"{"status":"ok","version":""}"#).is_none());
        assert!(parse_health_payload(&vec![b'x'; MAX_HEALTH_BODY_BYTES + 1]).is_none());
    }

    #[tokio::test]
    async fn local_probe_uses_the_real_health_route_and_port() {
        use axum::routing::get;
        use axum::{Json, Router};

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let app = Router::new().route(
            "/api/health",
            get(|| async {
                Json(serde_json::json!({
                    "status": "ok",
                    "version": captain_types::version::captain_version(),
                }))
            }),
        );
        let server = tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });

        let assessment = probe_local_endpoint(address).await;

        assert_eq!(assessment.health.unwrap().status, "ok");
        assert!(assessment
            .checks
            .iter()
            .all(|check| check.status == DeploymentCheckStatus::Ok));
        server.abort();
    }

    #[cfg(unix)]
    #[test]
    fn snapshot_symlink_is_rejected() {
        use std::os::unix::fs::symlink;

        let temp = tempfile::tempdir().unwrap();
        let config = public_config(&temp);
        let path = snapshot_path(&config);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        let target = temp.path().join("untrusted.json");
        std::fs::write(&target, b"{}").unwrap();
        symlink(target, path).unwrap();

        assert!(load_snapshot(&config).is_err());
    }
}
