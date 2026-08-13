//! System tray for the lightweight Desktop Client.

use tauri::{
    menu::{CheckMenuItem, Menu, MenuItem, PredefinedMenuItem},
    tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent},
    Manager,
};
use tauri_plugin_autostart::ManagerExt;
use tracing::{info, warn};

const TRAY_ICON_PNG: &[u8] = include_bytes!("../icons/32x32.png");

pub fn setup_tray(app: &tauri::App) -> Result<(), Box<dyn std::error::Error>> {
    let show = MenuItem::with_id(app, "show", "Show Captain", true, None::<&str>)?;
    let paired = app
        .try_state::<crate::DesktopState>()
        .is_some_and(|state| state.paired_profile_loaded);
    let status = MenuItem::with_id(
        app,
        "status",
        if paired {
            "Paired Client · Hub profile loaded"
        } else {
            "Client setup required"
        },
        false,
        None::<&str>,
    )?;
    let launch_at_login = CheckMenuItem::with_id(
        app,
        "launch_at_login",
        "Launch at Login",
        true,
        app.autolaunch().is_enabled().unwrap_or(false),
        None::<&str>,
    )?;
    let quit = MenuItem::with_id(app, "quit", "Quit Captain", true, None::<&str>)?;
    let menu = Menu::with_items(
        app,
        &[
            &show,
            &PredefinedMenuItem::separator(app)?,
            &status,
            &PredefinedMenuItem::separator(app)?,
            &launch_at_login,
            &PredefinedMenuItem::separator(app)?,
            &quit,
        ],
    )?;
    let icon = tauri::image::Image::from_bytes(TRAY_ICON_PNG)?;

    TrayIconBuilder::new()
        .icon(icon)
        .menu(&menu)
        .tooltip("Captain Client")
        .on_menu_event(move |app, event| match event.id().as_ref() {
            "show" => reveal(app),
            "launch_at_login" => {
                let manager = app.autolaunch();
                let result = if manager.is_enabled().unwrap_or(false) {
                    manager.disable()
                } else {
                    manager.enable()
                };
                if let Err(error) = result {
                    warn!(error = %error, "Desktop Client autostart change failed");
                }
            }
            "quit" => {
                info!("Desktop Client exit requested");
                app.exit(0);
            }
            _ => {}
        })
        .on_tray_icon_event(|tray, event| {
            if matches!(
                event,
                TrayIconEvent::Click {
                    button: MouseButton::Left,
                    button_state: MouseButtonState::Up,
                    ..
                }
            ) {
                reveal(tray.app_handle());
            }
        })
        .build(app)?;
    Ok(())
}

fn reveal(app: &tauri::AppHandle) {
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.show();
        let _ = window.unminimize();
        let _ = window.set_focus();
    }
}

#[cfg(test)]
mod tests {
    use super::TRAY_ICON_PNG;

    fn png_dimensions(bytes: &[u8]) -> (u32, u32) {
        assert!(bytes.starts_with(b"\x89PNG\r\n\x1a\n"));
        (
            u32::from_be_bytes(bytes[16..20].try_into().unwrap()),
            u32::from_be_bytes(bytes[20..24].try_into().unwrap()),
        )
    }

    #[test]
    fn desktop_brand_assets_keep_the_expected_sizes() {
        assert_eq!(png_dimensions(TRAY_ICON_PNG), (32, 32));
        assert_eq!(
            png_dimensions(include_bytes!("../icons/128x128.png")),
            (128, 128)
        );
        assert_eq!(
            png_dimensions(include_bytes!("../icons/128x128@2x.png")),
            (256, 256)
        );
        assert_eq!(
            png_dimensions(include_bytes!("../icons/icon.png")),
            (512, 512)
        );
    }
}
