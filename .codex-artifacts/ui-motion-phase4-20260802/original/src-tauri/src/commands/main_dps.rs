use nte_dps_tool::{
    core::combat_details::CombatDetailFilter,
    core::live_capture::CaptureReplayKind,
    engine::model::AbyssHalf,
    platform::file_dialog::{
        OpenFileDialogOutcome, choose_json_open_path, choose_pcapng_open_path,
    },
    storage::i18n,
};
use tauri::{AppHandle, Emitter, Manager, State, WebviewWindow};

use crate::{
    contract::{
        CommandError,
        main_dps::{MainDpsActionResult, MainDpsResetResult, MainDpsSnapshot},
        main_dps_detail::{MainDpsDetailColumns, MainDpsDetailSnapshot},
    },
    state::{AppState, MainDpsDetailKind, MainDpsDetailRequest, SessionUndoError},
    windows::{combat_details, console, hud, island, main_dps},
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
    replace_current: bool,
    state: State<'_, AppState>,
    window: WebviewWindow,
) -> Result<MainDpsSnapshot, CommandError> {
    validate_main_or_detail_window(&window)?;
    if state.session_has_data() && !replace_current {
        return Err(confirmation_required());
    }
    state
        .request_capture_start(replace_current)
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
pub(crate) async fn reset_main_dps_session(
    confirmed: bool,
    app: AppHandle,
    state: State<'_, AppState>,
    window: WebviewWindow,
) -> Result<MainDpsResetResult, CommandError> {
    main_dps::validate_window(&window)?;
    let active = matches!(
        state.capture_phase(),
        nte_dps_tool::core::live_capture::LiveCapturePhase::Starting
            | nte_dps_tool::core::live_capture::LiveCapturePhase::Running
            | nte_dps_tool::core::live_capture::LiveCapturePhase::Stopping
    ) || state.replay_running();
    if active && !confirmed {
        return Err(confirmation_required());
    }
    let state = state.inner().clone();
    let worker_state = state.clone();
    let result = tauri::async_runtime::spawn_blocking(move || {
        let undo_token = if active {
            worker_state
                .stop_active_capture_and_wait(std::time::Duration::from_secs(5))
                .map_err(CommandError::from_core)?;
            worker_state.clear_session();
            None
        } else {
            worker_state.reset_session_with_undo()
        };
        worker_state.set_main_processing_paused(false);
        worker_state
            .set_main_selected_round_id(None)
            .expect("live main DPS round is always valid");
        Ok(MainDpsResetResult {
            snapshot: snapshot(&worker_state),
            undo_token,
        })
    })
    .await
    .map_err(|_| CommandError::main_dps("reset_failed", "Failed to reset the current session"))??;
    state.publish_island_notice(
        "success",
        if result.undo_token.is_some() {
            "Session reset · use Undo within 5 seconds"
        } else {
            "Stats reset"
        },
        Vec::new(),
        result.undo_token.clone(),
    );
    island::show_notice(&app, &state)?;
    Ok(result)
}

#[tauri::command]
pub(crate) fn undo_main_dps_reset(
    undo_token: String,
    app: AppHandle,
    state: State<'_, AppState>,
    window: WebviewWindow,
) -> Result<MainDpsSnapshot, CommandError> {
    main_dps::validate_window(&window)?;
    state
        .undo_session_reset(&undo_token)
        .map_err(|error| match error {
            SessionUndoError::Expired => {
                CommandError::main_dps("session_undo_expired", "The reset undo window has expired")
            }
            SessionUndoError::Busy | SessionUndoError::NewData => CommandError::main_dps(
                "session_undo_unavailable",
                "The previous session cannot be restored after new activity",
            ),
            SessionUndoError::Missing => CommandError::main_dps(
                "session_undo_missing",
                "The previous session is no longer available",
            ),
        })?;
    state.publish_island_notice("success", "Previous session restored", Vec::new(), None);
    island::show_notice(&app, state.inner())?;
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
pub(crate) fn set_main_dps_onboarding_step(
    step: usize,
    state: State<'_, AppState>,
    window: WebviewWindow,
) -> Result<MainDpsSnapshot, CommandError> {
    main_dps::validate_window(&window)?;
    if step > 3 {
        return Err(action_unavailable());
    }
    state
        .set_onboarding_progress(step, false)
        .map_err(|error| {
            log::error!("save main DPS onboarding step failed: {error}");
            CommandError::main_dps("config_save_failed", "Failed to save onboarding progress")
        })?;
    Ok(snapshot(state.inner()))
}

#[tauri::command]
pub(crate) fn finish_main_dps_onboarding(
    hud_preset: String,
    app: AppHandle,
    state: State<'_, AppState>,
    window: WebviewWindow,
) -> Result<MainDpsSnapshot, CommandError> {
    main_dps::validate_window(&window)?;
    let preset = match hud_preset.as_str() {
        "minimal" => crate::state::HudPreset::Minimal,
        "standard" => crate::state::HudPreset::Standard,
        "detailed" => crate::state::HudPreset::Detailed,
        _ => return Err(action_unavailable()),
    };
    state.finish_onboarding(preset).map_err(|error| {
        log::error!("finish main DPS onboarding failed: {error}");
        CommandError::main_dps("config_save_failed", "Failed to finish onboarding")
    })?;
    if let Some(hud_window) = app.get_webview_window(hud::HUD_WINDOW_LABEL) {
        let _ = hud::sync_content_height(&hud_window, state.inner());
    }
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
    replace_current: bool,
    state: State<'_, AppState>,
    window: WebviewWindow,
) -> Result<MainDpsActionResult, CommandError> {
    validate_main_detail_or_console_window(&window)?;
    if (state.session_has_data()
        || matches!(
            state.capture_phase(),
            nte_dps_tool::core::live_capture::LiveCapturePhase::Starting
                | nte_dps_tool::core::live_capture::LiveCapturePhase::Running
                | nte_dps_tool::core::live_capture::LiveCapturePhase::Stopping
        )
        || state.replay_running())
        && !replace_current
    {
        return Err(confirmation_required());
    }
    let reservation = state
        .begin_replay_import(replace_current)
        .map_err(CommandError::from_core)?;
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
                reservation
                    .start(kind, path, replace_current)
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
pub(crate) async fn import_main_dps_replay_path(
    path: String,
    replace_current: bool,
    state: State<'_, AppState>,
    window: WebviewWindow,
) -> Result<MainDpsActionResult, CommandError> {
    validate_main_detail_or_console_window(&window)?;
    let path = std::path::PathBuf::from(path);
    if !path.is_file() {
        return Err(CommandError::main_dps(
            "replay_file_invalid",
            "The dropped replay file is unavailable",
        ));
    }
    let kind = match path
        .extension()
        .and_then(|extension| extension.to_str())
        .map(str::to_ascii_lowercase)
        .as_deref()
    {
        Some("pcapng") => CaptureReplayKind::Pcapng,
        Some("json") => CaptureReplayKind::Json,
        _ => {
            return Err(CommandError::main_dps(
                "replay_file_unsupported",
                "Only .pcapng and .json replay files are supported",
            ));
        }
    };
    if (state.session_has_data()
        || matches!(
            state.capture_phase(),
            nte_dps_tool::core::live_capture::LiveCapturePhase::Starting
                | nte_dps_tool::core::live_capture::LiveCapturePhase::Running
                | nte_dps_tool::core::live_capture::LiveCapturePhase::Stopping
        )
        || state.replay_running())
        && !replace_current
    {
        return Err(confirmation_required());
    }
    let reservation = state
        .begin_replay_import(replace_current)
        .map_err(CommandError::from_core)?;
    let state = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        reservation
            .start(kind, path, replace_current)
            .map_err(CommandError::from_core)?;
        Ok(MainDpsActionResult {
            performed: true,
            snapshot: snapshot(&state),
        })
    })
    .await
    .map_err(|_| CommandError::main_dps("replay_import_failed", "Replay import did not complete"))?
}

#[tauri::command]
pub(crate) fn open_main_dps_console_shortcut(
    target: String,
    app: AppHandle,
    window: WebviewWindow,
) -> Result<(), CommandError> {
    main_dps::validate_window(&window)?;
    let console_window = app
        .get_webview_window(console::CONSOLE_WINDOW_LABEL)
        .ok_or_else(CommandError::window_operation_failed)?;
    match target.as_str() {
        "palette" => console_window
            .emit("console-command-palette-open", ())
            .map_err(window_error)?,
        "packets" => console_window
            .emit(console::CONSOLE_NAVIGATE_EVENT, "packets")
            .map_err(window_error)?,
        _ => return Err(action_unavailable()),
    }
    console_window.show().map_err(window_error)?;
    console_window.unminimize().map_err(window_error)?;
    console_window.set_focus().map_err(window_error)
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
    hud_window
        .emit(crate::windows::WINDOW_MOTION_ENTER_EVENT, ())
        .map_err(window_error)?;
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
    state.set_main_dps_detail_request(
        MainDpsDetailKind::Character,
        MainDpsDetailRequest {
            character_id: Some(character_id),
            filter: CombatDetailFilter::All,
            skill_filter: None,
        },
    );
    show_combat_details(&app, state.inner(), MainDpsDetailKind::Character)
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
    open_team_details_with_filter(&app, state.inner(), filter)
}

pub(crate) fn open_team_details(app: &AppHandle, state: &AppState) -> Result<(), CommandError> {
    open_team_details_with_filter(app, state, CombatDetailFilter::All)
}

fn open_team_details_with_filter(
    app: &AppHandle,
    state: &AppState,
    filter: CombatDetailFilter,
) -> Result<(), CommandError> {
    if !snapshot(state).actions.team_details_available {
        return Err(action_unavailable());
    }
    state.set_main_dps_detail_request(
        MainDpsDetailKind::Team,
        MainDpsDetailRequest {
            character_id: None,
            filter,
            skill_filter: None,
        },
    );
    show_combat_details(app, state, MainDpsDetailKind::Team)
}

#[tauri::command]
pub(crate) fn get_main_dps_detail_snapshot(
    offset: usize,
    limit: Option<usize>,
    state: State<'_, AppState>,
    window: WebviewWindow,
) -> Result<MainDpsDetailSnapshot, CommandError> {
    combat_details::validate_window(&window)?;
    let kind = combat_details::window_kind(&window)?;
    Ok(MainDpsDetailSnapshot::from_state(
        state.inner(),
        kind,
        offset,
        limit.unwrap_or(MAIN_DPS_DETAIL_DEFAULT_LIMIT),
    ))
}

#[tauri::command]
pub(crate) fn set_main_dps_detail_view(
    filter: String,
    qte_type: Option<String>,
    skill_filter: Option<String>,
    state: State<'_, AppState>,
    window: WebviewWindow,
) -> Result<MainDpsDetailSnapshot, CommandError> {
    combat_details::validate_window(&window)?;
    let kind = combat_details::window_kind(&window)?;
    let current = state.main_dps_detail_request(kind);
    let filter = if filter == "qteType" {
        let attack_type = qte_type
            .filter(|value| !value.trim().is_empty())
            .ok_or_else(action_unavailable)?;
        CombatDetailFilter::QteType(attack_type)
    } else {
        parse_detail_filter(&filter)?
    };
    state.set_main_dps_detail_request(
        kind,
        MainDpsDetailRequest {
            character_id: current.character_id,
            filter,
            skill_filter: skill_filter.filter(|value| !value.trim().is_empty()),
        },
    );
    Ok(MainDpsDetailSnapshot::from_state(
        state.inner(),
        kind,
        0,
        MAIN_DPS_DETAIL_DEFAULT_LIMIT,
    ))
}

#[tauri::command]
pub(crate) fn set_main_dps_detail_columns(
    columns: MainDpsDetailColumns,
    state: State<'_, AppState>,
    window: WebviewWindow,
) -> Result<MainDpsDetailSnapshot, CommandError> {
    combat_details::validate_window(&window)?;
    let kind = combat_details::window_kind(&window)?;
    state
        .set_hit_detail_columns(columns.into())
        .map_err(|error| {
            log::error!("save combat detail columns failed: {error}");
            CommandError::main_dps(
                "config_save_failed",
                "Failed to save the column preferences",
            )
        })?;
    Ok(MainDpsDetailSnapshot::from_state(
        state.inner(),
        kind,
        0,
        MAIN_DPS_DETAIL_DEFAULT_LIMIT,
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
    main_window.show().map_err(window_error)?;
    main_window
        .emit(crate::windows::WINDOW_MOTION_ENTER_EVENT, ())
        .map_err(window_error)?;
    main_window.unminimize().map_err(window_error)?;
    main_window.set_focus().map_err(window_error)?;
    window.hide().map_err(window_error)
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

fn show_combat_details(
    app: &AppHandle,
    state: &AppState,
    kind: MainDpsDetailKind,
) -> Result<(), CommandError> {
    let label = match kind {
        MainDpsDetailKind::Character => combat_details::CHARACTER_DETAILS_WINDOW_LABEL,
        MainDpsDetailKind::Team => combat_details::TEAM_DETAILS_WINDOW_LABEL,
    };
    let details = app
        .get_webview_window(label)
        .ok_or_else(CommandError::window_operation_failed)?;
    combat_details::show(&details, state, kind)
}

fn validate_main_or_detail_window(window: &WebviewWindow) -> Result<(), CommandError> {
    if main_dps::validate_window(window).is_ok() || combat_details::validate_window(window).is_ok()
    {
        Ok(())
    } else {
        Err(CommandError::invalid_window())
    }
}

fn validate_main_detail_or_console_window(window: &WebviewWindow) -> Result<(), CommandError> {
    if validate_main_or_detail_window(window).is_ok() || console::validate_window(window).is_ok() {
        Ok(())
    } else {
        Err(CommandError::invalid_window())
    }
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

fn confirmation_required() -> CommandError {
    CommandError::main_dps(
        "confirmation_required",
        "Confirm replacing the current session before continuing",
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
