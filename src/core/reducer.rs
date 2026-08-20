//! The single `EngineEvent` -> `CombatState` merge point. Both the GUI event
//! loop and the CLI core loop route every engine event through
//! [`apply_engine_event`]; neither frontend may keep its own full match over
//! `EngineEvent` domain-state updates.

use crate::engine::model::{
    AbyssEvent, CombatState, EngineEvent, ModScriptApplyOutcome, ModScriptEvent,
};

/// What the caller still has to do after the domain state was updated.
/// Frontend-only side effects (toasts, cache invalidation, thread cleanup,
/// event forwarding) key off this instead of re-matching the event.
#[derive(Debug, PartialEq)]
pub enum CoreSignal {
    /// The event was valid but did not match or alter retained authoritative
    /// state. Callers must not advance combat revisions for this outcome.
    Unchanged,
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
    /// The game-side combat-clock provider changed availability/degradation.
    CombatClockHealthChanged,
    /// A typed script bridge message for frontend pre/post-processing.
    /// `state_changed` is set only when the applied event actually mutated a
    /// combat projection; revision bumps must key off that outcome, never off
    /// the event kind alone.
    ModScript {
        event: Box<ModScriptEvent>,
        state_changed: bool,
    },
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
            if state.apply_follow_up(follow_up) {
                CoreSignal::StateChanged
            } else {
                CoreSignal::Unchanged
            }
        }
        EngineEvent::HitDamageCorrection(correction) => {
            if state.apply_damage_correction(correction) {
                CoreSignal::StateChanged
            } else {
                CoreSignal::Unchanged
            }
        }
        EngineEvent::UnattributedServerDamage(observation) => {
            if state.observe_unattributed_server_damage(observation) {
                CoreSignal::StateChanged
            } else {
                CoreSignal::Unchanged
            }
        }
        EngineEvent::Packet(packet) => {
            if state.push_packet(*packet) {
                CoreSignal::DebugPacket
            } else {
                CoreSignal::Unchanged
            }
        }
        EngineEvent::PacketObservation(observation) => {
            state.observe_packet(observation);
            CoreSignal::PacketObserved
        }
        EngineEvent::Abyss(event) => {
            if matches!(&event, AbyssEvent::RestartDetected { .. })
                && state.abyss.active_half.is_none()
                && state.abyss.exited_at.is_some()
            {
                state.abyss = Default::default();
            }
            state.apply_abyss_event(event);
            CoreSignal::StateChanged
        }
        EngineEvent::TimeStop(event) => {
            state.apply_time_stop_event(event);
            CoreSignal::StateChanged
        }
        EngineEvent::CombatClockHealth(health) => {
            if state.set_combat_clock_health(health) {
                CoreSignal::CombatClockHealthChanged
            } else {
                CoreSignal::Unchanged
            }
        }
        EngineEvent::EmptyCurtain(items) => {
            if state.replace_empty_curtain(items) {
                CoreSignal::InventoryReplaced
            } else {
                CoreSignal::Unchanged
            }
        }
        EngineEvent::EmptyCurtainCharacters(characters) => {
            if state.replace_empty_curtain_characters(characters) {
                CoreSignal::InventoryCharactersReplaced
            } else {
                CoreSignal::Unchanged
            }
        }
        EngineEvent::ModScript(event) => {
            let outcome = state.apply_mod_script_event(&event);
            CoreSignal::ModScript {
                event: Box::new(event),
                state_changed: outcome == ModScriptApplyOutcome::ProjectionChanged,
            }
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
        AbyssEvent, CombatClockRuntimeHealth, EmptyCurtainCharacter, EmptyCurtainItem,
        EnemyIdentity, Hit, HitCharacterSource, HitDamageCorrection, HitDirection, HitFollowUp,
        HtItemNetId, ModScriptEventPhase, PacketDebug, PacketObservation, TimeStopEvent,
        UnattributedServerDamage,
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
    fn mod_script_event_without_backfill_does_not_change_combat_state() {
        let mut state = CombatState::default();
        let event = ModScriptEvent::from_bridge(
            4,
            9,
            "example".to_owned(),
            "pre.hit".to_owned(),
            vec![7, 8],
        );

        let signal = apply_engine_event(&mut state, EngineEvent::ModScript(event.clone()));

        assert_eq!(
            signal,
            CoreSignal::ModScript {
                event: Box::new(event),
                state_changed: false
            }
        );
        assert!(state.hits.is_empty());
        assert_eq!(state.total_damage, 0.0);
    }

    #[test]
    fn mod_script_backfill_reports_projection_change_for_revision_bump() {
        let mut state = CombatState::default();
        apply_engine_event(
            &mut state,
            EngineEvent::ModScript(enemy_identity_event(10.0)),
        );
        apply_engine_event(
            &mut state,
            EngineEvent::Hit(Box::new(test_hit(10.02, 7, 100.0))),
        );
        let generation = state.hits_generation;

        let signal = apply_engine_event(
            &mut state,
            EngineEvent::ModScript(enemy_hit_target_event_for(
                60,
                10.04,
                0x1234,
                0x4d88_7b49_05d5_dbaf,
                "Boss_016_BP",
                "Boss_16",
                "Imaginadough",
                "随心泥",
                "イメージクレイ",
            )),
        );

        match signal {
            CoreSignal::ModScript { state_changed, .. } => {
                assert!(
                    state_changed,
                    "backfill must be reported as a projection change"
                );
            }
            _ => panic!("expected a ModScript signal"),
        }
        assert_eq!(state.hits[0].target_name.as_deref(), Some("随心泥"));
        assert_ne!(state.hits_generation, generation);
    }

    #[test]
    fn duplicate_mod_script_backfill_is_reported_as_unchanged() {
        let mut state = CombatState::default();
        apply_engine_event(
            &mut state,
            EngineEvent::ModScript(enemy_identity_event(20.0)),
        );
        apply_engine_event(
            &mut state,
            EngineEvent::Hit(Box::new(test_hit(20.02, 7, 100.0))),
        );
        let event = enemy_hit_target_event_for(
            70,
            20.04,
            0x1234,
            0x4d88_7b49_05d5_dbaf,
            "Boss_016_BP",
            "Boss_16",
            "Imaginadough",
            "随心泥",
            "イメージクレイ",
        );
        let first = apply_engine_event(&mut state, EngineEvent::ModScript(event.clone()));
        let generation = state.hits_generation;
        let second = apply_engine_event(&mut state, EngineEvent::ModScript(event));

        match first {
            CoreSignal::ModScript { state_changed, .. } => assert!(state_changed),
            _ => panic!("expected a ModScript signal"),
        }
        match second {
            CoreSignal::ModScript { state_changed, .. } => {
                assert!(
                    !state_changed,
                    "idempotent backfill must not bump revisions"
                );
            }
            _ => panic!("expected a ModScript signal"),
        }
        assert_eq!(state.hits_generation, generation);
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

    #[allow(clippy::too_many_arguments)]
    fn enemy_hit_target_event_for(
        sequence: u64,
        timestamp: f64,
        target: u64,
        config_hash: u64,
        config_id: &str,
        monster_id: &str,
        name_en: &str,
        name_zh: &str,
        name_ja: &str,
    ) -> ModScriptEvent {
        let mut event = enemy_identity_event_for(
            timestamp,
            target,
            config_hash,
            config_id,
            monster_id,
            name_en,
            name_zh,
            name_ja,
        );
        event.sequence = sequence;
        event.phase = ModScriptEventPhase::Postprocess;
        event.name = "enemy.hit_target".to_owned();
        event
    }

    #[test]
    fn current_target_samples_do_not_claim_damage_without_an_exact_instance() {
        let mut state = CombatState::default();
        apply_engine_event(
            &mut state,
            EngineEvent::ModScript(enemy_identity_event(10.0)),
        );
        apply_engine_event(
            &mut state,
            EngineEvent::Hit(Box::new(test_hit(10.02, 7, 100.0))),
        );
        apply_engine_event(
            &mut state,
            EngineEvent::ModScript(enemy_identity_event(10.05)),
        );

        assert!(state.hits[0].target_name.is_none());
        assert!(state.hits[0].target_id.is_none());
    }

    #[test]
    fn exact_hit_targets_split_simultaneous_group_damage_by_instance() {
        let mut state = CombatState::default();
        apply_engine_event(
            &mut state,
            EngineEvent::Abyss(AbyssEvent::Stage {
                timestamp: 89.0,
                cycle: Some(1),
                floor: Some(11),
                half: crate::engine::model::AbyssHalf::First,
                allow_late_backfill: false,
            }),
        );
        apply_engine_event(
            &mut state,
            EngineEvent::ModScript(enemy_identity_event(90.0)),
        );

        let mut first = test_hit(90.02, 7, 100.0);
        first.byte_offset = 10;
        let mut second = test_hit(90.02, 7, 200.0);
        second.byte_offset = 20;
        apply_engine_event(&mut state, EngineEvent::Hit(Box::new(first)));
        apply_engine_event(&mut state, EngineEvent::Hit(Box::new(second)));
        apply_engine_event(
            &mut state,
            EngineEvent::ModScript(enemy_identity_event(90.05)),
        );

        apply_engine_event(
            &mut state,
            EngineEvent::ModScript(enemy_hit_target_event_for(
                10,
                90.06,
                0xaaaa,
                0x4d88_7b49_05d5_dbaf,
                "Boss_016_BP",
                "Boss_16",
                "Imaginadough",
                "随心泥",
                "イメージクレイ",
            )),
        );
        apply_engine_event(
            &mut state,
            EngineEvent::ModScript(enemy_hit_target_event_for(
                11,
                90.06,
                0xbbbb,
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
            EngineEvent::ModScript(enemy_identity_event(90.07)),
        );

        assert_eq!(state.hits[0].target_name.as_deref(), Some("随心泥"));
        assert_eq!(state.hits[1].target_name.as_deref(), Some("第二目标"));
        assert_eq!(
            state.hits[0].target_id.as_deref(),
            Some("enemy-instance:000000000000aaaa")
        );
        assert_eq!(
            state.hits[1].target_id.as_deref(),
            Some("enemy-instance:000000000000bbbb")
        );
        assert_ne!(state.hits[0].target_id, state.hits[1].target_id);
        assert!(
            state.hits[0]
                .target_context
                .iter()
                .any(|context| context == "enemy_target_instance=000000000000aaaa")
        );
        assert!(
            state.hits[1]
                .target_context
                .iter()
                .any(|context| context == "enemy_target_instance=000000000000bbbb")
        );
        assert!(state.hits.iter().all(|hit| {
            hit.target_context
                .iter()
                .any(|context| context == "target_name_resolution=enemy_telemetry_hit_instance")
        }));
        assert!(
            state.abyss.first_half.hits[1]
                .target_context
                .iter()
                .any(|context| context == "enemy_target_instance=000000000000bbbb")
        );
    }

    #[test]
    fn exact_hit_targets_queue_until_the_matching_hits_arrive() {
        let mut state = CombatState::default();
        apply_engine_event(
            &mut state,
            EngineEvent::ModScript(enemy_hit_target_event_for(
                20,
                100.0,
                0xaaaa,
                0x4d88_7b49_05d5_dbaf,
                "Boss_016_BP",
                "Boss_16",
                "Imaginadough",
                "随心泥",
                "イメージクレイ",
            )),
        );
        apply_engine_event(
            &mut state,
            EngineEvent::ModScript(enemy_hit_target_event_for(
                21,
                100.0,
                0xbbbb,
                0x2222,
                "Monster_002_BP",
                "Monster_2",
                "Second target",
                "第二目标",
                "第2ターゲット",
            )),
        );

        let mut first = test_hit(100.02, 7, 100.0);
        first.byte_offset = 10;
        first.target_max_hp = 0.0;
        let mut second = test_hit(100.02, 7, 200.0);
        second.byte_offset = 20;
        second.target_max_hp = 0.0;
        apply_engine_event(&mut state, EngineEvent::Hit(Box::new(first)));
        apply_engine_event(&mut state, EngineEvent::Hit(Box::new(second)));

        assert_eq!(state.hits[0].target_name.as_deref(), Some("随心泥"));
        assert_eq!(state.hits[1].target_name.as_deref(), Some("第二目标"));
        assert_eq!(
            state.hits[0].target_id.as_deref(),
            Some("enemy-instance:000000000000aaaa")
        );
        assert_eq!(
            state.hits[1].target_id.as_deref(),
            Some("enemy-instance:000000000000bbbb")
        );
    }

    #[test]
    fn exact_hit_target_does_not_claim_a_hit_outside_its_time_window() {
        let mut state = CombatState::default();
        apply_engine_event(
            &mut state,
            EngineEvent::Hit(Box::new(test_hit(110.0, 7, 100.0))),
        );
        apply_engine_event(
            &mut state,
            EngineEvent::ModScript(enemy_hit_target_event_for(
                30,
                110.351,
                0xaaaa,
                0x4d88_7b49_05d5_dbaf,
                "Boss_016_BP",
                "Boss_16",
                "Imaginadough",
                "随心泥",
                "イメージクレイ",
            )),
        );

        assert!(state.hits[0].target_name.is_none());
        apply_engine_event(
            &mut state,
            EngineEvent::Hit(Box::new(test_hit(110.4, 7, 200.0))),
        );
        assert_eq!(state.hits[1].target_name.as_deref(), Some("随心泥"));
    }

    #[test]
    fn exact_hit_target_backfill_promotes_candidate_output_and_rebuilds_totals() {
        let mut state = CombatState::default();
        apply_engine_event(
            &mut state,
            EngineEvent::Abyss(AbyssEvent::Stage {
                timestamp: 59.0,
                cycle: Some(1),
                floor: Some(11),
                half: crate::engine::model::AbyssHalf::First,
                allow_late_backfill: false,
            }),
        );
        let mut candidate = test_hit(60.02, 7, 100.0);
        candidate.direction = HitDirection::Unknown;
        apply_engine_event(&mut state, EngineEvent::Hit(Box::new(candidate)));
        assert_eq!(
            state
                .stats
                .get(&7)
                .expect("candidate stats should exist")
                .attributed_hits,
            0
        );
        let generation = state.hits_generation;
        let party_generation = state.abyss.first_half.hits_generation;

        apply_engine_event(
            &mut state,
            EngineEvent::ModScript(enemy_hit_target_event_for(
                40,
                60.05,
                0x1234,
                0x4d88_7b49_05d5_dbaf,
                "Boss_016_BP",
                "Boss_16",
                "Imaginadough",
                "随心泥",
                "イメージクレイ",
            )),
        );

        assert_eq!(state.hits[0].direction, HitDirection::Outgoing);
        assert_eq!(state.hits[0].target_name.as_deref(), Some("随心泥"));
        assert_eq!(state.hits_generation, generation.wrapping_add(1));
        assert_eq!(
            state
                .stats
                .get(&7)
                .expect("backfilled stats should exist")
                .attributed_hits,
            1
        );
        assert_eq!(
            state.abyss.first_half.hits[0].target_name.as_deref(),
            Some("随心泥")
        );
        assert_eq!(
            state.abyss.first_half.hits_generation,
            party_generation.wrapping_add(1)
        );
    }

    #[test]
    fn exact_hit_targets_do_not_project_incoming_damage() {
        let mut state = CombatState::default();
        let mut incoming = test_hit(80.02, 7, 100.0);
        incoming.direction = HitDirection::Incoming;
        apply_engine_event(&mut state, EngineEvent::Hit(Box::new(incoming)));
        apply_engine_event(
            &mut state,
            EngineEvent::ModScript(enemy_hit_target_event_for(
                50,
                80.05,
                0x1234,
                0x4d88_7b49_05d5_dbaf,
                "Boss_016_BP",
                "Boss_16",
                "Imaginadough",
                "随心泥",
                "イメージクレイ",
            )),
        );

        assert!(state.hits[0].target_name.is_none());
        assert_eq!(state.hits[0].direction, HitDirection::Incoming);
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
    fn unattributed_server_damage_is_explicit_but_does_not_pollute_totals_or_roles() {
        let mut state = CombatState::default();
        apply_engine_event(
            &mut state,
            EngineEvent::Hit(Box::new(test_hit(1.0, 7, 100.0))),
        );

        let signal = apply_engine_event(
            &mut state,
            EngineEvent::UnattributedServerDamage(UnattributedServerDamage {
                timestamp: 1.1,
                damage: 250.0,
                candidate_hits: 0,
            }),
        );

        assert_eq!(signal, CoreSignal::StateChanged);
        assert_eq!(state.unattributed_server_damage_events, 1);
        assert_eq!(state.unattributed_server_damage, 250.0);
        assert_eq!(state.total_damage, 100.0);
        assert_eq!(state.stats.get(&7).map(|row| row.damage), Some(100.0));

        let invalid = apply_engine_event(
            &mut state,
            EngineEvent::UnattributedServerDamage(UnattributedServerDamage {
                timestamp: f64::NAN,
                damage: 1.0,
                candidate_hits: 0,
            }),
        );
        assert_eq!(invalid, CoreSignal::Unchanged);
        assert_eq!(state.unattributed_server_damage_events, 1);
    }

    #[test]
    fn unmatched_or_identical_hit_mutations_do_not_advance_combat_state() {
        let mut state = CombatState::default();
        apply_engine_event(
            &mut state,
            EngineEvent::Hit(Box::new(test_hit(1.0, 7, 100.0))),
        );
        let generation = state.hits_generation;
        let total_damage = state.total_damage;

        let unmatched = apply_engine_event(
            &mut state,
            EngineEvent::HitFollowUp(HitFollowUp {
                source_timestamp: 99.0,
                source_char_id: 7,
                source_damage: 100.0,
                source_target_hp_before: 0.0,
                source_target_hp_after: 0.0,
                source_target_max_hp: 0.0,
                source_gameplay_effect_index: None,
                timestamp: 99.1,
                damage: 25.0,
                target_hp_after: 0.0,
                target_hp_percent: 0.0,
                damage_name: None,
                attack_type: None,
                damage_attribute: None,
            }),
        );
        assert_eq!(unmatched, CoreSignal::Unchanged);

        let identical = apply_engine_event(
            &mut state,
            EngineEvent::HitDamageCorrection(HitDamageCorrection {
                source_timestamp: 1.0,
                source_char_id: 7,
                source_damage: 100.0,
                source_target_hp_before: 0.0,
                source_target_hp_after: 0.0,
                source_target_max_hp: 0.0,
                source_gameplay_effect_index: None,
                damage: 100.0,
                target_hp_before: 0.0,
                target_hp_after: 0.0,
                target_hp_percent: 0.0,
            }),
        );
        assert_eq!(identical, CoreSignal::Unchanged);
        assert_eq!(state.hits_generation, generation);
        assert_eq!(state.total_damage, total_damage);
        assert_eq!(state.damage_correction_count, 0);
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
    fn oversized_debug_packet_is_a_noop_instead_of_breaking_the_byte_budget() {
        let mut state = CombatState::default();
        let mut packet = test_packet();
        packet.payload_hex = String::with_capacity(17 * 1024 * 1024);

        let signal = apply_engine_event(&mut state, EngineEvent::Packet(Box::new(packet)));

        assert_eq!(signal, CoreSignal::Unchanged);
        assert!(state.packets.is_empty());
        assert_eq!(state.packets_generation, 0);
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
    fn abyss_restart_after_exit_resets_previous_party_ownership() {
        let mut state = CombatState::default();
        apply_engine_event(
            &mut state,
            EngineEvent::Abyss(AbyssEvent::Stage {
                timestamp: 1.0,
                cycle: Some(1),
                floor: Some(12),
                half: crate::engine::model::AbyssHalf::First,
                allow_late_backfill: false,
            }),
        );
        apply_engine_event(
            &mut state,
            EngineEvent::Hit(Box::new(test_hit(2.0, 1, 100.0))),
        );
        apply_engine_event(
            &mut state,
            EngineEvent::Abyss(AbyssEvent::Stage {
                timestamp: 3.0,
                cycle: Some(1),
                floor: Some(12),
                half: crate::engine::model::AbyssHalf::Second,
                allow_late_backfill: false,
            }),
        );
        apply_engine_event(
            &mut state,
            EngineEvent::Hit(Box::new(test_hit(4.0, 2, 200.0))),
        );
        apply_engine_event(
            &mut state,
            EngineEvent::Abyss(AbyssEvent::Exit { timestamp: 5.0 }),
        );

        apply_engine_event(
            &mut state,
            EngineEvent::Abyss(AbyssEvent::RestartDetected { timestamp: 6.0 }),
        );
        apply_engine_event(
            &mut state,
            EngineEvent::Abyss(AbyssEvent::Stage {
                timestamp: 7.0,
                cycle: Some(2),
                floor: Some(12),
                half: crate::engine::model::AbyssHalf::First,
                allow_late_backfill: false,
            }),
        );
        apply_engine_event(
            &mut state,
            EngineEvent::Hit(Box::new(test_hit(8.0, 2, 300.0))),
        );

        assert_eq!(state.abyss.first_half.hits.len(), 1);
        assert_eq!(state.abyss.first_half.hits[0].char_id, 2);
        assert_eq!(state.abyss.first_half.hits[0].damage, 300.0);
        assert!(state.abyss.second_half.hits.is_empty());
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
        let generation = state.empty_curtain_generation;
        let duplicate_items = state.empty_curtain.clone();
        let duplicate = apply_engine_event(&mut state, EngineEvent::EmptyCurtain(duplicate_items));
        assert_eq!(duplicate, CoreSignal::Unchanged);
        assert_eq!(state.empty_curtain_generation, generation);
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
        let generation = state.empty_curtain_characters_generation;
        let duplicate = apply_engine_event(
            &mut state,
            EngineEvent::EmptyCurtainCharacters(vec![character]),
        );
        assert_eq!(duplicate, CoreSignal::Unchanged);
        assert_eq!(state.empty_curtain_characters_generation, generation);
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

    #[test]
    fn combat_clock_health_only_changes_revision_effect_on_transition() {
        let mut state = CombatState::default();
        assert_eq!(
            apply_engine_event(
                &mut state,
                EngineEvent::CombatClockHealth(CombatClockRuntimeHealth::ProviderUnavailable),
            ),
            CoreSignal::CombatClockHealthChanged
        );
        assert_eq!(
            apply_engine_event(
                &mut state,
                EngineEvent::CombatClockHealth(CombatClockRuntimeHealth::ProviderUnavailable),
            ),
            CoreSignal::Unchanged
        );
    }
}
