use serde::Serialize;

use nte_dps_tool::{
    core::hud::{HudCharacterSnapshot, HudDataState, HudSnapshot, HudSummarySnapshot},
    engine::model::{CharacterInfo, DamageAttributionSummary},
    storage::{
        config::{AccentColor, ThemePreset, UiConfig, UiDensity},
        history::HistoryRecord,
        i18n::Language,
    },
};

use crate::{
    contract::CaptureSnapshot,
    state::{AppState, MainDpsReadout},
};

pub(crate) const MAIN_DPS_CONTRACT_VERSION: u32 = 2;

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct MainDpsSnapshot {
    pub contract_version: u32,
    pub generation: String,
    pub capture_generation: String,
    pub presentation_generation: String,
    pub history_generation: String,
    pub adapter_version: &'static str,
    pub capture: CaptureSnapshot,
    pub processing_paused: bool,
    pub replay_running: bool,
    pub always_on_top: bool,
    pub passthrough: bool,
    pub appearance: MainDpsAppearanceSnapshot,
    pub rounds: Vec<MainDpsRoundSnapshot>,
    pub selected_round_id: Option<String>,
    pub readout: MainDpsReadoutSnapshot,
    pub actions: MainDpsActionsSnapshot,
    pub game_detected: bool,
}

impl MainDpsSnapshot {
    pub(crate) fn from_state(state: &AppState) -> Self {
        let generation = state.next_sequence();
        let revisions = state.main_dps_stream_revision();
        let config = state.ui_config_snapshot();
        let capture_status = state.live_capture_status();
        let replay_running = state.replay_running();
        let processing_paused = state.main_processing_paused();
        let always_on_top = state.always_on_top();
        let rounds = state.main_round_records();
        let selected_round_id = state.main_selected_round_id();
        let MainDpsReadout {
            hud,
            has_hits,
            game_paused,
            damage_attribution,
            separate_reaction_damage,
            character_durations,
        } = state.main_dps_readout(&rounds, selected_round_id.as_deref());
        let resources = state.live_capture_resources();
        let data_empty = matches!(hud.data_state, HudDataState::Empty);
        let abyss_detected = hud.status.abyss_detected;
        let capture_active = matches!(
            capture_status.phase,
            nte_dps_tool::core::live_capture::LiveCapturePhase::Starting
                | nte_dps_tool::core::live_capture::LiveCapturePhase::Running
                | nte_dps_tool::core::live_capture::LiveCapturePhase::Stopping
        );
        let live_round_selected = selected_round_id.is_none();
        let can_import_replay = !capture_active && !replay_running;
        let game_detected = if data_empty {
            nte_dps_tool::platform::network::game_process_is_running().unwrap_or(false)
        } else {
            true
        };

        Self {
            contract_version: MAIN_DPS_CONTRACT_VERSION,
            generation: generation.to_string(),
            capture_generation: revisions.capture.to_string(),
            presentation_generation: revisions.presentation.to_string(),
            history_generation: revisions.history.to_string(),
            adapter_version: env!("CARGO_PKG_VERSION"),
            capture: capture_status.into(),
            processing_paused,
            replay_running,
            always_on_top,
            passthrough: state.passthrough(),
            appearance: MainDpsAppearanceSnapshot::from(&config),
            rounds: round_snapshots(&rounds),
            selected_round_id,
            readout: MainDpsReadoutSnapshot::from_hud(
                hud,
                &resources.characters,
                config.language,
                damage_attribution,
                separate_reaction_damage,
                &character_durations,
            ),
            actions: MainDpsActionsSnapshot {
                can_start_capture: !capture_active && !replay_running,
                can_stop_capture: capture_active || replay_running,
                can_reset: live_round_selected && has_hits,
                can_start_new_round: live_round_selected
                    && matches!(
                        capture_status.phase,
                        nte_dps_tool::core::live_capture::LiveCapturePhase::Running
                    )
                    && !processing_paused
                    && !game_paused
                    && !abyss_detected
                    && has_hits,
                can_pause: live_round_selected
                    && (capture_active || replay_running)
                    && !processing_paused,
                can_resume: live_round_selected && processing_paused,
                can_import_replay,
                character_details_available: has_hits,
                team_details_available: has_hits,
            },
            game_detected,
        }
    }
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct MainDpsAppearanceSnapshot {
    pub language: &'static str,
    pub dark_mode: bool,
    pub theme_preset: &'static str,
    pub accent: &'static str,
    pub density: &'static str,
    pub reduce_motion: bool,
    pub opacity: f32,
}

impl From<&UiConfig> for MainDpsAppearanceSnapshot {
    fn from(config: &UiConfig) -> Self {
        Self {
            language: config.language.code(),
            dark_mode: config.dark_mode,
            theme_preset: theme_preset_id(config.theme_preset),
            accent: accent_id(config.accent),
            density: density_id(config.density),
            reduce_motion: config.reduce_motion,
            opacity: config.opacity,
        }
    }
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct MainDpsRoundSnapshot {
    pub id: Option<String>,
    pub live: bool,
    pub display_time: Option<String>,
    pub abyss_floor: Option<u32>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct MainDpsReadoutSnapshot {
    pub data_state: &'static str,
    pub summary: MainDpsSummarySnapshot,
    pub characters: Vec<MainDpsCharacterSnapshot>,
    pub damage_attribution: MainDpsDamageAttributionSnapshot,
    pub abyss: MainDpsAbyssSnapshot,
}

impl MainDpsReadoutSnapshot {
    fn from_hud(
        hud: HudSnapshot,
        characters: &std::collections::HashMap<u32, CharacterInfo>,
        language: Language,
        damage_attribution: DamageAttributionSummary,
        separate_reaction_damage: bool,
        character_durations: &std::collections::HashMap<u32, f64>,
    ) -> Self {
        let summary = hud.summary.unwrap_or(HudSummarySnapshot {
            team_dps: 0.0,
            duration_seconds: 0.0,
            total_damage: 0.0,
            total_damage_taken: 0.0,
        });
        Self {
            data_state: match hud.data_state {
                HudDataState::Empty => "empty",
                HudDataState::Preview => "preview",
                HudDataState::Live => "live",
            },
            summary: MainDpsSummarySnapshot {
                team_dps: summary.team_dps,
                duration_seconds: summary.duration_seconds,
                total_damage: summary.total_damage,
                total_damage_taken: summary.total_damage_taken,
            },
            characters: hud
                .characters
                .into_iter()
                .map(|row| {
                    MainDpsCharacterSnapshot::from_hud(
                        row,
                        characters,
                        language,
                        character_durations,
                    )
                })
                .collect(),
            damage_attribution: MainDpsDamageAttributionSnapshot {
                total_damage: damage_attribution.total_damage,
                character_direct_damage: damage_attribution.character_direct_damage,
                character_reaction_damage: damage_attribution.character_reaction_damage,
                shared_damage: damage_attribution.shared_damage,
                unattributed_damage: damage_attribution.unattributed_damage,
                separate_reaction_damage,
            },
            abyss: MainDpsAbyssSnapshot {
                detected: hud.status.abyss_detected,
                floor: hud.status.abyss_floor,
                half: hud.status.abyss_half.map(|half| match half {
                    nte_dps_tool::core::hud::HudAbyssHalf::First => "first",
                    nte_dps_tool::core::hud::HudAbyssHalf::Second => "second",
                }),
                success: hud.status.abyss_success,
            },
        }
    }
}

#[derive(Clone, Debug, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct MainDpsSummarySnapshot {
    pub team_dps: f64,
    pub duration_seconds: f64,
    pub total_damage: f64,
    pub total_damage_taken: f64,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct MainDpsCharacterSnapshot {
    pub character_id: u32,
    pub name: String,
    pub hits: String,
    pub damage: f64,
    pub dps: f64,
    pub damage_share_percent: f64,
    pub damage_taken: f64,
    pub duration_seconds: f64,
    pub color: Option<String>,
    pub attribute: Option<String>,
}

impl MainDpsCharacterSnapshot {
    fn from_hud(
        row: HudCharacterSnapshot,
        characters: &std::collections::HashMap<u32, CharacterInfo>,
        language: Language,
        character_durations: &std::collections::HashMap<u32, f64>,
    ) -> Self {
        let info = characters.get(&row.character_id);
        let duration_seconds = character_durations
            .get(&row.character_id)
            .copied()
            .unwrap_or_default();
        Self {
            character_id: row.character_id,
            name: localized_character_name(info, language, &row.name),
            hits: row.hits,
            damage: row.damage,
            dps: row.dps,
            damage_share_percent: row.damage_share_percent,
            damage_taken: row.damage_taken,
            duration_seconds,
            color: info.and_then(|value| value.color.clone()),
            attribute: info.and_then(|value| value.attribute.clone()),
        }
    }
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct MainDpsDamageAttributionSnapshot {
    pub total_damage: f64,
    pub character_direct_damage: f64,
    pub character_reaction_damage: f64,
    pub shared_damage: f64,
    pub unattributed_damage: f64,
    pub separate_reaction_damage: bool,
}

fn localized_character_name(
    info: Option<&CharacterInfo>,
    language: Language,
    fallback: &str,
) -> String {
    let candidate = info.map(|value| match language {
        Language::SimplifiedChinese => value.name_zh.trim(),
        Language::English | Language::Japanese => value.name_en.trim(),
    });
    candidate
        .filter(|value| !value.is_empty())
        .unwrap_or(fallback)
        .to_owned()
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct MainDpsAbyssSnapshot {
    pub detected: bool,
    pub floor: Option<u32>,
    pub half: Option<&'static str>,
    pub success: bool,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct MainDpsActionsSnapshot {
    pub can_start_capture: bool,
    pub can_stop_capture: bool,
    pub can_reset: bool,
    pub can_start_new_round: bool,
    pub can_pause: bool,
    pub can_resume: bool,
    pub can_import_replay: bool,
    pub character_details_available: bool,
    pub team_details_available: bool,
}

#[derive(Clone, Debug, Serialize)]
#[serde(tag = "event", content = "payload", rename_all = "camelCase")]
pub(crate) enum MainDpsEvent {
    Snapshot(MainDpsSnapshot),
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct MainDpsActionResult {
    pub performed: bool,
    pub snapshot: MainDpsSnapshot,
}

fn round_snapshots(records: &[HistoryRecord]) -> Vec<MainDpsRoundSnapshot> {
    std::iter::once(MainDpsRoundSnapshot {
        id: None,
        live: true,
        display_time: None,
        abyss_floor: None,
    })
    .chain(records.iter().filter_map(|record| {
        record.details.as_ref()?;
        Some(MainDpsRoundSnapshot {
            id: Some(record.id.clone()),
            live: false,
            display_time: Some(record.display_time()),
            abyss_floor: record.summary.abyss.floor,
        })
    }))
    .collect()
}

fn theme_preset_id(value: ThemePreset) -> &'static str {
    match value {
        ThemePreset::Zinc => "zinc",
        ThemePreset::Tactical => "tactical",
        ThemePreset::HighContrast => "high-contrast",
    }
}

fn accent_id(value: AccentColor) -> &'static str {
    match value {
        AccentColor::Zinc => "zinc",
        AccentColor::Blue => "blue",
        AccentColor::Violet => "violet",
        AccentColor::Orange => "orange",
        AccentColor::Green => "green",
    }
}

fn density_id(value: UiDensity) -> &'static str {
    match value {
        UiDensity::Compact => "compact",
        UiDensity::Cozy => "cozy",
        UiDensity::Comfortable => "comfortable",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_snapshot_is_bounded_and_uses_string_generations() {
        let snapshot = MainDpsSnapshot::from_state(&AppState::default());
        let value = serde_json::to_value(snapshot).expect("snapshot serializes");

        assert_eq!(value["contractVersion"], MAIN_DPS_CONTRACT_VERSION);
        assert!(value["generation"].as_str().is_some());
        assert_eq!(value["rounds"][0]["live"], true);
        assert_eq!(value["readout"]["dataState"], "empty");
        assert_eq!(
            value["readout"]["characters"].as_array().map(Vec::len),
            Some(0)
        );
    }

    #[test]
    fn character_name_tracks_the_snapshot_language() {
        let info = CharacterInfo {
            name_zh: "娜娜莉".to_owned(),
            name_en: "Nanally".to_owned(),
            color: None,
            avatar: None,
            attribute: None,
        };

        assert_eq!(
            localized_character_name(Some(&info), Language::SimplifiedChinese, "fallback"),
            "娜娜莉"
        );
        assert_eq!(
            localized_character_name(Some(&info), Language::English, "fallback"),
            "Nanally"
        );
        assert_eq!(
            localized_character_name(Some(&info), Language::Japanese, "fallback"),
            "Nanally"
        );
    }
}
