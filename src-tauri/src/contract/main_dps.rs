use serde::Serialize;

use nte_dps_tool::{
    core::hud::{HudCharacterSnapshot, HudDataState, HudSnapshot, HudSummarySnapshot},
    engine::model::{CharacterInfo, DamageAttributionSummary},
    storage::{
        config::{AccentColor, ThemePreset, UiConfig, UiDensity},
        history::{HistoryRecord, MAX_HISTORY_RECORDS},
        i18n::Language,
    },
};

use crate::{
    contract::CaptureSnapshot,
    state::{AppState, MainDpsReadout},
};

pub(crate) const MAIN_DPS_CONTRACT_VERSION: u32 = 4;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) enum GameDetectionStatus {
    Running,
    NotRunning,
    ProbeFailed,
}

fn resolve_game_detection_status(
    data_empty: bool,
    probe: impl FnOnce() -> Result<bool, String>,
) -> GameDetectionStatus {
    if !data_empty {
        return GameDetectionStatus::Running;
    }
    match probe() {
        Ok(true) => GameDetectionStatus::Running,
        Ok(false) => GameDetectionStatus::NotRunning,
        Err(error) => {
            log::warn!("game process detection failed while projecting Main DPS: {error}");
            GameDetectionStatus::ProbeFailed
        }
    }
}

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
    pub paused_pending_events: String,
    pub paused_debug_packets: String,
    pub replay_running: bool,
    pub always_on_top: bool,
    pub passthrough: bool,
    pub appearance: MainDpsAppearanceSnapshot,
    pub rounds: Vec<MainDpsRoundSnapshot>,
    pub selected_round_id: Option<String>,
    pub readout: MainDpsReadoutSnapshot,
    pub actions: MainDpsActionsSnapshot,
    pub game_detected: bool,
    pub game_detection_status: GameDetectionStatus,
    pub has_live_session_data: bool,
    pub onboarding: MainDpsOnboardingSnapshot,
}

impl MainDpsSnapshot {
    pub(crate) fn from_state(state: &AppState) -> Self {
        let generation = state.next_sequence();
        let revisions = state.main_dps_stream_revision();
        let config = state.ui_config_snapshot();
        let capture_status = state.live_capture_status();
        let replay_running = state.replay_running();
        let processing_paused = state.main_processing_paused();
        let (paused_pending_events, paused_debug_packets) = state.main_paused_event_counts();
        let always_on_top = state.window_always_on_top(crate::state::DesktopWindowKind::MainDps);
        let rounds = state.main_round_records();
        let selected_round_id = state.main_selected_round_id();
        let round_snapshots = round_snapshots(&rounds, selected_round_id.as_deref());
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
        let has_live_session_data = state.session_has_data();
        let can_import_replay = !capture_active && !replay_running;
        let game_detection_status = resolve_game_detection_status(data_empty, || {
            nte_dps_tool::platform::network::game_process_is_running()
        });
        let game_detected = game_detection_status == GameDetectionStatus::Running;

        Self {
            contract_version: MAIN_DPS_CONTRACT_VERSION,
            generation: generation.to_string(),
            capture_generation: revisions.capture.to_string(),
            presentation_generation: revisions.presentation.to_string(),
            history_generation: revisions.history.to_string(),
            adapter_version: env!("CARGO_PKG_VERSION"),
            capture: capture_status.into(),
            processing_paused,
            paused_pending_events: paused_pending_events.to_string(),
            paused_debug_packets: paused_debug_packets.to_string(),
            replay_running,
            always_on_top,
            passthrough: state.passthrough(),
            appearance: MainDpsAppearanceSnapshot::from(&config),
            rounds: round_snapshots,
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
                start_capture_requires_confirmation: has_live_session_data,
                reset_requires_confirmation: capture_active || replay_running,
                import_replay_requires_confirmation: has_live_session_data
                    || capture_active
                    || replay_running,
            },
            game_detected,
            game_detection_status,
            has_live_session_data,
            onboarding: MainDpsOnboardingSnapshot {
                done: config.onboarding_done,
                step: state.onboarding_step(),
                capture_device_count: state.capture_device_count(),
                game_detected,
                game_detection_status,
                passthrough_hotkey_label: state.passthrough_hotkey().label(),
                passthrough_hotkey_ready: state.passthrough_hotkey_ready(),
            },
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
    pub start_capture_requires_confirmation: bool,
    pub reset_requires_confirmation: bool,
    pub import_replay_requires_confirmation: bool,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct MainDpsOnboardingSnapshot {
    pub done: bool,
    pub step: usize,
    pub capture_device_count: usize,
    pub game_detected: bool,
    pub game_detection_status: GameDetectionStatus,
    pub passthrough_hotkey_label: &'static str,
    pub passthrough_hotkey_ready: bool,
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

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct MainDpsResetResult {
    pub snapshot: MainDpsSnapshot,
    pub undo_token: Option<String>,
}

fn round_snapshots(
    records: &[HistoryRecord],
    selected_round_id: Option<&str>,
) -> Vec<MainDpsRoundSnapshot> {
    let mut visible = records
        .iter()
        .filter(|record| record.details.is_some())
        .take(MAX_HISTORY_RECORDS)
        .collect::<Vec<_>>();
    if let Some(selected_round_id) = selected_round_id
        && !visible.iter().any(|record| record.id == selected_round_id)
        && let Some(selected) = records
            .iter()
            .find(|record| record.id == selected_round_id && record.details.is_some())
    {
        if visible.len() == MAX_HISTORY_RECORDS {
            visible.pop();
        }
        visible.push(selected);
    }

    visible
        .into_iter()
        .rev()
        .map(|record| MainDpsRoundSnapshot {
            id: Some(record.id.clone()),
            live: false,
            display_time: Some(record.display_time()),
            abyss_floor: record.summary.abyss.floor,
        })
        .chain(std::iter::once(MainDpsRoundSnapshot {
            id: None,
            live: true,
            display_time: None,
            abyss_floor: None,
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
    fn game_detection_distinguishes_normal_negative_from_probe_failure() {
        assert_eq!(
            resolve_game_detection_status(true, || Ok(true)),
            GameDetectionStatus::Running
        );
        assert_eq!(
            resolve_game_detection_status(true, || Ok(false)),
            GameDetectionStatus::NotRunning
        );
        assert_eq!(
            resolve_game_detection_status(true, || Err("access denied".to_owned())),
            GameDetectionStatus::ProbeFailed
        );
        assert_eq!(
            resolve_game_detection_status(false, || Err("must not be called".to_owned())),
            GameDetectionStatus::Running
        );
    }

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
    fn round_snapshots_follow_previous_next_chronology() {
        let newest = HistoryRecord {
            id: "newest".to_owned(),
            details: Some(Default::default()),
            ..Default::default()
        };
        let older = HistoryRecord {
            id: "older".to_owned(),
            details: Some(Default::default()),
            ..Default::default()
        };

        let rounds = round_snapshots(&[newest, older], None);
        assert_eq!(rounds.len(), 3);
        assert_eq!(rounds[0].id.as_deref(), Some("older"));
        assert_eq!(rounds[1].id.as_deref(), Some("newest"));
        assert!(rounds[2].live);
        assert!(rounds[2].id.is_none());
    }

    #[test]
    fn round_snapshots_cap_history_and_always_append_one_live_row() {
        let records = (0..=MAX_HISTORY_RECORDS)
            .map(|index| HistoryRecord {
                id: format!("record-{index}"),
                details: Some(Default::default()),
                ..Default::default()
            })
            .collect::<Vec<_>>();

        let rounds = round_snapshots(&records, None);

        assert_eq!(rounds.len(), MAX_HISTORY_RECORDS + 1);
        assert_eq!(rounds.iter().filter(|row| row.live).count(), 1);
        assert!(
            rounds
                .last()
                .is_some_and(|row| row.live && row.id.is_none())
        );
        let omitted_id = format!("record-{MAX_HISTORY_RECORDS}");
        assert!(
            !rounds
                .iter()
                .any(|row| row.id.as_deref() == Some(omitted_id.as_str()))
        );
    }

    #[test]
    fn round_snapshots_skip_history_without_details_before_reversing() {
        let missing_details_id = format!("record-{}", MAX_HISTORY_RECORDS / 2);
        let records = (0..=MAX_HISTORY_RECORDS)
            .map(|index| HistoryRecord {
                id: format!("record-{index}"),
                details: if index == MAX_HISTORY_RECORDS / 2 {
                    None
                } else {
                    Some(Default::default())
                },
                ..Default::default()
            })
            .collect::<Vec<_>>();

        let rounds = round_snapshots(&records, None);
        let history_rows = rounds.iter().filter(|row| !row.live).collect::<Vec<_>>();

        assert_eq!(history_rows.len(), MAX_HISTORY_RECORDS);
        assert_eq!(rounds.iter().filter(|row| row.live).count(), 1);
        assert!(rounds.last().is_some_and(|row| row.live));
        assert!(
            !history_rows
                .iter()
                .any(|row| row.id.as_deref() == Some(missing_details_id.as_str()))
        );
    }

    #[test]
    fn round_snapshots_keep_the_selected_overflow_record_visible() {
        let records = (0..=MAX_HISTORY_RECORDS)
            .map(|index| HistoryRecord {
                id: format!("record-{index}"),
                details: Some(Default::default()),
                ..Default::default()
            })
            .collect::<Vec<_>>();
        let selected_id = format!("record-{MAX_HISTORY_RECORDS}");

        let rounds = round_snapshots(&records, Some(&selected_id));

        assert_eq!(rounds.len(), MAX_HISTORY_RECORDS + 1);
        assert!(
            rounds
                .iter()
                .any(|row| row.id.as_deref() == Some(selected_id.as_str()))
        );
        assert!(rounds.last().is_some_and(|row| row.live));
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
