//! Bounded adapter for UE Tools combat report schema 14, independent of packet
//! decoding. Unobserved identities stay unknown; gaps are errors, never zeros.
use crate::{
    engine::{
        capture::EngineEventSink,
        model::{EngineEvent, Hit, HitCharacterSource, HitDirection},
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
    participants: Vec<Participant>,
    events: Vec<Damage>,
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
}
impl ReportCursor {
    pub fn ingest(&mut self, bytes: &[u8]) -> Result<Vec<Hit>, ToolkitError> {
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
                wire_event: None,
            });
        }
        if count > 0 || !identity.1.is_empty() {
            self.identity = Some(identity);
        }
        self.consumed = count;
        self.previous = report.events;
        Ok(hits)
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
            for hit in cursor.ingest(&bytes)? {
                if cancelled() || sender.send(EngineEvent::Hit(Box::new(hit))).is_err() {
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
    fn report(damage: f64, quality: &str) -> serde_json::Value {
        serde_json::json!({"schemaVersion":14,"captureGeneration":1,"encounterId":"one",
            "totals":{"allHits":1},"quality":{"droppedEvents":0},
            "participants":[{"objectIndex":1,"roleId":"1023","displayName":"role"}],
            "events":[{"unixUs":1000000,"damage":damage,"attackerObjectIndex":1,"victimObjectIndex":2,
            "quality":quality,"direction":"outgoing","victimName":"target","victimHp":null,"victimMaxHp":null,
            "skillName":"skill","damageAttribute":"physical","damageLane":"direct"}]})
    }
    fn ingest(c: &mut ReportCursor, value: &serde_json::Value) -> Result<Vec<Hit>, ToolkitError> {
        c.ingest(&serde_json::to_vec(value).unwrap())
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
