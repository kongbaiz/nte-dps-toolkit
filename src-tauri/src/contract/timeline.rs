use serde::Serialize;

use nte_dps_tool::{
    core::timeline::{TimelineMarkerProjectionKind, TimelineProjection, TimelineScope},
    storage::config::TimelineDpsViewMode,
};

pub(crate) const TIMELINE_CONTRACT_VERSION: u32 = 2;

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct TimelineSnapshot {
    pub contract_version: u32,
    pub generation: String,
    pub scope: &'static str,
    pub view_mode: &'static str,
    pub bucket_seconds: f64,
    pub bucket_seconds_min: f64,
    pub bucket_seconds_max: f64,
    pub bucket_seconds_step: f64,
    pub has_data: bool,
    pub duration: f64,
    pub total_damage: f64,
    pub peak_dps: f64,
    pub time_stop_duration: f64,
    pub time_stop_intervals: Vec<TimelineIntervalSnapshot>,
    pub markers: Vec<TimelineMarkerSnapshot>,
    pub characters: Vec<TimelineCharacterSnapshot>,
    pub buckets: Vec<TimelineBucketSnapshot>,
    pub segments: Vec<TimelineSegmentSnapshot>,
}

impl TimelineSnapshot {
    pub(crate) fn from_projection(
        projection: TimelineProjection,
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
            bucket_seconds_min: projection.bucket_seconds_min,
            bucket_seconds_max: projection.bucket_seconds_max,
            bucket_seconds_step: projection.bucket_seconds_step,
            has_data: !projection.buckets.is_empty(),
            duration: projection.duration,
            total_damage: projection.total_damage,
            peak_dps: projection.peak_dps,
            time_stop_duration: projection.time_stop_duration,
            time_stop_intervals: projection
                .time_stop_intervals
                .into_iter()
                .map(|interval| TimelineIntervalSnapshot {
                    start: interval.start,
                    end: interval.end,
                })
                .collect(),
            markers: projection
                .markers
                .into_iter()
                .map(|marker| TimelineMarkerSnapshot {
                    offset: marker.offset,
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
                .map(|character| TimelineCharacterSnapshot {
                    id: character.id,
                    name: character.name,
                    color: character.color,
                    total_damage: character.total_damage,
                })
                .collect(),
            buckets: projection
                .buckets
                .into_iter()
                .map(|bucket| TimelineBucketSnapshot {
                    start: bucket.start,
                    end: bucket.end,
                    team_dps: bucket.team_dps,
                    damage: bucket.damage,
                    hits: bucket.hits.to_string(),
                    cumulative_damage: bucket.cumulative_damage,
                    roles: bucket
                        .roles
                        .into_iter()
                        .map(|role| TimelineRoleSnapshot {
                            character_id: role.character_id,
                            dps: role.dps,
                        })
                        .collect(),
                })
                .collect(),
            segments: projection
                .segments
                .into_iter()
                .map(|segment| TimelineSegmentSnapshot {
                    start: segment.start,
                    end: segment.end,
                    dps: segment.dps,
                })
                .collect(),
        }
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
            TimelineProjection::default(),
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
    }
}
