use tauri::{State, WebviewWindow};

use crate::{
    contract::CommandError,
    state::{AppState, DesktopWindowKind},
    windows::{abyss_values, combat_details, console, hud, main_dps},
};

#[tauri::command]
pub(crate) fn set_desktop_window_always_on_top(
    enabled: bool,
    state: State<'_, AppState>,
    window: WebviewWindow,
) -> Result<(), CommandError> {
    let kind = window_kind(window.label())?;
    set_window_always_on_top(&window, state.inner(), kind, enabled)
}

pub(crate) fn set_window_always_on_top(
    window: &WebviewWindow,
    state: &AppState,
    kind: DesktopWindowKind,
    enabled: bool,
) -> Result<(), CommandError> {
    let previous = window
        .is_always_on_top()
        .map_err(|_| CommandError::window_operation_failed())?;
    window
        .set_always_on_top(enabled)
        .map_err(|_| CommandError::window_operation_failed())?;
    if let Err(error) = state.set_window_always_on_top(kind, enabled) {
        log::error!("save per-window always-on-top preference failed: {error}");
        let _ = window.set_always_on_top(previous);
        return Err(CommandError::hud_config_save_failed());
    }
    Ok(())
}

fn window_kind(label: &str) -> Result<DesktopWindowKind, CommandError> {
    match label {
        main_dps::MAIN_DPS_WINDOW_LABEL => Ok(DesktopWindowKind::MainDps),
        hud::HUD_WINDOW_LABEL => Ok(DesktopWindowKind::Hud),
        console::CONSOLE_WINDOW_LABEL => Ok(DesktopWindowKind::Console),
        abyss_values::ABYSS_VALUES_WINDOW_LABEL => Ok(DesktopWindowKind::AbyssValues),
        combat_details::CHARACTER_DETAILS_WINDOW_LABEL => Ok(DesktopWindowKind::CharacterDetails),
        combat_details::TEAM_DETAILS_WINDOW_LABEL => Ok(DesktopWindowKind::TeamDetails),
        _ => Err(CommandError::window_operation_failed()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn always_on_top_targets_only_stable_desktop_window_labels() {
        assert_eq!(
            window_kind(console::CONSOLE_WINDOW_LABEL).expect("Console"),
            DesktopWindowKind::Console
        );
        assert_eq!(
            window_kind(combat_details::TEAM_DETAILS_WINDOW_LABEL).expect("team details"),
            DesktopWindowKind::TeamDetails
        );
        assert!(window_kind("notification-island").is_err());
        assert!(window_kind("../console").is_err());
    }
}
