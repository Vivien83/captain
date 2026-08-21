//! System tray for the lightweight Desktop Client.

use captain_console::ConsoleProfileSummary;
use std::sync::Arc;
use tauri::{
    menu::{CheckMenuItem, Menu, MenuItem, PredefinedMenuItem},
    tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent},
    Manager,
};
use tauri_plugin_autostart::ManagerExt;
use tracing::{info, warn};

const TRAY_ICON_PNG: &[u8] = include_bytes!("../icons/32x32.png");
const PROFILE_MENU_PREFIX: &str = "captain_profile:";

pub fn setup_tray(
    app: &tauri::App,
    profiles: &[ConsoleProfileSummary],
) -> Result<(), Box<dyn std::error::Error>> {
    let show = MenuItem::with_id(app, "show", "Show Captain", true, None::<&str>)?;
    let authority = app
        .try_state::<crate::DesktopState>()
        .and_then(|state| state.authority().ok());
    let status = MenuItem::with_id(
        app,
        "status",
        authority_status_label(authority.as_ref()),
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
    let profile_items = profiles
        .iter()
        .map(|profile| {
            Ok((
                profile.id.clone(),
                CheckMenuItem::with_id(
                    app,
                    format!("{PROFILE_MENU_PREFIX}{}", profile.id),
                    &profile.label,
                    profile.configured,
                    profile.active,
                    None::<&str>,
                )?,
            ))
        })
        .collect::<Result<Vec<_>, tauri::Error>>()?;
    let profile_items = Arc::new(profile_items);
    let menu = Menu::new(app)?;
    menu.append(&show)?;
    menu.append(&PredefinedMenuItem::separator(app)?)?;
    menu.append(&status)?;
    if !profile_items.is_empty() {
        menu.append(&PredefinedMenuItem::separator(app)?)?;
        menu.append(&MenuItem::with_id(
            app,
            "captains",
            "Captains",
            false,
            None::<&str>,
        )?)?;
        for (_, item) in profile_items.iter() {
            menu.append(item)?;
        }
    }
    menu.append(&PredefinedMenuItem::separator(app)?)?;
    menu.append(&launch_at_login)?;
    menu.append(&PredefinedMenuItem::separator(app)?)?;
    menu.append(&quit)?;
    let icon = tauri::image::Image::from_bytes(TRAY_ICON_PNG)?;
    let event_profile_items = Arc::clone(&profile_items);
    let event_status = status.clone();

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
            menu_id => {
                let Some(profile_id) = menu_id.strip_prefix(PROFILE_MENU_PREFIX) else {
                    return;
                };
                let profile_id = profile_id.to_string();
                let state = app.state::<crate::DesktopState>();
                match crate::switch_authority(app, &state, &profile_id) {
                    Ok(authority) => {
                        for (candidate_id, item) in event_profile_items.iter() {
                            let _ = item.set_checked(candidate_id == &profile_id);
                        }
                        let _ = event_status.set_text(authority_status_label(Some(&authority)));
                    }
                    Err(error) => {
                        warn!(code = error.code(), "Desktop Captain switch failed");
                    }
                }
            }
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

fn authority_status_label(authority: Option<&crate::DesktopAuthority>) -> String {
    match authority {
        Some(authority) if authority.paired_profile_loaded => authority
            .profile
            .as_ref()
            .map(|profile| format!("Connected · {}", profile.label))
            .unwrap_or_else(|| "Paired Client".to_string()),
        Some(authority) => authority
            .profile
            .as_ref()
            .map(|profile| format!("Pairing required · {}", profile.label))
            .unwrap_or_else(|| "Client setup required".to_string()),
        None => "Client state unavailable".to_string(),
    }
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
    use super::{authority_status_label, ConsoleProfileSummary, TRAY_ICON_PNG};

    fn authority(paired: bool) -> crate::DesktopAuthority {
        crate::DesktopAuthority {
            profile: Some(ConsoleProfileSummary {
                id: "00000000-0000-4000-8000-000000000001".to_string(),
                label: "Production".to_string(),
                active: true,
                configured: true,
            }),
            paired_profile_loaded: paired,
        }
    }

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

    #[test]
    fn tray_status_names_the_selected_authority_without_exposing_its_origin() {
        assert_eq!(
            authority_status_label(Some(&authority(true))),
            "Connected · Production"
        );
        assert_eq!(
            authority_status_label(Some(&authority(false))),
            "Pairing required · Production"
        );
        assert_eq!(authority_status_label(None), "Client state unavailable");
    }
}
