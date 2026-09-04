//! Frontend-neutral projection for the Console timeline page.

use std::collections::HashMap;

use crate::{
    engine::model::{
        AbyssHalf, COMBAT_SEGMENT_GAP_SECONDS, CharacterInfo, CombatState, TimelineMarkerKind,
        summarize_combat_segments,
    },
    storage::{
        config::{
            TIMELINE_BUCKET_SECONDS_MAX, TIMELINE_BUCKET_SECONDS_MIN,
            sanitize_timeline_bucket_seconds,
        },
        i18n::Language,
    },
};

pub const TIMELINE_BUCKET_SECONDS_STEP: f32 = 0.1;
/// UTF-8 byte budget for a projected character label. Replay/import names are
/// untrusted and must not be able to exceed the bounded Timeline delivery.
pub const MAX_TIMELINE_CHARACTER_NAME_BYTES: usize = 128;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum TimelineScope {
    #[default]
    Whole,
    First,
    Second,
}

#[derive(Clone, Copy, Debug)]
pub struct TimelineProjectionOptions {
    pub scope: TimelineScope,
    pub bucket_seconds: f32,
    /// Hard source-side output budget. Long spans are coarsened before bucket
    /// allocation; authoritative hits remain complete.
    pub max_buckets: usize,
    /// Per-bucket role row cap enforced during aggregation.
    pub max_roles_per_bucket: usize,
    /// Cross-bucket character identity cap enforced during aggregation.
    pub max_characters: usize,
    pub subtract_time_stop: bool,
    pub language: Language,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct TimelineProjection {
    /// Sanitized user preference used by controls/persistence.
    pub bucket_seconds: f64,
    /// Actual source aggregation width after enforcing `max_buckets`.
    pub effective_bucket_seconds: f64,
    pub bucket_seconds_min: f64,
    pub bucket_seconds_max: f64,
    pub bucket_seconds_step: f64,
    pub duration: f64,
    pub total_damage: f64,
    pub omitted_role_damage: f64,
    pub omitted_role_hits: u64,
    pub peak_dps: f64,
    pub time_stop_duration: f64,
    pub time_stop_intervals: Vec<TimelineIntervalProjection>,
    pub compacted_time_stop_intervals: u64,
    pub markers: Vec<TimelineMarkerProjection>,
    pub characters: Vec<TimelineCharacterProjection>,
    pub buckets: Vec<TimelineBucketProjection>,
    pub segments: Vec<TimelineSegmentProjection>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct TimelineIntervalProjection {
    pub start: f64,
    pub end: f64,
    pub pause_type_mask: Option<u32>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct TimelineMarkerProjection {
    pub offset: f64,
    pub label_key: String,
    pub kind: TimelineMarkerProjectionKind,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TimelineMarkerProjectionKind {
    Half,
    Clear,
    Exit,
}

#[derive(Clone, Debug, PartialEq)]
pub struct TimelineCharacterProjection {
    pub id: u32,
    pub name: String,
    pub color: String,
    pub total_damage: f64,
}

#[derive(Clone, Debug, PartialEq)]
pub struct TimelineBucketProjection {
    pub start: f64,
    pub end: f64,
    pub team_dps: f64,
    pub damage: f64,
    pub hits: u64,
    pub cumulative_damage: f64,
    pub roles: Vec<TimelineRoleProjection>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TimelineRoleProjection {
    pub character_id: u32,
    pub dps: f64,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TimelineSegmentProjection {
    pub start: f64,
    pub end: f64,
    pub dps: f64,
}

pub fn project_timeline(
    state: &CombatState,
    characters: &HashMap<u32, CharacterInfo>,
    options: TimelineProjectionOptions,
) -> TimelineProjection {
    let bucket_seconds = sanitize_timeline_bucket_seconds(options.bucket_seconds);
    let mut series = match options.scope {
        TimelineScope::Whole => state.timeline_bounded(
            f64::from(bucket_seconds),
            options.subtract_time_stop,
            options.max_buckets,
            options.max_roles_per_bucket,
            options.max_characters,
        ),
        TimelineScope::First => state.abyss.half(AbyssHalf::First).timeline_bounded(
            f64::from(bucket_seconds),
            options.subtract_time_stop,
            options.max_buckets,
            options.max_roles_per_bucket,
            options.max_characters,
        ),
        TimelineScope::Second => state.abyss.half(AbyssHalf::Second).timeline_bounded(
            f64::from(bucket_seconds),
            options.subtract_time_stop,
            options.max_buckets,
            options.max_roles_per_bucket,
            options.max_characters,
        ),
    };
    if let (TimelineScope::First | TimelineScope::Second, Some(start), Some(end)) =
        (options.scope, series.start_timestamp, series.end_timestamp)
    {
        let half = match options.scope {
            TimelineScope::First => AbyssHalf::First,
            TimelineScope::Second => AbyssHalf::Second,
            TimelineScope::Whole => unreachable!("whole scope handled above"),
        };
        series.markers = state.abyss.timeline_markers_for_half(half, start, end);
    }

    // The chart owns complete fixed-width buckets. A one-hit capture therefore
    // still spans one bucket instead of producing a zero-width x-axis.
    let duration = series
        .buckets
        .last()
        .map(|bucket| finite_non_negative(bucket.end_offset))
        .unwrap_or(0.0);
    let peak_dps = series
        .buckets
        .iter()
        .map(|bucket| finite_non_negative(bucket.dps))
        .fold(0.0, f64::max);

    let mut totals = HashMap::<u32, (String, f64)>::new();
    for bucket in &series.buckets {
        for role in &bucket.role_damage {
            let entry = totals
                .entry(role.char_id)
                .or_insert_with(|| (role.char_name.clone(), 0.0));
            if !role.char_name.is_empty() {
                entry.0.clone_from(&role.char_name);
            }
            entry.1 += finite_non_negative(role.damage);
        }
    }
    let mut projected_characters = totals
        .into_iter()
        .map(
            |(id, (fallback_name, total_damage))| TimelineCharacterProjection {
                id,
                name: character_name(id, &fallback_name, characters, options.language),
                color: character_color(id, characters),
                total_damage,
            },
        )
        .collect::<Vec<_>>();
    projected_characters.sort_by(|left, right| {
        right
            .total_damage
            .total_cmp(&left.total_damage)
            .then_with(|| left.name.cmp(&right.name))
            .then_with(|| left.id.cmp(&right.id))
    });

    let segments = summarize_combat_segments(&series, COMBAT_SEGMENT_GAP_SECONDS)
        .into_iter()
        .map(|segment| TimelineSegmentProjection {
            start: finite_non_negative(segment.start_offset),
            end: finite_non_negative(segment.end_offset),
            dps: finite_non_negative(segment.dps),
        })
        .collect();

    let time_stop_intervals = series
        .time_stop_intervals
        .iter()
        .map(|interval| TimelineIntervalProjection {
            start: finite_non_negative(interval.start_offset),
            end: finite_non_negative(interval.end_offset),
            pause_type_mask: interval.pause_type_mask,
        })
        .collect::<Vec<_>>();
    let time_stop_duration = timeline_time_stop_duration(&time_stop_intervals);

    TimelineProjection {
        bucket_seconds: f64::from(bucket_seconds),
        effective_bucket_seconds: series.bucket_seconds,
        bucket_seconds_min: f64::from(TIMELINE_BUCKET_SECONDS_MIN),
        bucket_seconds_max: f64::from(TIMELINE_BUCKET_SECONDS_MAX),
        bucket_seconds_step: f64::from(TIMELINE_BUCKET_SECONDS_STEP),
        duration,
        total_damage: finite_non_negative(series.total_damage),
        omitted_role_damage: finite_non_negative(series.omitted_role_damage),
        omitted_role_hits: series.omitted_role_hits,
        peak_dps,
        time_stop_duration,
        time_stop_intervals,
        compacted_time_stop_intervals: series.compacted_time_stop_intervals,
        markers: series
            .markers
            .iter()
            .map(|marker| TimelineMarkerProjection {
                offset: finite_non_negative(marker.offset),
                label_key: marker.label.clone(),
                kind: match marker.kind {
                    TimelineMarkerKind::HalfStart => TimelineMarkerProjectionKind::Half,
                    TimelineMarkerKind::Clear => TimelineMarkerProjectionKind::Clear,
                    TimelineMarkerKind::Exit => TimelineMarkerProjectionKind::Exit,
                },
            })
            .collect(),
        characters: projected_characters,
        buckets: series
            .buckets
            .iter()
            .map(|bucket| TimelineBucketProjection {
                start: finite_non_negative(bucket.start_offset),
                end: finite_non_negative(bucket.end_offset),
                team_dps: finite_non_negative(bucket.dps),
                damage: finite_non_negative(bucket.damage),
                hits: bucket.hits,
                cumulative_damage: finite_non_negative(bucket.cumulative_damage),
                roles: bucket
                    .role_damage
                    .iter()
                    .map(|role| TimelineRoleProjection {
                        character_id: role.char_id,
                        dps: finite_non_negative(role.dps),
                    })
                    .collect(),
            })
            .collect(),
        segments,
    }
}

pub(super) fn character_name(
    id: u32,
    fallback: &str,
    characters: &HashMap<u32, CharacterInfo>,
    language: Language,
) -> String {
    let localized = characters.get(&id).map(|character| match language {
        Language::SimplifiedChinese => character.name_zh.as_str(),
        Language::English | Language::Japanese => character.name_en.as_str(),
    });
    let selected = localized
        .filter(|name| !name.is_empty())
        .or_else(|| (!fallback.is_empty()).then_some(fallback))
        .filter(|name| name.len() <= MAX_TIMELINE_CHARACTER_NAME_BYTES);
    selected
        .map(str::to_owned)
        .unwrap_or_else(|| format!("#{id}"))
}

pub(super) fn character_color(id: u32, characters: &HashMap<u32, CharacterInfo>) -> String {
    if let Some(color) = characters
        .get(&id)
        .and_then(|character| character.color.as_deref())
        .filter(|color| valid_css_hex_color(color))
    {
        return color.to_owned();
    }
    const FALLBACK: [&str; 8] = [
        "#ef4444", "#f59e0b", "#8b5cf6", "#06b6d4", "#22c55e", "#ec4899", "#3b82f6", "#84cc16",
    ];
    FALLBACK[id as usize % FALLBACK.len()].to_owned()
}

fn valid_css_hex_color(value: &str) -> bool {
    matches!(value.len(), 4 | 7)
        && value.starts_with('#')
        && value[1..].bytes().all(|byte| byte.is_ascii_hexdigit())
}

pub(super) fn finite_non_negative(value: f64) -> f64 {
    if value.is_finite() {
        value.max(0.0)
    } else {
        0.0
    }
}

fn timeline_time_stop_duration(intervals: &[TimelineIntervalProjection]) -> f64 {
    intervals
        .iter()
        .map(|interval| (interval.end - interval.start).max(0.0))
        .sum()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::model::{Hit, HitCharacterSource, HitDirection};

    fn hit(timestamp: f64, char_id: u32, name: &str, damage: f64) -> Hit {
        Hit {
            timestamp,
            char_id,
            char_name: name.to_owned(),
            damage,
            char_known: true,
            byte_offset: 0,
            bit_shift: 0,
            char_source: HitCharacterSource::Packet,
            direction: HitDirection::Outgoing,
            target_hp_before: 1_000.0,
            target_hp_after: 1_000.0 - damage,
            target_max_hp: 1_000.0,
            max_hp_reduction: 0.0,
            target_hp_percent: 100.0 - damage / 10.0,
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
    fn projection_keeps_aggregated_rows_and_stable_character_order() {
        let mut state = CombatState::default();
        state.push_hit(hit(10.0, 2, "Second", 25.0));
        state.push_hit(hit(11.0, 1, "First", 75.0));
        let characters = HashMap::from([(
            1,
            CharacterInfo {
                name_zh: "第一".to_owned(),
                name_en: "First localized".to_owned(),
                color: Some("#123abc".to_owned()),
                avatar: None,
                attribute: None,
            },
        )]);

        let projection = project_timeline(
            &state,
            &characters,
            TimelineProjectionOptions {
                scope: TimelineScope::Whole,
                bucket_seconds: 1.0,
                max_buckets: 20_000,
                max_roles_per_bucket: 256,
                max_characters: 256,
                subtract_time_stop: false,
                language: Language::SimplifiedChinese,
            },
        );

        assert_eq!(projection.total_damage, 100.0);
        assert_eq!(projection.characters[0].id, 1);
        assert_eq!(projection.characters[0].name, "第一");
        assert_eq!(projection.characters[0].color, "#123abc");
        assert_eq!(projection.characters[1].name, "Second");
        assert_eq!(
            projection.buckets.iter().map(|row| row.hits).sum::<u64>(),
            2
        );
        assert!(projection.peak_dps > 0.0);
    }

    #[test]
    fn projection_sanitizes_bucket_interval_and_empty_state() {
        let projection = project_timeline(
            &CombatState::default(),
            &HashMap::new(),
            TimelineProjectionOptions {
                scope: TimelineScope::Whole,
                bucket_seconds: f32::NAN,
                max_buckets: 20_000,
                max_roles_per_bucket: 256,
                max_characters: 256,
                subtract_time_stop: true,
                language: Language::English,
            },
        );

        assert_eq!(projection.bucket_seconds, 1.0);
        assert_eq!(projection.duration, 0.0);
        assert!(projection.buckets.is_empty());
    }

    #[test]
    fn projection_bounds_character_names_by_utf8_bytes_without_splitting() {
        let exact = format!("{}ab", "界".repeat(42));
        assert_eq!(exact.len(), MAX_TIMELINE_CHARACTER_NAME_BYTES);
        assert_eq!(
            character_name(7, &exact, &HashMap::new(), Language::English),
            exact
        );

        let oversized = "界".repeat(43);
        assert_eq!(oversized.len(), MAX_TIMELINE_CHARACTER_NAME_BYTES + 1);
        assert_eq!(
            character_name(7, &oversized, &HashMap::new(), Language::English),
            "#7"
        );
    }

    #[test]
    fn projection_normalizes_zero_source_budgets_and_reports_effective_width() {
        let mut state = CombatState::default();
        state.push_hit(hit(0.0, 1, "First", 25.0));
        state.push_hit(hit(1_000.0, 2, "Second", 75.0));

        let projection = project_timeline(
            &state,
            &HashMap::new(),
            TimelineProjectionOptions {
                scope: TimelineScope::Whole,
                bucket_seconds: 0.1,
                max_buckets: 0,
                max_roles_per_bucket: 0,
                max_characters: 0,
                subtract_time_stop: false,
                language: Language::English,
            },
        );

        assert_eq!(
            projection.bucket_seconds,
            f64::from(sanitize_timeline_bucket_seconds(0.1))
        );
        assert!(projection.effective_bucket_seconds > projection.bucket_seconds);
        assert_eq!(projection.buckets.len(), 1);
        assert!(projection.buckets[0].roles.len() <= 1);
        assert!(projection.characters.len() <= 1);
        assert_eq!(projection.total_damage, 100.0);
        assert_eq!(projection.buckets[0].hits, 2);
    }

    #[test]
    fn time_stop_duration_sums_projected_intervals() {
        let duration = timeline_time_stop_duration(&[
            TimelineIntervalProjection {
                start: 1.0,
                end: 2.5,
                pause_type_mask: None,
            },
            TimelineIntervalProjection {
                start: 5.0,
                end: 7.25,
                pause_type_mask: None,
            },
        ]);

        assert_eq!(duration, 3.75);
    }
}
