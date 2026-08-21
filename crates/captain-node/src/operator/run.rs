use super::support::{
    current_time_ms, next_retry, node_root, pairing_retry_delay, retryable_link_error, safe_error,
    transport_name, wait_or_stop, NodeEventSink, NodeOperatorEvent, NodeProxyPasswordResolver,
    NODE_STATE_DIR,
};
use crate::{
    NodeLinkError, NodeLocalConfigStore, NodeLocalToolDriver, NodePairingClient, NodePairingStore,
    NodeRailLink, NodeRailSnapshot, NodeRailStore, NodeRuntimeStatus, NodeRuntimeStatusStore,
    NodeShutdown, NodeToolDriver, NodeWorker,
};
use captain_types::config::ExecPolicy;
use captain_wire::{DeviceGrant, NodeTransport};
use std::{path::Path, sync::Arc, time::Duration};

const WORKER_INTERVAL: Duration = Duration::from_millis(100);
const WORKER_SHUTDOWN_GRACE: Duration = Duration::from_secs(3);

pub async fn run_node(
    home: &Path,
    captain_version: &str,
    exec_policy: ExecPolicy,
    secrets: &dyn NodeProxyPasswordResolver,
    events: &dyn NodeEventSink,
    mut shutdown: NodeShutdown,
) -> Result<(), String> {
    let root = node_root(home);
    let config_store = NodeLocalConfigStore::open(&root).map_err(safe_error)?;
    let config = config_store.load().map_err(safe_error)?.ok_or_else(|| {
        "This machine is not configured; pair this Captain Node first".to_string()
    })?;
    let proxy_password = secrets.resolve(&config.network)?;
    let http = config
        .network
        .build_client(proxy_password.as_ref())
        .map_err(safe_error)?;
    let pairing_store =
        NodePairingStore::open(config_store.root().join(NODE_STATE_DIR)).map_err(safe_error)?;
    let rail = NodeRailStore::open(&pairing_store).map_err(safe_error)?;
    let pairing = NodePairingClient::new(http.clone(), pairing_store);
    let capabilities = config.capabilities(captain_version);
    let Some((link, grants, expires_at_ms)) =
        connect_until_available(&pairing, &http, &rail, &capabilities, &mut shutdown).await?
    else {
        return Ok(());
    };
    let policy = config
        .execution_policy(grants.clone())
        .map_err(safe_error)?;
    let driver: Arc<dyn NodeToolDriver> = Arc::new(NodeLocalToolDriver::new(exec_policy));
    let worker = NodeWorker::new(rail.clone(), policy, driver);
    let status_store = NodeRuntimeStatusStore::open(config_store.root()).map_err(safe_error)?;

    events.emit(NodeOperatorEvent::Connected {
        transport: transport_name(link.transport()).to_string(),
        allow_mutation: grants.allow_mutation,
    });
    run_connected(
        ConnectedRuntime {
            pairing: &pairing,
            rail: &rail,
            status_store: &status_store,
            approved_grants: grants,
            worker,
            events,
            shutdown,
        },
        link,
        expires_at_ms,
    )
    .await
}

async fn connect_until_available(
    pairing: &NodePairingClient,
    http: &crate::NodeHttpClient,
    rail: &NodeRailStore,
    capabilities: &captain_wire::CapabilityDescriptor,
    shutdown: &mut NodeShutdown,
) -> Result<Option<(NodeRailLink, DeviceGrant, i64)>, String> {
    let mut retry = Duration::from_secs(1);
    loop {
        let token = tokio::select! {
            _ = shutdown.wait() => return Ok(None),
            result = pairing.issue_access_token() => match result {
                Ok(token) => token,
                Err(error) if pairing_retry_delay(&error).is_some() => {
                    tracing::warn!(error_class = %error, "Node credential exchange will retry");
                    if wait_or_stop(pairing_retry_delay(&error).unwrap_or(retry), shutdown).await {
                        return Ok(None);
                    }
                    retry = next_retry(retry);
                    continue;
                }
                Err(error) => return Err(safe_error(error)),
            }
        };
        let grants = token.approved_grants().clone();
        let expires_at_ms = token.expires_at_ms;
        let active_runs = rail.active_run_ids().map_err(safe_error)?;
        let connected = tokio::select! {
            _ = shutdown.wait() => return Ok(None),
            result = NodeRailLink::connect(
                http.clone(),
                rail.clone(),
                token,
                capabilities,
                &active_runs,
            ) => result,
        };
        match connected {
            Ok(link) => return Ok(Some((link, grants, expires_at_ms))),
            Err(error) if retryable_link_error(&error) => {
                tracing::warn!(error_class = %error, "Node transport connect will retry");
                if wait_or_stop(retry, shutdown).await {
                    return Ok(None);
                }
                retry = next_retry(retry);
            }
            Err(error) => return Err(safe_error(error)),
        }
    }
}

struct ConnectedRuntime<'a> {
    pairing: &'a NodePairingClient,
    rail: &'a NodeRailStore,
    status_store: &'a NodeRuntimeStatusStore,
    approved_grants: DeviceGrant,
    worker: NodeWorker<dyn NodeToolDriver>,
    events: &'a dyn NodeEventSink,
    shutdown: NodeShutdown,
}

async fn run_connected(
    runtime: ConnectedRuntime<'_>,
    mut link: NodeRailLink,
    mut expires_at_ms: i64,
) -> Result<(), String> {
    let ConnectedRuntime {
        pairing,
        rail,
        status_store,
        approved_grants,
        worker,
        events,
        mut shutdown,
    } = runtime;
    let mut refresh_at_ms = token_refresh_at(expires_at_ms, current_time_ms()?);
    persist_runtime_status(
        status_store,
        &link,
        rail,
        approved_grants.allow_mutation,
        None,
    )?;
    let (shutdown_tx, shutdown_rx) = tokio::sync::watch::channel(false);
    let mut worker_task = tokio::spawn(worker_loop(worker, shutdown_rx));
    let mut worker_joined = false;
    let mut terminal_error = None;
    let mut retry = Duration::ZERO;
    let mut next_heartbeat_at = tokio::time::Instant::now() + link.heartbeat_policy().interval();

    loop {
        let refresh_wait = match duration_until(refresh_at_ms) {
            Ok(wait) => wait,
            Err(error) => {
                terminal_error = Some(error);
                break;
            }
        };
        let network_wait = retry;
        let heartbeat_due = tokio::time::Instant::now() >= next_heartbeat_at;
        let may_interrupt_receive =
            heartbeat_may_interrupt_receive(link.transport(), heartbeat_due);
        let event = tokio::select! {
            _ = shutdown.wait() => NodeLoopEvent::Stop,
            joined = &mut worker_task => NodeLoopEvent::Worker(joined),
            _ = tokio::time::sleep(refresh_wait) => NodeLoopEvent::Refresh,
            _ = async {
                if may_interrupt_receive {
                    tokio::time::sleep_until(next_heartbeat_at).await;
                } else {
                    std::future::pending::<()>().await;
                }
            } => NodeLoopEvent::Heartbeat,
            synchronized = async {
                if !network_wait.is_zero() {
                    tokio::time::sleep(network_wait).await;
                }
                link.synchronize_once().await
            } => NodeLoopEvent::Synchronized(synchronized),
        };

        match event {
            NodeLoopEvent::Stop => break,
            NodeLoopEvent::Worker(joined) => {
                worker_joined = true;
                terminal_error = Some(match joined {
                    Ok(Ok(())) => "The local Node worker stopped unexpectedly".to_string(),
                    Ok(Err(error)) => error,
                    Err(_) => "The local Node worker task failed".to_string(),
                });
                break;
            }
            NodeLoopEvent::Refresh => {
                let refreshed = tokio::select! {
                    _ = shutdown.wait() => break,
                    result = pairing.issue_access_token() => result,
                };
                match refreshed {
                    Ok(token) if token.approved_grants() == &approved_grants => {
                        expires_at_ms = token.expires_at_ms;
                        if let Err(error) = link.replace_access_token(token) {
                            terminal_error = Some(safe_error(error));
                            break;
                        }
                        let now_ms = match current_time_ms() {
                            Ok(now_ms) => now_ms,
                            Err(error) => {
                                terminal_error = Some(error);
                                break;
                            }
                        };
                        refresh_at_ms = token_refresh_at(expires_at_ms, now_ms);
                    }
                    Ok(_) => {
                        terminal_error = Some(
                            "Hub grants changed; restart the Node to apply the new local authority"
                                .to_string(),
                        );
                        break;
                    }
                    Err(error) if pairing_retry_delay(&error).is_some() => {
                        tracing::warn!(error_class = %error, "Node access token refresh will retry");
                        match current_time_ms() {
                            Ok(now_ms) => refresh_at_ms = now_ms.saturating_add(5_000),
                            Err(error) => {
                                terminal_error = Some(error);
                                break;
                            }
                        }
                    }
                    Err(error) => {
                        terminal_error = Some(safe_error(error));
                        break;
                    }
                }
            }
            NodeLoopEvent::Heartbeat => {
                let active_runs = match rail.active_run_ids() {
                    Ok(active_runs) => active_runs,
                    Err(error) => {
                        terminal_error = Some(safe_error(error));
                        break;
                    }
                };
                match link.refresh_presence(&active_runs).await {
                    Ok(_) => {
                        retry = Duration::ZERO;
                        next_heartbeat_at =
                            tokio::time::Instant::now() + link.heartbeat_policy().interval();
                        if let Err(error) = persist_runtime_status(
                            status_store,
                            &link,
                            rail,
                            approved_grants.allow_mutation,
                            None,
                        ) {
                            terminal_error = Some(error);
                            break;
                        }
                    }
                    Err(NodeLinkError::InvalidAccessToken) => match current_time_ms() {
                        Ok(now_ms) => refresh_at_ms = now_ms,
                        Err(error) => {
                            terminal_error = Some(error);
                            break;
                        }
                    },
                    Err(error) if retryable_link_error(&error) => {
                        retry = if retry.is_zero() {
                            Duration::from_secs(1)
                        } else {
                            next_retry(retry)
                        };
                        tracing::warn!(
                            error_class = %error,
                            retry_secs = retry.as_secs(),
                            "Node presence heartbeat will retry"
                        );
                        if let Err(error) = persist_runtime_status(
                            status_store,
                            &link,
                            rail,
                            approved_grants.allow_mutation,
                            Some("heartbeat_retry"),
                        ) {
                            terminal_error = Some(error);
                            break;
                        }
                    }
                    Err(error) => {
                        terminal_error = Some(safe_error(error));
                        break;
                    }
                }
            }
            NodeLoopEvent::Synchronized(Ok(_)) => {
                retry = Duration::ZERO;
                let active_runs = match rail.active_run_ids() {
                    Ok(active_runs) => active_runs,
                    Err(error) => {
                        terminal_error = Some(safe_error(error));
                        break;
                    }
                };
                let heartbeat_due = tokio::time::Instant::now() >= next_heartbeat_at;
                let active_runs_changed = !link.active_runs_match(&active_runs);
                let presence = if heartbeat_due {
                    link.refresh_presence(&active_runs).await
                } else {
                    link.set_active_runs(&active_runs).await
                };
                match presence {
                    Ok(_) if heartbeat_due || active_runs_changed => {
                        next_heartbeat_at =
                            tokio::time::Instant::now() + link.heartbeat_policy().interval();
                    }
                    Ok(_) => {}
                    Err(error) if retryable_link_error(&error) => {
                        retry = Duration::from_secs(1);
                        tracing::warn!(error_class = %error, "Node heartbeat update will retry");
                    }
                    Err(error) => {
                        terminal_error = Some(safe_error(error));
                        break;
                    }
                }
                if let Err(error) = persist_runtime_status(
                    status_store,
                    &link,
                    rail,
                    approved_grants.allow_mutation,
                    None,
                ) {
                    terminal_error = Some(error);
                    break;
                }
            }
            NodeLoopEvent::Synchronized(Err(NodeLinkError::InvalidAccessToken)) => {
                match current_time_ms() {
                    Ok(now_ms) => refresh_at_ms = now_ms,
                    Err(error) => {
                        terminal_error = Some(error);
                        break;
                    }
                }
            }
            NodeLoopEvent::Synchronized(Err(error)) if retryable_link_error(&error) => {
                retry = if retry.is_zero() {
                    Duration::from_secs(1)
                } else {
                    next_retry(retry)
                };
                tracing::warn!(error_class = %error, retry_secs = retry.as_secs(), "Node transport will retry");
                if let Err(error) = persist_runtime_status(
                    status_store,
                    &link,
                    rail,
                    approved_grants.allow_mutation,
                    Some("transport_retry"),
                ) {
                    terminal_error = Some(error);
                    break;
                }
            }
            NodeLoopEvent::Synchronized(Err(error)) => {
                terminal_error = Some(safe_error(error));
                break;
            }
        }
    }

    let _ = shutdown_tx.send(true);
    if !worker_joined
        && tokio::time::timeout(WORKER_SHUTDOWN_GRACE, &mut worker_task)
            .await
            .is_err()
    {
        worker_task.abort();
        let _ = worker_task.await;
    }
    if let Ok(now_ms) = current_time_ms() {
        let _ = status_store.save(&NodeRuntimeStatus::stopped(now_ms));
    }
    if let Err(error) = link.close(None).await {
        tracing::warn!(error_class = %error, "Node close evidence remains durable for reconnect");
    }
    if let Some(error) = terminal_error {
        Err(error)
    } else {
        events.emit(NodeOperatorEvent::Stopped);
        Ok(())
    }
}

enum NodeLoopEvent {
    Stop,
    Worker(Result<Result<(), String>, tokio::task::JoinError>),
    Refresh,
    Heartbeat,
    Synchronized(Result<NodeRailSnapshot, NodeLinkError>),
}

fn heartbeat_may_interrupt_receive(transport: NodeTransport, heartbeat_due: bool) -> bool {
    transport != NodeTransport::LongPoll || heartbeat_due
}

async fn worker_loop(
    mut worker: NodeWorker<dyn NodeToolDriver>,
    mut shutdown: tokio::sync::watch::Receiver<bool>,
) -> Result<(), String> {
    let mut interval = tokio::time::interval(WORKER_INTERVAL);
    interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    loop {
        tokio::select! {
            changed = shutdown.changed() => {
                if changed.is_err() || *shutdown.borrow() {
                    return Ok(());
                }
            }
            _ = interval.tick() => {
                worker.advance(current_time_ms()?).await.map_err(safe_error)?;
            }
        }
    }
}

fn persist_runtime_status(
    store: &NodeRuntimeStatusStore,
    link: &NodeRailLink,
    rail: &NodeRailStore,
    allow_mutation: bool,
    last_error_code: Option<&str>,
) -> Result<(), String> {
    let status = NodeRuntimeStatus::connected(
        current_time_ms()?,
        link.transport(),
        link.capability_state(),
        allow_mutation,
        rail.snapshot().map_err(safe_error)?,
        link.fallbacks().len(),
        last_error_code,
    )
    .map_err(safe_error)?;
    store.save(&status).map_err(safe_error)
}

fn token_refresh_at(expires_at_ms: i64, now_ms: i64) -> i64 {
    let ttl = expires_at_ms.saturating_sub(now_ms).max(1);
    let margin = (ttl / 5).clamp(5_000, 60_000).min((ttl / 2).max(1));
    expires_at_ms.saturating_sub(margin)
}

fn duration_until(deadline_ms: i64) -> Result<Duration, String> {
    let remaining = deadline_ms.saturating_sub(current_time_ms()?).max(0);
    Ok(Duration::from_millis(remaining as u64))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn token_refresh_is_early_but_bounded_for_short_tokens() {
        assert_eq!(token_refresh_at(20_000, 0), 15_000);
        assert_eq!(token_refresh_at(300_000, 0), 240_000);
    }

    #[test]
    fn long_poll_is_not_interrupted_before_presence_is_due() {
        assert!(!heartbeat_may_interrupt_receive(
            NodeTransport::LongPoll,
            false
        ));
        assert!(heartbeat_may_interrupt_receive(
            NodeTransport::LongPoll,
            true
        ));
        assert!(heartbeat_may_interrupt_receive(
            NodeTransport::WebSocket,
            false
        ));
        assert!(heartbeat_may_interrupt_receive(
            NodeTransport::HttpStream,
            false
        ));
    }
}
