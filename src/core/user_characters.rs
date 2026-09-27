//! Read-only projection of User plugin schema 1. Never fills gaps from resources
//! or combat history. The native snapshot is a bounded, non-atomic observation.
use serde::Serialize;
use serde_json::Value;

pub const PAGE_SIZE: usize = 16;
pub const USER_CHARACTERS_CONTRACT_VERSION: u32 = 1;
const MAX_BYTES: usize = 32 * 1024 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SnapshotError {
    TooLarge,
    UnsupportedVersion,
    InvalidFormat,
    SessionChanged,
}
type Result<T> = std::result::Result<T, SnapshotError>;

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CharacterPage {
    pub snapshot_id: String,
    pub observed_unix_us: String,
    pub complete: bool,
    pub total: usize,
    pub offset: usize,
    pub records: Vec<CharacterSummary>,
    pub detail: Option<CharacterDetail>,
}
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CharacterSummary {
    pub uid: String,
    pub item_id: Option<String>,
    pub name: Option<String>,
    pub level: Option<String>,
}
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CharacterDetail {
    pub summary: CharacterSummary,
    pub sections: Vec<CharacterSection>,
}
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CharacterSection {
    pub title: &'static str,
    pub available: bool,
    pub entries: Vec<CharacterEntry>,
}
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CharacterEntry {
    pub name: Option<String>,
    pub fields: Vec<CharacterField>,
}
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CharacterField {
    pub label: String,
    pub value: Option<String>,
}

fn invalid<T>() -> Result<T> {
    Err(SnapshotError::InvalidFormat)
}
fn field<'a>(v: &'a Value, key: &str) -> Result<&'a Value> {
    v.as_object()
        .and_then(|o| o.get(key))
        .ok_or(SnapshotError::InvalidFormat)
}
fn text(v: &Value) -> Result<String> {
    match v.as_str() {
        Some(s) if s.len() <= 512 => Ok(s.to_owned()),
        _ => invalid(),
    }
}
fn scalar(v: &Value) -> Result<Option<String>> {
    match v {
        Value::Null => Ok(None),
        Value::Number(n) if n.as_f64().is_some_and(|v| v.is_finite()) => Ok(Some(n.to_string())),
        _ => invalid(),
    }
}
fn nullable_text(v: &Value) -> Result<Option<String>> {
    if v.is_null() {
        Ok(None)
    } else {
        text(v).map(Some)
    }
}
fn level(v: &Value) -> Result<Option<String>> {
    if v.is_null() {
        return Ok(None);
    }
    match v.as_i64() {
        Some(n) if (0..=i32::MAX as i64).contains(&n) => Ok(Some(n.to_string())),
        _ => invalid(),
    }
}
fn flag(v: &Value, key: &str) -> Result<bool> {
    field(v, key)?.as_bool().ok_or(SnapshotError::InvalidFormat)
}
pub fn decimal(v: &Value) -> Result<String> {
    let s = text(v)?;
    if s.is_empty()
        || s.parse::<u64>().is_err()
        || s.len() > 20
        || (s.len() > 1 && s.starts_with('0'))
    {
        return invalid();
    }
    Ok(s)
}
fn uid(v: &Value) -> Result<String> {
    let a = field(v, "solt")?
        .as_u64()
        .filter(|x| *x <= u32::MAX as u64)
        .ok_or(SnapshotError::InvalidFormat)?;
    let b = field(v, "serial")?
        .as_u64()
        .filter(|x| *x <= u32::MAX as u64)
        .ok_or(SnapshotError::InvalidFormat)?;
    let key = format!("{a}:{b}");
    if text(field(v, "key")?)? != key {
        return invalid();
    }
    Ok(key)
}
fn summary(v: &Value) -> Result<CharacterSummary> {
    if field(v, "ownership")? != "permanent_CARD_member" {
        return invalid();
    }
    Ok(CharacterSummary {
        uid: uid(field(v, "UniqueID")?)?,
        item_id: nullable_text(field(v, "ItemID")?)?,
        name: nullable_text(field(v, "displayName")?)?,
        level: level(field(v, "CharacterLevel")?)?,
    })
}
fn fields(v: &Value, keys: &[(&str, &str)]) -> Result<Vec<CharacterField>> {
    keys.iter()
        .map(|(key, label)| {
            Ok(CharacterField {
                label: (*label).into(),
                value: if matches!(
                    *key,
                    "CharacterLevel"
                        | "BreakthroughLevel"
                        | "AwakenLevel"
                        | "EXP"
                        | "StrengthenLevel"
                        | "StarLevel"
                        | "SkillLevel"
                        | "baseLevel"
                        | "awakenLevelDelta"
                        | "effectiveLevel"
                        | "slotNumber"
                ) {
                    level(field(v, key)?)?
                } else if *key == "slotState" {
                    scalar(field(v, key)?)?
                } else {
                    nullable_text(field(v, key)?)?
                },
            })
        })
        .collect()
}
fn entry(v: &Value, keys: &[(&str, &str)]) -> Result<CharacterEntry> {
    Ok(CharacterEntry {
        name: None,
        fields: fields(v, keys)?,
    })
}
fn list_section(
    title: &'static str,
    value: &Value,
    keys: &[(&str, &str)],
) -> Result<CharacterSection> {
    let entries = if value.is_null() {
        vec![]
    } else {
        value
            .as_array()
            .filter(|a| a.len() <= 256)
            .ok_or(SnapshotError::InvalidFormat)?
            .iter()
            .map(|v| {
                if v.is_null() {
                    Ok(CharacterEntry {
                        name: None,
                        fields: vec![CharacterField {
                            label: "Unavailable observation".into(),
                            value: None,
                        }],
                    })
                } else {
                    entry(v, keys)
                }
            })
            .collect::<Result<Vec<_>>>()?
    };
    Ok(CharacterSection {
        title,
        available: !value.is_null(),
        entries,
    })
}
fn equipment(title: &'static str, value: &Value) -> Result<CharacterSection> {
    let mut section = CharacterSection {
        title,
        available: !value.is_null(),
        entries: vec![],
    };
    if value.is_null() {
        return Ok(section);
    }
    let rows = value
        .as_array()
        .filter(|a| a.len() <= 256)
        .ok_or(SnapshotError::InvalidFormat)?;
    for row in rows {
        let mut e = entry(
            row,
            &[
                ("itemId", "Item ID"),
                ("slotState", "Slot state"),
                ("source", "Slot source"),
            ],
        )?;
        e.fields.push(CharacterField {
            label: "Equipment UID".into(),
            value: Some(uid(field(row, "uniqueId")?)?),
        });
        let item = field(row, "item")?;
        if item.is_null() {
            e.fields.push(CharacterField {
                label: "Equipment details".into(),
                value: None,
            });
        } else {
            e.name = nullable_text(field(item, "displayName")?)?;
            if field(item, "attributeStage")? != "inventory_raw"
                || field(item, "attributeUnits")? != "unknown"
            {
                return invalid();
            }
            e.fields.extend(fields(
                item,
                &[
                    ("StrengthenLevel", "Enhancement level"),
                    ("attributeStage", "Attribute stage"),
                    ("attributeUnits", "Attribute units"),
                ],
            )?);
            let base = field(item, "RandomBaseModifyData")?;
            if base.is_null() {
                e.fields.push(CharacterField {
                    label: "Base modifier ID".into(),
                    value: None,
                });
            } else {
                for id in base
                    .as_array()
                    .filter(|a| a.len() <= 64)
                    .ok_or(SnapshotError::InvalidFormat)?
                {
                    e.fields.push(CharacterField {
                        label: "Base modifier ID".into(),
                        value: nullable_text(id)?,
                    });
                }
            }
            let mods = field(item, "RandomModifyData")?;
            if mods.is_null() {
                e.fields.push(CharacterField {
                    label: "Equipment modifiers".into(),
                    value: None,
                });
            } else {
                for m in mods
                    .as_array()
                    .filter(|a| a.len() <= 64)
                    .ok_or(SnapshotError::InvalidFormat)?
                {
                    if m.is_null() {
                        e.fields.push(CharacterField {
                            label: "Equipment modifiers".into(),
                            value: None,
                        });
                    } else {
                        e.fields.push(CharacterField {
                            label: nullable_text(field(m, "PropName")?)?
                                .unwrap_or_else(|| "Unknown modifier".into()),
                            value: scalar(field(m, "PropValue")?)?,
                        });
                    }
                }
            }
        }
        section.entries.push(e);
    }
    Ok(section)
}
fn slots_known(value: &Value, grouped: bool) -> Result<bool> {
    if value.is_null() {
        return Ok(false);
    }
    let rows = value
        .as_array()
        .filter(|a| a.len() <= 256)
        .ok_or(SnapshotError::InvalidFormat)?;
    let mut known = true;
    for row in rows {
        if row.is_null() {
            known = false;
            continue;
        }
        if grouped {
            known &= slots_known(field(row, "SlotList")?, false)?;
        } else {
            let id = field(row, "EquipNetID")?;
            if id.is_null() {
                known = false;
            } else {
                uid(id)?;
            }
        }
    }
    Ok(known)
}
fn detail(v: &Value) -> Result<CharacterDetail> {
    let mut sections = vec![CharacterSection {
        title: "Character progression",
        available: true,
        entries: vec![entry(
            v,
            &[
                ("CharacterLevel", "Character level"),
                ("BreakthroughLevel", "Breakthrough level"),
                ("AwakenLevel", "Awakening level"),
                ("EXP", "Experience"),
            ],
        )?],
    }];
    let fork = field(v, "equippedFork")?;
    let mut fork_section = CharacterSection {
        title: "Arc",
        available: !fork.is_null(),
        entries: vec![],
    };
    if !fork.is_null() {
        let mut e = entry(
            fork,
            &[
                ("ItemID", "Item ID"),
                ("StrengthenLevel", "Enhancement level"),
                ("BreakthroughLevel", "Breakthrough level"),
                ("StarLevel", "Star level"),
            ],
        )?;
        e.name = nullable_text(field(fork, "displayName")?)?;
        fork_section.entries.push(e);
    } else {
        let id = field(v, "ForkItemNetId")?;
        fork_section.available = !id.is_null() && uid(id)? == "0:0";
    }
    sections.push(fork_section);
    let mut skills = list_section(
        "Skill levels",
        field(v, "SkillLevelData")?,
        &[
            ("SkillID", "Skill ID"),
            ("SkillLevel", "Saved level"),
            ("baseLevel", "Base level"),
            ("awakenLevelDelta", "Awakening bonus"),
            ("effectiveLevel", "Effective level"),
        ],
    )?;
    if let Some(rows) = field(v, "SkillLevelData")?.as_array() {
        for (row, entry) in rows.iter().zip(&mut skills.entries) {
            entry.name = match row.get("category").and_then(Value::as_str) {
                Some("normalAttack") => Some("Normal attack".into()),
                Some("skill") => Some("Variation skill".into()),
                Some("ultimate") => Some("Ultimate finale".into()),
                Some("assist") => Some("Assist skill".into()),
                Some(_) => return invalid(),
                None => None,
            };
        }
    }
    sections.push(skills);
    let mut awaken = list_section(
        "Active awakenings",
        field(v, "activeAwakeningEffects")?,
        &[
            ("slotNumber", "Awakening slot"),
            ("effectId", "Effect ID"),
            ("effectName", "Effect name"),
        ],
    )?;
    if let Some(rows) = field(v, "activeAwakeningEffects")?.as_array() {
        for (row, entry) in rows.iter().zip(&mut awaken.entries) {
            let number = match row.get("definitionNumber") {
                None | Some(Value::Null) => None,
                Some(v) => {
                    let n = v
                        .as_u64()
                        .filter(|n| (1..=64).contains(n))
                        .ok_or(SnapshotError::InvalidFormat)?;
                    Some(n.to_string())
                }
            };
            entry.fields.push(CharacterField {
                label: "Awakening number".into(),
                value: number,
            });
        }
        awaken.entries.sort_by_key(|e| {
            e.fields
                .iter()
                .find(|f| f.label == "Awakening number")
                .and_then(|f| f.value.as_ref())
                .and_then(|s| s.parse::<u32>().ok())
                .unwrap_or(u32::MAX)
        });
    }
    // Partial selection is never presented as a complete list of active effects.
    awaken.available &= flag(v, "awakeningSelectionComplete")?;
    sections.push(awaken);
    let mut cassettes = equipment("Cassettes", field(v, "equippedCassettes")?)?;
    cassettes.available &= slots_known(field(v, "EquipCoreSlots")?, false)?;
    sections.push(cassettes);
    let mut blocks = equipment("Drive blocks", field(v, "equippedDriveBlocks")?)?;
    blocks.available &= slots_known(field(v, "EquipmentSlots")?, true)?;
    sections.push(blocks);
    let runtime = field(v, "runtimeAttributes")?;
    let mut attributes = CharacterSection {
        title: "Runtime attributes",
        available: false,
        entries: vec![],
    };
    if !runtime.is_null() {
        let account = field(runtime, "source")? == "account_character_info";
        if !account && field(runtime, "source")? != "equipped_actor_uid" {
            return invalid();
        }
        let observed = decimal(field(runtime, "observedUnixUs")?)?;
        let values = field(runtime, "values")?
            .as_array()
            .filter(|a| a.len() <= 32)
            .ok_or(SnapshotError::InvalidFormat)?;
        let mut fields = vec![];
        let mut seen = std::collections::HashSet::new();
        for value in values {
            let key = text(field(value, "key")?)?;
            let (label, unit) = match key.as_str() {
                "maxHp" => ("HPMaxBase", "number"),
                "attack" => ("AtkBase", "number"),
                "defense" => ("DefBase", "number"),
                "crit" => ("CritBase", "ratio"),
                "critDamage" => ("CritDamageBase", "ratio"),
                "damageUpGeneral" => ("DamageUpGeneralBase", "ratio"),
                "chargeEfficiency" => ("ChargeGetEfficiencyBase", "ratio"),
                _ => return invalid(),
            };
            if !seen.insert(key) || field(value, "unit")? != unit {
                return invalid();
            }
            let number = scalar(field(value, "value")?)?;
            attributes.available |= number.is_some();
            fields.push(CharacterField {
                label: label.into(),
                value: number,
            });
        }
        fields.push(CharacterField {
            label: "Observed Unix microseconds".into(),
            value: Some(observed),
        });
        attributes.entries.push(CharacterEntry {
            name: Some(
                if account {
                    "Account configuration attributes"
                } else {
                    "Live team actor snapshot"
                }
                .into(),
            ),
            fields,
        });
    }
    sections.push(attributes);
    Ok(CharacterDetail {
        summary: summary(v)?,
        sections,
    })
}

// Bound ignored fields too, before projection. serde_json also enforces its
// recursion limit while parsing; payload bytes are bounded before allocation.
fn budget(v: &Value, depth: usize, nodes: &mut usize) -> Result<()> {
    *nodes += 1;
    if depth > 24 || *nodes > 500_000 {
        return Err(SnapshotError::TooLarge);
    }
    match v {
        Value::String(s) if s.len() > 4096 => return Err(SnapshotError::TooLarge),
        Value::Array(a) => {
            if a.len() > 16_384 {
                return Err(SnapshotError::TooLarge);
            }
            for x in a {
                budget(x, depth + 1, nodes)?;
            }
        }
        Value::Object(o) => {
            if o.len() > 128 || o.keys().any(|k| k.len() > 128) {
                return Err(SnapshotError::TooLarge);
            }
            for x in o.values() {
                budget(x, depth + 1, nodes)?;
            }
        }
        _ => {}
    }
    Ok(())
}
pub fn project(
    bytes: &[u8],
    identity: &str,
    snapshot_id: &str,
    offset: usize,
    selected: Option<&str>,
    query: &str,
) -> Result<CharacterPage> {
    if bytes.len() > MAX_BYTES || query.len() > 128 {
        return Err(SnapshotError::TooLarge);
    }
    let v: Value = serde_json::from_slice(bytes).map_err(|_| SnapshotError::InvalidFormat)?;
    budget(&v, 0, &mut 0)?;
    if field(&v, "schemaVersion")? != 1 {
        return Err(SnapshotError::UnsupportedVersion);
    }
    let pid = field(&v, "processId")?
        .as_u64()
        .filter(|n| *n > 0 && *n <= u32::MAX as u64)
        .ok_or(SnapshotError::InvalidFormat)?;
    if format!("{}:{}", pid, decimal(field(&v, "processCreatedFileTime")?)?) != identity
        || text(field(&v, "snapshotId")?)? != snapshot_id
    {
        return Err(SnapshotError::SessionChanged);
    }
    if field(&v, "consistency")? != "bounded_game_thread_collection_not_atomic" {
        return invalid();
    }
    let domain = field(&v, "ownedCharacters")?;
    if field(domain, "sourceCoverage")? != "client_observation_only" {
        return invalid();
    }
    let records = field(domain, "records")?
        .as_array()
        .filter(|a| a.len() <= 2048)
        .ok_or(SnapshotError::TooLarge)?;
    if field(domain, "recordCount")?.as_u64() != Some(records.len() as u64) {
        return invalid();
    }
    let collected = flag(domain, "collectionComplete")?;
    let enumerated = flag(domain, "enumerationComplete")?;
    let failed = flag(domain, "failed")?;
    let truncated = flag(domain, "truncated")?;
    let complete = collected && enumerated && !failed && !truncated;
    let mut seen = std::collections::HashSet::new();
    let mut filtered = vec![];
    let query = query.to_lowercase();
    for record in records {
        let row = summary(record)?;
        if !seen.insert(row.uid.clone()) {
            return invalid();
        }
        if query.is_empty()
            || [
                &row.uid,
                row.item_id.as_deref().unwrap_or(""),
                row.name.as_deref().unwrap_or(""),
            ]
            .iter()
            .any(|s| s.to_lowercase().contains(&query))
        {
            filtered.push((row, record));
        }
    }
    let total = filtered.len();
    if (offset >= total && offset != 0) || !offset.is_multiple_of(PAGE_SIZE) {
        return invalid();
    }
    let end = (offset + PAGE_SIZE).min(total);
    let selected_record = if let Some(key) = selected {
        filtered
            .iter()
            .find(|(s, _)| s.uid == key)
            .map(|(_, v)| *v)
            .ok_or(SnapshotError::InvalidFormat)
            .map(Some)?
    } else {
        filtered.get(offset).map(|(_, v)| *v)
    };
    let detail = selected_record.map(detail).transpose()?;
    let page_records = filtered
        .into_iter()
        .skip(offset)
        .take(end - offset)
        .map(|(s, _)| s)
        .collect();
    Ok(CharacterPage {
        snapshot_id: snapshot_id.into(),
        observed_unix_us: decimal(field(&v, "observedUnixUs")?)?,
        complete,
        total,
        offset,
        records: page_records,
        detail,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    fn fixture() -> Value {
        serde_json::from_str(
            r#"{
          "schemaVersion": 1,
          "snapshotId": "fixture:1",
          "providerId": "fixture",
          "processId": 42,
          "processCreatedFileTime": "99",
          "observedUnixUs": "1790000000000000",
          "consistency": "bounded_game_thread_collection_not_atomic",
          "executablePath": "PRIVATE_PATH_NOT_FOR_UI",
          "ownedCharacters": {
            "collectionComplete": true,
            "enumerationComplete": true,
            "failed": false,
            "truncated": false,
            "sourceCoverage": "client_observation_only",
            "recordCount": 1,
            "records": [
              {
                "UniqueID": {
                  "solt": 1,
                  "serial": 2,
                  "key": "1:2"
                },
                "ItemID": "fixture_character",
                "displayName": "Synthetic character",
                "ownership": "permanent_CARD_member",
                "CharacterLevel": 60,
                "BreakthroughLevel": 4,
                "AwakenLevel": 2,
                "EXP": 0,
                "ForkItemNetId": {
                  "solt": 2,
                  "serial": 3,
                  "key": "2:3"
                },
                "equippedFork": {
                  "ItemID": "fixture_arc",
                  "displayName": "Synthetic arc",
                  "StrengthenLevel": 50,
                  "BreakthroughLevel": 3,
                  "StarLevel": 1
                },
                "SkillLevelData": [
                  {
                    "SkillID": "fixture_skill",
                    "SkillLevel": 7,
                    "baseLevel": 7,
                    "awakenLevelDelta": 2,
                    "effectiveLevel": 9
                  }
                ],
                "activeAwakeningEffects": [
                  {
                    "slotNumber": 2,
                    "effectId": "fixture_effect_99",
                    "effectName": "Synthetic awakening"
                  }
                ],
                "awakeningSelectionComplete": true,
                "EquipCoreSlots": [],
                "EquipmentSlots": [],
                "equippedCassettes": [],
                "equippedDriveBlocks": [
                  {
                    "uniqueId": {
                      "solt": 3,
                      "serial": 4,
                      "key": "3:4"
                    },
                    "itemId": "fixture_drive",
                    "slotState": 1,
                    "source": "EquipmentSlots",
                    "item": {
                      "displayName": "Synthetic drive block",
                      "StrengthenLevel": 12,
                      "RandomBaseModifyData": [
                        "fixture_base_modifier"
                      ],
                      "RandomModifyData": [
                        {
                          "PropName": "fixture_attack",
                          "PropValue": 0
                        },
                        {
                          "PropName": "fixture_rate",
                          "PropValue": 0.123456789
                        }
                      ],
                      "attributeStage": "inventory_raw",
                      "attributeUnits": "unknown"
                    }
                  }
                ],
                "runtimeAttributes": null
              }
            ]
          },
          "inventory": {
            "records": []
          }
        }"#,
        )
        .unwrap()
    }
    fn read(v: &Value) -> Result<CharacterPage> {
        project(
            &serde_json::to_vec(v).unwrap(),
            "42:99",
            "fixture:1",
            0,
            None,
            "",
        )
    }
    #[test]
    fn projects_observed_progression_equipment_and_unknown_runtime_without_private_paths() {
        let page = read(&fixture()).unwrap();
        let output = serde_json::to_value(&page).unwrap();
        assert_eq!(
            output["detail"]["sections"][2]["entries"][0]["fields"][4]["value"],
            "9"
        );
        assert_eq!(
            output["detail"]["sections"][3]["entries"][0]["fields"][0]["value"],
            "2"
        );
        let text = output.to_string();
        assert!(text.contains("fixture_attack"));
        assert!(text.contains("0.123456789"));
        assert!(!text.contains("PRIVATE_PATH"));
        assert!(!text.contains("objectKey"));
        assert_eq!(output["detail"]["sections"][6]["available"], false);
        assert_eq!(page.records[0].level.as_deref(), Some("60"));
    }
    #[test]
    fn missing_values_stay_unknown_and_zero_is_a_reading() {
        let mut v = fixture();
        let c = &mut v["ownedCharacters"]["records"][0];
        c["CharacterLevel"] = Value::Null;
        c["equippedFork"] = Value::Null;
        c["activeAwakeningEffects"] = Value::Null;
        c["awakeningSelectionComplete"] = json!(false);
        c["EquipmentSlots"] = Value::Null;
        c["equippedDriveBlocks"] = json!([]);
        let p = read(&v).unwrap();
        let d = p.detail.unwrap();
        assert!(p.records[0].level.is_none());
        assert!(!d.sections[1].available);
        assert!(!d.sections[3].available);
        assert!(!d.sections[5].available);
        assert_eq!(
            d.sections[0].entries[0].fields[3].value.as_deref(),
            Some("0")
        );
        v["ownedCharacters"]["records"][0]["ForkItemNetId"] =
            json!({"solt":0,"serial":0,"key":"0:0"});
        assert!(read(&v).unwrap().detail.unwrap().sections[1].available);
    }
    #[test]
    fn paginates_filters_and_selects_by_exact_uid_without_preview_truncation() {
        let mut v = fixture();
        let source = v["ownedCharacters"]["records"][0].clone();
        let records = (0..35)
            .map(|i| {
                let mut r = source.clone();
                r["UniqueID"] = json!({"solt":1,"serial":i,"key":format!("1:{i}")});
                r["displayName"] = json!(format!("Fixture {i}"));
                r
            })
            .collect::<Vec<_>>();
        v["ownedCharacters"]["records"] = json!(records);
        v["ownedCharacters"]["recordCount"] = json!(35);
        let bytes = serde_json::to_vec(&v).unwrap();
        assert!(bytes.len() > 8192);
        let page = project(&bytes, "42:99", "fixture:1", 16, Some("1:20"), "").unwrap();
        assert_eq!(page.total, 35);
        assert_eq!(page.records.len(), 16);
        assert_eq!(page.detail.unwrap().summary.uid, "1:20");
        let page = project(&bytes, "42:99", "fixture:1", 0, None, "fixture 34").unwrap();
        assert_eq!(page.total, 1);
        let page = project(&bytes, "42:99", "fixture:1", 0, None, "absent").unwrap();
        assert_eq!(page.total, 0);
        assert!(page.detail.is_none());
        assert!(project(&bytes, "42:99", "fixture:1", 48, None, "").is_err());
    }
    #[test]
    fn rejects_wrong_version_identity_uid_and_malformed_required_fields() {
        let mut v = fixture();
        v["schemaVersion"] = json!(2);
        assert_eq!(read(&v).unwrap_err(), SnapshotError::UnsupportedVersion);
        let mut v = fixture();
        v["processCreatedFileTime"] = json!("100");
        assert_eq!(read(&v).unwrap_err(), SnapshotError::SessionChanged);
        for key in [
            "SkillLevelData",
            "equippedFork",
            "runtimeAttributes",
            "CharacterLevel",
        ] {
            let mut v = fixture();
            v["ownedCharacters"]["records"][0]
                .as_object_mut()
                .unwrap()
                .remove(key);
            assert!(read(&v).is_err(), "{key}");
        }
        let mut v = fixture();
        v["ownedCharacters"]["records"][0]["CharacterLevel"] = json!(-1);
        assert!(read(&v).is_err());
        let mut v = fixture();
        let c = v["ownedCharacters"]["records"][0].clone();
        v["ownedCharacters"]["records"]
            .as_array_mut()
            .unwrap()
            .push(c);
        v["ownedCharacters"]["recordCount"] = json!(2);
        assert!(read(&v).is_err());
        assert!(project(b"{", "42:99", "fixture:1", 0, None, "").is_err());
    }
    #[test]
    fn bounds_payload_nesting_rows_and_strings_and_recovers_after_failure() {
        assert_eq!(
            project(
                &vec![b' '; MAX_BYTES + 1],
                "42:99",
                "fixture:1",
                0,
                None,
                ""
            )
            .unwrap_err(),
            SnapshotError::TooLarge
        );
        let mut v = fixture();
        v["ownedCharacters"]["records"][0]["displayName"] = json!("x".repeat(513));
        assert!(read(&v).is_err());
        let mut v = fixture();
        let mut deep = Value::Null;
        for _ in 0..26 {
            deep = json!([deep]);
        }
        v["inventory"] = deep;
        assert_eq!(read(&v).unwrap_err(), SnapshotError::TooLarge);
        let mut v = fixture();
        v["ownedCharacters"]["records"] = json!(vec![Value::Null; 2049]);
        assert_eq!(read(&v).unwrap_err(), SnapshotError::TooLarge);
        assert!(read(&fixture()).is_ok());
    }
    #[test]
    fn reports_partial_coverage_and_validates_all_flags() {
        let mut v = fixture();
        v["ownedCharacters"]["collectionComplete"] = json!(false);
        assert!(!read(&v).unwrap().complete);
        v["ownedCharacters"]
            .as_object_mut()
            .unwrap()
            .remove("failed");
        assert!(read(&v).is_err());
    }
    #[test]
    fn unreadable_slot_entries_are_not_empty_loadouts() {
        let mut v = fixture();
        v["ownedCharacters"]["records"][0]["EquipCoreSlots"] = json!([null]);
        v["ownedCharacters"]["records"][0]["EquipmentSlots"] = json!([{"SlotList":null}]);
        let d = read(&v).unwrap().detail.unwrap();
        assert!(!d.sections[4].available);
        assert!(!d.sections[5].available);
    }
    #[test]
    fn projects_live_runtime_values_units_and_semantic_skill_names() {
        let mut v = fixture();
        let c = &mut v["ownedCharacters"]["records"][0];
        c["SkillLevelData"][0]["category"] = json!("skill");
        c["runtimeAttributes"] = json!({"source":"equipped_actor_uid","observedUnixUs":"1790000000000000","values":[
            {"key":"attack","value":2122.345,"unit":"number"},
            {"key":"crit","value":0.91,"unit":"ratio"},
            {"key":"maxHp","value":0,"unit":"number"}
        ]});
        let d = read(&v).unwrap().detail.unwrap();
        assert_eq!(
            d.sections[2].entries[0].name.as_deref(),
            Some("Variation skill")
        );
        assert!(d.sections[6].available);
        assert_eq!(
            d.sections[6].entries[0].fields[2].value.as_deref(),
            Some("0")
        );
        v["ownedCharacters"]["records"][0]["runtimeAttributes"]["values"][1]["unit"] =
            json!("percent");
        assert!(read(&v).is_err());
        v["ownedCharacters"]["records"][0]["runtimeAttributes"]["values"][1]["unit"] =
            json!("ratio");
        v["ownedCharacters"]["records"][0]["runtimeAttributes"]["source"] = json!("historical_hit");
        assert!(read(&v).is_err());
    }
    #[test]
    fn awakening_numbers_follow_definition_order_not_selected_slots() {
        let mut v = fixture();
        v["ownedCharacters"]["records"][0]["activeAwakeningEffects"] = json!([
            {"slotNumber":1,"definitionNumber":5,"effectId":"not_numeric_a","effectName":"Fifth"},
            {"slotNumber":2,"definitionNumber":3,"effectId":"not_numeric_b","effectName":"Third"}
        ]);
        let p = read(&v).unwrap();
        let entries = &p.detail.unwrap().sections[3].entries;
        assert_eq!(entries[0].fields[0].value.as_deref(), Some("2"));
        assert_eq!(entries[0].fields[3].value.as_deref(), Some("3"));
        assert_eq!(entries[1].fields[3].value.as_deref(), Some("5"));
        v["ownedCharacters"]["records"][0]["activeAwakeningEffects"][0]["definitionNumber"] =
            json!(0);
        assert!(read(&v).is_err());
    }
    #[test]
    fn accepts_account_calculator_values_including_charge_efficiency() {
        let mut v = fixture();
        v["ownedCharacters"]["records"][0]["runtimeAttributes"] = json!({"source":"account_character_info","observedUnixUs":"1790000000000000","values":[{"key":"chargeEfficiency","unit":"ratio","value":1.0}]});
        let d = read(&v).unwrap().detail.unwrap();
        assert_eq!(
            d.sections[6].entries[0].name.as_deref(),
            Some("Account configuration attributes")
        );
        assert_eq!(
            d.sections[6].entries[0].fields[0].label,
            "ChargeGetEfficiencyBase"
        );
    }
}
