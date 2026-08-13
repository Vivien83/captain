use axum::http::{HeaderMap, Uri};
use captain_kernel::hub_pairing_service::DeviceAccessIdentity;
use captain_types::config::{DeploymentConfig, SessionCookieSecurePolicy};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::sync::Mutex;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

pub const REALTIME_TICKET_TTL: Duration = Duration::from_secs(30);
const LOGIN_BACKOFF_AFTER_FAILURES: u32 = 5;
const LOGIN_BACKOFF_MAX: Duration = Duration::from_secs(15 * 60);
const LOGIN_STATE_RETENTION: Duration = Duration::from_secs(60 * 60);
const LOGIN_SATURATION_BACKOFF: Duration = Duration::from_secs(5);
const MAX_LOGIN_STATE_KEYS: usize = 4096;
const MAX_REALTIME_TICKETS: usize = 4096;

#[derive(Debug, Clone)]
pub struct RealtimeTicketAuthorization {
    pub client: Option<DeviceAccessIdentity>,
}

#[derive(Debug)]
pub struct RealtimeTicketGrant {
    pub ticket: String,
    pub expires_at_unix: u64,
}

#[derive(Debug, Default)]
pub struct WebAuthSecurity {
    login: Mutex<LoginAttemptState>,
    tickets: Mutex<RealtimeTicketState>,
    password_migration: Mutex<()>,
}

impl WebAuthSecurity {
    pub fn login_retry_after(&self, ip: IpAddr, username: &str, now: Instant) -> Option<Duration> {
        let mut state = self.login.lock().unwrap_or_else(|error| error.into_inner());
        state.prune(now);
        let ip_retry = state
            .by_ip
            .get(&ip)
            .and_then(|entry| entry.retry_after(now));
        let user_key = username_key(username);
        let user_retry = state
            .by_user
            .get(&user_key)
            .and_then(|entry| entry.retry_after(now));
        [ip_retry, user_retry, state.saturation_retry_after(now)]
            .into_iter()
            .flatten()
            .max()
    }

    pub fn record_login_failure(&self, ip: IpAddr, username: &str, now: Instant) {
        let mut state = self.login.lock().unwrap_or_else(|error| error.into_inner());
        state.prune(now);
        let mut saturated = false;
        if ensure_map_capacity(&mut state.by_ip, ip, now) {
            state
                .by_ip
                .entry(ip)
                .or_insert_with(|| AttemptRecord::new(now))
                .record_failure(now);
        } else {
            saturated = true;
        }
        let user_key = username_key(username);
        if ensure_map_capacity(&mut state.by_user, user_key, now) {
            state
                .by_user
                .entry(user_key)
                .or_insert_with(|| AttemptRecord::new(now))
                .record_failure(now);
        } else {
            saturated = true;
        }
        if saturated {
            state.activate_saturation(now);
        }
    }

    pub fn record_login_success(&self, ip: IpAddr, username: &str) {
        let mut state = self.login.lock().unwrap_or_else(|error| error.into_inner());
        state.by_ip.remove(&ip);
        state.by_user.remove(&username_key(username));
    }

    pub fn with_password_migration<T>(&self, operation: impl FnOnce() -> T) -> T {
        let _guard = self
            .password_migration
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        operation()
    }

    pub fn issue_realtime_ticket(
        &self,
        path: &str,
        ip: IpAddr,
        session_epoch: u64,
        client: Option<DeviceAccessIdentity>,
        now: Instant,
    ) -> Result<RealtimeTicketGrant, String> {
        if !is_realtime_transport_path(path) {
            return Err("Unsupported realtime path".to_string());
        }
        let ticket = captain_types::config::generate_session_secret()
            .map_err(|error| format!("OS CSPRNG unavailable: {error}"))?;
        let expires_at_unix = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .saturating_add(REALTIME_TICKET_TTL)
            .as_secs();
        let mut state = self
            .tickets
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        state.tickets.retain(|_, record| record.expires_at > now);
        if state.tickets.len() >= MAX_REALTIME_TICKETS {
            return Err("Realtime ticket capacity reached".to_string());
        }
        state.tickets.insert(
            ticket.clone(),
            RealtimeTicketRecord {
                path: path.to_string(),
                ip,
                session_epoch,
                client,
                expires_at: now + REALTIME_TICKET_TTL,
            },
        );
        Ok(RealtimeTicketGrant {
            ticket,
            expires_at_unix,
        })
    }

    pub fn consume_realtime_ticket(
        &self,
        ticket: &str,
        path: &str,
        ip: IpAddr,
        session_epoch: u64,
        now: Instant,
    ) -> Option<RealtimeTicketAuthorization> {
        if ticket.is_empty() || ticket.len() > 128 || !is_realtime_transport_path(path) {
            return None;
        }
        let mut state = self
            .tickets
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        let record = state.tickets.remove(ticket)?;
        (record.expires_at > now
            && record.path == path
            && record.ip == ip
            && record.session_epoch == session_epoch)
            .then_some(RealtimeTicketAuthorization {
                client: record.client,
            })
    }
}

#[derive(Debug, Default)]
struct LoginAttemptState {
    by_ip: HashMap<IpAddr, AttemptRecord>,
    by_user: HashMap<[u8; 32], AttemptRecord>,
    saturated_until: Option<Instant>,
}

impl LoginAttemptState {
    fn prune(&mut self, now: Instant) {
        self.by_ip.retain(|_, entry| {
            now.saturating_duration_since(entry.last_seen) < LOGIN_STATE_RETENTION
        });
        self.by_user.retain(|_, entry| {
            now.saturating_duration_since(entry.last_seen) < LOGIN_STATE_RETENTION
        });
        if self.saturated_until.is_some_and(|until| until <= now) {
            self.saturated_until = None;
        }
    }

    fn saturation_retry_after(&self, now: Instant) -> Option<Duration> {
        self.saturated_until
            .and_then(|until| until.checked_duration_since(now))
            .filter(|duration| !duration.is_zero())
    }

    fn activate_saturation(&mut self, now: Instant) {
        let was_active = self.saturation_retry_after(now).is_some();
        self.saturated_until = Some(now + LOGIN_SATURATION_BACKOFF);
        if !was_active {
            tracing::warn!(
                max_keys = MAX_LOGIN_STATE_KEYS,
                retry_after_seconds = LOGIN_SATURATION_BACKOFF.as_secs(),
                "Web login limiter saturated; applying global fail-closed backoff"
            );
        }
    }
}

#[derive(Debug)]
struct AttemptRecord {
    failures: u32,
    blocked_until: Instant,
    last_seen: Instant,
}

impl AttemptRecord {
    fn new(now: Instant) -> Self {
        Self {
            failures: 0,
            blocked_until: now,
            last_seen: now,
        }
    }

    fn retry_after(&self, now: Instant) -> Option<Duration> {
        self.blocked_until
            .checked_duration_since(now)
            .filter(|duration| !duration.is_zero())
    }

    fn record_failure(&mut self, now: Instant) {
        self.failures = self.failures.saturating_add(1);
        self.last_seen = now;
        if self.failures < LOGIN_BACKOFF_AFTER_FAILURES {
            return;
        }
        let shift = (self.failures - LOGIN_BACKOFF_AFTER_FAILURES).min(30);
        let seconds = 1u64
            .checked_shl(shift)
            .unwrap_or(LOGIN_BACKOFF_MAX.as_secs())
            .min(LOGIN_BACKOFF_MAX.as_secs());
        self.blocked_until = now + Duration::from_secs(seconds);
    }
}

fn ensure_map_capacity<K>(map: &mut HashMap<K, AttemptRecord>, key: K, now: Instant) -> bool
where
    K: Copy + Eq + std::hash::Hash,
{
    if map.contains_key(&key) || map.len() < MAX_LOGIN_STATE_KEYS {
        return true;
    }
    if let Some(oldest) = map
        .iter()
        .filter(|(_, entry)| entry.retry_after(now).is_none())
        .max_by_key(|(_, entry)| now.saturating_duration_since(entry.last_seen))
        .map(|(key, _)| *key)
    {
        map.remove(&oldest);
        return true;
    }
    false
}

fn username_key(username: &str) -> [u8; 32] {
    Sha256::digest(username.trim().to_ascii_lowercase().as_bytes()).into()
}

#[derive(Debug, Default)]
struct RealtimeTicketState {
    tickets: HashMap<String, RealtimeTicketRecord>,
}

#[derive(Debug)]
struct RealtimeTicketRecord {
    path: String,
    ip: IpAddr,
    session_epoch: u64,
    client: Option<DeviceAccessIdentity>,
    expires_at: Instant,
}

pub fn realtime_ticket_from_uri(uri: &Uri) -> Option<&str> {
    uri.query().and_then(|query| {
        query
            .split('&')
            .find_map(|pair| pair.strip_prefix("ticket="))
    })
}

pub fn is_realtime_transport_path(path: &str) -> bool {
    if matches!(
        path,
        "/api/memory/events" | "/api/logs/stream" | "/api/comms/events/stream"
    ) {
        return true;
    }
    if let Some(agent_id) = path
        .strip_prefix("/api/agents/")
        .and_then(|value| value.strip_suffix("/ws"))
    {
        return uuid::Uuid::parse_str(agent_id).is_ok();
    }
    path.strip_prefix("/api/sessions/")
        .and_then(|value| value.strip_suffix("/terminal"))
        .is_some_and(|session| {
            !session.is_empty()
                && session.len() <= 80
                && session
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'))
        })
}

pub fn request_client_ip(
    peer: Option<SocketAddr>,
    headers: &HeaderMap,
    deployment: &DeploymentConfig,
) -> IpAddr {
    let peer_ip = peer
        .map(|address| address.ip())
        .unwrap_or(IpAddr::V4(Ipv4Addr::LOCALHOST));
    if trusted_reverse_proxy(peer_ip, deployment) {
        if let Some(forwarded) = headers
            .get("x-forwarded-for")
            .and_then(|value| value.to_str().ok())
            .and_then(|value| value.split(',').next())
            .and_then(|value| value.trim().parse::<IpAddr>().ok())
        {
            return forwarded;
        }
    }
    peer_ip
}

/// Return whether the actual client is loopback.
///
/// A local reverse proxy configured for a public deployment must provide a
/// loopback `X-Forwarded-For` client too; its own loopback peer address is not
/// sufficient to enable credentialless access.
pub fn request_client_is_loopback(
    peer: Option<SocketAddr>,
    headers: &HeaderMap,
    deployment: &DeploymentConfig,
) -> bool {
    let Some(peer_ip) = peer.map(|address| address.ip()) else {
        return false;
    };
    if trusted_reverse_proxy(peer_ip, deployment) {
        return headers
            .get("x-forwarded-for")
            .and_then(|value| value.to_str().ok())
            .map(str::trim)
            .filter(|value| !value.contains(','))
            .and_then(|value| value.parse::<IpAddr>().ok())
            .is_some_and(|client| client.is_loopback());
    }
    peer_ip.is_loopback()
}

pub fn session_cookie_is_secure(
    policy: SessionCookieSecurePolicy,
    peer: Option<SocketAddr>,
    headers: &HeaderMap,
    deployment: &DeploymentConfig,
) -> bool {
    match policy {
        SessionCookieSecurePolicy::Always => true,
        SessionCookieSecurePolicy::Never => false,
        SessionCookieSecurePolicy::Auto => {
            public_url_is_https(&deployment.public_url)
                || trusted_forwarded_https(peer, headers, deployment)
        }
    }
}

fn public_url_is_https(public_url: &str) -> bool {
    public_url
        .trim()
        .get(..8)
        .is_some_and(|prefix| prefix.eq_ignore_ascii_case("https://"))
}

fn trusted_forwarded_https(
    peer: Option<SocketAddr>,
    headers: &HeaderMap,
    deployment: &DeploymentConfig,
) -> bool {
    let Some(peer_ip) = peer.map(|address| address.ip()) else {
        return false;
    };
    trusted_reverse_proxy(peer_ip, deployment)
        && headers
            .get("x-forwarded-proto")
            .and_then(|value| value.to_str().ok())
            .and_then(|value| value.split(',').next())
            .is_some_and(|value| value.trim().eq_ignore_ascii_case("https"))
}

fn trusted_reverse_proxy(peer_ip: IpAddr, deployment: &DeploymentConfig) -> bool {
    peer_ip.is_loopback()
        && !deployment.public_url.trim().is_empty()
        && !deployment.reverse_proxy.trim().is_empty()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sixth_immediate_attempt_is_rate_limited_by_ip_and_user() {
        let security = WebAuthSecurity::default();
        let ip: IpAddr = "203.0.113.7".parse().unwrap();
        let now = Instant::now();

        for _ in 0..5 {
            assert!(security.login_retry_after(ip, "owner", now).is_none());
            security.record_login_failure(ip, "owner", now);
        }

        assert!(security.login_retry_after(ip, "owner", now).is_some());
        assert!(security.login_retry_after(ip, "OWNER", now).is_some());
        assert!(security
            .login_retry_after("203.0.113.8".parse().unwrap(), "owner", now)
            .is_some());
    }

    #[test]
    fn realtime_ticket_is_path_ip_epoch_bound_and_single_use() {
        let security = WebAuthSecurity::default();
        let ip: IpAddr = "203.0.113.9".parse().unwrap();
        let now = Instant::now();
        let path = "/api/sessions/captain/terminal";
        let grant = security
            .issue_realtime_ticket(path, ip, 4, None, now)
            .unwrap();

        assert!(security
            .consume_realtime_ticket(&grant.ticket, path, "203.0.113.10".parse().unwrap(), 4, now,)
            .is_none());
        assert!(security
            .consume_realtime_ticket(&grant.ticket, path, ip, 4, now)
            .is_none());

        let grant = security
            .issue_realtime_ticket(path, ip, 4, None, now)
            .unwrap();
        assert!(security
            .consume_realtime_ticket(&grant.ticket, path, ip, 4, now)
            .is_some());
        assert!(security
            .consume_realtime_ticket(&grant.ticket, path, ip, 4, now)
            .is_none());

        let grant = security
            .issue_realtime_ticket(path, ip, 4, None, now)
            .unwrap();
        assert!(security
            .consume_realtime_ticket(&grant.ticket, path, ip, 5, now)
            .is_none());

        let grant = security
            .issue_realtime_ticket(path, ip, 4, None, now)
            .unwrap();
        assert!(security
            .consume_realtime_ticket(&grant.ticket, path, ip, 4, now + REALTIME_TICKET_TTL)
            .is_none());
    }

    #[test]
    fn realtime_ticket_preserves_paired_client_identity() {
        let security = WebAuthSecurity::default();
        let ip: IpAddr = "203.0.113.11".parse().unwrap();
        let now = Instant::now();
        let path = "/api/memory/events";
        let identity = DeviceAccessIdentity {
            device_id: "client-1".to_string(),
            role: captain_wire::DeviceRole::Client,
            grants_json: "{}".to_string(),
            protocol_version: captain_wire::HUB_NODE_PROTOCOL_VERSION,
        };

        let grant = security
            .issue_realtime_ticket(path, ip, 9, Some(identity.clone()), now)
            .unwrap();
        let authorization = security
            .consume_realtime_ticket(&grant.ticket, path, ip, 9, now)
            .expect("ticket should be accepted once");

        assert_eq!(authorization.client, Some(identity));
    }

    #[test]
    fn loopback_auth_bypass_uses_the_actual_client_behind_a_public_proxy() {
        let peer = Some("127.0.0.1:8443".parse().unwrap());
        let deployment = DeploymentConfig {
            public_url: "https://captain.example".to_string(),
            reverse_proxy: "caddy".to_string(),
            ..Default::default()
        };
        let mut headers = HeaderMap::new();

        assert!(!request_client_is_loopback(peer, &headers, &deployment));

        headers.insert("x-forwarded-for", "203.0.113.8".parse().unwrap());
        assert!(!request_client_is_loopback(peer, &headers, &deployment));

        headers.insert("x-forwarded-for", "127.0.0.1".parse().unwrap());
        assert!(request_client_is_loopback(peer, &headers, &deployment));

        headers.insert("x-forwarded-for", "127.0.0.1, 203.0.113.8".parse().unwrap());
        assert!(!request_client_is_loopback(peer, &headers, &deployment));
    }

    #[test]
    fn loopback_auth_bypass_requires_peer_metadata() {
        assert!(!request_client_is_loopback(
            None,
            &HeaderMap::new(),
            &DeploymentConfig::default(),
        ));
        assert!(request_client_is_loopback(
            Some("127.0.0.1:50051".parse().unwrap()),
            &HeaderMap::new(),
            &DeploymentConfig::default(),
        ));
        assert!(!request_client_is_loopback(
            Some("203.0.113.8:50051".parse().unwrap()),
            &HeaderMap::new(),
            &DeploymentConfig::default(),
        ));
    }

    #[test]
    fn login_backoff_never_exceeds_fifteen_minutes() {
        let now = Instant::now();
        let mut record = AttemptRecord::new(now);
        for _ in 0..40 {
            record.record_failure(now);
        }
        assert_eq!(record.retry_after(now), Some(LOGIN_BACKOFF_MAX));
    }

    #[test]
    fn capacity_pressure_never_evicts_an_active_login_block() {
        let now = Instant::now();
        let mut map = HashMap::new();
        for key in 0..MAX_LOGIN_STATE_KEYS {
            map.insert(
                key,
                AttemptRecord {
                    failures: LOGIN_BACKOFF_AFTER_FAILURES,
                    blocked_until: now + LOGIN_BACKOFF_MAX,
                    last_seen: now,
                },
            );
        }

        assert!(!ensure_map_capacity(
            &mut map,
            MAX_LOGIN_STATE_KEYS + 1,
            now
        ));
        assert_eq!(map.len(), MAX_LOGIN_STATE_KEYS);
        assert!(map.contains_key(&0));
    }

    #[test]
    fn capacity_pressure_reuses_only_a_non_blocked_login_slot() {
        let now = Instant::now();
        let mut map = HashMap::new();
        for key in 0..MAX_LOGIN_STATE_KEYS {
            map.insert(
                key,
                AttemptRecord {
                    failures: LOGIN_BACKOFF_AFTER_FAILURES,
                    blocked_until: now + LOGIN_BACKOFF_MAX,
                    last_seen: now,
                },
            );
        }
        map.insert(
            7,
            AttemptRecord {
                failures: LOGIN_BACKOFF_AFTER_FAILURES,
                blocked_until: now,
                last_seen: now.checked_sub(Duration::from_secs(30)).unwrap_or(now),
            },
        );

        assert!(ensure_map_capacity(&mut map, MAX_LOGIN_STATE_KEYS + 1, now));
        assert_eq!(map.len(), MAX_LOGIN_STATE_KEYS - 1);
        assert!(!map.contains_key(&7));
        assert!(map.contains_key(&8));
    }

    #[test]
    fn saturated_login_maps_apply_a_short_global_fail_closed_backoff() {
        let security = WebAuthSecurity::default();
        let now = Instant::now();
        {
            let mut state = security.login.lock().unwrap();
            for index in 0..MAX_LOGIN_STATE_KEYS {
                let ip = IpAddr::V6(std::net::Ipv6Addr::from(index as u128));
                let user = Sha256::digest(format!("blocked-user-{index}").as_bytes()).into();
                let record = || AttemptRecord {
                    failures: LOGIN_BACKOFF_AFTER_FAILURES,
                    blocked_until: now + LOGIN_BACKOFF_MAX,
                    last_seen: now,
                };
                state.by_ip.insert(ip, record());
                state.by_user.insert(user, record());
            }
        }

        let fresh_ip = IpAddr::V6(std::net::Ipv6Addr::from(
            u128::try_from(MAX_LOGIN_STATE_KEYS).unwrap() + 1,
        ));
        security.record_login_failure(fresh_ip, "fresh-user", now);

        assert_eq!(
            security.login_retry_after(fresh_ip, "fresh-user", now),
            Some(LOGIN_SATURATION_BACKOFF)
        );
        assert!(security
            .login_retry_after(
                IpAddr::V6(std::net::Ipv6Addr::from(
                    u128::try_from(MAX_LOGIN_STATE_KEYS).unwrap() + 2
                )),
                "another-fresh-user",
                now
            )
            .is_some());
        assert!(security
            .login_retry_after(fresh_ip, "fresh-user", now + LOGIN_SATURATION_BACKOFF)
            .is_none());

        let state = security.login.lock().unwrap();
        assert_eq!(state.by_ip.len(), MAX_LOGIN_STATE_KEYS);
        assert_eq!(state.by_user.len(), MAX_LOGIN_STATE_KEYS);
    }

    #[test]
    fn secure_cookie_auto_requires_declared_or_trusted_https() {
        let mut deployment = DeploymentConfig::default();
        deployment.public_url.clear();
        let peer: SocketAddr = "127.0.0.1:50000".parse().unwrap();
        let mut headers = HeaderMap::new();
        headers.insert("x-forwarded-proto", "https".parse().unwrap());

        assert!(!session_cookie_is_secure(
            SessionCookieSecurePolicy::Auto,
            Some(peer),
            &headers,
            &deployment,
        ));
        deployment.public_url = "https://captain.example.com".to_string();
        assert!(session_cookie_is_secure(
            SessionCookieSecurePolicy::Auto,
            Some(peer),
            &headers,
            &deployment,
        ));
        deployment.public_url = "http://captain.example.com".to_string();
        deployment.reverse_proxy = "caddy".to_string();
        assert!(session_cookie_is_secure(
            SessionCookieSecurePolicy::Auto,
            Some(peer),
            &headers,
            &deployment,
        ));
        let untrusted_peer: SocketAddr = "203.0.113.5:50000".parse().unwrap();
        assert!(!session_cookie_is_secure(
            SessionCookieSecurePolicy::Auto,
            Some(untrusted_peer),
            &headers,
            &deployment,
        ));
        assert!(!session_cookie_is_secure(
            SessionCookieSecurePolicy::Never,
            Some(peer),
            &headers,
            &deployment,
        ));
    }
}
