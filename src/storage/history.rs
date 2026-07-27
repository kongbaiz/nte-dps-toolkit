use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use chrono::{DateTime, Local, Utc};
use serde::{Deserialize, Serialize};

use crate::engine::model::{
    AbyssHalf, CombatSessionCharacterSummary, CombatSessionSkillSummary, CombatSessionSummary,
    CombatState, Hit, TeamDps, TeamDpsMember, TimeStopEvent,
};
use crate::storage::io_util::atomic_write_text;
use crate::storage::paths::software_dir;

pub const HISTORY_RECORD_VERSION: u32 = 1;
pub const MAX_HISTORY_RECORDS: usize = 200;
const MAX_HISTORY_DETAIL_HITS: usize = 100_000;
const MAX_HISTORY_IMPORT_BYTES: u64 = 128 * 1024 * 1024;

static HISTORY_ID_COUNTER: AtomicU64 = AtomicU64::new(0);

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct HistoryRecord {
    pub version: u32,
    pub id: String,
    pub saved_at: DateTime<Utc>,
    pub summary: CombatSessionSummary,
    pub details: Option<HistoryCombatDetails>,
}

impl Default for HistoryRecord {
    fn default() -> Self {
        Self {
            version: HISTORY_RECORD_VERSION,
            id: String::new(),
            saved_at: Utc::now(),
            summary: CombatSessionSummary::default(),
            details: None,
        }
    }
}

impl HistoryRecord {
    pub fn display_time(&self) -> String {
        self.saved_at
            .with_timezone(&Local)
            .format("%Y-%m-%d %H:%M:%S")
            .to_string()
    }

    pub fn to_team_dps(&self) -> Option<TeamDps> {
        team_from_characters(self.summary.total_dps, &self.summary.characters)
    }

    pub fn upper_team_dps(&self) -> Option<TeamDps> {
        self.summary
            .abyss
            .first_half
            .as_ref()
            .and_then(|half| team_from_characters(half.total_dps, &half.characters))
            .or_else(|| self.to_team_dps())
    }

    pub fn lower_team_dps(&self) -> Option<TeamDps> {
        self.summary
            .abyss
            .second_half
            .as_ref()
            .and_then(|half| team_from_characters(half.total_dps, &half.characters))
            .or_else(|| self.to_team_dps())
    }
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct HistoryCombatDetails {
    pub floor: Option<u32>,
    pub active_half: Option<AbyssHalf>,
    pub first_half_at: Option<f64>,
    pub second_half_at: Option<f64>,
    pub success_at: Option<f64>,
    pub exited_at: Option<f64>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub global_hits: Vec<Hit>,
    pub first_half_hits: Vec<Hit>,
    pub second_half_hits: Vec<Hit>,
    pub time_stop_events: Vec<TimeStopEvent>,
}

impl HistoryCombatDetails {
    pub fn from_state(state: &CombatState) -> Option<Self> {
        let abyss = &state.abyss;
        let has_abyss_hits =
            !abyss.first_half.hits.is_empty() || !abyss.second_half.hits.is_empty();
        if !has_abyss_hits && state.hits.is_empty() {
            return None;
        }
        let (round_started_at, round_ended_at) = if has_abyss_hits {
            (
                [
                    abyss.first_half_at,
                    abyss.second_half_at,
                    abyss.first_half.started_at,
                    abyss.second_half.started_at,
                ]
                .into_iter()
                .flatten()
                .min_by(f64::total_cmp),
                [
                    abyss.first_half.ended_at,
                    abyss.second_half.ended_at,
                    abyss.success_at,
                    abyss.exited_at,
                ]
                .into_iter()
                .flatten()
                .max_by(f64::total_cmp),
            )
        } else {
            (state.started_at, state.ended_at)
        };
        Some(Self {
            floor: if has_abyss_hits { abyss.floor } else { None },
            active_half: if has_abyss_hits {
                abyss.active_half
            } else {
                None
            },
            first_half_at: if has_abyss_hits {
                abyss.first_half_at
            } else {
                None
            },
            second_half_at: if has_abyss_hits {
                abyss.second_half_at
            } else {
                None
            },
            success_at: if has_abyss_hits {
                abyss.success_at
            } else {
                None
            },
            exited_at: if has_abyss_hits {
                abyss.exited_at
            } else {
                None
            },
            global_hits: if has_abyss_hits {
                Vec::new()
            } else {
                state.hits.iter().cloned().collect()
            },
            first_half_hits: abyss.first_half.hits.iter().cloned().collect(),
            second_half_hits: abyss.second_half.hits.iter().cloned().collect(),
            time_stop_events: state
                .time_stop_events
                .iter()
                .filter(|event| {
                    let timestamp = match event {
                        TimeStopEvent::GamePauseStarted { timestamp, .. }
                        | TimeStopEvent::GamePauseEnded { timestamp, .. } => *timestamp,
                    };
                    round_started_at.is_none_or(|start| timestamp >= start)
                        && round_ended_at.is_none_or(|end| timestamp <= end)
                })
                .cloned()
                .collect(),
        })
    }

    pub fn to_combat_state(&self) -> CombatState {
        let mut state = CombatState::default();
        state.abyss.floor = self.floor;
        state.abyss.active_half = self.active_half;
        state.abyss.first_half_at = self.first_half_at;
        state.abyss.second_half_at = self.second_half_at;
        state.abyss.success_at = self.success_at;
        state.abyss.exited_at = self.exited_at;
        for hit in &self.global_hits {
            state.push_hit(hit.clone());
        }
        for hit in &self.first_half_hits {
            state.abyss.first_half.push_hit(hit.clone());
        }
        for hit in &self.second_half_hits {
            state.abyss.second_half.push_hit(hit.clone());
        }
        for event in &self.time_stop_events {
            state.apply_time_stop_event(event.clone());
        }
        if self.global_hits.is_empty() {
            state.rebuild_global_from_abyss();
        }
        state
    }

    fn validate(&self) -> Result<(), String> {
        if !self.global_hits.is_empty()
            && (!self.first_half_hits.is_empty() || !self.second_half_hits.is_empty())
        {
            return Err("History detail mixes global and abyss hits".to_owned());
        }
        if self.global_hits.len() + self.first_half_hits.len() + self.second_half_hits.len()
            > MAX_HISTORY_DETAIL_HITS
        {
            return Err("History detail hit count exceeds the supported limit".to_owned());
        }
        for timestamp in [
            self.first_half_at,
            self.second_half_at,
            self.success_at,
            self.exited_at,
        ]
        .into_iter()
        .flatten()
        {
            if !timestamp.is_finite() {
                return Err("History detail contains an invalid timestamp".to_owned());
            }
        }
        if self
            .global_hits
            .iter()
            .chain(&self.first_half_hits)
            .chain(&self.second_half_hits)
            .any(|hit| {
                !hit.timestamp.is_finite()
                    || !hit.damage.is_finite()
                    || !hit.follow_up_damage.is_finite()
                    || hit
                        .follow_up_timestamp
                        .is_some_and(|timestamp| !timestamp.is_finite())
            })
        {
            return Err("History detail contains an invalid hit".to_owned());
        }
        if self.time_stop_events.iter().any(|event| {
            let timestamp = match event {
                TimeStopEvent::GamePauseStarted { timestamp, .. }
                | TimeStopEvent::GamePauseEnded { timestamp, .. } => timestamp,
            };
            !timestamp.is_finite()
        }) {
            return Err("History detail contains an invalid time-stop event".to_owned());
        }
        Ok(())
    }
}

fn team_from_characters(dps: f64, characters: &[CombatSessionCharacterSummary]) -> Option<TeamDps> {
    (dps > 0.0).then(|| TeamDps {
        dps,
        members: characters
            .iter()
            .filter(|row| row.damage > 0.0)
            .take(crate::engine::model::TEAM_DPS_MAX_MEMBERS)
            .map(|row| TeamDpsMember {
                id: row.char_id,
                dps: row.dps,
                name: row.name.clone(),
            })
            .collect(),
    })
}

#[derive(Clone, Debug, Default)]
pub struct HistoryLoadResult {
    pub records: Vec<HistoryRecord>,
    pub skipped_files: usize,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct HistoryComparison {
    pub left_id: String,
    pub right_id: String,
    pub total_dps_delta: f64,
    pub total_damage_delta: f64,
    pub duration_delta: f64,
    pub character_deltas: Vec<HistoryCharacterDelta>,
    pub skill_deltas: Vec<HistorySkillDelta>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct HistoryCharacterDelta {
    pub char_id: u32,
    pub name: String,
    pub left_dps: f64,
    pub right_dps: f64,
    pub delta_dps: f64,
    pub left_damage: f64,
    pub right_damage: f64,
    pub delta_damage: f64,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct HistorySkillDelta {
    pub name: String,
    pub category: String,
    pub ability_name: Option<String>,
    pub gameplay_effect_name: Option<String>,
    pub left_damage: f64,
    pub right_damage: f64,
    pub delta_damage: f64,
}

pub fn history_dir() -> PathBuf {
    software_dir().join("history")
}

pub fn load_history() -> HistoryLoadResult {
    load_history_from_dir(&history_dir())
}

pub fn load_history_from_dir(directory: &Path) -> HistoryLoadResult {
    let mut result = HistoryLoadResult::default();
    let Ok(entries) = fs::read_dir(directory) else {
        return result;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|ext| ext.to_str()) != Some("json") {
            continue;
        }
        match fs::read_to_string(&path)
            .map_err(|error| error.to_string())
            .and_then(|text| parse_history_record(&text, &path))
        {
            Ok(record) => result.records.push(record),
            Err(_) => result.skipped_files += 1,
        }
    }
    sort_records_newest_first(&mut result.records);
    result
}

pub fn save_summary(summary: CombatSessionSummary) -> Result<HistoryRecord, String> {
    save_summary_to_dir(&history_dir(), summary)
}

pub fn save_summary_with_details(
    summary: CombatSessionSummary,
    details: HistoryCombatDetails,
) -> Result<HistoryRecord, String> {
    save_summary_with_details_to_dir(&history_dir(), summary, details)
}

pub fn import_record(path: &Path) -> Result<HistoryRecord, String> {
    import_record_to_dir(&history_dir(), path)
}

pub fn import_record_to_dir(directory: &Path, source_path: &Path) -> Result<HistoryRecord, String> {
    let metadata = fs::metadata(source_path).map_err(|error| error.to_string())?;
    if !metadata.is_file() {
        return Err("History record path is not a file".to_owned());
    }
    if metadata.len() > MAX_HISTORY_IMPORT_BYTES {
        return Err("History record exceeds the supported file size".to_owned());
    }
    let text = fs::read_to_string(source_path).map_err(|error| error.to_string())?;
    let mut record = parse_history_record(&text, source_path)?;
    record.version = HISTORY_RECORD_VERSION;
    record.id = generate_record_id(Utc::now());

    fs::create_dir_all(directory).map_err(|error| error.to_string())?;
    let text = serde_json::to_string_pretty(&record).map_err(|error| error.to_string())?;
    atomic_write_text(&record_path(directory, &record), &format!("{text}\n"))?;
    prune_history_dir(directory, MAX_HISTORY_RECORDS)?;
    Ok(record)
}

pub fn save_summary_to_dir(
    directory: &Path,
    summary: CombatSessionSummary,
) -> Result<HistoryRecord, String> {
    save_record_to_dir(directory, summary, None)
}

pub fn save_summary_with_details_to_dir(
    directory: &Path,
    summary: CombatSessionSummary,
    details: HistoryCombatDetails,
) -> Result<HistoryRecord, String> {
    details.validate()?;
    save_record_to_dir(directory, summary, Some(details))
}

fn save_record_to_dir(
    directory: &Path,
    summary: CombatSessionSummary,
    details: Option<HistoryCombatDetails>,
) -> Result<HistoryRecord, String> {
    fs::create_dir_all(directory).map_err(|error| error.to_string())?;
    let saved_at = Utc::now();
    let id = generate_record_id(saved_at);
    let record = HistoryRecord {
        version: HISTORY_RECORD_VERSION,
        id,
        saved_at,
        summary,
        details,
    };
    let text = serde_json::to_string_pretty(&record).map_err(|error| error.to_string())?;
    atomic_write_text(&record_path(directory, &record), &format!("{text}\n"))?;
    prune_history_dir(directory, MAX_HISTORY_RECORDS)?;
    Ok(record)
}

pub fn delete_record(record_id: &str) -> Result<bool, String> {
    delete_record_from_dir(&history_dir(), record_id)
}

pub fn restore_record(record: &HistoryRecord) -> Result<(), String> {
    restore_record_to_dir(&history_dir(), record)
}

pub fn restore_record_to_dir(directory: &Path, record: &HistoryRecord) -> Result<(), String> {
    if !valid_record_id(&record.id) {
        return Err("Invalid history record ID".to_owned());
    }
    fs::create_dir_all(directory).map_err(|error| error.to_string())?;
    let text = serde_json::to_string_pretty(record).map_err(|error| error.to_string())?;
    // Undo must restore exactly the deleted record without evicting a different history entry.
    // A save performed during the undo window can temporarily put the directory one over the cap;
    // the next normal save applies the existing pruning policy.
    atomic_write_text(&record_path(directory, record), &format!("{text}\n"))
}

pub fn delete_record_from_dir(directory: &Path, record_id: &str) -> Result<bool, String> {
    if !valid_record_id(record_id) {
        return Err("Invalid history record ID".to_owned());
    }
    let mut deleted = false;
    for entry in fs::read_dir(directory).map_err(|error| error.to_string())? {
        let path = entry.map_err(|error| error.to_string())?.path();
        if path.extension().and_then(|ext| ext.to_str()) != Some("json") {
            continue;
        }
        let matches_id = fs::read_to_string(&path)
            .map_err(|error| error.to_string())
            .and_then(|text| parse_history_record(&text, &path))
            .is_ok_and(|record| record.id == record_id);
        if matches_id {
            fs::remove_file(&path).map_err(|error| error.to_string())?;
            deleted = true;
        }
    }
    Ok(deleted)
}

pub fn compare_records(left: &HistoryRecord, right: &HistoryRecord) -> HistoryComparison {
    let mut character_deltas =
        compare_characters(&left.summary.characters, &right.summary.characters);
    character_deltas.sort_by(|left, right| {
        right
            .delta_damage
            .abs()
            .total_cmp(&left.delta_damage.abs())
            .then_with(|| left.name.cmp(&right.name))
    });
    character_deltas.truncate(8);

    let mut skill_deltas = compare_skills(&left.summary.skills, &right.summary.skills);
    skill_deltas.sort_by(|left, right| {
        right
            .delta_damage
            .abs()
            .total_cmp(&left.delta_damage.abs())
            .then_with(|| left.name.cmp(&right.name))
    });
    skill_deltas.truncate(8);

    HistoryComparison {
        left_id: left.id.clone(),
        right_id: right.id.clone(),
        total_dps_delta: right.summary.total_dps - left.summary.total_dps,
        total_damage_delta: right.summary.total_damage - left.summary.total_damage,
        duration_delta: right.summary.duration_seconds - left.summary.duration_seconds,
        character_deltas,
        skill_deltas,
    }
}

fn parse_history_record(text: &str, path: &Path) -> Result<HistoryRecord, String> {
    let mut record: HistoryRecord =
        serde_json::from_str(text).map_err(|error| error.to_string())?;
    if record.version > HISTORY_RECORD_VERSION {
        return Err(format!("Unsupported history version {}", record.version));
    }
    if record.id.trim().is_empty() {
        record.id = path
            .file_stem()
            .and_then(|stem| stem.to_str())
            .unwrap_or("legacy")
            .to_owned();
    }
    if !valid_record_id(&record.id) {
        return Err("Invalid history record ID".to_owned());
    }
    if let Some(details) = &record.details {
        details.validate()?;
    }
    Ok(record)
}

fn valid_record_id(record_id: &str) -> bool {
    !record_id.is_empty()
        && record_id.len() <= 128
        && record_id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
}

fn record_path(directory: &Path, record: &HistoryRecord) -> PathBuf {
    directory.join(format!(
        "{}_{}.json",
        record
            .saved_at
            .with_timezone(&Local)
            .format("%Y%m%d_%H%M%S"),
        record.id
    ))
}

fn prune_history_dir(directory: &Path, max_records: usize) -> Result<(), String> {
    let mut files = Vec::new();
    for entry in fs::read_dir(directory).map_err(|error| error.to_string())? {
        let path = entry.map_err(|error| error.to_string())?.path();
        if path.extension().and_then(|ext| ext.to_str()) != Some("json") {
            continue;
        }
        let modified = fs::metadata(&path)
            .and_then(|metadata| metadata.modified())
            .map_err(|error| error.to_string())?;
        files.push((modified, path));
    }
    files.sort_by_key(|(modified, _)| *modified);
    let remove_count = files.len().saturating_sub(max_records);
    for (_, path) in files.into_iter().take(remove_count) {
        fs::remove_file(path).map_err(|error| error.to_string())?;
    }
    Ok(())
}

fn sort_records_newest_first(records: &mut [HistoryRecord]) {
    records.sort_by(|left, right| {
        right
            .saved_at
            .cmp(&left.saved_at)
            .then_with(|| right.id.cmp(&left.id))
    });
}

fn generate_record_id(saved_at: DateTime<Utc>) -> String {
    let counter = HISTORY_ID_COUNTER.fetch_add(1, Ordering::Relaxed);
    let nanos = saved_at
        .timestamp_nanos_opt()
        .unwrap_or_else(|| saved_at.timestamp_millis().saturating_mul(1_000_000));
    let mut value = (nanos as u64) ^ ((std::process::id() as u64) << 32) ^ counter;
    value ^= value << 13;
    value ^= value >> 7;
    value ^= value << 17;
    format!("{:08x}", value & 0xffff_ffff)
}

fn compare_characters(
    left: &[CombatSessionCharacterSummary],
    right: &[CombatSessionCharacterSummary],
) -> Vec<HistoryCharacterDelta> {
    let mut rows = std::collections::HashMap::<u32, HistoryCharacterDelta>::new();
    for row in left {
        rows.insert(
            row.char_id,
            HistoryCharacterDelta {
                char_id: row.char_id,
                name: row.name.clone(),
                left_dps: row.dps,
                left_damage: row.damage,
                ..Default::default()
            },
        );
    }
    for row in right {
        let entry = rows
            .entry(row.char_id)
            .or_insert_with(|| HistoryCharacterDelta {
                char_id: row.char_id,
                name: row.name.clone(),
                ..Default::default()
            });
        if entry.name.is_empty() {
            entry.name.clone_from(&row.name);
        }
        entry.right_dps = row.dps;
        entry.right_damage = row.damage;
    }
    for row in rows.values_mut() {
        row.delta_dps = row.right_dps - row.left_dps;
        row.delta_damage = row.right_damage - row.left_damage;
    }
    rows.into_values().collect()
}

fn compare_skills(
    left: &[CombatSessionSkillSummary],
    right: &[CombatSessionSkillSummary],
) -> Vec<HistorySkillDelta> {
    let mut rows = std::collections::HashMap::<(String, String), HistorySkillDelta>::new();
    for row in left {
        let comparison_name = row.damage_name.as_ref().unwrap_or(&row.name);
        let key = skill_comparison_key(row, right);
        let entry = rows.entry(key).or_insert_with(|| HistorySkillDelta {
            name: comparison_name.clone(),
            category: row.category.clone(),
            ..Default::default()
        });
        preserve_skill_delta_identity(entry, row);
        entry.left_damage += row.damage;
    }
    for row in right {
        let comparison_name = row.damage_name.as_ref().unwrap_or(&row.name);
        let key = skill_comparison_key(row, left);
        let entry = rows.entry(key).or_insert_with(|| HistorySkillDelta {
            name: comparison_name.clone(),
            category: row.category.clone(),
            ..Default::default()
        });
        preserve_skill_delta_identity(entry, row);
        entry.right_damage += row.damage;
    }
    for row in rows.values_mut() {
        row.delta_damage = row.right_damage - row.left_damage;
    }
    rows.into_values().collect()
}

fn preserve_skill_delta_identity(
    delta: &mut HistorySkillDelta,
    summary: &CombatSessionSkillSummary,
) {
    if delta.ability_name.is_none() {
        delta.ability_name.clone_from(&summary.ability_name);
    }
    if delta.gameplay_effect_name.is_none() {
        delta
            .gameplay_effect_name
            .clone_from(&summary.gameplay_effect_name);
    }
}

fn skill_comparison_key(
    row: &CombatSessionSkillSummary,
    other: &[CombatSessionSkillSummary],
) -> (String, String) {
    if row.ability_name.is_none() && row.gameplay_effect_name.is_none() {
        return (format!("legacy:{}", row.name), row.category.clone());
    }
    if let Some(display_name) = row.damage_name.as_deref()
        && other.iter().any(|candidate| {
            candidate.category == row.category
                && candidate.ability_name.is_none()
                && candidate.gameplay_effect_name.is_none()
                && candidate.name == display_name
        })
    {
        return (format!("legacy:{display_name}"), row.category.clone());
    }
    let identity = match row.ability_name.as_deref() {
        Some(ability_name) => {
            if let Some(component_name) = row
                .damage_name
                .as_deref()
                .filter(|damage_name| *damage_name == row.name && *damage_name != ability_name)
            {
                format!("ability:{ability_name}:component:{component_name}")
            } else {
                format!("ability:{ability_name}")
            }
        }
        None => format!(
            "effect:{}",
            row.gameplay_effect_name
                .as_deref()
                .expect("skill comparison row has a stable effect identity")
        ),
    };
    (format!("stable:{identity}"), row.category.clone())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::model::{
        AbyssHalf, CombatSessionAbyssHalfSummary, CombatSessionAbyssSummary,
        CombatSessionCharacterSummary, CombatSessionSummary, DpsTimeBasis, HitCharacterSource,
        HitDirection,
    };

    #[test]
    fn loads_legacy_version_record() {
        let directory = temp_history_dir("legacy");
        fs::create_dir_all(&directory).unwrap();
        fs::write(
            directory.join("legacy.json"),
            r#"{"version":0,"id":"","saved_at":"2026-01-01T00:00:00Z","summary":{"dps_time_mode":"扣除时停","total_damage":100.0,"abyss":{"detected":true,"active_half":"上行线","first_half":{"half":"Ascending Line"},"second_half":{"half":"下りライン"}}}}"#,
        )
        .unwrap();

        let result = load_history_from_dir(&directory);

        assert_eq!(result.skipped_files, 0);
        assert_eq!(result.records.len(), 1);
        assert_eq!(result.records[0].version, 0);
        assert!(!result.records[0].id.is_empty());
        assert_eq!(
            result.records[0].summary.dps_time_mode,
            DpsTimeBasis::SubtractTimeStop
        );
        let abyss = &result.records[0].summary.abyss;
        assert_eq!(abyss.active_half, Some(AbyssHalf::First));
        assert_eq!(abyss.first_half.as_ref().unwrap().half, AbyssHalf::First);
        assert_eq!(abyss.second_half.as_ref().unwrap().half, AbyssHalf::Second);
        assert!(result.records[0].details.is_none());
        let _ = fs::remove_dir_all(directory);
    }

    #[test]
    fn detailed_abyss_record_roundtrips_and_rebuilds_both_halves() {
        let directory = temp_history_dir("details");
        let mut state = CombatState::default();
        state.abyss.floor = Some(12);
        state.abyss.active_half = Some(AbyssHalf::Second);
        state.abyss.first_half_at = Some(10.0);
        state.abyss.second_half_at = Some(20.0);
        state.abyss.first_half.push_hit(history_hit(11.0, 1, 100.0));
        state
            .abyss
            .second_half
            .push_hit(history_hit(21.0, 2, 200.0));
        state
            .time_stop_events
            .push(TimeStopEvent::GamePauseStarted {
                timestamp: 1.0,
                pause_type_mask: 1,
            });
        state.time_stop_events.push(TimeStopEvent::GamePauseEnded {
            timestamp: 2.0,
            pause_type_mask: 1,
        });
        state.apply_time_stop_event(TimeStopEvent::GamePauseStarted {
            timestamp: 12.0,
            pause_type_mask: 1,
        });
        state.apply_time_stop_event(TimeStopEvent::GamePauseEnded {
            timestamp: 13.0,
            pause_type_mask: 1,
        });
        state.rebuild_global_from_abyss();
        let details = HistoryCombatDetails::from_state(&state).unwrap();
        assert_eq!(details.time_stop_events.len(), 2);

        save_summary_with_details_to_dir(&directory, CombatSessionSummary::default(), details)
            .unwrap();
        let records = load_history_from_dir(&directory).records;
        let restored = records[0].details.as_ref().unwrap().to_combat_state();

        assert_eq!(restored.abyss.floor, Some(12));
        assert_eq!(restored.abyss.active_half, Some(AbyssHalf::Second));
        assert_eq!(restored.abyss.first_half.hits.len(), 1);
        assert_eq!(restored.abyss.second_half.hits.len(), 1);
        assert_eq!(restored.hits.len(), 2);
        assert_eq!(restored.total_damage, 300.0);
        assert_eq!(restored.time_stop_events.len(), 2);
        let _ = fs::remove_dir_all(directory);
    }

    #[test]
    fn detailed_global_record_roundtrips_without_creating_an_abyss_run() {
        let mut state = CombatState::default();
        state.push_hit(history_hit(11.0, 1, 100.0));
        state.push_hit(history_hit(12.0, 2, 200.0));
        state.apply_time_stop_event(TimeStopEvent::GamePauseStarted {
            timestamp: 11.2,
            pause_type_mask: 1,
        });
        state.apply_time_stop_event(TimeStopEvent::GamePauseEnded {
            timestamp: 11.5,
            pause_type_mask: 1,
        });

        let details = HistoryCombatDetails::from_state(&state).unwrap();
        let restored = details.to_combat_state();

        assert_eq!(details.global_hits.len(), 2);
        assert!(details.first_half_hits.is_empty());
        assert!(details.second_half_hits.is_empty());
        assert!(!restored.abyss.is_active());
        assert_eq!(restored.hits.len(), 2);
        assert_eq!(restored.total_damage, 300.0);
        assert_eq!(restored.time_stop_events.len(), 2);
    }

    #[test]
    fn imported_record_preserves_details_with_a_fresh_local_id() {
        let source_directory = temp_history_dir("import_source");
        let destination_directory = temp_history_dir("import_destination");
        let mut state = CombatState::default();
        state.push_hit(history_hit(11.0, 1, 100.0));
        let details = HistoryCombatDetails::from_state(&state).unwrap();
        let original = save_summary_with_details_to_dir(
            &source_directory,
            CombatSessionSummary::default(),
            details,
        )
        .unwrap();
        let export_path = source_directory.join("exported.json");
        fs::write(
            &export_path,
            serde_json::to_string_pretty(&original).unwrap(),
        )
        .unwrap();

        let first = import_record_to_dir(&destination_directory, &export_path).unwrap();
        let second = import_record_to_dir(&destination_directory, &export_path).unwrap();

        assert_ne!(first.id, original.id);
        assert_ne!(second.id, first.id);
        assert_eq!(first.version, HISTORY_RECORD_VERSION);
        assert_eq!(first.saved_at, original.saved_at);
        assert_eq!(
            first
                .details
                .as_ref()
                .unwrap()
                .to_combat_state()
                .total_damage,
            100.0
        );
        let loaded = load_history_from_dir(&destination_directory);
        assert_eq!(loaded.skipped_files, 0);
        assert_eq!(loaded.records.len(), 2);
        let _ = fs::remove_dir_all(source_directory);
        let _ = fs::remove_dir_all(destination_directory);
    }

    #[test]
    fn history_import_rejects_future_versions_and_oversized_files() {
        let source_directory = temp_history_dir("import_invalid_source");
        let destination_directory = temp_history_dir("import_invalid_destination");
        fs::create_dir_all(&source_directory).unwrap();
        let future_path = source_directory.join("future.json");
        fs::write(
            &future_path,
            r#"{"version":2,"id":"future","saved_at":"2026-01-01T00:00:00Z"}"#,
        )
        .unwrap();
        assert_eq!(
            import_record_to_dir(&destination_directory, &future_path).unwrap_err(),
            "Unsupported history version 2"
        );

        let oversized_path = source_directory.join("oversized.json");
        fs::File::create(&oversized_path)
            .unwrap()
            .set_len(MAX_HISTORY_IMPORT_BYTES + 1)
            .unwrap();
        assert_eq!(
            import_record_to_dir(&destination_directory, &oversized_path).unwrap_err(),
            "History record exceeds the supported file size"
        );
        let _ = fs::remove_dir_all(source_directory);
        let _ = fs::remove_dir_all(destination_directory);
    }

    #[test]
    fn skips_corrupt_json() {
        let directory = temp_history_dir("corrupt");
        fs::create_dir_all(&directory).unwrap();
        fs::write(directory.join("bad.json"), "{not json").unwrap();

        let result = load_history_from_dir(&directory);

        assert_eq!(result.records.len(), 0);
        assert_eq!(result.skipped_files, 1);
        let _ = fs::remove_dir_all(directory);
    }

    #[test]
    fn skips_history_with_unsafe_record_id() {
        let directory = temp_history_dir("unsafe_id");
        fs::create_dir_all(&directory).unwrap();
        fs::write(
            directory.join("unsafe.json"),
            r#"{"id":"../outside","saved_at":"2026-01-01T00:00:00Z"}"#,
        )
        .unwrap();

        let result = load_history_from_dir(&directory);

        assert!(result.records.is_empty());
        assert_eq!(result.skipped_files, 1);
        let _ = fs::remove_dir_all(directory);
    }

    #[test]
    fn deleted_record_can_be_restored_exactly() {
        let directory = temp_history_dir("restore");
        let record = HistoryRecord {
            id: "restore-me".to_owned(),
            summary: CombatSessionSummary {
                total_damage: 321.0,
                ..Default::default()
            },
            ..Default::default()
        };

        restore_record_to_dir(&directory, &record).unwrap();
        assert!(delete_record_from_dir(&directory, &record.id).unwrap());
        assert!(load_history_from_dir(&directory).records.is_empty());

        restore_record_to_dir(&directory, &record).unwrap();
        let restored = load_history_from_dir(&directory).records;
        assert_eq!(restored.len(), 1);
        assert_eq!(restored[0].id, record.id);
        assert_eq!(restored[0].summary.total_damage, 321.0);
        let _ = fs::remove_dir_all(directory);
    }

    #[test]
    fn deleting_record_does_not_match_an_id_suffix() {
        let directory = temp_history_dir("delete_exact_id");
        let selected = HistoryRecord {
            id: "abc".to_owned(),
            ..Default::default()
        };
        let other = HistoryRecord {
            id: "x_abc".to_owned(),
            ..Default::default()
        };
        restore_record_to_dir(&directory, &selected).unwrap();
        restore_record_to_dir(&directory, &other).unwrap();

        assert!(delete_record_from_dir(&directory, &selected.id).unwrap());
        let remaining = load_history_from_dir(&directory).records;
        assert_eq!(remaining.len(), 1);
        assert_eq!(remaining[0].id, other.id);
        let _ = fs::remove_dir_all(directory);
    }

    #[test]
    fn prunes_oldest_records() {
        let directory = temp_history_dir("prune");
        fs::create_dir_all(&directory).unwrap();
        for index in 0..3 {
            let mut summary = CombatSessionSummary {
                total_damage: index as f64,
                ..Default::default()
            };
            summary.characters.push(CombatSessionCharacterSummary {
                char_id: index,
                name: format!("角色{index}"),
                damage: index as f64,
                dps: index as f64,
                ..Default::default()
            });
            save_summary_to_dir(&directory, summary).unwrap();
        }

        prune_history_dir(&directory, 2).unwrap();
        let files = fs::read_dir(&directory).unwrap().count();

        assert_eq!(files, 2);
        let _ = fs::remove_dir_all(directory);
    }

    #[test]
    fn abyss_record_labels_and_prediction_teams_use_each_half() {
        let record = HistoryRecord {
            summary: CombatSessionSummary {
                total_dps: 999.0,
                characters: vec![
                    character(1, "上角色", 100.0, 10.0),
                    character(2, "下角色", 200.0, 20.0),
                ],
                abyss: CombatSessionAbyssSummary {
                    detected: true,
                    first_half: Some(CombatSessionAbyssHalfSummary {
                        half: AbyssHalf::First,
                        total_dps: 10.0,
                        characters: vec![character(1, "上角色", 100.0, 10.0)],
                        ..Default::default()
                    }),
                    second_half: Some(CombatSessionAbyssHalfSummary {
                        half: AbyssHalf::Second,
                        total_dps: 20.0,
                        characters: vec![character(2, "下角色", 200.0, 20.0)],
                        ..Default::default()
                    }),
                    ..Default::default()
                },
                ..Default::default()
            },
            ..Default::default()
        };

        assert_eq!(record.upper_team_dps().unwrap().members[0].id, 1);
        assert_eq!(record.lower_team_dps().unwrap().members[0].id, 2);
    }

    #[test]
    fn compare_records_aggregates_duplicate_skill_rows() {
        let left = HistoryRecord {
            id: "left".to_owned(),
            summary: CombatSessionSummary {
                skills: vec![
                    skill("待映射技能", "未知", 100.0),
                    skill("待映射技能", "未知", 25.0),
                ],
                ..Default::default()
            },
            ..Default::default()
        };
        let right = HistoryRecord {
            id: "right".to_owned(),
            summary: CombatSessionSummary {
                skills: vec![
                    skill("待映射技能", "未知", 10.0),
                    skill("待映射技能", "未知", 5.0),
                ],
                ..Default::default()
            },
            ..Default::default()
        };

        let comparison = compare_records(&left, &right);

        assert_eq!(comparison.skill_deltas.len(), 1);
        let delta = &comparison.skill_deltas[0];
        assert_eq!(delta.left_damage, 125.0);
        assert_eq!(delta.right_damage, 15.0);
        assert_eq!(delta.delta_damage, -110.0);
    }

    #[test]
    fn compare_records_matches_legacy_display_name_to_stable_skill_identity() {
        let left = HistoryRecord {
            id: "legacy".to_owned(),
            summary: CombatSessionSummary {
                skills: vec![skill("Test Ultimate", "Q技能", 100.0)],
                ..Default::default()
            },
            ..Default::default()
        };
        let right = HistoryRecord {
            id: "stable".to_owned(),
            summary: CombatSessionSummary {
                skills: vec![CombatSessionSkillSummary {
                    name: "GA_Test_UltraSkill".to_owned(),
                    category: "Q技能".to_owned(),
                    ability_name: Some("GA_Test_UltraSkill".to_owned()),
                    damage_name: Some("Test Ultimate".to_owned()),
                    damage: 125.0,
                    ..Default::default()
                }],
                ..Default::default()
            },
            ..Default::default()
        };

        let comparison = compare_records(&left, &right);

        assert_eq!(comparison.skill_deltas.len(), 1);
        let delta = &comparison.skill_deltas[0];
        assert_eq!(delta.name, "Test Ultimate");
        assert_eq!(delta.ability_name.as_deref(), Some("GA_Test_UltraSkill"));
        assert_eq!(delta.left_damage, 100.0);
        assert_eq!(delta.right_damage, 125.0);
        assert_eq!(delta.delta_damage, 25.0);
    }

    #[test]
    fn compare_records_keeps_distinct_stable_skills_with_shared_display_name() {
        let stable_skill = |ability_name: &str, damage: f64| CombatSessionSkillSummary {
            name: ability_name.to_owned(),
            category: "Q技能".to_owned(),
            ability_name: Some(ability_name.to_owned()),
            damage_name: Some("Shared Display".to_owned()),
            damage,
            ..Default::default()
        };
        let left = HistoryRecord {
            summary: CombatSessionSummary {
                skills: vec![
                    stable_skill("GA_Test_First", 100.0),
                    stable_skill("GA_Test_Second", 200.0),
                ],
                ..Default::default()
            },
            ..Default::default()
        };
        let right = HistoryRecord {
            summary: CombatSessionSummary {
                skills: vec![
                    stable_skill("GA_Test_First", 125.0),
                    stable_skill("GA_Test_Second", 250.0),
                ],
                ..Default::default()
            },
            ..Default::default()
        };

        let comparison = compare_records(&left, &right);

        assert_eq!(comparison.skill_deltas.len(), 2);
        let mut deltas = comparison
            .skill_deltas
            .iter()
            .map(|delta| delta.delta_damage)
            .collect::<Vec<_>>();
        deltas.sort_by(f64::total_cmp);
        assert_eq!(deltas, vec![25.0, 50.0]);
    }

    #[test]
    fn compare_records_preserves_gameplay_effect_identity_without_damage_name() {
        let left = HistoryRecord {
            summary: CombatSessionSummary {
                skills: vec![CombatSessionSkillSummary {
                    name: "GE_Test_Skill_Damage".to_owned(),
                    category: "E技能".to_owned(),
                    gameplay_effect_name: Some("GE_Test_Skill_Damage".to_owned()),
                    damage: 100.0,
                    ..Default::default()
                }],
                ..Default::default()
            },
            ..Default::default()
        };

        let comparison = compare_records(&left, &HistoryRecord::default());

        assert_eq!(comparison.skill_deltas.len(), 1);
        let delta = &comparison.skill_deltas[0];
        assert_eq!(delta.name, "GE_Test_Skill_Damage");
        assert_eq!(
            delta.gameplay_effect_name.as_deref(),
            Some("GE_Test_Skill_Damage")
        );
    }

    #[test]
    fn compare_records_keeps_semantic_effects_under_one_ability_distinct() {
        let semantic_skill =
            |effect_name: &str, damage_name: &str, damage: f64| CombatSessionSkillSummary {
                name: damage_name.to_owned(),
                category: "Passive Damage".to_owned(),
                ability_name: Some("GA_Test_Passive".to_owned()),
                gameplay_effect_name: Some(effect_name.to_owned()),
                damage_name: Some(damage_name.to_owned()),
                damage,
                ..Default::default()
            };
        let left = HistoryRecord {
            summary: CombatSessionSummary {
                skills: vec![
                    semantic_skill("GE_Test_Passive_First", "First Component", 100.0),
                    semantic_skill("GE_Test_Passive_Second", "Second Component", 200.0),
                ],
                ..Default::default()
            },
            ..Default::default()
        };

        let comparison = compare_records(&left, &HistoryRecord::default());

        assert_eq!(comparison.skill_deltas.len(), 2);
        assert!(comparison.skill_deltas.iter().any(|row| {
            row.gameplay_effect_name.as_deref() == Some("GE_Test_Passive_First")
                && row.left_damage == 100.0
        }));
        assert!(comparison.skill_deltas.iter().any(|row| {
            row.gameplay_effect_name.as_deref() == Some("GE_Test_Passive_Second")
                && row.left_damage == 200.0
        }));
    }

    #[test]
    fn compare_records_matches_one_ability_across_effect_variants() {
        let skill = |effect_name: Option<&str>, damage: f64| CombatSessionSkillSummary {
            name: "GA_Test_UltraSkill".to_owned(),
            category: "Q技能".to_owned(),
            ability_name: Some("GA_Test_UltraSkill".to_owned()),
            gameplay_effect_name: effect_name.map(str::to_owned),
            damage_name: Some("Test Ultimate".to_owned()),
            damage,
            ..Default::default()
        };
        let left = HistoryRecord {
            summary: CombatSessionSummary {
                skills: vec![skill(Some("GE_Test_UltraSkill1_Damage"), 100.0)],
                ..Default::default()
            },
            ..Default::default()
        };
        let right = HistoryRecord {
            summary: CombatSessionSummary {
                skills: vec![skill(Some("GE_Test_UltraSkill2_Damage"), 125.0)],
                ..Default::default()
            },
            ..Default::default()
        };

        let comparison = compare_records(&left, &right);

        assert_eq!(comparison.skill_deltas.len(), 1);
        assert_eq!(comparison.skill_deltas[0].left_damage, 100.0);
        assert_eq!(comparison.skill_deltas[0].right_damage, 125.0);
        assert_eq!(comparison.skill_deltas[0].delta_damage, 25.0);
    }

    #[test]
    fn compare_records_matches_aggregated_and_single_effect_ability_rows() {
        let skill = |effect_name: Option<&str>, damage: f64| CombatSessionSkillSummary {
            name: "GA_Test_UltraSkill".to_owned(),
            category: "Q技能".to_owned(),
            ability_name: Some("GA_Test_UltraSkill".to_owned()),
            gameplay_effect_name: effect_name.map(str::to_owned),
            damage_name: Some("Test Ultimate".to_owned()),
            damage,
            ..Default::default()
        };
        let left = HistoryRecord {
            summary: CombatSessionSummary {
                skills: vec![skill(None, 175.0)],
                ..Default::default()
            },
            ..Default::default()
        };
        let right = HistoryRecord {
            summary: CombatSessionSummary {
                skills: vec![skill(Some("GE_Test_UltraSkill1_Damage"), 200.0)],
                ..Default::default()
            },
            ..Default::default()
        };

        let comparison = compare_records(&left, &right);

        assert_eq!(comparison.skill_deltas.len(), 1);
        assert_eq!(comparison.skill_deltas[0].delta_damage, 25.0);
    }

    fn temp_history_dir(name: &str) -> PathBuf {
        std::env::temp_dir().join(format!(
            "nte_history_test_{}_{}_{}",
            name,
            std::process::id(),
            HISTORY_ID_COUNTER.fetch_add(1, Ordering::Relaxed)
        ))
    }

    fn character(char_id: u32, name: &str, damage: f64, dps: f64) -> CombatSessionCharacterSummary {
        CombatSessionCharacterSummary {
            char_id,
            name: name.to_owned(),
            damage,
            dps,
            ..Default::default()
        }
    }

    fn skill(name: &str, category: &str, damage: f64) -> CombatSessionSkillSummary {
        CombatSessionSkillSummary {
            name: name.to_owned(),
            category: category.to_owned(),
            damage,
            ..Default::default()
        }
    }

    fn history_hit(timestamp: f64, char_id: u32, damage: f64) -> Hit {
        Hit {
            timestamp,
            char_id,
            char_name: format!("角色{char_id}"),
            char_known: true,
            damage,
            byte_offset: 0,
            bit_shift: 0,
            char_source: HitCharacterSource::Packet,
            direction: HitDirection::Outgoing,
            target_hp_before: 1_000.0,
            target_hp_after: 1_000.0 - damage,
            target_max_hp: 1_000.0,
            target_hp_percent: (1_000.0 - damage) / 10.0,
            target_id: None,
            target_name: None,
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
}
