//! Captain Desktop — lightweight paired Client for multiple complete Hubs.
//!
//! This process contains no agent loop, provider, memory database or local
//! execution engine. It renders the Hub work surface through an ephemeral
//! loopback gateway that keeps the paired bearer out of JavaScript.

mod commands;
mod shortcuts;
mod tray;

use captain_console::{
    ConsoleManager, ConsoleManagerError, ConsoleProfileError, ConsoleProfileSummary, GatewayHandle,
};
use std::{
    collections::HashSet,
    sync::{Arc, Mutex, RwLock},
    time::Instant,
};
use tauri::{Manager, Url, WebviewUrl, WebviewWindowBuilder};
use tracing::{info, warn};

pub struct DesktopState {
    pub started_at: Instant,
    pub(crate) manager: Mutex<ConsoleManager>,
    authority: RwLock<DesktopAuthority>,
    allowed_ports: Arc<RwLock<HashSet<u16>>>,
    setup_gateway: Option<GatewayHandle>,
}

impl DesktopState {
    pub(crate) fn authority(&self) -> Result<DesktopAuthority, DesktopSwitchError> {
        self.authority
            .read()
            .map(|authority| authority.clone())
            .map_err(|_| DesktopSwitchError::StateUnavailable)
    }
}

impl Drop for DesktopState {
    fn drop(&mut self) {
        if let Some(gateway) = self.setup_gateway.take() {
            gateway.shutdown();
        }
    }
}

#[derive(Debug, Clone)]
pub(crate) struct DesktopAuthority {
    pub profile: Option<ConsoleProfileSummary>,
    pub paired_profile_loaded: bool,
}

impl DesktopAuthority {
    fn setup_required() -> Self {
        Self {
            profile: None,
            paired_profile_loaded: false,
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub(crate) enum DesktopSwitchError {
    AuthorityUnavailable,
    BootstrapUnavailable,
    WindowUnavailable,
    NavigationRejected,
    StateUnavailable,
}

impl DesktopSwitchError {
    pub const fn code(self) -> &'static str {
        match self {
            Self::AuthorityUnavailable => "authority_unavailable",
            Self::BootstrapUnavailable => "bootstrap_unavailable",
            Self::WindowUnavailable => "desktop_window_unavailable",
            Self::NavigationRejected => "desktop_navigation_rejected",
            Self::StateUnavailable => "desktop_state_unavailable",
        }
    }
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "captain_desktop=info,tauri=info".into()),
        )
        .init();

    let mut manager = ConsoleManager::open_default().expect("Captain Console state is unavailable");
    let initial_profiles = manager
        .list()
        .expect("Captain Console profile inventory is unavailable");
    let (port, bootstrap_url, authority, setup_gateway) = match manager.launch_active() {
        Ok(mut launch) => {
            let bootstrap_url = launch
                .take_bootstrap_url()
                .expect("Captain Desktop bootstrap URL is unavailable");
            (
                launch.port,
                bootstrap_url,
                DesktopAuthority {
                    profile: Some(launch.profile),
                    paired_profile_loaded: launch.paired_profile_loaded,
                },
                None,
            )
        }
        Err(ConsoleManagerError::NoActiveProfile)
        | Err(ConsoleManagerError::Profile(ConsoleProfileError::ProfileUnconfigured)) => {
            let mut gateway =
                captain_console::start_gateway().expect("Captain Desktop gateway failed to start");
            let bootstrap_url = gateway
                .take_bootstrap_url()
                .expect("Captain Desktop bootstrap URL is unavailable");
            (
                gateway.port,
                bootstrap_url,
                DesktopAuthority::setup_required(),
                Some(gateway),
            )
        }
        Err(error) => panic!("Captain Desktop authority failed to start: {error}"),
    };
    let paired_profile_loaded = authority.paired_profile_loaded;
    let allowed_ports = Arc::new(RwLock::new(HashSet::from([port])));
    let navigation_ports = Arc::clone(&allowed_ports);
    info!(
        port,
        paired_profile_loaded, "Starting lightweight Captain Desktop Client"
    );

    let mut builder = tauri::Builder::default();
    #[cfg(desktop)]
    {
        builder = builder.plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
            reveal_main_window(app);
        }));
        builder = builder.plugin(
            tauri_plugin_autostart::Builder::new()
                .args(["--minimized"])
                .build(),
        );
        match shortcuts::build_shortcut_plugin() {
            Ok(plugin) => builder = builder.plugin(plugin),
            Err(error) => warn!(error = %error, "Desktop shortcut registration unavailable"),
        }
    }

    builder
        .manage(DesktopState {
            started_at: Instant::now(),
            manager: Mutex::new(manager),
            authority: RwLock::new(authority),
            allowed_ports,
            setup_gateway,
        })
        .invoke_handler(tauri::generate_handler![
            commands::get_status,
            commands::get_autostart,
            commands::set_autostart,
        ])
        .setup(move |app| {
            WebviewWindowBuilder::new(
                app,
                "main",
                WebviewUrl::External(
                    bootstrap_url
                        .parse()
                        .map_err(|_| "invalid Desktop bootstrap URL")?,
                ),
            )
            .title("Captain")
            .inner_size(1280.0, 800.0)
            .min_inner_size(800.0, 600.0)
            .center()
            .visible(true)
            .on_navigation(move |url| navigation_is_allowed(url, navigation_ports.as_ref()))
            .build()?;

            #[cfg(desktop)]
            tray::setup_tray(app, &initial_profiles)?;
            Ok(())
        })
        .on_window_event(|window, event| {
            #[cfg(desktop)]
            if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                let _ = window.hide();
                api.prevent_close();
            }
        })
        .build(tauri::generate_context!())
        .expect("Failed to build Captain Desktop Client")
        .run(|_app, event| {
            if let tauri::RunEvent::ExitRequested { .. } = event {
                info!("Captain Desktop Client exit requested");
            }
        });
}

pub(crate) fn switch_authority(
    app: &tauri::AppHandle,
    state: &DesktopState,
    selector: &str,
) -> Result<DesktopAuthority, DesktopSwitchError> {
    let mut manager = state
        .manager
        .lock()
        .map_err(|_| DesktopSwitchError::StateUnavailable)?;
    let mut launch = manager
        .launch(selector)
        .map_err(|_| DesktopSwitchError::AuthorityUnavailable)?;
    let url = launch
        .take_bootstrap_url()
        .map_err(|_| DesktopSwitchError::BootstrapUnavailable)?;
    let url = Url::parse(&url).map_err(|_| DesktopSwitchError::BootstrapUnavailable)?;
    let window = app
        .get_webview_window("main")
        .ok_or(DesktopSwitchError::WindowUnavailable)?;
    state
        .allowed_ports
        .write()
        .map_err(|_| DesktopSwitchError::StateUnavailable)?
        .insert(launch.port);
    window
        .navigate(url)
        .map_err(|_| DesktopSwitchError::NavigationRejected)?;
    let authority = DesktopAuthority {
        profile: Some(launch.profile),
        paired_profile_loaded: launch.paired_profile_loaded,
    };
    *state
        .authority
        .write()
        .map_err(|_| DesktopSwitchError::StateUnavailable)? = authority.clone();
    reveal_main_window(app);
    Ok(authority)
}

fn navigation_is_allowed(url: &Url, allowed_ports: &RwLock<HashSet<u16>>) -> bool {
    url.scheme() == "http"
        && url.host_str() == Some("127.0.0.1")
        && url.username().is_empty()
        && url.password().is_none()
        && url.port().is_some_and(|port| {
            allowed_ports
                .read()
                .is_ok_and(|ports| ports.contains(&port))
        })
}

fn reveal_main_window(app: &tauri::AppHandle) {
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.show();
        let _ = window.unminimize();
        let _ = window.set_focus();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn desktop_navigation_accepts_only_registered_loopback_gateways() {
        let ports = RwLock::new(HashSet::from([41001]));
        assert!(navigation_is_allowed(
            &Url::parse("http://127.0.0.1:41001/#/chat").unwrap(),
            &ports,
        ));
        for rejected in [
            "http://127.0.0.1:41002/",
            "https://127.0.0.1:41001/",
            "http://localhost:41001/",
            "http://user@127.0.0.1:41001/",
            "http://127.0.0.1.example:41001/",
        ] {
            assert!(!navigation_is_allowed(
                &Url::parse(rejected).unwrap(),
                &ports,
            ));
        }
        ports.write().unwrap().insert(41002);
        assert!(navigation_is_allowed(
            &Url::parse("http://127.0.0.1:41002/").unwrap(),
            &ports,
        ));
    }
}
