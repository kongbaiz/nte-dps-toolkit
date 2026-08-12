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

use crossbeam_channel::{Receiver, RecvError, TryRecvError, bounded, select_biased};

use super::{
    CoreError, CoreErrorCode,
    capture::{CaptureController, CaptureControllerOptions},
    history::{PendingHistoryArchive, abyss_event_starts_new_round},
    reducer::{CoreSignal, apply_engine_event},
};
use crate::{
    engine::{
        capture::{
            CaptureResources, EngineEventSink, RawCaptureSnapshot, import_capture_json,
            import_pcapng,
        },
        model::{
            AbyssEvent, CaptureQualitySource, CaptureQualitySummary, CharacterInfo, CombatState,
            EngineEvent,
        },
        parser::{AbilityCatalog, CHARACTER_DATA_PATH, load_characters},
    },
    storage::{ability_names, history::HistoryCombatDetails, i18n::Language},
};

const RELIABLE_ENGINE_EVENT_CAPACITY: usize = 16_384;
const DEBUG_ENGINE_EVENT_CAPACITY: usize = 2_048;
const MAX_PENDING_ABYSS_ARCHIVES: usize = 64;

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
            Ok(characters) => {
                #[cfg(feature = "desktop")]
                let characters = {
                    let mut characters = characters;
                    crate::storage::resource::assign_missing_character_colors(&mut characters);
                    characters
                };
                Arc::new(characters)
            }
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

/// An authoritative round detached from live capture under `event_gate`.
/// Callers may prepare and persist it after the gate is released; new engine
/// events are already routed into the replacement state.
pub struct CutRound {
    pub state: CombatState,
    pub source: CaptureQualitySource,
}

struct DetachedAbyssRound {
    state: CombatState,
    source: CaptureQualitySource,
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
    pending_abyss_archives: Mutex<VecDeque<PendingHistoryArchive>>,
    dropped_history_archives: AtomicU64,
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
            dropped_history_archives: AtomicU64::new(0),
        }))
    }

    pub fn status(&self) -> LiveCaptureStatus {
        *self
            .0
            .status
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
    }

    pub fn replay_running(&self) -> bool {
        self.0
            .replay
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
            .is_some()
    }

    pub fn active_capture_filter(&self) -> Option<String> {
        self.0
            .controller
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
            .active_filter()
    }

    pub fn raw_capture_snapshot(&self) -> Option<RawCaptureSnapshot> {
        self.0
            .controller
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
            .raw_capture_snapshot()
    }

    pub fn save_last_raw_capture(&self, path: &Path) -> Result<(u64, u64), String> {
        self.0
            .controller
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
            .save_last_raw_capture(path)
    }

    pub fn quality_source(&self) -> CaptureQualitySource {
        *self
            .0
            .quality_source
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
    }

    pub fn quality_summary(&self) -> CaptureQualitySummary {
        self.with_state_and_source(|state, source| state.capture_quality_summary(source))
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
            .unwrap_or_else(|poison| poison.into_inner());
        read(&state)
    }

    /// Reads the authoritative state and its provenance from the same capture
    /// session. Lock order is `event_gate -> state -> quality_source`.
    fn with_state_and_source<T>(
        &self,
        read: impl FnOnce(&CombatState, CaptureQualitySource) -> T,
    ) -> T {
        let _gate = self
            .0
            .event_gate
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        let state = self
            .0
            .state
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        let source = *self
            .0
            .quality_source
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        read(&state, source)
    }

    /// Clones one low-frequency, provenance-frozen capture snapshot for work
    /// that must continue after the capture locks are released.
    pub fn state_and_source_snapshot(&self) -> (CombatState, CaptureQualitySource) {
        self.with_state_and_source(|state, source| (state.clone(), source))
    }

    pub fn with_packet_state<T>(
        &self,
        read: impl FnOnce(super::packets::PacketStreamRevision, usize, &CombatState) -> T,
    ) -> T {
        let _gate = self
            .0
            .event_gate
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        let state = self
            .0
            .state
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
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
            .unwrap_or_else(|poison| poison.into_inner())
            .map(|last| last.elapsed())
    }

    pub fn take_pending_abyss_archives(&self) -> Vec<PendingHistoryArchive> {
        self.0
            .pending_abyss_archives
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
            .drain(..)
            .collect()
    }

    pub fn restore_pending_abyss_archives(&self, archives: Vec<PendingHistoryArchive>) {
        let mut pending = self
            .0
            .pending_abyss_archives
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        let mut dropped = 0_usize;
        for archive in archives.into_iter().rev() {
            if pending.len() >= MAX_PENDING_ABYSS_ARCHIVES {
                pending.pop_back();
                dropped += 1;
            }
            pending.push_front(archive);
        }
        if dropped > 0 {
            self.0.note_dropped_history_archives(dropped as u64);
            eprintln!(
                "automatic Abyss History retry queue exceeded {MAX_PENDING_ABYSS_ARCHIVES} entries; discarded {dropped} newest archive(s) while restoring older failed retries"
            );
        }
    }

    pub fn dropped_history_archives(&self) -> u64 {
        self.0.dropped_history_archives.load(Ordering::Acquire)
    }

    /// Atomically installs a fresh combat state and returns the detached round.
    ///
    /// Lock order is `event_gate -> state -> quality_source -> round runtime`.
    /// No serialization or persistence is allowed in this critical section.
    pub fn cut_round(&self) -> Option<CutRound> {
        let _gate = self
            .0
            .event_gate
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        let mut state = self
            .0
            .state
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        if state.hits.is_empty() && state.stats.is_empty() && !state.abyss.is_active() {
            return None;
        }
        let detached = std::mem::take(&mut *state);
        let source = *self
            .0
            .quality_source
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        *self
            .0
            .last_outgoing_hit_at
            .lock()
            .unwrap_or_else(|poison| poison.into_inner()) = None;
        self.0.bump_packet_session();
        self.0.bump_revision();
        Some(CutRound {
            state: detached,
            source,
        })
    }

    /// Clears the current combat projection while keeping capture resources and
    /// the active capture controller intact.
    pub fn reset_session(&self) {
        let _gate = self
            .0
            .event_gate
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        *self
            .0
            .state
            .lock()
            .unwrap_or_else(|poison| poison.into_inner()) = CombatState::default();
        *self
            .0
            .last_outgoing_hit_at
            .lock()
            .unwrap_or_else(|poison| poison.into_inner()) = None;
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
            .unwrap_or_else(|poison| poison.into_inner());
        let has_outgoing = state.hits.iter().any(|hit| hit.direction.is_outgoing());
        *self
            .0
            .state
            .lock()
            .unwrap_or_else(|poison| poison.into_inner()) = state;
        *self
            .0
            .quality_source
            .lock()
            .unwrap_or_else(|poison| poison.into_inner()) = quality_source;
        *self
            .0
            .last_outgoing_hit_at
            .lock()
            .unwrap_or_else(|poison| poison.into_inner()) = has_outgoing.then_some(Instant::now());
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
                .unwrap_or_else(|poison| poison.into_inner());
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
            .unwrap_or_else(|poison| poison.into_inner());
        {
            let status = self
                .0
                .status
                .lock()
                .unwrap_or_else(|poison| poison.into_inner());
            if matches!(
                status.phase,
                LiveCapturePhase::Starting | LiveCapturePhase::Running | LiveCapturePhase::Stopping
            ) || self
                .0
                .replay
                .lock()
                .unwrap_or_else(|poison| poison.into_inner())
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
            .unwrap_or_else(|poison| poison.into_inner()) = CombatState::default();
        *self
            .0
            .last_outgoing_hit_at
            .lock()
            .unwrap_or_else(|poison| poison.into_inner()) = None;
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
        *self
            .0
            .replay
            .lock()
            .unwrap_or_else(|poison| poison.into_inner()) = Some(ReplayTask { stop, thread });
        *self
            .0
            .quality_source
            .lock()
            .unwrap_or_else(|poison| poison.into_inner()) = match kind {
            CaptureReplayKind::Pcapng => CaptureQualitySource::PcapngReplay,
            CaptureReplayKind::Json => CaptureQualitySource::JsonReplay,
        };
        *self
            .0
            .status
            .lock()
            .unwrap_or_else(|poison| poison.into_inner()) = LiveCaptureStatus {
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
            .unwrap_or_else(|poison| poison.into_inner())
            .as_ref()
            .map(|replay| Arc::clone(&replay.stop));
        let should_spawn = {
            let mut status = self
                .0
                .status
                .lock()
                .unwrap_or_else(|poison| poison.into_inner());
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
            .unwrap_or_else(|poison| poison.into_inner())
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
                    .unwrap_or_else(|poison| poison.into_inner()) =
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
                .unwrap_or_else(|poison| poison.into_inner());
            let result = self
                .0
                .controller
                .lock()
                .unwrap_or_else(|poison| poison.into_inner())
                .start(
                    options,
                    Arc::clone(&self.0.resources.characters),
                    Arc::clone(&self.0.resources.ability_catalog),
                    self.0.sender.clone(),
                );
            if result.is_ok() {
                *state = CombatState::default();
                *self
                    .0
                    .quality_source
                    .lock()
                    .unwrap_or_else(|poison| poison.into_inner()) = CaptureQualitySource::Live;
                self.0.bump_packet_session();
            }
            result
        };

        match result {
            Ok(()) => {
                let should_stop = {
                    let mut status = self
                        .0
                        .status
                        .lock()
                        .unwrap_or_else(|poison| poison.into_inner());
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
            .unwrap_or_else(|poison| poison.into_inner());

        if controller.is_running() {
            controller
                .stop()
                .expect("running live capture controller must stop");
            return;
        }

        drop(controller);

        let changed = {
            let mut status = self
                .0
                .status
                .lock()
                .unwrap_or_else(|poison| poison.into_inner());

            if status.phase == LiveCapturePhase::Stopping {
                *status = LiveCaptureStatus {
                    phase: LiveCapturePhase::Stopped,
                    issue: None,
                };
                true
            } else {
                false
            }
        };

        if changed {
            self.0.bump_capture_status_revision();
        }
    }

    fn record_start_failure(&self, code: CoreErrorCode) {
        *self
            .0
            .status
            .lock()
            .unwrap_or_else(|poison| poison.into_inner()) = LiveCaptureStatus {
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

    fn note_dropped_history_archives(&self, count: u64) {
        self.dropped_history_archives
            .fetch_add(count, Ordering::AcqRel);
        self.status
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
            .issue = Some(LiveCaptureIssue::RuntimeWarning);
        self.bump_revision();
    }

    /// Detaches the current Abyss round only when its hit content changed since
    /// the last archive. The expensive HistoryCombatDetails conversion happens
    /// after the event gate and state lock are released.
    ///
    /// Taking the state makes the boundary itself the dedupe barrier: after a
    /// round is detached, the replacement state contains no old Abyss hits.
    fn detach_abyss_round_if_changed(&self, state: &mut CombatState) -> Option<DetachedAbyssRound> {
        let has_abyss_hits =
            !state.abyss.first_half.hits.is_empty() || !state.abyss.second_half.hits.is_empty();
        if !has_abyss_hits {
            return None;
        }
        let source = *self
            .quality_source
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        let detached = std::mem::take(state);
        *self
            .last_outgoing_hit_at
            .lock()
            .unwrap_or_else(|poison| poison.into_inner()) = None;
        self.bump_packet_session();
        Some(DetachedAbyssRound {
            state: detached,
            source,
        })
    }

    fn queue_detached_abyss_round(&self, detached: DetachedAbyssRound) {
        let Some(details) = HistoryCombatDetails::from_state(&detached.state) else {
            return;
        };
        let mut pending = self
            .pending_abyss_archives
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        if pending.len() >= MAX_PENDING_ABYSS_ARCHIVES {
            drop(pending);
            self.note_dropped_history_archives(1);
            eprintln!(
                "automatic Abyss History retry queue is full at {MAX_PENDING_ABYSS_ARCHIVES} entries; newest round was not queued"
            );
            return;
        }
        pending.push_back(PendingHistoryArchive {
            details,
            source: detached.source,
        });
    }

    fn process_event(&self, event: EngineEvent) {
        let (signal, detached_abyss_round) = {
            let _gate = self
                .event_gate
                .lock()
                .unwrap_or_else(|poison| poison.into_inner());
            let mut state = self
                .state
                .lock()
                .unwrap_or_else(|poison| poison.into_inner());
            let pre_event_abyss_boundary = matches!(
                &event,
                EngineEvent::Abyss(abyss)
                    if abyss_event_starts_new_round(state.abyss.floor, abyss)
            );
            let post_event_abyss_archive = matches!(&event, EngineEvent::CaptureStopped)
                || matches!(&event, EngineEvent::Abyss(AbyssEvent::Exit { .. }));
            let mut detached_abyss_round = pre_event_abyss_boundary
                .then(|| self.detach_abyss_round_if_changed(&mut state))
                .flatten();
            if let EngineEvent::Hit(hit) = &event {
                if hit.direction.is_outgoing() {
                    *self
                        .last_outgoing_hit_at
                        .lock()
                        .unwrap_or_else(|poison| poison.into_inner()) = Some(Instant::now());
                    self.outgoing_hit_revision.fetch_add(1, Ordering::AcqRel);
                }
            }
            let signal = apply_engine_event(&mut state, event);
            if post_event_abyss_archive && detached_abyss_round.is_none() {
                detached_abyss_round = self.detach_abyss_round_if_changed(&mut state);
            }
            (signal, detached_abyss_round)
        };
        if let Some(detached_abyss_round) = detached_abyss_round {
            self.queue_detached_abyss_round(detached_abyss_round);
        }

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
            | CoreSignal::PacketObserved => false,
            CoreSignal::ModScript { state_changed, .. } => state_changed,
            CoreSignal::Status(_) => {
                let mut status = self
                    .status
                    .lock()
                    .unwrap_or_else(|poison| poison.into_inner());
                if status.phase == LiveCapturePhase::Starting {
                    status.phase = LiveCapturePhase::Running;
                }
                true
            }
            CoreSignal::Warning(_) => {
                self.status
                    .lock()
                    .unwrap_or_else(|poison| poison.into_inner())
                    .issue = Some(LiveCaptureIssue::RuntimeWarning);
                true
            }
            CoreSignal::Error(_) => {
                *self
                    .status
                    .lock()
                    .unwrap_or_else(|poison| poison.into_inner()) = LiveCaptureStatus {
                    phase: LiveCapturePhase::Failed,
                    issue: Some(LiveCaptureIssue::RuntimeError),
                };
                true
            }
            CoreSignal::CaptureStopped => {
                self.controller
                    .lock()
                    .unwrap_or_else(|poison| poison.into_inner())
                    .capture_stopped();
                if let Some(replay) = self
                    .replay
                    .lock()
                    .unwrap_or_else(|poison| poison.into_inner())
                    .take()
                {
                    let _ = replay.thread.join();
                }
                let mut status = self
                    .status
                    .lock()
                    .unwrap_or_else(|poison| poison.into_inner());
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
            .unwrap_or_else(|poison| poison.into_inner())
            .take()
        {
            replay.stop.store(true, Ordering::Release);
            let _ = replay.thread.join();
        }
        self.controller
            .get_mut()
            .unwrap_or_else(|poison| poison.into_inner())
            .stop_if_running();
    }
}

fn engine_event_loop(
    inner: Weak<LiveCaptureInner>,
    reliable_receiver: Receiver<EngineEvent>,
    debug_receiver: Receiver<EngineEvent>,
) {
    loop {
        let defer_debug = match inner.upgrade() {
            Some(inner) => inner
                .replay
                .lock()
                .unwrap_or_else(|poison| poison.into_inner())
                .is_some(),
            None => break,
        };
        let Ok(event) = next_engine_event(&reliable_receiver, &debug_receiver, defer_debug) else {
            break;
        };
        let Some(inner) = inner.upgrade() else {
            break;
        };
        inner.process_event(event);
    }
}

/// Preserve the established reliable-first replay behavior. Semantic events
/// and `CaptureStopped` must not wait behind thousands of optional full packet
/// payloads; the bounded debug lane is intentionally allowed to fill and drop
/// those payloads while a replay burst is still producing authoritative data.
fn next_engine_event(
    reliable_receiver: &Receiver<EngineEvent>,
    debug_receiver: &Receiver<EngineEvent>,
    defer_debug: bool,
) -> Result<EngineEvent, RecvError> {
    if defer_debug {
        return reliable_receiver.recv();
    }
    match reliable_receiver.try_recv() {
        Ok(event) => return Ok(event),
        Err(TryRecvError::Disconnected) => return debug_receiver.recv(),
        Err(TryRecvError::Empty) => {}
    }
    match debug_receiver.try_recv() {
        Ok(event) => return Ok(event),
        Err(TryRecvError::Disconnected) => return reliable_receiver.recv(),
        Err(TryRecvError::Empty) => {}
    }
    select_biased! {
        recv(reliable_receiver) -> event => event,
        recv(debug_receiver) -> event => event,
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
    fn event_worker_prioritizes_reliable_replay_events() {
        let (reliable_sender, reliable_receiver) = bounded(2);
        let (debug_sender, debug_receiver) = bounded(2);
        debug_sender
            .send(EngineEvent::Status("debug".to_owned()))
            .expect("debug event");
        reliable_sender
            .send(EngineEvent::Status("reliable".to_owned()))
            .expect("reliable event");

        let EngineEvent::Status(first) =
            next_engine_event(&reliable_receiver, &debug_receiver, false).expect("first event")
        else {
            panic!("test events are status values")
        };
        let EngineEvent::Status(second) =
            next_engine_event(&reliable_receiver, &debug_receiver, false).expect("second event")
        else {
            panic!("test events are status values")
        };

        assert_eq!(first, "reliable");
        assert_eq!(second, "debug");
    }

    #[test]
    fn replay_bursts_defer_optional_debug_packets() {
        let (reliable_sender, reliable_receiver) = bounded(1);
        let (debug_sender, debug_receiver) = bounded(1);
        debug_sender
            .send(EngineEvent::Status("debug".to_owned()))
            .expect("debug event");
        let producer = thread::spawn(move || {
            thread::sleep(Duration::from_millis(10));
            reliable_sender
                .send(EngineEvent::Status("reliable".to_owned()))
                .expect("reliable event");
        });

        let EngineEvent::Status(event) =
            next_engine_event(&reliable_receiver, &debug_receiver, true)
                .expect("reliable replay event")
        else {
            panic!("test event is a status value")
        };

        producer.join().expect("producer");
        assert_eq!(event, "reliable");
        assert_eq!(debug_receiver.len(), 1);
    }

    #[test]
    #[ignore = "set NTE_TEST_CAPTURE to a local pcapng path"]
    fn stress_replay_finishes_before_optional_debug_drain() {
        let path =
            PathBuf::from(std::env::var("NTE_TEST_CAPTURE").expect("NTE_TEST_CAPTURE must be set"));
        let (resources, warnings) = LiveCaptureResources::load(Language::SimplifiedChinese);
        assert!(warnings.is_empty(), "resource warnings: {warnings:#?}");
        let service = LiveCaptureService::new(resources);
        let started = Instant::now();

        service
            .request_replay(CaptureReplayKind::Pcapng, path, None, true, false)
            .expect("start replay");
        let deadline = started + Duration::from_secs(120);
        while service.status().phase != LiveCapturePhase::Stopped {
            assert!(Instant::now() < deadline, "replay timed out");
            thread::sleep(Duration::from_millis(5));
        }

        let pending_events = service.with_packet_state(|_, pending, _| pending);
        let dropped_debug_packets = service.0.sender.take_dropped_debug_packets();
        println!(
            "replay reached stopped in {:?}: pending_events={pending_events}, dropped_debug_packets={dropped_debug_packets}",
            started.elapsed()
        );
    }

    #[test]
    fn mod_script_backfill_advances_only_when_projection_changes() {
        use crate::engine::model::ModScriptEvent;

        fn identity_event(timestamp: f64) -> ModScriptEvent {
            let mut event = ModScriptEvent::from_bridge(
                1,
                filetime(timestamp),
                "enemy-telemetry".to_owned(),
                "pre.enemy.identity".to_owned(),
                vec![0x1234, 0x4d88_7b49_05d5_dbaf, 80],
            );
            event.enemy_identity = Some(crate::engine::model::EnemyIdentity {
                config_hash: 0x4d88_7b49_05d5_dbaf,
                config_id: "Boss_016_BP".to_owned(),
                monster_id: "Boss_16".to_owned(),
                name_en: "Imaginadough".to_owned(),
                name_zh: "随心泥".to_owned(),
                name_ja: "イメージクレイ".to_owned(),
            });
            event
        }

        fn hit_target_event(sequence: u64, timestamp: f64) -> ModScriptEvent {
            let mut event = identity_event(timestamp);
            event.sequence = sequence;
            event.phase = crate::engine::model::ModScriptEventPhase::Postprocess;
            event.name = "enemy.hit_target".to_owned();
            event
        }

        const FILETIME_UNIX_EPOCH_100NS: u64 = 116_444_736_000_000_000;
        fn filetime(timestamp: f64) -> u64 {
            FILETIME_UNIX_EPOCH_100NS + (timestamp * 10_000_000.0) as u64
        }

        let service = LiveCaptureService::new(LiveCaptureResources::default());
        service
            .0
            .process_event(EngineEvent::ModScript(identity_event(1.0)));
        service.0.process_event(hit(100.0));

        let before_target = service.revision();
        service
            .0
            .process_event(EngineEvent::ModScript(hit_target_event(2, 1.05)));
        assert!(
            service.revision() > before_target,
            "target backfill must advance the frontend revision"
        );
        assert_eq!(
            service.with_state(|state| state.hits[0].target_name.clone()),
            Some("随心泥".to_owned())
        );

        let before_duplicate = service.revision();
        service
            .0
            .process_event(EngineEvent::ModScript(hit_target_event(3, 1.05)));
        assert_eq!(
            service.revision(),
            before_duplicate,
            "idempotent ModScript backfill must not bump the revision"
        );
    }

    #[test]
    fn idle_policy_tracks_only_confirmed_outgoing_hits() {
        let service = LiveCaptureService::new(LiveCaptureResources::default());
        assert!(service.idle_elapsed().is_none());

        service.0.process_event(EngineEvent::Hit({
            let mut incoming = match hit(1.0) {
                EngineEvent::Hit(hit) => hit,
                _ => unreachable!("test helper returns a hit"),
            };
            incoming.direction = HitDirection::Incoming;
            incoming
        }));
        assert!(
            service.idle_elapsed().is_none(),
            "incoming hits must not reset the auto-round idle timer"
        );

        service.0.process_event(EngineEvent::Hit({
            let mut unknown = match hit(2.0) {
                EngineEvent::Hit(hit) => hit,
                _ => unreachable!("test helper returns a hit"),
            };
            unknown.direction = HitDirection::Unknown;
            unknown
        }));
        assert!(
            service.idle_elapsed().is_none(),
            "unknown-direction hits must not reset the auto-round idle timer"
        );

        service.0.process_event(hit(3.0));
        assert!(
            service.idle_elapsed().is_some(),
            "confirmed outgoing hits must reset the auto-round idle timer"
        );
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
    fn cut_round_installs_replacement_before_persistence_work() {
        let service = LiveCaptureService::new(LiveCaptureResources::default());
        service.0.process_event(hit(321.0));

        let cut = service.cut_round().expect("archivable round");
        service.0.process_event(hit(99.0));

        assert_eq!(cut.state.total_damage, 321.0);
        assert_eq!(cut.source, CaptureQualitySource::Unknown);
        assert_eq!(service.with_state(|state| state.total_damage), 99.0);
    }

    #[test]
    fn cut_round_freezes_replay_source_and_empty_round_is_a_no_op() {
        let service = LiveCaptureService::new(LiveCaptureResources::default());
        assert!(service.cut_round().is_none());
        *service
            .0
            .quality_source
            .lock()
            .expect("quality source lock") = CaptureQualitySource::JsonReplay;
        service.0.process_event(hit(321.0));

        let cut = service.cut_round().expect("archivable replay round");

        assert_eq!(cut.source, CaptureQualitySource::JsonReplay);
        assert_eq!(cut.state.total_damage, 321.0);
        assert!(service.with_state(|state| state.hits.is_empty()));
    }

    #[test]
    fn abyss_restart_queues_the_previous_round_before_reducer_changes_state() {
        let service = LiveCaptureService::new(LiveCaptureResources::default());
        *service
            .0
            .quality_source
            .lock()
            .expect("quality source lock") = CaptureQualitySource::PcapngReplay;
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
        assert_eq!(archives[0].details.first_half_hits.len(), 1);
        assert_eq!(archives[0].details.first_half_hits[0].damage, 321.0);
        assert_eq!(archives[0].source, CaptureQualitySource::PcapngReplay);
    }

    #[test]
    fn abyss_exit_archive_contains_the_real_exit_timestamp() {
        let service = LiveCaptureService::new(LiveCaptureResources::default());
        service
            .0
            .process_event(EngineEvent::Abyss(AbyssEvent::Stage {
                timestamp: 0.0,
                cycle: None,
                floor: Some(12),
                half: crate::engine::model::AbyssHalf::First,
                allow_late_backfill: false,
            }));
        service.0.process_event(hit(321.0));
        service
            .0
            .process_event(EngineEvent::Abyss(AbyssEvent::Exit { timestamp: 10.0 }));

        let archives = service.take_pending_abyss_archives();
        assert_eq!(archives.len(), 1);
        assert_eq!(archives[0].details.first_half_hits.len(), 1);
        assert_eq!(archives[0].details.first_half_hits[0].damage, 321.0);
        assert_eq!(archives[0].details.exited_at, Some(10.0));
    }

    #[test]
    fn final_abyss_round_stays_pending_when_the_session_is_reset() {
        let service = LiveCaptureService::new(LiveCaptureResources::default());
        let EngineEvent::Hit(previous_hit) = hit(654.0) else {
            unreachable!("test helper returns a hit")
        };
        {
            let mut state = service.0.state.lock().expect("live capture state lock");
            state.apply_abyss_event(AbyssEvent::Stage {
                timestamp: 0.0,
                cycle: None,
                floor: Some(12),
                half: crate::engine::model::AbyssHalf::Second,
                allow_late_backfill: false,
            });
            state.push_hit(*previous_hit);
        }

        service.0.process_event(EngineEvent::CaptureStopped);
        service.reset_session();

        let archives = service.take_pending_abyss_archives();
        assert_eq!(archives.len(), 1);
        assert_eq!(archives[0].details.second_half_hits.len(), 1);
        assert_eq!(archives[0].details.second_half_hits[0].damage, 654.0);
        assert!(service.with_state(|state| state.hits.is_empty()));
    }

    #[test]
    fn capture_stopped_archives_after_prior_semantic_events() {
        let service = LiveCaptureService::new(LiveCaptureResources::default());

        service
            .0
            .process_event(EngineEvent::Abyss(AbyssEvent::Stage {
                timestamp: 0.0,
                cycle: None,
                floor: Some(12),
                half: crate::engine::model::AbyssHalf::First,
                allow_late_backfill: false,
            }));
        service.0.process_event(hit(100.0));
        service.0.process_event(hit(200.0));

        service.0.process_event(EngineEvent::CaptureStopped);

        let archives = service.take_pending_abyss_archives();
        assert_eq!(archives.len(), 1);
        assert_eq!(archives[0].details.first_half_hits.len(), 2);
        assert_eq!(
            service.status().phase,
            LiveCapturePhase::Stopped,
            "CaptureStopped is the single final archive and Stopped barrier"
        );
    }

    #[test]
    fn pending_abyss_archive_queue_is_bounded() {
        let service = LiveCaptureService::new(LiveCaptureResources::default());
        service
            .0
            .process_event(EngineEvent::Abyss(AbyssEvent::Stage {
                timestamp: 0.0,
                cycle: None,
                floor: Some(12),
                half: crate::engine::model::AbyssHalf::First,
                allow_late_backfill: false,
            }));

        for round in 0..(MAX_PENDING_ABYSS_ARCHIVES + 4) {
            service.0.process_event(hit((round + 1) as f64));
            service
                .0
                .process_event(EngineEvent::Abyss(AbyssEvent::RestartDetected {
                    timestamp: (round + 1) as f64,
                }));
            service
                .0
                .process_event(EngineEvent::Abyss(AbyssEvent::Stage {
                    timestamp: (round + 1) as f64 + 0.5,
                    cycle: None,
                    floor: Some(12),
                    half: crate::engine::model::AbyssHalf::First,
                    allow_late_backfill: false,
                }));
        }

        let archives = service.take_pending_abyss_archives();
        assert_eq!(archives.len(), MAX_PENDING_ABYSS_ARCHIVES);
        assert_eq!(archives[0].details.first_half_hits[0].damage, 1.0);
        assert_eq!(
            archives[MAX_PENDING_ABYSS_ARCHIVES - 1]
                .details
                .first_half_hits[0]
                .damage,
            MAX_PENDING_ABYSS_ARCHIVES as f64
        );
        assert_eq!(service.dropped_history_archives(), 4);
    }

    #[test]
    fn restoring_abyss_archive_retries_keeps_queue_bounded() {
        let service = LiveCaptureService::new(LiveCaptureResources::default());
        service
            .0
            .process_event(EngineEvent::Abyss(AbyssEvent::Stage {
                timestamp: 0.0,
                cycle: None,
                floor: Some(12),
                half: crate::engine::model::AbyssHalf::First,
                allow_late_backfill: false,
            }));
        service.0.process_event(hit(321.0));
        service.0.process_event(EngineEvent::CaptureStopped);
        let template = service
            .take_pending_abyss_archives()
            .into_iter()
            .next()
            .expect("template archive");

        service.restore_pending_abyss_archives(vec![template; MAX_PENDING_ABYSS_ARCHIVES + 5]);

        assert_eq!(
            service.take_pending_abyss_archives().len(),
            MAX_PENDING_ABYSS_ARCHIVES
        );
        assert_eq!(service.dropped_history_archives(), 5);
    }

    #[test]
    fn abyss_restart_dedupe_does_not_collide_across_rounds() {
        let service = LiveCaptureService::new(LiveCaptureResources::default());

        // First round: four hits on the first half, then archive it through
        // the same final-archive path used when capture stops.
        service
            .0
            .process_event(EngineEvent::Abyss(AbyssEvent::Stage {
                timestamp: 0.0,
                cycle: None,
                floor: Some(12),
                half: crate::engine::model::AbyssHalf::First,
                allow_late_backfill: false,
            }));
        for _ in 0..4 {
            service.0.process_event(hit(100.0));
        }
        service.0.process_event(EngineEvent::CaptureStopped);

        // Exit then restart resets the Abyss party generations while the
        // global hits generation keeps advancing.
        service
            .0
            .process_event(EngineEvent::Abyss(AbyssEvent::Exit { timestamp: 10.0 }));
        service
            .0
            .process_event(EngineEvent::Abyss(AbyssEvent::RestartDetected {
                timestamp: 11.0,
            }));
        service
            .0
            .process_event(EngineEvent::Abyss(AbyssEvent::Stage {
                timestamp: 12.0,
                cycle: None,
                floor: Some(12),
                half: crate::engine::model::AbyssHalf::First,
                allow_late_backfill: false,
            }));

        // Second round with fewer hits. Its summed generation (6 + 2) equals
        // the first round's summed generation (4 + 4), so a sum-based dedupe
        // would suppress this round; the monotonic global marker does not.
        for _ in 0..2 {
            service.0.process_event(hit(200.0));
        }
        service
            .0
            .process_event(EngineEvent::Abyss(AbyssEvent::Stage {
                timestamp: 20.0,
                cycle: None,
                floor: Some(13),
                half: crate::engine::model::AbyssHalf::First,
                allow_late_backfill: false,
            }));

        let archives = service.take_pending_abyss_archives();
        assert_eq!(archives.len(), 2);
        assert_eq!(archives[0].details.first_half_hits.len(), 4);
        assert_eq!(archives[1].details.first_half_hits.len(), 2);
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
    fn stopping_after_completed_runtime_failure_does_not_stick() {
        let service = LiveCaptureService::new(LiveCaptureResources::default());

        // A running-then-failed lifecycle: an Abyss round with hits, the final
        // CaptureStopped drain barrier, then the runtime Error.
        service
            .0
            .process_event(EngineEvent::Abyss(AbyssEvent::Stage {
                timestamp: 0.0,
                cycle: None,
                floor: Some(12),
                half: crate::engine::model::AbyssHalf::First,
                allow_late_backfill: false,
            }));
        service.0.process_event(hit(100.0));
        service.0.process_event(hit(200.0));
        service.0.process_event(EngineEvent::CaptureStopped);
        service
            .0
            .process_event(EngineEvent::Error("capture failed".to_owned()));

        assert_eq!(service.status().phase, LiveCapturePhase::Failed);

        // The controller no longer owns a running capture, so Stop must
        // recover locally instead of waiting for a CaptureStopped that will
        // never arrive.
        service
            .request_stop()
            .expect("a failed capture must accept stop");

        let deadline = Instant::now() + Duration::from_secs(1);
        while service.status().phase != LiveCapturePhase::Stopped {
            assert!(Instant::now() < deadline, "stop recovery timed out");
            thread::yield_now();
        }

        let archives = service.take_pending_abyss_archives();
        assert_eq!(
            archives.len(),
            1,
            "recovery stop must not create a duplicate Abyss archive"
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
    fn state_and_source_snapshot_uses_one_session_gate() {
        let service = LiveCaptureService::new(LiveCaptureResources::default());
        let mut state = CombatState::default();
        let EngineEvent::Hit(hit) = hit(456.0) else {
            unreachable!("test helper returns a hit")
        };
        state.push_hit(*hit);
        service.restore_session(state, CaptureQualitySource::JsonReplay);

        let (snapshot, source) = service.state_and_source_snapshot();

        assert_eq!(snapshot.total_damage, 456.0);
        assert_eq!(source, CaptureQualitySource::JsonReplay);
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
