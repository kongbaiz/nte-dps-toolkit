//! Connection-owned native hit push consumer. No complete report polling.
use super::{Damage, Participant, project_damage, wire_u64};
use crate::engine::model::plugin_snapshot::{
    HitSnapshotData, MAX_SESSION_SNAPSHOT_BYTES, MAX_SESSION_SNAPSHOTS, PluginHitSnapshot,
};
use crate::{
    engine::{
        capture::EngineEventSink,
        model::{AbyssEvent, AbyssHalf, CombatClockRuntimeHealth, EngineEvent, TimeStopEvent},
    },
    platform::{
        capture_pipe::{CapturePipe, MAX_FRAME},
        toolkit::{ToolkitClient, ToolkitError},
    },
};
fn snapshot_error(
    error: crate::engine::model::plugin_snapshot::SnapshotValidationError,
) -> ToolkitError {
    match error {
        crate::engine::model::plugin_snapshot::SnapshotValidationError::TooLarge => {
            ToolkitError::TooLarge
        }
        crate::engine::model::plugin_snapshot::SnapshotValidationError::InvalidFormat => {
            ToolkitError::InvalidProtocol
        }
    }
}
use serde::Deserialize;
use serde_json::{Value, json};
use std::{
    collections::{HashMap, VecDeque},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ContextRef {
    context_id: String,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct StreamHit {
    #[serde(flatten)]
    event: Damage,
    #[serde(deserialize_with = "wire_u64")]
    hit_id: u64,
    context_ref: Option<ContextRef>,
    #[serde(flatten)]
    snapshot_data: HitSnapshotData,
    #[serde(default)]
    critical_known: bool,
    critical: Option<bool>,
    critical_source: Option<String>,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct TeamActor {
    object_key: Option<String>,
    #[serde(rename = "DefaultCharacterID")]
    role: Option<String>,
}
#[derive(Deserialize)]
struct Team {
    actors: Option<Vec<TeamActor>>,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Context {
    context_id: String,
    schema_version: u32,
    team: Option<Team>,
}
#[derive(Deserialize)]
struct ClockRow {
    #[serde(deserialize_with = "wire_u64")]
    sequence: u64,
    #[serde(deserialize_with = "wire_u64")]
    timestamp_100ns: u64,
    pause_type_mask: u32,
    state_flags: u32,
    reserved_value: u32,
}
#[derive(Deserialize)]
struct Clock {
    #[serde(rename = "providerId")]
    provider_id: String,
    transitions: Vec<ClockRow>,
}
fn decimal(v: &Value) -> Result<u64, ToolkitError> {
    let s = v.as_str().ok_or(ToolkitError::InvalidProtocol)?;
    if s.is_empty() || s.len() > 20 || !s.bytes().all(|c| c.is_ascii_digit()) {
        return Err(ToolkitError::InvalidProtocol);
    }
    s.parse().map_err(|_| ToolkitError::InvalidProtocol)
}
fn text<'a>(v: &'a Value, key: &str) -> Result<&'a str, ToolkitError> {
    v[key]
        .as_str()
        .filter(|s| !s.is_empty() && s.len() <= 256)
        .ok_or(ToolkitError::InvalidProtocol)
}
fn parse(bytes: &[u8]) -> Result<Value, ToolkitError> {
    if bytes.len() > MAX_FRAME {
        return Err(ToolkitError::TooLarge);
    }
    let v: Value = serde_json::from_slice(bytes).map_err(|_| ToolkitError::InvalidProtocol)?;
    if v["jsonrpc"] != "2.0" {
        return Err(ToolkitError::InvalidProtocol);
    }
    Ok(v)
}
struct Cursor {
    provider: String,
    capture: String,
    seq: u64,
    last_frame: Vec<u8>,
    hit: u64,
    hits: u64,
    contexts: HashMap<String, HashMap<String, String>>,
    order: VecDeque<String>,
    clock_seq: Option<u64>,
    clock_timestamp: u64,
    pending_events: Vec<EngineEvent>,
    snapshot_bytes: usize,
    snapshot_count: usize,
    clock_mask: u32,
    clock_ready: bool,
    ended: bool,
    abyss_timestamp: u64,
    round_id: u64,
}
impl Cursor {
    fn new(provider: String, capture: String) -> Self {
        Self {
            provider,
            capture,
            seq: 0,
            last_frame: vec![],
            hit: 0,
            hits: 0,
            contexts: HashMap::new(),
            order: VecDeque::new(),
            clock_seq: None,
            clock_timestamp: 0,
            pending_events: Vec::new(),
            snapshot_bytes: 0,
            snapshot_count: 0,
            clock_mask: 0,
            clock_ready: false,
            ended: false,
            abyss_timestamp: 0,
            round_id: 0,
        }
    }
    fn notification(
        &mut self,
        bytes: Vec<u8>,
        mut v: Value,
    ) -> Result<Vec<EngineEvent>, ToolkitError> {
        let method = text(&v, "method")?.to_owned();
        let mut p = v["params"].take();
        if text(&p, "providerId")? != self.provider || text(&p, "captureId")? != self.capture {
            return Err(ToolkitError::SessionChanged);
        }
        let seq = decimal(&p["seq"])?;
        if seq == self.seq && self.last_frame == bytes {
            return Ok(vec![]);
        }
        if seq != self.seq + 1 || self.ended {
            return Err(ToolkitError::DataGap);
        }
        let mut events = vec![];
        match method.as_str() {
            "event.combat.started" => {}
            "event.combat.context" => {
                let Value::Array(rows) = p["contexts"].take() else {
                    return Err(ToolkitError::InvalidProtocol);
                };
                if rows.len() > 6 {
                    return Err(ToolkitError::TooLarge);
                }
                for row in rows {
                    // Context change events do not replace the full observed team snapshot.
                    if row["recordType"] == "event" {
                        continue;
                    }
                    let c: Context =
                        serde_json::from_value(row).map_err(|_| ToolkitError::InvalidProtocol)?;
                    if c.schema_version != 1 || c.context_id.is_empty() || c.context_id.len() > 20 {
                        return Err(ToolkitError::InvalidProtocol);
                    }
                    let mut actors = HashMap::new();
                    if let Some(rows) = c.team.and_then(|t| t.actors) {
                        if rows.len() > 16 {
                            return Err(ToolkitError::TooLarge);
                        }
                        for a in rows {
                            if let (Some(key), Some(role)) = (a.object_key, a.role) {
                                if key.len() > 48 || role.len() > 64 {
                                    return Err(ToolkitError::TooLarge);
                                }
                                if actors.insert(key, role).is_some() {
                                    return Err(ToolkitError::InvalidProtocol);
                                }
                            }
                        }
                    }
                    if self.contexts.contains_key(&c.context_id) {
                        return Err(ToolkitError::InvalidProtocol);
                    }
                    if self.order.len() == 256
                        && let Some(old) = self.order.pop_front()
                    {
                        self.contexts.remove(&old);
                    }
                    self.order.push_back(c.context_id.clone());
                    self.contexts.insert(c.context_id, actors);
                }
            }
            "event.combat.hits" => {
                let Value::Array(rows) = p["hits"].take() else {
                    return Err(ToolkitError::InvalidProtocol);
                };
                let count = rows.len();
                if rows.len() > 64 {
                    return Err(ToolkitError::TooLarge);
                }
                // Damage must not enter a wall-time projection while the plugin clock
                // is still initializing. Bounded, reliable startup staging; overflow fails.
                if !self.clock_ready && self.pending_events.len() + count > 4096 {
                    return Err(ToolkitError::TooLarge);
                }
                let mut snapshot_bytes = self.snapshot_bytes;
                let mut snapshot_count = self.snapshot_count;
                let mut last = self.hit;
                for row in rows {
                    let h: StreamHit =
                        serde_json::from_value(row).map_err(|_| ToolkitError::InvalidProtocol)?;
                    let e = &h.event;
                    if h.hit_id <= last
                        || !e.damage.is_finite()
                        || e.damage <= 0.0
                        || e.unix_us > 9_007_199_254_740_991
                        || e.victim_hp.is_some_and(|v| !v.is_finite() || v < 0.0)
                        || e.victim_max_hp.is_some_and(|v| !v.is_finite() || v < 0.0)
                        || [
                            &e.skill_name,
                            &e.skill_key,
                            &e.attack_detail_key,
                            &e.attack_detail_name,
                            &e.attacker_name,
                            &e.victim_name,
                            &e.association,
                        ]
                        .iter()
                        .any(|v| v.len() > 512)
                    {
                        return Err(ToolkitError::InvalidProtocol);
                    }
                    super::direction(&e.direction)?;
                    let incoming = e.direction == "incoming";
                    let index = if incoming {
                        e.victim_object_index
                    } else {
                        e.attacker_object_index
                    };
                    let candidates = if incoming {
                        [
                            h.snapshot_data
                                .victim_attributes
                                .as_ref()
                                .map(|v| (v.actor_index, v.actor_serial)),
                            h.snapshot_data
                                .victim_effects
                                .as_ref()
                                .map(|v| (v.actor_index, v.actor_serial)),
                        ]
                    } else {
                        [
                            h.snapshot_data
                                .attacker_attributes
                                .as_ref()
                                .map(|v| (v.actor_index, v.actor_serial)),
                            h.snapshot_data
                                .attacker_effects
                                .as_ref()
                                .map(|v| (v.actor_index, v.actor_serial)),
                        ]
                    };
                    let mut candidates = candidates
                        .into_iter()
                        .flatten()
                        .filter(|w| w.0 == index && w.1 > 0);
                    let first = candidates.next();
                    let witness = first.filter(|first| candidates.all(|w| w.1 == first.1));
                    let role = h
                        .context_ref
                        .as_ref()
                        .and_then(|c| self.contexts.get(&c.context_id))
                        .and_then(|actors| {
                            witness
                                .filter(|w| w.0 == index && w.1 > 0)
                                .and_then(|w| actors.get(&format!("{}:{}", w.0, w.1)))
                        });
                    let participant = role.map(|role| Participant {
                        object_index: index,
                        role_id: role.clone(),
                        display_name: if incoming {
                            e.victim_name.clone()
                        } else {
                            e.attacker_name.clone()
                        },
                    });
                    let participants = participant
                        .as_ref()
                        .map(|p| HashMap::from([(index, p)]))
                        .unwrap_or_default();
                    let mut hit = project_damage(e, &participants)?;
                    let size = h.snapshot_data.validate().map_err(snapshot_error)?;
                    if h.critical_known && h.critical.is_none()
                        || h.critical_source.as_ref().is_some_and(|s| s.len() > 256)
                    {
                        return Err(ToolkitError::InvalidProtocol);
                    }
                    let attacker = h
                        .snapshot_data
                        .attacker_effects
                        .as_ref()
                        .and_then(|s| s.counts());
                    let victim = h
                        .snapshot_data
                        .victim_effects
                        .as_ref()
                        .and_then(|s| s.counts());
                    let (role_effects, enemy_effects) = match e.direction.as_str() {
                        "outgoing" => (attacker, victim),
                        "incoming" => (victim, attacker),
                        _ => (None, None),
                    };
                    let present = h.snapshot_data.has_observations();
                    let retain = present
                        && snapshot_count < MAX_SESSION_SNAPSHOTS
                        && snapshot_bytes.saturating_add(size) <= MAX_SESSION_SNAPSHOT_BYTES;
                    if retain {
                        snapshot_count += 1;
                        snapshot_bytes += size;
                    }
                    let snapshot = PluginHitSnapshot {
                        instance_id: 0,
                        key: format!("{}:{}", self.capture, h.hit_id),
                        critical: h.critical_known.then_some(h.critical).flatten(),
                        critical_source: if h.critical_known {
                            h.critical_source
                        } else {
                            None
                        },
                        role_effects,
                        enemy_effects,
                        retention: if retain {
                            "available"
                        } else if present {
                            "budget_exceeded"
                        } else {
                            "missing"
                        }
                        .into(),
                        data: retain.then_some(h.snapshot_data),
                    };
                    hit.plugin_snapshot = Some(snapshot.seal().map_err(snapshot_error)?);
                    hit.target_context
                        .push("plugin_transport:native_push".into());
                    if role.is_some() {
                        hit.target_context
                            .push("plugin_identity:frozen_context_actor_serial".into());
                    }
                    events.push(EngineEvent::Hit(Box::new(hit)));
                    last = h.hit_id;
                }
                self.snapshot_bytes = snapshot_bytes;
                self.snapshot_count = snapshot_count;
                self.hit = last;
                self.hits += count as u64;
                if !self.clock_ready {
                    self.pending_events.append(&mut events);
                }
            }
            "event.combat.rounds" => {
                let rows = p["events"]
                    .as_array()
                    .ok_or(ToolkitError::InvalidProtocol)?;
                if rows.is_empty()
                    || rows.len() > 64
                    || (!self.clock_ready && self.pending_events.len() + rows.len() > 4096)
                {
                    return Err(ToolkitError::TooLarge);
                }
                let mut last = self.round_id;
                for row in rows {
                    let id = decimal(&row["restartId"])?;
                    let us = decimal(&row["unixUs"])?;
                    if id <= last
                        || us == 0
                        || us > 9_007_199_254_740_991
                        || row["kind"] != "restart"
                    {
                        return Err(ToolkitError::InvalidProtocol);
                    }
                    let clone_type = match row["cloneType"].as_u64() {
                        Some(value @ (9 | 16)) => value as u8,
                        _ => return Err(ToolkitError::InvalidProtocol),
                    };
                    events.push(EngineEvent::ChallengeRestart {
                        timestamp: us as f64 / 1_000_000.0,
                        clone_type,
                    });
                    last = id;
                }
                self.round_id = last;
                if !self.clock_ready {
                    self.pending_events.append(&mut events);
                }
            }
            "event.combat.abyss" => {
                let rows = p["events"]
                    .as_array()
                    .ok_or(ToolkitError::InvalidProtocol)?;
                if rows.is_empty()
                    || rows.len() > 64
                    || (!self.clock_ready && self.pending_events.len() + rows.len() > 4096)
                {
                    return Err(ToolkitError::TooLarge);
                }
                let mut last = self.abyss_timestamp;
                for row in rows {
                    if row.get("half").is_none() || row.get("floor").is_none() {
                        return Err(ToolkitError::InvalidProtocol);
                    }
                    let us = decimal(&row["unixUs"])?;
                    if us == 0 || us > 9_007_199_254_740_991 || us < last {
                        return Err(ToolkitError::InvalidProtocol);
                    }
                    let timestamp = us as f64 / 1_000_000.0;
                    let event = match text(row, "kind")? {
                        "location" => {
                            if !row["half"].is_null() {
                                return Err(ToolkitError::InvalidProtocol);
                            }
                            let floor = row["floor"]
                                .as_u64()
                                .filter(|n| *n > 0 && *n <= i32::MAX as u64)
                                .ok_or(ToolkitError::InvalidProtocol)?
                                as u32;
                            AbyssEvent::Location { timestamp, floor }
                        }
                        "stage" => {
                            let half = match row["half"].as_u64() {
                                Some(1) => AbyssHalf::First,
                                Some(2) => AbyssHalf::Second,
                                _ => return Err(ToolkitError::InvalidProtocol),
                            };
                            let floor = if row["floor"].is_null() {
                                None
                            } else {
                                Some(
                                    row["floor"]
                                        .as_u64()
                                        .filter(|n| *n > 0 && *n <= i32::MAX as u64)
                                        .ok_or(ToolkitError::InvalidProtocol)?
                                        as u32,
                                )
                            };
                            AbyssEvent::Stage {
                                timestamp,
                                cycle: None,
                                floor,
                                half,
                                allow_late_backfill: false,
                            }
                        }
                        "restart_half" => {
                            if !row["floor"].is_null() {
                                return Err(ToolkitError::InvalidProtocol);
                            }
                            let half = match row["half"].as_u64() {
                                Some(1) => AbyssHalf::First,
                                Some(2) => AbyssHalf::Second,
                                _ => return Err(ToolkitError::InvalidProtocol),
                            };
                            AbyssEvent::RestartHalf { timestamp, half }
                        }
                        kind @ ("restart" | "success" | "exit") => {
                            if !row["half"].is_null() || !row["floor"].is_null() {
                                return Err(ToolkitError::InvalidProtocol);
                            }
                            match kind {
                                "restart" => AbyssEvent::RestartDetected { timestamp },
                                "success" => AbyssEvent::Success { timestamp },
                                _ => AbyssEvent::Exit { timestamp },
                            }
                        }
                        _ => return Err(ToolkitError::InvalidProtocol),
                    };
                    events.push(EngineEvent::Abyss(event));
                    last = us;
                }
                self.abyss_timestamp = last;
                if !self.clock_ready {
                    self.pending_events.append(&mut events);
                }
            }
            "event.combat.effects" => {} // Buff presence records are not damage events.
            "event.combat.ended" => {
                if decimal(&p["hitCount"])? != self.hits {
                    return Err(ToolkitError::DataGap);
                }
                for key in [
                    "droppedRecords",
                    "sourceDroppedRecords",
                    "rejectedTargets",
                    "droppedContexts",
                ] {
                    if decimal(&p[key])? != 0 {
                        return Err(ToolkitError::DataGap);
                    }
                }
                self.ended = true;
            }
            _ => return Err(ToolkitError::Unsupported),
        }
        self.seq = seq;
        self.last_frame = bytes;
        Ok(events)
    }
    fn clock(&mut self, v: Value) -> Result<Vec<EngineEvent>, ToolkitError> {
        let c: Clock = serde_json::from_value(v).map_err(|_| ToolkitError::InvalidProtocol)?;
        if c.provider_id != self.provider {
            return Err(ToolkitError::SessionChanged);
        }
        if c.transitions.len() > 64 {
            return Err(ToolkitError::TooLarge);
        }
        if c.transitions.windows(2).any(|rows| {
            rows[0].sequence >= rows[1].sequence
                || rows[0].timestamp_100ns > rows[1].timestamp_100ns
        }) {
            return Err(ToolkitError::InvalidProtocol);
        }
        let mut out = vec![];
        let (mut sequence, mut timestamp, mut mask, mut ready) = (
            self.clock_seq,
            self.clock_timestamp,
            self.clock_mask,
            self.clock_ready,
        );
        // CaptureClock::Reset clears the history but deliberately keeps its provider-wide
        // sequence. The first row is this capture's baseline, not necessarily sequence 1.
        // A completely full first ring cannot prove its initial boundary survived.
        if sequence.is_none() && c.transitions.len() == 64 {
            return Err(ToolkitError::DataGap);
        }
        for r in c.transitions {
            if r.sequence == 0
                || r.reserved_value != 0
                || r.state_flags > 1
                || r.pause_type_mask >= 128
            {
                return Err(ToolkitError::InvalidProtocol);
            }
            if sequence.is_some_and(|last| r.sequence <= last) {
                continue;
            }
            if sequence.is_some_and(|last| last.checked_add(1) != Some(r.sequence)) {
                return Err(ToolkitError::DataGap);
            }
            // Missing plugin time is an acquisition failure, never permission to
            // substitute desktop wall time for an active plugin capture.
            if r.state_flags != 1 {
                return Err(ToolkitError::Unavailable);
            }
            if r.timestamp_100ns < timestamp {
                return Err(ToolkitError::InvalidProtocol);
            }
            let ticks = r
                .timestamp_100ns
                .checked_sub(116444736000000000)
                .ok_or(ToolkitError::InvalidProtocol)?;
            let seconds = ticks as f64 / 10_000_000.0;
            if !ready {
                out.push(EngineEvent::CombatClockHealth(
                    CombatClockRuntimeHealth::Available,
                ));
                ready = true;
            }
            let event = match (mask, r.pause_type_mask) {
                (0, 0) => None,
                (0, next) => Some(TimeStopEvent::GamePauseStarted {
                    timestamp: seconds,
                    pause_type_mask: next,
                }),
                (old, 0) => Some(TimeStopEvent::GamePauseEnded {
                    timestamp: seconds,
                    pause_type_mask: old,
                }),
                (old, next) if old != next => Some(TimeStopEvent::GamePauseMaskChanged {
                    timestamp: seconds,
                    pause_type_mask: next,
                }),
                _ => None,
            };
            if let Some(event) = event {
                out.push(EngineEvent::TimeStop(event));
            }
            sequence = Some(r.sequence);
            timestamp = r.timestamp_100ns;
            mask = r.pause_type_mask;
        }
        // Commit only after validating the whole clock response, so retries of a
        // rejected response cannot lose edges or publish a partially advanced clock.
        self.clock_seq = sequence;
        self.clock_timestamp = timestamp;
        self.clock_mask = mask;
        self.clock_ready = ready;
        if ready {
            out.append(&mut self.pending_events);
        }
        Ok(out)
    }
}
fn request(
    pipe: &CapturePipe,
    id: &str,
    method: &str,
    params: Value,
    stop: &AtomicBool,
) -> Result<Value, ToolkitError> {
    pipe.send(id, method, params)?;
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        if let Some(bytes) = pipe.read(stop)? {
            let v = parse(&bytes)?;
            if v["id"] != id {
                return Err(ToolkitError::InvalidProtocol);
            }
            if v.get("error").is_some() {
                return Err(ToolkitError::Failed);
            }
            return v
                .get("result")
                .cloned()
                .ok_or(ToolkitError::InvalidProtocol);
        }
        if Instant::now() >= deadline {
            return Err(ToolkitError::Timeout);
        }
        std::thread::sleep(Duration::from_millis(2));
    }
}
pub(super) fn run(
    pid: u32,
    sender: EngineEventSink,
    stop: Arc<AtomicBool>,
    router: Arc<super::super::equipment_rpc::Router>,
) {
    let result = (|| {
        sender
            .send(EngineEvent::Status("plugin native push connecting".into()))
            .map_err(|_| ToolkitError::Cancelled)?;
        let host = ToolkitClient::open(pid)?;
        let relay = router.register(host.identity())?;
        host.describe()?;
        let pipe = CapturePipe::open(&host)?;
        let hello = request(&pipe, "hello", "hello", json!({"protocolVersion":1}), &stop)?;
        if hello["protocolVersion"] != 1 || hello["gamePid"] != pid || hello["ready"] != true {
            return Err(ToolkitError::Unavailable);
        }
        let capabilities = hello["capabilities"]
            .as_array()
            .filter(|v| v.len() <= 128)
            .ok_or(ToolkitError::InvalidProtocol)?;
        if ![
            "combat.context.v1",
            "combat.hit_attributes.v1",
            "combat.clock.v1",
            "combat.abyss.v2",
            "combat.abyss.restart_scope.v1",
            "combat.rounds.v1",
        ]
        .iter()
        .all(|needed| capabilities.iter().any(|c| c.as_str() == Some(*needed)))
        {
            return Err(ToolkitError::Unsupported);
        }
        let provider = text(&hello, "providerId")?.to_owned();
        let status = request(&pipe, "status", "status.get", json!({}), &stop)?;
        if text(&status, "providerId")? != provider {
            return Err(ToolkitError::SessionChanged);
        }
        if status["capturing"] != false {
            return Err(ToolkitError::Busy);
        }
        let start = request(
            &pipe,
            "start",
            "combat.start",
            json!({"rawEvidence":false,"abyssEvents":true,"abyssSchema":3,"roundEvents":true}),
            &stop,
        )?;
        if start["abyssEvents"] != true || start["abyssSchema"] != 3 || start["roundEvents"] != true
        {
            return Err(ToolkitError::Unsupported);
        }
        let capture = text(&start, "captureId")?.to_owned();
        let mut cursor = Cursor::new(provider, capture.clone());
        sender
            .send(EngineEvent::Status("plugin native push connected".into()))
            .map_err(|_| ToolkitError::Cancelled)?;
        sender
            .send(EngineEvent::CombatClockHealth(
                CombatClockRuntimeHealth::Unknown,
            ))
            .map_err(|_| ToolkitError::Cancelled)?;
        let clock_start_deadline = Instant::now() + Duration::from_secs(5);
        let never_cancel = AtomicBool::new(false);
        let mut next_clock = Instant::now();
        let mut clock_id = 0;
        let mut pending: Option<(String, Instant)> = None;
        let mut stopping: Option<Instant> = None;
        let mut stop_reply = false;
        let mut final_clock = false;
        let mut control: Option<super::super::equipment_rpc::Pending> = None;
        loop {
            if stop.load(Ordering::Acquire) && stopping.is_none() && !cursor.ended {
                pipe.send("stop", "combat.stop", json!({"captureId":capture}))?;
                stopping = Some(Instant::now() + Duration::from_secs(5));
            }
            if (Instant::now() >= next_clock || cursor.ended || stop_reply)
                && pending.is_none()
                && !final_clock
            {
                clock_id += 1;
                let id = format!("clock-{clock_id}");
                pipe.send(&id, "combat_clock.query_transitions", json!({}))?;
                pending = Some((id, Instant::now() + Duration::from_secs(5)));
            }
            if control.is_none()
                && stopping.is_none()
                && let Ok(request) = relay.receiver.try_recv()
            {
                if request.canceled.load(Ordering::Acquire) || Instant::now() >= request.deadline {
                    let _ = request.reply.try_send(Err(ToolkitError::Cancelled));
                } else {
                    match pipe.send(&request.id, &request.method, request.params.clone()) {
                        Ok(()) => control = Some(request),
                        Err(error) => {
                            let _ = request.reply.try_send(Err(error));
                        }
                    }
                }
            }
            if let Some(bytes) = pipe.read(&never_cancel)? {
                let v = parse(&bytes)?;
                if control.as_ref().is_some_and(|p| v["id"] == p.id) {
                    if let Some(request) = control.take() {
                        let result =
                            super::super::equipment_rpc::response(v, &request.id, &request.method);
                        let _ = request.reply.try_send(result);
                    }
                    continue;
                }
                let events = if let Some(id) = v["id"].as_str() {
                    if v.get("error").is_some() {
                        return Err(ToolkitError::Failed);
                    }
                    if id == "stop" {
                        if v["result"]["captureId"] != capture {
                            return Err(ToolkitError::SessionChanged);
                        }
                        stop_reply = true;
                        vec![]
                    } else if pending.as_ref().is_some_and(|p| p.0 == id) {
                        pending = None;
                        next_clock = Instant::now() + Duration::from_millis(250);
                        if cursor.ended || stop_reply {
                            final_clock = true;
                        }
                        cursor.clock(v["result"].clone())?
                    } else {
                        return Err(ToolkitError::InvalidProtocol);
                    }
                } else {
                    cursor.notification(bytes, v)?
                };
                for e in events {
                    sender.send(e).map_err(|_| ToolkitError::Cancelled)?;
                }
            } else {
                std::thread::sleep(Duration::from_millis(2));
            }
            if cursor.ended && final_clock {
                break;
            }
            if !cursor.clock_ready && Instant::now() >= clock_start_deadline {
                return Err(ToolkitError::Unavailable);
            }
            if pending.as_ref().is_some_and(|p| Instant::now() >= p.1)
                || stopping.is_some_and(|d| Instant::now() >= d)
            {
                return Err(ToolkitError::Timeout);
            }
        }
        Ok(())
    })();
    if let Err(e) = result
        && e != ToolkitError::Cancelled
    {
        let _ = sender.send(EngineEvent::Error(format!("plugin native push: {e}")));
    }
    let _ = sender.send(EngineEvent::CaptureStopped);
}

#[cfg(test)]
mod tests {
    use super::*;
    fn frame(seq: u64, method: &str, more: Value) -> Value {
        let mut p = json!({"providerId":"p","captureId":"c","seq":seq.to_string()});
        p.as_object_mut()
            .unwrap()
            .extend(more.as_object().unwrap().clone());
        json!({"jsonrpc":"2.0","method":method,"params":p})
    }
    fn accept(c: &mut Cursor, v: Value) -> Result<Vec<EngineEvent>, ToolkitError> {
        let b = serde_json::to_vec(&v).unwrap();
        c.notification(b, v)
    }
    fn setup() -> Cursor {
        let mut c = Cursor::new("p".into(), "c".into());
        accept(&mut c,frame(1,"event.combat.context",json!({"contexts":[{"contextId":"1","schemaVersion":1,"team":{"actors":[{"objectKey":"10:7","DefaultCharacterID":"1004"}]}}]}))).unwrap();
        c.clock(json!({"providerId":"p","transitions":[{"sequence":"1","timestamp_100ns":"116444736000000000","pause_type_mask":0,"state_flags":1,"reserved_value":0}]})).unwrap();
        c
    }
    fn hit() -> Value {
        json!({"hitId":"1","unixUs":"1000000","damage":20.0,"attackerObjectIndex":10,"victimObjectIndex":20,"quality":0,"direction":"outgoing","attackerName":"安魂曲","victimName":"target","victimHp":100.0,"victimMaxHp":200.0,"skillName":"紧急唤醒","skillKey":"GA_Lacrimosa_QTE_C","attackDetailKey":"GE_Player_Lacrimosa_QTE1_Damage_C","displayType":0,"association":"native_message_target_ordinal","contextRef":{"contextId":"1"},"attackerAttributes":{"actorIndex":10,"actorSerial":7}})
    }
    fn abyss(kind: &str, us: u64, half: Option<u8>, floor: Option<u32>) -> Value {
        json!({"kind":kind,"unixUs":us.to_string(),"half":half,"floor":floor})
    }
    #[test]
    fn push_abyss_events_drive_the_shared_reducer_for_halves_restarts_and_exit() {
        use crate::{core::reducer::apply_engine_event, engine::model::CombatState};
        let mut cursor = setup();
        let mut state = CombatState::default();
        let frames = [
            frame(
                2,
                "event.combat.abyss",
                json!({"events":[abyss("stage",1_000_000,Some(1),Some(12))]}),
            ),
            frame(3, "event.combat.hits", json!({"hits":[hit()]})),
            frame(
                4,
                "event.combat.abyss",
                json!({"events":[abyss("restart",2_000_000,None,None),abyss("stage",2_000_000,Some(1),Some(12))]}),
            ),
        ];
        for f in frames {
            for event in accept(&mut cursor, f).unwrap() {
                apply_engine_event(&mut state, event);
            }
        }
        assert_eq!(state.abyss.active_half, Some(AbyssHalf::First));
        assert_eq!(state.abyss.first_half.total_damage, 0.0);
        let mut h = hit();
        h["hitId"] = "2".into();
        h["unixUs"] = "3000000".into();
        for event in accept(
            &mut cursor,
            frame(5, "event.combat.hits", json!({"hits":[h]})),
        )
        .unwrap()
        {
            apply_engine_event(&mut state, event);
        }
        for event in accept(
            &mut cursor,
            frame(
                6,
                "event.combat.abyss",
                json!({"events":[abyss("stage",4_000_000,Some(2),Some(12))]}),
            ),
        )
        .unwrap()
        {
            apply_engine_event(&mut state, event);
        }
        let mut h = hit();
        h["hitId"] = "3".into();
        h["unixUs"] = "4000000".into();
        h["attackerObjectIndex"] = 11.into();
        for event in accept(
            &mut cursor,
            frame(7, "event.combat.hits", json!({"hits":[h]})),
        )
        .unwrap()
        {
            apply_engine_event(&mut state, event);
        }
        assert_eq!(state.abyss.first_half.total_damage, 20.0);
        assert_eq!(state.abyss.second_half.total_damage, 20.0);
        let restart = frame(
            8,
            "event.combat.abyss",
            json!({"events":[abyss("restart",5_000_000,None,None),abyss("stage",5_000_000,Some(1),Some(12))]}),
        );
        for event in accept(&mut cursor, restart.clone()).unwrap() {
            apply_engine_event(&mut state, event);
        }
        assert!(accept(&mut cursor, restart).unwrap().is_empty());
        assert_eq!(state.abyss.first_half.total_damage, 0.0);
        assert_eq!(state.abyss.second_half.total_damage, 0.0);
        for event in accept(&mut cursor,frame(9,"event.combat.abyss",json!({"events":[abyss("success",6_000_000,None,None),abyss("exit",7_000_000,None,None)]}))).unwrap(){apply_engine_event(&mut state,event);}
        assert_eq!(state.abyss.active_half, None);
        assert_eq!(state.abyss.success_at, Some(6.0));
        assert_eq!(state.abyss.exited_at, Some(7.0));
    }
    #[test]
    fn scoped_abyss_restart_preserves_its_explicit_half_and_rejects_missing_scope() {
        let mut c = setup();
        let event = frame(
            2,
            "event.combat.abyss",
            json!({"events":[abyss("restart_half",1_000_000,Some(2),None)]}),
        );
        assert!(matches!(
            accept(&mut c, event.clone()).unwrap().as_slice(),
            [EngineEvent::Abyss(AbyssEvent::RestartHalf {
                half: AbyssHalf::Second,
                ..
            })]
        ));
        assert!(accept(&mut c, event).unwrap().is_empty());
        for half in [None, Some(0), Some(3)] {
            assert!(
                accept(
                    &mut c,
                    frame(
                        3,
                        "event.combat.abyss",
                        json!({"events":[abyss("restart_half",2_000_000,half,None)]})
                    )
                )
                .is_err()
            );
        }
        assert_eq!(c.seq, 2);
    }
    #[test]
    fn challenge_restart_stream_is_ordered_deduplicated_and_transactional() {
        let mut cursor = setup();
        let row = |id: u64, kind: u8| json!({"kind":"restart","restartId":id.to_string(),"unixUs":"2000000","cloneType":kind});
        let valid = frame(2, "event.combat.rounds", json!({"events":[row(1,9)]}));
        assert!(matches!(
            accept(&mut cursor, valid.clone()).unwrap().as_slice(),
            [EngineEvent::ChallengeRestart { clone_type: 9, .. }]
        ));
        assert!(accept(&mut cursor, valid).unwrap().is_empty());
        let bad = frame(
            3,
            "event.combat.rounds",
            json!({"events":[row(2,16),row(3,8)]}),
        );
        assert!(accept(&mut cursor, bad).is_err());
        assert_eq!(cursor.round_id, 1);
        assert_eq!(cursor.seq, 2);
        assert!(matches!(
            accept(
                &mut cursor,
                frame(3, "event.combat.rounds", json!({"events":[row(2,16)]}))
            )
            .unwrap()
            .as_slice(),
            [EngineEvent::ChallengeRestart { clone_type: 16, .. }]
        ));
        let mut waiting = Cursor::new("p".into(), "c".into());
        assert!(
            accept(
                &mut waiting,
                frame(1, "event.combat.rounds", json!({"events":[row(1,9)]}))
            )
            .unwrap()
            .is_empty()
        );
        let events=waiting.clock(json!({"providerId":"p","transitions":[{"sequence":"1","timestamp_100ns":"116444736000000000","pause_type_mask":0,"state_flags":1,"reserved_value":0}]})).unwrap();
        assert!(matches!(
            events.last(),
            Some(EngineEvent::ChallengeRestart { clone_type: 9, .. })
        ));
    }
    #[test]
    fn current_abyss_location_detects_floor_without_inventing_half_or_backfilling_hits() {
        use crate::{
            core::reducer::{CoreSignal, apply_engine_event},
            engine::model::CombatState,
        };
        let mut cursor = setup();
        let mut state = CombatState::default();
        for event in accept(
            &mut cursor,
            frame(
                2,
                "event.combat.abyss",
                json!({"events":[abyss("location",1_000_000,None,Some(11))]}),
            ),
        )
        .unwrap()
        {
            assert_eq!(
                apply_engine_event(&mut state, event),
                CoreSignal::StateChanged
            );
        }
        assert!(state.abyss.is_active());
        assert_eq!(state.abyss.floor, Some(11));
        assert_eq!(state.abyss.active_half, None);
        for event in accept(
            &mut cursor,
            frame(
                3,
                "event.combat.abyss",
                json!({"events":[abyss("location",2_000_000,None,Some(11))]}),
            ),
        )
        .unwrap()
        {
            assert_eq!(apply_engine_event(&mut state, event), CoreSignal::Unchanged);
        }
        for event in accept(
            &mut cursor,
            frame(4, "event.combat.hits", json!({"hits":[hit()]})),
        )
        .unwrap()
        {
            apply_engine_event(&mut state, event);
        }
        for event in accept(
            &mut cursor,
            frame(
                5,
                "event.combat.abyss",
                json!({"events":[abyss("stage",3_000_000,Some(1),Some(11))]}),
            ),
        )
        .unwrap()
        {
            apply_engine_event(&mut state, event);
        }
        assert_eq!(state.abyss.active_half, Some(AbyssHalf::First));
        assert!(state.abyss.first_half.hits.is_empty());
        assert_eq!(state.total_damage, 20.0);
        for event in accept(
            &mut cursor,
            frame(
                6,
                "event.combat.abyss",
                json!({"events":[abyss("location",4_000_000,None,Some(12))]}),
            ),
        )
        .unwrap()
        {
            apply_engine_event(&mut state, event);
        }
        assert_eq!(state.abyss.floor, Some(12));
        assert_eq!(state.abyss.active_half, None);
    }
    #[test]
    fn abyss_batch_rejection_does_not_advance_the_delivery_cursor() {
        let mut c = setup();
        for bad in [
            abyss("stage", 0, Some(1), None),
            abyss("stage", 1, Some(3), None),
            abyss("restart", 1, Some(1), None),
            abyss("request_restart", 1, None, None),
            abyss("location", 1, Some(1), Some(11)),
            abyss("location", 1, None, None),
            abyss("location", 1, None, Some(0)),
        ] {
            let f = frame(
                2,
                "event.combat.abyss",
                json!({"events":[abyss("stage",1,Some(1),None),bad]}),
            );
            assert!(accept(&mut c, f).is_err());
            assert_eq!(c.seq, 1);
            assert_eq!(c.abyss_timestamp, 0);
        }
        assert!(
            accept(
                &mut c,
                frame(
                    2,
                    "event.combat.abyss",
                    json!({"events":vec![abyss("exit",1,None,None);65]})
                )
            )
            .is_err()
        );
        assert_eq!(
            accept(
                &mut c,
                frame(
                    2,
                    "event.combat.abyss",
                    json!({"events":[abyss("stage",1,Some(1),None)]})
                )
            )
            .unwrap()
            .len(),
            1
        );
    }
    #[test]
    fn clock_startup_preserves_abyss_and_hit_order_without_backfill() {
        for stage_first in [true, false] {
            let mut c = Cursor::new("p".into(), "c".into());
            let stage = |seq| {
                frame(
                    seq,
                    "event.combat.abyss",
                    json!({"events":[abyss("stage",1_000_000,Some(1),None)]}),
                )
            };
            let damage = |seq| frame(seq, "event.combat.hits", json!({"hits":[hit()]}));
            assert!(
                accept(&mut c, if stage_first { stage(1) } else { damage(1) })
                    .unwrap()
                    .is_empty()
            );
            assert!(
                accept(&mut c, if stage_first { damage(2) } else { stage(2) })
                    .unwrap()
                    .is_empty()
            );
            let events=c.clock(json!({"providerId":"p","transitions":[{"sequence":"1","timestamp_100ns":"116444736000000000","pause_type_mask":0,"state_flags":1,"reserved_value":0}]})).unwrap();
            assert_eq!(events.len(), 3);
            assert_eq!(matches!(events[1], EngineEvent::Abyss(_)), stage_first);
            assert_eq!(matches!(events[2], EngineEvent::Abyss(_)), !stage_first);
        }
    }
    fn detailed_hit(critical: Option<bool>) -> Value {
        let mut value = hit();
        value["criticalKnown"] = critical.is_some().into();
        value["critical"] = critical.into();
        value["criticalSource"] = json!("native_prediction");
        value["attackerAttributes"] = json!({"actorIndex":10,"actorSerial":7,"id":9007199254740993_u64,"sampledUnixUs":1000000,
            "actorName":"role","status":"partial","values":{
                "attack":{"value":123.5,"status":"ok","source":"HTAttributeComponent.GetAtk"},
                "hp":{"value":0.0,"status":"ok","source":"HTAttributeComponent.GetHPCurrent"},
                "crit":{"value":null,"status":"call_failed","source":"HTAttributeComponent.GetCrit"}}});
        let effect = |kind| json!({"instanceKey":"10:7:11","key":"GE_Test","name":"fixture effect","description":"fixture only","source":"fixture","kind":kind,"stacks":2,"duration":-1.0,"startWorldTime":1.0,"level":1.0,"inhibited":false});
        value["attackerEffects"] = json!({"actorIndex":10,"actorSerial":7,"id":1,"observedUs":"1000000","complete":true,"status":"ok","effects":[effect(2),effect(0)]});
        value["victimEffects"] = json!({"actorIndex":20,"actorSerial":8,"id":2,"observedUs":"1000000","complete":false,"status":"partial","effects":[effect(3)]});
        value
    }
    #[test]
    fn native_hit_snapshots_keep_tristate_critical_raw_values_and_frozen_history() {
        use crate::engine::model::{CombatState, Hit, plugin_snapshot::AttributeValue};
        let mut cursor = setup();
        let first = accept(
            &mut cursor,
            frame(
                2,
                "event.combat.hits",
                json!({"hits":[detailed_hit(Some(true))]}),
            ),
        )
        .unwrap();
        let EngineEvent::Hit(hit) = &first[0] else {
            panic!()
        };
        let snapshot = hit.plugin_snapshot.as_ref().unwrap();
        assert_eq!(snapshot.critical, Some(true));
        assert_eq!(snapshot.role_effects.unwrap().positive, 1);
        assert_eq!(snapshot.role_effects.unwrap().other, 1);
        assert_eq!(snapshot.enemy_effects.unwrap().negative, 1);
        let attributes = snapshot
            .data
            .as_ref()
            .unwrap()
            .attacker_attributes
            .as_ref()
            .unwrap();
        assert_eq!(attributes.id.as_deref(), Some("9007199254740993"));
        assert!(matches!(
            attributes.values.as_ref().unwrap()["hp"].value,
            Some(AttributeValue::Number(0.0))
        ));
        assert!(attributes.values.as_ref().unwrap()["crit"].value.is_none());
        let cloned = hit.clone();
        assert!(Arc::ptr_eq(
            cloned.plugin_snapshot.as_ref().unwrap(),
            snapshot
        ));
        let mut second = detailed_hit(Some(false));
        second["hitId"] = "2".into();
        second["attackerAttributes"]["values"]["attack"]["value"] = 999.into();
        let later = accept(
            &mut cursor,
            frame(3, "event.combat.hits", json!({"hits":[second]})),
        )
        .unwrap();
        let EngineEvent::Hit(later) = &later[0] else {
            panic!()
        };
        assert_eq!(
            later.plugin_snapshot.as_ref().unwrap().critical,
            Some(false)
        );
        assert!(matches!(
            attributes.values.as_ref().unwrap()["attack"].value,
            Some(AttributeValue::Number(123.5))
        ));
        let restored: Hit =
            serde_json::from_slice(&serde_json::to_vec(hit.as_ref()).unwrap()).unwrap();
        assert_eq!(
            restored.plugin_snapshot.as_ref().unwrap().critical,
            Some(true)
        );
        assert_ne!(
            restored.plugin_snapshot.as_ref().unwrap().reference(),
            snapshot.reference()
        );
        let mut combat = CombatState::default();
        combat.push_hit(*hit.clone());
        let history = crate::storage::history::HistoryCombatDetails::from_state(&combat).unwrap();
        let history: crate::storage::history::HistoryCombatDetails =
            serde_json::from_slice(&serde_json::to_vec(&history).unwrap()).unwrap();
        assert_eq!(
            history.to_combat_state().hits[0]
                .plugin_snapshot
                .as_ref()
                .unwrap()
                .critical,
            Some(true)
        );
    }
    #[test]
    fn snapshot_budget_keeps_damage_and_metadata_and_incoming_swaps_role_summaries() {
        let mut cursor = setup();
        cursor.snapshot_count = MAX_SESSION_SNAPSHOTS;
        let mut value = detailed_hit(None);
        value["direction"] = "incoming".into();
        let events = accept(
            &mut cursor,
            frame(2, "event.combat.hits", json!({"hits":[value]})),
        )
        .unwrap();
        let EngineEvent::Hit(hit) = &events[0] else {
            panic!()
        };
        assert_eq!(hit.damage, 20.0);
        let snapshot = hit.plugin_snapshot.as_ref().unwrap();
        assert_eq!(snapshot.critical, None);
        assert_eq!(snapshot.retention, "budget_exceeded");
        assert!(snapshot.data.is_none());
        assert_eq!(snapshot.role_effects.unwrap().negative, 1);
        assert_eq!(snapshot.enemy_effects.unwrap().positive, 1);
    }
    #[test]
    fn oversized_snapshot_is_rejected_without_advancing_hit_or_budget() {
        let mut cursor = setup();
        let mut value = detailed_hit(Some(true));
        value["attackerEffects"]["effects"][0]["description"] = "x".repeat(4097).into();
        assert_eq!(
            accept(
                &mut cursor,
                frame(2, "event.combat.hits", json!({"hits":[value]}))
            )
            .unwrap_err(),
            ToolkitError::TooLarge
        );
        assert_eq!(cursor.hit, 0);
        assert_eq!(cursor.snapshot_count, 0);
        assert!(
            accept(
                &mut cursor,
                frame(
                    2,
                    "event.combat.hits",
                    json!({"hits":[detailed_hit(Some(true))]})
                )
            )
            .is_ok()
        );
    }

    #[test]
    fn pushes_once_and_binds_exact_actor_serial_instead_of_current_character() {
        let mut c = setup();
        let f = frame(2, "event.combat.hits", json!({"hits":[hit()]}));
        let events = accept(&mut c, f.clone()).unwrap();
        let EngineEvent::Hit(h) = &events[0] else {
            panic!()
        };
        assert_eq!(h.char_id, 1004);
        assert_eq!(h.damage, 20.0);
        assert_ne!(h.damage_component.as_deref(), Some("character"));
        assert_eq!(h.ability_name.as_deref(), Some("GA_Lacrimosa_QTE"));
        assert!(accept(&mut c, f).unwrap().is_empty());
        let mut bad = hit();
        bad["attackerAttributes"]["actorSerial"] = 8.into();
        bad["hitId"] = "2".into();
        let events = accept(&mut c, frame(3, "event.combat.hits", json!({"hits":[bad]}))).unwrap();
        let EngineEvent::Hit(h) = &events[0] else {
            panic!()
        };
        assert!(!h.char_known);
    }
    #[test]
    fn server_reaction_with_metadata_unknown_keeps_proven_actor_and_category() {
        let mut c = setup();
        let mut h = hit();
        h["quality"] = 3.into();
        h["skillName"] = "".into();
        h["skillKey"] = "".into();
        h["attackDetailKey"] = "".into();
        h["association"] = "server_reaction_category".into();
        h["displayType"] = 25.into();
        let events = accept(&mut c, frame(2, "event.combat.hits", json!({"hits":[h]}))).unwrap();
        let EngineEvent::Hit(h) = &events[0] else {
            panic!()
        };
        assert_eq!(h.char_id, 1004);
        assert_eq!(h.damage_name.as_deref(), Some("浊燃"));
        assert!(
            h.target_context
                .contains(&"plugin_quality:unknown".to_string())
        );
    }
    #[test]
    fn witnesses_require_matching_serial_and_use_victim_for_incoming() {
        for (attributes, effects, known) in [
            (
                json!({"actorIndex":0,"actorSerial":0}),
                json!({"actorIndex":10,"actorSerial":7}),
                true,
            ),
            (
                json!({"actorIndex":10,"actorSerial":8}),
                json!({"actorIndex":10,"actorSerial":7}),
                false,
            ),
            (Value::Null, Value::Null, false),
        ] {
            let mut c = setup();
            let mut h = hit();
            h["direction"] = "incoming".into();
            h["attackerObjectIndex"] = 20.into();
            h["victimObjectIndex"] = 10.into();
            h["victimAttributes"] = attributes;
            h["victimEffects"] = effects;
            let events =
                accept(&mut c, frame(2, "event.combat.hits", json!({"hits":[h]}))).unwrap();
            let EngineEvent::Hit(h) = &events[0] else {
                panic!()
            };
            assert_eq!(h.char_known, known);
            assert_eq!(h.direction, crate::engine::model::HitDirection::Incoming);
        }
    }
    #[test]
    fn malformed_batch_is_atomic_and_end_detects_dropped_contexts() {
        let mut c = setup();
        let mut invalid = hit();
        invalid["hitId"] = "2".into();
        invalid["damage"] = (-1).into();
        assert!(
            accept(
                &mut c,
                frame(2, "event.combat.hits", json!({"hits":[hit(),invalid]}))
            )
            .is_err()
        );
        assert_eq!((c.seq, c.hit, c.hits), (1, 0, 0));
        assert_eq!(
            accept(
                &mut c,
                frame(2, "event.combat.hits", json!({"hits":[hit()]}))
            )
            .unwrap()
            .len(),
            1
        );
        let end = |dropped: &str| {
            frame(
                3,
                "event.combat.ended",
                json!({"hitCount":"1","droppedRecords":"0","sourceDroppedRecords":"0","rejectedTargets":"0","droppedContexts":dropped}),
            )
        };
        assert!(matches!(
            accept(&mut c, end("1")),
            Err(ToolkitError::DataGap)
        ));
        let end = end("0");
        assert!(accept(&mut c, end.clone()).unwrap().is_empty());
        assert!(c.ended);
        assert!(accept(&mut c, end).unwrap().is_empty());
        assert!(
            accept(
                &mut c,
                frame(4, "event.combat.hits", json!({"hits":[hit()]}))
            )
            .is_err()
        );
    }
    #[test]
    fn gaps_over_budget_rows_foreign_sessions_and_lost_hits_fail_closed() {
        let mut c = setup();
        assert!(matches!(
            accept(
                &mut c,
                frame(3, "event.combat.hits", json!({"hits":[hit()]}))
            ),
            Err(ToolkitError::DataGap)
        ));
        assert_eq!(c.hits, 0);
        let mut f = frame(2, "event.combat.hits", json!({"hits":[hit()]}));
        f["params"]["captureId"] = "old".into();
        assert!(matches!(
            accept(&mut c, f),
            Err(ToolkitError::SessionChanged)
        ));
        assert!(matches!(
            accept(
                &mut c,
                frame(2, "event.combat.hits", json!({"hits":vec![hit();65]}))
            ),
            Err(ToolkitError::TooLarge)
        ));
        assert!(matches!(
            accept(
                &mut c,
                frame(
                    2,
                    "event.combat.ended",
                    json!({"hitCount":"1","droppedRecords":"0","sourceDroppedRecords":"0","rejectedTargets":"0","droppedContexts":"0"})
                )
            ),
            Err(ToolkitError::DataGap)
        ));
    }
    fn timing_rows(first: u64, rows: &[(u64, u32, u32)]) -> Value {
        json!({"providerId":"p","transitions":rows.iter().enumerate().map(|(i,(second,mask,flags))|
            json!({"sequence":(first+i as u64).to_string(),"timestamp_100ns":(116444736000000000_u64+second*10_000_000).to_string(),
                "pause_type_mask":mask,"state_flags":flags,"reserved_value":0})).collect::<Vec<_>>()})
    }
    #[test]
    fn start_stop_restart_keeps_plugin_six_seconds_distinct_from_wall_ten_seconds() {
        use crate::{core::reducer::apply_engine_event, engine::model::CombatState};
        for baseline in [1, 41, 901] {
            let mut cursor = Cursor::new("p".into(), "c".into());
            let mut state = CombatState::default();
            let first = hit();
            let mut second = hit();
            second["hitId"] = "2".into();
            second["unixUs"] = "11000000".into();
            assert!(
                accept(
                    &mut cursor,
                    frame(1, "event.combat.hits", json!({"hits":[first,second]}))
                )
                .unwrap()
                .is_empty()
            );
            assert_eq!(
                cursor.pending_events.len(),
                2,
                "no wall-time DPS is published before clock readiness"
            );
            let timing = timing_rows(baseline, &[(0, 0, 1), (3, 4, 1), (7, 0, 1)]);
            for event in cursor.clock(timing.clone()).unwrap() {
                apply_engine_event(&mut state, event);
            }
            assert_eq!(state.duration_with_time_stop(false), 10.0);
            assert_eq!(state.duration_with_time_stop(true), 6.0);
            assert_eq!(
                state.combat_clock_health,
                CombatClockRuntimeHealth::Available
            );
            assert!(cursor.clock(timing).unwrap().is_empty());
            accept(&mut cursor,frame(2,"event.combat.ended",json!({"hitCount":"2","droppedRecords":"0","sourceDroppedRecords":"0","rejectedTargets":"0","droppedContexts":"0"}))).unwrap();
            assert!(cursor.ended);
        }
    }
    #[test]
    fn invalid_clock_does_not_release_hits_or_advance_any_partial_edges() {
        let mut cursor = Cursor::new("p".into(), "c".into());
        accept(
            &mut cursor,
            frame(1, "event.combat.hits", json!({"hits":[hit()]})),
        )
        .unwrap();
        assert_eq!(
            cursor
                .clock(timing_rows(42, &[(0, 0, 1), (1, 4, 0)]))
                .unwrap_err(),
            ToolkitError::Unavailable
        );
        assert_eq!(cursor.clock_seq, None);
        assert_eq!(cursor.pending_events.len(), 1);
        assert!(!cursor.clock_ready);
        let events = cursor.clock(timing_rows(42, &[(0, 0, 1)])).unwrap();
        assert_eq!(
            events
                .iter()
                .filter(|e| matches!(e, EngineEvent::Hit(_)))
                .count(),
            1
        );
        assert!(cursor.pending_events.is_empty());
        assert_eq!(
            cursor.clock(timing_rows(44, &[(2, 0, 1)])).unwrap_err(),
            ToolkitError::DataGap
        );
        assert_eq!(cursor.clock_seq, Some(42));
    }
    #[test]
    fn startup_hit_staging_is_bounded_and_clock_order_is_validated() {
        let mut cursor = Cursor::new("p".into(), "c".into());
        for batch in 0..64_u64 {
            let rows = (0..64_u64)
                .map(|i| {
                    let mut h = hit();
                    h["hitId"] = (batch * 64 + i + 1).to_string().into();
                    h
                })
                .collect::<Vec<_>>();
            assert!(
                accept(
                    &mut cursor,
                    frame(batch + 1, "event.combat.hits", json!({"hits":rows}))
                )
                .unwrap()
                .is_empty()
            );
        }
        assert_eq!(cursor.pending_events.len(), 4096);
        assert_eq!(
            accept(
                &mut cursor,
                frame(65, "event.combat.hits", json!({"hits":[hit()]}))
            )
            .unwrap_err(),
            ToolkitError::TooLarge
        );
        let mut unordered = timing_rows(9, &[(0, 0, 1), (1, 4, 1)]);
        unordered["transitions"][1]["sequence"] = "8".into();
        assert_eq!(
            cursor.clock(unordered).unwrap_err(),
            ToolkitError::InvalidProtocol
        );
        assert_eq!(cursor.clock_seq, None);
    }

    #[test]
    fn restarted_capture_accepts_provider_clock_sequence_not_starting_at_one() {
        let mut cursor = Cursor::new("p".into(), "c".into());
        let events = cursor
            .clock(json!({"providerId":"p","transitions":[{
                "sequence":"41","timestamp_100ns":"116444736010000000",
                "pause_type_mask":0,"state_flags":1,"reserved_value":0
            }]}))
            .unwrap();
        assert!(events.iter().any(|event| matches!(
            event,
            EngineEvent::CombatClockHealth(CombatClockRuntimeHealth::Available)
        )));
    }

    #[test]
    fn clock_reuses_observed_pause_transitions_without_claiming_missing_boundaries() {
        let mut c = Cursor::new("p".into(), "c".into());
        let clock = |seq: u64, mask: u32, flags: u32| json!({"providerId":"p","transitions":[{"sequence":seq.to_string(),"timestamp_100ns":"116444736010000000","pause_type_mask":mask,"state_flags":flags,"reserved_value":0}]});
        assert!(c.clock(clock(1, 4, 1)).unwrap().iter().any(|e| matches!(
            e,
            EngineEvent::TimeStop(TimeStopEvent::GamePauseStarted { .. })
        )));
        assert!(c.clock(clock(1, 4, 1)).unwrap().is_empty());
        assert_eq!(c.clock(clock(3, 0, 1)).unwrap_err(), ToolkitError::DataGap);
        assert_eq!(c.clock_seq, Some(1));
        assert!(c.clock(clock(2, 0, 1)).is_ok());
    }
}
