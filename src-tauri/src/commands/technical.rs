use tauri::{AppHandle, State, WebviewWindow};

use crate::{
    contract::{CommandError, TechnicalSnapshot},
    state::AppState,
    windows::{hud, island},
};

use super::{parse_hud_module, sanitize_hud_width};

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
    app: AppHandle,
    state: State<'_, AppState>,
    window: WebviewWindow,
) -> Result<TechnicalSnapshot, CommandError> {
    hud::validate_window(&window)?;
    hud::set_passthrough(&window, &state, enabled)?;
    let message_key = if enabled {
        "HUD passthrough on; press {} to enter edit mode"
    } else {
        "HUD edit mode on; press {} to return to game passthrough"
    };
    island::publish_notice(
        &app,
        state.inner(),
        "status",
        message_key,
        vec![state.passthrough_hotkey().label().to_owned()],
    )?;
    Ok(state.snapshot())
}

#[tauri::command]
pub(crate) fn set_hud_always_on_top(
    enabled: bool,
    app: AppHandle,
    state: State<'_, AppState>,
    window: WebviewWindow,
) -> Result<TechnicalSnapshot, CommandError> {
    hud::validate_window(&window)?;
    hud::set_always_on_top(&window, &state, enabled)?;
    island::publish_notice(
        &app,
        state.inner(),
        "status",
        if enabled {
            "Always-on-top enabled"
        } else {
            "Always-on-top disabled"
        },
        Vec::new(),
    )?;
    Ok(state.snapshot())
}

#[tauri::command]
pub(crate) fn move_hud_module(
    dragged: String,
    target: String,
    insert_after: bool,
    state: State<'_, AppState>,
    window: WebviewWindow,
) -> Result<TechnicalSnapshot, CommandError> {
    hud::validate_window(&window)?;
    let dragged = parse_hud_module(&dragged)?;
    let target = parse_hud_module(&target)?;
    state
        .move_hud_module(dragged, target, insert_after)
        .map_err(|error| {
            log::error!("save HUD module order failed: {error}");
            CommandError::hud_config_save_failed()
        })?;
    Ok(state.snapshot())
}

#[tauri::command]
pub(crate) fn set_hud_module_visibility(
    module: String,
    visible: bool,
    state: State<'_, AppState>,
    window: WebviewWindow,
) -> Result<TechnicalSnapshot, CommandError> {
    hud::validate_window(&window)?;
    let module = parse_hud_module(&module)?;
    let changed = state
        .set_hud_module_visibility(module, visible)
        .map_err(|error| {
            log::error!("save HUD module visibility failed: {error}");
            CommandError::hud_config_save_failed()
        })?;
    if changed && hud::sync_content_height(&window, &state).is_err() {
        log::warn!("native HUD height refresh failed after module visibility change");
    }
    Ok(state.snapshot())
}

#[tauri::command]
pub(crate) fn set_hud_width(
    width: i32,
    state: State<'_, AppState>,
    window: WebviewWindow,
) -> Result<TechnicalSnapshot, CommandError> {
    hud::validate_window(&window)?;
    let width = sanitize_hud_width(width);
    let changed = state.set_hud_width(width).map_err(|error| {
        log::error!("save HUD width failed: {error}");
        CommandError::hud_config_save_failed()
    })?;
    if changed && hud::sync_content_width(&window, &state).is_err() {
        log::warn!("native HUD width refresh failed after configuration change");
    }
    Ok(state.snapshot())
}

#[tauri::command]
pub(crate) fn start_hud_capture(
    app: AppHandle,
    state: State<'_, AppState>,
    window: WebviewWindow,
) -> Result<TechnicalSnapshot, CommandError> {
    hud::validate_window(&window)?;
    state
        .request_capture_start(false)
        .map_err(CommandError::from_core)?;
    island::publish_notice(
        &app,
        state.inner(),
        "status",
        "Starting live capture...",
        Vec::new(),
    )?;
    Ok(state.snapshot())
}

#[tauri::command]
pub(crate) fn stop_hud_capture(
    app: AppHandle,
    state: State<'_, AppState>,
    window: WebviewWindow,
) -> Result<TechnicalSnapshot, CommandError> {
    hud::validate_window(&window)?;
    state
        .request_capture_stop()
        .map_err(CommandError::from_core)?;
    island::publish_notice(
        &app,
        state.inner(),
        "status",
        "Stopping live capture...",
        Vec::new(),
    )?;
    Ok(state.snapshot())
}
