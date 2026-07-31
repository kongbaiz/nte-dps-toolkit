use tauri::{AppHandle, Manager, WebviewWindow, WindowEvent};

use crate::contract::CommandError;
use crate::windows::{abyss_values::ABYSS_VALUES_WINDOW_LABEL, hud::HUD_WINDOW_LABEL};

pub(crate) const CONSOLE_WINDOW_LABEL: &str = "console";

pub(crate) fn validate_window(window: &WebviewWindow) -> Result<(), CommandError> {
    if window.label() == CONSOLE_WINDOW_LABEL {
        Ok(())
    } else {
        Err(CommandError::invalid_window())
    }
}

#[tauri::command]
pub(crate) fn show_console_when_ready(window: WebviewWindow) -> Result<(), CommandError> {
    validate_window(&window)?;
    window.show().map_err(|error| {
        log::error!("show Console after the frontend first paint failed: {error}");
        CommandError::window_operation_failed()
    })
}

/// The Console owns the migration-period HUD window. The HUD starts hidden and
/// has no native close affordance, so closing the Console must close it as well
/// to let the desktop process terminate normally.
pub(crate) fn bind_owned_window_lifetime(window: &WebviewWindow, app: AppHandle) {
    window.on_window_event(move |event| {
        if matches!(event, WindowEvent::CloseRequested { .. })
            && let Some(hud_window) = app.get_webview_window(HUD_WINDOW_LABEL)
            && let Err(error) = hud_window.close()
        {
            log::error!("close owned HUD window with Console failed: {error}");
        }
        if matches!(event, WindowEvent::CloseRequested { .. })
            && let Some(abyss_window) = app.get_webview_window(ABYSS_VALUES_WINDOW_LABEL)
            && let Err(error) = abyss_window.destroy()
        {
            log::error!("destroy owned abyss values window with Console failed: {error}");
        }
    });
}
