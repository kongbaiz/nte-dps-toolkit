use nte_dps_tool::{
    core::combat_details::CombatDetailFilter,
    core::live_capture::{CaptureReplayKind, LiveCapturePhase},
    engine::model::AbyssHalf,
    platform::file_dialog::{
        OpenFileDialogOutcome, choose_json_open_path, choose_pcapng_open_path,
    },
    storage::i18n,
};
use tauri::{AppHandle, Manager, State, WebviewWindow};

use crate::{
    contract::{
        CommandError,
        main_dps::{MainDpsActionResult, MainDpsSnapshot},
        main_dps_detail::MainDpsDetailSnapshot,
    },
    state::{AppState, MainDpsDetailRequest},
    windows::{combat_details, console, hud, main_dps},
};

const MAIN_DPS_DETAIL_DEFAULT_LIMIT: usize = 200;

#[tauri::command]
pub(crate) fn get_main_dps_snapshot(
    state: State<'_, AppState>,
    window: WebviewWindow,
) -> Result<MainDpsSnapshot, CommandError> {
    main_dps::validate_window(&window)?;
    Ok(snapshot(state.inner()))
}

#[tauri::command]
pub(crate) fn start_main_dps_capture(
    state: State<'_, AppState>,
    window: WebviewWindow,
) -> Result<MainDpsSnapshot, CommandError> {
    main_dps::validate_window(&window)?;
    state
        .request_capture_start()
        .map_err(CommandError::from_core)?;
    Ok(snapshot(state.inner()))
}

#[tauri::command]
pub(crate) fn stop_main_dps_capture(
    state: State<'_, AppState>,
    window: WebviewWindow,
) -> Result<MainDpsSnapshot, CommandError> {
    main_dps::validate_window(&window)?;
    state
        .request_capture_stop()
        .map_err(CommandError::from_core)?;
    Ok(snapshot(state.inner()))
}

#[tauri::command]
pub(crate) fn reset_main_dps_session(
    state: State<'_, AppState>,
    window: WebviewWindow,
) -> Result<MainDpsSnapshot, CommandError> {
    main_dps::validate_window(&window)?;
    state.reset_session();
    state.set_main_processing_paused(false);
    state
        .set_main_selected_round_id(None)
        .expect("live main DPS round is always valid");
    Ok(snapshot(state.inner()))
}

#[tauri::command]
pub(crate) fn start_main_dps_new_round(
    state: State<'_, AppState>,
    window: WebviewWindow,
) -> Result<MainDpsSnapshot, CommandError> {
    main_dps::validate_window(&window)?;
    let current = snapshot(state.inner());
    if !current.actions.can_start_new_round {
        return Err(action_unavailable());
    }
    state.archive_current_history_round().map_err(|error| {
        log::error!("archive current main DPS round failed: {error}");
        CommandError::main_dps("round_archive_failed", "Failed to start a new combat round")
    })?;
    Ok(snapshot(state.inner()))
}

#[tauri::command]
pub(crate) fn set_main_dps_paused(
    paused: bool,
    state: State<'_, AppState>,
    window: WebviewWindow,
) -> Result<MainDpsSnapshot, CommandError> {
    main_dps::validate_window(&window)?;
    state.set_main_processing_paused(paused);
    Ok(snapshot(state.inner()))
}

#[tauri::command]
pub(crate) fn select_main_dps_round(
    record_id: Option<String>,
    state: State<'_, AppState>,
    window: WebviewWindow,
) -> Result<MainDpsSnapshot, CommandError> {
    main_dps::validate_window(&window)?;
    state.set_main_selected_round_id(record_id).map_err(|_| {
        CommandError::main_dps("history_round_missing", "Combat round no longer exists")
    })?;
    Ok(snapshot(state.inner()))
}

#[tauri::command]
pub(crate) fn select_main_dps_abyss_half(
    half: String,
    state: State<'_, AppState>,
    window: WebviewWindow,
) -> Result<MainDpsSnapshot, CommandError> {
    main_dps::validate_window(&window)?;
    let half = match half.as_str() {
        "all" => None,
        "first" => Some(AbyssHalf::First),
        "second" => Some(AbyssHalf::Second),
        _ => return Err(action_unavailable()),
    };
    state.set_main_selected_abyss_half(half);
    Ok(snapshot(state.inner()))
}

#[tauri::command]
pub(crate) async fn import_main_dps_replay(
    kind: String,
    state: State<'_, AppState>,
    window: WebviewWindow,
) -> Result<MainDpsActionResult, CommandError> {
    main_dps::validate_window(&window)?;
    if matches!(
        state.capture_phase(),
        LiveCapturePhase::Starting | LiveCapturePhase::Running | LiveCapturePhase::Stopping
    ) || state.replay_running()
    {
        return Err(action_unavailable());
    }
    let kind = match kind.as_str() {
        "pcapng" => CaptureReplayKind::Pcapng,
        "json" => CaptureReplayKind::Json,
        _ => return Err(action_unavailable()),
    };
    let owner = window.hwnd().map_err(|_| file_dialog_error())?.0 as isize;
    let title = i18n::t(match kind {
        CaptureReplayKind::Pcapng => "Wireshark capture",
        CaptureReplayKind::Json => "NTE exported capture",
    });
    let state = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        let selection = match kind {
            CaptureReplayKind::Pcapng => choose_pcapng_open_path(owner, &title),
            CaptureReplayKind::Json => choose_json_open_path(owner, &title),
        };
        match selection {
            Ok(OpenFileDialogOutcome::Selected(path)) => {
                state
                    .request_diagnostics_replay(kind, path)
                    .map_err(CommandError::from_core)?;
                Ok(MainDpsActionResult {
                    performed: true,
                    snapshot: snapshot(&state),
                })
            }
            Ok(OpenFileDialogOutcome::Cancelled) => Ok(MainDpsActionResult {
                performed: false,
                snapshot: snapshot(&state),
            }),
            Err(code) => {
                log::error!("native main DPS replay dialog failed: {code:#010x}");
                Err(file_dialog_error())
            }
        }
    })
    .await
    .map_err(|_| CommandError::main_dps("replay_import_failed", "Replay import did not complete"))?
}

#[tauri::command]
pub(crate) fn set_main_dps_always_on_top(
    enabled: bool,
    app: AppHandle,
    state: State<'_, AppState>,
    window: WebviewWindow,
) -> Result<MainDpsSnapshot, CommandError> {
    main_dps::validate_window(&window)?;
    let _transaction = state.lock_always_on_top_transaction();
    let previous = state.always_on_top();
    let hud_window = app.get_webview_window(hud::HUD_WINDOW_LABEL);
    window.set_always_on_top(enabled).map_err(window_error)?;
    if let Some(hud_window) = hud_window.as_ref()
        && let Err(error) = hud_window.set_always_on_top(enabled)
    {
        let _ = window.set_always_on_top(previous);
        return Err(window_error(error));
    }
    if let Err(error) = state.set_always_on_top(enabled) {
        log::error!("save main DPS always-on-top failed: {error}");
        let _ = window.set_always_on_top(previous);
        if let Some(hud_window) = hud_window {
            let _ = hud_window.set_always_on_top(previous);
        }
        return Err(CommandError::main_dps(
            "config_save_failed",
            "Failed to save the window preference",
        ));
    }
    Ok(snapshot(state.inner()))
}

#[tauri::command]
pub(crate) fn set_main_dps_appearance(
    dark_mode: bool,
    opacity: f32,
    state: State<'_, AppState>,
    window: WebviewWindow,
) -> Result<MainDpsSnapshot, CommandError> {
    main_dps::validate_window(&window)?;
    let previous = state.ui_config_snapshot();
    let opacity = opacity.clamp(0.35, 1.0);
    main_dps::set_opacity(&window, opacity)?;
    if let Err(error) = state.update_main_appearance(dark_mode, opacity) {
        log::error!("save main DPS appearance failed: {error}");
        let _ = main_dps::set_opacity(&window, previous.opacity);
        return Err(CommandError::main_dps(
            "config_save_failed",
            "Failed to save the appearance preference",
        ));
    }
    Ok(snapshot(state.inner()))
}

#[tauri::command]
pub(crate) fn open_main_dps_console(
    app: AppHandle,
    window: WebviewWindow,
) -> Result<(), CommandError> {
    main_dps::validate_window(&window)?;
    let console_window = app
        .get_webview_window(console::CONSOLE_WINDOW_LABEL)
        .ok_or_else(CommandError::window_operation_failed)?;
    console_window.show().map_err(window_error)?;
    console_window.unminimize().map_err(window_error)?;
    console_window.set_focus().map_err(window_error)
}

#[tauri::command]
pub(crate) fn open_main_dps_hud(
    app: AppHandle,
    state: State<'_, AppState>,
    window: WebviewWindow,
) -> Result<(), CommandError> {
    main_dps::validate_window(&window)?;
    let hud_window = app
        .get_webview_window(hud::HUD_WINDOW_LABEL)
        .ok_or_else(CommandError::window_operation_failed)?;
    main_dps::set_passthrough(&window, &state, false)?;
    hud::set_passthrough(&hud_window, &state, false)?;
    hud_window.show().map_err(window_error)?;
    hud_window.unminimize().map_err(window_error)?;
    hud_window.set_focus().map_err(window_error)?;
    window.hide().map_err(window_error)?;
    Ok(())
}

#[tauri::command]
pub(crate) fn open_main_dps_character_details(
    character_id: u32,
    app: AppHandle,
    state: State<'_, AppState>,
    window: WebviewWindow,
) -> Result<(), CommandError> {
    main_dps::validate_window(&window)?;
    if !snapshot(state.inner())
        .readout
        .characters
        .iter()
        .any(|row| row.character_id == character_id)
    {
        return Err(action_unavailable());
    }
    state.set_main_dps_detail_request(MainDpsDetailRequest {
        character_id: Some(character_id),
        filter: CombatDetailFilter::All,
    });
    show_combat_details(&app)
}

#[tauri::command]
pub(crate) fn open_main_dps_team_details(
    filter: String,
    app: AppHandle,
    state: State<'_, AppState>,
    window: WebviewWindow,
) -> Result<(), CommandError> {
    main_dps::validate_window(&window)?;
    let filter = parse_detail_filter(&filter)?;
    if !snapshot(state.inner()).actions.team_details_available {
        return Err(action_unavailable());
    }
    state.set_main_dps_detail_request(MainDpsDetailRequest {
        character_id: None,
        filter,
    });
    show_combat_details(&app)
}

#[tauri::command]
pub(crate) fn get_main_dps_detail_snapshot(
    offset: usize,
    limit: Option<usize>,
    state: State<'_, AppState>,
    window: WebviewWindow,
) -> Result<MainDpsDetailSnapshot, CommandError> {
    combat_details::validate_window(&window)?;
    Ok(MainDpsDetailSnapshot::from_state(
        state.inner(),
        offset,
        limit.unwrap_or(MAIN_DPS_DETAIL_DEFAULT_LIMIT),
    ))
}

#[tauri::command]
pub(crate) fn set_main_dps_passthrough(
    enabled: bool,
    state: State<'_, AppState>,
    window: WebviewWindow,
) -> Result<MainDpsSnapshot, CommandError> {
    main_dps::validate_window(&window)?;
    main_dps::set_passthrough(&window, &state, enabled)?;
    Ok(snapshot(state.inner()))
}

#[tauri::command]
pub(crate) fn show_main_dps_from_hud(
    app: AppHandle,
    state: State<'_, AppState>,
    window: WebviewWindow,
) -> Result<(), CommandError> {
    hud::validate_window(&window)?;
    hud::set_passthrough(&window, &state, false)?;
    let main_window = app
        .get_webview_window(main_dps::MAIN_DPS_WINDOW_LABEL)
        .ok_or_else(CommandError::window_operation_failed)?;
    main_dps::set_passthrough(&main_window, &state, false)?;
    window.hide().map_err(window_error)?;
    main_window.show().map_err(window_error)?;
    main_window.unminimize().map_err(window_error)?;
    main_window.set_focus().map_err(window_error)
}

#[tauri::command]
pub(crate) fn minimize_main_dps_window(window: WebviewWindow) -> Result<(), CommandError> {
    main_dps::validate_window(&window)?;
    window.minimize().map_err(window_error)
}

#[tauri::command]
pub(crate) fn toggle_main_dps_maximized(window: WebviewWindow) -> Result<(), CommandError> {
    main_dps::validate_window(&window)?;
    if window.is_maximized().map_err(window_error)? {
        window.unmaximize().map_err(window_error)
    } else {
        window.maximize().map_err(window_error)
    }
}

#[tauri::command]
pub(crate) fn close_main_dps_window(window: WebviewWindow) -> Result<(), CommandError> {
    main_dps::validate_window(&window)?;
    window.close().map_err(window_error)
}

pub(crate) fn snapshot(state: &AppState) -> MainDpsSnapshot {
    MainDpsSnapshot::from_state(state)
}

fn show_combat_details(app: &AppHandle) -> Result<(), CommandError> {
    let details = app
        .get_webview_window(combat_details::COMBAT_DETAILS_WINDOW_LABEL)
        .ok_or_else(CommandError::window_operation_failed)?;
    combat_details::show(&details)
}

fn parse_detail_filter(value: &str) -> Result<CombatDetailFilter, CommandError> {
    match value {
        "all" => Ok(CombatDetailFilter::All),
        "outgoing" => Ok(CombatDetailFilter::Outgoing),
        "incoming" => Ok(CombatDetailFilter::Incoming),
        "characterAttributed" => Ok(CombatDetailFilter::CharacterAttributed),
        "characterDirect" => Ok(CombatDetailFilter::CharacterDirect),
        "reactionDamage" => Ok(CombatDetailFilter::ReactionDamage),
        "sharedMechanics" => Ok(CombatDetailFilter::SharedMechanics),
        "unattributed" => Ok(CombatDetailFilter::Unattributed),
        _ => Err(action_unavailable()),
    }
}

fn action_unavailable() -> CommandError {
    CommandError::main_dps(
        "action_unavailable",
        "This action is not available right now",
    )
}

fn file_dialog_error() -> CommandError {
    CommandError::main_dps("file_dialog_failed", "The native file dialog failed")
}

fn window_error(error: tauri::Error) -> CommandError {
    log::error!("main DPS window operation failed: {error}");
    CommandError::window_operation_failed()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detail_filter_ids_are_bounded_to_the_shared_contract() {
        assert_eq!(
            parse_detail_filter("characterDirect").expect("character filter"),
            CombatDetailFilter::CharacterDirect
        );
        assert_eq!(
            parse_detail_filter("sharedMechanics").expect("shared filter"),
            CombatDetailFilter::SharedMechanics
        );
        assert!(parse_detail_filter("../all").is_err());
        assert!(parse_detail_filter("qteType").is_err());
    }
}
