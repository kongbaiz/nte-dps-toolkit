//! The single `EngineEvent` -> `CombatState` merge point. Both the GUI event
//! loop and the CLI core loop route every engine event through
//! [`apply_engine_event`]; neither frontend may keep its own full match over
//! `EngineEvent` domain-state updates.

use crate::engine::model::{CombatState, EngineEvent, ModScriptEvent};

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
    /// A typed script bridge message for frontend pre/post-processing.
    ModScript(ModScriptEvent),
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
        EngineEvent::ModScript(event) => {
            state.apply_mod_script_event(&event);
            CoreSignal::ModScript(event)
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
        AbyssEvent, EmptyCurtainCharacter, EmptyCurtainItem, EnemyIdentity, Hit,
        HitCharacterSource, HitDamageCorrection, HitDirection, HitFollowUp, HtItemNetId,
        PacketDebug, PacketObservation, TimeStopEvent,
    };

    const FILETIME_UNIX_EPOCH_100NS: u64 = 116_444_736_000_000_000;

    fn filetime(timestamp: f64) -> u64 {
        FILETIME_UNIX_EPOCH_100NS + (timestamp * 10_000_000.0) as u64
    }

    fn test_hit(timestamp: f64, char_id: u32, damage: f64) -> Hit {
        Hit {
            timestamp,
            char_id,
            char_name: format!("角色{char_id}"),
            char_known: true,
            damage,
            byte_offset: 0,
            bit_shift: 0,
            char_source: HitCharacterSource::Unknown,
            direction: HitDirection::Outgoing,
            target_hp_before: 0.0,
            target_hp_after: 0.0,
            target_max_hp: 0.0,
            target_hp_percent: 0.0,
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
    fn mod_script_event_is_forwarded_without_changing_combat_state() {
        let mut state = CombatState::default();
        let event = ModScriptEvent::from_bridge(
            4,
            9,
            "example".to_owned(),
            "pre.hit".to_owned(),
            vec![7, 8],
        );

        let signal = apply_engine_event(&mut state, EngineEvent::ModScript(event.clone()));

        assert_eq!(signal, CoreSignal::ModScript(event));
        assert!(state.hits.is_empty());
        assert_eq!(state.total_damage, 0.0);
    }

    fn enemy_identity_event(timestamp: f64) -> ModScriptEvent {
        enemy_identity_event_for(
            timestamp,
            0x1234,
            0x4d88_7b49_05d5_dbaf,
            "Boss_016_BP",
            "Boss_16",
            "Imaginadough",
            "随心泥",
            "イメージクレイ",
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn enemy_identity_event_for(
        timestamp: f64,
        target: u64,
        config_hash: u64,
        config_id: &str,
        monster_id: &str,
        name_en: &str,
        name_zh: &str,
        name_ja: &str,
    ) -> ModScriptEvent {
        let mut event = ModScriptEvent::from_bridge(
            1,
            filetime(timestamp),
            "enemy-telemetry".to_owned(),
            "pre.enemy.identity".to_owned(),
            vec![target, config_hash, 80],
        );
        event.enemy_identity = Some(EnemyIdentity {
            config_hash,
            config_id: config_id.to_owned(),
            monster_id: monster_id.to_owned(),
            name_en: name_en.to_owned(),
            name_zh: name_zh.to_owned(),
            name_ja: name_ja.to_owned(),
        });
        event
    }

    fn enemy_vitals_event(timestamp: f64, hp: f64, max_hp: f64) -> ModScriptEvent {
        enemy_vitals_event_for(timestamp, 0x1234, hp, max_hp)
    }

    fn enemy_vitals_event_for(timestamp: f64, target: u64, hp: f64, max_hp: f64) -> ModScriptEvent {
        ModScriptEvent::from_bridge(
            2,
            filetime(timestamp),
            "enemy-telemetry".to_owned(),
            "post.enemy.vitals".to_owned(),
            vec![target, (hp * 1000.0) as u64, (max_hp * 1000.0) as u64],
        )
    }

    #[test]
    fn enemy_telemetry_projects_identity_only_across_matching_hp_stream() {
        let mut state = CombatState::default();
        apply_engine_event(
            &mut state,
            EngineEvent::ModScript(enemy_identity_event(10.0)),
        );
        apply_engine_event(
            &mut state,
            EngineEvent::ModScript(enemy_vitals_event(10.0, 2_220_578.0, 2_220_578.0)),
        );
        let mut matching = test_hit(10.1, 7, 775.0);
        matching.target_hp_before = 2_220_578.0;
        matching.target_hp_after = 2_219_803.0;
        matching.target_max_hp = 2_220_578.0;
        matching.target_hp_percent = 99.965;
        apply_engine_event(&mut state, EngineEvent::Hit(Box::new(matching)));

        let projected = state.hits.back().expect("matching hit should be retained");
        assert_eq!(projected.target_name.as_deref(), Some("随心泥"));
        assert_eq!(projected.target_name_en.as_deref(), Some("Imaginadough"));
        assert_eq!(projected.target_name_ja.as_deref(), Some("イメージクレイ"));
        assert_eq!(projected.target_monster_id.as_deref(), Some("Boss_16"));
        assert_eq!(
            projected.target_id.as_deref(),
            Some("enemy:4d887b4905d5dbaf")
        );

        let mut continuation = test_hit(10.15, 7, 311.0);
        continuation.target_hp_before = 2_219_803.0;
        continuation.target_hp_after = 2_219_492.0;
        continuation.target_max_hp = 2_220_578.0;
        apply_engine_event(&mut state, EngineEvent::Hit(Box::new(continuation)));
        assert_eq!(
            state
                .hits
                .back()
                .expect("cached identity should project to the continued HP stream")
                .target_name
                .as_deref(),
            Some("随心泥")
        );

        let mut mismatching = test_hit(10.2, 7, 100.0);
        mismatching.target_hp_before = 500_000.0;
        mismatching.target_hp_after = 499_900.0;
        mismatching.target_max_hp = 2_220_578.0;
        apply_engine_event(&mut state, EngineEvent::Hit(Box::new(mismatching)));
        assert!(
            state
                .hits
                .back()
                .expect("hit should be retained")
                .target_name
                .is_none()
        );
    }

    #[test]
    fn enemy_vitals_backfills_only_the_bounded_matching_hp_chain() {
        let mut state = CombatState::default();
        apply_engine_event(
            &mut state,
            EngineEvent::Abyss(AbyssEvent::Stage {
                timestamp: 19.0,
                cycle: Some(1),
                floor: Some(11),
                half: crate::engine::model::AbyssHalf::First,
                allow_late_backfill: false,
            }),
        );
        apply_engine_event(
            &mut state,
            EngineEvent::ModScript(enemy_identity_event(20.0)),
        );
        let mut first = test_hit(20.1, 7, 100.0);
        first.target_hp_before = 1_000.0;
        first.target_hp_after = 900.0;
        first.target_max_hp = 1_000.0;
        apply_engine_event(&mut state, EngineEvent::Hit(Box::new(first)));
        let mut second = test_hit(20.2, 7, 100.0);
        second.target_hp_before = 900.0;
        second.target_hp_after = 800.0;
        second.target_max_hp = 1_000.0;
        apply_engine_event(&mut state, EngineEvent::Hit(Box::new(second)));
        assert!(state.hits.iter().all(|hit| hit.target_name.is_none()));
        let generation_before_backfill = state.hits_generation;
        let party_generation_before_backfill = state.abyss.first_half.hits_generation;

        apply_engine_event(
            &mut state,
            EngineEvent::ModScript(enemy_vitals_event(20.25, 800.0, 1_000.0)),
        );

        assert!(
            state
                .hits
                .iter()
                .all(|hit| hit.target_name.as_deref() == Some("随心泥"))
        );
        assert_eq!(
            state.hits_generation,
            generation_before_backfill.wrapping_add(1)
        );
        assert!(
            state
                .abyss
                .first_half
                .hits
                .iter()
                .all(|hit| hit.target_name.as_deref() == Some("随心泥"))
        );
        assert_eq!(
            state.abyss.first_half.hits_generation,
            party_generation_before_backfill.wrapping_add(1)
        );
    }

    #[test]
    fn enemy_telemetry_keeps_independent_hp_streams_for_multiple_targets() {
        let mut state = CombatState::default();
        apply_engine_event(
            &mut state,
            EngineEvent::ModScript(enemy_identity_event(30.0)),
        );
        apply_engine_event(
            &mut state,
            EngineEvent::ModScript(enemy_vitals_event_for(30.0, 0x1234, 1_000.0, 1_000.0)),
        );
        apply_engine_event(
            &mut state,
            EngineEvent::ModScript(enemy_identity_event_for(
                30.0,
                0x5678,
                0x2222,
                "Monster_002_BP",
                "Monster_2",
                "Second target",
                "第二目标",
                "第2ターゲット",
            )),
        );
        apply_engine_event(
            &mut state,
            EngineEvent::ModScript(enemy_vitals_event_for(30.0, 0x5678, 2_000.0, 2_000.0)),
        );

        let mut first_target = test_hit(30.1, 7, 100.0);
        first_target.target_hp_before = 1_000.0;
        first_target.target_hp_after = 900.0;
        first_target.target_max_hp = 1_000.0;
        apply_engine_event(&mut state, EngineEvent::Hit(Box::new(first_target)));

        let mut second_target = test_hit(30.2, 7, 200.0);
        second_target.target_hp_before = 2_000.0;
        second_target.target_hp_after = 1_800.0;
        second_target.target_max_hp = 2_000.0;
        apply_engine_event(&mut state, EngineEvent::Hit(Box::new(second_target)));

        let mut first_target_again = test_hit(30.3, 7, 100.0);
        first_target_again.target_hp_before = 900.0;
        first_target_again.target_hp_after = 800.0;
        first_target_again.target_max_hp = 1_000.0;
        apply_engine_event(&mut state, EngineEvent::Hit(Box::new(first_target_again)));

        assert_eq!(state.hits[0].target_name.as_deref(), Some("随心泥"));
        assert_eq!(state.hits[1].target_name.as_deref(), Some("第二目标"));
        assert_eq!(state.hits[2].target_name.as_deref(), Some("随心泥"));
    }

    #[test]
    fn enemy_telemetry_keeps_identity_across_ahead_of_hit_sampling() {
        let mut state = CombatState::default();
        apply_engine_event(
            &mut state,
            EngineEvent::ModScript(enemy_identity_event(35.0)),
        );
        apply_engine_event(
            &mut state,
            EngineEvent::ModScript(enemy_vitals_event(35.0, 1_851_161.0, 2_220_578.0)),
        );

        let mut first = test_hit(35.01, 7, 9_582.0);
        first.target_hp_before = 1_851_161.0;
        first.target_hp_after = 1_841_579.0;
        first.target_max_hp = 2_220_578.0;
        apply_engine_event(&mut state, EngineEvent::Hit(Box::new(first)));

        apply_engine_event(
            &mut state,
            EngineEvent::ModScript(enemy_vitals_event(35.02, 1_833_670.0, 2_220_578.0)),
        );
        for (timestamp, damage, hp_before, hp_after) in [
            (35.03, 492.0, 1_841_579.0, 1_841_087.0),
            (35.03, 492.0, 1_841_087.0, 1_841_087.0),
            (35.04, 7_909.0, 1_841_087.0, 1_833_670.0),
        ] {
            let mut hit = test_hit(timestamp, 7, damage);
            hit.target_hp_before = hp_before;
            hit.target_hp_after = hp_after;
            hit.target_max_hp = 2_220_578.0;
            apply_engine_event(&mut state, EngineEvent::Hit(Box::new(hit)));
        }

        assert!(
            state
                .hits
                .iter()
                .all(|hit| hit.target_name.as_deref() == Some("随心泥"))
        );
        assert!(state.hits.iter().skip(1).any(|hit| {
            hit.target_context
                .iter()
                .any(|context| context == "target_name_resolution=enemy_telemetry_continuity")
        }));
    }

    #[test]
    fn enemy_telemetry_disables_continuity_for_same_max_hp_multi_target_state() {
        let mut state = CombatState::default();
        apply_engine_event(
            &mut state,
            EngineEvent::ModScript(enemy_identity_event(37.0)),
        );
        apply_engine_event(
            &mut state,
            EngineEvent::ModScript(enemy_vitals_event_for(37.0, 0x1234, 1_000.0, 1_000.0)),
        );
        let mut first = test_hit(37.1, 7, 100.0);
        first.target_hp_before = 1_000.0;
        first.target_hp_after = 900.0;
        first.target_max_hp = 1_000.0;
        apply_engine_event(&mut state, EngineEvent::Hit(Box::new(first)));

        apply_engine_event(
            &mut state,
            EngineEvent::ModScript(enemy_identity_event_for(
                37.15,
                0x5678,
                0x2222,
                "Monster_002_BP",
                "Monster_2",
                "Second target",
                "第二目标",
                "第2ターゲット",
            )),
        );
        apply_engine_event(
            &mut state,
            EngineEvent::ModScript(enemy_vitals_event_for(37.15, 0x5678, 700.0, 1_000.0)),
        );
        apply_engine_event(
            &mut state,
            EngineEvent::ModScript(enemy_vitals_event_for(37.16, 0x1234, 600.0, 1_000.0)),
        );
        let mut ambiguous = test_hit(37.2, 7, 100.0);
        ambiguous.target_hp_before = 900.0;
        ambiguous.target_hp_after = 800.0;
        ambiguous.target_max_hp = 1_000.0;
        apply_engine_event(&mut state, EngineEvent::Hit(Box::new(ambiguous)));

        assert!(state.hits[1].target_name.is_none());
    }

    #[test]
    fn enemy_telemetry_leaves_identical_multi_target_hp_streams_unnamed() {
        let mut state = CombatState::default();
        apply_engine_event(
            &mut state,
            EngineEvent::ModScript(enemy_identity_event(40.0)),
        );
        apply_engine_event(
            &mut state,
            EngineEvent::ModScript(enemy_vitals_event_for(40.0, 0x1234, 1_000.0, 1_000.0)),
        );
        apply_engine_event(
            &mut state,
            EngineEvent::ModScript(enemy_identity_event_for(
                40.0,
                0x5678,
                0x2222,
                "Monster_002_BP",
                "Monster_2",
                "Second target",
                "第二目标",
                "第2ターゲット",
            )),
        );
        apply_engine_event(
            &mut state,
            EngineEvent::ModScript(enemy_vitals_event_for(40.0, 0x5678, 1_000.0, 1_000.0)),
        );

        let mut ambiguous = test_hit(40.1, 7, 100.0);
        ambiguous.target_hp_before = 1_000.0;
        ambiguous.target_hp_after = 900.0;
        ambiguous.target_max_hp = 1_000.0;
        apply_engine_event(&mut state, EngineEvent::Hit(Box::new(ambiguous)));

        assert!(state.hits[0].target_name.is_none());
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
        assert_eq!(state.packet_count, 0);
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
    fn packet_observation_and_debug_payload_count_once() {
        let mut state = CombatState::default();
        apply_engine_event(
            &mut state,
            EngineEvent::PacketObservation(PacketObservation { parsed_hits: 1 }),
        );
        apply_engine_event(&mut state, EngineEvent::Packet(Box::new(test_packet())));

        assert_eq!(state.packets.len(), 1);
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
            EngineEvent::TimeStop(TimeStopEvent::GamePauseStarted {
                timestamp: 1.0,
                pause_type_mask: 1 << 2,
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
