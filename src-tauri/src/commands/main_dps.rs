use nte_dps_tool::{
    core::combat_details::CombatDetailFilter,
    core::live_capture::CaptureReplayKind,
    engine::model::{AbyssHalf, MAX_INDEXED_DETAIL_KEY_BYTES},
    storage::i18n,
};
use tauri::{AppHandle, Emitter, Manager, State, WebviewWindow};

use crate::{
    commands::{desktop_window, settings},
    contract::{
        CommandError,
        main_dps::{MainDpsActionResult, MainDpsResetResult, MainDpsSnapshot},
        main_dps_detail::{
            MAIN_DPS_DETAIL_DEFAULT_LIMIT, MainDpsDetailColumns, MainDpsDetailSnapshot,
        },
        update::UpdatePromptSnapshot,
    },
    file_dialog::{self, DialogOutcome},
    state::{
        AppState, DesktopWindowKind, MainDpsDetailKind, MainDpsDetailRequest, PresentationError,
        ReplayImportError, SessionUndoError,
    },
    windows::{combat_details, console, hud, island, main_dps},
};

#[tauri::command]
pub(crate) fn get_main_dps_snapshot(
    state: State<'_, AppState>,
    window: WebviewWindow,
) -> Result<MainDpsSnapshot, CommandError> {
    main_dps::validate_window(&window)?;
    snapshot(state.inner())
}

#[tauri::command]
pub(crate) fn get_main_dps_update_prompt(
    state: State<'_, AppState>,
    window: WebviewWindow,
) -> Result<UpdatePromptSnapshot, CommandError> {
    main_dps::validate_window(&window)?;
    Ok(UpdatePromptSnapshot::from_updates(
        state.update_settings_snapshot(),
    ))
}

#[tauri::command]
pub(crate) async fn download_main_dps_update(
    component: String,
    state: State<'_, AppState>,
    window: WebviewWindow,
) -> Result<UpdatePromptSnapshot, CommandError> {
    main_dps::validate_window(&window)?;
    let component = settings::parse_update_component(&component)?;
    let state = state.inner().clone();
    settings::run_update_download(state.clone(), component)
        .await
        .map_err(settings::update_action_error)?;
    Ok(UpdatePromptSnapshot::from_updates(
        state.update_settings_snapshot(),
    ))
}

#[tauri::command]
pub(crate) async fn install_main_dps_update(
    app: AppHandle,
    state: State<'_, AppState>,
    window: WebviewWindow,
) -> Result<UpdatePromptSnapshot, CommandError> {
    main_dps::validate_window(&window)?;
    let state = state.inner().clone();
    if let Some(message_key) = state
        .update_install_blocked_message_key()
        .map_err(settings::update_action_error)?
    {
        return Err(CommandError::update_install_blocked(message_key));
    }
    settings::install_update(app, state.clone())
        .await
        .map_err(settings::update_action_error)?;
    Ok(UpdatePromptSnapshot::from_updates(
        state.update_settings_snapshot(),
    ))
}

#[tauri::command]
pub(crate) fn start_main_dps_capture(
    replace_current: bool,
    app: AppHandle,
    state: State<'_, AppState>,
    window: WebviewWindow,
) -> Result<MainDpsSnapshot, CommandError> {
    validate_main_or_detail_window(&window)?;
    if state.session_has_data().map_err(CommandError::from_core)? && !replace_current {
        return Err(confirmation_required());
    }
    state
        .request_capture_start(replace_current)
        .map_err(CommandError::from_core)?;
    island::publish_notice(
        &app,
        state.inner(),
        "status",
        "Starting live capture...",
        Vec::new(),
    )?;
    snapshot(state.inner())
}

#[tauri::command]
pub(crate) fn stop_main_dps_capture(
    app: AppHandle,
    state: State<'_, AppState>,
    window: WebviewWindow,
) -> Result<MainDpsSnapshot, CommandError> {
    main_dps::validate_window(&window)?;
    state
        .request_capture_stop()
        .map_err(CommandError::from_core)?;
    island::publish_notice(
        &app,
        state.inner(),
        "status",
        "Stopping live capture...",
        Vec::new(),
    )?;
    snapshot(state.inner())
}

#[tauri::command]
pub(crate) async fn reset_main_dps_session(
    confirmed: bool,
    app: AppHandle,
    state: State<'_, AppState>,
    window: WebviewWindow,
) -> Result<MainDpsResetResult, CommandError> {
    main_dps::validate_window(&window)?;
    state
        .ensure_session_undo_runtime_available()
        .map_err(session_undo_error)?;
    let active = matches!(
        state.capture_phase(),
        nte_dps_tool::core::live_capture::LiveCapturePhase::Starting
            | nte_dps_tool::core::live_capture::LiveCapturePhase::Running
            | nte_dps_tool::core::live_capture::LiveCapturePhase::Stopping
    ) || state.replay_running().map_err(CommandError::from_core)?;
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
            worker_state
                .clear_session_action()
                .map_err(session_undo_error)?;
            None
        } else {
            worker_state
                .reset_session_with_undo_action()
                .map_err(session_undo_error)?
        };
        worker_state
            .set_main_processing_paused(false)
            .map_err(presentation_error)?;
        worker_state
            .set_main_selected_round_id(None)
            .map_err(presentation_selection_error)?;
        Ok(MainDpsResetResult {
            snapshot: snapshot(&worker_state)?,
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
        .map_err(session_undo_error)?;
    state.publish_island_notice("success", "Previous session restored", Vec::new(), None);
    island::show_notice(&app, state.inner())?;
    snapshot(state.inner())
}

#[tauri::command]
pub(crate) fn start_main_dps_new_round(
    app: AppHandle,
    state: State<'_, AppState>,
    window: WebviewWindow,
) -> Result<MainDpsSnapshot, CommandError> {
    main_dps::validate_window(&window)?;
    let current = snapshot(state.inner())?;
    if !current.actions.can_start_new_round {
        return Err(action_unavailable());
    }
    state.archive_current_history_round().map_err(|error| {
        log::error!("archive current main DPS round failed: {error}");
        CommandError::main_dps("round_archive_failed", "Failed to start a new combat round")
    })?;
    island::publish_notice(
        &app,
        state.inner(),
        "success",
        "New combat round started",
        Vec::new(),
    )?;
    snapshot(state.inner())
}

#[tauri::command]
pub(crate) fn set_main_dps_paused(
    paused: bool,
    app: AppHandle,
    state: State<'_, AppState>,
    window: WebviewWindow,
) -> Result<MainDpsSnapshot, CommandError> {
    main_dps::validate_window(&window)?;
    state
        .set_main_processing_paused(paused)
        .map_err(presentation_error)?;
    island::publish_notice(
        &app,
        state.inner(),
        "status",
        if paused {
            "Processing paused"
        } else {
            "Processing resumed"
        },
        Vec::new(),
    )?;
    snapshot(state.inner())
}

#[tauri::command]
pub(crate) async fn select_main_dps_round(
    record_id: Option<String>,
    state: State<'_, AppState>,
    window: WebviewWindow,
) -> Result<MainDpsSnapshot, CommandError> {
    main_dps::validate_window(&window)?;
    let state = state.inner().clone();
    // Reserve latest intent before worker scheduling. A previously submitted
    // slow worker that starts later therefore cannot obtain a newer token.
    let operation = state.reserve_main_round_selection();
    tauri::async_runtime::spawn_blocking(move || {
        state
            .set_main_selected_round_id_for_operation(record_id, operation)
            .map_err(presentation_selection_error)?;
        snapshot(&state)
    })
    .await
    .map_err(|_| {
        CommandError::main_dps(
            "history_selection_failed",
            "History selection did not finish.",
        )
    })?
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
    snapshot(state.inner())
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
    island::publish_notice(&app, state.inner(), "success", "Setup complete", Vec::new())?;
    snapshot(state.inner())
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
    state
        .set_main_selected_abyss_half(half)
        .map_err(presentation_error)?;
    snapshot(state.inner())
}

#[tauri::command]
pub(crate) async fn import_main_dps_replay(
    kind: String,
    replace_current: bool,
    app: AppHandle,
    state: State<'_, AppState>,
    window: WebviewWindow,
) -> Result<MainDpsActionResult, CommandError> {
    validate_main_detail_or_console_window(&window)?;
    if (state.session_has_data().map_err(CommandError::from_core)?
        || matches!(
            state.capture_phase(),
            nte_dps_tool::core::live_capture::LiveCapturePhase::Starting
                | nte_dps_tool::core::live_capture::LiveCapturePhase::Running
                | nte_dps_tool::core::live_capture::LiveCapturePhase::Stopping
        )
        || state.replay_running().map_err(CommandError::from_core)?)
        && !replace_current
    {
        return Err(confirmation_required());
    }
    let reservation = state
        .begin_replay_import(replace_current)
        .map_err(replay_import_error)?;
    let kind = match kind.as_str() {
        "pcapng" => CaptureReplayKind::Pcapng,
        "json" => CaptureReplayKind::Json,
        _ => return Err(action_unavailable()),
    };
    let title = i18n::t(match kind {
        CaptureReplayKind::Pcapng => "Wireshark capture",
        CaptureReplayKind::Json => "NTE exported capture",
    });
    let selection = match kind {
        CaptureReplayKind::Pcapng => file_dialog::choose_pcapng_open_path(&window, title).await,
        CaptureReplayKind::Json => file_dialog::choose_json_open_path(&window, title).await,
    }
    .map_err(|error| {
        log::error!("native main DPS replay dialog failed: {error}");
        file_dialog_error()
    })?;
    let state = state.inner().clone();
    let worker_state = state.clone();
    let result = tauri::async_runtime::spawn_blocking(move || match selection {
        DialogOutcome::Selected(path) => {
            reservation
                .start(kind, path, replace_current)
                .map_err(replay_import_error)?;
            Ok(MainDpsActionResult {
                performed: true,
                snapshot: snapshot(&worker_state)?,
            })
        }
        DialogOutcome::Cancelled => Ok(MainDpsActionResult {
            performed: false,
            snapshot: snapshot(&worker_state)?,
        }),
    })
    .await
    .map_err(|_| {
        CommandError::main_dps("replay_import_failed", "Replay import did not complete")
    })??;
    if result.performed {
        island::publish_notice(
            &app,
            &state,
            "status",
            "Importing and parsing capture",
            Vec::new(),
        )?;
    }
    Ok(result)
}

#[tauri::command]
pub(crate) async fn import_main_dps_replay_path(
    path: String,
    replace_current: bool,
    app: AppHandle,
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
    if (state.session_has_data().map_err(CommandError::from_core)?
        || matches!(
            state.capture_phase(),
            nte_dps_tool::core::live_capture::LiveCapturePhase::Starting
                | nte_dps_tool::core::live_capture::LiveCapturePhase::Running
                | nte_dps_tool::core::live_capture::LiveCapturePhase::Stopping
        )
        || state.replay_running().map_err(CommandError::from_core)?)
        && !replace_current
    {
        return Err(confirmation_required());
    }
    let reservation = state
        .begin_replay_import(replace_current)
        .map_err(replay_import_error)?;
    let state = state.inner().clone();
    let worker_state = state.clone();
    let result = tauri::async_runtime::spawn_blocking(move || {
        reservation
            .start(kind, path, replace_current)
            .map_err(replay_import_error)?;
        Ok(MainDpsActionResult {
            performed: true,
            snapshot: snapshot(&worker_state)?,
        })
    })
    .await
    .map_err(|_| {
        CommandError::main_dps("replay_import_failed", "Replay import did not complete")
    })??;
    island::publish_notice(
        &app,
        &state,
        "status",
        "Importing and parsing capture",
        Vec::new(),
    )?;
    Ok(result)
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
    desktop_window::set_window_always_on_top(
        &window,
        state.inner(),
        DesktopWindowKind::MainDps,
        enabled,
    )?;
    island::publish_notice(
        &app,
        state.inner(),
        "status",
        if enabled {
            "Always-on-top enabled"
        } else {
            "Always-on-top disabled"
        },
        Vec::new(),
    )?;
    snapshot(state.inner())
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
    snapshot(state.inner())
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
    app.emit_to(
        hud::HUD_WINDOW_LABEL,
        crate::windows::WINDOW_MOTION_ENTER_EVENT,
        hud::HUD_WINDOW_LABEL,
    )
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
    if !snapshot(state.inner())?
        .readout
        .characters
        .iter()
        .any(|row| row.character_id == character_id)
    {
        return Err(action_unavailable());
    }
    state
        .set_main_dps_detail_request(
            MainDpsDetailKind::Character,
            MainDpsDetailRequest {
                character_id: Some(character_id),
                filter: CombatDetailFilter::All,
                skill_filter: None,
            },
        )
        .map_err(presentation_error)?;
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
    if !snapshot(state)?.actions.team_details_available {
        return Err(action_unavailable());
    }
    state
        .set_main_dps_detail_request(
            MainDpsDetailKind::Team,
            MainDpsDetailRequest {
                character_id: None,
                filter,
                skill_filter: None,
            },
        )
        .map_err(presentation_error)?;
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
    MainDpsDetailSnapshot::from_state(
        state.inner(),
        kind,
        offset,
        limit.unwrap_or(MAIN_DPS_DETAIL_DEFAULT_LIMIT),
    )
    .map_err(CommandError::from_core)
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
    let qte_type = validate_optional_detail_key(qte_type)?;
    let skill_filter = validate_optional_detail_key(skill_filter)?;
    let filter = if filter == "qteType" {
        let attack_type = qte_type.ok_or_else(action_unavailable)?;
        CombatDetailFilter::QteType(attack_type)
    } else {
        parse_detail_filter(&filter)?
    };
    state
        .set_main_dps_detail_request(
            kind,
            MainDpsDetailRequest {
                character_id: current.character_id,
                filter,
                skill_filter,
            },
        )
        .map_err(presentation_error)?;
    MainDpsDetailSnapshot::from_state(state.inner(), kind, 0, MAIN_DPS_DETAIL_DEFAULT_LIMIT)
        .map_err(CommandError::from_core)
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
    MainDpsDetailSnapshot::from_state(state.inner(), kind, 0, MAIN_DPS_DETAIL_DEFAULT_LIMIT)
        .map_err(CommandError::from_core)
}

#[tauri::command]
pub(crate) fn set_main_dps_passthrough(
    enabled: bool,
    app: AppHandle,
    state: State<'_, AppState>,
    window: WebviewWindow,
) -> Result<MainDpsSnapshot, CommandError> {
    main_dps::validate_window(&window)?;
    main_dps::set_passthrough(&window, &state, enabled)?;
    let (message_key, message_arguments) = if enabled {
        (
            "Mouse passthrough on; press {} to turn off",
            vec![state.passthrough_hotkey().label().to_owned()],
        )
    } else {
        ("Mouse passthrough off", Vec::new())
    };
    island::publish_notice(
        &app,
        state.inner(),
        "status",
        message_key,
        message_arguments,
    )?;
    snapshot(state.inner())
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
    app.emit_to(
        main_dps::MAIN_DPS_WINDOW_LABEL,
        crate::windows::WINDOW_MOTION_ENTER_EVENT,
        main_dps::MAIN_DPS_WINDOW_LABEL,
    )
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

pub(crate) fn snapshot(state: &AppState) -> Result<MainDpsSnapshot, CommandError> {
    MainDpsSnapshot::from_state(state).map_err(CommandError::from_core)
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

fn validate_optional_detail_key(value: Option<String>) -> Result<Option<String>, CommandError> {
    let Some(value) = value else {
        return Ok(None);
    };
    let value = value.trim();
    if value.is_empty() {
        return Ok(None);
    }
    if value.len() > MAX_INDEXED_DETAIL_KEY_BYTES {
        return Err(CommandError::main_dps(
            "detail_filter_too_large",
            "The combat detail filter is too large",
        ));
    }
    Ok(Some(value.to_owned()))
}

fn action_unavailable() -> CommandError {
    CommandError::main_dps(
        "action_unavailable",
        "This action is not available right now",
    )
}

pub(crate) fn presentation_error(error: PresentationError) -> CommandError {
    match error {
        PresentationError::StateUnavailable => CommandError::main_dps(
            "presentation_state_unavailable",
            "Presentation state is unavailable",
        ),
        PresentationError::Capture(error) => CommandError::from_core(error),
        PresentationError::RoundUnavailable => {
            CommandError::main_dps("history_round_missing", "Combat round no longer exists")
        }
        PresentationError::RoundTooLarge => CommandError::main_dps(
            "history_round_too_large",
            "Combat round exceeds the interactive History budget",
        ),
    }
}

fn presentation_selection_error(error: PresentationError) -> CommandError {
    presentation_error(error)
}

fn replay_import_error(error: ReplayImportError) -> CommandError {
    match error {
        ReplayImportError::RuntimeUnavailable => CommandError::replay_import_runtime_unavailable(),
        ReplayImportError::Capture(error) => CommandError::from_core(error),
        ReplayImportError::JsonImport(error) => {
            CommandError::main_dps(error.stable_code(), "Failed to start capture replay")
        }
    }
}

pub(crate) fn session_undo_error(error: SessionUndoError) -> CommandError {
    match error {
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
        SessionUndoError::StateUnavailable => CommandError::main_dps(
            "capture_state_unavailable",
            "Live capture state is unavailable",
        ),
        SessionUndoError::RuntimeUnavailable => CommandError::session_undo_runtime_unavailable(),
    }
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

    #[test]
    fn detail_string_filters_enforce_utf8_byte_budget() {
        let exact = "x".repeat(MAX_INDEXED_DETAIL_KEY_BYTES);
        assert_eq!(
            validate_optional_detail_key(Some(format!(" {exact} ")))
                .expect("exact byte budget is valid")
                .as_deref(),
            Some(exact.as_str())
        );

        let oversized =
            validate_optional_detail_key(Some("x".repeat(MAX_INDEXED_DETAIL_KEY_BYTES + 1)))
                .expect_err("oversized detail key must fail");
        assert_eq!(oversized.code, "detail_filter_too_large");

        let multibyte = "界".repeat(MAX_INDEXED_DETAIL_KEY_BYTES / "界".len() + 1);
        assert!(multibyte.chars().count() < MAX_INDEXED_DETAIL_KEY_BYTES);
        let error = validate_optional_detail_key(Some(multibyte))
            .expect_err("UTF-8 byte length, not character count, is authoritative");
        assert_eq!(error.code, "detail_filter_too_large");
        assert_eq!(
            validate_optional_detail_key(Some("  ".to_owned())).unwrap(),
            None
        );
    }

    #[test]
    fn poisoned_replay_and_session_runtimes_use_stable_command_errors() {
        let replay = replay_import_error(ReplayImportError::RuntimeUnavailable);
        assert_eq!(replay.code, "replay_import_runtime_unavailable");
        assert_eq!(replay.message_key, "Replay import did not complete");

        let session = session_undo_error(SessionUndoError::RuntimeUnavailable);
        assert_eq!(session.code, "session_undo_runtime_unavailable");
        assert_eq!(
            session.message_key,
            "The previous session is no longer available"
        );
        let serialized =
            serde_json::to_string(&(replay, session)).expect("runtime command errors serialize");
        assert!(!serialized.contains("poison"));
        assert!(!serialized.contains("private"));
    }

    #[test]
    fn json_replay_boundary_error_keeps_its_stable_code() {
        let error = replay_import_error(ReplayImportError::JsonImport(
            nte_dps_tool::engine::capture::CaptureImportError::UnsupportedVersion {
                found: 2,
                expected: 1,
            },
        ));

        assert_eq!(error.code, "replay_version_unsupported");
        assert_eq!(error.message_key, "Failed to start capture replay");
        assert!(error.message_arguments.is_empty());
    }

    #[test]
    fn presentation_unavailable_uses_a_stable_redacted_command_error() {
        let error = presentation_error(PresentationError::StateUnavailable);

        assert_eq!(error.code, "presentation_state_unavailable");
        assert_eq!(error.message_key, "Presentation state is unavailable");
        assert!(error.message_arguments.is_empty());
        let serialized = serde_json::to_string(&error).expect("presentation error serializes");
        assert!(!serialized.contains("poison"));
        assert!(!serialized.contains("private"));
    }

    #[test]
    fn oversized_interactive_history_round_has_a_stable_typed_error() {
        let error = presentation_error(PresentationError::RoundTooLarge);

        assert_eq!(error.code, "history_round_too_large");
        assert_eq!(
            error.message_key,
            "Combat round exceeds the interactive History budget"
        );
        assert!(error.message_arguments.is_empty());
    }
}
