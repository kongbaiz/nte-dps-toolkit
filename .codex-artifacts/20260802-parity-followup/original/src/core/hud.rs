//! Stable, UI-neutral projection used by every desktop HUD frontend.
//!
//! The projection deliberately contains aggregate readouts only. It never
//! exposes `CombatState`, packets, hits, mutable references, or GUI types.

use std::collections::{HashMap, HashSet, VecDeque};

use serde::Serialize;

use crate::{
    engine::model::{
        AbyssHalf, CharacterStats, CombatState, DpsTimeBasis, Hit, PartyCombatState,
        TEAM_DPS_MAX_MEMBERS, TimelineSeries, is_qte_follow_up_damage_hit,
    },
    storage::config::{HudConfig, HudModule},
};

pub const HUD_SNAPSHOT_VERSION: u32 = 2;
pub const HUD_TIMELINE_MAX_BUCKETS: usize = 60;
const HUD_PREVIEW_ROW_COUNT: usize = 4;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum HudDataState {
    Empty,
    Preview,
    Live,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum HudAbyssHalf {
    First,
    Second,
}

impl From<AbyssHalf> for HudAbyssHalf {
    fn from(value: AbyssHalf) -> Self {
        match value {
            AbyssHalf::First => Self::First,
            AbyssHalf::Second => Self::Second,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum HudModuleSnapshot {
    Title,
    Summary,
    Status,
    Characters,
    Timeline,
}

impl From<HudModule> for HudModuleSnapshot {
    fn from(value: HudModule) -> Self {
        match value {
            HudModule::Title => Self::Title,
            HudModule::Summary => Self::Summary,
            HudModule::Status => Self::Status,
            HudModule::Characters => Self::Characters,
            HudModule::Timeline => Self::Timeline,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HudConfigSnapshot {
    pub width: u16,
    pub module_order: Vec<HudModuleSnapshot>,
    pub show_title: bool,
    pub show_team_dps: bool,
    pub show_duration: bool,
    pub show_total_damage: bool,
    pub show_character_rows: bool,
    pub show_damage_taken: bool,
    pub show_abyss_half: bool,
    pub show_passthrough_state: bool,
    pub show_mini_timeline: bool,
}

impl From<&HudConfig> for HudConfigSnapshot {
    fn from(config: &HudConfig) -> Self {
        let config = config.clone().sanitized();
        Self {
            width: config.width,
            module_order: config
                .module_order
                .into_iter()
                .map(HudModuleSnapshot::from)
                .collect(),
            show_title: config.show_title,
            show_team_dps: config.show_team_dps,
            show_duration: config.show_duration,
            show_total_damage: config.show_total_damage,
            show_character_rows: config.show_character_rows,
            show_damage_taken: config.show_damage_taken,
            show_abyss_half: config.show_abyss_half,
            show_passthrough_state: config.show_passthrough_state,
            show_mini_timeline: config.show_mini_timeline,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HudSummarySnapshot {
    pub team_dps: f64,
    pub duration_seconds: f64,
    pub total_damage: f64,
    pub total_damage_taken: f64,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HudCharacterSnapshot {
    pub character_id: u32,
    pub name: String,
    pub preview_label_suffix: Option<String>,
    pub hits: String,
    pub damage: f64,
    pub dps: f64,
    pub damage_share_percent: f64,
    pub damage_taken: f64,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HudStatusSnapshot {
    pub abyss_detected: bool,
    pub abyss_floor: Option<u32>,
    pub abyss_half: Option<HudAbyssHalf>,
    pub abyss_success: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HudTimelineBucketSnapshot {
    pub start_seconds: f64,
    pub end_seconds: f64,
    pub damage: f64,
    pub dps: f64,
    pub hits: String,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HudTimelineSnapshot {
    pub bucket_seconds: f64,
    pub duration_seconds: f64,
    pub peak_dps: f64,
    pub buckets: Vec<HudTimelineBucketSnapshot>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HudSnapshot {
    pub version: u32,
    pub data_state: HudDataState,
    pub config: HudConfigSnapshot,
    pub summary: Option<HudSummarySnapshot>,
    pub characters: Vec<HudCharacterSnapshot>,
    pub status: HudStatusSnapshot,
    pub timeline: Option<HudTimelineSnapshot>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct HudProjectionOptions {
    pub dps_time_basis: DpsTimeBasis,
    pub separate_reaction_damage: bool,
    pub selected_abyss_half: Option<AbyssHalf>,
    pub preview_when_empty: bool,
    pub timeline_bucket_seconds: f64,
}

impl Default for HudProjectionOptions {
    fn default() -> Self {
        Self {
            dps_time_basis: DpsTimeBasis::SubtractTimeStop,
            separate_reaction_damage: false,
            selected_abyss_half: None,
            preview_when_empty: false,
            timeline_bucket_seconds: 1.0,
        }
    }
}

pub fn project_hud(
    state: &CombatState,
    config: &HudConfig,
    hidden_character_ids: &HashSet<u32>,
    options: HudProjectionOptions,
) -> HudSnapshot {
    let selected_half = state.abyss.is_active().then(|| {
        options
            .selected_abyss_half
            .or(state.abyss.active_half)
            .unwrap_or(AbyssHalf::First)
    });
    let subtract_time_stop = options.dps_time_basis.subtracts_time_stop();

    let (characters, summary) = selected_half.map_or_else(
        || {
            project_readout(
                &state.stats,
                &state.hits,
                state.total_damage,
                state.total_damage_taken,
                state.dps_with_time_stop(subtract_time_stop),
                state.duration_with_time_stop(subtract_time_stop),
                hidden_character_ids,
                options.separate_reaction_damage,
                |row| state.character_dps_with_time_stop(row, subtract_time_stop),
            )
        },
        |half| {
            let party = state.abyss.half(half);
            project_party_readout(
                party,
                hidden_character_ids,
                options.separate_reaction_damage,
                subtract_time_stop,
            )
        },
    );

    let has_readout = summary.total_damage > 0.0
        || summary.team_dps > 0.0
        || summary.total_damage_taken > 0.0
        || !characters.is_empty();
    let (data_state, summary, characters) = if has_readout {
        (HudDataState::Live, Some(summary), characters)
    } else if options.preview_when_empty {
        let (summary, characters) = preview_readout();
        (HudDataState::Preview, Some(summary), characters)
    } else {
        (HudDataState::Empty, None, Vec::new())
    };

    let config = HudConfigSnapshot::from(config);
    let timeline = if !config.show_mini_timeline {
        None
    } else {
        match data_state {
            HudDataState::Empty => None,
            HudDataState::Preview => Some(preview_timeline()),
            HudDataState::Live => {
                let series = selected_half.map_or_else(
                    || {
                        state.timeline(
                            options.timeline_bucket_seconds,
                            options.dps_time_basis.subtracts_time_stop(),
                        )
                    },
                    |half| {
                        state.abyss.half(half).timeline(
                            options.timeline_bucket_seconds,
                            options.dps_time_basis.subtracts_time_stop(),
                        )
                    },
                );
                project_timeline(series)
            }
        }
    };

    HudSnapshot {
        version: HUD_SNAPSHOT_VERSION,
        data_state,
        config,
        summary,
        characters,
        status: HudStatusSnapshot {
            abyss_detected: state.abyss.is_active(),
            abyss_floor: state.abyss.floor,
            abyss_half: selected_half.map(HudAbyssHalf::from),
            abyss_success: state.abyss.success_at.is_some(),
        },
        timeline,
    }
}

fn project_party_readout(
    party: &PartyCombatState,
    hidden_character_ids: &HashSet<u32>,
    separate_reaction_damage: bool,
    subtract_time_stop: bool,
) -> (Vec<HudCharacterSnapshot>, HudSummarySnapshot) {
    project_readout(
        &party.stats,
        &party.hits,
        party.total_damage,
        party.total_damage_taken,
        party.dps_with_time_stop(subtract_time_stop),
        party.duration_with_time_stop(subtract_time_stop),
        hidden_character_ids,
        separate_reaction_damage,
        |row| party.character_dps_with_time_stop(row, subtract_time_stop),
    )
}

#[allow(clippy::too_many_arguments)]
fn project_readout(
    stats: &HashMap<u32, CharacterStats>,
    hits: &VecDeque<Hit>,
    total_damage: f64,
    total_damage_taken: f64,
    team_dps: f64,
    duration_seconds: f64,
    hidden_character_ids: &HashSet<u32>,
    separate_reaction_damage: bool,
    character_dps: impl Fn(&CharacterStats) -> f64,
) -> (Vec<HudCharacterSnapshot>, HudSummarySnapshot) {
    let mut rows = stats
        .values()
        .filter(|row| {
            !hidden_character_ids.contains(&row.char_id)
                && hits
                    .iter()
                    .any(|hit| hit.char_id == row.char_id && !is_qte_follow_up_damage_hit(hit))
        })
        .map(|row| row.for_reaction_damage_policy(separate_reaction_damage))
        .filter(character_has_visible_totals)
        .map(|row| {
            let dps = character_dps(&row);
            HudCharacterSnapshot {
                character_id: row.char_id,
                name: row.name,
                preview_label_suffix: None,
                hits: row.hits.to_string(),
                damage: finite_nonnegative(row.damage),
                dps: finite_nonnegative(dps),
                damage_share_percent: percentage(row.damage, total_damage),
                damage_taken: finite_nonnegative(row.damage_taken),
            }
        })
        .collect::<Vec<_>>();
    rows.sort_by(|left, right| {
        right
            .damage
            .total_cmp(&left.damage)
            .then_with(|| left.character_id.cmp(&right.character_id))
    });
    rows.truncate(TEAM_DPS_MAX_MEMBERS);

    (
        rows,
        HudSummarySnapshot {
            team_dps: finite_nonnegative(team_dps),
            duration_seconds: finite_nonnegative(duration_seconds),
            total_damage: finite_nonnegative(total_damage),
            total_damage_taken: finite_nonnegative(total_damage_taken),
        },
    )
}

fn preview_readout() -> (HudSummarySnapshot, Vec<HudCharacterSnapshot>) {
    let values = [
        ("A", 38_u64, 1_227_500.0),
        ("B", 27_u64, 579_000.0),
        ("C", 16_u64, 260_000.0),
        ("D", 11_u64, 179_785.0),
    ];
    let total_damage = values.iter().map(|(_, _, damage)| damage).sum::<f64>();
    let duration_seconds = 34.9;
    let characters = values
        .into_iter()
        .enumerate()
        .map(|(index, (suffix, hits, damage))| HudCharacterSnapshot {
            character_id: u32::MAX - (HUD_PREVIEW_ROW_COUNT - index - 1) as u32,
            name: String::new(),
            preview_label_suffix: Some(suffix.to_owned()),
            hits: hits.to_string(),
            damage,
            dps: damage / duration_seconds,
            damage_share_percent: percentage(damage, total_damage),
            damage_taken: 0.0,
        })
        .collect();

    (
        HudSummarySnapshot {
            team_dps: 64_301.0,
            duration_seconds,
            total_damage,
            total_damage_taken: 2_834.0,
        },
        characters,
    )
}

fn preview_timeline() -> HudTimelineSnapshot {
    let dps_values = [
        16_000.0, 42_000.0, 28_000.0, 70_000.0, 48_000.0, 82_000.0, 36_000.0, 58_000.0, 24_000.0,
    ];
    let bucket_seconds = 1.0;
    let buckets = dps_values
        .into_iter()
        .enumerate()
        .map(|(index, dps)| HudTimelineBucketSnapshot {
            start_seconds: index as f64 * bucket_seconds,
            end_seconds: (index + 1) as f64 * bucket_seconds,
            damage: dps * bucket_seconds,
            dps,
            hits: "1".to_owned(),
        })
        .collect::<Vec<_>>();

    HudTimelineSnapshot {
        bucket_seconds,
        duration_seconds: buckets.last().map_or(0.0, |bucket| bucket.end_seconds),
        peak_dps: dps_values.into_iter().fold(0.0, f64::max),
        buckets,
    }
}

fn project_timeline(series: TimelineSeries) -> Option<HudTimelineSnapshot> {
    if series.buckets.is_empty() {
        return None;
    }

    let group_size = series
        .buckets
        .len()
        .div_ceil(HUD_TIMELINE_MAX_BUCKETS)
        .max(1);
    let buckets = series
        .buckets
        .chunks(group_size)
        .map(|group| {
            let start_seconds =
                finite_nonnegative(group.first().map_or(0.0, |bucket| bucket.start_offset));
            let end_seconds = finite_nonnegative(
                group
                    .last()
                    .map_or(start_seconds, |bucket| bucket.end_offset),
            )
            .max(start_seconds);
            let damage = finite_nonnegative(group.iter().map(|bucket| bucket.damage).sum());
            let hits = group
                .iter()
                .fold(0_u64, |total, bucket| total.saturating_add(bucket.hits));
            HudTimelineBucketSnapshot {
                start_seconds,
                end_seconds,
                damage,
                dps: finite_nonnegative(damage / (end_seconds - start_seconds).max(0.001)),
                hits: hits.to_string(),
            }
        })
        .collect::<Vec<_>>();
    let duration_seconds = buckets
        .last()
        .map_or(0.0, |bucket| bucket.end_seconds)
        .max(0.0);
    let peak_dps = buckets.iter().map(|bucket| bucket.dps).fold(0.0, f64::max);

    Some(HudTimelineSnapshot {
        bucket_seconds: finite_nonnegative(series.bucket_seconds * group_size as f64),
        duration_seconds,
        peak_dps,
        buckets,
    })
}

fn character_has_visible_totals(row: &CharacterStats) -> bool {
    row.hits > 0 || row.damage > 0.0 || row.hits_taken > 0 || row.damage_taken > 0.0
}

fn percentage(value: f64, total: f64) -> f64 {
    if value.is_finite() && total.is_finite() && total > 0.0 {
        (value / total * 100.0).clamp(0.0, 100.0)
    } else {
        0.0
    }
}

fn finite_nonnegative(value: f64) -> f64 {
    if value.is_finite() {
        value.max(0.0)
    } else {
        0.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::model::{HitCharacterSource, HitDirection};

    fn hit(timestamp: f64, character_id: u32, damage: f64) -> Hit {
        Hit {
            timestamp,
            char_id: character_id,
            char_name: format!("Character {character_id}"),
            char_known: true,
            damage,
            byte_offset: 0,
            bit_shift: 0,
            char_source: HitCharacterSource::Packet,
            direction: HitDirection::Outgoing,
            target_hp_before: 0.0,
            target_hp_after: 0.0,
            target_max_hp: 0.0,
            target_hp_percent: 0.0,
            target_id: None,
            target_name: None,
            target_name_en: None,
            target_name_ja: None,
            target_monster_id: None,
            target_context: Vec::new(),
            gameplay_effect_index: None,
            gameplay_effect_name: None,
            ability_name: None,
            damage_name: None,
            damage_component: None,
            attack_type: None,
            damage_attribute: None,
            follow_up_damage: 0.0,
            follow_up_timestamp: None,
            follow_up_damage_name: None,
            follow_up_attack_type: None,
            follow_up_damage_attribute: None,
        }
    }

    #[test]
    fn empty_state_projects_preview_only_when_requested() {
        let state = CombatState::default();
        let config = HudConfig::default();
        let hidden = HashSet::new();

        let empty = project_hud(&state, &config, &hidden, HudProjectionOptions::default());
        let preview = project_hud(
            &state,
            &config,
            &hidden,
            HudProjectionOptions {
                preview_when_empty: true,
                ..HudProjectionOptions::default()
            },
        );

        assert_eq!(empty.data_state, HudDataState::Empty);
        assert!(empty.summary.is_none());
        assert_eq!(preview.data_state, HudDataState::Preview);
        assert_eq!(preview.characters.len(), HUD_PREVIEW_ROW_COUNT);
        assert_eq!(
            preview.characters[0].preview_label_suffix.as_deref(),
            Some("A")
        );
    }

    #[test]
    fn live_projection_sorts_limits_and_keeps_u64_hits_as_strings() {
        let mut state = CombatState::default();
        for character_id in 1..=5 {
            state.push_hit(hit(1.0, character_id, f64::from(character_id) * 100.0));
            state.push_hit(hit(3.0, character_id, f64::from(character_id) * 100.0));
        }

        let snapshot = project_hud(
            &state,
            &HudConfig::default(),
            &HashSet::new(),
            HudProjectionOptions::default(),
        );

        assert_eq!(snapshot.data_state, HudDataState::Live);
        assert_eq!(snapshot.characters.len(), TEAM_DPS_MAX_MEMBERS);
        assert_eq!(snapshot.characters[0].character_id, 5);
        assert_eq!(snapshot.characters[0].hits, "2");
        assert!(snapshot.summary.expect("live summary").team_dps > 0.0);
    }

    #[test]
    fn hidden_and_qte_pseudo_characters_do_not_reach_hud_rows() {
        let mut state = CombatState::default();
        state.push_hit(hit(1.0, 1, 100.0));
        let mut pseudo = hit(1.0, 2, 200.0);
        pseudo.char_known = false;
        pseudo.attack_type = Some("覆纹".to_owned());
        state.push_hit(pseudo);

        let snapshot = project_hud(
            &state,
            &HudConfig::default(),
            &HashSet::from([1]),
            HudProjectionOptions::default(),
        );

        assert!(snapshot.characters.is_empty());
        assert_eq!(snapshot.data_state, HudDataState::Live);
    }

    #[test]
    fn config_projection_uses_sanitized_stable_module_order() {
        let config = HudConfig {
            module_order: vec![HudModule::Characters, HudModule::Characters],
            ..HudConfig::default()
        };

        let projected = HudConfigSnapshot::from(&config);

        assert_eq!(projected.module_order.len(), HudModule::all().len());
        assert_eq!(projected.module_order[0], HudModuleSnapshot::Characters);
    }

    #[test]
    fn preview_timeline_is_projected_only_when_the_module_is_visible() {
        let state = CombatState::default();
        let hidden = HashSet::new();
        let hidden_timeline = project_hud(
            &state,
            &HudConfig::default(),
            &hidden,
            HudProjectionOptions {
                preview_when_empty: true,
                ..HudProjectionOptions::default()
            },
        );
        let visible_timeline = project_hud(
            &state,
            &HudConfig::detailed(),
            &hidden,
            HudProjectionOptions {
                preview_when_empty: true,
                ..HudProjectionOptions::default()
            },
        );

        assert!(hidden_timeline.timeline.is_none());
        assert_eq!(
            visible_timeline
                .timeline
                .expect("detailed preview timeline")
                .buckets
                .len(),
            9
        );
    }

    #[test]
    fn live_timeline_is_bounded_and_preserves_aggregate_damage_and_hits() {
        let mut state = CombatState::default();
        for index in 0..121 {
            state.push_hit(hit(index as f64, 1, 100.0));
        }

        let snapshot = project_hud(
            &state,
            &HudConfig::detailed(),
            &HashSet::new(),
            HudProjectionOptions::default(),
        );
        let timeline = snapshot.timeline.expect("live timeline");

        assert!(timeline.buckets.len() <= HUD_TIMELINE_MAX_BUCKETS);
        assert_eq!(
            timeline
                .buckets
                .iter()
                .map(|bucket| bucket.damage)
                .sum::<f64>(),
            12_100.0
        );
        assert_eq!(
            timeline
                .buckets
                .iter()
                .map(|bucket| bucket.hits.parse::<u64>().expect("bucket hits"))
                .sum::<u64>(),
            121
        );
        assert_eq!(timeline.duration_seconds, 121.0);
        assert!(timeline.peak_dps > 0.0);
    }
}
