use std::fs;

use nte_dps_tool::{
    core::live_capture::LiveCapturePhase, platform::file_dialog::open_directory,
    storage::paths::capture_log_dir,
};
use tauri::{AppHandle, Manager, State, WebviewWindow};

use crate::{
    commands::desktop_window,
    contract::CommandError,
    state::{AppState, DesktopWindowKind},
    windows::{console, hud, main_dps},
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ConsoleControlAction {
    ToggleCapture,
    ResetSession,
    ToggleHud,
    TogglePassthrough,
    ToggleProcessing,
    TogglePin,
    OpenTeamDetails,
    OpenCaptureLogs,
}

#[tauri::command]
pub(crate) fn execute_console_control(
    action: String,
    app: AppHandle,
    state: State<'_, AppState>,
    window: WebviewWindow,
) -> Result<(), CommandError> {
    console::validate_window(&window)?;
    match parse_action(&action)? {
        ConsoleControlAction::ToggleCapture => toggle_capture(&state),
        ConsoleControlAction::ResetSession => {
            if matches!(
                state.capture_phase(),
                LiveCapturePhase::Starting | LiveCapturePhase::Running | LiveCapturePhase::Stopping
            ) || state.replay_running()
            {
                return Err(CommandError::main_dps(
                    "confirmation_required",
                    "Confirm resetting the active capture from the main window",
                ));
            }
            let undo_token = state.reset_session_with_undo();
            state.set_main_processing_paused(false);
            state
                .set_main_selected_round_id(None)
                .expect("live main DPS round is always valid");
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
            crate::windows::island::show_notice(&app, &state)?;
            Ok(())
        }
        ConsoleControlAction::ToggleHud => toggle_hud(&app, &state),
        ConsoleControlAction::TogglePassthrough => toggle_passthrough(&app, &state),
        ConsoleControlAction::ToggleProcessing => {
            state.set_main_processing_paused(!state.main_processing_paused());
            Ok(())
        }
        ConsoleControlAction::TogglePin => toggle_pin(&state, &window),
        ConsoleControlAction::OpenTeamDetails => super::main_dps::open_team_details(&app, &state),
        ConsoleControlAction::OpenCaptureLogs => open_capture_logs(),
    }
}

fn parse_action(action: &str) -> Result<ConsoleControlAction, CommandError> {
    match action {
        "toggle-capture" => Ok(ConsoleControlAction::ToggleCapture),
        "reset-session" => Ok(ConsoleControlAction::ResetSession),
        "toggle-hud" => Ok(ConsoleControlAction::ToggleHud),
        "toggle-passthrough" => Ok(ConsoleControlAction::TogglePassthrough),
        "toggle-processing" => Ok(ConsoleControlAction::ToggleProcessing),
        "toggle-pin" => Ok(ConsoleControlAction::TogglePin),
        "open-team-details" => Ok(ConsoleControlAction::OpenTeamDetails),
        "open-capture-logs" => Ok(ConsoleControlAction::OpenCaptureLogs),
        _ => Err(CommandError::invalid_settings_input()),
    }
}

fn toggle_capture(state: &AppState) -> Result<(), CommandError> {
    match state.capture_phase() {
        LiveCapturePhase::Starting | LiveCapturePhase::Running => state
            .request_capture_stop()
            .map_err(CommandError::from_core),
        LiveCapturePhase::Stopping => Ok(()),
        LiveCapturePhase::Idle | LiveCapturePhase::Stopped | LiveCapturePhase::Failed => state
            .request_capture_start(false)
            .map_err(CommandError::from_core),
    }
}

fn toggle_hud(app: &AppHandle, state: &AppState) -> Result<(), CommandError> {
    let window = app
        .get_webview_window(hud::HUD_WINDOW_LABEL)
        .ok_or_else(CommandError::window_operation_failed)?;
    if window
        .is_visible()
        .map_err(|_| CommandError::window_operation_failed())?
    {
        window
            .hide()
            .map_err(|_| CommandError::window_operation_failed())
    } else {
        hud::set_passthrough(&window, state, false)?;
        window
            .show()
            .and_then(|_| window.unminimize())
            .map_err(|_| CommandError::window_operation_failed())
    }
}

fn toggle_passthrough(app: &AppHandle, state: &AppState) -> Result<(), CommandError> {
    let hud_window = app.get_webview_window(hud::HUD_WINDOW_LABEL);
    if let Some(window) = hud_window.as_ref()
        && window
            .is_visible()
            .map_err(|_| CommandError::window_operation_failed())?
    {
        return hud::toggle_passthrough(window, state);
    }
    let main_window = app
        .get_webview_window(main_dps::MAIN_DPS_WINDOW_LABEL)
        .ok_or_else(CommandError::window_operation_failed)?;
    main_dps::toggle_passthrough(&main_window, state)
}

fn toggle_pin(state: &AppState, console_window: &WebviewWindow) -> Result<(), CommandError> {
    let enabled = !console_window
        .is_always_on_top()
        .map_err(|_| CommandError::window_operation_failed())?;
    desktop_window::set_window_always_on_top(
        console_window,
        state,
        DesktopWindowKind::Console,
        enabled,
    )
}

fn open_capture_logs() -> Result<(), CommandError> {
    let path = capture_log_dir();
    fs::create_dir_all(&path).map_err(|error| {
        log::error!("create capture log directory failed: {error}");
        CommandError::window_operation_failed()
    })?;
    open_directory(&path).map_err(|error| {
        log::error!("open capture log directory failed: {error}");
        CommandError::window_operation_failed()
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn console_control_actions_are_bounded_to_stable_ids() {
        assert_eq!(
            parse_action("toggle-capture").expect("capture action"),
            ConsoleControlAction::ToggleCapture
        );
        assert_eq!(
            parse_action("open-capture-logs").expect("logs action"),
            ConsoleControlAction::OpenCaptureLogs
        );
        assert!(parse_action("../toggle-capture").is_err());
    }
}
