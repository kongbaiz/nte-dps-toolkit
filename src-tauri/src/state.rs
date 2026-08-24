use std::{
    collections::{HashMap, HashSet, VecDeque},
    fmt,
    path::PathBuf,
    sync::{
        Arc, Mutex, MutexGuard,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use nte_dps_tool::{
    core::{
        CoreError, CoreErrorCode,
        capture::{
            CaptureControllerOptions, CaptureDeviceSelector, CaptureProfile, RawCaptureMode,
            enumerate_devices,
        },
        character_data::{CharacterDataProjection, CharacterDataRecordInput},
        combat_details::CombatDetailFilter,
        diagnostics::{DiagnosticRun, DiagnosticSnapshot},
        encrypted_ini::{EncryptedIniKey, EncryptedIniSaveOutcome},
        history::{
            HistoryArchivePolicy, PendingHistoryArchive, PreparedHistoryArchive, auto_round_due,
            prepare_history_archive_owned,
        },
        hud::{HudProjectionOptions, HudSnapshot, project_hud},
        live_capture::{
            CaptureReplayKind, LiveCapturePhase, LiveCaptureResources, LiveCaptureService,
            LiveCaptureStatus, ReplayStartError,
        },
        mod_studio::{
            ModStudioError, ModStudioRuntimeEvent, ModStudioRuntimeLog, ModStudioWorkspaceService,
            poll_mod_studio_runtime_events, poll_mod_studio_runtime_logs,
        },
        packets::{
            PacketStreamRevision, PacketsProjection, project_packets_since, project_recent_packets,
        },
        skills::{SkillsProjection, SkillsProjectionOptions, SkillsScope, project_skills},
        snapshot::{InventorySnapshot, inventory_snapshot},
        timeline::{
            TimelineProjection, TimelineProjectionOptions, TimelineScope, project_timeline,
        },
        update::{AvailableComponentUpdate, UpdateComponent},
    },
    engine::{
        capture::{
            CaptureExportNetwork, CaptureExportOptions, CaptureExportPlan, CaptureImportError,
            PacketEmissionMode, PreparedCaptureJsonReplay, prepare_capture_json_replay,
        },
        model::{
            AbyssHalf, CaptureQualitySource, CaptureQualitySummary, CombatClockRuntimeHealth,
            CombatState, DamageAttributionSummary, DpsTimeBasis, EmptyCurtainCharacter,
            EmptyCurtainItem, Hit, TeamDps, TeamDpsExport,
        },
        parser::{
            CHARACTER_DATA_PATH, EQUIPMENT_CATALOG_PATH, EquipmentCatalog, load_equipment_catalog,
        },
    },
    platform::{
        mod_loader::ModLoaderRuntimeService,
        mods_plugin::{ModsPluginGameRegion, ModsPluginOperation},
    },
    storage::{
        capture_logs::{ClearOutcome, clear_capture_logs, scan_capture_logs},
        config::{
            self, AccentColor, DpsTimeMode, GlobalHotkeys, HotkeyBinding, HudConfig, HudModule,
            MainDpsDisplayConfig, ModStudioLoadingMethod, ThemePreset, TimelineDpsViewMode,
            UiConfig, UiDensity, sanitize_timeline_bucket_seconds,
        },
        history::{
            BorrowedHistorySaveOutcome, HistoryCombatDetails, HistoryDeleteTombstone,
            HistoryIndexRecord, discard_tombstoned_record, load_history_index,
            load_history_record_by_id_for_interactive_selection, save_borrowed_archive_outcome,
        },
        i18n::Language,
        paths::{capture_log_dir, software_dir},
        update::PreparedUpdate,
    },
};

#[cfg(not(test))]
use nte_dps_tool::storage::history::cleanup_orphaned_history_tombstones_at_startup;
#[cfg(not(test))]
use std::sync::Once;

#[cfg(test)]
use nte_dps_tool::{
    core::history::prepare_history_archive,
    storage::history::{HistoryRecord, HistorySaveOutcome},
};

use crate::{
    character_data_service::{CharacterDataService, CharacterDataServiceError},
    contract::{
        HudWindowSnapshot, TECHNICAL_CONTRACT_VERSION, TechnicalSnapshot,
        main_dps_detail::MainDpsDetailSnapshot,
        settings::{CaptureDeviceSnapshot, SettingsSnapshot, UpdateSettingsSnapshot},
        timeline::{MAX_TIMELINE_BUCKETS, MAX_TIMELINE_CHARACTERS, MAX_TIMELINE_ROLES_PER_BUCKET},
    },
    desktop_runtime::DesktopRuntime,
    diagnostics_runtime::DiagnosticsRuntime,
    encrypted_ini_service::{
        EncryptedIniProjection, EncryptedIniService, EncryptedIniServiceError,
    },
    equipment_operation_service::{
        EmptyCurtainOperationState, EquipmentOperationError, EquipmentOperationService,
        EquipmentOperationSnapshot,
    },
    settings_service::{
        ConfigUpdate, PassthroughTransactionGuard, SettingsService, SettingsServiceError,
    },
    team_import_service::{TeamImportError, TeamImportService},
    update_runtime::{UpdateRuntimeService, UpdateRuntimeSnapshot},
    windows::hud::HUD_WINDOW_LABEL,
};

pub(crate) use crate::update_runtime::UpdateActionError;

#[derive(Clone, Debug)]
pub(crate) enum EmptyCurtainRuntimeError {
    Capture(CoreError),
    Operation(EquipmentOperationError),
}

impl From<CoreError> for EmptyCurtainRuntimeError {
    fn from(error: CoreError) -> Self {
        Self::Capture(error)
    }
}

impl From<EquipmentOperationError> for EmptyCurtainRuntimeError {
    fn from(error: EquipmentOperationError) -> Self {
        Self::Operation(error)
    }
}

#[derive(Clone, Debug)]
pub(crate) enum TeamOperationError {
    State(TeamImportError),
    Capture(CoreError),
}

impl From<TeamImportError> for TeamOperationError {
    fn from(error: TeamImportError) -> Self {
        Self::State(error)
    }
}

impl From<CoreError> for TeamOperationError {
    fn from(error: CoreError) -> Self {
        Self::Capture(error)
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct SettingsMutationEffects {
    settings: bool,
    technical: bool,
    main: bool,
    history: bool,
}

impl SettingsMutationEffects {
    const NONE: Self = Self {
        settings: false,
        technical: false,
        main: false,
        history: false,
    };
    const SETTINGS: Self = Self {
        settings: true,
        technical: false,
        main: false,
        history: false,
    };
    const MAIN: Self = Self {
        settings: false,
        technical: false,
        main: true,
        history: false,
    };
    const SETTINGS_AND_MAIN: Self = Self {
        settings: true,
        technical: false,
        main: true,
        history: false,
    };
    const SETTINGS_AND_TECHNICAL: Self = Self {
        settings: true,
        technical: true,
        main: false,
        history: false,
    };
}

/// Maximum coalescing latency for a changed HUD projection. The stream checks
/// only cheap revisions at this cadence and skips full snapshots while idle.
pub(crate) const TECHNICAL_STREAM_INTERVAL_MS: u32 = 100;
pub(crate) const MAX_ACTIVE_STREAMS: usize = 128;
const MAX_PENDING_STREAMS: usize = 128;
const HUD_BASE_INITIAL_HEIGHT: u16 = 58;
// Keeps the five-row module editor and width field fully visible even when
// every HUD module is hidden. The WebView boundary clips HTML overlays.
const HUD_EDITOR_MIN_HEIGHT: u16 = 260;
const HUD_EDITOR_MODULE_HEADER_HEIGHT: u16 = 20;
const HUD_SUMMARY_HEIGHT: u16 = 64;
const HUD_CHARACTERS_HEIGHT: u16 = 116;
const HUD_OPTIONAL_TITLE_HEIGHT: u16 = 22;
const HUD_OPTIONAL_STATUS_HEIGHT: u16 = 22;
const HUD_MINI_TIMELINE_HEIGHT: u16 = 42;
const MAIN_DPS_DETAIL_CACHE_CAPACITY: usize = 4;
/// The desktop exposes a live packet-inspection page, so its capture must emit
/// the bounded debug-packet projection as well as semantic observations.
const DESKTOP_PACKET_EMISSION_MODE: PacketEmissionMode = PacketEmissionMode::FullDebug;
type EmptyCurtainDataSnapshot = (
    Vec<EmptyCurtainItem>,
    Vec<EmptyCurtainCharacter>,
    Arc<EquipmentCatalog>,
);

#[derive(Clone)]
pub(crate) struct AppState(Arc<AppStateInner>);

pub(crate) struct ReplayImportReservation {
    state: AppState,
    active: bool,
}

#[derive(Debug)]
pub(crate) enum ReplayImportError {
    RuntimeUnavailable,
    Capture(CoreError),
    JsonImport(CaptureImportError),
}

#[derive(Clone, Debug)]
pub(crate) enum PresentationError {
    StateUnavailable,
    Capture(CoreError),
    RoundUnavailable,
    RoundTooLarge,
}

impl From<CoreError> for PresentationError {
    fn from(error: CoreError) -> Self {
        Self::Capture(error)
    }
}

impl From<CoreError> for ReplayImportError {
    fn from(error: CoreError) -> Self {
        Self::Capture(error)
    }
}

impl ReplayImportError {
    fn into_core(self) -> CoreError {
        match self {
            Self::RuntimeUnavailable => CoreError::new(
                CoreErrorCode::CaptureStateUnavailable,
                "replay import runtime unavailable",
            ),
            Self::Capture(error) => error,
            Self::JsonImport(error) => {
                CoreError::new(CoreErrorCode::SystemProbeFailed, error.to_string())
            }
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct StreamRevision {
    capture: u64,
    presentation: u64,
}

#[derive(Clone, Debug, Hash, PartialEq, Eq)]
struct StreamIdentity {
    owner_window: String,
    subscription_key: String,
}

struct StreamEntry {
    generation: u64,
    stop: Arc<AtomicBool>,
}

struct PendingStreamEntry {
    stop: Arc<AtomicBool>,
}

#[derive(Default)]
struct StreamSlot {
    current: Option<StreamEntry>,
    latest_generation: u64,
    pending: HashMap<u64, PendingStreamEntry>,
}

#[derive(Default)]
struct StreamRegistry {
    next_generation: AtomicU64,
    entries: Mutex<HashMap<StreamIdentity, StreamSlot>>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum StreamRegistryError {
    InvalidIdentity,
    CapacityUnavailable,
    RuntimeUnavailable,
}

#[derive(Clone)]
pub(crate) struct StreamRegistration {
    identity: StreamIdentity,
    generation: u64,
    stop: Arc<AtomicBool>,
    active: Arc<AtomicBool>,
}

impl StreamRegistration {
    pub(crate) fn generation(&self) -> u64 {
        self.generation
    }

    pub(crate) fn stop_token(&self) -> Arc<AtomicBool> {
        Arc::clone(&self.stop)
    }

    pub(crate) fn activation_token(&self) -> Arc<AtomicBool> {
        Arc::clone(&self.active)
    }

    pub(crate) fn cancel(&self) {
        self.stop.store(true, Ordering::Release);
    }

    #[cfg(test)]
    fn is_cancelled(&self) -> bool {
        self.stop.load(Ordering::Acquire)
    }
}

impl StreamRegistry {
    fn reserve(
        &self,
        owner_window: &str,
        subscription_key: &str,
    ) -> Result<StreamRegistration, StreamRegistryError> {
        if !valid_stream_identity(owner_window, subscription_key) {
            return Err(StreamRegistryError::InvalidIdentity);
        }
        let generation = self
            .next_generation
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |current| {
                current.checked_add(1)
            })
            .map_err(|_| StreamRegistryError::RuntimeUnavailable)?
            + 1;
        let identity = StreamIdentity {
            owner_window: owner_window.to_owned(),
            subscription_key: subscription_key.to_owned(),
        };
        let stop = Arc::new(AtomicBool::new(false));
        let mut entries = self.lock_entries()?;
        let pending_count = entries
            .values()
            .map(|slot| slot.pending.len())
            .sum::<usize>();
        if pending_count >= MAX_PENDING_STREAMS {
            return Err(StreamRegistryError::CapacityUnavailable);
        }
        if !entries.contains_key(&identity) && entries.len() >= MAX_ACTIVE_STREAMS {
            return Err(StreamRegistryError::CapacityUnavailable);
        }
        let slot = entries.entry(identity.clone()).or_default();
        slot.latest_generation = generation;
        slot.pending.insert(
            generation,
            PendingStreamEntry {
                stop: Arc::clone(&stop),
            },
        );
        drop(entries);

        Ok(StreamRegistration {
            identity,
            generation,
            stop,
            active: Arc::new(AtomicBool::new(false)),
        })
    }

    fn activate(&self, registration: &StreamRegistration) -> Result<bool, StreamRegistryError> {
        let mut cancelled = Vec::new();
        let activated =
            {
                let mut entries = self.lock_entries()?;
                let Some(slot) = entries.get_mut(&registration.identity) else {
                    registration.cancel();
                    return Ok(false);
                };
                let is_latest = slot.latest_generation == registration.generation
                    && slot
                        .pending
                        .get(&registration.generation)
                        .is_some_and(|pending| Arc::ptr_eq(&pending.stop, &registration.stop));
                if !is_latest || registration.stop.load(Ordering::Acquire) {
                    slot.pending.remove(&registration.generation);
                    let remove_slot = slot.current.is_none() && slot.pending.is_empty();
                    if remove_slot {
                        entries.remove(&registration.identity);
                    }
                    false
                } else {
                    slot.pending.remove(&registration.generation);
                    cancelled.extend(slot.pending.drain().map(|(generation, pending)| {
                        StreamEntry {
                            generation,
                            stop: pending.stop,
                        }
                    }));
                    if let Some(previous) = slot.current.replace(StreamEntry {
                        generation: registration.generation,
                        stop: Arc::clone(&registration.stop),
                    }) {
                        cancelled.push(previous);
                    }
                    true
                }
            };

        for entry in cancelled {
            cancel_stream_entry(entry);
        }
        if activated {
            registration.active.store(true, Ordering::Release);
        } else {
            registration.cancel();
        }
        Ok(activated)
    }

    fn finish(&self, registration: &StreamRegistration) -> Result<bool, StreamRegistryError> {
        let removed = {
            let mut entries = self.lock_entries()?;
            if let Some(slot) = entries.get_mut(&registration.identity) {
                slot.pending.remove(&registration.generation);
                let is_current = slot.current.as_ref().is_some_and(|current| {
                    current.generation == registration.generation
                        && Arc::ptr_eq(&current.stop, &registration.stop)
                });
                if is_current {
                    slot.current.take();
                }
                let remove_slot = slot.current.is_none() && slot.pending.is_empty();
                if remove_slot {
                    entries.remove(&registration.identity);
                }
                is_current
            } else {
                false
            }
        };
        Ok(removed)
    }

    fn stop(
        &self,
        owner_window: &str,
        subscription_key: &str,
    ) -> Result<bool, StreamRegistryError> {
        if !valid_stream_identity(owner_window, subscription_key) {
            return Err(StreamRegistryError::InvalidIdentity);
        }
        let identity = StreamIdentity {
            owner_window: owner_window.to_owned(),
            subscription_key: subscription_key.to_owned(),
        };
        let removed = {
            let mut entries = self.lock_entries()?;
            entries.remove(&identity)
        };
        let Some(slot) = removed else {
            return Ok(false);
        };
        cancel_stream_slot(slot);
        Ok(true)
    }

    fn stop_for_window(&self, owner_window: &str) -> Result<usize, StreamRegistryError> {
        if !valid_stream_owner(owner_window) {
            return Err(StreamRegistryError::InvalidIdentity);
        }
        let removed = {
            let mut entries = self.lock_entries()?;
            let identities = entries
                .keys()
                .filter(|identity| identity.owner_window == owner_window)
                .cloned()
                .collect::<Vec<_>>();
            identities
                .into_iter()
                .filter_map(|identity| entries.remove(&identity))
                .collect::<Vec<_>>()
        };
        let count = removed.len();
        for slot in removed {
            cancel_stream_slot(slot);
        }
        Ok(count)
    }

    fn shutdown(&self) -> Result<usize, StreamRegistryError> {
        let removed = {
            let mut entries = self.lock_entries()?;
            std::mem::take(&mut *entries)
        };
        let count = removed.len();
        for slot in removed.into_values() {
            cancel_stream_slot(slot);
        }
        Ok(count)
    }

    fn lock_entries(
        &self,
    ) -> Result<MutexGuard<'_, HashMap<StreamIdentity, StreamSlot>>, StreamRegistryError> {
        match self.entries.lock() {
            Ok(entries) => Ok(entries),
            Err(mut poison) => {
                let removed = std::mem::take(&mut **poison.get_mut());
                self.entries.clear_poison();
                drop(poison);
                for slot in removed.into_values() {
                    cancel_stream_slot(slot);
                }
                Err(StreamRegistryError::RuntimeUnavailable)
            }
        }
    }

    #[cfg(test)]
    fn len(&self) -> Result<usize, StreamRegistryError> {
        Ok(self.lock_entries()?.len())
    }
}

fn cancel_stream_slot(slot: StreamSlot) {
    if let Some(current) = slot.current {
        cancel_stream_entry(current);
    }
    for (generation, pending) in slot.pending {
        cancel_stream_entry(StreamEntry {
            generation,
            stop: pending.stop,
        });
    }
}

fn cancel_stream_entry(entry: StreamEntry) {
    entry.stop.store(true, Ordering::Release);
}

fn valid_stream_identity(owner_window: &str, subscription_key: &str) -> bool {
    valid_stream_owner(owner_window)
        && (1..=128).contains(&subscription_key.len())
        && subscription_key
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b':' | b'.'))
}

fn valid_stream_owner(owner_window: &str) -> bool {
    (1..=64).contains(&owner_window.len())
        && owner_window
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct MainDpsStreamRevision {
    pub(crate) capture: u64,
    pub(crate) packet: u64,
    pub(crate) presentation: u64,
    pub(crate) history: u64,
    pub(crate) main: u64,
}

#[derive(Clone)]
pub(crate) struct MainDpsReadout {
    pub(crate) hud: HudSnapshot,
    pub(crate) has_hits: bool,
    pub(crate) game_paused: bool,
    pub(crate) damage_attribution: DamageAttributionSummary,
    pub(crate) separate_reaction_damage: bool,
    pub(crate) character_durations: HashMap<u32, f64>,
}

#[derive(Clone, Debug)]
pub(crate) struct IslandNoticeState {
    pub(crate) id: String,
    pub(crate) tone: &'static str,
    pub(crate) message_key: &'static str,
    pub(crate) message_arguments: Vec<String>,
    pub(crate) undo_token: Option<String>,
    pub(crate) expires_at: Instant,
}

#[derive(Default)]
struct IslandNoticeRuntime {
    revision: AtomicU64,
    notice: Mutex<Option<IslandNoticeState>>,
}

impl IslandNoticeRuntime {
    fn publish(
        &self,
        tone: &'static str,
        message_key: &'static str,
        message_arguments: Vec<String>,
        undo_token: Option<String>,
    ) -> String {
        // Ephemeral notification state is safe to reset after an interrupted
        // mutation. The replacement is the sole externally visible change.
        let (mut notice, _) = self.lock_discarding_poison();
        let revision = self.bump_revision();
        let id = format!("notice-{revision:016x}");
        *notice = Some(IslandNoticeState {
            id: id.clone(),
            tone,
            message_key,
            message_arguments,
            undo_token,
            expires_at: Instant::now() + ISLAND_NOTICE_WINDOW,
        });
        id
    }

    fn current(&self) -> Option<IslandNoticeState> {
        let (mut notice, discarded_notice) = self.lock_discarding_poison();
        let expired = notice
            .as_ref()
            .is_some_and(|notice| Instant::now() > notice.expires_at);
        if expired {
            notice.take();
        }
        if discarded_notice || expired {
            self.bump_revision();
        }
        notice.clone()
    }

    fn dismiss(&self, id: &str) -> bool {
        let (mut notice, discarded_notice) = self.lock_discarding_poison();
        if discarded_notice {
            self.bump_revision();
            return true;
        }
        if notice.as_ref().is_some_and(|notice| notice.id == id) {
            notice.take();
            self.bump_revision();
            return true;
        }
        false
    }

    fn lock_discarding_poison(&self) -> (MutexGuard<'_, Option<IslandNoticeState>>, bool) {
        match self.notice.lock() {
            Ok(notice) => (notice, false),
            Err(mut poison) => {
                let discarded_notice = poison.get_ref().is_some();
                **poison.get_mut() = None;
                self.notice.clear_poison();
                (poison.into_inner(), discarded_notice)
            }
        }
    }

    fn bump_revision(&self) -> u64 {
        self.revision.fetch_add(1, Ordering::AcqRel) + 1
    }

    #[cfg(test)]
    fn revision(&self) -> u64 {
        self.revision.load(Ordering::Acquire)
    }
}

#[derive(Clone)]
struct PausedPresentation {
    state: Arc<CombatState>,
    packet_revision: PacketStreamRevision,
}

#[derive(Clone)]
struct SelectedRoundPresentation {
    record_id: String,
    history_revision: u64,
    state: Arc<CombatState>,
}

#[derive(Default)]
struct PresentationModeState {
    paused: Option<PausedPresentation>,
    selected_round: Option<SelectedRoundPresentation>,
    selected_outgoing_revision: u64,
}

#[derive(Clone, Default)]
struct PresentationModeSnapshot {
    paused: Option<PausedPresentation>,
    selected_round: Option<SelectedRoundPresentation>,
}

impl PresentationModeSnapshot {
    fn processing_paused(&self) -> bool {
        self.paused.is_some()
    }

    fn selected_round_id(&self) -> Option<String> {
        self.selected_round
            .as_ref()
            .map(|selection| selection.record_id.clone())
    }

    fn presented_state(&self) -> Option<Arc<CombatState>> {
        self.selected_round
            .as_ref()
            .map(|selection| Arc::clone(&selection.state))
            .or_else(|| self.paused.as_ref().map(|paused| Arc::clone(&paused.state)))
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct AbyssPresentationState {
    selected: Option<AbyssHalf>,
    observed: Option<AbyssHalf>,
}

#[derive(Default)]
struct MainDpsDetailsState {
    character: MainDpsDetailRequest,
    team: MainDpsDetailRequest,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct MainDpsDetailRequest {
    pub(crate) character_id: Option<u32>,
    pub(crate) filter: CombatDetailFilter,
    pub(crate) skill_filter: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum MainDpsDetailKind {
    Character,
    Team,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum DesktopWindowKind {
    MainDps,
    Hud,
    Console,
    AbyssValues,
    CharacterDetails,
    TeamDetails,
}

#[derive(Clone, Debug)]
pub(crate) struct HistoryRoundIndex {
    pub(crate) id: String,
    pub(crate) display_time: String,
    pub(crate) abyss_floor: Option<u32>,
    pub(crate) has_details: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct MainDpsDetailCacheKey {
    revision: MainDpsStreamRevision,
    kind: MainDpsDetailKind,
    request: MainDpsDetailRequest,
    offset: usize,
    limit: usize,
}

struct MainDpsDetailCache {
    key: MainDpsDetailCacheKey,
    snapshot: Arc<MainDpsDetailSnapshot>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct TimelineProjectionCacheKey {
    revision: MainDpsStreamRevision,
    scope: TimelineScope,
    bucket_seconds_bits: u32,
    subtract_time_stop: bool,
    language: &'static str,
}

struct TimelineProjectionCache {
    key: TimelineProjectionCacheKey,
    projection: Arc<TimelineProjection>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct MainReadoutCacheKey {
    revision: MainDpsStreamRevision,
}

struct MainReadoutCache {
    key: MainReadoutCacheKey,
    readout: Arc<MainDpsReadout>,
}

impl HistoryRoundIndex {
    fn from_storage(record: &HistoryIndexRecord) -> Self {
        Self {
            id: record.id.clone(),
            display_time: record.display_time.clone(),
            abyss_floor: record.abyss_floor,
            has_details: record.has_details,
        }
    }

    #[cfg(test)]
    pub(crate) fn for_test(id: &str, has_details: bool) -> Self {
        Self {
            id: id.to_owned(),
            display_time: id.to_owned(),
            abyss_floor: None,
            has_details,
        }
    }
}

struct MainRoundCache {
    revision: Option<u64>,
    index: Arc<Vec<HistoryRoundIndex>>,
}

impl Default for MainRoundCache {
    fn default() -> Self {
        Self {
            revision: None,
            index: Arc::new(Vec::new()),
        }
    }
}

/// Owns UI-only presentation state and its revision protocol. Rust combat
/// state remains authoritative in `LiveCaptureService`; this service stores
/// only frozen/selected projections and interaction requests.
#[derive(Default)]
struct PresentationState {
    revision: AtomicU64,
    main_revision: AtomicU64,
    /// Every History selection intent reserves a monotonically increasing
    /// generation before blocking I/O. Only the latest generation may commit.
    selection_generation: AtomicU64,
    mode: Mutex<PresentationModeState>,
    abyss: Mutex<AbyssPresentationState>,
    details: Mutex<MainDpsDetailsState>,
    cache: Mutex<Vec<MainDpsDetailCache>>,
    /// Bounded, exact-revision cache shared by every main-readout consumer.
    /// A miss is never served stale; racing projections are stored only when
    /// the revision is unchanged before and after projection.
    main_readout_cache: Mutex<Vec<MainReadoutCache>>,
    timeline_cache: Mutex<Vec<TimelineProjectionCache>>,
    #[cfg(test)]
    main_readout_projection_count: AtomicU64,
}

impl PresentationState {
    fn lock_mode(&self) -> (MutexGuard<'_, PresentationModeState>, bool) {
        match self.mode.lock() {
            Ok(mode) => (mode, false),
            Err(mut poison) => {
                **poison.get_mut() = PresentationModeState::default();
                self.mode.clear_poison();
                log::warn!("Presentation mode was reset after an interrupted update");
                (poison.into_inner(), true)
            }
        }
    }

    fn lock_abyss(&self) -> (MutexGuard<'_, AbyssPresentationState>, bool) {
        match self.abyss.lock() {
            Ok(abyss) => (abyss, false),
            Err(mut poison) => {
                **poison.get_mut() = AbyssPresentationState::default();
                self.abyss.clear_poison();
                log::warn!("Presentation abyss selection was reset after an interrupted update");
                (poison.into_inner(), true)
            }
        }
    }

    fn lock_details(&self) -> (MutexGuard<'_, MainDpsDetailsState>, bool) {
        match self.details.lock() {
            Ok(details) => (details, false),
            Err(mut poison) => {
                **poison.get_mut() = MainDpsDetailsState::default();
                self.details.clear_poison();
                log::warn!("Presentation detail requests were reset after an interrupted update");
                (poison.into_inner(), true)
            }
        }
    }

    fn lock_cache(&self) -> (MutexGuard<'_, Vec<MainDpsDetailCache>>, bool) {
        match self.cache.lock() {
            Ok(cache) => (cache, false),
            Err(mut poison) => {
                poison.get_mut().clear();
                self.cache.clear_poison();
                log::warn!("Presentation detail cache was discarded after an interrupted update");
                (poison.into_inner(), true)
            }
        }
    }

    fn lock_timeline_cache(&self) -> (MutexGuard<'_, Vec<TimelineProjectionCache>>, bool) {
        match self.timeline_cache.lock() {
            Ok(cache) => (cache, false),
            Err(mut poison) => {
                poison.get_mut().clear();
                self.timeline_cache.clear_poison();
                log::warn!("Timeline projection cache was discarded after an interrupted update");
                (poison.into_inner(), true)
            }
        }
    }

    fn lock_main_readout_cache(&self) -> (MutexGuard<'_, Vec<MainReadoutCache>>, bool) {
        match self.main_readout_cache.lock() {
            Ok(cache) => (cache, false),
            Err(mut poison) => {
                poison.get_mut().clear();
                self.main_readout_cache.clear_poison();
                log::warn!("Main readout cache was discarded after an interrupted update");
                (poison.into_inner(), true)
            }
        }
    }

    fn publish_technical_and_main(&self) {
        self.revision.fetch_add(1, Ordering::AcqRel);
        self.main_revision.fetch_add(1, Ordering::AcqRel);
    }

    fn publish_main(&self) {
        self.main_revision.fetch_add(1, Ordering::AcqRel);
    }
}

/// Serializes history persistence and owns revision/cache/retry/undo state.
/// Capture only cuts rounds; disk I/O and retry lifecycle stay here.
struct HistoryService {
    revision: AtomicU64,
    round_cache: Mutex<MainRoundCache>,
    transaction: Mutex<()>,
    archive_transaction: Mutex<()>,
    pending_archives: Mutex<VecDeque<PreparedHistoryArchive>>,
    undo: Mutex<Option<HistoryUndoEntry>>,
}

#[cfg(not(test))]
static HISTORY_TOMBSTONE_STARTUP_CLEANUP: Once = Once::new();

impl Default for HistoryService {
    fn default() -> Self {
        #[cfg(not(test))]
        HISTORY_TOMBSTONE_STARTUP_CLEANUP.call_once(|| {
            if let Err(error) = cleanup_orphaned_history_tombstones_at_startup() {
                log::warn!("orphaned History undo cleanup did not finish: {error}");
            }
        });
        Self {
            revision: AtomicU64::new(0),
            round_cache: Mutex::new(MainRoundCache::default()),
            transaction: Mutex::new(()),
            archive_transaction: Mutex::new(()),
            pending_archives: Mutex::new(VecDeque::new()),
            undo: Mutex::new(None),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum HistoryRuntimeError {
    Unavailable,
    ReplayActive,
    RetryQueueFull,
    Persistence,
    PermanentPersistence,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum HistoryPersistenceOutcome {
    Committed,
    CommittedWithMaintenanceWarning,
}

impl fmt::Display for HistoryRuntimeError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unavailable => formatter.write_str("History operation did not finish."),
            Self::ReplayActive => {
                formatter.write_str("History round cuts are unavailable during replay")
            }
            Self::RetryQueueFull => {
                formatter.write_str("History retry queue is full; the current round was kept live")
            }
            Self::Persistence => formatter.write_str("History persistence did not finish."),
            Self::PermanentPersistence => {
                formatter.write_str("History archive failed permanent validation.")
            }
        }
    }
}

impl std::error::Error for HistoryRuntimeError {}

fn next_live_abyss_selection(
    selected: Option<AbyssHalf>,
    observed: Option<AbyssHalf>,
    active: Option<AbyssHalf>,
) -> (Option<AbyssHalf>, Option<AbyssHalf>) {
    if active == observed {
        (selected, observed)
    } else {
        (active, active)
    }
}

fn subtract_time_stop_for_state(mode: DpsTimeMode, state: &CombatState) -> bool {
    matches!(mode, DpsTimeMode::TimeStopAdjusted)
        && matches!(
            state.combat_clock_health,
            CombatClockRuntimeHealth::Available | CombatClockRuntimeHealth::Recorded
        )
}

fn history_archive_policy(config: &UiConfig) -> HistoryArchivePolicy {
    HistoryArchivePolicy {
        requested_dps_time_mode: match config.dps_time_mode {
            DpsTimeMode::TimeStopAdjusted => DpsTimeBasis::SubtractTimeStop,
            DpsTimeMode::RealTime => DpsTimeBasis::WallClock,
        },
        separate_reaction_damage: config.separate_reaction_damage,
    }
}

#[cfg(test)]
fn selected_round_combat_state(
    rounds: &[HistoryRecord],
    selected_round_id: Option<&str>,
) -> Option<CombatState> {
    let record_id = selected_round_id?;
    rounds
        .iter()
        .find(|record| record.id == record_id)
        .and_then(|record| record.details.as_ref())
        .map(HistoryCombatDetails::to_combat_state)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum HudSettingOption {
    Title,
    TeamDps,
    Duration,
    TotalDamage,
    DamageTaken,
    CharacterRows,
    AbyssHalf,
    PassthroughState,
    MiniTimeline,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum HudPreset {
    Minimal,
    Standard,
    Detailed,
}

struct AppStateInner {
    desktop: DesktopRuntime,
    replay_import: ReplayImportRuntime,
    streams: StreamRegistry,
    live_capture: LiveCaptureService,
    mod_studio: ModStudioWorkspaceService,
    mod_loader: ModLoaderRuntimeService,
    equipment_catalog: Arc<EquipmentCatalog>,
    equipment_operation: EquipmentOperationService,
    settings: SettingsService,
    team_import: TeamImportService,
    update_runtime: UpdateRuntimeService,
    presentation: PresentationState,
    history: HistoryService,
    character_data: CharacterDataService,
    encrypted_ini: EncryptedIniService,
    diagnostics: DiagnosticsRuntime,
    session_undo: SessionUndoRuntime,
    island_notice: IslandNoticeRuntime,
}

struct HistoryUndoEntry {
    token: String,
    tombstone: HistoryDeleteTombstone,
    expires_at: Instant,
}

#[derive(Clone)]
struct SessionUndoEntry {
    token: String,
    state: CombatState,
    quality_source: CaptureQualitySource,
    expires_at: Instant,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum ReplayImportReservationState {
    #[default]
    Idle,
    ReplayImport,
    CaptureStart,
}

#[derive(Default)]
struct ReplayImportRuntime {
    reservation: Mutex<ReplayImportReservationState>,
}

impl ReplayImportRuntime {
    // The mutex protects only the short reservation transition. Capture waits,
    // dialogs, file setup, and replay startup always run after this guard drops.
    // Poison is sticky and fail-closed because reservation ownership is unknown.
    fn reserve(&self, requested: ReplayImportReservationState) -> Result<(), ReplayImportError> {
        let mut reservation = self
            .reservation
            .lock()
            .map_err(|_| ReplayImportError::RuntimeUnavailable)?;
        if *reservation != ReplayImportReservationState::Idle {
            return Err(ReplayImportError::Capture(CoreError::new(
                CoreErrorCode::CaptureAlreadyRunning,
                "capture or replay reservation is already active",
            )));
        }
        *reservation = requested;
        Ok(())
    }

    fn ensure_owned(
        &self,
        expected: ReplayImportReservationState,
    ) -> Result<(), ReplayImportError> {
        let reservation = self
            .reservation
            .lock()
            .map_err(|_| ReplayImportError::RuntimeUnavailable)?;
        if *reservation == expected {
            Ok(())
        } else {
            Err(ReplayImportError::RuntimeUnavailable)
        }
    }

    fn release(&self, expected: ReplayImportReservationState) -> Result<(), ReplayImportError> {
        let mut reservation = self
            .reservation
            .lock()
            .map_err(|_| ReplayImportError::RuntimeUnavailable)?;
        if *reservation != expected {
            return Err(ReplayImportError::RuntimeUnavailable);
        }
        *reservation = ReplayImportReservationState::Idle;
        Ok(())
    }
}

#[derive(Default)]
struct SessionUndoRuntime {
    sequence: AtomicU64,
    entry: Mutex<Option<SessionUndoEntry>>,
}

impl SessionUndoRuntime {
    // A poisoned entry is never projected or mutated. Sequence allocation happens
    // only while a healthy slot is held, and live-state reset/restore runs unlocked.
    fn ensure_available(&self) -> Result<(), SessionUndoError> {
        drop(
            self.entry
                .lock()
                .map_err(|_| SessionUndoError::RuntimeUnavailable)?,
        );
        Ok(())
    }

    fn remember(
        &self,
        state: CombatState,
        quality_source: CaptureQualitySource,
    ) -> Result<String, SessionUndoError> {
        let mut entry = self
            .entry
            .lock()
            .map_err(|_| SessionUndoError::RuntimeUnavailable)?;
        let sequence = self.sequence.fetch_add(1, Ordering::AcqRel) + 1;
        let token = format!("session-undo-{sequence:016x}");
        *entry = Some(SessionUndoEntry {
            token: token.clone(),
            state,
            quality_source,
            expires_at: Instant::now() + SESSION_UNDO_WINDOW,
        });
        Ok(token)
    }

    fn entry_for_restore(&self, token: &str) -> Result<SessionUndoEntry, SessionUndoError> {
        let mut entry = self
            .entry
            .lock()
            .map_err(|_| SessionUndoError::RuntimeUnavailable)?;
        let Some(current) = entry.as_ref() else {
            return Err(SessionUndoError::Missing);
        };
        if current.token != token {
            return Err(SessionUndoError::Missing);
        }
        if Instant::now() > current.expires_at {
            entry.take();
            return Err(SessionUndoError::Expired);
        }
        Ok(current.clone())
    }

    fn consume_matching(&self, token: &str) -> Result<bool, SessionUndoError> {
        let mut entry = self
            .entry
            .lock()
            .map_err(|_| SessionUndoError::RuntimeUnavailable)?;
        if entry.as_ref().is_some_and(|current| current.token == token) {
            entry.take();
            Ok(true)
        } else {
            Ok(false)
        }
    }

    fn clear(&self) -> Result<(), SessionUndoError> {
        self.entry
            .lock()
            .map_err(|_| SessionUndoError::RuntimeUnavailable)?
            .take();
        Ok(())
    }
}

pub(crate) const HISTORY_UNDO_WINDOW: Duration = Duration::from_secs(5);
pub(crate) const SESSION_UNDO_WINDOW: Duration = Duration::from_secs(5);
pub(crate) const ISLAND_NOTICE_WINDOW: Duration = Duration::from_secs(5);
/// FIFO retry queue for already-cut rounds. A full queue rejects the next
/// round boundary before capture state is detached, preserving live data.
const MAX_PENDING_HISTORY_ARCHIVES: usize = 64;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum SessionUndoError {
    Missing,
    Expired,
    Busy,
    NewData,
    StateUnavailable,
    RuntimeUnavailable,
}

impl SessionUndoError {
    fn into_core(self) -> CoreError {
        let detail = match self {
            Self::RuntimeUnavailable => "session undo runtime unavailable",
            Self::StateUnavailable => "live capture state unavailable during session undo",
            Self::Missing => "session undo entry missing",
            Self::Expired => "session undo entry expired",
            Self::Busy => "session undo blocked by active capture",
            Self::NewData => "session undo blocked by new capture data",
        };
        CoreError::new(CoreErrorCode::CaptureStateUnavailable, detail)
    }
}

impl Default for AppState {
    fn default() -> Self {
        Self::new(
            UiConfig::default(),
            LiveCaptureService::new(LiveCaptureResources::default()),
        )
    }
}

impl ReplayImportReservation {
    pub(crate) fn start(
        self,
        kind: CaptureReplayKind,
        path: PathBuf,
        replace_current: bool,
    ) -> Result<(), ReplayImportError> {
        self.finish_with(move |state| {
            if kind == CaptureReplayKind::Json {
                // Parse and validate the bounded document before stopping a
                // live owner or changing any authoritative session revision.
                let prepared =
                    prepare_capture_json_replay(&path).map_err(ReplayImportError::JsonImport)?;
                if replace_current {
                    state
                        .stop_active_capture_and_wait(Duration::from_secs(5))
                        .map_err(ReplayImportError::Capture)?;
                }
                state.request_diagnostics_prepared_json_replay(prepared)
            } else {
                if replace_current {
                    state
                        .stop_active_capture_and_wait(Duration::from_secs(5))
                        .map_err(ReplayImportError::Capture)?;
                }
                state.request_diagnostics_replay(kind, path)
            }
        })
    }

    fn finish_with(
        mut self,
        operation: impl FnOnce(&AppState) -> Result<(), ReplayImportError>,
    ) -> Result<(), ReplayImportError> {
        self.state
            .0
            .replay_import
            .ensure_owned(ReplayImportReservationState::ReplayImport)?;
        let result = operation(&self.state);
        self.release()?;
        result
    }

    fn release(&mut self) -> Result<(), ReplayImportError> {
        if !self.active {
            return Ok(());
        }
        self.state
            .0
            .replay_import
            .release(ReplayImportReservationState::ReplayImport)?;
        self.active = false;
        Ok(())
    }
}

impl Drop for ReplayImportReservation {
    fn drop(&mut self) {
        let _ = self.release();
    }
}

impl AppState {
    pub(crate) fn new(config: UiConfig, live_capture: LiveCaptureService) -> Self {
        Self::new_with_config_path(config, live_capture, config::config_path())
    }

    fn new_with_config_path(
        mut config: UiConfig,
        live_capture: LiveCaptureService,
        config_path: PathBuf,
    ) -> Self {
        config = config.sanitized();
        live_capture.set_history_archive_policy(history_archive_policy(&config));
        let capture_devices = enumerate_devices()
            .map(|devices| devices.iter().map(CaptureDeviceSnapshot::from).collect());
        if capture_devices.is_err() {
            log::warn!("Capture device catalog is unavailable at startup");
        }
        let equipment_catalog = load_equipment_catalog(std::path::Path::new(
            EQUIPMENT_CATALOG_PATH,
        ))
        .unwrap_or_else(|error| {
            log::error!("load Console equipment catalog for Tauri failed: {error:#}");
            EquipmentCatalog::default()
        });
        Self(Arc::new(AppStateInner {
            desktop: DesktopRuntime::new(
                config
                    .hud_always_on_top
                    .expect("sanitized HUD always-on-top state"),
            ),
            replay_import: ReplayImportRuntime::default(),
            streams: StreamRegistry::default(),
            live_capture,
            mod_studio: ModStudioWorkspaceService::default(),
            mod_loader: ModLoaderRuntimeService::default(),
            equipment_catalog: Arc::new(equipment_catalog),
            equipment_operation: EquipmentOperationService::default(),
            settings: SettingsService::new(config, config_path, capture_devices),
            team_import: TeamImportService::default(),
            update_runtime: UpdateRuntimeService::default(),
            presentation: PresentationState::default(),
            history: HistoryService::default(),
            character_data: CharacterDataService::new(software_dir().join(CHARACTER_DATA_PATH)),
            encrypted_ini: EncryptedIniService::default(),
            diagnostics: DiagnosticsRuntime::default(),
            session_undo: SessionUndoRuntime::default(),
            island_notice: IslandNoticeRuntime::default(),
        }))
    }

    pub(crate) fn snapshot(&self) -> Result<TechnicalSnapshot, CoreError> {
        let sequence = self.next_sequence();
        let config = self.ui_config();
        let hud_config = config.hud.clone();
        let selected_abyss_half = self.abyss_presentation_snapshot().selected;
        let supported_locales = Language::all()
            .iter()
            .map(|language| language.code())
            .collect();

        Ok(TechnicalSnapshot {
            contract_version: TECHNICAL_CONTRACT_VERSION,
            sequence: sequence.to_string(),
            bridge_status: "ready",
            adapter_version: env!("CARGO_PKG_VERSION"),
            window_label: HUD_WINDOW_LABEL,
            uptime_ms: self.uptime_ms().to_string(),
            stream_interval_ms: TECHNICAL_STREAM_INTERVAL_MS,
            supported_locales,
            window: HudWindowSnapshot {
                passthrough: self.passthrough(),
                always_on_top: self.always_on_top(),
            },
            capture: self.0.live_capture.status().into(),
            hud: self.with_main_presented_state(|state| {
                let mut hud = project_hud(
                    state,
                    &hud_config,
                    &HashSet::new(),
                    HudProjectionOptions {
                        dps_time_basis: DpsTimeBasis::from_subtract_time_stop(
                            subtract_time_stop_for_state(config.dps_time_mode, state),
                        ),
                        separate_reaction_damage: config.separate_reaction_damage,
                        include_max_hp_reduction_in_total_damage: config
                            .include_max_hp_reduction_in_total_damage,
                        selected_abyss_half,
                        preview_when_empty: !self.passthrough(),
                        timeline_bucket_seconds: f64::from(sanitize_timeline_bucket_seconds(
                            config.timeline_bucket_seconds,
                        )),
                    },
                );
                let resources = self.live_capture_resources();
                for row in &mut hud.characters {
                    row.color = resources
                        .characters
                        .get(&row.character_id)
                        .and_then(|character| character.color.clone());
                }
                hud
            })?,
        })
    }

    pub(crate) fn next_sequence(&self) -> u64 {
        self.0.desktop.next_sequence()
    }

    pub(crate) fn publish_island_notice(
        &self,
        tone: &'static str,
        message_key: &'static str,
        message_arguments: Vec<String>,
        undo_token: Option<String>,
    ) -> String {
        self.0
            .island_notice
            .publish(tone, message_key, message_arguments, undo_token)
    }

    pub(crate) fn island_notice(&self) -> Option<IslandNoticeState> {
        self.0.island_notice.current()
    }

    pub(crate) fn dismiss_island_notice(&self, id: &str) -> bool {
        self.0.island_notice.dismiss(id)
    }

    pub(crate) fn ui_config_snapshot(&self) -> UiConfig {
        self.ui_config()
    }

    pub(crate) fn capture_device_catalog_status(&self) -> (usize, bool) {
        let snapshot = self.0.settings.device_catalog_snapshot(|| {
            self.publish_settings_effects(SettingsMutationEffects::SETTINGS_AND_MAIN);
        });
        (snapshot.devices.len(), snapshot.available)
    }

    pub(crate) fn set_onboarding_progress(
        &self,
        step: usize,
        done: bool,
    ) -> Result<bool, SettingsServiceError> {
        self.0.desktop.set_onboarding_step(step);
        self.bump_main_dps_revision();
        self.update_ui_config_with_effects(SettingsMutationEffects::NONE, |config| {
            config.onboarding_done = done;
        })
    }

    pub(crate) fn finish_onboarding(
        &self,
        preset: HudPreset,
    ) -> Result<bool, SettingsServiceError> {
        let changed = self.update_ui_config(|config| {
            let width = config.hud.width;
            let module_order = config.hud.module_order.clone();
            let mut hud = match preset {
                HudPreset::Minimal => HudConfig::minimal(),
                HudPreset::Standard => HudConfig::default(),
                HudPreset::Detailed => HudConfig::detailed(),
            };
            hud.width = width;
            hud.module_order = module_order;
            config.hud = hud;
            config.onboarding_done = true;
        })?;
        self.0.desktop.set_onboarding_step(3);
        self.bump_main_dps_revision();
        Ok(changed)
    }

    pub(crate) fn onboarding_step(&self) -> usize {
        self.0.desktop.onboarding_step()
    }

    pub(crate) fn console_window_geometry(&self) -> (Option<[f32; 2]>, Option<[f32; 2]>) {
        let config = self.ui_config();
        (config.console_window_size, config.console_window_position)
    }

    pub(crate) fn abyss_window_geometry(&self) -> (Option<[f32; 2]>, Option<[f32; 2]>) {
        let config = self.ui_config();
        (config.abyss_window_size, config.abyss_window_position)
    }

    pub(crate) fn set_abyss_window_geometry(
        &self,
        size: [f32; 2],
        position: [f32; 2],
    ) -> Result<bool, SettingsServiceError> {
        self.update_ui_config_with_effects(SettingsMutationEffects::NONE, |config| {
            config.abyss_window_size = Some(size);
            config.abyss_window_position = Some(position);
        })
    }

    pub(crate) fn set_console_window_geometry(
        &self,
        size: [f32; 2],
        position: [f32; 2],
    ) -> Result<bool, SettingsServiceError> {
        self.update_ui_config_with_effects(SettingsMutationEffects::NONE, |config| {
            config.console_window_size = Some(size);
            config.console_window_position = Some(position);
        })
    }

    pub(crate) fn set_hit_detail_columns(
        &self,
        columns: nte_dps_tool::storage::config::HitDetailColumnsConfig,
    ) -> Result<bool, SettingsServiceError> {
        self.update_ui_config_with_effects(SettingsMutationEffects::MAIN, |config| {
            config.hit_detail_columns = columns;
        })
    }

    pub(crate) fn main_dps_detail_window_geometry(
        &self,
        kind: MainDpsDetailKind,
    ) -> (Option<[f32; 2]>, Option<[f32; 2]>) {
        let config = self.ui_config();
        match kind {
            MainDpsDetailKind::Character => (
                config.hit_detail_window_size,
                config.hit_detail_window_position,
            ),
            MainDpsDetailKind::Team => (
                config.team_hit_detail_window_size,
                config.team_hit_detail_window_position,
            ),
        }
    }

    pub(crate) fn set_main_dps_detail_window_geometry(
        &self,
        size: [f32; 2],
        position: [f32; 2],
        kind: MainDpsDetailKind,
    ) -> Result<bool, SettingsServiceError> {
        self.update_ui_config_with_effects(SettingsMutationEffects::NONE, |config| match kind {
            MainDpsDetailKind::Character => {
                config.hit_detail_window_size = Some(size);
                config.hit_detail_window_position = Some(position);
            }
            MainDpsDetailKind::Team => {
                config.team_hit_detail_window_size = Some(size);
                config.team_hit_detail_window_position = Some(position);
            }
        })
    }

    pub(crate) fn live_capture_status(&self) -> LiveCaptureStatus {
        self.0.live_capture.status()
    }

    pub(crate) fn combat_clock_health(
        &self,
    ) -> Result<nte_dps_tool::engine::model::CombatClockRuntimeHealth, CoreError> {
        self.0.live_capture.combat_clock_health()
    }

    pub(crate) fn main_presented_combat_clock_health(
        &self,
    ) -> Result<CombatClockRuntimeHealth, CoreError> {
        self.with_main_presented_state(|state| state.combat_clock_health)
    }

    pub(crate) fn replay_running(&self) -> Result<bool, CoreError> {
        self.0.live_capture.replay_running()
    }

    fn presentation_mode_snapshot(&self) -> PresentationModeSnapshot {
        let outgoing_revision = self.0.live_capture.outgoing_hit_revision();
        let history_revision = self.history_revision();
        let (mut mode, recovered) = self.0.presentation.lock_mode();
        let stale_selection = mode.selected_round.as_ref().is_some_and(|selection| {
            selection.history_revision != history_revision
                || (mode.paused.is_none() && outgoing_revision != mode.selected_outgoing_revision)
        });
        if stale_selection {
            mode.selected_round = None;
        }
        let snapshot = PresentationModeSnapshot {
            paused: mode.paused.clone(),
            selected_round: mode.selected_round.clone(),
        };
        drop(mode);
        if recovered || stale_selection {
            self.0.presentation.publish_technical_and_main();
        }
        snapshot
    }

    pub(crate) fn main_processing_paused(&self) -> bool {
        self.presentation_mode_snapshot().processing_paused()
    }

    pub(crate) fn main_paused_event_counts(&self) -> Result<(u64, u64), CoreError> {
        let paused = self.presentation_mode_snapshot().paused;
        let Some(paused) = paused else {
            return Ok((0, 0));
        };
        self.0.live_capture.with_state(|live| {
            let semantic = live
                .hits_generation
                .wrapping_sub(paused.state.hits_generation)
                .wrapping_add(
                    live.empty_curtain_generation
                        .wrapping_sub(paused.state.empty_curtain_generation),
                )
                .wrapping_add(
                    live.empty_curtain_characters_generation
                        .wrapping_sub(paused.state.empty_curtain_characters_generation),
                );
            let debug = live
                .packets_generation
                .wrapping_sub(paused.state.packets_generation);
            (semantic, debug)
        })
    }

    pub(crate) fn set_main_processing_paused(
        &self,
        paused: bool,
    ) -> Result<bool, PresentationError> {
        let (mut mode, recovered) = self.0.presentation.lock_mode();
        if recovered {
            drop(mode);
            self.0.presentation.publish_technical_and_main();
            return Err(PresentationError::StateUnavailable);
        }
        if mode.paused.is_some() == paused {
            return Ok(false);
        }
        if !paused {
            mode.paused = None;
            drop(mode);
            self.0.presentation.publish_technical_and_main();
            return Ok(true);
        }
        drop(mode);

        // The capture snapshot can be large. Build it without holding any
        // Presentation lock, then re-check the requested transition.
        let frozen = self
            .0
            .live_capture
            .with_packet_state(|packet_revision, _, state| PausedPresentation {
                state: Arc::new(state.clone()),
                packet_revision,
            })
            .map_err(PresentationError::Capture)?;
        let (mut mode, recovered) = self.0.presentation.lock_mode();
        if recovered {
            drop(mode);
            self.0.presentation.publish_technical_and_main();
            return Err(PresentationError::StateUnavailable);
        }
        if mode.paused.is_some() {
            return Ok(false);
        }
        mode.paused = Some(frozen);
        drop(mode);
        self.0.presentation.publish_technical_and_main();
        Ok(true)
    }

    pub(crate) fn main_selected_round_id(&self) -> Option<String> {
        self.presentation_mode_snapshot().selected_round_id()
    }

    pub(crate) fn set_main_selected_round_id(
        &self,
        record_id: Option<String>,
    ) -> Result<bool, PresentationError> {
        let operation = self.reserve_main_round_selection();
        self.set_main_selected_round_id_for_operation(record_id, operation)
    }

    pub(crate) fn reserve_main_round_selection(&self) -> u64 {
        self.0
            .presentation
            .selection_generation
            .fetch_add(1, Ordering::AcqRel)
            .wrapping_add(1)
    }

    pub(crate) fn set_main_selected_round_id_for_operation(
        &self,
        record_id: Option<String>,
        operation: u64,
    ) -> Result<bool, PresentationError> {
        self.set_main_selected_round_id_with(record_id, operation, |record_id| {
            let mut record = load_history_record_by_id_for_interactive_selection(record_id)
                .map_err(|error| {
                    match error {
                    nte_dps_tool::storage::history::HistoryRecordLoadError::DetailsTooLarge {
                        ..
                    }
                    | nte_dps_tool::storage::history::HistoryRecordLoadError::DetailHitsTooLarge {
                        ..
                    } => PresentationError::RoundTooLarge,
                    _ => PresentationError::RoundUnavailable,
                }
                })?
                .filter(|record| record.details.is_some())
                .ok_or(PresentationError::RoundUnavailable)?;
            record
                .details
                .take()
                .map(HistoryCombatDetails::into_combat_state)
                .ok_or(PresentationError::RoundUnavailable)
        })
    }

    fn set_main_selected_round_id_with(
        &self,
        record_id: Option<String>,
        operation: u64,
        load: impl FnOnce(&str) -> Result<CombatState, PresentationError>,
    ) -> Result<bool, PresentationError> {
        let history_revision = self.history_revision();
        if self
            .0
            .presentation
            .selection_generation
            .load(Ordering::Acquire)
            != operation
        {
            return Ok(false);
        }
        let (mode, recovered) = self.0.presentation.lock_mode();
        if recovered {
            drop(mode);
            self.0.presentation.publish_technical_and_main();
            return Err(PresentationError::StateUnavailable);
        }
        if mode.selected_round.as_ref().is_some_and(|value| {
            Some(value.record_id.as_str()) == record_id.as_deref()
                && value.history_revision == history_revision
        }) || (record_id.is_none() && mode.selected_round.is_none())
        {
            return Ok(false);
        }
        drop(mode);

        // History loading and reconstruction are blocking/O(N) and remain
        // outside every Presentation lock. Superseded failures are ignored:
        // their newer intent owns both the visible state and command snapshot.
        let selection = match record_id {
            Some(record_id) => {
                let state = match load(&record_id) {
                    Ok(state) => state,
                    Err(_error)
                        if self
                            .0
                            .presentation
                            .selection_generation
                            .load(Ordering::Acquire)
                            != operation =>
                    {
                        return Ok(false);
                    }
                    Err(error) => return Err(error),
                };
                Some(SelectedRoundPresentation {
                    record_id,
                    history_revision,
                    state: Arc::new(state),
                })
            }
            None => None,
        };
        let (mut mode, recovered) = self.0.presentation.lock_mode();
        if recovered {
            drop(mode);
            self.0.presentation.publish_technical_and_main();
            return Err(PresentationError::StateUnavailable);
        }
        if self.history_revision() != history_revision
            || self
                .0
                .presentation
                .selection_generation
                .load(Ordering::Acquire)
                != operation
        {
            return Ok(false);
        }
        if mode
            .selected_round
            .as_ref()
            .map(|value| (value.record_id.as_str(), value.history_revision))
            == selection
                .as_ref()
                .map(|value| (value.record_id.as_str(), value.history_revision))
        {
            return Ok(false);
        }
        mode.selected_outgoing_revision = self.0.live_capture.outgoing_hit_revision();
        mode.selected_round = selection;
        drop(mode);
        self.0.presentation.publish_technical_and_main();
        Ok(true)
    }

    pub(crate) fn main_round_index(&self) -> Arc<Vec<HistoryRoundIndex>> {
        let index = self.load_main_round_index();
        let stale_selection = self
            .main_selected_round_id()
            .is_some_and(|id| !index.iter().any(|record| record.id == id));
        if stale_selection {
            let _ = self.set_main_selected_round_id(None);
        }
        index
    }

    fn load_main_round_index(&self) -> Arc<Vec<HistoryRoundIndex>> {
        let revision = self.history_revision();
        let cache = match self.0.history.round_cache.lock() {
            Ok(cache) => cache,
            Err(mut poison) => {
                **poison.get_mut() = MainRoundCache::default();
                self.0.history.round_cache.clear_poison();
                log::warn!("History round index cache was discarded after an interrupted update");
                poison.into_inner()
            }
        };
        if cache.revision == Some(revision) {
            return Arc::clone(&cache.index);
        }
        drop(cache);

        // Directory scan and JSON parsing stay outside the cache mutex. A
        // concurrent history mutation makes this projection non-cacheable but
        // does not justify holding a UI lock across disk I/O.
        let loaded = Arc::new(
            load_history_index()
                .records
                .iter()
                .map(HistoryRoundIndex::from_storage)
                .collect(),
        );
        let mut cache = match self.0.history.round_cache.lock() {
            Ok(cache) => cache,
            Err(mut poison) => {
                **poison.get_mut() = MainRoundCache::default();
                self.0.history.round_cache.clear_poison();
                log::warn!("History round index cache was discarded after an interrupted update");
                poison.into_inner()
            }
        };
        if self.history_revision() == revision {
            cache.index = Arc::clone(&loaded);
            cache.revision = Some(revision);
        }
        loaded
    }

    pub(crate) fn main_dps_readout(&self) -> Result<MainDpsReadout, CoreError> {
        const MAIN_READOUT_CACHE_CAPACITY: usize = 6;
        let mode = self.presentation_mode_snapshot();
        let follow_live_half = mode.selected_round.is_none() && !mode.processing_paused();
        let abyss_before = self.abyss_presentation_snapshot();
        let abyss_after = self.with_presentation_mode_state(&mode, |state| {
            if !follow_live_half {
                return abyss_before;
            }
            let (selected, observed) = next_live_abyss_selection(
                abyss_before.selected,
                abyss_before.observed,
                state.abyss.active_half,
            );
            AbyssPresentationState { selected, observed }
        })?;
        if follow_live_half {
            self.commit_follow_live_abyss(abyss_before, abyss_after);
        }

        let revision = self.main_dps_stream_revision()?;
        let key = MainReadoutCacheKey { revision };
        let (cache, recovered) = self.0.presentation.lock_main_readout_cache();
        if !recovered && let Some(cached) = cache.iter().find(|cached| cached.key == key) {
            return Ok(cached.readout.as_ref().clone());
        }
        drop(cache);

        #[cfg(test)]
        self.0
            .presentation
            .main_readout_projection_count
            .fetch_add(1, Ordering::AcqRel);
        let readout = self.with_presentation_mode_state(&mode, |state| {
            self.project_main_readout(state, abyss_after.selected)
        })?;

        // A concurrent hit/settings/presentation mutation makes this result a
        // valid one-shot snapshot, but it must not be published under an older
        // key. The next consumer projects the newer exact revision instead.
        if self.main_dps_stream_revision()? != revision {
            return Ok(readout);
        }
        let readout = Arc::new(readout);
        let (mut cache, _) = self.0.presentation.lock_main_readout_cache();
        if let Some(cached) = cache.iter().find(|cached| cached.key == key) {
            return Ok(cached.readout.as_ref().clone());
        }
        cache.push(MainReadoutCache {
            key,
            readout: Arc::clone(&readout),
        });
        if cache.len() > MAIN_READOUT_CACHE_CAPACITY {
            let remove_count = cache.len() - MAIN_READOUT_CACHE_CAPACITY;
            cache.drain(..remove_count);
        }
        drop(cache);
        let readout = readout.as_ref().clone();
        Ok(readout)
    }

    pub(crate) fn main_dps_detail_request(&self, kind: MainDpsDetailKind) -> MainDpsDetailRequest {
        let (details, recovered) = self.0.presentation.lock_details();
        let request = match kind {
            MainDpsDetailKind::Character => details.character.clone(),
            MainDpsDetailKind::Team => details.team.clone(),
        };
        drop(details);
        if recovered {
            self.0.presentation.publish_main();
        }
        request
    }

    pub(crate) fn main_dps_detail_cache_get(
        &self,
        revision: MainDpsStreamRevision,
        kind: MainDpsDetailKind,
        request: &MainDpsDetailRequest,
        offset: usize,
        limit: usize,
    ) -> Option<Arc<MainDpsDetailSnapshot>> {
        let key = MainDpsDetailCacheKey {
            revision,
            kind,
            request: request.clone(),
            offset,
            limit,
        };
        let (cache, recovered) = self.0.presentation.lock_cache();
        if recovered {
            return None;
        }
        cache
            .iter()
            .find(|cache| cache.key == key)
            .map(|cache| Arc::clone(&cache.snapshot))
    }

    pub(crate) fn main_dps_detail_cache_store(
        &self,
        revision: MainDpsStreamRevision,
        kind: MainDpsDetailKind,
        request: &MainDpsDetailRequest,
        offset: usize,
        limit: usize,
        snapshot: Arc<MainDpsDetailSnapshot>,
    ) {
        let key = MainDpsDetailCacheKey {
            revision,
            kind,
            request: request.clone(),
            offset,
            limit,
        };
        let (mut cache, _) = self.0.presentation.lock_cache();
        cache.retain(|entry| entry.key != key);
        cache.push(MainDpsDetailCache { key, snapshot });
        if cache.len() > MAIN_DPS_DETAIL_CACHE_CAPACITY {
            let remove_count = cache.len() - MAIN_DPS_DETAIL_CACHE_CAPACITY;
            cache.drain(..remove_count);
        }
    }

    pub(crate) fn set_main_dps_detail_request(
        &self,
        kind: MainDpsDetailKind,
        request: MainDpsDetailRequest,
    ) -> Result<bool, PresentationError> {
        let (mut details, recovered) = self.0.presentation.lock_details();
        if recovered {
            drop(details);
            self.0.presentation.publish_main();
            return Err(PresentationError::StateUnavailable);
        }
        let current = match kind {
            MainDpsDetailKind::Character => &mut details.character,
            MainDpsDetailKind::Team => &mut details.team,
        };
        if *current == request {
            return Ok(false);
        }
        *current = request;
        drop(details);
        self.0.presentation.publish_main();
        Ok(true)
    }

    fn abyss_presentation_snapshot(&self) -> AbyssPresentationState {
        let (abyss, recovered) = self.0.presentation.lock_abyss();
        let snapshot = *abyss;
        drop(abyss);
        if recovered {
            self.0.presentation.publish_technical_and_main();
        }
        snapshot
    }

    pub(crate) fn with_main_dps_detail_state<T>(
        &self,
        project: impl FnOnce(&CombatState, Option<AbyssHalf>) -> T,
    ) -> Result<T, CoreError> {
        let selected_abyss_half = self.abyss_presentation_snapshot().selected;
        self.with_main_presented_state(|state| {
            let selected_half = state.abyss.is_active().then(|| {
                selected_abyss_half
                    .or(state.abyss.active_half)
                    .unwrap_or(AbyssHalf::First)
            });
            project(state, selected_half)
        })
    }

    pub(crate) fn set_main_selected_abyss_half(
        &self,
        half: Option<AbyssHalf>,
    ) -> Result<bool, PresentationError> {
        let (mut abyss, recovered) = self.0.presentation.lock_abyss();
        if recovered {
            drop(abyss);
            self.0.presentation.publish_technical_and_main();
            return Err(PresentationError::StateUnavailable);
        }
        if abyss.selected == half {
            return Ok(false);
        }
        abyss.selected = half;
        drop(abyss);
        self.0.presentation.publish_technical_and_main();
        Ok(true)
    }

    pub(crate) fn update_main_appearance(
        &self,
        dark_mode: bool,
        opacity: f32,
    ) -> Result<bool, SettingsServiceError> {
        self.update_ui_config_with_effects(SettingsMutationEffects::SETTINGS_AND_MAIN, |config| {
            config.dark_mode = dark_mode;
            config.opacity = opacity;
        })
    }

    pub(crate) fn main_window_geometry(&self) -> (Option<[f32; 2]>, Option<[f32; 2]>) {
        let config = self.ui_config();
        (config.main_window_size, config.main_window_position)
    }

    pub(crate) fn set_main_window_geometry(
        &self,
        size: Option<[f32; 2]>,
        position: Option<[f32; 2]>,
    ) -> Result<bool, SettingsServiceError> {
        self.update_ui_config_with_effects(SettingsMutationEffects::NONE, |config| {
            config.main_window_size = size;
            config.main_window_position = position;
        })
    }

    pub(crate) fn main_dps_stream_revision(&self) -> Result<MainDpsStreamRevision, CoreError> {
        let mode = self.presentation_mode_snapshot();
        let _ = self.abyss_presentation_snapshot();
        let _ = self.main_dps_detail_request(MainDpsDetailKind::Character);
        let selected_history = mode.selected_round.is_some();
        let paused = mode.processing_paused();
        Ok(MainDpsStreamRevision {
            capture: if selected_history {
                0
            } else {
                self.0.live_capture.revision()
            },
            packet: if paused && !selected_history {
                self.0
                    .live_capture
                    .with_packet_state(|revision, _, _| revision.generation)?
            } else {
                0
            },
            presentation: self.0.presentation.revision.load(Ordering::Acquire),
            history: self.history_revision(),
            main: self.0.presentation.main_revision.load(Ordering::Acquire),
        })
    }

    fn project_main_readout(
        &self,
        state: &CombatState,
        selected_abyss_half: Option<AbyssHalf>,
    ) -> MainDpsReadout {
        let config = self.ui_config();
        let subtract_time_stop = subtract_time_stop_for_state(config.dps_time_mode, state);
        let projection_half = state.abyss.is_active().then(|| {
            selected_abyss_half
                .or(state.abyss.active_half)
                .unwrap_or(AbyssHalf::First)
        });
        let hud = project_hud(
            state,
            &HudConfig::detailed(),
            &HashSet::new(),
            HudProjectionOptions {
                dps_time_basis: DpsTimeBasis::from_subtract_time_stop(subtract_time_stop),
                separate_reaction_damage: config.separate_reaction_damage,
                include_max_hp_reduction_in_total_damage: config
                    .include_max_hp_reduction_in_total_damage,
                selected_abyss_half,
                preview_when_empty: false,
                timeline_bucket_seconds: f64::from(sanitize_timeline_bucket_seconds(
                    config.timeline_bucket_seconds,
                )),
            },
        );
        let (damage_attribution, character_durations) = projection_half.map_or_else(
            || {
                let durations = hud
                    .characters
                    .iter()
                    .filter_map(|row| {
                        state.stats.get(&row.character_id).map(|stats| {
                            (
                                row.character_id,
                                state.character_duration_with_time_stop(stats, subtract_time_stop),
                            )
                        })
                    })
                    .collect();
                (
                    state
                        .damage_attribution_summary()
                        .with_max_hp_reduction_in_total(
                            config.include_max_hp_reduction_in_total_damage,
                        ),
                    durations,
                )
            },
            |half| {
                let party = state.abyss.half(half);
                let durations = hud
                    .characters
                    .iter()
                    .filter_map(|row| {
                        party.stats.get(&row.character_id).map(|stats| {
                            (
                                row.character_id,
                                party.character_duration_with_time_stop(stats, subtract_time_stop),
                            )
                        })
                    })
                    .collect();
                (
                    party
                        .damage_attribution_summary()
                        .with_max_hp_reduction_in_total(
                            config.include_max_hp_reduction_in_total_damage,
                        ),
                    durations,
                )
            },
        );
        MainDpsReadout {
            hud,
            has_hits: !state.hits.is_empty(),
            game_paused: state.is_game_paused(),
            damage_attribution,
            separate_reaction_damage: config.separate_reaction_damage,
            character_durations,
        }
    }

    fn commit_follow_live_abyss(
        &self,
        expected: AbyssPresentationState,
        candidate: AbyssPresentationState,
    ) {
        if expected == candidate {
            return;
        }
        let (mut abyss, recovered) = self.0.presentation.lock_abyss();
        if recovered {
            drop(abyss);
            self.0.presentation.publish_technical_and_main();
            return;
        }
        // An explicit user selection wins over a stale projection. The next
        // capture revision will compute another follow-live candidate.
        if *abyss == expected {
            *abyss = candidate;
        }
    }

    fn bump_main_dps_revision(&self) -> u64 {
        self.0
            .presentation
            .main_revision
            .fetch_add(1, Ordering::AcqRel)
            + 1
    }

    pub(crate) fn settings_snapshot(&self) -> SettingsSnapshot {
        self.settings_snapshot_with_config_hook(|| {})
    }

    fn settings_snapshot_with_config_hook(
        &self,
        mut after_config: impl FnMut(),
    ) -> SettingsSnapshot {
        let (generation, config, devices, teams, updates) = loop {
            let before = self.settings_revision();
            let config = self.ui_config();
            after_config();
            let devices = self.0.settings.device_catalog_snapshot(|| {
                self.publish_settings_effects(SettingsMutationEffects::SETTINGS_AND_MAIN);
            });
            let teams = self.0.team_import.status(|| {
                self.publish_settings_effects(SettingsMutationEffects::SETTINGS);
            });
            // Use the same config generation for both the main Settings DTO and
            // its nested update projection.
            let updates = self.update_settings_snapshot_for_config(&config);
            let after = self.settings_revision();
            if before == after {
                break (after, config, devices, teams, updates);
            }
        };
        // Capture-log scanning is filesystem I/O and intentionally happens only
        // after every settings/team/device lock has been released.
        let capture_files = scan_capture_logs(&capture_log_dir());
        SettingsSnapshot::from_config(
            &config,
            generation,
            self.always_on_top(),
            devices.devices,
            devices.available,
            capture_files,
            teams.available,
            teams.upper_imported,
            teams.lower_imported,
            updates,
            self.combat_clock_health().unwrap_or_else(|error| {
                log::warn!("project combat-clock health for Settings failed: {error:?}");
                nte_dps_tool::engine::model::CombatClockRuntimeHealth::Unknown
            }),
        )
    }

    pub(crate) fn update_settings_snapshot(&self) -> UpdateSettingsSnapshot {
        let config = self.ui_config();
        self.update_settings_snapshot_for_config(&config)
    }

    fn update_settings_snapshot_for_config(&self, config: &UiConfig) -> UpdateSettingsSnapshot {
        let (update, install_blocked_message_key) = match self.0.update_runtime.snapshot() {
            Ok(update) => {
                let install_blocked_message_key = self.install_blocked_message_key_for(
                    update.prepared.as_ref().map(PreparedUpdate::component),
                );
                (update, install_blocked_message_key)
            }
            Err(UpdateActionError::RuntimeUnavailable) => {
                (UpdateRuntimeSnapshot::unavailable(), None)
            }
            Err(error) => {
                debug_assert!(false, "unexpected update snapshot error: {error:?}");
                (UpdateRuntimeSnapshot::unavailable(), None)
            }
        };
        UpdateSettingsSnapshot::from_runtime(
            config,
            update.status,
            update.message_key,
            update.message_arguments,
            &update.available,
            update.active_component,
            update.downloaded_bytes,
            update.total_bytes,
            update.prepared.as_ref(),
            install_blocked_message_key,
        )
    }

    pub(crate) fn request_capture_start(&self, replace_current: bool) -> Result<(), CoreError> {
        self.0
            .replay_import
            .reserve(ReplayImportReservationState::CaptureStart)
            .map_err(ReplayImportError::into_core)?;
        let result = (|| {
            if self.session_has_data()? && !replace_current {
                return Err(CoreError::new(
                    CoreErrorCode::CaptureAlreadyRunning,
                    "starting capture requires confirmation before replacing the current session",
                ));
            }
            let config = self.ui_config();
            let device = config
                .manual_capture_device
                .clone()
                .map_or(CaptureDeviceSelector::Auto, CaptureDeviceSelector::Name);
            self.0.live_capture.request_start(CaptureControllerOptions {
                profile: CaptureProfile::Combat,
                device,
                filter: config.capture_filter.clone(),
                include_incoming: true,
                server_damage_calibration: config.server_damage_calibration,
                raw_capture: RawCaptureMode::Enabled,
                raw_capture_directory: capture_log_dir(),
                expose_raw_capture_path: false,
                packet_emission: DESKTOP_PACKET_EMISSION_MODE,
            })
        })();
        self.0
            .replay_import
            .release(ReplayImportReservationState::CaptureStart)
            .map_err(ReplayImportError::into_core)?;
        if result.is_ok() {
            self.return_main_presentation_to_live();
        }
        result
    }

    pub(crate) fn request_capture_stop(&self) -> Result<(), CoreError> {
        self.0.live_capture.request_stop()
    }

    pub(crate) fn stop_active_capture_and_wait(&self, timeout: Duration) -> Result<(), CoreError> {
        if matches!(
            self.capture_phase(),
            LiveCapturePhase::Starting | LiveCapturePhase::Running | LiveCapturePhase::Failed
        ) || self.replay_running()?
        {
            self.request_capture_stop()?;
        }
        let deadline = Instant::now() + timeout;
        while matches!(
            self.capture_phase(),
            LiveCapturePhase::Starting | LiveCapturePhase::Running | LiveCapturePhase::Stopping
        ) || self.replay_running()?
        {
            if Instant::now() >= deadline {
                return Err(CoreError::new(
                    CoreErrorCode::SystemProbeFailed,
                    "capture or replay did not stop before the replacement timeout",
                ));
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        Ok(())
    }

    pub(crate) fn capture_phase(&self) -> LiveCapturePhase {
        self.0.live_capture.status().phase
    }

    pub(crate) fn passthrough(&self) -> bool {
        self.0.desktop.passthrough()
    }

    pub(crate) fn set_passthrough(&self, enabled: bool) {
        if self.0.desktop.set_passthrough(enabled) {
            self.0.presentation.revision.fetch_add(1, Ordering::AcqRel);
        }
    }

    pub(crate) fn passthrough_hotkey(&self) -> HotkeyBinding {
        self.ui_config().passthrough_hotkey
    }

    pub(crate) fn global_hotkeys(&self) -> GlobalHotkeys {
        self.ui_config().global_hotkeys
    }

    pub(crate) fn passthrough_hotkey_ready(&self) -> bool {
        self.0.desktop.passthrough_hotkey_ready()
    }

    pub(crate) fn set_passthrough_hotkey_ready(&self, ready: bool) {
        self.0.desktop.set_passthrough_hotkey_ready(ready);
    }

    pub(crate) fn lock_passthrough_transaction(
        &self,
    ) -> Result<PassthroughTransactionGuard<'_>, SettingsServiceError> {
        self.0.settings.lock_passthrough_transaction()
    }

    pub(crate) fn always_on_top(&self) -> bool {
        self.0.desktop.hud_always_on_top()
    }

    pub(crate) fn window_always_on_top(&self, window: DesktopWindowKind) -> bool {
        let config = self.ui_config();
        match window {
            DesktopWindowKind::MainDps => config.main_dps_always_on_top,
            DesktopWindowKind::Hud => config.hud_always_on_top,
            DesktopWindowKind::Console => config.console_always_on_top,
            DesktopWindowKind::AbyssValues => config.abyss_values_always_on_top,
            DesktopWindowKind::CharacterDetails => config.character_details_always_on_top,
            DesktopWindowKind::TeamDetails => config.team_details_always_on_top,
        }
        .expect("sanitized per-window always-on-top state")
    }

    pub(crate) fn hud_width(&self) -> u16 {
        self.hud_config().width
    }

    pub(crate) fn hud_window_position(&self) -> Option<[i32; 2]> {
        self.ui_config().hud_window_position
    }

    pub(crate) fn hud_initial_height(&self) -> u16 {
        let hud_config = self.hud_config();
        let content_height = HUD_BASE_INITIAL_HEIGHT
            + if hud_config.has_summary_row() {
                HUD_SUMMARY_HEIGHT
            } else {
                0
            }
            + if hud_config.show_character_rows {
                HUD_CHARACTERS_HEIGHT
            } else {
                0
            }
            + if hud_config.show_title {
                HUD_OPTIONAL_TITLE_HEIGHT
            } else {
                0
            }
            + if hud_config.show_abyss_half || hud_config.show_passthrough_state {
                HUD_OPTIONAL_STATUS_HEIGHT
            } else {
                0
            }
            + if hud_config.show_mini_timeline {
                HUD_MINI_TIMELINE_HEIGHT
            } else {
                0
            };
        if self.passthrough() {
            content_height
        } else {
            let editor_headers = hud_config
                .module_order
                .iter()
                .filter(|module| hud_config.module_visible(**module))
                .count() as u16
                * HUD_EDITOR_MODULE_HEADER_HEIGHT;
            (content_height + editor_headers).max(HUD_EDITOR_MIN_HEIGHT)
        }
    }

    pub(crate) fn set_hud_module_visibility(
        &self,
        module: HudModule,
        visible: bool,
    ) -> Result<bool, SettingsServiceError> {
        self.update_hud_config(|hud| hud.set_module_visible(module, visible))
    }

    pub(crate) fn move_hud_module(
        &self,
        dragged: HudModule,
        target: HudModule,
        insert_after: bool,
    ) -> Result<bool, SettingsServiceError> {
        self.update_hud_config(|hud| hud.move_module(dragged, target, insert_after))
    }

    pub(crate) fn set_hud_width(&self, width: u16) -> Result<bool, SettingsServiceError> {
        self.update_hud_config(|hud| hud.width = width)
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn update_interface_settings(
        &self,
        language: Language,
        dark_mode: bool,
        theme_preset: ThemePreset,
        accent: AccentColor,
        density: UiDensity,
        reduce_motion: bool,
        island_notifications: bool,
        island_offset_x: f32,
    ) -> Result<bool, SettingsServiceError> {
        self.update_ui_config_with_effects_for_change(
            |change| SettingsMutationEffects {
                history: change.previous.language != change.current.language,
                ..SettingsMutationEffects::SETTINGS_AND_MAIN
            },
            |config| {
                config.language = language;
                config.dark_mode = dark_mode;
                config.theme_preset = theme_preset;
                config.accent = accent;
                config.density = density;
                config.reduce_motion = reduce_motion;
                config.island_notifications = island_notifications;
                config.island_offset_x = island_offset_x;
            },
        )
    }

    pub(crate) fn update_update_settings(
        &self,
        auto_check: bool,
        auto_download: bool,
    ) -> Result<bool, SettingsServiceError> {
        self.update_ui_config_with_effects(SettingsMutationEffects::SETTINGS, |config| {
            config.auto_check_updates = auto_check;
            config.auto_download_updates = auto_download;
        })
    }

    pub(crate) fn auto_check_updates(&self) -> bool {
        self.ui_config().auto_check_updates
    }

    pub(crate) fn auto_download_updates(&self) -> bool {
        self.ui_config().auto_download_updates
    }

    pub(crate) fn begin_update_check(&self) -> Result<(), UpdateActionError> {
        self.0.update_runtime.begin_check()?;
        self.bump_settings_revision();
        Ok(())
    }

    pub(crate) fn finish_update_check(
        &self,
        available: Vec<AvailableComponentUpdate>,
    ) -> Result<(), UpdateActionError> {
        self.0.update_runtime.finish_check(available)?;
        self.bump_settings_revision();
        Ok(())
    }

    pub(crate) fn fail_update_check(
        &self,
        message_key: &'static str,
    ) -> Result<(), UpdateActionError> {
        self.0.update_runtime.fail_check(message_key)?;
        self.bump_settings_revision();
        Ok(())
    }

    pub(crate) fn begin_update_download(
        &self,
        component: UpdateComponent,
    ) -> Result<AvailableComponentUpdate, UpdateActionError> {
        let selected = self.0.update_runtime.begin_download(component)?;
        self.bump_settings_revision();
        Ok(selected)
    }

    pub(crate) fn update_download_progress(
        &self,
        component: UpdateComponent,
        downloaded_bytes: u64,
        total_bytes: u64,
    ) -> Result<bool, UpdateActionError> {
        let changed = self.0.update_runtime.update_download_progress(
            component,
            downloaded_bytes,
            total_bytes,
        )?;
        if changed {
            self.bump_settings_revision();
        }
        Ok(changed)
    }

    pub(crate) fn finish_update_download(
        &self,
        prepared: PreparedUpdate,
    ) -> Result<(), UpdateActionError> {
        self.0.update_runtime.finish_download(prepared)?;
        self.bump_settings_revision();
        Ok(())
    }

    pub(crate) fn fail_update_download(&self) -> Result<(), UpdateActionError> {
        self.fail_update_operation("Update download failed.")
    }

    pub(crate) fn begin_update_install(&self) -> Result<PreparedUpdate, UpdateActionError> {
        let prepared = self.0.update_runtime.begin_install()?;
        self.bump_settings_revision();
        Ok(prepared)
    }

    pub(crate) fn finish_plugin_update_install(
        &self,
        version: String,
    ) -> Result<(), UpdateActionError> {
        self.0.update_runtime.finish_plugin_install(version)?;
        self.bump_settings_revision();
        Ok(())
    }

    pub(crate) fn fail_update_install(&self) -> Result<(), UpdateActionError> {
        self.fail_update_operation("Update installation could not start.")
    }

    pub(crate) fn update_install_blocked_message_key(
        &self,
    ) -> Result<Option<&'static str>, UpdateActionError> {
        let prepared_component = self.0.update_runtime.prepared_component()?;
        Ok(self.install_blocked_message_key_for(prepared_component))
    }

    pub(crate) fn notify_update_install_blocker_changed(&self) {
        self.bump_settings_revision();
    }

    #[cfg(test)]
    pub(crate) fn poison_update_runtime_for_test(&self) {
        self.0.update_runtime.poison_for_test();
    }

    fn install_blocked_message_key_for(
        &self,
        prepared_component: Option<UpdateComponent>,
    ) -> Option<&'static str> {
        match prepared_component {
            Some(UpdateComponent::App)
                if !matches!(
                    self.capture_phase(),
                    LiveCapturePhase::Idle | LiveCapturePhase::Stopped | LiveCapturePhase::Failed
                ) =>
            {
                Some("Stop capture or replay before installing the update")
            }
            Some(UpdateComponent::ModsPlugin) => {
                match nte_dps_tool::platform::network::game_process_is_running() {
                    Ok(true) => Some("Close HTGame.exe before installing the Mod loader update"),
                    Ok(false) => None,
                    Err(error) => {
                        log::error!("check game process before Mod loader update failed: {error}");
                        Some("Game process state could not be checked.")
                    }
                }
            }
            _ => None,
        }
    }

    fn fail_update_operation(&self, message_key: &'static str) -> Result<(), UpdateActionError> {
        self.0.update_runtime.fail_operation(message_key)?;
        self.bump_settings_revision();
        Ok(())
    }

    fn bump_settings_revision(&self) {
        self.0.desktop.bump_settings_revision();
    }

    pub(crate) fn set_density(&self, density: UiDensity) -> Result<bool, SettingsServiceError> {
        self.update_ui_config_with_effects(SettingsMutationEffects::SETTINGS_AND_MAIN, |config| {
            config.density = density
        })
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn update_capture_settings(
        &self,
        filter: String,
        manual_capture_device: Option<String>,
        server_damage_calibration: bool,
        include_max_hp_reduction_in_total_damage: bool,
        separate_reaction_damage: bool,
        auto_round_after_idle: bool,
        auto_round_idle_seconds: u32,
        dps_time_mode: DpsTimeMode,
        passthrough_hotkey: HotkeyBinding,
    ) -> Result<bool, SettingsServiceError> {
        let changed = self.update_ui_config(|config| {
            config.capture_filter = filter;
            config.manual_capture_device = manual_capture_device;
            config.server_damage_calibration = server_damage_calibration;
            config.include_max_hp_reduction_in_total_damage =
                include_max_hp_reduction_in_total_damage;
            config.separate_reaction_damage = separate_reaction_damage;
            config.auto_round_after_idle = auto_round_after_idle;
            config.auto_round_idle_seconds = auto_round_idle_seconds;
            config.dps_time_mode = dps_time_mode;
            config.passthrough_hotkey = passthrough_hotkey;
        })?;
        // Keep the capture-side automatic round-boundary policy synchronized
        // only after the durable settings transaction succeeds. A boundary
        // then freezes this effective basis and reaction mode together with
        // its detached state instead of consulting mutable settings later.
        self.0
            .live_capture
            .set_history_archive_policy(history_archive_policy(&self.ui_config()));
        Ok(changed)
    }

    pub(crate) fn update_global_hotkeys(
        &self,
        global_hotkeys: GlobalHotkeys,
    ) -> Result<bool, SettingsServiceError> {
        self.update_ui_config_with_effects(SettingsMutationEffects::SETTINGS, |config| {
            config.global_hotkeys = global_hotkeys;
        })
    }

    pub(crate) fn update_main_dps_display(
        &self,
        display: MainDpsDisplayConfig,
    ) -> Result<bool, SettingsServiceError> {
        self.update_ui_config_with_effects(SettingsMutationEffects::SETTINGS_AND_MAIN, |config| {
            config.main_dps_display = display
        })
    }

    pub(crate) fn refresh_capture_devices(&self) -> Result<(), CoreError> {
        self.apply_capture_device_refresh(self.0.settings.refresh_devices(|| {
            self.publish_settings_effects(SettingsMutationEffects::SETTINGS_AND_MAIN);
        }))
    }

    fn apply_capture_device_refresh(
        &self,
        outcome: crate::settings_service::DeviceRefreshOutcome,
    ) -> Result<(), CoreError> {
        match outcome.error {
            Some(error) => Err(error),
            None => Ok(()),
        }
    }

    #[cfg(test)]
    fn refresh_capture_devices_with(
        &self,
        enumerate: impl FnOnce() -> Result<Vec<CaptureDeviceSnapshot>, CoreError>,
    ) -> Result<(), CoreError> {
        self.apply_capture_device_refresh(self.0.settings.refresh_devices_with(enumerate, || {
            self.publish_settings_effects(SettingsMutationEffects::SETTINGS_AND_MAIN);
        }))
    }

    pub(crate) fn clear_capture_files(&self) -> ClearOutcome {
        clear_capture_logs(&capture_log_dir())
    }

    pub(crate) fn refresh_capture_file_stats(&self) {
        self.bump_settings_revision();
    }

    pub(crate) fn session_has_data(&self) -> Result<bool, CoreError> {
        self.0.live_capture.with_state(|state| {
            !state.hits.is_empty()
                || !state.packets.is_empty()
                || !state.stats.is_empty()
                || !state.empty_curtain.is_empty()
                || state.abyss.is_active()
        })
    }

    pub(crate) fn ensure_session_undo_runtime_available(&self) -> Result<(), SessionUndoError> {
        self.0.session_undo.ensure_available()
    }

    pub(crate) fn reset_session_with_undo_action(
        &self,
    ) -> Result<Option<String>, SessionUndoError> {
        self.0.session_undo.ensure_available()?;
        let (previous, quality_source) = self
            .0
            .live_capture
            .state_and_source_snapshot()
            .map_err(|_| SessionUndoError::StateUnavailable)?;
        let has_data = !previous.hits.is_empty()
            || !previous.packets.is_empty()
            || !previous.stats.is_empty()
            || !previous.empty_curtain.is_empty()
            || previous.abyss.is_active();
        if !has_data {
            self.0
                .live_capture
                .reset_session()
                .map_err(|_| SessionUndoError::StateUnavailable)?;
            self.return_main_presentation_to_live();
            return Ok(None);
        }
        let token = self.0.session_undo.remember(previous, quality_source)?;
        if self.0.live_capture.reset_session().is_err() {
            self.0.session_undo.consume_matching(&token)?;
            return Err(SessionUndoError::StateUnavailable);
        }
        self.return_main_presentation_to_live();
        Ok(Some(token))
    }

    pub(crate) fn reset_session_with_undo(&self) -> Result<Option<String>, CoreError> {
        self.reset_session_with_undo_action()
            .map_err(SessionUndoError::into_core)
    }

    pub(crate) fn clear_session_action(&self) -> Result<(), SessionUndoError> {
        self.0.session_undo.ensure_available()?;
        self.0
            .live_capture
            .reset_session()
            .map_err(|_| SessionUndoError::StateUnavailable)?;
        self.0.session_undo.clear()?;
        self.return_main_presentation_to_live();
        Ok(())
    }

    pub(crate) fn clear_session(&self) -> Result<(), CoreError> {
        self.clear_session_action()
            .map_err(SessionUndoError::into_core)
    }

    pub(crate) fn undo_session_reset(&self, token: &str) -> Result<(), SessionUndoError> {
        self.0.session_undo.ensure_available()?;
        if matches!(
            self.capture_phase(),
            LiveCapturePhase::Starting | LiveCapturePhase::Running | LiveCapturePhase::Stopping
        ) || self
            .replay_running()
            .map_err(|_| SessionUndoError::StateUnavailable)?
        {
            return Err(SessionUndoError::Busy);
        }
        if self
            .session_has_data()
            .map_err(|_| SessionUndoError::StateUnavailable)?
        {
            return Err(SessionUndoError::NewData);
        }
        let entry = self.0.session_undo.entry_for_restore(token)?;
        self.restore_session_undo_entry(&entry, |state, source| {
            self.0.live_capture.restore_session(state, source)
        })?;
        if !self.0.session_undo.consume_matching(token)? {
            return Err(SessionUndoError::RuntimeUnavailable);
        }
        Ok(())
    }

    fn restore_session_undo_entry(
        &self,
        entry: &SessionUndoEntry,
        restore: impl FnOnce(CombatState, CaptureQualitySource) -> Result<(), CoreError>,
    ) -> Result<(), SessionUndoError> {
        let restore_state = entry.state.clone();
        if restore(restore_state, entry.quality_source).is_err() {
            return Err(SessionUndoError::StateUnavailable);
        }
        Ok(())
    }

    pub(crate) fn import_team_data(&self, export: TeamDpsExport) -> Result<bool, TeamImportError> {
        let outcome = self.0.team_import.replace_export(export, || {
            self.publish_settings_effects(SettingsMutationEffects::SETTINGS);
        })?;
        Ok(outcome.changed)
    }

    pub(crate) fn imported_abyss_teams(
        &self,
    ) -> Result<(Option<TeamDps>, Option<TeamDps>), TeamImportError> {
        self.0
            .team_import
            .snapshot(|| {
                self.publish_settings_effects(SettingsMutationEffects::SETTINGS);
            })
            .map(|teams| (teams.upper, teams.lower))
    }

    pub(crate) fn import_abyss_team(
        &self,
        export: TeamDpsExport,
        upper: bool,
    ) -> Result<bool, TeamImportError> {
        let preferred = if upper { export.upper } else { export.lower };
        let Some(team) = preferred.or(export.single) else {
            return Ok(false);
        };
        let _outcome = self.0.team_import.replace_half(upper, team, || {
            self.publish_settings_effects(SettingsMutationEffects::SETTINGS);
        })?;
        Ok(true)
    }

    pub(crate) fn import_current_abyss_team(
        &self,
        upper: bool,
    ) -> Result<bool, TeamOperationError> {
        let Some(team) = self.current_abyss_team(upper)? else {
            return Ok(false);
        };
        let _outcome = self
            .0
            .team_import
            .replace_half(upper, team, || {
                self.publish_settings_effects(SettingsMutationEffects::SETTINGS);
            })
            .map_err(TeamOperationError::State)?;
        Ok(true)
    }

    pub(crate) fn current_abyss_team_availability(&self) -> Result<[bool; 2], CoreError> {
        Ok([
            self.current_abyss_team(true)?.is_some(),
            self.current_abyss_team(false)?.is_some(),
        ])
    }

    fn current_abyss_team(&self, upper: bool) -> Result<Option<TeamDps>, CoreError> {
        let config = self.ui_config();
        self.with_main_presented_state(|state| {
            let export = nte_dps_tool::core::team_data::export_team_data(
                state,
                subtract_time_stop_for_state(config.dps_time_mode, state),
                config.separate_reaction_damage,
                None,
                None,
            )?;
            if state.abyss.is_active() {
                if upper { export.upper } else { export.lower }
            } else {
                export.single
            }
        })
    }

    pub(crate) fn clear_abyss_team(&self, upper: bool) -> Result<bool, TeamImportError> {
        let outcome = self.0.team_import.clear_half(upper, || {
            self.publish_settings_effects(SettingsMutationEffects::SETTINGS);
        })?;
        Ok(outcome.changed)
    }

    pub(crate) fn swap_abyss_teams(&self) -> Result<bool, TeamImportError> {
        let outcome = self.0.team_import.swap(|| {
            self.publish_settings_effects(SettingsMutationEffects::SETTINGS);
        })?;
        Ok(outcome.changed)
    }

    pub(crate) fn export_team_data(&self) -> Result<Option<TeamDpsExport>, TeamOperationError> {
        let config = self.ui_config();
        let imported = self
            .0
            .team_import
            .snapshot(|| {
                self.publish_settings_effects(SettingsMutationEffects::SETTINGS);
            })
            .map_err(TeamOperationError::State)?;
        Ok(self.with_main_presented_state(|state| {
            nte_dps_tool::core::team_data::export_team_data(
                state,
                subtract_time_stop_for_state(config.dps_time_mode, state),
                config.separate_reaction_damage,
                imported.upper,
                imported.lower,
            )
        })?)
    }

    pub(crate) fn poll_mod_studio_runtime_logs(
        &self,
    ) -> Result<Vec<ModStudioRuntimeLog>, ModStudioError> {
        poll_mod_studio_runtime_logs()
    }

    pub(crate) fn poll_mod_studio_runtime_events(
        &self,
    ) -> Result<Vec<ModStudioRuntimeEvent>, ModStudioError> {
        poll_mod_studio_runtime_events()
    }

    pub(crate) fn set_hud_option(
        &self,
        option: HudSettingOption,
        enabled: bool,
    ) -> Result<bool, SettingsServiceError> {
        self.update_hud_config(|hud| match option {
            HudSettingOption::Title => hud.show_title = enabled,
            HudSettingOption::TeamDps => hud.show_team_dps = enabled,
            HudSettingOption::Duration => hud.show_duration = enabled,
            HudSettingOption::TotalDamage => hud.show_total_damage = enabled,
            HudSettingOption::DamageTaken => hud.show_damage_taken = enabled,
            HudSettingOption::CharacterRows => hud.show_character_rows = enabled,
            HudSettingOption::AbyssHalf => hud.show_abyss_half = enabled,
            HudSettingOption::PassthroughState => hud.show_passthrough_state = enabled,
            HudSettingOption::MiniTimeline => hud.show_mini_timeline = enabled,
        })
    }

    pub(crate) fn apply_hud_preset(&self, preset: HudPreset) -> Result<bool, SettingsServiceError> {
        self.update_hud_config(|hud| {
            let width = hud.width;
            let module_order = hud.module_order.clone();
            let mut candidate = match preset {
                HudPreset::Minimal => HudConfig::minimal(),
                HudPreset::Standard => HudConfig::default(),
                HudPreset::Detailed => HudConfig::detailed(),
            };
            candidate.width = width;
            candidate.module_order = module_order;
            *hud = candidate;
        })
    }

    pub(crate) fn set_hud_window_position(
        &self,
        position: [i32; 2],
    ) -> Result<bool, SettingsServiceError> {
        self.update_ui_config_with_effects(SettingsMutationEffects::NONE, |config| {
            config.hud_window_position = Some(position);
        })
    }

    fn update_hud_config(
        &self,
        update: impl FnOnce(&mut HudConfig),
    ) -> Result<bool, SettingsServiceError> {
        self.update_ui_config(|config| update(&mut config.hud))
    }

    pub(crate) fn set_always_on_top(&self, enabled: bool) -> Result<bool, SettingsServiceError> {
        self.set_window_always_on_top(DesktopWindowKind::Hud, enabled)
    }

    pub(crate) fn set_window_always_on_top(
        &self,
        window: DesktopWindowKind,
        enabled: bool,
    ) -> Result<bool, SettingsServiceError> {
        let effects = match window {
            DesktopWindowKind::Hud => SettingsMutationEffects::SETTINGS_AND_TECHNICAL,
            DesktopWindowKind::MainDps => SettingsMutationEffects::MAIN,
            DesktopWindowKind::Console
            | DesktopWindowKind::AbyssValues
            | DesktopWindowKind::CharacterDetails
            | DesktopWindowKind::TeamDetails => SettingsMutationEffects::NONE,
        };
        let changed = self.update_ui_config_with_effects(effects, |config| match window {
            DesktopWindowKind::MainDps => config.main_dps_always_on_top = Some(enabled),
            DesktopWindowKind::Hud => {
                config.always_on_top = enabled;
                config.hud_always_on_top = Some(enabled);
            }
            DesktopWindowKind::Console => config.console_always_on_top = Some(enabled),
            DesktopWindowKind::AbyssValues => config.abyss_values_always_on_top = Some(enabled),
            DesktopWindowKind::CharacterDetails => {
                config.character_details_always_on_top = Some(enabled)
            }
            DesktopWindowKind::TeamDetails => config.team_details_always_on_top = Some(enabled),
        })?;
        if window == DesktopWindowKind::Hud {
            self.0.desktop.set_hud_always_on_top(enabled);
        }
        Ok(changed)
    }

    pub(crate) fn prepare_current_history_archive(
        &self,
    ) -> Result<Option<PreparedHistoryArchive>, CoreError> {
        let config = self.ui_config();
        let (state, source) = self.0.live_capture.history_state_and_source_snapshot()?;
        let dps_time_mode = DpsTimeBasis::from_subtract_time_stop(subtract_time_stop_for_state(
            config.dps_time_mode,
            &state,
        ));
        Ok(prepare_history_archive_owned(
            state,
            source,
            dps_time_mode,
            config.separate_reaction_damage,
        ))
    }

    fn persist_history_archive(
        &self,
        archive: &PreparedHistoryArchive,
    ) -> Result<HistoryPersistenceOutcome, HistoryRuntimeError> {
        self.with_history_transaction(|| self.persist_history_archive_locked(archive))?
    }

    fn persist_history_archive_locked(
        &self,
        archive: &PreparedHistoryArchive,
    ) -> Result<HistoryPersistenceOutcome, HistoryRuntimeError> {
        self.persist_history_parts_locked(&archive.summary, archive.details.as_ref())
    }

    fn persist_pending_history_archive(
        &self,
        archive: &PendingHistoryArchive,
    ) -> Result<HistoryPersistenceOutcome, HistoryRuntimeError> {
        self.with_history_transaction(|| {
            self.persist_history_parts_locked(&archive.summary, Some(&archive.details))
        })?
    }

    fn persist_history_parts_locked(
        &self,
        summary: &nte_dps_tool::engine::model::CombatSessionSummary,
        details: Option<&HistoryCombatDetails>,
    ) -> Result<HistoryPersistenceOutcome, HistoryRuntimeError> {
        let outcome = save_borrowed_archive_outcome(summary, details).map_err(|error| {
            log::warn!("History archive storage operation did not finish: {error}");
            if error.is_retryable() {
                HistoryRuntimeError::Persistence
            } else {
                HistoryRuntimeError::PermanentPersistence
            }
        })?;
        Ok(self.finish_borrowed_history_save(outcome))
    }

    fn finish_borrowed_history_save(
        &self,
        outcome: BorrowedHistorySaveOutcome,
    ) -> HistoryPersistenceOutcome {
        let persistence_outcome = match outcome {
            BorrowedHistorySaveOutcome::Committed => HistoryPersistenceOutcome::Committed,
            BorrowedHistorySaveOutcome::CommittedWithMaintenanceWarning(warning) => {
                log::warn!("{warning}");
                HistoryPersistenceOutcome::CommittedWithMaintenanceWarning
            }
        };
        self.bump_history_revision();
        persistence_outcome
    }

    #[cfg(test)]
    fn finish_history_save(&self, outcome: HistorySaveOutcome) -> HistoryPersistenceOutcome {
        let persistence_outcome = match outcome {
            HistorySaveOutcome::Committed(_) => HistoryPersistenceOutcome::Committed,
            HistorySaveOutcome::CommittedWithMaintenanceWarning { warning, .. } => {
                log::warn!("{warning}");
                HistoryPersistenceOutcome::CommittedWithMaintenanceWarning
            }
        };
        // Both variants crossed the atomic-write commit boundary. The record
        // is observable even when retention maintenance did not finish, so
        // the read model advances exactly once and the archive must not be
        // placed back into the record retry queue.
        self.bump_history_revision();
        persistence_outcome
    }

    pub(crate) fn archive_current_history_round(&self) -> Result<bool, HistoryRuntimeError> {
        if self
            .replay_running()
            .map_err(|_| HistoryRuntimeError::Unavailable)?
        {
            return Err(HistoryRuntimeError::ReplayActive);
        }
        self.archive_current_history_round_with(|state, archive| {
            state.persist_history_archive_locked(archive)
        })
    }

    fn archive_current_history_round_with(
        &self,
        persist: impl FnOnce(
            &Self,
            &PreparedHistoryArchive,
        ) -> Result<HistoryPersistenceOutcome, HistoryRuntimeError>,
    ) -> Result<bool, HistoryRuntimeError> {
        let _archive_transaction = self
            .0
            .history
            .archive_transaction
            .lock()
            .map_err(|_| HistoryRuntimeError::Unavailable)?;
        // Lock order is archive_transaction -> transaction ->
        // pending_archives. Commands acquire only transaction (then undo), so
        // no inverse order exists. Both serialization guards are acquired
        // before capture state is inspected or detached.
        let _history_transaction = self.lock_history_transaction()?;
        if self
            .0
            .history
            .pending_archives
            .lock()
            .map_err(|_| HistoryRuntimeError::Unavailable)?
            .len()
            >= MAX_PENDING_HISTORY_ARCHIVES
        {
            return Err(HistoryRuntimeError::RetryQueueFull);
        }
        let config = self.ui_config();
        let policy = history_archive_policy(&config);
        let Some(cut) = self
            .0
            .live_capture
            .cut_round_with_history_policy(policy)
            .map_err(|_| HistoryRuntimeError::Unavailable)?
        else {
            return Ok(false);
        };
        let Some(archive) = prepare_history_archive_owned(
            cut.state,
            cut.source,
            cut.dps_time_mode,
            cut.separate_reaction_damage,
        ) else {
            log::warn!("cut History round had no archivable summary");
            return Ok(true);
        };
        if let Err(error) = persist(self, &archive) {
            log::warn!("History persistence failed after the live round was cut: {error}");
            if matches!(
                error,
                HistoryRuntimeError::Unavailable | HistoryRuntimeError::Persistence
            ) {
                self.0
                    .history
                    .pending_archives
                    .lock()
                    .map_err(|_| HistoryRuntimeError::Unavailable)?
                    .push_back(archive);
            } else {
                return Err(error);
            }
        }
        Ok(true)
    }

    pub(crate) fn maintain_history_rounds(&self) -> Result<(), HistoryRuntimeError> {
        self.retry_pending_history_archives()?;
        let mut pending = self
            .0
            .live_capture
            .take_pending_abyss_archives()
            .map_err(|_| HistoryRuntimeError::Unavailable)?
            .into_iter();
        let mut retry = Vec::new();
        while let Some(current) = pending.next() {
            match self.persist_pending_history_archive(&current) {
                Ok(_) => {}
                Err(HistoryRuntimeError::Unavailable) => {
                    retry.push(current);
                    retry.extend(pending);
                    self.0
                        .live_capture
                        .restore_pending_abyss_archives(retry)
                        .map_err(|_| HistoryRuntimeError::Unavailable)?;
                    return Err(HistoryRuntimeError::Unavailable);
                }
                Err(HistoryRuntimeError::Persistence) => {
                    retry.push(current);
                }
                Err(error) => {
                    log::warn!("automatic Abyss History archive failed: {error}");
                }
            }
        }
        if !retry.is_empty() {
            self.0
                .live_capture
                .restore_pending_abyss_archives(retry)
                .map_err(|_| HistoryRuntimeError::Unavailable)?;
            return Ok(());
        }

        let config = self.ui_config();
        if !config.auto_round_after_idle {
            return Ok(());
        }
        let status = self.0.live_capture.status();
        let replay_running = self
            .replay_running()
            .map_err(|_| HistoryRuntimeError::Unavailable)?;
        let idle_elapsed = self
            .0
            .live_capture
            .idle_elapsed()
            .map_err(|_| HistoryRuntimeError::Unavailable)?;
        let due = self
            .0
            .live_capture
            .with_state(|state| {
                auto_round_due(
                    status.phase == LiveCapturePhase::Running && !replay_running,
                    false,
                    state.abyss.is_active(),
                    state.is_game_paused(),
                    !state.hits.is_empty(),
                    idle_elapsed,
                    config.auto_round_idle_seconds,
                )
            })
            .map_err(|_| HistoryRuntimeError::Unavailable)?;
        if due {
            match self.archive_current_history_round() {
                Ok(_) => {}
                Err(HistoryRuntimeError::Unavailable) => {
                    return Err(HistoryRuntimeError::Unavailable);
                }
                Err(error) => {
                    log::warn!("automatic idle History archive failed: {error}");
                }
            }
        }
        Ok(())
    }

    fn retry_pending_history_archives(&self) -> Result<(), HistoryRuntimeError> {
        self.retry_pending_history_archives_with(|state, archive| {
            state.persist_history_archive(archive)
        })
    }

    fn retry_pending_history_archives_with(
        &self,
        mut persist: impl FnMut(
            &Self,
            &PreparedHistoryArchive,
        ) -> Result<HistoryPersistenceOutcome, HistoryRuntimeError>,
    ) -> Result<(), HistoryRuntimeError> {
        let _archive_transaction = self
            .0
            .history
            .archive_transaction
            .lock()
            .map_err(|_| HistoryRuntimeError::Unavailable)?;
        // Check the serializer even when the retry queue is empty so a
        // poisoned maintenance runtime terminates. Release it immediately;
        // each archive persistence acquires its own transaction guard.
        drop(self.lock_history_transaction()?);
        let pending = {
            let mut pending = self
                .0
                .history
                .pending_archives
                .lock()
                .map_err(|_| HistoryRuntimeError::Unavailable)?;
            std::mem::take(&mut *pending)
        };
        if pending.is_empty() {
            return Ok(());
        }

        let mut pending = pending.into_iter();
        let mut retry = VecDeque::new();
        while let Some(archive) = pending.next() {
            match persist(self, &archive) {
                Ok(_) => {}
                Err(HistoryRuntimeError::Unavailable) => {
                    retry.push_back(archive);
                    retry.extend(pending);
                    self.restore_history_retry_queue(retry)?;
                    return Err(HistoryRuntimeError::Unavailable);
                }
                Err(HistoryRuntimeError::Persistence) => {
                    log::warn!("retrying a pending History archive failed");
                    retry.push_back(archive);
                }
                Err(error) => {
                    log::warn!("retrying a pending History archive failed: {error}");
                }
            }
        }
        if retry.is_empty() {
            return Ok(());
        }

        self.restore_history_retry_queue(retry)
    }

    fn restore_history_retry_queue(
        &self,
        mut retry: VecDeque<PreparedHistoryArchive>,
    ) -> Result<(), HistoryRuntimeError> {
        let mut pending = self
            .0
            .history
            .pending_archives
            .lock()
            .map_err(|_| HistoryRuntimeError::Unavailable)?;
        retry.append(&mut *pending);
        *pending = retry;
        Ok(())
    }

    pub(crate) fn with_history_transaction<T>(
        &self,
        action: impl FnOnce() -> T,
    ) -> Result<T, HistoryRuntimeError> {
        let _guard = self.lock_history_transaction()?;
        Ok(action())
    }

    fn lock_history_transaction(&self) -> Result<MutexGuard<'_, ()>, HistoryRuntimeError> {
        self.0
            .history
            .transaction
            .lock()
            .map_err(|_| HistoryRuntimeError::Unavailable)
    }

    pub(crate) fn history_revision(&self) -> u64 {
        self.0.history.revision.load(Ordering::Acquire)
    }

    pub(crate) fn live_capture_resources(&self) -> LiveCaptureResources {
        self.0.live_capture.resources()
    }

    #[cfg(test)]
    pub(crate) fn restore_live_state_for_test(
        &self,
        state: CombatState,
        source: CaptureQualitySource,
    ) {
        let _ = self.0.live_capture.restore_session(state, source);
    }

    pub(crate) fn character_data_snapshot(
        &self,
    ) -> Result<(CharacterDataProjection, u64), CharacterDataServiceError> {
        self.0.character_data.snapshot()
    }

    pub(crate) fn save_character_data_record(
        &self,
        input: CharacterDataRecordInput,
    ) -> Result<(CharacterDataProjection, u64), CharacterDataServiceError> {
        self.0.character_data.save_record(input)
    }

    pub(crate) fn encrypted_ini_snapshot(
        &self,
    ) -> Result<EncryptedIniProjection, EncryptedIniServiceError> {
        self.0.encrypted_ini.snapshot()
    }

    pub(crate) fn open_encrypted_ini(
        &self,
        path: PathBuf,
    ) -> Result<EncryptedIniProjection, EncryptedIniServiceError> {
        self.0.encrypted_ini.open(path)
    }

    pub(crate) fn reload_encrypted_ini(
        &self,
    ) -> Result<EncryptedIniProjection, EncryptedIniServiceError> {
        self.0.encrypted_ini.reload()
    }

    pub(crate) fn save_encrypted_ini(
        &self,
        expected_generation: u64,
        plaintext: String,
        key: EncryptedIniKey,
    ) -> Result<(EncryptedIniProjection, EncryptedIniSaveOutcome), EncryptedIniServiceError> {
        self.0
            .encrypted_ini
            .save(expected_generation, plaintext, key)
    }

    pub(crate) fn clear_encrypted_ini(
        &self,
    ) -> Result<EncryptedIniProjection, EncryptedIniServiceError> {
        self.0.encrypted_ini.clear()
    }

    pub(crate) fn empty_curtain_snapshot(&self) -> Result<InventorySnapshot, CoreError> {
        let resources = self.0.live_capture.resources();
        let observed_at_unix_ms = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis()
            .min(u128::from(u64::MAX)) as u64;
        self.with_main_presented_state(|state| {
            inventory_snapshot(
                &state.empty_curtain,
                &state.empty_curtain_characters,
                &self.0.equipment_catalog,
                &resources.characters,
                state.empty_curtain_generation,
                observed_at_unix_ms,
            )
        })
    }

    pub(crate) fn empty_curtain_data_snapshot(
        &self,
    ) -> Result<EmptyCurtainDataSnapshot, CoreError> {
        let (items, characters) = self.with_main_presented_state(|state| {
            (
                state.empty_curtain.clone(),
                state.empty_curtain_characters.clone(),
            )
        })?;
        Ok((items, characters, Arc::clone(&self.0.equipment_catalog)))
    }

    pub(crate) fn equipment_catalog(&self) -> Arc<EquipmentCatalog> {
        Arc::clone(&self.0.equipment_catalog)
    }

    pub(crate) fn empty_curtain_operation_snapshot(
        &self,
    ) -> Result<EquipmentOperationSnapshot, EquipmentOperationError> {
        self.0.equipment_operation.poll_snapshot()
    }

    pub(crate) fn empty_curtain_revision_and_operation(
        &self,
    ) -> Result<((u64, u64, u64), EmptyCurtainOperationState), EmptyCurtainRuntimeError> {
        let operation = self.empty_curtain_operation_snapshot()?;
        let (inventory, characters) = if self.main_processing_paused() {
            self.with_main_presented_state(|state| {
                (
                    state.empty_curtain_generation,
                    state.empty_curtain_characters_generation,
                )
            })?
        } else {
            self.0.live_capture.inventory_revision()?
        };
        Ok((
            (inventory, characters, operation.revision),
            operation.operation,
        ))
    }

    pub(crate) fn diagnostics_revision(&self) -> Result<(u64, u64, u64), CoreError> {
        let packet_generation = self
            .0
            .live_capture
            .with_packet_state(|revision, _, _| revision.generation)?;
        Ok((
            self.0.live_capture.revision(),
            packet_generation,
            self.0.diagnostics.revision(),
        ))
    }

    pub(crate) fn diagnostics_input(&self) -> Result<DiagnosticSnapshot, CoreError> {
        let config = self.ui_config();
        let status = self.0.live_capture.status();
        let replay_running = self.0.live_capture.replay_running()?;
        let raw_packet_count = self
            .0
            .live_capture
            .raw_capture_snapshot()?
            .map_or(0, |raw| raw.packet_count as usize);
        let (parsed_packet_count, hit_count) = self
            .0
            .live_capture
            .with_state(|state| (state.packet_count, state.hits.len()))?;
        Ok(DiagnosticSnapshot {
            capture_running: matches!(
                status.phase,
                LiveCapturePhase::Starting | LiveCapturePhase::Running | LiveCapturePhase::Stopping
            ) && !replay_running,
            replay_running,
            active_capture_filter: self.0.live_capture.active_capture_filter()?,
            raw_packet_count,
            parsed_packet_count,
            hit_count,
            dropped_history_archives: self.0.live_capture.dropped_history_archives(),
            include_incoming: true,
            server_damage_calibration: config.server_damage_calibration,
            last_diagnostic: status.issue.map(|issue| format!("{issue:?}")),
            manual_capture_device: config.manual_capture_device,
        })
    }

    pub(crate) fn diagnostics_report(&self) -> Option<DiagnosticRun> {
        self.0.diagnostics.snapshot().1
    }

    pub(crate) fn diagnostics_report_snapshot(&self) -> (u64, Option<DiagnosticRun>) {
        self.0.diagnostics.snapshot()
    }

    pub(crate) fn store_diagnostics_report(&self, report: DiagnosticRun) -> bool {
        self.0.diagnostics.store(report)
    }

    pub(crate) fn diagnostics_quality(&self) -> Result<CaptureQualitySummary, CoreError> {
        self.0.live_capture.quality_summary()
    }

    pub(crate) fn diagnostics_raw_capture(
        &self,
    ) -> Result<Option<nte_dps_tool::engine::capture::RawCaptureSnapshot>, CoreError> {
        self.0.live_capture.raw_capture_snapshot()
    }

    pub(crate) fn save_diagnostics_raw_capture(
        &self,
        path: &std::path::Path,
    ) -> Result<Result<(u64, u64), String>, CoreError> {
        self.0.live_capture.save_last_raw_capture(path)
    }

    pub(crate) fn begin_replay_import(
        &self,
        replace_current: bool,
    ) -> Result<ReplayImportReservation, ReplayImportError> {
        self.0
            .replay_import
            .reserve(ReplayImportReservationState::ReplayImport)?;
        let reservation = ReplayImportReservation {
            state: self.clone(),
            active: true,
        };
        let active = matches!(
            self.capture_phase(),
            LiveCapturePhase::Starting | LiveCapturePhase::Running | LiveCapturePhase::Stopping
        ) || self.replay_running()?;
        if active && !replace_current {
            return Err(ReplayImportError::Capture(CoreError::new(
                CoreErrorCode::CaptureAlreadyRunning,
                "capture or replay is already active",
            )));
        }
        if self.session_has_data()? && !replace_current {
            return Err(ReplayImportError::Capture(CoreError::new(
                CoreErrorCode::CaptureAlreadyRunning,
                "replay import requires confirmation before replacing the current session",
            )));
        }
        Ok(reservation)
    }

    fn request_diagnostics_replay(
        &self,
        kind: CaptureReplayKind,
        path: PathBuf,
    ) -> Result<(), ReplayImportError> {
        let config = self.ui_config();
        let local_ip_hint = self
            .diagnostics_report()
            .and_then(|run| run.environment.local_ip)
            .and_then(|local_ip| local_ip.parse().ok());
        let result = self
            .0
            .live_capture
            .request_replay(
                kind,
                path,
                local_ip_hint,
                true,
                config.server_damage_calibration,
            )
            .map_err(|error| match error {
                ReplayStartError::Capture(error) => ReplayImportError::Capture(error),
                ReplayStartError::JsonImport(error) => ReplayImportError::JsonImport(error),
            });
        if result.is_ok() {
            self.return_main_presentation_to_live();
        }
        result
    }

    fn request_diagnostics_prepared_json_replay(
        &self,
        prepared: PreparedCaptureJsonReplay,
    ) -> Result<(), ReplayImportError> {
        let result = self
            .0
            .live_capture
            .request_prepared_json_replay(prepared)
            .map_err(ReplayImportError::Capture);
        if result.is_ok() {
            self.return_main_presentation_to_live();
        }
        result
    }

    pub(crate) fn diagnostics_capture_export_plan(
        &self,
    ) -> Result<(CaptureExportPlan, u64), CoreError> {
        let config = self.ui_config();
        let game_network = self
            .diagnostics_report()
            .and_then(|run| run.environment.game_connection)
            .map(|network| CaptureExportNetwork {
                pid: network.pid,
                local_ip: network.local_ip,
                remote_ip: network.remote_ip,
                remote_port: network.remote_port,
            });
        self.0.live_capture.with_packet_state(|revision, _, state| {
            (
                nte_dps_tool::engine::capture::CaptureExportDocument::prepare(
                    state,
                    CaptureExportOptions {
                        filter: config.capture_filter,
                        include_incoming: true,
                        game_network,
                        dps_time_mode: DpsTimeBasis::from_subtract_time_stop(
                            subtract_time_stop_for_state(config.dps_time_mode, state),
                        ),
                    },
                ),
                revision.session_generation,
            )
        })
    }

    /// Copies one bounded hit page while proving it still belongs to the
    /// immutable export plan. Disk serialization happens after this closure
    /// releases the authoritative state lock.
    pub(crate) fn diagnostics_capture_export_hit_page(
        &self,
        session_generation: u64,
        hits_generation: u64,
        hit_count: usize,
        start: usize,
        limit: usize,
    ) -> Result<Vec<Hit>, CoreError> {
        self.0
            .live_capture
            .with_packet_state(|revision, _, state| {
                let Some(end) = start.checked_add(limit).filter(|end| *end <= hit_count) else {
                    return Err(CoreError::new(
                        CoreErrorCode::CaptureStateUnavailable,
                        "capture export requested an invalid hit page",
                    ));
                };
                if revision.session_generation != session_generation
                    || state.hits_generation != hits_generation
                    || state.hits.len() != hit_count
                {
                    return Err(CoreError::new(
                        CoreErrorCode::CaptureStateUnavailable,
                        "capture export source changed during streaming",
                    ));
                }
                Ok(state.hits.range(start..end).cloned().collect())
            })?
    }

    pub(crate) fn diagnostics_has_exportable_state(&self) -> Result<bool, CoreError> {
        self.0.live_capture.with_state(|state| {
            !state.hits.is_empty() || !state.packets.is_empty() || !state.empty_curtain.is_empty()
        })
    }

    pub(crate) fn submit_empty_curtain_operation(
        &self,
        character: nte_dps_tool::engine::model::HtItemNetId,
        operation: ModsPluginOperation,
    ) -> Result<u64, EquipmentOperationError> {
        self.0.equipment_operation.submit(character, operation)
    }

    pub(crate) fn timeline_projection(
        &self,
        scope: TimelineScope,
    ) -> Result<Arc<TimelineProjection>, CoreError> {
        const TIMELINE_CACHE_CAPACITY: usize = 6;
        let config = self.ui_config();
        let revision = self.main_dps_stream_revision()?;
        let subtract_time_stop = matches!(config.dps_time_mode, DpsTimeMode::TimeStopAdjusted)
            && matches!(
                self.main_presented_combat_clock_health()?,
                CombatClockRuntimeHealth::Available | CombatClockRuntimeHealth::Recorded
            );
        let key = TimelineProjectionCacheKey {
            revision,
            scope,
            bucket_seconds_bits: config.timeline_bucket_seconds.to_bits(),
            subtract_time_stop,
            language: config.language.code(),
        };
        let (cache, recovered) = self.0.presentation.lock_timeline_cache();
        if !recovered && let Some(cached) = cache.iter().find(|cached| cached.key == key) {
            return Ok(Arc::clone(&cached.projection));
        }
        drop(cache);

        let resources = self.0.live_capture.resources();
        let projection = Arc::new(self.with_main_presented_state(|state| {
            project_timeline(
                state,
                &resources.characters,
                TimelineProjectionOptions {
                    scope,
                    bucket_seconds: config.timeline_bucket_seconds,
                    max_buckets: MAX_TIMELINE_BUCKETS,
                    max_roles_per_bucket: MAX_TIMELINE_ROLES_PER_BUCKET,
                    max_characters: MAX_TIMELINE_CHARACTERS,
                    subtract_time_stop,
                    language: config.language,
                },
            )
        })?);

        let (mut cache, _) = self.0.presentation.lock_timeline_cache();
        if let Some(cached) = cache.iter().find(|cached| cached.key == key) {
            return Ok(Arc::clone(&cached.projection));
        }
        cache.push(TimelineProjectionCache {
            key,
            projection: Arc::clone(&projection),
        });
        if cache.len() > TIMELINE_CACHE_CAPACITY {
            let remove_count = cache.len() - TIMELINE_CACHE_CAPACITY;
            cache.drain(..remove_count);
        }
        Ok(projection)
    }

    pub(crate) fn packet_stream_revision(&self) -> Result<PacketStreamRevision, CoreError> {
        if let Some(paused) = self.presentation_mode_snapshot().paused {
            return Ok(paused.packet_revision);
        }
        self.0
            .live_capture
            .with_packet_state(|revision, _, _| revision)
    }

    pub(crate) fn packets_projection(
        &self,
        after: Option<PacketStreamRevision>,
    ) -> Result<(PacketStreamRevision, bool, PacketsProjection), CoreError> {
        if let Some(paused) = self.presentation_mode_snapshot().paused {
            let revision = paused.packet_revision;
            let incremental = after
                .filter(|cursor| cursor.session_generation == revision.session_generation)
                .and_then(|cursor| {
                    project_packets_since(&paused.state, revision, 0, cursor.packet_generation)
                });
            return Ok(match incremental {
                Some(projection) => (revision, false, projection),
                None => (
                    revision,
                    true,
                    project_recent_packets(&paused.state, revision, 0),
                ),
            });
        }
        self.0
            .live_capture
            .with_packet_state(|revision, queued_event_count, state| {
                let incremental = after
                    .filter(|cursor| cursor.session_generation == revision.session_generation)
                    .and_then(|cursor| {
                        project_packets_since(
                            state,
                            revision,
                            queued_event_count,
                            cursor.packet_generation,
                        )
                    });
                match incremental {
                    Some(projection) => (revision, false, projection),
                    None => (
                        revision,
                        true,
                        project_recent_packets(state, revision, queued_event_count),
                    ),
                }
            })
    }

    pub(crate) fn skills_projection(
        &self,
        scope: SkillsScope,
    ) -> Result<SkillsProjection, CoreError> {
        let config = self.ui_config();
        let resources = self.0.live_capture.resources();
        self.with_main_presented_state(|state| {
            project_skills(
                state,
                &resources.characters,
                SkillsProjectionOptions {
                    scope,
                    language: config.language,
                },
            )
        })
    }

    pub(crate) fn timeline_preferences(&self) -> (f32, TimelineDpsViewMode) {
        let config = self.ui_config();
        (
            sanitize_timeline_bucket_seconds(config.timeline_bucket_seconds),
            config.timeline_dps_view_mode,
        )
    }

    pub(crate) fn update_timeline_preferences(
        &self,
        bucket_seconds: f32,
        view_mode: TimelineDpsViewMode,
    ) -> Result<bool, SettingsServiceError> {
        self.update_ui_config_with_effects(
            SettingsMutationEffects {
                technical: true,
                ..SettingsMutationEffects::NONE
            },
            |config| {
                config.timeline_bucket_seconds = bucket_seconds;
                config.timeline_dps_view_mode = view_mode;
            },
        )
    }

    pub(crate) fn bump_history_revision(&self) -> u64 {
        self.0.history.revision.fetch_add(1, Ordering::AcqRel) + 1
    }

    pub(crate) fn ensure_history_undo_available(&self) -> Result<(), HistoryRuntimeError> {
        drop(
            self.0
                .history
                .undo
                .lock()
                .map_err(|_| HistoryRuntimeError::Unavailable)?,
        );
        Ok(())
    }

    pub(crate) fn remember_deleted_history_tombstone(
        &self,
        tombstone: HistoryDeleteTombstone,
    ) -> Result<String, HistoryRuntimeError> {
        let token = tombstone.token().to_owned();
        let replaced = {
            let mut undo = self
                .0
                .history
                .undo
                .lock()
                .map_err(|_| HistoryRuntimeError::Unavailable)?;
            undo.replace(HistoryUndoEntry {
                token: token.clone(),
                tombstone,
                expires_at: Instant::now() + HISTORY_UNDO_WINDOW,
            })
        };
        // Disk cleanup is deliberately outside the undo mutex. History command
        // callers hold the broader transaction guard, so replacement remains
        // serialized while slow filesystem work cannot block token reads.
        if let Some(replaced) = replaced
            && let Err(error) = discard_tombstoned_record(&replaced.tombstone)
        {
            log::warn!("replaced History undo tombstone cleanup did not finish: {error}");
        }
        Ok(token)
    }

    pub(crate) fn peek_deleted_history_tombstone(
        &self,
        token: &str,
    ) -> Result<Option<HistoryDeleteTombstone>, HistoryRuntimeError> {
        let (active, expired) = {
            let mut undo = self
                .0
                .history
                .undo
                .lock()
                .map_err(|_| HistoryRuntimeError::Unavailable)?;
            if undo
                .as_ref()
                .is_some_and(|entry| Instant::now() > entry.expires_at)
            {
                (None, undo.take())
            } else {
                (
                    undo.as_ref()
                        .filter(|entry| entry.token == token)
                        .map(|entry| entry.tombstone.clone()),
                    None,
                )
            }
        };
        // Expiry cleanup is also outside the mutex. A failed cleanup becomes a
        // bounded orphan for startup/maintenance cleanup; it is never restored
        // as a valid undo entry after its deadline.
        if let Some(expired) = expired
            && let Err(error) = discard_tombstoned_record(&expired.tombstone)
        {
            log::warn!("expired History undo tombstone cleanup did not finish: {error}");
        }
        Ok(active)
    }

    pub(crate) fn consume_deleted_history(&self, token: &str) -> Result<bool, HistoryRuntimeError> {
        let mut undo = self
            .0
            .history
            .undo
            .lock()
            .map_err(|_| HistoryRuntimeError::Unavailable)?;
        let matches = undo.as_ref().is_some_and(|entry| entry.token == token);
        if matches {
            undo.take();
        }
        Ok(matches)
    }

    pub(crate) fn set_history_prediction_team(
        &self,
        team: TeamDps,
        upper: bool,
    ) -> Result<bool, TeamImportError> {
        let outcome = self.0.team_import.replace_half(upper, team, || {
            self.publish_settings_effects(SettingsMutationEffects::SETTINGS);
        })?;
        Ok(outcome.changed)
    }

    fn ui_config(&self) -> UiConfig {
        self.0.settings.config_snapshot()
    }

    fn update_ui_config(
        &self,
        update: impl FnOnce(&mut UiConfig),
    ) -> Result<bool, SettingsServiceError> {
        self.update_ui_config_with_effects(SettingsMutationEffects::SETTINGS_AND_TECHNICAL, update)
    }

    fn update_ui_config_with_effects(
        &self,
        effects: SettingsMutationEffects,
        update: impl FnOnce(&mut UiConfig),
    ) -> Result<bool, SettingsServiceError> {
        self.update_ui_config_with_effects_for_change(|_| effects, update)
    }

    fn update_ui_config_with_effects_for_change(
        &self,
        effects: impl FnOnce(&ConfigUpdate) -> SettingsMutationEffects,
        update: impl FnOnce(&mut UiConfig),
    ) -> Result<bool, SettingsServiceError> {
        let Some(update) = self.0.settings.update_config(update, |change| {
            self.publish_settings_effects(effects(change));
        })?
        else {
            return Ok(false);
        };
        debug_assert_ne!(update.previous, update.current);
        Ok(true)
    }

    fn publish_settings_effects(&self, effects: SettingsMutationEffects) {
        if effects.settings {
            self.bump_settings_revision();
        }
        if effects.technical {
            self.0.presentation.revision.fetch_add(1, Ordering::AcqRel);
        }
        if effects.main {
            self.bump_main_dps_revision();
        }
        if effects.history {
            self.bump_history_revision();
        }
    }

    pub(crate) fn stream_revision(&self) -> StreamRevision {
        let paused = self.main_processing_paused();
        let _ = self.abyss_presentation_snapshot();
        StreamRevision {
            capture: if paused {
                0
            } else {
                self.0.live_capture.revision()
            },
            presentation: self.0.presentation.revision.load(Ordering::Acquire),
        }
    }

    pub(crate) fn settings_revision(&self) -> u64 {
        self.0.desktop.settings_revision()
    }

    fn with_main_presented_state<T>(
        &self,
        project: impl FnOnce(&CombatState) -> T,
    ) -> Result<T, CoreError> {
        let mode = self.presentation_mode_snapshot();
        self.with_presentation_mode_state(&mode, project)
    }

    fn with_presentation_mode_state<T>(
        &self,
        mode: &PresentationModeSnapshot,
        project: impl FnOnce(&CombatState) -> T,
    ) -> Result<T, CoreError> {
        if let Some(state) = mode.presented_state() {
            return Ok(project(state.as_ref()));
        }
        self.0.live_capture.with_state(project)
    }

    fn return_main_presentation_to_live(&self) {
        let outgoing_revision = self.0.live_capture.outgoing_hit_revision();
        let (mut mode, recovered_mode) = self.0.presentation.lock_mode();
        let mode_changed = mode.paused.is_some() || mode.selected_round.is_some();
        mode.paused = None;
        mode.selected_round = None;
        mode.selected_outgoing_revision = outgoing_revision;
        drop(mode);

        let (mut abyss, recovered_abyss) = self.0.presentation.lock_abyss();
        let abyss_changed = *abyss != AbyssPresentationState::default();
        *abyss = AbyssPresentationState::default();
        drop(abyss);

        if recovered_mode || recovered_abyss || mode_changed || abyss_changed {
            self.0.presentation.publish_technical_and_main();
        }
    }

    pub(crate) fn mod_studio(&self) -> ModStudioWorkspaceService {
        self.0.mod_studio.clone()
    }

    pub(crate) fn mod_loader(&self) -> ModLoaderRuntimeService {
        self.0.mod_loader.clone()
    }

    pub(crate) fn mod_studio_game_directory(&self, region: ModsPluginGameRegion) -> Option<String> {
        let config = self.ui_config();
        match region {
            ModsPluginGameRegion::China => config.mod_studio_china_game_directory,
            ModsPluginGameRegion::Global => config.mod_studio_global_game_directory,
        }
    }

    pub(crate) fn set_mod_studio_game_directory(
        &self,
        region: ModsPluginGameRegion,
        directory: Option<String>,
    ) -> Result<bool, SettingsServiceError> {
        self.update_ui_config_with_effects(SettingsMutationEffects::NONE, |config| match region {
            ModsPluginGameRegion::China => config.mod_studio_china_game_directory = directory,
            ModsPluginGameRegion::Global => config.mod_studio_global_game_directory = directory,
        })
    }

    pub(crate) fn mod_studio_loading_method(&self) -> ModStudioLoadingMethod {
        self.ui_config().mod_studio_loading_method
    }

    pub(crate) fn set_mod_studio_loading_method(
        &self,
        method: ModStudioLoadingMethod,
    ) -> Result<bool, SettingsServiceError> {
        self.update_ui_config_with_effects(SettingsMutationEffects::NONE, |config| {
            config.mod_studio_loading_method = method;
        })
    }

    pub(crate) fn mod_studio_risk_acknowledged(&self) -> bool {
        self.ui_config().mod_studio_risk_acknowledged
    }

    pub(crate) fn acknowledge_mod_studio_risk(&self) -> Result<bool, SettingsServiceError> {
        self.update_ui_config_with_effects(SettingsMutationEffects::NONE, |config| {
            config.mod_studio_risk_acknowledged = true;
        })
    }

    pub(crate) fn uptime_ms(&self) -> u128 {
        self.0.desktop.uptime_ms()
    }

    fn hud_config(&self) -> HudConfig {
        self.ui_config().hud
    }

    pub(crate) fn reserve_stream(
        &self,
        owner_window: &str,
        subscription_key: &str,
    ) -> Result<StreamRegistration, StreamRegistryError> {
        self.0.streams.reserve(owner_window, subscription_key)
    }

    pub(crate) fn activate_stream(
        &self,
        registration: &StreamRegistration,
    ) -> Result<bool, StreamRegistryError> {
        self.0.streams.activate(registration)
    }

    pub(crate) fn finish_stream(
        &self,
        registration: &StreamRegistration,
    ) -> Result<bool, StreamRegistryError> {
        self.0.streams.finish(registration)
    }

    pub(crate) fn stop_stream(
        &self,
        owner_window: &str,
        subscription_key: &str,
    ) -> Result<bool, StreamRegistryError> {
        self.0.streams.stop(owner_window, subscription_key)
    }

    pub(crate) fn stop_streams_for_window(
        &self,
        owner_window: &str,
    ) -> Result<usize, StreamRegistryError> {
        self.0.streams.stop_for_window(owner_window)
    }

    pub(crate) fn shutdown_streams(&self) -> Result<usize, StreamRegistryError> {
        self.0.streams.shutdown()
    }

    #[cfg(test)]
    pub(crate) fn stream_registry_len(&self) -> Result<usize, StreamRegistryError> {
        self.0.streams.len()
    }
}

#[cfg(test)]
mod tests {
    use std::{fs, time::SystemTime};

    use super::*;
    use nte_dps_tool::core::hud::{HudDataState, HudModuleSnapshot};

    #[test]
    fn desktop_capture_retains_packets_for_the_live_inspector() {
        assert_eq!(DESKTOP_PACKET_EMISSION_MODE, PacketEmissionMode::FullDebug);
    }

    #[test]
    fn capture_export_rejects_same_length_hits_from_a_replacement_session() {
        let live_capture = LiveCaptureService::new(LiveCaptureResources::default());
        let mut original = CombatState::default();
        original.push_hit(test_hit(100.0));
        live_capture
            .restore_session(original, CaptureQualitySource::JsonReplay)
            .expect("original capture session should install");
        let state = AppState::new(UiConfig::default(), live_capture.clone());
        let (plan, session_generation) = state
            .diagnostics_capture_export_plan()
            .expect("original export plan should be available");

        let mut replacement = CombatState::default();
        replacement.push_hit(test_hit(999.0));
        assert_eq!(
            replacement.hits_generation,
            plan.hits_generation(),
            "fixture proves per-state hit generations can collide"
        );
        live_capture
            .restore_session(replacement, CaptureQualitySource::JsonReplay)
            .expect("replacement capture session should install");

        let error = state
            .diagnostics_capture_export_hit_page(
                session_generation,
                plan.hits_generation(),
                plan.hit_count(),
                0,
                1,
            )
            .expect_err("a replacement session must invalidate the export plan");
        assert_eq!(error.code, CoreErrorCode::CaptureStateUnavailable);
        assert!(error.detail.contains("source changed"));
    }

    fn test_hit(damage: f64) -> nte_dps_tool::engine::model::Hit {
        use nte_dps_tool::engine::model::{Hit, HitCharacterSource, HitDirection};

        Hit {
            timestamp: 1.0,
            char_id: 7,
            char_name: "Fixture".to_owned(),
            char_known: true,
            damage,
            byte_offset: 0,
            bit_shift: 0,
            char_source: HitCharacterSource::Packet,
            direction: HitDirection::Outgoing,
            target_hp_before: 1_000.0,
            target_hp_after: 1_000.0 - damage,
            target_max_hp: 1_000.0,
            max_hp_reduction: 0.0,
            target_hp_percent: 50.0,
            target_id: None,
            target_name: None,
            target_name_en: None,
            target_name_ja: None,
            target_monster_id: None,
            target_context: Vec::new(),
            gameplay_effect_index: Some(17),
            gameplay_effect_name: Some("GE_Fixture".to_owned()),
            ability_name: Some("GA_Fixture".to_owned()),
            damage_name: Some("Fixture Damage".to_owned()),
            damage_component: None,
            attack_type: Some("Skill".to_owned()),
            damage_attribute: None,
            follow_up_damage: 0.0,
            follow_up_timestamp: None,
            follow_up_damage_name: None,
            follow_up_attack_type: None,
            follow_up_damage_attribute: None,
            reconciled_overkill_damage: None,
            wire_event: None,
        }
    }

    fn temporary_config_path(tag: &str) -> PathBuf {
        let directory = std::env::temp_dir().join(format!(
            "nte_tauri_hud_{tag}_{}_{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(SystemTime::UNIX_EPOCH)
                .expect("system clock after epoch")
                .as_nanos()
        ));
        fs::create_dir_all(&directory).expect("temporary config directory");
        directory.join("config.json")
    }

    fn temporary_history_tombstone(
        tag: &str,
    ) -> (
        PathBuf,
        nte_dps_tool::storage::history::HistoryDeleteTombstone,
    ) {
        use nte_dps_tool::storage::history::{save_summary_to_dir, tombstone_record_from_dir};

        let directory = std::env::temp_dir().join(format!(
            "nte-tauri-history-undo-{tag}-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("system time after epoch")
                .as_nanos()
        ));
        let record = save_summary_to_dir(
            &directory,
            nte_dps_tool::engine::model::CombatSessionSummary::default(),
        )
        .expect("save History tombstone fixture");
        let tombstone = tombstone_record_from_dir(&directory, &record.id)
            .expect("create History tombstone fixture")
            .expect("History fixture exists");
        (directory, tombstone)
    }

    #[test]
    fn replacing_subscription_stops_previous_stream() {
        let state = AppState::default();
        let previous = state
            .reserve_stream("hud", "technical")
            .expect("reserve previous stream");
        assert!(
            state
                .activate_stream(&previous)
                .expect("activate previous stream")
        );
        let current = state
            .reserve_stream("hud", "technical")
            .expect("reserve current stream");
        assert!(
            state
                .activate_stream(&current)
                .expect("activate current stream")
        );

        assert!(previous.is_cancelled());
        assert!(!current.is_cancelled());
    }

    #[test]
    fn newer_stream_generation_rejects_a_stale_late_activation() {
        let state = AppState::default();
        let stale = state
            .reserve_stream("hud", "technical")
            .expect("reserve stale stream generation");
        let current = state
            .reserve_stream("hud", "technical")
            .expect("reserve current stream generation");

        assert!(
            state
                .activate_stream(&current)
                .expect("activate current stream")
        );
        assert!(!state.activate_stream(&stale).expect("reject stale stream"));
        assert!(stale.is_cancelled());
        assert!(!current.is_cancelled());
    }

    #[test]
    fn full_registry_allows_same_identity_replacement_without_dropping_current() {
        let state = AppState::default();
        let current = state
            .reserve_stream("hud", "technical")
            .expect("reserve current stream");
        assert!(
            state
                .activate_stream(&current)
                .expect("activate current stream")
        );
        for index in 1..MAX_ACTIVE_STREAMS {
            let pending = state
                .reserve_stream("hud", &format!("technical:{index}"))
                .expect("fill bounded stream registry");
            assert!(
                state
                    .activate_stream(&pending)
                    .expect("activate filler stream")
            );
        }

        assert!(state.reserve_stream("hud", "overflow").is_err());
        let replacement = state
            .reserve_stream("hud", "technical")
            .expect("same identity must reuse its registry capacity");
        assert!(
            state
                .activate_stream(&replacement)
                .expect("activate replacement")
        );
        assert!(current.is_cancelled());
        assert!(!replacement.is_cancelled());
    }

    #[test]
    fn pending_replacement_reservations_are_globally_bounded() {
        let state = AppState::default();
        let pending = (0..MAX_PENDING_STREAMS)
            .map(|_| {
                state
                    .reserve_stream("hud", "technical")
                    .expect("reserve bounded pending replacement")
            })
            .collect::<Vec<_>>();

        assert_eq!(pending.len(), MAX_PENDING_STREAMS);
        assert_eq!(
            state.reserve_stream("hud", "technical").err(),
            Some(StreamRegistryError::CapacityUnavailable)
        );
        assert_eq!(state.shutdown_streams().expect("cancel pending streams"), 1);
        assert!(pending.iter().all(StreamRegistration::is_cancelled));
    }

    #[test]
    fn poisoned_stream_registry_resets_atomically_and_fails_the_triggering_operation() {
        let state = AppState::default();
        let current = state
            .reserve_stream("hud", "technical")
            .expect("reserve current stream");
        assert!(
            state
                .activate_stream(&current)
                .expect("activate current stream")
        );
        let poison_state = state.clone();
        let _ = std::thread::spawn(move || {
            let _guard = poison_state
                .0
                .streams
                .entries
                .lock()
                .expect("healthy stream registry lock");
            panic!("poison stream registry");
        })
        .join();

        assert_eq!(
            state.reserve_stream("hud", "next").err(),
            Some(StreamRegistryError::RuntimeUnavailable)
        );
        assert!(current.is_cancelled());
        assert_eq!(state.stream_registry_len().expect("reset registry"), 0);

        let recovered = state
            .reserve_stream("hud", "next")
            .expect("reserve after controlled reset");
        assert!(
            state
                .activate_stream(&recovered)
                .expect("activate recovered stream")
        );
    }

    #[test]
    fn stream_identity_includes_owner_and_wrong_owner_stop_is_a_no_op() {
        let state = AppState::default();
        let hud = state
            .reserve_stream("hud", "shared:key")
            .expect("reserve HUD stream");
        let console = state
            .reserve_stream("console", "shared:key")
            .expect("reserve Console stream");
        assert!(state.activate_stream(&hud).expect("activate HUD stream"));
        assert!(
            state
                .activate_stream(&console)
                .expect("activate Console stream")
        );
        let revision = state.stream_revision();

        assert!(
            !state
                .stop_stream("main-dps", "shared:key")
                .expect("wrong owner no-op")
        );
        assert_eq!(state.stream_revision(), revision);
        assert!(!hud.is_cancelled());
        assert!(!console.is_cancelled());
        assert!(
            state
                .stop_stream("hud", "shared:key")
                .expect("stop HUD stream")
        );
        assert!(hud.is_cancelled());
        assert!(!console.is_cancelled());
    }

    #[test]
    fn stream_registry_rejects_unbounded_or_non_ascii_identities() {
        let state = AppState::default();

        assert_eq!(
            state.reserve_stream("", "technical").err(),
            Some(StreamRegistryError::InvalidIdentity)
        );
        assert_eq!(
            state.reserve_stream("hud", "technical/path").err(),
            Some(StreamRegistryError::InvalidIdentity)
        );
        assert_eq!(
            state.reserve_stream("hud", &"a".repeat(129)).err(),
            Some(StreamRegistryError::InvalidIdentity)
        );
    }

    #[test]
    fn stream_shutdown_is_idempotent_and_cancels_current_and_pending_tokens() {
        let state = AppState::default();
        let current = state
            .reserve_stream("hud", "technical")
            .expect("reserve current stream");
        assert!(
            state
                .activate_stream(&current)
                .expect("activate current stream")
        );
        let pending = state
            .reserve_stream("hud", "technical")
            .expect("reserve replacement stream");

        assert_eq!(state.shutdown_streams().expect("shutdown streams"), 1);
        assert!(current.is_cancelled());
        assert!(pending.is_cancelled());
        assert_eq!(state.shutdown_streams().expect("duplicate shutdown"), 0);
        assert_eq!(state.stream_registry_len().expect("empty registry"), 0);
    }

    #[test]
    fn selected_round_drives_timeline_and_skills_projection_input() {
        let mut selected = CombatState::default();
        selected.push_hit(test_hit(125.0));
        let records = vec![HistoryRecord {
            id: "selected-round".to_owned(),
            details: HistoryCombatDetails::from_state(&selected),
            ..Default::default()
        }];

        let projected = selected_round_combat_state(&records, Some("selected-round"))
            .expect("selected round combat state");
        let timeline = project_timeline(
            &projected,
            &HashMap::new(),
            TimelineProjectionOptions {
                scope: TimelineScope::Whole,
                bucket_seconds: 1.0,
                max_buckets: MAX_TIMELINE_BUCKETS,
                max_roles_per_bucket: MAX_TIMELINE_ROLES_PER_BUCKET,
                max_characters: MAX_TIMELINE_CHARACTERS,
                subtract_time_stop: true,
                language: Language::English,
            },
        );
        let skills = project_skills(
            &projected,
            &HashMap::new(),
            SkillsProjectionOptions {
                scope: SkillsScope::Whole,
                language: Language::English,
            },
        );

        assert_eq!(timeline.total_damage, 125.0);
        assert_eq!(skills.total_damage, 125.0);
        assert!(selected_round_combat_state(&records, Some("other-round")).is_none());
    }

    #[test]
    fn late_starting_history_worker_cannot_overwrite_a_newer_intent() {
        let state = AppState::default();
        let slow_operation = state.reserve_main_round_selection();
        let newest_operation = state.reserve_main_round_selection();

        assert!(
            state
                .set_main_selected_round_id_with(
                    Some("newest".to_owned()),
                    newest_operation,
                    |_| {
                        let mut selected = CombatState::default();
                        selected.push_hit(test_hit(200.0));
                        Ok(selected)
                    },
                )
                .expect("commit newer History selection")
        );
        let stale_loader_ran = AtomicBool::new(false);
        assert!(
            !state
                .set_main_selected_round_id_with(Some("slow".to_owned()), slow_operation, |_| {
                    stale_loader_ran.store(true, Ordering::Release);
                    let mut selected = CombatState::default();
                    selected.push_hit(test_hit(100.0));
                    Ok(selected)
                },)
                .expect("superseded History selection")
        );
        assert!(!stale_loader_ran.load(Ordering::Acquire));

        assert_eq!(state.main_selected_round_id().as_deref(), Some("newest"));
        assert_eq!(
            state
                .with_main_presented_state(|selected| selected.total_damage)
                .expect("project latest History selection"),
            200.0
        );
    }

    #[test]
    fn selected_history_round_reloads_same_id_after_history_revision_changes() {
        let state = AppState::default();
        let first_operation = state.reserve_main_round_selection();
        assert!(
            state
                .set_main_selected_round_id_with(
                    Some("reimported".to_owned()),
                    first_operation,
                    |_| {
                        let mut selected = CombatState::default();
                        selected.push_hit(test_hit(100.0));
                        Ok(selected)
                    },
                )
                .expect("select first History contents")
        );

        state.bump_history_revision();
        assert!(
            state.presentation_mode_snapshot().selected_round.is_none(),
            "a History mutation must not keep projecting materialized old contents"
        );
        let second_operation = state.reserve_main_round_selection();
        assert!(
            state
                .set_main_selected_round_id_with(
                    Some("reimported".to_owned()),
                    second_operation,
                    |_| {
                        let mut selected = CombatState::default();
                        selected.push_hit(test_hit(250.0));
                        Ok(selected)
                    },
                )
                .expect("reload replaced History contents")
        );
        assert_eq!(
            state
                .with_main_presented_state(|selected| selected.total_damage)
                .expect("project replaced History contents"),
            250.0
        );
    }

    #[test]
    fn already_loading_history_selection_cannot_overwrite_a_newer_intent() {
        let state = AppState::default();
        let slow_operation = state.reserve_main_round_selection();
        let slow_state = state.clone();
        let (entered_tx, entered_rx) = std::sync::mpsc::channel();
        let (release_tx, release_rx) = std::sync::mpsc::channel();
        let slow = std::thread::spawn(move || {
            slow_state.set_main_selected_round_id_with(
                Some("slow".to_owned()),
                slow_operation,
                |_| {
                    entered_tx.send(()).expect("signal slow History load");
                    release_rx.recv().expect("release slow History load");
                    let mut selected = CombatState::default();
                    selected.push_hit(test_hit(100.0));
                    Ok(selected)
                },
            )
        });
        entered_rx.recv().expect("slow History load started");
        let newest_operation = state.reserve_main_round_selection();
        assert!(
            state
                .set_main_selected_round_id_with(
                    Some("newest".to_owned()),
                    newest_operation,
                    |_| {
                        let mut selected = CombatState::default();
                        selected.push_hit(test_hit(200.0));
                        Ok(selected)
                    },
                )
                .expect("commit newer History selection")
        );
        release_tx.send(()).expect("finish slow History load");
        assert!(
            !slow
                .join()
                .expect("join slow History selection")
                .expect("superseded History selection")
        );
        assert_eq!(state.main_selected_round_id().as_deref(), Some("newest"));
    }

    #[test]
    fn timeline_projection_cache_reuses_one_revision_across_consumers() {
        let state = AppState::default();
        let first = state
            .timeline_projection(TimelineScope::Whole)
            .expect("first projection");
        let second = state
            .timeline_projection(TimelineScope::Whole)
            .expect("same-revision projection");
        let other_scope = state
            .timeline_projection(TimelineScope::First)
            .expect("other-scope projection");

        assert!(Arc::ptr_eq(&first, &second));
        assert!(!Arc::ptr_eq(&first, &other_scope));
        assert!(first.buckets.len() <= MAX_TIMELINE_BUCKETS);
    }

    #[test]
    fn main_readout_cache_reuses_exact_revision_and_invalidates_on_hit() {
        let state = AppState::default();

        let first = state.main_dps_readout().expect("first main readout");
        let second = state
            .main_dps_readout()
            .expect("same-revision main readout");
        assert_eq!(first.hud, second.hud);
        assert_eq!(
            state
                .0
                .presentation
                .main_readout_projection_count
                .load(Ordering::Acquire),
            1,
            "same-revision consumers must share one projection"
        );

        let mut combat = CombatState::default();
        combat.push_hit(test_hit(250.0));
        state.restore_live_state_for_test(combat, CaptureQualitySource::Live);
        let changed = state
            .main_dps_readout()
            .expect("new-hit main readout projection");
        assert_eq!(
            changed.hud.summary.expect("live summary").total_damage,
            250.0
        );
        assert_eq!(
            state
                .0
                .presentation
                .main_readout_projection_count
                .load(Ordering::Acquire),
            2,
            "a new capture revision must invalidate the cached readout"
        );

        // Drive more exact revisions than the cache can retain. Each snapshot
        // remains independently usable, while old revisions are evicted.
        for damage in 1..=8 {
            let mut combat = CombatState::default();
            combat.push_hit(test_hit(f64::from(damage)));
            state.restore_live_state_for_test(combat, CaptureQualitySource::Live);
            let readout = state
                .main_dps_readout()
                .expect("bounded-cache main readout projection");
            assert_eq!(
                readout
                    .hud
                    .summary
                    .expect("bounded-cache summary")
                    .total_damage,
                f64::from(damage)
            );
        }
        let (cache, recovered) = state.0.presentation.lock_main_readout_cache();
        assert!(!recovered);
        assert_eq!(cache.len(), 6, "readout cache memory must remain bounded");
    }

    #[test]
    fn main_readout_can_include_max_hp_reduction_in_total_and_denominator() {
        let config_path = temporary_config_path("max_hp_reduction_total");
        let config = UiConfig {
            include_max_hp_reduction_in_total_damage: true,
            ..UiConfig::default()
        };
        let state = AppState::new_with_config_path(
            config,
            LiveCaptureService::new(LiveCaptureResources::default()),
            config_path.clone(),
        );
        let mut hit = test_hit(100.0);
        hit.max_hp_reduction = 50.0;
        let mut combat = CombatState::default();
        combat.push_hit(hit);
        state.restore_live_state_for_test(combat, CaptureQualitySource::Live);

        let readout = state.main_dps_readout().expect("combined main readout");
        let summary = readout.hud.summary.expect("combined summary");
        assert_eq!(summary.total_damage, 150.0);
        assert_eq!(summary.team_dps, 150.0);
        assert_eq!(readout.damage_attribution.total_damage, 150.0);
        assert_eq!(readout.damage_attribution.max_hp_reduction, 50.0);
        assert!((readout.hud.characters[0].damage_share_percent - 66.666_666).abs() < 0.001);

        fs::remove_dir_all(config_path.parent().expect("config parent"))
            .expect("remove temporary config");
    }

    #[test]
    fn empty_curtain_data_snapshot_is_detached_from_live_state() {
        use nte_dps_tool::engine::model::{EmptyCurtainItem, HtItemNetId};

        let state = AppState::default();
        let mut combat = CombatState::default();
        combat.empty_curtain.push(EmptyCurtainItem {
            id: HtItemNetId { solt: 1, serial: 2 },
            item_id: "fixture-item".to_owned(),
            level: 1,
            main_stats: Vec::new(),
            sub_stats: Vec::new(),
            locked: false,
            discarded: false,
            character_net_id: None,
            equipped_character_id: None,
            equipped_placement: None,
        });
        state.restore_live_state_for_test(combat, CaptureQualitySource::Live);

        let (items, characters, _) = state
            .empty_curtain_data_snapshot()
            .expect("healthy live-capture empty-curtain snapshot");
        state.restore_live_state_for_test(CombatState::default(), CaptureQualitySource::Live);

        assert_eq!(items.len(), 1);
        assert_eq!(items[0].item_id, "fixture-item");
        assert!(characters.is_empty());
    }

    #[test]
    fn pause_freezes_the_presented_state_until_resume() {
        let config_path = temporary_config_path("pause_freezes_projection");
        let live_capture = LiveCaptureService::new(LiveCaptureResources::default());
        let mut first = CombatState::default();
        first.push_hit(test_hit(125.0));
        live_capture
            .restore_session(first, CaptureQualitySource::Live)
            .expect("healthy live-capture state");
        let state = AppState::new_with_config_path(
            UiConfig::default(),
            live_capture.clone(),
            config_path.clone(),
        );

        state
            .set_main_processing_paused(true)
            .expect("pause healthy live capture");
        let paused_revision = state
            .main_dps_stream_revision()
            .expect("healthy live-capture revision");
        let mut second = CombatState::default();
        second.push_hit(test_hit(300.0));
        second.push_hit(test_hit(25.0));
        live_capture
            .restore_session(second, CaptureQualitySource::Live)
            .expect("healthy live-capture state");

        assert_eq!(
            state
                .with_main_presented_state(|presented| presented.total_damage)
                .expect("healthy paused presentation"),
            125.0
        );
        assert!(
            state
                .main_paused_event_counts()
                .expect("healthy paused event counts")
                .0
                > 0
        );
        assert_ne!(
            state
                .main_dps_stream_revision()
                .expect("healthy live-capture revision"),
            paused_revision
        );

        state
            .set_main_processing_paused(false)
            .expect("resume healthy live capture");
        assert_eq!(
            state
                .with_main_presented_state(|presented| presented.total_damage)
                .expect("healthy live presentation"),
            325.0
        );

        fs::remove_dir_all(config_path.parent().expect("config parent"))
            .expect("remove temporary config");
    }

    #[test]
    fn cut_round_freezes_replay_source_and_queues_failed_persistence() {
        let config_path = temporary_config_path("cut_round_source_retry");
        let live_capture = LiveCaptureService::new(LiveCaptureResources::default());
        let mut replay = CombatState::default();
        replay.push_hit(test_hit(444.0));
        live_capture
            .restore_session(replay, CaptureQualitySource::JsonReplay)
            .expect("healthy live-capture state");
        let state = AppState::new_with_config_path(
            UiConfig::default(),
            live_capture.clone(),
            config_path.clone(),
        );
        let initial_revision = state.history_revision();

        let prepared = state
            .prepare_current_history_archive()
            .expect("healthy live-capture history snapshot")
            .expect("current replay archive");
        assert_eq!(
            prepared.summary.quality.source,
            CaptureQualitySource::JsonReplay
        );
        assert_eq!(
            state.archive_current_history_round_with(|_, _| Err(HistoryRuntimeError::Persistence)),
            Ok(true)
        );
        assert!(
            live_capture
                .with_state(|current| current.hits.is_empty())
                .expect("healthy live-capture state")
        );

        let pending = state
            .0
            .history
            .pending_archives
            .lock()
            .expect("pending History archives lock");
        assert_eq!(pending.len(), 1);
        assert_eq!(
            pending[0].summary.quality.source,
            CaptureQualitySource::JsonReplay,
            "retry must retain source captured at the round boundary"
        );
        assert_eq!(
            state.history_revision(),
            initial_revision,
            "a precommit failure must not advance the History read model"
        );
        drop(pending);

        let mut next = CombatState::default();
        next.push_hit(test_hit(99.0));
        live_capture
            .restore_session(next, CaptureQualitySource::Live)
            .expect("healthy live-capture state");
        assert_eq!(
            live_capture
                .with_state(|current| current.total_damage)
                .expect("healthy live-capture state"),
            99.0
        );
        let retry_template = state
            .prepare_current_history_archive()
            .expect("healthy live-capture history snapshot")
            .expect("retry queue template");
        {
            let mut pending = state
                .0
                .history
                .pending_archives
                .lock()
                .expect("pending History archives lock");
            while pending.len() < MAX_PENDING_HISTORY_ARCHIVES {
                pending.push_back(retry_template.clone());
            }
        }
        assert!(
            state
                .archive_current_history_round_with(|_, _| {
                    panic!("a full retry queue must reject before persistence")
                })
                .is_err()
        );
        assert_eq!(
            live_capture
                .with_state(|current| current.total_damage)
                .expect("healthy live-capture state"),
            99.0,
            "full retry queue must preserve the current live round"
        );

        fs::remove_dir_all(config_path.parent().expect("config parent"))
            .expect("remove temporary config");
    }

    #[test]
    fn committed_maintenance_warning_is_not_requeued_and_bumps_revision_once() {
        let live_capture = LiveCaptureService::new(LiveCaptureResources::default());
        let mut current = CombatState::default();
        current.push_hit(test_hit(512.0));
        live_capture
            .restore_session(current, CaptureQualitySource::PcapngReplay)
            .expect("healthy live-capture state");
        let state = AppState::new(UiConfig::default(), live_capture);
        let initial_revision = state.history_revision();
        let attempts = AtomicU64::new(0);

        assert_eq!(
            state.archive_current_history_round_with(|state, _| {
                attempts.fetch_add(1, Ordering::AcqRel);
                Ok(state.finish_history_save(
                    nte_dps_tool::storage::history::HistorySaveOutcome::CommittedWithMaintenanceWarning {
                        record: HistoryRecord::default(),
                        warning: nte_dps_tool::storage::history::HistoryMaintenanceWarning::RetentionPruneFailed,
                    },
                ))
            }),
            Ok(true)
        );
        assert_eq!(attempts.load(Ordering::Acquire), 1);
        assert_eq!(state.history_revision(), initial_revision + 1);
        assert!(
            state
                .0
                .history
                .pending_archives
                .lock()
                .expect("pending History archives lock")
                .is_empty(),
            "post-commit maintenance warnings must not retry the committed archive"
        );
    }

    #[test]
    fn permanent_history_failure_is_reported_once_and_never_enters_the_retry_queue() {
        let live_capture = LiveCaptureService::new(LiveCaptureResources::default());
        let mut current = CombatState::default();
        current.push_hit(test_hit(700.0));
        live_capture
            .restore_session(current, CaptureQualitySource::Live)
            .expect("healthy live-capture state");
        let state = AppState::new(UiConfig::default(), live_capture);
        let attempts = AtomicU64::new(0);

        assert_eq!(
            state.archive_current_history_round_with(|_, _| {
                attempts.fetch_add(1, Ordering::AcqRel);
                Err(HistoryRuntimeError::PermanentPersistence)
            }),
            Err(HistoryRuntimeError::PermanentPersistence)
        );
        assert_eq!(attempts.load(Ordering::Acquire), 1);
        assert!(
            state
                .0
                .history
                .pending_archives
                .lock()
                .expect("pending History archives lock")
                .is_empty(),
            "permanent validation failures must not create an infinite retry loop"
        );
    }

    #[test]
    fn poisoned_history_transaction_does_not_run_the_action() {
        let state = AppState::default();
        let initial_revision = state.history_revision();
        let poison_state = state.clone();
        let _ = std::thread::spawn(move || {
            let _guard = poison_state
                .0
                .history
                .transaction
                .lock()
                .expect("history transaction lock");
            panic!("poison History transaction");
        })
        .join();

        let action_ran = AtomicBool::new(false);
        let _ = state.with_history_transaction(|| {
            action_ran.store(true, Ordering::Release);
            state.bump_history_revision();
        });

        assert!(
            !action_ran.load(Ordering::Acquire),
            "a poisoned History transaction must fail before its action runs"
        );
        assert_eq!(state.history_revision(), initial_revision);
    }

    #[test]
    fn poisoned_history_transaction_keeps_the_live_round_before_persistence() {
        let live_capture = LiveCaptureService::new(LiveCaptureResources::default());
        let mut current = CombatState::default();
        current.push_hit(test_hit(123.0));
        live_capture
            .restore_session(current, CaptureQualitySource::Live)
            .expect("healthy live-capture state");
        let state = AppState::new(UiConfig::default(), live_capture.clone());
        let initial_revision = state.history_revision();
        let poison_state = state.clone();
        let _ = std::thread::spawn(move || {
            let _guard = poison_state
                .0
                .history
                .transaction
                .lock()
                .expect("history transaction lock");
            panic!("poison History transaction before round cut");
        })
        .join();

        assert!(state.archive_current_history_round().is_err());
        assert_eq!(
            live_capture
                .with_state(|current| current.total_damage)
                .expect("healthy live-capture state"),
            123.0,
            "History runtime failure must be detected before the live round is cut"
        );
        assert_eq!(state.history_revision(), initial_revision);
    }

    #[test]
    fn poisoned_history_maintenance_stops_worker_before_waiting() {
        let state = AppState::default();
        let poison_state = state.clone();
        let _ = std::thread::spawn(move || {
            let _guard = poison_state
                .0
                .history
                .transaction
                .lock()
                .expect("history transaction lock");
            panic!("poison History maintenance transaction");
        })
        .join();
        let stop = AtomicBool::new(false);
        let wait_called = AtomicBool::new(false);

        let result = crate::history_runtime::run_history_maintenance_worker(&state, &stop, || {
            wait_called.store(true, Ordering::Release)
        });

        assert_eq!(result, Err(HistoryRuntimeError::Unavailable));
        assert!(
            !wait_called.load(Ordering::Acquire),
            "a poisoned maintenance iteration must terminate before waiting"
        );
    }

    #[test]
    fn history_retry_releases_transaction_and_requeues_unavailable_tail_in_order() {
        fn archive(damage: f64, source: CaptureQualitySource) -> PreparedHistoryArchive {
            let mut state = CombatState::default();
            state.push_hit(test_hit(damage));
            prepare_history_archive(&state, source, DpsTimeBasis::SubtractTimeStop, false)
                .expect("test History archive")
        }

        let state = AppState::default();
        {
            let mut pending = state
                .0
                .history
                .pending_archives
                .lock()
                .expect("pending History archives lock");
            pending.push_back(archive(1.0, CaptureQualitySource::Live));
            pending.push_back(archive(2.0, CaptureQualitySource::JsonReplay));
            pending.push_back(archive(3.0, CaptureQualitySource::PcapngReplay));
        }
        let mut attempted = Vec::new();

        let result = state.retry_pending_history_archives_with(|state, archive| {
            assert!(
                state.0.history.transaction.try_lock().is_ok(),
                "retry I/O must acquire a fresh per-item History transaction"
            );
            attempted.push(archive.summary.total_damage);
            if archive.summary.total_damage == 2.0 {
                Err(HistoryRuntimeError::Unavailable)
            } else {
                Ok(HistoryPersistenceOutcome::Committed)
            }
        });

        assert_eq!(result, Err(HistoryRuntimeError::Unavailable));
        assert_eq!(attempted, vec![1.0, 2.0]);
        let pending = state
            .0
            .history
            .pending_archives
            .lock()
            .expect("pending History archives lock");
        assert_eq!(
            pending
                .iter()
                .map(|archive| (archive.summary.total_damage, archive.summary.quality.source))
                .collect::<Vec<_>>(),
            vec![
                (2.0, CaptureQualitySource::JsonReplay),
                (3.0, CaptureQualitySource::PcapngReplay),
            ]
        );
    }

    #[test]
    fn history_retry_consumes_committed_warning_before_requeueing_the_unavailable_tail() {
        fn archive(damage: f64, source: CaptureQualitySource) -> PreparedHistoryArchive {
            let mut state = CombatState::default();
            state.push_hit(test_hit(damage));
            prepare_history_archive(&state, source, DpsTimeBasis::SubtractTimeStop, false)
                .expect("test History archive")
        }

        let state = AppState::default();
        {
            let mut pending = state
                .0
                .history
                .pending_archives
                .lock()
                .expect("pending History archives lock");
            pending.push_back(archive(1.0, CaptureQualitySource::JsonReplay));
            pending.push_back(archive(2.0, CaptureQualitySource::PcapngReplay));
        }
        let initial_revision = state.history_revision();
        let mut attempted = Vec::new();

        let result = state.retry_pending_history_archives_with(|state, archive| {
            attempted.push((
                archive.summary.total_damage,
                archive.summary.quality.source,
            ));
            if archive.summary.total_damage == 1.0 {
                Ok(state.finish_history_save(
                    HistorySaveOutcome::CommittedWithMaintenanceWarning {
                        record: HistoryRecord::default(),
                        warning: nte_dps_tool::storage::history::HistoryMaintenanceWarning::RetentionPruneFailed,
                    },
                ))
            } else {
                Err(HistoryRuntimeError::Unavailable)
            }
        });

        assert_eq!(result, Err(HistoryRuntimeError::Unavailable));
        assert_eq!(
            attempted,
            vec![
                (1.0, CaptureQualitySource::JsonReplay),
                (2.0, CaptureQualitySource::PcapngReplay),
            ]
        );
        assert_eq!(state.history_revision(), initial_revision + 1);
        let pending = state
            .0
            .history
            .pending_archives
            .lock()
            .expect("pending History archives lock");
        assert_eq!(pending.len(), 1);
        assert_eq!(pending[0].summary.total_damage, 2.0);
        assert_eq!(
            pending[0].summary.quality.source,
            CaptureQualitySource::PcapngReplay
        );
    }

    #[test]
    fn history_retry_drops_permanent_failure_and_continues_with_the_remaining_queue() {
        fn archive(damage: f64) -> PreparedHistoryArchive {
            let mut state = CombatState::default();
            state.push_hit(test_hit(damage));
            prepare_history_archive(
                &state,
                CaptureQualitySource::Live,
                DpsTimeBasis::SubtractTimeStop,
                false,
            )
            .expect("test History archive")
        }

        let state = AppState::default();
        {
            let mut pending = state
                .0
                .history
                .pending_archives
                .lock()
                .expect("pending History archives lock");
            pending.push_back(archive(1.0));
            pending.push_back(archive(2.0));
        }
        let mut attempted = Vec::new();

        assert_eq!(
            state.retry_pending_history_archives_with(|_, archive| {
                attempted.push(archive.summary.total_damage);
                if archive.summary.total_damage == 1.0 {
                    Err(HistoryRuntimeError::PermanentPersistence)
                } else {
                    Ok(HistoryPersistenceOutcome::Committed)
                }
            }),
            Ok(())
        );
        assert_eq!(attempted, vec![1.0, 2.0]);
        assert!(
            state
                .0
                .history
                .pending_archives
                .lock()
                .expect("pending History archives lock")
                .is_empty()
        );
    }

    #[test]
    fn poisoned_history_channel_worker_cleans_its_registry_entry() {
        let state = AppState::default();
        let stream_key = "history:poison-cleanup";
        let registration = state
            .reserve_stream("console", stream_key)
            .expect("reserve History stream");
        assert!(
            state
                .activate_stream(&registration)
                .expect("activate History stream")
        );
        let poison_state = state.clone();
        let _ = std::thread::spawn(move || {
            let _guard = poison_state
                .0
                .history
                .transaction
                .lock()
                .expect("history transaction lock");
            panic!("poison History channel transaction");
        })
        .join();
        let sent = AtomicBool::new(false);
        let waited = AtomicBool::new(false);

        crate::channels::history::run_history_stream_worker(
            &state,
            &registration,
            |_| {
                sent.store(true, Ordering::Release);
                true
            },
            || waited.store(true, Ordering::Release),
        );

        assert!(!sent.load(Ordering::Acquire));
        assert!(!waited.load(Ordering::Acquire));
        let replacement = state
            .reserve_stream("console", stream_key)
            .expect("reserve replacement History stream");
        assert!(
            !registration.is_cancelled(),
            "beginning a replacement must not find the finished poisoned worker"
        );
        state
            .finish_stream(&replacement)
            .expect("finish pending replacement");
    }

    #[test]
    fn poisoned_history_archive_transaction_keeps_the_live_round() {
        let live_capture = LiveCaptureService::new(LiveCaptureResources::default());
        let mut current = CombatState::default();
        current.push_hit(test_hit(321.0));
        live_capture
            .restore_session(current, CaptureQualitySource::Live)
            .expect("healthy live-capture state");
        let state = AppState::new(UiConfig::default(), live_capture.clone());
        let initial_revision = state.history_revision();
        let poison_state = state.clone();
        let _ = std::thread::spawn(move || {
            let _guard = poison_state
                .0
                .history
                .archive_transaction
                .lock()
                .expect("history archive transaction lock");
            panic!("poison History archive transaction");
        })
        .join();

        let persist_ran = AtomicBool::new(false);
        assert!(
            state
                .archive_current_history_round_with(|_, _| {
                    persist_ran.store(true, Ordering::Release);
                    Ok(HistoryPersistenceOutcome::Committed)
                })
                .is_err()
        );
        assert_eq!(state.history_revision(), initial_revision);
        assert!(!persist_ran.load(Ordering::Acquire));
        assert_eq!(
            live_capture
                .with_state(|current| current.total_damage)
                .expect("healthy live-capture state"),
            321.0,
            "the live round must not be cut when archive serialization is unavailable"
        );
    }

    #[test]
    fn poisoned_pending_history_queue_keeps_the_live_round() {
        let live_capture = LiveCaptureService::new(LiveCaptureResources::default());
        let mut current = CombatState::default();
        current.push_hit(test_hit(654.0));
        live_capture
            .restore_session(current, CaptureQualitySource::Live)
            .expect("healthy live-capture state");
        let state = AppState::new(UiConfig::default(), live_capture.clone());
        let poison_state = state.clone();
        let _ = std::thread::spawn(move || {
            let _guard = poison_state
                .0
                .history
                .pending_archives
                .lock()
                .expect("pending History archives lock");
            panic!("poison pending History archives");
        })
        .join();

        let persist_ran = AtomicBool::new(false);
        let initial_revision = state.history_revision();
        assert!(
            state
                .archive_current_history_round_with(|_, _| {
                    persist_ran.store(true, Ordering::Release);
                    Ok(HistoryPersistenceOutcome::Committed)
                })
                .is_err()
        );
        assert!(!persist_ran.load(Ordering::Acquire));
        assert_eq!(state.history_revision(), initial_revision);
        assert_eq!(
            live_capture
                .with_state(|current| current.total_damage)
                .expect("healthy live-capture state"),
            654.0,
            "the live round must not be cut when retry capacity cannot be checked"
        );
    }

    #[test]
    fn poisoned_history_round_cache_is_discarded_before_rebuild() {
        let state = AppState::default();
        let poison_state = state.clone();
        let _ = std::thread::spawn(move || {
            let mut cache = poison_state
                .0
                .history
                .round_cache
                .lock()
                .expect("History round cache lock");
            cache.revision = Some(poison_state.history_revision());
            cache.index = Arc::new(vec![HistoryRoundIndex::for_test(
                "poisoned-cache-entry",
                false,
            )]);
            panic!("poison History round cache");
        })
        .join();

        let index = state.main_round_index();

        assert!(
            index
                .iter()
                .all(|record| record.id != "poisoned-cache-entry"),
            "a partially mutated cache must never be projected"
        );
        assert!(
            !state.0.history.round_cache.is_poisoned(),
            "a rebuilt cache must clear the poison flag"
        );
    }

    #[test]
    fn poisoned_history_undo_slot_rejects_mutation() {
        let state = AppState::default();
        let initial_revision = state.history_revision();
        let (directory, tombstone) = temporary_history_tombstone("poisoned-slot");
        let poison_state = state.clone();
        let _ = std::thread::spawn(move || {
            let _guard = poison_state
                .0
                .history
                .undo
                .lock()
                .expect("History undo lock");
            panic!("poison History undo slot");
        })
        .join();

        let result = state.remember_deleted_history_tombstone(tombstone.clone());

        assert!(result.is_err());
        assert_eq!(state.history_revision(), initial_revision);
        discard_tombstoned_record(&tombstone).expect("discard unused tombstone fixture");
        let _ = fs::remove_dir_all(directory);
    }

    #[test]
    fn history_persistence_error_display_is_stable_and_redacted() {
        let error = HistoryRuntimeError::Persistence;

        assert_eq!(error.to_string(), "History persistence did not finish.");
        assert!(!error.to_string().contains("private-storage-detail"));
    }

    #[test]
    fn reset_undo_restores_session_and_rejects_a_wrong_token_without_consuming_it() {
        let config_path = temporary_config_path("session_reset_undo");
        let live_capture = LiveCaptureService::new(LiveCaptureResources::default());
        let mut previous = CombatState::default();
        previous.push_hit(test_hit(222.0));
        live_capture
            .restore_session(previous, CaptureQualitySource::Live)
            .expect("healthy live-capture state");
        let state =
            AppState::new_with_config_path(UiConfig::default(), live_capture, config_path.clone());

        state
            .set_main_processing_paused(true)
            .expect("pause healthy live capture");
        let token = state
            .reset_session_with_undo()
            .expect("healthy live-capture reset")
            .expect("reset undo token");
        assert!(
            !state
                .session_has_data()
                .expect("healthy live-capture state")
        );
        assert!(!state.main_processing_paused());
        assert_eq!(
            state.undo_session_reset("wrong-token"),
            Err(SessionUndoError::Missing)
        );
        state
            .undo_session_reset(&token)
            .expect("restore reset session");
        assert_eq!(
            state
                .with_main_presented_state(|presented| presented.total_damage)
                .expect("healthy restored presentation"),
            222.0
        );

        fs::remove_dir_all(config_path.parent().expect("config parent"))
            .expect("remove temporary config");
    }

    #[test]
    fn failed_session_undo_restore_keeps_the_entry_for_retry() {
        let state = AppState::default();
        let entry = SessionUndoEntry {
            token: "session-undo-fixture".to_owned(),
            state: CombatState::default(),
            quality_source: CaptureQualitySource::JsonReplay,
            expires_at: Instant::now() + SESSION_UNDO_WINDOW,
        };

        *state
            .0
            .session_undo
            .entry
            .lock()
            .expect("session undo lock") = Some(entry.clone());
        let result = state.restore_session_undo_entry(&entry, |_, _| {
            Err(CoreError::new(
                CoreErrorCode::CaptureStateUnavailable,
                "private poison detail",
            ))
        });

        assert_eq!(result, Err(SessionUndoError::StateUnavailable));
        let undo = state
            .0
            .session_undo
            .entry
            .lock()
            .expect("session undo lock");
        let restored = undo.as_ref().expect("failed undo entry remains available");
        assert_eq!(restored.token, "session-undo-fixture");
        assert_eq!(restored.quality_source, CaptureQualitySource::JsonReplay);
    }

    #[test]
    fn onboarding_progress_and_completion_persist() {
        let config_path = temporary_config_path("onboarding_progress");
        let state = AppState::new_with_config_path(
            UiConfig::default(),
            LiveCaptureService::new(LiveCaptureResources::default()),
            config_path.clone(),
        );

        state
            .set_onboarding_progress(2, false)
            .expect("save onboarding step");
        assert_eq!(state.onboarding_step(), 2);
        state
            .finish_onboarding(HudPreset::Detailed)
            .expect("finish onboarding");

        let saved: UiConfig =
            serde_json::from_str(&fs::read_to_string(&config_path).expect("saved UI config"))
                .expect("valid saved UI config");
        assert!(saved.onboarding_done);
        assert!(saved.hud.show_mini_timeline);
        assert_eq!(state.onboarding_step(), 3);

        fs::remove_dir_all(config_path.parent().expect("config parent"))
            .expect("remove temporary config");
    }

    #[test]
    fn replay_import_reservation_blocks_live_capture_until_released() {
        let state = AppState::default();
        let reservation = state
            .begin_replay_import(false)
            .expect("reserve replay import");

        let duplicate_error = match state.begin_replay_import(false) {
            Ok(_) => panic!("second replay reservation must be rejected"),
            Err(error) => error,
        };
        assert!(matches!(
            duplicate_error,
            ReplayImportError::Capture(error)
                if error.code == CoreErrorCode::CaptureAlreadyRunning
        ));
        assert_eq!(
            state
                .request_capture_start(false)
                .expect_err("capture start must respect replay reservation")
                .code,
            CoreErrorCode::CaptureAlreadyRunning
        );

        drop(reservation);
        assert!(state.begin_replay_import(false).is_ok());
    }

    #[test]
    fn invalid_json_replay_releases_reservation_without_replacing_session() {
        let live_capture = LiveCaptureService::new(LiveCaptureResources::default());
        let mut previous = CombatState::default();
        previous.push_hit(test_hit(432.0));
        live_capture
            .restore_session(previous, CaptureQualitySource::PcapngReplay)
            .expect("install existing session");
        let state = AppState::new(UiConfig::default(), live_capture);
        let initial_revision = state.0.live_capture.revision();
        let initial_packet_revision = state
            .0
            .live_capture
            .with_packet_state(|revision, _, _| revision)
            .expect("healthy packet projection");
        let initial_source = state
            .0
            .live_capture
            .quality_source()
            .expect("healthy capture source");
        let path = temporary_config_path("invalid_json_replay_preflight").with_extension("json");
        fs::write(&path, b"{").expect("write malformed JSON replay");

        let error = state
            .begin_replay_import(true)
            .expect("reserve replacing replay")
            .start(CaptureReplayKind::Json, path.clone(), true)
            .expect_err("malformed replay must fail synchronously");

        fs::remove_file(path).expect("remove malformed replay fixture");
        assert!(matches!(
            error,
            ReplayImportError::JsonImport(CaptureImportError::InvalidFormat)
        ));
        assert_eq!(state.0.live_capture.revision(), initial_revision);
        assert_eq!(
            state
                .0
                .live_capture
                .with_packet_state(|revision, _, _| revision)
                .expect("healthy packet projection"),
            initial_packet_revision
        );
        assert_eq!(
            state
                .0
                .live_capture
                .quality_source()
                .expect("healthy capture source"),
            initial_source
        );
        assert_eq!(
            state
                .0
                .live_capture
                .with_state(|combat| (combat.hits.len(), combat.total_damage))
                .expect("healthy capture state"),
            (1, 432.0)
        );
        assert_eq!(
            *state
                .0
                .replay_import
                .reservation
                .lock()
                .expect("healthy reservation runtime"),
            ReplayImportReservationState::Idle
        );
    }

    #[test]
    fn poisoned_replay_import_runtime_rejects_reservation_before_replay_work() {
        let state = AppState::default();
        let initial_revision = state.0.live_capture.revision();
        let poison_state = state.clone();
        let _ = std::thread::spawn(move || {
            let _reserved = poison_state
                .0
                .replay_import
                .reservation
                .lock()
                .expect("replay import reservation lock");
            panic!("poison replay import reservation");
        })
        .join();

        let error = match state.begin_replay_import(false) {
            Ok(_) => panic!("poisoned replay import runtime must fail closed"),
            Err(error) => error,
        };

        assert!(matches!(error, ReplayImportError::RuntimeUnavailable));
        assert_eq!(
            state
                .request_capture_start(false)
                .expect_err("poisoned replay runtime must reject live capture")
                .code,
            CoreErrorCode::CaptureStateUnavailable
        );
        assert_eq!(state.0.live_capture.revision(), initial_revision);
    }

    #[test]
    fn replay_import_reservation_lock_is_released_before_expensive_work() {
        let state = AppState::default();
        let reservation = state
            .begin_replay_import(false)
            .expect("reserve replay import");

        reservation
            .finish_with(|operation_state| {
                let reservation = operation_state
                    .0
                    .replay_import
                    .reservation
                    .try_lock()
                    .expect("reservation lock must not span replay I/O or capture wait");
                assert_eq!(*reservation, ReplayImportReservationState::ReplayImport);
                Ok(())
            })
            .expect("finish replay reservation");

        assert_eq!(
            *state
                .0
                .replay_import
                .reservation
                .lock()
                .expect("replay import reservation lock"),
            ReplayImportReservationState::Idle
        );
    }

    #[test]
    fn poisoned_replay_import_runtime_rejects_start_without_consuming_reservation() {
        let state = AppState::default();
        let reservation = state
            .begin_replay_import(false)
            .expect("reserve replay import");
        let poison_state = state.clone();
        let _ = std::thread::spawn(move || {
            let _reserved = poison_state
                .0
                .replay_import
                .reservation
                .lock()
                .expect("replay import reservation lock");
            panic!("poison replay import reservation after dialog reservation");
        })
        .join();
        let initial_revision = state.0.live_capture.revision();

        let error = reservation
            .start(
                CaptureReplayKind::Json,
                PathBuf::from("missing-poisoned-replay.json"),
                false,
            )
            .expect_err("poisoned replay import runtime must reject start");

        assert!(matches!(error, ReplayImportError::RuntimeUnavailable));
        assert_eq!(state.0.live_capture.revision(), initial_revision);
        let reserved = state
            .0
            .replay_import
            .reservation
            .lock()
            .expect_err("runtime remains poisoned")
            .into_inner();
        assert_eq!(
            *reserved,
            ReplayImportReservationState::ReplayImport,
            "Drop must not blindly mutate poisoned state"
        );
    }

    #[test]
    fn poisoned_session_undo_runtime_rejects_reset_without_sequence_or_capture_change() {
        let live_capture = LiveCaptureService::new(LiveCaptureResources::default());
        let mut previous = CombatState::default();
        previous.push_hit(test_hit(444.0));
        live_capture
            .restore_session(previous, CaptureQualitySource::Live)
            .expect("healthy live-capture state");
        let state = AppState::new(UiConfig::default(), live_capture);
        let initial_sequence = state.0.session_undo.sequence.load(Ordering::Acquire);
        let initial_revision = state.0.live_capture.revision();
        let poison_state = state.clone();
        let _ = std::thread::spawn(move || {
            let _undo = poison_state
                .0
                .session_undo
                .entry
                .lock()
                .expect("session undo lock");
            panic!("poison session undo runtime");
        })
        .join();

        let error = state
            .reset_session_with_undo_action()
            .expect_err("poisoned session undo runtime must reject reset");

        assert_eq!(error, SessionUndoError::RuntimeUnavailable);
        assert_eq!(
            state.0.session_undo.sequence.load(Ordering::Acquire),
            initial_sequence
        );
        assert_eq!(state.0.live_capture.revision(), initial_revision);
        assert_eq!(
            state
                .0
                .live_capture
                .with_state(|current| current.total_damage)
                .expect("healthy live-capture state"),
            444.0
        );
    }

    #[test]
    fn poisoned_session_undo_runtime_rejects_restore_without_capture_change() {
        let state = AppState::default();
        let entry = SessionUndoEntry {
            token: "poisoned-session-undo".to_owned(),
            state: {
                let mut previous = CombatState::default();
                previous.push_hit(test_hit(333.0));
                previous
            },
            quality_source: CaptureQualitySource::JsonReplay,
            expires_at: Instant::now() + SESSION_UNDO_WINDOW,
        };
        *state
            .0
            .session_undo
            .entry
            .lock()
            .expect("session undo lock") = Some(entry);
        let poison_state = state.clone();
        let _ = std::thread::spawn(move || {
            let _undo = poison_state
                .0
                .session_undo
                .entry
                .lock()
                .expect("session undo lock");
            panic!("poison session undo runtime before restore");
        })
        .join();
        let initial_revision = state.0.live_capture.revision();

        assert_eq!(
            state.undo_session_reset("poisoned-session-undo"),
            Err(SessionUndoError::RuntimeUnavailable)
        );
        assert_eq!(state.0.live_capture.revision(), initial_revision);
        assert!(
            !state
                .session_has_data()
                .expect("healthy live-capture state"),
            "poisoned undo state must not be restored"
        );
    }

    #[test]
    fn poisoned_session_undo_runtime_rejects_clear_without_capture_change() {
        let live_capture = LiveCaptureService::new(LiveCaptureResources::default());
        let mut previous = CombatState::default();
        previous.push_hit(test_hit(555.0));
        live_capture
            .restore_session(previous, CaptureQualitySource::Live)
            .expect("healthy live-capture state");
        let state = AppState::new(UiConfig::default(), live_capture);
        let initial_revision = state.0.live_capture.revision();
        let poison_state = state.clone();
        let _ = std::thread::spawn(move || {
            let _undo = poison_state
                .0
                .session_undo
                .entry
                .lock()
                .expect("session undo lock");
            panic!("poison session undo runtime before clear");
        })
        .join();

        assert_eq!(
            state.clear_session_action(),
            Err(SessionUndoError::RuntimeUnavailable)
        );
        assert_eq!(state.0.live_capture.revision(), initial_revision);
        assert_eq!(
            state
                .0
                .live_capture
                .with_state(|current| current.total_damage)
                .expect("healthy live-capture state"),
            555.0
        );
    }

    #[test]
    fn capture_file_refresh_advances_settings_generation() {
        let state = AppState::default();
        let generation = state.settings_revision();

        state.refresh_capture_file_stats();

        assert!(state.settings_revision() > generation);
    }

    #[test]
    fn device_catalog_revision_effects_are_change_and_availability_aware() {
        let state = AppState::default();
        let candidate = vec![CaptureDeviceSnapshot {
            id: "revision-fixture-device".to_owned(),
            label: "Revision fixture".to_owned(),
        }];
        let settings_before = state.settings_revision();
        let main_before = state.0.presentation.main_revision.load(Ordering::Acquire);

        state
            .refresh_capture_devices_with(|| Ok(candidate.clone()))
            .expect("replace device catalog");
        assert_eq!(state.settings_revision(), settings_before + 1);
        assert_eq!(
            state.0.presentation.main_revision.load(Ordering::Acquire),
            main_before + 1
        );

        let settings_after_replace = state.settings_revision();
        let main_after_replace = state.0.presentation.main_revision.load(Ordering::Acquire);
        state
            .refresh_capture_devices_with(|| Ok(candidate.clone()))
            .expect("identical device catalog");
        assert_eq!(state.settings_revision(), settings_after_replace);
        assert_eq!(
            state.0.presentation.main_revision.load(Ordering::Acquire),
            main_after_replace
        );

        assert!(
            state
                .refresh_capture_devices_with(|| {
                    Err(CoreError::new(
                        CoreErrorCode::SystemProbeFailed,
                        "private adapter detail",
                    ))
                })
                .is_err()
        );
        assert_eq!(state.settings_revision(), settings_after_replace + 1);
        assert_eq!(
            state.0.presentation.main_revision.load(Ordering::Acquire),
            main_after_replace + 1
        );
        let degraded = state.0.settings.device_catalog_snapshot(|| {});
        assert!(!degraded.available);
        assert_eq!(degraded.devices, candidate);

        let settings_after_failure = state.settings_revision();
        let main_after_failure = state.0.presentation.main_revision.load(Ordering::Acquire);
        assert!(
            state
                .refresh_capture_devices_with(|| {
                    Err(CoreError::new(
                        CoreErrorCode::SystemProbeFailed,
                        "another private adapter detail",
                    ))
                })
                .is_err()
        );
        assert_eq!(state.settings_revision(), settings_after_failure);
        assert_eq!(
            state.0.presentation.main_revision.load(Ordering::Acquire),
            main_after_failure
        );
    }

    #[test]
    fn encrypted_ini_session_orders_save_and_clear_generations() {
        let config_path = temporary_config_path("encrypted_ini_session");
        let ini_path = config_path.with_file_name("Engine.ini");
        fs::write(&ini_path, "Value=1\n").expect("write INI fixture");
        let state = AppState::default();

        let opened = state
            .open_encrypted_ini(ini_path.clone())
            .expect("open INI fixture");
        assert_eq!(opened.generation, 1);
        assert_eq!(opened.plaintext, "Value=1");
        assert!(matches!(
            state.save_encrypted_ini(0, "Value=2".to_owned(), EncryptedIniKey::Global,),
            Err(EncryptedIniServiceError::StaleGeneration)
        ));

        let (saved, outcome) = state
            .save_encrypted_ini(
                opened.generation,
                "Value=2".to_owned(),
                EncryptedIniKey::Global,
            )
            .expect("save INI fixture");
        assert_eq!(outcome, EncryptedIniSaveOutcome::Saved);
        assert_eq!(saved.generation, 2);
        assert_eq!(saved.plaintext, "Value=2");
        assert_eq!(
            state
                .clear_encrypted_ini()
                .expect("clear encrypted INI")
                .generation,
            3
        );

        fs::remove_dir_all(config_path.parent().expect("fixture parent"))
            .expect("remove INI fixture");
    }

    #[test]
    fn abyss_team_mutations_keep_each_prediction_line_explicit() {
        let state = AppState::default();
        let team = TeamDps {
            dps: 12_345.0,
            members: Vec::new(),
        };
        let export = TeamDpsExport {
            version: nte_dps_tool::engine::model::TEAM_DPS_EXPORT_VERSION,
            single: Some(team.clone()),
            upper: None,
            lower: None,
        };

        let initial_revision = state.settings_revision();
        assert!(
            state
                .import_abyss_team(export.clone(), true)
                .expect("import upper team")
        );
        assert!(state.settings_revision() > initial_revision);
        let imported_revision = state.settings_revision();
        assert!(
            state
                .import_abyss_team(export, true)
                .expect("repeat upper team")
        );
        assert_eq!(state.settings_revision(), imported_revision);
        let (upper, lower) = state.imported_abyss_teams().expect("imported teams");
        assert_eq!(upper.as_ref().map(|team| team.dps), Some(12_345.0));
        assert!(lower.is_none());

        assert!(state.swap_abyss_teams().expect("swap imported teams"));
        let (upper, lower) = state.imported_abyss_teams().expect("swapped teams");
        assert!(upper.is_none());
        assert_eq!(lower.as_ref().map(|team| team.dps), Some(12_345.0));

        assert!(state.clear_abyss_team(false).expect("clear lower team"));
        let (upper, lower) = state.imported_abyss_teams().expect("cleared teams");
        assert!(upper.is_none());
        assert!(lower.is_none());
    }

    #[test]
    fn poisoned_team_state_is_projected_unavailable_once_without_partial_data() {
        let state = AppState::default();
        let export = TeamDpsExport {
            version: nte_dps_tool::engine::model::TEAM_DPS_EXPORT_VERSION,
            single: Some(TeamDps {
                dps: 42.0,
                members: Vec::new(),
            }),
            upper: None,
            lower: None,
        };
        state.import_team_data(export).expect("seed imported teams");
        state.0.team_import.poison_for_test();
        let revision_before_recovery = state.settings_revision();

        let snapshot = state.settings_snapshot();

        assert!(!snapshot.team_data.available);
        assert!(!snapshot.team_data.upper_imported);
        assert!(!snapshot.team_data.lower_imported);
        assert_eq!(state.settings_revision(), revision_before_recovery + 1);
        assert!(state.imported_abyss_teams().is_err());
        assert_eq!(state.settings_revision(), revision_before_recovery + 1);
    }

    #[test]
    fn finishing_replaced_stream_keeps_current_registration() {
        let state = AppState::default();
        let previous = state
            .reserve_stream("hud", "technical")
            .expect("reserve previous stream");
        assert!(
            state
                .activate_stream(&previous)
                .expect("activate previous stream")
        );
        let current = state
            .reserve_stream("hud", "technical")
            .expect("reserve current stream");
        assert!(
            state
                .activate_stream(&current)
                .expect("activate current stream")
        );

        assert!(!state.finish_stream(&previous).expect("finish stale stream"));
        assert!(
            state
                .stop_stream("hud", "technical")
                .expect("stop current stream")
        );

        assert!(current.is_cancelled());
    }

    #[test]
    fn destroying_owner_window_stops_only_its_streams_and_clears_registry_entries() {
        let state = AppState::default();
        let hud_first = state
            .reserve_stream("hud", "hud:first")
            .expect("reserve HUD stream");
        let hud_second = state
            .reserve_stream("hud", "hud:second")
            .expect("reserve HUD stream");
        let console = state
            .reserve_stream("console", "console:first")
            .expect("reserve Console stream");
        assert!(
            state
                .activate_stream(&hud_first)
                .expect("activate HUD stream")
        );
        assert!(
            state
                .activate_stream(&hud_second)
                .expect("activate HUD stream")
        );
        assert!(
            state
                .activate_stream(&console)
                .expect("activate Console stream")
        );
        assert_eq!(state.stream_registry_len().expect("registry length"), 3);

        assert_eq!(
            state
                .stop_streams_for_window("hud")
                .expect("stop HUD streams"),
            2
        );
        assert!(hud_first.is_cancelled());
        assert!(hud_second.is_cancelled());
        assert!(!console.is_cancelled());
        assert_eq!(
            state.stop_streams_for_window("hud").expect("repeat stop"),
            0
        );
        assert_eq!(state.stream_registry_len().expect("registry length"), 1);

        assert!(
            state
                .stop_stream("console", "console:first")
                .expect("stop Console stream")
        );
        assert!(console.is_cancelled());
        assert_eq!(state.stream_registry_len().expect("registry length"), 0);
    }

    #[test]
    fn editor_snapshot_uses_rust_preview_and_passthrough_snapshot_is_empty() {
        let state = AppState::default();

        assert_eq!(
            state
                .snapshot()
                .expect("healthy live-capture snapshot")
                .hud
                .data_state,
            HudDataState::Preview
        );

        state.set_passthrough(true);

        assert_eq!(
            state
                .snapshot()
                .expect("healthy live-capture snapshot")
                .hud
                .data_state,
            HudDataState::Empty
        );
    }

    #[test]
    fn initial_window_and_hud_projection_follow_loaded_config() {
        let mut config = UiConfig {
            always_on_top: false,
            passthrough_hotkey: HotkeyBinding::new(false, false, false, config::HotkeyKey::F8),
            ..UiConfig::default()
        };
        config.hud.width = 512;
        config.hud.show_total_damage = false;

        let state = AppState::new(
            config,
            LiveCaptureService::new(LiveCaptureResources::default()),
        );
        let snapshot = state.snapshot().expect("healthy live-capture snapshot");

        assert!(!state.always_on_top());
        assert_eq!(
            state.passthrough_hotkey(),
            HotkeyBinding::new(false, false, false, config::HotkeyKey::F8)
        );
        assert!(!state.passthrough_hotkey_ready());
        assert_eq!(state.hud_width(), 512);
        assert_eq!(
            state.hud_initial_height(),
            (HUD_BASE_INITIAL_HEIGHT
                + HUD_SUMMARY_HEIGHT
                + HUD_CHARACTERS_HEIGHT
                + HUD_EDITOR_MODULE_HEADER_HEIGHT * 2)
                .max(HUD_EDITOR_MIN_HEIGHT)
        );
        assert_eq!(snapshot.hud.config.width, 512);
        assert!(!snapshot.hud.config.show_total_damage);
    }

    #[test]
    fn passthrough_hotkey_readiness_is_runtime_only() {
        let state = AppState::default();
        let initial_revision = state.stream_revision();

        state.set_passthrough_hotkey_ready(true);

        assert!(state.passthrough_hotkey_ready());
        assert_eq!(state.stream_revision(), initial_revision);
    }

    #[test]
    fn poisoned_paused_presentation_discards_partial_snapshot_once() {
        let state = AppState::default();
        state
            .set_main_processing_paused(true)
            .expect("pause healthy presentation");
        let initial_stream_revision = state.stream_revision();
        let initial_main_revision = state.0.presentation.main_revision.load(Ordering::Acquire);
        let poison_state = state.clone();
        let _ = std::thread::spawn(move || {
            let mut paused = poison_state
                .0
                .presentation
                .mode
                .lock()
                .expect("presentation mode lock");
            let mut partial = CombatState::default();
            partial.push_hit(test_hit(987_654.0));
            paused.paused.as_mut().expect("paused snapshot").state = Arc::new(partial);
            paused.selected_round = Some(SelectedRoundPresentation {
                record_id: "private-partial-round".to_owned(),
                history_revision: 0,
                state: Arc::new(CombatState::default()),
            });
            panic!("poison paused presentation after a partial replacement");
        })
        .join();

        assert_eq!(
            state
                .with_main_presented_state(|current| current.total_damage)
                .expect("recover presentation to live"),
            0.0
        );
        assert!(!state.main_processing_paused());
        assert!(state.main_selected_round_id().is_none());
        assert!(!state.0.presentation.mode.is_poisoned());
        assert_ne!(state.stream_revision(), initial_stream_revision);
        assert_eq!(
            state.0.presentation.main_revision.load(Ordering::Acquire),
            initial_main_revision + 1
        );
        let recovered_stream_revision = state.stream_revision();
        let recovered_main_revision = state.0.presentation.main_revision.load(Ordering::Acquire);
        assert_eq!(
            state
                .with_main_presented_state(|current| current.total_damage)
                .expect("repeat healthy live projection"),
            0.0
        );
        assert_eq!(state.stream_revision(), recovered_stream_revision);
        assert_eq!(
            state.0.presentation.main_revision.load(Ordering::Acquire),
            recovered_main_revision
        );
    }

    #[test]
    fn mode_noops_do_not_advance_technical_or_main_revisions() {
        let state = AppState::default();
        let initial_stream_revision = state.stream_revision();
        let initial_main_revision = state.0.presentation.main_revision.load(Ordering::Acquire);

        assert!(
            !state
                .set_main_processing_paused(false)
                .expect("repeat live mode")
        );
        assert!(
            !state
                .set_main_selected_round_id(None)
                .expect("repeat live round")
        );
        state.return_main_presentation_to_live();

        assert_eq!(state.stream_revision(), initial_stream_revision);
        assert_eq!(
            state.0.presentation.main_revision.load(Ordering::Acquire),
            initial_main_revision
        );
    }

    #[test]
    fn selected_abyss_effects_are_exact_and_noop_aware() {
        let state = AppState::default();
        let initial_stream_revision = state.stream_revision();
        let initial_main_revision = state.0.presentation.main_revision.load(Ordering::Acquire);

        assert!(
            state
                .set_main_selected_abyss_half(Some(AbyssHalf::Second))
                .expect("select second abyss half")
        );

        assert_ne!(state.stream_revision(), initial_stream_revision);
        assert_eq!(
            state.0.presentation.main_revision.load(Ordering::Acquire),
            initial_main_revision + 1
        );
        let changed_stream_revision = state.stream_revision();
        let changed_main_revision = state.0.presentation.main_revision.load(Ordering::Acquire);

        assert!(
            !state
                .set_main_selected_abyss_half(Some(AbyssHalf::Second))
                .expect("repeat second abyss half")
        );

        assert_eq!(state.stream_revision(), changed_stream_revision);
        assert_eq!(
            state.0.presentation.main_revision.load(Ordering::Acquire),
            changed_main_revision
        );
    }

    #[test]
    fn history_selection_ignores_incoming_hits_then_returns_live_on_outgoing_hit() {
        fn replay_path(tag: &str, direction: &str) -> PathBuf {
            let path = temporary_config_path(tag).with_extension("json");
            let document = serde_json::json!({
                "version": 1,
                "hits": [{
                    "timestamp_unix": 1.0,
                    "char_id": 7,
                    "char_name": "Fixture",
                    "damage": 25.0,
                    "direction": direction
                }],
                "packets": []
            });
            fs::write(
                &path,
                serde_json::to_vec(&document).expect("serialize replay fixture"),
            )
            .expect("write replay fixture");
            path
        }

        fn run_replay(state: &AppState, path: PathBuf) {
            state
                .0
                .live_capture
                .request_replay(CaptureReplayKind::Json, path, None, true, false)
                .expect("start JSON replay");
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
            while state.live_capture_status().phase != LiveCapturePhase::Stopped {
                assert!(
                    std::time::Instant::now() < deadline,
                    "JSON replay did not reach its stopped barrier"
                );
                std::thread::yield_now();
            }
        }

        let state = AppState::default();
        state
            .set_main_selected_abyss_half(Some(AbyssHalf::Second))
            .expect("select second half");
        let selected_outgoing_revision = state.0.live_capture.outgoing_hit_revision();
        {
            let (mut mode, recovered) = state.0.presentation.lock_mode();
            assert!(!recovered);
            mode.selected_outgoing_revision = selected_outgoing_revision;
            mode.selected_round = Some(SelectedRoundPresentation {
                record_id: "history-round".to_owned(),
                history_revision: state.history_revision(),
                state: Arc::new(CombatState::default()),
            });
        }

        let incoming_path = replay_path("history_incoming_stays_selected", "incoming");
        run_replay(&state, incoming_path.clone());
        assert_eq!(
            state.0.live_capture.outgoing_hit_revision(),
            selected_outgoing_revision
        );
        assert_eq!(
            state.main_selected_round_id().as_deref(),
            Some("history-round")
        );
        assert_eq!(
            state.abyss_presentation_snapshot().selected,
            Some(AbyssHalf::Second)
        );

        let outgoing_path = replay_path("history_outgoing_returns_live", "outgoing");
        run_replay(&state, outgoing_path.clone());
        assert_eq!(
            state.0.live_capture.outgoing_hit_revision(),
            selected_outgoing_revision + 1
        );
        assert!(state.main_selected_round_id().is_none());
        assert_eq!(
            state.abyss_presentation_snapshot().selected,
            Some(AbyssHalf::Second),
            "switching back to live must not erase the user's half selection"
        );

        fs::remove_dir_all(incoming_path.parent().expect("incoming fixture parent"))
            .expect("remove incoming fixture");
        fs::remove_dir_all(outgoing_path.parent().expect("outgoing fixture parent"))
            .expect("remove outgoing fixture");
    }

    #[test]
    fn detail_request_effect_is_exact_and_noop_aware() {
        let state = AppState::default();
        let initial_stream_revision = state.stream_revision();
        let initial_main_revision = state.0.presentation.main_revision.load(Ordering::Acquire);
        let request = MainDpsDetailRequest {
            character_id: Some(10),
            filter: CombatDetailFilter::Outgoing,
            skill_filter: Some("skill".to_owned()),
        };

        assert!(
            state
                .set_main_dps_detail_request(MainDpsDetailKind::Character, request.clone())
                .expect("set character detail request")
        );

        assert_eq!(state.stream_revision(), initial_stream_revision);
        assert_eq!(
            state.0.presentation.main_revision.load(Ordering::Acquire),
            initial_main_revision + 1
        );
        assert!(
            !state
                .set_main_dps_detail_request(MainDpsDetailKind::Character, request)
                .expect("repeat character detail request")
        );
        assert_eq!(
            state.0.presentation.main_revision.load(Ordering::Acquire),
            initial_main_revision + 1
        );
    }

    #[test]
    fn poisoned_abyss_domain_resets_both_fields_once() {
        let state = AppState::default();
        let initial_stream_revision = state.stream_revision();
        let initial_main_revision = state.0.presentation.main_revision.load(Ordering::Acquire);
        let poison_state = state.clone();
        let _ = std::thread::spawn(move || {
            let mut abyss = poison_state
                .0
                .presentation
                .abyss
                .lock()
                .expect("abyss presentation lock");
            abyss.selected = Some(AbyssHalf::Second);
            abyss.observed = Some(AbyssHalf::First);
            panic!("poison abyss presentation after a partial transition");
        })
        .join();

        assert_eq!(
            state.abyss_presentation_snapshot(),
            AbyssPresentationState::default()
        );
        assert!(!state.0.presentation.abyss.is_poisoned());
        assert_ne!(state.stream_revision(), initial_stream_revision);
        assert_eq!(
            state.0.presentation.main_revision.load(Ordering::Acquire),
            initial_main_revision + 1
        );
        let recovered_stream_revision = state.stream_revision();
        let recovered_main_revision = state.0.presentation.main_revision.load(Ordering::Acquire);
        assert_eq!(
            state.abyss_presentation_snapshot(),
            AbyssPresentationState::default()
        );
        assert_eq!(state.stream_revision(), recovered_stream_revision);
        assert_eq!(
            state.0.presentation.main_revision.load(Ordering::Acquire),
            recovered_main_revision
        );
    }

    #[test]
    fn poisoned_detail_domain_resets_both_requests_once() {
        let state = AppState::default();
        let initial_stream_revision = state.stream_revision();
        let initial_main_revision = state.0.presentation.main_revision.load(Ordering::Acquire);
        let poison_state = state.clone();
        let _ = std::thread::spawn(move || {
            let mut details = poison_state
                .0
                .presentation
                .details
                .lock()
                .expect("detail presentation lock");
            details.character.character_id = Some(7);
            details.team.skill_filter = Some("private-partial-filter".to_owned());
            panic!("poison detail presentation after a partial transition");
        })
        .join();

        assert_eq!(
            state.main_dps_detail_request(MainDpsDetailKind::Character),
            MainDpsDetailRequest::default()
        );
        assert_eq!(
            state.main_dps_detail_request(MainDpsDetailKind::Team),
            MainDpsDetailRequest::default()
        );
        assert!(!state.0.presentation.details.is_poisoned());
        assert_eq!(state.stream_revision(), initial_stream_revision);
        assert_eq!(
            state.0.presentation.main_revision.load(Ordering::Acquire),
            initial_main_revision + 1
        );
        let recovered_main_revision = state.0.presentation.main_revision.load(Ordering::Acquire);
        assert_eq!(
            state.main_dps_detail_request(MainDpsDetailKind::Character),
            MainDpsDetailRequest::default()
        );
        assert_eq!(
            state.0.presentation.main_revision.load(Ordering::Acquire),
            recovered_main_revision
        );
    }

    #[test]
    fn poisoned_detail_mutation_reports_unavailable_after_reset() {
        let state = AppState::default();
        let poison_state = state.clone();
        let _ = std::thread::spawn(move || {
            let _details = poison_state
                .0
                .presentation
                .details
                .lock()
                .expect("detail presentation lock");
            panic!("poison detail presentation before a command mutation");
        })
        .join();

        assert!(matches!(
            state.set_main_dps_detail_request(
                MainDpsDetailKind::Character,
                MainDpsDetailRequest {
                    character_id: Some(42),
                    ..Default::default()
                },
            ),
            Err(PresentationError::StateUnavailable)
        ));
        assert_eq!(
            state.main_dps_detail_request(MainDpsDetailKind::Character),
            MainDpsDetailRequest::default()
        );
    }

    #[test]
    fn poisoned_detail_cache_is_a_clean_miss_without_revision_effect() {
        let state = AppState::default();
        let initial_stream_revision = state.stream_revision();
        let initial_main_revision = state.0.presentation.main_revision.load(Ordering::Acquire);
        let poison_state = state.clone();
        let _ = std::thread::spawn(move || {
            let _cache = poison_state
                .0
                .presentation
                .cache
                .lock()
                .expect("detail cache lock");
            panic!("poison detail cache");
        })
        .join();

        assert!(
            state
                .main_dps_detail_cache_get(
                    MainDpsStreamRevision {
                        capture: 0,
                        packet: 0,
                        presentation: 0,
                        history: 0,
                        main: 0,
                    },
                    MainDpsDetailKind::Character,
                    &MainDpsDetailRequest::default(),
                    0,
                    1,
                )
                .is_none()
        );
        assert!(!state.0.presentation.cache.is_poisoned());
        assert_eq!(state.stream_revision(), initial_stream_revision);
        assert_eq!(
            state.0.presentation.main_revision.load(Ordering::Acquire),
            initial_main_revision
        );
    }

    #[test]
    fn live_abyss_selection_follows_only_real_half_transitions() {
        assert_eq!(
            next_live_abyss_selection(None, None, Some(AbyssHalf::First)),
            (Some(AbyssHalf::First), Some(AbyssHalf::First))
        );
        assert_eq!(
            next_live_abyss_selection(
                Some(AbyssHalf::First),
                Some(AbyssHalf::First),
                Some(AbyssHalf::Second),
            ),
            (Some(AbyssHalf::Second), Some(AbyssHalf::Second))
        );
        assert_eq!(
            next_live_abyss_selection(
                Some(AbyssHalf::First),
                Some(AbyssHalf::Second),
                Some(AbyssHalf::Second),
            ),
            (Some(AbyssHalf::First), Some(AbyssHalf::Second))
        );
        assert_eq!(
            next_live_abyss_selection(Some(AbyssHalf::Second), Some(AbyssHalf::Second), None,),
            (None, None)
        );
    }

    #[test]
    fn detailed_hud_initial_height_reserves_optional_modules() {
        let config = UiConfig {
            hud: HudConfig::detailed(),
            ..UiConfig::default()
        };
        let state = AppState::new(
            config,
            LiveCaptureService::new(LiveCaptureResources::default()),
        );

        assert_eq!(
            state.hud_initial_height(),
            HUD_BASE_INITIAL_HEIGHT
                + HUD_SUMMARY_HEIGHT
                + HUD_CHARACTERS_HEIGHT
                + HUD_OPTIONAL_TITLE_HEIGHT
                + HUD_OPTIONAL_STATUS_HEIGHT
                + HUD_MINI_TIMELINE_HEIGHT
                + HUD_EDITOR_MODULE_HEADER_HEIGHT * 5
        );
    }

    #[test]
    fn hidden_core_modules_keep_the_full_editor_height() {
        let mut config = UiConfig::default();
        config.hud.set_module_visible(HudModule::Summary, false);
        config.hud.set_module_visible(HudModule::Characters, false);
        let config_path = temporary_config_path("hidden_core_module_height");
        let state = AppState::new_with_config_path(
            config,
            LiveCaptureService::new(LiveCaptureResources::default()),
            config_path.clone(),
        );

        assert_eq!(state.hud_initial_height(), HUD_EDITOR_MIN_HEIGHT);
        fs::remove_dir_all(config_path.parent().expect("config parent"))
            .expect("remove temporary config");
    }

    #[test]
    fn stream_revision_changes_only_when_presentation_state_changes() {
        let state = AppState::default();
        let initial = state.stream_revision();

        state.set_passthrough(false);
        assert!(
            !state
                .set_always_on_top(state.always_on_top())
                .expect("same-value always-on-top")
        );
        assert_eq!(state.stream_revision(), initial);

        state.set_passthrough(true);
        assert_ne!(state.stream_revision(), initial);
    }

    #[test]
    fn settings_revision_tracks_async_update_status_changes() {
        let state = AppState::default();
        let initial = state.settings_revision();

        state.begin_update_check().expect("begin update check");

        assert!(state.settings_revision() > initial);
        assert_eq!(state.settings_snapshot().updates.status, "checking");
    }

    #[test]
    fn update_runtime_projects_available_download_and_prepared_states() {
        let state = AppState::default();
        let available = AvailableComponentUpdate {
            component: UpdateComponent::App,
            release_id: "release".to_owned(),
            version: "0.4.0".parse().expect("semantic version"),
            published_at: "2026-07-31T00:00:00Z".to_owned(),
            notes: "notes".to_owned(),
            artifact_url: "https://example.invalid/app.zip".to_owned(),
            artifact_size: 1_024,
            artifact_sha256: [7; 32],
        };

        state.begin_update_check().expect("begin update check");
        state
            .finish_update_check(vec![available])
            .expect("finish update check");
        let snapshot = state.settings_snapshot();
        assert_eq!(snapshot.updates.status, "available");
        assert_eq!(snapshot.updates.available[0].component, "app");

        state
            .begin_update_download(UpdateComponent::App)
            .expect("begin update download");
        assert!(
            state
                .update_download_progress(UpdateComponent::App, 512, 1_024)
                .expect("update download progress")
        );
        let progress_revision = state.settings_revision();
        assert!(
            !state
                .update_download_progress(UpdateComponent::App, 512, 1_024)
                .expect("repeat update download progress")
        );
        assert_eq!(state.settings_revision(), progress_revision);
        let snapshot = state.settings_snapshot();
        assert_eq!(snapshot.updates.status, "downloading");
        assert_eq!(snapshot.updates.downloaded_bytes, "512");

        state
            .finish_update_download(PreparedUpdate::App {
                version: "0.4.0".parse().expect("semantic version"),
                transaction_path: PathBuf::from("transaction.json"),
                updater_path: PathBuf::from("nte-updater.exe"),
            })
            .expect("finish update download");
        let snapshot = state.settings_snapshot();
        assert_eq!(snapshot.updates.status, "ready");
        assert_eq!(
            snapshot
                .updates
                .prepared
                .as_ref()
                .map(|item| item.component),
            Some("app")
        );
        assert!(snapshot.updates.install_enabled);
    }

    #[test]
    fn poisoned_update_runtime_fails_closed_without_revision_effect() {
        let state = AppState::default();
        let available = AvailableComponentUpdate {
            component: UpdateComponent::App,
            release_id: "release".to_owned(),
            version: "0.4.0".parse().expect("semantic version"),
            published_at: "2026-07-31T00:00:00Z".to_owned(),
            notes: "notes".to_owned(),
            artifact_url: "https://example.invalid/app.zip".to_owned(),
            artifact_size: 1_024,
            artifact_sha256: [7; 32],
        };
        state.begin_update_check().expect("begin update check");
        state
            .finish_update_check(vec![available])
            .expect("finish update check");
        state
            .begin_update_download(UpdateComponent::App)
            .expect("begin update download");
        state
            .finish_update_download(PreparedUpdate::App {
                version: "0.4.0".parse().expect("semantic version"),
                transaction_path: PathBuf::from("private-update-transaction.json"),
                updater_path: PathBuf::from("private-updater.exe"),
            })
            .expect("finish update download");
        let initial_revision = state.settings_revision();
        state.poison_update_runtime_for_test();

        assert_eq!(
            state.begin_update_check(),
            Err(UpdateActionError::RuntimeUnavailable)
        );
        assert_eq!(
            state.begin_update_download(UpdateComponent::App),
            Err(UpdateActionError::RuntimeUnavailable)
        );
        assert!(matches!(
            state.begin_update_install(),
            Err(UpdateActionError::RuntimeUnavailable)
        ));
        assert_eq!(state.settings_revision(), initial_revision);
        let snapshot = state.settings_snapshot();
        assert_eq!(snapshot.updates.status, "unavailable");
        assert_eq!(
            snapshot.updates.message_key,
            "Update operation did not finish."
        );
        assert!(snapshot.updates.available.is_empty());
        assert!(snapshot.updates.prepared.is_none());
        assert!(!snapshot.updates.install_enabled);
        let serialized =
            serde_json::to_string(&snapshot.updates).expect("serialize unavailable update state");
        assert!(!serialized.contains("private-update-transaction"));
        assert!(!serialized.contains("private-updater"));
    }

    #[test]
    fn module_visibility_is_saved_before_the_projection_changes() {
        let config_path = temporary_config_path("module_visibility");
        let state = AppState::new_with_config_path(
            UiConfig::default(),
            LiveCaptureService::new(LiveCaptureResources::default()),
            config_path.clone(),
        );
        let initial_revision = state.stream_revision();

        assert!(
            state
                .set_hud_module_visibility(HudModule::Timeline, true)
                .expect("module visibility save")
        );
        assert!(
            state
                .snapshot()
                .expect("healthy live-capture snapshot")
                .hud
                .config
                .show_mini_timeline
        );
        assert_ne!(state.stream_revision(), initial_revision);

        let saved: UiConfig =
            serde_json::from_str(&fs::read_to_string(&config_path).expect("saved UI config"))
                .expect("valid saved UI config");
        assert!(saved.hud.show_mini_timeline);

        let revision = state.stream_revision();
        assert!(
            !state
                .set_hud_module_visibility(HudModule::Timeline, true)
                .expect("same-value module visibility")
        );
        assert_eq!(state.stream_revision(), revision);

        fs::remove_dir_all(config_path.parent().expect("config parent"))
            .expect("remove temporary config");
    }

    #[test]
    fn settings_hud_option_is_persisted_and_projected() {
        let config_path = temporary_config_path("settings_hud_option");
        let state = AppState::new_with_config_path(
            UiConfig::default(),
            LiveCaptureService::new(LiveCaptureResources::default()),
            config_path.clone(),
        );

        assert!(
            state
                .set_hud_option(HudSettingOption::DamageTaken, true)
                .expect("HUD option save")
        );
        assert!(state.settings_snapshot().hud.show_damage_taken);

        let saved: UiConfig =
            serde_json::from_str(&fs::read_to_string(&config_path).expect("saved UI config"))
                .expect("valid saved UI config");
        assert!(saved.hud.show_damage_taken);

        fs::remove_dir_all(config_path.parent().expect("config parent"))
            .expect("remove temporary config");
    }

    #[test]
    fn config_noop_and_density_revision_effects_are_exact() {
        let config_path = temporary_config_path("settings_density_effects");
        let state = AppState::new_with_config_path(
            UiConfig::default(),
            LiveCaptureService::new(LiveCaptureResources::default()),
            config_path.clone(),
        );
        let initial_settings = state.settings_revision();
        let initial_technical = state.0.presentation.revision.load(Ordering::Acquire);
        let initial_main = state.0.presentation.main_revision.load(Ordering::Acquire);
        let initial_history = state.history_revision();

        assert!(!state.set_density(UiDensity::Cozy).expect("density no-op"));
        assert_eq!(state.settings_revision(), initial_settings);
        assert_eq!(
            state.0.presentation.revision.load(Ordering::Acquire),
            initial_technical
        );
        assert_eq!(
            state.0.presentation.main_revision.load(Ordering::Acquire),
            initial_main
        );
        assert_eq!(state.history_revision(), initial_history);
        assert!(!config_path.exists());

        assert!(
            state
                .set_density(UiDensity::Compact)
                .expect("change density")
        );
        assert_eq!(state.settings_revision(), initial_settings + 1);
        assert_eq!(
            state.0.presentation.revision.load(Ordering::Acquire),
            initial_technical
        );
        assert_eq!(
            state.0.presentation.main_revision.load(Ordering::Acquire),
            initial_main + 1
        );
        assert_eq!(state.history_revision(), initial_history);

        let changed_settings = state.settings_revision();
        let changed_main = state.0.presentation.main_revision.load(Ordering::Acquire);
        assert!(
            !state
                .set_density(UiDensity::Compact)
                .expect("repeat density")
        );
        assert_eq!(state.settings_revision(), changed_settings);
        assert_eq!(
            state.0.presentation.main_revision.load(Ordering::Acquire),
            changed_main
        );

        fs::remove_dir_all(config_path.parent().expect("config parent"))
            .expect("remove temporary config");
    }

    #[test]
    fn settings_snapshot_retries_and_reuses_one_config_generation() {
        let config_path = temporary_config_path("settings_snapshot_generation");
        let state = AppState::new_with_config_path(
            UiConfig::default(),
            LiveCaptureService::new(LiveCaptureResources::default()),
            config_path.clone(),
        );
        let (cloned_tx, cloned_rx) = std::sync::mpsc::channel();
        let (release_tx, release_rx) = std::sync::mpsc::channel();
        let first_clone = Arc::new(AtomicBool::new(true));
        let snapshot_state = state.clone();
        let snapshot_first_clone = Arc::clone(&first_clone);
        let snapshot_thread = std::thread::spawn(move || {
            snapshot_state.settings_snapshot_with_config_hook(|| {
                if snapshot_first_clone.swap(false, Ordering::AcqRel) {
                    cloned_tx.send(()).expect("announce first config clone");
                    release_rx.recv().expect("release first projection");
                }
            })
        });
        cloned_rx.recv().expect("snapshot cloned old config");

        state
            .update_ui_config_with_effects(SettingsMutationEffects::SETTINGS, |config| {
                config.dark_mode = true;
                config.auto_check_updates = false;
            })
            .expect("commit replacement config generation");
        release_tx.send(()).expect("release snapshot retry");
        let snapshot = snapshot_thread.join().expect("join settings snapshot");

        assert!(snapshot.interface.dark_mode);
        assert!(!snapshot.updates.auto_check);
        assert_eq!(
            snapshot.generation,
            state.settings_revision().to_string(),
            "the DTO generation must cover both projections"
        );

        fs::remove_dir_all(config_path.parent().expect("config parent"))
            .expect("remove temporary config");
    }

    #[test]
    fn settings_hud_preset_preserves_width_and_canonical_order() {
        let config_path = temporary_config_path("settings_hud_preset");
        let mut config = UiConfig::default();
        config.hud.width = 620;
        config.hud.module_order = vec![
            HudModule::Timeline,
            HudModule::Characters,
            HudModule::Status,
            HudModule::Summary,
            HudModule::Title,
        ];
        let state = AppState::new_with_config_path(
            config,
            LiveCaptureService::new(LiveCaptureResources::default()),
            config_path.clone(),
        );

        assert!(
            state
                .apply_hud_preset(HudPreset::Detailed)
                .expect("HUD preset save")
        );
        let snapshot = state.settings_snapshot();
        assert_eq!(snapshot.hud.width, 620);
        assert_eq!(
            snapshot.hud.module_order,
            [
                HudModuleSnapshot::Timeline,
                HudModuleSnapshot::Characters,
                HudModuleSnapshot::Status,
                HudModuleSnapshot::Summary,
                HudModuleSnapshot::Title,
            ]
        );
        assert!(snapshot.hud.show_mini_timeline);
        assert!(snapshot.hud.show_damage_taken);

        fs::remove_dir_all(config_path.parent().expect("config parent"))
            .expect("remove temporary config");
    }

    #[test]
    fn non_hud_settings_are_saved_and_projected_from_rust_state() {
        let config_path = temporary_config_path("settings_sections");
        let state = AppState::new_with_config_path(
            UiConfig::default(),
            LiveCaptureService::new(LiveCaptureResources::default()),
            config_path.clone(),
        );

        assert!(
            state
                .update_interface_settings(
                    Language::Japanese,
                    true,
                    ThemePreset::Tactical,
                    AccentColor::Orange,
                    UiDensity::Compact,
                    true,
                    false,
                    48.0,
                )
                .expect("interface settings save")
        );
        assert!(
            state
                .update_capture_settings(
                    "udp port 30196".to_owned(),
                    Some("capture-device".to_owned()),
                    true,
                    true,
                    true,
                    true,
                    45,
                    DpsTimeMode::RealTime,
                    HotkeyBinding::new(false, false, false, config::HotkeyKey::Insert),
                )
                .expect("capture settings save")
        );
        let mut hotkeys = state.global_hotkeys();
        hotkeys.set_binding(
            nte_dps_tool::storage::config::GlobalHotkeyAction::ToggleCapture,
            Some(nte_dps_tool::storage::config::HotkeyBinding::new(
                true,
                false,
                true,
                nte_dps_tool::storage::config::HotkeyKey::F8,
            )),
        );
        state
            .update_global_hotkeys(hotkeys)
            .expect("global hotkeys save");
        let settings_revision = state.settings_revision();
        let main_revision = state.0.presentation.main_revision.load(Ordering::Acquire);
        assert!(
            state
                .update_main_dps_display(MainDpsDisplayConfig {
                    metrics: vec![
                        config::MainDpsMetric::TeamDps,
                        config::MainDpsMetric::Duration,
                    ],
                    attributions: vec![
                        config::MainDpsAttribution::Character,
                        config::MainDpsAttribution::MaxHpReduction,
                    ],
                })
                .expect("main DPS display settings save")
        );
        assert_eq!(state.settings_revision(), settings_revision + 1);
        assert_eq!(
            state.0.presentation.main_revision.load(Ordering::Acquire),
            main_revision + 1
        );

        let snapshot = state.settings_snapshot();
        assert_eq!(snapshot.interface.language, "ja");
        assert!(snapshot.interface.dark_mode);
        assert_eq!(snapshot.interface.theme_preset, "tactical");
        assert_eq!(snapshot.capture.bpf_filter, "udp port 30196");
        assert_eq!(
            snapshot.capture.manual_capture_device.as_deref(),
            Some("capture-device")
        );
        assert!(snapshot.capture.include_max_hp_reduction_in_total_damage);
        assert!(snapshot.capture.separate_reaction_damage);
        assert_eq!(snapshot.capture.auto_round_idle_seconds, 45);
        assert_eq!(
            state.0.live_capture.history_archive_policy(),
            HistoryArchivePolicy {
                requested_dps_time_mode: DpsTimeBasis::WallClock,
                separate_reaction_damage: true,
            },
            "successful settings persistence must update the capture-side round policy",
        );
        assert_eq!(
            snapshot.hotkeys.bindings[0]
                .binding
                .as_ref()
                .map(|binding| binding.key.as_str()),
            Some("F8")
        );
        assert_eq!(snapshot.main_dps.metrics, ["team-dps", "duration"]);
        assert_eq!(
            snapshot.main_dps.attributions,
            ["character", "max-hp-reduction"]
        );

        let saved: UiConfig =
            serde_json::from_str(&fs::read_to_string(&config_path).expect("saved UI config"))
                .expect("valid saved UI config");
        assert_eq!(saved.language, Language::Japanese);
        assert_eq!(saved.theme_preset, ThemePreset::Tactical);
        assert_eq!(saved.accent, AccentColor::Orange);
        assert_eq!(saved.capture_filter, "udp port 30196");
        assert!(saved.reduce_motion);
        assert!(saved.include_max_hp_reduction_in_total_damage);
        assert_eq!(
            saved.manual_capture_device.as_deref(),
            Some("capture-device")
        );
        assert_eq!(saved.dps_time_mode, DpsTimeMode::RealTime);
        assert_eq!(
            saved.main_dps_display.metrics,
            [
                config::MainDpsMetric::TeamDps,
                config::MainDpsMetric::Duration,
            ]
        );
        assert_eq!(
            saved.main_dps_display.attributions,
            [
                config::MainDpsAttribution::Character,
                config::MainDpsAttribution::MaxHpReduction,
            ]
        );

        let restored = AppState::new_with_config_path(
            saved,
            LiveCaptureService::new(LiveCaptureResources::default()),
            config_path.clone(),
        );
        assert_eq!(
            restored.settings_snapshot().capture.bpf_filter,
            "udp port 30196"
        );
        assert_eq!(
            restored.settings_snapshot().main_dps.metrics,
            ["team-dps", "duration"]
        );
        assert_eq!(
            restored.0.live_capture.history_archive_policy(),
            HistoryArchivePolicy {
                requested_dps_time_mode: DpsTimeBasis::WallClock,
                separate_reaction_damage: true,
            },
            "startup must seed the capture-side round policy from sanitized config",
        );

        fs::remove_dir_all(config_path.parent().expect("config parent"))
            .expect("remove temporary config");
    }

    #[test]
    fn mod_studio_game_directory_survives_state_reload() {
        let config_path = temporary_config_path("mod_studio_game_directory");
        let state = AppState::new_with_config_path(
            UiConfig::default(),
            LiveCaptureService::new(LiveCaptureResources::default()),
            config_path.clone(),
        );

        assert!(
            state
                .set_mod_studio_game_directory(
                    ModsPluginGameRegion::China,
                    Some("D:\\CustomGame".to_owned()),
                )
                .expect("save Mod Studio game directory")
        );
        assert_eq!(
            state.mod_studio_game_directory(ModsPluginGameRegion::China),
            Some("D:\\CustomGame".to_owned())
        );

        let saved: UiConfig =
            serde_json::from_str(&fs::read_to_string(&config_path).expect("saved UI config"))
                .expect("valid saved UI config");
        let restored = AppState::new_with_config_path(
            saved,
            LiveCaptureService::new(LiveCaptureResources::default()),
            config_path.clone(),
        );
        assert_eq!(
            restored.mod_studio_game_directory(ModsPluginGameRegion::China),
            Some("D:\\CustomGame".to_owned())
        );

        restored
            .set_mod_studio_game_directory(ModsPluginGameRegion::China, None)
            .expect("clear Mod Studio game directory");
        assert_eq!(
            restored.mod_studio_game_directory(ModsPluginGameRegion::China),
            None
        );

        fs::remove_dir_all(config_path.parent().expect("config parent"))
            .expect("remove temporary config");
    }

    #[test]
    fn mod_studio_loading_method_survives_state_reload() {
        let config_path = temporary_config_path("mod_studio_loading_method");
        let state = AppState::new_with_config_path(
            UiConfig::default(),
            LiveCaptureService::new(LiveCaptureResources::default()),
            config_path.clone(),
        );

        assert_eq!(
            state.mod_studio_loading_method(),
            ModStudioLoadingMethod::Proxy
        );
        assert!(!state.mod_studio_risk_acknowledged());
        assert!(
            state
                .set_mod_studio_loading_method(ModStudioLoadingMethod::Loader)
                .expect("save Mod Studio loading method")
        );
        assert!(
            state
                .acknowledge_mod_studio_risk()
                .expect("save Mod Studio risk acknowledgement")
        );

        let saved: UiConfig =
            serde_json::from_str(&fs::read_to_string(&config_path).expect("saved UI config"))
                .expect("valid saved UI config");
        let restored = AppState::new_with_config_path(
            saved,
            LiveCaptureService::new(LiveCaptureResources::default()),
            config_path.clone(),
        );
        assert_eq!(
            restored.mod_studio_loading_method(),
            ModStudioLoadingMethod::Loader
        );
        assert!(restored.mod_studio_risk_acknowledged());

        fs::remove_dir_all(config_path.parent().expect("config parent"))
            .expect("remove temporary config");
    }

    #[test]
    fn module_order_is_saved_before_the_projection_changes() {
        let config_path = temporary_config_path("module_order");
        let state = AppState::new_with_config_path(
            UiConfig::default(),
            LiveCaptureService::new(LiveCaptureResources::default()),
            config_path.clone(),
        );
        let initial_revision = state.stream_revision();

        assert!(
            state
                .move_hud_module(HudModule::Title, HudModule::Characters, true)
                .expect("module order save")
        );
        assert_ne!(state.stream_revision(), initial_revision);
        assert_eq!(
            state
                .snapshot()
                .expect("healthy live-capture snapshot")
                .hud
                .config
                .module_order,
            [
                HudModuleSnapshot::Summary,
                HudModuleSnapshot::Status,
                HudModuleSnapshot::Characters,
                HudModuleSnapshot::Title,
                HudModuleSnapshot::Timeline,
            ]
        );

        let saved: UiConfig =
            serde_json::from_str(&fs::read_to_string(&config_path).expect("saved UI config"))
                .expect("valid saved UI config");
        assert_eq!(
            saved.hud.module_order,
            [
                HudModule::Summary,
                HudModule::Status,
                HudModule::Characters,
                HudModule::Title,
                HudModule::Timeline,
            ]
        );

        let revision = state.stream_revision();
        assert!(
            !state
                .move_hud_module(HudModule::Title, HudModule::Characters, true)
                .expect("same module order")
        );
        assert_eq!(state.stream_revision(), revision);

        fs::remove_dir_all(config_path.parent().expect("config parent"))
            .expect("remove temporary config");
    }

    #[test]
    fn hud_width_is_sanitized_and_saved_before_projection_changes() {
        let config_path = temporary_config_path("hud_width");
        let state = AppState::new_with_config_path(
            UiConfig::default(),
            LiveCaptureService::new(LiveCaptureResources::default()),
            config_path.clone(),
        );
        let initial_revision = state.stream_revision();

        assert!(state.set_hud_width(u16::MAX).expect("HUD width save"));
        assert_eq!(
            state
                .snapshot()
                .expect("healthy live-capture snapshot")
                .hud
                .config
                .width,
            nte_dps_tool::storage::config::HUD_WIDTH_MAX
        );
        assert_ne!(state.stream_revision(), initial_revision);

        let saved: UiConfig =
            serde_json::from_str(&fs::read_to_string(&config_path).expect("saved UI config"))
                .expect("valid saved UI config");
        assert_eq!(
            saved.hud.width,
            nte_dps_tool::storage::config::HUD_WIDTH_MAX
        );

        fs::remove_dir_all(config_path.parent().expect("config parent"))
            .expect("remove temporary config");
    }

    #[test]
    fn always_on_top_is_saved_before_projection_changes() {
        let config_path = temporary_config_path("always_on_top");
        let state = AppState::new_with_config_path(
            UiConfig::default(),
            LiveCaptureService::new(LiveCaptureResources::default()),
            config_path.clone(),
        );
        let initial_revision = state.stream_revision();

        assert!(state.set_always_on_top(false).expect("always-on-top save"));
        assert!(!state.always_on_top());
        assert_ne!(state.stream_revision(), initial_revision);

        let saved: UiConfig =
            serde_json::from_str(&fs::read_to_string(&config_path).expect("saved UI config"))
                .expect("valid saved UI config");
        assert_eq!(saved.hud_always_on_top, Some(false));
        assert!(!saved.always_on_top);
        assert_eq!(saved.main_dps_always_on_top, Some(true));

        let revision = state.stream_revision();
        assert!(
            !state
                .set_always_on_top(false)
                .expect("same-value always-on-top")
        );
        assert_eq!(state.stream_revision(), revision);

        fs::remove_dir_all(config_path.parent().expect("config parent"))
            .expect("remove temporary config");
    }

    #[test]
    fn desktop_window_always_on_top_preferences_are_independent() {
        let config_path = temporary_config_path("independent_always_on_top");
        let state = AppState::new_with_config_path(
            UiConfig::default(),
            LiveCaptureService::new(LiveCaptureResources::default()),
            config_path.clone(),
        );

        assert!(
            state
                .set_window_always_on_top(DesktopWindowKind::MainDps, false)
                .expect("main DPS always-on-top save")
        );
        assert!(
            state
                .set_window_always_on_top(DesktopWindowKind::Console, true)
                .expect("Console always-on-top save")
        );

        assert!(!state.window_always_on_top(DesktopWindowKind::MainDps));
        assert!(state.window_always_on_top(DesktopWindowKind::Hud));
        assert!(state.window_always_on_top(DesktopWindowKind::Console));
        assert!(!state.window_always_on_top(DesktopWindowKind::AbyssValues));

        let saved: UiConfig =
            serde_json::from_str(&fs::read_to_string(&config_path).expect("saved UI config"))
                .expect("valid saved UI config");
        assert_eq!(saved.main_dps_always_on_top, Some(false));
        assert_eq!(saved.hud_always_on_top, Some(true));
        assert_eq!(saved.console_always_on_top, Some(true));
        assert_eq!(saved.abyss_values_always_on_top, Some(false));

        fs::remove_dir_all(config_path.parent().expect("config parent"))
            .expect("remove temporary config");
    }

    #[test]
    fn hud_window_position_is_saved_without_changing_projection_revision() {
        let config_path = temporary_config_path("hud_window_position");
        let state = AppState::new_with_config_path(
            UiConfig::default(),
            LiveCaptureService::new(LiveCaptureResources::default()),
            config_path.clone(),
        );
        let initial_revision = state.stream_revision();

        assert!(
            state
                .set_hud_window_position([-1920, 84])
                .expect("HUD position save")
        );
        assert_eq!(state.hud_window_position(), Some([-1920, 84]));
        assert_eq!(state.stream_revision(), initial_revision);

        let saved: UiConfig =
            serde_json::from_str(&fs::read_to_string(&config_path).expect("saved UI config"))
                .expect("valid saved UI config");
        assert_eq!(saved.hud_window_position, Some([-1920, 84]));
        assert!(
            !state
                .set_hud_window_position([-1920, 84])
                .expect("same-value HUD position")
        );

        fs::remove_dir_all(config_path.parent().expect("config parent"))
            .expect("remove temporary config");
    }

    #[test]
    fn console_geometry_is_saved_without_publishing_combat_or_settings_state() {
        let config_path = temporary_config_path("console_geometry");
        let state = AppState::new_with_config_path(
            UiConfig::default(),
            LiveCaptureService::new(LiveCaptureResources::default()),
            config_path.clone(),
        );
        let initial_stream_revision = state.stream_revision();
        let initial_settings_revision = state.settings_revision();

        assert!(
            state
                .set_console_window_geometry([1180.0, 760.0], [-1920.0, 84.0])
                .expect("Console geometry save")
        );
        assert_eq!(
            state.console_window_geometry(),
            (Some([1180.0, 760.0]), Some([-1920.0, 84.0]))
        );
        assert_eq!(state.stream_revision(), initial_stream_revision);
        assert_eq!(state.settings_revision(), initial_settings_revision);

        let saved: UiConfig =
            serde_json::from_str(&fs::read_to_string(&config_path).expect("saved UI config"))
                .expect("valid saved UI config");
        assert_eq!(saved.console_window_size, Some([1180.0, 760.0]));
        assert_eq!(saved.console_window_position, Some([-1920.0, 84.0]));
        assert!(
            !state
                .set_console_window_geometry([1180.0, 760.0], [-1920.0, 84.0])
                .expect("same-value Console geometry")
        );

        fs::remove_dir_all(config_path.parent().expect("config parent"))
            .expect("remove temporary config");
    }

    #[test]
    fn combat_detail_columns_and_positions_persist_in_the_existing_ui_config() {
        let config_path = temporary_config_path("combat_detail_preferences");
        let state = AppState::new_with_config_path(
            UiConfig::default(),
            LiveCaptureService::new(LiveCaptureResources::default()),
            config_path.clone(),
        );
        let columns = nte_dps_tool::storage::config::HitDetailColumnsConfig {
            show_time: false,
            type_width: u16::MAX,
            ..Default::default()
        };
        assert!(
            state
                .set_hit_detail_columns(columns)
                .expect("detail columns save")
        );
        assert!(!state.ui_config_snapshot().hit_detail_columns.show_time);
        assert_eq!(
            state.ui_config_snapshot().hit_detail_columns.type_width,
            600
        );

        assert!(
            state
                .set_main_dps_detail_window_geometry(
                    [920.0, 640.0],
                    [120.0, 80.0],
                    MainDpsDetailKind::Team,
                )
                .expect("team detail geometry save")
        );
        assert!(
            state
                .set_main_dps_detail_window_geometry(
                    [840.0, 600.0],
                    [240.0, 160.0],
                    MainDpsDetailKind::Character,
                )
                .expect("character detail geometry save")
        );
        assert_eq!(
            state.main_dps_detail_window_geometry(MainDpsDetailKind::Character),
            (Some([840.0, 600.0]), Some([240.0, 160.0]))
        );

        let saved: UiConfig =
            serde_json::from_str(&fs::read_to_string(&config_path).expect("saved UI config"))
                .expect("valid saved UI config");
        assert_eq!(saved.team_hit_detail_window_position, Some([120.0, 80.0]));
        assert_eq!(saved.hit_detail_window_position, Some([240.0, 160.0]));
        assert_eq!(saved.team_hit_detail_window_size, Some([920.0, 640.0]));
        assert_eq!(saved.hit_detail_window_size, Some([840.0, 600.0]));

        fs::remove_dir_all(config_path.parent().expect("config parent"))
            .expect("remove temporary config");
    }

    #[test]
    fn failed_module_visibility_save_keeps_the_previous_projection() {
        let config_path = temporary_config_path("module_visibility_failure");
        fs::create_dir(&config_path).expect("directory blocks config file replacement");
        let state = AppState::new_with_config_path(
            UiConfig::default(),
            LiveCaptureService::new(LiveCaptureResources::default()),
            config_path.clone(),
        );
        let initial_revision = state.stream_revision();

        assert!(
            state
                .set_hud_module_visibility(HudModule::Title, true)
                .is_err()
        );
        assert!(
            !state
                .snapshot()
                .expect("healthy live-capture snapshot")
                .hud
                .config
                .show_title
        );
        assert_eq!(state.stream_revision(), initial_revision);

        fs::remove_dir_all(config_path.parent().expect("config parent"))
            .expect("remove temporary config");
    }

    #[test]
    fn failed_always_on_top_save_keeps_the_previous_projection() {
        let config_path = temporary_config_path("always_on_top_failure");
        fs::create_dir(&config_path).expect("directory blocks config file replacement");
        let state = AppState::new_with_config_path(
            UiConfig::default(),
            LiveCaptureService::new(LiveCaptureResources::default()),
            config_path.clone(),
        );
        let initial_revision = state.stream_revision();

        assert!(state.set_always_on_top(false).is_err());
        assert!(state.always_on_top());
        assert_eq!(state.stream_revision(), initial_revision);

        fs::remove_dir_all(config_path.parent().expect("config parent"))
            .expect("remove temporary config");
    }

    #[test]
    fn failed_hud_window_position_save_keeps_the_previous_position() {
        let config_path = temporary_config_path("hud_window_position_failure");
        fs::create_dir(&config_path).expect("directory blocks config file replacement");
        let state = AppState::new_with_config_path(
            UiConfig::default(),
            LiveCaptureService::new(LiveCaptureResources::default()),
            config_path.clone(),
        );
        let initial_revision = state.stream_revision();

        assert!(state.set_hud_window_position([120, 80]).is_err());
        assert_eq!(state.hud_window_position(), None);
        assert_eq!(state.stream_revision(), initial_revision);

        fs::remove_dir_all(config_path.parent().expect("config parent"))
            .expect("remove temporary config");
    }

    #[test]
    fn deleted_history_undo_token_is_opaque_and_single_use() {
        let state = AppState::default();
        let (directory, tombstone) = temporary_history_tombstone("opaque-single-use");
        let record_id = tombstone.record_id().to_owned();

        let token = state
            .remember_deleted_history_tombstone(tombstone)
            .expect("remember deleted History record");

        assert!(!token.contains(&record_id));
        assert_eq!(token.len(), 37);
        assert_eq!(
            state
                .peek_deleted_history_tombstone(&token)
                .expect("read deleted History record")
                .expect("active undo record")
                .record_id(),
            record_id
        );
        assert!(
            state
                .consume_deleted_history(&token)
                .expect("consume deleted History record")
        );
        assert!(
            state
                .peek_deleted_history_tombstone(&token)
                .expect("read consumed History undo")
                .is_none()
        );
        let _ = fs::remove_dir_all(directory);
    }

    #[test]
    fn replacing_and_expiring_history_undo_discards_old_tombstones_outside_the_slot() {
        let state = AppState::default();
        let (first_directory, first) = temporary_history_tombstone("replace-first");
        let (second_directory, second) = temporary_history_tombstone("replace-second");
        state
            .remember_deleted_history_tombstone(first)
            .expect("remember first History tombstone");
        let second_token = state
            .remember_deleted_history_tombstone(second)
            .expect("replace History tombstone");
        assert!(
            !first_directory.join(".nte-history-undo").exists(),
            "replacement must discard the previous on-disk tombstone"
        );
        {
            let mut undo = state.0.history.undo.lock().expect("History undo slot");
            undo.as_mut().expect("active second tombstone").expires_at =
                Instant::now() - Duration::from_millis(1);
        }
        assert!(
            state
                .peek_deleted_history_tombstone(&second_token)
                .expect("expire History tombstone")
                .is_none()
        );
        assert!(
            !second_directory.join(".nte-history-undo").exists(),
            "expiry must discard the on-disk tombstone"
        );
        let _ = fs::remove_dir_all(first_directory);
        let _ = fs::remove_dir_all(second_directory);
    }

    #[test]
    fn stale_island_dismiss_keeps_the_newer_notice() {
        let state = AppState::default();
        let stale_id = state.publish_island_notice("info", "First notice", Vec::new(), None);
        let current_id = state.publish_island_notice("success", "Second notice", Vec::new(), None);
        let published_revision = state.0.island_notice.revision();

        assert!(!state.dismiss_island_notice(&stale_id));
        assert_eq!(state.0.island_notice.revision(), published_revision);
        assert_eq!(
            state.island_notice().map(|notice| notice.id),
            Some(current_id.clone())
        );
        assert_eq!(state.0.island_notice.revision(), published_revision);
        assert!(state.dismiss_island_notice(&current_id));
        assert_eq!(state.0.island_notice.revision(), published_revision + 1);
        assert!(state.island_notice().is_none());
        assert_eq!(state.0.island_notice.revision(), published_revision + 1);
    }

    #[test]
    fn poisoned_island_notice_is_discarded_once_without_projecting_partial_data() {
        let state = AppState::default();
        state.publish_island_notice("info", "Healthy notice", Vec::new(), None);
        let initial_revision = state.0.island_notice.revision();
        let poison_state = state.clone();
        let _ = std::thread::spawn(move || {
            let mut notice = poison_state
                .0
                .island_notice
                .notice
                .lock()
                .expect("island notice lock");
            notice.as_mut().expect("published notice").message_arguments =
                vec!["private-partial-notice".to_owned()];
            panic!("poison island notice");
        })
        .join();

        assert!(state.island_notice().is_none());
        assert!(!state.0.island_notice.notice.is_poisoned());
        assert_eq!(state.0.island_notice.revision(), initial_revision + 1);
        let serialized =
            serde_json::to_string(&crate::contract::island::IslandSnapshot::from_state(&state))
                .expect("serialize recovered island snapshot");
        assert!(!serialized.contains("private-partial-notice"));
        assert!(!serialized.contains("poison"));
        assert!(state.island_notice().is_none());
        assert_eq!(
            state.0.island_notice.revision(),
            initial_revision + 1,
            "repeated empty reads must not spam the revision"
        );
    }

    #[test]
    fn poisoned_island_dismiss_reports_the_visible_clear_once() {
        let state = AppState::default();
        state.publish_island_notice("info", "Healthy notice", Vec::new(), None);
        let initial_revision = state.0.island_notice.revision();
        let poison_state = state.clone();
        let _ = std::thread::spawn(move || {
            let _notice = poison_state
                .0
                .island_notice
                .notice
                .lock()
                .expect("island notice lock");
            panic!("poison island notice before dismiss");
        })
        .join();

        assert!(state.dismiss_island_notice("untrusted-notice-id"));
        assert_eq!(state.0.island_notice.revision(), initial_revision + 1);
        assert!(!state.dismiss_island_notice("untrusted-notice-id"));
        assert_eq!(state.0.island_notice.revision(), initial_revision + 1);
        assert!(state.island_notice().is_none());
    }

    #[test]
    fn island_notice_revision_tracks_only_publish_dismiss_and_expiry_changes() {
        let state = AppState::default();
        let initial_revision = state.0.island_notice.revision();
        let notice_id = state.publish_island_notice("info", "Expiring notice", Vec::new(), None);
        assert_eq!(state.0.island_notice.revision(), initial_revision + 1);

        assert!(!state.dismiss_island_notice("stale-notice"));
        assert_eq!(state.0.island_notice.revision(), initial_revision + 1);
        {
            let mut notice = state
                .0
                .island_notice
                .notice
                .lock()
                .expect("island notice lock");
            notice.as_mut().expect("published notice").expires_at =
                Instant::now() - Duration::from_millis(1);
        }
        assert!(state.island_notice().is_none());
        assert_eq!(state.0.island_notice.revision(), initial_revision + 2);
        assert!(state.island_notice().is_none());
        assert!(!state.dismiss_island_notice(&notice_id));
        assert_eq!(state.0.island_notice.revision(), initial_revision + 2);
    }
}
