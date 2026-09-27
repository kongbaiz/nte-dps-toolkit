//! Bounded adapter for UE Tools combat report schema 14, independent of packet
//! decoding. Unobserved identities stay unknown; gaps are errors, never zeros.
#[path = "toolkit_stream.rs"]
mod stream;
use crate::{
    engine::{
        capture::EngineEventSink,
        model::{
            AbyssEvent, AbyssHalf, CombatClockRuntimeHealth, EngineEvent, Hit, HitCharacterSource,
            HitDirection, TimeStopEvent,
        },
    },
    platform::toolkit::{MAX_BLOB_BYTES, ToolkitError},
};
use serde::{Deserialize, Serialize};
use std::{
    collections::HashMap,
    sync::{Arc, atomic::AtomicBool},
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
    #[serde(default)]
    abyss: Option<AbyssTimeline>,
    participants: Vec<Participant>,
    events: Vec<Damage>,
}
// Additive schema-14 extension. Older native plugins remain usable, but never
// fabricate an initial half when no delivered stage notification was observed.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct AbyssTimeline {
    schema_version: u32,
    dropped_events: u64,
    transitions: Vec<AbyssTransition>,
}
#[derive(Clone, Debug, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase")]
struct AbyssTransition {
    unix_us: u64,
    from: u8,
    to: u8,
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
pub(super) struct Participant {
    object_index: i32,
    role_id: String,
    display_name: String,
}
#[derive(Clone, Debug, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct Damage {
    #[serde(deserialize_with = "wire_u64")]
    unix_us: u64,
    damage: f64,
    attacker_object_index: i32,
    victim_object_index: i32,
    #[serde(deserialize_with = "wire_quality")]
    quality: String,
    direction: String,
    victim_name: String,
    #[serde(default)]
    attacker_name: String,
    victim_hp: Option<f64>,
    victim_max_hp: Option<f64>,
    skill_name: String,
    #[serde(default)]
    skill_key: String,
    #[serde(default)]
    attack_detail_key: String,
    #[serde(default)]
    attack_detail_name: String,
    #[serde(default)]
    display_type: Option<i32>,
    #[serde(default)]
    association: String,
    #[serde(default)]
    damage_attribute: String,
    #[serde(default)]
    damage_lane: String,
}
fn wire_u64<'de, D: serde::Deserializer<'de>>(d: D) -> Result<u64, D::Error> {
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum Value {
        Number(u64),
        Text(String),
    }
    match Value::deserialize(d)? {
        Value::Number(v) => Ok(v),
        Value::Text(v)
            if !v.is_empty() && v.len() <= 20 && v.bytes().all(|b| b.is_ascii_digit()) =>
        {
            v.parse().map_err(serde::de::Error::custom)
        }
        _ => Err(serde::de::Error::custom("invalid decimal identity")),
    }
}
fn wire_quality<'de, D: serde::Deserializer<'de>>(d: D) -> Result<String, D::Error> {
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum Value {
        Number(u8),
        Text(String),
    }
    match Value::deserialize(d)? {
        Value::Number(v) => ["direct", "correlated", "inferred", "unknown"]
            .get(v as usize)
            .map(|s| s.to_string())
            .ok_or_else(|| serde::de::Error::custom("invalid quality")),
        Value::Text(v)
            if matches!(v.as_str(), "direct" | "correlated" | "inferred" | "unknown") =>
        {
            Ok(v)
        }
        _ => Err(serde::de::Error::custom("invalid quality")),
    }
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
    abyss: Vec<AbyssTransition>,
    abyss_supported: bool,
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
        // Native `valid` describes an anchored outgoing DPS interval, not whether
        // pause acquisition is ready. Before the first outgoing hit the clock
        // reports awaiting_outgoing_damage even after observing valid pause edges.
        let awaiting_first_hit = clock.status == "awaiting_outgoing_damage"
            && !clock.transitions.is_empty()
            && report
                .events
                .iter()
                .all(|event| event.direction != "outgoing");
        let clock_available = (clock.valid || awaiting_first_hit)
            && clock.transitions.iter().skip(1).all(|t| t.boundary);
        let health = if clock_available {
            CombatClockRuntimeHealth::Available
        } else {
            CombatClockRuntimeHealth::DataUnavailable
        };
        let abyss = match &report.abyss {
            Some(abyss) => {
                if abyss.schema_version != 1 {
                    return Err(ToolkitError::Unsupported);
                }
                if abyss.transitions.len() > 4096 {
                    return Err(ToolkitError::TooLarge);
                }
                if abyss.dropped_events != 0 {
                    return Err(ToolkitError::DataGap);
                }
                if abyss.transitions.iter().any(|t| {
                    t.unix_us == 0 || t.unix_us > 9_007_199_254_740_991 || t.from > 2 || t.to > 2
                }) || abyss
                    .transitions
                    .windows(2)
                    .any(|p| p[1].unix_us < p[0].unix_us)
                {
                    return Err(ToolkitError::InvalidProtocol);
                }
                abyss.transitions.as_slice()
            }
            None if self.abyss_supported => return Err(ToolkitError::DataGap),
            None => &[],
        };
        if abyss.len() < self.abyss.len() || abyss[..self.abyss.len()] != self.abyss {
            return Err(ToolkitError::DataGap);
        }
        // A newly disclosed stage edge cannot silently reassign hits already delivered.
        if let Some(last) = self.previous.last()
            && abyss[self.abyss.len()..]
                .iter()
                .any(|t| t.unix_us < last.unix_us)
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
                    &event.skill_key,
                    &event.attack_detail_key,
                    &event.attack_detail_name,
                    &event.association,
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
            hits.push(project_damage(event, &participants)?);
        }
        let mut timed = Vec::with_capacity(hits.len() + clock.transitions.len() - self.clock.len());
        for transition in &abyss[self.abyss.len()..] {
            let timestamp = transition.unix_us as f64 / 1_000_000.0;
            let event = match transition.to {
                0 => AbyssEvent::Exit { timestamp },
                1 | 2 => AbyssEvent::Stage {
                    timestamp,
                    cycle: None,
                    floor: None,
                    half: if transition.to == 1 {
                        AbyssHalf::First
                    } else {
                        AbyssHalf::Second
                    },
                    // A delivered notification only proves state from its timestamp.
                    allow_late_backfill: false,
                },
                _ => unreachable!("validated stage"),
            };
            timed.push((transition.unix_us, EngineEvent::Abyss(event)));
        }
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
        // Stage and pause transitions precede hits at the same timestamp. This preserves
        // authoritative event order through the existing capture/reducer path.
        timed.sort_by_key(|(timestamp, _)| *timestamp);
        let mut events = Vec::with_capacity(timed.len() + 1);
        if self.clock_health != Some(health) {
            events.push(EngineEvent::CombatClockHealth(health));
        }
        events.extend(timed.into_iter().map(|(_, event)| event));
        self.abyss = abyss.to_vec();
        self.abyss_supported = report.abyss.is_some();
        self.clock_health = Some(health);
        self.clock = report.game_clock.transitions;
        if count > 0 || !identity.1.is_empty() || !self.abyss.is_empty() {
            self.identity = Some(identity);
        }
        self.consumed = count;
        self.previous = report.events;
        Ok(events)
    }
}

fn project_damage(
    event: &Damage,
    participants: &HashMap<i32, &Participant>,
) -> Result<Hit, ToolkitError> {
    let participant = participants.get(&if event.direction == "incoming" {
        event.victim_object_index
    } else {
        event.attacker_object_index
    });
    let role = participant
        .and_then(|p| p.role_id.parse::<u32>().ok())
        .filter(|id| *id != 0);
    // Attribution quality also describes missing skill/critical metadata.
    // Explicit server categories retain their independently observed actor.
    let server_category = event.display_type.is_some_and(|v| (22..=28).contains(&v))
        && event.association == "server_reaction_category";
    let known = role.is_some()
        && (matches!(event.quality.as_str(), "direct" | "correlated") || server_category);
    let special = event
        .display_type
        .and_then(|v| crate::engine::parser::DamageDisplayType::try_from(v).ok())
        .and_then(|v| v.damage_name().map(|name| (name, v.attack_type())));
    let effect = event
        .attack_detail_key
        .strip_suffix("_C")
        .unwrap_or(&event.attack_detail_key);
    let effect = (!effect.is_empty()).then_some(effect);
    let skill = event
        .skill_key
        .strip_suffix("_C")
        .unwrap_or(&event.skill_key);
    let component = if special.is_none() || event.display_type == Some(22) {
        effect
            .and_then(crate::storage::ability_names::resolve_damage_name)
            .or_else(|| {
                (!event.attack_detail_name.is_empty()).then(|| event.attack_detail_name.clone())
            })
    } else {
        None
    };
    Ok(Hit {
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
        target_id: (event.victim_object_index > 0).then(|| event.victim_object_index.to_string()),
        target_name: (!event.victim_name.is_empty()).then(|| event.victim_name.clone()),
        target_name_en: None,
        target_name_ja: None,
        target_monster_id: None,
        target_context: vec![
            format!("plugin_quality:{}", event.quality),
            format!("plugin_direction:{}", event.direction),
        ],
        gameplay_effect_index: None,
        gameplay_effect_name: effect.map(str::to_owned),
        ability_name: (!skill.is_empty()).then(|| skill.to_owned()),
        damage_name: special
            .map(|v| v.0.to_owned())
            .or_else(|| (!event.skill_name.is_empty()).then(|| event.skill_name.clone())),
        damage_component: component,
        attack_type: special.and_then(|v| v.1.map(str::to_owned)),
        damage_attribute: (!event.damage_attribute.is_empty())
            .then(|| event.damage_attribute.clone()),
        follow_up_damage: 0.0,
        follow_up_timestamp: None,
        follow_up_damage_name: None,
        follow_up_attack_type: None,
        follow_up_damage_attribute: None,
        reconciled_overkill_damage: Some(0.0),
        exact: None,
        plugin_snapshot: None,
        wire_event: None,
    })
}

pub fn run(
    pid: u32,
    sender: EngineEventSink,
    stop: Arc<AtomicBool>,
    router: Arc<super::equipment_rpc::Router>,
) {
    stream::run(pid, sender, stop, router);
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
    fn plugin_clock_ready_before_first_outgoing_hit_is_not_a_missing_pause_state() {
        let mut value = clock_report();
        value["events"] = serde_json::json!([]);
        value["totals"]["allHits"] = serde_json::json!(0);
        value["gameClock"]["valid"] = serde_json::json!(false);
        value["gameClock"]["status"] = serde_json::json!("awaiting_outgoing_damage");
        let mut cursor = ReportCursor::default();
        let events = cursor.ingest(&serde_json::to_vec(&value).unwrap()).unwrap();
        assert!(matches!(
            events.first(),
            Some(EngineEvent::CombatClockHealth(
                CombatClockRuntimeHealth::Available
            ))
        ));
        assert!(
            cursor
                .ingest(&serde_json::to_vec(&value).unwrap())
                .unwrap()
                .is_empty()
        );

        // A real sampling failure must still degrade, even with no outgoing hits.
        value["gameClock"]["status"] = serde_json::json!("pause_clock_read_failed");
        let events = cursor.ingest(&serde_json::to_vec(&value).unwrap()).unwrap();
        assert!(matches!(
            events.as_slice(),
            [EngineEvent::CombatClockHealth(
                CombatClockRuntimeHealth::DataUnavailable
            )]
        ));
    }

    #[test]
    fn awaiting_plugin_clock_requires_observed_boundaries_and_no_outgoing_hits() {
        let mut value = clock_report();
        value["gameClock"]["valid"] = serde_json::json!(false);
        value["gameClock"]["status"] = serde_json::json!("awaiting_outgoing_damage");
        for variant in 0..3 {
            let mut invalid = value.clone();
            if variant != 0 {
                invalid["events"] = serde_json::json!([]);
                invalid["totals"]["allHits"] = serde_json::json!(0);
            }
            if variant == 1 {
                invalid["gameClock"]["transitions"] = serde_json::json!([]);
            }
            if variant == 2 {
                invalid["gameClock"]["transitions"][1]["boundary"] = serde_json::json!(false);
            }
            let events = ReportCursor::default()
                .ingest(&serde_json::to_vec(&invalid).unwrap())
                .unwrap();
            assert!(matches!(
                events.first(),
                Some(EngineEvent::CombatClockHealth(
                    CombatClockRuntimeHealth::DataUnavailable
                ))
            ));
        }
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
    fn with_abyss(mut value: serde_json::Value) -> serde_json::Value {
        value["abyss"] = serde_json::json!({"schemaVersion":1,"droppedEvents":0,"transitions":[
            {"unixUs":1_000_000,"from":0,"to":1},
            {"unixUs":11_000_000,"from":1,"to":2}
        ]});
        value
    }

    #[test]
    fn plugin_stage_notifications_split_halves_before_same_timestamp_hits() {
        use crate::{core::reducer::apply_engine_event, engine::model::CombatState};
        let mut value = with_abyss(clock_report());
        // Separate team members, as in a two-team Abyss challenge.
        value["participants"]
            .as_array_mut()
            .unwrap()
            .push(serde_json::json!({
                "objectIndex":3,"roleId":"1010","displayName":"second role"
            }));
        value["events"][1]["attackerObjectIndex"] = 3.into();
        let mut cursor = ReportCursor::default();
        let mut state = CombatState::default();
        for event in cursor.ingest(&serde_json::to_vec(&value).unwrap()).unwrap() {
            apply_engine_event(&mut state, event);
        }
        assert_eq!(state.abyss.active_half, Some(AbyssHalf::Second));
        assert_eq!(state.abyss.first_half.total_damage, 100.0);
        assert_eq!(state.abyss.second_half.total_damage, 100.0);
        assert_eq!(state.total_damage, 200.0);
        assert_eq!(
            state.abyss.floor, None,
            "a layer notification does not prove a floor number"
        );
        assert!(
            cursor
                .ingest(&serde_json::to_vec(&value).unwrap())
                .unwrap()
                .is_empty()
        );

        value["abyss"]["transitions"]
            .as_array_mut()
            .unwrap()
            .push(serde_json::json!({
                "unixUs":12_000_000,"from":2,"to":0
            }));
        let events = cursor.ingest(&serde_json::to_vec(&value).unwrap()).unwrap();
        assert!(matches!(
            events.as_slice(),
            [EngineEvent::Abyss(AbyssEvent::Exit { .. })]
        ));
        for event in events {
            apply_engine_event(&mut state, event);
        }
        assert_eq!(state.abyss.active_half, None);
    }

    #[test]
    fn plugin_stage_switch_without_damage_emits_once_and_never_backfills_unknown_hits() {
        use crate::{core::reducer::apply_engine_event, engine::model::CombatState};
        let mut cursor = ReportCursor::default();
        let mut state = CombatState::default();
        let mut value = report(100.0, "direct");
        value["abyss"] = serde_json::json!({"schemaVersion":1,"droppedEvents":0,"transitions":[]});
        for event in cursor.ingest(&serde_json::to_vec(&value).unwrap()).unwrap() {
            apply_engine_event(&mut state, event);
        }
        value["abyss"]["transitions"] = serde_json::json!([{ "unixUs":2_000_000,"from":1,"to":2 }]);
        let events = cursor.ingest(&serde_json::to_vec(&value).unwrap()).unwrap();
        assert!(matches!(
            events.as_slice(),
            [EngineEvent::Abyss(AbyssEvent::Stage {
                half: AbyssHalf::Second,
                allow_late_backfill: false,
                ..
            })]
        ));
        for event in events {
            apply_engine_event(&mut state, event);
        }
        assert_eq!(state.abyss.active_half, Some(AbyssHalf::Second));
        assert_eq!(state.abyss.first_half.total_damage, 0.0);
        assert_eq!(state.abyss.second_half.total_damage, 0.0);
        assert!(
            cursor
                .ingest(&serde_json::to_vec(&value).unwrap())
                .unwrap()
                .is_empty()
        );
    }

    #[test]
    fn plugin_stage_malformed_gap_and_budget_failure_leave_cursor_retryable() {
        let good = with_abyss(clock_report());
        for (field, value, expected) in [
            ("schemaVersion", 2.into(), ToolkitError::Unsupported),
            ("droppedEvents", 1.into(), ToolkitError::DataGap),
            (
                "transitions",
                serde_json::json!([{"unixUs":0,"from":0,"to":1}]),
                ToolkitError::InvalidProtocol,
            ),
            (
                "transitions",
                serde_json::json!([{"unixUs":1,"from":0,"to":3}]),
                ToolkitError::InvalidProtocol,
            ),
            (
                "transitions",
                serde_json::json!([{"unixUs":2,"from":0,"to":1},{"unixUs":1,"from":1,"to":2}]),
                ToolkitError::InvalidProtocol,
            ),
            (
                "transitions",
                serde_json::json!([{ "unixUs":9_007_199_254_740_992_u64,"from":0,"to":1 }]),
                ToolkitError::InvalidProtocol,
            ),
            (
                "transitions",
                serde_json::json!(vec![serde_json::json!({"unixUs":1,"from":0,"to":1}); 4097]),
                ToolkitError::TooLarge,
            ),
        ] {
            let mut cursor = ReportCursor::default();
            let mut bad = good.clone();
            bad["abyss"][field] = value;
            assert_eq!(
                cursor
                    .ingest(&serde_json::to_vec(&bad).unwrap())
                    .unwrap_err(),
                expected
            );
            assert!(cursor.ingest(&serde_json::to_vec(&good).unwrap()).is_ok());
        }
    }

    #[test]
    fn plugin_stage_observation_freezes_generation_even_before_damage_or_encounter_label() {
        let mut value = with_abyss(clock_report());
        value["events"] = serde_json::json!([]);
        value["totals"]["allHits"] = 0.into();
        value["encounterId"] = "".into();
        let mut cursor = ReportCursor::default();
        cursor.ingest(&serde_json::to_vec(&value).unwrap()).unwrap();
        value["captureGeneration"] = 2.into();
        assert_eq!(
            cursor
                .ingest(&serde_json::to_vec(&value).unwrap())
                .unwrap_err(),
            ToolkitError::SessionChanged
        );
    }

    #[test]
    fn plugin_stage_prefix_cannot_change_shrink_or_disappear() {
        let good = with_abyss(clock_report());
        let mut cursor = ReportCursor::default();
        cursor.ingest(&serde_json::to_vec(&good).unwrap()).unwrap();
        for variant in 0..3 {
            let mut bad = good.clone();
            match variant {
                0 => bad["abyss"]["transitions"][0]["to"] = 2.into(),
                1 => {
                    bad["abyss"]["transitions"].as_array_mut().unwrap().pop();
                }
                _ => {
                    bad.as_object_mut().unwrap().remove("abyss");
                }
            }
            assert_eq!(
                cursor
                    .ingest(&serde_json::to_vec(&bad).unwrap())
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
