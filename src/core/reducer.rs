//! The single `EngineEvent` -> `CombatState` merge point. Both the GUI event
//! loop and the CLI core loop route every engine event through
//! [`apply_engine_event`]; neither frontend may keep its own full match over
//! `EngineEvent` domain-state updates.

use crate::engine::model::{CombatState, EngineEvent};

/// What the caller still has to do after the domain state was updated.
/// Frontend-only side effects (toasts, cache invalidation, thread cleanup,
/// event forwarding) key off this instead of re-matching the event.
#[derive(Debug, PartialEq)]
pub enum CoreSignal {
    /// Combat state changed (hit, follow-up, correction, abyss, time stop).
    StateChanged,
    /// The equipment snapshot was replaced wholesale.
    InventoryReplaced,
    /// The captured character-template to session-item mapping changed.
    InventoryCharactersReplaced,
    /// A debug packet was recorded into the state's packet ring.
    DebugPacket,
    /// A lightweight packet observation updated quality counters without
    /// retaining debug payload fields.
    PacketObserved,
    /// Engine status line to surface to the user.
    Status(String),
    /// Non-fatal degradation (e.g. resource load failure).
    Warning(String),
    /// The engine task failed.
    Error(String),
    /// The capture/replay task ended; the frontend owns handle/thread cleanup.
    CaptureStopped,
}

pub fn apply_engine_event(state: &mut CombatState, event: EngineEvent) -> CoreSignal {
    match event {
        EngineEvent::Hit(hit) => {
            state.push_hit(*hit);
            CoreSignal::StateChanged
        }
        EngineEvent::HitFollowUp(follow_up) => {
            state.apply_follow_up(follow_up);
            CoreSignal::StateChanged
        }
        EngineEvent::HitDamageCorrection(correction) => {
            state.apply_damage_correction(correction);
            CoreSignal::StateChanged
        }
        EngineEvent::Packet(packet) => {
            state.push_packet(*packet);
            CoreSignal::DebugPacket
        }
        EngineEvent::PacketObservation(observation) => {
            state.observe_packet(observation);
            CoreSignal::PacketObserved
        }
        EngineEvent::Abyss(event) => {
            state.apply_abyss_event(event);
            CoreSignal::StateChanged
        }
        EngineEvent::TimeStop(event) => {
            state.apply_time_stop_event(event);
            CoreSignal::StateChanged
        }
        EngineEvent::EmptyCurtain(items) => {
            state.replace_empty_curtain(items);
            CoreSignal::InventoryReplaced
        }
        EngineEvent::EmptyCurtainCharacters(characters) => {
            state.replace_empty_curtain_characters(characters);
            CoreSignal::InventoryCharactersReplaced
        }
        EngineEvent::Status(status) => CoreSignal::Status(status),
        EngineEvent::Warning(warning) => CoreSignal::Warning(warning),
        EngineEvent::Error(error) => CoreSignal::Error(error),
        EngineEvent::CaptureStopped => CoreSignal::CaptureStopped,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::model::{
        AbyssEvent, EmptyCurtainCharacter, EmptyCurtainItem, Hit, HitDamageCorrection, HitFollowUp,
        HtItemNetId, PacketDebug, PacketObservation, TimeStopEvent,
    };

    fn test_hit(timestamp: f64, char_id: u32, damage: f64) -> Hit {
        Hit {
            timestamp,
            char_id,
            char_name: format!("角色{char_id}"),
            char_known: true,
            damage,
            byte_offset: 0,
            bit_shift: 0,
            char_source: "test".to_owned(),
            direction: "outgoing".to_owned(),
            target_hp_before: 0.0,
            target_hp_after: 0.0,
            target_max_hp: 0.0,
            target_hp_percent: 0.0,
            target_id: None,
            target_name: None,
            target_context: Vec::new(),
            gameplay_effect_index: None,
            gameplay_effect_name: None,
            ability_name: None,
            damage_name: None,
            attack_type: None,
            damage_attribute: None,
            follow_up_damage: 0.0,
            follow_up_timestamp: None,
            follow_up_damage_name: None,
            follow_up_attack_type: None,
            follow_up_damage_attribute: None,
        }
    }

    fn test_packet() -> PacketDebug {
        PacketDebug {
            timestamp: 1.0,
            source: "10.0.0.1:1".to_owned(),
            destination: "10.0.0.2:2".to_owned(),
            direction: "outgoing".to_owned(),
            payload_len: 0,
            declared_ids: Vec::new(),
            parsed_hits: 0,
            note: String::new(),
            payload_preview: String::new(),
            payload_hex: String::new(),
            decoded_text: String::new(),
        }
    }

    #[test]
    fn hit_pushes_into_state() {
        let mut state = CombatState::default();
        let signal = apply_engine_event(
            &mut state,
            EngineEvent::Hit(Box::new(test_hit(1.0, 7, 100.0))),
        );
        assert_eq!(signal, CoreSignal::StateChanged);
        assert_eq!(state.hits.len(), 1);
        assert_eq!(state.total_damage, 100.0);
    }

    #[test]
    fn follow_up_applies_to_matching_hit() {
        let mut state = CombatState::default();
        apply_engine_event(
            &mut state,
            EngineEvent::Hit(Box::new(test_hit(1.0, 7, 100.0))),
        );
        let follow_up = HitFollowUp {
            source_timestamp: 1.0,
            source_char_id: 7,
            source_damage: 100.0,
            source_target_hp_before: 0.0,
            source_target_hp_after: 0.0,
            source_target_max_hp: 0.0,
            source_gameplay_effect_index: None,
            timestamp: 1.5,
            damage: 25.0,
            target_hp_after: 0.0,
            target_hp_percent: 0.0,
            damage_name: None,
            attack_type: None,
            damage_attribute: None,
        };
        let signal = apply_engine_event(&mut state, EngineEvent::HitFollowUp(follow_up));
        assert_eq!(signal, CoreSignal::StateChanged);
        assert_eq!(state.hits[0].follow_up_damage, 25.0);
        assert_eq!(state.total_damage, 125.0);
    }

    #[test]
    fn damage_correction_applies_to_matching_hit() {
        let mut state = CombatState::default();
        apply_engine_event(
            &mut state,
            EngineEvent::Hit(Box::new(test_hit(1.0, 7, 100.0))),
        );
        let correction = HitDamageCorrection {
            source_timestamp: 1.0,
            source_char_id: 7,
            source_damage: 100.0,
            source_target_hp_before: 0.0,
            source_target_hp_after: 0.0,
            source_target_max_hp: 0.0,
            source_gameplay_effect_index: None,
            damage: 150.0,
            target_hp_before: 0.0,
            target_hp_after: 0.0,
            target_hp_percent: 0.0,
        };
        let signal = apply_engine_event(&mut state, EngineEvent::HitDamageCorrection(correction));
        assert_eq!(signal, CoreSignal::StateChanged);
        assert_eq!(state.damage_correction_count, 1);
        assert_eq!(state.total_damage, 150.0);
    }

    #[test]
    fn packet_lands_in_debug_ring() {
        let mut state = CombatState::default();
        let signal = apply_engine_event(&mut state, EngineEvent::Packet(Box::new(test_packet())));
        assert_eq!(signal, CoreSignal::DebugPacket);
        assert_eq!(state.packets.len(), 1);
        assert_eq!(state.packet_count, 1);
    }

    #[test]
    fn packet_observation_updates_quality_without_debug_payload() {
        let mut state = CombatState::default();
        let signal = apply_engine_event(
            &mut state,
            EngineEvent::PacketObservation(PacketObservation { parsed_hits: 2 }),
        );
        assert_eq!(signal, CoreSignal::PacketObserved);
        assert!(state.packets.is_empty());
        assert_eq!(state.packet_count, 1);
        assert_eq!(state.packets_with_hits, 1);
    }

    #[test]
    fn abyss_event_reaches_abyss_state() {
        let mut state = CombatState::default();
        let signal = apply_engine_event(
            &mut state,
            EngineEvent::Abyss(AbyssEvent::RestartDetected { timestamp: 1.0 }),
        );
        assert_eq!(signal, CoreSignal::StateChanged);
    }

    #[test]
    fn time_stop_event_reaches_tracker() {
        let mut state = CombatState::default();
        let signal = apply_engine_event(
            &mut state,
            EngineEvent::TimeStop(TimeStopEvent::ExtraStart {
                timestamp: 1.0,
                reason: "test".to_owned(),
            }),
        );
        assert_eq!(signal, CoreSignal::StateChanged);
    }

    #[test]
    fn empty_curtain_replaces_inventory() {
        let mut state = CombatState::default();
        let items = vec![EmptyCurtainItem {
            id: HtItemNetId { solt: 1, serial: 2 },
            item_id: "cell2_style1_1_Orange".to_owned(),
            level: 20,
            main_stats: Vec::new(),
            sub_stats: Vec::new(),
            locked: true,
            discarded: false,
            character_net_id: None,
            equipped_character_id: None,
            equipped_placement: None,
        }];
        let generation_before = state.empty_curtain_generation;
        let signal = apply_engine_event(&mut state, EngineEvent::EmptyCurtain(items));
        assert_eq!(signal, CoreSignal::InventoryReplaced);
        assert_eq!(state.empty_curtain.len(), 1);
        assert_eq!(
            state.empty_curtain_generation,
            generation_before.wrapping_add(1)
        );
    }

    #[test]
    fn empty_curtain_character_mapping_reaches_state() {
        let mut state = CombatState::default();
        let character = EmptyCurtainCharacter {
            net_id: HtItemNetId { solt: 3, serial: 4 },
            character_id: 1020,
        };
        let signal = apply_engine_event(
            &mut state,
            EngineEvent::EmptyCurtainCharacters(vec![character]),
        );
        assert_eq!(signal, CoreSignal::InventoryCharactersReplaced);
        assert_eq!(state.empty_curtain_characters, vec![character]);
    }

    #[test]
    fn lifecycle_events_pass_through_without_state_change() {
        let mut state = CombatState::default();
        assert_eq!(
            apply_engine_event(&mut state, EngineEvent::Status("s".to_owned())),
            CoreSignal::Status("s".to_owned())
        );
        assert_eq!(
            apply_engine_event(&mut state, EngineEvent::Warning("w".to_owned())),
            CoreSignal::Warning("w".to_owned())
        );
        assert_eq!(
            apply_engine_event(&mut state, EngineEvent::Error("e".to_owned())),
            CoreSignal::Error("e".to_owned())
        );
        assert_eq!(
            apply_engine_event(&mut state, EngineEvent::CaptureStopped),
            CoreSignal::CaptureStopped
        );
        assert_eq!(state.hits.len(), 0);
        assert_eq!(state.packets.len(), 0);
    }
}
