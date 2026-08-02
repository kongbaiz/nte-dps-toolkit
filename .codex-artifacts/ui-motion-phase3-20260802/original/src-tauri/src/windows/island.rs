use tauri::{AppHandle, Emitter, Manager, PhysicalPosition, WebviewWindow, WindowEvent};

use crate::{contract::CommandError, state::AppState};

pub(crate) const ISLAND_WINDOW_LABEL: &str = "notification-island";
pub(crate) const ISLAND_CHANGED_EVENT: &str = "notification-island-changed";

pub(crate) fn validate_window(window: &WebviewWindow) -> Result<(), CommandError> {
    if window.label() == ISLAND_WINDOW_LABEL {
        Ok(())
    } else {
        Err(CommandError::invalid_window())
    }
}

pub(crate) fn show_notice(app: &AppHandle, state: &AppState) -> Result<(), CommandError> {
    if !state.ui_config_snapshot().island_notifications {
        return Ok(());
    }
    let window = app
        .get_webview_window(ISLAND_WINDOW_LABEL)
        .ok_or_else(CommandError::window_operation_failed)?;
    if let Ok(Some(monitor)) = window.current_monitor()
        && let Ok(size) = window.outer_size()
    {
        let monitor_position = monitor.position();
        let monitor_size = monitor.size();
        let scale = monitor.scale_factor();
        let offset = f64::from(state.ui_config_snapshot().island_offset_x) * scale;
        let x = f64::from(monitor_position.x)
            + (f64::from(monitor_size.width) - f64::from(size.width)) * 0.5
            + offset;
        let y = f64::from(monitor_position.y) + 24.0 * scale;
        window
            .set_position(PhysicalPosition::new(x.round() as i32, y.round() as i32))
            .map_err(|_| CommandError::window_operation_failed())?;
    }
    window
        .show()
        .map_err(|_| CommandError::window_operation_failed())?;
    window
        .emit(ISLAND_CHANGED_EVENT, ())
        .map_err(|_| CommandError::window_operation_failed())
}

pub(crate) fn bind_close_to_hide(window: &WebviewWindow) {
    let window = window.clone();
    window.clone().on_window_event(move |event| {
        if let WindowEvent::CloseRequested { api, .. } = event {
            api.prevent_close();
            let _ = window.hide();
        }
    });
}
