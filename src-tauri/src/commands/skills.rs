use nte_dps_tool::core::timeline::TimelineScope;
use tauri::{State, WebviewWindow};

use crate::{
    contract::{CommandError, skills::SkillsSnapshot},
    state::AppState,
    windows::console,
};

#[tauri::command]
pub(crate) fn get_skills_snapshot(
    scope: TimelineScope,
    state: State<'_, AppState>,
    window: WebviewWindow,
) -> Result<SkillsSnapshot, CommandError> {
    console::validate_window(&window)?;
    snapshot(state.inner(), scope)
}

pub(crate) fn snapshot(
    state: &AppState,
    scope: TimelineScope,
) -> Result<SkillsSnapshot, CommandError> {
    Ok(SkillsSnapshot::from_projection(
        state
            .skills_projection(scope)
            .map_err(CommandError::from_core)?,
        state.next_sequence(),
        scope,
    ))
}
