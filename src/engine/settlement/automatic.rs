//! Bounded midstream attachment to the two qualified PlayerController wire
//! layouts. Endpoint/prefix observation alone never authorizes damage: a lane
//! must first join a bidirectional request and settlement by the complete wire
//! key and actor references. No scan for plausible amounts or timestamp proximity.
use super::{
    Change, Error, Ledger, MessageKey, SkillCatalog,
    application::{Projection, project},
    runtime::{catalog_from_document, new_generation},
    transport::{Decoder, Profile, Rpc},
};
use crate::{engine::model::CharacterInfo, storage::resource::read_resource_text_bounded};
use std::{collections::HashMap, net::Ipv4Addr, path::Path};

pub const WAITING: &str = "exact_auto_waiting_for_exchange";
pub const READY: &str = "exact_auto_exchange_confirmed";
pub const GAP: &str = "exact_auto_incomplete_message";
pub const UNSUPPORTED_SETTLEMENT: &str = "exact_unsupported_settlement_extras_skipped";
pub const UNAVAILABLE: &str = "exact_auto_flow_unavailable";
const CATALOG: &str = "res/data/skills/skill_attribution_catalog.json";
const MAX_FLOWS: usize = 8;
const MAX_PROBE_MESSAGES: usize = 128;
const MAX_PROBE_ROWS: usize = 4096;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
struct FlowKey {
    local: (Ipv4Addr, u16),
    server: (Ipv4Addr, u16),
    prefix: u8,
}

struct Lane {
    decoder: Decoder,
    ledger: Ledger,
    waiting: Vec<(Change, f64, bool)>,
    waiting_rows: usize,
    qualified: bool,
}

impl Lane {
    fn new(prefix: u8, upper: u32, settlement: u32) -> Result<Self, &'static str> {
        Ok(Self {
            // These are reviewed wire layouts, not values searched for by a
            // successful body parse. Other channel/class layouts stay unsupported.
            decoder: Decoder::new(Profile {
                component_prefix: prefix,
                channel: 3,
                field_upper_exclusive: upper,
                request_index: 100,
                settlement_index: settlement,
            })
            .map_err(|_| UNAVAILABLE)?,
            ledger: Ledger::new(100_000, SkillCatalog::default()),
            waiting: Vec::new(),
            waiting_rows: 0,
            qualified: false,
        })
    }

    fn accept(
        &mut self,
        rpc: Rpc,
        time: f64,
        characters: &HashMap<u32, CharacterInfo>,
    ) -> Result<(), Error> {
        let is_settlement = matches!(&rpc, Rpc::Settlement(_));
        let (key, change) = match rpc {
            Rpc::Request(r) => (r.key, self.ledger.request(r)),
            Rpc::Settlement(s) => (s.key, self.ledger.settlement(s)),
            Rpc::Inventory { .. } => return Ok(()),
            Rpc::UnsupportedSettlementExtras => {
                self.ledger.invalidate_hp_continuity();
                return Ok(());
            }
        };
        let change = change?;
        self.qualified |= matching_exchange(&self.ledger, key, characters);
        if !self.qualified && self.ledger.entries.len() > MAX_PROBE_MESSAGES {
            return Err(Error::BudgetExceeded);
        }
        let mut changes = Vec::new();
        if let Some(change) = change {
            changes.push((change, is_settlement));
        }
        changes.extend(
            self.ledger
                .take_hp_updates()
                .into_iter()
                .map(|change| (change, false)),
        );
        for (change, is_settlement) in changes {
            if self.waiting.len() >= MAX_PROBE_MESSAGES
                || self.waiting_rows.saturating_add(change.rows.len()) > MAX_PROBE_ROWS
            {
                return Err(Error::BudgetExceeded);
            }
            self.waiting_rows += change.rows.len();
            self.waiting.push((change, time, is_settlement));
        }
        Ok(())
    }

    fn reset_probe(&mut self) {
        self.qualified = false;
        self.decoder.clear();
        self.ledger.clear();
        self.waiting.clear();
        self.waiting_rows = 0;
    }
}

fn matching_exchange(
    ledger: &Ledger,
    key: MessageKey,
    characters: &HashMap<u32, CharacterInfo>,
) -> bool {
    let Some(entry) = ledger.entries.get(&key) else {
        return false;
    };
    if entry.request_conflict || entry.settlement_conflict {
        return false;
    }
    let (Some(request), Some(settlement)) = (&entry.request, &entry.settlement) else {
        return false;
    };
    let player_involved = settlement
        .source
        .character_id()
        .is_some_and(|id| characters.contains_key(&id))
        || settlement.targets.iter().any(|t| {
            t.target
                .character_id()
                .is_some_and(|id| characters.contains_key(&id))
        });
    player_involved
        && !settlement.targets.is_empty()
        && settlement.targets.iter().all(|target| {
            request.targets.iter().any(|r| {
                r.source == settlement.source
                    && r.target == target.target
                    && r.effect_index.is_some()
            })
        })
}

struct Flow {
    identity: String,
    lanes: Vec<Lane>,
    selected: bool,
    failed: bool,
}

pub struct Automatic {
    generation: String,
    next_flow: u64,
    catalog: SkillCatalog,
    flows: HashMap<FlowKey, Flow>,
    pub last_settlement_components: usize,
    pub ready: bool,
    pub warning: Option<&'static str>,
}

impl Automatic {
    pub fn bundled() -> Result<Self, &'static str> {
        let text = read_resource_text_bounded(Path::new(CATALOG), 2 * 1024 * 1024)
            .map_err(|_| "exact_auto_catalog_unavailable")?;
        let document: serde_json::Value =
            serde_json::from_str(&text).map_err(|_| "exact_auto_catalog_invalid")?;
        if document["schema"].as_str() != Some("nte.assets.skill_attribution_catalog/1") {
            return Err("exact_auto_catalog_version");
        }
        Ok(Self {
            generation: new_generation()?,
            next_flow: 0,
            catalog: catalog_from_document(&document)?,
            flows: HashMap::new(),
            last_settlement_components: 0,
            ready: false,
            warning: None,
        })
    }

    #[allow(clippy::too_many_arguments)]
    pub fn datagram(
        &mut self,
        src: Ipv4Addr,
        sport: u16,
        dst: Ipv4Addr,
        dport: u16,
        local_hint: Option<Ipv4Addr>,
        payload: &[u8],
        time: Option<f64>,
        characters: &HashMap<u32, CharacterInfo>,
        include_incoming: bool,
    ) -> Vec<Projection> {
        self.last_settlement_components = 0;
        self.warning = None;
        let Some(time) = time.filter(|t| t.is_finite()) else {
            return Vec::new();
        };
        let Some(&first) = payload.first() else {
            return Vec::new();
        };
        if payload.len() > 65535 {
            return Vec::new();
        }
        let local = match local_hint {
            Some(ip) if ip == src || ip == dst => ip,
            Some(_) => return Vec::new(),
            None if src.is_private() && !dst.is_private() => src,
            None if dst.is_private() && !src.is_private() => dst,
            // Ambiguous direction is not silently turned into a field value.
            None => return Vec::new(),
        };
        let inbound = dst == local;
        let (local, server) = if inbound {
            ((dst, dport), (src, sport))
        } else {
            ((src, sport), (dst, dport))
        };
        let key = FlowKey {
            local,
            server,
            prefix: first & 31,
        };
        if first & 32 != 0 {
            // A delayed handshake on an accounted tuple is framing, not damage
            // evidence. Keep the flow: discarding its dedup identities would
            // double-count messages that arrive again, while permanently
            // failing it stopped every later damage record on the connection
            // (issue #62). Only fragment progress is discarded.
            let mut degraded = false;
            self.flows.retain(|k, flow| {
                if k.local != local || k.server != server {
                    return true;
                }
                if flow.selected {
                    for lane in &mut flow.lanes {
                        lane.decoder.discard_incomplete();
                        // The discarded fragment may have carried damage or
                        // healing, so the HP witness chain must not span the
                        // gap and derive later reduction from stale values.
                        lane.ledger.invalidate_hp_continuity();
                    }
                    degraded = true;
                    true
                } else {
                    false
                }
            });
            if degraded {
                self.warning = Some(GAP);
            }
            self.ready = self.flows.values().any(|f| f.selected && !f.failed);
            return Vec::new();
        }
        let mut fresh = false;
        if !self.flows.contains_key(&key) {
            if self.flows.len() >= MAX_FLOWS {
                // Unqualified probes have published nothing, so evicting one
                // cannot double-count. Without this, unrelated connections that
                // scatter across prefixes can fill the registry and starve the
                // real game connection until the capture is restarted (#62).
                let evict = self
                    .flows
                    .iter()
                    .find(|(_, flow)| !flow.selected)
                    .map(|(probe, _)| *probe);
                match evict {
                    Some(probe) => {
                        self.flows.remove(&probe);
                    }
                    None => {
                        // Never evict an accounted flow and thereby forget dedup keys.
                        self.warning = Some(UNAVAILABLE);
                        return Vec::new();
                    }
                }
            }
            let lanes = [(219, 142), (213, 139)]
                .into_iter()
                .map(|(upper, index)| Lane::new(key.prefix, upper, index))
                .collect::<Result<Vec<_>, _>>();
            let Ok(lanes) = lanes else {
                self.warning = Some(UNAVAILABLE);
                return Vec::new();
            };
            self.next_flow += 1;
            self.flows.insert(
                key,
                Flow {
                    identity: format!("auto-{}", self.next_flow),
                    lanes,
                    selected: false,
                    failed: false,
                },
            );
            fresh = true;
        }
        let flow = self
            .flows
            .get_mut(&key)
            .expect("flow just inserted or existed");
        if flow.failed {
            return Vec::new();
        }
        let mut saw_channel = false;
        for lane in &mut flow.lanes {
            match lane.decoder.datagram(payload, inbound) {
                Ok(rpcs) => {
                    saw_channel |= lane.decoder.saw_channel;
                    for rpc in rpcs {
                        if flow.selected && matches!(&rpc, Rpc::UnsupportedSettlementExtras) {
                            self.warning = Some(UNSUPPORTED_SETTLEMENT);
                        }
                        if let Err(error) = lane.accept(rpc, time, characters) {
                            lane.decoder.discard_incomplete();
                            lane.ledger.invalidate_hp_continuity();
                            if flow.selected && error == Error::BudgetExceeded {
                                // Dedup capacity is a hard limit: no later
                                // message can be accepted, so silently
                                // skipping would look like a single gap while
                                // recording has actually stopped.
                                flow.failed = true;
                                self.warning = Some(UNAVAILABLE);
                            } else if flow.selected {
                                // One unusable message must not retire an
                                // accounted connection for the rest of the
                                // capture (issue #62). Keep the ledger so later
                                // identities stay deduped.
                                self.warning = Some(GAP);
                            } else {
                                lane.reset_probe();
                            }
                            break;
                        }
                    }
                }
                Err(_) if flow.selected => {
                    // A skipped middle fragment or an undecodable datagram is
                    // one incomplete message, not a dead connection. Discard
                    // only fragment progress and keep the ledger so dedup
                    // identities survive for later messages (issue #62).
                    lane.decoder.discard_incomplete();
                    lane.ledger.invalidate_hp_continuity();
                    self.warning = Some(GAP);
                }
                Err(_) => lane.reset_probe(),
            }
        }
        if flow.failed {
            return Vec::new();
        }
        if !flow.selected {
            let qualified: Vec<_> = flow
                .lanes
                .iter()
                .enumerate()
                .filter(|(_, l)| l.qualified)
                .map(|(i, _)| i)
                .collect();
            match qualified.as_slice() {
                [index] => {
                    let mut lane = flow.lanes.swap_remove(*index);
                    // Load the immutable catalog into this ledger only once,
                    // after qualification, never on every unrelated UDP packet.
                    lane.ledger.catalog = self.catalog.clone();
                    for (change, _, _) in &mut lane.waiting {
                        if let Some(enriched) = lane.ledger.project(change.key) {
                            *change = enriched;
                        }
                    }
                    flow.lanes = vec![lane];
                    flow.selected = true;
                    self.ready = true;
                }
                [] => {
                    if fresh && !saw_channel {
                        self.flows.remove(&key);
                    }
                    return Vec::new();
                }
                _ => {
                    flow.failed = true;
                    self.warning = Some(UNAVAILABLE);
                    return Vec::new();
                }
            }
        }
        let lane = &mut flow.lanes[0];
        lane.waiting_rows = 0;
        lane.waiting
            .drain(..)
            .map(|(change, observed_time, is_settlement)| {
                let projection = project(
                    change,
                    &self.generation,
                    &flow.identity,
                    observed_time,
                    characters,
                    include_incoming,
                );
                if is_settlement {
                    self.last_settlement_components += projection.hits.len();
                }
                projection
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::settlement::{
        ActorRef, Component, Name, Request, RequestTarget, SettledTarget, Settlement,
    };

    fn characters() -> HashMap<u32, CharacterInfo> {
        HashMap::from([(
            1,
            CharacterInfo {
                name_zh: "角色".into(),
                name_en: "Player".into(),
                color: None,
                avatar: None,
                attribute: None,
            },
        )])
    }
    fn actor() -> ActorRef {
        ActorRef {
            flags: 1,
            name: Some(Name::Text {
                text: "1".into(),
                number: 0,
            }),
            fields: vec![],
        }
    }
    fn enemy() -> ActorRef {
        ActorRef {
            flags: 0,
            name: None,
            fields: vec![(0, 9), (24, 2)],
        }
    }
    fn key() -> MessageKey {
        MessageKey {
            channel: 3,
            message: 7,
            timestamp_bits: 123,
        }
    }
    fn request() -> Request {
        Request {
            key: key(),
            targets: vec![RequestTarget {
                source: actor(),
                target: enemy(),
                effect_index: Some(1),
                hp_before_bits: Some(500f32.to_bits()),
                max_hp_bits: Some(1000f32.to_bits()),
                calculated_damage_bits: Some(1f32.to_bits()),
            }],
        }
    }
    fn settlement() -> Settlement {
        Settlement {
            recoveries: vec![],
            key: key(),
            source: actor(),
            targets: vec![SettledTarget {
                target: enemy(),
                current_hp_bits: 300f32.to_bits(),
                dead_state: 0,
                shield_damage_bits: 0,
                lock_target: 0,
                components: vec![Component {
                    damage: 200,
                    display_type: 0,
                }],
            }],
        }
    }
    fn lane() -> Lane {
        Lane::new(20, 219, 142).unwrap()
    }

    #[test]
    fn odd_observed_prefix_reaches_transport_but_cannot_qualify_alone() {
        let mut a = Automatic::bundled().unwrap();
        let mut packet = super::super::transport::tests::fragment(10, true, false, 3, 8);
        packet[0] = (packet[0] & !63) | 25;
        let local = Ipv4Addr::new(192, 168, 1, 2);
        let remote = Ipv4Addr::new(192, 0, 2, 1);
        assert!(
            a.datagram(
                remote,
                456,
                local,
                123,
                Some(local),
                &packet,
                Some(1.0),
                &characters(),
                true
            )
            .is_empty()
        );
        assert!(a.flows.keys().any(|k| k.prefix == 25));
        assert!(!a.ready);
    }
    #[test]
    fn unsupported_extra_message_cannot_clear_qualification_or_duplicate_damage() {
        let mut l = lane();
        l.accept(Rpc::Request(request()), 1.0, &characters())
            .unwrap();
        l.accept(Rpc::Settlement(settlement()), 2.0, &characters())
            .unwrap();
        l.waiting.clear();
        l.waiting_rows = 0;
        l.accept(Rpc::UnsupportedSettlementExtras, 3.0, &characters())
            .unwrap();
        assert!(l.qualified);
        assert!(l.ledger.hp_cursors.is_empty());
        l.accept(Rpc::Settlement(settlement()), 4.0, &characters())
            .unwrap();
        assert!(l.waiting.is_empty());
        let mut next = settlement();
        next.key.message = 2;
        l.accept(Rpc::Settlement(next), 5.0, &characters()).unwrap();
        assert_eq!(l.waiting.len(), 1);
        assert_eq!(l.waiting[0].0.rows[0].damage, 200);
    }

    #[test]
    fn request_alone_never_qualifies_or_creates_damage() {
        let mut l = lane();
        l.accept(Rpc::Request(request()), 1.0, &characters())
            .unwrap();
        assert!(!l.qualified);
        assert!(l.waiting.is_empty());
    }
    #[test]
    fn complete_key_and_full_target_identity_are_required_not_amount_similarity() {
        let mut l = lane();
        l.accept(Rpc::Request(request()), 1.0, &characters())
            .unwrap();
        let mut wrong = settlement();
        wrong.targets[0].target.fields[1].1 = 3;
        l.accept(Rpc::Settlement(wrong), 2.0, &characters())
            .unwrap();
        assert!(!l.qualified);
        let mut l = lane();
        l.accept(Rpc::Request(request()), 1.0, &characters())
            .unwrap();
        let mut wrong = settlement();
        wrong.key.timestamp_bits += 1;
        l.accept(Rpc::Settlement(wrong), 2.0, &characters())
            .unwrap();
        assert!(!l.qualified);
        let mut l = lane();
        l.accept(Rpc::Request(request()), 1.0, &characters())
            .unwrap();
        l.accept(Rpc::Settlement(settlement()), 2.0, &characters())
            .unwrap();
        assert!(l.qualified);
        assert_eq!(l.waiting[0].0.rows[0].damage, 200);
    }
    #[test]
    fn late_request_qualifies_without_retiming_the_observed_settlement() {
        let mut l = lane();
        l.accept(Rpc::Settlement(settlement()), 10.0, &characters())
            .unwrap();
        assert!(!l.qualified);
        l.accept(Rpc::Request(request()), 11.0, &characters())
            .unwrap();
        assert!(l.qualified);
        assert_eq!(l.waiting[0].1, 10.0);
        assert_eq!(l.waiting[1].1, 11.0);
        assert!(l.waiting[0].2);
        assert!(!l.waiting[1].2);
    }
    #[test]
    fn conflicting_request_and_unknown_actor_cannot_qualify_a_lane() {
        let mut l = lane();
        l.accept(Rpc::Request(request()), 1.0, &characters())
            .unwrap();
        let mut r = request();
        r.targets[0].effect_index = Some(2);
        l.accept(Rpc::Request(r), 2.0, &characters()).unwrap();
        l.accept(Rpc::Settlement(settlement()), 3.0, &characters())
            .unwrap();
        assert!(!l.qualified);
        let mut l = lane();
        l.accept(Rpc::Request(request()), 1.0, &HashMap::new())
            .unwrap();
        l.accept(Rpc::Settlement(settlement()), 2.0, &HashMap::new())
            .unwrap();
        assert!(!l.qualified);
    }
    #[test]
    fn probe_budget_is_bounded_and_reset_clears_qualification_and_provenance() {
        let mut l = lane();
        for i in 0..MAX_PROBE_MESSAGES {
            let mut r = request();
            r.key.message = i as i64;
            l.accept(Rpc::Request(r), 1.0, &characters()).unwrap();
        }
        let mut r = request();
        r.key.message = MAX_PROBE_MESSAGES as i64;
        assert!(l.accept(Rpc::Request(r), 1.0, &characters()).is_err());
        l.reset_probe();
        assert!(l.waiting.is_empty());
        assert!(l.ledger.entries.is_empty());
        assert!(!l.qualified);
        l.accept(Rpc::Request(request()), 2.0, &characters())
            .unwrap();
        l.accept(Rpc::Settlement(settlement()), 3.0, &characters())
            .unwrap();
        assert!(l.qualified);
        l.reset_probe();
        assert!(!l.qualified);
    }
    #[test]
    fn random_or_over_budget_udp_cannot_accumulate_flows_or_publish() {
        let mut a = Automatic::bundled().unwrap();
        let local = Ipv4Addr::new(192, 168, 1, 2);
        let server = Ipv4Addr::new(192, 0, 2, 1);
        for len in 0..64 {
            assert!(
                a.datagram(
                    local,
                    123,
                    server,
                    456,
                    Some(local),
                    &vec![20; len],
                    Some(1.0),
                    &characters(),
                    true
                )
                .is_empty()
            );
        }
        assert!(a.flows.is_empty());
        assert!(!a.ready);
        assert!(
            a.datagram(
                local,
                123,
                server,
                456,
                Some(local),
                &vec![20; 65536],
                Some(1.0),
                &characters(),
                true
            )
            .is_empty()
        );
        assert!(a.flows.is_empty());
    }
    #[test]
    fn reconnect_retires_flow_identity_without_reusing_old_requests() {
        let mut a = Automatic::bundled().unwrap();
        let local = (Ipv4Addr::new(192, 168, 1, 2), 123);
        let server = (Ipv4Addr::new(192, 0, 2, 1), 456);
        let k = FlowKey {
            local,
            server,
            prefix: 20,
        };
        let mut l = lane();
        l.accept(Rpc::Request(request()), 1.0, &characters())
            .unwrap();
        a.flows.insert(
            k,
            Flow {
                identity: "old".into(),
                lanes: vec![l],
                selected: false,
                failed: false,
            },
        );
        assert!(
            a.datagram(
                local.0,
                local.1,
                server.0,
                server.1,
                Some(local.0),
                &[52],
                Some(2.0),
                &characters(),
                true
            )
            .is_empty()
        );
        assert!(a.flows.is_empty());
        assert!(!a.ready);
    }
    #[test]
    fn delayed_handshake_keeps_the_accounted_flow_available() {
        let mut a = Automatic::bundled().unwrap();
        let local = (Ipv4Addr::new(192, 168, 1, 2), 123);
        let server = (Ipv4Addr::new(192, 0, 2, 1), 456);
        let k = FlowKey {
            local,
            server,
            prefix: 20,
        };
        let mut l = lane();
        l.accept(Rpc::Request(request()), 1.0, &characters())
            .unwrap();
        l.accept(Rpc::Settlement(settlement()), 2.0, &characters())
            .unwrap();
        a.flows.insert(
            k,
            Flow {
                identity: "accounted".into(),
                lanes: vec![l],
                selected: true,
                failed: false,
            },
        );
        a.ready = true;
        a.datagram(
            local.0,
            local.1,
            server.0,
            server.1,
            Some(local.0),
            &[52],
            Some(3.0),
            &characters(),
            true,
        );
        // Issue #62: this used to mark the flow failed for the rest of the
        // capture, so every later damage record on the connection was dropped.
        assert_eq!(a.warning, Some(GAP));
        assert!(!a.flows[&k].failed);
        assert_eq!(a.flows[&k].identity, "accounted");
        assert!(a.flows[&k].lanes[0].ledger.entries.contains_key(&key()));
        assert!(!a.flows[&k].lanes[0].decoder.has_incomplete_fragments());
        assert!(
            a.flows[&k].lanes[0].ledger.hp_cursors.is_empty(),
            "a discarded fragment must break the HP witness chain"
        );
    }

    #[test]
    fn ledger_capacity_exhaustion_is_a_hard_failure_not_a_skipped_message() {
        let mut l = lane();
        l.ledger.capacity = 1;
        l.accept(Rpc::Request(request()), 1.0, &characters())
            .unwrap();
        let mut next = settlement();
        next.key.message = 2;
        // The ledger cannot accept another dedup identity. Continuing would
        // silently drop every later message, so this must stay distinguishable
        // from a single skipped message.
        assert!(matches!(
            l.accept(Rpc::Settlement(next), 2.0, &characters()),
            Err(Error::BudgetExceeded)
        ));
        assert!(l.ledger.entries.contains_key(&key()));
    }

    #[test]
    fn unparseable_datagram_does_not_disable_an_accounted_flow() {
        let mut a = Automatic::bundled().unwrap();
        let local = (Ipv4Addr::new(192, 168, 1, 2), 123);
        let server = (Ipv4Addr::new(192, 0, 2, 1), 456);
        let k = FlowKey {
            local,
            server,
            prefix: 28,
        };
        let mut l = Lane::new(28, 219, 142).unwrap();
        l.accept(Rpc::Request(request()), 1.0, &characters())
            .unwrap();
        l.accept(Rpc::Settlement(settlement()), 2.0, &characters())
            .unwrap();
        l.waiting.clear();
        l.waiting_rows = 0;
        a.flows.insert(
            k,
            Flow {
                identity: "qualified".into(),
                lanes: vec![l],
                selected: true,
                failed: false,
            },
        );
        a.ready = true;
        // The zero termination byte is invalid framing: one skipped message,
        // not a reason to retire the connection (issue #62).
        assert!(
            a.datagram(
                server.0,
                server.1,
                local.0,
                local.1,
                Some(local.0),
                &[28, 0],
                Some(2.5),
                &characters(),
                true
            )
            .is_empty()
        );
        assert_eq!(a.warning, Some(GAP));
        assert!(!a.flows[&k].failed);
        assert!(a.flows[&k].lanes[0].ledger.entries.contains_key(&key()));
        // Later complete traffic on the same flow is still decoded.
        let next = super::super::transport::tests::fragment(21, true, false, 3, 8);
        a.datagram(
            server.0,
            server.1,
            local.0,
            local.1,
            Some(local.0),
            &next,
            Some(3.0),
            &characters(),
            true,
        );
        assert!(a.warning.is_none());
        assert!(!a.flows[&k].failed);
    }

    #[test]
    fn unqualified_probe_yields_its_slot_to_a_new_connection() {
        let mut a = Automatic::bundled().unwrap();
        let local = Ipv4Addr::new(192, 168, 1, 2);
        let server = Ipv4Addr::new(192, 0, 2, 1);
        for port in 0..MAX_FLOWS {
            a.flows.insert(
                FlowKey {
                    local: (local, port as u16),
                    server: (server, 456),
                    prefix: 20,
                },
                Flow {
                    identity: port.to_string(),
                    lanes: vec![],
                    selected: false,
                    failed: false,
                },
            );
        }
        let packet = super::super::transport::tests::fragment(3, true, false, 3, 8);
        a.datagram(
            local,
            99,
            server,
            456,
            Some(local),
            &packet,
            Some(1.0),
            &characters(),
            true,
        );
        assert_eq!(a.flows.len(), MAX_FLOWS);
        assert!(a.flows.contains_key(&FlowKey {
            local: (local, 99),
            server: (server, 456),
            prefix: 28,
        }));
        assert!(a.warning.is_none());
    }

    #[test]
    fn full_flow_registry_never_evicts_accounted_dedup_state() {
        let mut a = Automatic::bundled().unwrap();
        let local = Ipv4Addr::new(192, 168, 1, 2);
        let server = Ipv4Addr::new(192, 0, 2, 1);
        for port in 0..MAX_FLOWS {
            a.flows.insert(
                FlowKey {
                    local: (local, port as u16),
                    server: (server, 456),
                    prefix: 20,
                },
                Flow {
                    identity: port.to_string(),
                    lanes: vec![],
                    selected: true,
                    failed: false,
                },
            );
        }
        assert!(
            a.datagram(
                local,
                99,
                server,
                456,
                Some(local),
                &[20],
                Some(1.0),
                &characters(),
                true
            )
            .is_empty()
        );
        assert_eq!(a.flows.len(), MAX_FLOWS);
        assert_eq!(a.warning, Some(UNAVAILABLE));
    }

    #[test]
    fn a_missing_fragment_does_not_disable_later_complete_messages_or_forget_the_ledger() {
        let mut a = Automatic::bundled().unwrap();
        let local = (Ipv4Addr::new(192, 168, 1, 2), 123);
        let server = (Ipv4Addr::new(192, 0, 2, 1), 456);
        let k = FlowKey {
            local,
            server,
            prefix: 28,
        };
        let mut l = Lane::new(28, 219, 142).unwrap();
        l.accept(Rpc::Request(request()), 1.0, &characters())
            .unwrap();
        l.accept(Rpc::Settlement(settlement()), 2.0, &characters())
            .unwrap();
        l.waiting.clear();
        l.waiting_rows = 0;
        a.flows.insert(
            k,
            Flow {
                identity: "qualified".into(),
                lanes: vec![l],
                selected: true,
                failed: false,
            },
        );
        a.ready = true;
        // Issue #62: losing a middle fragment used to be InvalidValue, permanently
        // disabling the qualified flow instead of discarding only the incomplete message.
        let first = super::super::transport::tests::fragment(9, true, false, 3, 8);
        a.datagram(
            server.0,
            server.1,
            local.0,
            local.1,
            Some(local.0),
            &first,
            Some(2.5),
            &characters(),
            true,
        );
        let tail = super::super::transport::tests::fragment(11, false, true, 0, 2);
        assert!(
            a.datagram(
                server.0,
                server.1,
                local.0,
                local.1,
                Some(local.0),
                &tail,
                Some(3.0),
                &characters(),
                true
            )
            .is_empty()
        );
        assert_eq!(a.warning, Some(GAP));
        assert!(!a.flows[&k].failed);
        for (seq, initial, final_part, byte, bits) in
            [(12, true, false, 3, 8), (13, false, true, 0, 2)]
        {
            let packet =
                super::super::transport::tests::fragment(seq, initial, final_part, byte, bits);
            a.datagram(
                server.0,
                server.1,
                local.0,
                local.1,
                Some(local.0),
                &packet,
                Some(4.0),
                &characters(),
                true,
            );
            assert!(a.warning.is_none());
        }
        assert!(!a.flows[&k].failed);
        assert!(!a.flows[&k].lanes[0].decoder.has_incomplete_fragments());
        assert!(a.flows[&k].lanes[0].ledger.entries.contains_key(&key()));
    }
}
