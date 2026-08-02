use nte_dps_tool::{core::timeline::TimelineScope, storage::config::TimelineDpsViewMode};
use tauri::{State, WebviewWindow};

use crate::{
    contract::{CommandError, timeline::TimelineSnapshot},
    state::AppState,
    windows::console,
};

#[tauri::command]
pub(crate) fn get_timeline_snapshot(
    scope: String,
    state: State<'_, AppState>,
    window: WebviewWindow,
) -> Result<TimelineSnapshot, CommandError> {
    console::validate_window(&window)?;
    Ok(snapshot(state.inner(), parse_scope(&scope)?))
}

#[tauri::command]
pub(crate) fn set_timeline_preferences(
    scope: String,
    bucket_seconds: f32,
    view_mode: String,
    state: State<'_, AppState>,
    window: WebviewWindow,
) -> Result<TimelineSnapshot, CommandError> {
    console::validate_window(&window)?;
    let scope = parse_scope(&scope)?;
    let view_mode = parse_view_mode(&view_mode)?;
    state
        .update_timeline_preferences(bucket_seconds, view_mode)
        .map_err(|error| {
            log::warn!("save Timeline preferences failed: {error}");
            CommandError::timeline(
                "timeline_config_save_failed",
                "Timeline preferences could not be saved.",
            )
        })?;
    Ok(snapshot(state.inner(), scope))
}

pub(crate) fn snapshot(state: &AppState, scope: TimelineScope) -> TimelineSnapshot {
    let (_, view_mode) = state.timeline_preferences();
    TimelineSnapshot::from_projection(
        state.timeline_projection(scope),
        state.next_sequence(),
        scope,
        view_mode,
    )
}

pub(crate) fn parse_scope(value: &str) -> Result<TimelineScope, CommandError> {
    match value {
        "all" => Ok(TimelineScope::Whole),
        "upper" => Ok(TimelineScope::First),
        "lower" => Ok(TimelineScope::Second),
        _ => Err(CommandError::timeline(
            "timeline_scope_invalid",
            "Timeline scope is invalid.",
        )),
    }
}

fn parse_view_mode(value: &str) -> Result<TimelineDpsViewMode, CommandError> {
    match value {
        "team" => Ok(TimelineDpsViewMode::Team),
        "characters" => Ok(TimelineDpsViewMode::Characters),
        _ => Err(CommandError::timeline(
            "timeline_view_mode_invalid",
            "Timeline curve mode is invalid.",
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn contract_values_are_explicit_and_bounded() {
        assert_eq!(parse_scope("all").expect("whole"), TimelineScope::Whole);
        assert_eq!(parse_scope("upper").expect("first"), TimelineScope::First);
        assert!(parse_scope("../upper").is_err());
        assert_eq!(
            parse_view_mode("characters").expect("characters"),
            TimelineDpsViewMode::Characters
        );
        assert!(parse_view_mode("stacked").is_err());
    }
}
