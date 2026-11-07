mod combat;
mod diagnostics;
mod equipment;
mod history;
mod session;
mod settings;
mod teams;
#[cfg(test)]
mod tests;

use crate::contract::island::IslandTone;
use nte_dps_tool::storage::config::ModStudioLoadingMethod;
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
        capture::enumerate_devices,
        character_data::CharacterDataProjection,
        combat_details::CombatDetailFilter,
        encrypted_ini::{EncryptedIniKey, EncryptedIniSaveOutcome},
        history::{HistoryArchivePolicy, PreparedHistoryArchive},
        hud::{HudProjectionOptions, HudSnapshot, project_hud},
        live_capture::{
            CaptureReplayKind, LiveCapturePhase, LiveCaptureResources, LiveCaptureService,
        },
        packets::PacketStreamRevision,
        skills::{SkillsProjectionOptions, project_skills},
        sync::lock_or_repair,
        timeline::{TimelineProjection, TimelineScope},
        update::{AvailableComponentUpdate, UpdateComponent},
    },
    engine::{
        capture::{CaptureImportError, PacketEmissionMode, prepare_capture_json_replay},
        model::{
            AbyssHalf, CaptureQualitySource, CombatClockRuntimeHealth, CombatState,
            DamageAttributionSummary, DpsTimeBasis, EmptyCurtainCharacter, EmptyCurtainItem, Hit,
            TeamDps, TeamDpsExport,
        },
        parser::{
            CHARACTER_DATA_PATH, EQUIPMENT_CATALOG_PATH, EquipmentCatalog, load_equipment_catalog,
        },
    },
    storage::{
        config::{
            self, AccentColor, DpsTimeMode, HotkeyBinding, HudConfig, HudModule,
            MainDpsDisplayConfig, ThemePreset, UiConfig, UiDensity,
            sanitize_timeline_bucket_seconds,
        },
        history::{
            BorrowedHistorySaveOutcome, HistoryCombatDetails, HistoryDeleteTombstone,
            HistoryIndexRecord, discard_tombstoned_record,
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

use crate::{
    character_data_service::{CharacterDataService, CharacterDataServiceError},
    contract::{
        HudWindowSnapshot, TechnicalSnapshot,
        main_dps_detail::MainDpsDetailSnapshot,
        settings::{CaptureDeviceSnapshot, SettingsSnapshot},
        timeline::MAX_TIMELINE_BUCKETS,
    },
    desktop_runtime::DesktopRuntime,
    diagnostics_runtime::DiagnosticsRuntime,
    encrypted_ini_service::{
        EncryptedIniProjection, EncryptedIniService, EncryptedIniServiceError,
    },
    equipment_operation_service::{EquipmentOperationError, EquipmentOperationService},
    settings_service::{SettingsService, SettingsServiceError},
    team_import_service::{TeamImportError, TeamImportService},
    update_runtime::UpdateRuntimeService,
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

pub(crate) struct PluginControlReservation(AppState);
impl Drop for PluginControlReservation {
    fn drop(&mut self) {
        if self
            .0
            .0
            .replay_import
            .release(ReplayImportReservationState::PluginControl)
            .is_err()
        {
            log::error!("plugin control reservation release failed");
        }
    }
}

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

#[derive(Default)]
struct StreamDeliveryProgress {
    sent: AtomicU64,
    acknowledged: AtomicU64,
}

struct StreamEntry {
    generation: u64,
    stop: Arc<AtomicBool>,
    delivery: Arc<StreamDeliveryProgress>,
}

struct PendingStreamEntry {
    stop: Arc<AtomicBool>,
    delivery: Arc<StreamDeliveryProgress>,
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
    delivery: Arc<StreamDeliveryProgress>,
}

impl StreamRegistration {
    pub(crate) fn delivery_ready(&self) -> bool {
        !self.stop.load(Ordering::Acquire)
            && self.delivery.sent.load(Ordering::Acquire)
                == self.delivery.acknowledged.load(Ordering::Acquire)
    }

    // One in-flight delivery, counted from 1 on each ordered Tauri Channel.
    pub(crate) fn begin_delivery(&self) -> bool {
        if self.stop.load(Ordering::Acquire) {
            return false;
        }
        let acknowledged = self.delivery.acknowledged.load(Ordering::Acquire);
        let Some(next) = acknowledged.checked_add(1) else {
            return false;
        };
        self.delivery
            .sent
            .compare_exchange(acknowledged, next, Ordering::AcqRel, Ordering::Acquire)
            .is_ok()
    }

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
        let delivery = Arc::new(StreamDeliveryProgress::default());
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
                delivery: Arc::clone(&delivery),
            },
        );
        drop(entries);

        Ok(StreamRegistration {
            identity,
            generation,
            stop,
            active: Arc::new(AtomicBool::new(false)),
            delivery,
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
                            delivery: pending.delivery,
                        }
                    }));
                    if let Some(previous) = slot.current.replace(StreamEntry {
                        generation: registration.generation,
                        stop: Arc::clone(&registration.stop),
                        delivery: Arc::clone(&registration.delivery),
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

    fn acknowledge(
        &self,
        owner_window: &str,
        subscription_key: &str,
        generation: u64,
        sequence: u64,
    ) -> Result<bool, StreamRegistryError> {
        if !valid_stream_identity(owner_window, subscription_key)
            || generation == 0
            || sequence == 0
        {
            return Err(StreamRegistryError::InvalidIdentity);
        }
        let entries = self.lock_entries()?;
        let current = entries
            .get(&StreamIdentity {
                owner_window: owner_window.to_owned(),
                subscription_key: subscription_key.to_owned(),
            })
            .and_then(|slot| slot.current.as_ref());
        let Some(current) = current
            .filter(|entry| entry.generation == generation && !entry.stop.load(Ordering::Acquire))
        else {
            return Ok(false);
        };
        if current.delivery.sent.load(Ordering::Acquire) != sequence {
            return Ok(false);
        }
        Ok(current
            .delivery
            .acknowledged
            .compare_exchange(sequence - 1, sequence, Ordering::AcqRel, Ordering::Acquire)
            .is_ok())
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
        let mut removed = HashMap::new();
        let (entries, recovered) =
            lock_or_repair(&self.entries, |entries| removed = std::mem::take(entries));
        if !recovered {
            return Ok(entries);
        }
        drop(entries);
        for slot in removed.into_values() {
            cancel_stream_slot(slot);
        }
        Err(StreamRegistryError::RuntimeUnavailable)
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
            delivery: pending.delivery,
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
    pub(crate) tone: IslandTone,
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
        tone: IslandTone,
        message_key: &'static str,
        message_arguments: Vec<String>,
        undo_token: Option<String>,
    ) -> String {
        // Ephemeral notification state is safe to reset after an interrupted
        // mutation. The replacement is the sole externally visible change.
        let (mut notice, _) = self.lock_discarding_poison();
        if let Some(current) = notice.as_ref()
            && Instant::now() < current.expires_at
            && current.tone == tone
            && current.message_key == message_key
            && current.message_arguments == message_arguments
            && current.undo_token == undo_token
        {
            return current.id.clone();
        }
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
        let mut discarded_notice = false;
        let (notice, _) = lock_or_repair(&self.notice, |notice| {
            discarded_notice = notice.take().is_some();
        });
        (notice, discarded_notice)
    }

    fn bump_revision(&self) -> u64 {
        self.revision.fetch_add(1, Ordering::AcqRel) + 1
    }
}

#[derive(Clone)]
struct PausedPresentation {
    dps_time_mode: DpsTimeMode,
    state: Arc<CombatState>,
    packet_revision: PacketStreamRevision,
}

#[derive(Clone)]
struct SelectedRoundPresentation {
    dps_time_mode: DpsTimeMode,
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
}

/// Presentation state and caches are rebuildable projections, so poison
/// resets them to their default.
fn lock_or_reset<'a, T: Default>(mutex: &'a Mutex<T>, warning: &str) -> (MutexGuard<'a, T>, bool) {
    let (guard, recovered) = lock_or_repair(mutex, |value| *value = T::default());
    if recovered {
        log::warn!("{warning}");
    }
    (guard, recovered)
}

impl PresentationState {
    fn lock_mode(&self) -> (MutexGuard<'_, PresentationModeState>, bool) {
        lock_or_reset(
            &self.mode,
            "Presentation mode was reset after an interrupted update",
        )
    }

    fn lock_abyss(&self) -> (MutexGuard<'_, AbyssPresentationState>, bool) {
        lock_or_reset(
            &self.abyss,
            "Presentation abyss selection was reset after an interrupted update",
        )
    }

    fn lock_details(&self) -> (MutexGuard<'_, MainDpsDetailsState>, bool) {
        lock_or_reset(
            &self.details,
            "Presentation detail requests were reset after an interrupted update",
        )
    }

    fn lock_cache(&self) -> (MutexGuard<'_, Vec<MainDpsDetailCache>>, bool) {
        lock_or_reset(
            &self.cache,
            "Presentation detail cache was discarded after an interrupted update",
        )
    }

    fn lock_timeline_cache(&self) -> (MutexGuard<'_, Vec<TimelineProjectionCache>>, bool) {
        lock_or_reset(
            &self.timeline_cache,
            "Timeline projection cache was discarded after an interrupted update",
        )
    }

    fn lock_main_readout_cache(&self) -> (MutexGuard<'_, Vec<MainReadoutCache>>, bool) {
        lock_or_reset(
            &self.main_readout_cache,
            "Main readout cache was discarded after an interrupted update",
        )
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
        requested_dps_time_mode: match config.dps_time_mode() {
            DpsTimeMode::TimeStopAdjusted => DpsTimeBasis::SubtractTimeStop,
            DpsTimeMode::RealTime => DpsTimeBasis::WallClock,
        },
        separate_reaction_damage: config.separate_reaction_damage,
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Deserialize, ts_rs::TS)]
#[ts(export)]
#[serde(rename_all = "snake_case")]
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

#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Deserialize, ts_rs::TS)]
#[ts(export)]
#[serde(rename_all = "snake_case")]
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
    PluginControl,
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
        let dps_time_mode = self.main_presented_dps_time_mode()?;
        let hud_config = config.hud.clone();
        let selected_abyss_half = self.abyss_presentation_snapshot().selected;
        let supported_locales = Language::all()
            .iter()
            .map(|language| language.code())
            .collect();

        Ok(TechnicalSnapshot {
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
                            subtract_time_stop_for_state(dps_time_mode, state),
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
        tone: IslandTone,
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

    pub(crate) fn character_data_snapshot(
        &self,
    ) -> Result<CharacterDataProjection, CharacterDataServiceError> {
        self.0.character_data.snapshot()
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

    pub(crate) fn uptime_ms(&self) -> u128 {
        self.0.desktop.uptime_ms()
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

    pub(crate) fn acknowledge_stream(
        &self,
        owner_window: &str,
        subscription_key: &str,
        generation: u64,
        sequence: u64,
    ) -> Result<bool, StreamRegistryError> {
        self.0
            .streams
            .acknowledge(owner_window, subscription_key, generation, sequence)
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
}
