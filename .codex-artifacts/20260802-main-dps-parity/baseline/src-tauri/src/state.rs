use std::{
    collections::HashMap,
    collections::HashSet,
    path::PathBuf,
    sync::{
        Arc, Mutex, MutexGuard,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use nte_dps_tool::{
    core::{
        CoreError,
        capture::{
            CaptureControllerOptions, CaptureDeviceSelector, CaptureProfile, RawCaptureMode,
            enumerate_devices,
        },
        character_data::{
            CharacterDataError, CharacterDataProjection, CharacterDataRecordInput,
            load_character_data, save_character_data_record,
        },
        diagnostics::{DiagnosticRun, DiagnosticSnapshot},
        encrypted_ini::{
            EncryptedIniDocument, EncryptedIniError, EncryptedIniKey, EncryptedIniSaveOutcome,
            load_encrypted_ini_document, save_encrypted_ini_document,
        },
        history::{PreparedHistoryArchive, auto_round_due, prepare_history_archive},
        hud::{HudProjectionOptions, HudSnapshot, project_hud},
        live_capture::{
            CaptureReplayKind, LiveCapturePhase, LiveCaptureResources, LiveCaptureService,
            LiveCaptureStatus,
        },
        mod_studio::ModStudioWorkspaceService,
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
    platform::mods_plugin::{ModsPluginClient, ModsPluginOperation, ModsPluginSubmitError},
    storage::{
        capture_logs::{ClearOutcome, clear_capture_logs, scan_capture_logs},
        config::{
            self, AccentColor, DpsTimeMode, GlobalHotkeys, HudConfig, HudModule, PassthroughHotkey,
            ThemePreset, TimelineDpsViewMode, UiConfig, UiDensity,
            sanitize_timeline_bucket_seconds,
        },
        history::{
            HistoryCombatDetails, HistoryRecord, load_history, save_summary,
            save_summary_with_details,
        },
        i18n::Language,
        paths::{capture_log_dir, software_dir},
        update::PreparedUpdate,
    },
};

use crate::{
    contract::{
        HudWindowSnapshot, TECHNICAL_CONTRACT_VERSION, TechnicalSnapshot,
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
const HUD_SUMMARY_HEIGHT: u16 = 64;
const HUD_CHARACTERS_HEIGHT: u16 = 116;
const HUD_OPTIONAL_TITLE_HEIGHT: u16 = 22;
const HUD_OPTIONAL_STATUS_HEIGHT: u16 = 22;
const HUD_MINI_TIMELINE_HEIGHT: u16 = 42;

#[derive(Clone)]
pub(crate) struct AppState(Arc<AppStateInner>);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct StreamRevision {
    capture: u64,
    presentation: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct MainDpsStreamRevision {
    pub(crate) capture: u64,
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

#[derive(Default)]
struct MainRoundCache {
    revision: Option<u64>,
    records: Vec<HistoryRecord>,
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
    presentation_revision: AtomicU64,
    settings_revision: AtomicU64,
    history_revision: AtomicU64,
    main_dps_revision: AtomicU64,
    main_processing_paused: AtomicBool,
    diagnostics_revision: AtomicU64,
    history_undo_sequence: AtomicU64,
    character_data_revision: AtomicU64,
    empty_curtain_operation_revision: AtomicU64,
    streams: Mutex<HashMap<String, Arc<AtomicBool>>>,
    live_capture: LiveCaptureService,
    mod_studio: ModStudioWorkspaceService,
    equipment_catalog: Arc<EquipmentCatalog>,
    mods_plugin: Mutex<ModsPluginClient>,
    empty_curtain_operation: Mutex<EmptyCurtainOperationState>,
    capture_devices: Mutex<Vec<CaptureDeviceSnapshot>>,
    imported_teams: Mutex<(Option<TeamDps>, Option<TeamDps>)>,
    update_runtime: Mutex<UpdateRuntimeState>,
    selected_abyss_half: Mutex<Option<AbyssHalf>>,
    main_selected_round_id: Mutex<Option<String>>,
    main_frozen_readout: Mutex<Option<MainDpsReadout>>,
    main_round_cache: Mutex<MainRoundCache>,
    passthrough_transaction: Mutex<()>,
    always_on_top_transaction: Mutex<()>,
    config_transaction: Mutex<()>,
    history_transaction: Mutex<()>,
    character_data_transaction: Mutex<()>,
    encrypted_ini: Mutex<EncryptedIniRuntimeState>,
    diagnostics_report: Mutex<Option<DiagnosticRun>>,
    history_undo: Mutex<Option<HistoryUndoEntry>>,
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

pub(crate) const HISTORY_UNDO_WINDOW: Duration = Duration::from_secs(5);

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
            always_on_top: AtomicBool::new(config.always_on_top),
            presentation_revision: AtomicU64::new(0),
            settings_revision: AtomicU64::new(0),
            history_revision: AtomicU64::new(0),
            main_dps_revision: AtomicU64::new(0),
            main_processing_paused: AtomicBool::new(false),
            diagnostics_revision: AtomicU64::new(0),
            history_undo_sequence: AtomicU64::new(0),
            character_data_revision: AtomicU64::new(0),
            empty_curtain_operation_revision: AtomicU64::new(0),
            streams: Mutex::new(HashMap::new()),
            live_capture,
            mod_studio: ModStudioWorkspaceService::default(),
            equipment_catalog: Arc::new(equipment_catalog),
            mods_plugin: Mutex::new(ModsPluginClient::new()),
            empty_curtain_operation: Mutex::new(EmptyCurtainOperationState::default()),
            capture_devices: Mutex::new(capture_devices),
            imported_teams: Mutex::new((None, None)),
            update_runtime: Mutex::new(UpdateRuntimeState::default()),
            selected_abyss_half: Mutex::new(None),
            main_selected_round_id: Mutex::new(None),
            main_frozen_readout: Mutex::new(None),
            main_round_cache: Mutex::new(MainRoundCache::default()),
            passthrough_transaction: Mutex::new(()),
            always_on_top_transaction: Mutex::new(()),
            config_transaction: Mutex::new(()),
            history_transaction: Mutex::new(()),
            character_data_transaction: Mutex::new(()),
            encrypted_ini: Mutex::new(EncryptedIniRuntimeState::default()),
            diagnostics_report: Mutex::new(None),
            history_undo: Mutex::new(None),
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
            hud: self.0.live_capture.with_state(|state| {
                let selected_abyss_half = *self
                    .0
                    .selected_abyss_half
                    .lock()
                    .expect("HUD abyss selection lock poisoned");
                project_hud(
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
                )
            }),
        }
    }

    pub(crate) fn next_sequence(&self) -> u64 {
        self.0.sequence.fetch_add(1, Ordering::Relaxed) + 1
    }

    pub(crate) fn ui_config_snapshot(&self) -> UiConfig {
        self.ui_config()
    }

    pub(crate) fn live_capture_status(&self) -> LiveCaptureStatus {
        self.0.live_capture.status()
    }

    pub(crate) fn replay_running(&self) -> bool {
        self.0.live_capture.replay_running()
    }

    pub(crate) fn main_processing_paused(&self) -> bool {
        self.0.main_processing_paused.load(Ordering::Acquire)
    }

    pub(crate) fn set_main_processing_paused(&self, paused: bool) {
        if self.main_processing_paused() == paused {
            return;
        }
        if paused {
            let frozen = self
                .0
                .live_capture
                .with_state(|state| self.project_main_readout(state));
            *self
                .0
                .main_frozen_readout
                .lock()
                .expect("main DPS frozen readout lock poisoned") = Some(frozen);
        } else {
            self.0
                .main_frozen_readout
                .lock()
                .expect("main DPS frozen readout lock poisoned")
                .take();
        }
        self.0
            .main_processing_paused
            .store(paused, Ordering::Release);
        self.bump_main_dps_revision();
    }

    pub(crate) fn main_selected_round_id(&self) -> Option<String> {
        self.0
            .main_selected_round_id
            .lock()
            .expect("main DPS selected round lock poisoned")
            .clone()
    }

    pub(crate) fn set_main_selected_round_id(&self, record_id: Option<String>) -> Result<(), ()> {
        if let Some(id) = record_id.as_deref()
            && !self
                .main_round_records()
                .iter()
                .any(|record| record.id == id && record.details.is_some())
        {
            return Err(());
        }
        let mut selected = self
            .0
            .main_selected_round_id
            .lock()
            .expect("main DPS selected round lock poisoned");
        if *selected == record_id {
            return Ok(());
        }
        *selected = record_id;
        drop(selected);
        self.bump_main_dps_revision();
        Ok(())
    }

    pub(crate) fn main_round_records(&self) -> Vec<HistoryRecord> {
        let revision = self.history_revision();
        let mut cache = self
            .0
            .main_round_cache
            .lock()
            .expect("main DPS round cache lock poisoned");
        if cache.revision != Some(revision) {
            cache.records = load_history().records;
            cache.revision = Some(revision);
        }
        let records = cache.records.clone();
        drop(cache);
        let stale_selection = self
            .main_selected_round_id()
            .is_some_and(|id| !records.iter().any(|record| record.id == id));
        if stale_selection {
            *self
                .0
                .main_selected_round_id
                .lock()
                .expect("main DPS selected round lock poisoned") = None;
            self.bump_main_dps_revision();
        }
        records
    }

    pub(crate) fn main_dps_readout(
        &self,
        rounds: &[HistoryRecord],
        selected_round_id: Option<&str>,
    ) -> MainDpsReadout {
        if let Some(record_id) = selected_round_id
            && let Some(state) = rounds
                .iter()
                .find(|record| record.id == record_id)
                .and_then(|record| record.details.as_ref())
                .map(HistoryCombatDetails::to_combat_state)
        {
            return self.project_main_readout(&state);
        }
        if self.main_processing_paused()
            && let Some(readout) = self
                .0
                .main_frozen_readout
                .lock()
                .expect("main DPS frozen readout lock poisoned")
                .clone()
        {
            return readout;
        }
        self.0
            .live_capture
            .with_state(|state| self.project_main_readout(state))
    }

    pub(crate) fn set_main_selected_abyss_half(&self, half: Option<AbyssHalf>) {
        let mut selected = self
            .0
            .selected_abyss_half
            .lock()
            .expect("main DPS abyss selection lock poisoned");
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
        MainDpsStreamRevision {
            capture: if self.main_processing_paused() || selected_history {
                0
            } else {
                self.0.live_capture.revision()
            },
            presentation: self.0.presentation_revision.load(Ordering::Acquire),
            history: self.history_revision(),
            main: self.0.main_dps_revision.load(Ordering::Acquire),
        }
    }

    fn project_main_readout(&self, state: &CombatState) -> MainDpsReadout {
        let config = self.ui_config();
        let selected_abyss_half = *self
            .0
            .selected_abyss_half
            .lock()
            .expect("main DPS abyss selection lock poisoned");
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

    fn bump_main_dps_revision(&self) -> u64 {
        self.0.main_dps_revision.fetch_add(1, Ordering::AcqRel) + 1
    }

    pub(crate) fn settings_snapshot(&self) -> SettingsSnapshot {
        let generation = self.settings_revision();
        let config = self.ui_config();
        let devices = self
            .0
            .capture_devices
            .lock()
            .expect("capture device cache lock poisoned")
            .clone();
        let (upper_imported, lower_imported) = {
            let imported = self
                .0
                .imported_teams
                .lock()
                .expect("imported team lock poisoned");
            (imported.0.is_some(), imported.1.is_some())
        };
        let update = self
            .0
            .update_runtime
            .lock()
            .expect("update runtime lock poisoned")
            .clone();
        let install_blocked_message_key = self.install_blocked_message_key_for(&update);
        let updates = UpdateSettingsSnapshot::from_runtime(
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
        );
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

    pub(crate) fn request_capture_start(&self) -> Result<(), CoreError> {
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
            packet_emission: PacketEmissionMode::FullDebug,
        })
    }

    pub(crate) fn request_capture_stop(&self) -> Result<(), CoreError> {
        self.0.live_capture.request_stop()
    }

    pub(crate) fn capture_phase(&self) -> LiveCapturePhase {
        self.0.live_capture.status().phase
    }

    pub(crate) fn passthrough(&self) -> bool {
        self.0.passthrough.load(Ordering::Acquire)
    }

    pub(crate) fn set_passthrough(&self, enabled: bool) {
        if self.0.passthrough.swap(enabled, Ordering::AcqRel) != enabled {
            self.0.presentation_revision.fetch_add(1, Ordering::AcqRel);
        }
    }

    pub(crate) fn passthrough_hotkey(&self) -> PassthroughHotkey {
        self.0
            .ui_config
            .lock()
            .expect("UI config lock poisoned")
            .passthrough_hotkey
    }

    pub(crate) fn global_hotkeys(&self) -> GlobalHotkeys {
        self.0
            .ui_config
            .lock()
            .expect("UI config lock poisoned")
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
            .expect("passthrough transaction lock poisoned")
    }

    pub(crate) fn always_on_top(&self) -> bool {
        self.0.always_on_top.load(Ordering::Acquire)
    }

    pub(crate) fn lock_always_on_top_transaction(&self) -> MutexGuard<'_, ()> {
        self.0
            .always_on_top_transaction
            .lock()
            .expect("always-on-top transaction lock poisoned")
    }

    pub(crate) fn hud_width(&self) -> u16 {
        self.hud_config().width
    }

    pub(crate) fn hud_window_position(&self) -> Option<[i32; 2]> {
        self.0
            .ui_config
            .lock()
            .expect("UI config lock poisoned")
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
            content_height.max(HUD_EDITOR_MIN_HEIGHT)
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
        theme_preset: ThemePreset,
        accent: AccentColor,
        density: UiDensity,
        reduce_motion: bool,
        island_notifications: bool,
        island_offset_x: f32,
    ) -> Result<bool, String> {
        self.update_ui_config(|config| {
            config.language = language;
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
            .expect("UI config lock poisoned")
            .auto_check_updates
    }

    pub(crate) fn auto_download_updates(&self) -> bool {
        self.0
            .ui_config
            .lock()
            .expect("UI config lock poisoned")
            .auto_download_updates
    }

    pub(crate) fn begin_update_check(&self) -> Result<(), UpdateActionError> {
        let mut update = self
            .0
            .update_runtime
            .lock()
            .expect("update runtime lock poisoned");
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
            .expect("update runtime lock poisoned");
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
            .expect("update runtime lock poisoned");
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
            .expect("update runtime lock poisoned");
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
            .expect("update runtime lock poisoned");
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
            .expect("update runtime lock poisoned");
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
            .expect("update runtime lock poisoned");
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
            .expect("update runtime lock poisoned");
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
            .expect("update runtime lock poisoned")
            .clone();
        self.install_blocked_message_key_for(&update)
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
            .expect("update runtime lock poisoned");
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
            .expect("capture device cache lock poisoned") = devices;
        self.0.settings_revision.fetch_add(1, Ordering::AcqRel);
        Ok(())
    }

    pub(crate) fn clear_capture_files(&self) -> ClearOutcome {
        clear_capture_logs(&capture_log_dir())
    }

    pub(crate) fn reset_session(&self) {
        match self.archive_current_history_round() {
            Ok(true) => {}
            Ok(false) => self.0.live_capture.reset_session(),
            Err(error) => log::warn!("manual History archive failed; current round kept: {error}"),
        }
    }

    pub(crate) fn import_team_data(&self, export: TeamDpsExport) {
        let mut imported = self
            .0
            .imported_teams
            .lock()
            .expect("imported team lock poisoned");
        let fallback = export.single;
        imported.0 = export.upper.or_else(|| fallback.clone());
        imported.1 = export.lower.or(fallback);
        self.0.settings_revision.fetch_add(1, Ordering::AcqRel);
    }

    pub(crate) fn imported_abyss_teams(&self) -> (Option<TeamDps>, Option<TeamDps>) {
        self.0
            .imported_teams
            .lock()
            .expect("imported team lock poisoned")
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
            .expect("imported team lock poisoned");
        if upper {
            imported.0 = Some(team);
        } else {
            imported.1 = Some(team);
        }
        self.0.settings_revision.fetch_add(1, Ordering::AcqRel);
        true
    }

    pub(crate) fn clear_abyss_team(&self, upper: bool) {
        let mut imported = self
            .0
            .imported_teams
            .lock()
            .expect("imported team lock poisoned");
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
            .expect("imported team lock poisoned");
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
            .expect("imported team lock poisoned");
        self.0.live_capture.with_state(|state| {
            nte_dps_tool::core::team_data::export_team_data(
                state,
                matches!(config.dps_time_mode, DpsTimeMode::TimeStopAdjusted),
                config.separate_reaction_damage,
                imported.0.clone(),
                imported.1.clone(),
            )
        })
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
            .expect("UI config transaction lock poisoned");
        let mut candidate = self.ui_config();
        if candidate.hud_window_position == Some(position) {
            return Ok(false);
        }
        candidate.hud_window_position = Some(position);
        config::save(&self.0.config_path, &candidate)?;
        *self.0.ui_config.lock().expect("UI config lock poisoned") = candidate;
        Ok(true)
    }

    fn update_hud_config(&self, update: impl FnOnce(&mut HudConfig)) -> Result<bool, String> {
        self.update_ui_config(|config| update(&mut config.hud))
    }

    pub(crate) fn set_always_on_top(&self, enabled: bool) -> Result<bool, String> {
        let changed = self.update_ui_config(|config| config.always_on_top = enabled)?;
        if self.0.always_on_top.swap(enabled, Ordering::AcqRel) != enabled {
            self.0.presentation_revision.fetch_add(1, Ordering::AcqRel);
        }
        Ok(changed)
    }

    pub(crate) fn prepare_current_history_archive(&self) -> Option<PreparedHistoryArchive> {
        let config = self.ui_config();
        self.0.live_capture.with_state(|state| {
            prepare_history_archive(
                state,
                CaptureQualitySource::Live,
                DpsTimeBasis::from_subtract_time_stop(matches!(
                    config.dps_time_mode,
                    DpsTimeMode::TimeStopAdjusted
                )),
                config.separate_reaction_damage,
            )
        })
    }

    fn prepare_history_details(
        &self,
        details: HistoryCombatDetails,
    ) -> Option<PreparedHistoryArchive> {
        let config = self.ui_config();
        let state = details.to_combat_state();
        state
            .session_summary(
                CaptureQualitySource::Live,
                DpsTimeBasis::from_subtract_time_stop(matches!(
                    config.dps_time_mode,
                    DpsTimeMode::TimeStopAdjusted
                )),
                config.separate_reaction_damage,
            )
            .map(|summary| PreparedHistoryArchive {
                summary,
                details: Some(details),
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
        let config = self.ui_config();
        let result = self.0.live_capture.archive_and_reset(
            |state| {
                prepare_history_archive(
                    state,
                    CaptureQualitySource::Live,
                    DpsTimeBasis::from_subtract_time_stop(matches!(
                        config.dps_time_mode,
                        DpsTimeMode::TimeStopAdjusted
                    )),
                    config.separate_reaction_damage,
                )
            },
            |archive| self.persist_history_archive(archive),
        )?;
        Ok(result.is_some())
    }

    pub(crate) fn maintain_history_rounds(&self) {
        let pending = self.0.live_capture.take_pending_abyss_archives();
        let mut retry = Vec::new();
        for details in pending {
            let Some(archive) = self.prepare_history_details(details.clone()) else {
                continue;
            };
            if let Err(error) = self.persist_history_archive(archive) {
                log::warn!("automatic Abyss History archive failed: {error}");
                retry.push(details);
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
        let due = self.0.live_capture.with_state(|state| {
            auto_round_due(
                status.phase == LiveCapturePhase::Running,
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

    pub(crate) fn with_history_transaction<T>(&self, action: impl FnOnce() -> T) -> T {
        let _guard = self
            .0
            .history_transaction
            .lock()
            .expect("history transaction lock poisoned");
        action()
    }

    pub(crate) fn history_revision(&self) -> u64 {
        self.0.history_revision.load(Ordering::Acquire)
    }

    pub(crate) fn live_capture_resources(&self) -> LiveCaptureResources {
        self.0.live_capture.resources()
    }

    pub(crate) fn character_data_snapshot(
        &self,
    ) -> Result<(CharacterDataProjection, u64), CharacterDataError> {
        let _guard = self
            .0
            .character_data_transaction
            .lock()
            .expect("character data transaction lock poisoned");
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
            .expect("character data transaction lock poisoned");
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
            .expect("encrypted INI runtime lock poisoned");
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
            .expect("encrypted INI runtime lock poisoned");
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
            .expect("encrypted INI runtime lock poisoned");
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
            .expect("encrypted INI runtime lock poisoned");
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
            .expect("encrypted INI runtime lock poisoned");
        runtime.generation = runtime.generation.wrapping_add(1);
        runtime.path = None;
        runtime.document = None;
        encrypted_ini_projection(&runtime)
    }

    pub(crate) fn empty_curtain_snapshot(&self) -> InventorySnapshot {
        self.refresh_empty_curtain_operation();
        let resources = self.0.live_capture.resources();
        let observed_at_unix_ms = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis()
            .min(u128::from(u64::MAX)) as u64;
        self.0.live_capture.with_state(|state| {
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

    pub(crate) fn with_empty_curtain<T>(
        &self,
        action: impl FnOnce(
            &[nte_dps_tool::engine::model::EmptyCurtainItem],
            &[nte_dps_tool::engine::model::EmptyCurtainCharacter],
            &EquipmentCatalog,
        ) -> T,
    ) -> T {
        self.0.live_capture.with_state(|state| {
            action(
                &state.empty_curtain,
                &state.empty_curtain_characters,
                &self.0.equipment_catalog,
            )
        })
    }

    pub(crate) fn equipment_catalog(&self) -> Arc<EquipmentCatalog> {
        Arc::clone(&self.0.equipment_catalog)
    }

    pub(crate) fn empty_curtain_operation(&self) -> EmptyCurtainOperationState {
        self.refresh_empty_curtain_operation();
        self.0
            .empty_curtain_operation
            .lock()
            .expect("Console equipment operation lock poisoned")
            .clone()
    }

    pub(crate) fn empty_curtain_revision(&self) -> (u64, u64, u64) {
        self.refresh_empty_curtain_operation();
        let (inventory, characters) = self.0.live_capture.inventory_revision();
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
            .expect("diagnostics report lock poisoned")
            .clone()
    }

    pub(crate) fn store_diagnostics_report(&self, report: DiagnosticRun) {
        *self
            .0
            .diagnostics_report
            .lock()
            .expect("diagnostics report lock poisoned") = Some(report);
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

    pub(crate) fn request_diagnostics_replay(
        &self,
        kind: CaptureReplayKind,
        path: PathBuf,
    ) -> Result<(), CoreError> {
        let config = self.ui_config();
        let local_ip_hint = self
            .diagnostics_report()
            .and_then(|run| run.environment.local_ip)
            .and_then(|local_ip| local_ip.parse().ok());
        self.0.live_capture.request_replay(
            kind,
            path,
            local_ip_hint,
            true,
            config.server_damage_calibration,
        )
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
            .expect("Console equipment operation lock poisoned")
            .request_id
            .is_some()
        {
            return Err(ModsPluginSubmitError::Busy);
        }
        let request_id = self
            .0
            .mods_plugin
            .lock()
            .expect("Mod loader client lock poisoned")
            .submit(character, operation)?;
        *self
            .0
            .empty_curtain_operation
            .lock()
            .expect("Console equipment operation lock poisoned") = EmptyCurtainOperationState {
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

    fn refresh_empty_curtain_operation(&self) {
        let response = self
            .0
            .mods_plugin
            .lock()
            .expect("Mod loader client lock poisoned")
            .try_recv();
        let Some(response) = response else {
            return;
        };
        let mut operation = self
            .0
            .empty_curtain_operation
            .lock()
            .expect("Console equipment operation lock poisoned");
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
        self.0.live_capture.with_state(|state| {
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
        self.0
            .live_capture
            .with_packet_state(|revision, _, _| revision)
    }

    pub(crate) fn packets_projection(
        &self,
        after: Option<PacketStreamRevision>,
    ) -> (PacketStreamRevision, bool, PacketsProjection) {
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
        self.0.live_capture.with_state(|state| {
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
        self.0.history_revision.fetch_add(1, Ordering::AcqRel) + 1
    }

    pub(crate) fn remember_deleted_history(&self, record: HistoryRecord) -> String {
        let sequence = self.0.history_undo_sequence.fetch_add(1, Ordering::AcqRel) + 1;
        let token = format!("history-undo-{sequence}");
        *self
            .0
            .history_undo
            .lock()
            .expect("history undo lock poisoned") = Some(HistoryUndoEntry {
            token: token.clone(),
            record,
            expires_at: Instant::now() + HISTORY_UNDO_WINDOW,
        });
        token
    }

    pub(crate) fn take_deleted_history(&self, token: &str) -> Option<HistoryRecord> {
        let mut undo = self
            .0
            .history_undo
            .lock()
            .expect("history undo lock poisoned");
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
            .expect("imported team lock poisoned");
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
            .expect("UI config lock poisoned")
            .clone()
    }

    fn update_ui_config(&self, update: impl FnOnce(&mut UiConfig)) -> Result<bool, String> {
        let _transaction = self
            .0
            .config_transaction
            .lock()
            .expect("UI config transaction lock poisoned");
        let previous = self.ui_config();
        let mut candidate = previous.clone();
        update(&mut candidate);
        candidate = candidate.sanitized();
        if candidate == previous {
            return Ok(false);
        }
        config::save(&self.0.config_path, &candidate)?;
        *self.0.ui_config.lock().expect("UI config lock poisoned") = candidate;
        self.0.presentation_revision.fetch_add(1, Ordering::AcqRel);
        self.0.settings_revision.fetch_add(1, Ordering::AcqRel);
        Ok(true)
    }

    pub(crate) fn stream_revision(&self) -> StreamRevision {
        StreamRevision {
            capture: self.0.live_capture.revision(),
            presentation: self.0.presentation_revision.load(Ordering::Acquire),
        }
    }

    pub(crate) fn settings_revision(&self) -> u64 {
        self.0.settings_revision.load(Ordering::Acquire)
    }

    pub(crate) fn mod_studio(&self) -> ModStudioWorkspaceService {
        self.0.mod_studio.clone()
    }

    pub(crate) fn uptime_ms(&self) -> u128 {
        self.0.started_at.elapsed().as_millis()
    }

    fn hud_config(&self) -> HudConfig {
        self.0
            .ui_config
            .lock()
            .expect("UI config lock poisoned")
            .hud
            .clone()
    }

    pub(crate) fn begin_stream(&self, subscription_id: String) -> Arc<AtomicBool> {
        let stop = Arc::new(AtomicBool::new(false));
        let replaced = self
            .0
            .streams
            .lock()
            .expect("technical stream registry lock poisoned")
            .insert(subscription_id, Arc::clone(&stop));

        if let Some(replaced) = replaced {
            replaced.store(true, Ordering::Release);
        }

        stop
    }

    pub(crate) fn stop_stream(&self, subscription_id: &str) {
        if let Some(stop) = self
            .0
            .streams
            .lock()
            .expect("technical stream registry lock poisoned")
            .remove(subscription_id)
        {
            stop.store(true, Ordering::Release);
        }
    }

    pub(crate) fn finish_stream(&self, subscription_id: &str, stop: &Arc<AtomicBool>) {
        let mut streams = self
            .0
            .streams
            .lock()
            .expect("technical stream registry lock poisoned");
        let is_current = streams
            .get(subscription_id)
            .is_some_and(|current| Arc::ptr_eq(current, stop));

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
        let previous = state.begin_stream("technical".to_owned());
        let current = state.begin_stream("technical".to_owned());

        assert!(previous.load(Ordering::Acquire));
        assert!(!current.load(Ordering::Acquire));
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
        let previous = state.begin_stream("technical".to_owned());
        let current = state.begin_stream("technical".to_owned());

        state.finish_stream("technical", &previous);
        state.stop_stream("technical");

        assert!(current.load(Ordering::Acquire));
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
        let mut config = UiConfig::default();
        config.always_on_top = false;
        config.hud.width = 512;
        config.hud.show_total_damage = false;
        config.passthrough_hotkey = PassthroughHotkey::F8;

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
            (HUD_BASE_INITIAL_HEIGHT + HUD_SUMMARY_HEIGHT + HUD_CHARACTERS_HEIGHT)
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
        assert!(!saved.always_on_top);

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
}
