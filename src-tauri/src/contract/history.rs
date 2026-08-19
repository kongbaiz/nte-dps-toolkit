use std::collections::HashMap;

use serde::Serialize;

use nte_dps_tool::{
    engine::model::{
        AbyssHalf, CaptureQualitySource, CharacterInfo, CombatSessionAbyssHalfSummary,
        CombatSessionCharacterSummary, CombatSessionSkillSummary, CombatSessionSummary,
    },
    storage::{
        ability_names,
        history::{
            HistoryCharacterDelta, HistoryComparison, HistoryLoadResult, HistoryRecord,
            HistorySkillDelta, MAX_HISTORY_IMPORT_BYTES, MAX_HISTORY_RECORDS,
        },
        i18n::{self, Language},
    },
};

pub(crate) const HISTORY_CONTRACT_VERSION: u32 = 2;
const DISPLAY_ROW_LIMIT: usize = 6;

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct HistorySnapshot {
    pub contract_version: u32,
    pub revision: String,
    pub max_import_bytes: String,
    pub skipped_files: u32,
    pub records: Vec<HistoryRecordSnapshot>,
}

impl HistorySnapshot {
    pub(crate) fn from_load(value: HistoryLoadResult, revision: u64) -> Self {
        let value = bounded_history_load(value);
        Self {
            contract_version: HISTORY_CONTRACT_VERSION,
            revision: revision.to_string(),
            max_import_bytes: MAX_HISTORY_IMPORT_BYTES.to_string(),
            skipped_files: value.skipped_files.min(u32::MAX as usize) as u32,
            records: value
                .records
                .iter()
                .take(MAX_HISTORY_RECORDS)
                .map(HistoryRecordSnapshot::from)
                .collect(),
        }
    }

    pub(crate) fn from_localized_load(
        value: HistoryLoadResult,
        revision: u64,
        characters: &HashMap<u32, CharacterInfo>,
    ) -> Self {
        Self::from_localized_load_for_language(
            value,
            revision,
            characters,
            i18n::current_language(),
        )
    }

    pub(crate) fn from_localized_load_for_language(
        value: HistoryLoadResult,
        revision: u64,
        characters: &HashMap<u32, CharacterInfo>,
        language: Language,
    ) -> Self {
        let mut value = bounded_history_load(value);
        for record in &mut value.records {
            localize_summary(&mut record.summary, characters, language);
        }
        Self::from_load(value, revision)
    }
}

fn bounded_history_load(mut value: HistoryLoadResult) -> HistoryLoadResult {
    let overflow = value.records.len().saturating_sub(MAX_HISTORY_RECORDS);
    value.records.truncate(MAX_HISTORY_RECORDS);
    value.skipped_files = value.skipped_files.saturating_add(overflow);
    value
}

#[derive(Clone, Debug, Serialize)]
#[serde(tag = "event", content = "payload", rename_all = "camelCase")]
pub(crate) enum HistoryEvent {
    Snapshot(HistorySnapshot),
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct HistoryDeleteSnapshot {
    pub history: HistorySnapshot,
    pub undo_token: String,
    pub undo_expires_ms: u32,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct HistoryImportFileSnapshot {
    pub performed: bool,
    pub imported_record_id: Option<String>,
    pub history: HistorySnapshot,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct HistoryFileActionSnapshot {
    pub performed: bool,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct HistoryRecordSnapshot {
    pub id: String,
    pub display_time: String,
    pub recorded_at: String,
    pub has_details: bool,
    pub party_label: String,
    pub can_set_upper_prediction: bool,
    pub can_set_lower_prediction: bool,
    pub summary: HistorySummarySnapshot,
}

impl From<&HistoryRecord> for HistoryRecordSnapshot {
    fn from(record: &HistoryRecord) -> Self {
        Self {
            id: record.id.clone(),
            display_time: record.display_time(),
            recorded_at: record.effective_timestamp().to_rfc3339(),
            has_details: record.details.is_some(),
            party_label: history_party_label(record),
            can_set_upper_prediction: record.upper_team_dps().is_some(),
            can_set_lower_prediction: record.lower_team_dps().is_some(),
            summary: HistorySummarySnapshot::from(&record.summary),
        }
    }
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct HistorySummarySnapshot {
    pub duration_seconds: f64,
    pub dps_time_basis: &'static str,
    pub total_damage: f64,
    pub total_dps: f64,
    pub total_damage_taken: f64,
    pub total_hits: String,
    pub reaction_damage_separated: bool,
    pub characters: Vec<HistoryCharacterSnapshot>,
    pub skills: Vec<HistorySkillSnapshot>,
    pub abyss: HistoryAbyssSnapshot,
    pub quality: HistoryQualitySnapshot,
    pub hidden_character_count: u32,
    pub hidden_skill_count: u32,
}

impl From<&CombatSessionSummary> for HistorySummarySnapshot {
    fn from(summary: &CombatSessionSummary) -> Self {
        Self {
            duration_seconds: summary.duration_seconds,
            dps_time_basis: summary.dps_time_mode.protocol_code(),
            total_damage: summary.total_damage,
            total_dps: summary.total_dps,
            total_damage_taken: summary.total_damage_taken,
            total_hits: summary.total_hits.to_string(),
            reaction_damage_separated: summary.reaction_damage_separated,
            characters: summary
                .characters
                .iter()
                .take(DISPLAY_ROW_LIMIT)
                .map(HistoryCharacterSnapshot::from)
                .collect(),
            skills: summary
                .skills
                .iter()
                .take(DISPLAY_ROW_LIMIT)
                .map(HistorySkillSnapshot::from)
                .collect(),
            abyss: HistoryAbyssSnapshot {
                detected: summary.abyss.detected,
                floor: summary.abyss.floor,
                active_half: summary.abyss.active_half.map(abyss_half_code),
                success: summary.abyss.success,
                first_half: summary
                    .abyss
                    .first_half
                    .as_ref()
                    .map(HistoryAbyssHalfSnapshot::from),
                second_half: summary
                    .abyss
                    .second_half
                    .as_ref()
                    .map(HistoryAbyssHalfSnapshot::from),
            },
            quality: HistoryQualitySnapshot {
                source: capture_source_code(summary.quality.source),
                packet_count: summary.quality.packet_count.to_string(),
                hit_count: summary.quality.hit_count.to_string(),
                unmapped_skill_hits: summary.quality.unmapped_skill_hits.to_string(),
                unknown_character_hits: summary.quality.unknown_character_hits.to_string(),
            },
            hidden_character_count: hidden_count(summary.characters.len()),
            hidden_skill_count: hidden_count(summary.skills.len()),
        }
    }
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct HistoryCharacterSnapshot {
    pub char_id: u32,
    pub name: String,
    pub hits: String,
    pub damage: f64,
    pub dps: f64,
    pub damage_share_percent: f64,
    pub hits_taken: String,
    pub damage_taken: f64,
}

impl From<&CombatSessionCharacterSummary> for HistoryCharacterSnapshot {
    fn from(row: &CombatSessionCharacterSummary) -> Self {
        Self {
            char_id: row.char_id,
            name: row.name.clone(),
            hits: row.hits.to_string(),
            damage: row.damage,
            dps: row.dps,
            damage_share_percent: row.damage_share_percent,
            hits_taken: row.hits_taken.to_string(),
            damage_taken: row.damage_taken,
        }
    }
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct HistorySkillSnapshot {
    pub char_id: u32,
    pub char_name: String,
    pub name: String,
    pub category: String,
    pub hits: String,
    pub damage: f64,
    pub damage_share_percent: f64,
    pub is_follow_up: bool,
}

impl From<&CombatSessionSkillSummary> for HistorySkillSnapshot {
    fn from(row: &CombatSessionSkillSummary) -> Self {
        Self {
            char_id: row.char_id,
            char_name: row.char_name.clone(),
            name: row.name.clone(),
            category: row.category.clone(),
            hits: row.hits.to_string(),
            damage: row.damage,
            damage_share_percent: row.damage_share_percent,
            is_follow_up: row.is_follow_up,
        }
    }
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct HistoryAbyssSnapshot {
    pub detected: bool,
    pub floor: Option<u32>,
    pub active_half: Option<&'static str>,
    pub success: bool,
    pub first_half: Option<HistoryAbyssHalfSnapshot>,
    pub second_half: Option<HistoryAbyssHalfSnapshot>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct HistoryAbyssHalfSnapshot {
    pub half: &'static str,
    pub duration_seconds: f64,
    pub total_damage: f64,
    pub total_dps: f64,
    pub characters: Vec<HistoryCharacterSnapshot>,
    pub skills: Vec<HistorySkillSnapshot>,
    pub hidden_character_count: u32,
    pub hidden_skill_count: u32,
}

impl From<&CombatSessionAbyssHalfSummary> for HistoryAbyssHalfSnapshot {
    fn from(summary: &CombatSessionAbyssHalfSummary) -> Self {
        Self {
            half: abyss_half_code(summary.half),
            duration_seconds: summary.duration_seconds,
            total_damage: summary.total_damage,
            total_dps: summary.total_dps,
            characters: summary
                .characters
                .iter()
                .take(DISPLAY_ROW_LIMIT)
                .map(HistoryCharacterSnapshot::from)
                .collect(),
            skills: summary
                .skills
                .iter()
                .take(DISPLAY_ROW_LIMIT)
                .map(HistorySkillSnapshot::from)
                .collect(),
            hidden_character_count: hidden_count(summary.characters.len()),
            hidden_skill_count: hidden_count(summary.skills.len()),
        }
    }
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct HistoryQualitySnapshot {
    pub source: &'static str,
    pub packet_count: String,
    pub hit_count: String,
    pub unmapped_skill_hits: String,
    pub unknown_character_hits: String,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct HistoryComparisonSnapshot {
    pub left_id: String,
    pub right_id: String,
    pub total_dps_delta: f64,
    pub total_damage_delta: f64,
    pub duration_delta: f64,
    pub different_time_basis: bool,
    pub different_reaction_accounting: bool,
    pub character_deltas: Vec<HistoryCharacterDeltaSnapshot>,
    pub skill_deltas: Vec<HistorySkillDeltaSnapshot>,
}

impl HistoryComparisonSnapshot {
    pub(crate) fn from_records(
        comparison: HistoryComparison,
        left: &HistoryRecord,
        right: &HistoryRecord,
        characters: &HashMap<u32, CharacterInfo>,
    ) -> Self {
        Self {
            left_id: comparison.left_id,
            right_id: comparison.right_id,
            total_dps_delta: comparison.total_dps_delta,
            total_damage_delta: comparison.total_damage_delta,
            duration_delta: comparison.duration_delta,
            different_time_basis: left.summary.dps_time_mode != right.summary.dps_time_mode,
            different_reaction_accounting: left.summary.reaction_damage_separated
                != right.summary.reaction_damage_separated,
            character_deltas: comparison
                .character_deltas
                .iter()
                .map(|row| HistoryCharacterDeltaSnapshot::localized(row, characters))
                .collect(),
            skill_deltas: comparison
                .skill_deltas
                .iter()
                .map(HistorySkillDeltaSnapshot::localized)
                .collect(),
        }
    }
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct HistoryCharacterDeltaSnapshot {
    pub char_id: u32,
    pub name: String,
    pub left_dps: f64,
    pub right_dps: f64,
    pub delta_dps: f64,
    pub left_damage: f64,
    pub right_damage: f64,
    pub delta_damage: f64,
}

impl From<&HistoryCharacterDelta> for HistoryCharacterDeltaSnapshot {
    fn from(row: &HistoryCharacterDelta) -> Self {
        Self {
            char_id: row.char_id,
            name: row.name.clone(),
            left_dps: row.left_dps,
            right_dps: row.right_dps,
            delta_dps: row.delta_dps,
            left_damage: row.left_damage,
            right_damage: row.right_damage,
            delta_damage: row.delta_damage,
        }
    }
}

impl HistoryCharacterDeltaSnapshot {
    fn localized(row: &HistoryCharacterDelta, characters: &HashMap<u32, CharacterInfo>) -> Self {
        let mut snapshot = Self::from(row);
        snapshot.name =
            localized_character_name(characters, row.char_id, &row.name, i18n::current_language());
        snapshot
    }
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct HistorySkillDeltaSnapshot {
    pub name: String,
    pub category: String,
    pub left_damage: f64,
    pub right_damage: f64,
    pub delta_damage: f64,
}

impl From<&HistorySkillDelta> for HistorySkillDeltaSnapshot {
    fn from(row: &HistorySkillDelta) -> Self {
        Self {
            name: row.name.clone(),
            category: row.category.clone(),
            left_damage: row.left_damage,
            right_damage: row.right_damage,
            delta_damage: row.delta_damage,
        }
    }
}

impl HistorySkillDeltaSnapshot {
    fn localized(row: &HistorySkillDelta) -> Self {
        let mut snapshot = Self::from(row);
        snapshot.name = localized_skill_name(
            &row.name,
            row.ability_name.as_deref(),
            row.gameplay_effect_name.as_deref(),
            None,
        );
        snapshot
    }
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct HistoryExportSnapshot {
    pub file_name: String,
    pub json: String,
}

fn abyss_half_code(half: AbyssHalf) -> &'static str {
    match half {
        AbyssHalf::First => "first",
        AbyssHalf::Second => "second",
    }
}

fn capture_source_code(source: CaptureQualitySource) -> &'static str {
    match source {
        CaptureQualitySource::Live => "live",
        CaptureQualitySource::PcapngReplay => "pcapng_replay",
        CaptureQualitySource::JsonReplay => "json_replay",
        CaptureQualitySource::Unknown => "unknown",
    }
}

fn hidden_count(len: usize) -> u32 {
    len.saturating_sub(DISPLAY_ROW_LIMIT).min(u32::MAX as usize) as u32
}

fn localize_summary(
    summary: &mut CombatSessionSummary,
    characters: &HashMap<u32, CharacterInfo>,
    language: Language,
) {
    for row in &mut summary.characters {
        row.name = localized_character_name(characters, row.char_id, &row.name, language);
    }
    for row in &mut summary.skills {
        row.char_name = localized_character_name(characters, row.char_id, &row.char_name, language);
        row.name = localized_skill_name(
            &row.name,
            row.ability_name.as_deref(),
            row.gameplay_effect_name.as_deref(),
            row.damage_name.as_deref(),
        );
    }
    for half in [
        summary.abyss.first_half.as_mut(),
        summary.abyss.second_half.as_mut(),
    ]
    .into_iter()
    .flatten()
    {
        for row in &mut half.characters {
            row.name = localized_character_name(characters, row.char_id, &row.name, language);
        }
        for row in &mut half.skills {
            row.char_name =
                localized_character_name(characters, row.char_id, &row.char_name, language);
            row.name = localized_skill_name(
                &row.name,
                row.ability_name.as_deref(),
                row.gameplay_effect_name.as_deref(),
                row.damage_name.as_deref(),
            );
        }
    }
}

fn localized_character_name(
    characters: &HashMap<u32, CharacterInfo>,
    char_id: u32,
    fallback: &str,
    language: Language,
) -> String {
    let Some(info) = characters.get(&char_id) else {
        return fallback.to_owned();
    };
    let candidate = if language == Language::SimplifiedChinese {
        info.name_zh.trim()
    } else {
        info.name_en.trim()
    };
    if !candidate.is_empty() {
        candidate.to_owned()
    } else {
        fallback.to_owned()
    }
}

fn localized_skill_name(
    fallback: &str,
    ability_name: Option<&str>,
    gameplay_effect_name: Option<&str>,
    damage_name: Option<&str>,
) -> String {
    let ability_name = ability_name.or_else(|| fallback.starts_with("GA_").then_some(fallback));
    let gameplay_effect_name =
        gameplay_effect_name.or_else(|| fallback.starts_with("GE_").then_some(fallback));
    gameplay_effect_name
        .and_then(ability_names::resolve_damage_name)
        .or_else(|| ability_name.and_then(ability_names::resolve_ability_name))
        .or_else(|| damage_name.map(str::to_owned))
        .unwrap_or_else(|| fallback.to_owned())
}

fn history_party_label(record: &HistoryRecord) -> String {
    let mut names = Vec::new();
    for row in &record.summary.characters {
        if !names.contains(&row.name) {
            names.push(row.name.clone());
        }
        if names.len() == 4 {
            break;
        }
    }
    names.join(" / ")
}

#[cfg(test)]
mod tests {
    use super::*;
    use nte_dps_tool::engine::model::DpsTimeBasis;

    #[test]
    fn history_snapshot_never_projects_more_than_the_rust_record_limit() {
        let records = (0..=nte_dps_tool::storage::history::MAX_HISTORY_RECORDS)
            .map(|index| HistoryRecord {
                id: format!("record-{index}"),
                ..Default::default()
            })
            .collect();

        let snapshot = HistorySnapshot::from_load(
            HistoryLoadResult {
                records,
                skipped_files: 2,
            },
            9,
        );

        assert_eq!(
            snapshot.records.len(),
            nte_dps_tool::storage::history::MAX_HISTORY_RECORDS
        );
        assert_eq!(snapshot.skipped_files, 3);
    }

    #[test]
    fn record_projection_uses_stable_codes_and_string_counters() {
        let record = HistoryRecord {
            id: "record-1".to_owned(),
            summary: CombatSessionSummary {
                total_hits: u64::MAX,
                dps_time_mode: DpsTimeBasis::WallClock,
                ..Default::default()
            },
            ..Default::default()
        };

        let value = serde_json::to_value(HistoryRecordSnapshot::from(&record))
            .expect("history record must serialize");

        assert_eq!(value["id"], "record-1");
        assert_eq!(value["summary"]["totalHits"], u64::MAX.to_string());
        assert_eq!(value["summary"]["dpsTimeBasis"], "wall_clock");
        assert!(value.get("details").is_none());
    }

    #[test]
    fn native_file_action_projection_does_not_expose_local_paths() {
        let value = serde_json::to_value(HistoryImportFileSnapshot {
            performed: true,
            imported_record_id: Some("record-1".to_owned()),
            history: HistorySnapshot {
                contract_version: HISTORY_CONTRACT_VERSION,
                revision: "1".to_owned(),
                max_import_bytes: MAX_HISTORY_IMPORT_BYTES.to_string(),
                skipped_files: 0,
                records: Vec::new(),
            },
        })
        .expect("history import result must serialize");

        assert_eq!(value["performed"], true);
        assert_eq!(value["importedRecordId"], "record-1");
        assert!(value.get("path").is_none());
    }
}
