use axum::extract::{Request, State};
use axum::http::header::{ACCEPT, AUTHORIZATION, CONTENT_TYPE, HOST};
use axum::http::{HeaderName, HeaderValue, Method, StatusCode};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use captain_types::config::{ApiConfig, DeploymentConfig};
use std::collections::{BTreeMap, BTreeSet};
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;
use tower_http::cors::CorsLayer;
use url::Url;

const LOCALHOST: &str = "localhost";
const IPV4_LOOPBACK: &str = "127.0.0.1";
const IPV6_LOOPBACK: &str = "::1";

#[derive(Clone, Debug)]
pub(crate) struct HostPolicy {
    allowed_hosts: Arc<BTreeSet<String>>,
}

#[derive(Clone, Debug)]
pub(crate) struct RequestOriginPolicy {
    allowed_origins: Vec<HeaderValue>,
    host_policy: HostPolicy,
}

impl RequestOriginPolicy {
    pub(crate) fn from_config(
        api: &ApiConfig,
        deployment: &DeploymentConfig,
        listen_addr: SocketAddr,
    ) -> Self {
        let mut origins = BTreeMap::<String, HeaderValue>::new();
        let mut allowed_hosts = BTreeSet::from([
            LOCALHOST.to_string(),
            IPV4_LOOPBACK.to_string(),
            IPV6_LOOPBACK.to_string(),
        ]);

        for origin in default_loopback_origins(listen_addr.port()) {
            insert_origin(
                &origin,
                "loopback default",
                &mut origins,
                &mut allowed_hosts,
            );
        }

        if !listen_addr.ip().is_unspecified() {
            allowed_hosts.insert(normalize_host(&listen_addr.ip().to_string()));
        }

        if !deployment.public_url.trim().is_empty() {
            insert_origin(
                deployment.public_url.trim(),
                "deployment.public_url",
                &mut origins,
                &mut allowed_hosts,
            );
        }

        for (index, origin) in api.allowed_origins.iter().enumerate() {
            insert_origin(
                origin.trim(),
                "api.allowed_origins",
                &mut origins,
                &mut allowed_hosts,
            );
            if origin.trim().is_empty() {
                tracing::warn!(
                    origin_index = index,
                    "ignored empty api.allowed_origins entry"
                );
            }
        }

        Self {
            allowed_origins: origins.into_values().collect(),
            host_policy: HostPolicy {
                allowed_hosts: Arc::new(allowed_hosts),
            },
        }
    }

    pub(crate) fn cors_layer(&self) -> CorsLayer {
        CorsLayer::new()
            .allow_origin(self.allowed_origins.clone())
            .allow_methods([
                Method::GET,
                Method::HEAD,
                Method::POST,
                Method::PUT,
                Method::PATCH,
                Method::DELETE,
                Method::OPTIONS,
            ])
            .allow_headers([
                ACCEPT,
                AUTHORIZATION,
                CONTENT_TYPE,
                HeaderName::from_static("x-filename"),
            ])
            .allow_credentials(true)
            .max_age(Duration::from_secs(600))
    }

    pub(crate) fn host_policy(&self) -> HostPolicy {
        self.host_policy.clone()
    }
}

fn default_loopback_origins(port: u16) -> [String; 3] {
    [
        format!("http://{LOCALHOST}:{port}"),
        format!("http://{IPV4_LOOPBACK}:{port}"),
        format!("http://[{IPV6_LOOPBACK}]:{port}"),
    ]
}

fn insert_origin(
    raw: &str,
    source: &'static str,
    origins: &mut BTreeMap<String, HeaderValue>,
    allowed_hosts: &mut BTreeSet<String>,
) {
    match parse_origin(raw) {
        Ok((canonical, host)) => {
            let Ok(header) = HeaderValue::from_str(&canonical) else {
                tracing::warn!(source, "ignored API origin with an invalid header value");
                return;
            };
            origins.insert(canonical, header);
            allowed_hosts.insert(host);
        }
        Err(reason) if !raw.is_empty() => {
            tracing::warn!(source, reason, "ignored invalid API origin");
        }
        Err(_) => {}
    }
}

fn parse_origin(raw: &str) -> Result<(String, String), &'static str> {
    let url = Url::parse(raw).map_err(|_| "URL parse failed")?;
    if !matches!(url.scheme(), "http" | "https") {
        return Err("only http and https origins are supported");
    }
    if !url.username().is_empty() || url.password().is_some() {
        return Err("credentials are forbidden in origins");
    }
    if url.query().is_some() || url.fragment().is_some() || url.path() != "/" {
        return Err("origins cannot contain a path, query, or fragment");
    }
    let host = url.host_str().ok_or("origin has no host")?;
    let canonical = url.origin().ascii_serialization();
    if canonical == "null" {
        return Err("opaque origins are forbidden");
    }
    Ok((canonical, normalize_host(host)))
}

fn normalize_host(host: &str) -> String {
    host.trim()
        .trim_start_matches('[')
        .trim_end_matches(']')
        .trim_end_matches('.')
        .to_ascii_lowercase()
}

fn request_host(request: &Request) -> Option<String> {
    let host_headers = request.headers().get_all(HOST);
    let mut values = host_headers.iter();
    let first = values.next();
    if values.next().is_some() {
        return None;
    }

    let authority = if let Some(value) = first {
        value
            .to_str()
            .ok()?
            .parse::<axum::http::uri::Authority>()
            .ok()?
    } else {
        request.uri().authority()?.clone()
    };
    if authority.as_str().contains('@') {
        return None;
    }
    Some(normalize_host(authority.host()))
}

pub(crate) async fn validate_host(
    State(policy): State<HostPolicy>,
    request: Request,
    next: Next,
) -> Response {
    let Some(host) = request_host(&request) else {
        return (StatusCode::BAD_REQUEST, "invalid Host header").into_response();
    };
    if !policy.allowed_hosts.contains(&host) {
        return (StatusCode::BAD_REQUEST, "Host is not allowed").into_response();
    }
    next.run(request).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;
    use axum::http::header::{
        ACCESS_CONTROL_ALLOW_HEADERS, ACCESS_CONTROL_ALLOW_METHODS, ACCESS_CONTROL_ALLOW_ORIGIN,
        ACCESS_CONTROL_REQUEST_HEADERS, ACCESS_CONTROL_REQUEST_METHOD, ORIGIN,
    };
    use axum::http::Request;
    use axum::middleware;
    use axum::routing::get;
    use axum::Router;
    use tower::ServiceExt;

    fn policy(api: ApiConfig, deployment: DeploymentConfig) -> RequestOriginPolicy {
        RequestOriginPolicy::from_config(&api, &deployment, "127.0.0.1:50051".parse().unwrap())
    }

    fn app(policy: RequestOriginPolicy) -> Router {
        Router::new()
            .route("/ok", get(|| async { "ok" }))
            .layer(policy.cors_layer())
            .layer(middleware::from_fn_with_state(
                policy.host_policy(),
                validate_host,
            ))
    }

    fn request(origin: Option<&str>, host: Option<&str>) -> Request<Body> {
        let mut builder = Request::builder().uri("/ok");
        if let Some(origin) = origin {
            builder = builder.header(ORIGIN, origin);
        }
        if let Some(host) = host {
            builder = builder.header(HOST, host);
        }
        builder.body(Body::empty()).unwrap()
    }

    #[tokio::test]
    async fn default_policy_rejects_untrusted_cross_origin_access() {
        let response = app(policy(ApiConfig::default(), DeploymentConfig::default()))
            .oneshot(request(
                Some("https://evil.example"),
                Some("127.0.0.1:50051"),
            ))
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        assert!(!response.headers().contains_key(ACCESS_CONTROL_ALLOW_ORIGIN));
    }

    #[tokio::test]
    async fn default_policy_accepts_the_loopback_origin() {
        let response = app(policy(ApiConfig::default(), DeploymentConfig::default()))
            .oneshot(request(
                Some("http://127.0.0.1:50051"),
                Some("127.0.0.1:50051"),
            ))
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(
            response.headers().get(ACCESS_CONTROL_ALLOW_ORIGIN),
            Some(&HeaderValue::from_static("http://127.0.0.1:50051"))
        );
    }

    #[tokio::test]
    async fn configured_origin_is_allowed_for_cors_and_host() {
        let api = ApiConfig {
            allowed_origins: vec!["https://console.example.com".to_string()],
        };
        let response = app(policy(api, DeploymentConfig::default()))
            .oneshot(request(
                Some("https://console.example.com"),
                Some("console.example.com"),
            ))
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(
            response.headers().get(ACCESS_CONTROL_ALLOW_ORIGIN),
            Some(&HeaderValue::from_static("https://console.example.com"))
        );
    }

    #[tokio::test]
    async fn deployment_public_url_allows_the_declared_reverse_proxy_host() {
        let deployment = DeploymentConfig {
            public_url: "https://captain.example.com".to_string(),
            ..DeploymentConfig::default()
        };
        let response = app(policy(ApiConfig::default(), deployment))
            .oneshot(request(None, Some("captain.example.com")))
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn attacker_host_is_rejected_before_routing() {
        let response = app(policy(ApiConfig::default(), DeploymentConfig::default()))
            .oneshot(request(None, Some("attacker.example")))
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn missing_or_ambiguous_host_is_rejected() {
        let missing = app(policy(ApiConfig::default(), DeploymentConfig::default()))
            .oneshot(request(None, None))
            .await
            .unwrap();
        assert_eq!(missing.status(), StatusCode::BAD_REQUEST);

        let ambiguous = Request::builder()
            .uri("/ok")
            .header(HOST, "localhost")
            .header(HOST, "attacker.example")
            .body(Body::empty())
            .unwrap();
        let response = app(policy(ApiConfig::default(), DeploymentConfig::default()))
            .oneshot(ambiguous)
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn cors_preflight_accepts_only_the_reviewed_method_and_headers() {
        let request = Request::builder()
            .method(Method::OPTIONS)
            .uri("/ok")
            .header(HOST, "localhost:50051")
            .header(ORIGIN, "http://localhost:50051")
            .header(ACCESS_CONTROL_REQUEST_METHOD, "POST")
            .header(ACCESS_CONTROL_REQUEST_HEADERS, "content-type,x-filename")
            .body(Body::empty())
            .unwrap();
        let response = app(policy(ApiConfig::default(), DeploymentConfig::default()))
            .oneshot(request)
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(
            response.headers().get(ACCESS_CONTROL_ALLOW_ORIGIN),
            Some(&HeaderValue::from_static("http://localhost:50051"))
        );
    }

    #[tokio::test]
    async fn cors_preflight_does_not_authorize_unreviewed_methods_or_headers() {
        let trace_request = Request::builder()
            .method(Method::OPTIONS)
            .uri("/ok")
            .header(HOST, "localhost:50051")
            .header(ORIGIN, "http://localhost:50051")
            .header(ACCESS_CONTROL_REQUEST_METHOD, "TRACE")
            .header(ACCESS_CONTROL_REQUEST_HEADERS, "content-type")
            .body(Body::empty())
            .unwrap();
        let trace_response = app(policy(ApiConfig::default(), DeploymentConfig::default()))
            .oneshot(trace_request)
            .await
            .unwrap();
        let allowed_methods = trace_response
            .headers()
            .get(ACCESS_CONTROL_ALLOW_METHODS)
            .and_then(|value| value.to_str().ok())
            .unwrap_or_default();
        assert!(!allowed_methods
            .split(',')
            .any(|value| value.trim() == "TRACE"));

        let header_request = Request::builder()
            .method(Method::OPTIONS)
            .uri("/ok")
            .header(HOST, "localhost:50051")
            .header(ORIGIN, "http://localhost:50051")
            .header(ACCESS_CONTROL_REQUEST_METHOD, "POST")
            .header(ACCESS_CONTROL_REQUEST_HEADERS, "x-unreviewed")
            .body(Body::empty())
            .unwrap();
        let header_response = app(policy(ApiConfig::default(), DeploymentConfig::default()))
            .oneshot(header_request)
            .await
            .unwrap();
        let allowed_headers = header_response
            .headers()
            .get(ACCESS_CONTROL_ALLOW_HEADERS)
            .and_then(|value| value.to_str().ok())
            .unwrap_or_default();
        assert!(!allowed_headers
            .split(',')
            .any(|value| value.trim().eq_ignore_ascii_case("x-unreviewed")));
    }

    #[test]
    fn invalid_origins_never_expand_the_allowlists() {
        for origin in [
            "javascript:alert(1)",
            "https://user:secret@example.com",
            "https://example.com/path",
            "https://example.com?query=yes",
        ] {
            assert!(parse_origin(origin).is_err());
        }
    }

    #[test]
    fn host_matching_is_exact_and_case_insensitive() {
        let policy = policy(ApiConfig::default(), DeploymentConfig::default()).host_policy;
        assert!(policy.allowed_hosts.contains("localhost"));
        assert!(!policy.allowed_hosts.contains("localhost.evil.example"));
        assert_eq!(normalize_host("LOCALHOST."), "localhost");
        assert_eq!(normalize_host("[::1]"), "::1");
        assert_eq!(
            normalize_host(&std::net::Ipv6Addr::LOCALHOST.to_string()),
            "::1"
        );
    }
}
