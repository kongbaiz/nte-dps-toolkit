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
