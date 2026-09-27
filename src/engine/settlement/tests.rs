use super::*;

fn actor(id: u32) -> ActorRef {
    ActorRef {
        flags: 1,
        name: Some(Name::Text {
            text: id.to_string(),
            number: 0,
        }),
        fields: vec![],
    }
}
fn enemy(tail: u32) -> ActorRef {
    ActorRef {
        flags: 0,
        name: None,
        fields: vec![
            (0, 7),
            (4, 8),
            (8, 9),
            (12, 10),
            (16, 0),
            (20, 1),
            (24, tail),
        ],
    }
}
fn key() -> MessageKey {
    MessageKey {
        channel: 3,
        message: 1,
        timestamp_bits: 123,
    }
}
fn request() -> Request {
    Request {
        key: key(),
        targets: vec![RequestTarget {
            source: actor(1),
            target: enemy(0),
            effect_index: Some(9),
            hp_before_bits: Some(800f32.to_bits()),
            max_hp_bits: Some(1000f32.to_bits()),
            calculated_damage_bits: Some(100f32.to_bits()),
        }],
    }
}
fn settlement() -> Settlement {
    Settlement {
        recoveries: vec![],
        key: key(),
        source: actor(1),
        targets: vec![SettledTarget {
            target: enemy(0),
            current_hp_bits: 650f32.to_bits(),
            dead_state: 0,
            shield_damage_bits: 0,
            lock_target: 0,
            components: vec![Component {
                damage: 150,
                display_type: 0,
            }],
        }],
    }
}
fn ledger() -> Ledger {
    let mut catalog = SkillCatalog::default();
    catalog.effects.insert(
        9,
        vec![Skill {
            key: "GA_A".into(),
            name: None,
            owners: BTreeSet::from([1]),
        }],
    );
    Ledger::new(32, catalog)
}

fn hp_ledger() -> Ledger {
    let mut l = ledger();
    l.catalog.mechanics.insert(
        9,
        EffectMechanic {
            unbalance_label: false,
            effect_name: "GE_fixture".into(),
            display_name: Some("Nightmare".into()),
            max_hp_reduction_percent: 200,
            owner: Some(1),
        },
    );
    let mut seed = settlement();
    seed.key.message = 0;
    seed.targets[0].current_hp_bits = 1000f32.to_bits();
    seed.targets[0].components[0].damage = 50;
    l.settlement(seed).unwrap();
    l
}
fn hp_settlement() -> Settlement {
    let mut s = settlement();
    s.targets[0].current_hp_bits = 720f32.to_bits();
    s.targets[0].components[0].damage = 100;
    s
}

#[test]
fn max_hp_scaling_uses_server_predecessor_not_stale_client_hp_or_prediction() {
    let mut l = hp_ledger();
    let mut r = request();
    r.targets[0].hp_before_bits = Some(500f32.to_bits());
    r.targets[0].calculated_damage_bits = Some(99999f32.to_bits());
    l.request(r).unwrap();
    let change = l.settlement(hp_settlement()).unwrap().unwrap();
    let a = change.rows[0].hp_adjustment.as_ref().unwrap();
    assert_eq!(a.hp_before_bits, 1000f32.to_bits());
    assert_eq!(a.reduction(), 200.0);
    assert_eq!(a.additional_loss(720f32.to_bits()), 180.0);
    assert_eq!(change.rows[0].damage, 100);
    assert_eq!(change.rows[0].mechanic.as_deref(), Some("Nightmare"));
    assert!(l.settlement(hp_settlement()).unwrap().is_none());
}

#[test]
fn late_request_uses_frozen_hp_witness_after_newer_messages_have_arrived() {
    let mut l = hp_ledger();
    assert!(
        l.settlement(hp_settlement()).unwrap().unwrap().rows[0]
            .hp_adjustment
            .is_none()
    );
    let mut newer = settlement();
    newer.key.message = 2;
    newer.targets[0].current_hp_bits = 100f32.to_bits();
    l.settlement(newer).unwrap();
    let change = l.request(request()).unwrap().unwrap();
    let a = change.rows[0].hp_adjustment.as_ref().unwrap();
    assert_eq!(a.hp_before_bits, 1000f32.to_bits());
    assert_eq!(a.additional_loss(720f32.to_bits()), 180.0);
}

#[test]
fn repeated_target_rows_do_not_apply_max_hp_rule_to_an_ordinary_following_component() {
    let mut l = hp_ledger();
    let mut r = request();
    let mut child = r.targets[0].clone();
    child.source = actor(2);
    child.effect_index = Some(10);
    r.targets.push(child);
    l.request(r).unwrap();
    let mut s = hp_settlement();
    let mut second = s.targets[0].clone();
    second.current_hp_bits = 640f32.to_bits();
    second.components[0].damage = 80;
    s.targets.push(second);
    let c = l.settlement(s).unwrap().unwrap();
    assert_eq!(c.rows.len(), 2);
    assert!(c.rows[0].hp_adjustment.is_some());
    assert!(c.rows[1].hp_adjustment.is_none());
}

#[test]
fn nonmatching_hp_owner_or_multiple_damage_components_are_not_guessed() {
    for case in 0..3 {
        let mut l = hp_ledger();
        l.request(request()).unwrap();
        let mut s = hp_settlement();
        match case {
            0 => s.targets[0].current_hp_bits = 719f32.to_bits(),
            1 => l.catalog.mechanics.get_mut(&9).unwrap().owner = Some(2),
            _ => s.targets[0].components.push(Component {
                damage: 1,
                display_type: 0,
            }),
        }
        assert!(
            l.settlement(s)
                .unwrap()
                .unwrap()
                .rows
                .iter()
                .all(|r| r.hp_adjustment.is_none())
        );
    }
}

#[test]
fn conflicting_server_witness_retracts_only_derived_child_contribution() {
    let mut l = hp_ledger();
    l.request(request()).unwrap();
    assert!(
        l.settlement(hp_settlement()).unwrap().unwrap().rows[0]
            .hp_adjustment
            .is_some()
    );
    let mut conflict = settlement();
    conflict.key.message = 0;
    conflict.targets[0].current_hp_bits = 999f32.to_bits();
    l.settlement(conflict.clone()).unwrap();
    let updates = l.take_hp_updates();
    assert_eq!(updates.len(), 1);
    assert_eq!(updates[0].key, key());
    assert_eq!(updates[0].rows[0].damage, 100);
    assert!(updates[0].rows[0].hp_adjustment.is_none());
    assert!(l.settlement(conflict).unwrap().is_none());
    assert!(l.take_hp_updates().is_empty());
}

#[test]
fn missing_or_interrupted_server_hp_continuity_remains_unknown() {
    let mut l = hp_ledger();
    l.invalidate_hp_continuity();
    l.request(request()).unwrap();
    assert!(
        l.settlement(hp_settlement()).unwrap().unwrap().rows[0]
            .hp_adjustment
            .is_none()
    );
    let mut l = hp_ledger();
    l.clear();
    l.request(request()).unwrap();
    assert!(
        l.settlement(hp_settlement()).unwrap().unwrap().rows[0]
            .hp_adjustment
            .is_none()
    );
}

#[test]
fn request_never_creates_damage() {
    assert_eq!(ledger().request(request()), Ok(None));
}

#[test]
fn server_values_override_neither_request_nor_hp_by_arithmetic() {
    let mut l = ledger();
    l.request(request()).unwrap();
    let c = l.settlement(settlement()).unwrap().unwrap();
    assert_eq!(c.rows[0].damage, 150);
    assert_eq!(c.rows[0].current_hp_bits, 650f32.to_bits());
    assert_eq!(c.rows[0].max_hp_at_request_bits, Some(1000f32.to_bits()));
    assert_eq!(c.rows[0].skill_key.as_deref(), Some("GA_A"));
}

#[test]
fn late_request_replaces_projection_not_an_additional_hit() {
    let mut l = ledger();
    let first = l.settlement(settlement()).unwrap().unwrap();
    assert_eq!(first.rows[0].max_hp_at_request_bits, None);
    let update = l.request(request()).unwrap().unwrap();
    assert_eq!(first.key, update.key);
    assert_eq!(first.rows[0].damage, update.rows[0].damage);
    assert_eq!(
        update.rows[0].max_hp_at_request_bits,
        Some(1000f32.to_bits())
    );
}

#[test]
fn repeated_settlement_does_not_emit_twice() {
    let mut l = ledger();
    assert!(l.settlement(settlement()).unwrap().is_some());
    assert_eq!(l.settlement(settlement()), Ok(None));
}

#[test]
fn conflicting_settlement_retracts_and_quarantines() {
    let mut l = ledger();
    l.settlement(settlement()).unwrap();
    let mut other = settlement();
    other.targets[0].components[0].damage = 151;
    let c = l.settlement(other).unwrap().unwrap();
    assert!(c.rows.is_empty());
    assert_eq!(c.conflict, Some(Error::ConflictingSettlement));
    assert_eq!(l.settlement(settlement()), Ok(None));
}

#[test]
fn conflicting_request_clears_only_enrichment() {
    let mut l = ledger();
    l.request(request()).unwrap();
    l.settlement(settlement()).unwrap();
    let mut other = request();
    other.targets[0].max_hp_bits = Some(1200f32.to_bits());
    let c = l.request(other).unwrap().unwrap();
    assert_eq!(c.rows[0].damage, 150);
    assert_eq!(c.rows[0].max_hp_at_request_bits, None);
    assert_eq!(c.rows[0].skill_key, None);
}

#[test]
fn same_species_tail_fields_and_order_are_distinct() {
    let mut r = request();
    let mut second = r.targets[0].clone();
    second.target = enemy(1);
    second.max_hp_bits = Some(2000f32.to_bits());
    r.targets.push(second);
    let mut s = settlement();
    let mut second = s.targets[0].clone();
    second.target = enemy(1);
    s.targets.insert(0, second);
    let mut l = ledger();
    l.request(r).unwrap();
    let c = l.settlement(s).unwrap().unwrap();
    assert_eq!(c.rows.len(), 2);
    assert_eq!(c.rows[0].max_hp_at_request_bits, Some(2000f32.to_bits()));
    assert_eq!(c.rows[1].max_hp_at_request_bits, Some(1000f32.to_bits()));
}

#[test]
fn reaction_is_a_separate_component_not_trigger_skill() {
    let mut s = settlement();
    s.targets[0].components.push(Component {
        damage: 30,
        display_type: 24,
    });
    let mut l = ledger();
    l.request(request()).unwrap();
    let c = l.settlement(s).unwrap().unwrap();
    assert_eq!(c.rows.len(), 2);
    assert_eq!(
        c.rows[1].attribution,
        Attribution::ExplicitSettlementCategory
    );
    assert_eq!(c.rows[1].skill_key, None);
}

#[test]
fn owner_table_never_reassigns_source() {
    let mut r = request();
    r.targets[0].source = actor(2);
    let mut s = settlement();
    s.source = actor(2);
    let mut l = ledger();
    l.request(r).unwrap();
    let c = l.settlement(s).unwrap().unwrap();
    assert_eq!(c.rows[0].character_id, Some(2));
    assert_eq!(c.rows[0].attribution, Attribution::OwnerConflict);
}

#[test]
fn unresolved_effect_candidate_cannot_be_dropped_to_manufacture_consensus() {
    let mut l = ledger();
    l.catalog.unresolved_effects.insert(9);
    l.request(request()).unwrap();
    let c = l.settlement(settlement()).unwrap().unwrap();
    assert_eq!(c.rows[0].attribution, Attribution::SkillUnresolved);
    assert_eq!(c.rows[0].damage, 150);
}

#[test]
fn timestamp_is_exact_not_nearest() {
    let mut r = request();
    r.key.timestamp_bits += 1;
    let mut l = ledger();
    l.request(r).unwrap();
    assert_eq!(
        l.settlement(settlement()).unwrap().unwrap().rows[0].skill_key,
        None
    );
}

#[test]
fn capacity_failure_preserves_prior_dedup_and_explicit_reset() {
    let mut l = Ledger::new(1, SkillCatalog::default());
    l.settlement(settlement()).unwrap();
    let mut other = settlement();
    other.key.message += 1;
    assert_eq!(l.settlement(other), Err(Error::BudgetExceeded));
    assert_eq!(l.settlement(settlement()), Ok(None));
    l.clear();
    assert!(l.settlement(settlement()).unwrap().is_some());
}

#[test]
fn generation_instances_cannot_leak_request_metadata() {
    let mut old = ledger();
    old.request(request()).unwrap();
    let mut new = ledger();
    assert_eq!(
        new.settlement(settlement()).unwrap().unwrap().rows[0].max_hp_at_request_bits,
        None
    );
}

#[test]
fn raw_rpc_empty_truncated_and_oversized_are_errors() {
    for bits in [0, 1, 8, 64, usize::MAX] {
        assert!(decode_request(&[], bits, 3).is_err());
        assert!(decode_settlement(&[], bits, 3).is_err());
    }
}

#[test]
fn malformed_transport_does_not_poison_decoder() {
    let mut d = transport::Decoder::new(transport::Profile {
        component_prefix: 20,
        channel: 3,
        field_upper_exclusive: 219,
        request_index: 100,
        settlement_index: 142,
    })
    .unwrap();
    for data in [&[][..], &[0][..], &[1][..], &[20][..]] {
        assert!(d.datagram(data, true).is_err());
        assert!(!d.has_incomplete_fragments());
    }
}

#[test]
fn mixed_source_repeated_victim_uses_complete_ordinal_layout_not_parent_source() {
    let mut l = ledger();
    l.catalog.effects.insert(
        10,
        vec![Skill {
            key: "GA_B".into(),
            name: None,
            owners: BTreeSet::from([2]),
        }],
    );
    let mut r = request();
    let mut child = r.targets[0].clone();
    child.source = actor(2);
    child.effect_index = Some(10);
    child.calculated_damage_bits = Some(99999f32.to_bits());
    r.targets.push(child);
    let mut s = settlement();
    let mut second = s.targets[0].clone();
    second.components[0].damage = 37;
    second.components.push(Component {
        damage: 13,
        display_type: 25,
    });
    s.targets.push(second);
    let initial = l.settlement(s.clone()).unwrap().unwrap();
    assert_eq!(initial.rows[1].character_id, Some(1));
    let fixed = l.request(r.clone()).unwrap().unwrap();
    assert_eq!(fixed.rows.len(), 3);
    assert_eq!(fixed.rows[0].effect_candidates, vec![9]);
    assert_eq!(fixed.rows[1].source, actor(1));
    assert_eq!(fixed.rows[1].request_source, Some(actor(2)));
    assert_eq!(fixed.rows[1].character_id, Some(2));
    assert_eq!(fixed.rows[1].effect_candidates, vec![10]);
    assert_eq!(fixed.rows[1].skill_key.as_deref(), Some("GA_B"));
    assert_eq!(fixed.rows[1].damage, 37);
    assert_eq!(fixed.rows[2].damage, 13);
    assert_eq!(
        fixed.rows[2].attribution,
        Attribution::ExplicitSettlementCategory
    );
    assert!(l.request(r.clone()).unwrap().is_none());
    assert!(l.settlement(s).unwrap().is_none());
    r.targets[1].effect_index = Some(11);
    let conflict = l.request(r).unwrap().unwrap();
    assert_eq!(conflict.rows[1].character_id, Some(1));
    assert!(conflict.rows[1].request_source.is_none());
    assert!(conflict.rows[1].effect_candidates.is_empty());
    assert_eq!(conflict.rows[1].damage, 37);
}

#[test]
fn incomplete_repeated_victim_layout_cannot_reuse_the_parent_effect() {
    for wrong_tail in [false, true] {
        let mut l = ledger();
        let mut r = request();
        let mut s = settlement();
        s.targets.push(s.targets[0].clone());
        if wrong_tail {
            let mut child = r.targets[0].clone();
            child.target = enemy(99);
            child.source = actor(2);
            r.targets.push(child);
        }
        l.request(r).unwrap();
        let c = l.settlement(s).unwrap().unwrap();
        assert_eq!(c.rows.len(), 2);
        assert!(
            c.rows
                .iter()
                .all(|r| r.request_source.is_none() && r.effect_candidates.is_empty())
        );
        assert_eq!(c.rows.iter().map(|r| r.damage).sum::<i32>(), 300);
    }
}

#[test]
fn unbalance_subtype_labels_require_matching_semantics_and_do_not_replace_reactions() {
    for (category, named_unbalance, expected) in [
        (22, false, None),
        (22, true, Some("Extra Break")),
        (25, true, None),
    ] {
        let mut l = hp_ledger();
        let mechanic = l.catalog.mechanics.get_mut(&9).unwrap();
        mechanic.unbalance_label = named_unbalance;
        mechanic.display_name = Some("Extra Break".into());
        l.request(request()).unwrap();
        let mut s = settlement();
        s.targets[0].components[0].display_type = category;
        let change = l.settlement(s).unwrap().unwrap();
        let p = application::project(change, "break-test", "test", 1.0, &HashMap::new(), true);
        assert_eq!(p.hits[0].damage_component.as_deref(), expected);
        assert_eq!(p.hits[0].damage, 150.0);
        assert_eq!(p.hits[0].exact.as_ref().unwrap().display_type, category);
    }
}

#[test]
fn recovery_is_deduplicated_and_breaks_only_its_targets_hp_witness_chain() {
    let mut l = hp_ledger();
    let mut heal = settlement();
    heal.key.message = 7;
    heal.targets.clear();
    heal.recoveries.push(RecoveredTarget {
        target: enemy(0),
        current_hp_bits: 1000f32.to_bits(),
    });
    assert!(l.settlement(heal.clone()).unwrap().unwrap().rows.is_empty());
    assert!(l.settlement(heal).unwrap().is_none());
    l.request(request()).unwrap();
    let c = l.settlement(hp_settlement()).unwrap().unwrap();
    assert_eq!(c.rows[0].damage, 100);
    assert!(c.rows[0].hp_adjustment.is_none());
    let mut unrelated = hp_ledger();
    let mut heal = settlement();
    heal.key.message = 7;
    heal.targets.clear();
    heal.recoveries.push(RecoveredTarget {
        target: enemy(99),
        current_hp_bits: 2000f32.to_bits(),
    });
    unrelated.settlement(heal).unwrap();
    unrelated.request(request()).unwrap();
    assert!(
        unrelated.settlement(hp_settlement()).unwrap().unwrap().rows[0]
            .hp_adjustment
            .is_some()
    );
}
