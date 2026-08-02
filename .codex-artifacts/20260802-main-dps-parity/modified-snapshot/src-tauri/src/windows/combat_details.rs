use tauri::{Emitter, WebviewWindow, WindowEvent};

use crate::contract::CommandError;

pub(crate) const COMBAT_DETAILS_WINDOW_LABEL: &str = "combat-details";
pub(crate) const COMBAT_DETAILS_CHANGED_EVENT: &str = "main-dps-detail-requested";

pub(crate) fn validate_window(window: &WebviewWindow) -> Result<(), CommandError> {
    if window.label() == COMBAT_DETAILS_WINDOW_LABEL {
        Ok(())
    } else {
        Err(CommandError::invalid_window())
    }
}

pub(crate) fn show(window: &WebviewWindow) -> Result<(), CommandError> {
    window.show().map_err(window_error)?;
    window.unminimize().map_err(window_error)?;
    window.set_focus().map_err(window_error)?;
    window
        .emit(COMBAT_DETAILS_CHANGED_EVENT, ())
        .map_err(window_error)
}

pub(crate) fn bind_close_to_hide(window: &WebviewWindow) {
    let webview = window.as_ref().clone();
    if let Err(error) = webview.set_auto_resize(true) {
        log::warn!("enable native combat details WebView auto-resize failed: {error}");
    }
    let close_window = window.clone();
    window.on_window_event(move |event| {
        if let WindowEvent::CloseRequested { api, .. } = event {
            api.prevent_close();
            if let Err(error) = close_window.hide() {
                log::error!("hide combat details window failed: {error}");
            }
        }
    });
}

fn window_error(error: tauri::Error) -> CommandError {
    log::error!("combat details window operation failed: {error}");
    CommandError::window_operation_failed()
}
