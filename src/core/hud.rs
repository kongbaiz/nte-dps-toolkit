//! Stable, UI-neutral projection used by every desktop HUD frontend.
//!
//! The projection deliberately contains aggregate readouts only. It never
//! exposes `CombatState`, packets, hits, mutable references, or GUI types.

use std::collections::{HashMap, HashSet};

use serde::Serialize;

use crate::{
    engine::model::{
        AbyssHalf, CharacterStats, CombatState, CompactTimelineSeries, DpsTimeBasis,
        PartyCombatState, TEAM_DPS_MAX_MEMBERS,
    },
    storage::config::{HudConfig, HudModule},
};

#[cfg(test)]
use crate::engine::model::TimelineSeries;

pub const HUD_SNAPSHOT_VERSION: u32 = 3;
pub const HUD_TIMELINE_MAX_BUCKETS: usize = 60;
/// UTF-8 byte budget for a live/read-model character label. Imported hit names
/// are untrusted and a single label must never be able to overflow a desktop
/// stream delivery.
pub const HUD_CHARACTER_NAME_MAX_BYTES: usize = 128;
const HUD_PREVIEW_ROW_COUNT: usize = 4;

pub fn bounded_character_name(character_id: u32, candidate: &str) -> String {
    let candidate = candidate.trim();
    if !candidate.is_empty() && candidate.len() <= HUD_CHARACTER_NAME_MAX_BYTES {
        candidate.to_owned()
    } else {
        format!("#{character_id}")
    }
}

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
    pub color: Option<String>,
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
    pub include_max_hp_reduction_in_total_damage: bool,
    pub selected_abyss_half: Option<AbyssHalf>,
    pub preview_when_empty: bool,
    pub timeline_bucket_seconds: f64,
}

impl Default for HudProjectionOptions {
    fn default() -> Self {
        Self {
            dps_time_basis: DpsTimeBasis::SubtractTimeStop,
            separate_reaction_damage: false,
            include_max_hp_reduction_in_total_damage: false,
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
                state.total_damage,
                state.max_hp_reduction,
                state.total_damage_taken,
                state.dps_with_time_stop(subtract_time_stop),
                state.duration_with_time_stop(subtract_time_stop),
                hidden_character_ids,
                options.separate_reaction_damage,
                options.include_max_hp_reduction_in_total_damage,
                |row| state.character_dps_with_time_stop(row, subtract_time_stop),
            )
        },
        |half| {
            let party = state.abyss.half(half);
            project_party_readout(
                party,
                hidden_character_ids,
                options.separate_reaction_damage,
                options.include_max_hp_reduction_in_total_damage,
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
                // The authoritative state maintains an exact timestamp index as hits mutate.
                // Querying its bounded aggregate avoids scanning an unbounded hit history while
                // the live state lock is held at HUD refresh frequency.
                let compact = selected_half.map_or_else(
                    || {
                        state.compact_timeline(
                            options.timeline_bucket_seconds,
                            HUD_TIMELINE_MAX_BUCKETS,
                        )
                    },
                    |half| {
                        state.abyss.half(half).compact_timeline(
                            options.timeline_bucket_seconds,
                            HUD_TIMELINE_MAX_BUCKETS,
                        )
                    },
                );
                compact.map(project_compact_timeline)
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
    include_max_hp_reduction_in_total_damage: bool,
    subtract_time_stop: bool,
) -> (Vec<HudCharacterSnapshot>, HudSummarySnapshot) {
    project_readout(
        &party.stats,
        party.total_damage,
        party.max_hp_reduction,
        party.total_damage_taken,
        party.dps_with_time_stop(subtract_time_stop),
        party.duration_with_time_stop(subtract_time_stop),
        hidden_character_ids,
        separate_reaction_damage,
        include_max_hp_reduction_in_total_damage,
        |row| party.character_dps_with_time_stop(row, subtract_time_stop),
    )
}

#[allow(clippy::too_many_arguments)]
fn project_readout(
    stats: &HashMap<u32, CharacterStats>,
    total_damage: f64,
    max_hp_reduction: f64,
    total_damage_taken: f64,
    team_dps: f64,
    duration_seconds: f64,
    hidden_character_ids: &HashSet<u32>,
    separate_reaction_damage: bool,
    include_max_hp_reduction_in_total_damage: bool,
    character_dps: impl Fn(&CharacterStats) -> f64,
) -> (Vec<HudCharacterSnapshot>, HudSummarySnapshot) {
    let effective_total_damage = total_damage
        + if include_max_hp_reduction_in_total_damage {
            max_hp_reduction
        } else {
            0.0
        };
    let effective_team_dps = if include_max_hp_reduction_in_total_damage {
        effective_total_damage / duration_seconds.max(1.0)
    } else {
        team_dps
    };
    let mut rows = stats
        .values()
        .filter(|row| !hidden_character_ids.contains(&row.char_id) && row.has_hud_visible_hit())
        .map(|row| row.for_reaction_damage_policy(separate_reaction_damage))
        .filter(character_has_visible_totals)
        .map(|row| {
            let dps = character_dps(&row);
            HudCharacterSnapshot {
                character_id: row.char_id,
                name: bounded_character_name(row.char_id, &row.name),
                preview_label_suffix: None,
                hits: row.hits.to_string(),
                damage: finite_nonnegative(row.damage),
                dps: finite_nonnegative(dps),
                damage_share_percent: percentage(row.damage, effective_total_damage),
                damage_taken: finite_nonnegative(row.damage_taken),
                color: None,
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
            team_dps: finite_nonnegative(effective_team_dps),
            duration_seconds: finite_nonnegative(duration_seconds),
            total_damage: finite_nonnegative(effective_total_damage),
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
            color: None,
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

fn project_compact_timeline(series: CompactTimelineSeries) -> HudTimelineSnapshot {
    let buckets = series
        .buckets
        .into_iter()
        .map(|bucket| {
            let start_seconds = finite_nonnegative(bucket.start_offset);
            let end_seconds = finite_nonnegative(bucket.end_offset).max(start_seconds);
            let damage = finite_nonnegative(bucket.damage);
            HudTimelineBucketSnapshot {
                start_seconds,
                end_seconds,
                damage,
                dps: finite_nonnegative(damage / (end_seconds - start_seconds).max(0.001)),
                hits: bucket.hits.to_string(),
            }
        })
        .collect::<Vec<_>>();
    let duration_seconds = buckets
        .last()
        .map_or(0.0, |bucket| bucket.end_seconds)
        .max(0.0);
    let peak_dps = buckets.iter().map(|bucket| bucket.dps).fold(0.0, f64::max);

    HudTimelineSnapshot {
        bucket_seconds: finite_nonnegative(series.bucket_seconds),
        duration_seconds,
        peak_dps,
        buckets,
    }
}

#[cfg(test)]
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
    use crate::engine::model::{
        AbyssEvent, Hit, HitCharacterSource, HitDamageCorrection, HitDirection, HitFollowUp,
        TimeStopEvent,
    };

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
            max_hp_reduction: 0.0,
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
            reconciled_overkill_damage: None,
            wire_event: None,
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
    fn live_projection_bounds_untrusted_character_names_by_utf8_bytes() {
        let mut state = CombatState::default();
        let mut exact = hit(1.0, 1, 100.0);
        exact.char_name = "a".repeat(HUD_CHARACTER_NAME_MAX_BYTES);
        state.push_hit(exact);
        let mut oversized = hit(1.0, 2, 200.0);
        oversized.char_name = "界".repeat(43);
        state.push_hit(oversized);

        let snapshot = project_hud(
            &state,
            &HudConfig::default(),
            &HashSet::new(),
            HudProjectionOptions::default(),
        );

        let exact = snapshot
            .characters
            .iter()
            .find(|row| row.character_id == 1)
            .expect("exactly bounded name");
        assert_eq!(exact.name.len(), HUD_CHARACTER_NAME_MAX_BYTES);
        let oversized = snapshot
            .characters
            .iter()
            .find(|row| row.character_id == 2)
            .expect("oversized name fallback");
        assert_eq!(oversized.name, "#2");
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
    fn known_character_with_reaction_follow_up_remains_in_hud_rows() {
        let mut state = CombatState::default();
        let mut character = hit(1.0, 1003, 100.0);
        character.attack_type = Some("普攻".to_owned());
        character.follow_up_damage = 25.0;
        character.follow_up_attack_type = Some("覆纹".to_owned());
        state.push_hit(character);

        let snapshot = project_hud(
            &state,
            &HudConfig::default(),
            &HashSet::new(),
            HudProjectionOptions::default(),
        );

        assert_eq!(snapshot.characters.len(), 1);
        assert_eq!(snapshot.characters[0].character_id, 1003);
        assert_eq!(snapshot.characters[0].damage, 125.0);
    }

    #[test]
    fn max_hp_reduction_total_policy_uses_one_combined_denominator() {
        let mut state = CombatState::default();
        let mut source = hit(1.0, 1004, 100.0);
        source.max_hp_reduction = 50.0;
        state.push_hit(source);

        let traditional = project_hud(
            &state,
            &HudConfig::default(),
            &HashSet::new(),
            HudProjectionOptions::default(),
        );
        let combined = project_hud(
            &state,
            &HudConfig::default(),
            &HashSet::new(),
            HudProjectionOptions {
                include_max_hp_reduction_in_total_damage: true,
                ..HudProjectionOptions::default()
            },
        );

        let traditional_summary = traditional.summary.expect("traditional summary");
        let combined_summary = combined.summary.expect("combined summary");
        assert_eq!(traditional_summary.total_damage, 100.0);
        assert_eq!(traditional_summary.team_dps, 100.0);
        assert_eq!(traditional.characters[0].damage_share_percent, 100.0);
        assert_eq!(combined_summary.total_damage, 150.0);
        assert_eq!(combined_summary.team_dps, 150.0);
        assert!((combined.characters[0].damage_share_percent - 66.666_666).abs() < 0.001);
    }

    #[test]
    fn follow_up_updates_incremental_visibility_and_compact_timeline() {
        let mut state = CombatState::default();
        let mut source = hit(1.0, 77, 100.0);
        source.char_known = false;
        source.direction = HitDirection::Incoming;
        state.push_hit(source);

        let before = project_hud(
            &state,
            &HudConfig::detailed(),
            &HashSet::new(),
            HudProjectionOptions::default(),
        );
        assert_eq!(before.characters.len(), 1);

        assert!(state.apply_follow_up(HitFollowUp {
            source_timestamp: 1.0,
            source_char_id: 77,
            source_damage: 100.0,
            source_target_hp_before: 0.0,
            source_target_hp_after: 0.0,
            source_target_max_hp: 0.0,
            source_gameplay_effect_index: None,
            timestamp: 1.1,
            damage: 25.0,
            target_hp_after: 0.0,
            target_hp_percent: 0.0,
            damage_name: Some("覆纹追加攻击".to_owned()),
            attack_type: Some("覆纹".to_owned()),
            damage_attribute: None,
        }));

        let after = project_hud(
            &state,
            &HudConfig::detailed(),
            &HashSet::new(),
            HudProjectionOptions::default(),
        );
        assert!(after.characters.is_empty());

        state.push_hit(hit(2.0, 78, 100.0));
        assert!(state.apply_follow_up(HitFollowUp {
            source_timestamp: 2.0,
            source_char_id: 78,
            source_damage: 100.0,
            source_target_hp_before: 0.0,
            source_target_hp_after: 0.0,
            source_target_max_hp: 0.0,
            source_gameplay_effect_index: None,
            timestamp: 2.1,
            damage: 25.0,
            target_hp_after: 0.0,
            target_hp_percent: 0.0,
            damage_name: Some("覆纹追加攻击".to_owned()),
            attack_type: Some("覆纹".to_owned()),
            damage_attribute: None,
        }));
        let timeline = project_hud(
            &state,
            &HudConfig::detailed(),
            &HashSet::new(),
            HudProjectionOptions::default(),
        )
        .timeline
        .expect("incremental compact timeline");
        assert_eq!(timeline.buckets.len(), 1);
        assert_eq!(timeline.buckets[0].damage, 125.0);
        assert_eq!(timeline.buckets[0].hits, "1");

        assert!(state.apply_damage_correction(HitDamageCorrection {
            source_timestamp: 2.0,
            source_char_id: 78,
            source_damage: 100.0,
            source_target_hp_before: 0.0,
            source_target_hp_after: 0.0,
            source_target_max_hp: 0.0,
            source_gameplay_effect_index: None,
            damage: 150.0,
            target_hp_before: 0.0,
            target_hp_after: 0.0,
            target_hp_percent: 0.0,
            max_hp_reduction: None,
            reconciled_overkill_damage: None,
        }));
        let corrected = state
            .compact_timeline(1.0, HUD_TIMELINE_MAX_BUCKETS)
            .expect("corrected compact timeline");
        assert_eq!(corrected.buckets[0].damage, 175.0);
        assert_eq!(corrected.buckets[0].hits, 1);
    }

    #[test]
    fn compact_timeline_adds_one_hit_without_rebuilding_from_history() {
        let mut state = CombatState::default();
        for index in 0..5_000 {
            state.push_hit(hit(index as f64, 1, 1.0));
        }
        let before = state
            .compact_timeline(1.0, HUD_TIMELINE_MAX_BUCKETS)
            .expect("indexed timeline before append");
        assert_eq!(
            before.buckets.iter().map(|bucket| bucket.hits).sum::<u64>(),
            5_000
        );

        state.push_hit(hit(5_000.0, 1, 7.0));
        let after = state
            .compact_timeline(1.0, HUD_TIMELINE_MAX_BUCKETS)
            .expect("indexed timeline after append");
        assert_eq!(
            after.buckets.iter().map(|bucket| bucket.hits).sum::<u64>(),
            5_001
        );
        assert_eq!(
            after
                .buckets
                .iter()
                .map(|bucket| bucket.damage)
                .sum::<f64>(),
            5_007.0
        );
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

        let legacy_projection =
            project_timeline(state.timeline(1.0, true)).expect("legacy live timeline projection");

        let snapshot = project_hud(
            &state,
            &HudConfig::detailed(),
            &HashSet::new(),
            HudProjectionOptions::default(),
        );
        let timeline = snapshot.timeline.expect("live timeline");

        assert_eq!(timeline, legacy_projection);
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

    #[test]
    fn long_span_live_timeline_bounds_the_source_layout_before_allocation() {
        const LONG_SPAN_SECONDS: f64 = 10_000_000.0;
        const REQUESTED_BUCKET_SECONDS: f64 = 0.001;

        let mut state = CombatState::default();
        state.push_hit(hit(0.0, 1, 100.0));
        state.push_hit(hit(LONG_SPAN_SECONDS / 2.0, 1, 250.0));
        state.push_hit(hit(LONG_SPAN_SECONDS, 1, 400.0));

        let timeline = project_hud(
            &state,
            &HudConfig::detailed(),
            &HashSet::new(),
            HudProjectionOptions {
                timeline_bucket_seconds: REQUESTED_BUCKET_SECONDS,
                ..HudProjectionOptions::default()
            },
        )
        .timeline
        .expect("long-span live timeline");

        assert_eq!(timeline.buckets.len(), HUD_TIMELINE_MAX_BUCKETS);
        assert!(timeline.bucket_seconds >= LONG_SPAN_SECONDS / HUD_TIMELINE_MAX_BUCKETS as f64);
        assert_eq!(
            timeline
                .buckets
                .iter()
                .map(|bucket| bucket.damage)
                .sum::<f64>(),
            750.0
        );
        assert_eq!(
            timeline
                .buckets
                .iter()
                .map(|bucket| bucket.hits.parse::<u64>().expect("bucket hits"))
                .sum::<u64>(),
            3
        );
    }

    #[test]
    fn bounded_hud_timeline_keeps_wall_clock_and_engine_annotation_semantics() {
        let mut state = CombatState::default();
        state.apply_abyss_event(AbyssEvent::Stage {
            timestamp: 0.0,
            cycle: Some(1),
            floor: Some(1),
            half: AbyssHalf::First,
            allow_late_backfill: false,
        });
        state.push_hit(hit(1.0, 1, 100.0));
        state.apply_time_stop_event(TimeStopEvent::GamePauseStarted {
            timestamp: 1.25,
            pause_type_mask: 1 << 2,
        });
        state.apply_time_stop_event(TimeStopEvent::GamePauseEnded {
            timestamp: 1.75,
            pause_type_mask: 1 << 2,
        });
        state.push_hit(hit(2.0, 1, 200.0));
        state.apply_abyss_event(AbyssEvent::Success { timestamp: 3.0 });
        state.apply_abyss_event(AbyssEvent::Exit { timestamp: 4.0 });

        let wall_clock = project_hud(
            &state,
            &HudConfig::detailed(),
            &HashSet::new(),
            HudProjectionOptions {
                dps_time_basis: DpsTimeBasis::WallClock,
                ..HudProjectionOptions::default()
            },
        )
        .timeline
        .expect("wall-clock HUD timeline");
        let subtract_time_stop = project_hud(
            &state,
            &HudConfig::detailed(),
            &HashSet::new(),
            HudProjectionOptions {
                dps_time_basis: DpsTimeBasis::SubtractTimeStop,
                ..HudProjectionOptions::default()
            },
        )
        .timeline
        .expect("time-stop-adjusted HUD timeline");

        assert_eq!(wall_clock, subtract_time_stop);
        assert_eq!(wall_clock.duration_seconds, 2.0);
        assert_eq!(wall_clock.buckets[0].dps, 100.0);

        let engine_timeline = state.timeline(1.0, true);
        assert_eq!(engine_timeline.time_stop_intervals.len(), 1);
        assert!((engine_timeline.time_stop_intervals[0].start_offset - 0.25).abs() < 1e-9);
        assert!((engine_timeline.time_stop_intervals[0].end_offset - 0.75).abs() < 1e-9);
        assert!(
            engine_timeline
                .markers
                .iter()
                .any(|marker| marker.label == "Ascending Line" && marker.offset == 0.0)
        );
        assert!(
            engine_timeline
                .markers
                .iter()
                .any(|marker| marker.label == "Cleared" && (marker.offset - 1.0).abs() < 1e-9)
        );
    }
}
