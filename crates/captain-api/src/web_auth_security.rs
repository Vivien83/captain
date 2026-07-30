use axum::http::{HeaderMap, Uri};
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
const MAX_LOGIN_STATE_KEYS: usize = 4096;
const MAX_REALTIME_TICKETS: usize = 4096;

#[derive(Debug, Clone, Copy)]
pub struct RealtimeTicketAuthorization;

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
        match (ip_retry, user_retry) {
            (Some(left), Some(right)) => Some(left.max(right)),
            (Some(retry), None) | (None, Some(retry)) => Some(retry),
            (None, None) => None,
        }
    }

    pub fn record_login_failure(&self, ip: IpAddr, username: &str, now: Instant) {
        let mut state = self.login.lock().unwrap_or_else(|error| error.into_inner());
        state.prune(now);
        ensure_map_capacity(&mut state.by_ip, ip, now);
        state
            .by_ip
            .entry(ip)
            .or_insert_with(|| AttemptRecord::new(now))
            .record_failure(now);
        let user_key = username_key(username);
        ensure_map_capacity(&mut state.by_user, user_key, now);
        state
            .by_user
            .entry(user_key)
            .or_insert_with(|| AttemptRecord::new(now))
            .record_failure(now);
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
    ) -> bool {
        if ticket.is_empty() || ticket.len() > 128 || !is_realtime_transport_path(path) {
            return false;
        }
        let mut state = self
            .tickets
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        let Some(record) = state.tickets.remove(ticket) else {
            return false;
        };
        record.expires_at > now
            && record.path == path
            && record.ip == ip
            && record.session_epoch == session_epoch
    }
}

#[derive(Debug, Default)]
struct LoginAttemptState {
    by_ip: HashMap<IpAddr, AttemptRecord>,
    by_user: HashMap<[u8; 32], AttemptRecord>,
}

impl LoginAttemptState {
    fn prune(&mut self, now: Instant) {
        self.by_ip.retain(|_, entry| {
            now.saturating_duration_since(entry.last_seen) < LOGIN_STATE_RETENTION
        });
        self.by_user.retain(|_, entry| {
            now.saturating_duration_since(entry.last_seen) < LOGIN_STATE_RETENTION
        });
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

fn ensure_map_capacity<K>(map: &mut HashMap<K, AttemptRecord>, key: K, now: Instant)
where
    K: Copy + Eq + std::hash::Hash,
{
    if map.contains_key(&key) || map.len() < MAX_LOGIN_STATE_KEYS {
        return;
    }
    if let Some(oldest) = map
        .iter()
        .max_by_key(|(_, entry)| now.saturating_duration_since(entry.last_seen))
        .map(|(key, _)| *key)
    {
        map.remove(&oldest);
    }
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
        let grant = security.issue_realtime_ticket(path, ip, 4, now).unwrap();

        assert!(!security.consume_realtime_ticket(
            &grant.ticket,
            path,
            "203.0.113.10".parse().unwrap(),
            4,
            now,
        ));
        assert!(!security.consume_realtime_ticket(&grant.ticket, path, ip, 4, now));

        let grant = security.issue_realtime_ticket(path, ip, 4, now).unwrap();
        assert!(security.consume_realtime_ticket(&grant.ticket, path, ip, 4, now));
        assert!(!security.consume_realtime_ticket(&grant.ticket, path, ip, 4, now));

        let grant = security.issue_realtime_ticket(path, ip, 4, now).unwrap();
        assert!(!security.consume_realtime_ticket(&grant.ticket, path, ip, 5, now));

        let grant = security.issue_realtime_ticket(path, ip, 4, now).unwrap();
        assert!(!security.consume_realtime_ticket(
            &grant.ticket,
            path,
            ip,
            4,
            now + REALTIME_TICKET_TTL
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
