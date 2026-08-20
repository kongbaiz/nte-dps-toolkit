use std::time::Duration;

use nte_dps_tool::core::live_capture::LiveCapturePhase;
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
    state.snapshot().map_err(CommandError::from_core)
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
    state.snapshot().map_err(CommandError::from_core)
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
    state.snapshot().map_err(CommandError::from_core)
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
    state.snapshot().map_err(CommandError::from_core)
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
    state.snapshot().map_err(CommandError::from_core)
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
    state.snapshot().map_err(CommandError::from_core)
}

#[tauri::command]
pub(crate) fn start_hud_capture(
    app: AppHandle,
    state: State<'_, AppState>,
    window: WebviewWindow,
) -> Result<TechnicalSnapshot, CommandError> {
    hud::validate_window(&window)?;
    // Starting a new capture from the compact HUD is an explicit new-session
    // action. Replace the stopped session instead of surfacing the main-window
    // confirmation flow, which the HUD cannot present.
    state
        .request_capture_start(true)
        .map_err(CommandError::from_core)?;
    island::publish_notice(
        &app,
        state.inner(),
        "status",
        "Starting live capture...",
        Vec::new(),
    )?;
    state.snapshot().map_err(CommandError::from_core)
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
    state.snapshot().map_err(CommandError::from_core)
}

#[tauri::command]
pub(crate) async fn reset_hud_session(
    app: AppHandle,
    state: State<'_, AppState>,
    window: WebviewWindow,
) -> Result<TechnicalSnapshot, CommandError> {
    hud::validate_window(&window)?;
    let active = matches!(
        state.capture_phase(),
        LiveCapturePhase::Starting | LiveCapturePhase::Running | LiveCapturePhase::Stopping
    ) || state.replay_running().map_err(CommandError::from_core)?;
    let state = state.inner().clone();
    let worker_state = state.clone();
    let undo_token = tauri::async_runtime::spawn_blocking(move || {
        let undo_token = if active {
            worker_state
                .stop_active_capture_and_wait(Duration::from_secs(5))
                .map_err(CommandError::from_core)?;
            worker_state
                .clear_session()
                .map_err(CommandError::from_core)?;
            None
        } else {
            worker_state
                .reset_session_with_undo()
                .map_err(CommandError::from_core)?
        };
        worker_state
            .set_main_processing_paused(false)
            .map_err(super::main_dps::presentation_error)?;
        worker_state
            .set_main_selected_round_id(None)
            .map_err(super::main_dps::presentation_error)?;
        Ok::<_, CommandError>(undo_token)
    })
    .await
    .map_err(|_| CommandError::main_dps("reset_failed", "Failed to reset the current session"))??;
    state.publish_island_notice(
        "success",
        if undo_token.is_some() {
            "Session reset · use Undo within 5 seconds"
        } else {
            "Stats reset"
        },
        Vec::new(),
        undo_token,
    );
    if let Err(error) = island::show_notice(&app, &state) {
        log::warn!("show reset notification from HUD failed: {error:?}");
    }
    state.snapshot().map_err(CommandError::from_core)
}
