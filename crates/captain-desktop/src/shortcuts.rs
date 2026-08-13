//! System-wide shortcuts for the lightweight Desktop Client.

use tauri::Manager;
use tauri_plugin_global_shortcut::ShortcutState;

/// `Ctrl+Shift+O` only reveals the existing Client window. The Desktop does
/// not expose local execution or administrative shortcuts.
pub fn build_shortcut_plugin<R: tauri::Runtime>(
) -> Result<tauri::plugin::TauriPlugin<R>, tauri_plugin_global_shortcut::Error> {
    Ok(tauri_plugin_global_shortcut::Builder::new()
        .with_shortcuts(["ctrl+shift+o"])?
        .with_handler(|app, _shortcut, event| {
            if event.state == ShortcutState::Pressed {
                if let Some(window) = app.get_webview_window("main") {
                    let _ = window.show();
                    let _ = window.unminimize();
                    let _ = window.set_focus();
                }
            }
        })
        .build())
}
