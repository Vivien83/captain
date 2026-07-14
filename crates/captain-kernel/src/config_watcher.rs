//! HR.1 — Universal config hot-reload watcher (#168).
//!
//! Watches `~/.captain/config.toml` and, on debounced modifications,
//! emits a `SystemEvent::IntegrationConfigured { name: "config" }`
//! on the kernel event bus. The existing reload pipeline in
//! `captain-api/src/server.rs` already listens for that event,
//! re-runs `kernel.reload_config()` to apply hot-reloadable sections
//! (cf. `config_reload::ReloadPlan` / `should_apply_hot`), and
//! re-spawns the channel bridge with the fresh config snapshot.
//!
//! Before this module: the only path that triggered a reload was the
//! `tool_channel_reconfigure` tool (commit A.1) — the user had to
//! ask Captain to "reload telegram" by hand. Now any `vim`/`nano`
//! save on the file fires the same path automatically.
//!
//! ## Reload mode honoring
//!
//! `[reload] mode` in the config controls whether the watcher emits
//! at all:
//!   - `off`    — watcher never fires (loaded but a no-op).
//!   - `restart` — emits the event so the API layer can log a hint
//!     ("config changed — restart daemon to apply"), but the
//!     downstream `should_apply_hot` decides nothing is hot.
//!   - `hot` / `hybrid` (default) — full pipeline runs.
//!
//! ## Debounce
//!
//! Editors save in 2-3 quick writes (rename + truncate + write).
//! `notify-debouncer-mini` collapses those bursts into one event
//! using `[reload] debounce_ms` (default 500 ms).

use crate::event_bus::EventBus;
use captain_types::agent::AgentId;
use captain_types::config::ReloadMode;
use captain_types::event::{Event, EventPayload, EventTarget, SystemEvent};
use notify_debouncer_mini::{
    new_debouncer_opt,
    notify::{Config as NotifyConfig, RecommendedWatcher, RecursiveMode, Watcher},
    Config as DebouncerConfig, DebounceEventResult, DebouncedEvent, DebouncedEventKind,
};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;
use tracing::{debug, info, warn};

fn watch_target_for(config_path: &Path) -> PathBuf {
    config_path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .map(|parent| parent.to_path_buf())
        .unwrap_or_else(|| config_path.to_path_buf())
}

fn event_targets_config(path: &Path, config_path: &Path) -> bool {
    let parent_matches = config_path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .map(|parent| path == parent)
        .unwrap_or(false);

    path == config_path
        || parent_matches
        || (path.parent() == config_path.parent() && path.file_name() == config_path.file_name())
}

fn contains_config_change(events: &[DebouncedEvent], config_path: &Path) -> bool {
    events.iter().any(|event| {
        matches!(
            event.kind,
            DebouncedEventKind::Any | DebouncedEventKind::AnyContinuous
        ) && event_targets_config(&event.path, config_path)
    })
}

/// Spawn a background task that watches `config_path` and emits
/// `IntegrationConfigured { name: "config" }` on every debounced
/// modification. Returns `Ok(())` once the watcher is armed; the
/// task itself runs until the kernel shuts down (the debouncer is
/// Dropped when the returned guard is dropped — we leak it via
/// `std::mem::forget` because the kernel's lifetime ≥ daemon's).
///
/// Returns `Ok(())` even when `mode == Off` — caller doesn't have
/// to branch on the config; the no-op path stays silent.
pub fn spawn_config_watcher(
    config_path: PathBuf,
    event_bus: Arc<EventBus>,
    mode: ReloadMode,
    debounce_ms: u64,
) -> Result<(), String> {
    spawn_config_watcher_with_backend::<RecommendedWatcher>(
        config_path,
        event_bus,
        mode,
        debounce_ms,
        NotifyConfig::default(),
    )
}

fn spawn_config_watcher_with_backend<T>(
    config_path: PathBuf,
    event_bus: Arc<EventBus>,
    mode: ReloadMode,
    debounce_ms: u64,
    notify_config: NotifyConfig,
) -> Result<(), String>
where
    T: Watcher + 'static,
{
    if mode == ReloadMode::Off {
        info!(path = %config_path.display(), "config watcher disabled (reload.mode = off)");
        return Ok(());
    }

    if !config_path.exists() {
        warn!(
            path = %config_path.display(),
            "config watcher: file does not exist yet — skipping"
        );
        return Ok(());
    }

    let path_for_log = config_path.display().to_string();
    let watched_config_path = config_path
        .canonicalize()
        .unwrap_or_else(|_| config_path.clone());
    let watch_target = watch_target_for(&watched_config_path);
    let watched_config_for_handler = watched_config_path.clone();
    let event_bus_for_handler = Arc::clone(&event_bus);
    let runtime = tokio::runtime::Handle::try_current().map_err(|e| {
        format!("config_watcher needs an active tokio runtime to spawn its publishes: {e}")
    })?;

    let debouncer_config = DebouncerConfig::default()
        .with_timeout(Duration::from_millis(debounce_ms.max(50)))
        .with_notify_config(notify_config);
    let mut debouncer = new_debouncer_opt::<_, T>(
        debouncer_config,
        move |res: DebounceEventResult| match res {
            Ok(events) => {
                let any_modify = contains_config_change(&events, &watched_config_for_handler);
                if !any_modify {
                    return;
                }
                debug!(
                    events = events.len(),
                    "config watcher: debounced modification"
                );
                let bus = Arc::clone(&event_bus_for_handler);
                runtime.spawn(async move {
                    let event = Event::new(
                        AgentId::default(),
                        EventTarget::Broadcast,
                        EventPayload::System(SystemEvent::IntegrationConfigured {
                            name: "config".to_string(),
                        }),
                    );
                    bus.publish(event).await;
                });
            }
            Err(e) => {
                warn!(error = ?e, "config watcher: error from debouncer");
            }
        },
    )
    .map_err(|e| format!("notify_debouncer init failed: {e}"))?;

    debouncer
        .watcher()
        .watch(&watched_config_path, RecursiveMode::NonRecursive)
        .map_err(|e| format!("notify_debouncer watch failed: {e}"))?;
    if watch_target != watched_config_path {
        debouncer
            .watcher()
            .watch(&watch_target, RecursiveMode::NonRecursive)
            .map_err(|e| format!("notify_debouncer parent watch failed: {e}"))?;
    }

    info!(
        path = %path_for_log,
        watch_target = %watch_target.display(),
        debounce_ms = debounce_ms,
        mode = ?mode,
        "config watcher armed"
    );

    // Keep the debouncer alive for the lifetime of the daemon. The
    // kernel never gracefully drops itself before exit; if it ever
    // does we'd revisit and store the handle on the kernel struct.
    std::mem::forget(debouncer);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use notify_debouncer_mini::notify::PollWatcher;

    #[test]
    fn config_change_matches_target_and_continuous_events() {
        let config = PathBuf::from("/tmp/captain/config.toml");
        let events = vec![DebouncedEvent {
            path: config.clone(),
            kind: DebouncedEventKind::AnyContinuous,
        }];
        assert!(contains_config_change(&events, &config));

        let parent = vec![DebouncedEvent {
            path: config.parent().unwrap().to_path_buf(),
            kind: DebouncedEventKind::Any,
        }];
        assert!(contains_config_change(&parent, &config));

        let other = vec![DebouncedEvent {
            path: PathBuf::from("/tmp/captain/other.toml"),
            kind: DebouncedEventKind::Any,
        }];
        assert!(!contains_config_change(&other, &config));
    }

    #[test]
    fn watcher_skips_when_mode_is_off() {
        let bus = Arc::new(EventBus::new());
        // Pass a non-existent path: even so, mode=Off short-circuits
        // before the file existence check would fire a warn.
        let res = spawn_config_watcher(
            PathBuf::from("/tmp/captain_nope_does_not_exist.toml"),
            bus,
            ReloadMode::Off,
            500,
        );
        assert!(res.is_ok(), "Off mode must succeed without armed watcher");
    }

    #[test]
    fn watcher_returns_ok_when_file_missing() {
        // Defensive: a fresh install without a config.toml shouldn't
        // crash the daemon boot. Watcher reports the missing file
        // and returns Ok so the rest of the boot pipeline continues.
        let bus = Arc::new(EventBus::new());
        let res = spawn_config_watcher(
            PathBuf::from("/tmp/captain_definitely_missing_xyz.toml"),
            bus,
            ReloadMode::Hot,
            500,
        );
        assert!(res.is_ok());
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn watcher_publishes_on_modification() {
        // Integration test: write a config, watch it, modify it,
        // expect an event on the bus within a small window.
        let tmp_root = std::env::temp_dir()
            .canonicalize()
            .unwrap_or_else(|_| std::env::temp_dir());
        let tmp_dir = tmp_root.join(format!(
            "captain_watcher_test_{}_{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&tmp_dir).unwrap();
        let tmp = tmp_dir.join("config.toml");
        std::fs::write(&tmp, "v = 1\n").unwrap();

        let bus = Arc::new(EventBus::new());
        let mut rx = bus.subscribe_all();

        spawn_config_watcher_with_backend::<PollWatcher>(
            tmp.clone(),
            Arc::clone(&bus),
            ReloadMode::Hybrid,
            80,
            NotifyConfig::default().with_poll_interval(Duration::from_millis(50)),
        )
        .unwrap();

        tokio::time::sleep(Duration::from_millis(150)).await;
        let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
        let mut version = 2;
        let event = loop {
            std::fs::write(&tmp, format!("v = {version}\n")).unwrap();
            match tokio::time::timeout(Duration::from_millis(350), rx.recv()).await {
                Ok(Ok(event)) => break event,
                Ok(Err(err)) => panic!("event channel closed before watcher fired: {err}"),
                Err(_) if tokio::time::Instant::now() >= deadline => {
                    panic!("watcher should have fired within 5s")
                }
                Err(_) => {
                    version += 1;
                    tokio::time::sleep(Duration::from_millis(50)).await;
                }
            }
        };
        match event.payload {
            EventPayload::System(SystemEvent::IntegrationConfigured { name }) => {
                assert_eq!(name, "config");
            }
            other => panic!("expected IntegrationConfigured, got {other:?}"),
        }
        let _ = std::fs::remove_dir_all(&tmp_dir);
    }
}
