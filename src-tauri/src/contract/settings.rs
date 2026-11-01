use serde::{Deserialize, Serialize};
use ts_rs::TS;

use nte_dps_tool::{
    core::{
        hud::HudConfigSnapshot,
        update::{AvailableComponentUpdate, UpdateComponent},
    },
    engine::{capture::CaptureDevice, model::CombatClockRuntimeHealth},
    storage::{
        capture_logs::{CaptureLogStats, format_bytes},
        config::{
            AccentColor, DpsTimeMode, GlobalHotkeyAction, GlobalHotkeys, HUD_WIDTH_MAX,
            HUD_WIDTH_MIN, HotkeyBinding, MainDpsDisplayConfig, ThemePreset, UiConfig, UiDensity,
        },
        i18n::Language,
        update::PreparedUpdate,
    },
};

use crate::contract::dps_time::DpsTimeRuntimeSnapshot;

#[derive(Clone, Debug, Serialize, TS)]
#[ts(export)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SettingsSnapshot {
    pub generation: String,
    pub adapter_version: &'static str,
    pub interface: InterfaceSettingsSnapshot,
    pub updates: UpdateSettingsSnapshot,
    pub capture: CaptureSettingsSnapshot,
    pub hotkeys: GlobalHotkeysSnapshot,
    pub main_dps: MainDpsDisplayConfig,
    pub capture_files: CaptureFilesSnapshot,
    pub team_data: TeamDataSnapshot,
    pub always_on_top: bool,
    pub hud_width_min: u16,
    pub hud_width_max: u16,
    pub hud: HudConfigSnapshot,
}

#[derive(Clone, Debug, Serialize, TS)]
#[ts(export)]
#[serde(tag = "event", content = "payload", rename_all = "camelCase")]
pub(crate) enum SettingsEvent {
    Snapshot(SettingsSnapshot),
}

#[derive(Clone, Copy, Debug, Serialize, TS)]
#[ts(export)]
#[serde(rename_all = "camelCase")]
pub(crate) struct TeamDataExportResult {
    pub saved: bool,
}

#[derive(Clone, Debug, Serialize, TS)]
#[ts(export)]
#[serde(rename_all = "camelCase")]
pub(crate) struct TeamDataImportFileResult {
    pub performed: bool,
    pub settings: SettingsSnapshot,
}

#[derive(Clone, Debug, Serialize, TS)]
#[ts(export)]
#[serde(rename_all = "camelCase")]
pub(crate) struct InterfaceSettingsSnapshot {
    pub language: Language,
    pub dark_mode: bool,
    pub theme_preset: ThemePreset,
    pub accent: AccentColor,
    pub density: UiDensity,
    pub reduce_motion: bool,
    pub island_notifications: bool,
    pub island_offset_x: f32,
}

#[derive(Clone, Debug, Serialize, TS)]
#[ts(export)]
#[serde(rename_all = "camelCase")]
pub(crate) struct UpdateSettingsSnapshot {
    pub current_version: &'static str,
    pub auto_check: bool,
    pub auto_download: bool,
    pub status: String,
    pub message_key: String,
    pub message_arguments: Vec<String>,
    pub available: Vec<AvailableUpdateSnapshot>,
    pub active_component: Option<UpdateComponent>,
    pub downloaded_bytes: String,
    pub total_bytes: String,
    pub prepared: Option<PreparedUpdateSnapshot>,
    pub install_enabled: bool,
    pub install_blocked_message_key: Option<&'static str>,
}

#[derive(Clone, Debug, Serialize, TS)]
#[ts(export)]
#[serde(rename_all = "camelCase")]
pub(crate) struct AvailableUpdateSnapshot {
    pub component: UpdateComponent,
    pub version: String,
    pub published_at: String,
    pub notes: String,
    pub artifact_size: String,
}

#[derive(Clone, Debug, Serialize, TS)]
#[ts(export)]
#[serde(rename_all = "camelCase")]
pub(crate) struct PreparedUpdateSnapshot {
    pub component: UpdateComponent,
    pub version: String,
}

#[derive(Clone, Debug, Serialize, TS)]
#[ts(export)]
#[serde(rename_all = "camelCase")]
pub(crate) struct CaptureSettingsSnapshot {
    pub bpf_filter: String,
    pub devices: Vec<CaptureDeviceSnapshot>,
    pub devices_available: bool,
    pub manual_capture_device: Option<String>,
    pub server_damage_calibration: bool,
    pub include_max_hp_reduction_in_total_damage: bool,
    pub separate_reaction_damage: bool,
    pub auto_round_after_idle: bool,
    pub auto_round_idle_seconds: u32,
    pub auto_round_idle_seconds_min: u32,
    pub auto_round_idle_seconds_max: u32,
    pub dps_time_mode: DpsTimeMode,
    pub dps_time_runtime: DpsTimeRuntimeSnapshot,
    pub passthrough_hotkey: HotkeyBindingSnapshot,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, TS)]
#[ts(export)]
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
            label = format!("{label} ({})", addresses.join(", "));
        }
        Self {
            id: device.name.clone(),
            label,
        }
    }
}

#[derive(Clone, Debug, Serialize, TS)]
#[ts(export)]
#[serde(rename_all = "camelCase")]
pub(crate) struct GlobalHotkeysSnapshot {
    pub enabled: bool,
    pub bindings: Vec<GlobalHotkeyBindingSnapshot>,
}

#[derive(Clone, Debug, Serialize, TS)]
#[ts(export)]
#[serde(rename_all = "camelCase")]
pub(crate) struct GlobalHotkeyBindingSnapshot {
    pub action: GlobalHotkeyAction,
    pub binding: Option<HotkeyBindingSnapshot>,
}

#[derive(Clone, Debug, Serialize, Deserialize, TS)]
#[ts(export)]
#[serde(rename_all = "camelCase")]
pub(crate) struct HotkeyBindingSnapshot {
    pub ctrl: bool,
    pub alt: bool,
    pub shift: bool,
    pub key: String,
}

#[derive(Clone, Debug, Serialize, TS)]
#[ts(export)]
#[serde(rename_all = "camelCase")]
pub(crate) struct CaptureFilesSnapshot {
    pub count: usize,
    pub total_bytes: String,
    pub formatted_size: String,
}

#[derive(Clone, Copy, Debug, Serialize, TS)]
#[ts(export)]
#[serde(rename_all = "camelCase")]
pub(crate) struct TeamDataSnapshot {
    pub available: bool,
    pub upper_imported: bool,
    pub lower_imported: bool,
}

#[derive(Clone, Debug, Deserialize, TS)]
#[ts(export)]
#[serde(rename_all = "camelCase")]
pub(crate) struct InterfaceSettingsInput {
    pub language: Language,
    pub dark_mode: bool,
    pub theme_preset: ThemePreset,
    pub accent: AccentColor,
    pub density: UiDensity,
    pub reduce_motion: bool,
    pub island_notifications: bool,
    pub island_offset_x: f32,
}

#[derive(Clone, Copy, Debug, Deserialize, TS)]
#[ts(export)]
#[serde(rename_all = "camelCase")]
pub(crate) struct UpdateSettingsInput {
    pub auto_check: bool,
    pub auto_download: bool,
}

#[derive(Clone, Debug, Deserialize, TS)]
#[ts(export)]
#[serde(rename_all = "camelCase")]
pub(crate) struct CaptureSettingsInput {
    pub bpf_filter: String,
    pub manual_capture_device: Option<String>,
    pub server_damage_calibration: bool,
    pub include_max_hp_reduction_in_total_damage: bool,
    pub separate_reaction_damage: bool,
    pub auto_round_after_idle: bool,
    pub auto_round_idle_seconds: u32,
    pub passthrough_hotkey: HotkeyBindingSnapshot,
}

impl SettingsSnapshot {
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn from_config(
        config: &UiConfig,
        generation: u64,
        always_on_top: bool,
        devices: Vec<CaptureDeviceSnapshot>,
        devices_available: bool,
        capture_files: CaptureLogStats,
        team_data_available: bool,
        upper_imported: bool,
        lower_imported: bool,
        updates: UpdateSettingsSnapshot,
        combat_clock_health: CombatClockRuntimeHealth,
    ) -> Self {
        Self {
            generation: generation.to_string(),
            adapter_version: env!("CARGO_PKG_VERSION"),
            interface: InterfaceSettingsSnapshot {
                language: config.language,
                dark_mode: config.dark_mode,
                theme_preset: config.theme_preset,
                accent: config.accent,
                density: config.density,
                reduce_motion: config.reduce_motion,
                island_notifications: config.island_notifications,
                island_offset_x: config.island_offset_x,
            },
            updates,
            capture: CaptureSettingsSnapshot {
                bpf_filter: config.capture_filter.clone(),
                devices,
                devices_available,
                manual_capture_device: config.manual_capture_device.clone(),
                server_damage_calibration: config.server_damage_calibration,
                include_max_hp_reduction_in_total_damage: config
                    .include_max_hp_reduction_in_total_damage,
                separate_reaction_damage: config.separate_reaction_damage,
                auto_round_after_idle: config.auto_round_after_idle,
                auto_round_idle_seconds: config.auto_round_idle_seconds,
                auto_round_idle_seconds_min:
                    nte_dps_tool::storage::config::AUTO_ROUND_IDLE_SECONDS_MIN,
                auto_round_idle_seconds_max:
                    nte_dps_tool::storage::config::AUTO_ROUND_IDLE_SECONDS_MAX,
                dps_time_mode: config.dps_time_mode(),
                dps_time_runtime: DpsTimeRuntimeSnapshot::new(
                    config.dps_time_mode(),
                    combat_clock_health,
                ),
                passthrough_hotkey: hotkey_binding_snapshot(config.passthrough_hotkey),
            },
            hotkeys: hotkeys_snapshot(config.global_hotkeys),
            main_dps: config.main_dps_display.clone(),
            capture_files: CaptureFilesSnapshot {
                count: capture_files.count,
                total_bytes: capture_files.total_bytes.to_string(),
                formatted_size: format_bytes(capture_files.total_bytes),
            },
            team_data: TeamDataSnapshot {
                available: team_data_available,
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
                .map(|update| AvailableUpdateSnapshot::from_update(update, config.language))
                .collect(),
            active_component,
            downloaded_bytes: downloaded_bytes.to_string(),
            total_bytes: total_bytes.to_string(),
            prepared: prepared.map(PreparedUpdateSnapshot::from),
            install_enabled: prepared.is_some() && install_blocked_message_key.is_none(),
            install_blocked_message_key,
        }
    }
}

impl AvailableUpdateSnapshot {
    fn from_update(update: &AvailableComponentUpdate, language: Language) -> Self {
        Self {
            component: update.component,
            version: update.version.to_string(),
            published_at: update.published_at.clone(),
            notes: localized_update_notes(&update.notes, language),
            artifact_size: update.artifact_size.to_string(),
        }
    }
}

fn localized_update_notes(notes: &str, language: Language) -> String {
    let sections = release_note_sections(notes);
    if sections.is_empty() {
        return notes.to_owned();
    }

    sections
        .iter()
        .find(|(section_language, _)| *section_language == language)
        .or_else(|| {
            sections
                .iter()
                .find(|(section_language, _)| *section_language == Language::English)
        })
        .or_else(|| sections.first())
        .map(|(_, content)| content.clone())
        .unwrap_or_default()
}

fn release_note_sections(notes: &str) -> Vec<(Language, String)> {
    let mut sections = Vec::new();
    let mut current_language = None;
    let mut current_lines = Vec::new();

    let finish_section = |sections: &mut Vec<(Language, String)>,
                          current_language: &mut Option<Language>,
                          current_lines: &mut Vec<&str>| {
        let Some(language) = current_language.take() else {
            current_lines.clear();
            return;
        };
        let content = current_lines.join("\n").trim().to_owned();
        current_lines.clear();
        if !content.is_empty() {
            sections.push((language, content));
        }
    };

    for line in notes.lines() {
        let heading = line.trim();
        if let Some(label) = heading.strip_prefix("## ") {
            finish_section(&mut sections, &mut current_language, &mut current_lines);
            current_language = update_note_language(label.trim());
        } else if current_language.is_some() {
            current_lines.push(line);
        }
    }
    finish_section(&mut sections, &mut current_language, &mut current_lines);
    sections
}

fn update_note_language(label: &str) -> Option<Language> {
    match label {
        "English" | "en" => Some(Language::English),
        "日本語" | "ja" => Some(Language::Japanese),
        "简体中文" | "zh" | "zh-CN" => Some(Language::SimplifiedChinese),
        _ => None,
    }
}

impl From<&PreparedUpdate> for PreparedUpdateSnapshot {
    fn from(update: &PreparedUpdate) -> Self {
        Self {
            component: update.component(),
            version: update.version().to_string(),
        }
    }
}

fn hotkeys_snapshot(hotkeys: GlobalHotkeys) -> GlobalHotkeysSnapshot {
    GlobalHotkeysSnapshot {
        enabled: hotkeys.enabled,
        bindings: GlobalHotkeyAction::all()
            .iter()
            .copied()
            .map(|action| GlobalHotkeyBindingSnapshot {
                action,
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

#[cfg(test)]
mod tests {
    use super::*;
    use nte_dps_tool::storage::config::UiConfig;

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
            true,
            CaptureLogStats::default(),
            true,
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
            CombatClockRuntimeHealth::Unknown,
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

    #[test]
    fn update_notes_select_only_the_active_language_section() {
        let notes = "# Release\n\n## 简体中文\n\n中文更新\n\n## English\n\nEnglish update\n\n## 日本語\n\n日本語の更新";

        assert_eq!(
            localized_update_notes(notes, Language::SimplifiedChinese),
            "中文更新"
        );
        assert_eq!(
            localized_update_notes(notes, Language::English),
            "English update"
        );
        assert_eq!(
            localized_update_notes(notes, Language::Japanese),
            "日本語の更新"
        );
    }

    #[test]
    fn update_notes_keep_legacy_unsectioned_text_and_use_english_fallback() {
        assert_eq!(
            localized_update_notes("Updater client", Language::Japanese),
            "Updater client"
        );

        let notes = "## English\n\nEnglish update\n\n## 简体中文\n\n中文更新";
        assert_eq!(
            localized_update_notes(notes, Language::Japanese),
            "English update"
        );
    }
}
