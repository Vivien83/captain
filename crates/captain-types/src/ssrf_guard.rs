//! Single source of truth for validating outbound callback/webhook URLs
//! against SSRF (server-side request forgery).
//!
//! Three independent copies of this check existed (captain-api's outbound
//! event webhooks, captain-api's agent-API egress callbacks, and
//! captain-kernel's agent-API provisioning-time check) and had already
//! drifted: only one of them exempted `metadata.google.internal` from
//! rejection under the local-testing escape hatch. Consolidating avoids a
//! future fix landing in one copy and not the others.
//!
//! `validate_outbound_callback_url` checks the URL as *given* — it does
//! not resolve DNS, so a hostname that simply resolves to a private or
//! loopback IP passes untouched. Callers that actually perform the HTTP
//! request need two more things this module doesn't do for them: resolve
//! the host themselves and re-check each candidate with
//! [`is_safe_public_ip`], pinning the connection to a validated address
//! (`captain-api`'s `agent_api_egress::resolve_pinned_socket_addr` does
//! this); and disable redirect-following on the client, since a 3xx
//! response from an otherwise-valid host can point anywhere regardless of
//! what was validated.

/// Validate a URL is safe to send an outbound callback/webhook request to.
///
/// - Requires `https`, unless running in a debug build or `allow_local` is
///   set, in which case `http` is also accepted (local smoke testing).
/// - Rejects `localhost` / `ip6-localhost` and loopback IP literals unless
///   `allow_local` is set.
/// - Always rejects `metadata.google.internal` (cloud metadata endpoint)
///   and any private, unspecified, or link-local IP literal — regardless
///   of `allow_local`, since none of those are needed for local testing.
pub fn validate_outbound_callback_url(url: &str, allow_local: bool) -> Result<(), String> {
    let parsed = url::Url::parse(url).map_err(|err| format!("invalid URL: {err}"))?;
    match parsed.scheme() {
        "https" => {}
        "http" => {
            if !(cfg!(debug_assertions) || allow_local) {
                return Err("callback URL must use https".to_string());
            }
        }
        _ => return Err("callback URL must use http or https".to_string()),
    }

    // `Url::host()` gives a typed Domain/Ipv4/Ipv6, unlike `host_str()`
    // which renders IPv6 addresses bracketed (`"[::1]"`) — a bracketed
    // string fails a plain `.parse::<IpAddr>()`, so re-parsing host_str()
    // silently let every IPv6 loopback/private/link-local literal through
    // undetected. Matching on the typed host closes that gap entirely
    // instead of adding another bracket-stripping special case.
    let host = parsed
        .host()
        .ok_or_else(|| "callback URL must include a host".to_string())?;

    match host {
        url::Host::Domain(domain) => {
            let domain = domain.to_ascii_lowercase();
            if domain == "metadata.google.internal" {
                return Err("callback URL must not target metadata hosts".to_string());
            }
            if matches!(domain.as_str(), "localhost" | "ip6-localhost") {
                return if allow_local {
                    Ok(())
                } else {
                    Err("callback URL must not target localhost".to_string())
                };
            }
            Ok(())
        }
        url::Host::Ipv4(ip) => check_ip(std::net::IpAddr::V4(ip), allow_local),
        url::Host::Ipv6(ip) => check_ip(std::net::IpAddr::V6(ip), allow_local),
    }
}

fn check_ip(ip: std::net::IpAddr, allow_local: bool) -> Result<(), String> {
    if is_safe_public_ip(ip, allow_local) {
        Ok(())
    } else if unwrap_ipv4_mapped(ip).is_loopback() {
        Err("callback URL must not target loopback addresses".to_string())
    } else {
        Err("callback URL must not target private or link-local IPs".to_string())
    }
}

/// Same classification as [`validate_outbound_callback_url`]'s IP checks,
/// exposed standalone so callers that resolve a hostname themselves (to
/// pin the connection against DNS rebinding — see module docs) can
/// re-validate each candidate address the same way.
pub fn is_safe_public_ip(ip: std::net::IpAddr, allow_local: bool) -> bool {
    // An IPv4-mapped IPv6 address (`::ffff:127.0.0.1`) doesn't trip
    // `Ipv6Addr::is_loopback()` (only `::1` does) — unwrap it to the
    // embedded IPv4 first, a known SSRF-filter bypass technique.
    let ip = unwrap_ipv4_mapped(ip);
    if ip.is_loopback() {
        return allow_local;
    }
    !(ip.is_unspecified() || is_private_or_link_local(ip))
}

fn unwrap_ipv4_mapped(ip: std::net::IpAddr) -> std::net::IpAddr {
    match ip {
        std::net::IpAddr::V6(v6) => v6.to_ipv4_mapped().map(std::net::IpAddr::V4).unwrap_or(ip),
        v4 => v4,
    }
}

fn is_private_or_link_local(ip: std::net::IpAddr) -> bool {
    match ip {
        std::net::IpAddr::V4(ip) => ip.is_private() || ip.is_link_local(),
        std::net::IpAddr::V6(ip) => ip.is_unique_local() || (ip.segments()[0] & 0xffc0) == 0xfe80,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_localhost_and_loopback_without_escape_hatch() {
        assert!(validate_outbound_callback_url("http://localhost:8080/hook", false).is_err());
        assert!(validate_outbound_callback_url("https://127.0.0.1/hook", false).is_err());
        assert!(validate_outbound_callback_url("https://[::1]/hook", false).is_err());
    }

    #[test]
    fn allows_localhost_and_loopback_with_escape_hatch() {
        assert!(validate_outbound_callback_url("http://localhost:8080/hook", true).is_ok());
        assert!(validate_outbound_callback_url("https://127.0.0.1/hook", true).is_ok());
    }

    #[test]
    fn metadata_host_is_never_allowed() {
        assert!(
            validate_outbound_callback_url("https://metadata.google.internal/hook", false).is_err()
        );
        assert!(
            validate_outbound_callback_url("https://metadata.google.internal/hook", true).is_err(),
            "metadata host must stay rejected even with the local escape hatch"
        );
    }

    #[test]
    fn rejects_private_and_link_local_ips_even_with_escape_hatch() {
        assert!(validate_outbound_callback_url("https://192.168.1.5/hook", true).is_err());
        assert!(validate_outbound_callback_url("https://10.0.0.1/hook", true).is_err());
        assert!(validate_outbound_callback_url("https://169.254.169.254/hook", true).is_err());
    }

    #[test]
    fn rejects_ipv4_mapped_ipv6_loopback_bypass() {
        assert!(
            validate_outbound_callback_url("https://[::ffff:127.0.0.1]/hook", false).is_err(),
            "IPv4-mapped IPv6 loopback must not bypass the loopback check"
        );
        assert!(
            validate_outbound_callback_url("https://[::ffff:169.254.169.254]/hook", true).is_err(),
            "IPv4-mapped IPv6 link-local must not bypass the private/link-local check"
        );
    }

    #[test]
    fn rejects_unspecified_address() {
        assert!(validate_outbound_callback_url("https://0.0.0.0/hook", false).is_err());
    }

    #[test]
    fn accepts_public_https_url() {
        assert!(validate_outbound_callback_url("https://example.com/hook", false).is_ok());
    }

    #[test]
    fn rejects_non_http_scheme() {
        assert!(validate_outbound_callback_url("ftp://example.com/hook", false).is_err());
    }

    #[test]
    fn rejects_malformed_url() {
        assert!(validate_outbound_callback_url("not a url", false).is_err());
    }
}
