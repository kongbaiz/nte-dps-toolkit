use std::{
    fs::File,
    future::Future,
    io::Read,
    path::Path,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};

use nte_dps_tool::{
    core::{
        team_data::{MAX_TEAM_DPS_EXPORT_BYTES, parse_team_data},
        update::{
            AvailableComponentUpdate, MAX_MANIFEST_BYTES, UpdateComponent, UpdateEndpoint,
            UpdateError, installed_app_version, verify_manifest,
        },
    },
    platform::update_http,
    storage::{
        config::{
            GlobalHotkeyAction, HotkeyBinding, HotkeyKey, HudModule, MainDpsDisplayConfig,
            UiDensity,
        },
        i18n,
        io_util::atomic_write_text,
        update::{self as update_storage, PreparedUpdate, installed_component_versions},
    },
};
use serde::Deserialize;
use tauri::{AppHandle, Emitter, Manager, State, WebviewWindow};
use ts_rs::TS;

use crate::{
    contract::{
        CommandError,
        settings::{
            CaptureSettingsInput, HotkeyBindingSnapshot, InterfaceSettingsInput, SettingsSnapshot,
            TeamDataExportResult, TeamDataImportFileResult, UpdateSettingsInput,
        },
    },
    file_dialog::{self, DialogOutcome},
    settings_service::SettingsServiceError,
    state::{AppState, HudPreset, HudSettingOption, UpdateActionError},
    windows::{abyss_values, combat_details, console, hud, island, main_dps},
};

#[derive(Clone, Copy, Debug, Deserialize, TS)]
#[ts(export)]
#[serde(rename_all = "snake_case")]
pub(crate) enum LayoutProfile {
    Combat,
    Review,
    Research,
}

const UPDATE_PROGRESS_INTERVAL: Duration = Duration::from_millis(100);

#[tauri::command]
pub(crate) fn set_settings_interface(
    settings: InterfaceSettingsInput,
    app: AppHandle,
    state: State<'_, AppState>,
    window: WebviewWindow,
) -> Result<SettingsSnapshot, CommandError> {
    console::validate_window(&window)?;
    if !settings.island_offset_x.is_finite() {
        return Err(CommandError::invalid_settings_input());
    }
    let language = settings.language;
    state
        .update_interface_settings(
            language,
            settings.dark_mode,
            settings.theme_preset,
            settings.accent,
            settings.density,
            settings.reduce_motion,
            settings.island_notifications,
            settings.island_offset_x,
        )
        .map_err(settings_save_error)?;
    nte_dps_tool::storage::i18n::set_language(language);
    if let Some(error) = nte_dps_tool::storage::ability_names::reload(language) {
        log::warn!("reload localized ability names after language change failed: {error}");
    }
    if let Some(console_window) = app.get_webview_window(console::CONSOLE_WINDOW_LABEL)
        && let Err(error) = console_window.set_title(&nte_dps_tool::storage::i18n::t("NTE Console"))
    {
        log::warn!("refresh Console title after language change failed: {error}");
    }
    if let Some(island_window) = app.get_webview_window(island::ISLAND_WINDOW_LABEL) {
        if settings.island_notifications && state.island_notice().is_some() {
            island::show_notice(&app, state.inner())?;
        } else if !settings.island_notifications {
            island_window
                .hide()
                .map_err(|_| CommandError::window_operation_failed())?;
        }
    }
    Ok(state.settings_snapshot())
}

#[tauri::command]
pub(crate) fn set_settings_update_preferences(
    settings: UpdateSettingsInput,
    state: State<'_, AppState>,
    window: WebviewWindow,
) -> Result<SettingsSnapshot, CommandError> {
    console::validate_window(&window)?;
    state
        .update_update_settings(settings.auto_check, settings.auto_download)
        .map_err(settings_save_error)?;
    Ok(state.settings_snapshot())
}

#[tauri::command]
pub(crate) async fn check_settings_updates(
    state: State<'_, AppState>,
    window: WebviewWindow,
) -> Result<SettingsSnapshot, CommandError> {
    console::validate_window(&window)?;
    let state = state.inner().clone();
    run_update_check(state.clone()).await?;
    Ok(state.settings_snapshot())
}

#[tauri::command]
pub(crate) async fn download_settings_update(
    component: UpdateComponent,
    state: State<'_, AppState>,
    window: WebviewWindow,
) -> Result<SettingsSnapshot, CommandError> {
    console::validate_window(&window)?;
    let component = app_update_component(component)?;
    let state = state.inner().clone();
    run_update_download(state.clone(), component).await?;
    Ok(state.settings_snapshot())
}

#[tauri::command]
pub(crate) async fn install_settings_update(
    app: AppHandle,
    state: State<'_, AppState>,
    window: WebviewWindow,
) -> Result<SettingsSnapshot, CommandError> {
    console::validate_window(&window)?;
    let state = state.inner().clone();
    if let Some(message_key) = state.update_install_blocked_message_key()? {
        return Err(CommandError::update_install_blocked(message_key));
    }
    install_update(app, state.clone()).await?;
    Ok(state.settings_snapshot())
}

pub(crate) async fn install_update(
    app: AppHandle,
    state: AppState,
) -> Result<(), UpdateActionError> {
    install_update_with(state, move |prepared, state| async move {
        let version = prepared.version().to_string();
        match prepared.component() {
            UpdateComponent::App => match update_storage::launch_prepared_app_update(&prepared) {
                Ok(_) => {
                    std::thread::spawn(move || {
                        std::thread::sleep(Duration::from_millis(200));
                        app.exit(0);
                    });
                }
                Err(error) => {
                    log::error!("launch prepared application update failed: {error}");
                    state.fail_update_install()?;
                }
            },
            UpdateComponent::ModsPlugin => {
                let result = tauri::async_runtime::spawn_blocking(move || {
                    update_storage::install_prepared_plugin_update(&prepared)
                })
                .await;
                match result {
                    Ok(Ok(())) => state.finish_plugin_update_install(version)?,
                    Ok(Err(error)) => {
                        log::error!("install prepared Mod loader update failed: {error}");
                        state.fail_update_install()?;
                    }
                    Err(error) => {
                        log::error!("Mod loader update worker failed: {error}");
                        state.fail_update_install()?;
                    }
                }
            }
        }
        Ok(())
    })
    .await
}

async fn install_update_with<Start, Worker>(
    state: AppState,
    start: Start,
) -> Result<(), UpdateActionError>
where
    Start: FnOnce(PreparedUpdate, AppState) -> Worker,
    Worker: Future<Output = Result<(), UpdateActionError>>,
{
    let prepared = state.begin_update_install()?;
    start(prepared, state).await
}

pub(crate) fn schedule_automatic_update_check(app: AppHandle, state: AppState) {
    if !state.auto_check_updates() {
        return;
    }
    tauri::async_runtime::spawn(async move {
        if let Err(error) = run_update_check(state.clone()).await {
            log::debug!("automatic update check skipped: {error:?}");
            return;
        }
        if !state.update_settings_snapshot().available.is_empty()
            && let Err(error) = app.emit_to(
                crate::windows::main_dps::MAIN_DPS_WINDOW_LABEL,
                crate::windows::main_dps::UPDATE_AVAILABLE_EVENT,
                (),
            )
        {
            log::debug!("notify main DPS about an available update failed: {error}");
        }
    });
}

pub(crate) fn schedule_completed_update_cleanup() {
    update_storage::cleanup_completed_update_staging();
    match update_storage::mark_update_healthy_from_environment() {
        Ok(Some(marker)) => {
            std::thread::spawn(move || update_storage::cleanup_completed_app_update(marker));
        }
        Ok(None) => {}
        Err(error) => log::error!("record application update health failed: {error}"),
    }
}

async fn run_update_check(state: AppState) -> Result<(), UpdateActionError> {
    run_update_check_with(state, || async {
        match tauri::async_runtime::spawn_blocking(check_for_updates).await {
            Ok(result) => result,
            Err(error) => {
                log::error!("Settings update worker failed: {error}");
                Err(CheckSettingsUpdateError::WorkerFailed)
            }
        }
    })
    .await
}

async fn run_update_check_with<Start, Worker>(
    state: AppState,
    start: Start,
) -> Result<(), UpdateActionError>
where
    Start: FnOnce() -> Worker,
    Worker: Future<Output = Result<Vec<AvailableComponentUpdate>, CheckSettingsUpdateError>>,
{
    state.begin_update_check()?;
    match start().await {
        Ok(updates) => {
            let auto_download = state
                .auto_download_updates()
                .then(|| updates.first().map(|update| update.component))
                .flatten();
            state.finish_update_check(updates)?;
            if let Some(component) = auto_download {
                run_update_download(state, component).await?;
            }
        }
        Err(CheckSettingsUpdateError::NotConfigured) => {
            state
                .fail_update_check("The official update channel is not configured in this build")?;
        }
        Err(CheckSettingsUpdateError::Failed(error)) => {
            log::error!("Settings update check failed: {error}");
            state.fail_update_check("Update check failed.")?;
        }
        Err(CheckSettingsUpdateError::WorkerFailed) => {
            state.fail_update_check("Update check failed.")?;
        }
    }
    Ok(())
}

pub(crate) async fn run_update_download(
    state: AppState,
    component: UpdateComponent,
) -> Result<(), UpdateActionError> {
    run_update_download_with(state, component, |update, progress_state| async move {
        let runtime_unavailable = Arc::new(AtomicBool::new(false));
        let progress_runtime_unavailable = Arc::clone(&runtime_unavailable);
        let result = tauri::async_runtime::spawn_blocking(move || {
            let mut last_progress = Instant::now() - UPDATE_PROGRESS_INTERVAL;
            update_storage::prepare_update(&update, |downloaded, total| {
                let now = Instant::now();
                if downloaded == total
                    || now.saturating_duration_since(last_progress) >= UPDATE_PROGRESS_INTERVAL
                {
                    last_progress = now;
                    if matches!(
                        progress_state.update_download_progress(component, downloaded, total),
                        Err(UpdateActionError::RuntimeUnavailable)
                    ) {
                        progress_runtime_unavailable.store(true, Ordering::Release);
                    }
                }
            })
        })
        .await;
        if runtime_unavailable.load(Ordering::Acquire) {
            return Err(UpdateDownloadWorkerError::RuntimeUnavailable);
        }
        match result {
            Ok(Ok(prepared)) => Ok(prepared),
            Ok(Err(error)) => {
                log::error!("prepare signed update failed: {error}");
                Err(UpdateDownloadWorkerError::Failed)
            }
            Err(error) => {
                log::error!("update download worker failed: {error}");
                Err(UpdateDownloadWorkerError::Failed)
            }
        }
    })
    .await
}

async fn run_update_download_with<Start, Worker>(
    state: AppState,
    component: UpdateComponent,
    start: Start,
) -> Result<(), UpdateActionError>
where
    Start: FnOnce(AvailableComponentUpdate, AppState) -> Worker,
    Worker: Future<Output = Result<PreparedUpdate, UpdateDownloadWorkerError>>,
{
    let update = state.begin_update_download(component)?;
    match start(update, state.clone()).await {
        Ok(prepared) => state.finish_update_download(prepared)?,
        Err(UpdateDownloadWorkerError::Failed) => {
            state.fail_update_download()?;
        }
        Err(UpdateDownloadWorkerError::RuntimeUnavailable) => {
            return Err(UpdateActionError::RuntimeUnavailable);
        }
    }
    Ok(())
}

#[tauri::command]
pub(crate) fn set_settings_capture(
    settings: CaptureSettingsInput,
    state: State<'_, AppState>,
    window: WebviewWindow,
) -> Result<SettingsSnapshot, CommandError> {
    console::validate_window(&window)?;
    let filter = validate_filter(settings.bpf_filter)?;
    let manual_device = settings
        .manual_capture_device
        .map(|device| device.trim().to_owned())
        .filter(|device| !device.is_empty());
    if manual_device
        .as_ref()
        .is_some_and(|device| device.len() > 512)
    {
        return Err(CommandError::invalid_settings_input());
    }
    let passthrough_hotkey = parse_hotkey_binding(settings.passthrough_hotkey)?;
    if passthrough_hotkey.is_reserved()
        || hotkey_conflicts(
            passthrough_hotkey,
            state.passthrough_hotkey(),
            state.global_hotkeys(),
            None,
        )
    {
        return Err(if passthrough_hotkey.is_reserved() {
            CommandError::invalid_settings_input()
        } else {
            CommandError::hotkey_conflict()
        });
    }
    state
        .update_capture_settings(
            filter,
            manual_device,
            settings.server_damage_calibration,
            settings.include_max_hp_reduction_in_total_damage,
            settings.separate_reaction_damage,
            settings.auto_round_after_idle,
            settings.auto_round_idle_seconds,
            passthrough_hotkey,
        )
        .map_err(settings_save_error)?;
    refresh_hotkey_configuration(&state);
    Ok(state.settings_snapshot())
}

#[tauri::command]
pub(crate) fn refresh_settings_capture_devices(
    state: State<'_, AppState>,
    window: WebviewWindow,
) -> Result<SettingsSnapshot, CommandError> {
    console::validate_window(&window)?;
    state.refresh_capture_devices().map_err(|_| {
        CommandError::new(
            "capture_devices_unavailable",
            "Capture devices are unavailable.",
        )
    })?;
    Ok(state.settings_snapshot())
}

#[tauri::command]
pub(crate) fn set_settings_hotkeys_enabled(
    enabled: bool,
    state: State<'_, AppState>,
    window: WebviewWindow,
) -> Result<SettingsSnapshot, CommandError> {
    console::validate_window(&window)?;
    let mut hotkeys = state.global_hotkeys();
    hotkeys.enabled = enabled;
    state
        .update_global_hotkeys(hotkeys)
        .map_err(settings_save_error)?;
    refresh_hotkey_configuration(&state);
    Ok(state.settings_snapshot())
}

#[tauri::command]
pub(crate) fn set_settings_hotkey_binding(
    action: GlobalHotkeyAction,
    binding: Option<HotkeyBindingSnapshot>,
    state: State<'_, AppState>,
    window: WebviewWindow,
) -> Result<SettingsSnapshot, CommandError> {
    console::validate_window(&window)?;
    let binding = binding.map(parse_hotkey_binding).transpose()?;
    if binding.is_some_and(HotkeyBinding::is_reserved) {
        return Err(CommandError::invalid_settings_input());
    }
    let mut hotkeys = state.global_hotkeys();
    if binding.is_some_and(|binding| {
        hotkey_conflicts(binding, state.passthrough_hotkey(), hotkeys, Some(action))
    }) {
        return Err(CommandError::hotkey_conflict());
    }
    hotkeys.set_binding(action, binding);
    state
        .update_global_hotkeys(hotkeys)
        .map_err(settings_save_error)?;
    refresh_hotkey_configuration(&state);
    Ok(state.settings_snapshot())
}

#[tauri::command]
pub(crate) fn set_settings_main_dps_display(
    settings: MainDpsDisplayConfig,
    state: State<'_, AppState>,
    window: WebviewWindow,
) -> Result<SettingsSnapshot, CommandError> {
    console::validate_window(&window)?;
    state
        .update_main_dps_display(settings)
        .map_err(settings_save_error)?;
    Ok(state.settings_snapshot())
}

#[tauri::command]
pub(crate) fn apply_settings_layout_profile(
    profile: LayoutProfile,
    app: AppHandle,
    state: State<'_, AppState>,
    window: WebviewWindow,
) -> Result<SettingsSnapshot, CommandError> {
    console::validate_window(&window)?;
    let hud_window = hud_window(&app)?;
    let abyss_window = app.get_webview_window(abyss_values::ABYSS_VALUES_WINDOW_LABEL);
    let detail_windows = [
        app.get_webview_window(combat_details::CHARACTER_DETAILS_WINDOW_LABEL),
        app.get_webview_window(combat_details::TEAM_DETAILS_WINDOW_LABEL),
    ];
    match profile {
        LayoutProfile::Combat => {
            if !state.passthrough_hotkey_ready() {
                return Err(CommandError::passthrough_hotkey_unavailable());
            }
            state
                .set_density(UiDensity::Compact)
                .map_err(settings_save_error)?;
            state
                .apply_hud_preset(HudPreset::Minimal)
                .map_err(settings_save_error)?;
            hud_window.show().map_err(window_operation_error)?;
            hud::set_passthrough(&hud_window, &state, true)?;
            window.hide().map_err(window_operation_error)?;
            if let Some(window) = abyss_window.as_ref() {
                window.hide().map_err(window_operation_error)?;
            }
            for detail_window in detail_windows.iter().flatten() {
                detail_window.hide().map_err(window_operation_error)?;
            }
        }
        LayoutProfile::Review => {
            state
                .set_density(UiDensity::Cozy)
                .map_err(settings_save_error)?;
            hud::set_passthrough(&hud_window, &state, false)?;
            hud_window.hide().map_err(window_operation_error)?;
            window.set_focus().map_err(window_operation_error)?;
            window
                .emit(console::CONSOLE_NAVIGATE_EVENT, "timeline")
                .map_err(window_operation_error)?;
            if let Some(window) = abyss_window.as_ref() {
                window.hide().map_err(window_operation_error)?;
            }
            for detail_window in detail_windows.iter().flatten() {
                detail_window.hide().map_err(window_operation_error)?;
            }
        }
        LayoutProfile::Research => {
            state
                .set_density(UiDensity::Compact)
                .map_err(settings_save_error)?;
            hud::set_passthrough(&hud_window, &state, false)?;
            hud_window.hide().map_err(window_operation_error)?;
            window.set_focus().map_err(window_operation_error)?;
            window
                .emit(console::CONSOLE_NAVIGATE_EVENT, "packets")
                .map_err(window_operation_error)?;
            if let Some(window) = abyss_window.as_ref() {
                window.hide().map_err(window_operation_error)?;
            }
            for detail_window in detail_windows.iter().flatten() {
                detail_window.hide().map_err(window_operation_error)?;
            }
            if super::main_dps::snapshot(state.inner())?
                .actions
                .team_details_available
            {
                super::main_dps::open_team_details(&app, state.inner())?;
            }
        }
    }
    sync_hud_height(&app, &state);
    Ok(state.settings_snapshot())
}

#[tauri::command]
pub(crate) fn import_settings_team_data(
    json: String,
    state: State<'_, AppState>,
    window: WebviewWindow,
) -> Result<SettingsSnapshot, CommandError> {
    console::validate_window(&window)?;
    let export = parse_team_data(&json).map_err(|detail| {
        log::warn!("reject imported team DPS data: {detail}");
        CommandError::team_data_invalid()
    })?;
    state.import_team_data(export)?;
    Ok(state.settings_snapshot())
}

#[tauri::command]
pub(crate) async fn import_settings_team_data_file(
    state: State<'_, AppState>,
    window: WebviewWindow,
) -> Result<TeamDataImportFileResult, CommandError> {
    console::validate_window(&window)?;

    #[cfg(windows)]
    {
        let title = i18n::t("Import DPS Data");
        let selection = file_dialog::choose_json_open_path(&window, title)
            .await
            .map_err(|error| {
                log::error!("native team DPS import dialog failed: {error}");
                CommandError::team_data_file_dialog_failed()
            })?;
        let json = match selection {
            DialogOutcome::Selected(path) => {
                tauri::async_runtime::spawn_blocking(move || read_team_data_file(&path).map(Some))
                    .await
                    .map_err(|_| CommandError::team_data_import_failed())??
            }
            DialogOutcome::Cancelled => None,
        };

        let performed = if let Some(json) = json {
            let export = parse_team_data(&json).map_err(|detail| {
                log::warn!("reject imported team DPS data: {detail}");
                CommandError::team_data_invalid()
            })?;
            state.import_team_data(export)?;
            true
        } else {
            false
        };
        Ok(TeamDataImportFileResult {
            performed,
            settings: state.settings_snapshot(),
        })
    }

    #[cfg(not(windows))]
    {
        let _ = state;
        Err(CommandError::team_data_file_dialog_failed())
    }
}

#[tauri::command]
pub(crate) async fn export_settings_team_data(
    state: State<'_, AppState>,
    window: WebviewWindow,
) -> Result<TeamDataExportResult, CommandError> {
    console::validate_window(&window)?;
    let export = state
        .export_team_data()?
        .ok_or_else(CommandError::team_data_unavailable)?;
    let json = serialize_team_data(&export)?;

    #[cfg(windows)]
    {
        let title = i18n::t("Export Team Data");
        let selection =
            file_dialog::choose_json_save_path(&window, title, "nte-team-dps.json".to_owned())
                .await
                .map_err(|error| {
                    log::error!("native team DPS export dialog failed: {error}");
                    CommandError::team_data_export_failed()
                })?;
        let saved = match selection {
            DialogOutcome::Selected(path) => tauri::async_runtime::spawn_blocking(move || {
                atomic_write_text(&path, &json).map(|_| true).map_err(|_| {
                    log::error!("team DPS export file write failed");
                    CommandError::team_data_export_failed()
                })
            })
            .await
            .map_err(|_| CommandError::team_data_export_failed())??,
            DialogOutcome::Cancelled => false,
        };
        Ok(TeamDataExportResult { saved })
    }

    #[cfg(not(windows))]
    {
        let _ = json;
        Err(CommandError::team_data_export_failed())
    }
}

#[tauri::command]
pub(crate) fn refresh_settings_capture_files(
    state: State<'_, AppState>,
    window: WebviewWindow,
) -> Result<SettingsSnapshot, CommandError> {
    console::validate_window(&window)?;
    state.refresh_capture_file_stats();
    Ok(state.settings_snapshot())
}

#[tauri::command]
pub(crate) fn clear_settings_capture_files(
    state: State<'_, AppState>,
    window: WebviewWindow,
) -> Result<SettingsSnapshot, CommandError> {
    console::validate_window(&window)?;
    let outcome = state.clear_capture_files();
    state.refresh_capture_file_stats();
    if outcome.failed > 0 {
        log::warn!(
            "capture log cleanup skipped {} locked or unreadable file(s)",
            outcome.failed
        );
    }
    Ok(state.settings_snapshot())
}

#[tauri::command]
pub(crate) fn open_settings_abyss_values(
    app: AppHandle,
    state: State<'_, AppState>,
    window: WebviewWindow,
) -> Result<SettingsSnapshot, CommandError> {
    console::validate_window(&window)?;
    let abyss_window = app
        .get_webview_window(abyss_values::ABYSS_VALUES_WINDOW_LABEL)
        .ok_or_else(CommandError::window_operation_failed)?;
    abyss_values::show(&abyss_window, state.inner())?;
    Ok(state.settings_snapshot())
}

#[tauri::command]
pub(crate) fn get_settings_snapshot(
    state: State<'_, AppState>,
    window: WebviewWindow,
) -> Result<SettingsSnapshot, CommandError> {
    console::validate_window(&window)?;
    Ok(state.settings_snapshot())
}

#[tauri::command]
pub(crate) fn set_settings_hud_option(
    option: HudSettingOption,
    enabled: bool,
    app: AppHandle,
    state: State<'_, AppState>,
    window: WebviewWindow,
) -> Result<SettingsSnapshot, CommandError> {
    console::validate_window(&window)?;
    state
        .set_hud_option(option, enabled)
        .map_err(config_save_error)?;
    sync_hud_height(&app, &state);
    Ok(state.settings_snapshot())
}

#[tauri::command]
pub(crate) fn apply_settings_hud_preset(
    preset: HudPreset,
    app: AppHandle,
    state: State<'_, AppState>,
    window: WebviewWindow,
) -> Result<SettingsSnapshot, CommandError> {
    console::validate_window(&window)?;
    state.apply_hud_preset(preset).map_err(config_save_error)?;
    sync_hud_height(&app, &state);
    Ok(state.settings_snapshot())
}

#[tauri::command]
pub(crate) fn set_settings_hud_module_visibility(
    module: HudModule,
    visible: bool,
    app: AppHandle,
    state: State<'_, AppState>,
    window: WebviewWindow,
) -> Result<SettingsSnapshot, CommandError> {
    console::validate_window(&window)?;
    state
        .set_hud_module_visibility(module, visible)
        .map_err(config_save_error)?;
    sync_hud_height(&app, &state);
    Ok(state.settings_snapshot())
}

#[tauri::command]
pub(crate) fn move_settings_hud_module(
    dragged: HudModule,
    target: HudModule,
    insert_after: bool,
    app: AppHandle,
    state: State<'_, AppState>,
    window: WebviewWindow,
) -> Result<SettingsSnapshot, CommandError> {
    console::validate_window(&window)?;
    state
        .move_hud_module(dragged, target, insert_after)
        .map_err(config_save_error)?;
    sync_hud_height(&app, &state);
    Ok(state.settings_snapshot())
}

#[tauri::command]
pub(crate) fn set_settings_hud_width(
    width: u16,
    app: AppHandle,
    state: State<'_, AppState>,
    window: WebviewWindow,
) -> Result<SettingsSnapshot, CommandError> {
    console::validate_window(&window)?;
    state.set_hud_width(width).map_err(config_save_error)?;
    if let Some(hud_window) = app.get_webview_window(hud::HUD_WINDOW_LABEL)
        && let Err(error) = hud::sync_content_width(&hud_window, &state)
    {
        log::warn!("native HUD width refresh failed after Settings change: {error:?}");
    }
    Ok(state.settings_snapshot())
}

#[tauri::command]
pub(crate) fn set_settings_hud_always_on_top(
    enabled: bool,
    app: AppHandle,
    state: State<'_, AppState>,
    window: WebviewWindow,
) -> Result<SettingsSnapshot, CommandError> {
    console::validate_window(&window)?;
    let hud_window = hud_window(&app)?;
    hud::set_always_on_top(&hud_window, &state, enabled)?;
    Ok(state.settings_snapshot())
}

#[tauri::command]
pub(crate) fn open_settings_hud_editor(
    app: AppHandle,
    state: State<'_, AppState>,
    window: WebviewWindow,
) -> Result<SettingsSnapshot, CommandError> {
    console::validate_window(&window)?;
    let hud_window = hud_window(&app)?;
    let main_window = app
        .get_webview_window(main_dps::MAIN_DPS_WINDOW_LABEL)
        .ok_or_else(CommandError::window_operation_failed)?;
    main_dps::set_passthrough(&main_window, &state, false)?;
    hud::set_passthrough(&hud_window, &state, false)?;
    hud_window.show().map_err(window_operation_error)?;
    hud_window.unminimize().map_err(window_operation_error)?;
    hud_window.set_focus().map_err(window_operation_error)?;
    if let Err(error) = main_window.hide() {
        let _ = hud_window.hide();
        return Err(window_operation_error(error));
    }
    Ok(state.settings_snapshot())
}

fn read_team_data_file(path: &Path) -> Result<String, CommandError> {
    // Use one opened handle for type/size validation and bounded reading so a
    // path replacement cannot bypass the pre-read budget.
    let file = File::open(path).map_err(|_| CommandError::team_data_import_failed())?;
    let metadata = file
        .metadata()
        .map_err(|_| CommandError::team_data_import_failed())?;
    if !metadata.is_file() {
        return Err(CommandError::team_data_import_failed());
    }
    if metadata.len() > MAX_TEAM_DPS_EXPORT_BYTES as u64 {
        return Err(CommandError::team_data_invalid());
    }
    let mut bytes = Vec::with_capacity(metadata.len() as usize);
    file.take((MAX_TEAM_DPS_EXPORT_BYTES + 1) as u64)
        .read_to_end(&mut bytes)
        .map_err(|_| CommandError::team_data_import_failed())?;
    if bytes.len() > MAX_TEAM_DPS_EXPORT_BYTES {
        return Err(CommandError::team_data_invalid());
    }
    String::from_utf8(bytes).map_err(|_| CommandError::team_data_invalid())
}

fn serialize_team_data(
    export: &nte_dps_tool::engine::model::TeamDpsExport,
) -> Result<String, CommandError> {
    let json = serde_json::to_string_pretty(export).map_err(|error| {
        log::error!("serialize team DPS export failed: {error}");
        CommandError::team_data_unavailable()
    })?;
    if json.len() > MAX_TEAM_DPS_EXPORT_BYTES {
        log::error!("serialized team DPS export exceeded its byte budget");
        return Err(CommandError::team_data_export_failed());
    }
    Ok(json)
}

fn config_save_error(error: SettingsServiceError) -> CommandError {
    log::error!("save HUD configuration from Settings failed: {error:?}");
    match error {
        SettingsServiceError::ConfigSave => CommandError::hud_config_save_failed(),
        SettingsServiceError::TransactionUnavailable => {
            CommandError::settings_transaction_unavailable()
        }
        SettingsServiceError::PassthroughUnavailable => {
            CommandError::passthrough_state_unavailable()
        }
    }
}

fn hud_window(app: &AppHandle) -> Result<WebviewWindow, CommandError> {
    app.get_webview_window(hud::HUD_WINDOW_LABEL)
        .ok_or_else(CommandError::window_operation_failed)
}

fn sync_hud_height(app: &AppHandle, state: &AppState) {
    if let Some(hud_window) = app.get_webview_window(hud::HUD_WINDOW_LABEL)
        && let Err(error) = hud::sync_content_height(&hud_window, state)
    {
        log::warn!("native HUD height refresh failed after Settings change: {error:?}");
    }
}

fn window_operation_error(error: tauri::Error) -> CommandError {
    log::error!("open HUD editor from Settings failed: {error}");
    CommandError::window_operation_failed()
}

fn settings_save_error(error: SettingsServiceError) -> CommandError {
    log::error!("save Settings configuration failed: {error:?}");
    match error {
        SettingsServiceError::ConfigSave => CommandError::settings_config_save_failed(),
        SettingsServiceError::TransactionUnavailable => {
            CommandError::settings_transaction_unavailable()
        }
        SettingsServiceError::PassthroughUnavailable => {
            CommandError::passthrough_state_unavailable()
        }
    }
}

#[derive(Debug)]
enum CheckSettingsUpdateError {
    NotConfigured,
    Failed(String),
    WorkerFailed,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum UpdateDownloadWorkerError {
    Failed,
    RuntimeUnavailable,
}

fn check_for_updates() -> Result<Vec<AvailableComponentUpdate>, CheckSettingsUpdateError> {
    let endpoint = match UpdateEndpoint::official() {
        Ok(endpoint) => endpoint,
        Err(UpdateError::ClientNotConfigured) => {
            return Err(CheckSettingsUpdateError::NotConfigured);
        }
        Err(error) => return Err(CheckSettingsUpdateError::Failed(error.to_string())),
    };
    let manifest = update_http::get_bytes(&endpoint.manifest_url, MAX_MANIFEST_BYTES)
        .map_err(|error| CheckSettingsUpdateError::Failed(error.to_string()))?;
    let app = installed_app_version(env!("CARGO_PKG_VERSION"))
        .map_err(|error| CheckSettingsUpdateError::Failed(error.to_string()))?
        .app;
    let installed = installed_component_versions(app)
        .map_err(|error| CheckSettingsUpdateError::Failed(error.to_string()))?;
    verify_manifest(&manifest, &endpoint, &installed)
        .map(|updates| {
            updates
                .into_iter()
                .filter(|u| u.component == UpdateComponent::App)
                .collect()
        })
        .map_err(|error| CheckSettingsUpdateError::Failed(error.to_string()))
}

pub(crate) fn app_update_component(
    component: UpdateComponent,
) -> Result<UpdateComponent, CommandError> {
    match component {
        UpdateComponent::App => Ok(component),
        UpdateComponent::ModsPlugin => Err(super::toolkit::unsupported()),
    }
}

impl From<UpdateActionError> for CommandError {
    fn from(error: UpdateActionError) -> Self {
        let (code, message_key) = match error {
            UpdateActionError::Busy => (
                "update_operation_busy",
                "Another update operation is already in progress.",
            ),
            UpdateActionError::Unavailable => (
                "update_component_unavailable",
                "The selected update component is no longer available.",
            ),
            UpdateActionError::NotPrepared => (
                "update_not_prepared",
                "Download and verify the update before installing it.",
            ),
            UpdateActionError::RuntimeUnavailable => (
                "update_runtime_unavailable",
                "Update operation did not finish.",
            ),
        };
        Self::new(code, message_key)
    }
}

fn validate_filter(filter: String) -> Result<String, CommandError> {
    let filter = filter.trim();
    if filter.is_empty()
        || filter.len() > 512
        || filter
            .chars()
            .any(|character| character == '\0' || character == '\r' || character == '\n')
    {
        return Err(CommandError::invalid_settings_input());
    }
    Ok(filter.to_owned())
}

fn parse_hotkey_binding(binding: HotkeyBindingSnapshot) -> Result<HotkeyBinding, CommandError> {
    let key = HotkeyKey::all()
        .iter()
        .copied()
        .find(|key| key.label() == binding.key)
        .ok_or_else(CommandError::invalid_settings_input)?;
    Ok(HotkeyBinding::new(
        binding.ctrl,
        binding.alt,
        binding.shift,
        key,
    ))
}

fn hotkey_conflicts(
    binding: HotkeyBinding,
    passthrough: HotkeyBinding,
    hotkeys: nte_dps_tool::storage::config::GlobalHotkeys,
    excluded_action: Option<GlobalHotkeyAction>,
) -> bool {
    (excluded_action.is_some() && binding == passthrough)
        || GlobalHotkeyAction::all().iter().copied().any(|action| {
            excluded_action != Some(action) && hotkeys.binding(action) == Some(binding)
        })
}

fn refresh_hotkey_configuration(state: &AppState) {
    #[cfg(windows)]
    nte_dps_tool::platform::passthrough_hotkey::PassthroughHotkeyHandle::set_configuration(
        state.passthrough_hotkey(),
        state.global_hotkeys(),
    );
}

#[cfg(test)]
mod tests {
    use std::{
        fs,
        path::PathBuf,
        time::{SystemTime, UNIX_EPOCH},
    };

    use nte_dps_tool::engine::model::{TeamDps, TeamDpsExport, TeamDpsMember};

    use super::*;

    #[test]
    fn custom_hotkeys_parse_and_conflict_across_global_and_passthrough_actions() {
        let plain_k = parse_hotkey_binding(HotkeyBindingSnapshot {
            ctrl: false,
            alt: false,
            shift: false,
            key: "K".to_owned(),
        })
        .expect("letter hotkey");
        assert_eq!(plain_k.key, HotkeyKey::K);

        let passthrough = HotkeyBinding::new(true, false, false, HotkeyKey::Home);
        let hotkeys = nte_dps_tool::storage::config::GlobalHotkeys {
            capture: Some(plain_k),
            ..Default::default()
        };
        assert!(hotkey_conflicts(plain_k, passthrough, hotkeys, None));
        assert!(hotkey_conflicts(
            passthrough,
            passthrough,
            hotkeys,
            Some(GlobalHotkeyAction::ToggleHud)
        ));
        assert!(!hotkey_conflicts(
            plain_k,
            passthrough,
            hotkeys,
            Some(GlobalHotkeyAction::ToggleCapture)
        ));
    }

    struct TeamFileFixture {
        root: PathBuf,
        path: PathBuf,
    }

    impl TeamFileFixture {
        fn new(tag: &str) -> Self {
            let root = std::env::temp_dir().join(format!(
                "nte_team_import_{tag}_{}_{}",
                std::process::id(),
                SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .expect("system clock after epoch")
                    .as_nanos()
            ));
            fs::create_dir_all(&root).expect("create team import fixture directory");
            let path = root.join("team.json");
            Self { root, path }
        }
    }

    impl Drop for TeamFileFixture {
        fn drop(&mut self) {
            let _ = fs::remove_file(&self.path);
            let _ = fs::remove_dir(&self.root);
        }
    }

    fn serialized_error(error: &CommandError) -> String {
        serde_json::to_string(error).expect("serialize command error")
    }

    #[test]
    fn general_settings_inputs_are_strictly_bounded() {
        assert_eq!(validate_filter(" udp ".to_owned()).expect("filter"), "udp");
        assert!(validate_filter("".to_owned()).is_err());
        assert!(validate_filter("udp\nor tcp".to_owned()).is_err());
        assert!(
            parse_hotkey_binding(HotkeyBindingSnapshot {
                ctrl: true,
                alt: false,
                shift: false,
                key: "F12".to_owned(),
            })
            .is_ok()
        );
    }

    #[test]
    fn team_file_reader_accepts_only_bounded_regular_utf8_files() {
        let valid = TeamFileFixture::new("valid");
        fs::write(&valid.path, br#"{"version":1}"#).expect("write valid fixture");
        assert_eq!(
            read_team_data_file(&valid.path).expect("read regular UTF-8 file"),
            r#"{"version":1}"#
        );

        let directory = TeamFileFixture::new("directory");
        let directory_error =
            read_team_data_file(&directory.root).expect_err("directory is not a regular file");
        assert_eq!(directory_error.code, "team_data_import_failed");

        let invalid_utf8 = TeamFileFixture::new("invalid_utf8");
        fs::write(&invalid_utf8.path, [0xff, 0xfe]).expect("write invalid UTF-8 fixture");
        let utf8_error =
            read_team_data_file(&invalid_utf8.path).expect_err("invalid UTF-8 must fail");
        assert_eq!(utf8_error.code, "team_data_invalid");

        let oversized = TeamFileFixture::new("oversized");
        fs::write(&oversized.path, vec![b'x'; MAX_TEAM_DPS_EXPORT_BYTES + 1])
            .expect("write oversized fixture");
        let size_error =
            read_team_data_file(&oversized.path).expect_err("oversized input must fail");
        assert_eq!(size_error.code, "team_data_invalid");
    }

    #[test]
    fn team_file_errors_do_not_expose_the_selected_path() {
        let fixture = TeamFileFixture::new("private_selected_path");
        let error = read_team_data_file(&fixture.path).expect_err("missing file must fail");
        let serialized = serialized_error(&error);

        assert!(!serialized.contains("private_selected_path"));
        assert!(!serialized.contains(fixture.root.to_string_lossy().as_ref()));
        assert!(error.diagnostic_line.is_none());
    }

    #[test]
    fn serialized_team_export_is_bounded_before_file_io() {
        let export = TeamDpsExport {
            version: 1,
            single: Some(TeamDps {
                dps: 1.0,
                members: vec![TeamDpsMember {
                    id: 1,
                    dps: 1.0,
                    name: "x".repeat(MAX_TEAM_DPS_EXPORT_BYTES),
                }],
            }),
            upper: None,
            lower: None,
        };

        let error = serialize_team_data(&export).expect_err("oversized export must fail");
        assert_eq!(error.code, "team_data_export_failed");
    }

    #[test]
    fn poisoned_update_runtime_stops_workers_before_spawn_or_io() {
        let state = AppState::default();
        state.poison_update_runtime_for_test();
        let initial_revision = state.settings_revision();

        let check_started = Arc::new(AtomicBool::new(false));
        let check_marker = Arc::clone(&check_started);
        let check_result =
            tauri::async_runtime::block_on(run_update_check_with(state.clone(), move || {
                check_marker.store(true, Ordering::Release);
                std::future::ready(Ok(Vec::new()))
            }));

        let download_started = Arc::new(AtomicBool::new(false));
        let download_marker = Arc::clone(&download_started);
        let download_result = tauri::async_runtime::block_on(run_update_download_with(
            state.clone(),
            UpdateComponent::App,
            move |_, _| {
                download_marker.store(true, Ordering::Release);
                std::future::ready(Err(UpdateDownloadWorkerError::Failed))
            },
        ));

        let install_started = Arc::new(AtomicBool::new(false));
        let install_marker = Arc::clone(&install_started);
        let install_result =
            tauri::async_runtime::block_on(install_update_with(state.clone(), move |_, _| {
                install_marker.store(true, Ordering::Release);
                std::future::ready(Ok(()))
            }));

        assert_eq!(check_result, Err(UpdateActionError::RuntimeUnavailable));
        assert_eq!(download_result, Err(UpdateActionError::RuntimeUnavailable));
        assert_eq!(install_result, Err(UpdateActionError::RuntimeUnavailable));
        assert!(!check_started.load(Ordering::Acquire));
        assert!(!download_started.load(Ordering::Acquire));
        assert!(!install_started.load(Ordering::Acquire));
        assert_eq!(state.settings_revision(), initial_revision);
    }

    #[test]
    fn update_runtime_unavailable_has_a_stable_redacted_command_error() {
        let error = CommandError::from(UpdateActionError::RuntimeUnavailable);

        assert_eq!(error.code, "update_runtime_unavailable");
        assert_eq!(error.message_key, "Update operation did not finish.");
        assert!(error.message_arguments.is_empty());
        let serialized = serde_json::to_string(&error).expect("serialize command error");
        assert!(!serialized.contains("private-update-transaction"));
        assert!(!serialized.contains("poison"));
        assert!(!serialized.contains("mutex"));
    }
}
