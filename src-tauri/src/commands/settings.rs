use nte_dps_tool::{
    core::{
        team_data::parse_team_data,
        update::{
            MAX_MANIFEST_BYTES, UpdateComponent, UpdateEndpoint, installed_app_version,
            verify_manifest,
        },
    },
    platform::update_http,
    storage::{
        config::{
            AccentColor, DpsTimeMode, GlobalHotkeyAction, HotkeyBinding, HotkeyKey,
            PassthroughHotkey, ThemePreset, UiDensity,
        },
        i18n::Language,
    },
};
use tauri::{AppHandle, Manager, State, WebviewWindow};

use crate::{
    contract::{
        CommandError,
        settings::{
            CaptureSettingsInput, HotkeyBindingSnapshot, InterfaceSettingsInput, SettingsSnapshot,
            UpdateSettingsInput,
        },
    },
    state::{AppState, HudPreset, HudSettingOption, UpdateStatusState},
    windows::{abyss_values, console, hud},
};

use super::{parse_hud_module, sanitize_hud_width};

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
    let language = parse_language(&settings.language)?;
    state
        .update_interface_settings(
            language,
            parse_theme_preset(&settings.theme_preset)?,
            parse_accent(&settings.accent)?,
            parse_density(&settings.density)?,
            settings.reduce_motion,
            settings.island_notifications,
            settings.island_offset_x,
        )
        .map_err(settings_save_error)?;
    nte_dps_tool::storage::i18n::set_language(language);
    if let Some(console_window) = app.get_webview_window(console::CONSOLE_WINDOW_LABEL)
        && let Err(error) = console_window.set_title(&nte_dps_tool::storage::i18n::t("NTE Console"))
    {
        log::warn!("refresh Console title after language change failed: {error}");
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
    run_update_check(state.clone()).await;
    Ok(state.settings_snapshot())
}

pub(crate) fn schedule_automatic_update_check(state: AppState) {
    if !state.auto_check_updates() {
        return;
    }
    tauri::async_runtime::spawn(async move {
        run_update_check(state).await;
    });
}

async fn run_update_check(state: AppState) {
    state.set_update_status(UpdateStatusState {
        status: "checking",
        message_key: "Checking for updates...",
        message_arguments: Vec::new(),
    });
    let result = tauri::async_runtime::spawn_blocking(check_for_updates).await;
    match result {
        Ok(Ok(updates)) if updates.is_empty() => {
            state.set_update_status(UpdateStatusState {
                status: "up-to-date",
                message_key: "NTE DPS Tool is up to date",
                message_arguments: Vec::new(),
            });
        }
        Ok(Ok(updates)) => {
            let update = updates
                .iter()
                .find(|update| update.component == UpdateComponent::App)
                .unwrap_or(&updates[0]);
            state.set_update_status(UpdateStatusState {
                status: "available",
                message_key: match update.component {
                    UpdateComponent::App => "Version {} is available",
                    UpdateComponent::ModsPlugin => "Mod loader version {} is available",
                },
                message_arguments: vec![update.version.to_string()],
            });
        }
        Ok(Err(error)) => {
            log::error!("Settings update check failed: {error}");
            state.set_update_status(UpdateStatusState {
                status: "error",
                message_key: "Update check failed.",
                message_arguments: Vec::new(),
            });
        }
        Err(error) => {
            log::error!("Settings update worker failed: {error}");
            state.set_update_status(UpdateStatusState {
                status: "error",
                message_key: "Update check failed.",
                message_arguments: Vec::new(),
            });
        }
    }
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
    state
        .update_capture_settings(
            filter,
            manual_device,
            settings.server_damage_calibration,
            settings.separate_reaction_damage,
            settings.auto_round_after_idle,
            settings.auto_round_idle_seconds,
            parse_dps_time_mode(&settings.dps_time_mode)?,
            parse_passthrough_hotkey(&settings.passthrough_hotkey)?,
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
    state
        .refresh_capture_devices()
        .map_err(CommandError::from_core)?;
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
    action: String,
    binding: Option<HotkeyBindingSnapshot>,
    state: State<'_, AppState>,
    window: WebviewWindow,
) -> Result<SettingsSnapshot, CommandError> {
    console::validate_window(&window)?;
    let action = parse_global_hotkey_action(&action)?;
    let binding = binding.map(parse_hotkey_binding).transpose()?;
    if binding.is_some_and(|binding| {
        (!binding.ctrl && !binding.alt && !binding.shift) || binding.is_reserved()
    }) {
        return Err(CommandError::invalid_settings_input());
    }
    let mut hotkeys = state.global_hotkeys();
    hotkeys.set_binding(action, binding);
    state
        .update_global_hotkeys(hotkeys)
        .map_err(settings_save_error)?;
    refresh_hotkey_configuration(&state);
    Ok(state.settings_snapshot())
}

#[tauri::command]
pub(crate) fn apply_settings_layout_profile(
    profile: String,
    app: AppHandle,
    state: State<'_, AppState>,
    window: WebviewWindow,
) -> Result<SettingsSnapshot, CommandError> {
    console::validate_window(&window)?;
    let hud_window = hud_window(&app)?;
    match profile.as_str() {
        "combat" => {
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
        }
        "review" => {
            state
                .set_density(UiDensity::Cozy)
                .map_err(settings_save_error)?;
            hud::set_passthrough(&hud_window, &state, false)?;
            hud_window.hide().map_err(window_operation_error)?;
            window.set_focus().map_err(window_operation_error)?;
        }
        "research" => {
            state
                .set_density(UiDensity::Compact)
                .map_err(settings_save_error)?;
            hud::set_passthrough(&hud_window, &state, false)?;
            hud_window.hide().map_err(window_operation_error)?;
            window.set_focus().map_err(window_operation_error)?;
        }
        _ => return Err(CommandError::invalid_settings_input()),
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
    state.import_team_data(export);
    Ok(state.settings_snapshot())
}

#[tauri::command]
pub(crate) fn export_settings_team_data(
    state: State<'_, AppState>,
    window: WebviewWindow,
) -> Result<String, CommandError> {
    console::validate_window(&window)?;
    let export = state
        .export_team_data()
        .ok_or_else(CommandError::team_data_unavailable)?;
    serde_json::to_string(&export).map_err(|error| {
        log::error!("serialize team DPS export failed: {error}");
        CommandError::team_data_unavailable()
    })
}

#[tauri::command]
pub(crate) fn refresh_settings_capture_files(
    state: State<'_, AppState>,
    window: WebviewWindow,
) -> Result<SettingsSnapshot, CommandError> {
    console::validate_window(&window)?;
    Ok(state.settings_snapshot())
}

#[tauri::command]
pub(crate) fn clear_settings_capture_files(
    state: State<'_, AppState>,
    window: WebviewWindow,
) -> Result<SettingsSnapshot, CommandError> {
    console::validate_window(&window)?;
    let outcome = state.clear_capture_files();
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
    abyss_values::show(&abyss_window)?;
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
    option: String,
    enabled: bool,
    app: AppHandle,
    state: State<'_, AppState>,
    window: WebviewWindow,
) -> Result<SettingsSnapshot, CommandError> {
    console::validate_window(&window)?;
    let option = parse_hud_option(&option)?;
    state
        .set_hud_option(option, enabled)
        .map_err(config_save_error)?;
    sync_hud_height(&app, &state);
    Ok(state.settings_snapshot())
}

#[tauri::command]
pub(crate) fn apply_settings_hud_preset(
    preset: String,
    app: AppHandle,
    state: State<'_, AppState>,
    window: WebviewWindow,
) -> Result<SettingsSnapshot, CommandError> {
    console::validate_window(&window)?;
    let preset = parse_hud_preset(&preset)?;
    state.apply_hud_preset(preset).map_err(config_save_error)?;
    sync_hud_height(&app, &state);
    Ok(state.settings_snapshot())
}

#[tauri::command]
pub(crate) fn set_settings_hud_module_visibility(
    module: String,
    visible: bool,
    app: AppHandle,
    state: State<'_, AppState>,
    window: WebviewWindow,
) -> Result<SettingsSnapshot, CommandError> {
    console::validate_window(&window)?;
    let module = parse_hud_module(&module)?;
    state
        .set_hud_module_visibility(module, visible)
        .map_err(config_save_error)?;
    sync_hud_height(&app, &state);
    Ok(state.settings_snapshot())
}

#[tauri::command]
pub(crate) fn move_settings_hud_module(
    dragged: String,
    target: String,
    insert_after: bool,
    app: AppHandle,
    state: State<'_, AppState>,
    window: WebviewWindow,
) -> Result<SettingsSnapshot, CommandError> {
    console::validate_window(&window)?;
    let dragged = parse_hud_module(&dragged)?;
    let target = parse_hud_module(&target)?;
    state
        .move_hud_module(dragged, target, insert_after)
        .map_err(config_save_error)?;
    sync_hud_height(&app, &state);
    Ok(state.settings_snapshot())
}

#[tauri::command]
pub(crate) fn set_settings_hud_width(
    width: i32,
    app: AppHandle,
    state: State<'_, AppState>,
    window: WebviewWindow,
) -> Result<SettingsSnapshot, CommandError> {
    console::validate_window(&window)?;
    state
        .set_hud_width(sanitize_hud_width(width))
        .map_err(config_save_error)?;
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
    hud::set_passthrough(&hud_window, &state, false)?;
    hud_window.show().map_err(window_operation_error)?;
    hud_window.unminimize().map_err(window_operation_error)?;
    hud_window.set_focus().map_err(window_operation_error)?;
    Ok(state.settings_snapshot())
}

fn parse_hud_option(option: &str) -> Result<HudSettingOption, CommandError> {
    match option {
        "title" => Ok(HudSettingOption::Title),
        "team_dps" => Ok(HudSettingOption::TeamDps),
        "duration" => Ok(HudSettingOption::Duration),
        "total_damage" => Ok(HudSettingOption::TotalDamage),
        "damage_taken" => Ok(HudSettingOption::DamageTaken),
        "character_rows" => Ok(HudSettingOption::CharacterRows),
        "abyss_half" => Ok(HudSettingOption::AbyssHalf),
        "passthrough_state" => Ok(HudSettingOption::PassthroughState),
        "mini_timeline" => Ok(HudSettingOption::MiniTimeline),
        _ => Err(CommandError::invalid_hud_option()),
    }
}

fn parse_hud_preset(preset: &str) -> Result<HudPreset, CommandError> {
    match preset {
        "minimal" => Ok(HudPreset::Minimal),
        "standard" => Ok(HudPreset::Standard),
        "detailed" => Ok(HudPreset::Detailed),
        _ => Err(CommandError::invalid_hud_preset()),
    }
}

fn config_save_error(error: String) -> CommandError {
    log::error!("save HUD configuration from Settings failed: {error}");
    CommandError::hud_config_save_failed()
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

fn settings_save_error(error: String) -> CommandError {
    log::error!("save Settings configuration failed: {error}");
    CommandError::settings_config_save_failed()
}

fn check_for_updates() -> Result<Vec<nte_dps_tool::core::update::AvailableComponentUpdate>, String>
{
    let endpoint = UpdateEndpoint::official().map_err(|error| error.to_string())?;
    let manifest = update_http::get_bytes(&endpoint.manifest_url, MAX_MANIFEST_BYTES)
        .map_err(|error| error.to_string())?;
    let installed =
        installed_app_version(env!("CARGO_PKG_VERSION")).map_err(|error| error.to_string())?;
    verify_manifest(&manifest, &endpoint, &installed).map_err(|error| error.to_string())
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

fn parse_language(value: &str) -> Result<Language, CommandError> {
    match value {
        "en" => Ok(Language::English),
        "ja" => Ok(Language::Japanese),
        "zh-CN" => Ok(Language::SimplifiedChinese),
        _ => Err(CommandError::invalid_settings_input()),
    }
}

fn parse_theme_preset(value: &str) -> Result<ThemePreset, CommandError> {
    match value {
        "zinc" => Ok(ThemePreset::Zinc),
        "tactical" => Ok(ThemePreset::Tactical),
        "high-contrast" => Ok(ThemePreset::HighContrast),
        _ => Err(CommandError::invalid_settings_input()),
    }
}

fn parse_accent(value: &str) -> Result<AccentColor, CommandError> {
    match value {
        "zinc" => Ok(AccentColor::Zinc),
        "blue" => Ok(AccentColor::Blue),
        "violet" => Ok(AccentColor::Violet),
        "orange" => Ok(AccentColor::Orange),
        "green" => Ok(AccentColor::Green),
        _ => Err(CommandError::invalid_settings_input()),
    }
}

fn parse_density(value: &str) -> Result<UiDensity, CommandError> {
    match value {
        "compact" => Ok(UiDensity::Compact),
        "cozy" => Ok(UiDensity::Cozy),
        "comfortable" => Ok(UiDensity::Comfortable),
        _ => Err(CommandError::invalid_settings_input()),
    }
}

fn parse_dps_time_mode(value: &str) -> Result<DpsTimeMode, CommandError> {
    match value {
        "time-stop-adjusted" => Ok(DpsTimeMode::TimeStopAdjusted),
        "real-time" => Ok(DpsTimeMode::RealTime),
        _ => Err(CommandError::invalid_settings_input()),
    }
}

fn parse_passthrough_hotkey(value: &str) -> Result<PassthroughHotkey, CommandError> {
    match value {
        "home" => Ok(PassthroughHotkey::Home),
        "insert" => Ok(PassthroughHotkey::Insert),
        "f8" => Ok(PassthroughHotkey::F8),
        "f9" => Ok(PassthroughHotkey::F9),
        _ => Err(CommandError::invalid_settings_input()),
    }
}

fn parse_global_hotkey_action(value: &str) -> Result<GlobalHotkeyAction, CommandError> {
    match value {
        "capture" => Ok(GlobalHotkeyAction::ToggleCapture),
        "reset" => Ok(GlobalHotkeyAction::ResetSession),
        "hud" => Ok(GlobalHotkeyAction::ToggleHud),
        _ => Err(CommandError::invalid_settings_input()),
    }
}

fn parse_hotkey_binding(binding: HotkeyBindingSnapshot) -> Result<HotkeyBinding, CommandError> {
    let key = match binding.key.as_str() {
        "F1" => HotkeyKey::F1,
        "F2" => HotkeyKey::F2,
        "F3" => HotkeyKey::F3,
        "F4" => HotkeyKey::F4,
        "F5" => HotkeyKey::F5,
        "F6" => HotkeyKey::F6,
        "F7" => HotkeyKey::F7,
        "F8" => HotkeyKey::F8,
        "F9" => HotkeyKey::F9,
        "F10" => HotkeyKey::F10,
        "F11" => HotkeyKey::F11,
        "F12" => HotkeyKey::F12,
        _ => return Err(CommandError::invalid_settings_input()),
    };
    Ok(HotkeyBinding::new(
        binding.ctrl,
        binding.alt,
        binding.shift,
        key,
    ))
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
    use super::*;

    #[test]
    fn hud_option_input_accepts_only_stable_contract_values() {
        assert_eq!(
            parse_hud_option("team_dps").expect("team DPS"),
            HudSettingOption::TeamDps
        );
        assert_eq!(
            parse_hud_option("mini_timeline").expect("timeline"),
            HudSettingOption::MiniTimeline
        );
        assert!(parse_hud_option("future").is_err());
        assert!(parse_hud_option("../title").is_err());
    }

    #[test]
    fn hud_preset_input_accepts_only_stable_contract_values() {
        assert_eq!(
            parse_hud_preset("minimal").expect("minimal"),
            HudPreset::Minimal
        );
        assert_eq!(
            parse_hud_preset("detailed").expect("detailed"),
            HudPreset::Detailed
        );
        assert!(parse_hud_preset("custom").is_err());
    }

    #[test]
    fn general_settings_inputs_are_strictly_bounded() {
        assert_eq!(validate_filter(" udp ".to_owned()).expect("filter"), "udp");
        assert!(validate_filter("".to_owned()).is_err());
        assert!(validate_filter("udp\nor tcp".to_owned()).is_err());
        assert_eq!(parse_language("ja").expect("Japanese"), Language::Japanese);
        assert!(parse_language("future").is_err());
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
}
