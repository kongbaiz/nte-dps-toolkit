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
    panic::{AssertUnwindSafe, catch_unwind},
    path::Path,
    path::PathBuf,
    sync::{
        Arc, Mutex, MutexGuard, Weak,
        atomic::{AtomicBool, AtomicU8, AtomicU64, Ordering},
    },
    thread,
    time::{Duration, Instant},
};

use crossbeam_channel::{Receiver, RecvError, Sender, TryRecvError, bounded, select_biased};

#[cfg(feature = "desktop")]
use super::diagnostics::DiagnosticQualityCache;
use super::{
    CoreError, CoreErrorCode,
    capture::{CaptureController, CaptureControllerOptions},
    history::{HistoryArchivePolicy, PendingHistoryArchive, abyss_event_starts_new_round},
    reducer::{CoreSignal, apply_engine_event},
};
use crate::{
    engine::{
        capture::{
            CaptureImportError, CaptureResources, EngineEventSink, PreparedCaptureJsonReplay,
            RawCaptureSnapshot, import_pcapng, import_prepared_capture_json,
            prepare_capture_json_replay,
        },
        model::{
            AbyssEvent, CaptureQualitySource, CaptureQualitySummary, CharacterInfo,
            CombatClockRuntimeHealth, CombatState, DpsTimeBasis, EngineEvent,
        },
        parser::{AbilityCatalog, CHARACTER_DATA_PATH, load_characters},
    },
    platform::network::NetworkProbeErrorCode,
    storage::{ability_names, history::HistoryCombatDetails, i18n::Language},
};

const RELIABLE_ENGINE_EVENT_CAPACITY: usize = 16_384;
const DEBUG_ENGINE_EVENT_CAPACITY: usize = 2_048;
const MAX_PENDING_ABYSS_ARCHIVES: usize = 64;

#[cfg(all(test, feature = "desktop"))]
std::thread_local! {
    /// Guardrail counter: the incremental quality path must never fall back to
    /// scanning the authoritative hit deque, including after cache poison.
    static DIAGNOSTIC_QUALITY_FULL_REBUILD_COUNT: std::cell::Cell<usize> = const {
        std::cell::Cell::new(0)
    };
}

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
    NetworkProbeDegraded(NetworkProbeErrorCode),
    RuntimeWarning,
    RuntimeError,
    StateUnavailable,
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

/// Adds the lowest-priority runtime issue without replacing a more specific
/// start, probe, or terminal failure. Returns whether the visible status
/// changed and therefore needs a status revision.
fn merge_runtime_warning(issue: &mut Option<LiveCaptureIssue>) -> bool {
    if issue.is_some() {
        return false;
    }
    *issue = Some(LiveCaptureIssue::RuntimeWarning);
    true
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

#[derive(Debug)]
pub enum ReplayStartError {
    Capture(CoreError),
    JsonImport(CaptureImportError),
}

impl From<CoreError> for ReplayStartError {
    fn from(error: CoreError) -> Self {
        Self::Capture(error)
    }
}

/// An authoritative round detached from live capture under `event_gate`.
/// Callers may prepare and persist it after the gate is released; new engine
/// events are already routed into the replacement state.
pub struct CutRound {
    pub state: CombatState,
    pub source: CaptureQualitySource,
    pub dps_time_mode: DpsTimeBasis,
    pub separate_reaction_damage: bool,
}

#[derive(Debug)]
pub struct PendingArchiveRestoreError {
    pub error: CoreError,
    pub archives: Vec<PendingHistoryArchive>,
}

struct DetachedAbyssRound {
    state: CombatState,
    source: CaptureQualitySource,
    dps_time_mode: DpsTimeBasis,
    separate_reaction_damage: bool,
}

const HISTORY_POLICY_SUBTRACT_TIME_STOP: u8 = 1 << 0;
const HISTORY_POLICY_SEPARATE_REACTION_DAMAGE: u8 = 1 << 1;

const fn encode_history_archive_policy(policy: HistoryArchivePolicy) -> u8 {
    let mut encoded = 0;
    if policy.requested_dps_time_mode.subtracts_time_stop() {
        encoded |= HISTORY_POLICY_SUBTRACT_TIME_STOP;
    }
    if policy.separate_reaction_damage {
        encoded |= HISTORY_POLICY_SEPARATE_REACTION_DAMAGE;
    }
    encoded
}

const fn decode_history_archive_policy(encoded: u8) -> HistoryArchivePolicy {
    HistoryArchivePolicy {
        requested_dps_time_mode: DpsTimeBasis::from_subtract_time_stop(
            encoded & HISTORY_POLICY_SUBTRACT_TIME_STOP != 0,
        ),
        separate_reaction_damage: encoded & HISTORY_POLICY_SEPARATE_REACTION_DAMAGE != 0,
    }
}

struct ReplayTask {
    stop: Arc<AtomicBool>,
    thread: thread::JoinHandle<()>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum EventWorkerControl {
    FailClosed,
    Shutdown,
}

struct EventWorkerRuntime {
    receivers: Option<(Receiver<EngineEvent>, Receiver<EngineEvent>)>,
    control: Option<Sender<EventWorkerControl>>,
    worker: Option<thread::JoinHandle<()>>,
}

struct FailureProducers {
    controller: CaptureController,
    replay: Option<ReplayTask>,
}

type IntegrityResult<T> = Result<T, ()>;

fn capture_state_unavailable() -> CoreError {
    CoreError::new(
        CoreErrorCode::CaptureStateUnavailable,
        "live capture state is unavailable",
    )
}

const fn terminal_capture_status() -> LiveCaptureStatus {
    LiveCaptureStatus {
        phase: LiveCapturePhase::Failed,
        issue: Some(LiveCaptureIssue::StateUnavailable),
    }
}

fn checked_lock<T>(mutex: &Mutex<T>) -> IntegrityResult<MutexGuard<'_, T>> {
    mutex.lock().map_err(drop)
}

impl FailureProducers {
    fn stop_and_join(mut self) {
        if let Some(replay) = self.replay.as_ref() {
            replay.stop.store(true, Ordering::Release);
        }
        self.controller.stop_if_running();
        if let Some(replay) = self.replay.take() {
            let _ = replay.thread.join();
        }
    }
}

struct LiveCaptureInner {
    // Session transactions acquire `event_gate` first, then shared session
    // locks in this order: state -> quality_source -> last_outgoing_hit_at ->
    // producer owner (controller/replay) -> status. Cleanup takes owners only
    // after session guards are released and performs stop/join lock-free.
    state: Mutex<CombatState>,
    event_gate: Mutex<()>,
    controller: Mutex<CaptureController>,
    replay: Mutex<Option<ReplayTask>>,
    status: Mutex<LiveCaptureStatus>,
    quality_source: Mutex<CaptureQualitySource>,
    /// Atomically packed future-archive projection policy. Capture event
    /// boundaries load one coherent value without adding another hot lock.
    history_archive_policy: AtomicU8,
    /// Derived, bounded diagnostics read model. This cache is the only lock
    /// acquired before `event_gate`; no capture mutation ever acquires it, so
    /// it cannot participate in a reverse lock-order cycle.
    #[cfg(feature = "desktop")]
    diagnostic_quality: Mutex<DiagnosticQualityCache>,
    revision: AtomicU64,
    packet_revision: AtomicU64,
    packet_session_generation: AtomicU64,
    outgoing_hit_revision: AtomicU64,
    integrity_failed: AtomicBool,
    sender: EngineEventSink,
    event_worker: Mutex<EventWorkerRuntime>,
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
            history_archive_policy: AtomicU8::new(encode_history_archive_policy(
                HistoryArchivePolicy::default(),
            )),
            #[cfg(feature = "desktop")]
            diagnostic_quality: Mutex::new(DiagnosticQualityCache),
            revision: AtomicU64::new(0),
            packet_revision: AtomicU64::new(0),
            packet_session_generation: AtomicU64::new(0),
            outgoing_hit_revision: AtomicU64::new(0),
            integrity_failed: AtomicBool::new(false),
            sender: EngineEventSink::split(reliable_sender, debug_sender),
            event_worker: Mutex::new(EventWorkerRuntime {
                receivers: Some((reliable_receiver, debug_receiver)),
                control: None,
                worker: None,
            }),
            resources,
            last_outgoing_hit_at: Mutex::new(None),
            pending_abyss_archives: Mutex::new(VecDeque::new()),
            dropped_history_archives: AtomicU64::new(0),
        }))
    }

    pub fn status(&self) -> LiveCaptureStatus {
        if self.0.integrity_failed.load(Ordering::Acquire) {
            return terminal_capture_status();
        }
        match self.0.status.lock() {
            Ok(status) => *status,
            Err(error) => {
                drop(error);
                self.0.trip_integrity_failure();
                terminal_capture_status()
            }
        }
    }

    fn checked<T>(
        &self,
        operation: impl FnOnce(&LiveCaptureInner) -> IntegrityResult<T>,
    ) -> Result<T, CoreError> {
        if self.0.integrity_failed.load(Ordering::Acquire) {
            return Err(capture_state_unavailable());
        }
        match operation(&self.0) {
            Ok(value) => Ok(value),
            Err(()) => {
                self.0.trip_integrity_failure();
                Err(capture_state_unavailable())
            }
        }
    }

    fn checked_guard<'a, T>(&self, mutex: &'a Mutex<T>) -> Result<MutexGuard<'a, T>, CoreError> {
        if self.0.integrity_failed.load(Ordering::Acquire) {
            return Err(capture_state_unavailable());
        }
        match mutex.lock() {
            Ok(guard) => Ok(guard),
            Err(error) => {
                drop(error);
                self.0.trip_integrity_failure();
                Err(capture_state_unavailable())
            }
        }
    }

    pub fn replay_running(&self) -> Result<bool, CoreError> {
        self.checked(|inner| Ok(checked_lock(&inner.replay)?.is_some()))
    }

    pub fn active_capture_filter(&self) -> Result<Option<String>, CoreError> {
        self.checked(|inner| Ok(checked_lock(&inner.controller)?.active_filter()))
    }

    pub fn raw_capture_snapshot(&self) -> Result<Option<RawCaptureSnapshot>, CoreError> {
        self.checked(|inner| Ok(checked_lock(&inner.controller)?.raw_capture_snapshot()))
    }

    pub fn save_last_raw_capture(
        &self,
        path: &Path,
    ) -> Result<Result<(u64, u64), String>, CoreError> {
        self.checked(|inner| Ok(checked_lock(&inner.controller)?.save_last_raw_capture(path)))
    }

    pub fn quality_source(&self) -> Result<CaptureQualitySource, CoreError> {
        self.checked(|inner| Ok(*checked_lock(&inner.quality_source)?))
    }

    /// O(1) typed provider health for settings/status contracts. This reads one
    /// Copy field and never clones combat hits or takes the event gate.
    pub fn combat_clock_health(&self) -> Result<CombatClockRuntimeHealth, CoreError> {
        self.with_state(|state| state.combat_clock_health)
    }

    /// Updates the projection policy that future automatic round boundaries
    /// freeze into their pending archive. The two values share one atomic byte,
    /// so a boundary cannot observe a torn settings update.
    pub fn set_history_archive_policy(&self, policy: HistoryArchivePolicy) {
        self.0
            .history_archive_policy
            .store(encode_history_archive_policy(policy), Ordering::Release);
    }

    pub fn history_archive_policy(&self) -> HistoryArchivePolicy {
        decode_history_archive_policy(self.0.history_archive_policy.load(Ordering::Acquire))
    }

    pub fn quality_summary(&self) -> Result<CaptureQualitySummary, CoreError> {
        #[cfg(feature = "desktop")]
        {
            self.cached_quality_summary()
        }
        #[cfg(not(feature = "desktop"))]
        {
            self.with_state_and_source(|state, source| state.capture_quality_summary(source))
        }
    }

    #[cfg(feature = "desktop")]
    fn cached_quality_summary(&self) -> Result<CaptureQualitySummary, CoreError> {
        let mut cache = match self.0.diagnostic_quality.lock() {
            Ok(cache) => cache,
            Err(error) => {
                // This cache is a derived read model with no authoritative
                // state. Resetting it fully restores its only invariant; the
                // next projection rebuilds from the retained hit ring.
                let mut cache = error.into_inner();
                *cache = DiagnosticQualityCache;
                self.0.diagnostic_quality.clear_poison();
                cache
            }
        };
        let input = self.with_state_and_source(|state, source| {
            cache.prepare(
                state,
                source,
                self.0.packet_session_generation.load(Ordering::Acquire),
            )
        })?;
        Ok(cache.finish(input))
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
    pub fn inventory_revision(&self) -> Result<(u64, u64), CoreError> {
        self.with_state(|state| {
            (
                state.empty_curtain_generation,
                state.empty_curtain_characters_generation,
            )
        })
    }

    pub fn with_state<T>(&self, read: impl FnOnce(&CombatState) -> T) -> Result<T, CoreError> {
        self.checked(|inner| {
            let state = checked_lock(&inner.state)?;
            Ok(read(&state))
        })
    }

    /// Reads the authoritative state and its provenance from the same capture
    /// session. Lock order is `event_gate -> state -> quality_source`.
    pub fn with_state_and_source<T>(
        &self,
        read: impl FnOnce(&CombatState, CaptureQualitySource) -> T,
    ) -> Result<T, CoreError> {
        self.checked(|inner| {
            let _gate = checked_lock(&inner.event_gate)?;
            let state = checked_lock(&inner.state)?;
            let source = *checked_lock(&inner.quality_source)?;
            Ok(read(&state, source))
        })
    }

    /// Clones one low-frequency, provenance-frozen capture snapshot for work
    /// that must continue after the capture locks are released.
    pub fn state_and_source_snapshot(
        &self,
    ) -> Result<(CombatState, CaptureQualitySource), CoreError> {
        self.with_state_and_source(|state, source| (state.clone(), source))
    }

    /// Freezes only the authoritative fields consumed by History. The clone is
    /// made while provenance is protected by the same event gate, then all
    /// summary projection and disk work happens after the hot locks are gone.
    pub fn history_state_and_source_snapshot(
        &self,
    ) -> Result<(CombatState, CaptureQualitySource), CoreError> {
        self.with_state_and_source(|state, source| (state.clone_for_history_archive(), source))
    }

    pub fn with_packet_state<T>(
        &self,
        read: impl FnOnce(super::packets::PacketStreamRevision, usize, &CombatState) -> T,
    ) -> Result<T, CoreError> {
        self.checked(|inner| {
            let _gate = checked_lock(&inner.event_gate)?;
            let state = checked_lock(&inner.state)?;
            let revision = super::packets::PacketStreamRevision {
                generation: inner.packet_revision.load(Ordering::Acquire),
                session_generation: inner.packet_session_generation.load(Ordering::Acquire),
                packet_generation: state.packets_generation,
                observed_packet_count: state.packet_count,
            };
            Ok(read(revision, inner.sender.pending_len(), &state))
        })
    }

    pub fn resources(&self) -> LiveCaptureResources {
        self.0.resources.clone()
    }

    pub fn idle_elapsed(&self) -> Result<Option<Duration>, CoreError> {
        self.checked(|inner| {
            Ok(checked_lock(&inner.last_outgoing_hit_at)?.map(|last| last.elapsed()))
        })
    }

    pub fn take_pending_abyss_archives(&self) -> Result<Vec<PendingHistoryArchive>, CoreError> {
        // This queue is durability salvage, not authoritative capture state.
        // It remains drainable after a sticky capture failure so already-cut
        // rounds can still reach storage while every capture projection and
        // mutation stays fail-closed.
        Ok(self.pending_archives_for_cleanup().drain(..).collect())
    }

    pub fn restore_pending_abyss_archives(
        &self,
        archives: Vec<PendingHistoryArchive>,
    ) -> Result<(), PendingArchiveRestoreError> {
        // Like `take_pending_abyss_archives`, retry restoration is a cleanup
        // operation and remains available after the sticky terminal failure.
        // This prevents a persistence failure racing capture poison from
        // dropping the retry payload at the adapter boundary.
        let mut pending = self.pending_archives_for_cleanup();
        let mut dropped = 0_usize;
        for archive in archives.into_iter().rev() {
            if pending.len() >= MAX_PENDING_ABYSS_ARCHIVES {
                pending.pop_back();
                dropped += 1;
            }
            pending.push_front(archive);
        }
        drop(pending);
        if dropped > 0 {
            if self.0.integrity_failed.load(Ordering::Acquire) {
                self.0
                    .dropped_history_archives
                    .fetch_add(dropped as u64, Ordering::AcqRel);
                self.0.bump_revision();
            } else if self
                .0
                .note_dropped_history_archives(dropped as u64)
                .is_err()
            {
                self.0.trip_integrity_failure();
            }
            eprintln!(
                "automatic Abyss History retry queue exceeded {MAX_PENDING_ABYSS_ARCHIVES} entries; discarded {dropped} newest archive(s) while restoring older failed retries"
            );
        }
        Ok(())
    }

    fn pending_archives_for_cleanup(&self) -> MutexGuard<'_, VecDeque<PendingHistoryArchive>> {
        loop {
            match self.0.pending_abyss_archives.lock() {
                Ok(pending) => return pending,
                Err(error) => {
                    // `VecDeque` remains structurally valid across unwind. Its
                    // only service invariant is the explicit capacity bound,
                    // which cleanup repairs before clearing poison. No capture
                    // producer is stopped or joined while this guard is held.
                    let mut pending = error.into_inner();
                    let mut dropped = 0_u64;
                    while pending.len() > MAX_PENDING_ABYSS_ARCHIVES {
                        pending.pop_back();
                        dropped += 1;
                    }
                    self.0.pending_abyss_archives.clear_poison();
                    drop(pending);
                    if dropped > 0 {
                        self.0
                            .dropped_history_archives
                            .fetch_add(dropped, Ordering::AcqRel);
                    }
                    self.0.trip_integrity_failure();
                }
            }
        }
    }

    pub fn dropped_history_archives(&self) -> u64 {
        self.0.dropped_history_archives.load(Ordering::Acquire)
    }

    /// Atomically installs a fresh combat state and returns the detached round.
    ///
    /// Lock order is `event_gate -> state -> quality_source -> round runtime`.
    /// No serialization or persistence is allowed in this critical section.
    pub fn cut_round(&self) -> Result<Option<CutRound>, CoreError> {
        self.cut_round_with_history_policy(self.history_archive_policy())
    }

    /// Variant for adapters that already hold the current settings snapshot.
    /// Passing the policy into the cut transaction removes any interval in
    /// which a later config read could rewrite the detached round's meaning.
    pub fn cut_round_with_history_policy(
        &self,
        policy: HistoryArchivePolicy,
    ) -> Result<Option<CutRound>, CoreError> {
        self.checked(|inner| {
            let _gate = checked_lock(&inner.event_gate)?;
            let mut state = checked_lock(&inner.state)?;
            let source = *checked_lock(&inner.quality_source)?;
            let mut idle_timer = checked_lock(&inner.last_outgoing_hit_at)?;
            if state.hits.is_empty() && state.stats.is_empty() && !state.abyss.is_active() {
                return Ok(None);
            }
            let detached = state.take_battle_preserving_inventory();
            let dps_time_mode = policy.effective_for(&detached);
            *idle_timer = None;
            inner.bump_packet_session();
            inner.bump_revision();
            Ok(Some(CutRound {
                state: detached,
                source,
                dps_time_mode,
                separate_reaction_damage: policy.separate_reaction_damage,
            }))
        })
    }

    /// Clears the current combat projection while keeping capture resources and
    /// the active capture controller intact.
    pub fn reset_session(&self) -> Result<(), CoreError> {
        self.checked(|inner| {
            let _gate = checked_lock(&inner.event_gate)?;
            let mut state = checked_lock(&inner.state)?;
            let mut idle_timer = checked_lock(&inner.last_outgoing_hit_at)?;
            let combat_clock_health = state.combat_clock_health;
            *state = CombatState::default();
            // The monitor publishes health only on transitions. Resetting a
            // combat round must therefore preserve the capture-session
            // provider state or an unchanged Available provider would never
            // repopulate it.
            state.combat_clock_health = combat_clock_health;
            *idle_timer = None;
            inner.bump_packet_session();
            inner.bump_revision();
            Ok(())
        })
    }

    /// Restores a previously reset session while keeping the capture service
    /// and its frontend-neutral resources intact.
    pub fn restore_session(
        &self,
        state: CombatState,
        quality_source: CaptureQualitySource,
    ) -> Result<(), CoreError> {
        self.checked(|inner| {
            let _gate = checked_lock(&inner.event_gate)?;
            let mut current = checked_lock(&inner.state)?;
            let mut source = checked_lock(&inner.quality_source)?;
            let mut idle_timer = checked_lock(&inner.last_outgoing_hit_at)?;
            let has_outgoing = state.hits.iter().any(|hit| hit.direction.is_outgoing());
            *current = state;
            *source = quality_source;
            *idle_timer = has_outgoing.then_some(Instant::now());
            inner.bump_packet_session();
            inner.bump_revision();
            Ok(())
        })
    }

    pub fn request_start(&self, options: CaptureControllerOptions) -> Result<(), CoreError> {
        self.ensure_event_worker()?;
        {
            let mut status = self.checked_guard(&self.0.status)?;
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
    ) -> Result<(), ReplayStartError> {
        match kind {
            CaptureReplayKind::Pcapng => self
                .request_pcapng_replay(
                    path,
                    local_ip_hint,
                    include_incoming,
                    server_damage_calibration,
                )
                .map_err(ReplayStartError::Capture),
            CaptureReplayKind::Json => {
                let prepared =
                    prepare_capture_json_replay(&path).map_err(ReplayStartError::JsonImport)?;
                self.request_prepared_json_replay(prepared)
                    .map_err(ReplayStartError::Capture)
            }
        }
    }

    fn request_pcapng_replay(
        &self,
        path: PathBuf,
        local_ip_hint: Option<Ipv4Addr>,
        include_incoming: bool,
        server_damage_calibration: bool,
    ) -> Result<(), CoreError> {
        let resources = CaptureResources {
            characters: Arc::clone(&self.0.resources.characters),
            ability_catalog: Arc::clone(&self.0.resources.ability_catalog),
        };
        self.start_replay(CaptureQualitySource::PcapngReplay, move |sender, stop| {
            import_pcapng(
                path,
                resources,
                local_ip_hint,
                include_incoming,
                server_damage_calibration,
                sender,
                stop,
            )
        })
    }

    pub fn request_prepared_json_replay(
        &self,
        prepared: PreparedCaptureJsonReplay,
    ) -> Result<(), CoreError> {
        self.start_replay(CaptureQualitySource::JsonReplay, move |sender, stop| {
            import_prepared_capture_json(prepared, sender, stop)
        })
    }

    fn start_replay(
        &self,
        source_kind: CaptureQualitySource,
        spawn: impl FnOnce(EngineEventSink, Arc<AtomicBool>) -> std::io::Result<thread::JoinHandle<()>>,
    ) -> Result<(), CoreError> {
        self.ensure_event_worker()?;
        let stop = Arc::new(AtomicBool::new(false));
        let (sender, delivery_permit) = self.0.sender.clone().pause_delivery();
        let thread = spawn(sender, Arc::clone(&stop))
            .map_err(|error| CoreError::new(CoreErrorCode::SystemProbeFailed, error.to_string()))?;
        let mut staged = Some(ReplayTask { stop, thread });
        let committed = self.checked(|inner| {
            // Spawn/file setup occurs before this critical section. The event
            // sink barrier prevents the staged producer from applying anything
            // until owner, source, state, timer, and status commit together.
            let _gate = checked_lock(&inner.event_gate)?;
            let mut state = checked_lock(&inner.state)?;
            let mut source = checked_lock(&inner.quality_source)?;
            let mut idle_timer = checked_lock(&inner.last_outgoing_hit_at)?;
            let mut replay = checked_lock(&inner.replay)?;
            let mut status = checked_lock(&inner.status)?;
            if inner.integrity_failed.load(Ordering::Acquire) {
                return Err(());
            }
            if matches!(
                status.phase,
                LiveCapturePhase::Starting | LiveCapturePhase::Running | LiveCapturePhase::Stopping
            ) || replay.is_some()
            {
                return Ok(false);
            }
            let Some(task) = staged.take() else {
                return Err(());
            };
            *state = CombatState::default();
            *idle_timer = None;
            *replay = Some(task);
            *source = source_kind;
            *status = LiveCaptureStatus {
                phase: LiveCapturePhase::Running,
                issue: None,
            };
            inner.bump_packet_session();
            inner.bump_revision();
            Ok(true)
        });

        match committed {
            Ok(true) => {
                delivery_permit.release();
                Ok(())
            }
            Ok(false) => {
                if let Some(task) = staged.take() {
                    task.stop.store(true, Ordering::Release);
                    delivery_permit.cancel();
                    let _ = task.thread.join();
                }
                Err(CoreError::new(
                    CoreErrorCode::CaptureAlreadyRunning,
                    "capture or replay is already active",
                ))
            }
            Err(error) => {
                if let Some(task) = staged.take() {
                    task.stop.store(true, Ordering::Release);
                    delivery_permit.cancel();
                    let _ = task.thread.join();
                }
                Err(error)
            }
        }
    }

    pub fn request_stop(&self) -> Result<(), CoreError> {
        let replay_stop = self
            .checked_guard(&self.0.replay)?
            .as_ref()
            .map(|replay| Arc::clone(&replay.stop));
        let should_spawn = {
            let mut status = self.checked_guard(&self.0.status)?;
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
        let mut runtime = self.checked_guard(&self.0.event_worker)?;
        if let Some(worker) = runtime.worker.as_ref() {
            if !worker.is_finished() {
                return Ok(());
            }
            drop(runtime);
            self.0.trip_integrity_failure();
            return Err(capture_state_unavailable());
        }
        let Some((reliable_receiver, debug_receiver)) = runtime.receivers.as_ref() else {
            drop(runtime);
            self.0.trip_integrity_failure();
            return Err(capture_state_unavailable());
        };
        let worker_reliable_receiver = reliable_receiver.clone();
        let worker_debug_receiver = debug_receiver.clone();
        let (control_sender, control_receiver) = bounded(4);
        let weak = Arc::downgrade(&self.0);
        match thread::Builder::new()
            .name("nte-live-engine-events".to_owned())
            .spawn(move || {
                engine_event_loop(
                    weak,
                    worker_reliable_receiver,
                    worker_debug_receiver,
                    control_receiver,
                )
            }) {
            Ok(worker) => {
                runtime.receivers = None;
                runtime.control = Some(control_sender);
                runtime.worker = Some(worker);
                Ok(())
            }
            Err(error) => Err(CoreError::new(
                CoreErrorCode::SystemProbeFailed,
                error.to_string(),
            )),
        }
    }

    fn finish_start(&self, options: CaptureControllerOptions) {
        if self.0.integrity_failed.load(Ordering::Acquire) {
            return;
        }
        let mut controller = match self
            .checked(|inner| Ok(std::mem::take(&mut *checked_lock(&inner.controller)?)))
        {
            Ok(controller) => controller,
            Err(_) => return,
        };
        let (sender, delivery_permit) = self.0.sender.clone().pause_delivery();
        let result = controller.start(
            options,
            Arc::clone(&self.0.resources.characters),
            Arc::clone(&self.0.resources.ability_catalog),
            sender,
        );

        match result {
            Ok(()) => {
                let network_probe_degradation = controller
                    .network_probe_degradation()
                    .map(|failure| failure.code);
                let mut staged_controller = Some(controller);
                let committed = self.checked(|inner| {
                    // All fallible locks are acquired before any session field
                    // changes. The producer is already running, but its sink is
                    // paused, so no event can cross this transaction boundary.
                    let _gate = checked_lock(&inner.event_gate)?;
                    let mut state = checked_lock(&inner.state)?;
                    let mut source = checked_lock(&inner.quality_source)?;
                    let mut idle_timer = checked_lock(&inner.last_outgoing_hit_at)?;
                    let mut controller = checked_lock(&inner.controller)?;
                    let mut status = checked_lock(&inner.status)?;
                    if !matches!(
                        status.phase,
                        LiveCapturePhase::Starting | LiveCapturePhase::Stopping
                    ) {
                        return Err(());
                    }
                    let Some(ready_controller) = staged_controller.take() else {
                        return Err(());
                    };
                    let should_stop = status.phase == LiveCapturePhase::Stopping;
                    *state = CombatState::default();
                    *source = CaptureQualitySource::Live;
                    *controller = ready_controller;
                    *idle_timer = None;
                    if !should_stop {
                        *status = LiveCaptureStatus {
                            phase: LiveCapturePhase::Running,
                            issue: network_probe_degradation
                                .map(LiveCaptureIssue::NetworkProbeDegraded),
                        };
                    }
                    // This one commit changes the authoritative session and its
                    // visible lifecycle exactly once per affected revision.
                    inner.bump_packet_session();
                    inner.bump_revision();
                    Ok(should_stop)
                });
                let Ok(should_stop) = committed else {
                    delivery_permit.cancel();
                    if let Some(mut controller) = staged_controller {
                        controller.stop_if_running();
                    }
                    return;
                };
                delivery_permit.release();
                if should_stop && self.spawn_stop_worker().is_err() {
                    self.record_start_failure(CoreErrorCode::SystemProbeFailed);
                }
            }
            Err(error) => {
                delivery_permit.cancel();
                let restored = self.checked(|inner| {
                    *checked_lock(&inner.controller)? = controller;
                    Ok(())
                });
                if restored.is_ok() {
                    self.record_start_failure(error.code);
                }
            }
        }
    }

    /// Publishes both the running phase and any manual-network degradation as
    /// one status mutation. Revision effect: exactly one capture/status
    /// revision and one packet revision advance immediately after this critical
    /// section, so consumers cannot observe Running without its warning.
    #[cfg(test)]
    fn publish_start_success(
        &self,
        network_probe_degradation: Option<NetworkProbeErrorCode>,
    ) -> Result<bool, CoreError> {
        let should_stop = {
            let mut status = self.checked_guard(&self.0.status)?;
            if status.phase == LiveCapturePhase::Stopping {
                true
            } else {
                *status = LiveCaptureStatus {
                    phase: LiveCapturePhase::Running,
                    issue: network_probe_degradation.map(LiveCaptureIssue::NetworkProbeDegraded),
                };
                false
            }
        };
        self.0.bump_capture_status_revision();
        Ok(should_stop)
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
        let mut controller = match self.checked_guard(&self.0.controller) {
            Ok(mut controller) => std::mem::take(&mut *controller),
            Err(_) => return,
        };

        if controller.is_running() {
            if let Err(error) = controller.stop() {
                let restored = self.checked(|inner| {
                    *checked_lock(&inner.controller)? = controller;
                    Ok(())
                });
                if restored.is_ok() {
                    self.record_start_failure(error.code);
                }
                return;
            }
            if self.0.integrity_failed.load(Ordering::Acquire) {
                return;
            }
            let mut slot = match self.checked_guard(&self.0.controller) {
                Ok(slot) => slot,
                Err(_) => return,
            };
            *slot = controller;
            return;
        }

        let changed = {
            let mut status = match self.checked_guard(&self.0.status) {
                Ok(status) => status,
                Err(_) => return,
            };

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
        if self.0.integrity_failed.load(Ordering::Acquire) {
            return;
        }
        let mut status = match self.checked_guard(&self.0.status) {
            Ok(status) => status,
            Err(_) => return,
        };
        *status = LiveCaptureStatus {
            phase: LiveCapturePhase::Failed,
            issue: Some(LiveCaptureIssue::Start(code)),
        };
        drop(status);
        self.0.bump_capture_status_revision();
    }
}

impl LiveCaptureInner {
    fn trip_integrity_failure(self: &Arc<Self>) {
        if self
            .integrity_failed
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .is_err()
        {
            return;
        }

        match self.status.lock() {
            Ok(mut status) => *status = terminal_capture_status(),
            Err(mut error) => {
                **error.get_mut() = terminal_capture_status();
                self.status.clear_poison();
            }
        }
        // One terminal state mutation invalidates both general and packet
        // projections. Outgoing-hit revision is deliberately untouched.
        self.revision.fetch_add(1, Ordering::AcqRel);
        self.packet_session_generation
            .fetch_add(1, Ordering::AcqRel);
        self.packet_revision.fetch_add(1, Ordering::AcqRel);

        let control = match self.event_worker.lock() {
            Ok(runtime) => runtime.control.clone(),
            Err(mut error) => {
                let control = error.get_mut().control.clone();
                self.event_worker.clear_poison();
                control
            }
        };
        if let Some(control) = control {
            let _ = control.try_send(EventWorkerControl::FailClosed);
        } else {
            // `ensure_event_worker` precedes every producer start. With no
            // worker there should be no producer, but destructive cleanup is
            // still idempotent and closes a corrupted lifecycle safely.
            self.take_failure_producers().stop_and_join();
        }
    }

    fn take_failure_producers(&self) -> FailureProducers {
        let controller = match self.controller.lock() {
            Ok(mut controller) => std::mem::take(&mut *controller),
            Err(mut error) => {
                let controller = std::mem::take(&mut **error.get_mut());
                self.controller.clear_poison();
                controller
            }
        };
        let replay = match self.replay.lock() {
            Ok(mut replay) => replay.take(),
            Err(mut error) => {
                let replay = error.get_mut().take();
                self.replay.clear_poison();
                replay
            }
        };
        FailureProducers { controller, replay }
    }

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

    fn note_dropped_history_archives(&self, count: u64) -> IntegrityResult<()> {
        self.dropped_history_archives
            .fetch_add(count, Ordering::AcqRel);
        let mut status = checked_lock(&self.status)?;
        merge_runtime_warning(&mut status.issue);
        drop(status);
        // The dropped archive counter changed even when a higher-priority
        // status issue remains visible.
        self.bump_revision();
        Ok(())
    }

    /// Detaches the current Abyss round only when its hit content changed since
    /// the last archive. The expensive HistoryCombatDetails conversion happens
    /// after the event gate and state lock are released.
    ///
    /// Taking the state makes the boundary itself the dedupe barrier: after a
    /// round is detached, the replacement state contains no old Abyss hits.
    fn detach_abyss_round_if_changed(
        &self,
        state: &mut CombatState,
        source: CaptureQualitySource,
        policy: HistoryArchivePolicy,
        idle_timer: &mut Option<Instant>,
    ) -> Option<DetachedAbyssRound> {
        let has_abyss_hits =
            !state.abyss.first_half.hits.is_empty() || !state.abyss.second_half.hits.is_empty();
        if !has_abyss_hits {
            return None;
        }
        let detached = state.take_battle_preserving_inventory();
        let dps_time_mode = policy.effective_for(&detached);
        *idle_timer = None;
        Some(DetachedAbyssRound {
            state: detached,
            source,
            dps_time_mode,
            separate_reaction_damage: policy.separate_reaction_damage,
        })
    }

    fn queue_detached_abyss_round(&self, detached: DetachedAbyssRound) -> IntegrityResult<()> {
        let Some(summary) = detached.state.session_summary(
            detached.source,
            detached.dps_time_mode,
            detached.separate_reaction_damage,
        ) else {
            return Ok(());
        };
        let Some(details) = HistoryCombatDetails::from_state_owned(detached.state) else {
            return Ok(());
        };
        let mut pending = checked_lock(&self.pending_abyss_archives)?;
        if pending.len() >= MAX_PENDING_ABYSS_ARCHIVES {
            drop(pending);
            self.note_dropped_history_archives(1)?;
            eprintln!(
                "automatic Abyss History retry queue is full at {MAX_PENDING_ABYSS_ARCHIVES} entries; newest round was not queued"
            );
            return Ok(());
        }
        pending.push_back(PendingHistoryArchive {
            summary,
            details,
            source: detached.source,
            dps_time_mode: detached.dps_time_mode,
            separate_reaction_damage: detached.separate_reaction_damage,
        });
        Ok(())
    }

    fn process_event(&self, event: EngineEvent) -> IntegrityResult<()> {
        if self.integrity_failed.load(Ordering::Acquire) {
            return Err(());
        }
        let (signal, detached_abyss_round) = {
            let _gate = checked_lock(&self.event_gate)?;
            let mut state = checked_lock(&self.state)?;
            let pre_event_abyss_boundary = matches!(
                &event,
                EngineEvent::Abyss(abyss)
                    if abyss_event_starts_new_round(state.abyss.floor, abyss)
            );
            let post_event_abyss_archive = matches!(&event, EngineEvent::CaptureStopped)
                || matches!(&event, EngineEvent::Abyss(AbyssEvent::Exit { .. }));
            let outgoing_hit = matches!(
                &event,
                EngineEvent::Hit(hit) if hit.direction.is_outgoing()
            );
            let boundary_context = pre_event_abyss_boundary || post_event_abyss_archive;
            let history_policy = boundary_context.then(|| {
                decode_history_archive_policy(self.history_archive_policy.load(Ordering::Acquire))
            });
            let source = if boundary_context {
                Some(*checked_lock(&self.quality_source)?)
            } else {
                None
            };
            let mut idle_timer = if boundary_context || outgoing_hit {
                Some(checked_lock(&self.last_outgoing_hit_at)?)
            } else {
                None
            };
            let mut detached_abyss_round = if pre_event_abyss_boundary {
                let (Some(source), Some(idle_timer)) = (source, idle_timer.as_deref_mut()) else {
                    return Err(());
                };
                let Some(policy) = history_policy else {
                    return Err(());
                };
                self.detach_abyss_round_if_changed(&mut state, source, policy, idle_timer)
            } else {
                None
            };
            let signal = apply_engine_event(&mut state, event);
            if post_event_abyss_archive && detached_abyss_round.is_none() {
                let (Some(source), Some(idle_timer)) = (source, idle_timer.as_deref_mut()) else {
                    return Err(());
                };
                let Some(policy) = history_policy else {
                    return Err(());
                };
                detached_abyss_round =
                    self.detach_abyss_round_if_changed(&mut state, source, policy, idle_timer);
            }
            if outgoing_hit {
                let Some(idle_timer) = idle_timer.as_mut() else {
                    return Err(());
                };
                **idle_timer = Some(Instant::now());
                self.outgoing_hit_revision.fetch_add(1, Ordering::AcqRel);
            }
            if detached_abyss_round.is_some() {
                self.bump_packet_session();
            }
            (signal, detached_abyss_round)
        };
        if let Some(detached_abyss_round) = detached_abyss_round {
            self.queue_detached_abyss_round(detached_abyss_round)?;
        }

        let affects_packet_projection = matches!(
            &signal,
            CoreSignal::DebugPacket
                | CoreSignal::PacketObserved
                | CoreSignal::Error(_)
                | CoreSignal::CaptureStopped
        );
        let affects_frontend_projection = match signal {
            CoreSignal::Unchanged => false,
            CoreSignal::StateChanged => true,
            CoreSignal::CombatClockHealthChanged => true,
            CoreSignal::InventoryReplaced
            | CoreSignal::InventoryCharactersReplaced
            | CoreSignal::DebugPacket
            | CoreSignal::PacketObserved => false,
            CoreSignal::ModScript { state_changed, .. } => state_changed,
            CoreSignal::Status(_) => {
                let mut status = checked_lock(&self.status)?;
                if status.phase == LiveCapturePhase::Starting {
                    status.phase = LiveCapturePhase::Running;
                    true
                } else {
                    false
                }
            }
            CoreSignal::Warning(_) => {
                let mut status = checked_lock(&self.status)?;
                merge_runtime_warning(&mut status.issue)
            }
            CoreSignal::Error(_) => {
                *checked_lock(&self.status)? = LiveCaptureStatus {
                    phase: LiveCapturePhase::Failed,
                    issue: Some(LiveCaptureIssue::RuntimeError),
                };
                true
            }
            CoreSignal::CaptureStopped => {
                let mut controller = {
                    let mut slot = checked_lock(&self.controller)?;
                    std::mem::take(&mut *slot)
                };
                controller.capture_stopped();
                *checked_lock(&self.controller)? = controller;
                let replay = checked_lock(&self.replay)?.take();
                if let Some(replay) = replay {
                    let _ = replay.thread.join();
                }
                let mut status = checked_lock(&self.status)?;
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
        Ok(())
    }
}

impl Drop for LiveCaptureInner {
    fn drop(&mut self) {
        let runtime = match self.event_worker.get_mut() {
            Ok(runtime) => runtime,
            Err(error) => error.into_inner(),
        };
        let dropping_on_event_worker = runtime
            .worker
            .as_ref()
            .is_some_and(|worker| worker.thread().id() == thread::current().id());
        if let Some(control) = runtime.control.take() {
            let _ = control.try_send(EventWorkerControl::Shutdown);
        }
        if let Some(worker) = runtime.worker.take()
            && !dropping_on_event_worker
        {
            let _ = worker.join();
        }

        let replay = match self.replay.get_mut() {
            Ok(replay) => replay.take(),
            Err(error) => error.into_inner().take(),
        };
        let controller = match self.controller.get_mut() {
            Ok(controller) => std::mem::take(controller),
            Err(error) => std::mem::take(error.into_inner()),
        };
        let producers = FailureProducers { controller, replay };
        if dropping_on_event_worker {
            // The worker can transiently own the last strong Arc while applying
            // an event. Joining itself would deadlock, and joining a producer
            // here could block behind the worker's still-live receivers. Move
            // cleanup to a detached owner; the event loop drops its receivers
            // immediately after this destructor returns.
            let cleanup = Arc::new(Mutex::new(Some(producers)));
            let worker_cleanup = Arc::clone(&cleanup);
            if thread::Builder::new()
                .name("nte-live-capture-drop-cleanup".to_owned())
                .spawn(move || {
                    let producers = worker_cleanup
                        .lock()
                        .ok()
                        .and_then(|mut cleanup| cleanup.take());
                    if let Some(producers) = producers {
                        producers.stop_and_join();
                    }
                })
                .is_err()
                && let Ok(mut cleanup) = cleanup.lock()
                && let Some(producers) = cleanup.take()
            {
                // Thread creation failure leaves no safe synchronous join path
                // on the event worker. Leaking only this terminal owner avoids
                // a process-wide deadlock; channel disconnect still makes its
                // producers observe loss of their consumer.
                std::mem::forget(producers);
            }
        } else {
            producers.stop_and_join();
        }
    }
}

// Engine events are the high-frequency path. Keeping them inline avoids one
// heap allocation per packet/hit; the control variant is intentionally tiny.
#[allow(clippy::large_enum_variant)]
enum EngineLoopInput {
    Event(EngineEvent),
    Control(EventWorkerControl),
}

fn engine_event_loop(
    inner: Weak<LiveCaptureInner>,
    reliable_receiver: Receiver<EngineEvent>,
    debug_receiver: Receiver<EngineEvent>,
    control_receiver: Receiver<EventWorkerControl>,
) {
    loop {
        let Some(strong) = inner.upgrade() else {
            break;
        };
        if Arc::strong_count(&strong) == 1 {
            drain_failed_runtime(strong, reliable_receiver, debug_receiver, control_receiver);
            return;
        }
        if strong.integrity_failed.load(Ordering::Acquire) {
            drain_failed_runtime(strong, reliable_receiver, debug_receiver, control_receiver);
            return;
        }
        let defer_debug = match checked_lock(&strong.replay).map(|replay| replay.is_some()) {
            Ok(defer_debug) => defer_debug,
            Err(()) => {
                strong.trip_integrity_failure();
                drain_failed_runtime(strong, reliable_receiver, debug_receiver, control_receiver);
                return;
            }
        };
        drop(strong);
        let input = match next_engine_input(
            &reliable_receiver,
            &debug_receiver,
            &control_receiver,
            defer_debug,
        ) {
            Ok(input) => input,
            Err(_) => break,
        };
        match input {
            EngineLoopInput::Control(EventWorkerControl::Shutdown) => break,
            EngineLoopInput::Control(EventWorkerControl::FailClosed) => {
                let Some(inner) = inner.upgrade() else {
                    break;
                };
                drain_failed_runtime(inner, reliable_receiver, debug_receiver, control_receiver);
                return;
            }
            EngineLoopInput::Event(event) => {
                let Some(inner) = inner.upgrade() else {
                    break;
                };
                let result = catch_unwind(AssertUnwindSafe(|| inner.process_event(event)));
                if !matches!(result, Ok(Ok(()))) {
                    inner.trip_integrity_failure();
                    drain_failed_runtime(
                        inner,
                        reliable_receiver,
                        debug_receiver,
                        control_receiver,
                    );
                    return;
                }
                if Arc::strong_count(&inner) == 1 {
                    drain_failed_runtime(
                        inner,
                        reliable_receiver,
                        debug_receiver,
                        control_receiver,
                    );
                    return;
                }
            }
        }
    }
}

fn drain_failed_runtime(
    inner: Arc<LiveCaptureInner>,
    reliable_receiver: Receiver<EngineEvent>,
    debug_receiver: Receiver<EngineEvent>,
    control_receiver: Receiver<EventWorkerControl>,
) {
    let producers = Arc::new(Mutex::new(Some(inner.take_failure_producers())));
    let worker_producers = Arc::clone(&producers);
    let cleanup = thread::Builder::new()
        .name("nte-live-capture-fail-closed".to_owned())
        .spawn(move || {
            let producers = worker_producers
                .lock()
                .ok()
                .and_then(|mut producers| producers.take());
            if let Some(producers) = producers {
                producers.stop_and_join();
            }
        });

    match cleanup {
        Ok(cleanup) => {
            while !cleanup.is_finished() {
                while reliable_receiver.try_recv().is_ok() {}
                while debug_receiver.try_recv().is_ok() {}
                let _ = control_receiver.recv_timeout(Duration::from_millis(1));
            }
            let _ = cleanup.join();
        }
        Err(_) => {
            // Dropping the last receivers makes every blocked reliable send
            // return Disconnected before synchronous producer joins begin.
            drop(reliable_receiver);
            drop(debug_receiver);
            if let Ok(mut producers) = producers.lock()
                && let Some(producers) = producers.take()
            {
                producers.stop_and_join();
            }
        }
    }
}

/// Preserve the established reliable-first replay behavior. Semantic events
/// and `CaptureStopped` must not wait behind thousands of optional full packet
/// payloads; the bounded debug lane is intentionally allowed to fill and drop
/// those payloads while a replay burst is still producing authoritative data.
fn next_engine_input(
    reliable_receiver: &Receiver<EngineEvent>,
    debug_receiver: &Receiver<EngineEvent>,
    control_receiver: &Receiver<EventWorkerControl>,
    defer_debug: bool,
) -> Result<EngineLoopInput, RecvError> {
    if let Ok(control) = control_receiver.try_recv() {
        return Ok(EngineLoopInput::Control(control));
    }
    if defer_debug {
        return select_biased! {
            recv(control_receiver) -> control => control.map(EngineLoopInput::Control).map_err(|_| RecvError),
            recv(reliable_receiver) -> event => event.map(EngineLoopInput::Event),
        };
    }
    match reliable_receiver.try_recv() {
        Ok(event) => return Ok(EngineLoopInput::Event(event)),
        Err(TryRecvError::Disconnected) => {
            return debug_receiver.recv().map(EngineLoopInput::Event);
        }
        Err(TryRecvError::Empty) => {}
    }
    match debug_receiver.try_recv() {
        Ok(event) => return Ok(EngineLoopInput::Event(event)),
        Err(TryRecvError::Disconnected) => {
            return reliable_receiver.recv().map(EngineLoopInput::Event);
        }
        Err(TryRecvError::Empty) => {}
    }
    select_biased! {
        recv(control_receiver) -> control => control.map(EngineLoopInput::Control).map_err(|_| RecvError),
        recv(reliable_receiver) -> event => event.map(EngineLoopInput::Event),
        recv(debug_receiver) -> event => event.map(EngineLoopInput::Event),
    }
}

#[cfg(test)]
fn next_engine_event(
    reliable_receiver: &Receiver<EngineEvent>,
    debug_receiver: &Receiver<EngineEvent>,
    defer_debug: bool,
) -> Result<EngineEvent, RecvError> {
    let (_control_sender, control_receiver) = bounded(1);
    match next_engine_input(
        reliable_receiver,
        debug_receiver,
        &control_receiver,
        defer_debug,
    )? {
        EngineLoopInput::Event(event) => Ok(event),
        EngineLoopInput::Control(_) => Err(RecvError),
    }
}

#[cfg(test)]
mod tests {
    use std::{
        panic::{AssertUnwindSafe, catch_unwind},
        time::{Duration, Instant},
    };

    use super::*;
    use crate::core::packets::PacketStreamRevision;
    use crate::engine::model::{
        AbyssEvent, EmptyCurtainCharacter, EmptyCurtainItem, Hit, HitCharacterSource, HitDirection,
        HtItemNetId, PacketObservation,
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

    fn process(service: &LiveCaptureService, event: EngineEvent) {
        service
            .0
            .process_event(event)
            .expect("healthy live-capture runtime");
    }

    #[cfg(feature = "desktop")]
    fn reset_diagnostic_quality_full_rebuild_count() {
        DIAGNOSTIC_QUALITY_FULL_REBUILD_COUNT.with(|count| count.set(0));
    }

    #[cfg(feature = "desktop")]
    fn diagnostic_quality_full_rebuild_count() -> usize {
        DIAGNOSTIC_QUALITY_FULL_REBUILD_COUNT.with(std::cell::Cell::get)
    }

    fn project<T>(service: &LiveCaptureService, projection: impl FnOnce(&CombatState) -> T) -> T {
        service
            .with_state(projection)
            .expect("healthy live-capture state")
    }

    #[test]
    fn status_heartbeat_only_advances_revision_for_starting_to_running_transition() {
        let service = LiveCaptureService::new(LiveCaptureResources::default());
        let initial_revision = service.revision();

        process(&service, EngineEvent::Status("idle heartbeat".to_owned()));
        assert_eq!(service.revision(), initial_revision);
        assert_eq!(service.status().phase, LiveCapturePhase::Idle);

        *service.0.status.lock().expect("status lock") = LiveCaptureStatus {
            phase: LiveCapturePhase::Starting,
            issue: None,
        };
        process(&service, EngineEvent::Status("capture ready".to_owned()));
        let running_revision = service.revision();
        assert_eq!(running_revision, initial_revision.wrapping_add(1));
        assert_eq!(service.status().phase, LiveCapturePhase::Running);

        process(
            &service,
            EngineEvent::Status("running heartbeat".to_owned()),
        );
        assert_eq!(service.revision(), running_revision);
    }

    fn packet_projection<T>(
        service: &LiveCaptureService,
        projection: impl FnOnce(PacketStreamRevision, usize, &CombatState) -> T,
    ) -> T {
        service
            .with_packet_state(projection)
            .expect("healthy packet projection")
    }

    #[test]
    #[cfg(feature = "desktop")]
    fn diagnostics_quality_reuses_hit_projection_for_packet_only_revisions() {
        reset_diagnostic_quality_full_rebuild_count();
        let service = LiveCaptureService::new(LiveCaptureResources::default());
        process(&service, hit(100.0));

        let initial = service
            .quality_summary()
            .expect("initial diagnostics quality projection");
        process(
            &service,
            EngineEvent::PacketObservation(PacketObservation { parsed_hits: 0 }),
        );
        let packet_updated = service
            .quality_summary()
            .expect("packet-only diagnostics quality projection");

        assert_eq!(initial.hit_count, 1);
        assert_eq!(packet_updated.hit_count, 1);
        assert_eq!(packet_updated.packet_count, initial.packet_count + 1);
        assert_eq!(
            diagnostic_quality_full_rebuild_count(),
            0,
            "packet-only generations must never scan the authoritative hit log"
        );
    }

    #[test]
    #[cfg(feature = "desktop")]
    fn diagnostics_quality_extends_append_only_hits_without_full_rebuild() {
        reset_diagnostic_quality_full_rebuild_count();
        let service = LiveCaptureService::new(LiveCaptureResources::default());
        process(&service, hit(100.0));
        service
            .quality_summary()
            .expect("initial diagnostics quality projection");

        process(&service, hit(50.0));
        let incremental = service
            .quality_summary()
            .expect("incremental diagnostics quality projection");
        let legacy = service
            .with_state_and_source(|state, source| state.capture_quality_summary(source))
            .expect("legacy quality reference");

        assert_eq!(incremental, legacy);
        assert_eq!(incremental.hit_count, 2);
        assert_eq!(
            diagnostic_quality_full_rebuild_count(),
            0,
            "append-only hits are already reflected by the reducer-maintained aggregate"
        );
    }

    #[test]
    #[cfg(feature = "desktop")]
    fn diagnostics_quality_reuses_fifty_thousand_hit_projection_on_packet_update() {
        let mut state = CombatState::default();
        for _ in 0..50_000 {
            let EngineEvent::Hit(hit) = hit(1.0) else {
                unreachable!("test hit helper always returns a hit event");
            };
            state.push_hit(*hit);
        }
        let service = LiveCaptureService::new(LiveCaptureResources::default());
        service
            .restore_session(state, CaptureQualitySource::Live)
            .expect("install retained maximum fixture");
        reset_diagnostic_quality_full_rebuild_count();

        let initial = service
            .quality_summary()
            .expect("initial maximum-size quality projection");
        process(
            &service,
            EngineEvent::PacketObservation(PacketObservation { parsed_hits: 0 }),
        );
        let packet_updated = service
            .quality_summary()
            .expect("cached maximum-size quality projection");

        assert_eq!(initial.hit_count, 50_000);
        assert_eq!(packet_updated.hit_count, 50_000);
        assert_eq!(packet_updated.packet_count, 1);
        assert_eq!(diagnostic_quality_full_rebuild_count(), 0);
    }

    #[test]
    #[cfg(feature = "desktop")]
    fn poisoned_diagnostics_cache_rebuilds_without_failing_capture_state() {
        reset_diagnostic_quality_full_rebuild_count();
        let service = LiveCaptureService::new(LiveCaptureResources::default());
        process(&service, hit(100.0));
        service
            .quality_summary()
            .expect("initial diagnostics quality projection");
        let poison_service = service.clone();
        let _ = catch_unwind(AssertUnwindSafe(move || {
            let _cache = poison_service
                .0
                .diagnostic_quality
                .lock()
                .expect("diagnostics cache lock");
            panic!("poison derived diagnostics cache");
        }));

        let rebuilt = service
            .quality_summary()
            .expect("derived cache poison must be recoverable");

        assert_eq!(rebuilt.hit_count, 1);
        assert_eq!(service.status().phase, LiveCapturePhase::Idle);
        assert!(!service.0.integrity_failed.load(Ordering::Acquire));
        assert_eq!(diagnostic_quality_full_rebuild_count(), 0);
    }

    fn take_archives(service: &LiveCaptureService) -> Vec<PendingHistoryArchive> {
        service
            .take_pending_abyss_archives()
            .expect("healthy pending archive queue")
    }

    fn install_test_inventory(service: &LiveCaptureService) -> (u64, u64) {
        process(
            service,
            EngineEvent::EmptyCurtain(vec![EmptyCurtainItem {
                id: HtItemNetId { solt: 1, serial: 2 },
                item_id: "test-item".to_owned(),
                level: 1,
                main_stats: Vec::new(),
                sub_stats: Vec::new(),
                locked: false,
                discarded: false,
                character_net_id: None,
                equipped_character_id: None,
                equipped_placement: None,
            }]),
        );
        process(
            service,
            EngineEvent::EmptyCurtainCharacters(vec![EmptyCurtainCharacter {
                net_id: HtItemNetId { solt: 3, serial: 4 },
                character_id: 1020,
            }]),
        );
        project(service, |state| {
            (
                state.empty_curtain_generation,
                state.empty_curtain_characters_generation,
            )
        })
    }

    fn assert_test_inventory(service: &LiveCaptureService, generations: (u64, u64)) {
        project(service, |state| {
            assert_eq!(state.empty_curtain.len(), 1);
            assert_eq!(state.empty_curtain[0].item_id, "test-item");
            assert_eq!(state.empty_curtain_characters.len(), 1);
            assert_eq!(state.empty_curtain_characters[0].character_id, 1020);
            assert_eq!(
                (
                    state.empty_curtain_generation,
                    state.empty_curtain_characters_generation,
                ),
                generations
            );
        });
    }

    #[test]
    fn event_worker_routes_hits_through_the_shared_reducer() {
        let service = LiveCaptureService::new(LiveCaptureResources::default());
        let initial_revision = service.revision();
        service.ensure_event_worker().expect("event worker");
        service.0.sender.send(hit(321.0)).expect("test hit");

        let deadline = Instant::now() + Duration::from_secs(1);
        while project(&service, |state| state.total_damage) != 321.0
            || service.revision() == initial_revision
        {
            assert!(Instant::now() < deadline, "event worker timed out");
            thread::yield_now();
        }

        assert_eq!(project(&service, |state| state.hits.len()), 1);
        assert!(service.revision() > initial_revision);
    }

    #[test]
    fn rejected_json_replay_preserves_authoritative_session_and_revisions() {
        let directory = std::env::temp_dir().join(format!(
            "nte-live-json-preflight-test-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("clock")
                .as_nanos()
        ));
        std::fs::create_dir_all(&directory).expect("create JSON replay fixture directory");
        struct Cleanup(PathBuf);
        impl Drop for Cleanup {
            fn drop(&mut self) {
                let _ = std::fs::remove_dir_all(&self.0);
            }
        }
        let _cleanup = Cleanup(directory.clone());

        let service = LiveCaptureService::new(LiveCaptureResources::default());
        process(&service, hit(321.0));
        let initial_revision = service.revision();
        let initial_packet_revision = service.0.packet_revision.load(Ordering::Acquire);
        let initial_packet_session = service.0.packet_session_generation.load(Ordering::Acquire);
        let initial_outgoing_revision = service.outgoing_hit_revision();
        let initial_source = service.quality_source().expect("healthy capture source");
        let initial_status = service.status();

        let malformed = directory.join("malformed.json");
        std::fs::write(&malformed, b"{").expect("write malformed replay");
        assert!(matches!(
            service.request_replay(CaptureReplayKind::Json, malformed, None, true, false),
            Err(ReplayStartError::JsonImport(
                CaptureImportError::InvalidFormat
            ))
        ));

        let unsupported = directory.join("unsupported.json");
        std::fs::write(&unsupported, br#"{"version":2}"#).expect("write unsupported replay");
        assert!(matches!(
            service.request_replay(CaptureReplayKind::Json, unsupported, None, true, false),
            Err(ReplayStartError::JsonImport(
                CaptureImportError::UnsupportedVersion {
                    found: 2,
                    expected: crate::engine::capture::CAPTURE_EXPORT_VERSION
                }
            ))
        ));

        assert_eq!(service.revision(), initial_revision);
        assert_eq!(
            service.0.packet_revision.load(Ordering::Acquire),
            initial_packet_revision
        );
        assert_eq!(
            service.0.packet_session_generation.load(Ordering::Acquire),
            initial_packet_session
        );
        assert_eq!(service.outgoing_hit_revision(), initial_outgoing_revision);
        assert_eq!(
            service.quality_source().expect("healthy capture source"),
            initial_source
        );
        assert_eq!(service.status(), initial_status);
        assert_eq!(project(&service, |state| state.total_damage), 321.0);
        assert_eq!(project(&service, |state| state.hits.len()), 1);
        assert!(!service.replay_running().expect("healthy replay owner"));
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

        let pending_events = packet_projection(&service, |_, pending, _| pending);
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
        process(&service, EngineEvent::ModScript(identity_event(1.0)));
        process(&service, hit(100.0));

        let before_target = service.revision();
        process(&service, EngineEvent::ModScript(hit_target_event(2, 1.05)));
        assert!(
            service.revision() > before_target,
            "target backfill must advance the frontend revision"
        );
        assert_eq!(
            project(&service, |state| state.hits[0].target_name.clone()),
            Some("随心泥".to_owned())
        );

        let before_duplicate = service.revision();
        process(&service, EngineEvent::ModScript(hit_target_event(3, 1.05)));
        assert_eq!(
            service.revision(),
            before_duplicate,
            "idempotent ModScript backfill must not bump the revision"
        );
    }

    #[test]
    fn idle_policy_tracks_only_confirmed_outgoing_hits() {
        let service = LiveCaptureService::new(LiveCaptureResources::default());
        assert!(
            service
                .idle_elapsed()
                .expect("healthy idle timer")
                .is_none()
        );

        process(
            &service,
            EngineEvent::Hit({
                let mut incoming = match hit(1.0) {
                    EngineEvent::Hit(hit) => hit,
                    _ => unreachable!("test helper returns a hit"),
                };
                incoming.direction = HitDirection::Incoming;
                incoming
            }),
        );
        assert!(
            service
                .idle_elapsed()
                .expect("healthy idle timer")
                .is_none(),
            "incoming hits must not reset the auto-round idle timer"
        );

        process(
            &service,
            EngineEvent::Hit({
                let mut unknown = match hit(2.0) {
                    EngineEvent::Hit(hit) => hit,
                    _ => unreachable!("test helper returns a hit"),
                };
                unknown.direction = HitDirection::Unknown;
                unknown
            }),
        );
        assert!(
            service
                .idle_elapsed()
                .expect("healthy idle timer")
                .is_none(),
            "unknown-direction hits must not reset the auto-round idle timer"
        );

        process(&service, hit(3.0));
        assert!(
            service
                .idle_elapsed()
                .expect("healthy idle timer")
                .is_some(),
            "confirmed outgoing hits must reset the auto-round idle timer"
        );
    }

    #[test]
    fn inventory_events_advance_only_the_inventory_projection() {
        let service = LiveCaptureService::new(LiveCaptureResources::default());
        let initial_revision = service.revision();
        let initial_inventory_revision = service
            .inventory_revision()
            .expect("healthy inventory revision");

        // A repeated empty snapshot is a true no-op, not an event counter.
        process(&service, EngineEvent::EmptyCurtain(Vec::new()));
        assert_eq!(
            service
                .inventory_revision()
                .expect("healthy inventory revision"),
            initial_inventory_revision
        );

        process(
            &service,
            EngineEvent::EmptyCurtain(vec![EmptyCurtainItem {
                id: HtItemNetId { solt: 1, serial: 2 },
                item_id: "test-item".to_owned(),
                level: 1,
                main_stats: Vec::new(),
                sub_stats: Vec::new(),
                locked: false,
                discarded: false,
                character_net_id: None,
                equipped_character_id: None,
                equipped_placement: None,
            }]),
        );
        let item_revision = service
            .inventory_revision()
            .expect("healthy inventory revision");
        assert!(item_revision.0 > initial_inventory_revision.0);
        assert_eq!(service.revision(), initial_revision);
        let duplicate_items = project(&service, |state| state.empty_curtain.clone());
        process(&service, EngineEvent::EmptyCurtain(duplicate_items));
        assert_eq!(
            service
                .inventory_revision()
                .expect("healthy inventory revision"),
            item_revision
        );

        process(&service, EngineEvent::EmptyCurtainCharacters(Vec::new()));
        assert_eq!(
            service
                .inventory_revision()
                .expect("healthy inventory revision"),
            item_revision
        );
        process(
            &service,
            EngineEvent::EmptyCurtainCharacters(vec![EmptyCurtainCharacter {
                net_id: HtItemNetId { solt: 3, serial: 4 },
                character_id: 1020,
            }]),
        );
        let character_revision = service
            .inventory_revision()
            .expect("healthy inventory revision");
        assert!(character_revision.1 > item_revision.1);
        let duplicate_characters =
            project(&service, |state| state.empty_curtain_characters.clone());
        process(
            &service,
            EngineEvent::EmptyCurtainCharacters(duplicate_characters),
        );
        assert_eq!(
            service
                .inventory_revision()
                .expect("healthy inventory revision"),
            character_revision
        );
        assert_eq!(service.revision(), initial_revision);
    }

    #[test]
    fn cut_round_installs_replacement_before_persistence_work() {
        let service = LiveCaptureService::new(LiveCaptureResources::default());
        let inventory_generations = install_test_inventory(&service);
        process(&service, hit(321.0));

        let cut = service
            .cut_round()
            .expect("healthy live-capture state")
            .expect("archivable round");
        process(&service, hit(99.0));

        assert_eq!(cut.state.total_damage, 321.0);
        assert_eq!(cut.source, CaptureQualitySource::Unknown);
        assert!(cut.state.empty_curtain.is_empty());
        assert!(cut.state.empty_curtain_characters.is_empty());
        assert_test_inventory(&service, inventory_generations);
        assert_eq!(project(&service, |state| state.total_damage), 99.0);
    }

    #[test]
    fn cut_round_freezes_replay_source_and_empty_round_is_a_no_op() {
        let service = LiveCaptureService::new(LiveCaptureResources::default());
        assert!(
            service
                .cut_round()
                .expect("healthy live-capture state")
                .is_none()
        );
        *service
            .0
            .quality_source
            .lock()
            .expect("quality source lock") = CaptureQualitySource::JsonReplay;
        process(&service, hit(321.0));

        let cut = service
            .cut_round()
            .expect("healthy live-capture state")
            .expect("archivable replay round");

        assert_eq!(cut.source, CaptureQualitySource::JsonReplay);
        assert_eq!(cut.state.total_damage, 321.0);
        assert!(project(&service, |state| state.hits.is_empty()));
    }

    #[test]
    fn abyss_restart_queues_the_previous_round_before_reducer_changes_state() {
        let service = LiveCaptureService::new(LiveCaptureResources::default());
        let inventory_generations = install_test_inventory(&service);
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
        process(
            &service,
            EngineEvent::Abyss(AbyssEvent::RestartDetected { timestamp: 3.0 }),
        );

        let archives = take_archives(&service);
        assert_eq!(archives.len(), 1);
        assert_eq!(archives[0].details.first_half_hits.len(), 1);
        assert_eq!(archives[0].details.first_half_hits[0].damage, 321.0);
        assert_eq!(archives[0].source, CaptureQualitySource::PcapngReplay);
        assert_test_inventory(&service, inventory_generations);
        assert!(project(&service, |state| state.hits.is_empty()));
    }

    #[test]
    fn abyss_floor_transition_archives_previous_round_and_preserves_inventory() {
        let service = LiveCaptureService::new(LiveCaptureResources::default());
        let inventory_generations = install_test_inventory(&service);
        process(
            &service,
            EngineEvent::Abyss(AbyssEvent::Stage {
                timestamp: 0.0,
                cycle: None,
                floor: Some(12),
                half: crate::engine::model::AbyssHalf::First,
                allow_late_backfill: false,
            }),
        );
        process(&service, hit(321.0));
        process(
            &service,
            EngineEvent::Abyss(AbyssEvent::Stage {
                timestamp: 3.0,
                cycle: None,
                floor: Some(13),
                half: crate::engine::model::AbyssHalf::First,
                allow_late_backfill: false,
            }),
        );

        let archives = take_archives(&service);
        assert_eq!(archives.len(), 1);
        assert_eq!(archives[0].details.first_half_hits[0].damage, 321.0);
        assert_test_inventory(&service, inventory_generations);
        project(&service, |state| {
            assert_eq!(state.abyss.floor, Some(13));
            assert!(state.hits.is_empty());
        });
    }

    #[test]
    fn abyss_exit_archive_contains_the_real_exit_timestamp() {
        let service = LiveCaptureService::new(LiveCaptureResources::default());
        let inventory_generations = install_test_inventory(&service);
        process(
            &service,
            EngineEvent::Abyss(AbyssEvent::Stage {
                timestamp: 0.0,
                cycle: None,
                floor: Some(12),
                half: crate::engine::model::AbyssHalf::First,
                allow_late_backfill: false,
            }),
        );
        process(&service, hit(321.0));
        process(
            &service,
            EngineEvent::Abyss(AbyssEvent::Exit { timestamp: 10.0 }),
        );

        let archives = take_archives(&service);
        assert_eq!(archives.len(), 1);
        assert_eq!(archives[0].details.first_half_hits.len(), 1);
        assert_eq!(archives[0].details.first_half_hits[0].damage, 321.0);
        assert_eq!(archives[0].details.exited_at, Some(10.0));
        assert_test_inventory(&service, inventory_generations);
        assert!(project(&service, |state| state.hits.is_empty()));
    }

    #[test]
    fn automatic_archive_freezes_effective_clock_policy_and_preserves_provider_health() {
        let service = LiveCaptureService::new(LiveCaptureResources::default());
        service.set_history_archive_policy(HistoryArchivePolicy {
            requested_dps_time_mode: DpsTimeBasis::SubtractTimeStop,
            separate_reaction_damage: true,
        });
        process(
            &service,
            EngineEvent::CombatClockHealth(CombatClockRuntimeHealth::Available),
        );
        process(
            &service,
            EngineEvent::Abyss(AbyssEvent::Stage {
                timestamp: 0.0,
                cycle: None,
                floor: Some(12),
                half: crate::engine::model::AbyssHalf::First,
                allow_late_backfill: false,
            }),
        );
        process(&service, hit(321.0));
        process(
            &service,
            EngineEvent::Abyss(AbyssEvent::Exit { timestamp: 10.0 }),
        );

        // A later setting must not reinterpret the already detached round.
        service.set_history_archive_policy(HistoryArchivePolicy {
            requested_dps_time_mode: DpsTimeBasis::WallClock,
            separate_reaction_damage: false,
        });
        let archive = take_archives(&service).pop().expect("pending archive");
        assert_eq!(archive.dps_time_mode, DpsTimeBasis::SubtractTimeStop);
        assert!(archive.separate_reaction_damage);
        assert_eq!(
            archive.details.combat_clock_health,
            CombatClockRuntimeHealth::Recorded
        );
        assert_eq!(
            archive.details.to_combat_state().combat_clock_health,
            CombatClockRuntimeHealth::Recorded
        );
        assert_eq!(
            service.combat_clock_health().unwrap(),
            CombatClockRuntimeHealth::Available,
            "round cut must preserve capture-session provider health"
        );
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

        process(&service, EngineEvent::CaptureStopped);
        service.reset_session().expect("healthy live-capture reset");

        let archives = take_archives(&service);
        assert_eq!(archives.len(), 1);
        assert_eq!(archives[0].details.second_half_hits.len(), 1);
        assert_eq!(archives[0].details.second_half_hits[0].damage, 654.0);
        assert!(project(&service, |state| state.hits.is_empty()));
    }

    #[test]
    fn capture_stopped_archives_after_prior_semantic_events() {
        let service = LiveCaptureService::new(LiveCaptureResources::default());
        let inventory_generations = install_test_inventory(&service);

        process(
            &service,
            EngineEvent::Abyss(AbyssEvent::Stage {
                timestamp: 0.0,
                cycle: None,
                floor: Some(12),
                half: crate::engine::model::AbyssHalf::First,
                allow_late_backfill: false,
            }),
        );
        process(&service, hit(100.0));
        process(&service, hit(200.0));

        process(&service, EngineEvent::CaptureStopped);

        let archives = take_archives(&service);
        assert_eq!(archives.len(), 1);
        assert_eq!(archives[0].details.first_half_hits.len(), 2);
        assert_eq!(
            service.status().phase,
            LiveCapturePhase::Stopped,
            "CaptureStopped is the single final archive and Stopped barrier"
        );
        assert_test_inventory(&service, inventory_generations);
        assert!(project(&service, |state| state.hits.is_empty()));
    }

    #[test]
    fn pending_abyss_archive_queue_is_bounded() {
        let service = LiveCaptureService::new(LiveCaptureResources::default());
        process(
            &service,
            EngineEvent::Abyss(AbyssEvent::Stage {
                timestamp: 0.0,
                cycle: None,
                floor: Some(12),
                half: crate::engine::model::AbyssHalf::First,
                allow_late_backfill: false,
            }),
        );

        for round in 0..(MAX_PENDING_ABYSS_ARCHIVES + 4) {
            process(&service, hit((round + 1) as f64));
            process(
                &service,
                EngineEvent::Abyss(AbyssEvent::RestartDetected {
                    timestamp: (round + 1) as f64,
                }),
            );
            process(
                &service,
                EngineEvent::Abyss(AbyssEvent::Stage {
                    timestamp: (round + 1) as f64 + 0.5,
                    cycle: None,
                    floor: Some(12),
                    half: crate::engine::model::AbyssHalf::First,
                    allow_late_backfill: false,
                }),
            );
        }

        let archives = take_archives(&service);
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
        process(
            &service,
            EngineEvent::Abyss(AbyssEvent::Stage {
                timestamp: 0.0,
                cycle: None,
                floor: Some(12),
                half: crate::engine::model::AbyssHalf::First,
                allow_late_backfill: false,
            }),
        );
        process(&service, hit(321.0));
        process(&service, EngineEvent::CaptureStopped);
        let template = take_archives(&service)
            .into_iter()
            .next()
            .expect("template archive");

        service
            .restore_pending_abyss_archives(vec![template; MAX_PENDING_ABYSS_ARCHIVES + 5])
            .expect("healthy pending archive queue");

        assert_eq!(take_archives(&service).len(), MAX_PENDING_ABYSS_ARCHIVES);
        assert_eq!(service.dropped_history_archives(), 5);
    }

    #[test]
    fn abyss_restart_dedupe_does_not_collide_across_rounds() {
        let service = LiveCaptureService::new(LiveCaptureResources::default());

        // First round: four hits on the first half, then archive it through
        // the same final-archive path used when capture stops.
        process(
            &service,
            EngineEvent::Abyss(AbyssEvent::Stage {
                timestamp: 0.0,
                cycle: None,
                floor: Some(12),
                half: crate::engine::model::AbyssHalf::First,
                allow_late_backfill: false,
            }),
        );
        for _ in 0..4 {
            process(&service, hit(100.0));
        }
        process(&service, EngineEvent::CaptureStopped);

        // Exit then restart resets the Abyss party generations while the
        // global hits generation keeps advancing.
        process(
            &service,
            EngineEvent::Abyss(AbyssEvent::Exit { timestamp: 10.0 }),
        );
        process(
            &service,
            EngineEvent::Abyss(AbyssEvent::RestartDetected { timestamp: 11.0 }),
        );
        process(
            &service,
            EngineEvent::Abyss(AbyssEvent::Stage {
                timestamp: 12.0,
                cycle: None,
                floor: Some(12),
                half: crate::engine::model::AbyssHalf::First,
                allow_late_backfill: false,
            }),
        );

        // Second round with fewer hits. Its summed generation (6 + 2) equals
        // the first round's summed generation (4 + 4), so a sum-based dedupe
        // would suppress this round; the monotonic global marker does not.
        for _ in 0..2 {
            process(&service, hit(200.0));
        }
        process(
            &service,
            EngineEvent::Abyss(AbyssEvent::Stage {
                timestamp: 20.0,
                cycle: None,
                floor: Some(13),
                half: crate::engine::model::AbyssHalf::First,
                allow_late_backfill: false,
            }),
        );

        let archives = take_archives(&service);
        assert_eq!(archives.len(), 2);
        assert_eq!(archives[0].details.first_half_hits.len(), 4);
        assert_eq!(archives[1].details.first_half_hits.len(), 2);
    }

    #[test]
    fn runtime_failures_keep_private_details_out_of_status() {
        let service = LiveCaptureService::new(LiveCaptureResources::default());
        let initial_revision = service.revision();
        let initial_packet_revision = packet_projection(&service, |revision, _, _| revision);

        process(&service, EngineEvent::Warning("private warning".to_owned()));
        assert_eq!(
            service.status().issue,
            Some(LiveCaptureIssue::RuntimeWarning)
        );
        let warning_revision = service.revision();
        assert!(warning_revision > initial_revision);

        process(&service, EngineEvent::Error("private failure".to_owned()));
        assert_eq!(
            service.status(),
            LiveCaptureStatus {
                phase: LiveCapturePhase::Failed,
                issue: Some(LiveCaptureIssue::RuntimeError),
            }
        );
        assert!(service.revision() > warning_revision);
        assert!(
            packet_projection(&service, |revision, _, _| revision.generation)
                > initial_packet_revision.generation
        );
    }

    #[test]
    fn manual_network_probe_degradation_is_published_with_status_revision() {
        let service = LiveCaptureService::new(LiveCaptureResources::default());
        let initial_revision = service.revision();
        let initial_packet_revision = packet_projection(&service, |revision, _, _| revision);

        assert!(
            !service
                .publish_start_success(Some(NetworkProbeErrorCode::ProcessSnapshotFailed))
                .expect("healthy capture status")
        );

        assert_eq!(
            service.status(),
            LiveCaptureStatus {
                phase: LiveCapturePhase::Running,
                issue: Some(LiveCaptureIssue::NetworkProbeDegraded(
                    NetworkProbeErrorCode::ProcessSnapshotFailed,
                )),
            }
        );
        assert_eq!(service.revision(), initial_revision + 1);
        assert_eq!(
            packet_projection(&service, |revision, _, _| revision.generation),
            initial_packet_revision.generation + 1
        );
        assert_eq!(
            packet_projection(&service, |revision, _, _| revision.session_generation),
            initial_packet_revision.session_generation
        );
    }

    #[test]
    fn manual_network_probe_degradation_survives_runtime_warnings() {
        let service = LiveCaptureService::new(LiveCaptureResources::default());
        let code = NetworkProbeErrorCode::ProcessEnumerationFailed;

        assert!(
            !service
                .publish_start_success(Some(code))
                .expect("healthy capture status")
        );
        let running_revision = service.revision();
        process(
            &service,
            EngineEvent::Warning("private runtime warning".to_owned()),
        );

        assert_eq!(
            service.status().issue,
            Some(LiveCaptureIssue::NetworkProbeDegraded(code))
        );
        assert_eq!(
            service.revision(),
            running_revision,
            "a hidden lower-priority warning is not a visible status mutation"
        );

        service
            .0
            .note_dropped_history_archives(1)
            .expect("healthy capture status");

        assert_eq!(
            service.status().issue,
            Some(LiveCaptureIssue::NetworkProbeDegraded(code))
        );
        assert_eq!(service.dropped_history_archives(), 1);
        assert_eq!(service.revision(), running_revision + 1);
        let status = format!("{:?}", service.status());
        assert_eq!(code.as_str(), "PROCESS_ENUMERATION_FAILED");
        assert!(!status.contains("Win32 error 5"));
        assert!(!status.contains("private runtime warning"));
    }

    #[test]
    fn stopping_after_completed_runtime_failure_does_not_stick() {
        let service = LiveCaptureService::new(LiveCaptureResources::default());

        // A running-then-failed lifecycle: an Abyss round with hits, the final
        // CaptureStopped drain barrier, then the runtime Error.
        process(
            &service,
            EngineEvent::Abyss(AbyssEvent::Stage {
                timestamp: 0.0,
                cycle: None,
                floor: Some(12),
                half: crate::engine::model::AbyssHalf::First,
                allow_late_backfill: false,
            }),
        );
        process(&service, hit(100.0));
        process(&service, hit(200.0));
        process(&service, EngineEvent::CaptureStopped);
        process(&service, EngineEvent::Error("capture failed".to_owned()));

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

        let archives = take_archives(&service);
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
        let initial_packet_revision = packet_projection(&service, |revision, _, _| revision);

        process(
            &service,
            EngineEvent::PacketObservation(PacketObservation { parsed_hits: 1 }),
        );

        assert_eq!(service.revision(), initial_revision);
        assert_eq!(project(&service, |state| state.packet_count), 1);
        assert!(
            packet_projection(&service, |revision, _, _| revision.generation)
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

        process(&service, EngineEvent::Hit(incoming));
        assert_eq!(service.outgoing_hit_revision(), initial);

        process(&service, hit(34.0));
        assert_eq!(service.outgoing_hit_revision(), initial + 1);
    }

    #[test]
    fn reset_session_can_restore_the_exact_previous_projection() {
        let service = LiveCaptureService::new(LiveCaptureResources::default());
        process(&service, hit(321.0));
        let previous = project(&service, Clone::clone);

        service.reset_session().expect("healthy capture reset");
        assert!(project(&service, |state| state.hits.is_empty()));

        service
            .restore_session(previous, CaptureQualitySource::PcapngReplay)
            .expect("healthy capture restore");
        assert_eq!(project(&service, |state| state.total_damage), 321.0);
        assert_eq!(
            service.quality_source().expect("healthy capture source"),
            CaptureQualitySource::PcapngReplay
        );
    }

    #[test]
    fn reset_session_preserves_available_combat_clock_health_without_a_new_transition() {
        let service = LiveCaptureService::new(LiveCaptureResources::default());
        process(
            &service,
            EngineEvent::CombatClockHealth(CombatClockRuntimeHealth::Available),
        );

        service.reset_session().expect("healthy capture reset");

        assert_eq!(
            service
                .combat_clock_health()
                .expect("healthy combat-clock projection"),
            CombatClockRuntimeHealth::Available
        );
        let revision = service.revision();
        process(
            &service,
            EngineEvent::CombatClockHealth(CombatClockRuntimeHealth::Available),
        );
        assert_eq!(
            service.revision(),
            revision,
            "the unchanged provider heartbeat remains a no-op after reset"
        );
    }

    #[test]
    fn state_and_source_snapshot_uses_one_session_gate() {
        let service = LiveCaptureService::new(LiveCaptureResources::default());
        let mut state = CombatState::default();
        let EngineEvent::Hit(hit) = hit(456.0) else {
            unreachable!("test helper returns a hit")
        };
        state.push_hit(*hit);
        service
            .restore_session(state, CaptureQualitySource::JsonReplay)
            .expect("healthy capture restore");

        let (snapshot, source) = service
            .state_and_source_snapshot()
            .expect("healthy capture snapshot");

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

    #[test]
    fn poisoned_authoritative_state_does_not_run_a_projection() {
        let service = LiveCaptureService::new(LiveCaptureResources::default());
        let initial_revision = service.revision();
        let initial_packet_revision = service.0.packet_revision.load(Ordering::Acquire);
        let initial_packet_session = service.0.packet_session_generation.load(Ordering::Acquire);
        let poison_service = service.clone();
        let _ = catch_unwind(AssertUnwindSafe(move || {
            let _state = poison_service.0.state.lock().expect("lock live state");
            panic!("poison authoritative live state");
        }));
        let projection_ran = AtomicBool::new(false);

        let result = service.with_state(|_| projection_ran.store(true, Ordering::Release));

        assert_eq!(
            result.expect_err("a poisoned state must fail closed").code,
            CoreErrorCode::CaptureStateUnavailable
        );
        assert!(!projection_ran.load(Ordering::Acquire));
        assert_eq!(
            service.status(),
            LiveCaptureStatus {
                phase: LiveCapturePhase::Failed,
                issue: Some(LiveCaptureIssue::StateUnavailable),
            }
        );
        assert_eq!(service.revision(), initial_revision + 1);
        assert_eq!(
            service.0.packet_revision.load(Ordering::Acquire),
            initial_packet_revision + 1
        );
        assert_eq!(
            service.0.packet_session_generation.load(Ordering::Acquire),
            initial_packet_session + 1
        );

        assert_eq!(
            service
                .with_state(|_| ())
                .expect_err("sticky failure must reject every later projection")
                .code,
            CoreErrorCode::CaptureStateUnavailable
        );
        assert_eq!(
            service.revision(),
            initial_revision + 1,
            "the terminal revision effect must be applied exactly once"
        );
    }

    #[test]
    fn poisoned_event_gate_rejects_reset_before_mutating_the_live_round() {
        let service = LiveCaptureService::new(LiveCaptureResources::default());
        process(&service, hit(42.0));
        let poison_service = service.clone();
        let _ = catch_unwind(AssertUnwindSafe(move || {
            let _gate = poison_service.0.event_gate.lock().expect("lock event gate");
            panic!("poison live event gate");
        }));

        let result = service.reset_session();
        let total_damage = service
            .0
            .state
            .lock()
            .expect("state itself remains healthy")
            .total_damage;

        assert_eq!(
            result
                .expect_err("a poisoned transaction gate must fail closed")
                .code,
            CoreErrorCode::CaptureStateUnavailable
        );
        assert_eq!(total_damage, 42.0, "reset must not partially commit");
    }

    #[test]
    fn poisoned_idle_timer_is_a_sticky_terminal_capture_failure() {
        let service = LiveCaptureService::new(LiveCaptureResources::default());
        let poison_service = service.clone();
        let _ = catch_unwind(AssertUnwindSafe(move || {
            let _timer = poison_service
                .0
                .last_outgoing_hit_at
                .lock()
                .expect("lock idle timer");
            panic!("poison live idle timer");
        }));

        let error = service
            .idle_elapsed()
            .expect_err("a poisoned idle timer must fail closed");

        assert_eq!(error.code, CoreErrorCode::CaptureStateUnavailable);
        assert_eq!(service.status(), terminal_capture_status());
        assert_eq!(
            service
                .idle_elapsed()
                .expect_err("idle timer failure must remain sticky")
                .code,
            CoreErrorCode::CaptureStateUnavailable
        );
    }

    #[test]
    fn replay_final_event_joins_the_owner_without_deadlocking() {
        let service = LiveCaptureService::new(LiveCaptureResources::default());
        service
            .ensure_event_worker()
            .expect("event worker should start");
        let (sender, permit) = service.0.sender.clone().pause_delivery();
        let producer_done = Arc::new(AtomicBool::new(false));
        let worker_done = Arc::clone(&producer_done);
        let thread = thread::Builder::new()
            .name("nte-test-replay-final-event".to_owned())
            .spawn(move || {
                sender
                    .send(EngineEvent::CaptureStopped)
                    .expect("final replay event should be delivered");
                worker_done.store(true, Ordering::Release);
            })
            .expect("test replay producer should start");
        *service.0.replay.lock().expect("replay owner lock") = Some(ReplayTask {
            stop: Arc::new(AtomicBool::new(false)),
            thread,
        });
        *service.0.status.lock().expect("capture status lock") = LiveCaptureStatus {
            phase: LiveCapturePhase::Running,
            issue: None,
        };

        permit.release();
        let deadline = Instant::now() + Duration::from_secs(2);
        while service.status().phase != LiveCapturePhase::Stopped && Instant::now() < deadline {
            thread::sleep(Duration::from_millis(1));
        }

        assert_eq!(service.status().phase, LiveCapturePhase::Stopped);
        assert!(producer_done.load(Ordering::Acquire));
        assert!(
            service
                .0
                .replay
                .lock()
                .expect("replay owner lock")
                .is_none(),
            "the completed replay owner must be removed"
        );
    }

    #[test]
    fn fail_closed_worker_cancels_and_drains_the_replay_owner() {
        let service = LiveCaptureService::new(LiveCaptureResources::default());
        service
            .ensure_event_worker()
            .expect("event worker should start");
        let stop = Arc::new(AtomicBool::new(false));
        let worker_stop = Arc::clone(&stop);
        let thread = thread::Builder::new()
            .name("nte-test-replay-owner".to_owned())
            .spawn(move || {
                while !worker_stop.load(Ordering::Acquire) {
                    thread::yield_now();
                }
            })
            .expect("test replay owner should start");
        *service.0.replay.lock().expect("replay owner lock") = Some(ReplayTask {
            stop: Arc::clone(&stop),
            thread,
        });
        let poison_service = service.clone();
        let _ = catch_unwind(AssertUnwindSafe(move || {
            let _state = poison_service.0.state.lock().expect("lock live state");
            panic!("poison authoritative state with active replay owner");
        }));

        assert_eq!(
            service
                .with_state(|_| ())
                .expect_err("state poison must fail closed")
                .code,
            CoreErrorCode::CaptureStateUnavailable
        );
        let deadline = Instant::now() + Duration::from_secs(2);
        loop {
            let finished = service
                .0
                .event_worker
                .lock()
                .expect("event worker runtime lock")
                .worker
                .as_ref()
                .is_some_and(thread::JoinHandle::is_finished);
            if finished || Instant::now() >= deadline {
                break;
            }
            thread::sleep(Duration::from_millis(1));
        }

        assert!(stop.load(Ordering::Acquire));
        assert!(
            service
                .0
                .replay
                .lock()
                .expect("replay owner lock")
                .is_none(),
            "fail-closed cleanup must remove the replay owner"
        );
        assert!(
            service
                .0
                .event_worker
                .lock()
                .expect("event worker runtime lock")
                .worker
                .as_ref()
                .is_some_and(thread::JoinHandle::is_finished),
            "fail-closed cleanup must terminate the event worker"
        );
    }

    #[test]
    fn sticky_failure_keeps_already_cut_abyss_retries_salvageable() {
        let service = LiveCaptureService::new(LiveCaptureResources::default());
        process(
            &service,
            EngineEvent::Abyss(AbyssEvent::Stage {
                timestamp: 0.0,
                cycle: None,
                floor: Some(12),
                half: crate::engine::model::AbyssHalf::First,
                allow_late_backfill: false,
            }),
        );
        process(&service, hit(123.0));
        process(&service, EngineEvent::CaptureStopped);
        let archive = take_archives(&service)
            .pop()
            .expect("finished Abyss round should be queued");
        let poison_service = service.clone();
        let _ = catch_unwind(AssertUnwindSafe(move || {
            let _state = poison_service.0.state.lock().expect("lock live state");
            panic!("poison capture after round cut");
        }));
        assert!(service.with_state(|_| ()).is_err());

        service
            .restore_pending_abyss_archives(vec![archive])
            .expect("durability cleanup must accept retry after capture failure");

        let salvaged = service
            .take_pending_abyss_archives()
            .expect("durability cleanup must drain retry after capture failure");
        assert_eq!(salvaged.len(), 1);
        assert_eq!(salvaged[0].source, CaptureQualitySource::Unknown);
        assert_eq!(service.status(), terminal_capture_status());
    }

    #[test]
    fn dropping_the_last_service_cancels_an_event_worker_owned_replay() {
        let service = LiveCaptureService::new(LiveCaptureResources::default());
        service
            .ensure_event_worker()
            .expect("event worker should start");
        let stop = Arc::new(AtomicBool::new(false));
        let worker_stop = Arc::clone(&stop);
        let thread = thread::Builder::new()
            .name("nte-test-drop-replay-owner".to_owned())
            .spawn(move || {
                while !worker_stop.load(Ordering::Acquire) {
                    thread::yield_now();
                }
            })
            .expect("test replay owner should start");
        *service.0.replay.lock().expect("replay owner lock") = Some(ReplayTask {
            stop: Arc::clone(&stop),
            thread,
        });
        for damage in 1..=128 {
            service
                .0
                .sender
                .send(hit(f64::from(damage)))
                .expect("queued event should be accepted");
        }

        drop(service);
        let deadline = Instant::now() + Duration::from_secs(2);
        while !stop.load(Ordering::Acquire) && Instant::now() < deadline {
            thread::sleep(Duration::from_millis(1));
        }

        assert!(
            stop.load(Ordering::Acquire),
            "dropping the service must cancel the replay owner without self-join"
        );
    }
}
