//! Bounded adapter for UE Tools combat report schema 14, independent of packet
//! decoding. Unobserved identities stay unknown; gaps are errors, never zeros.
use crate::{
    engine::{
        capture::EngineEventSink,
        model::{
            CombatClockRuntimeHealth, EngineEvent, Hit, HitCharacterSource, HitDirection,
            TimeStopEvent,
        },
    },
    platform::toolkit::{MAX_BLOB_BYTES, ToolkitClient, ToolkitError},
};
use serde::{Deserialize, Serialize};
use std::{
    collections::HashMap,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};

pub use crate::storage::config::DataMode;

#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CombatStatus {
    pub capture_generation: u64,
    pub capturing: bool,
    pub encounter_id: String,
    pub hits: u64,
    pub total_damage: f64,
    pub quality: Quality,
}
#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
pub struct Quality {
    pub direct: f64,
    pub correlated: f64,
    pub inferred: f64,
    pub unknown: f64,
}
#[derive(Deserialize)]
pub struct Operation {
    pub busy: bool,
    pub ok: bool,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Report {
    schema_version: u32,
    capture_generation: u64,
    encounter_id: String,
    totals: Totals,
    quality: ReportQuality,
    game_clock: GameClock,
    participants: Vec<Participant>,
    events: Vec<Damage>,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct GameClock {
    valid: bool,
    status: String,
    transitions: Vec<ClockTransition>,
}

#[derive(Clone, Debug, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ClockTransition {
    world_seconds: f64,
    unix_us: u64,
    pause_mask: u32,
    boundary: bool,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Totals {
    all_hits: u64,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ReportQuality {
    dropped_events: u64,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Participant {
    object_index: i32,
    role_id: String,
    display_name: String,
}
#[derive(Clone, Debug, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Damage {
    unix_us: u64,
    damage: f64,
    attacker_object_index: i32,
    victim_object_index: i32,
    quality: String,
    direction: String,
    victim_name: String,
    victim_hp: Option<f64>,
    victim_max_hp: Option<f64>,
    skill_name: String,
    damage_attribute: String,
    damage_lane: String,
}
fn direction(value: &str) -> Result<HitDirection, ToolkitError> {
    if value == "other" {
        return Ok(HitDirection::Unknown);
    }
    HitDirection::try_from(value).map_err(|_| ToolkitError::InvalidProtocol)
}
const MAX_EVENTS: usize = 16_384;
const MAX_PARTICIPANTS: usize = 256;
#[derive(Default)]
pub struct ReportCursor {
    identity: Option<(u64, String)>,
    consumed: u64,
    previous: Vec<Damage>,
    clock: Vec<ClockTransition>,
    clock_health: Option<CombatClockRuntimeHealth>,
}
impl ReportCursor {
    pub fn ingest(&mut self, bytes: &[u8]) -> Result<Vec<EngineEvent>, ToolkitError> {
        if bytes.len() > MAX_BLOB_BYTES {
            return Err(ToolkitError::TooLarge);
        }
        let report: Report =
            serde_json::from_slice(bytes).map_err(|_| ToolkitError::InvalidProtocol)?;
        if report.schema_version != 14 {
            return Err(ToolkitError::Unsupported);
        }
        if report.events.len() > MAX_EVENTS || report.participants.len() > MAX_PARTICIPANTS {
            return Err(ToolkitError::TooLarge);
        }
        if report.encounter_id.len() > 256 {
            return Err(ToolkitError::InvalidProtocol);
        }
        let identity = (report.capture_generation, report.encounter_id.clone());
        if self.identity.as_ref().is_some_and(|old| old != &identity) {
            return Err(ToolkitError::SessionChanged);
        }
        let count = report.totals.all_hits;
        let added = count
            .checked_sub(self.consumed)
            .ok_or(ToolkitError::SessionChanged)?;
        if added > report.events.len() as u64
            || report.events.len() as u64 > count
            || report.quality.dropped_events != 0
        {
            return Err(ToolkitError::DataGap);
        }
        let retained = report.events.len() - added as usize;
        // Delayed corrections without a new hit must not remain invisibly stale.
        // v1 has no event-revision cursor: stop explicitly if an already consumed
        // retained event changes, rather than guessing a replacement identity.
        if retained > self.previous.len()
            || report.events[..retained] != self.previous[self.previous.len() - retained..]
        {
            return Err(ToolkitError::DataGap);
        }
        // A full report is an append-only clock history in the same capture generation.
        // Validate everything before advancing either cursor so a failed report is retryable.
        let clock = &report.game_clock;
        if clock.transitions.len() > 4096 {
            return Err(ToolkitError::TooLarge);
        }
        if clock.status.len() > 128
            || clock.transitions.iter().any(|t| {
                !t.world_seconds.is_finite()
                    || t.world_seconds < 0.0
                    || t.unix_us > 9_007_199_254_740_991
                    || t.pause_mask >= 128
            })
            || clock.transitions.windows(2).any(|pair| {
                pair[1].unix_us < pair[0].unix_us || pair[1].world_seconds < pair[0].world_seconds
            })
            || (clock.valid && (clock.status != "ok" || clock.transitions.is_empty()))
        {
            return Err(ToolkitError::InvalidProtocol);
        }
        if clock.transitions.len() < self.clock.len()
            || clock.transitions[..self.clock.len()] != self.clock
        {
            return Err(ToolkitError::DataGap);
        }
        // Native validity can preserve the previous last-hit interval after a later
        // sampling failure. Conservatively expose unobserved boundaries as unavailable.
        let clock_available = clock.valid && clock.transitions.iter().skip(1).all(|t| t.boundary);
        let health = if clock_available {
            CombatClockRuntimeHealth::Available
        } else {
            CombatClockRuntimeHealth::DataUnavailable
        };
        let mut participants = HashMap::new();
        for p in &report.participants {
            if p.role_id.len() > 64
                || p.display_name.len() > 256
                || participants.insert(p.object_index, p).is_some()
            {
                return Err(ToolkitError::InvalidProtocol);
            }
        }
        let mut hits = Vec::with_capacity(added as usize);
        for event in &report.events {
            if !event.damage.is_finite()
                || event.damage < 0.0
                || event.unix_us > 9_007_199_254_740_991
                || event.victim_hp.is_some_and(|x| !x.is_finite() || x < 0.0)
                || event
                    .victim_max_hp
                    .is_some_and(|x| !x.is_finite() || x < 0.0)
                || [
                    &event.victim_name,
                    &event.skill_name,
                    &event.damage_attribute,
                    &event.damage_lane,
                ]
                .iter()
                .any(|x| x.len() > 512)
                || !matches!(
                    event.quality.as_str(),
                    "direct" | "correlated" | "inferred" | "unknown"
                )
            {
                return Err(ToolkitError::InvalidProtocol);
            }
            direction(&event.direction)?;
        }
        for event in &report.events[retained..] {
            let participant = participants.get(&if event.direction == "incoming" {
                event.victim_object_index
            } else {
                event.attacker_object_index
            });
            let role = participant
                .and_then(|p| p.role_id.parse::<u32>().ok())
                .filter(|id| *id != 0);
            let known = role.is_some() && matches!(event.quality.as_str(), "direct" | "correlated");
            hits.push(Hit {
                timestamp: event.unix_us as f64 / 1_000_000.0,
                char_id: if known { role.unwrap_or(0) } else { 0 },
                char_name: if known {
                    participant
                        .map(|p| p.display_name.clone())
                        .unwrap_or_default()
                } else {
                    String::new()
                },
                char_known: known,
                damage: event.damage,
                byte_offset: 0,
                bit_shift: 0,
                char_source: HitCharacterSource::Plugin,
                direction: direction(&event.direction)?,
                // Report only supplies post-hit HP. Never invent pre-hit HP or overkill.
                target_hp_before: 0.0,
                target_hp_after: event.victim_hp.unwrap_or(0.0),
                target_max_hp: event.victim_max_hp.unwrap_or(0.0),
                max_hp_reduction: 0.0,
                target_hp_percent: 0.0,
                target_id: (event.victim_object_index > 0)
                    .then(|| event.victim_object_index.to_string()),
                target_name: (!event.victim_name.is_empty()).then(|| event.victim_name.clone()),
                target_name_en: None,
                target_name_ja: None,
                target_monster_id: None,
                target_context: vec![
                    format!("plugin_quality:{}", event.quality),
                    format!("plugin_direction:{}", event.direction),
                ],
                gameplay_effect_index: None,
                gameplay_effect_name: None,
                ability_name: (!event.skill_name.is_empty()).then(|| event.skill_name.clone()),
                damage_name: None,
                damage_component: Some(event.damage_lane.clone()),
                attack_type: None,
                damage_attribute: (!event.damage_attribute.is_empty())
                    .then(|| event.damage_attribute.clone()),
                follow_up_damage: 0.0,
                follow_up_timestamp: None,
                follow_up_damage_name: None,
                follow_up_attack_type: None,
                follow_up_damage_attribute: None,
                reconciled_overkill_damage: Some(0.0),
                exact: None,
                wire_event: None,
            });
        }
        let mut timed = Vec::with_capacity(hits.len() + clock.transitions.len() - self.clock.len());
        let mut mask = self.clock.last().map_or(0, |t| t.pause_mask);
        for transition in &clock.transitions[self.clock.len()..] {
            let timestamp = transition.unix_us as f64 / 1_000_000.0;
            let event = match (mask, transition.pause_mask) {
                (0, 0) => None,
                (0, next) => Some(TimeStopEvent::GamePauseStarted {
                    timestamp,
                    pause_type_mask: next,
                }),
                (old, 0) => Some(TimeStopEvent::GamePauseEnded {
                    timestamp,
                    pause_type_mask: old,
                }),
                (old, next) if old != next => Some(TimeStopEvent::GamePauseMaskChanged {
                    timestamp,
                    pause_type_mask: next,
                }),
                _ => None,
            };
            if let Some(event) = event {
                timed.push((transition.unix_us, EngineEvent::TimeStop(event)));
            }
            mask = transition.pause_mask;
        }
        for (hit, raw) in hits.into_iter().zip(&report.events[retained..]) {
            timed.push((raw.unix_us, EngineEvent::Hit(Box::new(hit))));
        }
        // Pause transitions precede hits at the same timestamp. This preserves
        // authoritative event order through the existing capture/reducer path.
        timed.sort_by_key(|(timestamp, _)| *timestamp);
        let mut events = Vec::with_capacity(timed.len() + 1);
        if self.clock_health != Some(health) {
            events.push(EngineEvent::CombatClockHealth(health));
        }
        events.extend(timed.into_iter().map(|(_, event)| event));
        self.clock_health = Some(health);
        self.clock = report.game_clock.transitions;
        if count > 0 || !identity.1.is_empty() {
            self.identity = Some(identity);
        }
        self.consumed = count;
        self.previous = report.events;
        Ok(events)
    }
}

pub fn run(pid: u32, sender: EngineEventSink, stop: Arc<AtomicBool>) {
    let cancelled = || stop.load(Ordering::Acquire);
    // Wait for the existing session delivery permit before starting any foreign operation.
    if sender
        .send(EngineEvent::Status("plugin connecting".into()))
        .is_err()
    {
        return;
    }
    if cancelled() {
        let _ = sender.send(EngineEvent::CaptureStopped);
        return;
    }
    let mut started = false;
    let result = (|| {
        let client = ToolkitClient::open(pid)?;
        client.describe()?;
        let initial: CombatStatus = client.json(100, 0, "", &cancelled)?;
        // Do not reset a foreign capture that this session does not own.
        if initial.capturing {
            return Err(ToolkitError::Busy);
        }
        client.call(101, 0, "", &cancelled)?;
        started = true;
        wait_operation(&client, &cancelled)?;
        let mut cursor = ReportCursor::default();
        while !cancelled() {
            let status: CombatStatus = client.json(100, 0, "", &cancelled)?;
            let bytes = client.call(107, 0, "", &cancelled)?;
            for event in cursor.ingest(&bytes)? {
                if cancelled() || sender.send(event).is_err() {
                    return Err(ToolkitError::Cancelled);
                }
            }
            if !status.capturing {
                return Ok(());
            }
            // Full reports are bounded to one per second, never per frame/hit.
            for _ in 0..100 {
                if cancelled() {
                    break;
                }
                std::thread::sleep(Duration::from_millis(10));
            }
        }
        Ok(())
    })();
    if started {
        // A fresh finite group can stop our capture after cancellation. A failed
        // or timed-out stop is surfaced; mutation requests are never retried.
        let stopped = ToolkitClient::open(pid).and_then(|c| {
            c.call(102, 0, "", &|| false)?;
            wait_operation(&c, &|| false)
        });
        if let Err(error) = stopped {
            let _ = sender.send(EngineEvent::Error(format!("plugin stop: {error}")));
        }
    }
    if let Err(error) = result
        && error != ToolkitError::Cancelled
    {
        let _ = sender.send(EngineEvent::Error(format!("plugin data source: {error}")));
    }
    let _ = sender.send(EngineEvent::CaptureStopped);
}
fn wait_operation(
    client: &ToolkitClient,
    cancelled: &dyn Fn() -> bool,
) -> Result<(), ToolkitError> {
    let deadline = Instant::now() + Duration::from_secs(10);
    while Instant::now() < deadline {
        let operation: Operation = client.json(106, 0, "", cancelled)?;
        if !operation.busy {
            return if operation.ok {
                Ok(())
            } else {
                Err(ToolkitError::Failed)
            };
        }
        if cancelled() {
            return Err(ToolkitError::Cancelled);
        }
        std::thread::sleep(Duration::from_millis(25));
    }
    Err(ToolkitError::Timeout)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn clock_report() -> serde_json::Value {
        let mut value = report(100.0, "direct");
        let mut second = value["events"][0].clone();
        second["unixUs"] = serde_json::json!(11_000_000);
        value["events"].as_array_mut().unwrap().push(second);
        value["totals"]["allHits"] = serde_json::json!(2);
        value["gameClock"] = serde_json::json!({"valid":true,"status":"ok","transitions":[
            {"worldSeconds":1.0,"unixUs":1_000_000,"pauseMask":0,"boundary":false},
            {"worldSeconds":3.0,"unixUs":3_000_000,"pauseMask":4,"boundary":true},
            {"worldSeconds":7.0,"unixUs":7_000_000,"pauseMask":0,"boundary":true}
        ]});
        value
    }

    #[test]
    fn plugin_pause_intervals_reach_shared_reducer_and_duplicate_is_noop() {
        use crate::{core::reducer::apply_engine_event, engine::model::CombatState};
        let mut cursor = ReportCursor::default();
        let report = clock_report();
        let bytes = serde_json::to_vec(&report).unwrap();
        let mut state = CombatState::default();
        for event in cursor.ingest(&bytes).unwrap() {
            apply_engine_event(&mut state, event);
        }
        assert_eq!(
            state.combat_clock_health,
            CombatClockRuntimeHealth::Available
        );
        assert_eq!(state.duration_with_time_stop(false), 10.0);
        assert_eq!(state.duration_with_time_stop(true), 6.0);
        assert!(cursor.ingest(&bytes).unwrap().is_empty());

        let mut next = report;
        next["gameClock"]["transitions"]
            .as_array_mut()
            .unwrap()
            .push(serde_json::json!({
                "worldSeconds":12.0,"unixUs":12_000_000,"pauseMask":4,"boundary":true
            }));
        let events = cursor.ingest(&serde_json::to_vec(&next).unwrap()).unwrap();
        assert_eq!(
            events.len(),
            1,
            "a pause edge must be delivered even without new hits"
        );
        assert!(matches!(
            &events[0],
            EngineEvent::TimeStop(TimeStopEvent::GamePauseStarted { .. })
        ));
        assert!(
            cursor
                .ingest(&serde_json::to_vec(&next).unwrap())
                .unwrap()
                .is_empty()
        );
    }

    #[test]
    fn invalid_or_missing_plugin_clock_never_claims_adjustment() {
        let mut value = clock_report();
        value["gameClock"]["valid"] = serde_json::json!(false);
        value["gameClock"]["status"] = serde_json::json!("clock_context_changed");
        let events = ReportCursor::default()
            .ingest(&serde_json::to_vec(&value).unwrap())
            .unwrap();
        assert!(matches!(
            events.first(),
            Some(EngineEvent::CombatClockHealth(
                CombatClockRuntimeHealth::DataUnavailable
            ))
        ));
        value.as_object_mut().unwrap().remove("gameClock");
        assert_eq!(
            ReportCursor::default()
                .ingest(&serde_json::to_vec(&value).unwrap())
                .unwrap_err(),
            ToolkitError::InvalidProtocol
        );
    }

    #[test]
    fn plugin_clock_rejects_changed_prefix_and_bad_input_without_advancing() {
        let good = clock_report();
        let mut cursor = ReportCursor::default();
        let mut invalid = good.clone();
        invalid["gameClock"]["transitions"][1]["pauseMask"] = serde_json::json!(128);
        assert_eq!(
            cursor
                .ingest(&serde_json::to_vec(&invalid).unwrap())
                .unwrap_err(),
            ToolkitError::InvalidProtocol
        );
        let mut reversed = good.clone();
        reversed["gameClock"]["transitions"][1]["unixUs"] = serde_json::json!(0);
        assert_eq!(
            cursor
                .ingest(&serde_json::to_vec(&reversed).unwrap())
                .unwrap_err(),
            ToolkitError::InvalidProtocol
        );
        let mut oversized = good.clone();
        oversized["gameClock"]["transitions"] =
            serde_json::json!(vec![good["gameClock"]["transitions"][0].clone(); 4097]);
        assert_eq!(
            cursor
                .ingest(&serde_json::to_vec(&oversized).unwrap())
                .unwrap_err(),
            ToolkitError::TooLarge
        );
        assert!(
            !cursor
                .ingest(&serde_json::to_vec(&good).unwrap())
                .unwrap()
                .is_empty()
        );
        invalid = good.clone();
        invalid["gameClock"]["transitions"][1]["unixUs"] = serde_json::json!(4_000_000);
        assert_eq!(
            cursor
                .ingest(&serde_json::to_vec(&invalid).unwrap())
                .unwrap_err(),
            ToolkitError::DataGap
        );
        assert!(
            cursor
                .ingest(&serde_json::to_vec(&good).unwrap())
                .unwrap()
                .is_empty()
        );
    }
    fn report(damage: f64, quality: &str) -> serde_json::Value {
        serde_json::json!({"schemaVersion":14,"captureGeneration":1,"encounterId":"one",
            "totals":{"allHits":1},"quality":{"droppedEvents":0},
            "gameClock":{"valid":false,"status":"pause_clock_not_recorded","transitions":[]},
            "participants":[{"objectIndex":1,"roleId":"1023","displayName":"role"}],
            "events":[{"unixUs":1000000,"damage":damage,"attackerObjectIndex":1,"victimObjectIndex":2,
            "quality":quality,"direction":"outgoing","victimName":"target","victimHp":null,"victimMaxHp":null,
            "skillName":"skill","damageAttribute":"physical","damageLane":"direct"}]})
    }
    fn ingest(c: &mut ReportCursor, value: &serde_json::Value) -> Result<Vec<Hit>, ToolkitError> {
        c.ingest(&serde_json::to_vec(value).unwrap()).map(|events| {
            events
                .into_iter()
                .filter_map(|event| {
                    if let EngineEvent::Hit(hit) = event {
                        Some(*hit)
                    } else {
                        None
                    }
                })
                .collect()
        })
    }
    #[test]
    fn plugin_reports_exact_damage_no_duplicate_and_keep_unknown() {
        let mut c = ReportCursor::default();
        let r = report(123.456789, "direct");
        let hits = ingest(&mut c, &r).unwrap();
        assert_eq!(hits[0].damage, 123.456789);
        assert_eq!(hits[0].char_id, 1023);
        assert!(ingest(&mut c, &r).unwrap().is_empty());
        assert!(
            !ingest(&mut ReportCursor::default(), &report(1.0, "inferred")).unwrap()[0].char_known
        );
        println!(
            "PLUGIN_DATA_PASS: exact native damage; duplicate no-op; inferred identity stays unknown"
        );
    }
    #[test]
    fn plugin_report_boundaries_and_generation_are_fail_closed() {
        let mut c = ReportCursor::default();
        let mut r = report(1.0, "direct");
        ingest(&mut c, &r).unwrap();
        r["captureGeneration"] = 2.into();
        assert_eq!(
            ingest(&mut c, &r).unwrap_err(),
            ToolkitError::SessionChanged
        );
        r["captureGeneration"] = 1.into();
        r["events"][0]["damage"] = 2.into();
        assert_eq!(ingest(&mut c, &r).unwrap_err(), ToolkitError::DataGap);
        r["totals"]["allHits"] = 99.into();
        assert_eq!(ingest(&mut c, &r).unwrap_err(), ToolkitError::DataGap);
        r["schemaVersion"] = 15.into();
        assert_eq!(ingest(&mut c, &r).unwrap_err(), ToolkitError::Unsupported);
        assert_eq!(c.ingest(b"{}").unwrap_err(), ToolkitError::InvalidProtocol);
        assert_eq!(DataMode::default(), DataMode::PacketCapture);
    }
}
