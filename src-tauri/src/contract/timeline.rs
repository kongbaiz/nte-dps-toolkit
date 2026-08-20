use serde::Serialize;

use nte_dps_tool::{
    core::timeline::{
        MAX_TIMELINE_CHARACTER_NAME_BYTES, TimelineMarkerProjectionKind, TimelineProjection,
        TimelineScope,
    },
    storage::config::TimelineDpsViewMode,
};

pub(crate) const TIMELINE_CONTRACT_VERSION: u32 = 3;
/// Source-side budget for the Console read model. Longer combat remains
/// complete in authoritative state and is projected into wider buckets.
pub(crate) const MAX_TIMELINE_BUCKETS: usize = 10_000;
pub(crate) const MAX_TIMELINE_CHARACTERS: usize = 256;
/// Together with `MAX_TIMELINE_BUCKETS`, this caps a snapshot at 160,000
/// role rows. The worst-case serialized contract is regression-tested below
/// the shared 16 MiB stream-delivery budget.
pub(crate) const MAX_TIMELINE_ROLES_PER_BUCKET: usize = 16;

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct TimelineSnapshot {
    pub contract_version: u32,
    pub generation: String,
    pub scope: &'static str,
    pub view_mode: &'static str,
    pub bucket_seconds: f64,
    pub effective_bucket_seconds: f64,
    pub bucket_seconds_min: f64,
    pub bucket_seconds_max: f64,
    pub bucket_seconds_step: f64,
    pub has_data: bool,
    pub duration: f64,
    pub total_damage: f64,
    pub omitted_role_damage: f64,
    pub omitted_role_hits: String,
    pub peak_dps: f64,
    pub time_stop_duration: f64,
    pub compacted_time_stop_intervals: String,
    pub time_stop_intervals: Vec<TimelineIntervalSnapshot>,
    pub markers: Vec<TimelineMarkerSnapshot>,
    pub characters: Vec<TimelineCharacterSnapshot>,
    pub buckets: Vec<TimelineBucketSnapshot>,
    pub segments: Vec<TimelineSegmentSnapshot>,
}

impl TimelineSnapshot {
    pub(crate) fn from_projection(
        projection: &TimelineProjection,
        generation: u64,
        scope: TimelineScope,
        view_mode: TimelineDpsViewMode,
    ) -> Self {
        Self {
            contract_version: TIMELINE_CONTRACT_VERSION,
            generation: generation.to_string(),
            scope: scope_code(scope),
            view_mode: view_mode_code(view_mode),
            bucket_seconds: projection.bucket_seconds,
            effective_bucket_seconds: projection.effective_bucket_seconds,
            bucket_seconds_min: projection.bucket_seconds_min,
            bucket_seconds_max: projection.bucket_seconds_max,
            bucket_seconds_step: projection.bucket_seconds_step,
            has_data: !projection.buckets.is_empty(),
            duration: projection.duration,
            total_damage: projection.total_damage,
            omitted_role_damage: projection.omitted_role_damage,
            omitted_role_hits: projection.omitted_role_hits.to_string(),
            peak_dps: projection.peak_dps,
            time_stop_duration: projection.time_stop_duration,
            compacted_time_stop_intervals: projection.compacted_time_stop_intervals.to_string(),
            time_stop_intervals: projection
                .time_stop_intervals
                .iter()
                .map(|interval| TimelineIntervalSnapshot {
                    start: interval.start,
                    end: interval.end,
                })
                .collect(),
            markers: projection
                .markers
                .iter()
                .map(|marker| TimelineMarkerSnapshot {
                    offset: marker.offset,
                    label_key: marker.label_key.clone(),
                    kind: match marker.kind {
                        TimelineMarkerProjectionKind::Half => "half",
                        TimelineMarkerProjectionKind::Clear => "clear",
                        TimelineMarkerProjectionKind::Exit => "exit",
                    },
                })
                .collect(),
            characters: projection
                .characters
                .iter()
                .map(|character| TimelineCharacterSnapshot {
                    id: character.id,
                    name: bounded_character_name(character.id, &character.name),
                    color: character.color.clone(),
                    total_damage: character.total_damage,
                })
                .collect(),
            buckets: projection
                .buckets
                .iter()
                .map(|bucket| TimelineBucketSnapshot {
                    start: bucket.start,
                    end: bucket.end,
                    team_dps: bucket.team_dps,
                    damage: bucket.damage,
                    hits: bucket.hits.to_string(),
                    cumulative_damage: bucket.cumulative_damage,
                    roles: bucket
                        .roles
                        .iter()
                        .map(|role| TimelineRoleSnapshot {
                            character_id: role.character_id,
                            dps: role.dps,
                        })
                        .collect(),
                })
                .collect(),
            segments: projection
                .segments
                .iter()
                .map(|segment| TimelineSegmentSnapshot {
                    start: segment.start,
                    end: segment.end,
                    dps: segment.dps,
                })
                .collect(),
        }
    }
}

fn bounded_character_name(id: u32, name: &str) -> String {
    if !name.is_empty() && name.len() <= MAX_TIMELINE_CHARACTER_NAME_BYTES {
        name.to_owned()
    } else {
        format!("#{id}")
    }
}

#[derive(Clone, Copy, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct TimelineIntervalSnapshot {
    pub start: f64,
    pub end: f64,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct TimelineMarkerSnapshot {
    pub offset: f64,
    pub label_key: String,
    pub kind: &'static str,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct TimelineCharacterSnapshot {
    pub id: u32,
    pub name: String,
    pub color: String,
    pub total_damage: f64,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct TimelineBucketSnapshot {
    pub start: f64,
    pub end: f64,
    pub team_dps: f64,
    pub damage: f64,
    pub hits: String,
    pub cumulative_damage: f64,
    pub roles: Vec<TimelineRoleSnapshot>,
}

#[derive(Clone, Copy, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct TimelineRoleSnapshot {
    pub character_id: u32,
    pub dps: f64,
}

#[derive(Clone, Copy, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct TimelineSegmentSnapshot {
    pub start: f64,
    pub end: f64,
    pub dps: f64,
}

#[derive(Clone, Debug, Serialize)]
#[serde(tag = "event", content = "payload", rename_all = "camelCase")]
pub(crate) enum TimelineEvent {
    Snapshot(TimelineSnapshot),
}

pub(crate) const fn scope_code(scope: TimelineScope) -> &'static str {
    match scope {
        TimelineScope::Whole => "all",
        TimelineScope::First => "upper",
        TimelineScope::Second => "lower",
    }
}

pub(crate) const fn view_mode_code(mode: TimelineDpsViewMode) -> &'static str {
    match mode {
        TimelineDpsViewMode::Team => "team",
        TimelineDpsViewMode::Characters => "characters",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn snapshot_serializes_js_unsafe_fields_as_decimal_strings() {
        let snapshot = TimelineSnapshot::from_projection(
            &TimelineProjection::default(),
            9_007_199_254_740_992,
            TimelineScope::Whole,
            TimelineDpsViewMode::Team,
        );
        let value = serde_json::to_value(snapshot).expect("timeline snapshot serializes");
        assert_eq!(value["generation"], "9007199254740992");
        assert_eq!(value["scope"], "all");
        assert_eq!(value["viewMode"], "team");
        assert_eq!(value["hasData"], false);
        assert_eq!(value["timeStopDuration"], 0.0);
        assert_eq!(value["compactedTimeStopIntervals"], "0");
    }

    #[test]
    fn snapshot_replaces_oversized_character_names_before_serialization() {
        let exact = format!("{}ab", "界".repeat(42));
        let oversized = "界".repeat(43);
        let projection = TimelineProjection {
            characters: vec![
                nte_dps_tool::core::timeline::TimelineCharacterProjection {
                    id: 7,
                    name: exact.clone(),
                    color: "#123abc".to_owned(),
                    total_damage: 1.0,
                },
                nte_dps_tool::core::timeline::TimelineCharacterProjection {
                    id: 8,
                    name: oversized,
                    color: "#123abc".to_owned(),
                    total_damage: 1.0,
                },
            ],
            ..TimelineProjection::default()
        };

        let snapshot = TimelineSnapshot::from_projection(
            &projection,
            1,
            TimelineScope::Whole,
            TimelineDpsViewMode::Characters,
        );
        assert_eq!(snapshot.characters[0].name, exact);
        assert_eq!(snapshot.characters[1].name, "#8");
    }
}
