use serde::Serialize;

use nte_dps_tool::{
    core::empty_curtain::substat_unlock_level,
    core::snapshot::{InventoryItem, InventorySnapshot, InventoryStat, ItemUid},
    engine::parser::EquipmentCatalog,
    storage::i18n::{self, Language},
};

use crate::equipment_operation_service::EmptyCurtainOperationState;

pub(crate) const EMPTY_CURTAIN_CONTRACT_VERSION: u32 = 2;

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
        Self {
            contract_version: EMPTY_CURTAIN_CONTRACT_VERSION,
            generation: inventory.generation.to_string(),
            observed_at_unix_ms: inventory.observed_at_unix_ms.to_string(),
            has_data: !inventory.items.is_empty(),
            complete: inventory.complete,
            characters: inventory
                .characters
                .into_iter()
                .map(|character| EmptyCurtainCharacterSnapshot {
                    uid: character.uid.into(),
                    character_id: character.character_id,
                    name: characters
                        .get(&character.character_id)
                        .map(|definition| match language {
                            Language::English | Language::Japanese => definition.name_en.as_str(),
                            Language::SimplifiedChinese => definition.name_zh.as_str(),
                        })
                        .filter(|name| !name.is_empty())
                        .map(str::to_owned)
                        .unwrap_or_else(|| character.character_id.to_string()),
                })
                .collect(),
            items: inventory
                .items
                .into_iter()
                .map(|item| item_snapshot(item, catalog, language))
                .collect(),
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
            message_arguments: value.message_arguments,
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
) -> EmptyCurtainItemSnapshot {
    let definition = catalog.items.get(&item.item_id);
    let suit = item
        .suit_id
        .as_deref()
        .and_then(|suit_id| catalog.suits.get(suit_id));
    let mut stats = item
        .main_stats
        .into_iter()
        .map(|stat| stat_snapshot(stat, language, true, None, true))
        .collect::<Vec<_>>();
    let definition_substat_count = definition.map_or(item.sub_stats.len(), |definition| {
        definition.sub_count.max(item.sub_stats.len())
    });
    stats.extend(item.sub_stats.into_iter().enumerate().map(|(index, stat)| {
        let unlock_level = substat_unlock_level(
            definition.map_or(item.max_level.unwrap_or(20), |definition| {
                definition.max_level
            }),
            definition_substat_count,
            index,
        );
        stat_snapshot(
            stat,
            language,
            false,
            unlock_level,
            unlock_level.is_none_or(|required| item.level >= required),
        )
    }));
    EmptyCurtainItemSnapshot {
        uid: item.uid.into(),
        item_id: item.item_id.clone(),
        filter_id: definition
            .map(|definition| match definition.kind {
                nte_dps_tool::engine::parser::EquipmentKind::Module => definition.geometry.clone(),
                nte_dps_tool::engine::parser::EquipmentKind::Core => definition
                    .suit
                    .clone()
                    .unwrap_or_else(|| item.item_id.clone()),
            })
            .unwrap_or_else(|| item.item_id.clone()),
        kind: item.kind,
        quality: item.quality,
        name: item
            .names
            .as_ref()
            .map(|names| localized_names(names, language))
            .unwrap_or_else(|| item.item_id.clone()),
        icon: definition.map(|definition| definition.icon.clone()),
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
        set_name: suit.map(|suit| suit.name(language).to_owned()),
        set_effects: suit
            .map(|suit| {
                suit.effects
                    .iter()
                    .map(|effect| EmptyCurtainSetEffectSnapshot {
                        count: effect.count,
                        text: effect.text(language).to_owned(),
                    })
                    .collect()
            })
            .unwrap_or_default(),
    }
}

fn stat_snapshot(
    stat: InventoryStat,
    language: Language,
    main: bool,
    unlock_level: Option<u32>,
    unlocked: bool,
) -> EmptyCurtainStatSnapshot {
    EmptyCurtainStatSnapshot {
        property: stat.property_id.clone(),
        label: stat
            .names
            .as_ref()
            .map(|names| localized_names(names, language))
            .unwrap_or(stat.property_id),
        value: stat.value,
        percent: stat.percent.unwrap_or(false),
        main,
        unlock_level,
        unlocked,
    }
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
}
