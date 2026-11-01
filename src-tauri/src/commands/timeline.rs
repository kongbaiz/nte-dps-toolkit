use nte_dps_tool::{core::timeline::TimelineScope, storage::config::TimelineDpsViewMode};
use tauri::{State, WebviewWindow};

use crate::{
    contract::{CommandError, timeline::TimelineSnapshot},
    state::AppState,
    windows::console,
};

#[tauri::command]
pub(crate) fn get_timeline_snapshot(
    scope: TimelineScope,
    state: State<'_, AppState>,
    window: WebviewWindow,
) -> Result<TimelineSnapshot, CommandError> {
    console::validate_window(&window)?;
    snapshot(state.inner(), scope)
}

#[tauri::command]
pub(crate) fn set_timeline_preferences(
    scope: TimelineScope,
    bucket_seconds: f32,
    view_mode: TimelineDpsViewMode,
    state: State<'_, AppState>,
    window: WebviewWindow,
) -> Result<TimelineSnapshot, CommandError> {
    console::validate_window(&window)?;
    state
        .update_timeline_preferences(bucket_seconds, view_mode)
        .map_err(|error| {
            log::warn!("save Timeline preferences failed: {error}");
            CommandError::timeline(
                "timeline_config_save_failed",
                "Timeline preferences could not be saved.",
            )
        })?;
    snapshot(state.inner(), scope)
}

pub(crate) fn snapshot(
    state: &AppState,
    scope: TimelineScope,
) -> Result<TimelineSnapshot, CommandError> {
    let (_, view_mode) = state.timeline_preferences();
    let projection = state
        .timeline_projection(scope)
        .map_err(CommandError::from_core)?;
    Ok(TimelineSnapshot::from_projection(
        projection.as_ref(),
        state.next_sequence(),
        scope,
        view_mode,
    ))
}
