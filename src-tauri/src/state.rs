use std::{
    collections::HashMap,
    collections::HashSet,
    path::PathBuf,
    sync::{
        Arc, Mutex, MutexGuard,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
    time::Instant,
};

use nte_dps_tool::{
    core::{
        CoreError,
        capture::{
            CaptureControllerOptions, CaptureDeviceSelector, CaptureProfile, RawCaptureMode,
        },
        hud::{HudProjectionOptions, project_hud},
        live_capture::{LiveCaptureResources, LiveCaptureService},
    },
    engine::{
        capture::PacketEmissionMode,
        model::{AbyssHalf, DpsTimeBasis},
    },
    storage::{
        config::{
            self, DpsTimeMode, HudConfig, HudModule, PassthroughHotkey, UiConfig,
            sanitize_timeline_bucket_seconds,
        },
        i18n::Language,
        paths::capture_log_dir,
    },
};

use crate::{
    contract::{HudWindowSnapshot, TECHNICAL_CONTRACT_VERSION, TechnicalSnapshot},
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

struct AppStateInner {
    started_at: Instant,
    sequence: AtomicU64,
    passthrough: AtomicBool,
    passthrough_hotkey_ready: AtomicBool,
    always_on_top: AtomicBool,
    presentation_revision: AtomicU64,
    streams: Mutex<HashMap<String, Arc<AtomicBool>>>,
    live_capture: LiveCaptureService,
    capture_device: CaptureDeviceSelector,
    server_damage_calibration: bool,
    dps_time_basis: DpsTimeBasis,
    separate_reaction_damage: bool,
    timeline_bucket_seconds: f64,
    selected_abyss_half: Mutex<Option<AbyssHalf>>,
    passthrough_transaction: Mutex<()>,
    always_on_top_transaction: Mutex<()>,
    ui_config: Mutex<UiConfig>,
    config_path: PathBuf,
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
        config.hud = config.hud.clone().sanitized();
        let capture_device = config
            .manual_capture_device
            .clone()
            .map_or(CaptureDeviceSelector::Auto, CaptureDeviceSelector::Name);
        Self(Arc::new(AppStateInner {
            started_at: Instant::now(),
            sequence: AtomicU64::new(0),
            passthrough: AtomicBool::new(false),
            passthrough_hotkey_ready: AtomicBool::new(false),
            always_on_top: AtomicBool::new(config.always_on_top),
            presentation_revision: AtomicU64::new(0),
            streams: Mutex::new(HashMap::new()),
            live_capture,
            capture_device,
            server_damage_calibration: config.server_damage_calibration,
            dps_time_basis: DpsTimeBasis::from_subtract_time_stop(matches!(
                config.dps_time_mode,
                DpsTimeMode::TimeStopAdjusted
            )),
            separate_reaction_damage: config.separate_reaction_damage,
            timeline_bucket_seconds: f64::from(sanitize_timeline_bucket_seconds(
                config.timeline_bucket_seconds,
            )),
            selected_abyss_half: Mutex::new(None),
            passthrough_transaction: Mutex::new(()),
            always_on_top_transaction: Mutex::new(()),
            ui_config: Mutex::new(config),
            config_path,
        }))
    }

    pub(crate) fn snapshot(&self) -> TechnicalSnapshot {
        let sequence = self.0.sequence.fetch_add(1, Ordering::Relaxed) + 1;
        let hud_config = self.hud_config();
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
                        dps_time_basis: self.0.dps_time_basis,
                        separate_reaction_damage: self.0.separate_reaction_damage,
                        selected_abyss_half,
                        preview_when_empty: !self.passthrough(),
                        timeline_bucket_seconds: self.0.timeline_bucket_seconds,
                    },
                )
            }),
        }
    }

    pub(crate) fn request_capture_start(&self) -> Result<(), CoreError> {
        self.0.live_capture.request_start(CaptureControllerOptions {
            profile: CaptureProfile::Combat,
            device: self.0.capture_device.clone(),
            include_incoming: true,
            server_damage_calibration: self.0.server_damage_calibration,
            raw_capture: RawCaptureMode::Enabled,
            raw_capture_directory: capture_log_dir(),
            expose_raw_capture_path: false,
            packet_emission: PacketEmissionMode::SummaryOnly,
        })
    }

    pub(crate) fn request_capture_stop(&self) -> Result<(), CoreError> {
        self.0.live_capture.request_stop()
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

    pub(crate) fn set_hud_window_position(&self, position: [i32; 2]) -> Result<bool, String> {
        let mut config = self.0.ui_config.lock().expect("UI config lock poisoned");
        let previous = config.hud_window_position;
        if previous == Some(position) {
            return Ok(false);
        }

        config.hud_window_position = Some(position);
        if let Err(error) = config::save(&self.0.config_path, &config) {
            config.hud_window_position = previous;
            return Err(error);
        }
        Ok(true)
    }

    fn update_hud_config(&self, update: impl FnOnce(&mut HudConfig)) -> Result<bool, String> {
        let mut config = self.0.ui_config.lock().expect("UI config lock poisoned");
        let previous = config.hud.clone();
        let mut candidate = previous.clone();
        update(&mut candidate);
        candidate = candidate.sanitized();
        if candidate == previous {
            return Ok(false);
        }

        config.hud = candidate;
        if let Err(error) = config::save(&self.0.config_path, &config) {
            config.hud = previous;
            return Err(error);
        }
        drop(config);
        self.0.presentation_revision.fetch_add(1, Ordering::AcqRel);
        Ok(true)
    }

    pub(crate) fn set_always_on_top(&self, enabled: bool) -> Result<bool, String> {
        let mut config = self.0.ui_config.lock().expect("UI config lock poisoned");
        let previous = config.always_on_top;
        if previous == enabled {
            return Ok(false);
        }

        config.always_on_top = enabled;
        if let Err(error) = config::save(&self.0.config_path, &config) {
            config.always_on_top = previous;
            return Err(error);
        }
        if self.0.always_on_top.swap(enabled, Ordering::AcqRel) != enabled {
            self.0.presentation_revision.fetch_add(1, Ordering::AcqRel);
        }
        Ok(true)
    }

    pub(crate) fn stream_revision(&self) -> StreamRevision {
        StreamRevision {
            capture: self.0.live_capture.revision(),
            presentation: self.0.presentation_revision.load(Ordering::Acquire),
        }
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
}
