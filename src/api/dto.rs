use serde::Serialize;

use crate::core::capture::CaptureEnvironment;
use crate::core::snapshot::{
    InventoryItem, InventorySnapshot, InventoryStat, ItemUid, LocalizedNames,
};
use crate::engine::capture::CaptureDevice;
use crate::engine::model::{
    CaptureQualitySource, CaptureQualitySummary, CombatSessionAbyssHalfSummary,
    CombatSessionAbyssSummary, CombatSessionCharacterSummary, CombatSessionSkillSummary,
    CombatSessionSummary,
};

#[derive(Debug, Serialize)]
pub struct DeviceDto {
    pub name: String,
    pub description: String,
    pub ipv4: Vec<String>,
}

impl From<&CaptureDevice> for DeviceDto {
    fn from(device: &CaptureDevice) -> Self {
        Self {
            name: device.name.clone(),
            description: device.description.clone(),
            ipv4: device.ipv4.iter().map(ToString::to_string).collect(),
        }
    }
}

#[derive(Debug, Serialize)]
pub struct DevicesResult {
    pub devices: Vec<DeviceDto>,
}

impl DevicesResult {
    pub fn new(devices: &[CaptureDevice]) -> Self {
        Self {
            devices: devices.iter().map(DeviceDto::from).collect(),
        }
    }
}

#[derive(Debug, Serialize)]
pub struct CaptureDetectResult {
    pub game_process_detected: bool,
    pub recommended_device: Option<String>,
    pub local_ip_detected: bool,
    pub devices: Vec<DeviceDto>,
}

impl From<CaptureEnvironment> for CaptureDetectResult {
    fn from(environment: CaptureEnvironment) -> Self {
        Self {
            game_process_detected: environment.game_process_detected,
            recommended_device: environment.recommended_device,
            local_ip_detected: environment.local_ip_detected,
            devices: environment.devices.iter().map(DeviceDto::from).collect(),
        }
    }
}

#[derive(Clone, Debug, Serialize)]
pub struct LocalizedNamesDto {
    pub zh_cn: String,
    pub en: String,
    pub ja: String,
}

impl From<&LocalizedNames> for LocalizedNamesDto {
    fn from(names: &LocalizedNames) -> Self {
        Self {
            zh_cn: names.zh_cn.clone(),
            en: names.en.clone(),
            ja: names.ja.clone(),
        }
    }
}

#[derive(Clone, Copy, Debug, Serialize)]
pub struct ItemUidDto {
    pub slot: u32,
    pub serial: u32,
}

impl From<ItemUid> for ItemUidDto {
    fn from(uid: ItemUid) -> Self {
        Self {
            slot: uid.slot,
            serial: uid.serial,
        }
    }
}

#[derive(Clone, Debug, Serialize)]
pub struct InventoryStatDto {
    pub property_id: String,
    pub value: f32,
    pub percent: Option<bool>,
    pub names: Option<LocalizedNamesDto>,
}

impl From<&InventoryStat> for InventoryStatDto {
    fn from(stat: &InventoryStat) -> Self {
        Self {
            property_id: stat.property_id.clone(),
            value: stat.value,
            percent: stat.percent,
            names: stat.names.as_ref().map(LocalizedNamesDto::from),
        }
    }
}

#[derive(Clone, Debug, Serialize)]
pub struct InventoryItemDto {
    pub uid: ItemUidDto,
    pub item_id: String,
    pub kind: Option<&'static str>,
    pub quality: Option<String>,
    pub geometry: Option<String>,
    pub grid: Option<u32>,
    pub suit_id: Option<String>,
    pub names: Option<LocalizedNamesDto>,
    pub suit_names: Option<LocalizedNamesDto>,
    pub level: u32,
    pub max_level: Option<u32>,
    pub locked: bool,
    pub equipped: bool,
    pub equipped_character_uid: Option<ItemUidDto>,
    pub equipped_character_id: Option<u32>,
    pub main_stats: Vec<InventoryStatDto>,
    pub sub_stats: Vec<InventoryStatDto>,
}

impl From<&InventoryItem> for InventoryItemDto {
    fn from(item: &InventoryItem) -> Self {
        Self {
            uid: item.uid.into(),
            item_id: item.item_id.clone(),
            kind: item.kind,
            quality: item.quality.clone(),
            geometry: item.geometry.clone(),
            grid: item.grid,
            suit_id: item.suit_id.clone(),
            names: item.names.as_ref().map(LocalizedNamesDto::from),
            suit_names: item.suit_names.as_ref().map(LocalizedNamesDto::from),
            level: item.level,
            max_level: item.max_level,
            locked: item.locked,
            equipped: item.equipped,
            equipped_character_uid: item.equipped_character_uid.map(ItemUidDto::from),
            equipped_character_id: item.equipped_character_id,
            main_stats: item.main_stats.iter().map(InventoryStatDto::from).collect(),
            sub_stats: item.sub_stats.iter().map(InventoryStatDto::from).collect(),
        }
    }
}

#[derive(Clone, Debug, Serialize)]
pub struct InventorySnapshotDto {
    pub generation: u64,
    pub observed_at_unix_ms: u64,
    pub complete: bool,
    pub item_count: usize,
    pub items: Vec<InventoryItemDto>,
}

impl From<&InventorySnapshot> for InventorySnapshotDto {
    fn from(snapshot: &InventorySnapshot) -> Self {
        Self {
            generation: snapshot.generation,
            observed_at_unix_ms: snapshot.observed_at_unix_ms,
            complete: snapshot.complete,
            item_count: snapshot.item_count(),
            items: snapshot.items.iter().map(InventoryItemDto::from).collect(),
        }
    }
}

#[derive(Clone, Debug, Serialize)]
pub struct InventorySnapshotEvent {
    pub sequence: u64,
    #[serde(flatten)]
    pub snapshot: InventorySnapshotDto,
}

#[derive(Clone, Debug, Serialize)]
pub struct BattleCharacterDto {
    pub char_id: u32,
    pub name: String,
    pub hits: u64,
    pub damage: f64,
    pub dps: f64,
    pub damage_share_percent: f64,
    pub hits_taken: u64,
    pub damage_taken: f64,
}

impl From<&CombatSessionCharacterSummary> for BattleCharacterDto {
    fn from(summary: &CombatSessionCharacterSummary) -> Self {
        Self {
            char_id: summary.char_id,
            name: summary.name.clone(),
            hits: summary.hits,
            damage: summary.damage,
            dps: summary.dps,
            damage_share_percent: summary.damage_share_percent,
            hits_taken: summary.hits_taken,
            damage_taken: summary.damage_taken,
        }
    }
}

#[derive(Clone, Debug, Serialize)]
pub struct BattleSkillDto {
    pub char_id: u32,
    pub char_name: String,
    pub name: String,
    pub category: String,
    pub hits: u64,
    pub damage: f64,
    pub damage_share_percent: f64,
    pub is_follow_up: bool,
}

impl From<&CombatSessionSkillSummary> for BattleSkillDto {
    fn from(summary: &CombatSessionSkillSummary) -> Self {
        Self {
            char_id: summary.char_id,
            char_name: summary.char_name.clone(),
            name: summary.name.clone(),
            category: summary.category.clone(),
            hits: summary.hits,
            damage: summary.damage,
            damage_share_percent: summary.damage_share_percent,
            is_follow_up: summary.is_follow_up,
        }
    }
}

#[derive(Clone, Debug, Serialize)]
pub struct BattleAbyssHalfDto {
    pub half: String,
    pub duration_seconds: f64,
    pub total_damage: f64,
    pub total_dps: f64,
    pub characters: Vec<BattleCharacterDto>,
    pub skills: Vec<BattleSkillDto>,
}

impl From<&CombatSessionAbyssHalfSummary> for BattleAbyssHalfDto {
    fn from(summary: &CombatSessionAbyssHalfSummary) -> Self {
        Self {
            half: summary.half.clone(),
            duration_seconds: summary.duration_seconds,
            total_damage: summary.total_damage,
            total_dps: summary.total_dps,
            characters: summary
                .characters
                .iter()
                .map(BattleCharacterDto::from)
                .collect(),
            skills: summary.skills.iter().map(BattleSkillDto::from).collect(),
        }
    }
}

#[derive(Clone, Debug, Serialize)]
pub struct BattleAbyssDto {
    pub detected: bool,
    pub floor: Option<u32>,
    pub active_half: Option<String>,
    pub success: bool,
    pub first_half: Option<BattleAbyssHalfDto>,
    pub second_half: Option<BattleAbyssHalfDto>,
}

impl From<&CombatSessionAbyssSummary> for BattleAbyssDto {
    fn from(summary: &CombatSessionAbyssSummary) -> Self {
        Self {
            detected: summary.detected,
            floor: summary.floor,
            active_half: summary.active_half.clone(),
            success: summary.success,
            first_half: summary.first_half.as_ref().map(BattleAbyssHalfDto::from),
            second_half: summary.second_half.as_ref().map(BattleAbyssHalfDto::from),
        }
    }
}

#[derive(Clone, Debug, Serialize)]
pub struct BattleQualityDto {
    pub source: &'static str,
    pub packet_count: usize,
    pub packets_with_hits: usize,
    pub hit_count: usize,
    pub outgoing_hits: u64,
    pub outgoing_damage: f64,
    pub unknown_direction_hits: u64,
    pub unknown_direction_damage: f64,
    pub incoming_hits: u64,
    pub incoming_damage: f64,
    pub unknown_character_count: usize,
    pub unknown_character_hits: u64,
    pub unmapped_skill_rows: usize,
    pub unmapped_skill_hits: u64,
    pub unmapped_gameplay_effect_count: usize,
    pub time_stop_event_count: u64,
    pub time_stop_interval_count: usize,
    pub abyss_event_count: u64,
    pub server_damage_corrections: u64,
}

impl From<&CaptureQualitySummary> for BattleQualityDto {
    fn from(summary: &CaptureQualitySummary) -> Self {
        Self {
            source: match summary.source {
                CaptureQualitySource::Live => "live",
                CaptureQualitySource::PcapngReplay => "pcapng_replay",
                CaptureQualitySource::JsonReplay => "json_replay",
                CaptureQualitySource::Unknown => "unknown",
            },
            packet_count: summary.packet_count,
            packets_with_hits: summary.packets_with_hits,
            hit_count: summary.hit_count,
            outgoing_hits: summary.outgoing_hits,
            outgoing_damage: summary.outgoing_damage,
            unknown_direction_hits: summary.unknown_direction_hits,
            unknown_direction_damage: summary.unknown_direction_damage,
            incoming_hits: summary.incoming_hits,
            incoming_damage: summary.incoming_damage,
            unknown_character_count: summary.unknown_character_count,
            unknown_character_hits: summary.unknown_character_hits,
            unmapped_skill_rows: summary.unmapped_skill_rows,
            unmapped_skill_hits: summary.unmapped_skill_hits,
            unmapped_gameplay_effect_count: summary.unmapped_gameplay_effect_count,
            time_stop_event_count: summary.time_stop_event_count,
            time_stop_interval_count: summary.time_stop_interval_count,
            abyss_event_count: summary.abyss_event_count,
            server_damage_corrections: summary.server_damage_corrections,
        }
    }
}

#[derive(Clone, Debug, Serialize)]
pub struct BattleSummaryDto {
    pub duration_seconds: f64,
    pub dps_time_mode: String,
    pub total_damage: f64,
    pub total_dps: f64,
    pub total_damage_taken: f64,
    pub total_hits: u64,
    pub characters: Vec<BattleCharacterDto>,
    pub skills: Vec<BattleSkillDto>,
    pub abyss: BattleAbyssDto,
    pub quality: BattleQualityDto,
}

impl From<&CombatSessionSummary> for BattleSummaryDto {
    fn from(summary: &CombatSessionSummary) -> Self {
        Self {
            duration_seconds: summary.duration_seconds,
            dps_time_mode: summary.dps_time_mode.clone(),
            total_damage: summary.total_damage,
            total_dps: summary.total_dps,
            total_damage_taken: summary.total_damage_taken,
            total_hits: summary.total_hits,
            characters: summary
                .characters
                .iter()
                .map(BattleCharacterDto::from)
                .collect(),
            skills: summary.skills.iter().map(BattleSkillDto::from).collect(),
            abyss: BattleAbyssDto::from(&summary.abyss),
            quality: BattleQualityDto::from(&summary.quality),
        }
    }
}

#[derive(Clone, Debug, Serialize)]
pub struct BattleSummaryEvent {
    pub sequence: u64,
    #[serde(flatten)]
    pub summary: BattleSummaryDto,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn inventory_uid_serializes_slot_without_internal_typo() {
        let json = serde_json::to_value(ItemUidDto { slot: 1, serial: 2 }).unwrap();
        assert_eq!(json, serde_json::json!({"slot": 1, "serial": 2}));
        assert!(json.get("solt").is_none());
    }

    #[test]
    fn battle_summary_mapping_is_independent_and_uses_stable_source_codes() {
        let summary = CombatSessionSummary {
            duration_seconds: 10.0,
            dps_time_mode: "subtract_time_stop".to_owned(),
            total_damage: 1_000.0,
            total_dps: 100.0,
            total_damage_taken: 25.0,
            total_hits: 4,
            characters: vec![CombatSessionCharacterSummary {
                char_id: 7,
                name: "Character".to_owned(),
                hits: 4,
                damage: 1_000.0,
                dps: 100.0,
                damage_share_percent: 100.0,
                hits_taken: 1,
                damage_taken: 25.0,
            }],
            skills: vec![CombatSessionSkillSummary {
                char_id: 7,
                char_name: "Character".to_owned(),
                name: "Skill".to_owned(),
                category: "normal".to_owned(),
                hits: 4,
                damage: 1_000.0,
                damage_share_percent: 100.0,
                is_follow_up: false,
            }],
            abyss: CombatSessionAbyssSummary::default(),
            quality: CaptureQualitySummary {
                source: CaptureQualitySource::Live,
                packet_count: 8,
                hit_count: 4,
                ..CaptureQualitySummary::default()
            },
        };
        let dto = BattleSummaryDto::from(&summary);
        let json = serde_json::to_value(dto).unwrap();
        assert_eq!(json["dps_time_mode"], "subtract_time_stop");
        assert_eq!(json["characters"][0]["char_id"], 7);
        assert_eq!(json["skills"][0]["name"], "Skill");
        assert_eq!(json["quality"]["source"], "live");
        assert_eq!(json["quality"]["packet_count"], 8);
    }
}
