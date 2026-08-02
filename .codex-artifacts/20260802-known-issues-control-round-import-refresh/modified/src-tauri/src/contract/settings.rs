use serde::{Deserialize, Serialize};

use nte_dps_tool::{
    core::{
        hud::HudConfigSnapshot,
        update::{AvailableComponentUpdate, UpdateComponent},
    },
    engine::capture::CaptureDevice,
    storage::{
        capture_logs::{CaptureLogStats, format_bytes},
        config::{
            AccentColor, DpsTimeMode, GlobalHotkeyAction, GlobalHotkeys, HUD_WIDTH_MAX,
            HUD_WIDTH_MIN, HotkeyBinding, PassthroughHotkey, ThemePreset, UiConfig, UiDensity,
        },
        update::PreparedUpdate,
    },
};

pub(crate) const SETTINGS_CONTRACT_VERSION: u32 = 4;

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SettingsSnapshot {
    pub contract_version: u32,
    pub generation: String,
    pub adapter_version: &'static str,
    pub interface: InterfaceSettingsSnapshot,
    pub updates: UpdateSettingsSnapshot,
    pub capture: CaptureSettingsSnapshot,
    pub hotkeys: GlobalHotkeysSnapshot,
    pub capture_files: CaptureFilesSnapshot,
    pub team_data: TeamDataSnapshot,
    pub always_on_top: bool,
    pub hud_width_min: u16,
    pub hud_width_max: u16,
    pub hud: HudConfigSnapshot,
}

#[derive(Clone, Debug, Serialize)]
#[serde(tag = "event", content = "payload", rename_all = "camelCase")]
pub(crate) enum SettingsEvent {
    Snapshot(SettingsSnapshot),
}

#[derive(Clone, Copy, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct TeamDataExportResult {
    pub saved: bool,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct TeamDataImportFileResult {
    pub performed: bool,
    pub settings: SettingsSnapshot,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct InterfaceSettingsSnapshot {
    pub language: &'static str,
    pub dark_mode: bool,
    pub theme_preset: &'static str,
    pub accent: &'static str,
    pub density: &'static str,
    pub reduce_motion: bool,
    pub island_notifications: bool,
    pub island_offset_x: f32,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct UpdateSettingsSnapshot {
    pub current_version: &'static str,
    pub auto_check: bool,
    pub auto_download: bool,
    pub status: String,
    pub message_key: String,
    pub message_arguments: Vec<String>,
    pub available: Vec<AvailableUpdateSnapshot>,
    pub active_component: Option<&'static str>,
    pub downloaded_bytes: String,
    pub total_bytes: String,
    pub prepared: Option<PreparedUpdateSnapshot>,
    pub install_enabled: bool,
    pub install_blocked_message_key: Option<&'static str>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct AvailableUpdateSnapshot {
    pub component: &'static str,
    pub version: String,
    pub published_at: String,
    pub notes: String,
    pub artifact_size: String,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct PreparedUpdateSnapshot {
    pub component: &'static str,
    pub version: String,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct CaptureSettingsSnapshot {
    pub bpf_filter: String,
    pub devices: Vec<CaptureDeviceSnapshot>,
    pub manual_capture_device: Option<String>,
    pub server_damage_calibration: bool,
    pub separate_reaction_damage: bool,
    pub auto_round_after_idle: bool,
    pub auto_round_idle_seconds: u32,
    pub auto_round_idle_seconds_min: u32,
    pub auto_round_idle_seconds_max: u32,
    pub dps_time_mode: &'static str,
    pub passthrough_hotkey: &'static str,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct CaptureDeviceSnapshot {
    pub id: String,
    pub label: String,
}

impl From<&CaptureDevice> for CaptureDeviceSnapshot {
    fn from(device: &CaptureDevice) -> Self {
        let addresses = device
            .ipv4
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>();
        let mut label = if device.description.trim().is_empty() {
            device.name.clone()
        } else {
            device.description.clone()
        };
        if !addresses.is_empty() {
            label.push_str(" · ");
            label.push_str(&addresses.join(", "));
        }
        Self {
            id: device.name.clone(),
            label,
        }
    }
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct GlobalHotkeysSnapshot {
    pub enabled: bool,
    pub bindings: Vec<GlobalHotkeyBindingSnapshot>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct GlobalHotkeyBindingSnapshot {
    pub action: &'static str,
    pub binding: Option<HotkeyBindingSnapshot>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct HotkeyBindingSnapshot {
    pub ctrl: bool,
    pub alt: bool,
    pub shift: bool,
    pub key: String,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct CaptureFilesSnapshot {
    pub count: usize,
    pub total_bytes: String,
    pub formatted_size: String,
}

#[derive(Clone, Copy, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct TeamDataSnapshot {
    pub upper_imported: bool,
    pub lower_imported: bool,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct InterfaceSettingsInput {
    pub language: String,
    pub dark_mode: bool,
    pub theme_preset: String,
    pub accent: String,
    pub density: String,
    pub reduce_motion: bool,
    pub island_notifications: bool,
    pub island_offset_x: f32,
}

#[derive(Clone, Copy, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct UpdateSettingsInput {
    pub auto_check: bool,
    pub auto_download: bool,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct CaptureSettingsInput {
    pub bpf_filter: String,
    pub manual_capture_device: Option<String>,
    pub server_damage_calibration: bool,
    pub separate_reaction_damage: bool,
    pub auto_round_after_idle: bool,
    pub auto_round_idle_seconds: u32,
    pub dps_time_mode: String,
    pub passthrough_hotkey: String,
}

impl SettingsSnapshot {
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn from_config(
        config: &UiConfig,
        generation: u64,
        always_on_top: bool,
        devices: Vec<CaptureDeviceSnapshot>,
        capture_files: CaptureLogStats,
        upper_imported: bool,
        lower_imported: bool,
        updates: UpdateSettingsSnapshot,
    ) -> Self {
        Self {
            contract_version: SETTINGS_CONTRACT_VERSION,
            generation: generation.to_string(),
            adapter_version: env!("CARGO_PKG_VERSION"),
            interface: InterfaceSettingsSnapshot {
                language: config.language.code(),
                dark_mode: config.dark_mode,
                theme_preset: theme_preset_id(config.theme_preset),
                accent: accent_id(config.accent),
                density: density_id(config.density),
                reduce_motion: config.reduce_motion,
                island_notifications: config.island_notifications,
                island_offset_x: config.island_offset_x,
            },
            updates,
            capture: CaptureSettingsSnapshot {
                bpf_filter: config.capture_filter.clone(),
                devices,
                manual_capture_device: config.manual_capture_device.clone(),
                server_damage_calibration: config.server_damage_calibration,
                separate_reaction_damage: config.separate_reaction_damage,
                auto_round_after_idle: config.auto_round_after_idle,
                auto_round_idle_seconds: config.auto_round_idle_seconds,
                auto_round_idle_seconds_min:
                    nte_dps_tool::storage::config::AUTO_ROUND_IDLE_SECONDS_MIN,
                auto_round_idle_seconds_max:
                    nte_dps_tool::storage::config::AUTO_ROUND_IDLE_SECONDS_MAX,
                dps_time_mode: dps_time_mode_id(config.dps_time_mode),
                passthrough_hotkey: passthrough_hotkey_id(config.passthrough_hotkey),
            },
            hotkeys: hotkeys_snapshot(config.global_hotkeys),
            capture_files: CaptureFilesSnapshot {
                count: capture_files.count,
                total_bytes: capture_files.total_bytes.to_string(),
                formatted_size: format_bytes(capture_files.total_bytes),
            },
            team_data: TeamDataSnapshot {
                upper_imported,
                lower_imported,
            },
            always_on_top,
            hud_width_min: HUD_WIDTH_MIN,
            hud_width_max: HUD_WIDTH_MAX,
            hud: HudConfigSnapshot::from(&config.hud),
        }
    }
}

impl UpdateSettingsSnapshot {
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn from_runtime(
        config: &UiConfig,
        status: &'static str,
        message_key: &'static str,
        message_arguments: Vec<String>,
        available: &[AvailableComponentUpdate],
        active_component: Option<UpdateComponent>,
        downloaded_bytes: u64,
        total_bytes: u64,
        prepared: Option<&PreparedUpdate>,
        install_blocked_message_key: Option<&'static str>,
    ) -> Self {
        Self {
            current_version: env!("CARGO_PKG_VERSION"),
            auto_check: config.auto_check_updates,
            auto_download: config.auto_download_updates,
            status: status.to_owned(),
            message_key: message_key.to_owned(),
            message_arguments,
            available: available
                .iter()
                .map(AvailableUpdateSnapshot::from)
                .collect(),
            active_component: active_component.map(update_component_id),
            downloaded_bytes: downloaded_bytes.to_string(),
            total_bytes: total_bytes.to_string(),
            prepared: prepared.map(PreparedUpdateSnapshot::from),
            install_enabled: prepared.is_some() && install_blocked_message_key.is_none(),
            install_blocked_message_key,
        }
    }
}

impl From<&AvailableComponentUpdate> for AvailableUpdateSnapshot {
    fn from(update: &AvailableComponentUpdate) -> Self {
        Self {
            component: update_component_id(update.component),
            version: update.version.to_string(),
            published_at: update.published_at.clone(),
            notes: update.notes.clone(),
            artifact_size: update.artifact_size.to_string(),
        }
    }
}

impl From<&PreparedUpdate> for PreparedUpdateSnapshot {
    fn from(update: &PreparedUpdate) -> Self {
        Self {
            component: update_component_id(update.component()),
            version: update.version().to_string(),
        }
    }
}

pub(crate) const fn update_component_id(component: UpdateComponent) -> &'static str {
    match component {
        UpdateComponent::App => "app",
        UpdateComponent::ModsPlugin => "mods-plugin",
    }
}

fn hotkeys_snapshot(hotkeys: GlobalHotkeys) -> GlobalHotkeysSnapshot {
    GlobalHotkeysSnapshot {
        enabled: hotkeys.enabled,
        bindings: GlobalHotkeyAction::all()
            .iter()
            .copied()
            .map(|action| GlobalHotkeyBindingSnapshot {
                action: global_hotkey_action_id(action),
                binding: hotkeys.binding(action).map(hotkey_binding_snapshot),
            })
            .collect(),
    }
}

fn hotkey_binding_snapshot(binding: HotkeyBinding) -> HotkeyBindingSnapshot {
    HotkeyBindingSnapshot {
        ctrl: binding.ctrl,
        alt: binding.alt,
        shift: binding.shift,
        key: binding.key.label().to_owned(),
    }
}

pub(crate) const fn global_hotkey_action_id(action: GlobalHotkeyAction) -> &'static str {
    match action {
        GlobalHotkeyAction::ToggleCapture => "capture",
        GlobalHotkeyAction::ResetSession => "reset",
        GlobalHotkeyAction::ToggleHud => "hud",
    }
}

const fn theme_preset_id(value: ThemePreset) -> &'static str {
    match value {
        ThemePreset::Zinc => "zinc",
        ThemePreset::Tactical => "tactical",
        ThemePreset::HighContrast => "high-contrast",
    }
}

const fn accent_id(value: AccentColor) -> &'static str {
    match value {
        AccentColor::Zinc => "zinc",
        AccentColor::Blue => "blue",
        AccentColor::Violet => "violet",
        AccentColor::Orange => "orange",
        AccentColor::Green => "green",
    }
}

const fn density_id(value: UiDensity) -> &'static str {
    match value {
        UiDensity::Compact => "compact",
        UiDensity::Cozy => "cozy",
        UiDensity::Comfortable => "comfortable",
    }
}

const fn dps_time_mode_id(value: DpsTimeMode) -> &'static str {
    match value {
        DpsTimeMode::TimeStopAdjusted => "time-stop-adjusted",
        DpsTimeMode::RealTime => "real-time",
    }
}

const fn passthrough_hotkey_id(value: PassthroughHotkey) -> &'static str {
    match value {
        PassthroughHotkey::Home => "home",
        PassthroughHotkey::Insert => "insert",
        PassthroughHotkey::F8 => "f8",
        PassthroughHotkey::F9 => "f9",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nte_dps_tool::storage::config::{HudModule, UiConfig};

    #[test]
    fn settings_snapshot_projects_all_stable_settings_sections() {
        let mut config = UiConfig::default();
        config.hud.width = 512;
        config.hud.module_order = vec![HudModule::Timeline, HudModule::Title];
        let value = serde_json::to_value(SettingsSnapshot::from_config(
            &config,
            7,
            false,
            Vec::new(),
            CaptureLogStats::default(),
            false,
            false,
            UpdateSettingsSnapshot::from_runtime(
                &config,
                "idle",
                "Updates have not been checked in this session",
                Vec::new(),
                &[],
                None,
                0,
                0,
                None,
                None,
            ),
        ))
        .expect("settings snapshot must serialize");

        assert_eq!(value["contractVersion"], SETTINGS_CONTRACT_VERSION);
        assert_eq!(value["generation"], "7");
        assert_eq!(value["interface"]["language"], "zh-CN");
        assert_eq!(value["updates"]["autoCheck"], true);
        assert_eq!(
            value["updates"]["available"].as_array().map(Vec::len),
            Some(0)
        );
        assert_eq!(value["updates"]["downloadedBytes"], "0");
        assert_eq!(value["capture"]["bpfFilter"], "udp");
        assert_eq!(value["hotkeys"]["bindings"][0]["action"], "capture");
        assert_eq!(value["hud"]["width"], 512);
        assert_eq!(value["hud"]["moduleOrder"][0], "timeline");
        assert_eq!(
            value["hud"]["moduleOrder"].as_array().map(Vec::len),
            Some(5)
        );
        assert!(value.get("contract_version").is_none());
    }

    #[test]
    fn team_data_export_result_distinguishes_save_from_cancel() {
        let saved = serde_json::to_value(TeamDataExportResult { saved: true })
            .expect("saved result must serialize");
        let cancelled = serde_json::to_value(TeamDataExportResult { saved: false })
            .expect("cancelled result must serialize");

        assert_eq!(saved["saved"], true);
        assert_eq!(cancelled["saved"], false);
    }

    #[test]
    fn team_data_file_import_result_keeps_local_paths_out_of_the_contract() {
        let config = UiConfig::default();
        let settings = SettingsSnapshot::from_config(
            &config,
            8,
            false,
            Vec::new(),
            CaptureLogStats::default(),
            false,
            false,
            UpdateSettingsSnapshot::from_runtime(
                &config,
                "idle",
                "Updates have not been checked in this session",
                Vec::new(),
                &[],
                None,
                0,
                0,
                None,
                None,
            ),
        );
        let value = serde_json::to_value(TeamDataImportFileResult {
            performed: true,
            settings,
        })
        .expect("import result must serialize");

        assert_eq!(value["performed"], true);
        assert_eq!(value["settings"]["generation"], "8");
        assert!(value.get("path").is_none());
    }
}
