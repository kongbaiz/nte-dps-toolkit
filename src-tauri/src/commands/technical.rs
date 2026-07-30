use tauri::{State, WebviewWindow};

use crate::{
    contract::{CommandError, TechnicalSnapshot},
    state::AppState,
    windows::hud,
};

#[tauri::command]
pub(crate) fn get_technical_snapshot(
    state: State<'_, AppState>,
    window: WebviewWindow,
) -> Result<TechnicalSnapshot, CommandError> {
    hud::validate_window(&window)?;
    Ok(state.snapshot())
}

#[tauri::command]
pub(crate) fn set_hud_passthrough(
    enabled: bool,
    state: State<'_, AppState>,
    window: WebviewWindow,
) -> Result<TechnicalSnapshot, CommandError> {
    hud::validate_window(&window)?;
    hud::set_passthrough(&window, &state, enabled)?;
    Ok(state.snapshot())
}

#[tauri::command]
pub(crate) fn set_hud_always_on_top(
    enabled: bool,
    state: State<'_, AppState>,
    window: WebviewWindow,
) -> Result<TechnicalSnapshot, CommandError> {
    hud::validate_window(&window)?;
    hud::set_always_on_top(&window, &state, enabled)?;
    Ok(state.snapshot())
}
