use nte_dps_tool::{
    core::team_data::parse_team_data, engine::model::AbyssHalf, storage::abyss_remote,
};
use tauri::{State, WebviewWindow};

use crate::{
    contract::{
        CommandError,
        abyss_values::{AbyssPredictionTeamsSnapshot, AbyssValuesSnapshot},
    },
    state::{AppState, TeamOperationError},
    team_import_service::TeamImportError,
    windows::abyss_values,
};

#[tauri::command]
pub(crate) async fn get_abyss_values_snapshot(
    state: State<'_, AppState>,
    window: WebviewWindow,
) -> Result<AbyssValuesSnapshot, CommandError> {
    abyss_values::validate_window(&window)?;
    let teams = state.imported_abyss_teams().map_err(team_import_error)?;
    let current_team_available = state
        .current_abyss_team_availability()
        .map_err(CommandError::from_core)?;
    let loaded = tauri::async_runtime::spawn_blocking(abyss_remote::load_latest_abyss_dataset)
        .await
        .map_err(|error| {
            log::error!("abyss values loader worker failed: {error}");
            CommandError::abyss_values_unavailable()
        })?
        .map_err(|error| {
            log::error!("load remote abyss values failed: {error}");
            CommandError::abyss_values_unavailable()
        })?;
    if loaded.stale {
        log::warn!(
            "abyss data refresh failed; using verified cache version {}",
            loaded.data_version
        );
    }
    Ok(AbyssValuesSnapshot::new(
        loaded,
        teams,
        current_team_available,
    ))
}

#[tauri::command]
pub(crate) fn import_abyss_prediction_team(
    half: AbyssHalf,
    json: String,
    state: State<'_, AppState>,
    window: WebviewWindow,
) -> Result<AbyssPredictionTeamsSnapshot, CommandError> {
    abyss_values::validate_window(&window)?;
    let upper = half == AbyssHalf::First;
    let export = parse_team_data(&json).map_err(|detail| {
        log::warn!("reject imported abyss team data: {detail}");
        CommandError::team_data_invalid()
    })?;
    if !state
        .import_abyss_team(export, upper)
        .map_err(team_import_error)?
    {
        return Err(CommandError::abyss_team_unavailable());
    }
    Ok(state
        .imported_abyss_teams()
        .map_err(team_import_error)?
        .into())
}

#[tauri::command]
pub(crate) fn import_current_abyss_prediction_team(
    half: AbyssHalf,
    state: State<'_, AppState>,
    window: WebviewWindow,
) -> Result<AbyssPredictionTeamsSnapshot, CommandError> {
    abyss_values::validate_window(&window)?;
    if !state
        .import_current_abyss_team(half == AbyssHalf::First)
        .map_err(team_operation_error)?
    {
        return Err(CommandError::abyss_team_unavailable());
    }
    Ok(state
        .imported_abyss_teams()
        .map_err(team_import_error)?
        .into())
}

#[tauri::command]
pub(crate) fn clear_abyss_prediction_team(
    half: AbyssHalf,
    state: State<'_, AppState>,
    window: WebviewWindow,
) -> Result<AbyssPredictionTeamsSnapshot, CommandError> {
    abyss_values::validate_window(&window)?;
    state
        .clear_abyss_team(half == AbyssHalf::First)
        .map_err(team_import_error)?;
    Ok(state
        .imported_abyss_teams()
        .map_err(team_import_error)?
        .into())
}

#[tauri::command]
pub(crate) fn swap_abyss_prediction_teams(
    state: State<'_, AppState>,
    window: WebviewWindow,
) -> Result<AbyssPredictionTeamsSnapshot, CommandError> {
    abyss_values::validate_window(&window)?;
    state.swap_abyss_teams().map_err(team_import_error)?;
    Ok(state
        .imported_abyss_teams()
        .map_err(team_import_error)?
        .into())
}

fn team_import_error(_error: TeamImportError) -> CommandError {
    CommandError::team_import_state_unavailable()
}

fn team_operation_error(error: TeamOperationError) -> CommandError {
    match error {
        TeamOperationError::State(error) => team_import_error(error),
        TeamOperationError::Capture(error) => CommandError::from_core(error),
    }
}
