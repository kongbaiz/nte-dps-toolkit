//! Pure conversion from validated internal inventory items to enriched,
//! frontend-neutral snapshots. API DTOs map these types explicitly so internal
//! parser spellings never become protocol commitments.

use std::collections::{HashMap, HashSet};

use serde::{Deserialize, Serialize};

use crate::engine::model::{
    CharacterInfo, EmptyCurtainCharacter, EmptyCurtainItem, EquipmentStat, HtItemNetId,
};
use crate::engine::parser::{EquipmentCatalog, EquipmentKind, valid_item_net_id};

pub const CHARACTER_LOADOUT_FORMAT_VERSION: u32 = 1;
pub const CHARACTER_LOADOUT_MAX_JSON_BYTES: usize = 64 * 1024;
const CHARACTER_LOADOUT_MAX_MODULES: usize = 49;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LocalizedNames {
    pub zh_cn: String,
    pub en: String,
    pub ja: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
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

impl From<ItemUid> for HtItemNetId {
    fn from(uid: ItemUid) -> Self {
        Self {
            solt: uid.slot,
            serial: uid.serial,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CharacterLoadoutItem {
    pub uid: ItemUid,
    pub item_id: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CharacterLoadoutModule {
    pub uid: ItemUid,
    pub item_id: String,
    pub row: i32,
    pub column: i32,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CharacterLoadoutFile {
    pub format_version: u32,
    pub character_id: u32,
    pub core: CharacterLoadoutItem,
    pub modules: Vec<CharacterLoadoutModule>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CharacterLoadoutPlacement {
    pub equipment: HtItemNetId,
    pub row: i32,
    pub column: i32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ValidatedCharacterLoadout {
    pub character: EmptyCurtainCharacter,
    pub core: HtItemNetId,
    pub placements: Vec<CharacterLoadoutPlacement>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CharacterLoadoutError {
    JsonTooLarge,
    InvalidJson,
    UnsupportedVersion(u32),
    CharacterUnavailable(u32),
    MissingCharacterPlan(u32),
    MissingCore,
    MultipleCores,
    MissingModules,
    TooManyModules,
    MissingPlacement(ItemUid),
    InvalidUid(ItemUid),
    DuplicateItem(ItemUid),
    UnknownItem(String),
    MissingInventoryItem(ItemUid),
    ItemMismatch(ItemUid),
    WrongKind(ItemUid),
    ItemInUse(ItemUid),
    InvalidPosition(ItemUid),
    OverlappingModules,
}

pub fn export_character_loadout_json(
    character: EmptyCurtainCharacter,
    items: &[EmptyCurtainItem],
    catalog: &EquipmentCatalog,
) -> Result<String, CharacterLoadoutError> {
    let mut core = None;
    let mut modules = Vec::new();
    for item in items
        .iter()
        .filter(|item| item.character_net_id == Some(character.net_id))
    {
        let definition = catalog
            .items
            .get(&item.item_id)
            .ok_or_else(|| CharacterLoadoutError::UnknownItem(item.item_id.clone()))?;
        match definition.kind {
            EquipmentKind::Core => {
                if core.is_some() {
                    return Err(CharacterLoadoutError::MultipleCores);
                }
                core = Some(CharacterLoadoutItem {
                    uid: item.id.into(),
                    item_id: item.item_id.clone(),
                });
            }
            EquipmentKind::Module => {
                let placement = item
                    .equipped_placement
                    .ok_or(CharacterLoadoutError::MissingPlacement(item.id.into()))?;
                modules.push(CharacterLoadoutModule {
                    uid: item.id.into(),
                    item_id: item.item_id.clone(),
                    row: placement.row,
                    column: placement.column,
                });
            }
        }
    }
    modules.sort_by_key(|module| {
        (
            module.row,
            module.column,
            module.uid.slot,
            module.uid.serial,
        )
    });
    let file = CharacterLoadoutFile {
        format_version: CHARACTER_LOADOUT_FORMAT_VERSION,
        character_id: character.character_id,
        core: core.ok_or(CharacterLoadoutError::MissingCore)?,
        modules,
    };
    validate_character_loadout(&file, &[character], items, catalog)?;
    Ok(serde_json::to_string_pretty(&file)
        .expect("validated character loadout contains only serializable fields"))
}

pub fn parse_character_loadout_json(
    json: &str,
) -> Result<CharacterLoadoutFile, CharacterLoadoutError> {
    if json.len() > CHARACTER_LOADOUT_MAX_JSON_BYTES {
        return Err(CharacterLoadoutError::JsonTooLarge);
    }
    let file: CharacterLoadoutFile =
        serde_json::from_str(json).map_err(|_| CharacterLoadoutError::InvalidJson)?;
    if file.format_version != CHARACTER_LOADOUT_FORMAT_VERSION {
        return Err(CharacterLoadoutError::UnsupportedVersion(
            file.format_version,
        ));
    }
    Ok(file)
}

pub fn validate_character_loadout(
    file: &CharacterLoadoutFile,
    characters: &[EmptyCurtainCharacter],
    items: &[EmptyCurtainItem],
    catalog: &EquipmentCatalog,
) -> Result<ValidatedCharacterLoadout, CharacterLoadoutError> {
    if file.format_version != CHARACTER_LOADOUT_FORMAT_VERSION {
        return Err(CharacterLoadoutError::UnsupportedVersion(
            file.format_version,
        ));
    }
    let character = characters
        .iter()
        .copied()
        .find(|character| character.character_id == file.character_id)
        .ok_or(CharacterLoadoutError::CharacterUnavailable(
            file.character_id,
        ))?;
    if !catalog.plans.contains_key(&character.character_id) {
        return Err(CharacterLoadoutError::MissingCharacterPlan(
            character.character_id,
        ));
    }
    if file.modules.is_empty() {
        return Err(CharacterLoadoutError::MissingModules);
    }
    if file.modules.len() > CHARACTER_LOADOUT_MAX_MODULES {
        return Err(CharacterLoadoutError::TooManyModules);
    }

    let inventory = items
        .iter()
        .map(|item| (item.id, item))
        .collect::<HashMap<_, _>>();
    let mut equipment = HashSet::with_capacity(file.modules.len() + 1);
    let core = validate_loadout_item(
        file.core.uid,
        &file.core.item_id,
        EquipmentKind::Core,
        character.net_id,
        &inventory,
        catalog,
    )?;
    equipment.insert(core.id);

    let mut occupied = HashSet::new();
    let mut placements = Vec::with_capacity(file.modules.len());
    for module in &file.modules {
        let item = validate_loadout_item(
            module.uid,
            &module.item_id,
            EquipmentKind::Module,
            character.net_id,
            &inventory,
            catalog,
        )?;
        if !equipment.insert(item.id) {
            return Err(CharacterLoadoutError::DuplicateItem(module.uid));
        }
        let origin = crate::engine::parser::EquipmentGridCell {
            row: module.row,
            column: module.column,
        };
        let valid_positions = catalog
            .valid_module_positions(character.character_id, &module.item_id)
            .ok_or(CharacterLoadoutError::InvalidPosition(module.uid))?;
        if !valid_positions.contains(&origin) {
            return Err(CharacterLoadoutError::InvalidPosition(module.uid));
        }
        let definition = catalog
            .items
            .get(&module.item_id)
            .expect("validated loadout module must have catalog metadata");
        let shape = catalog
            .shapes
            .get(&definition.geometry)
            .expect("validated module definition must have a shape");
        for cell in &shape.cells {
            let occupied_cell = (module.row + cell.row, module.column + cell.column);
            if !occupied.insert(occupied_cell) {
                return Err(CharacterLoadoutError::OverlappingModules);
            }
        }
        placements.push(CharacterLoadoutPlacement {
            equipment: item.id,
            row: module.row,
            column: module.column,
        });
    }
    Ok(ValidatedCharacterLoadout {
        character,
        core: core.id,
        placements,
    })
}

fn validate_loadout_item<'a>(
    uid: ItemUid,
    item_id: &str,
    expected_kind: EquipmentKind,
    character: HtItemNetId,
    inventory: &HashMap<HtItemNetId, &'a EmptyCurtainItem>,
    catalog: &EquipmentCatalog,
) -> Result<&'a EmptyCurtainItem, CharacterLoadoutError> {
    let id = HtItemNetId::from(uid);
    if !valid_item_net_id(id) {
        return Err(CharacterLoadoutError::InvalidUid(uid));
    }
    let definition = catalog
        .items
        .get(item_id)
        .ok_or_else(|| CharacterLoadoutError::UnknownItem(item_id.to_owned()))?;
    if definition.kind != expected_kind {
        return Err(CharacterLoadoutError::WrongKind(uid));
    }
    let item = inventory
        .get(&id)
        .copied()
        .ok_or(CharacterLoadoutError::MissingInventoryItem(uid))?;
    if item.item_id != item_id {
        return Err(CharacterLoadoutError::ItemMismatch(uid));
    }
    if item
        .character_net_id
        .is_some_and(|equipped| equipped != character)
    {
        return Err(CharacterLoadoutError::ItemInUse(uid));
    }
    Ok(item)
}

#[derive(Clone, Debug, PartialEq)]
pub struct InventoryStat {
    pub property_id: String,
    pub value: f32,
    pub percent: Option<bool>,
    pub names: Option<LocalizedNames>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct InventoryPlacement {
    pub row: i32,
    pub column: i32,
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
    pub discarded: bool,
    pub equipped: bool,
    pub equipped_character_uid: Option<ItemUid>,
    pub equipped_character_id: Option<u32>,
    pub equipped_placement: Option<InventoryPlacement>,
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
    characters: &HashMap<u32, CharacterInfo>,
    generation: u64,
    observed_at_unix_ms: u64,
) -> InventorySnapshot {
    InventorySnapshot {
        generation,
        observed_at_unix_ms,
        complete: true,
        items: items
            .iter()
            .map(|item| inventory_item(item, catalog, characters))
            .collect(),
    }
}

fn inventory_item(
    item: &EmptyCurtainItem,
    catalog: &EquipmentCatalog,
    characters: &HashMap<u32, CharacterInfo>,
) -> InventoryItem {
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
        discarded: item.discarded,
        equipped: item.is_equipped(),
        equipped_character_uid: item.character_net_id.map(ItemUid::from),
        equipped_character_id: item
            .equipped_character_id
            .filter(|character_id| characters.contains_key(character_id)),
        equipped_placement: item.equipped_placement.map(|placement| InventoryPlacement {
            row: placement.row,
            column: placement.column,
        }),
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
    use crate::engine::model::EmptyCurtainPlacement;
    use crate::engine::parser::{EQUIPMENT_CATALOG_PATH, load_equipment_catalog};

    fn catalog() -> EquipmentCatalog {
        load_equipment_catalog(Path::new(EQUIPMENT_CATALOG_PATH)).unwrap()
    }

    fn characters() -> HashMap<u32, CharacterInfo> {
        HashMap::from([(
            1020,
            CharacterInfo {
                name_zh: String::new(),
                name_en: "Haniel".to_owned(),
                color: None,
                avatar: None,
                attribute: None,
            },
        )])
    }

    fn loadout_character(slot: u32, serial: u32) -> EmptyCurtainCharacter {
        EmptyCurtainCharacter {
            net_id: HtItemNetId { solt: slot, serial },
            character_id: 1023,
        }
    }

    fn loadout_items(
        catalog: &EquipmentCatalog,
        character: EmptyCurtainCharacter,
    ) -> Vec<EmptyCurtainItem> {
        let plan = catalog
            .plans
            .get(&character.character_id)
            .expect("test character must have an equipment plan");
        let mut items = plan
            .recommended_modules
            .iter()
            .enumerate()
            .map(|(index, module)| EmptyCurtainItem {
                id: HtItemNetId {
                    solt: 100 + index as u32,
                    serial: 1_000 + index as u32,
                },
                item_id: module.item_id.clone(),
                level: 1,
                main_stats: Vec::new(),
                sub_stats: Vec::new(),
                locked: false,
                discarded: false,
                character_net_id: Some(character.net_id),
                equipped_character_id: Some(character.character_id),
                equipped_placement: Some(EmptyCurtainPlacement {
                    row: module.row,
                    column: module.column,
                }),
            })
            .collect::<Vec<_>>();
        items.push(EmptyCurtainItem {
            id: HtItemNetId {
                solt: 200,
                serial: 2_000,
            },
            item_id: plan.recommended_core.clone(),
            level: 1,
            main_stats: Vec::new(),
            sub_stats: Vec::new(),
            locked: false,
            discarded: false,
            character_net_id: Some(character.net_id),
            equipped_character_id: Some(character.character_id),
            equipped_placement: None,
        });
        items
    }

    #[test]
    fn character_loadout_round_trips_and_rebinds_current_character_session() {
        let catalog = catalog();
        let previous_character = loadout_character(10, 20);
        let mut items = loadout_items(&catalog, previous_character);
        let json = export_character_loadout_json(previous_character, &items, &catalog)
            .expect("complete equipped loadout must export");
        assert!(json.contains("\"slot\""));
        assert!(!json.contains("\"solt\""));
        assert!(!json.contains("character_net_id"));

        let file = parse_character_loadout_json(&json).expect("exported loadout must parse");
        let current_character = loadout_character(30, 40);
        for item in &mut items {
            item.character_net_id = None;
            item.equipped_character_id = None;
            item.equipped_placement = None;
        }
        let validated = validate_character_loadout(&file, &[current_character], &items, &catalog)
            .expect("persistent item UIDs must rebind to the current character session");

        assert_eq!(validated.character, current_character);
        assert_eq!(
            validated.core,
            HtItemNetId {
                solt: 200,
                serial: 2_000
            }
        );
        assert_eq!(
            validated.placements.len(),
            catalog
                .plans
                .get(&current_character.character_id)
                .expect("test character must have a plan")
                .recommended_modules
                .len()
        );
    }

    #[test]
    fn character_loadout_export_requires_complete_equipment_state() {
        let catalog = catalog();
        let character = loadout_character(10, 20);
        let mut items = loadout_items(&catalog, character);
        let module = items
            .iter_mut()
            .find(|item| item.equipped_placement.is_some())
            .expect("test loadout must contain a module");
        let missing_uid = ItemUid::from(module.id);
        module.equipped_placement = None;
        assert_eq!(
            export_character_loadout_json(character, &items, &catalog),
            Err(CharacterLoadoutError::MissingPlacement(missing_uid))
        );

        let mut items = loadout_items(&catalog, character);
        items.retain(|item| item.equipped_placement.is_some());
        assert_eq!(
            export_character_loadout_json(character, &items, &catalog),
            Err(CharacterLoadoutError::MissingCore)
        );
    }

    #[test]
    fn character_loadout_import_rejects_missing_mismatched_and_in_use_items() {
        let catalog = catalog();
        let character = loadout_character(10, 20);
        let items = loadout_items(&catalog, character);
        let json = export_character_loadout_json(character, &items, &catalog)
            .expect("complete equipped loadout must export");
        let file = parse_character_loadout_json(&json).expect("exported loadout must parse");
        let missing_uid = file.modules[0].uid;

        let mut missing_items = items.clone();
        missing_items.retain(|item| ItemUid::from(item.id) != missing_uid);
        assert_eq!(
            validate_character_loadout(&file, &[character], &missing_items, &catalog),
            Err(CharacterLoadoutError::MissingInventoryItem(missing_uid))
        );

        let mut mismatched_file = file.clone();
        mismatched_file.modules[0].item_id = file.modules[1].item_id.clone();
        assert_eq!(
            validate_character_loadout(&mismatched_file, &[character], &items, &catalog),
            Err(CharacterLoadoutError::ItemMismatch(missing_uid))
        );

        let mut in_use_items = items.clone();
        let in_use_item = in_use_items
            .iter_mut()
            .find(|item| ItemUid::from(item.id) == missing_uid)
            .expect("test inventory must contain referenced module");
        in_use_item.character_net_id = Some(HtItemNetId {
            solt: 999,
            serial: 999,
        });
        assert_eq!(
            validate_character_loadout(&file, &[character], &in_use_items, &catalog),
            Err(CharacterLoadoutError::ItemInUse(missing_uid))
        );
    }

    #[test]
    fn character_loadout_import_rejects_invalid_or_overlapping_layout() {
        let catalog = catalog();
        let character = loadout_character(10, 20);
        let items = loadout_items(&catalog, character);
        let json = export_character_loadout_json(character, &items, &catalog)
            .expect("complete equipped loadout must export");
        let file = parse_character_loadout_json(&json).expect("exported loadout must parse");

        let mut invalid = file.clone();
        let invalid_uid = invalid.modules[0].uid;
        invalid.modules[0].row = 0;
        invalid.modules[0].column = 0;
        assert_eq!(
            validate_character_loadout(&invalid, &[character], &items, &catalog),
            Err(CharacterLoadoutError::InvalidPosition(invalid_uid))
        );

        let mut duplicate = file.clone();
        duplicate.modules.push(duplicate.modules[0].clone());
        assert_eq!(
            validate_character_loadout(&duplicate, &[character], &items, &catalog),
            Err(CharacterLoadoutError::DuplicateItem(file.modules[0].uid))
        );

        let repeated = file
            .modules
            .iter()
            .enumerate()
            .find_map(|(first_index, first)| {
                file.modules
                    .iter()
                    .enumerate()
                    .skip(first_index + 1)
                    .find(|(_, second)| second.item_id == first.item_id)
                    .map(|(second_index, _)| (first_index, second_index))
            })
            .expect("test plan must contain repeated module geometry");
        let mut overlapping = file.clone();
        overlapping.modules[repeated.1].row = overlapping.modules[repeated.0].row;
        overlapping.modules[repeated.1].column = overlapping.modules[repeated.0].column;
        assert_eq!(
            validate_character_loadout(&overlapping, &[character], &items, &catalog),
            Err(CharacterLoadoutError::OverlappingModules)
        );
    }

    #[test]
    fn character_loadout_json_boundary_is_strict_and_versioned() {
        let unknown_field = r#"{
            "format_version": 1,
            "character_id": 1023,
            "core": {"uid": {"slot": 1, "serial": 2}, "item_id": "Incantation_orange"},
            "modules": [],
            "unexpected": true
        }"#;
        assert_eq!(
            parse_character_loadout_json(unknown_field),
            Err(CharacterLoadoutError::InvalidJson)
        );

        let unsupported = r#"{
            "format_version": 2,
            "character_id": 1023,
            "core": {"uid": {"slot": 1, "serial": 2}, "item_id": "Incantation_orange"},
            "modules": []
        }"#;
        assert_eq!(
            parse_character_loadout_json(unsupported),
            Err(CharacterLoadoutError::UnsupportedVersion(2))
        );

        let oversized = " ".repeat(CHARACTER_LOADOUT_MAX_JSON_BYTES + 1);
        assert_eq!(
            parse_character_loadout_json(&oversized),
            Err(CharacterLoadoutError::JsonTooLarge)
        );
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
            discarded: true,
            character_net_id: Some(HtItemNetId {
                solt: 11,
                serial: 13,
            }),
            equipped_character_id: Some(1020),
            equipped_placement: None,
        };
        let snapshot = inventory_snapshot(&[item], &catalog(), &characters(), 3, 1234);
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
        assert!(mapped.discarded);
        assert_eq!(mapped.equipped_character_id, Some(1020));
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
            discarded: false,
            character_net_id: Some(HtItemNetId {
                solt: 20,
                serial: 21,
            }),
            equipped_character_id: Some(9999),
            equipped_placement: None,
        };
        let snapshot = inventory_snapshot(&[item], &catalog(), &HashMap::new(), 1, 1);
        let mapped = &snapshot.items[0];
        assert_eq!(mapped.item_id, "unknown-item");
        assert_eq!(mapped.kind, None);
        assert_eq!(mapped.names, None);
        assert_eq!(mapped.max_level, None);
        assert_eq!(mapped.main_stats[0].property_id, "unknown-property");
        assert_eq!(mapped.main_stats[0].percent, None);
        assert_eq!(mapped.main_stats[0].names, None);
        assert_eq!(mapped.equipped_character_id, None);
    }
}
