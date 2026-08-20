use serde::Serialize;

use nte_dps_tool::{
    core::empty_curtain::substat_unlock_level,
    core::snapshot::{InventoryItem, InventorySnapshot, InventoryStat, ItemUid},
    engine::parser::EquipmentCatalog,
    storage::i18n::{self, Language},
};

use crate::equipment_operation_service::EmptyCurtainOperationState;

pub(crate) const EMPTY_CURTAIN_CONTRACT_VERSION: u32 = 2;
pub(crate) const EMPTY_CURTAIN_MAX_CHARACTERS: usize = 64;
pub(crate) const EMPTY_CURTAIN_MAX_ITEMS: usize = 4_096;
pub(crate) const EMPTY_CURTAIN_MAX_DETAILS_PER_ITEM: usize = 16;
pub(crate) const EMPTY_CURTAIN_MAX_TOTAL_DETAILS: usize = 20_000;
pub(crate) const EMPTY_CURTAIN_MAX_TEXT_BYTES: usize = 256;
pub(crate) const EMPTY_CURTAIN_MAX_ICON_BYTES: usize = 1_024;
pub(crate) const EMPTY_CURTAIN_MAX_PROJECTED_TEXT_BYTES: usize = 1024 * 1024;

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct EmptyCurtainSnapshot {
    pub contract_version: u32,
    pub generation: String,
    pub observed_at_unix_ms: String,
    pub has_data: bool,
    pub complete: bool,
    pub characters: Vec<EmptyCurtainCharacterSnapshot>,
    pub items: Vec<EmptyCurtainItemSnapshot>,
    pub operation: EmptyCurtainOperationSnapshot,
}

impl EmptyCurtainSnapshot {
    pub(crate) fn from_inventory(
        inventory: InventorySnapshot,
        catalog: &EquipmentCatalog,
        characters: &std::collections::HashMap<u32, nte_dps_tool::engine::model::CharacterInfo>,
        operation: EmptyCurtainOperationState,
    ) -> Self {
        let language = i18n::current_language();
        let has_data = !inventory.items.is_empty();
        let mut complete = inventory.complete;
        if inventory.characters.len() > EMPTY_CURTAIN_MAX_CHARACTERS {
            complete = false;
        }
        let mut projected_characters =
            Vec::with_capacity(inventory.characters.len().min(EMPTY_CURTAIN_MAX_CHARACTERS));
        for character in inventory
            .characters
            .into_iter()
            .take(EMPTY_CURTAIN_MAX_CHARACTERS)
        {
            let fallback = character.character_id.to_string();
            let name = characters
                .get(&character.character_id)
                .map(|definition| match language {
                    Language::English | Language::Japanese => definition.name_en.as_str(),
                    Language::SimplifiedChinese => definition.name_zh.as_str(),
                })
                .filter(|name| !name.is_empty())
                .unwrap_or(&fallback);
            let (name, exact) =
                bounded_text(name.to_owned(), EMPTY_CURTAIN_MAX_TEXT_BYTES, fallback);
            complete &= exact;
            projected_characters.push(EmptyCurtainCharacterSnapshot {
                uid: character.uid.into(),
                character_id: character.character_id,
                name,
            });
        }

        let mut projected_items =
            Vec::with_capacity(inventory.items.len().min(EMPTY_CURTAIN_MAX_ITEMS));
        let mut projected_text_bytes = projected_characters
            .iter()
            .fold(0_usize, |total, row| total.saturating_add(row.name.len()));
        let mut projected_detail_rows = 0_usize;
        for item in inventory.items {
            if projected_items.len() >= EMPTY_CURTAIN_MAX_ITEMS {
                complete = false;
                continue;
            }
            let (candidate, candidate_complete) = item_snapshot(item, catalog, language);
            let candidate_details = candidate
                .stats
                .len()
                .saturating_add(candidate.set_effects.len());
            let candidate_text = item_text_bytes(&candidate);
            if projected_detail_rows.saturating_add(candidate_details)
                > EMPTY_CURTAIN_MAX_TOTAL_DETAILS
                || projected_text_bytes.saturating_add(candidate_text)
                    > EMPTY_CURTAIN_MAX_PROJECTED_TEXT_BYTES
            {
                complete = false;
                continue;
            }
            complete &= candidate_complete;
            projected_detail_rows = projected_detail_rows.saturating_add(candidate_details);
            projected_text_bytes = projected_text_bytes.saturating_add(candidate_text);
            projected_items.push(candidate);
        }
        Self {
            contract_version: EMPTY_CURTAIN_CONTRACT_VERSION,
            generation: inventory.generation.to_string(),
            observed_at_unix_ms: inventory.observed_at_unix_ms.to_string(),
            has_data,
            complete,
            characters: projected_characters,
            items: projected_items,
            operation: operation.into(),
        }
    }
}

#[derive(Clone, Copy, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ItemUidSnapshot {
    pub slot: u32,
    pub serial: u32,
}

impl From<ItemUid> for ItemUidSnapshot {
    fn from(value: ItemUid) -> Self {
        Self {
            slot: value.slot,
            serial: value.serial,
        }
    }
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct EmptyCurtainCharacterSnapshot {
    pub uid: ItemUidSnapshot,
    pub character_id: u32,
    pub name: String,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct EmptyCurtainItemSnapshot {
    pub uid: ItemUidSnapshot,
    pub item_id: String,
    pub filter_id: String,
    pub kind: Option<&'static str>,
    pub quality: Option<String>,
    pub name: String,
    pub icon: Option<String>,
    pub level: u32,
    pub max_level: Option<u32>,
    pub locked: bool,
    pub discarded: bool,
    pub equipped_character_uid: Option<ItemUidSnapshot>,
    pub equipped_character_id: Option<u32>,
    pub equipped_placement: Option<EmptyCurtainPlacementSnapshot>,
    pub stats: Vec<EmptyCurtainStatSnapshot>,
    pub set_name: Option<String>,
    pub set_effects: Vec<EmptyCurtainSetEffectSnapshot>,
}

#[derive(Clone, Copy, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct EmptyCurtainPlacementSnapshot {
    pub row: i32,
    pub column: i32,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct EmptyCurtainStatSnapshot {
    pub property: String,
    pub label: String,
    pub value: f32,
    pub percent: bool,
    pub main: bool,
    pub unlock_level: Option<u32>,
    pub unlocked: bool,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct EmptyCurtainSetEffectSnapshot {
    pub count: u32,
    pub text: String,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct EmptyCurtainOperationSnapshot {
    pub status: &'static str,
    pub message_key: &'static str,
    pub message_arguments: Vec<String>,
}

impl From<EmptyCurtainOperationState> for EmptyCurtainOperationSnapshot {
    fn from(value: EmptyCurtainOperationState) -> Self {
        Self {
            status: value.status,
            message_key: value.message_key,
            message_arguments: value
                .message_arguments
                .into_iter()
                .take(EMPTY_CURTAIN_MAX_DETAILS_PER_ITEM)
                .map(|argument| {
                    bounded_text(
                        argument,
                        EMPTY_CURTAIN_MAX_TEXT_BYTES,
                        "[omitted]".to_owned(),
                    )
                    .0
                })
                .collect(),
        }
    }
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct EmptyCurtainFileResult {
    pub completed: bool,
    pub snapshot: EmptyCurtainSnapshot,
}

#[derive(Clone, Copy, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct EmptyCurtainPositionSnapshot {
    pub row: i32,
    pub column: i32,
}

#[derive(Clone, Debug, Serialize)]
#[serde(tag = "event", content = "payload", rename_all = "camelCase")]
pub(crate) enum EmptyCurtainEvent {
    Snapshot(EmptyCurtainSnapshot),
}

fn item_snapshot(
    item: InventoryItem,
    catalog: &EquipmentCatalog,
    language: Language,
) -> (EmptyCurtainItemSnapshot, bool) {
    let item_fallback = format!("#{}:{}", item.uid.slot, item.uid.serial);
    let definition = catalog.items.get(&item.item_id);
    let suit = item
        .suit_id
        .as_deref()
        .and_then(|suit_id| catalog.suits.get(suit_id));
    let original_stat_count = item.main_stats.len().saturating_add(item.sub_stats.len());
    let definition_substat_count = definition.map_or(item.sub_stats.len(), |definition| {
        definition.sub_count.max(item.sub_stats.len())
    });
    let mut complete = original_stat_count <= EMPTY_CURTAIN_MAX_DETAILS_PER_ITEM;
    let mut stats = Vec::with_capacity(original_stat_count.min(EMPTY_CURTAIN_MAX_DETAILS_PER_ITEM));
    for stat in item
        .main_stats
        .into_iter()
        .take(EMPTY_CURTAIN_MAX_DETAILS_PER_ITEM)
    {
        let (stat, exact) = stat_snapshot(stat, language, true, None, true);
        complete &= exact;
        stats.push(stat);
    }
    let remaining_stats = EMPTY_CURTAIN_MAX_DETAILS_PER_ITEM.saturating_sub(stats.len());
    for (index, stat) in item.sub_stats.into_iter().enumerate().take(remaining_stats) {
        let unlock_level = substat_unlock_level(
            definition.map_or(item.max_level.unwrap_or(20), |definition| {
                definition.max_level
            }),
            definition_substat_count,
            index,
        );
        let (stat, exact) = stat_snapshot(
            stat,
            language,
            false,
            unlock_level,
            unlock_level.is_none_or(|required| item.level >= required),
        );
        complete &= exact;
        stats.push(stat);
    }
    let (item_id, item_id_exact) = bounded_text(
        item.item_id.clone(),
        EMPTY_CURTAIN_MAX_TEXT_BYTES,
        item_fallback.clone(),
    );
    complete &= item_id_exact;
    let set_effect_count = suit.map_or(0, |suit| suit.effects.len());
    complete &= set_effect_count <= EMPTY_CURTAIN_MAX_DETAILS_PER_ITEM;
    let (filter_id, filter_exact) = definition
        .map(|definition| match definition.kind {
            nte_dps_tool::engine::parser::EquipmentKind::Module => definition.geometry.clone(),
            nte_dps_tool::engine::parser::EquipmentKind::Core => {
                definition.suit.clone().unwrap_or_else(|| item_id.clone())
            }
        })
        .map(|value| bounded_text(value, EMPTY_CURTAIN_MAX_TEXT_BYTES, item_id.clone()))
        .unwrap_or_else(|| (item_id.clone(), true));
    complete &= filter_exact;
    let (quality, quality_exact) = item.quality.map_or((None, true), |value| {
        bounded_optional_text(value, EMPTY_CURTAIN_MAX_TEXT_BYTES)
    });
    complete &= quality_exact;
    let (name, name_exact) = item
        .names
        .as_ref()
        .map(|names| localized_names(names, language))
        .map(|value| bounded_text(value, EMPTY_CURTAIN_MAX_TEXT_BYTES, item_id.clone()))
        .unwrap_or_else(|| (item_id.clone(), true));
    complete &= name_exact;
    let (icon, icon_exact) = definition.map_or((None, true), |definition| {
        bounded_optional_text(definition.icon.clone(), EMPTY_CURTAIN_MAX_ICON_BYTES)
    });
    complete &= icon_exact;
    let (set_name, set_name_exact) = suit.map_or((None, true), |suit| {
        bounded_optional_text(suit.name(language).to_owned(), EMPTY_CURTAIN_MAX_TEXT_BYTES)
    });
    complete &= set_name_exact;
    let mut set_effects =
        Vec::with_capacity(set_effect_count.min(EMPTY_CURTAIN_MAX_DETAILS_PER_ITEM));
    if let Some(suit) = suit {
        for effect in suit.effects.iter().take(EMPTY_CURTAIN_MAX_DETAILS_PER_ITEM) {
            let (text, exact) = bounded_text(
                effect.text(language).to_owned(),
                EMPTY_CURTAIN_MAX_TEXT_BYTES,
                String::new(),
            );
            complete &= exact;
            set_effects.push(EmptyCurtainSetEffectSnapshot {
                count: effect.count,
                text,
            });
        }
    }
    let snapshot = EmptyCurtainItemSnapshot {
        uid: item.uid.into(),
        item_id: item_id.clone(),
        filter_id,
        kind: item.kind,
        quality,
        name,
        icon,
        level: item.level,
        max_level: item.max_level,
        locked: item.locked,
        discarded: item.discarded,
        equipped_character_uid: item.equipped_character_uid.map(Into::into),
        equipped_character_id: item.equipped_character_id,
        equipped_placement: item.equipped_placement.map(|placement| {
            EmptyCurtainPlacementSnapshot {
                row: placement.row,
                column: placement.column,
            }
        }),
        stats,
        set_name,
        set_effects,
    };
    (snapshot, complete)
}

fn stat_snapshot(
    stat: InventoryStat,
    language: Language,
    main: bool,
    unlock_level: Option<u32>,
    unlocked: bool,
) -> (EmptyCurtainStatSnapshot, bool) {
    let property_id = stat.property_id;
    let (property, property_exact) = bounded_text(
        property_id.clone(),
        EMPTY_CURTAIN_MAX_TEXT_BYTES,
        String::new(),
    );
    let (label, label_exact) = stat
        .names
        .as_ref()
        .map(|names| localized_names(names, language))
        .map(|value| bounded_text(value, EMPTY_CURTAIN_MAX_TEXT_BYTES, property_id.clone()))
        .unwrap_or_else(|| bounded_text(property_id, EMPTY_CURTAIN_MAX_TEXT_BYTES, String::new()));
    (
        EmptyCurtainStatSnapshot {
            property,
            label,
            value: stat.value,
            percent: stat.percent.unwrap_or(false),
            main,
            unlock_level,
            unlocked,
        },
        property_exact && label_exact,
    )
}

fn bounded_text(value: String, limit: usize, fallback: String) -> (String, bool) {
    if value.len() <= limit {
        (value, true)
    } else if fallback.len() <= limit {
        (fallback, false)
    } else {
        (String::new(), false)
    }
}

fn bounded_optional_text(value: String, limit: usize) -> (Option<String>, bool) {
    if value.len() <= limit {
        (Some(value), true)
    } else {
        (None, false)
    }
}

fn item_text_bytes(item: &EmptyCurtainItemSnapshot) -> usize {
    let mut total = item
        .item_id
        .len()
        .saturating_add(item.filter_id.len())
        .saturating_add(item.quality.as_deref().map_or(0, str::len))
        .saturating_add(item.name.len())
        .saturating_add(item.icon.as_deref().map_or(0, str::len))
        .saturating_add(item.set_name.as_deref().map_or(0, str::len));
    for stat in &item.stats {
        total = total
            .saturating_add(stat.property.len())
            .saturating_add(stat.label.len());
    }
    for effect in &item.set_effects {
        total = total.saturating_add(effect.text.len());
    }
    total
}

fn localized_names(
    names: &nte_dps_tool::core::snapshot::LocalizedNames,
    language: Language,
) -> String {
    match language {
        Language::English => names.en.clone(),
        Language::Japanese => names.ja.clone(),
        Language::SimplifiedChinese => names.zh_cn.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nte_dps_tool::{
        core::snapshot::{InventoryCharacter, InventoryPlacement, LocalizedNames},
        engine::{
            model::CharacterInfo,
            parser::{
                EquipmentItemDefinition, EquipmentKind, EquipmentSuitDefinition,
                EquipmentSuitEffect,
            },
        },
    };

    use crate::{
        channels::stream_runtime::serialize_stream_events,
        contract::stream::MAX_STREAM_DELIVERY_BYTES,
    };

    #[test]
    fn item_uids_remain_bounded_numbers() {
        let uid = ItemUidSnapshot::from(ItemUid {
            slot: u32::MAX,
            serial: u32::MAX,
        });
        let json = serde_json::to_value(uid).unwrap();
        assert_eq!(json["slot"], u32::MAX);
        assert_eq!(json["serial"], u32::MAX);
    }

    #[test]
    fn secondary_stat_unlock_state_uses_explicit_contract_fields() {
        let json = serde_json::to_value(EmptyCurtainStatSnapshot {
            property: "CritAdd".to_owned(),
            label: "Critical Rate".to_owned(),
            value: 0.08,
            percent: true,
            main: false,
            unlock_level: Some(15),
            unlocked: false,
        })
        .unwrap();

        assert_eq!(json["unlockLevel"], 15);
        assert_eq!(json["unlocked"], false);
    }

    fn adversarial_item(index: u32, text: &str) -> InventoryItem {
        let stat = InventoryStat {
            property_id: text.to_owned(),
            value: 1.0,
            percent: Some(false),
            names: None,
        };
        InventoryItem {
            uid: ItemUid {
                slot: index,
                serial: index,
            },
            item_id: text.to_owned(),
            kind: None,
            quality: None,
            geometry: None,
            grid: None,
            suit_id: None,
            names: None,
            suit_names: None,
            level: 1,
            max_level: Some(20),
            locked: false,
            discarded: false,
            equipped: false,
            equipped_character_uid: None,
            equipped_character_id: None,
            equipped_placement: None::<InventoryPlacement>,
            main_stats: vec![stat; EMPTY_CURTAIN_MAX_DETAILS_PER_ITEM],
            sub_stats: Vec::new(),
        }
    }

    #[test]
    fn oversized_inventory_text_falls_back_without_splitting_utf8() {
        let inventory = InventorySnapshot {
            generation: 1,
            observed_at_unix_ms: 2,
            complete: true,
            characters: vec![InventoryCharacter {
                uid: ItemUid { slot: 1, serial: 1 },
                character_id: 7,
            }],
            items: vec![adversarial_item(1, &"界".repeat(86))],
        };

        let snapshot = EmptyCurtainSnapshot::from_inventory(
            inventory,
            &EquipmentCatalog::default(),
            &std::collections::HashMap::new(),
            EmptyCurtainOperationState::default(),
        );

        assert_eq!(snapshot.items[0].item_id, "#1:1");
        assert!(
            !snapshot.complete,
            "text fallback must mark the snapshot partial"
        );
        assert!(
            snapshot.items[0]
                .stats
                .iter()
                .all(|stat| stat.property.len() <= EMPTY_CURTAIN_MAX_TEXT_BYTES)
        );
    }

    #[test]
    fn exact_text_limits_remain_complete_and_large_stat_sources_allocate_only_the_contract_page() {
        let exact = "x".repeat(EMPTY_CURTAIN_MAX_TEXT_BYTES);
        let exact_snapshot = EmptyCurtainSnapshot::from_inventory(
            InventorySnapshot {
                generation: 1,
                observed_at_unix_ms: 2,
                complete: true,
                characters: Vec::new(),
                items: vec![adversarial_item(1, &exact)],
            },
            &EquipmentCatalog::default(),
            &std::collections::HashMap::new(),
            EmptyCurtainOperationState::default(),
        );
        assert!(exact_snapshot.complete);
        assert_eq!(
            exact_snapshot.items[0].stats.len(),
            EMPTY_CURTAIN_MAX_DETAILS_PER_ITEM
        );

        let mut large = adversarial_item(2, "stat");
        large.main_stats = vec![large.main_stats[0].clone(); 100_000];
        let large_snapshot = EmptyCurtainSnapshot::from_inventory(
            InventorySnapshot {
                generation: 1,
                observed_at_unix_ms: 2,
                complete: true,
                characters: Vec::new(),
                items: vec![large],
            },
            &EquipmentCatalog::default(),
            &std::collections::HashMap::new(),
            EmptyCurtainOperationState::default(),
        );
        assert!(!large_snapshot.complete);
        assert_eq!(
            large_snapshot.items[0].stats.len(),
            EMPTY_CURTAIN_MAX_DETAILS_PER_ITEM
        );
    }

    #[test]
    fn every_catalog_and_character_text_omission_marks_snapshot_incomplete() {
        let oversized = "界".repeat(86);
        let mut catalog = EquipmentCatalog::default();
        catalog.items.insert(
            "known".to_owned(),
            EquipmentItemDefinition {
                kind: EquipmentKind::Module,
                name_zh: oversized.clone(),
                name_en: oversized.clone(),
                name_ja: oversized.clone(),
                quality: "orange".to_owned(),
                geometry: oversized.clone(),
                grid: Some(1),
                suit: Some("suit".to_owned()),
                icon: "i".repeat(EMPTY_CURTAIN_MAX_ICON_BYTES + 1),
                max_level: 20,
                main_count: 1,
                sub_count: 1,
            },
        );
        catalog.suits.insert(
            "suit".to_owned(),
            EquipmentSuitDefinition {
                name_zh: oversized.clone(),
                name_en: oversized.clone(),
                name_ja: oversized.clone(),
                effects: vec![EquipmentSuitEffect {
                    count: 2,
                    text_zh: oversized.clone(),
                    text_en: oversized.clone(),
                    text_ja: oversized.clone(),
                }],
            },
        );
        let mut item = adversarial_item(1, "known");
        item.suit_id = Some("suit".to_owned());
        item.quality = Some("q".repeat(EMPTY_CURTAIN_MAX_TEXT_BYTES + 1));
        item.names = Some(LocalizedNames {
            zh_cn: oversized.clone(),
            en: oversized.clone(),
            ja: oversized.clone(),
        });
        item.main_stats[0].names = Some(LocalizedNames {
            zh_cn: oversized.clone(),
            en: oversized.clone(),
            ja: oversized.clone(),
        });
        let characters = std::collections::HashMap::from([(
            7,
            CharacterInfo {
                name_zh: oversized.clone(),
                name_en: oversized,
                color: None,
                avatar: None,
                attribute: None,
            },
        )]);
        let snapshot = EmptyCurtainSnapshot::from_inventory(
            InventorySnapshot {
                generation: 1,
                observed_at_unix_ms: 2,
                complete: true,
                characters: vec![InventoryCharacter {
                    uid: ItemUid { slot: 7, serial: 7 },
                    character_id: 7,
                }],
                items: vec![item],
            },
            &catalog,
            &characters,
            EmptyCurtainOperationState::default(),
        );

        assert!(!snapshot.complete);
        assert_eq!(snapshot.characters[0].name, "7");
        assert_eq!(snapshot.items[0].filter_id, "known");
        assert_eq!(snapshot.items[0].name, "known");
        assert!(snapshot.items[0].quality.is_none());
        assert!(snapshot.items[0].icon.is_none());
        assert!(snapshot.items[0].set_name.is_none());
        assert_eq!(snapshot.items[0].set_effects[0].text, "");
    }

    #[test]
    fn maximal_projected_inventory_stays_below_stream_delivery_budget() {
        let escape_heavy = "\u{0001}".repeat(EMPTY_CURTAIN_MAX_TEXT_BYTES);
        let inventory = InventorySnapshot {
            generation: u64::MAX,
            observed_at_unix_ms: u64::MAX,
            complete: true,
            characters: Vec::new(),
            items: (0..EMPTY_CURTAIN_MAX_ITEMS as u32)
                .map(|index| adversarial_item(index, &escape_heavy))
                .collect(),
        };

        let snapshot = EmptyCurtainSnapshot::from_inventory(
            inventory,
            &EquipmentCatalog::default(),
            &std::collections::HashMap::new(),
            EmptyCurtainOperationState::default(),
        );
        assert!(!snapshot.complete, "projection omissions must be explicit");
        assert!(snapshot.items.len() <= EMPTY_CURTAIN_MAX_ITEMS);
        assert!(
            snapshot
                .items
                .iter()
                .map(|item| item.stats.len() + item.set_effects.len())
                .sum::<usize>()
                <= EMPTY_CURTAIN_MAX_TOTAL_DETAILS
        );
        let delivery = serialize_stream_events(vec![EmptyCurtainEvent::Snapshot(snapshot)])
            .expect("the maximal legal equipment snapshot must serialize");
        println!(
            "EMPTY_CURTAIN_MAX_STREAM_BYTES={} limit={MAX_STREAM_DELIVERY_BYTES}",
            delivery.len()
        );
        assert!(
            delivery.len() < MAX_STREAM_DELIVERY_BYTES,
            "{} bytes exceeded the stream budget",
            delivery.len()
        );
    }
}
