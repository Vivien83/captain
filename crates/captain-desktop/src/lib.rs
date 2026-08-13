//! Captain Desktop — lightweight paired Client for one complete Hub.
//!
//! This process contains no agent loop, provider, memory database or local
//! execution engine. It renders the Hub work surface through an ephemeral
//! loopback gateway that keeps the paired bearer out of JavaScript.

mod commands;
mod server;
mod shortcuts;
mod tray;

use std::time::Instant;
use tauri::{Manager, WebviewUrl, WebviewWindowBuilder};
use tracing::{info, warn};

pub struct DesktopState {
    pub started_at: Instant,
    pub paired_profile_loaded: bool,
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "captain_desktop=info,tauri=info".into()),
        )
        .init();

    let mut gateway = server::start_gateway().expect("Captain Desktop gateway failed to start");
    let port = gateway.port;
    let paired_profile_loaded = gateway.paired_profile_loaded;
    let bootstrap_url = gateway
        .take_bootstrap_url()
        .expect("Captain Desktop bootstrap URL is unavailable");
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
            paired_profile_loaded,
        })
        .invoke_handler(tauri::generate_handler![
            commands::get_status,
            commands::get_autostart,
            commands::set_autostart,
        ])
        .setup(move |app| {
            let allowed_port = port;
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
            .on_navigation(move |url| {
                url.scheme() == "http"
                    && url.host_str() == Some("127.0.0.1")
                    && url.port() == Some(allowed_port)
            })
            .build()?;

            #[cfg(desktop)]
            tray::setup_tray(app)?;
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

    gateway.shutdown();
}

fn reveal_main_window(app: &tauri::AppHandle) {
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.show();
        let _ = window.unminimize();
        let _ = window.set_focus();
    }
}
