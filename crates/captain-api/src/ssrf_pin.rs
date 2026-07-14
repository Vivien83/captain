//! DNS resolution pinned against SSRF, shared by every outbound callback
//! path (agent-API egress callbacks, outbound event webhooks).
//!
//! `captain_types::ssrf_guard::validate_outbound_callback_url` checks the
//! URL string as given — it never resolves DNS, so a callback host that
//! simply resolves to a private/loopback address passes untouched. This
//! module resolves the host, re-validates each candidate address with
//! [`captain_types::ssrf_guard::is_safe_public_ip`], and returns one safe
//! address to pin the actual request to via `reqwest::ClientBuilder::resolve`
//! — so a second, different DNS answer at connect time can't retarget the
//! request either.

/// A DNS resolution pinned to one address safe to connect to.
#[derive(Debug)]
pub(crate) struct PinnedAddr {
    /// The hostname the caller's URL used — passed to `.resolve()` so
    /// reqwest overrides only lookups for that exact host.
    pub(crate) host: String,
    pub(crate) addr: std::net::SocketAddr,
}

/// Resolve `url`'s host and pick the first candidate address that is not
/// loopback/private/link-local/unspecified (respecting `allow_local`, same
/// rule as `validate_outbound_callback_url`). Fails if resolution finds no
/// safe candidate at all — DNS answering only with internal addresses is
/// exactly the case this guards against, so there is nothing safe to fail
/// open to.
pub(crate) async fn resolve_pinned_socket_addr(
    url: &str,
    allow_local: bool,
) -> Result<PinnedAddr, String> {
    let parsed = reqwest::Url::parse(url).map_err(|err| format!("invalid URL: {err}"))?;
    let host = parsed
        .host_str()
        .ok_or_else(|| "callback URL must include a host".to_string())?
        .to_string();
    let port = parsed
        .port_or_known_default()
        .ok_or_else(|| "callback URL scheme has no default port".to_string())?;

    let candidates: Vec<std::net::SocketAddr> = tokio::net::lookup_host((host.as_str(), port))
        .await
        .map_err(|err| format!("DNS resolution failed: {err}"))?
        .collect();

    candidates
        .into_iter()
        .find(|addr| captain_types::ssrf_guard::is_safe_public_ip(addr.ip(), allow_local))
        .map(|addr| PinnedAddr { host, addr })
        .ok_or_else(|| "callback host did not resolve to any public IP address".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn rejects_loopback_without_escape_hatch() {
        let err = resolve_pinned_socket_addr("http://localhost:9/", false)
            .await
            .expect_err("loopback resolution must fail closed without allow_local");
        assert!(err.contains("public IP"), "got: {err}");
    }

    #[tokio::test]
    async fn allows_loopback_with_escape_hatch() {
        let pinned = resolve_pinned_socket_addr("http://localhost:9/", true)
            .await
            .expect("loopback resolution must succeed with allow_local");
        assert!(pinned.addr.ip().is_loopback());
        assert_eq!(pinned.host, "localhost");
    }

    /// An IP literal in the URL needs no DNS lookup — same function,
    /// same safety rule, immediate result.
    #[tokio::test]
    async fn handles_ip_literal_without_lookup() {
        let pinned = resolve_pinned_socket_addr("http://127.0.0.1:9/", true)
            .await
            .expect("literal loopback IP with allow_local must resolve");
        assert_eq!(pinned.addr.ip(), std::net::IpAddr::from([127, 0, 0, 1]));
    }
}
