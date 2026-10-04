//! The desktop player (Windows, macOS, Linux), built with the `desktop` feature.

pub mod commands;
pub mod convert;
pub mod decode;
pub mod engine;
pub mod media_controls;
pub mod persist;
pub mod pipeline;
pub mod player;
pub mod queue;
pub mod tags;

use std::sync::Arc;

use tauri::{AppHandle, Runtime};

/// How the system media controls find this app: its name, and (Windows) its first window.
pub fn panel_config<R: Runtime>(app: &AppHandle<R>) -> media_controls::PanelConfig {
    let identifier = app.config().identifier.clone();
    let handle = app.clone();
    media_controls::PanelConfig {
        display_name: app.package_info().name.clone(),
        // D-Bus names allow letters, digits, _ and dots.
        dbus_name: identifier.replace(|c: char| !(c.is_ascii_alphanumeric() || c == '.' || c == '_'), "_"),
        window: Arc::new(move || {
            #[cfg(target_os = "windows")]
            {
                use tauri::Manager;
                handle.webview_windows().values().next().and_then(|w| w.hwnd().ok()).map(|h| h.0 as isize)
            }
            #[cfg(not(target_os = "windows"))]
            {
                let _ = &handle;
                None
            }
        }),
    }
}
