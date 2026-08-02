//! Frontend-neutral ownership for one live capture and its authoritative
//! [`CombatState`].
//!
//! The service keeps Npcap setup and parser work off UI threads, routes every
//! [`EngineEvent`] through the shared reducer, and exposes read-only state
//! access plus stable lifecycle categories. Frontends remain responsible for
//! translating those categories at their display boundary.

use std::{
    collections::{HashMap, VecDeque},
    net::Ipv4Addr,
    path::Path,
    path::PathBuf,
    sync::{
        Arc, Mutex, Weak,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
    thread,
    time::{Duration, Instant},
};

use crossbeam_channel::{Receiver, bounded, select};

use super::{
    CoreError, CoreErrorCode,
    capture::{CaptureController, CaptureControllerOptions},
    history::abyss_event_starts_new_round,
    reducer::{CoreSignal, apply_engine_event},
};
use crate::{
    engine::{
        capture::{
            CaptureResources, EngineEventSink, RawCaptureSnapshot, import_capture_json,
            import_pcapng,
        },
        model::{
            CaptureQualitySource, CaptureQualitySummary, CharacterInfo, CombatState, EngineEvent,
        },
        parser::{AbilityCatalog, CHARACTER_DATA_PATH, load_characters},
    },
    storage::{ability_names, history::HistoryCombatDetails, i18n::Language},
};

const RELIABLE_ENGINE_EVENT_CAPACITY: usize = 16_384;
const DEBUG_ENGINE_EVENT_CAPACITY: usize = 2_048;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LiveCapturePhase {
    Idle,
    Starting,
    Running,
    Stopping,
    Stopped,
    Failed,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LiveCaptureIssue {
    Start(CoreErrorCode),
    RuntimeWarning,
    RuntimeError,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LiveCaptureStatus {
    pub phase: LiveCapturePhase,
    pub issue: Option<LiveCaptureIssue>,
}

impl Default for LiveCaptureStatus {
    fn default() -> Self {
        Self {
            phase: LiveCapturePhase::Idle,
            issue: None,
        }
    }
}

#[derive(Clone, Default)]
pub struct LiveCaptureResources {
    pub characters: Arc<HashMap<u32, CharacterInfo>>,
    pub ability_catalog: Arc<AbilityCatalog>,
}

impl LiveCaptureResources {
    pub fn load(language: Language) -> (Self, Vec<String>) {
        let mut warnings = Vec::new();
        let characters = match load_characters(Path::new(CHARACTER_DATA_PATH)) {
            Ok(characters) => Arc::new(characters),
            Err(error) => {
                warnings.push(error.to_string());
                Arc::new(HashMap::new())
            }
        };
        let (ability_catalog, ability_warning) = ability_names::init(language);
        warnings.extend(ability_warning);
        (
            Self {
                characters,
                ability_catalog,
            },
            warnings,
        )
    }
}

#[derive(Clone)]
pub struct LiveCaptureService(Arc<LiveCaptureInner>);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CaptureReplayKind {
    Pcapng,
    Json,
}

struct ReplayTask {
    stop: Arc<AtomicBool>,
    thread: thread::JoinHandle<()>,
}

struct LiveCaptureInner {
    state: Mutex<CombatState>,
    event_gate: Mutex<()>,
    controller: Mutex<CaptureController>,
    replay: Mutex<Option<ReplayTask>>,
    status: Mutex<LiveCaptureStatus>,
    quality_source: Mutex<CaptureQualitySource>,
    revision: AtomicU64,
    packet_revision: AtomicU64,
    packet_session_generation: AtomicU64,
    outgoing_hit_revision: AtomicU64,
    sender: EngineEventSink,
    receiver: Mutex<Option<(Receiver<EngineEvent>, Receiver<EngineEvent>)>>,
    resources: LiveCaptureResources,
    last_outgoing_hit_at: Mutex<Option<Instant>>,
    pending_abyss_archives: Mutex<VecDeque<HistoryCombatDetails>>,
    last_abyss_archive_hits_generation: AtomicU64,
}

impl LiveCaptureService {
    pub fn new(resources: LiveCaptureResources) -> Self {
        let (reliable_sender, reliable_receiver) = bounded(RELIABLE_ENGINE_EVENT_CAPACITY);
        let (debug_sender, debug_receiver) = bounded(DEBUG_ENGINE_EVENT_CAPACITY);
        Self(Arc::new(LiveCaptureInner {
            state: Mutex::new(CombatState::default()),
            event_gate: Mutex::new(()),
            controller: Mutex::new(CaptureController::default()),
            replay: Mutex::new(None),
            status: Mutex::new(LiveCaptureStatus::default()),
            quality_source: Mutex::new(CaptureQualitySource::Unknown),
            revision: AtomicU64::new(0),
            packet_revision: AtomicU64::new(0),
            packet_session_generation: AtomicU64::new(0),
            outgoing_hit_revision: AtomicU64::new(0),
            sender: EngineEventSink::split(reliable_sender, debug_sender),
            receiver: Mutex::new(Some((reliable_receiver, debug_receiver))),
            resources,
            last_outgoing_hit_at: Mutex::new(None),
            pending_abyss_archives: Mutex::new(VecDeque::new()),
            last_abyss_archive_hits_generation: AtomicU64::new(u64::MAX),
        }))
    }

    pub fn status(&self) -> LiveCaptureStatus {
        *self
            .0
            .status
            .lock()
            .expect("live capture status lock poisoned")
    }

    pub fn replay_running(&self) -> bool {
        self.0
            .replay
            .lock()
            .expect("capture replay lock poisoned")
            .is_some()
    }

    pub fn active_capture_filter(&self) -> Option<String> {
        self.0
            .controller
            .lock()
            .expect("live capture controller lock poisoned")
            .active_filter()
    }

    pub fn raw_capture_snapshot(&self) -> Option<RawCaptureSnapshot> {
        self.0
            .controller
            .lock()
            .expect("live capture controller lock poisoned")
            .raw_capture_snapshot()
    }

    pub fn save_last_raw_capture(&self, path: &Path) -> Result<(u64, u64), String> {
        self.0
            .controller
            .lock()
            .expect("live capture controller lock poisoned")
            .save_last_raw_capture(path)
    }

    pub fn quality_source(&self) -> CaptureQualitySource {
        *self
            .0
            .quality_source
            .lock()
            .expect("capture quality source lock poisoned")
    }

    pub fn quality_summary(&self) -> CaptureQualitySummary {
        let source = self.quality_source();
        self.with_state(|state| state.capture_quality_summary(source))
    }

    /// Returns a cheap monotonic marker for capture state that can affect
    /// frontend projections.
    pub fn revision(&self) -> u64 {
        self.0.revision.load(Ordering::Acquire)
    }

    /// Monotonic marker advanced only by confirmed outgoing hits. UI adapters
    /// use it to leave a historical projection when genuinely new live output
    /// arrives without polling or copying the bounded hit ring.
    pub fn outgoing_hit_revision(&self) -> u64 {
        self.0.outgoing_hit_revision.load(Ordering::Acquire)
    }

    /// Returns the two domain generations that can change the inventory page.
    /// This intentionally excludes combat hits so the inventory Channel does
    /// not serialize thousands of items for unrelated high-frequency events.
    pub fn inventory_revision(&self) -> (u64, u64) {
        self.with_state(|state| {
            (
                state.empty_curtain_generation,
                state.empty_curtain_characters_generation,
            )
        })
    }

    pub fn with_state<T>(&self, read: impl FnOnce(&CombatState) -> T) -> T {
        let state = self
            .0
            .state
            .lock()
            .expect("live capture state lock poisoned");
        read(&state)
    }

    pub fn with_packet_state<T>(
        &self,
        read: impl FnOnce(super::packets::PacketStreamRevision, usize, &CombatState) -> T,
    ) -> T {
        let _gate = self
            .0
            .event_gate
            .lock()
            .expect("live capture event gate poisoned");
        let state = self
            .0
            .state
            .lock()
            .expect("live capture state lock poisoned");
        let revision = super::packets::PacketStreamRevision {
            generation: self.0.packet_revision.load(Ordering::Acquire),
            session_generation: self.0.packet_session_generation.load(Ordering::Acquire),
            packet_generation: state.packets_generation,
            observed_packet_count: state.packet_count,
        };
        read(revision, self.0.sender.pending_len(), &state)
    }

    pub fn resources(&self) -> LiveCaptureResources {
        self.0.resources.clone()
    }

    pub fn idle_elapsed(&self) -> Option<Duration> {
        self.0
            .last_outgoing_hit_at
            .lock()
            .expect("live capture activity lock poisoned")
            .map(|last| last.elapsed())
    }

    pub fn take_pending_abyss_archives(&self) -> Vec<HistoryCombatDetails> {
        self.0
            .pending_abyss_archives
            .lock()
            .expect("live capture Abyss archive lock poisoned")
            .drain(..)
            .collect()
    }

    pub fn restore_pending_abyss_archives(&self, archives: Vec<HistoryCombatDetails>) {
        let mut pending = self
            .0
            .pending_abyss_archives
            .lock()
            .expect("live capture Abyss archive lock poisoned");
        for archive in archives.into_iter().rev() {
            pending.push_front(archive);
        }
    }

    pub fn archive_and_reset<P, T, E>(
        &self,
        prepare: impl FnOnce(&CombatState) -> Option<P>,
        persist: impl FnOnce(P) -> Result<T, E>,
    ) -> Result<Option<T>, E> {
        let _gate = self
            .0
            .event_gate
            .lock()
            .expect("live capture event gate poisoned");
        let prepared = {
            let state = self
                .0
                .state
                .lock()
                .expect("live capture state lock poisoned");
            prepare(&state)
        };
        let Some(prepared) = prepared else {
            return Ok(None);
        };
        let result = persist(prepared)?;
        *self
            .0
            .state
            .lock()
            .expect("live capture state lock poisoned") = CombatState::default();
        *self
            .0
            .last_outgoing_hit_at
            .lock()
            .expect("live capture activity lock poisoned") = None;
        self.0
            .last_abyss_archive_hits_generation
            .store(u64::MAX, Ordering::Release);
        self.0.bump_packet_session();
        self.0.bump_revision();
        Ok(Some(result))
    }

    /// Clears the current combat projection while keeping capture resources and
    /// the active capture controller intact.
    pub fn reset_session(&self) {
        let _gate = self
            .0
            .event_gate
            .lock()
            .expect("live capture event gate poisoned");
        *self
            .0
            .state
            .lock()
            .expect("live capture state lock poisoned") = CombatState::default();
        *self
            .0
            .last_outgoing_hit_at
            .lock()
            .expect("live capture activity lock poisoned") = None;
        self.0
            .last_abyss_archive_hits_generation
            .store(u64::MAX, Ordering::Release);
        self.0.bump_packet_session();
        self.0.bump_revision();
    }

    /// Restores a previously reset session while keeping the capture service
    /// and its frontend-neutral resources intact.
    pub fn restore_session(&self, state: CombatState, quality_source: CaptureQualitySource) {
        let _gate = self
            .0
            .event_gate
            .lock()
            .expect("live capture event gate poisoned");
        let has_outgoing = state.hits.iter().any(|hit| hit.direction.is_outgoing());
        *self
            .0
            .state
            .lock()
            .expect("live capture state lock poisoned") = state;
        *self
            .0
            .quality_source
            .lock()
            .expect("capture quality source lock poisoned") = quality_source;
        *self
            .0
            .last_outgoing_hit_at
            .lock()
            .expect("live capture activity lock poisoned") = has_outgoing.then_some(Instant::now());
        self.0
            .last_abyss_archive_hits_generation
            .store(u64::MAX, Ordering::Release);
        self.0.bump_packet_session();
        self.0.bump_revision();
    }

    pub fn request_start(&self, options: CaptureControllerOptions) -> Result<(), CoreError> {
        self.ensure_event_worker()?;
        {
            let mut status = self
                .0
                .status
                .lock()
                .expect("live capture status lock poisoned");
            match status.phase {
                LiveCapturePhase::Idle | LiveCapturePhase::Stopped | LiveCapturePhase::Failed => {
                    *status = LiveCaptureStatus {
                        phase: LiveCapturePhase::Starting,
                        issue: None,
                    };
                }
                LiveCapturePhase::Starting
                | LiveCapturePhase::Running
                | LiveCapturePhase::Stopping => {
                    return Err(CoreError::new(
                        CoreErrorCode::CaptureAlreadyRunning,
                        "capture start is already active",
                    ));
                }
            }
        }
        self.0.bump_capture_status_revision();

        let service = self.clone();
        thread::Builder::new()
            .name("nte-live-capture-start".to_owned())
            .spawn(move || service.finish_start(options))
            .map(|_| ())
            .map_err(|error| {
                self.record_start_failure(CoreErrorCode::SystemProbeFailed);
                CoreError::new(CoreErrorCode::SystemProbeFailed, error.to_string())
            })
    }

    pub fn request_replay(
        &self,
        kind: CaptureReplayKind,
        path: PathBuf,
        local_ip_hint: Option<Ipv4Addr>,
        include_incoming: bool,
        server_damage_calibration: bool,
    ) -> Result<(), CoreError> {
        self.ensure_event_worker()?;
        let _gate = self
            .0
            .event_gate
            .lock()
            .expect("live capture event gate poisoned");
        {
            let status = self
                .0
                .status
                .lock()
                .expect("live capture status lock poisoned");
            if matches!(
                status.phase,
                LiveCapturePhase::Starting | LiveCapturePhase::Running | LiveCapturePhase::Stopping
            ) || self
                .0
                .replay
                .lock()
                .expect("capture replay lock poisoned")
                .is_some()
            {
                return Err(CoreError::new(
                    CoreErrorCode::CaptureAlreadyRunning,
                    "capture or replay is already active",
                ));
            }
        }

        *self
            .0
            .state
            .lock()
            .expect("live capture state lock poisoned") = CombatState::default();
        *self
            .0
            .last_outgoing_hit_at
            .lock()
            .expect("live capture activity lock poisoned") = None;
        self.0.bump_packet_session();
        let stop = Arc::new(AtomicBool::new(false));
        let thread = match kind {
            CaptureReplayKind::Pcapng => import_pcapng(
                path,
                CaptureResources {
                    characters: Arc::clone(&self.0.resources.characters),
                    ability_catalog: Arc::clone(&self.0.resources.ability_catalog),
                },
                local_ip_hint,
                include_incoming,
                server_damage_calibration,
                self.0.sender.clone(),
                Arc::clone(&stop),
            ),
            CaptureReplayKind::Json => {
                import_capture_json(path, self.0.sender.clone(), Arc::clone(&stop))
            }
        };
        *self.0.replay.lock().expect("capture replay lock poisoned") =
            Some(ReplayTask { stop, thread });
        *self
            .0
            .quality_source
            .lock()
            .expect("capture quality source lock poisoned") = match kind {
            CaptureReplayKind::Pcapng => CaptureQualitySource::PcapngReplay,
            CaptureReplayKind::Json => CaptureQualitySource::JsonReplay,
        };
        *self
            .0
            .status
            .lock()
            .expect("live capture status lock poisoned") = LiveCaptureStatus {
            phase: LiveCapturePhase::Running,
            issue: None,
        };
        self.0.bump_revision();
        Ok(())
    }

    pub fn request_stop(&self) -> Result<(), CoreError> {
        let replay_stop = self
            .0
            .replay
            .lock()
            .expect("capture replay lock poisoned")
            .as_ref()
            .map(|replay| Arc::clone(&replay.stop));
        let should_spawn = {
            let mut status = self
                .0
                .status
                .lock()
                .expect("live capture status lock poisoned");
            match status.phase {
                LiveCapturePhase::Starting => {
                    status.phase = LiveCapturePhase::Stopping;
                    false
                }
                LiveCapturePhase::Running | LiveCapturePhase::Failed => {
                    status.phase = LiveCapturePhase::Stopping;
                    replay_stop.is_none()
                }
                LiveCapturePhase::Stopping => return Ok(()),
                LiveCapturePhase::Idle | LiveCapturePhase::Stopped => {
                    return Err(CoreError::new(
                        CoreErrorCode::CaptureNotRunning,
                        "capture is not running",
                    ));
                }
            }
        };
        self.0.bump_capture_status_revision();

        if let Some(stop) = replay_stop {
            stop.store(true, Ordering::Release);
        }
        if should_spawn {
            self.spawn_stop_worker()?;
        }
        Ok(())
    }

    fn ensure_event_worker(&self) -> Result<(), CoreError> {
        let Some((reliable_receiver, debug_receiver)) = self
            .0
            .receiver
            .lock()
            .expect("live capture receiver lock poisoned")
            .take()
        else {
            return Ok(());
        };
        let weak = Arc::downgrade(&self.0);
        let worker_reliable_receiver = reliable_receiver.clone();
        let worker_debug_receiver = debug_receiver.clone();
        match thread::Builder::new()
            .name("nte-live-engine-events".to_owned())
            .spawn(move || engine_event_loop(weak, worker_reliable_receiver, worker_debug_receiver))
        {
            Ok(_) => Ok(()),
            Err(error) => {
                *self
                    .0
                    .receiver
                    .lock()
                    .expect("live capture receiver lock poisoned") =
                    Some((reliable_receiver, debug_receiver));
                Err(CoreError::new(
                    CoreErrorCode::SystemProbeFailed,
                    error.to_string(),
                ))
            }
        }
    }

    fn finish_start(&self, options: CaptureControllerOptions) {
        let result = {
            // Holding the state lock keeps the event worker from applying the
            // first packet until a successful start has atomically reset the
            // previous combat session.
            let mut state = self
                .0
                .state
                .lock()
                .expect("live capture state lock poisoned");
            let result = self
                .0
                .controller
                .lock()
                .expect("live capture controller lock poisoned")
                .start(
                    options,
                    Arc::clone(&self.0.resources.characters),
                    Arc::clone(&self.0.resources.ability_catalog),
                    self.0.sender.clone(),
                );
            if result.is_ok() {
                *state = CombatState::default();
                self.0.bump_packet_session();
            }
            result
        };

        match result {
            Ok(()) => {
                *self
                    .0
                    .quality_source
                    .lock()
                    .expect("capture quality source lock poisoned") = CaptureQualitySource::Live;
                let should_stop = {
                    let mut status = self
                        .0
                        .status
                        .lock()
                        .expect("live capture status lock poisoned");
                    if status.phase == LiveCapturePhase::Stopping {
                        true
                    } else {
                        *status = LiveCaptureStatus {
                            phase: LiveCapturePhase::Running,
                            issue: None,
                        };
                        false
                    }
                };
                self.0.bump_capture_status_revision();
                if should_stop && self.spawn_stop_worker().is_err() {
                    self.record_start_failure(CoreErrorCode::SystemProbeFailed);
                }
            }
            Err(error) => self.record_start_failure(error.code),
        }
    }

    fn spawn_stop_worker(&self) -> Result<(), CoreError> {
        let service = self.clone();
        thread::Builder::new()
            .name("nte-live-capture-stop".to_owned())
            .spawn(move || service.finish_stop())
            .map(|_| ())
            .map_err(|error| {
                self.record_start_failure(CoreErrorCode::SystemProbeFailed);
                CoreError::new(CoreErrorCode::SystemProbeFailed, error.to_string())
            })
    }

    fn finish_stop(&self) {
        let mut controller = self
            .0
            .controller
            .lock()
            .expect("live capture controller lock poisoned");
        if controller.is_running() {
            controller
                .stop()
                .expect("running live capture controller must stop");
        }
        drop(controller);

        let mut status = self
            .0
            .status
            .lock()
            .expect("live capture status lock poisoned");
        if status.phase != LiveCapturePhase::Failed {
            *status = LiveCaptureStatus {
                phase: LiveCapturePhase::Stopped,
                issue: None,
            };
        }
        drop(status);
        self.0.bump_capture_status_revision();
    }

    fn record_start_failure(&self, code: CoreErrorCode) {
        *self
            .0
            .status
            .lock()
            .expect("live capture status lock poisoned") = LiveCaptureStatus {
            phase: LiveCapturePhase::Failed,
            issue: Some(LiveCaptureIssue::Start(code)),
        };
        self.0.bump_capture_status_revision();
    }
}

impl LiveCaptureInner {
    fn bump_revision(&self) {
        self.revision.fetch_add(1, Ordering::AcqRel);
    }

    fn bump_capture_status_revision(&self) {
        self.bump_revision();
        self.packet_revision.fetch_add(1, Ordering::AcqRel);
    }

    fn bump_packet_session(&self) {
        self.packet_session_generation
            .fetch_add(1, Ordering::AcqRel);
        self.packet_revision.fetch_add(1, Ordering::AcqRel);
    }

    fn process_event(&self, event: EngineEvent) {
        let _gate = self
            .event_gate
            .lock()
            .expect("live capture event gate poisoned");
        let signal = {
            let mut state = self.state.lock().expect("live capture state lock poisoned");
            let history_hits_generation = state
                .hits_generation
                .wrapping_add(state.abyss.first_half.hits_generation)
                .wrapping_add(state.abyss.second_half.hits_generation);
            if let EngineEvent::Abyss(abyss) = &event
                && abyss_event_starts_new_round(state.abyss.floor, abyss)
                && history_hits_generation
                    != self
                        .last_abyss_archive_hits_generation
                        .load(Ordering::Acquire)
                && let Some(details) = HistoryCombatDetails::from_state(&state)
            {
                self.last_abyss_archive_hits_generation
                    .store(history_hits_generation, Ordering::Release);
                self.pending_abyss_archives
                    .lock()
                    .expect("live capture Abyss archive lock poisoned")
                    .push_back(details);
            }
            if let EngineEvent::Hit(hit) = &event {
                if !hit.direction.is_incoming() {
                    *self
                        .last_outgoing_hit_at
                        .lock()
                        .expect("live capture activity lock poisoned") = Some(Instant::now());
                }
                if hit.direction.is_outgoing() {
                    self.outgoing_hit_revision.fetch_add(1, Ordering::AcqRel);
                }
            }
            apply_engine_event(&mut state, event)
        };

        let affects_packet_projection = matches!(
            &signal,
            CoreSignal::DebugPacket
                | CoreSignal::PacketObserved
                | CoreSignal::Status(_)
                | CoreSignal::Error(_)
                | CoreSignal::CaptureStopped
        );
        let affects_frontend_projection = match signal {
            CoreSignal::StateChanged => true,
            CoreSignal::InventoryReplaced
            | CoreSignal::InventoryCharactersReplaced
            | CoreSignal::DebugPacket
            | CoreSignal::PacketObserved
            | CoreSignal::ModScript(_) => false,
            CoreSignal::Status(_) => {
                let mut status = self
                    .status
                    .lock()
                    .expect("live capture status lock poisoned");
                if status.phase == LiveCapturePhase::Starting {
                    status.phase = LiveCapturePhase::Running;
                }
                true
            }
            CoreSignal::Warning(_) => {
                self.status
                    .lock()
                    .expect("live capture status lock poisoned")
                    .issue = Some(LiveCaptureIssue::RuntimeWarning);
                true
            }
            CoreSignal::Error(_) => {
                *self
                    .status
                    .lock()
                    .expect("live capture status lock poisoned") = LiveCaptureStatus {
                    phase: LiveCapturePhase::Failed,
                    issue: Some(LiveCaptureIssue::RuntimeError),
                };
                true
            }
            CoreSignal::CaptureStopped => {
                self.controller
                    .lock()
                    .expect("live capture controller lock poisoned")
                    .capture_stopped();
                if let Some(replay) = self
                    .replay
                    .lock()
                    .expect("capture replay lock poisoned")
                    .take()
                {
                    let _ = replay.thread.join();
                }
                let mut status = self
                    .status
                    .lock()
                    .expect("live capture status lock poisoned");
                if status.phase != LiveCapturePhase::Failed {
                    *status = LiveCaptureStatus {
                        phase: LiveCapturePhase::Stopped,
                        issue: None,
                    };
                }
                true
            }
        };

        if affects_frontend_projection {
            self.bump_revision();
        }
        if affects_packet_projection {
            self.packet_revision.fetch_add(1, Ordering::AcqRel);
        }
    }
}

impl Drop for LiveCaptureInner {
    fn drop(&mut self) {
        if let Some(replay) = self
            .replay
            .get_mut()
            .expect("capture replay lock poisoned")
            .take()
        {
            replay.stop.store(true, Ordering::Release);
            let _ = replay.thread.join();
        }
        self.controller
            .get_mut()
            .expect("live capture controller lock poisoned")
            .stop_if_running();
    }
}

fn engine_event_loop(
    inner: Weak<LiveCaptureInner>,
    reliable_receiver: Receiver<EngineEvent>,
    debug_receiver: Receiver<EngineEvent>,
) {
    loop {
        let event = select! {
            recv(reliable_receiver) -> event => event,
            recv(debug_receiver) -> event => event,
        };
        let Ok(event) = event else {
            break;
        };
        let Some(inner) = inner.upgrade() else {
            break;
        };
        inner.process_event(event);
    }
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, Instant};

    use super::*;
    use crate::engine::model::{
        AbyssEvent, Hit, HitCharacterSource, HitDirection, PacketObservation,
    };

    fn hit(damage: f64) -> EngineEvent {
        EngineEvent::Hit(Box::new(Hit {
            timestamp: 1.0,
            char_id: 7,
            char_name: "Character 7".to_owned(),
            char_known: true,
            damage,
            byte_offset: 0,
            bit_shift: 0,
            char_source: HitCharacterSource::Packet,
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
        }))
    }

    #[test]
    fn event_worker_routes_hits_through_the_shared_reducer() {
        let service = LiveCaptureService::new(LiveCaptureResources::default());
        let initial_revision = service.revision();
        service.ensure_event_worker().expect("event worker");
        service.0.sender.send(hit(321.0)).expect("test hit");

        let deadline = Instant::now() + Duration::from_secs(1);
        while service.with_state(|state| state.total_damage) != 321.0
            || service.revision() == initial_revision
        {
            assert!(Instant::now() < deadline, "event worker timed out");
            thread::yield_now();
        }

        assert_eq!(service.with_state(|state| state.hits.len()), 1);
        assert!(service.revision() > initial_revision);
    }

    #[test]
    fn inventory_events_advance_only_the_inventory_projection() {
        let service = LiveCaptureService::new(LiveCaptureResources::default());
        let initial_revision = service.revision();
        let initial_inventory_revision = service.inventory_revision();

        service
            .0
            .process_event(EngineEvent::EmptyCurtain(Vec::new()));
        let inventory_revision = service.inventory_revision();
        assert!(inventory_revision.0 > initial_inventory_revision.0);
        assert_eq!(service.revision(), initial_revision);

        service
            .0
            .process_event(EngineEvent::EmptyCurtainCharacters(Vec::new()));
        assert!(service.inventory_revision().1 > inventory_revision.1);
        assert_eq!(service.revision(), initial_revision);
    }

    #[test]
    fn archive_reset_keeps_the_round_when_persistence_fails() {
        let service = LiveCaptureService::new(LiveCaptureResources::default());
        service.0.process_event(hit(321.0));

        let result: Result<Option<()>, &str> = service.archive_and_reset(
            |state| (!state.hits.is_empty()).then(|| state.total_damage),
            |_| Err("disk full"),
        );

        assert_eq!(result, Err("disk full"));
        assert_eq!(service.with_state(|state| state.total_damage), 321.0);
    }

    #[test]
    fn archive_reset_clears_the_round_only_after_persistence_succeeds() {
        let service = LiveCaptureService::new(LiveCaptureResources::default());
        service.0.process_event(hit(321.0));

        let result: Result<Option<f64>, &str> = service.archive_and_reset(
            |state| (!state.hits.is_empty()).then(|| state.total_damage),
            Ok,
        );

        assert_eq!(result, Ok(Some(321.0)));
        assert!(service.with_state(|state| state.hits.is_empty()));
    }

    #[test]
    fn abyss_restart_queues_the_previous_round_before_reducer_changes_state() {
        let service = LiveCaptureService::new(LiveCaptureResources::default());
        let EngineEvent::Hit(previous_hit) = hit(321.0) else {
            unreachable!("test helper returns a hit")
        };
        {
            let mut state = service.0.state.lock().expect("live capture state lock");
            state.apply_abyss_event(AbyssEvent::Stage {
                timestamp: 0.0,
                cycle: None,
                floor: Some(12),
                half: crate::engine::model::AbyssHalf::First,
                allow_late_backfill: false,
            });
            state.push_hit(*previous_hit);
        }
        service
            .0
            .process_event(EngineEvent::Abyss(AbyssEvent::RestartDetected {
                timestamp: 3.0,
            }));

        let archives = service.take_pending_abyss_archives();
        assert_eq!(archives.len(), 1);
        assert_eq!(archives[0].first_half_hits.len(), 1);
        assert_eq!(archives[0].first_half_hits[0].damage, 321.0);
    }

    #[test]
    fn runtime_failures_keep_private_details_out_of_status() {
        let service = LiveCaptureService::new(LiveCaptureResources::default());
        let initial_revision = service.revision();
        let initial_packet_revision = service.with_packet_state(|revision, _, _| revision);

        service
            .0
            .process_event(EngineEvent::Warning("private warning".to_owned()));
        assert_eq!(
            service.status().issue,
            Some(LiveCaptureIssue::RuntimeWarning)
        );
        let warning_revision = service.revision();
        assert!(warning_revision > initial_revision);

        service
            .0
            .process_event(EngineEvent::Error("private failure".to_owned()));
        assert_eq!(
            service.status(),
            LiveCaptureStatus {
                phase: LiveCapturePhase::Failed,
                issue: Some(LiveCaptureIssue::RuntimeError),
            }
        );
        assert!(service.revision() > warning_revision);
        assert!(
            service.with_packet_state(|revision, _, _| revision.generation)
                > initial_packet_revision.generation
        );
    }

    #[test]
    fn packet_quality_observations_do_not_invalidate_hud_projection() {
        let service = LiveCaptureService::new(LiveCaptureResources::default());
        let initial_revision = service.revision();
        let initial_packet_revision = service.with_packet_state(|revision, _, _| revision);

        service
            .0
            .process_event(EngineEvent::PacketObservation(PacketObservation {
                parsed_hits: 1,
            }));

        assert_eq!(service.revision(), initial_revision);
        assert_eq!(service.with_state(|state| state.packet_count), 1);
        assert!(
            service.with_packet_state(|revision, _, _| revision.generation)
                > initial_packet_revision.generation
        );
    }

    #[test]
    fn outgoing_revision_advances_only_for_outgoing_hits() {
        let service = LiveCaptureService::new(LiveCaptureResources::default());
        let initial = service.outgoing_hit_revision();
        let EngineEvent::Hit(mut incoming) = hit(12.0) else {
            unreachable!("test helper returns a hit")
        };
        incoming.direction = HitDirection::Incoming;

        service.0.process_event(EngineEvent::Hit(incoming));
        assert_eq!(service.outgoing_hit_revision(), initial);

        service.0.process_event(hit(34.0));
        assert_eq!(service.outgoing_hit_revision(), initial + 1);
    }

    #[test]
    fn reset_session_can_restore_the_exact_previous_projection() {
        let service = LiveCaptureService::new(LiveCaptureResources::default());
        service.0.process_event(hit(321.0));
        let previous = service.with_state(Clone::clone);

        service.reset_session();
        assert!(service.with_state(|state| state.hits.is_empty()));

        service.restore_session(previous, CaptureQualitySource::PcapngReplay);
        assert_eq!(service.with_state(|state| state.total_damage), 321.0);
        assert_eq!(service.quality_source(), CaptureQualitySource::PcapngReplay);
    }

    #[test]
    fn stopping_an_idle_service_reports_the_stable_core_error() {
        let service = LiveCaptureService::new(LiveCaptureResources::default());

        let error = service
            .request_stop()
            .expect_err("idle capture must reject stop");

        assert_eq!(error.code, CoreErrorCode::CaptureNotRunning);
        assert_eq!(service.status(), LiveCaptureStatus::default());
    }
}
