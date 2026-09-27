//! Validated EQUIP/CARD read model. No packet replay or resource-derived account
//! ownership. Catalog curves resolve native base-modifier IDs at observed levels.
use crate::engine::{
    model::{
        EmptyCurtainCharacter, EmptyCurtainItem, EmptyCurtainPlacement, EquipmentStat, HtItemNetId,
    },
    parser::{EquipmentCatalog, EquipmentKind},
};
use serde::Deserialize;
use serde_json::Value;
use std::collections::{HashMap, HashSet};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Error {
    InvalidFormat,
    TooLarge,
    Changed,
    Incomplete,
    Unavailable,
}
type Result<T> = std::result::Result<T, Error>;
#[derive(Debug, Clone, PartialEq)]
pub struct Inventory {
    pub provider: String,
    pub domain: String,
    pub snapshot: String,
    pub observed_us: u64,
    pub items: Vec<EmptyCurtainItem>,
    pub characters: Vec<EmptyCurtainCharacter>,
}
fn field<'a>(v: &'a Value, key: &str) -> Result<&'a Value> {
    v.get(key).ok_or(Error::InvalidFormat)
}
fn text(v: &Value, key: &str, max: usize) -> Result<String> {
    let s = field(v, key)?.as_str().ok_or(Error::InvalidFormat)?;
    if s.is_empty() || s.len() > max {
        return Err(Error::InvalidFormat);
    }
    Ok(s.into())
}
fn array(v: &Value, max: usize) -> Result<&Vec<Value>> {
    v.as_array()
        .filter(|a| a.len() <= max)
        .ok_or(Error::TooLarge)
}
fn flag(v: &Value, key: &str, expected: bool) -> Result<()> {
    if field(v, key)?.as_bool() != Some(expected) {
        return Err(Error::Incomplete);
    }
    Ok(())
}
fn number(v: &Value, key: &str, max: u64) -> Result<u64> {
    field(v, key)?
        .as_u64()
        .filter(|n| *n <= max)
        .ok_or(Error::InvalidFormat)
}
fn uid(v: &Value) -> Result<HtItemNetId> {
    Ok(HtItemNetId {
        solt: number(v, "solt", u32::MAX.into())? as u32,
        serial: number(v, "serial", u32::MAX.into())? as u32,
    })
}
fn nonzero(v: &Value) -> Result<HtItemNetId> {
    let id = uid(v)?;
    if id.is_zero() {
        Err(Error::InvalidFormat)
    } else {
        Ok(id)
    }
}
pub fn metadata(v: &Value) -> Result<(String, String, String, usize)> {
    if field(v, "domain")? != "inventory" || field(v, "collectionScope")? != "EQUIP" {
        return Err(Error::InvalidFormat);
    }
    for k in [
        "ready",
        "enumerationComplete",
        "collectionComplete",
        "characterRefsComplete",
    ] {
        flag(v, k, true)?
    }
    for k in ["dirty", "failed", "truncated"] {
        flag(v, k, false)?
    }
    Ok((
        text(v, "providerId", 128)?,
        text(v, "domainKey", 1024)?,
        text(v, "snapshotId", 128)?,
        number(v, "recordCount", 4096)? as usize,
    ))
}
#[derive(Deserialize)]
struct Modifier {
    #[serde(rename = "PropName")]
    name: String,
    #[serde(rename = "PropValue")]
    value: f32,
}
pub fn project(meta: &Value, records: &[Value], catalog: &EquipmentCatalog) -> Result<Inventory> {
    let (provider, domain, snapshot, total) = metadata(meta)?;
    if records.len() != total {
        return Err(Error::Incomplete);
    }
    let observed_us = text(meta, "observedUnixUs", 20)?
        .parse()
        .map_err(|_| Error::InvalidFormat)?;
    let refs = array(field(meta, "characterRefs")?, 64)?;
    let mut characters = Vec::new();
    let mut owners = HashMap::new();
    let mut char_ids = HashSet::new();
    let mut slots: HashMap<
        HtItemNetId,
        (HtItemNetId, String, Option<EmptyCurtainPlacement>, bool),
    > = HashMap::new();
    for c in refs {
        if field(c, "kind")? != "HTCharacterItem" || field(c, "source")? != "InventoryItemsMap.CARD"
        {
            return Err(Error::InvalidFormat);
        }
        flag(c, "bIsTemporary", false)?;
        field(c, "bUnSaved")?
            .as_bool()
            .ok_or(Error::InvalidFormat)?;
        let id = nonzero(field(c, "UniqueID")?)?;
        let character_id = text(c, "ItemID", 16)?
            .parse::<u32>()
            .map_err(|_| Error::InvalidFormat)?;
        if character_id == 0 || !char_ids.insert(id) {
            return Err(Error::InvalidFormat);
        }
        characters.push(EmptyCurtainCharacter {
            net_id: id,
            character_id,
        });
        owners.insert(id, character_id);
        let mut groups = Vec::new();
        for g in array(field(c, "EquipmentSlots")?, 64)? {
            groups.push((array(field(g, "SlotList")?, 64)?, false));
        }
        groups.push((array(field(c, "EquipCoreSlots")?, 64)?, true));
        let mut cells = 0;
        for (group, core) in groups {
            for slot in group {
                cells += 1;
                if cells > 256 {
                    return Err(Error::TooLarge);
                }
                if number(slot, "State", 1)? != 1 {
                    return Err(Error::InvalidFormat);
                }
                let item = nonzero(field(slot, "EquipNetID")?)?;
                let item_id = text(slot, "EquipmentID", 128)?;
                let first = field(slot, "bFirstStep")?
                    .as_bool()
                    .ok_or(Error::InvalidFormat)?;
                let placement = if !core && first {
                    Some(EmptyCurtainPlacement {
                        row: number(slot, "Row", 5)? as i32,
                        column: number(slot, "Column", 5)? as i32,
                    })
                } else {
                    None
                };
                if placement.is_some_and(|p| p.row < 1 || p.column < 1) {
                    return Err(Error::InvalidFormat);
                }
                if let Some(previous) = slots.get_mut(&item) {
                    if previous.0 != id
                        || !previous.1.eq_ignore_ascii_case(&item_id)
                        || previous.3 != core
                        || (previous.2.is_some() && placement.is_some())
                    {
                        return Err(Error::InvalidFormat);
                    }
                    if placement.is_some() {
                        previous.2 = placement
                    }
                } else {
                    slots.insert(item, (id, item_id, placement, core));
                }
            }
        }
    }
    let mut seen = HashSet::new();
    let mut items = Vec::with_capacity(total);
    for row in records {
        if field(row, "kind")? != "HTEquipmentItem"
            || field(row, "source")? != "InventoryItemsMap"
            || field(row, "attributeStage")? != "inventory_raw"
            || field(row, "attributeUnits")? != "unknown"
            || number(row, "containerType", 255)? != 1
        {
            return Err(Error::InvalidFormat);
        }
        flag(row, "bIsTemporary", false)?;
        flag(row, "mapUidMatchesItem", true)?;
        flag(row, "duplicateMapUidObserved", false)?;
        let id = nonzero(field(row, "UniqueID")?)?;
        if nonzero(field(row, "mapUniqueID")?)? != id || !seen.insert(id) {
            return Err(Error::InvalidFormat);
        }
        let raw = text(row, "ItemID", 128)?;
        let mut matches = catalog
            .items
            .iter()
            .filter(|(k, _)| k.eq_ignore_ascii_case(&raw));
        let (item_id, definition) = matches.next().ok_or(Error::Unavailable)?;
        if matches.next().is_some() {
            return Err(Error::InvalidFormat);
        }
        let level = number(row, "StrengthenLevel", definition.max_level.into())? as u32;
        let mut main_stats = Vec::new();
        for prop in array(field(row, "RandomBaseModifyData")?, 8)? {
            let property = prop
                .as_str()
                .filter(|s| s.len() <= 128)
                .ok_or(Error::InvalidFormat)?;
            let value = catalog
                .main_stat_value(definition, property, level)
                .ok_or(Error::Unavailable)?;
            main_stats.push(EquipmentStat {
                property: property.into(),
                value,
            });
        }
        if main_stats.len() != definition.main_count {
            return Err(Error::Incomplete);
        }
        let mut sub_stats = Vec::new();
        for prop in array(field(row, "RandomModifyData")?, 16)? {
            let m: Modifier =
                serde_json::from_value(prop.clone()).map_err(|_| Error::InvalidFormat)?;
            if m.name.is_empty() || m.name.len() > 128 || !m.value.is_finite() {
                return Err(Error::InvalidFormat);
            }
            sub_stats.push(EquipmentStat {
                property: m.name,
                value: m.value,
            });
        }
        let owner = uid(field(row, "CharacterNetID")?)?;
        // IsEquiped is the base-item flag and is 0 even on observed equipped
        // HTEquipmentItem rows. Ownership requires matching CharacterNetID and
        // the permanent CARD slot references, not this unrelated base flag.
        number(row, "IsEquiped", u8::MAX.into())?;
        let placement = if !owner.is_zero() {
            let s = slots.get(&id).ok_or(Error::Incomplete)?;
            if s.0 != owner
                || !s.1.eq_ignore_ascii_case(item_id)
                || s.3 != (definition.kind == EquipmentKind::Core)
            {
                return Err(Error::Changed);
            }
            if !s.3 && s.2.is_none() {
                return Err(Error::Incomplete);
            }
            s.2
        } else {
            if slots.contains_key(&id) {
                return Err(Error::Changed);
            }
            None
        };
        items.push(EmptyCurtainItem {
            id,
            item_id: item_id.clone(),
            level,
            main_stats,
            sub_stats,
            locked: field(row, "IsLocked")?
                .as_bool()
                .ok_or(Error::InvalidFormat)?,
            discarded: field(row, "IsDiscarded")?
                .as_bool()
                .ok_or(Error::InvalidFormat)?,
            character_net_id: (!owner.is_zero()).then_some(owner),
            equipped_character_id: owners.get(&owner).copied(),
            equipped_placement: placement,
        });
    }
    if slots.keys().any(|id| !seen.contains(id)) {
        return Err(Error::Incomplete);
    }
    Ok(Inventory {
        provider,
        domain,
        snapshot,
        observed_us,
        items,
        characters,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    fn fixture() -> (Value, Vec<Value>, EquipmentCatalog) {
        let catalog = crate::engine::parser::load_equipment_catalog(std::path::Path::new(
            crate::engine::parser::EQUIPMENT_CATALOG_PATH,
        ))
        .unwrap();
        let meta = json!({"providerId":"fixture","domain":"inventory","domainKey":"fixture-session","snapshotId":"1","observedUnixUs":"1000000","ready":true,"collectionScope":"EQUIP","collectionComplete":true,"characterRefsComplete":true,"enumerationComplete":true,"dirty":false,"failed":false,"truncated":false,"recordCount":1,
            "characterRefs":[{"kind":"HTCharacterItem","source":"InventoryItemsMap.CARD","ItemID":"1051","UniqueID":{"solt":3,"serial":4},"bIsTemporary":false,"bUnSaved":true,"EquipmentSlots":[{"SlotList":[{"State":1,"EquipNetID":{"solt":1,"serial":2},"EquipmentID":"cell3_style6_1_Orange","bFirstStep":true,"Row":1,"Column":1}]}],"EquipCoreSlots":[]}]});
        let rows = vec![
            json!({"kind":"HTEquipmentItem","source":"InventoryItemsMap","attributeStage":"inventory_raw","attributeUnits":"unknown","containerType":1,"bIsTemporary":false,"mapUidMatchesItem":true,"duplicateMapUidObserved":false,"UniqueID":{"solt":1,"serial":2},"mapUniqueID":{"solt":1,"serial":2},"ItemID":"CELL3_STYLE6_1_ORANGE","StrengthenLevel":20,"RandomBaseModifyData":["AtkAdd","HPMaxAdd"],"RandomModifyData":[{"PropName":"CritBase","PropValue":0.03}],"CharacterNetID":{"solt":3,"serial":4},"IsEquiped":0,"IsLocked":true,"IsDiscarded":false}),
        ];
        (meta, rows, catalog)
    }
    #[test]
    fn native_slots_not_base_item_flag_determine_equipped_and_curves_match_legacy() {
        let (m, r, c) = fixture();
        let data = project(&m, &r, &c).unwrap();
        let item = &data.items[0];
        assert!(item.is_equipped());
        assert_eq!(item.equipped_character_id, Some(1051));
        assert_eq!(
            item.equipped_placement,
            Some(EmptyCurtainPlacement { row: 1, column: 1 })
        );
        assert_eq!(
            item.main_stats[0].value,
            c.main_stat_value(&c.items[&item.item_id], "AtkAdd", 20)
                .unwrap()
        );
        assert_eq!(item.sub_stats[0].value, 0.03);
        assert_eq!(item.item_id, "cell3_style6_1_Orange");
    }
    #[test]
    fn incomplete_stale_foreign_and_duplicate_rows_are_not_inventory() {
        let (m, r, c) = fixture();
        for key in ["dirty", "failed", "truncated"] {
            let mut bad = m.clone();
            bad[key] = json!(true);
            assert!(project(&bad, &r, &c).is_err());
        }
        let mut bad = m.clone();
        bad["collectionComplete"] = json!(false);
        assert!(project(&bad, &r, &c).is_err());
        let mut bad = r.clone();
        bad[0]["CharacterNetID"]["serial"] = json!(8);
        assert!(project(&m, &bad, &c).is_err());
        let mut bad = r.clone();
        bad[0]["mapUniqueID"]["solt"] = json!(9);
        assert!(project(&m, &bad, &c).is_err());
        let mut bad = m.clone();
        bad["recordCount"] = json!(2);
        assert!(project(&bad, &[r[0].clone(), r[0].clone()], &c).is_err());
        let mut bad = m.clone();
        bad["recordCount"] = json!(4097);
        assert!(metadata(&bad).is_err());
        let mut bad = r.clone();
        bad[0]["IsLocked"] = Value::Null;
        assert!(project(&m, &bad, &c).is_err());
        let mut bad = r.clone();
        bad[0]["RandomModifyData"][0]["PropValue"] = json!("0.03");
        assert!(project(&m, &bad, &c).is_err());
    }
}
