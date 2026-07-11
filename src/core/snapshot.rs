//! Pure conversion from validated internal inventory items to enriched,
//! frontend-neutral snapshots. API DTOs map these types explicitly so internal
//! parser spellings never become protocol commitments.

use crate::engine::model::{EmptyCurtainItem, EquipmentStat, HtItemNetId};
use crate::engine::parser::{EquipmentCatalog, EquipmentKind};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LocalizedNames {
    pub zh_cn: String,
    pub en: String,
    pub ja: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ItemUid {
    pub slot: u32,
    pub serial: u32,
}

impl From<HtItemNetId> for ItemUid {
    fn from(id: HtItemNetId) -> Self {
        Self {
            slot: id.solt,
            serial: id.serial,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct InventoryStat {
    pub property_id: String,
    pub value: f32,
    pub percent: Option<bool>,
    pub names: Option<LocalizedNames>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct InventoryItem {
    pub uid: ItemUid,
    pub item_id: String,
    pub kind: Option<&'static str>,
    pub quality: Option<String>,
    pub geometry: Option<String>,
    pub grid: Option<u32>,
    pub suit_id: Option<String>,
    pub names: Option<LocalizedNames>,
    pub suit_names: Option<LocalizedNames>,
    pub level: u32,
    pub max_level: Option<u32>,
    pub locked: bool,
    pub equipped: bool,
    pub equipped_character_uid: Option<ItemUid>,
    pub main_stats: Vec<InventoryStat>,
    pub sub_stats: Vec<InventoryStat>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct InventorySnapshot {
    pub generation: u64,
    pub observed_at_unix_ms: u64,
    pub complete: bool,
    pub items: Vec<InventoryItem>,
}

impl InventorySnapshot {
    pub fn item_count(&self) -> usize {
        self.items.len()
    }
}

pub fn inventory_snapshot(
    items: &[EmptyCurtainItem],
    catalog: &EquipmentCatalog,
    generation: u64,
    observed_at_unix_ms: u64,
) -> InventorySnapshot {
    InventorySnapshot {
        generation,
        observed_at_unix_ms,
        complete: true,
        items: items
            .iter()
            .map(|item| inventory_item(item, catalog))
            .collect(),
    }
}

fn inventory_item(item: &EmptyCurtainItem, catalog: &EquipmentCatalog) -> InventoryItem {
    let definition = catalog.items.get(&item.item_id);
    let suit = definition
        .and_then(|definition| definition.suit.as_ref())
        .and_then(|suit_id| catalog.suits.get(suit_id));
    InventoryItem {
        uid: item.id.into(),
        item_id: item.item_id.clone(),
        kind: definition.map(|definition| match definition.kind {
            EquipmentKind::Module => "module",
            EquipmentKind::Core => "core",
        }),
        quality: definition.map(|definition| definition.quality.clone()),
        geometry: definition.map(|definition| definition.geometry.clone()),
        grid: definition.and_then(|definition| definition.grid),
        suit_id: definition.and_then(|definition| definition.suit.clone()),
        names: definition.map(|definition| LocalizedNames {
            zh_cn: definition.name_zh.clone(),
            en: definition.name_en.clone(),
            ja: definition.name_ja.clone(),
        }),
        suit_names: suit.map(|suit| LocalizedNames {
            zh_cn: suit.name_zh.clone(),
            en: suit.name_en.clone(),
            ja: suit.name_ja.clone(),
        }),
        level: item.level,
        max_level: definition.map(|definition| definition.max_level),
        locked: item.locked,
        equipped: item.is_equipped(),
        equipped_character_uid: item.character_net_id.map(ItemUid::from),
        main_stats: stats(&item.main_stats, catalog),
        sub_stats: stats(&item.sub_stats, catalog),
    }
}

fn stats(stats: &[EquipmentStat], catalog: &EquipmentCatalog) -> Vec<InventoryStat> {
    stats
        .iter()
        .map(|stat| {
            let definition = catalog.attributes.get(&stat.property);
            InventoryStat {
                property_id: stat.property.clone(),
                value: stat.value,
                percent: definition.map(|definition| definition.percent),
                names: definition.map(|definition| LocalizedNames {
                    zh_cn: definition.name_zh.clone(),
                    en: definition.name_en.clone(),
                    ja: definition.name_ja.clone(),
                }),
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::*;
    use crate::engine::parser::{EQUIPMENT_CATALOG_PATH, load_equipment_catalog};

    fn catalog() -> EquipmentCatalog {
        load_equipment_catalog(Path::new(EQUIPMENT_CATALOG_PATH)).unwrap()
    }

    #[test]
    fn maps_internal_solt_and_enriches_catalog_fields() {
        let item = EmptyCurtainItem {
            id: HtItemNetId { solt: 7, serial: 9 },
            item_id: "Attack_blue".to_owned(),
            level: 20,
            main_stats: vec![EquipmentStat {
                property: "AtkUp".to_owned(),
                value: 0.12,
            }],
            sub_stats: Vec::new(),
            locked: true,
            character_net_id: Some(HtItemNetId {
                solt: 11,
                serial: 13,
            }),
        };
        let snapshot = inventory_snapshot(&[item], &catalog(), 3, 1234);
        let mapped = &snapshot.items[0];
        assert_eq!(mapped.uid, ItemUid { slot: 7, serial: 9 });
        assert_eq!(mapped.kind, Some("core"));
        assert_eq!(mapped.suit_id.as_deref(), Some("Suit8"));
        assert_eq!(mapped.suit_names.as_ref().unwrap().en, "Shadow Creed");
        assert_eq!(mapped.main_stats[0].property_id, "AtkUp");
        assert_eq!(mapped.main_stats[0].percent, Some(true));
        assert_eq!(mapped.main_stats[0].names.as_ref().unwrap().en, "ATK Bonus");
        assert_eq!(
            mapped.equipped_character_uid,
            Some(ItemUid {
                slot: 11,
                serial: 13
            })
        );
        assert!(mapped.equipped);
        assert_eq!(snapshot.generation, 3);
        assert_eq!(snapshot.observed_at_unix_ms, 1234);
        assert!(snapshot.complete);
    }

    #[test]
    fn unknown_definition_preserves_ids_without_fabricating_metadata() {
        let item = EmptyCurtainItem {
            id: HtItemNetId { solt: 1, serial: 2 },
            item_id: "unknown-item".to_owned(),
            level: 1,
            main_stats: vec![EquipmentStat {
                property: "unknown-property".to_owned(),
                value: 5.0,
            }],
            sub_stats: Vec::new(),
            locked: false,
            character_net_id: None,
        };
        let snapshot = inventory_snapshot(&[item], &catalog(), 1, 1);
        let mapped = &snapshot.items[0];
        assert_eq!(mapped.item_id, "unknown-item");
        assert_eq!(mapped.kind, None);
        assert_eq!(mapped.names, None);
        assert_eq!(mapped.max_level, None);
        assert_eq!(mapped.main_stats[0].property_id, "unknown-property");
        assert_eq!(mapped.main_stats[0].percent, None);
        assert_eq!(mapped.main_stats[0].names, None);
    }
}
