use nte_dps_tool::core::skills::SkillsScope;
use tauri::{State, WebviewWindow};

use crate::{
    contract::{CommandError, skills::SkillsSnapshot},
    state::AppState,
    windows::console,
};

#[tauri::command]
pub(crate) fn get_skills_snapshot(
    scope: String,
    state: State<'_, AppState>,
    window: WebviewWindow,
) -> Result<SkillsSnapshot, CommandError> {
    console::validate_window(&window)?;
    snapshot(state.inner(), parse_scope(&scope)?)
}

pub(crate) fn snapshot(
    state: &AppState,
    scope: SkillsScope,
) -> Result<SkillsSnapshot, CommandError> {
    Ok(SkillsSnapshot::from_projection(
        state
            .skills_projection(scope)
            .map_err(CommandError::from_core)?,
        state.next_sequence(),
        scope,
    ))
}

pub(crate) fn parse_scope(value: &str) -> Result<SkillsScope, CommandError> {
    match value {
        "all" => Ok(SkillsScope::Whole),
        "upper" => Ok(SkillsScope::First),
        "lower" => Ok(SkillsScope::Second),
        _ => Err(CommandError::skills(
            "skills_scope_invalid",
            "Skills scope is invalid.",
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scope_values_are_explicit() {
        assert_eq!(parse_scope("all").expect("whole"), SkillsScope::Whole);
        assert_eq!(parse_scope("upper").expect("first"), SkillsScope::First);
        assert!(parse_scope("../upper").is_err());
    }
}
