//! Versioned, bounded read models for stdio battle record, axis and timeline queries.

use std::collections::{HashMap, HashSet};

use serde::Serialize;

use crate::{
    core::timeline::{
        TimelineMarkerProjectionKind, TimelineProjectionOptions, TimelineScope, project_timeline,
    },
    engine::model::{
        AbyssHalf, CaptureQualitySource, CharacterInfo, CombatState, Hit, HitCharacterSource,
        HitDirection,
    },
    storage::i18n::Language,
};

use super::dto::{BattleQualityDto, BattleSummaryDto};

pub const BATTLE_READ_CONTRACT_VERSION: u32 = 1;
pub const BATTLE_TIMELINE_BUCKET_LIMIT: usize = 10_000;
pub const BATTLE_TIMELINE_ROLE_LIMIT: usize = 100_000;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BattleReadError {
    RecordNotFound,
    AxisCursorExpired { first_available: u64 },
    AxisCursorInvalid { last_available: u64 },
    TimelineTooLarge,
}

#[derive(Clone, Copy, Debug)]
pub struct BattleRecordContext<'a> {
    pub id: &'a str,
    pub capture_operation_id: Option<&'a str>,
    pub generation: u64,
    pub finalized: bool,
    pub finalized_at_unix_ms: Option<u64>,
    pub axis_base_sequence: u64,
    pub source: CaptureQualitySource,
}

#[derive(Clone, Debug, Serialize)]
pub struct BattleRecordDto {
    pub contract_version: u32,
    pub battle_record_id: String,
    pub capture_operation_id: Option<String>,
    pub team_snapshot_id: Option<String>,
    pub generation: String,
    pub state: &'static str,
    pub source: &'static str,
    pub started_at_unix: Option<f64>,
    pub ended_at_unix: Option<f64>,
    pub finalized_at_unix_ms: Option<u64>,
    pub axis_complete: bool,
    pub axis_first_sequence: String,
    pub axis_total_hits: String,
    pub time_stop_intervals: Vec<BattleTimeStopIntervalDto>,
    pub abyss: BattleRecordAbyssDto,
    pub summary: Option<BattleSummaryDto>,
    pub quality: BattleQualityDto,
}

#[derive(Clone, Copy, Debug, Serialize)]
pub struct BattleTimeStopIntervalDto {
    pub start_offset_seconds: f64,
    pub end_offset_seconds: f64,
}

#[derive(Clone, Debug, Serialize)]
pub struct BattleRecordAbyssDto {
    pub detected: bool,
    pub floor: Option<u32>,
    pub active_half: Option<&'static str>,
    pub first_half_at_unix: Option<f64>,
    pub second_half_at_unix: Option<f64>,
    pub success_at_unix: Option<f64>,
    pub exited_at_unix: Option<f64>,
}

pub fn battle_record(
    state: &CombatState,
    context: BattleRecordContext<'_>,
    subtract_time_stop: bool,
) -> BattleRecordDto {
    let summary = state.session_summary(
        context.source,
        crate::engine::model::DpsTimeBasis::from_subtract_time_stop(subtract_time_stop),
        false,
    );
    let quality = state.capture_quality_summary(context.source);
    let time_stop_intervals = match (state.started_at, state.ended_at) {
        (Some(start), Some(end)) => state
            .time_stop_intervals_between(start, end)
            .into_iter()
            .map(|interval| BattleTimeStopIntervalDto {
                start_offset_seconds: finite_non_negative(interval.start_offset),
                end_offset_seconds: finite_non_negative(interval.end_offset),
            })
            .collect(),
        _ => Vec::new(),
    };
    let axis_total_hits = context
        .axis_base_sequence
        .saturating_add(state.hits.len() as u64);

    BattleRecordDto {
        contract_version: BATTLE_READ_CONTRACT_VERSION,
        battle_record_id: context.id.to_owned(),
        capture_operation_id: context.capture_operation_id.map(str::to_owned),
        team_snapshot_id: None,
        generation: context.generation.to_string(),
        state: if context.finalized {
            "finalized"
        } else {
            "live"
        },
        source: capture_source_code(context.source),
        started_at_unix: state.started_at,
        ended_at_unix: state.ended_at,
        finalized_at_unix_ms: context.finalized_at_unix_ms,
        axis_complete: context.axis_base_sequence == 0,
        axis_first_sequence: context.axis_base_sequence.saturating_add(1).to_string(),
        axis_total_hits: axis_total_hits.to_string(),
        time_stop_intervals,
        abyss: BattleRecordAbyssDto {
            detected: state.abyss.is_active(),
            floor: state.abyss.floor,
            active_half: state.abyss.active_half.map(abyss_half_code),
            first_half_at_unix: state.abyss.first_half_at,
            second_half_at_unix: state.abyss.second_half_at,
            success_at_unix: state.abyss.success_at,
            exited_at_unix: state.abyss.exited_at,
        },
        summary: summary.as_ref().map(BattleSummaryDto::from),
        quality: BattleQualityDto::from(&quality),
    }
}

#[derive(Clone, Debug, Serialize)]
pub struct BattleAxisDto {
    pub contract_version: u32,
    pub battle_record_id: String,
    pub generation: String,
    pub finalized: bool,
    pub complete: bool,
    pub first_available_cursor: String,
    pub cursor: String,
    pub next_cursor: Option<String>,
    pub total_hits: String,
    pub retained_hits: usize,
    pub rows: Vec<BattleAxisHitDto>,
}

#[derive(Clone, Debug, Serialize)]
pub struct BattleAxisHitDto {
    pub battle_record_id: String,
    pub sequence: String,
    pub timestamp_unix: f64,
    pub relative_time_seconds: f64,
    pub abyss_half: Option<&'static str>,
    pub character_id: u32,
    pub character_name: String,
    pub character_known: bool,
    pub character_source: &'static str,
    pub attribution_status: &'static str,
    pub attribution_source: &'static str,
    pub attribution_unknown_reason: Option<&'static str>,
    pub team_snapshot_id: Option<String>,
    pub direction: &'static str,
    pub damage: f64,
    pub follow_up_damage: f64,
    pub total_damage: f64,
    pub follow_up_timestamp_unix: Option<f64>,
    pub target_id: Option<String>,
    pub target_name: Option<String>,
    pub target_name_en: Option<String>,
    pub target_name_ja: Option<String>,
    pub target_monster_id: Option<String>,
    pub target_context: Vec<String>,
    pub target_hp_before: f64,
    pub target_hp_after: f64,
    pub target_max_hp: f64,
    pub target_hp_percent: f64,
    pub gameplay_effect_index: Option<u32>,
    pub gameplay_effect_name: Option<String>,
    pub ability_name: Option<String>,
    pub damage_name: Option<String>,
    pub damage_component: Option<String>,
    pub attack_type: Option<String>,
    pub damage_attribute: Option<String>,
    pub follow_up_damage_name: Option<String>,
    pub follow_up_attack_type: Option<String>,
    pub follow_up_damage_attribute: Option<String>,
}

pub fn battle_axis(
    state: &CombatState,
    context: BattleRecordContext<'_>,
    cursor: Option<u64>,
    limit: usize,
) -> Result<BattleAxisDto, BattleReadError> {
    let first_available = context.axis_base_sequence.saturating_add(1);
    let total_hits = context
        .axis_base_sequence
        .saturating_add(state.hits.len() as u64);
    let cursor = cursor.unwrap_or(first_available);
    if cursor < first_available {
        return Err(BattleReadError::AxisCursorExpired { first_available });
    }
    if cursor > total_hits.saturating_add(1) {
        return Err(BattleReadError::AxisCursorInvalid {
            last_available: total_hits,
        });
    }

    let last_requested = cursor
        .saturating_add(limit.saturating_sub(1) as u64)
        .min(total_hits);
    let mut first_half = half_hit_counts(&state.abyss.first_half.hits);
    let mut second_half = half_hit_counts(&state.abyss.second_half.hits);
    let started_at = state.started_at.unwrap_or(0.0);
    let mut rows = Vec::with_capacity(limit.min(state.hits.len()));
    for (index, hit) in state.hits.iter().enumerate() {
        let sequence = context
            .axis_base_sequence
            .saturating_add(index as u64)
            .saturating_add(1);
        let half = take_hit_half(hit, &mut first_half, &mut second_half);
        if sequence < cursor {
            continue;
        }
        if sequence > last_requested {
            break;
        }
        rows.push(axis_hit(hit, context.id, sequence, started_at, half));
    }
    let next_cursor = rows
        .last()
        .and_then(|row| row.sequence.parse::<u64>().ok())
        .and_then(|last| (last < total_hits).then(|| last.saturating_add(1).to_string()));

    Ok(BattleAxisDto {
        contract_version: BATTLE_READ_CONTRACT_VERSION,
        battle_record_id: context.id.to_owned(),
        generation: context.generation.to_string(),
        finalized: context.finalized,
        complete: context.axis_base_sequence == 0,
        first_available_cursor: first_available.to_string(),
        cursor: cursor.to_string(),
        next_cursor,
        total_hits: total_hits.to_string(),
        retained_hits: state.hits.len(),
        rows,
    })
}

fn axis_hit(
    hit: &Hit,
    battle_record_id: &str,
    sequence: u64,
    started_at: f64,
    half: Option<AbyssHalf>,
) -> BattleAxisHitDto {
    let attribution_source = character_source_code(hit.char_source);
    BattleAxisHitDto {
        battle_record_id: battle_record_id.to_owned(),
        sequence: sequence.to_string(),
        timestamp_unix: hit.timestamp,
        relative_time_seconds: finite_non_negative(hit.timestamp - started_at),
        abyss_half: half.map(abyss_half_code),
        character_id: hit.char_id,
        character_name: hit.char_name.clone(),
        character_known: hit.char_known,
        character_source: attribution_source,
        attribution_status: if hit.char_known {
            "attributed"
        } else {
            "unknown"
        },
        attribution_source,
        attribution_unknown_reason: (!hit.char_known).then_some("character_unknown"),
        team_snapshot_id: None,
        direction: hit_direction_code(hit.direction),
        damage: hit.damage,
        follow_up_damage: hit.follow_up_damage,
        total_damage: hit.total_damage(),
        follow_up_timestamp_unix: hit.follow_up_timestamp,
        target_id: hit.target_id.clone(),
        target_name: hit.target_name.clone(),
        target_name_en: hit.target_name_en.clone(),
        target_name_ja: hit.target_name_ja.clone(),
        target_monster_id: hit.target_monster_id.clone(),
        target_context: hit.target_context.clone(),
        target_hp_before: hit.target_hp_before,
        target_hp_after: hit.target_hp_after,
        target_max_hp: hit.target_max_hp,
        target_hp_percent: hit.target_hp_percent,
        gameplay_effect_index: hit.gameplay_effect_index,
        gameplay_effect_name: hit.gameplay_effect_name.clone(),
        ability_name: hit.ability_name.clone(),
        damage_name: hit.damage_name.clone(),
        damage_component: hit.damage_component.clone(),
        attack_type: hit.attack_type.clone(),
        damage_attribute: hit.damage_attribute.clone(),
        follow_up_damage_name: hit.follow_up_damage_name.clone(),
        follow_up_attack_type: hit.follow_up_attack_type.clone(),
        follow_up_damage_attribute: hit.follow_up_damage_attribute.clone(),
    }
}

#[derive(Clone, Copy, Debug, Hash, PartialEq, Eq)]
struct AxisHitKey {
    timestamp: u64,
    character_id: u32,
    byte_offset: usize,
    bit_shift: u8,
    damage: u64,
    target_hp_after: u64,
}

impl From<&Hit> for AxisHitKey {
    fn from(hit: &Hit) -> Self {
        Self {
            timestamp: hit.timestamp.to_bits(),
            character_id: hit.char_id,
            byte_offset: hit.byte_offset,
            bit_shift: hit.bit_shift,
            damage: hit.damage.to_bits(),
            target_hp_after: hit.target_hp_after.to_bits(),
        }
    }
}

fn half_hit_counts(hits: &std::collections::VecDeque<Hit>) -> HashMap<AxisHitKey, usize> {
    let mut counts = HashMap::new();
    for hit in hits {
        *counts.entry(AxisHitKey::from(hit)).or_insert(0) += 1;
    }
    counts
}

fn take_hit_half(
    hit: &Hit,
    first: &mut HashMap<AxisHitKey, usize>,
    second: &mut HashMap<AxisHitKey, usize>,
) -> Option<AbyssHalf> {
    let key = AxisHitKey::from(hit);
    if take_count(first, key) {
        Some(AbyssHalf::First)
    } else if take_count(second, key) {
        Some(AbyssHalf::Second)
    } else {
        None
    }
}

fn take_count(counts: &mut HashMap<AxisHitKey, usize>, key: AxisHitKey) -> bool {
    let Some(count) = counts.get_mut(&key) else {
        return false;
    };
    *count -= 1;
    if *count == 0 {
        counts.remove(&key);
    }
    true
}

#[derive(Clone, Debug, Serialize)]
pub struct BattleTimelineDto {
    pub contract_version: u32,
    pub battle_record_id: String,
    pub generation: String,
    pub finalized: bool,
    pub complete: bool,
    pub scope: &'static str,
    pub bucket_seconds: f64,
    pub bucket_seconds_min: f64,
    pub bucket_seconds_max: f64,
    pub bucket_seconds_step: f64,
    pub started_at_unix: Option<f64>,
    pub duration_seconds: f64,
    pub total_damage: f64,
    pub peak_dps: f64,
    pub time_stop_duration_seconds: f64,
    pub time_stop_intervals: Vec<BattleTimelineIntervalDto>,
    pub markers: Vec<BattleTimelineMarkerDto>,
    pub characters: Vec<BattleTimelineCharacterDto>,
    pub buckets: Vec<BattleTimelineBucketDto>,
    pub segments: Vec<BattleTimelineSegmentDto>,
}

#[derive(Clone, Copy, Debug, Serialize)]
pub struct BattleTimelineIntervalDto {
    pub start_offset_seconds: f64,
    pub end_offset_seconds: f64,
}

#[derive(Clone, Debug, Serialize)]
pub struct BattleTimelineMarkerDto {
    pub offset_seconds: f64,
    pub label_key: String,
    pub kind: &'static str,
}

#[derive(Clone, Debug, Serialize)]
pub struct BattleTimelineCharacterDto {
    pub character_id: u32,
    pub name: String,
    pub total_damage: f64,
}

#[derive(Clone, Debug, Serialize)]
pub struct BattleTimelineBucketDto {
    pub start_offset_seconds: f64,
    pub end_offset_seconds: f64,
    pub team_dps: f64,
    pub damage: f64,
    pub hits: String,
    pub cumulative_damage: f64,
    pub roles: Vec<BattleTimelineRoleDto>,
}

#[derive(Clone, Copy, Debug, Serialize)]
pub struct BattleTimelineRoleDto {
    pub character_id: u32,
    pub dps: f64,
}

#[derive(Clone, Copy, Debug, Serialize)]
pub struct BattleTimelineSegmentDto {
    pub start_offset_seconds: f64,
    pub end_offset_seconds: f64,
    pub dps: f64,
}

pub fn battle_timeline(
    state: &CombatState,
    characters: &HashMap<u32, CharacterInfo>,
    context: BattleRecordContext<'_>,
    scope: TimelineScope,
    bucket_seconds: f32,
    subtract_time_stop: bool,
) -> Result<BattleTimelineDto, BattleReadError> {
    let span = timeline_span(state, scope);
    if let Some((start, end)) = span {
        let bucket_count =
            ((end - start).max(0.0) / f64::from(bucket_seconds)).floor() as usize + 1;
        let role_upper_bound = bucket_count.saturating_mul(timeline_character_count(state, scope));
        if bucket_count > BATTLE_TIMELINE_BUCKET_LIMIT
            || role_upper_bound > BATTLE_TIMELINE_ROLE_LIMIT
        {
            return Err(BattleReadError::TimelineTooLarge);
        }
    }
    let projection = project_timeline(
        state,
        characters,
        TimelineProjectionOptions {
            scope,
            bucket_seconds,
            subtract_time_stop,
            language: Language::English,
        },
    );
    let role_count = projection
        .buckets
        .iter()
        .map(|bucket| bucket.roles.len())
        .sum::<usize>();
    if projection.buckets.len() > BATTLE_TIMELINE_BUCKET_LIMIT
        || role_count > BATTLE_TIMELINE_ROLE_LIMIT
    {
        return Err(BattleReadError::TimelineTooLarge);
    }

    Ok(BattleTimelineDto {
        contract_version: BATTLE_READ_CONTRACT_VERSION,
        battle_record_id: context.id.to_owned(),
        generation: context.generation.to_string(),
        finalized: context.finalized,
        complete: context.axis_base_sequence == 0,
        scope: timeline_scope_code(scope),
        bucket_seconds: projection.bucket_seconds,
        bucket_seconds_min: projection.bucket_seconds_min,
        bucket_seconds_max: projection.bucket_seconds_max,
        bucket_seconds_step: projection.bucket_seconds_step,
        started_at_unix: span.map(|(start, _)| start),
        duration_seconds: projection.duration,
        total_damage: projection.total_damage,
        peak_dps: projection.peak_dps,
        time_stop_duration_seconds: projection.time_stop_duration,
        time_stop_intervals: projection
            .time_stop_intervals
            .into_iter()
            .map(|interval| BattleTimelineIntervalDto {
                start_offset_seconds: interval.start,
                end_offset_seconds: interval.end,
            })
            .collect(),
        markers: projection
            .markers
            .into_iter()
            .map(|marker| BattleTimelineMarkerDto {
                offset_seconds: marker.offset,
                label_key: marker.label_key,
                kind: match marker.kind {
                    TimelineMarkerProjectionKind::Half => "half",
                    TimelineMarkerProjectionKind::Clear => "clear",
                    TimelineMarkerProjectionKind::Exit => "exit",
                },
            })
            .collect(),
        characters: projection
            .characters
            .into_iter()
            .map(|character| BattleTimelineCharacterDto {
                character_id: character.id,
                name: character.name,
                total_damage: character.total_damage,
            })
            .collect(),
        buckets: projection
            .buckets
            .into_iter()
            .map(|bucket| BattleTimelineBucketDto {
                start_offset_seconds: bucket.start,
                end_offset_seconds: bucket.end,
                team_dps: bucket.team_dps,
                damage: bucket.damage,
                hits: bucket.hits.to_string(),
                cumulative_damage: bucket.cumulative_damage,
                roles: bucket
                    .roles
                    .into_iter()
                    .map(|role| BattleTimelineRoleDto {
                        character_id: role.character_id,
                        dps: role.dps,
                    })
                    .collect(),
            })
            .collect(),
        segments: projection
            .segments
            .into_iter()
            .map(|segment| BattleTimelineSegmentDto {
                start_offset_seconds: segment.start,
                end_offset_seconds: segment.end,
                dps: segment.dps,
            })
            .collect(),
    })
}

fn timeline_span(state: &CombatState, scope: TimelineScope) -> Option<(f64, f64)> {
    match scope {
        TimelineScope::Whole => hit_span(state.hits.iter()),
        TimelineScope::First => hit_span(state.abyss.first_half.hits.iter()),
        TimelineScope::Second => hit_span(state.abyss.second_half.hits.iter()),
    }
}

fn timeline_character_count(state: &CombatState, scope: TimelineScope) -> usize {
    let hits = match scope {
        TimelineScope::Whole => &state.hits,
        TimelineScope::First => &state.abyss.first_half.hits,
        TimelineScope::Second => &state.abyss.second_half.hits,
    };
    hits.iter()
        .filter(|hit| !hit.direction.is_incoming() && hit.timestamp.is_finite())
        .map(|hit| hit.char_id)
        .collect::<HashSet<_>>()
        .len()
}

fn hit_span<'a>(hits: impl Iterator<Item = &'a Hit>) -> Option<(f64, f64)> {
    let mut range: Option<(f64, f64)> = None;
    for timestamp in hits
        .filter(|hit| !hit.direction.is_incoming() && hit.timestamp.is_finite())
        .map(|hit| hit.timestamp)
    {
        range = Some(match range {
            Some((start, end)) => (start.min(timestamp), end.max(timestamp)),
            None => (timestamp, timestamp),
        });
    }
    range
}

pub const fn timeline_scope_code(scope: TimelineScope) -> &'static str {
    match scope {
        TimelineScope::Whole => "all",
        TimelineScope::First => "upper",
        TimelineScope::Second => "lower",
    }
}

pub const fn abyss_half_code(half: AbyssHalf) -> &'static str {
    match half {
        AbyssHalf::First => "upper",
        AbyssHalf::Second => "lower",
    }
}

const fn capture_source_code(source: CaptureQualitySource) -> &'static str {
    match source {
        CaptureQualitySource::Live => "live",
        CaptureQualitySource::PcapngReplay => "pcapng_replay",
        CaptureQualitySource::JsonReplay => "json_replay",
        CaptureQualitySource::Unknown => "unknown",
    }
}

const fn character_source_code(source: HitCharacterSource) -> &'static str {
    match source {
        HitCharacterSource::Packet => "packet",
        HitCharacterSource::Session => "session",
        HitCharacterSource::GameplayEffect => "gameplay_effect",
        HitCharacterSource::ExportJson => "export_json",
        HitCharacterSource::Unknown => "unknown",
    }
}

const fn hit_direction_code(direction: HitDirection) -> &'static str {
    match direction {
        HitDirection::Outgoing => "outgoing",
        HitDirection::Incoming => "incoming",
        HitDirection::Unknown => "unknown",
    }
}

fn finite_non_negative(value: f64) -> f64 {
    if value.is_finite() && value >= 0.0 {
        value
    } else {
        0.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn context(axis_base_sequence: u64) -> BattleRecordContext<'static> {
        BattleRecordContext {
            id: "battle-7",
            capture_operation_id: Some("capture-3"),
            generation: 9,
            finalized: false,
            finalized_at_unix_ms: None,
            axis_base_sequence,
            source: CaptureQualitySource::Live,
        }
    }

    #[test]
    fn battle_read_axis_is_cursor_paginated_and_reports_trimmed_history() {
        let mut state = CombatState::default();
        state.push_hit(test_hit(1.0, 100.0));
        state.push_hit(test_hit(2.0, 200.0));

        let first = battle_axis(&state, context(0), None, 1).expect("first page");
        assert!(first.complete);
        assert_eq!(first.cursor, "1");
        assert_eq!(first.next_cursor.as_deref(), Some("2"));
        assert_eq!(first.total_hits, "2");
        assert_eq!(first.rows.len(), 1);
        assert_eq!(first.rows[0].battle_record_id, "battle-7");
        assert_eq!(first.rows[0].sequence, "1");
        assert_eq!(first.rows[0].attribution_status, "attributed");
        assert!(first.rows[0].attribution_unknown_reason.is_none());
        assert!(first.rows[0].team_snapshot_id.is_none());

        let trimmed = battle_axis(&state, context(5), Some(6), 10).expect("retained page");
        assert!(!trimmed.complete);
        assert_eq!(trimmed.first_available_cursor, "6");
        assert_eq!(trimmed.total_hits, "7");
        assert_eq!(trimmed.rows[0].sequence, "6");
        assert!(matches!(
            battle_axis(&state, context(5), Some(5), 10),
            Err(BattleReadError::AxisCursorExpired { first_available: 6 })
        ));
        assert!(matches!(
            battle_axis(&state, context(5), Some(9), 10),
            Err(BattleReadError::AxisCursorInvalid { last_available: 7 })
        ));
    }

    #[test]
    fn battle_read_timeline_rejects_a_response_that_exceeds_the_bucket_budget() {
        let mut state = CombatState::default();
        state.push_hit(test_hit(0.0, 100.0));
        state.push_hit(test_hit(3_000.0, 200.0));

        assert!(matches!(
            battle_timeline(
                &state,
                &HashMap::new(),
                context(0),
                TimelineScope::Whole,
                0.2,
                true,
            ),
            Err(BattleReadError::TimelineTooLarge)
        ));
    }

    #[test]
    fn battle_read_timeline_rejects_a_role_upper_bound_before_projection() {
        let mut state = CombatState::default();
        for character_id in 1..=11 {
            let mut hit = test_hit(f64::from(character_id - 1), 1.0);
            hit.char_id = character_id;
            state.push_hit(hit);
        }
        let mut last = test_hit(9_999.0, 1.0);
        last.char_id = 11;
        state.push_hit(last);

        assert!(matches!(
            battle_timeline(
                &state,
                &HashMap::new(),
                context(0),
                TimelineScope::Whole,
                1.0,
                true,
            ),
            Err(BattleReadError::TimelineTooLarge)
        ));
    }

    fn test_hit(timestamp: f64, damage: f64) -> Hit {
        Hit {
            timestamp,
            char_id: 7,
            char_name: "Character".to_owned(),
            char_known: true,
            damage,
            byte_offset: 0,
            bit_shift: 0,
            char_source: HitCharacterSource::Unknown,
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
            damage_name: Some("Skill".to_owned()),
            damage_component: None,
            attack_type: Some("normal".to_owned()),
            damage_attribute: None,
            follow_up_damage: 0.0,
            follow_up_timestamp: None,
            follow_up_damage_name: None,
            follow_up_attack_type: None,
            follow_up_damage_attribute: None,
        }
    }
}
