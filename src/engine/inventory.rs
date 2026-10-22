//! Exact login inventory RPCs, bound to the captured PlayerController archetype.
//! Never scans arbitrary bit offsets. Unsupported updates stay visibly unavailable.
use super::{
    model::{
        EmptyCurtainCharacter, EmptyCurtainItem, EmptyCurtainPlacement, EquipmentStat, HtItemNetId,
    },
    parser::{EquipmentCatalog, EquipmentKind},
    settlement::{
        Error, Name,
        transport::{Decoder, Rpc},
        wire::Bits,
    },
};
use serde::Deserialize;
use serde_json::{Value, json};
use std::collections::{BTreeMap, HashMap};
use std::sync::{Arc, OnceLock};
#[derive(Deserialize)]
struct Field {
    name: String,
    kind: String,
}
fn schema() -> &'static BTreeMap<String, Vec<Field>> {
    static SCHEMA: OnceLock<BTreeMap<String, Vec<Field>>> = OnceLock::new();
    SCHEMA.get_or_init(|| {
        serde_json::from_str(include_str!("inventory/schema.json"))
            .expect("compiled inventory schema")
    })
}
fn reference(r: &mut Bits<'_>) -> Result<u64, Error> {
    reference_at(r, 0)
}
fn reference_at(r: &mut Bits<'_>, depth: usize) -> Result<u64, Error> {
    if depth > 16 {
        return Err(Error::BudgetExceeded);
    }
    let mut out = 0u64;
    for i in 0..10 {
        let b = r.take(8)?;
        if i == 9 && b >> 1 > 1 {
            return Err(Error::InvalidValue);
        }
        out |= (b >> 1) << (i * 7);
        if b & 1 == 0 {
            if out == 1 {
                let flags = r.take(8)?;
                if flags & 1 != 0 {
                    reference_at(r, depth + 1)?;
                    r.string()?;
                    if flags & 4 != 0 {
                        r.take(32)?;
                    }
                }
            }
            return Ok(out);
        }
    }
    Err(Error::InvalidValue)
}

fn name(r: &mut Bits<'_>) -> Result<Value, Error> {
    match r.name()? {
        Name::Hardcoded(0) => Ok(json!("None")),
        Name::Hardcoded(index) => Ok(json!({"hardcodedIndex":index})),
        Name::Text { text, number: 0 } => Ok(json!(text)),
        Name::Text { text, number } => Ok(json!({"text":text,"number":number})),
    }
}
fn value(r: &mut Bits<'_>, kind: &str, depth: usize, nodes: &mut usize) -> Result<Value, Error> {
    *nodes += 1;
    if depth > 24 || *nodes > 100000 {
        return Err(Error::BudgetExceeded);
    }
    let kind = kind
        .strip_prefix("struct ")
        .or_else(|| kind.strip_prefix("class "))
        .unwrap_or(kind);
    if let Some(inner) = kind
        .strip_prefix("TArray<")
        .and_then(|s| s.strip_suffix('>'))
    {
        let count = r.take(16)? as usize;
        if count > 4096 || count > r.len - r.pos {
            return Err(Error::BudgetExceeded);
        }
        return (0..count)
            .map(|_| value(r, inner, depth + 1, nodes))
            .collect::<Result<Vec<_>, _>>()
            .map(Value::Array);
    }
    Ok(match kind {
        "FName" => return name(r),
        "FString" => json!(r.string()?),
        "bool" => json!(r.take(1)? != 0),
        "int32" => json!(r.take(32)? as u32 as i32),
        "uint32" => json!(r.take(32)?),
        "int64" => json!(r.take(64)? as i64),
        "uint64" => json!(r.take(64)?),
        "float" => {
            let v = f32::from_bits(r.take(32)? as u32);
            if !v.is_finite() {
                return Err(Error::InvalidValue);
            }
            json!(v)
        }
        "T1ByteEnum<ESkillInputIDType>" => json!(r.take(8)?),
        k if k.starts_with("TSubclassOf<") => json!(reference(r)?),
        k => {
            let fields = schema().get(k).ok_or(Error::Unsupported)?;
            let mut result = serde_json::Map::new();
            for field in fields {
                result.insert(field.name.clone(), value(r, &field.kind, depth + 1, nodes)?);
            }
            Value::Object(result)
        }
    })
}
struct Notification {
    inventory: u64,
    kind: u8,
    items: Vec<Value>,
}
fn notification(data: &[u8], bits: usize) -> Result<Notification, Error> {
    let mut r = Bits::new(data, bits)?;
    if r.take(1)? == 0 {
        return Err(Error::Unsupported);
    }
    let inventory = reference(&mut r)?;
    if inventory <= 1 {
        return Err(Error::InvalidValue);
    }
    if r.take(1)? == 0 {
        return Err(Error::Unsupported);
    }
    let kind = r.take(7)? as u8;
    if kind > 75 {
        return Err(Error::InvalidValue);
    }
    if kind != 3 {
        return Ok(Notification {
            inventory,
            kind,
            items: vec![],
        });
    }
    if r.take(1)? == 0 {
        return Err(Error::Unsupported);
    }
    let count = r.take(16)?;
    if count > 4096 {
        return Err(Error::BudgetExceeded);
    }
    let mut items = Vec::new();
    let mut nodes = 0;
    for _ in 0..count {
        items.push(value(&mut r, "FHTItemNetInfo", 0, &mut nodes)?);
    }
    if r.take(1)? != 0 {
        // The reviewed login notification contains one opaque enum argument.
        // Its integer is not treated as a container identifier or completion flag.
        let count = r.take(32)?;
        if count != 1 {
            return Err(Error::Unsupported);
        }
        if r.take(8)? != 31 || r.take(32)? != 4 {
            return Err(Error::Unsupported);
        }
        r.take(32)?;
    }
    if r.pos != r.len {
        return Err(Error::InvalidValue);
    }
    Ok(Notification {
        inventory,
        kind,
        items,
    })
}
fn text<'a>(v: &'a Value, key: &str) -> Result<&'a str, Error> {
    v[key]
        .as_str()
        .filter(|s| s.len() <= 512)
        .ok_or(Error::InvalidValue)
}
fn uid(v: &Value) -> Result<HtItemNetId, Error> {
    Ok(HtItemNetId {
        solt: v["solt"]
            .as_u64()
            .and_then(|n| n.try_into().ok())
            .ok_or(Error::InvalidValue)?,
        serial: v["serial"]
            .as_u64()
            .and_then(|n| n.try_into().ok())
            .ok_or(Error::InvalidValue)?,
    })
}
fn array<'a>(v: &'a Value, key: &str) -> Result<&'a Vec<Value>, Error> {
    v[key].as_array().ok_or(Error::InvalidValue)
}
#[derive(Default, Clone)]
struct Observed {
    items: BTreeMap<(u32, u32), Value>,
    characters: BTreeMap<(u32, u32), Value>,
    budget: usize,
}
fn retained_cost(value: &Value) -> usize {
    match value {
        Value::String(s) => 128 + s.len() * 2,
        Value::Array(a) => 128 + a.iter().map(retained_cost).sum::<usize>(),
        Value::Object(o) => {
            128 + o
                .iter()
                .map(|(k, v)| 128 + k.len() * 2 + retained_cost(v))
                .sum::<usize>()
        }
        _ => 128,
    }
}
impl Observed {
    fn apply(&mut self, n: &Notification) -> Result<(), Error> {
        if n.kind != 3 {
            return Err(Error::Unsupported);
        }
        let mut gear = Vec::new();
        let mut characters = Vec::new();
        let mut seen = std::collections::HashSet::new();
        for row in &n.items {
            if row["bIsTemporary"].as_bool() != Some(false) {
                continue;
            }
            let key = uid(&row["UniqueID"])?;
            if key.is_zero() || !seen.insert(key) {
                return Err(Error::InvalidValue);
            }
            let eq = array(row, "EquipmentNetInfo")?.len();
            let ch = array(row, "CharacterNetInfo")?.len();
            if eq > 1 || ch > 1 || (eq > 0 && ch > 0) {
                return Err(Error::Unsupported);
            }
            if (eq == 1 || ch == 1) && row["Amount"] != 1 {
                return Err(Error::InvalidValue);
            }
            if eq == 1 {
                gear.push(((key.solt, key.serial), row.clone()));
            }
            if ch == 1 {
                characters.push(((key.solt, key.serial), row.clone()));
            }
        }
        if self.items.len()
            + gear
                .iter()
                .filter(|(id, _)| !self.items.contains_key(id))
                .count()
            > 4096
            || self.characters.len()
                + characters
                    .iter()
                    .filter(|(id, _)| !self.characters.contains_key(id))
                    .count()
                > 64
        {
            return Err(Error::BudgetExceeded);
        }
        let mut budget = self.budget;
        for (target, changes) in [(&self.items, &gear), (&self.characters, &characters)] {
            for (id, value) in changes {
                budget = budget
                    .saturating_sub(target.get(id).map_or(0, retained_cost))
                    .saturating_add(retained_cost(value));
            }
        }
        if budget > 32 * 1024 * 1024 {
            return Err(Error::BudgetExceeded);
        }
        self.items.extend(gear);
        self.characters.extend(characters);
        self.budget = budget;
        Ok(())
    }

    fn project(
        &self,
        catalog: &EquipmentCatalog,
    ) -> Result<(Vec<EmptyCurtainItem>, Vec<EmptyCurtainCharacter>), Error> {
        let mut characters = Vec::new();
        let mut slots = HashMap::new();
        for row in self.characters.values() {
            let id = uid(&row["UniqueID"])?;
            let character_id = text(row, "ItemID")?
                .parse::<u32>()
                .map_err(|_| Error::InvalidValue)?;
            characters.push(EmptyCurtainCharacter {
                net_id: id,
                character_id,
            });
            let info = &row["CharacterNetInfo"][0];
            let mut all = Vec::new();
            for group in array(info, "EquipmentSlots")? {
                for slot in array(group, "SlotList")? {
                    all.push((slot, false));
                }
            }
            for slot in array(info, "EquipCoreSlots")? {
                all.push((slot, true));
            }
            for (slot, core) in all {
                if slot["State"] != 1 {
                    continue;
                }
                let gear = uid(&slot["EquipNetID"])?;
                if !core && slot["bFirstStep"] != true {
                    continue;
                }
                let position = if core {
                    None
                } else {
                    Some(EmptyCurtainPlacement {
                        row: slot["Row"]
                            .as_i64()
                            .and_then(|n| n.try_into().ok())
                            .ok_or(Error::InvalidValue)?,
                        column: slot["Column"]
                            .as_i64()
                            .and_then(|n| n.try_into().ok())
                            .ok_or(Error::InvalidValue)?,
                    })
                };
                if slots
                    .insert(
                        gear,
                        (
                            id,
                            character_id,
                            text(slot, "EquipmentID")?.to_owned(),
                            position,
                            core,
                        ),
                    )
                    .is_some()
                {
                    return Err(Error::InvalidValue);
                }
            }
        }
        let mut items = Vec::new();
        for row in self.items.values() {
            let id = uid(&row["UniqueID"])?;
            let item_id = text(row, "ItemID")?;
            let mut matches = catalog
                .items
                .iter()
                .filter(|(k, _)| k.eq_ignore_ascii_case(item_id));
            let (key, def) = matches.next().ok_or(Error::Unsupported)?;
            if matches.next().is_some() {
                return Err(Error::InvalidValue);
            }
            let info = &row["EquipmentNetInfo"][0];
            let level = info["StrengthenLevel"]
                .as_u64()
                .and_then(|n| u32::try_from(n).ok())
                .ok_or(Error::InvalidValue)?;
            let mut main_stats = Vec::new();
            for property in array(row, "RandomModifyBaseData")? {
                let property = property.as_str().ok_or(Error::InvalidValue)?;
                let value = catalog
                    .main_stat_sample(def, property, level)
                    .ok_or(Error::Unsupported)?;
                main_stats.push(EquipmentStat {
                    property: property.into(),
                    value,
                });
            }
            let mut sub_stats = Vec::new();
            for stat in array(row, "RandomModifyData")? {
                sub_stats.push(EquipmentStat {
                    property: text(stat, "PropName")?.into(),
                    value: stat["PropValue"].as_f64().ok_or(Error::InvalidValue)? as f32,
                });
            }
            if main_stats.len() != def.main_count || sub_stats.len() != def.sub_count {
                return Err(Error::InvalidValue);
            }
            let owner = uid(&info["CharacterNetID"])?;
            let link = slots.get(&id);
            if link.is_some_and(|s| {
                s.0 != owner
                    || !s.2.eq_ignore_ascii_case(key)
                    || s.4 != (def.kind == EquipmentKind::Core)
            }) {
                return Err(Error::InvalidValue);
            }
            // Missing earlier character records cannot become guessed placements.
            if !owner.is_zero() && link.is_none() {
                return Err(Error::Truncated);
            }
            items.push(EmptyCurtainItem {
                id,
                item_id: key.clone(),
                level,
                main_stats,
                sub_stats,
                locked: info["IsLocked"].as_bool().ok_or(Error::InvalidValue)?,
                discarded: info["IsDiscarded"].as_bool().ok_or(Error::InvalidValue)?,
                character_net_id: (!owner.is_zero()).then_some(owner),
                equipped_character_id: link.map(|s| s.1),
                equipped_placement: link.and_then(|s| s.3),
            });
        }
        Ok((items, characters))
    }
}
struct Flow {
    decoder: Decoder,
    observed: Observed,
    inventory: Option<u64>,
    failed: bool,
}
pub struct InventoryDecoder {
    catalog: Arc<EquipmentCatalog>,
    flows: HashMap<(String, String, u8), Flow>,
    last_emit: f64,
    dirty: bool,
    active: Option<(String, String, u8)>,
}
pub struct Update {
    pub items: Vec<EmptyCurtainItem>,
    pub characters: Vec<EmptyCurtainCharacter>,
}
impl InventoryDecoder {
    pub fn finish(&mut self) -> Result<Option<Update>, Error> {
        let Some(flow) = self.active.as_ref().and_then(|key| self.flows.get(key)) else {
            return Ok(None);
        };
        if flow.failed {
            return Ok(None);
        }
        // A trailing partial RPC may contain an inventory change even when the
        // last complete notification was already published. Never retain that
        // older observation as if this capture ended cleanly.
        if flow.decoder.has_incomplete_fragments() {
            return Err(Error::Truncated);
        }
        if !self.dirty {
            return Ok(None);
        }
        let (items, characters) = flow.observed.project(&self.catalog)?;
        self.dirty = false;
        Ok(Some(Update { items, characters }))
    }
    pub fn new(catalog: Arc<EquipmentCatalog>) -> Self {
        Self {
            catalog,
            flows: HashMap::new(),
            last_emit: f64::NEG_INFINITY,
            dirty: false,
            active: None,
        }
    }
    pub fn datagram(
        &mut self,
        source: String,
        destination: String,
        data: &[u8],
        time: f64,
    ) -> Result<Option<Update>, Error> {
        let Some(first) = data.first() else {
            return Ok(None);
        };
        let key = (source, destination, first & 31);
        if first & 32 != 0 {
            if self.active.as_ref() == Some(&key) {
                self.flows.remove(&key);
                self.active = None;
                self.dirty = false;
                return Ok(Some(Update {
                    items: vec![],
                    characters: vec![],
                }));
            }
            return Ok(None);
        }
        if self.active.as_ref().is_some_and(|active| active != &key) {
            return Ok(None);
        }
        if !self.flows.contains_key(&key) {
            if self.flows.len() >= 8 {
                return Err(Error::BudgetExceeded);
            }
            self.flows.insert(
                key.clone(),
                Flow {
                    decoder: Decoder::inventory(first & 31)?,
                    observed: Observed::default(),
                    inventory: None,
                    failed: false,
                },
            );
        }
        let flow = self.flows.get_mut(&key).ok_or(Error::InvalidValue)?;
        if flow.failed {
            return Ok(None);
        }
        let rpcs = match flow.decoder.datagram(data, true) {
            Ok(r) => r,
            Err(e) => {
                if flow.decoder.controller_bound || flow.inventory.is_some() {
                    flow.failed = true;
                    return Err(e);
                }
                return Ok(None);
            }
        };
        for rpc in rpcs {
            if let Rpc::Inventory { data, bits } = rpc {
                let n = match notification(&data, bits) {
                    Ok(n) => n,
                    Err(e) => {
                        flow.failed = true;
                        return Err(e);
                    }
                };
                if n.kind != 3 {
                    if flow.inventory.is_some()
                        && matches!(n.kind, 1 | 2 | 8 | 25..=30 | 40 | 41 | 46 | 51)
                    {
                        flow.failed = true;
                        return Err(Error::Unsupported);
                    }
                    continue;
                }
                self.active = Some(key.clone());
                if flow.inventory.is_some_and(|i| i != n.inventory) {
                    flow.failed = true;
                    return Err(Error::InvalidValue);
                }
                flow.inventory = Some(n.inventory);
                if let Err(e) = flow.observed.apply(&n) {
                    flow.failed = true;
                    return Err(e);
                }
                self.dirty = true;
            }
        }
        if self.dirty && time - self.last_emit >= 0.25 {
            match flow.observed.project(&self.catalog) {
                Ok((items, characters)) if !items.is_empty() => {
                    self.dirty = false;
                    self.last_emit = time;
                    return Ok(Some(Update { items, characters }));
                }
                Err(Error::Truncated) => {}
                Err(e) => {
                    flow.failed = true;
                    return Err(e);
                }
                _ => {}
            }
        }
        Ok(None)
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[derive(Default)]
    struct Writer {
        data: Vec<u8>,
        bits: usize,
    }
    impl Writer {
        fn put(&mut self, n: u64, bits: usize) {
            self.data.resize((self.bits + bits).div_ceil(8), 0);
            for i in 0..bits {
                self.data[(self.bits + i) / 8] |= (((n >> i) & 1) as u8) << ((self.bits + i) % 8);
            }
            self.bits += bits;
        }
    }
    #[test]
    fn notification_requires_exact_rpc_consumption_and_bounded_arrays() {
        let mut w = Writer::default();
        w.put(1, 1);
        w.put(4, 8);
        w.put(1, 1);
        w.put(3, 7);
        w.put(1, 1);
        w.put(0, 16);
        w.put(0, 1);
        assert!(notification(&w.data, w.bits).is_ok());
        assert!(notification(&w.data, w.bits - 1).is_err());
        w.put(1, 1);
        assert!(notification(&w.data, w.bits).is_err());
        let mut r = Bits::new(&[0xff, 0xff], 16).unwrap();
        assert!(value(&mut r, "TArray<int32>", 0, &mut 0).is_err());
    }
    #[test]
    fn unknown_fnames_are_preserved_not_guessed_into_item_ids() {
        let mut w = Writer::default();
        w.put(1, 1);
        w.put(84, 8);
        let mut r = Bits::new(&w.data, w.bits).unwrap();
        assert_eq!(name(&mut r).unwrap(), json!({"hardcodedIndex":42}));
    }
    #[test]
    fn a_bad_batch_does_not_publish_its_valid_prefix() {
        let mut observed = Observed::default();
        let good = json!({"bIsTemporary":false,"UniqueID":{"solt":1,"serial":2},"Amount":1,"EquipmentNetInfo":[{}],"CharacterNetInfo":[]});
        let bad = json!({"bIsTemporary":false,"UniqueID":{"solt":0,"serial":0}});
        assert!(
            observed
                .apply(&Notification {
                    inventory: 2,
                    kind: 3,
                    items: vec![good, bad]
                })
                .is_err()
        );
        assert!(observed.items.is_empty());
    }
    #[test]
    fn rejected_batch_can_be_followed_by_a_valid_batch_and_duplicates_are_bounded() {
        let mut observed = Observed::default();
        let row = json!({"bIsTemporary":false,"UniqueID":{"solt":1,"serial":2},"Amount":1,"EquipmentNetInfo":[{}],"CharacterNetInfo":[]});
        let mut n = Notification {
            inventory: 2,
            kind: 3,
            items: vec![row.clone(), row.clone()],
        };
        assert!(observed.apply(&n).is_err());
        assert!(observed.items.is_empty());
        n.items = vec![row];
        observed.apply(&n).unwrap();
        let budget = observed.budget;
        observed.apply(&n).unwrap();
        assert_eq!(observed.items.len(), 1);
        assert_eq!(observed.budget, budget);
        observed.budget = 32 * 1024 * 1024;
        n.items[0]["UniqueID"]["serial"] = json!(3);
        assert!(matches!(observed.apply(&n), Err(Error::BudgetExceeded)));
        assert_eq!(observed.items.len(), 1);
    }
    #[test]
    fn handshake_retires_observed_state_and_finish_is_idempotent() {
        let mut d = InventoryDecoder::new(Arc::new(EquipmentCatalog::default()));
        let key = ("server".into(), "client".into(), 28);
        d.flows.insert(
            key.clone(),
            Flow {
                decoder: Decoder::inventory(28).unwrap(),
                observed: Observed::default(),
                inventory: Some(2),
                failed: false,
            },
        );
        d.active = Some(key.clone());
        d.dirty = true;
        assert!(d.finish().unwrap().is_some());
        assert!(d.finish().unwrap().is_none());
        let update = d.datagram(key.0, key.1, &[28 | 32], 1.0).unwrap().unwrap();
        assert!(update.items.is_empty() && update.characters.is_empty());
        assert!(d.active.is_none() && d.flows.is_empty());
        assert!(d.finish().unwrap().is_none());
    }
    #[test]
    fn exact_main_stats_do_not_interpolate_unobserved_curve_samples() {
        let catalog = super::super::parser::load_equipment_catalog(std::path::Path::new(
            super::super::parser::EQUIPMENT_CATALOG_PATH,
        ))
        .unwrap();
        let item = &catalog.items["cell3_style6_1_Orange"];
        assert_eq!(catalog.main_stat_sample(item, "AtkAdd", 20), Some(63.0));
        assert_eq!(catalog.main_stat_sample(item, "AtkAdd", 19), None);
    }
    #[test]
    #[ignore = "requires local captured datagrams; never a committed fixture"]
    fn captured_login_probe() {
        let path = std::env::var("NTE_TEST_INVENTORY_FRAMES").unwrap();
        let bytes = std::fs::read(path).unwrap();
        let v: Value = serde_json::from_slice(&bytes).unwrap();
        let catalog = super::super::parser::load_equipment_catalog(std::path::Path::new(
            super::super::parser::EQUIPMENT_CATALOG_PATH,
        ))
        .unwrap();
        let mut decoder = InventoryDecoder::new(Arc::new(catalog));
        let mut failures = BTreeMap::new();
        let mut last = None;
        for row in v["records"].as_array().unwrap() {
            if row["direction"] != "inbound" {
                continue;
            }
            let text = row["originalPayloadHex"].as_str().unwrap();
            let data = (0..text.len())
                .step_by(2)
                .map(|i| u8::from_str_radix(&text[i..i + 2], 16).unwrap())
                .collect::<Vec<_>>();
            match decoder.datagram(
                "server".into(),
                "local".into(),
                &data,
                row["operationId"].as_u64().unwrap() as f64 / 20.0,
            ) {
                Ok(Some(u)) => last = Some(u),
                Err(e) => *failures.entry(format!("{e:?}")).or_insert(0) += 1,
                _ => {}
            }
        }
        eprintln!(
            "failures={failures:?} bound={:?}",
            decoder
                .flows
                .values()
                .map(|f| (
                    f.decoder.controller_bound,
                    f.observed.items.len(),
                    f.observed.characters.len()
                ))
                .collect::<Vec<_>>()
        );
        if let Some(update) = decoder.finish().unwrap() {
            last = Some(update);
        }
        assert_eq!(
            last.map(|u| (u.items.len(), u.characters.len())),
            Some((561, 21))
        );
        assert!(failures.is_empty());
    }
}
