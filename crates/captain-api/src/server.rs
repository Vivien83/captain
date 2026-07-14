//! Captain daemon server — boots the kernel and serves the HTTP API.

use crate::channel_bridge;
use crate::middleware;
use crate::rate_limiter;
use crate::routes::AppState;
use axum::http::HeaderValue;
use axum::Router;
use captain_channels::bridge::BridgeManager;
use captain_kernel::CaptainKernel;
use std::future::{Future, IntoFuture};
use std::net::SocketAddr;
use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tower_http::compression::CompressionLayer;
use tower_http::cors::CorsLayer;
use tower_http::trace::TraceLayer;
use tracing::{info, warn};

const CHANNEL_BRIDGE_SHUTDOWN_TIMEOUT: Duration = Duration::from_secs(15);
const HTTP_CONNECTION_DRAIN_TIMEOUT: Duration = Duration::from_secs(15);

pub use crate::daemon_info::{read_daemon_info, DaemonInfo};

/// Build the full API router with all routes, middleware, and state.
///
/// This is extracted from `run_daemon()` so that embedders (e.g. captain-desktop)
/// can create the router without starting the full daemon lifecycle.
///
/// Returns `(router, shared_state)`. The caller can use `state.bridge_manager`
/// to shut down the bridge on exit.
pub async fn build_router(
    kernel: Arc<CaptainKernel>,
    listen_addr: SocketAddr,
) -> (Router<()>, Arc<AppState>) {
    let state = build_app_state(kernel).await;
    spawn_router_background_tasks(&state, listen_addr);

    let app = apply_api_layers(
        mount_api_routes(),
        state.clone(),
        build_auth_state(&state),
        rate_limiter::create_rate_limiter(),
        build_cors_layer(&state.kernel.config.api_key, listen_addr),
    );
    (app, state)
}

async fn build_app_state(kernel: Arc<CaptainKernel>) -> Arc<AppState> {
    let bridge = channel_bridge::start_channel_bridge(kernel.clone()).await;
    app_state_from_bridge(kernel, bridge)
}

fn app_state_from_bridge(
    kernel: Arc<CaptainKernel>,
    bridge: Option<BridgeManager>,
) -> Arc<AppState> {
    let channels_config = kernel.config.channels.clone();
    Arc::new(AppState {
        kernel: kernel.clone(),
        started_at: Instant::now(),
        peer_registry: kernel.peer_registry.get().map(|r| Arc::new(r.clone())),
        bridge_manager: tokio::sync::Mutex::new(bridge),
        channels_config: tokio::sync::RwLock::new(channels_config),
        shutdown_notify: Arc::new(tokio::sync::Notify::new()),
        clawhub_cache: dashmap::DashMap::new(),
        ask_user_channels: dashmap::DashMap::new(),
        provider_probe_cache: captain_runtime::provider_health::ProbeCache::new(),
    })
}

fn spawn_router_background_tasks(state: &Arc<AppState>, listen_addr: SocketAddr) {
    crate::server_runtime_adapters::spawn_integration_hot_reload(state.clone());
    crate::event_webhooks::spawn_outbound_webhook_dispatcher(state.clone());
    crate::agent_api_egress_queue::spawn_agent_api_egress_queue_drain(
        state.kernel.config.home_dir.clone(),
        state.kernel.audit_log.clone(),
    );
    spawn_config_watcher(&state.kernel);
    spawn_goal_loops(state);
    spawn_goal_reflection(state);
    crate::server_runtime_adapters::spawn_peer_discovery(state.kernel.clone(), listen_addr.port());
}

fn spawn_config_watcher(kernel: &Arc<CaptainKernel>) {
    let config_path = kernel.config.home_dir.join("config.toml");
    let mode = kernel.config.reload.mode;
    let debounce_ms = kernel.config.reload.debounce_ms;
    let bus = std::sync::Arc::new(kernel.event_bus.clone());
    if let Err(e) =
        captain_kernel::config_watcher::spawn_config_watcher(config_path, bus, mode, debounce_ms)
    {
        tracing::warn!(error = %e, "config watcher failed to arm — manual reloads still work");
    }
}

fn spawn_goal_loops(state: &Arc<AppState>) {
    let kh: std::sync::Arc<dyn captain_runtime::kernel_handle::KernelHandle> = state.kernel.clone();
    let ops: std::sync::Arc<dyn captain_runtime::goal_loop::GoalLoopOps> =
        std::sync::Arc::new(captain_runtime::goal_loop::KernelOps { kh });
    captain_runtime::goal_loop::spawn_goal_loops(ops);
}

fn spawn_goal_reflection(state: &Arc<AppState>) {
    let kh: std::sync::Arc<dyn captain_runtime::kernel_handle::KernelHandle> = state.kernel.clone();
    let ref_ops: std::sync::Arc<dyn captain_runtime::goal_reflection::GoalReflectionOps> =
        std::sync::Arc::new(captain_runtime::goal_reflection::KernelReflectionOps { kh });
    let completer = state.kernel.build_reflection_completer();
    let model = state.kernel.resolve_learning_reflection_model();
    captain_runtime::goal_reflection::spawn_reflection_cron(
        ref_ops,
        completer,
        model,
        captain_runtime::goal_reflection::REFLECTION_INTERVAL_SECS,
    );
}

fn build_cors_layer(api_key: &str, listen_addr: SocketAddr) -> CorsLayer {
    if api_key.trim().is_empty() {
        return CorsLayer::new()
            .allow_origin(tower_http::cors::Any)
            .allow_methods(tower_http::cors::Any)
            .allow_headers(tower_http::cors::Any);
    }
    CorsLayer::new()
        .allow_origin(restricted_cors_origins(listen_addr))
        .allow_methods(tower_http::cors::Any)
        .allow_headers(tower_http::cors::Any)
}

fn restricted_cors_origins(listen_addr: SocketAddr) -> Vec<HeaderValue> {
    let mut origins: Vec<HeaderValue> = vec![
        format!("http://{listen_addr}").parse().unwrap(),
        "http://localhost:4200".parse().unwrap(),
        "http://127.0.0.1:4200".parse().unwrap(),
        "http://localhost:8080".parse().unwrap(),
        "http://127.0.0.1:8080".parse().unwrap(),
    ];
    if listen_addr.port() != 4200 && listen_addr.port() != 8080 {
        if let Ok(v) = format!("http://localhost:{}", listen_addr.port()).parse() {
            origins.push(v);
        }
        if let Ok(v) = format!("http://127.0.0.1:{}", listen_addr.port()).parse() {
            origins.push(v);
        }
    }
    origins
}

fn build_auth_state(state: &Arc<AppState>) -> middleware::AuthState {
    middleware::AuthState {
        api_key: state.kernel.config.api_key.trim().to_string(),
        home_dir: state.kernel.config.home_dir.clone(),
        fallback_auth: state.kernel.config.auth.clone(),
    }
}

fn mount_api_routes() -> Router<Arc<AppState>> {
    let app = crate::server_web_routes::mount_web_routes(Router::new());
    let app = crate::server_observability_routes::mount_observability_routes(app);
    let app = crate::server_agent_routes::mount_agent_routes(app);
    let app = crate::server_session_io_routes::mount_session_io_routes(app);
    let app = crate::server_channel_routes::mount_channel_routes(app);
    let app = crate::server_learning_routes::mount_learning_routes(app);
    let app = crate::server_automation_routes::mount_automation_routes(app);
    let app = crate::server_skill_routes::mount_skill_routes(app);
    let app = crate::server_hand_routes::mount_hand_routes(app);
    let app = crate::server_coordination_routes::mount_coordination_routes(app);
    mount_api_routes_tail(app)
}

fn mount_api_routes_tail(app: Router<Arc<AppState>>) -> Router<Arc<AppState>> {
    let app = crate::server_settings_routes::mount_settings_routes(app);
    let app = crate::server_governance_routes::mount_governance_routes(app);
    let app = crate::server_project_routes::mount_project_routes(app);
    let app = crate::server_learning_engine_routes::mount_learning_engine_routes(app);
    let app = crate::server_session_management_routes::mount_session_management_routes(app);
    let app = crate::server_capability_routes::mount_capability_routes(app);
    let app = crate::server_maintenance_routes::mount_maintenance_routes(app);
    let app = crate::server_memory_graph_routes::mount_memory_graph_routes(app);
    let app = crate::server_control_routes::mount_control_routes(app);
    let app = crate::server_a2a_routes::mount_a2a_routes(app);
    let app = crate::server_integration_routes::mount_integration_routes(app);
    crate::server_protocol_routes::mount_protocol_routes(app)
}

fn apply_api_layers(
    app: Router<Arc<AppState>>,
    state: Arc<AppState>,
    auth_state: middleware::AuthState,
    gcra_limiter: Arc<rate_limiter::KeyedRateLimiter>,
    cors: CorsLayer,
) -> Router<()> {
    app.layer(axum::middleware::from_fn_with_state(
        auth_state,
        middleware::auth,
    ))
    .layer(axum::middleware::from_fn_with_state(
        gcra_limiter,
        rate_limiter::gcra_rate_limit,
    ))
    .layer(axum::middleware::from_fn(middleware::security_headers))
    .layer(axum::middleware::from_fn(middleware::request_logging))
    .layer(CompressionLayer::new())
    .layer(TraceLayer::new_for_http())
    .layer(cors)
    .with_state(state)
}

/// Start the Captain daemon: boot kernel + HTTP API server.
///
/// This function blocks until Ctrl+C or a shutdown request.
pub async fn run_daemon(
    kernel: CaptainKernel,
    listen_addr: &str,
    daemon_info_path: Option<&Path>,
) -> Result<(), Box<dyn std::error::Error>> {
    let addr: SocketAddr = listen_addr.parse()?;

    // B.5 — Block startup if a non-loopback bind has no api_key configured.
    // The daemon would otherwise expose every tool to the local network.
    validate_bind_auth_policy(addr, &kernel.config.api_key)?;

    let kernel = Arc::new(kernel);
    kernel.set_self_handle();
    kernel.start_background_agents();

    spawn_autoscale_tick(kernel.clone());
    spawn_config_reload_poll(kernel.clone());

    let (app, state) = build_router(kernel.clone(), addr).await;

    if let Some(info_path) = daemon_info_path {
        crate::daemon_info::write_daemon_info_file(info_path, addr)?;
    }

    log_server_urls(addr);
    let listener = bind_reusable_listener(addr)?;
    serve_api(listener, app, state.clone()).await?;
    shutdown_daemon_state(daemon_info_path, &state, &kernel).await;

    info!("Captain daemon stopped");
    Ok(())
}

fn spawn_autoscale_tick(kernel: Arc<CaptainKernel>) {
    let tick_secs = autoscale_tick_secs();
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(std::time::Duration::from_secs(tick_secs));
        interval.tick().await; // skip first immediate tick
        loop {
            interval.tick().await;
            kernel.autoscale_tick().await;
        }
    });
}

fn autoscale_tick_secs() -> u64 {
    autoscale_tick_secs_from_env(std::env::var("CAPTAIN_AUTOSCALE_TICK_SECS").ok().as_deref())
}

fn autoscale_tick_secs_from_env(value: Option<&str>) -> u64 {
    value.and_then(|s| s.parse::<u64>().ok()).unwrap_or(30)
}

fn spawn_config_reload_poll(kernel: Arc<CaptainKernel>) {
    let config_path = kernel.config.home_dir.join("config.toml");
    tokio::spawn(async move {
        let mut last_modified = std::fs::metadata(&config_path)
            .and_then(|m| m.modified())
            .ok();
        loop {
            tokio::time::sleep(std::time::Duration::from_secs(30)).await;
            let current = std::fs::metadata(&config_path)
                .and_then(|m| m.modified())
                .ok();
            if current != last_modified && current.is_some() {
                last_modified = current;
                tracing::info!("Config file changed, reloading...");
                log_config_reload_result(kernel.reload_config());
            }
        }
    });
}

fn log_config_reload_result(plan: Result<captain_kernel::config_reload::ReloadPlan, String>) {
    match plan {
        Ok(plan) => {
            if plan.has_changes() {
                tracing::info!("Config hot-reload applied: {:?}", plan.hot_actions);
            } else {
                tracing::debug!("Config hot-reload: no actionable changes");
            }
        }
        Err(e) => tracing::warn!("Config hot-reload failed: {e}"),
    }
}

fn log_server_urls(addr: SocketAddr) {
    info!("Captain API server listening on http://{addr}");
    info!("Web terminal available at http://{addr}/terminal");
    info!("WebSocket endpoint: ws://{addr}/api/agents/{{id}}/ws");
}

fn bind_reusable_listener(
    addr: SocketAddr,
) -> Result<tokio::net::TcpListener, Box<dyn std::error::Error>> {
    let socket = socket2::Socket::new(socket_domain_for_addr(addr), socket2::Type::STREAM, None)?;
    socket.set_reuse_address(true)?;
    socket.set_nonblocking(true)?;
    socket.bind(&addr.into())?;
    socket.listen(1024)?;
    Ok(tokio::net::TcpListener::from_std(
        std::net::TcpListener::from(socket),
    )?)
}

fn socket_domain_for_addr(addr: SocketAddr) -> socket2::Domain {
    if addr.is_ipv4() {
        socket2::Domain::IPV4
    } else {
        socket2::Domain::IPV6
    }
}

async fn serve_api(
    listener: tokio::net::TcpListener,
    app: Router<()>,
    state: Arc<AppState>,
) -> Result<(), Box<dyn std::error::Error>> {
    let api_shutdown = state.shutdown_notify.clone();
    let shutdown_kernel = state.kernel.clone();
    let shutdown_drain = crate::shutdown_guard::shutdown_drain_state();
    let http_drain_started = Arc::new(tokio::sync::Notify::new());
    let http_drain_signal = Arc::clone(&http_drain_started);
    let shutdown_signal = async move {
        crate::shutdown_guard::shutdown_signal(api_shutdown, shutdown_kernel, shutdown_drain).await;
        http_drain_signal.notify_one();
    };
    let server = axum::serve(
        listener,
        app.into_make_service_with_connect_info::<SocketAddr>(),
    )
    .with_graceful_shutdown(shutdown_signal)
    .into_future();
    tokio::pin!(server);

    tokio::select! {
        result = &mut server => result?,
        _ = wait_for_shutdown_drain_deadline(
            http_drain_started,
            HTTP_CONNECTION_DRAIN_TIMEOUT,
        ) => {
            warn!(
                timeout_secs = HTTP_CONNECTION_DRAIN_TIMEOUT.as_secs_f64(),
                "HTTP connection drain timed out; forcing daemon shutdown"
            );
        }
    }
    Ok(())
}

async fn wait_for_shutdown_drain_deadline(
    drain_started: Arc<tokio::sync::Notify>,
    timeout: Duration,
) {
    drain_started.notified().await;
    tokio::time::sleep(timeout).await;
}

async fn shutdown_daemon_state(
    daemon_info_path: Option<&Path>,
    state: &Arc<AppState>,
    kernel: &Arc<CaptainKernel>,
) {
    if let Some(info_path) = daemon_info_path {
        crate::daemon_info::remove_daemon_info_file(info_path);
    }

    run_shutdown_phase_with_timeout("channel_bridge", CHANNEL_BRIDGE_SHUTDOWN_TIMEOUT, async {
        // Move the manager out before awaiting adapter shutdown. This keeps
        // the shared mutex available and makes timeout cancellation drop
        // the remaining adapters instead of leaving them globally owned.
        let bridge = state.bridge_manager.lock().await.take();
        if let Some(mut bridge) = bridge {
            bridge.stop().await;
        }
    })
    .await;

    kernel.shutdown();
}

async fn run_shutdown_phase_with_timeout<F>(
    phase: &'static str,
    timeout: Duration,
    future: F,
) -> bool
where
    F: Future<Output = ()>,
{
    match tokio::time::timeout(timeout, future).await {
        Ok(()) => true,
        Err(_) => {
            warn!(
                phase,
                timeout_secs = timeout.as_secs_f64(),
                "Daemon shutdown phase timed out; abandoning residual tasks"
            );
            false
        }
    }
}

/// B.5 — Refuse to start the daemon if it is binding a non-loopback
/// address (0.0.0.0, a public IP, …) without an `api_key` configured.
///
/// Captain's HTTP API exposes file_read, shell_exec, channel_send and
/// every other capability. On loopback that's fine — only processes on
/// the same machine can talk to it. As soon as the bind escapes
/// loopback the API is reachable from other machines, and without an
/// `api_key` the auth middleware lets every request through. A typo in
/// listen_addr ("0.0.0.0:4200" instead of "127.0.0.1:4200") would
/// otherwise expose the entire agent surface to the local network.
///
/// This function is pure so tests can pin the exact policy: loopback
/// is permissive, every other address requires an api_key. Whitespace
/// keys (`"   "`) are treated as empty.
pub(crate) fn validate_bind_auth_policy(
    listen: std::net::SocketAddr,
    api_key: &str,
) -> Result<(), String> {
    if listen.ip().is_loopback() {
        return Ok(());
    }
    if !api_key.trim().is_empty() {
        return Ok(());
    }
    Err(format!(
        "Refusing to start: API would bind {listen} (non-loopback) without an api_key. \
         Set CAPTAIN_DAEMON_API_KEY or CAPTAIN_API_KEY in secrets.env/environment, \
         or bind to 127.0.0.1 / [::1] for local-only access."
    ))
}

#[cfg(test)]
mod bind_auth_policy_tests {
    use super::{
        autoscale_tick_secs_from_env, restricted_cors_origins, run_shutdown_phase_with_timeout,
        validate_bind_auth_policy, wait_for_shutdown_drain_deadline,
    };

    fn sock(s: &str) -> std::net::SocketAddr {
        s.parse().unwrap()
    }

    fn origin_strings(listen: std::net::SocketAddr) -> Vec<String> {
        restricted_cors_origins(listen)
            .into_iter()
            .map(|value| value.to_str().unwrap().to_string())
            .collect()
    }

    #[test]
    fn loopback_v4_without_key_is_allowed() {
        assert!(validate_bind_auth_policy(sock("127.0.0.1:4200"), "").is_ok());
    }

    #[test]
    fn loopback_v6_without_key_is_allowed() {
        assert!(validate_bind_auth_policy(sock("[::1]:4200"), "").is_ok());
    }

    #[test]
    fn whitespace_only_key_treated_as_empty() {
        let res = validate_bind_auth_policy(sock("0.0.0.0:4200"), "   ");
        assert!(res.is_err(), "whitespace key must not unlock public bind");
        assert!(res.unwrap_err().contains("api_key"));
    }

    #[test]
    fn non_loopback_without_key_is_rejected() {
        let res = validate_bind_auth_policy(sock("0.0.0.0:4200"), "");
        assert!(res.is_err(), "0.0.0.0 with empty key must fail");
        let msg = res.unwrap_err();
        assert!(msg.contains("0.0.0.0"));
        assert!(msg.contains("non-loopback") || msg.contains("loopback"));
    }

    #[test]
    fn non_loopback_with_key_is_allowed() {
        assert!(validate_bind_auth_policy(sock("0.0.0.0:4200"), "real-key").is_ok());
    }

    #[test]
    fn public_ip_without_key_is_rejected() {
        let res = validate_bind_auth_policy(sock("203.0.113.7:4200"), "");
        assert!(res.is_err(), "public IP with empty key must fail");
    }

    #[test]
    fn restricted_cors_includes_listen_and_dev_origins() {
        let origins = origin_strings(sock("0.0.0.0:50051"));
        assert!(origins.contains(&"http://0.0.0.0:50051".to_string()));
        assert!(origins.contains(&"http://localhost:50051".to_string()));
        assert!(origins.contains(&"http://127.0.0.1:50051".to_string()));
        assert!(origins.contains(&"http://localhost:4200".to_string()));
        assert!(origins.contains(&"http://127.0.0.1:8080".to_string()));
    }

    #[test]
    fn restricted_cors_keeps_standard_dev_ports_without_extra_variants() {
        let origins = origin_strings(sock("0.0.0.0:4200"));
        assert!(origins.contains(&"http://0.0.0.0:4200".to_string()));
        assert!(origins.contains(&"http://localhost:4200".to_string()));
        assert!(!origins.contains(&"http://localhost:50051".to_string()));
    }

    #[test]
    fn autoscale_tick_secs_from_env_defaults_on_missing_or_invalid_value() {
        assert_eq!(autoscale_tick_secs_from_env(None), 30);
        assert_eq!(autoscale_tick_secs_from_env(Some("bad")), 30);
        assert_eq!(autoscale_tick_secs_from_env(Some("5")), 5);
    }

    #[tokio::test]
    async fn daemon_shutdown_phase_completes_ready_work() {
        assert!(
            run_shutdown_phase_with_timeout("test", std::time::Duration::from_millis(50), async {})
                .await
        );
    }

    #[tokio::test]
    async fn daemon_shutdown_phase_bounds_stuck_work() {
        assert!(
            !run_shutdown_phase_with_timeout(
                "test",
                std::time::Duration::from_millis(10),
                std::future::pending(),
            )
            .await
        );
    }

    #[tokio::test]
    async fn daemon_shutdown_http_deadline_waits_for_signal() {
        let drain_started = std::sync::Arc::new(tokio::sync::Notify::new());
        assert!(
            tokio::time::timeout(
                std::time::Duration::from_millis(5),
                wait_for_shutdown_drain_deadline(
                    std::sync::Arc::clone(&drain_started),
                    std::time::Duration::from_millis(1),
                ),
            )
            .await
            .is_err(),
            "the forced-drain deadline must not run before shutdown starts"
        );

        drain_started.notify_one();
        tokio::time::timeout(
            std::time::Duration::from_millis(50),
            wait_for_shutdown_drain_deadline(drain_started, std::time::Duration::from_millis(1)),
        )
        .await
        .expect("deadline completes after the shutdown signal");
    }
}
