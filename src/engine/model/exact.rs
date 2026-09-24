use super::*;
use crate::engine::settlement::application::Projection;

const MAX_EXACT_MESSAGES: usize = 100_000;

#[derive(Clone, Default)]
pub(super) struct Slot {
    positions: Vec<usize>,
    pub(super) quarantined: bool,
    retired: bool,
}

impl CombatState {
    pub fn has_exact_message(
        &self,
        identity: &crate::engine::settlement::application::MessageIdentity,
    ) -> bool {
        self.exact_index.contains_key(identity)
    }
    pub(super) fn retire_exact_messages_from(&mut self, previous: &CombatState) {
        for key in previous.exact_index.keys() {
            self.exact_index.insert(
                key.clone(),
                Slot {
                    positions: vec![],
                    quarantined: false,
                    retired: true,
                },
            );
        }
    }
    pub(crate) fn exact_quarantined_messages(
        &self,
    ) -> Vec<crate::engine::settlement::application::MessageIdentity> {
        self.exact_index
            .iter()
            .filter(|(_, s)| s.quarantined)
            .map(|(k, _)| k.clone())
            .collect()
    }
    pub(crate) fn restore_exact_quarantine(
        &mut self,
        keys: Vec<crate::engine::settlement::application::MessageIdentity>,
    ) {
        for key in keys {
            self.exact_index.insert(
                key,
                Slot {
                    positions: vec![],
                    quarantined: true,
                    retired: false,
                },
            );
        }
    }
    pub(super) fn rebuild_exact_index(&mut self) {
        // Rare history/structural mutation path, never per normal appended hit.
        for slot in self.exact_index.values_mut() {
            slot.positions.clear();
        }
        for (position, hit) in self.hits.iter().enumerate() {
            if let Some(e) = &hit.exact {
                self.exact_index
                    .entry(e.message.clone())
                    .or_default()
                    .positions
                    .push(position);
            }
        }
    }

    pub fn apply_exact_projection(
        &mut self,
        mut projection: Projection,
    ) -> Result<bool, &'static str> {
        if projection.identity.generation.is_empty()
            || projection.identity.generation.len() > 128
            || projection.identity.connection.is_empty()
            || projection.identity.connection.len() > 256
            || projection.hits.len() > 2048
        {
            return Err("exact_projection_invalid_identity_or_budget");
        }
        let mut ordinals = std::collections::HashSet::new();
        for h in &projection.hits {
            let e = h
                .exact
                .as_ref()
                .ok_or("exact_projection_missing_evidence")?;
            if e.message != projection.identity
                || !ordinals.insert((e.target_ordinal, e.component_ordinal))
                || !h.timestamp.is_finite()
                || !h.damage.is_finite()
                || h.damage <= 0.0
                || !f32::from_bits(e.current_hp_bits).is_finite()
                || e.max_hp_at_request_bits
                    .is_some_and(|v| !f32::from_bits(v).is_finite() || f32::from_bits(v) <= 0.0)
                || e.hp_before_request_bits
                    .is_some_and(|v| !f32::from_bits(v).is_finite() || f32::from_bits(v) < 0.0)
                || h.follow_up_damage != 0.0
                || h.wire_event.is_some()
            {
                return Err("exact_projection_invalid_row");
            }
        }
        if projection.quarantined && !projection.hits.is_empty() {
            return Err("exact_quarantine_contains_rows");
        }
        if self.exact_index.is_empty() && self.hits.iter().any(|h| h.exact.is_some()) {
            self.rebuild_exact_index();
        }
        if let Some(slot) = self.exact_index.get(&projection.identity).cloned() {
            if slot.quarantined || slot.retired {
                return Ok(false);
            }
            if projection.quarantined {
                let positions: std::collections::HashSet<_> =
                    slot.positions.iter().copied().collect();
                let mut at = 0usize;
                self.hits.retain(|_| {
                    let keep = !positions.contains(&at);
                    at += 1;
                    keep
                });
                at = 0;
                self.global_hit_abyss_halves.retain(|_| {
                    let keep = !positions.contains(&at);
                    at += 1;
                    keep
                });
                for half in [AbyssHalf::First, AbyssHalf::Second] {
                    let party = self.abyss.half_mut(half);
                    let old_len = party.hits.len();
                    party.hits.retain(|h| {
                        h.exact
                            .as_ref()
                            .is_none_or(|e| e.message != projection.identity)
                    });
                    if party.hits.len() != old_len {
                        party.rebuild_after_exact_retraction();
                    }
                }
                self.exact_index
                    .get_mut(&projection.identity)
                    .ok_or("exact_index_missing")?
                    .quarantined = true;
                self.rebuild_exact_index();
                if positions.is_empty() {
                    return Ok(false);
                }
                rebuild_all_combat_indexes(
                    &self.hits,
                    &mut self.stats,
                    &mut self.compact_timeline,
                    &mut self.skill_breakdown_index,
                    &mut self.combat_detail_index,
                    &mut self.started_at,
                    &mut self.ended_at,
                    &mut self.total_damage,
                    &mut self.total_damage_taken,
                    &mut self.max_hp_reduction,
                );
                self.recent_hit_records.clear();
                self.hits_generation = self.hits_generation.wrapping_add(1);
                self.sync_clock_with_time_stops();
                return Ok(true);
            }
            if slot.positions.len() != projection.hits.len() {
                return Err("exact_projection_component_set_changed");
            }
            // Preflight all immutable fields before mutating any aggregate.
            for (&position, new) in slot.positions.iter().zip(&mut projection.hits) {
                let old = self
                    .hits
                    .get(position)
                    .ok_or("exact_projection_position_missing")?;
                let (a, b) = (
                    old.exact.as_ref().ok_or("exact_evidence_missing")?,
                    new.exact.as_ref().ok_or("exact_evidence_missing")?,
                );
                if a.target_ordinal != b.target_ordinal
                    || a.component_ordinal != b.component_ordinal
                    || a.source != b.source
                    || a.target != b.target
                    || a.current_hp_bits != b.current_hp_bits
                    || a.display_type != b.display_type
                    || old.damage.to_bits() != new.damage.to_bits()
                    || old.char_id != new.char_id
                    || old.direction != new.direction
                {
                    return Err("exact_projection_immutable_field_changed");
                }
                new.timestamp = old.timestamp; // Freeze first settlement timing, not late request arrival.
                if let Some(half) = self
                    .global_hit_abyss_halves
                    .get(position)
                    .copied()
                    .flatten()
                {
                    let party = self.abyss.half(half);
                    let p = party
                        .exact_positions
                        .get(&(a.message.clone(), a.target_ordinal, a.component_ordinal))
                        .ok_or("exact_half_position_missing")?;
                    if party.hits.get(*p).and_then(|h| h.exact.as_ref()) != Some(a) {
                        return Err("exact_half_identity_mismatch");
                    }
                }
            }
            let mut changed = false;
            for (&position, new) in slot.positions.iter().zip(projection.hits) {
                let old = &self.hits[position];
                if old.exact == new.exact
                    && old.ability_name == new.ability_name
                    && old.damage_name == new.damage_name
                    && old.gameplay_effect_index == new.gameplay_effect_index
                {
                    continue;
                }
                let before = old.clone();
                self.hits[position] = new;
                let after = &self.hits[position];
                self.skill_breakdown_index.replace_hit(&before, after);
                self.combat_detail_index
                    .replace_hit(position, &before, after);
                // Identity, amount, direction and time are immutable; only skill
                // grouping and metadata changed, not team/character damage totals.
                if let Some(half) = self
                    .global_hit_abyss_halves
                    .get(position)
                    .copied()
                    .flatten()
                {
                    self.abyss.half_mut(half).enrich_exact_hit(&before, after)?;
                }
                changed = true;
            }
            if changed {
                self.hits_generation = self.hits_generation.wrapping_add(1);
            }
            return Ok(changed);
        }
        if self.exact_index.len() >= MAX_EXACT_MESSAGES {
            return Err("exact_projection_capacity_exceeded");
        }
        let start = self.hits.len();
        for hit in projection.hits {
            // Bypass legacy target telemetry and request-HP reconciliation.
            let source = hit.target_id.clone();
            let half = self.abyss.push_hit(hit.clone());
            self.finish_push_hit(hit, half, source);
        }
        let positions = (start..self.hits.len()).collect();
        self.exact_index.insert(
            projection.identity,
            Slot {
                positions,
                quarantined: projection.quarantined,
                retired: false,
            },
        );
        Ok(self.hits.len() != start)
    }
}

impl PartyCombatState {
    pub(super) fn rebuild_exact_positions(&mut self) {
        self.exact_positions.clear();
        for (position, h) in self.hits.iter().enumerate() {
            if let Some(e) = &h.exact {
                self.exact_positions.insert(
                    (e.message.clone(), e.target_ordinal, e.component_ordinal),
                    position,
                );
            }
        }
    }
    fn enrich_exact_hit(&mut self, before: &Hit, after: &Hit) -> Result<(), &'static str> {
        let evidence = before.exact.as_ref().ok_or("exact_evidence_missing")?;
        let position = *self
            .exact_positions
            .get(&(
                evidence.message.clone(),
                evidence.target_ordinal,
                evidence.component_ordinal,
            ))
            .ok_or("exact_half_position_missing")?;
        self.skill_breakdown_index.replace_hit(before, after);
        self.combat_detail_index
            .replace_hit(position, before, after);
        self.hits[position] = after.clone();
        self.hits_generation = self.hits_generation.wrapping_add(1);
        Ok(())
    }
    fn rebuild_after_exact_retraction(&mut self) {
        self.rebuild_exact_positions();
        rebuild_all_combat_indexes(
            &self.hits,
            &mut self.stats,
            &mut self.compact_timeline,
            &mut self.skill_breakdown_index,
            &mut self.combat_detail_index,
            &mut self.started_at,
            &mut self.ended_at,
            &mut self.total_damage,
            &mut self.total_damage_taken,
            &mut self.max_hp_reduction,
        );
        self.hits_generation = self.hits_generation.wrapping_add(1);
        self.sync_clock_with_time_stops();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::reducer::{CoreSignal, apply_engine_event};
    use crate::engine::settlement::application::project;
    use crate::engine::settlement::{
        ActorRef, Attribution, Change, Damage, Error, MessageKey, Name,
    };

    fn projection(generation: &str, enriched: bool) -> Projection {
        let key = MessageKey {
            channel: 3,
            message: 5,
            timestamp_bits: 4_715_286_787_042_624_543,
        };
        let row = Damage {
            key,
            target_ordinal: 0,
            component_ordinal: 0,
            source: ActorRef {
                flags: 1,
                name: Some(Name::Text {
                    text: "1".into(),
                    number: 0,
                }),
                fields: vec![],
            },
            target: ActorRef {
                flags: 0,
                name: None,
                fields: vec![(0, 1), (24, 2)],
            },
            character_id: Some(1),
            damage: 100,
            display_type: 0,
            current_hp_bits: 700f32.to_bits(),
            hp_before_request_bits: enriched.then_some(800f32.to_bits()),
            max_hp_at_request_bits: enriched.then_some(1000f32.to_bits()),
            skill_key: enriched.then_some("GA_fixture".into()),
            skill_name: enriched.then_some("Fixture".into()),
            effect_candidates: vec![],
            attribution: if enriched {
                Attribution::ExactRequestAssetGroup
            } else {
                Attribution::RequestMissing
            },
        };
        let characters = HashMap::from([(
            1,
            serde_json::from_str::<CharacterInfo>(r#"{"name_zh":"Fixture"}"#).unwrap(),
        )]);
        project(
            Change {
                key,
                rows: vec![row],
                conflict: None,
            },
            generation,
            "fixture",
            1.0,
            &characters,
            true,
        )
    }
    #[test]
    fn late_metadata_is_upsert_and_duplicate_is_noop() {
        let mut s = CombatState::default();
        assert_eq!(
            apply_engine_event(
                &mut s,
                EngineEvent::ExactSettlement(Box::new(projection("a", false)))
            ),
            CoreSignal::StateChanged
        );
        let generation = s.hits_generation;
        let mut update = projection("a", true);
        update.hits[0].timestamp = 20.0;
        COMBAT_TOTAL_REBUILD_COUNT.with(|v| v.set(0));
        assert_eq!(
            apply_engine_event(
                &mut s,
                EngineEvent::ExactSettlement(Box::new(update.clone()))
            ),
            CoreSignal::StateChanged
        );
        assert_eq!(s.hits.len(), 1);
        assert_eq!(s.total_damage, 100.0);
        assert_eq!(s.hits[0].timestamp, 1.0);
        assert_eq!(s.hits_generation, generation + 1);
        assert_eq!(
            apply_engine_event(&mut s, EngineEvent::ExactSettlement(Box::new(update))),
            CoreSignal::Unchanged
        );
        COMBAT_TOTAL_REBUILD_COUNT.with(|v| assert_eq!(v.get(), 0));
    }
    #[test]
    fn conflicting_projection_retracts_once_and_cannot_resurrect() {
        let mut s = CombatState::default();
        let p = projection("a", true);
        s.apply_exact_projection(p.clone()).unwrap();
        let retract = Projection {
            identity: p.identity.clone(),
            hits: vec![],
            quarantined: true,
        };
        assert!(s.apply_exact_projection(retract.clone()).unwrap());
        assert_eq!(s.total_damage, 0.0);
        assert!(s.hits.is_empty());
        assert!(!s.apply_exact_projection(retract).unwrap());
        assert!(!s.apply_exact_projection(p).unwrap());
    }
    #[test]
    fn generations_and_same_amount_targets_do_not_merge() {
        let mut s = CombatState::default();
        let mut p = projection("a", true);
        let mut other = p.hits[0].clone();
        other.exact.as_mut().unwrap().target_ordinal = 1;
        other.exact.as_mut().unwrap().target.fields[1].1 = 3;
        other.target_id = Some("another_fixture_target".into());
        p.hits.push(other);
        s.apply_exact_projection(p).unwrap();
        s.apply_exact_projection(projection("b", true)).unwrap();
        assert_eq!(s.hits.len(), 3);
        assert_eq!(s.total_damage, 300.0);
    }
    #[test]
    fn history_roundtrip_preserves_full_key_unknowns_and_dedup() {
        let p = projection("a", false);
        let encoded = serde_json::to_string(&p.hits).unwrap();
        assert!(encoded.contains("4715286787042624543"));
        let hits: Vec<Hit> = serde_json::from_str(&encoded).unwrap();
        assert_eq!(hits[0].known_max_hp(), None);
        assert_eq!(hits[0].known_hp_after(), Some(700.0));
        assert_eq!(hits[0].known_overkill(), None);
        let mut s = CombatState::default();
        s.replace_global_hits_bulk(hits);
        assert!(!s.apply_exact_projection(p).unwrap());
        assert_eq!(s.hits.len(), 1);
    }
    #[test]
    fn ordinary_hit_event_cannot_append_exact_records() {
        let mut s = CombatState::default();
        let p = projection("a", true);
        assert!(matches!(
            apply_engine_event(&mut s, EngineEvent::Hit(Box::new(p.hits[0].clone()))),
            CoreSignal::Error(_)
        ));
        assert!(s.hits.is_empty());
    }
    #[test]
    fn legacy_hp_limit_does_not_correct_exact_totals() {
        let mut s = CombatState::default();
        let p = projection("a", true);
        s.apply_exact_projection(p.clone()).unwrap();
        let mut marker = p.hits[0].clone();
        marker.exact = None;
        marker.char_id = 0;
        marker.char_source = HitCharacterSource::Unknown;
        marker.damage_name = Some("Server settlement residual".into());
        assert!(!s.reconcile_server_target_damage(marker));
        assert!(!s.reconcile_known_server_target_limits(1.0));
        assert_eq!(s.total_damage, 100.0);
    }
    #[test]
    fn same_message_amount_change_rejected_atomically() {
        let mut s = CombatState::default();
        s.apply_exact_projection(projection("a", true)).unwrap();
        let mut p = projection("a", true);
        p.hits[0].damage = 200.0;
        assert!(s.apply_exact_projection(p).is_err());
        assert_eq!(s.total_damage, 100.0);
    }
    #[test]
    fn named_conflict_retains_explicit_protocol_error_kind() {
        assert_ne!(Error::ConflictingRequest, Error::ConflictingSettlement);
    }

    #[test]
    fn late_metadata_and_retraction_keep_original_abyss_half() {
        let mut s = CombatState::default();
        s.apply_abyss_event(AbyssEvent::Stage {
            timestamp: 0.0,
            cycle: Some(1),
            floor: Some(1),
            half: AbyssHalf::First,
            allow_late_backfill: false,
        });
        let p = projection("a", false);
        s.apply_exact_projection(p.clone()).unwrap();
        s.apply_abyss_event(AbyssEvent::Stage {
            timestamp: 3.0,
            cycle: Some(1),
            floor: Some(1),
            half: AbyssHalf::Second,
            allow_late_backfill: false,
        });
        s.apply_exact_projection(projection("a", true)).unwrap();
        assert_eq!(s.abyss.first_half.hits.len(), 1);
        assert!(s.abyss.second_half.hits.is_empty());
        assert_eq!(s.abyss.first_half.total_damage, 100.0);
        s.apply_exact_projection(Projection {
            identity: p.identity,
            hits: vec![],
            quarantined: true,
        })
        .unwrap();
        assert_eq!(s.abyss.first_half.total_damage, 0.0);
        assert!(s.abyss.first_half.hits.is_empty());
    }

    #[test]
    fn absent_half_index_fails_before_global_mutation() {
        let mut s = CombatState::default();
        s.apply_abyss_event(AbyssEvent::Stage {
            timestamp: 0.0,
            cycle: Some(1),
            floor: Some(1),
            half: AbyssHalf::First,
            allow_late_backfill: false,
        });
        s.apply_exact_projection(projection("a", false)).unwrap();
        s.abyss.first_half.exact_positions.clear();
        let generation = s.hits_generation;
        assert!(s.apply_exact_projection(projection("a", true)).is_err());
        assert_eq!(s.hits[0].known_max_hp(), None);
        assert_eq!(s.hits_generation, generation);
    }

    #[test]
    fn actual_history_metadata_preserves_quarantine_and_exact_rows() {
        use crate::storage::history::HistoryCombatDetails;
        let mut s = CombatState::default();
        let a = projection("a", false);
        let b = projection("b", true);
        s.apply_exact_projection(a.clone()).unwrap();
        s.apply_exact_projection(b.clone()).unwrap();
        s.apply_exact_projection(Projection {
            identity: a.identity.clone(),
            hits: vec![],
            quarantined: true,
        })
        .unwrap();
        let history = HistoryCombatDetails::from_state(&s).unwrap();
        let json = serde_json::to_string(&history).unwrap();
        let read: HistoryCombatDetails = serde_json::from_str(&json).unwrap();
        let mut restored = read.into_combat_state();
        assert_eq!(restored.total_damage, 100.0);
        assert_eq!(restored.hits[0].exact, b.hits[0].exact);
        assert!(!restored.apply_exact_projection(a).unwrap());
        assert!(!restored.apply_exact_projection(b).unwrap());
    }

    #[test]
    fn round_cut_and_reset_reject_old_enrichment_without_new_damage() {
        let mut s = CombatState::default();
        s.apply_exact_projection(projection("a", false)).unwrap();
        let archived = s.take_battle_preserving_inventory();
        assert_eq!(archived.total_damage, 100.0);
        assert!(!s.apply_exact_projection(projection("a", true)).unwrap());
        assert!(s.hits.is_empty());
        s.apply_exact_projection(projection("b", true)).unwrap();
        s.clear();
        assert!(!s.apply_exact_projection(projection("b", true)).unwrap());
        assert!(s.apply_exact_projection(projection("c", true)).unwrap());
        assert_eq!(s.total_damage, 100.0);
    }
}
