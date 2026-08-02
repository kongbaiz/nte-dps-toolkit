use nte_dps_tool::{core::team_data::parse_team_data, engine::abyss_data::AbyssMonsterDataset};
use tauri::{State, WebviewWindow};

use crate::{
    contract::{
        CommandError,
        abyss_values::{AbyssPredictionTeamsSnapshot, AbyssValuesSnapshot},
    },
    state::AppState,
    windows::abyss_values,
};

#[tauri::command]
pub(crate) async fn get_abyss_values_snapshot(
    state: State<'_, AppState>,
    window: WebviewWindow,
) -> Result<AbyssValuesSnapshot, CommandError> {
    abyss_values::validate_window(&window)?;
    let teams = state.imported_abyss_teams();
    let dataset = tauri::async_runtime::spawn_blocking(AbyssMonsterDataset::load)
        .await
        .map_err(|error| {
            log::error!("abyss values loader worker failed: {error}");
            CommandError::abyss_values_unavailable()
        })?
        .map_err(|error| {
            log::error!("load abyss values failed: {error}");
            CommandError::abyss_values_unavailable()
        })?;
    Ok(AbyssValuesSnapshot::new(dataset, teams))
}

#[tauri::command]
pub(crate) fn import_abyss_prediction_team(
    half: String,
    json: String,
    state: State<'_, AppState>,
    window: WebviewWindow,
) -> Result<AbyssPredictionTeamsSnapshot, CommandError> {
    abyss_values::validate_window(&window)?;
    let upper = parse_half(&half)?;
    let export = parse_team_data(&json).map_err(|detail| {
        log::warn!("reject imported abyss team data: {detail}");
        CommandError::team_data_invalid()
    })?;
    if !state.import_abyss_team(export, upper) {
        return Err(CommandError::abyss_team_unavailable());
    }
    Ok(state.imported_abyss_teams().into())
}

#[tauri::command]
pub(crate) fn import_current_abyss_prediction_team(
    half: String,
    state: State<'_, AppState>,
    window: WebviewWindow,
) -> Result<AbyssPredictionTeamsSnapshot, CommandError> {
    abyss_values::validate_window(&window)?;
    if !state.import_current_abyss_team(parse_half(&half)?) {
        return Err(CommandError::abyss_team_unavailable());
    }
    Ok(state.imported_abyss_teams().into())
}

#[tauri::command]
pub(crate) fn clear_abyss_prediction_team(
    half: String,
    state: State<'_, AppState>,
    window: WebviewWindow,
) -> Result<AbyssPredictionTeamsSnapshot, CommandError> {
    abyss_values::validate_window(&window)?;
    state.clear_abyss_team(parse_half(&half)?);
    Ok(state.imported_abyss_teams().into())
}

#[tauri::command]
pub(crate) fn swap_abyss_prediction_teams(
    state: State<'_, AppState>,
    window: WebviewWindow,
) -> Result<AbyssPredictionTeamsSnapshot, CommandError> {
    abyss_values::validate_window(&window)?;
    state.swap_abyss_teams();
    Ok(state.imported_abyss_teams().into())
}

fn parse_half(half: &str) -> Result<bool, CommandError> {
    match half {
        "upper" => Ok(true),
        "lower" => Ok(false),
        _ => Err(CommandError::invalid_settings_input()),
    }
}

#[cfg(test)]
mod tests {
    use super::parse_half;

    #[test]
    fn parses_stable_half_identifiers() {
        assert_eq!(parse_half("upper").expect("upper is valid"), true);
        assert_eq!(parse_half("lower").expect("lower is valid"), false);
        assert!(parse_half("first").is_err());
    }
}
