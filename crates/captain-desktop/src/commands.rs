//! Minimal Tauri IPC for the lightweight Desktop Client.

use crate::DesktopState;
use tauri_plugin_autostart::ManagerExt;

#[tauri::command]
pub fn get_status(state: tauri::State<'_, DesktopState>) -> serde_json::Value {
    let authority = state.authority().ok();
    let paired_profile_loaded = authority
        .as_ref()
        .is_some_and(|authority| authority.paired_profile_loaded);
    serde_json::json!({
        "status": if paired_profile_loaded { "paired" } else { "setup_required" },
        "surface": "client",
        "uptime_secs": state.started_at.elapsed().as_secs(),
        "desktop_policy_version": captain_wire::DESKTOP_CLIENT_POLICY_VERSION,
        "paired_profile_loaded": paired_profile_loaded,
        "execution_capable": false,
    })
}

#[tauri::command]
pub fn get_autostart(app: tauri::AppHandle) -> Result<bool, String> {
    app.autolaunch()
        .is_enabled()
        .map_err(|error| error.to_string())
}

#[tauri::command]
pub fn set_autostart(app: tauri::AppHandle, enabled: bool) -> Result<bool, String> {
    let manager = app.autolaunch();
    if enabled {
        manager.enable().map_err(|error| error.to_string())?;
    } else {
        manager.disable().map_err(|error| error.to_string())?;
    }
    manager.is_enabled().map_err(|error| error.to_string())
}
