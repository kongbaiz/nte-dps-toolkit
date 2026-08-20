use nte_dps_tool::core::packets::PacketStreamRevision;
use tauri::{State, WebviewWindow};

use crate::{
    contract::{CommandError, packets::PacketsSnapshot},
    state::AppState,
    windows::console,
};

#[tauri::command]
pub(crate) fn get_packets_snapshot(
    state: State<'_, AppState>,
    window: WebviewWindow,
) -> Result<PacketsSnapshot, CommandError> {
    console::validate_window(&window)?;
    snapshot(state.inner())
}

pub(crate) fn snapshot(state: &AppState) -> Result<PacketsSnapshot, CommandError> {
    let (_, _, projection) = state
        .packets_projection(None)
        .map_err(CommandError::from_core)?;
    Ok(PacketsSnapshot::from_projection(
        projection,
        state.capture_phase(),
    ))
}

pub(crate) fn snapshot_since(
    state: &AppState,
    after: Option<PacketStreamRevision>,
) -> Result<(PacketStreamRevision, bool, PacketsSnapshot), CommandError> {
    let (revision, replace, projection) = state
        .packets_projection(after)
        .map_err(CommandError::from_core)?;
    Ok((
        revision,
        replace,
        PacketsSnapshot::from_projection(projection, state.capture_phase()),
    ))
}
