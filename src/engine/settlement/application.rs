//! Durable identities and application events. No timestamp/amount matching.
use super::{Change, Damage, MessageKey};
use crate::engine::model::{CharacterInfo, Hit, HitCharacterSource, HitDirection};
use crate::engine::parser::DamageDisplayType;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct MessageIdentity {
    pub generation: String,
    pub connection: String,
    pub channel: u32,
    // Decimal strings preserve all bits in JS and persisted JSON consumers.
    pub message: String,
    pub timestamp_bits: String,
}

impl MessageIdentity {
    pub fn new(generation: String, connection: String, key: MessageKey) -> Self {
        Self {
            generation,
            connection,
            channel: key.channel,
            message: key.message.to_string(),
            timestamp_bits: key.timestamp_bits.to_string(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Evidence {
    pub message: MessageIdentity,
    pub target_ordinal: usize,
    pub component_ordinal: usize,
    pub source: super::ActorRef,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub request_source: Option<super::ActorRef>,
    pub target: super::ActorRef,
    pub display_type: i32,
    pub attribution: super::Attribution,
    pub current_hp_bits: u32,
    pub hp_before_request_bits: Option<u32>,
    pub max_hp_at_request_bits: Option<u32>,
    /// Derived rule application, independently checked against server HP
    /// continuity. Absent in old archives and unverified observations.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hp_adjustment: Option<super::HpAdjustment>,
}

#[derive(Clone, Debug)]
pub struct Projection {
    pub identity: MessageIdentity,
    pub hits: Vec<Hit>,
    pub quarantined: bool,
}

pub fn project(
    change: Change,
    generation: &str,
    connection: &str,
    capture_timestamp: f64,
    characters: &HashMap<u32, CharacterInfo>,
    include_incoming: bool,
) -> Projection {
    let identity = MessageIdentity::new(generation.into(), connection.into(), change.key);
    let hits = change
        .rows
        .iter()
        .filter_map(|row| {
            to_hit(
                row,
                &identity,
                capture_timestamp,
                characters,
                include_incoming,
            )
        })
        .collect();
    Projection {
        identity,
        hits,
        quarantined: matches!(change.conflict, Some(super::Error::ConflictingSettlement)),
    }
}

fn to_hit(
    row: &Damage,
    identity: &MessageIdentity,
    time: f64,
    characters: &HashMap<u32, CharacterInfo>,
    include_incoming: bool,
) -> Option<Hit> {
    let source = row
        .request_source
        .as_ref()
        .unwrap_or(&row.source)
        .character_id()
        .filter(|id| characters.contains_key(id));
    let target = row
        .target
        .character_id()
        .filter(|id| characters.contains_key(id));
    let (role, direction) = if let Some(id) = source {
        (Some(id), HitDirection::Outgoing)
    } else if let Some(id) = target {
        (Some(id), HitDirection::Incoming)
    } else {
        (None, HitDirection::Unknown)
    };
    if direction.is_incoming() && !include_incoming {
        return None;
    }
    let character = role.and_then(|id| characters.get(&id));
    let special = DamageDisplayType::try_from(row.display_type)
        .ok()
        .and_then(|d| d.damage_name().map(|name| (name, d.attack_type())));
    let hp = f64::from(f32::from_bits(row.current_hp_bits));
    let before = row
        .hp_before_request_bits
        .map(|v| f64::from(f32::from_bits(v)));
    let maximum = row
        .max_hp_at_request_bits
        .map(|v| f64::from(f32::from_bits(v)));
    // Internal legacy numeric slots use an explicit -1 unknown sentinel. API
    // readers consult exact evidence and emit null; never expose this as HP.
    let target_id = format!(
        "wire:{}:{}:{}",
        identity.generation,
        identity.connection,
        serde_json::to_string(&row.target).ok()?
    );
    Some(Hit {
        timestamp: time,
        char_id: role.unwrap_or(0),
        char_name: character
            .map(|c| c.name_zh.clone())
            .unwrap_or_else(|| "Unknown".into()),
        char_known: character.is_some(),
        damage: f64::from(row.damage),
        byte_offset: 0,
        bit_shift: 0,
        char_source: if character.is_some() {
            HitCharacterSource::Packet
        } else {
            HitCharacterSource::Unknown
        },
        direction,
        target_hp_before: before.unwrap_or(-1.0),
        target_hp_after: hp,
        target_max_hp: maximum.unwrap_or(-1.0),
        max_hp_reduction: row
            .hp_adjustment
            .as_ref()
            .map_or(0.0, super::HpAdjustment::reduction),
        target_hp_percent: maximum
            .filter(|v| *v > 0.0)
            .map(|v| hp / v * 100.0)
            .unwrap_or(-1.0),
        target_id: Some(target_id),
        target_name: None,
        target_name_en: None,
        target_name_ja: None,
        target_monster_id: None,
        target_context: vec!["exact_settlement_reference".into()],
        gameplay_effect_index: if row.effect_candidates.len() == 1 {
            row.effect_candidates.first().copied()
        } else {
            None
        },
        gameplay_effect_name: row.effect_name.clone(),
        ability_name: row.skill_key.clone(),
        damage_name: special
            .map(|s| s.0.to_owned())
            .or_else(|| row.skill_name.clone()),
        damage_component: (special.is_none() || row.display_type == 22)
            .then(|| row.mechanic.clone())
            .flatten(),
        attack_type: special.and_then(|s| s.1.map(str::to_owned)),
        damage_attribute: None,
        follow_up_damage: row
            .hp_adjustment
            .as_ref()
            .map_or(0.0, |a| a.additional_loss(row.current_hp_bits)),
        follow_up_timestamp: None,
        follow_up_damage_name: row
            .hp_adjustment
            .as_ref()
            .map(|_| "Max HP scaling loss".to_owned()),
        follow_up_attack_type: None,
        follow_up_damage_attribute: None,
        reconciled_overkill_damage: None,
        plugin_snapshot: None,
        wire_event: None,
        exact: Some(Evidence {
            message: identity.clone(),
            target_ordinal: row.target_ordinal,
            component_ordinal: row.component_ordinal,
            source: row.source.clone(),
            request_source: row.request_source.clone(),
            target: row.target.clone(),
            display_type: row.display_type,
            attribution: row.attribution.clone(),
            current_hp_bits: row.current_hp_bits,
            hp_before_request_bits: row.hp_before_request_bits,
            max_hp_at_request_bits: row.max_hp_at_request_bits,
            hp_adjustment: row.hp_adjustment.clone(),
        }),
    })
}
