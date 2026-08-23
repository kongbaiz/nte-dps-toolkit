use serde::Serialize;

use nte_dps_tool::{
    core::{
        CoreError,
        hud::{HudCharacterSnapshot, HudDataState, HudSnapshot, HudSummarySnapshot},
    },
    engine::model::{CharacterInfo, DamageAttributionSummary},
    storage::{
        config::{AccentColor, ThemePreset, UiConfig, UiDensity},
        history::MAX_HISTORY_RECORDS,
        i18n::Language,
    },
};

use crate::{
    contract::{CaptureSnapshot, dps_time::DpsTimeRuntimeSnapshot},
    state::{AppState, HistoryRoundIndex, MainDpsReadout},
};

pub(crate) const MAIN_DPS_CONTRACT_VERSION: u32 = 7;
pub(crate) const MAIN_DPS_MAX_TEXT_BYTES: usize = 256;
pub(crate) const MAIN_DPS_MAX_PROJECTED_TEXT_BYTES: usize = 128 * 1024;

#[derive(Default)]
struct MainDpsTextBudget {
    projected_bytes: usize,
    truncated: bool,
}

impl MainDpsTextBudget {
    fn text(&mut self, value: String, fallback: &str) -> String {
        let value = if value.len() <= MAIN_DPS_MAX_TEXT_BYTES {
            value
        } else {
            self.truncated = true;
            fallback.to_owned()
        };
        if self
            .projected_bytes
            .checked_add(value.len())
            .is_some_and(|next| next <= MAIN_DPS_MAX_PROJECTED_TEXT_BYTES)
        {
            self.projected_bytes += value.len();
            return value;
        }
        self.truncated = true;
        if self
            .projected_bytes
            .checked_add(fallback.len())
            .is_some_and(|next| next <= MAIN_DPS_MAX_PROJECTED_TEXT_BYTES)
        {
            self.projected_bytes += fallback.len();
            fallback.to_owned()
        } else {
            String::new()
        }
    }

    fn optional(&mut self, value: Option<String>) -> Option<String> {
        value.and_then(|value| {
            let bounded = self.text(value, "");
            (!bounded.is_empty()).then_some(bounded)
        })
    }
}

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
    pub dps_time: DpsTimeRuntimeSnapshot,
    pub processing_paused: bool,
    pub paused_pending_events: String,
    pub paused_debug_packets: String,
    pub replay_running: bool,
    pub always_on_top: bool,
    pub passthrough: bool,
    pub appearance: MainDpsAppearanceSnapshot,
    pub display: MainDpsDisplaySnapshot,
    pub rounds: Vec<MainDpsRoundSnapshot>,
    pub selected_round_id: Option<String>,
    pub readout: MainDpsReadoutSnapshot,
    pub actions: MainDpsActionsSnapshot,
    pub game_detected: bool,
    pub game_detection_status: GameDetectionStatus,
    pub has_live_session_data: bool,
    pub onboarding: MainDpsOnboardingSnapshot,
    pub text_truncated: bool,
}

impl MainDpsSnapshot {
    pub(crate) fn from_state(state: &AppState) -> Result<Self, CoreError> {
        let generation = state.next_sequence();
        let revisions = state.main_dps_stream_revision()?;
        let config = state.ui_config_snapshot();
        let capture_status = state.live_capture_status();
        let replay_running = state.replay_running()?;
        let processing_paused = state.main_processing_paused();
        let (paused_pending_events, paused_debug_packets) = state.main_paused_event_counts()?;
        let always_on_top = state.window_always_on_top(crate::state::DesktopWindowKind::MainDps);
        let rounds = state.main_round_index();
        let selected_round_id = state.main_selected_round_id();
        let mut text_budget = MainDpsTextBudget::default();
        let round_snapshots = round_snapshots(
            rounds.as_ref(),
            selected_round_id.as_deref(),
            &mut text_budget,
        );
        let selected_round_id = text_budget.optional(selected_round_id);
        let MainDpsReadout {
            hud,
            has_hits,
            game_paused,
            damage_attribution,
            separate_reaction_damage,
            character_durations,
        } = state.main_dps_readout()?;
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
        let has_live_session_data = state.session_has_data()?;
        let can_import_replay = !capture_active && !replay_running;
        let game_detection_status = resolve_game_detection_status(data_empty, || {
            nte_dps_tool::platform::network::game_process_is_running()
        });
        let game_detected = game_detection_status == GameDetectionStatus::Running;

        Ok(Self {
            contract_version: MAIN_DPS_CONTRACT_VERSION,
            generation: generation.to_string(),
            capture_generation: revisions.capture.to_string(),
            presentation_generation: revisions.presentation.to_string(),
            history_generation: revisions.history.to_string(),
            adapter_version: env!("CARGO_PKG_VERSION"),
            capture: capture_status.into(),
            dps_time: DpsTimeRuntimeSnapshot::new(
                config.dps_time_mode,
                state.main_presented_combat_clock_health()?,
            ),
            processing_paused,
            paused_pending_events: paused_pending_events.to_string(),
            paused_debug_packets: paused_debug_packets.to_string(),
            replay_running,
            always_on_top,
            passthrough: state.passthrough(),
            appearance: MainDpsAppearanceSnapshot::from(&config),
            display: MainDpsDisplaySnapshot {
                metrics: config
                    .main_dps_display
                    .metrics
                    .iter()
                    .map(|value| value.id())
                    .collect(),
                attributions: config
                    .main_dps_display
                    .attributions
                    .iter()
                    .map(|value| value.id())
                    .collect(),
            },
            rounds: round_snapshots,
            selected_round_id,
            readout: MainDpsReadoutSnapshot::from_hud(
                hud,
                &resources.characters,
                config.language,
                damage_attribution,
                separate_reaction_damage,
                &character_durations,
                &mut text_budget,
            ),
            actions: MainDpsActionsSnapshot {
                can_start_capture: !capture_active && !replay_running,
                can_stop_capture: capture_active || replay_running,
                can_reset: live_round_selected && has_hits,
                can_start_new_round: can_start_new_round(
                    capture_status.phase,
                    replay_running,
                    live_round_selected,
                    processing_paused,
                    game_paused,
                    abyss_detected,
                    has_hits,
                ),
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
            onboarding: {
                let (capture_device_count, capture_devices_available) =
                    state.capture_device_catalog_status();
                MainDpsOnboardingSnapshot {
                    done: config.onboarding_done,
                    step: state.onboarding_step(),
                    capture_device_count,
                    capture_devices_available,
                    game_detected,
                    game_detection_status,
                    passthrough_hotkey_label: state.passthrough_hotkey().label(),
                    passthrough_hotkey_ready: state.passthrough_hotkey_ready(),
                }
            },
            text_truncated: text_budget.truncated,
        })
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

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct MainDpsDisplaySnapshot {
    pub metrics: Vec<&'static str>,
    pub attributions: Vec<&'static str>,
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
        text_budget: &mut MainDpsTextBudget,
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
                        text_budget,
                    )
                })
                .collect(),
            damage_attribution: MainDpsDamageAttributionSnapshot {
                total_damage: damage_attribution.total_damage,
                max_hp_reduction: damage_attribution.max_hp_reduction,
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
        text_budget: &mut MainDpsTextBudget,
    ) -> Self {
        let info = characters.get(&row.character_id);
        let duration_seconds = character_durations
            .get(&row.character_id)
            .copied()
            .unwrap_or_default();
        Self {
            character_id: row.character_id,
            name: text_budget.text(
                localized_character_name(row.character_id, info, language, &row.name),
                &format!("Character {}", row.character_id),
            ),
            hits: row.hits,
            damage: row.damage,
            dps: row.dps,
            damage_share_percent: row.damage_share_percent,
            damage_taken: row.damage_taken,
            duration_seconds,
            color: text_budget.optional(info.and_then(|value| value.color.clone())),
            attribute: text_budget.optional(info.and_then(|value| value.attribute.clone())),
        }
    }
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct MainDpsDamageAttributionSnapshot {
    pub total_damage: f64,
    pub max_hp_reduction: f64,
    pub character_direct_damage: f64,
    pub character_reaction_damage: f64,
    pub shared_damage: f64,
    pub unattributed_damage: f64,
    pub separate_reaction_damage: bool,
}

fn localized_character_name(
    _character_id: u32,
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
    pub capture_devices_available: bool,
    pub game_detected: bool,
    pub game_detection_status: GameDetectionStatus,
    pub passthrough_hotkey_label: String,
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
    records: &[HistoryRoundIndex],
    selected_round_id: Option<&str>,
    text_budget: &mut MainDpsTextBudget,
) -> Vec<MainDpsRoundSnapshot> {
    let mut visible = records
        .iter()
        .filter(|record| record.has_details)
        .take(MAX_HISTORY_RECORDS)
        .collect::<Vec<_>>();
    if let Some(selected_round_id) = selected_round_id
        && !visible.iter().any(|record| record.id == selected_round_id)
        && let Some(selected) = records
            .iter()
            .find(|record| record.id == selected_round_id && record.has_details)
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
            id: Some(text_budget.text(record.id.clone(), "")),
            live: false,
            display_time: text_budget.optional(Some(record.display_time.clone())),
            abyss_floor: record.abyss_floor,
        })
        .chain(std::iter::once(MainDpsRoundSnapshot {
            id: None,
            live: true,
            display_time: None,
            abyss_floor: None,
        }))
        .collect()
}

fn can_start_new_round(
    capture_phase: nte_dps_tool::core::live_capture::LiveCapturePhase,
    replay_running: bool,
    live_round_selected: bool,
    processing_paused: bool,
    game_paused: bool,
    abyss_detected: bool,
    has_hits: bool,
) -> bool {
    live_round_selected
        && matches!(
            capture_phase,
            nte_dps_tool::core::live_capture::LiveCapturePhase::Running
        )
        && !replay_running
        && !processing_paused
        && !game_paused
        && !abyss_detected
        && has_hits
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
    use crate::{
        channels::stream_runtime::serialize_stream_events,
        contract::stream::MAX_STREAM_DELIVERY_BYTES,
    };

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
        let snapshot = MainDpsSnapshot::from_state(&AppState::default())
            .expect("healthy live-capture main DPS snapshot");
        let value = serde_json::to_value(snapshot).expect("snapshot serializes");

        assert_eq!(value["contractVersion"], MAIN_DPS_CONTRACT_VERSION);
        assert!(value["generation"].as_str().is_some());
        assert_eq!(value["rounds"][0]["live"], true);
        assert_eq!(value["readout"]["dataState"], "empty");
        assert_eq!(value["display"]["metrics"][0], "team-dps");
        assert_eq!(value["display"]["attributions"][4], "max-hp-reduction");
        assert_eq!(value["readout"]["damageAttribution"]["maxHpReduction"], 0.0);
        assert_eq!(
            value["readout"]["characters"].as_array().map(Vec::len),
            Some(0)
        );
    }

    #[test]
    fn round_snapshots_follow_previous_next_chronology() {
        let newest = test_round("newest", true);
        let older = test_round("older", true);

        let rounds = round_snapshots(&[newest, older], None, &mut MainDpsTextBudget::default());
        assert_eq!(rounds.len(), 3);
        assert_eq!(rounds[0].id.as_deref(), Some("older"));
        assert_eq!(rounds[1].id.as_deref(), Some("newest"));
        assert!(rounds[2].live);
        assert!(rounds[2].id.is_none());
    }

    #[test]
    fn round_snapshots_cap_history_and_always_append_one_live_row() {
        let records = (0..=MAX_HISTORY_RECORDS)
            .map(|index| test_round(&format!("record-{index}"), true))
            .collect::<Vec<_>>();

        let rounds = round_snapshots(&records, None, &mut MainDpsTextBudget::default());

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
            .map(|index| test_round(&format!("record-{index}"), index != MAX_HISTORY_RECORDS / 2))
            .collect::<Vec<_>>();

        let rounds = round_snapshots(&records, None, &mut MainDpsTextBudget::default());
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
            .map(|index| test_round(&format!("record-{index}"), true))
            .collect::<Vec<_>>();
        let selected_id = format!("record-{MAX_HISTORY_RECORDS}");

        let rounds = round_snapshots(
            &records,
            Some(&selected_id),
            &mut MainDpsTextBudget::default(),
        );

        assert_eq!(rounds.len(), MAX_HISTORY_RECORDS + 1);
        assert!(
            rounds
                .iter()
                .any(|row| row.id.as_deref() == Some(selected_id.as_str()))
        );
        assert!(rounds.last().is_some_and(|row| row.live));
    }

    #[test]
    fn replay_is_read_only_for_manual_new_round_actions() {
        assert!(!can_start_new_round(
            nte_dps_tool::core::live_capture::LiveCapturePhase::Running,
            true,
            true,
            false,
            false,
            false,
            true,
        ));
        assert!(can_start_new_round(
            nte_dps_tool::core::live_capture::LiveCapturePhase::Running,
            false,
            true,
            false,
            false,
            false,
            true,
        ));
    }

    fn test_round(id: &str, has_details: bool) -> HistoryRoundIndex {
        HistoryRoundIndex::for_test(id, has_details)
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
            localized_character_name(1, Some(&info), Language::SimplifiedChinese, "fallback"),
            "娜娜莉"
        );
        assert_eq!(
            localized_character_name(1, Some(&info), Language::English, "fallback"),
            "Nanally"
        );
        assert_eq!(
            localized_character_name(1, Some(&info), Language::Japanese, "fallback"),
            "Nanally"
        );

        let within_budget = CharacterInfo {
            name_zh: "界".repeat(43),
            name_en: "x".repeat(129),
            color: None,
            avatar: None,
            attribute: None,
        };
        let mut budget = MainDpsTextBudget::default();
        assert_eq!(
            budget.text(
                localized_character_name(
                    7,
                    Some(&within_budget),
                    Language::SimplifiedChinese,
                    "fallback",
                ),
                "fallback",
            ),
            "界".repeat(43)
        );
        assert!(!budget.truncated);
    }

    #[test]
    fn character_name_budget_accepts_exact_utf8_limit_and_rejects_one_byte_over() {
        let exact_limit = format!("{}x", "界".repeat(85));
        assert_eq!(exact_limit.len(), MAIN_DPS_MAX_TEXT_BYTES);
        let mut budget = MainDpsTextBudget::default();
        assert_eq!(budget.text(exact_limit.clone(), "fallback"), exact_limit);
        assert!(!budget.truncated);

        let one_byte_over = format!("{}xy", "界".repeat(85));
        assert_eq!(one_byte_over.len(), MAIN_DPS_MAX_TEXT_BYTES + 1);
        let mut budget = MainDpsTextBudget::default();
        assert_eq!(budget.text(one_byte_over, "fallback"), "fallback");
        assert!(budget.truncated);
    }

    #[test]
    fn resource_character_fields_are_bounded_with_explicit_omission() {
        let oversized = "界".repeat(MAIN_DPS_MAX_TEXT_BYTES);
        let info = CharacterInfo {
            name_zh: oversized.clone(),
            name_en: oversized.clone(),
            color: Some(oversized.clone()),
            avatar: None,
            attribute: Some(oversized),
        };
        let characters = std::collections::HashMap::from([(7, info)]);
        let row = HudCharacterSnapshot {
            character_id: 7,
            name: "fallback".to_owned(),
            preview_label_suffix: None,
            hits: "1".to_owned(),
            damage: 1.0,
            dps: 1.0,
            damage_share_percent: 100.0,
            damage_taken: 0.0,
            color: None,
        };
        let mut budget = MainDpsTextBudget::default();

        let projected = MainDpsCharacterSnapshot::from_hud(
            row,
            &characters,
            Language::SimplifiedChinese,
            &std::collections::HashMap::new(),
            &mut budget,
        );

        assert_eq!(projected.name, "Character 7");
        assert!(projected.color.is_none());
        assert!(projected.attribute.is_none());
        assert!(budget.truncated);
    }

    #[test]
    fn maximum_escape_heavy_main_snapshot_stays_below_stream_budget() {
        let escape_heavy = "\u{0001}".repeat(MAIN_DPS_MAX_TEXT_BYTES);
        let mut snapshot =
            MainDpsSnapshot::from_state(&AppState::default()).expect("healthy main DPS snapshot");
        snapshot.rounds = (0..MAX_HISTORY_RECORDS)
            .map(|index| MainDpsRoundSnapshot {
                id: Some(format!(
                    "{index:03}{}",
                    "\u{0001}".repeat(MAIN_DPS_MAX_TEXT_BYTES - 3)
                )),
                live: false,
                display_time: Some(escape_heavy.clone()),
                abyss_floor: None,
            })
            .chain(std::iter::once(MainDpsRoundSnapshot {
                id: None,
                live: true,
                display_time: None,
                abyss_floor: None,
            }))
            .collect();
        snapshot.readout.characters = (0..4)
            .map(|character_id| MainDpsCharacterSnapshot {
                character_id,
                name: escape_heavy.clone(),
                hits: "18446744073709551615".to_owned(),
                damage: 1.0,
                dps: 1.0,
                damage_share_percent: 25.0,
                damage_taken: 0.0,
                duration_seconds: 1.0,
                color: Some(escape_heavy.clone()),
                attribute: Some(escape_heavy.clone()),
            })
            .collect();

        let delivery = serialize_stream_events(vec![MainDpsEvent::Snapshot(snapshot)])
            .expect("bounded main snapshot must serialize");
        assert!(
            delivery.len() < MAX_STREAM_DELIVERY_BYTES,
            "MAIN_DPS_MAX_STREAM_BYTES={} limit={MAX_STREAM_DELIVERY_BYTES}",
            delivery.len()
        );
    }
}
