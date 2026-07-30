use tauri::WebviewWindow;

use crate::{contract::CommandError, state::AppState};

pub(crate) const HUD_WINDOW_LABEL: &str = "hud-spike";

pub(crate) fn validate_window(window: &WebviewWindow) -> Result<(), CommandError> {
    if window.label() == HUD_WINDOW_LABEL {
        Ok(())
    } else {
        Err(CommandError::invalid_window())
    }
}

pub(crate) fn set_passthrough(
    window: &WebviewWindow,
    state: &AppState,
    enabled: bool,
) -> Result<(), CommandError> {
    window.set_ignore_cursor_events(enabled).map_err(|error| {
        log::error!("set_ignore_cursor_events failed: {error}");
        CommandError::window_operation_failed()
    })?;
    state.set_passthrough(enabled);
    Ok(())
}

pub(crate) fn set_always_on_top(
    window: &WebviewWindow,
    state: &AppState,
    enabled: bool,
) -> Result<(), CommandError> {
    window.set_always_on_top(enabled).map_err(|error| {
        log::error!("set_always_on_top failed: {error}");
        CommandError::window_operation_failed()
    })?;
    state.set_always_on_top(enabled);
    Ok(())
}
