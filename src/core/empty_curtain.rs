//! Frontend-neutral Console equipment helpers shared by egui and Tauri.

use std::collections::HashSet;

use serde_json::Value;

use crate::{
    engine::{
        model::{EmptyCurtainCharacter, EmptyCurtainItem, EquipmentStat, HtItemNetId},
        parser::{EquipmentCatalog, EquipmentKind},
    },
    storage::i18n::Language,
};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RecommendedLoadout {
    pub character: EmptyCurtainCharacter,
    pub core: HtItemNetId,
    pub placements: Vec<RecommendedPlacement>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RecommendedPlacement {
    pub equipment: HtItemNetId,
    pub row: i32,
    pub column: i32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RecommendedLoadoutError {
    MissingTemplate,
    MissingEquipment,
}

/// Returns the level at which a zero-based secondary-stat slot becomes active.
/// Slots are distributed evenly across the equipment's validated level range.
pub fn substat_unlock_level(
    max_level: u32,
    substat_count: usize,
    substat_index: usize,
) -> Option<u32> {
    if max_level == 0 || substat_count == 0 || substat_index >= substat_count {
        return None;
    }
    let numerator = u64::from(max_level) * (substat_index as u64 + 1);
    let denominator = substat_count as u64;
    Some(numerator.div_ceil(denominator) as u32)
}

pub fn recommended_loadout(
    character: EmptyCurtainCharacter,
    items: &[EmptyCurtainItem],
    catalog: &EquipmentCatalog,
) -> Result<RecommendedLoadout, RecommendedLoadoutError> {
    let plan = catalog
        .plans
        .get(&character.character_id)
        .ok_or(RecommendedLoadoutError::MissingTemplate)?;
    let mut selected = HashSet::new();
    let mut placements = Vec::with_capacity(plan.recommended_modules.len());

    for planned in &plan.recommended_modules {
        let geometry = &catalog
            .items
            .get(&planned.item_id)
            .expect("validated equipment plan must reference a module")
            .geometry;
        let item = items
            .iter()
            .filter(|item| {
                !selected.contains(&item.id)
                    && (item.character_net_id.is_none()
                        || item.character_net_id == Some(character.net_id))
                    && catalog.items.get(&item.item_id).is_some_and(|definition| {
                        definition.kind == EquipmentKind::Module && definition.geometry == *geometry
                    })
            })
            .max_by_key(|item| {
                let definition = catalog
                    .items
                    .get(&item.item_id)
                    .expect("candidate equipment must have catalog metadata");
                (
                    item.item_id == planned.item_id,
                    quality_rank(&definition.quality),
                    item.level,
                    item.id.solt,
                    item.id.serial,
                )
            })
            .ok_or(RecommendedLoadoutError::MissingEquipment)?;
        selected.insert(item.id);
        placements.push(RecommendedPlacement {
            equipment: item.id,
            row: planned.row,
            column: planned.column,
        });
    }

    let recommended_core = catalog
        .items
        .get(&plan.recommended_core)
        .expect("validated equipment plan must reference a cassette");
    let core = items
        .iter()
        .filter(|item| {
            (item.character_net_id.is_none() || item.character_net_id == Some(character.net_id))
                && catalog
                    .items
                    .get(&item.item_id)
                    .is_some_and(|definition| definition.kind == EquipmentKind::Core)
        })
        .max_by_key(|item| {
            let definition = catalog
                .items
                .get(&item.item_id)
                .expect("candidate equipment must have catalog metadata");
            (
                item.item_id == plan.recommended_core,
                definition.suit == recommended_core.suit,
                quality_rank(&definition.quality),
                item.level,
                item.id.solt,
                item.id.serial,
            )
        })
        .ok_or(RecommendedLoadoutError::MissingEquipment)?;

    Ok(RecommendedLoadout {
        character,
        core: core.id,
        placements,
    })
}

pub fn build_drive_calculator_inventory(
    items: &[EmptyCurtainItem],
    catalog: &EquipmentCatalog,
) -> Vec<Value> {
    let mut records = Vec::with_capacity(items.len());
    for item in items {
        let Some(definition) = catalog.items.get(&item.item_id) else {
            continue;
        };
        let uid = format!("{}_{}", item.id.solt, item.id.serial);
        let quality = calculator_quality(&definition.quality);
        let sub_stats = calculator_stat_map(&item.sub_stats, catalog);
        let record = match definition.kind {
            EquipmentKind::Module => {
                let area = definition.grid.unwrap_or(1);
                serde_json::json!({
                    "uid": uid,
                    "item_type": "drive",
                    "quality": quality,
                    "area": area,
                    "shape_id": module_shape_id(&definition.geometry, area),
                    "set_name": "未知套装",
                    "main_stats": calculator_stat_map(&item.main_stats, catalog),
                    "sub_stats": sub_stats,
                })
            }
            EquipmentKind::Core => {
                let set_name = definition
                    .suit
                    .as_deref()
                    .and_then(|suit| catalog.suits.get(suit))
                    .map(|suit| calculator_set_name(&suit.name_zh))
                    .unwrap_or_else(|| "未知套装".to_owned());
                let main_stat = item
                    .main_stats
                    .first()
                    .and_then(|stat| calculator_stat_key(&stat.property, catalog))
                    .unwrap_or_else(|| "未知主词条".to_owned());
                serde_json::json!({
                    "uid": uid,
                    "item_type": "tape",
                    "quality": quality,
                    "area": 15,
                    "shape_id": "TAPE_15",
                    "set_name": set_name,
                    "main_stats": main_stat,
                    "sub_stats": sub_stats,
                })
            }
        };
        records.push(record);
    }
    records
}

fn quality_rank(quality: &str) -> u8 {
    match quality {
        "blue" => 0,
        "purple" => 1,
        "orange" => 2,
        _ => 0,
    }
}

fn calculator_quality(quality: &str) -> &'static str {
    match quality {
        "blue" => "Blue",
        "purple" => "Purple",
        _ => "Gold",
    }
}

fn module_shape_id(geometry: &str, area: u32) -> String {
    let mapped = match geometry {
        "Hen2" => "H_2",
        "Hen3" => "H_3",
        "Hen4" => "H_4",
        "Shu2" => "V_2",
        "Shu3" => "V_3",
        "Shu4" => "V_4",
        "Z3" => "Trap_4_H",
        "Z4" => "Trap_4_V",
        "ZhiJiao1" => "L_3_TL",
        "ZhiJiao2" => "L_3_TR",
        "ZhiJiao3" => "L_3_BL",
        "ZhiJiao4" => "L_3_BR",
        _ => match area {
            4 => "H_4",
            3 => "H_3",
            _ => "H_2",
        },
    };
    mapped.to_owned()
}

fn calculator_set_name(name_zh: &str) -> String {
    let stripped = name_zh.trim_start_matches('「').trim_end_matches('」');
    match stripped {
        "缇娅的夜间酒馆" => "缇娜的夜间酒馆".to_owned(),
        other => other.to_owned(),
    }
}

fn calculator_stat_map(
    stats: &[EquipmentStat],
    catalog: &EquipmentCatalog,
) -> serde_json::Map<String, Value> {
    let mut map = serde_json::Map::new();
    for stat in stats {
        let Some(key) = calculator_stat_key(&stat.property, catalog) else {
            continue;
        };
        let percent = catalog
            .attributes
            .get(&stat.property)
            .is_some_and(|attribute| attribute.percent);
        map.insert(
            key,
            serde_json::json!(calculator_stat_value(stat.value, percent)),
        );
    }
    map
}

fn calculator_stat_value(value: f32, percent: bool) -> f64 {
    let scaled = f64::from(value) * if percent { 100.0 } else { 1.0 };
    (scaled * 100.0).round() / 100.0
}

fn calculator_stat_key(property: &str, catalog: &EquipmentCatalog) -> Option<String> {
    let mapped = match property {
        "AtkBase" | "AtkAdd" => "攻击力",
        "AtkUp" => "攻击力%",
        "HPMaxBase" | "HPMaxAdd" => "生命值",
        "HPMaxUp" => "生命值%",
        "DefBase" | "DefAdd" => "防御力",
        "DefUp" => "防御力%",
        "CritBase" | "CritAdd" => "暴击率%",
        "CritDamageBase" | "CritDamageAdd" => "暴击伤害%",
        "DamageUpGeneralBase" | "DamageUpGeneralAdd" => "伤害增加%",
        "Mag" | "MagBase" | "MagAdd" | "MagUp" => "环合强度",
        "UnbalIntensity" | "UnbalIntensityBase" | "UnbalIntensityAdd" | "UnbalIntensityUp" => {
            "倾陷强度"
        }
        "HealUp" => "治疗加成",
        "DamageUpCosmosBase" => "光属性异能伤害增强%",
        "DamageUpNatureBase" => "灵属性异能伤害增强%",
        "DamageUpIncantationBase" => "咒属性异能伤害增强%",
        "DamageUpChaosBase" => "暗属性异能伤害增强%",
        "DamageUpPsycheBase" => "魂属性异能伤害增强%",
        "DamageUpLakshanaBase" => "相属性异能伤害增强%",
        "DamageUpPsychicallyBase" => "心灵伤害增强%",
        "ReactionGeneralDamageUp" => "环合伤害增强",
        "ReactionGuangLingDamageUp" => "创生伤害增强",
        "ReactionZhouAnDamageUp" => "浊燃伤害增强",
        "ReactionAnHunDamageUp" => "黯星伤害增强",
        _ => {
            return catalog
                .attributes
                .get(property)
                .map(|attribute| attribute.name(Language::SimplifiedChinese).to_owned());
        }
    };
    Some(mapped.to_owned())
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::*;
    use crate::engine::{
        model::{EmptyCurtainPlacement, EquipmentStat},
        parser::{EQUIPMENT_CATALOG_PATH, load_equipment_catalog},
    };

    #[test]
    fn four_secondary_stats_unlock_at_each_quarter_of_the_level_range() {
        assert_eq!(
            (0..4)
                .map(|index| substat_unlock_level(20, 4, index))
                .collect::<Vec<_>>(),
            vec![Some(5), Some(10), Some(15), Some(20)]
        );
        assert_eq!(substat_unlock_level(20, 4, 4), None);
        assert_eq!(substat_unlock_level(0, 4, 0), None);
    }

    #[test]
    fn drive_calculator_export_keeps_uid_and_scales_percent_stats() {
        let catalog = load_equipment_catalog(Path::new(EQUIPMENT_CATALOG_PATH)).unwrap();
        let item_id = catalog
            .items
            .iter()
            .find(|(_, definition)| definition.kind == EquipmentKind::Module)
            .map(|(id, _)| id.clone())
            .unwrap();
        let definition = &catalog.items[&item_id];
        let property = catalog
            .attributes
            .iter()
            .find(|(_, definition)| definition.percent)
            .map(|(id, _)| id.clone())
            .unwrap();
        let item = EmptyCurtainItem {
            id: HtItemNetId { solt: 7, serial: 9 },
            item_id,
            level: 1,
            main_stats: Vec::new(),
            sub_stats: vec![EquipmentStat {
                property,
                value: 0.125,
            }],
            locked: false,
            discarded: false,
            character_net_id: None,
            equipped_character_id: None,
            equipped_placement: definition
                .grid
                .map(|_| EmptyCurtainPlacement { row: 0, column: 0 }),
        };
        let records = build_drive_calculator_inventory(&[item], &catalog);
        assert_eq!(records[0]["uid"], "7_9");
        assert!(
            records[0]["sub_stats"]
                .as_object()
                .unwrap()
                .values()
                .all(|value| value.as_f64() == Some(12.5))
        );
    }
}
