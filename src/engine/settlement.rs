//! Exact, connection-scoped request/settlement accounting.
//!
//! Input must be an independently bounded, qualified RPC payload, not an arbitrary
//! UDP byte-pattern match. Requests never create damage. No amount/time proximity,
//! active-character fallback, HP differencing, or inferred critical flags are used.

use serde::{Deserialize, Serialize};
use std::collections::{BTreeSet, HashMap};

pub mod application;
pub mod runtime;
pub mod transport;
mod wire;
pub use wire::{decode_request, decode_settlement};

#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Name {
    Text { text: String, number: u32 },
    Hardcoded(u32),
}

/// Full transmitted identity, including tail fields. No invented omitted defaults.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ActorRef {
    pub flags: u8,
    pub name: Option<Name>,
    pub fields: Vec<(u8, u32)>,
}

impl ActorRef {
    pub fn character_id(&self) -> Option<u32> {
        match (&self.name, self.flags, self.fields.is_empty()) {
            (Some(Name::Text { text, number: 0 }), 1, true) => text.parse().ok(),
            _ => None,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct MessageKey {
    pub channel: u32,
    pub message: i64,
    /// Raw correlation bits, NOT a validated DPS clock or server wall-clock time.
    pub timestamp_bits: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RequestTarget {
    pub source: ActorRef,
    pub target: ActorRef,
    pub effect_index: Option<u32>,
    pub hp_before_bits: Option<u32>,
    pub max_hp_bits: Option<u32>,
    pub calculated_damage_bits: Option<u32>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Request {
    pub key: MessageKey,
    pub targets: Vec<RequestTarget>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Component {
    pub damage: i32,
    pub display_type: i32,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SettledTarget {
    pub target: ActorRef,
    pub current_hp_bits: u32,
    pub dead_state: i32,
    pub shield_damage_bits: u32,
    pub lock_target: i32,
    pub components: Vec<Component>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Settlement {
    pub key: MessageKey,
    pub source: ActorRef,
    pub targets: Vec<SettledTarget>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Skill {
    pub key: String,
    pub name: Option<String>,
    pub owners: BTreeSet<u32>,
}

/// Only explicitly established GE -> statistical root mappings belong here.
#[derive(Clone, Debug, Default)]
pub struct SkillCatalog {
    pub effects: HashMap<u32, Vec<Skill>>,
    /// An index with any unresolved candidate cannot be promoted by dropping it.
    pub unresolved_effects: BTreeSet<u32>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Attribution {
    ExactRequestAssetGroup,
    ExplicitSettlementCategory,
    RequestMissing,
    SourceUnresolved,
    SkillUnresolved,
    SkillConflict,
    OwnerConflict,
    UnknownCategory,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Damage {
    pub key: MessageKey,
    pub target_ordinal: usize,
    pub component_ordinal: usize,
    pub source: ActorRef,
    pub target: ActorRef,
    pub character_id: Option<u32>,
    pub damage: i32,
    pub display_type: i32,
    pub current_hp_bits: u32,
    /// Request-time snapshots; missing/conflicting information remains None.
    pub hp_before_request_bits: Option<u32>,
    pub max_hp_at_request_bits: Option<u32>,
    pub skill_key: Option<String>,
    pub skill_name: Option<String>,
    pub effect_candidates: Vec<u32>,
    pub attribution: Attribution,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Error {
    Truncated,
    InvalidValue,
    Unsupported,
    BudgetExceeded,
    TrailingBits,
    ConflictingRequest,
    ConflictingSettlement,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Change {
    /// Replace this message's projection; empty rows also mean a retraction.
    /// Consumers must upsert/retract, never append replacement rows to totals.
    pub key: MessageKey,
    pub rows: Vec<Damage>,
    pub conflict: Option<Error>,
}

#[derive(Default)]
struct Entry {
    request: Option<Request>,
    settlement: Option<Settlement>,
    request_conflict: bool,
    settlement_conflict: bool,
}

/// One generation/connection per instance. Hard capacity failure requires an
/// explicit reset/new generation; evicting dedup identities would double-count.
pub struct Ledger {
    entries: HashMap<MessageKey, Entry>,
    capacity: usize,
    catalog: SkillCatalog,
}

impl Ledger {
    pub fn new(capacity: usize, catalog: SkillCatalog) -> Self {
        Self {
            entries: HashMap::new(),
            capacity,
            catalog,
        }
    }

    pub fn clear(&mut self) {
        self.entries.clear();
    }

    fn entry(&mut self, key: MessageKey) -> Result<&mut Entry, Error> {
        if !self.entries.contains_key(&key) && self.entries.len() >= self.capacity {
            return Err(Error::BudgetExceeded);
        }
        Ok(self.entries.entry(key).or_default())
    }

    pub fn request(&mut self, request: Request) -> Result<Option<Change>, Error> {
        let key = request.key;
        let entry = self.entry(key)?;
        if entry.request_conflict || entry.request.as_ref() == Some(&request) {
            return Ok(None);
        }
        if entry.request.is_some() {
            entry.request_conflict = true;
        } else {
            entry.request = Some(request);
        }
        Ok(self.project(key))
    }

    pub fn settlement(&mut self, settlement: Settlement) -> Result<Option<Change>, Error> {
        let key = settlement.key;
        let entry = self.entry(key)?;
        if entry.settlement_conflict || entry.settlement.as_ref() == Some(&settlement) {
            return Ok(None);
        }
        if entry.settlement.is_some() {
            entry.settlement_conflict = true;
        } else {
            entry.settlement = Some(settlement);
        }
        Ok(self.project(key))
    }

    fn project(&self, key: MessageKey) -> Option<Change> {
        let entry = self.entries.get(&key)?;
        if entry.settlement_conflict {
            return Some(Change {
                key,
                rows: vec![],
                conflict: Some(Error::ConflictingSettlement),
            });
        }
        let settlement = entry.settlement.as_ref()?;
        let mut rows = Vec::new();
        for (target_ordinal, target) in settlement.targets.iter().enumerate() {
            let matching: Vec<_> = if entry.request_conflict {
                vec![]
            } else {
                entry
                    .request
                    .iter()
                    .flat_map(|r| &r.targets)
                    .filter(|r| r.source == settlement.source && r.target == target.target)
                    .collect()
            };
            let before = consensus(&matching, |r| r.hp_before_bits);
            let maximum = consensus(&matching, |r| r.max_hp_bits);
            let mut effects = matching
                .iter()
                .filter_map(|r| r.effect_index)
                .collect::<Vec<_>>();
            effects.sort_unstable();
            effects.dedup();
            let character_id = settlement.source.character_id();
            for (component_ordinal, component) in target.components.iter().enumerate() {
                let mut row = Damage {
                    key,
                    target_ordinal,
                    component_ordinal,
                    source: settlement.source.clone(),
                    target: target.target.clone(),
                    character_id,
                    damage: component.damage,
                    display_type: component.display_type,
                    current_hp_bits: target.current_hp_bits,
                    hp_before_request_bits: before,
                    max_hp_at_request_bits: maximum,
                    skill_key: None,
                    skill_name: None,
                    effect_candidates: effects.clone(),
                    attribution: Attribution::RequestMissing,
                };
                row.attribution = if (22..=28).contains(&component.display_type) {
                    Attribution::ExplicitSettlementCategory
                } else if !(0..=31).contains(&component.display_type) {
                    Attribution::UnknownCategory
                } else if character_id.is_none() {
                    Attribution::SourceUnresolved
                } else if matching.is_empty() {
                    Attribution::RequestMissing
                } else {
                    let options: Option<Vec<_>> = matching
                        .iter()
                        .map(|r| {
                            r.effect_index
                                .filter(|i| !self.catalog.unresolved_effects.contains(i))
                                .and_then(|i| self.catalog.effects.get(&i))
                                .filter(|v| !v.is_empty())
                        })
                        .collect();
                    match options {
                        None => Attribution::SkillUnresolved,
                        Some(options) => {
                            let roots: Vec<_> = options.into_iter().flatten().collect();
                            let first = roots[0];
                            if roots.iter().any(|r| r.key != first.key) {
                                Attribution::SkillConflict
                            } else if roots
                                .iter()
                                .any(|r| !r.owners.contains(&character_id.unwrap_or_default()))
                            {
                                Attribution::OwnerConflict
                            } else {
                                row.skill_key = Some(first.key.clone());
                                row.skill_name = first.name.clone();
                                Attribution::ExactRequestAssetGroup
                            }
                        }
                    }
                };
                rows.push(row);
            }
        }
        Some(Change {
            key,
            rows,
            conflict: entry.request_conflict.then_some(Error::ConflictingRequest),
        })
    }
}

fn consensus(
    rows: &[&RequestTarget],
    field: impl Fn(&RequestTarget) -> Option<u32>,
) -> Option<u32> {
    let first = field(rows.first()?)?;
    rows.iter()
        .all(|r| field(r) == Some(first))
        .then_some(first)
}

#[cfg(test)]
mod tests;
