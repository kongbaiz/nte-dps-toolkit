use std::collections::HashMap;

use crate::engine::model::{
    CharacterInfo, Hit, IndexedCombatDetailFilter, is_unbalance_damage_hit, reaction_damage_for_hit,
};

/// Stable, frontend-neutral filtering for combat hit detail projections.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum CombatDetailFilter {
    #[default]
    All,
    Outgoing,
    Incoming,
    CharacterAttributed,
    CharacterDirect,
    ReactionDamage,
    SharedMechanics,
    Unattributed,
    QteType(String),
}

impl CombatDetailFilter {
    pub fn matches(&self, hit: &Hit) -> bool {
        match self {
            Self::All => true,
            Self::Outgoing => !hit.direction.is_incoming(),
            Self::Incoming => hit.direction.is_incoming(),
            Self::CharacterAttributed => {
                hit.direction.is_outgoing() && hit.char_known && !is_unbalance_damage_hit(hit)
            }
            Self::CharacterDirect => {
                hit.direction.is_outgoing()
                    && hit.char_known
                    && !is_unbalance_damage_hit(hit)
                    && hit.total_damage() > reaction_damage_for_hit(hit)
            }
            Self::ReactionDamage => {
                hit.direction.is_outgoing() && hit.char_known && reaction_damage_for_hit(hit) > 0.0
            }
            Self::SharedMechanics => !hit.direction.is_incoming() && is_unbalance_damage_hit(hit),
            Self::Unattributed => {
                !(hit.direction.is_incoming()
                    || is_unbalance_damage_hit(hit)
                    || (hit.direction.is_outgoing() && hit.char_known))
            }
            Self::QteType(attack_type) => {
                !hit.direction.is_incoming()
                    && (hit.attack_type.as_deref() == Some(attack_type)
                        || hit.follow_up_attack_type.as_deref() == Some(attack_type))
            }
        }
    }

    pub fn indexed(&self) -> IndexedCombatDetailFilter {
        match self {
            Self::All => IndexedCombatDetailFilter::All,
            Self::Outgoing => IndexedCombatDetailFilter::Outgoing,
            Self::Incoming => IndexedCombatDetailFilter::Incoming,
            Self::CharacterAttributed => IndexedCombatDetailFilter::CharacterAttributed,
            Self::CharacterDirect => IndexedCombatDetailFilter::CharacterDirect,
            Self::ReactionDamage => IndexedCombatDetailFilter::ReactionDamage,
            Self::SharedMechanics => IndexedCombatDetailFilter::SharedMechanics,
            Self::Unattributed => IndexedCombatDetailFilter::Unattributed,
            Self::QteType(value) => IndexedCombatDetailFilter::QteType(value.clone()),
        }
    }
}

/// Returns the localized reaction-text asset index used by both desktop frontends.
pub fn reaction_text_key_for_hit(hit: &Hit) -> Option<u8> {
    hit.attack_type
        .as_deref()
        .and_then(reaction_text_key_from_trigger_attack_type)
}

pub fn reaction_text_key_from_trigger_attack_type(attack_type: &str) -> Option<u8> {
    let reaction = attack_type.strip_prefix("环合·")?;
    match reaction {
        "创生" | "创生花" => Some(1),
        "覆纹" => Some(2),
        "黯星" => Some(3),
        "浊燃" | "灼燃" => Some(4),
        "浸染" => Some(5),
        "延滞" => Some(6),
        "盈蓄" => Some(7),
        "失谐" => Some(8),
        _ => None,
    }
}

/// Stable resource key for the primary damage digit set.
pub fn damage_digit_key_for_hit<'a>(
    hit: &'a Hit,
    characters: &'a HashMap<u32, CharacterInfo>,
) -> Option<&'a str> {
    if hit.direction.is_incoming() {
        return Some("HP");
    }
    let source_attribute = hit.damage_attribute.as_deref().or_else(|| {
        characters
            .get(&hit.char_id)
            .and_then(|character| character.attribute.as_deref())
    });
    let attack_type = hit.attack_type.as_deref();
    if attack_type == Some("倾陷伤害")
        || hit
            .damage_name
            .as_deref()
            .is_some_and(|name| name.contains("倾陷"))
    {
        return Some("真实");
    }
    attack_type
        .and_then(|value| mixed_damage_digit_key(value, source_attribute))
        .or(source_attribute)
}

/// Stable resource key for the separately rendered follow-up damage digits.
pub fn follow_up_damage_digit_key_for_hit(hit: &Hit) -> Option<&str> {
    let source_attribute = hit.follow_up_damage_attribute.as_deref()?;
    hit.follow_up_attack_type
        .as_deref()
        .and_then(|value| mixed_damage_digit_key(value, Some(source_attribute)))
        .or(Some(source_attribute))
}

pub fn mixed_damage_digit_key(
    attack_type: &str,
    source_attribute: Option<&str>,
) -> Option<&'static str> {
    if attack_type.starts_with("环合·") {
        return None;
    }
    match attack_type {
        "创生" | "创生花" => Some("Guangling_G"),
        "覆纹" => Some("lingzhou_L"),
        "黯星" => Some("Anhun_A"),
        "浊燃" => Some("Zhouan_A"),
        "延滞" => match source_attribute? {
            "光" => Some("Guangxiang_G"),
            "相" => Some("Guangxiang_X"),
            _ => None,
        },
        "浸染" | "魂相" => match source_attribute? {
            "魂" => Some("Hunxiang_H"),
            "相" => Some("Hunxiang_X"),
            _ => None,
        },
        "盈蓄" => match source_attribute? {
            "光" => Some("Guangling_G"),
            "相" => Some("Guangxiang_X"),
            _ => None,
        },
        "失谐" => match source_attribute? {
            "暗" => Some("Anhun_A"),
            _ => None,
        },
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::model::{HitCharacterSource, HitDirection};

    fn hit(direction: HitDirection, known: bool, attack_type: Option<&str>) -> Hit {
        Hit {
            timestamp: 1.0,
            char_id: 1,
            char_name: "Character".to_owned(),
            char_known: known,
            damage: 100.0,
            byte_offset: 0,
            bit_shift: 0,
            char_source: HitCharacterSource::Packet,
            direction,
            target_hp_before: 1000.0,
            target_hp_after: 900.0,
            target_max_hp: 1000.0,
            target_hp_percent: 90.0,
            target_id: None,
            target_name: None,
            target_name_en: None,
            target_name_ja: None,
            target_monster_id: None,
            target_context: Vec::new(),
            gameplay_effect_index: None,
            gameplay_effect_name: None,
            ability_name: None,
            damage_name: None,
            damage_component: None,
            attack_type: attack_type.map(str::to_owned),
            damage_attribute: None,
            follow_up_damage: 0.0,
            follow_up_timestamp: None,
            follow_up_damage_name: None,
            follow_up_attack_type: None,
            follow_up_damage_attribute: None,
            reconciled_overkill_damage: None,
            wire_event: None,
        }
    }

    #[test]
    fn attribution_filters_share_the_existing_engine_classification() {
        let direct = hit(HitDirection::Outgoing, true, None);
        assert!(CombatDetailFilter::CharacterAttributed.matches(&direct));
        assert!(CombatDetailFilter::CharacterDirect.matches(&direct));

        let shared = hit(HitDirection::Outgoing, true, Some("倾陷伤害"));
        assert!(CombatDetailFilter::SharedMechanics.matches(&shared));
        assert!(!CombatDetailFilter::CharacterAttributed.matches(&shared));

        let unknown = hit(HitDirection::Unknown, false, None);
        assert!(CombatDetailFilter::Unattributed.matches(&unknown));
    }

    #[test]
    fn shared_detail_assets_match_the_established_frontend_rules() {
        let characters = HashMap::from([(
            1,
            CharacterInfo {
                name_zh: "角色".to_owned(),
                name_en: String::new(),
                color: None,
                avatar: None,
                attribute: Some("灵".to_owned()),
            },
        )]);
        let mut value = hit(HitDirection::Outgoing, true, Some("环合·创生"));
        assert_eq!(reaction_text_key_for_hit(&value), Some(1));
        assert_eq!(damage_digit_key_for_hit(&value, &characters), Some("灵"));

        value.attack_type = Some("创生花".to_owned());
        value.damage_attribute = Some("光".to_owned());
        assert_eq!(
            damage_digit_key_for_hit(&value, &characters),
            Some("Guangling_G")
        );
    }
}
