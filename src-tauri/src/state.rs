use std::{
    collections::{HashMap, HashSet, VecDeque},
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
        character_data::{
            CharacterDataError, CharacterDataProjection, CharacterDataRecordInput,
            load_character_data, save_character_data_record,
        },
        combat_details::CombatDetailFilter,
        diagnostics::{DiagnosticRun, DiagnosticSnapshot},
        encrypted_ini::{
            EncryptedIniDocument, EncryptedIniError, EncryptedIniKey, EncryptedIniSaveOutcome,
            load_encrypted_ini_document, save_encrypted_ini_document,
        },
        history::{
            PendingHistoryArchive, PreparedHistoryArchive, auto_round_due, prepare_history_archive,
        },
        hud::{HudProjectionOptions, HudSnapshot, project_hud},
        live_capture::{
            CaptureReplayKind, LiveCapturePhase, LiveCaptureResources, LiveCaptureService,
            LiveCaptureStatus,
        },
        mod_studio::{
            ModStudioError, ModStudioRuntimeSnapshot, ModStudioWorkspaceService,
            poll_mod_studio_runtime,
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
            CaptureExportDocument, CaptureExportNetwork, CaptureExportOptions, PacketEmissionMode,
        },
        model::{
            AbyssHalf, CaptureQualitySource, CaptureQualitySummary, CombatState,
            DamageAttributionSummary, DpsTimeBasis, TeamDps, TeamDpsExport,
        },
        parser::{
            CHARACTER_DATA_PATH, EQUIPMENT_CATALOG_PATH, EquipmentCatalog, load_equipment_catalog,
        },
    },
    platform::{
        mod_loader::ModLoaderRuntimeService,
        mods_plugin::{
            ModsPluginClient, ModsPluginGameRegion, ModsPluginOperation, ModsPluginReceiveError,
            ModsPluginSubmitError,
        },
    },
    storage::{
        capture_logs::{ClearOutcome, clear_capture_logs, scan_capture_logs},
        config::{
            self, AccentColor, DpsTimeMode, GlobalHotkeys, HudConfig, HudModule,
            ModStudioLoadingMethod, PassthroughHotkey, ThemePreset, TimelineDpsViewMode, UiConfig,
            UiDensity, sanitize_timeline_bucket_seconds,
        },
        history::{
            HistoryCombatDetails, HistoryIndexRecord, HistoryRecord, load_history_index,
            load_history_record_from_path, save_summary, save_summary_with_details,
        },
        i18n::Language,
        paths::{capture_log_dir, software_dir},
        update::PreparedUpdate,
    },
};

use crate::{
    contract::{
        HudWindowSnapshot, TECHNICAL_CONTRACT_VERSION, TechnicalSnapshot,
        main_dps_detail::MainDpsDetailSnapshot,
        settings::{CaptureDeviceSnapshot, SettingsSnapshot, UpdateSettingsSnapshot},
    },
    windows::hud::HUD_WINDOW_LABEL,
};

/// Maximum coalescing latency for a changed HUD projection. The stream checks
/// only cheap revisions at this cadence and skips full snapshots while idle.
pub(crate) const TECHNICAL_STREAM_INTERVAL_MS: u32 = 100;
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

#[derive(Clone)]
pub(crate) struct AppState(Arc<AppStateInner>);

pub(crate) struct ReplayImportReservation {
    state: AppState,
    active: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct StreamRevision {
    capture: u64,
    presentation: u64,
}

struct StreamEntry {
    owner_window: String,
    stop: Arc<AtomicBool>,
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

#[derive(Clone)]
struct PausedPresentation {
    state: Arc<CombatState>,
    packet_revision: PacketStreamRevision,
}

#[derive(Clone)]
struct SelectedRoundPresentation {
    record_id: String,
    state: Arc<CombatState>,
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
    path: PathBuf,
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

impl HistoryRoundIndex {
    fn from_storage(record: &HistoryIndexRecord) -> Self {
        Self {
            path: record.path.clone(),
            id: record.id.clone(),
            display_time: record.display_time.clone(),
            abyss_floor: record.abyss_floor,
            has_details: record.has_details,
        }
    }

    #[cfg(test)]
    pub(crate) fn for_test(id: &str, has_details: bool) -> Self {
        Self {
            path: PathBuf::new(),
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
    processing_paused: AtomicBool,
    selected_abyss_half: Mutex<Option<AbyssHalf>>,
    observed_abyss_half: Mutex<Option<AbyssHalf>>,
    selected_round: Mutex<Option<SelectedRoundPresentation>>,
    selected_outgoing_revision: AtomicU64,
    character_detail_request: Mutex<MainDpsDetailRequest>,
    team_detail_request: Mutex<MainDpsDetailRequest>,
    detail_cache: Mutex<Vec<MainDpsDetailCache>>,
    paused: Mutex<Option<PausedPresentation>>,
}

/// Serializes history persistence and owns revision/cache/retry/undo state.
/// Capture only cuts rounds; disk I/O and retry lifecycle stay here.
#[derive(Default)]
struct HistoryService {
    revision: AtomicU64,
    undo_sequence: AtomicU64,
    round_cache: Mutex<MainRoundCache>,
    transaction: Mutex<()>,
    archive_transaction: Mutex<()>,
    pending_archives: Mutex<VecDeque<PreparedHistoryArchive>>,
    undo: Mutex<Option<HistoryUndoEntry>>,
}

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
    started_at: Instant,
    sequence: AtomicU64,
    passthrough: AtomicBool,
    passthrough_hotkey_ready: AtomicBool,
    always_on_top: AtomicBool,
    settings_revision: AtomicU64,
    onboarding_step: AtomicU64,
    island_notice_revision: AtomicU64,
    replay_import_reserved: Mutex<bool>,
    diagnostics_revision: AtomicU64,
    session_undo_sequence: AtomicU64,
    character_data_revision: AtomicU64,
    empty_curtain_operation_revision: AtomicU64,
    streams: Mutex<HashMap<String, StreamEntry>>,
    live_capture: LiveCaptureService,
    mod_studio: ModStudioWorkspaceService,
    mod_loader: ModLoaderRuntimeService,
    equipment_catalog: Arc<EquipmentCatalog>,
    mods_plugin: Mutex<ModsPluginClient>,
    empty_curtain_operation: Mutex<EmptyCurtainOperationState>,
    capture_devices: Mutex<Vec<CaptureDeviceSnapshot>>,
    imported_teams: Mutex<(Option<TeamDps>, Option<TeamDps>)>,
    update_runtime: Mutex<UpdateRuntimeState>,
    presentation: PresentationState,
    history: HistoryService,
    passthrough_transaction: Mutex<()>,
    config_transaction: Mutex<()>,
    character_data_transaction: Mutex<()>,
    encrypted_ini: Mutex<EncryptedIniRuntimeState>,
    diagnostics_report: Mutex<Option<DiagnosticRun>>,
    session_undo: Mutex<Option<SessionUndoEntry>>,
    island_notice: Mutex<Option<IslandNoticeState>>,
    ui_config: Mutex<UiConfig>,
    config_path: PathBuf,
    character_data_path: PathBuf,
}

#[derive(Clone, Debug)]
pub(crate) struct EmptyCurtainOperationState {
    pub status: &'static str,
    pub message_key: &'static str,
    pub message_arguments: Vec<String>,
    request_id: Option<u64>,
}

#[derive(Default)]
struct EncryptedIniRuntimeState {
    generation: u64,
    path: Option<PathBuf>,
    document: Option<EncryptedIniDocument>,
}

#[derive(Clone, Debug)]
pub(crate) struct EncryptedIniProjection {
    pub generation: u64,
    pub display_path: Option<String>,
    pub file_name: Option<String>,
    pub key: EncryptedIniKey,
    pub plaintext: String,
    pub encrypted_line_count: usize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum EncryptedIniRuntimeError {
    NoFile,
    StaleGeneration,
    Document(EncryptedIniError),
}

impl From<EncryptedIniError> for EncryptedIniRuntimeError {
    fn from(error: EncryptedIniError) -> Self {
        Self::Document(error)
    }
}

impl Default for EmptyCurtainOperationState {
    fn default() -> Self {
        Self {
            status: "idle",
            message_key: "No equipment operation is pending",
            message_arguments: Vec::new(),
            request_id: None,
        }
    }
}

struct HistoryUndoEntry {
    token: String,
    record: HistoryRecord,
    expires_at: Instant,
}

struct SessionUndoEntry {
    token: String,
    state: CombatState,
    quality_source: CaptureQualitySource,
    expires_at: Instant,
}

pub(crate) const HISTORY_UNDO_WINDOW: Duration = Duration::from_secs(5);
pub(crate) const SESSION_UNDO_WINDOW: Duration = Duration::from_secs(5);
pub(crate) const ISLAND_NOTICE_WINDOW: Duration = Duration::from_secs(5);
/// FIFO retry queue for already-cut rounds. A full queue rejects the next
/// round boundary before capture state is detached, preserving live data.
const MAX_PENDING_HISTORY_ARCHIVES: usize = 64;

#[derive(Clone, Debug)]
struct UpdateRuntimeState {
    status: &'static str,
    message_key: &'static str,
    message_arguments: Vec<String>,
    available: Vec<AvailableComponentUpdate>,
    active_component: Option<UpdateComponent>,
    downloaded_bytes: u64,
    total_bytes: u64,
    prepared: Option<PreparedUpdate>,
}

impl Default for UpdateRuntimeState {
    fn default() -> Self {
        Self {
            status: "idle",
            message_key: "Updates have not been checked in this session",
            message_arguments: Vec::new(),
            available: Vec::new(),
            active_component: None,
            downloaded_bytes: 0,
            total_bytes: 0,
            prepared: None,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum UpdateActionError {
    Busy,
    Unavailable,
    NotPrepared,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum SessionUndoError {
    Missing,
    Expired,
    Busy,
    NewData,
}

fn update_is_busy(status: &str) -> bool {
    matches!(
        status,
        "checking" | "downloading" | "installing" | "restarting"
    )
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
        mut self,
        kind: CaptureReplayKind,
        path: PathBuf,
        replace_current: bool,
    ) -> Result<(), CoreError> {
        let mut reserved = self
            .state
            .0
            .replay_import_reserved
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        let result = if replace_current {
            self.state
                .stop_active_capture_and_wait(Duration::from_secs(5))
                .and_then(|()| self.state.request_diagnostics_replay(kind, path))
        } else {
            self.state.request_diagnostics_replay(kind, path)
        };
        *reserved = false;
        drop(reserved);
        self.active = false;
        result
    }
}

impl Drop for ReplayImportReservation {
    fn drop(&mut self) {
        if self.active {
            *self
                .state
                .0
                .replay_import_reserved
                .lock()
                .unwrap_or_else(|poison| poison.into_inner()) = false;
        }
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
        let capture_devices = enumerate_devices()
            .unwrap_or_default()
            .iter()
            .map(CaptureDeviceSnapshot::from)
            .collect();
        let equipment_catalog = load_equipment_catalog(std::path::Path::new(
            EQUIPMENT_CATALOG_PATH,
        ))
        .unwrap_or_else(|error| {
            log::error!("load Console equipment catalog for Tauri failed: {error:#}");
            EquipmentCatalog::default()
        });
        Self(Arc::new(AppStateInner {
            started_at: Instant::now(),
            sequence: AtomicU64::new(0),
            passthrough: AtomicBool::new(false),
            passthrough_hotkey_ready: AtomicBool::new(false),
            always_on_top: AtomicBool::new(
                config
                    .hud_always_on_top
                    .expect("sanitized HUD always-on-top state"),
            ),
            settings_revision: AtomicU64::new(0),
            onboarding_step: AtomicU64::new(0),
            island_notice_revision: AtomicU64::new(0),
            replay_import_reserved: Mutex::new(false),
            diagnostics_revision: AtomicU64::new(0),
            session_undo_sequence: AtomicU64::new(0),
            character_data_revision: AtomicU64::new(0),
            empty_curtain_operation_revision: AtomicU64::new(0),
            streams: Mutex::new(HashMap::new()),
            live_capture,
            mod_studio: ModStudioWorkspaceService::default(),
            mod_loader: ModLoaderRuntimeService::default(),
            equipment_catalog: Arc::new(equipment_catalog),
            mods_plugin: Mutex::new(ModsPluginClient::new()),
            empty_curtain_operation: Mutex::new(EmptyCurtainOperationState::default()),
            capture_devices: Mutex::new(capture_devices),
            imported_teams: Mutex::new((None, None)),
            update_runtime: Mutex::new(UpdateRuntimeState::default()),
            presentation: PresentationState::default(),
            history: HistoryService::default(),
            passthrough_transaction: Mutex::new(()),
            config_transaction: Mutex::new(()),
            character_data_transaction: Mutex::new(()),
            encrypted_ini: Mutex::new(EncryptedIniRuntimeState::default()),
            diagnostics_report: Mutex::new(None),
            session_undo: Mutex::new(None),
            island_notice: Mutex::new(None),
            ui_config: Mutex::new(config),
            config_path,
            character_data_path: software_dir().join(CHARACTER_DATA_PATH),
        }))
    }

    pub(crate) fn snapshot(&self) -> TechnicalSnapshot {
        let sequence = self.next_sequence();
        let config = self.ui_config();
        let hud_config = config.hud.clone();
        let supported_locales = Language::all()
            .iter()
            .map(|language| language.code())
            .collect();

        TechnicalSnapshot {
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
                let selected_abyss_half = *self
                    .0
                    .presentation
                    .selected_abyss_half
                    .lock()
                    .unwrap_or_else(|poison| poison.into_inner());
                let mut hud = project_hud(
                    state,
                    &hud_config,
                    &HashSet::new(),
                    HudProjectionOptions {
                        dps_time_basis: DpsTimeBasis::from_subtract_time_stop(matches!(
                            config.dps_time_mode,
                            DpsTimeMode::TimeStopAdjusted
                        )),
                        separate_reaction_damage: config.separate_reaction_damage,
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
            }),
        }
    }

    pub(crate) fn next_sequence(&self) -> u64 {
        self.0.sequence.fetch_add(1, Ordering::Relaxed) + 1
    }

    pub(crate) fn publish_island_notice(
        &self,
        tone: &'static str,
        message_key: &'static str,
        message_arguments: Vec<String>,
        undo_token: Option<String>,
    ) -> String {
        let revision = self.0.island_notice_revision.fetch_add(1, Ordering::AcqRel) + 1;
        let id = format!("notice-{revision:016x}");
        *self
            .0
            .island_notice
            .lock()
            .unwrap_or_else(|poison| poison.into_inner()) = Some(IslandNoticeState {
            id: id.clone(),
            tone,
            message_key,
            message_arguments,
            undo_token,
            expires_at: Instant::now() + ISLAND_NOTICE_WINDOW,
        });
        id
    }

    pub(crate) fn island_notice(&self) -> Option<IslandNoticeState> {
        let mut notice = self
            .0
            .island_notice
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        if notice
            .as_ref()
            .is_some_and(|notice| Instant::now() > notice.expires_at)
        {
            notice.take();
        }
        notice.clone()
    }

    pub(crate) fn dismiss_island_notice(&self, id: &str) -> bool {
        let mut notice = self
            .0
            .island_notice
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        if notice.as_ref().is_some_and(|notice| notice.id == id) {
            notice.take();
            return true;
        }
        false
    }

    pub(crate) fn ui_config_snapshot(&self) -> UiConfig {
        self.ui_config()
    }

    pub(crate) fn capture_device_count(&self) -> usize {
        self.0
            .capture_devices
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
            .len()
    }

    pub(crate) fn set_onboarding_progress(&self, step: usize, done: bool) -> Result<bool, String> {
        self.0
            .onboarding_step
            .store(step.min(3) as u64, Ordering::Release);
        self.bump_main_dps_revision();
        self.update_ui_config(|config| config.onboarding_done = done)
    }

    pub(crate) fn finish_onboarding(&self, preset: HudPreset) -> Result<bool, String> {
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
        self.0.onboarding_step.store(3, Ordering::Release);
        self.bump_main_dps_revision();
        Ok(changed)
    }

    pub(crate) fn onboarding_step(&self) -> usize {
        self.0.onboarding_step.load(Ordering::Acquire).min(3) as usize
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
    ) -> Result<bool, String> {
        self.update_ui_config(|config| {
            config.abyss_window_size = Some(size);
            config.abyss_window_position = Some(position);
        })
    }

    pub(crate) fn set_console_window_geometry(
        &self,
        size: [f32; 2],
        position: [f32; 2],
    ) -> Result<bool, String> {
        let _transaction = self
            .0
            .config_transaction
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        let previous = self.ui_config();
        let mut candidate = previous.clone();
        candidate.console_window_size = Some(size);
        candidate.console_window_position = Some(position);
        candidate = candidate.sanitized();
        if candidate == previous {
            return Ok(false);
        }
        config::save(&self.0.config_path, &candidate)?;
        *self
            .0
            .ui_config
            .lock()
            .unwrap_or_else(|poison| poison.into_inner()) = candidate;
        Ok(true)
    }

    pub(crate) fn set_hit_detail_columns(
        &self,
        columns: nte_dps_tool::storage::config::HitDetailColumnsConfig,
    ) -> Result<bool, String> {
        self.update_ui_config(|config| config.hit_detail_columns = columns)
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
    ) -> Result<bool, String> {
        self.update_ui_config(|config| match kind {
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

    pub(crate) fn replay_running(&self) -> bool {
        self.0.live_capture.replay_running()
    }

    pub(crate) fn main_processing_paused(&self) -> bool {
        self.0
            .presentation
            .processing_paused
            .load(Ordering::Acquire)
    }

    pub(crate) fn main_paused_event_counts(&self) -> (u64, u64) {
        let paused = self
            .0
            .presentation
            .paused
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
            .clone();
        let Some(paused) = paused else {
            return (0, 0);
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

    pub(crate) fn set_main_processing_paused(&self, paused: bool) {
        if self.main_processing_paused() == paused {
            return;
        }
        if paused {
            let frozen = self
                .0
                .live_capture
                .with_packet_state(|packet_revision, _, state| PausedPresentation {
                    state: Arc::new(state.clone()),
                    packet_revision,
                });
            *self
                .0
                .presentation
                .paused
                .lock()
                .unwrap_or_else(|poison| poison.into_inner()) = Some(frozen);
        } else {
            self.0
                .presentation
                .paused
                .lock()
                .unwrap_or_else(|poison| poison.into_inner())
                .take();
        }
        self.0
            .presentation
            .processing_paused
            .store(paused, Ordering::Release);
        self.0.presentation.revision.fetch_add(1, Ordering::AcqRel);
        self.bump_main_dps_revision();
    }

    pub(crate) fn main_selected_round_id(&self) -> Option<String> {
        let mut selected = self
            .0
            .presentation
            .selected_round
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        if selected.is_some()
            && !self.main_processing_paused()
            && self.0.live_capture.outgoing_hit_revision()
                != self
                    .0
                    .presentation
                    .selected_outgoing_revision
                    .load(Ordering::Acquire)
        {
            selected.take();
            drop(selected);
            self.bump_main_dps_revision();
            return None;
        }
        selected
            .as_ref()
            .map(|selection| selection.record_id.clone())
    }

    pub(crate) fn set_main_selected_round_id(&self, record_id: Option<String>) -> Result<(), ()> {
        let selection = match record_id {
            Some(record_id) => {
                let index = self.main_round_index();
                let record = index
                    .iter()
                    .find(|record| record.id == record_id && record.has_details)
                    .and_then(|record| load_history_record_from_path(&record.path).ok())
                    .ok_or(())?;
                let state = record
                    .details
                    .as_ref()
                    .map(HistoryCombatDetails::to_combat_state)
                    .ok_or(())?;
                Some(SelectedRoundPresentation {
                    record_id,
                    state: Arc::new(state),
                })
            }
            None => None,
        };
        let mut selected = self
            .0
            .presentation
            .selected_round
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        if selected.as_ref().map(|value| value.record_id.as_str())
            == selection.as_ref().map(|value| value.record_id.as_str())
        {
            return Ok(());
        }
        self.0.presentation.selected_outgoing_revision.store(
            self.0.live_capture.outgoing_hit_revision(),
            Ordering::Release,
        );
        *selected = selection;
        drop(selected);
        self.bump_main_dps_revision();
        Ok(())
    }

    pub(crate) fn main_round_index(&self) -> Arc<Vec<HistoryRoundIndex>> {
        let revision = self.history_revision();
        let mut cache = self
            .0
            .history
            .round_cache
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        if cache.revision != Some(revision) {
            let records = load_history_index().records;
            cache.index = Arc::new(
                records
                    .iter()
                    .map(HistoryRoundIndex::from_storage)
                    .collect(),
            );
            cache.revision = Some(revision);
        }
        let index = Arc::clone(&cache.index);
        drop(cache);
        let stale_selection = self
            .main_selected_round_id()
            .is_some_and(|id| !index.iter().any(|record| record.id == id));
        if stale_selection {
            self.0
                .presentation
                .selected_round
                .lock()
                .unwrap_or_else(|poison| poison.into_inner())
                .take();
            self.bump_main_dps_revision();
        }
        index
    }

    pub(crate) fn main_dps_readout(&self, selected_round_id: Option<&str>) -> MainDpsReadout {
        let follow_live_half = selected_round_id.is_none() && !self.main_processing_paused();
        self.with_main_presented_state(|state| self.project_main_readout(state, follow_live_half))
    }

    pub(crate) fn main_dps_detail_request(&self, kind: MainDpsDetailKind) -> MainDpsDetailRequest {
        match kind {
            MainDpsDetailKind::Character => self
                .0
                .presentation
                .character_detail_request
                .lock()
                .unwrap_or_else(|poison| poison.into_inner())
                .clone(),
            MainDpsDetailKind::Team => self
                .0
                .presentation
                .team_detail_request
                .lock()
                .unwrap_or_else(|poison| poison.into_inner())
                .clone(),
        }
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
        let cache = self
            .0
            .presentation
            .detail_cache
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
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
        let mut cache = self
            .0
            .presentation
            .detail_cache
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
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
    ) {
        match kind {
            MainDpsDetailKind::Character => {
                *self
                    .0
                    .presentation
                    .character_detail_request
                    .lock()
                    .unwrap_or_else(|poison| poison.into_inner()) = request;
            }
            MainDpsDetailKind::Team => {
                *self
                    .0
                    .presentation
                    .team_detail_request
                    .lock()
                    .unwrap_or_else(|poison| poison.into_inner()) = request;
            }
        }
    }

    pub(crate) fn with_main_dps_detail_state<T>(
        &self,
        project: impl FnOnce(&CombatState, Option<AbyssHalf>) -> T,
    ) -> T {
        self.with_main_presented_state(|state| {
            let selected_half = state.abyss.is_active().then(|| {
                (*self
                    .0
                    .presentation
                    .selected_abyss_half
                    .lock()
                    .unwrap_or_else(|poison| poison.into_inner()))
                .or(state.abyss.active_half)
                .unwrap_or(AbyssHalf::First)
            });
            project(state, selected_half)
        })
    }

    pub(crate) fn set_main_selected_abyss_half(&self, half: Option<AbyssHalf>) {
        let mut selected = self
            .0
            .presentation
            .selected_abyss_half
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        if *selected == half {
            return;
        }
        *selected = half;
        drop(selected);
        self.bump_main_dps_revision();
    }

    pub(crate) fn update_main_appearance(
        &self,
        dark_mode: bool,
        opacity: f32,
    ) -> Result<bool, String> {
        self.update_ui_config(|config| {
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
    ) -> Result<bool, String> {
        self.update_ui_config(|config| {
            config.main_window_size = size;
            config.main_window_position = position;
        })
    }

    pub(crate) fn main_dps_stream_revision(&self) -> MainDpsStreamRevision {
        let selected_history = self.main_selected_round_id().is_some();
        let paused = self.main_processing_paused();
        MainDpsStreamRevision {
            capture: if selected_history {
                0
            } else {
                self.0.live_capture.revision()
            },
            packet: if paused && !selected_history {
                self.0
                    .live_capture
                    .with_packet_state(|revision, _, _| revision.generation)
            } else {
                0
            },
            presentation: self.0.presentation.revision.load(Ordering::Acquire),
            history: self.history_revision(),
            main: self.0.presentation.main_revision.load(Ordering::Acquire),
        }
    }

    fn project_main_readout(&self, state: &CombatState, follow_live_half: bool) -> MainDpsReadout {
        let config = self.ui_config();
        let selected_abyss_half = if follow_live_half {
            self.follow_live_abyss_half(state.abyss.active_half)
        } else {
            *self
                .0
                .presentation
                .selected_abyss_half
                .lock()
                .unwrap_or_else(|poison| poison.into_inner())
        };
        let subtract_time_stop = matches!(config.dps_time_mode, DpsTimeMode::TimeStopAdjusted);
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
                (state.damage_attribution_summary(), durations)
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
                (party.damage_attribution_summary(), durations)
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

    fn follow_live_abyss_half(&self, active_half: Option<AbyssHalf>) -> Option<AbyssHalf> {
        let mut selected = self
            .0
            .presentation
            .selected_abyss_half
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        let mut observed = self
            .0
            .presentation
            .observed_abyss_half
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        let (next_selected, next_observed) =
            next_live_abyss_selection(*selected, *observed, active_half);
        *selected = next_selected;
        *observed = next_observed;
        next_selected
    }

    fn bump_main_dps_revision(&self) -> u64 {
        self.0
            .presentation
            .main_revision
            .fetch_add(1, Ordering::AcqRel)
            + 1
    }

    pub(crate) fn settings_snapshot(&self) -> SettingsSnapshot {
        let generation = self.settings_revision();
        let config = self.ui_config();
        let devices = self
            .0
            .capture_devices
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
            .clone();
        let (upper_imported, lower_imported) = {
            let imported = self
                .0
                .imported_teams
                .lock()
                .unwrap_or_else(|poison| poison.into_inner());
            (imported.0.is_some(), imported.1.is_some())
        };
        let updates = self.update_settings_snapshot();
        SettingsSnapshot::from_config(
            &config,
            generation,
            self.always_on_top(),
            devices,
            scan_capture_logs(&capture_log_dir()),
            upper_imported,
            lower_imported,
            updates,
        )
    }

    pub(crate) fn update_settings_snapshot(&self) -> UpdateSettingsSnapshot {
        let config = self.ui_config();
        let update = self
            .0
            .update_runtime
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
            .clone();
        let install_blocked_message_key = self.install_blocked_message_key_for(&update);
        UpdateSettingsSnapshot::from_runtime(
            &config,
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
        let replay_import_reserved = self
            .0
            .replay_import_reserved
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        if *replay_import_reserved {
            return Err(CoreError::new(
                CoreErrorCode::CaptureAlreadyRunning,
                "a replay import dialog is active",
            ));
        }
        if self.session_has_data() && !replace_current {
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
        let result = self.0.live_capture.request_start(CaptureControllerOptions {
            profile: CaptureProfile::Combat,
            device,
            filter: config.capture_filter.clone(),
            include_incoming: true,
            server_damage_calibration: config.server_damage_calibration,
            raw_capture: RawCaptureMode::Enabled,
            raw_capture_directory: capture_log_dir(),
            expose_raw_capture_path: false,
            packet_emission: PacketEmissionMode::FullDebug,
        });
        drop(replay_import_reserved);
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
        ) || self.replay_running()
        {
            self.request_capture_stop()?;
        }
        let deadline = Instant::now() + timeout;
        while matches!(
            self.capture_phase(),
            LiveCapturePhase::Starting | LiveCapturePhase::Running | LiveCapturePhase::Stopping
        ) || self.replay_running()
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
        self.0.passthrough.load(Ordering::Acquire)
    }

    pub(crate) fn set_passthrough(&self, enabled: bool) {
        if self.0.passthrough.swap(enabled, Ordering::AcqRel) != enabled {
            self.0.presentation.revision.fetch_add(1, Ordering::AcqRel);
        }
    }

    pub(crate) fn passthrough_hotkey(&self) -> PassthroughHotkey {
        self.0
            .ui_config
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
            .passthrough_hotkey
    }

    pub(crate) fn global_hotkeys(&self) -> GlobalHotkeys {
        self.0
            .ui_config
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
            .global_hotkeys
    }

    pub(crate) fn passthrough_hotkey_ready(&self) -> bool {
        self.0.passthrough_hotkey_ready.load(Ordering::Acquire)
    }

    pub(crate) fn set_passthrough_hotkey_ready(&self, ready: bool) {
        self.0
            .passthrough_hotkey_ready
            .store(ready, Ordering::Release);
    }

    pub(crate) fn lock_passthrough_transaction(&self) -> MutexGuard<'_, ()> {
        self.0
            .passthrough_transaction
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
    }

    pub(crate) fn always_on_top(&self) -> bool {
        self.0.always_on_top.load(Ordering::Acquire)
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
        self.0
            .ui_config
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
            .hud_window_position
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
    ) -> Result<bool, String> {
        self.update_hud_config(|hud| hud.set_module_visible(module, visible))
    }

    pub(crate) fn move_hud_module(
        &self,
        dragged: HudModule,
        target: HudModule,
        insert_after: bool,
    ) -> Result<bool, String> {
        self.update_hud_config(|hud| hud.move_module(dragged, target, insert_after))
    }

    pub(crate) fn set_hud_width(&self, width: u16) -> Result<bool, String> {
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
    ) -> Result<bool, String> {
        self.update_ui_config(|config| {
            config.language = language;
            config.dark_mode = dark_mode;
            config.theme_preset = theme_preset;
            config.accent = accent;
            config.density = density;
            config.reduce_motion = reduce_motion;
            config.island_notifications = island_notifications;
            config.island_offset_x = island_offset_x;
        })
    }

    pub(crate) fn update_update_settings(
        &self,
        auto_check: bool,
        auto_download: bool,
    ) -> Result<bool, String> {
        self.update_ui_config(|config| {
            config.auto_check_updates = auto_check;
            config.auto_download_updates = auto_download;
        })
    }

    pub(crate) fn auto_check_updates(&self) -> bool {
        self.0
            .ui_config
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
            .auto_check_updates
    }

    pub(crate) fn auto_download_updates(&self) -> bool {
        self.0
            .ui_config
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
            .auto_download_updates
    }

    pub(crate) fn begin_update_check(&self) -> Result<(), UpdateActionError> {
        let mut update = self
            .0
            .update_runtime
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        if update_is_busy(update.status) || update.prepared.is_some() {
            return Err(UpdateActionError::Busy);
        }
        update.status = "checking";
        update.message_key = "Checking for updates...";
        update.message_arguments.clear();
        update.active_component = None;
        update.downloaded_bytes = 0;
        update.total_bytes = 0;
        drop(update);
        self.bump_settings_revision();
        Ok(())
    }

    pub(crate) fn finish_update_check(&self, available: Vec<AvailableComponentUpdate>) {
        let mut update = self
            .0
            .update_runtime
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        update.prepared = None;
        update.active_component = None;
        update.downloaded_bytes = 0;
        update.total_bytes = 0;
        if available.is_empty() {
            update.available.clear();
            update.status = "up-to-date";
            update.message_key = "All available update components are up to date";
            update.message_arguments.clear();
        } else {
            let preferred = available
                .iter()
                .find(|item| item.component == UpdateComponent::App)
                .unwrap_or(&available[0]);
            update.status = "available";
            update.message_key = match preferred.component {
                UpdateComponent::App => "Version {} is available",
                UpdateComponent::ModsPlugin => "Mod loader version {} is available",
            };
            update.message_arguments = vec![preferred.version.to_string()];
            update.available = available;
        }
        drop(update);
        self.bump_settings_revision();
    }

    pub(crate) fn fail_update_check(&self, message_key: &'static str) {
        let mut update = self
            .0
            .update_runtime
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        update.status =
            if message_key == "The official update channel is not configured in this build" {
                "not-configured"
            } else {
                "error"
            };
        update.message_key = message_key;
        update.message_arguments.clear();
        update.available.clear();
        update.prepared = None;
        update.active_component = None;
        update.downloaded_bytes = 0;
        update.total_bytes = 0;
        drop(update);
        self.bump_settings_revision();
    }

    pub(crate) fn begin_update_download(
        &self,
        component: UpdateComponent,
    ) -> Result<AvailableComponentUpdate, UpdateActionError> {
        let mut update = self
            .0
            .update_runtime
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        if update_is_busy(update.status) || update.prepared.is_some() {
            return Err(UpdateActionError::Busy);
        }
        let selected = update
            .available
            .iter()
            .find(|item| item.component == component)
            .cloned()
            .ok_or(UpdateActionError::Unavailable)?;
        update.status = "downloading";
        update.message_key = match component {
            UpdateComponent::App => "Downloading verified update...",
            UpdateComponent::ModsPlugin => "Downloading verified Mod loader...",
        };
        update.message_arguments.clear();
        update.active_component = Some(component);
        update.downloaded_bytes = 0;
        update.total_bytes = selected.artifact_size;
        drop(update);
        self.bump_settings_revision();
        Ok(selected)
    }

    pub(crate) fn update_download_progress(
        &self,
        component: UpdateComponent,
        downloaded_bytes: u64,
        total_bytes: u64,
    ) {
        let mut update = self
            .0
            .update_runtime
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        if update.status != "downloading" || update.active_component != Some(component) {
            return;
        }
        update.downloaded_bytes = downloaded_bytes.min(total_bytes);
        update.total_bytes = total_bytes;
        drop(update);
        self.bump_settings_revision();
    }

    pub(crate) fn finish_update_download(&self, prepared: PreparedUpdate) {
        let component = prepared.component();
        let version = prepared.version().to_string();
        let mut update = self
            .0
            .update_runtime
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        update.status = "ready";
        update.message_key = match component {
            UpdateComponent::App => "Version {} is ready to install",
            UpdateComponent::ModsPlugin => "Mod loader {} is ready to install",
        };
        update.message_arguments = vec![version];
        update.active_component = None;
        update.downloaded_bytes = update.total_bytes;
        update.prepared = Some(prepared);
        drop(update);
        self.bump_settings_revision();
    }

    pub(crate) fn fail_update_download(&self) {
        self.fail_update_operation("Update download failed.");
    }

    pub(crate) fn begin_update_install(&self) -> Result<PreparedUpdate, UpdateActionError> {
        let mut update = self
            .0
            .update_runtime
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        if update_is_busy(update.status) {
            return Err(UpdateActionError::Busy);
        }
        let prepared = update
            .prepared
            .as_ref()
            .cloned()
            .ok_or(UpdateActionError::NotPrepared)?;
        update.status = match prepared.component() {
            UpdateComponent::App => "restarting",
            UpdateComponent::ModsPlugin => "installing",
        };
        update.message_key = match prepared.component() {
            UpdateComponent::App => "Restarting to install the update...",
            UpdateComponent::ModsPlugin => "Installing Mod loader update...",
        };
        update.message_arguments.clear();
        update.active_component = Some(prepared.component());
        drop(update);
        self.bump_settings_revision();
        Ok(prepared)
    }

    pub(crate) fn finish_plugin_update_install(&self, version: String) {
        let mut update = self
            .0
            .update_runtime
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        update
            .available
            .retain(|item| item.component != UpdateComponent::ModsPlugin);
        update.status = if update.available.is_empty() {
            "up-to-date"
        } else {
            "available"
        };
        update.message_key = "Mod loader {} was installed";
        update.message_arguments = vec![version];
        update.active_component = None;
        update.downloaded_bytes = 0;
        update.total_bytes = 0;
        update.prepared = None;
        drop(update);
        self.bump_settings_revision();
    }

    pub(crate) fn fail_update_install(&self) {
        self.fail_update_operation("Update installation could not start.");
    }

    pub(crate) fn update_install_blocked_message_key(&self) -> Option<&'static str> {
        let update = self
            .0
            .update_runtime
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
            .clone();
        self.install_blocked_message_key_for(&update)
    }

    pub(crate) fn notify_update_install_blocker_changed(&self) {
        self.bump_settings_revision();
    }

    fn install_blocked_message_key_for(&self, update: &UpdateRuntimeState) -> Option<&'static str> {
        match update.prepared.as_ref().map(PreparedUpdate::component) {
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

    fn fail_update_operation(&self, message_key: &'static str) {
        let mut update = self
            .0
            .update_runtime
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        update.status = "error";
        update.message_key = message_key;
        update.message_arguments.clear();
        update.active_component = None;
        update.downloaded_bytes = 0;
        update.total_bytes = 0;
        drop(update);
        self.bump_settings_revision();
    }

    fn bump_settings_revision(&self) {
        self.0.settings_revision.fetch_add(1, Ordering::AcqRel);
    }

    pub(crate) fn set_density(&self, density: UiDensity) -> Result<bool, String> {
        self.update_ui_config(|config| config.density = density)
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn update_capture_settings(
        &self,
        filter: String,
        manual_capture_device: Option<String>,
        server_damage_calibration: bool,
        separate_reaction_damage: bool,
        auto_round_after_idle: bool,
        auto_round_idle_seconds: u32,
        dps_time_mode: DpsTimeMode,
        passthrough_hotkey: PassthroughHotkey,
    ) -> Result<bool, String> {
        self.update_ui_config(|config| {
            config.capture_filter = filter;
            config.manual_capture_device = manual_capture_device;
            config.server_damage_calibration = server_damage_calibration;
            config.separate_reaction_damage = separate_reaction_damage;
            config.auto_round_after_idle = auto_round_after_idle;
            config.auto_round_idle_seconds = auto_round_idle_seconds;
            config.dps_time_mode = dps_time_mode;
            config.passthrough_hotkey = passthrough_hotkey;
        })
    }

    pub(crate) fn update_global_hotkeys(
        &self,
        global_hotkeys: GlobalHotkeys,
    ) -> Result<bool, String> {
        self.update_ui_config(|config| config.global_hotkeys = global_hotkeys)
    }

    pub(crate) fn refresh_capture_devices(&self) -> Result<(), CoreError> {
        let devices = enumerate_devices()?
            .iter()
            .map(CaptureDeviceSnapshot::from)
            .collect();
        *self
            .0
            .capture_devices
            .lock()
            .unwrap_or_else(|poison| poison.into_inner()) = devices;
        self.0.settings_revision.fetch_add(1, Ordering::AcqRel);
        Ok(())
    }

    pub(crate) fn clear_capture_files(&self) -> ClearOutcome {
        clear_capture_logs(&capture_log_dir())
    }

    pub(crate) fn refresh_capture_file_stats(&self) {
        self.bump_settings_revision();
    }

    pub(crate) fn session_has_data(&self) -> bool {
        self.0.live_capture.with_state(|state| {
            !state.hits.is_empty()
                || !state.packets.is_empty()
                || !state.stats.is_empty()
                || !state.empty_curtain.is_empty()
                || state.abyss.is_active()
        })
    }

    pub(crate) fn reset_session_with_undo(&self) -> Option<String> {
        let previous = self.0.live_capture.with_state(Clone::clone);
        let has_data = !previous.hits.is_empty()
            || !previous.packets.is_empty()
            || !previous.stats.is_empty()
            || !previous.empty_curtain.is_empty()
            || previous.abyss.is_active();
        if !has_data {
            self.0.live_capture.reset_session();
            self.return_main_presentation_to_live();
            return None;
        }
        let sequence = self.0.session_undo_sequence.fetch_add(1, Ordering::AcqRel) + 1;
        let token = format!("session-undo-{sequence:016x}");
        *self
            .0
            .session_undo
            .lock()
            .unwrap_or_else(|poison| poison.into_inner()) = Some(SessionUndoEntry {
            token: token.clone(),
            state: previous,
            quality_source: self.0.live_capture.quality_source(),
            expires_at: Instant::now() + SESSION_UNDO_WINDOW,
        });
        self.0.live_capture.reset_session();
        self.return_main_presentation_to_live();
        Some(token)
    }

    pub(crate) fn clear_session(&self) {
        self.0
            .session_undo
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
            .take();
        self.0.live_capture.reset_session();
        self.return_main_presentation_to_live();
    }

    pub(crate) fn undo_session_reset(&self, token: &str) -> Result<(), SessionUndoError> {
        if matches!(
            self.capture_phase(),
            LiveCapturePhase::Starting | LiveCapturePhase::Running | LiveCapturePhase::Stopping
        ) || self.replay_running()
        {
            return Err(SessionUndoError::Busy);
        }
        if self.session_has_data() {
            return Err(SessionUndoError::NewData);
        }
        let mut undo = self
            .0
            .session_undo
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        if undo.as_ref().is_none_or(|entry| entry.token != token) {
            return Err(SessionUndoError::Missing);
        }
        let entry = undo.take().expect("validated session undo entry missing");
        drop(undo);
        if Instant::now() > entry.expires_at {
            return Err(SessionUndoError::Expired);
        }
        self.0
            .live_capture
            .restore_session(entry.state, entry.quality_source);
        Ok(())
    }

    pub(crate) fn import_team_data(&self, export: TeamDpsExport) {
        let mut imported = self
            .0
            .imported_teams
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        let fallback = export.single;
        imported.0 = export.upper.or_else(|| fallback.clone());
        imported.1 = export.lower.or(fallback);
        self.0.settings_revision.fetch_add(1, Ordering::AcqRel);
    }

    pub(crate) fn imported_abyss_teams(&self) -> (Option<TeamDps>, Option<TeamDps>) {
        self.0
            .imported_teams
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
            .clone()
    }

    pub(crate) fn import_abyss_team(&self, export: TeamDpsExport, upper: bool) -> bool {
        let preferred = if upper { export.upper } else { export.lower };
        let Some(team) = preferred.or(export.single) else {
            return false;
        };
        let mut imported = self
            .0
            .imported_teams
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        if upper {
            imported.0 = Some(team);
        } else {
            imported.1 = Some(team);
        }
        self.0.settings_revision.fetch_add(1, Ordering::AcqRel);
        true
    }

    pub(crate) fn import_current_abyss_team(&self, upper: bool) -> bool {
        let Some(team) = self.current_abyss_team(upper) else {
            return false;
        };
        let mut imported = self
            .0
            .imported_teams
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        if upper {
            imported.0 = Some(team);
        } else {
            imported.1 = Some(team);
        }
        self.0.settings_revision.fetch_add(1, Ordering::AcqRel);
        true
    }

    pub(crate) fn current_abyss_team_availability(&self) -> [bool; 2] {
        [
            self.current_abyss_team(true).is_some(),
            self.current_abyss_team(false).is_some(),
        ]
    }

    fn current_abyss_team(&self, upper: bool) -> Option<TeamDps> {
        let config = self.ui_config();
        self.with_main_presented_state(|state| {
            let export = nte_dps_tool::core::team_data::export_team_data(
                state,
                matches!(config.dps_time_mode, DpsTimeMode::TimeStopAdjusted),
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

    pub(crate) fn clear_abyss_team(&self, upper: bool) {
        let mut imported = self
            .0
            .imported_teams
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        if upper {
            imported.0 = None;
        } else {
            imported.1 = None;
        }
        self.0.settings_revision.fetch_add(1, Ordering::AcqRel);
    }

    pub(crate) fn swap_abyss_teams(&self) {
        let mut imported = self
            .0
            .imported_teams
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        let (upper, lower) = &mut *imported;
        std::mem::swap(upper, lower);
        self.0.settings_revision.fetch_add(1, Ordering::AcqRel);
    }

    pub(crate) fn export_team_data(&self) -> Option<TeamDpsExport> {
        let config = self.ui_config();
        let imported = self
            .0
            .imported_teams
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        let imported = imported.clone();
        self.with_main_presented_state(|state| {
            nte_dps_tool::core::team_data::export_team_data(
                state,
                matches!(config.dps_time_mode, DpsTimeMode::TimeStopAdjusted),
                config.separate_reaction_damage,
                imported.0,
                imported.1,
            )
        })
    }

    pub(crate) fn poll_mod_studio_runtime(
        &self,
    ) -> Result<ModStudioRuntimeSnapshot, ModStudioError> {
        poll_mod_studio_runtime()
    }

    pub(crate) fn set_hud_option(
        &self,
        option: HudSettingOption,
        enabled: bool,
    ) -> Result<bool, String> {
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

    pub(crate) fn apply_hud_preset(&self, preset: HudPreset) -> Result<bool, String> {
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

    pub(crate) fn set_hud_window_position(&self, position: [i32; 2]) -> Result<bool, String> {
        let _transaction = self
            .0
            .config_transaction
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        let mut candidate = self.ui_config();
        if candidate.hud_window_position == Some(position) {
            return Ok(false);
        }
        candidate.hud_window_position = Some(position);
        config::save(&self.0.config_path, &candidate)?;
        *self
            .0
            .ui_config
            .lock()
            .unwrap_or_else(|poison| poison.into_inner()) = candidate;
        Ok(true)
    }

    fn update_hud_config(&self, update: impl FnOnce(&mut HudConfig)) -> Result<bool, String> {
        self.update_ui_config(|config| update(&mut config.hud))
    }

    pub(crate) fn set_always_on_top(&self, enabled: bool) -> Result<bool, String> {
        self.set_window_always_on_top(DesktopWindowKind::Hud, enabled)
    }

    pub(crate) fn set_window_always_on_top(
        &self,
        window: DesktopWindowKind,
        enabled: bool,
    ) -> Result<bool, String> {
        let changed = self.update_ui_config(|config| match window {
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
            self.0.always_on_top.store(enabled, Ordering::Release);
        }
        Ok(changed)
    }

    pub(crate) fn prepare_current_history_archive(&self) -> Option<PreparedHistoryArchive> {
        let config = self.ui_config();
        let (state, source) = self.0.live_capture.state_and_source_snapshot();
        prepare_history_archive(
            &state,
            source,
            DpsTimeBasis::from_subtract_time_stop(matches!(
                config.dps_time_mode,
                DpsTimeMode::TimeStopAdjusted
            )),
            config.separate_reaction_damage,
        )
    }

    fn prepare_history_details(
        &self,
        pending: PendingHistoryArchive,
    ) -> Option<PreparedHistoryArchive> {
        let config = self.ui_config();
        let state = pending.details.to_combat_state();
        state
            .session_summary(
                pending.source,
                DpsTimeBasis::from_subtract_time_stop(matches!(
                    config.dps_time_mode,
                    DpsTimeMode::TimeStopAdjusted
                )),
                config.separate_reaction_damage,
            )
            .map(|summary| PreparedHistoryArchive {
                summary,
                details: Some(pending.details),
            })
    }

    fn persist_history_archive(&self, archive: PreparedHistoryArchive) -> Result<(), String> {
        self.with_history_transaction(|| {
            match archive.details {
                Some(details) => save_summary_with_details(archive.summary, details),
                None => save_summary(archive.summary),
            }
            .map(|_| ())
        })?;
        self.bump_history_revision();
        Ok(())
    }

    pub(crate) fn archive_current_history_round(&self) -> Result<bool, String> {
        if self.replay_running() {
            return Err("History round cuts are unavailable during replay".to_owned());
        }
        self.archive_current_history_round_with(|state, archive| {
            state.persist_history_archive(archive)
        })
    }

    fn archive_current_history_round_with(
        &self,
        persist: impl FnOnce(&Self, PreparedHistoryArchive) -> Result<(), String>,
    ) -> Result<bool, String> {
        let _archive_transaction = self
            .0
            .history
            .archive_transaction
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        if self
            .0
            .history
            .pending_archives
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
            .len()
            >= MAX_PENDING_HISTORY_ARCHIVES
        {
            return Err("History retry queue is full; the current round was kept live".to_owned());
        }
        let config = self.ui_config();
        let Some(cut) = self.0.live_capture.cut_round() else {
            return Ok(false);
        };
        let Some(archive) = prepare_history_archive(
            &cut.state,
            cut.source,
            DpsTimeBasis::from_subtract_time_stop(matches!(
                config.dps_time_mode,
                DpsTimeMode::TimeStopAdjusted
            )),
            config.separate_reaction_damage,
        ) else {
            log::warn!("cut History round had no archivable summary");
            return Ok(true);
        };
        if let Err(error) = persist(self, archive.clone()) {
            log::warn!("History persistence failed after the live round was cut: {error}");
            self.0
                .history
                .pending_archives
                .lock()
                .unwrap_or_else(|poison| poison.into_inner())
                .push_back(archive);
        }
        Ok(true)
    }

    pub(crate) fn maintain_history_rounds(&self) {
        self.retry_pending_history_archives();
        let pending = self.0.live_capture.take_pending_abyss_archives();
        let mut retry = Vec::new();
        for pending in pending {
            let Some(archive) = self.prepare_history_details(pending.clone()) else {
                continue;
            };
            if let Err(error) = self.persist_history_archive(archive) {
                log::warn!("automatic Abyss History archive failed: {error}");
                retry.push(pending);
            }
        }
        if !retry.is_empty() {
            self.0.live_capture.restore_pending_abyss_archives(retry);
            return;
        }

        let config = self.ui_config();
        if !config.auto_round_after_idle {
            return;
        }
        let status = self.0.live_capture.status();
        let replay_running = self.replay_running();
        let due = self.0.live_capture.with_state(|state| {
            auto_round_due(
                status.phase == LiveCapturePhase::Running && !replay_running,
                false,
                state.abyss.is_active(),
                state.is_game_paused(),
                !state.hits.is_empty(),
                self.0.live_capture.idle_elapsed(),
                config.auto_round_idle_seconds,
            )
        });
        if due && let Err(error) = self.archive_current_history_round() {
            log::warn!("automatic idle History archive failed: {error}");
        }
    }

    fn retry_pending_history_archives(&self) {
        let _archive_transaction = self
            .0
            .history
            .archive_transaction
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        let pending = {
            let mut pending = self
                .0
                .history
                .pending_archives
                .lock()
                .unwrap_or_else(|poison| poison.into_inner());
            std::mem::take(&mut *pending)
        };
        if pending.is_empty() {
            return;
        }

        let mut retry = VecDeque::new();
        for archive in pending {
            if let Err(error) = self.persist_history_archive(archive.clone()) {
                log::warn!("retrying a pending History archive failed: {error}");
                retry.push_back(archive);
            }
        }
        if retry.is_empty() {
            return;
        }
        let mut pending = self
            .0
            .history
            .pending_archives
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        retry.append(&mut *pending);
        *pending = retry;
    }

    pub(crate) fn with_history_transaction<T>(&self, action: impl FnOnce() -> T) -> T {
        let _guard = self
            .0
            .history
            .transaction
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        action()
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
        self.0.live_capture.restore_session(state, source);
    }

    pub(crate) fn character_data_snapshot(
        &self,
    ) -> Result<(CharacterDataProjection, u64), CharacterDataError> {
        let _guard = self
            .0
            .character_data_transaction
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        let projection = load_character_data(&self.0.character_data_path)?;
        let revision = self.0.character_data_revision.load(Ordering::Acquire);
        Ok((projection, revision))
    }

    pub(crate) fn save_character_data_record(
        &self,
        input: CharacterDataRecordInput,
    ) -> Result<(CharacterDataProjection, u64), CharacterDataError> {
        let _guard = self
            .0
            .character_data_transaction
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        let projection = save_character_data_record(&self.0.character_data_path, input)?;
        let revision = self
            .0
            .character_data_revision
            .fetch_add(1, Ordering::AcqRel)
            .wrapping_add(1);
        Ok((projection, revision))
    }

    pub(crate) fn encrypted_ini_snapshot(&self) -> EncryptedIniProjection {
        let runtime = self
            .0
            .encrypted_ini
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        encrypted_ini_projection(&runtime)
    }

    pub(crate) fn open_encrypted_ini(
        &self,
        path: PathBuf,
    ) -> Result<EncryptedIniProjection, EncryptedIniRuntimeError> {
        let mut runtime = self
            .0
            .encrypted_ini
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        let document = load_encrypted_ini_document(&path)?;
        runtime.generation = runtime.generation.wrapping_add(1);
        runtime.path = Some(path);
        runtime.document = Some(document);
        Ok(encrypted_ini_projection(&runtime))
    }

    pub(crate) fn reload_encrypted_ini(
        &self,
    ) -> Result<EncryptedIniProjection, EncryptedIniRuntimeError> {
        let mut runtime = self
            .0
            .encrypted_ini
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        let path = runtime
            .path
            .clone()
            .ok_or(EncryptedIniRuntimeError::NoFile)?;
        let document = load_encrypted_ini_document(&path)?;
        runtime.generation = runtime.generation.wrapping_add(1);
        runtime.document = Some(document);
        Ok(encrypted_ini_projection(&runtime))
    }

    pub(crate) fn save_encrypted_ini(
        &self,
        expected_generation: u64,
        plaintext: String,
        key: EncryptedIniKey,
    ) -> Result<(EncryptedIniProjection, EncryptedIniSaveOutcome), EncryptedIniRuntimeError> {
        let mut runtime = self
            .0
            .encrypted_ini
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        if runtime.generation != expected_generation {
            return Err(EncryptedIniRuntimeError::StaleGeneration);
        }
        let path = runtime
            .path
            .clone()
            .ok_or(EncryptedIniRuntimeError::NoFile)?;
        let document = runtime
            .document
            .as_mut()
            .ok_or(EncryptedIniRuntimeError::NoFile)?;
        let outcome = save_encrypted_ini_document(&path, document, plaintext, key)?;
        runtime.generation = runtime.generation.wrapping_add(1);
        Ok((encrypted_ini_projection(&runtime), outcome))
    }

    pub(crate) fn clear_encrypted_ini(&self) -> EncryptedIniProjection {
        let mut runtime = self
            .0
            .encrypted_ini
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        runtime.generation = runtime.generation.wrapping_add(1);
        runtime.path = None;
        runtime.document = None;
        encrypted_ini_projection(&runtime)
    }

    pub(crate) fn empty_curtain_snapshot(&self) -> InventorySnapshot {
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
    ) -> (
        Vec<nte_dps_tool::engine::model::EmptyCurtainItem>,
        Vec<nte_dps_tool::engine::model::EmptyCurtainCharacter>,
        Arc<EquipmentCatalog>,
    ) {
        let (items, characters) = self.with_main_presented_state(|state| {
            (
                state.empty_curtain.clone(),
                state.empty_curtain_characters.clone(),
            )
        });
        (items, characters, Arc::clone(&self.0.equipment_catalog))
    }

    pub(crate) fn equipment_catalog(&self) -> Arc<EquipmentCatalog> {
        Arc::clone(&self.0.equipment_catalog)
    }

    pub(crate) fn empty_curtain_operation(&self) -> EmptyCurtainOperationState {
        self.refresh_empty_curtain_operation();
        self.0
            .empty_curtain_operation
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
            .clone()
    }

    pub(crate) fn empty_curtain_revision(&self) -> (u64, u64, u64) {
        self.refresh_empty_curtain_operation();
        let (inventory, characters) = if self.main_processing_paused() {
            self.with_main_presented_state(|state| {
                (
                    state.empty_curtain_generation,
                    state.empty_curtain_characters_generation,
                )
            })
        } else {
            self.0.live_capture.inventory_revision()
        };
        (
            inventory,
            characters,
            self.0
                .empty_curtain_operation_revision
                .load(Ordering::Acquire),
        )
    }

    pub(crate) fn diagnostics_revision(&self) -> (u64, u64, u64) {
        let packet_generation = self
            .0
            .live_capture
            .with_packet_state(|revision, _, _| revision.generation);
        (
            self.0.live_capture.revision(),
            packet_generation,
            self.0.diagnostics_revision.load(Ordering::Acquire),
        )
    }

    pub(crate) fn diagnostics_input(&self) -> DiagnosticSnapshot {
        let config = self.ui_config();
        let status = self.0.live_capture.status();
        let replay_running = self.0.live_capture.replay_running();
        let raw_packet_count = self
            .0
            .live_capture
            .raw_capture_snapshot()
            .map_or(0, |raw| raw.packet_count as usize);
        let (parsed_packet_count, hit_count) = self
            .0
            .live_capture
            .with_state(|state| (state.packet_count, state.hits.len()));
        DiagnosticSnapshot {
            capture_running: matches!(
                status.phase,
                LiveCapturePhase::Starting | LiveCapturePhase::Running | LiveCapturePhase::Stopping
            ) && !replay_running,
            replay_running,
            active_capture_filter: self.0.live_capture.active_capture_filter(),
            raw_packet_count,
            parsed_packet_count,
            hit_count,
            dropped_history_archives: self.0.live_capture.dropped_history_archives(),
            include_incoming: true,
            server_damage_calibration: config.server_damage_calibration,
            last_diagnostic: status.issue.map(|issue| format!("{issue:?}")),
            manual_capture_device: config.manual_capture_device,
        }
    }

    pub(crate) fn diagnostics_report(&self) -> Option<DiagnosticRun> {
        self.0
            .diagnostics_report
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
            .clone()
    }

    pub(crate) fn store_diagnostics_report(&self, report: DiagnosticRun) {
        *self
            .0
            .diagnostics_report
            .lock()
            .unwrap_or_else(|poison| poison.into_inner()) = Some(report);
        self.0.diagnostics_revision.fetch_add(1, Ordering::AcqRel);
    }

    pub(crate) fn diagnostics_quality(&self) -> CaptureQualitySummary {
        self.0.live_capture.quality_summary()
    }

    pub(crate) fn diagnostics_raw_capture(
        &self,
    ) -> Option<nte_dps_tool::engine::capture::RawCaptureSnapshot> {
        self.0.live_capture.raw_capture_snapshot()
    }

    pub(crate) fn save_diagnostics_raw_capture(
        &self,
        path: &std::path::Path,
    ) -> Result<(u64, u64), String> {
        self.0.live_capture.save_last_raw_capture(path)
    }

    pub(crate) fn begin_replay_import(
        &self,
        replace_current: bool,
    ) -> Result<ReplayImportReservation, CoreError> {
        let mut reserved = self
            .0
            .replay_import_reserved
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        let active = matches!(
            self.capture_phase(),
            LiveCapturePhase::Starting | LiveCapturePhase::Running | LiveCapturePhase::Stopping
        ) || self.replay_running();
        if *reserved || (active && !replace_current) {
            return Err(CoreError::new(
                CoreErrorCode::CaptureAlreadyRunning,
                "capture or replay is already active",
            ));
        }
        if self.session_has_data() && !replace_current {
            return Err(CoreError::new(
                CoreErrorCode::CaptureAlreadyRunning,
                "replay import requires confirmation before replacing the current session",
            ));
        }
        *reserved = true;
        drop(reserved);
        Ok(ReplayImportReservation {
            state: self.clone(),
            active: true,
        })
    }

    fn request_diagnostics_replay(
        &self,
        kind: CaptureReplayKind,
        path: PathBuf,
    ) -> Result<(), CoreError> {
        let config = self.ui_config();
        let local_ip_hint = self
            .diagnostics_report()
            .and_then(|run| run.environment.local_ip)
            .and_then(|local_ip| local_ip.parse().ok());
        let result = self.0.live_capture.request_replay(
            kind,
            path,
            local_ip_hint,
            true,
            config.server_damage_calibration,
        );
        if result.is_ok() {
            self.return_main_presentation_to_live();
        }
        result
    }

    pub(crate) fn diagnostics_capture_export(&self) -> CaptureExportDocument {
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
        self.0.live_capture.with_state(|state| {
            CaptureExportDocument::snapshot(
                state,
                CaptureExportOptions {
                    filter: config.capture_filter,
                    include_incoming: true,
                    game_network,
                    dps_time_mode: DpsTimeBasis::from_subtract_time_stop(matches!(
                        config.dps_time_mode,
                        DpsTimeMode::TimeStopAdjusted
                    )),
                },
            )
        })
    }

    pub(crate) fn diagnostics_has_exportable_state(&self) -> bool {
        self.0.live_capture.with_state(|state| {
            !state.hits.is_empty() || !state.packets.is_empty() || !state.empty_curtain.is_empty()
        })
    }

    pub(crate) fn submit_empty_curtain_operation(
        &self,
        character: nte_dps_tool::engine::model::HtItemNetId,
        operation: ModsPluginOperation,
    ) -> Result<u64, ModsPluginSubmitError> {
        self.refresh_empty_curtain_operation();
        if self
            .0
            .empty_curtain_operation
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
            .request_id
            .is_some()
        {
            return Err(ModsPluginSubmitError::Busy);
        }
        let request_id = self
            .0
            .mods_plugin
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
            .submit(character, operation)?;
        *self
            .0
            .empty_curtain_operation
            .lock()
            .unwrap_or_else(|poison| poison.into_inner()) = EmptyCurtainOperationState {
            status: "pending",
            message_key: "Sending equipment request...",
            message_arguments: Vec::new(),
            request_id: Some(request_id),
        };
        self.0
            .empty_curtain_operation_revision
            .fetch_add(1, Ordering::AcqRel);
        Ok(request_id)
    }

    pub(crate) fn refresh_empty_curtain_operation(&self) {
        let response = match self
            .0
            .mods_plugin
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
            .try_recv()
        {
            Ok(response) => response,
            Err(ModsPluginReceiveError::Disconnected) => {
                let mut operation = self
                    .0
                    .empty_curtain_operation
                    .lock()
                    .unwrap_or_else(|poison| poison.into_inner());
                if operation.request_id.is_some() {
                    *operation = EmptyCurtainOperationState {
                        status: "error",
                        message_key: "Mod loader worker disconnected",
                        message_arguments: Vec::new(),
                        request_id: None,
                    };
                    self.0
                        .empty_curtain_operation_revision
                        .fetch_add(1, Ordering::AcqRel);
                }
                return;
            }
        };
        let Some(response) = response else {
            return;
        };
        let mut operation = self
            .0
            .empty_curtain_operation
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        if operation.request_id != Some(response.request_id) {
            return;
        }
        *operation = match response.status {
            Ok(0) => EmptyCurtainOperationState {
                status: "success",
                message_key: "Equipment RPC dispatched; waiting for game synchronization",
                message_arguments: Vec::new(),
                request_id: None,
            },
            Ok(1) => EmptyCurtainOperationState {
                status: "success",
                message_key: "Equipment request passed plugin dry-run validation",
                message_arguments: Vec::new(),
                request_id: None,
            },
            Ok(status) => EmptyCurtainOperationState {
                status: "error",
                message_key: "Mod loader rejected the request (status {})",
                message_arguments: vec![status.to_string()],
                request_id: None,
            },
            Err(error) => EmptyCurtainOperationState {
                status: "error",
                message_key: "Mod loader is unavailable: {}",
                message_arguments: vec![error],
                request_id: None,
            },
        };
        self.0
            .empty_curtain_operation_revision
            .fetch_add(1, Ordering::AcqRel);
    }

    pub(crate) fn timeline_projection(&self, scope: TimelineScope) -> TimelineProjection {
        let config = self.ui_config();
        let resources = self.0.live_capture.resources();
        self.with_main_presented_state(|state| {
            project_timeline(
                state,
                &resources.characters,
                TimelineProjectionOptions {
                    scope,
                    bucket_seconds: config.timeline_bucket_seconds,
                    subtract_time_stop: matches!(
                        config.dps_time_mode,
                        DpsTimeMode::TimeStopAdjusted
                    ),
                    language: config.language,
                },
            )
        })
    }

    pub(crate) fn packet_stream_revision(&self) -> PacketStreamRevision {
        if self.main_processing_paused()
            && let Some(paused) = self
                .0
                .presentation
                .paused
                .lock()
                .unwrap_or_else(|poison| poison.into_inner())
                .as_ref()
        {
            return paused.packet_revision;
        }
        self.0
            .live_capture
            .with_packet_state(|revision, _, _| revision)
    }

    pub(crate) fn packets_projection(
        &self,
        after: Option<PacketStreamRevision>,
    ) -> (PacketStreamRevision, bool, PacketsProjection) {
        if self.main_processing_paused()
            && let Some(paused) = self
                .0
                .presentation
                .paused
                .lock()
                .unwrap_or_else(|poison| poison.into_inner())
                .as_ref()
        {
            let revision = paused.packet_revision;
            let incremental = after
                .filter(|cursor| cursor.session_generation == revision.session_generation)
                .and_then(|cursor| {
                    project_packets_since(&paused.state, revision, 0, cursor.packet_generation)
                });
            return match incremental {
                Some(projection) => (revision, false, projection),
                None => (
                    revision,
                    true,
                    project_recent_packets(&paused.state, revision, 0),
                ),
            };
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

    pub(crate) fn skills_projection(&self, scope: SkillsScope) -> SkillsProjection {
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
    ) -> Result<bool, String> {
        self.update_ui_config(|config| {
            config.timeline_bucket_seconds = bucket_seconds;
            config.timeline_dps_view_mode = view_mode;
        })
    }

    pub(crate) fn bump_history_revision(&self) -> u64 {
        self.0.history.revision.fetch_add(1, Ordering::AcqRel) + 1
    }

    pub(crate) fn remember_deleted_history(&self, record: HistoryRecord) -> String {
        let sequence = self.0.history.undo_sequence.fetch_add(1, Ordering::AcqRel) + 1;
        let token = format!("history-undo-{sequence}");
        *self
            .0
            .history
            .undo
            .lock()
            .unwrap_or_else(|poison| poison.into_inner()) = Some(HistoryUndoEntry {
            token: token.clone(),
            record,
            expires_at: Instant::now() + HISTORY_UNDO_WINDOW,
        });
        token
    }

    pub(crate) fn take_deleted_history(&self, token: &str) -> Option<HistoryRecord> {
        let mut undo = self
            .0
            .history
            .undo
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        let matches = undo
            .as_ref()
            .is_some_and(|entry| entry.token == token && Instant::now() <= entry.expires_at);
        matches.then(|| undo.take().expect("checked history undo entry").record)
    }

    pub(crate) fn set_history_prediction_team(&self, team: TeamDps, upper: bool) {
        let mut imported = self
            .0
            .imported_teams
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        if upper {
            imported.0 = Some(team);
        } else {
            imported.1 = Some(team);
        }
        drop(imported);
        self.bump_settings_revision();
    }

    fn ui_config(&self) -> UiConfig {
        self.0
            .ui_config
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
            .clone()
    }

    fn update_ui_config(&self, update: impl FnOnce(&mut UiConfig)) -> Result<bool, String> {
        let _transaction = self
            .0
            .config_transaction
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        let previous = self.ui_config();
        let mut candidate = previous.clone();
        update(&mut candidate);
        candidate = candidate.sanitized();
        if candidate == previous {
            return Ok(false);
        }
        config::save(&self.0.config_path, &candidate)?;
        *self
            .0
            .ui_config
            .lock()
            .unwrap_or_else(|poison| poison.into_inner()) = candidate;
        self.0.presentation.revision.fetch_add(1, Ordering::AcqRel);
        self.0.settings_revision.fetch_add(1, Ordering::AcqRel);
        Ok(true)
    }

    pub(crate) fn stream_revision(&self) -> StreamRevision {
        StreamRevision {
            capture: if self.main_processing_paused() {
                0
            } else {
                self.0.live_capture.revision()
            },
            presentation: self.0.presentation.revision.load(Ordering::Acquire),
        }
    }

    pub(crate) fn settings_revision(&self) -> u64 {
        self.0.settings_revision.load(Ordering::Acquire)
    }

    fn with_main_presented_state<T>(&self, project: impl FnOnce(&CombatState) -> T) -> T {
        if self.main_selected_round_id().is_some() {
            let selected = self
                .0
                .presentation
                .selected_round
                .lock()
                .unwrap_or_else(|poison| poison.into_inner());
            if let Some(selected) = selected.as_ref() {
                return project(selected.state.as_ref());
            }
        }
        if self.main_processing_paused() {
            let paused = self
                .0
                .presentation
                .paused
                .lock()
                .unwrap_or_else(|poison| poison.into_inner());
            if let Some(paused) = paused.as_ref() {
                return project(paused.state.as_ref());
            }
        }
        self.0.live_capture.with_state(project)
    }

    fn return_main_presentation_to_live(&self) {
        self.0
            .presentation
            .selected_round
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
            .take();
        self.0.presentation.selected_outgoing_revision.store(
            self.0.live_capture.outgoing_hit_revision(),
            Ordering::Release,
        );
        self.0
            .presentation
            .paused
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
            .take();
        self.0
            .presentation
            .processing_paused
            .store(false, Ordering::Release);
        *self
            .0
            .presentation
            .selected_abyss_half
            .lock()
            .unwrap_or_else(|poison| poison.into_inner()) = None;
        *self
            .0
            .presentation
            .observed_abyss_half
            .lock()
            .unwrap_or_else(|poison| poison.into_inner()) = None;
        self.bump_main_dps_revision();
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
    ) -> Result<bool, String> {
        self.update_ui_config(|config| match region {
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
    ) -> Result<bool, String> {
        self.update_ui_config(|config| config.mod_studio_loading_method = method)
    }

    pub(crate) fn mod_studio_risk_acknowledged(&self) -> bool {
        self.ui_config().mod_studio_risk_acknowledged
    }

    pub(crate) fn acknowledge_mod_studio_risk(&self) -> Result<bool, String> {
        self.update_ui_config(|config| config.mod_studio_risk_acknowledged = true)
    }

    pub(crate) fn uptime_ms(&self) -> u128 {
        self.0.started_at.elapsed().as_millis()
    }

    fn hud_config(&self) -> HudConfig {
        self.0
            .ui_config
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
            .hud
            .clone()
    }

    pub(crate) fn begin_stream(
        &self,
        owner_window: String,
        subscription_id: String,
    ) -> Arc<AtomicBool> {
        let stop = Arc::new(AtomicBool::new(false));
        let replaced = self
            .0
            .streams
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
            .insert(
                subscription_id,
                StreamEntry {
                    owner_window,
                    stop: Arc::clone(&stop),
                },
            );

        if let Some(replaced) = replaced {
            replaced.stop.store(true, Ordering::Release);
        }

        stop
    }

    pub(crate) fn stop_stream(&self, subscription_id: &str) {
        if let Some(entry) = self
            .0
            .streams
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
            .remove(subscription_id)
        {
            entry.stop.store(true, Ordering::Release);
        }
    }

    pub(crate) fn stop_streams_for_window(&self, owner_window: &str) -> usize {
        let mut streams = self
            .0
            .streams
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        let owned = streams
            .iter()
            .filter(|(_, entry)| entry.owner_window == owner_window)
            .map(|(subscription_id, _)| subscription_id.clone())
            .collect::<Vec<_>>();
        for subscription_id in &owned {
            if let Some(entry) = streams.remove(subscription_id) {
                entry.stop.store(true, Ordering::Release);
            }
        }
        owned.len()
    }

    pub(crate) fn finish_stream(&self, subscription_id: &str, stop: &Arc<AtomicBool>) {
        let mut streams = self
            .0
            .streams
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        let is_current = streams
            .get(subscription_id)
            .is_some_and(|current| Arc::ptr_eq(&current.stop, stop));

        if is_current {
            streams.remove(subscription_id);
        }
    }
}

fn encrypted_ini_projection(runtime: &EncryptedIniRuntimeState) -> EncryptedIniProjection {
    let document = runtime.document.as_ref();
    EncryptedIniProjection {
        generation: runtime.generation,
        display_path: runtime.path.as_ref().map(|path| path.display().to_string()),
        file_name: runtime
            .path
            .as_ref()
            .and_then(|path| path.file_name())
            .map(|name| name.to_string_lossy().into_owned()),
        key: document.map_or(EncryptedIniKey::Global, EncryptedIniDocument::key),
        plaintext: document
            .map(EncryptedIniDocument::plaintext)
            .unwrap_or_default()
            .to_owned(),
        encrypted_line_count: document.map_or(0, EncryptedIniDocument::encrypted_line_count),
    }
}

#[cfg(test)]
mod tests {
    use std::{fs, time::SystemTime};

    use super::*;
    use nte_dps_tool::core::hud::{HudDataState, HudModuleSnapshot};

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

    #[test]
    fn replacing_subscription_stops_previous_stream() {
        let state = AppState::default();
        let previous = state.begin_stream("hud".to_owned(), "technical".to_owned());
        let current = state.begin_stream("hud".to_owned(), "technical".to_owned());

        assert!(previous.load(Ordering::Acquire));
        assert!(!current.load(Ordering::Acquire));
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

        let (items, characters, _) = state.empty_curtain_data_snapshot();
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
        live_capture.restore_session(first, CaptureQualitySource::Live);
        let state = AppState::new_with_config_path(
            UiConfig::default(),
            live_capture.clone(),
            config_path.clone(),
        );

        state.set_main_processing_paused(true);
        let paused_revision = state.main_dps_stream_revision();
        let mut second = CombatState::default();
        second.push_hit(test_hit(300.0));
        second.push_hit(test_hit(25.0));
        live_capture.restore_session(second, CaptureQualitySource::Live);

        assert_eq!(
            state.with_main_presented_state(|presented| presented.total_damage),
            125.0
        );
        assert!(state.main_paused_event_counts().0 > 0);
        assert_ne!(state.main_dps_stream_revision(), paused_revision);

        state.set_main_processing_paused(false);
        assert_eq!(
            state.with_main_presented_state(|presented| presented.total_damage),
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
        live_capture.restore_session(replay, CaptureQualitySource::JsonReplay);
        let state = AppState::new_with_config_path(
            UiConfig::default(),
            live_capture.clone(),
            config_path.clone(),
        );

        let prepared = state
            .prepare_current_history_archive()
            .expect("current replay archive");
        assert_eq!(
            prepared.summary.quality.source,
            CaptureQualitySource::JsonReplay
        );
        assert_eq!(
            state.archive_current_history_round_with(|_, _| Err("disk full".to_owned())),
            Ok(true)
        );
        assert!(live_capture.with_state(|current| current.hits.is_empty()));

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
        drop(pending);

        let mut next = CombatState::default();
        next.push_hit(test_hit(99.0));
        live_capture.restore_session(next, CaptureQualitySource::Live);
        assert_eq!(
            live_capture.with_state(|current| current.total_damage),
            99.0
        );
        let retry_template = state
            .prepare_current_history_archive()
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
            live_capture.with_state(|current| current.total_damage),
            99.0,
            "full retry queue must preserve the current live round"
        );

        fs::remove_dir_all(config_path.parent().expect("config parent"))
            .expect("remove temporary config");
    }

    #[test]
    fn reset_undo_restores_session_and_rejects_a_wrong_token_without_consuming_it() {
        let config_path = temporary_config_path("session_reset_undo");
        let live_capture = LiveCaptureService::new(LiveCaptureResources::default());
        let mut previous = CombatState::default();
        previous.push_hit(test_hit(222.0));
        live_capture.restore_session(previous, CaptureQualitySource::Live);
        let state =
            AppState::new_with_config_path(UiConfig::default(), live_capture, config_path.clone());

        state.set_main_processing_paused(true);
        let token = state.reset_session_with_undo().expect("reset undo token");
        assert!(!state.session_has_data());
        assert!(!state.main_processing_paused());
        assert_eq!(
            state.undo_session_reset("wrong-token"),
            Err(SessionUndoError::Missing)
        );
        state
            .undo_session_reset(&token)
            .expect("restore reset session");
        assert_eq!(
            state.with_main_presented_state(|presented| presented.total_damage),
            222.0
        );

        fs::remove_dir_all(config_path.parent().expect("config parent"))
            .expect("remove temporary config");
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
        assert_eq!(duplicate_error.code, CoreErrorCode::CaptureAlreadyRunning);
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
    fn capture_file_refresh_advances_settings_generation() {
        let state = AppState::default();
        let generation = state.settings_revision();

        state.refresh_capture_file_stats();

        assert!(state.settings_revision() > generation);
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
            Err(EncryptedIniRuntimeError::StaleGeneration)
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
        assert_eq!(state.clear_encrypted_ini().generation, 3);

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

        assert!(state.import_abyss_team(export, true));
        assert_eq!(
            state.imported_abyss_teams().0.as_ref().map(|team| team.dps),
            Some(12_345.0)
        );
        assert!(state.imported_abyss_teams().1.is_none());

        state.swap_abyss_teams();
        assert!(state.imported_abyss_teams().0.is_none());
        assert_eq!(
            state.imported_abyss_teams().1.as_ref().map(|team| team.dps),
            Some(12_345.0)
        );

        state.clear_abyss_team(false);
        let (upper, lower) = state.imported_abyss_teams();
        assert!(upper.is_none());
        assert!(lower.is_none());
    }

    #[test]
    fn finishing_replaced_stream_keeps_current_registration() {
        let state = AppState::default();
        let previous = state.begin_stream("hud".to_owned(), "technical".to_owned());
        let current = state.begin_stream("hud".to_owned(), "technical".to_owned());

        state.finish_stream("technical", &previous);
        state.stop_stream("technical");

        assert!(current.load(Ordering::Acquire));
    }

    #[test]
    fn destroying_owner_window_stops_only_its_streams_and_clears_registry_entries() {
        let state = AppState::default();
        let hud_first = state.begin_stream("hud".to_owned(), "hud:first".to_owned());
        let hud_second = state.begin_stream("hud".to_owned(), "hud:second".to_owned());
        let console = state.begin_stream("console".to_owned(), "console:first".to_owned());

        assert_eq!(state.stop_streams_for_window("hud"), 2);
        assert!(hud_first.load(Ordering::Acquire));
        assert!(hud_second.load(Ordering::Acquire));
        assert!(!console.load(Ordering::Acquire));
        assert_eq!(state.stop_streams_for_window("hud"), 0);

        state.stop_stream("console:first");
        assert!(console.load(Ordering::Acquire));
    }

    #[test]
    fn editor_snapshot_uses_rust_preview_and_passthrough_snapshot_is_empty() {
        let state = AppState::default();

        assert_eq!(state.snapshot().hud.data_state, HudDataState::Preview);

        state.set_passthrough(true);

        assert_eq!(state.snapshot().hud.data_state, HudDataState::Empty);
    }

    #[test]
    fn initial_window_and_hud_projection_follow_loaded_config() {
        let mut config = UiConfig {
            always_on_top: false,
            passthrough_hotkey: PassthroughHotkey::F8,
            ..UiConfig::default()
        };
        config.hud.width = 512;
        config.hud.show_total_damage = false;

        let state = AppState::new(
            config,
            LiveCaptureService::new(LiveCaptureResources::default()),
        );
        let snapshot = state.snapshot();

        assert!(!state.always_on_top());
        assert_eq!(state.passthrough_hotkey(), PassthroughHotkey::F8);
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
        state.finish_update_check(vec![available]);
        let snapshot = state.settings_snapshot();
        assert_eq!(snapshot.updates.status, "available");
        assert_eq!(snapshot.updates.available[0].component, "app");

        state
            .begin_update_download(UpdateComponent::App)
            .expect("begin update download");
        state.update_download_progress(UpdateComponent::App, 512, 1_024);
        let snapshot = state.settings_snapshot();
        assert_eq!(snapshot.updates.status, "downloading");
        assert_eq!(snapshot.updates.downloaded_bytes, "512");

        state.finish_update_download(PreparedUpdate::App {
            version: "0.4.0".parse().expect("semantic version"),
            transaction_path: PathBuf::from("transaction.json"),
            updater_path: PathBuf::from("nte-updater.exe"),
        });
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
        assert!(state.snapshot().hud.config.show_mini_timeline);
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
                    45,
                    DpsTimeMode::RealTime,
                    PassthroughHotkey::Insert,
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

        let snapshot = state.settings_snapshot();
        assert_eq!(snapshot.interface.language, "ja");
        assert!(snapshot.interface.dark_mode);
        assert_eq!(snapshot.interface.theme_preset, "tactical");
        assert_eq!(snapshot.capture.bpf_filter, "udp port 30196");
        assert_eq!(
            snapshot.capture.manual_capture_device.as_deref(),
            Some("capture-device")
        );
        assert!(snapshot.capture.separate_reaction_damage);
        assert_eq!(snapshot.capture.auto_round_idle_seconds, 45);
        assert_eq!(
            snapshot.hotkeys.bindings[0]
                .binding
                .as_ref()
                .map(|binding| binding.key.as_str()),
            Some("F8")
        );

        let saved: UiConfig =
            serde_json::from_str(&fs::read_to_string(&config_path).expect("saved UI config"))
                .expect("valid saved UI config");
        assert_eq!(saved.language, Language::Japanese);
        assert_eq!(saved.theme_preset, ThemePreset::Tactical);
        assert_eq!(saved.accent, AccentColor::Orange);
        assert_eq!(saved.capture_filter, "udp port 30196");
        assert!(saved.reduce_motion);
        assert_eq!(
            saved.manual_capture_device.as_deref(),
            Some("capture-device")
        );
        assert_eq!(saved.dps_time_mode, DpsTimeMode::RealTime);

        let restored = AppState::new_with_config_path(
            saved,
            LiveCaptureService::new(LiveCaptureResources::default()),
            config_path.clone(),
        );
        assert_eq!(
            restored.settings_snapshot().capture.bpf_filter,
            "udp port 30196"
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
            state.snapshot().hud.config.module_order,
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
            state.snapshot().hud.config.width,
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
        assert!(!state.snapshot().hud.config.show_title);
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
        let record = HistoryRecord {
            id: "history-record".to_owned(),
            ..Default::default()
        };

        let token = state.remember_deleted_history(record);

        assert!(!token.contains("history-record"));
        assert_eq!(
            state
                .take_deleted_history(&token)
                .expect("active undo record")
                .id,
            "history-record"
        );
        assert!(state.take_deleted_history(&token).is_none());
    }

    #[test]
    fn stale_island_dismiss_keeps_the_newer_notice() {
        let state = AppState::default();
        let stale_id = state.publish_island_notice("info", "First notice", Vec::new(), None);
        let current_id = state.publish_island_notice("success", "Second notice", Vec::new(), None);

        assert!(!state.dismiss_island_notice(&stale_id));
        assert_eq!(
            state.island_notice().map(|notice| notice.id),
            Some(current_id.clone())
        );
        assert!(state.dismiss_island_notice(&current_id));
        assert!(state.island_notice().is_none());
    }
}
