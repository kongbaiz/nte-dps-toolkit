use crate::engine::model::{Hit, is_unbalance_damage_hit, reaction_damage_for_hit};

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
                !hit.direction.is_incoming()
                    && !is_unbalance_damage_hit(hit)
                    && !(hit.direction.is_outgoing() && hit.char_known)
            }
            Self::QteType(attack_type) => {
                !hit.direction.is_incoming()
                    && (hit.attack_type.as_deref() == Some(attack_type)
                        || hit.follow_up_attack_type.as_deref() == Some(attack_type))
            }
        }
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
}
