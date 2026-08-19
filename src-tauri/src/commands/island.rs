use tauri::{AppHandle, State, WebviewWindow};

use crate::{
    contract::{CommandError, island::IslandSnapshot},
    state::AppState,
    windows::island,
};

#[tauri::command]
pub(crate) fn get_island_snapshot(
    state: State<'_, AppState>,
    window: WebviewWindow,
) -> Result<IslandSnapshot, CommandError> {
    island::validate_window(&window)?;
    Ok(IslandSnapshot::from_state(state.inner()))
}

#[tauri::command]
pub(crate) fn dismiss_island_notice(
    notice_id: String,
    state: State<'_, AppState>,
    window: WebviewWindow,
) -> Result<IslandSnapshot, CommandError> {
    island::validate_window(&window)?;
    if state.dismiss_island_notice(&notice_id) {
        window
            .hide()
            .map_err(|_| CommandError::window_operation_failed())?;
    }
    Ok(IslandSnapshot::from_state(state.inner()))
}

#[tauri::command]
pub(crate) fn undo_island_notice(
    notice_id: String,
    app: AppHandle,
    state: State<'_, AppState>,
    window: WebviewWindow,
) -> Result<IslandSnapshot, CommandError> {
    island::validate_window(&window)?;
    let notice = state
        .island_notice()
        .filter(|notice| notice.id == notice_id)
        .ok_or_else(|| {
            CommandError::main_dps(
                "session_undo_missing",
                "The previous session is no longer available",
            )
        })?;
    let token = notice.undo_token.ok_or_else(|| {
        CommandError::main_dps("session_undo_missing", "The notice has no undo action")
    })?;
    state
        .undo_session_reset(&token)
        .map_err(super::main_dps::session_undo_error)?;
    state.dismiss_island_notice(&notice_id);
    state.publish_island_notice("success", "Previous session restored", Vec::new(), None);
    island::show_notice(&app, state.inner())?;
    Ok(IslandSnapshot::from_state(state.inner()))
}
