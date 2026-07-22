use std::collections::{HashMap, HashSet, VecDeque};
use std::fmt::Write as _;
use std::net::Ipv4Addr;
use std::path::{Path, PathBuf};
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
use std::thread;
use std::time::{Duration, Instant};

use chrono::{DateTime, Local};
use crossbeam_channel::{Receiver, Sender, TryRecvError, bounded, unbounded};
use eframe::egui::{self, Color32, RichText, Stroke};

use crate::core::capture::{self as core_capture, CaptureStartOptions, RawCaptureMode};
use crate::core::reducer::CoreSignal;
use crate::engine::abyss_data::{
    AbyssFloor, AbyssMonsterDataset, AbyssMonsterEntry, AbyssStarThreshold, abyss_line_hp_total,
    abyss_monster_total_hp, line_hp_by_wave, predict_wave_clear_times,
    required_dps_for_target_time,
};
use crate::engine::capture::{
    CaptureDevice, CaptureExportDocument, CaptureExportNetwork, CaptureExportOptions,
    CaptureHandle, CaptureResources, EngineEventSink, PacketEmissionMode, RawCaptureBuffer,
    import_capture_json, import_pcapng, write_capture_export,
};
use crate::engine::model::{
    AbyssEvent, AbyssHalf, COMBAT_SEGMENT_GAP_SECONDS, CaptureQualitySource, CaptureQualitySummary,
    CharacterInfo, CharacterStats, CombatSegment, CombatSessionAbyssHalfSummary,
    CombatSessionCharacterSummary, CombatSessionSkillSummary, CombatState, DpsTimeBasis,
    EngineEvent, HitDirection, HitDirectionSummary, PartyCombatState, SkillBreakdown,
    SkillBreakdownRow, TEAM_DPS_EXPORT_VERSION, TEAM_DPS_MAX_MEMBERS, TeamDps, TeamDpsExport,
    TeamDpsMember, TimelineMarkerKind, TimelineSeries, UNBALANCE_ATTACK_TYPE,
    summarize_combat_segments, summarize_hit_directions,
};
use crate::engine::parser::{
    AbilityCatalog, CHARACTER_DATA_PATH, EQUIPMENT_CATALOG_PATH, EquipmentCatalog, find_data_file,
    load_characters, load_equipment_catalog,
};
use crate::platform::equipment_plugin::{
    EquipmentPluginClient, EquipmentPluginDeploymentError, EquipmentPluginDeploymentStatus,
    EquipmentPluginGameRegion, EquipmentPluginOperation, EquipmentPluginPlacement,
    EquipmentPluginSubmitError,
};
use crate::platform::file_drop::NativeFileDrop;
use crate::platform::hotkey::{
    HotkeyEvent, HotkeyHandle, hotkey_binding_matches_egui, hotkey_key_to_egui,
    passthrough_hotkey_matches_egui, passthrough_hotkey_to_egui,
};
use crate::platform::network::GameNetwork;
use crate::platform::window_attributes::{
    DialogOwner, WindowAttributeConfig, apply_island_base_style, apply_rounding_to_process_windows,
    apply_window_attributes, cursor_screen_pos, find_process_window_by_title, open_directory,
    set_island_click_through, window_monitor_rect,
};
use crate::storage::capture_logs::{self, CaptureLogStats};
use crate::storage::config::{
    self, AccentColor, DpsTimeMode, GlobalHotkeyAction, GlobalHotkeys, HUD_WIDTH_MAX,
    HUD_WIDTH_MIN, HitDetailColumn, HitDetailColumnsConfig, HotkeyBinding, HotkeyKey, HudConfig,
    HudModule, PassthroughHotkey, TIMELINE_BUCKET_SECONDS_MAX, TIMELINE_BUCKET_SECONDS_MIN,
    ThemePreset, TimelineDpsViewMode, UiConfig, UiDensity,
};
use crate::storage::history::{self, HistoryComparison, HistoryRecord};
use crate::storage::i18n::{self, Language, t, tf};
use crate::storage::io_util::atomic_write_text;
use crate::storage::paths;
use crate::storage::resource::{read_equipment_plugin, read_resource_bytes, read_resource_text};
use crate::support::character_editor::{
    CHARACTER_ATTRIBUTES, CharacterEditForm, CharacterEditorState, json_string_field,
};
use crate::support::diagnostics::{
    DiagnosticReport, DiagnosticSnapshot, DiagnosticStatus, run_capture_diagnostics,
};
use crate::support::encrypted_ini::{
    EncryptedIniKey, EncryptedIniRecord, encrypt_encrypted_ini_records,
    encrypted_ini_search_matches, encrypted_ini_text_fingerprint, parse_encrypted_ini_text,
};
use crate::support::resource_audit::{
    ResourceAuditCategory, ResourceAuditItem, ResourceAuditSeverity, ResourceAuditSummary,
    audit_runtime_resources,
};

const MAX_UI_EVENTS_PER_FRAME: usize = 2_048;
const MAX_UI_EVENTS_WHILE_SCROLLING: usize = 256;
const UI_EVENT_BUDGET: Duration = Duration::from_millis(4);
const DETAIL_CACHE_REFRESH_DELAY: Duration = Duration::from_millis(200);
const MAX_PAUSED_EVENTS: usize = 50_000;
/// Semantic events are rare relative to raw packet diagnostics. This capacity
/// absorbs long UI stalls without letting reliable event memory grow without a
/// bound; a full lane backpressures the parser until the UI catches up.
const RELIABLE_ENGINE_EVENT_CAPACITY: usize = 16_384;
/// Full PacketDebug records contain payload hex/text, so keep their producer
/// queue small and discard excess records before allocating more queued state.
const DEBUG_ENGINE_EVENT_CAPACITY: usize = 2_048;
const MAX_DETAIL_HITS: usize = 10_000;
const DETAIL_HIT_ROW_HEIGHT: f32 = 40.0;
const MAIN_TITLE_BAR_HEIGHT: f32 = 40.0;
const MAIN_CONTROLS_SINGLE_ROW_HEIGHT: f32 = 34.0;
const TITLE_BAR_BUTTON_SIZE: egui::Vec2 = egui::vec2(28.0, 28.0);
/// Default inner size each window opens at before the user has dragged it (and no persisted size
/// exists). `main` is also used by `main.rs` for the initial root viewport size. Windows are now
/// freely resizable from their edges (`window_resize_grips`); these are just the starting sizes.
pub(crate) const MAIN_WINDOW_BASE_SIZE: egui::Vec2 = egui::vec2(600.0, 420.0);
const ABYSS_WINDOW_BASE_SIZE: egui::Vec2 = egui::vec2(1040.0, 720.0);
const HIT_DETAIL_WINDOW_BASE_SIZE: egui::Vec2 = egui::vec2(1120.0, 760.0);
const TEAM_HIT_DETAIL_WINDOW_BASE_SIZE: egui::Vec2 = egui::vec2(980.0, 660.0);
const CONSOLE_WINDOW_BASE_SIZE: egui::Vec2 = egui::vec2(980.0, 640.0);
const PASSTHROUGH_HOTKEY_COMBO_WIDTH: f32 = 150.0;
const CHARACTER_ATTRIBUTE_COMBO_WIDTH: f32 = 150.0;
const CHARACTER_EDITOR_CARD_HEIGHT: f32 = 68.0;
const CHARACTER_EDITOR_AVATAR_SIZE: f32 = 48.0;
const UI_CONFIG_SAVE_DELAY: Duration = Duration::from_millis(350);
const UI_CONFIG_SAVE_RETRY_DELAY: Duration = Duration::from_secs(2);
const STATUS_TOAST_DURATION: Duration = Duration::from_secs(4);
const UNDO_TOAST_DURATION: Duration = Duration::from_secs(5);
const HUD_TRANSITION_TRACKING_GUARD_FRAMES: u8 = 8;
const ABYSS_STAT_NAMES_ZH_CN_PATH: &str = "res/data/abyss/monster_stat_names_zh_cn.json";
const INLINE_CONTROL_HEIGHT: f32 = 28.0;
const INLINE_CONTROL_TEXT_SIZE: f32 = 13.0;

#[derive(Clone, Copy)]
pub(crate) enum DebugImportKind {
    Pcapng,
    CaptureJson,
    EncryptedIni,
}

impl DebugImportKind {
    /// English key; wrap with [`crate::storage::i18n::t`] at the display site.
    fn label(self) -> &'static str {
        match self {
            Self::Pcapng => "PCAPNG",
            Self::CaptureJson => "Capture JSON",
            Self::EncryptedIni => "Encrypted INI",
        }
    }
}

struct ActiveImport {
    kind: DebugImportKind,
    path: PathBuf,
    started_at: Instant,
    viewport: egui::ViewportId,
}

/// What to do with the path once a native file dialog comes back. The blocking
/// `rfd` dialogs pump a Win32 modal message loop, which re-enters the winit
/// event loop and deadlocks the wgpu presenter when run on the UI thread, so
/// every dialog runs on a worker thread and reports back through this.
pub(crate) enum FileDialogPurpose {
    DebugImport { kind: DebugImportKind },
    TeamDpsImportAll,
    TeamDpsImportLine { upper: bool },
    TeamDpsExport { json: String },
    HistoryExport { json: String },
    EmptyCurtainExport { json: String },
    CharacterLoadoutImport,
    CharacterLoadoutExport { json: String },
    CaptureInfoExport,
    RawCaptureExport,
}

/// Parent a native dialog to our root window when we have its handle, so it
/// can't end up hidden behind an always-on-top window. An owned window always
/// renders above its owner regardless of topmost/z-order — see [`DialogOwner`]
/// for why that beats clearing topmost with `SetWindowPos`.
fn with_owner(dialog: rfd::FileDialog, owner: Option<DialogOwner>) -> rfd::FileDialog {
    match owner {
        Some(owner) => dialog.set_parent(&owner),
        None => dialog,
    }
}

struct PendingFileDialog {
    purpose: FileDialogPurpose,
    /// Viewport that opened the dialog; errors from the completion handler are
    /// routed back to it.
    viewport: egui::ViewportId,
    receiver: Receiver<Option<PathBuf>>,
}

struct PendingCaptureExport {
    viewport: egui::ViewportId,
    receiver: Receiver<Result<(), String>>,
    thread: Option<thread::JoinHandle<()>>,
}

pub(crate) enum ConfirmationAction {
    StartLive,
    ResetSession,
    ImportPcapng(PathBuf),
    ImportCaptureJson(PathBuf),
    ClearEncryptedIni,
    ReloadEncryptedIni(PathBuf),
    ClearCaptureLogs,
}

#[derive(Clone, Copy)]
pub(crate) enum ErrorAction {
    RefreshNetwork,
    OpenPcapng,
    OpenCaptureJson,
    OpenEncryptedIni,
    OpenTeamDpsImport,
    OpenConsole,
}

/// Tabs of the console window, including capture diagnostics and resource tools.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum ConsoleTab {
    #[default]
    Settings,
    Timeline,
    Skills,
    EmptyCurtain,
    History,
    Characters,
    EncryptedIni,
    Packets,
    Resources,
    Diagnostics,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum ConsoleGroup {
    Common,
    Review,
    Advanced,
}

impl ConsoleGroup {
    fn label_key(self) -> &'static str {
        match self {
            Self::Common => "Common",
            Self::Review => "Review",
            Self::Advanced => "Advanced",
        }
    }
}

impl ConsoleTab {
    fn visible_tabs() -> &'static [Self] {
        const TABS: &[ConsoleTab] = &[
            ConsoleTab::Settings,
            ConsoleTab::History,
            ConsoleTab::Timeline,
            ConsoleTab::Skills,
            ConsoleTab::EmptyCurtain,
            ConsoleTab::Characters,
            ConsoleTab::EncryptedIni,
            ConsoleTab::Packets,
            ConsoleTab::Resources,
            ConsoleTab::Diagnostics,
        ];
        TABS
    }

    fn label_key(self) -> &'static str {
        match self {
            Self::Settings => "Settings",
            Self::Timeline => "Timeline",
            Self::Skills => "Skills",
            Self::EmptyCurtain => "Console Loadout",
            Self::History => "History",
            Self::Characters => "Character Data",
            Self::EncryptedIni => "Encrypted INI",
            Self::Packets => "Packets",
            Self::Resources => "Resources",
            Self::Diagnostics => "Diagnostics",
        }
    }

    /// Material icon rendered with the dedicated font installed in `chrome.rs`.
    fn icon(self) -> egui_material_icons::MaterialIcon {
        use egui_material_icons::icons::{
            ICON_AUTO_AWESOME, ICON_BACKPACK, ICON_FOLDER, ICON_HISTORY, ICON_LOCK, ICON_PERSON,
            ICON_SENSORS, ICON_SETTINGS, ICON_TIMELINE, ICON_TROUBLESHOOT,
        };

        match self {
            Self::Settings => ICON_SETTINGS,
            Self::Timeline => ICON_TIMELINE,
            Self::Skills => ICON_AUTO_AWESOME,
            Self::EmptyCurtain => ICON_BACKPACK,
            Self::History => ICON_HISTORY,
            Self::Characters => ICON_PERSON,
            Self::EncryptedIni => ICON_LOCK,
            Self::Packets => ICON_SENSORS,
            Self::Resources => ICON_FOLDER,
            Self::Diagnostics => ICON_TROUBLESHOOT,
        }
    }

    fn group(self) -> ConsoleGroup {
        match self {
            Self::Settings | Self::History => ConsoleGroup::Common,
            Self::Timeline | Self::Skills | Self::EmptyCurtain => ConsoleGroup::Review,
            Self::Characters
            | Self::EncryptedIni
            | Self::Packets
            | Self::Resources
            | Self::Diagnostics => ConsoleGroup::Advanced,
        }
    }

    fn adjacent(self, offset: isize) -> Self {
        let tabs = Self::visible_tabs();
        let index = tabs
            .iter()
            .position(|tab| *tab == self)
            .expect("every ConsoleTab reachable from the UI is in visible_tabs")
            as isize;
        tabs[(index + offset).rem_euclid(tabs.len() as isize) as usize]
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) enum HitDetailFilter {
    #[default]
    All,
    Outgoing,
    Incoming,
    QteType(String),
}

impl HitDetailFilter {
    fn matches(&self, hit: &crate::engine::model::Hit) -> bool {
        match self {
            Self::All => true,
            Self::Outgoing => !hit.direction.is_incoming(),
            Self::Incoming => hit.direction.is_incoming(),
            Self::QteType(attack_type) => {
                !hit.direction.is_incoming()
                    && (hit.attack_type.as_deref() == Some(attack_type)
                        || hit.follow_up_attack_type.as_deref() == Some(attack_type))
            }
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct QteTypeFilterSummary {
    attack_type: String,
    hits: usize,
    damage: f64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum HitDetailSource {
    Global,
    AbyssFirst,
    AbyssSecond,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct HitDetailCacheKey {
    source: HitDetailSource,
    char_id: Option<u32>,
    filter: HitDetailFilter,
    skill_filter: String,
    limit: usize,
}

#[derive(Clone, Copy)]
pub(crate) struct CachedHitRow {
    index: usize,
    is_incoming: bool,
    damage: f64,
    char_id: u32,
    hp_fraction: f32,
    timestamp: f64,
    byte_offset: usize,
    bit_shift: u8,
    target_hp_after: f64,
    target_max_hp: f64,
}

#[derive(Default)]
pub(crate) struct HitDetailCache {
    key: Option<HitDetailCacheKey>,
    generation: u64,
    source_len: usize,
    rows: Vec<CachedHitRow>,
    filtered_count: usize,
    max_damage: f64,
    dirty_since: Option<Instant>,
    last_scroll_offset: Option<f32>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct SkillSummaryCacheKey {
    source: HitDetailSource,
    generation: u64,
    char_id: u32,
}

#[derive(Default)]
struct SkillSummaryCache {
    key: Option<SkillSummaryCacheKey>,
    /// Cache hits occur on every visible frame; snapshots must stay O(1) to hand out.
    rows: Arc<Vec<SkillDamageSummary>>,
    dirty_since: Option<Instant>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct TimelineCacheKey {
    source: HitDetailSource,
    generation: u64,
    subtract_time_stop: bool,
    bucket_millis: u32,
}

#[derive(Default)]
struct TimelineCache {
    key: Option<TimelineCacheKey>,
    /// A long timeline can contain thousands of buckets and nested role rows.
    series: Arc<TimelineSeries>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct SkillBreakdownCacheKey {
    source: HitDetailSource,
    generation: u64,
    char_id: Option<u32>,
}

#[derive(Default)]
struct SkillBreakdownCache {
    key: Option<SkillBreakdownCacheKey>,
    /// Shared with the visible view so a cache hit does not clone every label.
    breakdown: Arc<SkillBreakdown>,
}

const ATTRIBUTE_ICON_PATHS: [(&str, &str); 6] = [
    ("灵", "res/images/attributes/UI_avatarbg_Icon_01.png"),
    ("咒", "res/images/attributes/UI_avatarbg_Icon_06.png"),
    ("光", "res/images/attributes/UI_avatarbg_Icon_04.png"),
    ("魂", "res/images/attributes/UI_avatarbg_Icon_05.png"),
    ("暗", "res/images/attributes/UI_avatarbg_Icon_03.png"),
    ("相", "res/images/attributes/UI_avatarbg_Icon_02.png"),
];
const DAMAGE_DIGIT_IMAGE_DIR: &str = "res/images/font/tiaozi1";
const MONSTER_IMAGE_DIR: &str = "res/images/monsters";
const REACTION_TEXT_IMAGE_COUNT: u8 = 8;
// The `_L`/`_H`/`_Z` "non-trigger side" reaction digit sets (Guangling_L,
// Anhun_H, Zhouan_Z, lingzhou_Z) were removed along with their PNGs: reactions
// now always render the trigger-side series (see mixed_damage_digit_key), so
// those sets had no remaining users.
const DAMAGE_DIGIT_TEXTURE_SETS: [(&str, &str); 17] = [
    ("灵", "ling"),
    ("咒", "zhou"),
    ("光", "guang"),
    ("魂", "hun"),
    ("暗", "an"),
    ("相", "xiang"),
    ("物理", "wuli"),
    ("HP", "HP"),
    ("真实", "zhenshi"),
    ("Guangling_G", "Guangling_G"),
    ("Guangxiang_G", "Guangxiang_G"),
    ("Guangxiang_X", "Guangxiang_X"),
    ("Hunxiang_H", "Hunxiang_H"),
    ("Hunxiang_X", "Hunxiang_X"),
    ("Anhun_A", "Anhun_A"),
    ("Zhouan_A", "Zhouan_A"),
    ("lingzhou_L", "lingzhou_L"),
];

#[derive(Default)]
pub(crate) struct EncryptedIniLayoutCache {
    key: Option<EncryptedIniLayoutCacheKey>,
    galley: Option<Arc<egui::Galley>>,
}

impl EncryptedIniLayoutCache {
    fn clear(&mut self) {
        self.key = None;
        self.galley = None;
    }
}

#[derive(Clone, PartialEq, Eq)]
struct EncryptedIniLayoutCacheKey {
    text_len: usize,
    text_hash: u64,
    query: String,
    current_match_byte: Option<usize>,
    dark_mode: bool,
    accent: AccentColor,
    text_color: Color32,
}

pub(crate) struct EncryptedIniLayoutRequest<'a> {
    text: &'a str,
    query: &'a str,
    matches: &'a [usize],
    current_match_byte: Option<usize>,
    wrap_width: f32,
    dark_mode: bool,
    accent: AccentColor,
}

#[derive(Default)]
struct EncryptedIniEditorState {
    path: Option<PathBuf>,
    key: EncryptedIniKey,
    plaintext: String,
    search: String,
    search_match: Option<usize>,
    search_matches: Vec<usize>,
    search_cache_query: String,
    search_matches_dirty: bool,
    original_key: EncryptedIniKey,
    original_plaintext: String,
    records: Vec<EncryptedIniRecord>,
    line_ending: String,
    final_newline: bool,
    dirty: bool,
    message: String,
    layout_cache: EncryptedIniLayoutCache,
}

impl EncryptedIniEditorState {
    fn load(path: PathBuf) -> Result<Self, String> {
        let encrypted = std::fs::read_to_string(&path).map_err(|error| {
            tf(
                "Cannot read {}: {}",
                &[&path.display().to_string(), &error.to_string()],
            )
        })?;
        let (key, plaintext, records, line_ending, final_newline) =
            parse_encrypted_ini_text(&encrypted)?;
        let encrypted_lines = records.len();
        Ok(Self {
            path: Some(path),
            key,
            original_key: key,
            original_plaintext: plaintext.clone(),
            records,
            line_ending,
            final_newline,
            plaintext,
            search: String::new(),
            search_match: None,
            search_matches: Vec::new(),
            search_cache_query: String::new(),
            search_matches_dirty: false,
            dirty: false,
            message: tf(
                "Parsed {} lines of ciphertext using the {} key",
                &[&encrypted_lines.to_string(), &t(key.label())],
            ),
            layout_cache: EncryptedIniLayoutCache::default(),
        })
    }

    fn display_path(&self) -> String {
        self.path
            .as_ref()
            .map_or_else(|| t("No file open"), |path| path.display().to_string())
    }

    fn refresh_search_matches(&mut self) {
        if !self.search_matches_dirty && self.search_cache_query == self.search {
            return;
        }
        self.search_matches = encrypted_ini_search_matches(&self.plaintext, &self.search);
        self.search_cache_query = self.search.clone();
        self.search_matches_dirty = false;
        if self
            .search_match
            .is_some_and(|index| index >= self.search_matches.len())
        {
            self.search_match = None;
        }
    }
}

#[derive(Default)]
pub(crate) struct AbyssOverviewState {
    dataset: Option<AbyssMonsterDataset>,
    stat_display_names: HashMap<String, String>,
    load_error: Option<String>,
    selected_season: Option<u32>,
    selected_floor: Option<u32>,
    selected_monster_pack_id: Option<String>,
    expanded_season: Option<u32>,
    search: String,
    // Teams assigned to the upper (上行线) and lower (下行线) lines. Predicted
    // clear time for a line = that line's total HP / team.dps.
    upper_team: Option<TeamDps>,
    lower_team: Option<TeamDps>,
    upper_target_seconds: f64,
    lower_target_seconds: f64,
}

fn load_abyss_stat_display_names() -> HashMap<String, String> {
    let Some(path) = find_data_file(Path::new(ABYSS_STAT_NAMES_ZH_CN_PATH)) else {
        return HashMap::new();
    };
    let Ok(text) = read_resource_text(&path) else {
        return HashMap::new();
    };
    let Ok(names) = serde_json::from_str::<HashMap<String, String>>(&text) else {
        return HashMap::new();
    };

    let mut display_names = HashMap::with_capacity(names.len() * 2);
    for (key, value) in names {
        let value = value.trim();
        if value.is_empty() {
            continue;
        }
        display_names.insert(key.to_ascii_lowercase(), value.to_owned());
        display_names.insert(key, value.to_owned());
    }
    display_names
}

impl AbyssOverviewState {
    /// Every monster id referenced by the loaded dataset, used to drive the
    /// background monster-portrait loader without sharing the whole state.
    fn monster_ids(&self) -> Vec<String> {
        let Some(dataset) = &self.dataset else {
            return Vec::new();
        };
        dataset
            .seasons
            .iter()
            .flat_map(|season| season.floors.iter())
            .flat_map(|floor| floor.monsters.iter())
            .map(|monster| monster.monster_id.clone())
            .collect()
    }

    fn load() -> Self {
        let stat_display_names = load_abyss_stat_display_names();
        match AbyssMonsterDataset::load() {
            Ok(dataset) => {
                let first = dataset.first_floor_key();
                // Seed the Target field with the first floor's real top-star
                // clear time instead of a flat guess, when the summary
                // dataset provides one; otherwise keep the historical default.
                let initial_target_seconds = first
                    .and_then(|(season, floor)| dataset.floor(season, floor))
                    .and_then(|floor| {
                        floor
                            .star_thresholds
                            .iter()
                            .max_by_key(|threshold| threshold.stars)
                    })
                    .map(|threshold| threshold.seconds)
                    .unwrap_or(90.0);
                Self {
                    dataset: Some(dataset),
                    stat_display_names,
                    load_error: None,
                    selected_season: first.map(|(season, _)| season),
                    selected_floor: first.map(|(_, floor)| floor),
                    selected_monster_pack_id: None,
                    expanded_season: first.map(|(season, _)| season),
                    search: String::new(),
                    upper_team: None,
                    lower_team: None,
                    upper_target_seconds: initial_target_seconds,
                    lower_target_seconds: initial_target_seconds,
                }
            }
            Err(error) => Self {
                load_error: Some(error.to_string()),
                stat_display_names,
                ..Default::default()
            },
        }
    }

    fn reload(&mut self) {
        // Reloading only refreshes the monster dataset; keep the user's search
        // and imported prediction teams.
        let search = self.search.clone();
        let upper_team = self.upper_team.take();
        let lower_team = self.lower_team.take();
        let upper_target_seconds = self.upper_target_seconds;
        let lower_target_seconds = self.lower_target_seconds;
        *self = Self::load();
        self.search = search;
        self.upper_team = upper_team;
        self.lower_team = lower_team;
        self.upper_target_seconds = upper_target_seconds;
        self.lower_target_seconds = lower_target_seconds;
    }

    fn swap_teams(&mut self) {
        std::mem::swap(&mut self.upper_team, &mut self.lower_team);
    }

    fn ensure_selection(&mut self) {
        let Some(dataset) = &self.dataset else {
            return;
        };
        let valid = self
            .selected_season
            .zip(self.selected_floor)
            .is_some_and(|(season, floor)| dataset.floor(season, floor).is_some());
        if !valid && let Some((season, floor)) = dataset.first_floor_key() {
            self.selected_season = Some(season);
            self.selected_floor = Some(floor);
        }
        if self
            .expanded_season
            .is_none_or(|season| dataset.season(season).is_none())
        {
            self.expanded_season = self.selected_season;
        }
        if let Some(pack_id) = &self.selected_monster_pack_id
            && dataset.monster(pack_id).is_none()
        {
            self.selected_monster_pack_id = None;
        }
    }
}

#[derive(Default)]
struct HistoryState {
    records: Vec<HistoryRecord>,
    selected_id: Option<String>,
    compare_left_id: Option<String>,
    compare_right_id: Option<String>,
    skipped_files: usize,
    message: String,
}

impl HistoryState {
    fn load() -> Self {
        let result = history::load_history();
        let mut state = Self {
            records: result.records,
            skipped_files: result.skipped_files,
            ..Default::default()
        };
        state.ensure_selection();
        state
    }

    fn reload(&mut self) {
        let selected_id = self.selected_id.clone();
        let compare_left_id = self.compare_left_id.clone();
        let compare_right_id = self.compare_right_id.clone();
        let result = history::load_history();
        self.records = result.records;
        self.skipped_files = result.skipped_files;
        self.selected_id = selected_id;
        self.compare_left_id = compare_left_id;
        self.compare_right_id = compare_right_id;
        self.ensure_selection();
    }

    fn ensure_selection(&mut self) {
        if self
            .selected_id
            .as_deref()
            .is_none_or(|id| !self.records.iter().any(|record| record.id == id))
        {
            self.selected_id = self.records.first().map(|record| record.id.clone());
        }
        if self
            .compare_left_id
            .as_deref()
            .is_none_or(|id| !self.records.iter().any(|record| record.id == id))
        {
            self.compare_left_id = self.records.first().map(|record| record.id.clone());
        }
        if self
            .compare_right_id
            .as_deref()
            .is_none_or(|id| !self.records.iter().any(|record| record.id == id))
        {
            self.compare_right_id = self.records.get(1).map(|record| record.id.clone());
        }
    }

    fn selected_record(&self) -> Option<&HistoryRecord> {
        let selected_id = self.selected_id.as_deref()?;
        self.records.iter().find(|record| record.id == selected_id)
    }

    fn compare_records(&self) -> Option<(&HistoryRecord, &HistoryRecord, HistoryComparison)> {
        let left_id = self.compare_left_id.as_deref()?;
        let right_id = self.compare_right_id.as_deref()?;
        if left_id == right_id {
            return None;
        }
        let left = self.records.iter().find(|record| record.id == left_id)?;
        let right = self.records.iter().find(|record| record.id == right_id)?;
        Some((left, right, history::compare_records(left, right)))
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum ResourceAuditSeverityFilter {
    #[default]
    All,
    Error,
    Warning,
}

impl ResourceAuditSeverityFilter {
    /// English key; wrap with [`crate::storage::i18n::t`] at the display site.
    fn label(self) -> &'static str {
        match self {
            Self::All => "All Levels",
            Self::Error => "Errors Only",
            Self::Warning => "Warnings Only",
        }
    }

    fn matches(self, severity: ResourceAuditSeverity) -> bool {
        match self {
            Self::All => true,
            Self::Error => severity == ResourceAuditSeverity::Error,
            Self::Warning => severity == ResourceAuditSeverity::Warning,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum ResourceAuditCategoryFilter {
    #[default]
    All,
    Category(ResourceAuditCategory),
}

impl ResourceAuditCategoryFilter {
    /// English key; wrap with [`crate::storage::i18n::t`] at the display site.
    fn label(self) -> &'static str {
        match self {
            Self::All => "All Categories",
            Self::Category(category) => category.label(),
        }
    }

    fn matches(self, category: ResourceAuditCategory) -> bool {
        match self {
            Self::All => true,
            Self::Category(expected) => category == expected,
        }
    }
}

#[derive(Default)]
struct ResourceAuditState {
    summary: Option<ResourceAuditSummary>,
    loading: bool,
    message: String,
    severity_filter: ResourceAuditSeverityFilter,
    category_filter: ResourceAuditCategoryFilter,
}

#[derive(Clone, Copy)]
pub(crate) enum ToastTone {
    Status,
    Success,
    Warning,
    Danger,
}

pub(crate) enum UndoState {
    CombatSession(Box<CombatUndoSnapshot>),
    HistoryRecord(Box<HistoryRecord>),
}

pub(crate) struct CombatUndoSnapshot {
    state: CombatState,
    capture_quality_source: CaptureQualitySource,
    timeline_view: TimelineViewState,
    hidden_character_ids: HashSet<u32>,
    selected_abyss_half: AbyssHalf,
    abyss_compact_mode: bool,
}

struct StatusToast {
    id: u64,
    text: String,
    tone: ToastTone,
    viewport: egui::ViewportId,
    shown_until: Instant,
    last_tick: Instant,
    hovered: bool,
    animation_seeded: bool,
    undo_id: Option<u64>,
}

struct PassthroughNotice {
    enabled: bool,
    shown_until: Instant,
}

#[derive(Clone, Copy, PartialEq, Eq)]
struct AppliedStyleKey {
    dark_mode: bool,
    theme_preset: ThemePreset,
    accent: AccentColor,
    density: UiDensity,
    reduce_motion: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct HudSizeKey {
    rows: usize,
    show_title_strip: bool,
    show_status_row: bool,
    config: HudConfig,
}

#[derive(Clone, Copy)]
pub(crate) struct HudSummaryValues {
    total_damage: f64,
    team_dps: f64,
    duration: f64,
    damage_taken: f64,
}

#[derive(Clone, Copy)]
pub(crate) struct HudPaintColors {
    accent: Color32,
    text: Color32,
    muted: Color32,
    halo: Color32,
}

#[derive(Clone, Copy)]
pub(crate) struct HudRowsLayout {
    left: f32,
    right: f32,
    top: f32,
    width: f32,
}

/// A decoded texture set handed from the background loader thread to the UI.
/// Texture decode (the 6 MB of PNGs) runs off the main thread so the window
/// paints immediately; the maps start empty and every draw-site lookup already
/// falls back when a key is missing, so placeholders show until these arrive.
enum TextureLoad {
    Avatars(HashMap<String, egui::TextureHandle>),
    Attributes(HashMap<String, egui::TextureHandle>),
    DamageDigits(HashMap<String, Vec<egui::TextureHandle>>),
    Reactions(HashMap<u8, Vec<egui::TextureHandle>>),
    Monsters(HashMap<String, egui::TextureHandle>),
    Equipment(HashMap<String, egui::TextureHandle>),
}

/// Result of the startup capture-environment probe (Npcap device list + the
/// HTGame.exe NIC), computed on a background thread so `DpsApp::new` need not
/// block on device enumeration. `start_live` re-runs this on every capture
/// start, so this only seeds the initial status and device dropdown.
struct DeviceDetection {
    devices: Vec<CaptureDevice>,
    selected_device: usize,
    game_process_detected: bool,
    game_network: Option<GameNetwork>,
    local_ip: String,
    status: String,
    diagnostic: Option<String>,
}

struct CaptureUiState {
    devices: Vec<CaptureDevice>,
    selected_device: usize,
    /// Manual capture-NIC override (Npcap device `name`). `None` selects automatic detection.
    manual_capture_device: Option<String>,
    local_ip: String,
    game_process_detected: bool,
    game_network: Option<GameNetwork>,
    filter: String,
    active_capture_filter: Option<String>,
    capture_quality_source: CaptureQualitySource,
    include_incoming: bool,
    server_damage_calibration: bool,
    dps_time_mode: DpsTimeMode,
    timeline_bucket_seconds: f32,
    timeline_dps_view_mode: TimelineDpsViewMode,
    /// Cached size/count of `logs/nte_raw_*.pcapng`, scanned only on demand in settings.
    capture_log_stats: Option<CaptureLogStats>,
    paused: bool,
    dropped_debug_packets: u64,
}

impl CaptureUiState {
    fn from_config(config: &UiConfig) -> Self {
        Self {
            devices: Vec::new(),
            selected_device: 0,
            manual_capture_device: config.manual_capture_device.clone(),
            local_ip: String::new(),
            game_process_detected: false,
            game_network: None,
            filter: "udp".to_owned(),
            active_capture_filter: None,
            capture_quality_source: CaptureQualitySource::Unknown,
            include_incoming: true,
            server_damage_calibration: config.server_damage_calibration,
            dps_time_mode: config.dps_time_mode,
            timeline_bucket_seconds: config.timeline_bucket_seconds,
            timeline_dps_view_mode: config.timeline_dps_view_mode,
            capture_log_stats: None,
            paused: false,
            dropped_debug_packets: 0,
        }
    }
}

struct WindowState {
    /// Chrome-less combat overlay that replaces the normal root-window content.
    hud_mode: bool,
    /// Last HUD row/strip combination used to size the root window.
    hud_size_key: Option<HudSizeKey>,
    abyss_overview_open: bool,
    abyss_overview_corner_applied: bool,
    abyss_overview_geometry: SecondaryViewportGeometry,
    hit_detail_char_id: Option<u32>,
    hit_detail_corner_applied: bool,
    hit_detail_geometry: SecondaryViewportGeometry,
    team_hit_detail_open: bool,
    team_hit_detail_corner_applied: bool,
    team_hit_detail_geometry: SecondaryViewportGeometry,
    console_open: bool,
    console_corner_applied: bool,
    console_geometry: SecondaryViewportGeometry,
    applied_opacity: Option<f32>,
    corner_applied_hwnd: Option<isize>,
    /// Live logical inner sizes persisted after the user resizes each viewport.
    main_window_size: egui::Vec2,
    abyss_window_size: egui::Vec2,
    hit_detail_window_size: egui::Vec2,
    team_hit_detail_window_size: egui::Vec2,
    console_window_size: egui::Vec2,
    /// Guards root-size tracking while Windows applies the HUD exit resize.
    main_size_restore_frames: u8,
    /// Last root minimum size sent to egui, avoiding duplicate viewport commands.
    applied_main_min_size: egui::Vec2,
    opacity_reapply_frames: u8,
}

impl WindowState {
    fn from_config(config: &UiConfig) -> Self {
        Self {
            hud_mode: false,
            hud_size_key: None,
            abyss_overview_open: false,
            abyss_overview_corner_applied: false,
            abyss_overview_geometry: SecondaryViewportGeometry::default(),
            hit_detail_char_id: None,
            hit_detail_corner_applied: false,
            hit_detail_geometry: SecondaryViewportGeometry::default(),
            team_hit_detail_open: false,
            team_hit_detail_corner_applied: false,
            team_hit_detail_geometry: SecondaryViewportGeometry::default(),
            console_open: false,
            console_corner_applied: false,
            console_geometry: SecondaryViewportGeometry::default(),
            applied_opacity: None,
            corner_applied_hwnd: None,
            main_window_size: config
                .main_window_size
                .map(egui::Vec2::from)
                .unwrap_or(MAIN_WINDOW_BASE_SIZE),
            abyss_window_size: config
                .abyss_window_size
                .map(egui::Vec2::from)
                .unwrap_or(ABYSS_WINDOW_BASE_SIZE),
            hit_detail_window_size: config
                .hit_detail_window_size
                .map(egui::Vec2::from)
                .unwrap_or(HIT_DETAIL_WINDOW_BASE_SIZE),
            team_hit_detail_window_size: config
                .team_hit_detail_window_size
                .map(egui::Vec2::from)
                .unwrap_or(TEAM_HIT_DETAIL_WINDOW_BASE_SIZE),
            console_window_size: config
                .console_window_size
                .map(egui::Vec2::from)
                .unwrap_or(CONSOLE_WINDOW_BASE_SIZE),
            main_size_restore_frames: 0,
            applied_main_min_size: egui::Vec2::ZERO,
            opacity_reapply_frames: 4,
        }
    }
}

struct UiPreferences {
    hud_config: HudConfig,
    language: Language,
    dark_mode: bool,
    theme_preset: ThemePreset,
    accent: AccentColor,
    density: UiDensity,
    reduce_motion: bool,
    always_on_top: bool,
    mouse_passthrough: bool,
    passthrough_hotkey: PassthroughHotkey,
    global_hotkeys: GlobalHotkeys,
    recording_hotkey: Option<GlobalHotkeyAction>,
    hotkey_hook_available: bool,
    onboarding_done: bool,
    onboarding_step: u8,
    onboarding_hotkey_preview_generation: u32,
    console_sidebar_migration_seen: bool,
    console_sidebar_manually_collapsed: bool,
    opacity: f32,
    hit_detail_columns: HitDetailColumnsConfig,
}

impl UiPreferences {
    fn from_config(config: &UiConfig) -> Self {
        Self {
            hud_config: config.hud.clone(),
            language: config.language,
            dark_mode: config.dark_mode,
            theme_preset: config.theme_preset,
            accent: config.accent,
            density: config.density,
            reduce_motion: config.reduce_motion,
            always_on_top: config.always_on_top,
            mouse_passthrough: false,
            passthrough_hotkey: config.passthrough_hotkey,
            global_hotkeys: config.global_hotkeys,
            recording_hotkey: None,
            hotkey_hook_available: false,
            onboarding_done: config.onboarding_done,
            onboarding_step: 0,
            onboarding_hotkey_preview_generation: 0,
            console_sidebar_migration_seen: config.console_sidebar_migration_seen,
            console_sidebar_manually_collapsed: false,
            opacity: config.opacity,
            hit_detail_columns: config.hit_detail_columns,
        }
    }
}

struct BackgroundTasks {
    resource_audit_sender: Sender<ResourceAuditSummary>,
    resource_audit_receiver: Receiver<ResourceAuditSummary>,
    resource_audit_thread: Option<thread::JoinHandle<()>>,
    diagnostics_sender: Sender<DiagnosticReport>,
    diagnostics_receiver: Receiver<DiagnosticReport>,
    diagnostics_thread: Option<thread::JoinHandle<()>>,
    diagnostics_running: bool,
    texture_load_receiver: Receiver<TextureLoad>,
    device_detection_receiver: Receiver<DeviceDetection>,
    awaiting_device_detection: bool,
    game_process_monitor_receiver: Receiver<Result<bool, String>>,
    game_process_monitor_stop: Sender<()>,
    game_process_monitor_thread: Option<thread::JoinHandle<()>>,
    game_process_monitor_error: Option<String>,
    pending_file_dialog: Option<PendingFileDialog>,
    pending_capture_export: Option<PendingCaptureExport>,
}

impl BackgroundTasks {
    fn new(
        resource_audit: (Sender<ResourceAuditSummary>, Receiver<ResourceAuditSummary>),
        diagnostics: (Sender<DiagnosticReport>, Receiver<DiagnosticReport>),
        texture_load_receiver: Receiver<TextureLoad>,
        device_detection_receiver: Receiver<DeviceDetection>,
        game_process_monitor: (
            Receiver<Result<bool, String>>,
            Sender<()>,
            thread::JoinHandle<()>,
        ),
    ) -> Self {
        let (resource_audit_sender, resource_audit_receiver) = resource_audit;
        let (diagnostics_sender, diagnostics_receiver) = diagnostics;
        let (game_process_monitor_receiver, game_process_monitor_stop, game_process_monitor_thread) =
            game_process_monitor;
        Self {
            resource_audit_sender,
            resource_audit_receiver,
            resource_audit_thread: None,
            diagnostics_sender,
            diagnostics_receiver,
            diagnostics_thread: None,
            diagnostics_running: false,
            texture_load_receiver,
            device_detection_receiver,
            awaiting_device_detection: true,
            game_process_monitor_receiver,
            game_process_monitor_stop,
            game_process_monitor_thread: Some(game_process_monitor_thread),
            game_process_monitor_error: None,
            pending_file_dialog: None,
            pending_capture_export: None,
        }
    }

    fn stop_game_process_monitor(&mut self) {
        let _ = self.game_process_monitor_stop.send(());
        if let Some(thread) = self.game_process_monitor_thread.take() {
            let _ = thread.join();
        }
    }

    fn join_pending_capture_export(&mut self) {
        if let Some(mut pending) = self.pending_capture_export.take()
            && let Some(thread) = pending.thread.take()
        {
            let _ = thread.join();
        }
    }
}

struct NotificationState {
    status: String,
    last_status_toast: String,
    status_toasts: VecDeque<StatusToast>,
    undo_states: HashMap<u64, UndoState>,
    next_toast_id: u64,
    /// Global notification capsule (its own top-center overlay viewport). When
    /// enabled it receives every notice instead of the in-window
    /// `status_toasts`; disabling it falls back to the legacy toasts.
    island: IslandState,
    island_enabled: bool,
    island_offset_x: f32,
    diagnostic: Option<String>,
    last_error: Option<String>,
    last_error_action: Option<ErrorAction>,
    last_error_viewport: egui::ViewportId,
    passthrough_notice: Option<PassthroughNotice>,
    pending_confirmation: Option<ConfirmationAction>,
    pending_confirmation_viewport: egui::ViewportId,
}

impl NotificationState {
    fn new(
        config: &UiConfig,
        status: String,
        diagnostic: Option<String>,
        last_error: Option<String>,
    ) -> Self {
        Self {
            last_status_toast: status.clone(),
            status,
            status_toasts: VecDeque::new(),
            undo_states: HashMap::new(),
            next_toast_id: 1,
            island: IslandState::new(),
            island_enabled: config.island_notifications,
            island_offset_x: config.island_offset_x,
            diagnostic,
            last_error,
            last_error_action: None,
            last_error_viewport: egui::ViewportId::ROOT,
            passthrough_notice: None,
            pending_confirmation: None,
            pending_confirmation_viewport: egui::ViewportId::ROOT,
        }
    }

    fn allocate_toast_id(&mut self) -> u64 {
        let id = self.next_toast_id;
        self.next_toast_id = self.next_toast_id.wrapping_add(1).max(1);
        id
    }
}

pub struct DpsApp {
    characters: Arc<HashMap<u32, CharacterInfo>>,
    ability_catalog: Arc<AbilityCatalog>,
    avatar_textures: HashMap<String, egui::TextureHandle>,
    attribute_textures: HashMap<String, egui::TextureHandle>,
    monster_textures: HashMap<String, egui::TextureHandle>,
    damage_digit_textures: HashMap<String, Vec<egui::TextureHandle>>,
    reaction_textures: HashMap<u8, Vec<egui::TextureHandle>>,
    equipment_catalog: Arc<EquipmentCatalog>,
    equipment_textures: HashMap<String, egui::TextureHandle>,
    equipment_plugin: EquipmentPluginClient,
    kongmu_ui: KongmuUiState,
    state: CombatState,
    combat_active: bool,
    last_combat_timestamp: Option<f64>,
    last_combat_activity: Option<Instant>,
    combat_start_generation: u32,
    combat_end_generation: u32,
    hidden_character_ids: HashSet<u32>,
    selected_abyss_half: AbyssHalf,
    abyss_compact_mode: bool,
    abyss_overview: AbyssOverviewState,
    history: HistoryState,
    resource_audit: ResourceAuditState,
    hit_detail_filter: HitDetailFilter,
    hit_detail_skill_filter: String,
    team_hit_detail_filter: HitDetailFilter,
    character_hit_cache: HitDetailCache,
    team_hit_cache: HitDetailCache,
    skill_summary_cache: SkillSummaryCache,
    timeline_cache: TimelineCache,
    timeline_view: TimelineViewState,
    skill_breakdown_cache: SkillBreakdownCache,
    selected_timeline_char: Option<u32>,
    selected_skill_breakdown_char: Option<u32>,
    detail_last_scroll_activity: Option<Instant>,
    capture_ui: CaptureUiState,
    windows: WindowState,
    preferences: UiPreferences,
    capture: Option<CaptureHandle>,
    raw_capture: Option<RawCaptureBuffer>,
    replay_stop: Option<Arc<AtomicBool>>,
    replay_thread: Option<thread::JoinHandle<()>>,
    sender: EngineEventSink,
    receiver: Receiver<EngineEvent>,
    debug_receiver: Receiver<EngineEvent>,
    diagnostics_report: Option<DiagnosticReport>,
    background_tasks: BackgroundTasks,
    paused_events: VecDeque<EngineEvent>,
    notifications: NotificationState,
    console_tab: ConsoleTab,
    command_palette: CommandPaletteState,
    debug_only_hits: bool,
    debug_search: String,
    character_editor: CharacterEditorState,
    encrypted_ini_editor: EncryptedIniEditorState,
    style_key_applied: Option<AppliedStyleKey>,
    session_epoch: u64,
    theme_transition_from: Option<Color32>,
    active_import: Option<ActiveImport>,
    engine_task_viewport: Option<egui::ViewportId>,
    saved_ui_config: UiConfig,
    pending_ui_config: Option<(UiConfig, Instant)>,
    ui_config_path: PathBuf,
    native_file_drop: NativeFileDrop,
    last_dropped_file: Option<(PathBuf, Instant)>,
    hotkey_receiver: Receiver<HotkeyEvent>,
    hotkey: HotkeyHandle,
}

// Submodules carved out of the original monolithic app.rs (Phase 1b).
// Method submodules attach `impl DpsApp` blocks; free-fn submodules are
// re-exported so every submodule and the tests share one flat `app`
// namespace.
mod abyss;
mod chrome;
mod commands;
mod console_view;
mod detail_panels;
mod diagnostics_ui;
mod editor;
mod history_ui;
mod hit_detail;
mod hud;
mod island;
mod kongmu;
mod lifecycle;
mod main_view;
mod motion;
mod resources;
mod theme;
mod timeline;

pub(crate) use abyss::*;
pub(crate) use chrome::*;
pub(crate) use commands::*;
pub(crate) use diagnostics_ui::*;
pub(crate) use editor::*;
pub(crate) use history_ui::*;
pub(crate) use hit_detail::*;
pub(crate) use hud::*;
pub(crate) use island::*;
pub(crate) use kongmu::*;
pub(crate) use resources::*;
pub(crate) use theme::*;
pub(crate) use timeline::*;

impl eframe::App for DpsApp {
    /// Clear to transparent alpha. HUD mode relies on this so empty pixels
    /// disappear while painted text/images stay opaque; normal mode covers the
    /// clear with opaque panels.
    fn clear_color(&self, _visuals: &egui::Visuals) -> [f32; 4] {
        egui::Rgba::TRANSPARENT.to_array()
    }

    fn logic(&mut self, ctx: &egui::Context, frame: &mut eframe::Frame) {
        if ctx.input(|input| input.viewport().minimized == Some(true)) {
            // eframe skips `App::ui` while the root viewport is minimized, which removes all
            // immediate child viewports from the native backend. Mark their first-frame setup as
            // pending so restored children receive their recorded normal geometry, maximized
            // state, and Win32 corner style.
            self.windows.console_corner_applied = false;
            self.windows.hit_detail_corner_applied = false;
            self.windows.team_hit_detail_corner_applied = false;
            self.windows.abyss_overview_corner_applied = false;
            // The island window is also torn down with the other immediate
            // viewports; drop the stale HWND so the replacement window gets
            // its overlay styles, position and click-through re-applied.
            self.notifications.island.invalidate_window();
        }
        let style_key = AppliedStyleKey {
            dark_mode: self.preferences.dark_mode,
            theme_preset: self.preferences.theme_preset,
            accent: self.preferences.accent,
            density: self.preferences.density,
            reduce_motion: self.preferences.reduce_motion,
        };
        if self.style_key_applied != Some(style_key) {
            configure_style(
                ctx,
                self.preferences.dark_mode,
                self.preferences.theme_preset,
                self.preferences.accent,
                self.preferences.density,
                self.preferences.reduce_motion,
            );
            self.style_key_applied = Some(style_key);
        }
        let _ = motion::animate_generation(
            ctx,
            "combat_start_pulse",
            self.combat_start_generation,
            motion::dur::SLOW,
            self.preferences.reduce_motion,
        );
        let _ = motion::animate_generation(
            ctx,
            "combat_end_bounce",
            self.combat_end_generation,
            motion::dur::SLOW,
            self.preferences.reduce_motion,
        );
        self.note_detail_scroll_activity(ctx);
        self.drain_events();
        self.update_combat_visual();
        self.drain_resource_audit();
        self.drain_capture_diagnostics();
        self.drain_texture_loads();
        self.drain_device_detection();
        self.drain_game_process_monitor();
        self.drain_hotkeys(ctx);
        self.process_file_drops(ctx, frame);
        self.poll_file_dialog(ctx);
        self.poll_capture_info_export(ctx);
        let hud_progress = motion::animate_bool(
            ctx,
            "hud_mode_transition",
            self.windows.hud_mode,
            motion::dur::SLOW,
            self.preferences.reduce_motion,
            motion::ease::standard,
        );
        let force_opacity = self.windows.opacity_reapply_frames > 0;
        apply_window_attributes(
            frame,
            WindowAttributeConfig {
                opacity: egui::lerp(self.preferences.opacity..=1.0, hud_progress),
                force_opacity,
                hud_overlay: hud_progress >= 0.5,
                passthrough: self.preferences.mouse_passthrough,
            },
            &mut self.windows.applied_opacity,
            &mut self.windows.corner_applied_hwnd,
        );
        self.windows.opacity_reapply_frames = self.windows.opacity_reapply_frames.saturating_sub(1);
        if self.capture.is_some() || self.replay_thread.is_some() {
            ctx.request_repaint_after(Duration::from_millis(100));
        }

        // Keep the native window geometry on the same transition clock as the HUD content. This
        // prevents the DirectComposition surface from snapping before the in-app transition has
        // caught up, while remaining distinct from interactive edge-drag resizing.
        if self.windows.hud_mode {
            let rows = self.hud_visible_row_count();
            let show_title = !self.preferences.mouse_passthrough;
            let show_status_row = self.hud_status_row_visible();
            let size_key = HudSizeKey {
                rows,
                show_title_strip: show_title,
                show_status_row,
                config: self.preferences.hud_config.clone(),
            };
            let target = hud_window_size(
                rows,
                show_title,
                show_status_row,
                &self.preferences.hud_config,
            );
            if hud_progress < 1.0 || self.windows.hud_size_key.as_ref() != Some(&size_key) {
                let normal = self
                    .windows
                    .main_window_size
                    .max(self.windows.applied_main_min_size);
                let size = normal + (target - normal) * hud_progress;
                ctx.send_viewport_cmd(egui::ViewportCommand::InnerSize(size));
                self.windows.hud_size_key = Some(size_key);
            }
        } else if let Some(size_key) = self.windows.hud_size_key.as_ref() {
            let hud = hud_window_size(
                size_key.rows,
                size_key.show_title_strip,
                size_key.show_status_row,
                &size_key.config,
            );
            let restore = self
                .windows
                .main_window_size
                .max(self.windows.applied_main_min_size);
            let size = restore + (hud - restore) * hud_progress;
            ctx.send_viewport_cmd(egui::ViewportCommand::InnerSize(size));
            if hud_progress <= 0.0 {
                self.windows.hud_size_key = None;
                self.windows.main_size_restore_frames = HUD_TRANSITION_TRACKING_GUARD_FRAMES;
            }
        }
        self.update_status_toast(ctx);
    }

    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        let hud_progress = motion::animate_bool(
            &ctx,
            "hud_mode_transition",
            self.windows.hud_mode,
            motion::dur::SLOW,
            self.preferences.reduce_motion,
            motion::ease::standard,
        );
        let content_opacity = if self.windows.hud_mode {
            hud_progress
        } else {
            1.0 - hud_progress
        };
        let normal_visibility = 1.0 - hud_progress;
        // HUD mode replaces the full title bar with a compact strip (drag · 穿透 ·
        // 退出) and strips the panel fill so only the game shows behind it.
        let show_hud_title = if self.windows.hud_mode {
            !self.preferences.mouse_passthrough
        } else {
            self.windows
                .hud_size_key
                .as_ref()
                .is_some_and(|key| key.show_title_strip)
                && hud_progress > 0.0
        };
        let title_height = MAIN_TITLE_BAR_HEIGHT * normal_visibility
            + if show_hud_title {
                24.0 * hud_progress
            } else {
                0.0
            };
        if title_height > 0.5 {
            let side_margin = (10.0 * normal_visibility).round() as i8;
            let vertical_margin = (4.0 * normal_visibility).round() as i8;
            let title_frame = egui::Frame::new()
                .fill(
                    ctx.global_style()
                        .visuals
                        .panel_fill
                        .gamma_multiply(normal_visibility),
                )
                .inner_margin(egui::Margin::symmetric(side_margin, vertical_margin));
            egui::Panel::top("custom_title_bar")
                .exact_size(title_height)
                .frame(title_frame)
                .show_inside(ui, |ui| {
                    if show_hud_title && hud_progress >= 0.5 {
                        ui.set_opacity(hud_progress);
                        self.hud_title_bar(ui);
                    } else {
                        ui.set_opacity(normal_visibility);
                        self.title_bar(ui);
                    }
                });
        }

        let theme = self.theme();
        let central_fill = if self.windows.hud_mode && !self.preferences.mouse_passthrough {
            mix_color(theme.bg, theme.hud.edit_bg, hud_progress)
        } else {
            theme.bg.gamma_multiply(normal_visibility)
        };
        let central_margin = egui::Margin {
            // Match the title bar's side margins while the normal layout is visible, then
            // collapse them with the HUD transition so geometry and paint stay synchronized.
            left: (10.0 * normal_visibility).round() as i8,
            right: (10.0 * normal_visibility).round() as i8,
            top: 0,
            bottom: (8.0 * normal_visibility).round() as i8,
        };
        egui::CentralPanel::default()
            .frame(
                egui::Frame::new()
                    .fill(central_fill)
                    .inner_margin(central_margin),
            )
            .show_inside(ui, |ui| {
                ui.set_opacity(content_opacity);
                if self.windows.hud_mode {
                    self.hud_panel(ui);
                } else {
                    self.animated_controls(ui);
                    if self.replay_thread.is_some() {
                        self.import_loading_content(ui);
                    } else {
                        if self.state.abyss.is_active() {
                            self.abyss_selector(ui);
                        }
                        self.animated_party_content(ui);
                    }
                }
            });

        // Native edge/corner drag-resize for the borderless main window, plus tracking its size so
        // it restores on the next launch. HUD edit mode only exposes horizontal grips because its
        // height follows the visible modules.
        if self.windows.hud_mode && !self.preferences.mouse_passthrough && hud_progress >= 1.0 {
            window_width_resize_grips(&ctx);
            if ctx.input(|input| input.pointer.any_down()) {
                let width = ctx
                    .content_rect()
                    .width()
                    .round()
                    .clamp(HUD_WIDTH_MIN as f32, HUD_WIDTH_MAX as f32)
                    as u16;
                if width != self.preferences.hud_config.width {
                    self.preferences.hud_config.width = width;
                }
            }
        } else if !self.windows.hud_mode {
            let maximized = ctx
                .input(|input| input.viewport().maximized)
                .unwrap_or(false);
            if self.windows.hud_size_key.is_some() {
                // The HUD→window transition is still driving `InnerSize`; its duration is
                // time-based, so a fixed frame guard would expire too early on high-refresh
                // displays and persist an in-between size as the user's normal window size.
            } else if self.windows.main_size_restore_frames > 0 {
                // A programmatic resize (e.g. the HUD-exit restore) is still being
                // applied by Windows. Skip both tracking — so the transient size is
                // not written back over `main_window_size` — and min enforcement, so
                // it does not clamp a larger restored size down to the minimum before
                // the restore lands.
                self.windows.main_size_restore_frames -= 1;
            } else {
                if !maximized {
                    // While maximized the inner size is the screen work area, not a
                    // size the user chose — persisting it would make the window reopen
                    // huge, so only the last restored size is tracked.
                    track_window_size(&ctx, &mut self.windows.main_window_size);
                }
                // Grow the window minimum to whatever the current-language toolbar
                // needs, and heal an undersized window, so the button groups can never
                // be squeezed into overlapping.
                self.enforce_main_min_size(&ctx, maximized);
            }
            if self.windows.hud_size_key.is_none() {
                window_resize_grips(&ctx);
            }
        }

        if self.windows.console_open {
            self.console_panel(&ctx);
        }
        if let Some(char_id) = self.windows.hit_detail_char_id {
            self.hit_detail_panel(&ctx, char_id);
        }
        if self.windows.team_hit_detail_open {
            self.team_hit_detail_panel(&ctx);
        }
        if self.windows.abyss_overview_open {
            self.abyss_overview_panel(&ctx);
        }
        if ctx.input(|input| !input.raw.hovered_files.is_empty()) {
            egui::Area::new(egui::Id::new("pcapng_drop_overlay"))
                .order(egui::Order::Foreground)
                .anchor(egui::Align2::CENTER_CENTER, egui::Vec2::ZERO)
                .show(&ctx, |ui| {
                    egui::Frame::popup(ui.style())
                        .fill(self.theme().card)
                        .stroke(Stroke::new(2.0_f32, self.theme().accent))
                        .inner_margin(egui::Margin::symmetric(28, 20))
                        .show(ui, |ui| {
                            ui.label(
                                RichText::new(t("Release to import PCAPNG / JSON"))
                                    .size(18.0)
                                    .strong()
                                    .color(self.theme().accent),
                            );
                        });
                });
        }
        self.show_status_toast(&ctx);
        self.show_island(&ctx);
        self.show_passthrough_notice(&ctx);
        self.show_command_palette(&ctx);
        self.paint_theme_transition(&ctx);
        self.show_onboarding(&ctx);
        self.show_viewport_dialogs(&ctx);
        self.persist_ui_config();
        if self.capture.is_none()
            && self.replay_thread.is_none()
            && let Some((_, save_at)) = &self.pending_ui_config
        {
            ctx.request_repaint_after(save_at.saturating_duration_since(Instant::now()));
        }
    }
}

impl Drop for DpsApp {
    fn drop(&mut self) {
        self.background_tasks.stop_game_process_monitor();
        self.persist_ui_config_on_shutdown();
        self.stop_engine();
        self.background_tasks.join_pending_capture_export();
    }
}

#[cfg(test)]
mod tests {
    use super::{
        AbyssOverviewState, BackgroundTasks, CaptureUiState, ConsoleTab, DpsApp, HitDetailFilter,
        NotificationState, PendingCaptureExport, QteTypeFilterSummary, SkillBreakdownCache,
        SkillDamageSummary, SkillSummaryCache, TimelineCache, UiConfigSavePlan, UiPreferences,
        WindowState, adjusted_cached_index, build_team_dps_export, cached_hit_row, character_color,
        compare_cached_team_hits, damage_digit_key_for_hit, damage_digit_resource_path,
        damage_number_digits_text, fill_missing_character_colors_from_avatars,
        follow_up_damage_digit_key_for_hit, hit_detail_filter_available, hit_type_display_text,
        hit_type_label, is_party_member_row, mixed_damage_digit_key, parse_hex_color,
        qte_type_filter_label, reaction_text_key_for_hit,
        reaction_text_key_from_trigger_attack_type, reaction_text_resource_path,
        resolve_cached_hit, skill_display_name, snapshot_team_from_stats,
        summarize_qte_type_filters, translate_reaction_label,
    };
    use crate::engine::model::{
        CaptureQualitySource, CharacterInfo, CharacterStats, CombatSessionSkillSummary,
        CombatState, Hit, HitCharacterSource, HitDirection, SkillBreakdown, SkillBreakdownRow,
        TeamDps, TeamDpsMember, TimelineBucket, TimelineRoleBucket, TimelineSeries,
        UNBALANCE_ATTACK_TYPE,
    };
    use crate::storage::config::{
        AccentColor, DpsTimeMode, GlobalHotkeys, HitDetailColumnsConfig, HudConfig,
        PassthroughHotkey, ThemePreset, TimelineDpsViewMode, UiConfig, UiDensity,
    };
    use crate::storage::i18n::Language;
    use crate::support::encrypted_ini::{
        EncryptedIniKey, decrypt_encrypted_ini_text, encrypt_aes256_ecb,
        encrypt_encrypted_ini_records, encrypt_encrypted_ini_text, encrypted_ini_search_matches,
        encrypted_ini_text_fingerprint, parse_encrypted_ini_text, pkcs7_pad,
    };
    use base64::Engine as _;
    use base64::engine::general_purpose::STANDARD as BASE64;
    use eframe::egui;
    use std::collections::{HashMap, VecDeque};
    use std::sync::Arc;
    use std::time::{Duration, Instant};

    #[test]
    fn console_always_exposes_capture_diagnostics() {
        let tabs = ConsoleTab::visible_tabs();
        assert!(tabs.contains(&ConsoleTab::Packets));
        assert!(tabs.contains(&ConsoleTab::Resources));
        assert!(tabs.contains(&ConsoleTab::Diagnostics));
    }

    #[test]
    fn capture_ui_state_uses_configured_policy_and_fresh_runtime_state() {
        let config = UiConfig {
            manual_capture_device: Some("capture-device".to_owned()),
            server_damage_calibration: false,
            dps_time_mode: DpsTimeMode::RealTime,
            timeline_bucket_seconds: 2.5,
            timeline_dps_view_mode: TimelineDpsViewMode::Characters,
            ..UiConfig::default()
        };

        let state = CaptureUiState::from_config(&config);

        assert_eq!(
            state.manual_capture_device.as_deref(),
            Some("capture-device")
        );
        assert!(!state.server_damage_calibration);
        assert_eq!(state.dps_time_mode, DpsTimeMode::RealTime);
        assert_eq!(state.timeline_bucket_seconds, 2.5);
        assert_eq!(
            state.timeline_dps_view_mode,
            TimelineDpsViewMode::Characters
        );
        assert!(state.devices.is_empty());
        assert_eq!(state.selected_device, 0);
        assert_eq!(state.filter, "udp");
        assert!(state.active_capture_filter.is_none());
        assert_eq!(state.capture_quality_source, CaptureQualitySource::Unknown);
        assert!(state.include_incoming);
        assert!(state.capture_log_stats.is_none());
        assert!(!state.paused);
        assert_eq!(state.dropped_debug_packets, 0);
    }

    #[test]
    fn window_state_restores_sizes_and_starts_with_closed_viewports() {
        let defaults = WindowState::from_config(&UiConfig::default());
        assert_eq!(defaults.main_window_size, super::MAIN_WINDOW_BASE_SIZE);
        assert_eq!(defaults.abyss_window_size, super::ABYSS_WINDOW_BASE_SIZE);
        assert_eq!(
            defaults.hit_detail_window_size,
            super::HIT_DETAIL_WINDOW_BASE_SIZE
        );
        assert_eq!(
            defaults.team_hit_detail_window_size,
            super::TEAM_HIT_DETAIL_WINDOW_BASE_SIZE
        );
        assert_eq!(
            defaults.console_window_size,
            super::CONSOLE_WINDOW_BASE_SIZE
        );

        let config = UiConfig {
            main_window_size: Some([640.0, 480.0]),
            abyss_window_size: Some([900.0, 700.0]),
            hit_detail_window_size: Some([920.0, 710.0]),
            team_hit_detail_window_size: Some([880.0, 680.0]),
            console_window_size: Some([860.0, 620.0]),
            ..UiConfig::default()
        };

        let state = WindowState::from_config(&config);

        assert_eq!(state.main_window_size, egui::vec2(640.0, 480.0));
        assert_eq!(state.abyss_window_size, egui::vec2(900.0, 700.0));
        assert_eq!(state.hit_detail_window_size, egui::vec2(920.0, 710.0));
        assert_eq!(state.team_hit_detail_window_size, egui::vec2(880.0, 680.0));
        assert_eq!(state.console_window_size, egui::vec2(860.0, 620.0));
        assert!(!state.hud_mode);
        assert!(state.hud_size_key.is_none());
        assert!(!state.abyss_overview_open);
        assert!(state.hit_detail_char_id.is_none());
        assert!(!state.team_hit_detail_open);
        assert!(!state.console_open);
        assert_eq!(state.abyss_overview_geometry, Default::default());
        assert_eq!(state.hit_detail_geometry, Default::default());
        assert_eq!(state.team_hit_detail_geometry, Default::default());
        assert_eq!(state.console_geometry, Default::default());
        assert!(state.applied_opacity.is_none());
        assert!(state.corner_applied_hwnd.is_none());
        assert_eq!(state.main_size_restore_frames, 0);
        assert_eq!(state.applied_main_min_size, egui::Vec2::ZERO);
        assert_eq!(state.opacity_reapply_frames, 4);
    }

    #[test]
    fn ui_preferences_restore_config_and_reset_runtime_editing_state() {
        let hud = HudConfig::minimal();
        let columns = HitDetailColumnsConfig {
            show_time: false,
            character_width: 180,
            ..HitDetailColumnsConfig::default()
        };
        let global_hotkeys = GlobalHotkeys {
            enabled: false,
            capture: None,
            reset: None,
            hud: None,
        };
        let config = UiConfig {
            language: Language::Japanese,
            opacity: 0.72,
            dark_mode: true,
            theme_preset: ThemePreset::Tactical,
            accent: AccentColor::Violet,
            density: UiDensity::Comfortable,
            reduce_motion: true,
            always_on_top: false,
            hud: hud.clone(),
            hit_detail_columns: columns,
            passthrough_hotkey: PassthroughHotkey::F8,
            global_hotkeys,
            onboarding_done: true,
            console_sidebar_migration_seen: true,
            ..UiConfig::default()
        };

        let preferences = UiPreferences::from_config(&config);

        assert_eq!(preferences.hud_config, hud);
        assert_eq!(preferences.language, Language::Japanese);
        assert!(preferences.dark_mode);
        assert_eq!(preferences.theme_preset, ThemePreset::Tactical);
        assert_eq!(preferences.accent, AccentColor::Violet);
        assert_eq!(preferences.density, UiDensity::Comfortable);
        assert!(preferences.reduce_motion);
        assert!(!preferences.always_on_top);
        assert_eq!(preferences.passthrough_hotkey, PassthroughHotkey::F8);
        assert_eq!(preferences.global_hotkeys, global_hotkeys);
        assert!(preferences.onboarding_done);
        assert!(preferences.console_sidebar_migration_seen);
        assert_eq!(preferences.opacity, 0.72);
        assert_eq!(preferences.hit_detail_columns, columns);
        assert!(!preferences.mouse_passthrough);
        assert!(preferences.recording_hotkey.is_none());
        assert!(!preferences.hotkey_hook_available);
        assert_eq!(preferences.onboarding_step, 0);
        assert_eq!(preferences.onboarding_hotkey_preview_generation, 0);
        assert!(!preferences.console_sidebar_manually_collapsed);
    }

    #[test]
    fn notification_state_restores_island_preferences_and_starts_clean() {
        let config = UiConfig {
            island_notifications: false,
            island_offset_x: 42.5,
            ..UiConfig::default()
        };
        let mut notifications = NotificationState::new(
            &config,
            "Ready".to_owned(),
            Some("Network warning".to_owned()),
            Some("Startup warning".to_owned()),
        );

        assert_eq!(notifications.status, "Ready");
        assert_eq!(notifications.last_status_toast, "Ready");
        assert!(notifications.status_toasts.is_empty());
        assert!(notifications.undo_states.is_empty());
        assert!(!notifications.island_enabled);
        assert_eq!(notifications.island_offset_x, 42.5);
        assert_eq!(notifications.diagnostic.as_deref(), Some("Network warning"));
        assert_eq!(notifications.last_error.as_deref(), Some("Startup warning"));
        assert!(notifications.last_error_action.is_none());
        assert_eq!(notifications.last_error_viewport, egui::ViewportId::ROOT);
        assert!(notifications.passthrough_notice.is_none());
        assert!(notifications.pending_confirmation.is_none());
        assert_eq!(
            notifications.pending_confirmation_viewport,
            egui::ViewportId::ROOT
        );

        assert_eq!(notifications.allocate_toast_id(), 1);
        assert_eq!(notifications.allocate_toast_id(), 2);
        notifications.next_toast_id = u64::MAX;
        assert_eq!(notifications.allocate_toast_id(), u64::MAX);
        assert_eq!(notifications.next_toast_id, 1);
    }

    #[test]
    fn background_tasks_start_idle_and_join_owned_workers() {
        let resource_audit = crossbeam_channel::unbounded();
        let diagnostics = crossbeam_channel::unbounded();
        let (_texture_sender, texture_receiver) = crossbeam_channel::unbounded();
        let (_device_sender, device_receiver) = crossbeam_channel::unbounded();
        let (_monitor_sender, monitor_receiver) = crossbeam_channel::unbounded();
        let (monitor_stop, monitor_stop_receiver) = crossbeam_channel::unbounded();
        let monitor_thread = std::thread::spawn(move || {
            let _ = monitor_stop_receiver.recv();
        });
        let mut tasks = BackgroundTasks::new(
            resource_audit,
            diagnostics,
            texture_receiver,
            device_receiver,
            (monitor_receiver, monitor_stop, monitor_thread),
        );

        assert!(tasks.resource_audit_thread.is_none());
        assert!(tasks.diagnostics_thread.is_none());
        assert!(!tasks.diagnostics_running);
        assert!(tasks.awaiting_device_detection);
        assert!(tasks.game_process_monitor_thread.is_some());
        assert!(tasks.game_process_monitor_error.is_none());
        assert!(tasks.pending_file_dialog.is_none());
        assert!(tasks.pending_capture_export.is_none());

        let (_export_sender, export_receiver) = crossbeam_channel::unbounded();
        tasks.pending_capture_export = Some(PendingCaptureExport {
            viewport: egui::ViewportId::ROOT,
            receiver: export_receiver,
            thread: Some(std::thread::spawn(|| {})),
        });
        tasks.stop_game_process_monitor();
        tasks.join_pending_capture_export();

        assert!(tasks.game_process_monitor_thread.is_none());
        assert!(tasks.pending_capture_export.is_none());
    }

    #[test]
    #[ignore = "manual performance probe for cached derived-view cloning"]
    fn profile_cached_derived_view_clone_cost() {
        const ITERATIONS: usize = 500;

        let skill_summary_cache = SkillSummaryCache {
            key: None,
            rows: Arc::new(
                (0..512)
                    .map(|index| SkillDamageSummary {
                        name: format!("skill-{index}"),
                        category: format!("category-{}", index % 8),
                        hits: index as u64,
                        damage: index as f64 * 100.0,
                    })
                    .collect(),
            ),
            dirty_since: None,
        };
        let timeline_cache = TimelineCache {
            key: None,
            series: Arc::new(TimelineSeries {
                buckets: (0..3_600)
                    .map(|index| TimelineBucket {
                        start_offset: index as f64,
                        end_offset: index as f64 + 1.0,
                        role_damage: (0..4)
                            .map(|role| TimelineRoleBucket {
                                char_id: role,
                                char_name: format!("character-{role}"),
                                damage: index as f64 * 10.0,
                                dps: index as f64 * 10.0,
                            })
                            .collect(),
                        ..TimelineBucket::default()
                    })
                    .collect(),
                ..TimelineSeries::default()
            }),
        };
        let skill_breakdown_cache = SkillBreakdownCache {
            key: None,
            breakdown: Arc::new(SkillBreakdown {
                rows: (0..512)
                    .map(|index| SkillBreakdownRow {
                        char_id: (index % 4) as u32,
                        char_name: format!("character-{}", index % 4),
                        name: format!("skill-{index}"),
                        category: format!("category-{}", index % 8),
                        ability_name: Some(format!("ability-{index}")),
                        damage_name: Some(format!("damage-{index}")),
                        gameplay_effect_index: Some(index as u32),
                        gameplay_effect_name: Some(format!("effect-{index}")),
                        is_follow_up: false,
                        hits: index as u64,
                        damage: index as f64 * 100.0,
                    })
                    .collect(),
                ..SkillBreakdown::default()
            }),
        };

        let started = Instant::now();
        for _ in 0..ITERATIONS {
            std::hint::black_box(skill_summary_cache.rows.clone());
        }
        let skill_summaries = started.elapsed();

        let started = Instant::now();
        for _ in 0..ITERATIONS {
            std::hint::black_box(timeline_cache.series.clone());
        }
        let timeline = started.elapsed();

        let started = Instant::now();
        for _ in 0..ITERATIONS {
            std::hint::black_box(skill_breakdown_cache.breakdown.clone());
        }
        let skill_breakdown = started.elapsed();

        println!(
            "cached derived-view clones x{ITERATIONS}: skill summaries {skill_summaries:?}, timeline {timeline:?}, skill breakdown {skill_breakdown:?}"
        );
    }

    fn hit_with_direction(direction: &str) -> Hit {
        Hit {
            timestamp: 0.0,
            char_id: 1,
            char_name: "角色".to_owned(),
            char_known: true,
            damage: 1.0,
            byte_offset: 0,
            bit_shift: 0,
            char_source: HitCharacterSource::Unknown,
            direction: HitDirection::try_from(direction).expect("test direction must be valid"),
            target_hp_before: 0.0,
            target_hp_after: 0.0,
            target_max_hp: 0.0,
            target_hp_percent: 0.0,
            target_id: None,
            target_name: None,
            target_context: Vec::new(),
            gameplay_effect_index: None,
            gameplay_effect_name: None,
            ability_name: None,
            damage_name: Some("招式".to_owned()),
            attack_type: None,
            damage_attribute: None,
            follow_up_damage: 0.0,
            follow_up_timestamp: None,
            follow_up_damage_name: None,
            follow_up_attack_type: None,
            follow_up_damage_attribute: None,
        }
    }

    #[test]
    fn cached_index_tracks_front_trimming_during_deferred_refresh() {
        assert_eq!(
            adjusted_cached_index(45_000, 50_000, 50_000, 20),
            Some(44_980)
        );
        assert_eq!(adjusted_cached_index(10, 50_000, 50_000, 20), None);
        assert_eq!(
            adjusted_cached_index(9_000, 10_000, 10_020, 20),
            Some(9_000)
        );
    }

    #[test]
    fn cached_hit_resolves_when_generation_changes_without_appending() {
        let hit = hit_with_direction("outgoing");
        let row = cached_hit_row(0, &hit);
        let mut hits = VecDeque::from([hit]);
        hits[0].follow_up_damage = 921.0;

        let resolved = resolve_cached_hit(&hits, &row, 1, 1)
            .expect("same-index hit should resolve after in-place follow-up update");
        assert_eq!(resolved.follow_up_damage, 921.0);
    }

    #[test]
    fn history_skill_display_prefixes_unknown_descriptions_with_character() {
        let english = CombatSessionSkillSummary {
            name: "GA_Shinku_UltraSkill".to_owned(),
            category: "技能".to_owned(),
            ability_name: None,
            gameplay_effect_name: None,
            damage_name: None,
            char_id: 1076,
            char_name: "真红".to_owned(),
            damage: 593_779.0,
            hits: 29,
            damage_share_percent: 31.2,
            is_follow_up: false,
        };
        assert_eq!(skill_display_name(&english), "真红 · 大招");

        let chinese = CombatSessionSkillSummary {
            name: "安魂曲流血伤害L".to_owned(),
            char_name: "安魂曲".to_owned(),
            ..english
        };
        assert_eq!(skill_display_name(&chinese), "安魂曲流血伤害L");
    }

    #[test]
    fn team_hit_rows_order_higher_hp_first_within_same_second() {
        let mut high_hp = hit_with_direction("outgoing");
        high_hp.timestamp = 1.9;
        high_hp.target_hp_after = 30_000.0;
        high_hp.target_max_hp = 100_000.0;
        high_hp.target_hp_percent = 30.0;
        let mut low_hp = hit_with_direction("outgoing");
        low_hp.timestamp = 1.1;
        low_hp.target_hp_after = 7_000.0;
        low_hp.target_max_hp = 100_000.0;
        low_hp.target_hp_percent = 7.0;

        let mut rows = [cached_hit_row(0, &low_hp), cached_hit_row(1, &high_hp)];
        rows.sort_by(compare_cached_team_hits);

        assert_eq!(rows[0].target_hp_after, 30_000.0);
        assert_eq!(rows[1].target_hp_after, 7_000.0);
    }

    #[test]
    fn unknown_hit_uses_candidate_output_label() {
        let outgoing_hit = hit_with_direction("outgoing");
        let incoming_hit = hit_with_direction("incoming");
        let unknown_hit = hit_with_direction("unknown");
        let outgoing = hit_type_label(&outgoing_hit);
        let incoming = hit_type_label(&incoming_hit);
        let unknown = hit_type_label(&unknown_hit);
        assert!(!outgoing.is_empty());
        assert!(!incoming.is_empty());
        assert!(!unknown.is_empty());
        assert_ne!(unknown, incoming);
    }

    #[test]
    fn hit_type_display_text_joins_attack_type_and_skill_name() {
        let mut hit = hit_with_direction("outgoing");
        hit.attack_type = Some("普攻".to_owned());
        hit.damage_name = Some("酸甜口味的制裁".to_owned());

        assert_eq!(hit_type_display_text(&hit), "Basic Attack·酸甜口味的制裁");
    }

    #[test]
    fn hit_type_display_text_prefers_live_resolved_name_over_stale_stored_one() {
        crate::storage::ability_names::set_for_test(
            std::collections::HashMap::from([(
                "GE_Player_Sagiri_UltraSkill1_Damage".to_owned(),
                crate::engine::parser::GameplayEffectSkill {
                    damage_source_category: Some("Q".to_owned()),
                    ability_name: Some("GA_Sagiri_UltraSkill".to_owned()),
                    attack_type: "Q技能".to_owned(),
                },
            )]),
            std::collections::HashMap::from([(
                "GA_Sagiri_UltraSkill".to_owned(),
                "現在の言語での技名".to_owned(),
            )]),
        );

        let mut hit = hit_with_direction("outgoing");
        hit.attack_type = Some("Q技能".to_owned());
        // damage_name still holds whatever language was active when this hit was
        // originally captured; the live lookup (keyed by gameplay_effect_name)
        // must win instead of this stale value.
        hit.damage_name = Some("Feast of Gluttony".to_owned());
        hit.gameplay_effect_name = Some("GE_Player_Sagiri_UltraSkill1_Damage".to_owned());

        assert_eq!(hit_type_display_text(&hit), "Ultimate·現在の言語での技名");
    }

    #[test]
    fn hit_type_display_text_falls_back_to_whichever_half_is_present() {
        let mut attack_type_only = hit_with_direction("outgoing");
        attack_type_only.attack_type = Some("E技能".to_owned());
        attack_type_only.damage_name = None;
        assert_eq!(hit_type_display_text(&attack_type_only), "Skill");

        let mut name_only = hit_with_direction("outgoing");
        name_only.attack_type = None;
        name_only.damage_name = Some("判予秋".to_owned());
        assert_eq!(hit_type_display_text(&name_only), "判予秋");
    }

    #[test]
    fn hit_type_display_text_does_not_repeat_identical_halves() {
        let mut hit = hit_with_direction("outgoing");
        hit.attack_type = Some("倾陷伤害".to_owned());
        hit.damage_name = Some("倾陷伤害".to_owned());

        // "Break Damage" is the untranslated English key: reaction/category labels
        // route through `t()`, which returns the key as-is when no locale overlay
        // is loaded (as in this test's environment).
        assert_eq!(hit_type_display_text(&hit), "Break Damage");
    }

    #[test]
    fn translate_reaction_label_covers_conditions_and_qte_prefix() {
        assert_eq!(translate_reaction_label("创生花"), "Blossom Damage");
        assert_eq!(translate_reaction_label("覆纹"), "Hexed");
        assert_eq!(
            translate_reaction_label("环合·创生"),
            "Esper Cycle · Blossom"
        );
        assert_eq!(translate_reaction_label("环合·黯星"), "Esper Cycle · Nova");
        assert_eq!(translate_reaction_label("普攻"), "Basic Attack");
        assert_eq!(translate_reaction_label("Q技能"), "Ultimate");
        // Move names have no category match, so they pass through unchanged.
        assert_eq!(translate_reaction_label("酸甜口味的制裁"), "酸甜口味的制裁");
    }

    #[test]
    fn reaction_text_key_only_marks_reaction_trigger_hits() {
        assert_eq!(
            reaction_text_key_from_trigger_attack_type("环合·覆纹"),
            Some(2)
        );
        assert_eq!(reaction_text_key_from_trigger_attack_type("覆纹"), None);
        assert_eq!(reaction_text_key_from_trigger_attack_type("创生花"), None);
        assert_eq!(
            reaction_text_key_from_trigger_attack_type("环合·黯星"),
            Some(3)
        );
        assert_eq!(
            reaction_text_key_from_trigger_attack_type("环合·浊燃"),
            Some(4)
        );
        assert_eq!(
            reaction_text_key_from_trigger_attack_type("环合·浸染"),
            Some(5)
        );
        assert_eq!(
            reaction_text_key_from_trigger_attack_type("环合·延滞"),
            Some(6)
        );
        assert_eq!(
            reaction_text_key_from_trigger_attack_type("环合·盈蓄"),
            Some(7)
        );
        assert_eq!(
            reaction_text_key_from_trigger_attack_type("环合·失谐"),
            Some(8)
        );

        let mut hit = hit_with_direction("outgoing");
        hit.gameplay_effect_name = Some("GE_ActorReaction_1_Damage".to_owned());
        hit.attack_type = Some("创生花".to_owned());
        assert_eq!(reaction_text_key_for_hit(&hit), None);

        hit.attack_type = Some("环合·覆纹".to_owned());
        assert_eq!(reaction_text_key_for_hit(&hit), Some(2));
    }

    #[test]
    fn qte_type_filters_include_only_present_outgoing_qte_types() {
        let mut qte_a = hit_with_direction("outgoing");
        qte_a.attack_type = Some("覆纹".to_owned());
        qte_a.damage = 10.0;
        let mut qte_b = hit_with_direction("unknown");
        qte_b.attack_type = Some("创生花".to_owned());
        qte_b.damage = 20.0;
        let mut incoming_qte = hit_with_direction("incoming");
        incoming_qte.attack_type = Some("覆纹".to_owned());
        let mut entry_qte = hit_with_direction("outgoing");
        entry_qte.attack_type = Some("环合·覆纹".to_owned());
        let mut non_qte = hit_with_direction("outgoing");
        non_qte.attack_type = Some("普攻".to_owned());

        let hits = VecDeque::from([qte_a, qte_b, incoming_qte, entry_qte, non_qte]);
        let summaries = summarize_qte_type_filters(&hits, None);

        assert_eq!(summaries.len(), 2);
        assert_eq!(summaries[0].attack_type, "创生花");
        assert_eq!(summaries[0].hits, 1);
        assert_eq!(summaries[0].damage, 20.0);
        assert_eq!(summaries[1].attack_type, "覆纹");
        assert_eq!(summaries[1].hits, 1);
        assert_eq!(summaries[1].damage, 10.0);
        assert!(hit_detail_filter_available(
            &HitDetailFilter::QteType("覆纹".to_owned()),
            &summaries
        ));
        assert!(!hit_detail_filter_available(
            &HitDetailFilter::QteType("黯星".to_owned()),
            &summaries
        ));
    }

    #[test]
    fn qte_type_filters_include_unbalance_damage() {
        let mut unbalance = hit_with_direction("outgoing");
        unbalance.attack_type = Some(UNBALANCE_ATTACK_TYPE.to_owned());
        unbalance.damage = 62_966.0;
        let mut non_qte = hit_with_direction("outgoing");
        non_qte.attack_type = Some("普攻".to_owned());

        let hits = VecDeque::from([unbalance, non_qte]);
        let summaries = summarize_qte_type_filters(&hits, None);

        assert_eq!(summaries.len(), 1);
        assert_eq!(summaries[0].attack_type, UNBALANCE_ATTACK_TYPE);
        assert_eq!(summaries[0].hits, 1);
        assert_eq!(summaries[0].damage, 62_966.0);
        assert!(hit_detail_filter_available(
            &HitDetailFilter::QteType(UNBALANCE_ATTACK_TYPE.to_owned()),
            &summaries
        ));
        assert!(HitDetailFilter::QteType(UNBALANCE_ATTACK_TYPE.to_owned()).matches(&hits[0]));
    }

    #[test]
    fn qte_type_filters_include_merged_follow_up_damage() {
        let mut source = hit_with_direction("outgoing");
        source.attack_type = Some("Q技能".to_owned());
        source.follow_up_damage = 921.0;
        source.follow_up_attack_type = Some("覆纹".to_owned());
        source.follow_up_damage_attribute = Some("咒".to_owned());

        let hits = VecDeque::from([source]);
        let summaries = summarize_qte_type_filters(&hits, None);

        assert_eq!(summaries.len(), 1);
        assert_eq!(summaries[0].attack_type, "覆纹");
        assert_eq!(summaries[0].hits, 1);
        assert_eq!(summaries[0].damage, 921.0);
        assert!(HitDetailFilter::QteType("覆纹".to_owned()).matches(&hits[0]));
    }

    #[test]
    fn qte_type_filter_matches_only_that_attack_type() {
        let filter = HitDetailFilter::QteType("覆纹".to_owned());
        let mut matching = hit_with_direction("outgoing");
        matching.attack_type = Some("覆纹".to_owned());
        let mut different_qte = hit_with_direction("outgoing");
        different_qte.attack_type = Some("创生花".to_owned());
        let mut incoming = hit_with_direction("incoming");
        incoming.attack_type = Some("覆纹".to_owned());

        assert!(filter.matches(&matching));
        assert!(!filter.matches(&different_qte));
        assert!(!filter.matches(&incoming));
    }

    #[test]
    fn qte_type_filter_label_shows_damage_and_share() {
        let summary = QteTypeFilterSummary {
            attack_type: "覆纹".to_owned(),
            hits: 42,
            damage: 77_136.0,
        };

        assert_eq!(
            qte_type_filter_label(&summary, 1_928_356.0),
            "覆纹 77,136 · 4.0%"
        );
    }

    #[test]
    fn damage_digit_text_uses_plain_digits_for_image_rendering() {
        assert_eq!(damage_number_digits_text(77_136.2), "77136");
        assert_eq!(damage_number_digits_text(77_136.8), "77137");
        assert_eq!(
            damage_digit_resource_path("guang", 7),
            "res/images/font/tiaozi1/guang_7.png"
        );
        assert_eq!(
            reaction_text_resource_path(Language::SimplifiedChinese, 1, 1),
            "res/images/font/tiaozi1/zh/fanying01_01.png"
        );
        assert_eq!(
            reaction_text_resource_path(Language::English, 1, 1),
            "res/images/font/tiaozi1/en/fanying01_01.png"
        );
        assert_eq!(
            reaction_text_resource_path(Language::Japanese, 1, 1),
            "res/images/font/tiaozi1/ja/fanying01_01.png"
        );
    }

    #[test]
    fn damage_digit_key_prefers_special_damage_types() {
        let characters = HashMap::from([(
            1,
            CharacterInfo {
                name_zh: "角色".to_owned(),
                name_en: String::new(),
                color: None,
                avatar: None,
                attribute: Some("灵".to_owned()),
            },
        )]);
        let mut hit = hit_with_direction("outgoing");
        assert_eq!(damage_digit_key_for_hit(&hit, &characters), Some("灵"));

        hit.attack_type = Some("倾陷伤害".to_owned());
        assert_eq!(damage_digit_key_for_hit(&hit, &characters), Some("真实"));

        // 覆纹跳字固定为灵字系（lingzhou_L），即便伤害记给的是咒角色。
        hit.attack_type = Some("覆纹".to_owned());
        hit.damage_attribute = Some("咒".to_owned());
        assert_eq!(
            damage_digit_key_for_hit(&hit, &characters),
            Some("lingzhou_L")
        );

        hit.attack_type = Some("创生花".to_owned());
        hit.damage_attribute = Some("光".to_owned());
        assert_eq!(
            damage_digit_key_for_hit(&hit, &characters),
            Some("Guangling_G")
        );
        hit.attack_type = Some("载具伤害".to_owned());
        hit.damage_attribute = Some("物理".to_owned());
        assert_eq!(damage_digit_key_for_hit(&hit, &characters), Some("物理"));

        hit.direction = HitDirection::Incoming;
        hit.attack_type = Some("覆纹".to_owned());
        hit.damage_attribute = Some("咒".to_owned());
        assert_eq!(damage_digit_key_for_hit(&hit, &characters), Some("HP"));

        hit.direction = HitDirection::Outgoing;
        hit.follow_up_attack_type = Some("覆纹".to_owned());
        hit.follow_up_damage_attribute = Some("咒".to_owned());
        assert_eq!(follow_up_damage_digit_key_for_hit(&hit), Some("lingzhou_L"));
    }

    #[test]
    fn reaction_burst_digit_key_locks_to_trigger_side_series() {
        // 环合反应伤害本体（不带 "环合·" 前缀）的跳字固定为触发侧属性字系，
        // 无论记给环合双方哪一侧，也无论是否解析出 source_attribute。
        for attribute in [None, Some("光"), Some("灵")] {
            assert_eq!(
                mixed_damage_digit_key("创生花", attribute),
                Some("Guangling_G")
            );
        }
        for attribute in [None, Some("灵"), Some("咒")] {
            assert_eq!(
                mixed_damage_digit_key("覆纹", attribute),
                Some("lingzhou_L")
            );
        }
        for attribute in [None, Some("暗"), Some("魂")] {
            assert_eq!(mixed_damage_digit_key("黯星", attribute), Some("Anhun_A"));
        }
        for attribute in [None, Some("暗"), Some("咒")] {
            assert_eq!(mixed_damage_digit_key("浊燃", attribute), Some("Zhouan_A"));
        }
    }

    #[test]
    fn reaction_trigger_hit_digit_key_uses_caster_attribute() {
        // 触发环合的那一下伤害（"环合·xxx"）走造成伤害角色自身属性字系，
        // mixed_damage_digit_key 返回 None，由调用方回退到 source_attribute。
        for trigger in ["环合·创生", "环合·黯星", "环合·浊燃", "环合·覆纹"] {
            assert_eq!(mixed_damage_digit_key(trigger, Some("灵")), None);
        }
        // 端到端：娜娜莉（灵）触发创生的那一下，跳字应为灵字系而非创生字系。
        let characters = HashMap::from([(
            1010,
            CharacterInfo {
                name_zh: "娜娜莉".to_owned(),
                name_en: String::new(),
                color: None,
                avatar: None,
                attribute: Some("灵".to_owned()),
            },
        )]);
        let mut trigger_hit = hit_with_direction("outgoing");
        trigger_hit.char_id = 1010;
        trigger_hit.attack_type = Some("环合·创生".to_owned());
        assert_eq!(
            damage_digit_key_for_hit(&trigger_hit, &characters),
            Some("灵")
        );
    }

    #[test]
    fn party_member_rows_hide_qte_follow_up_pseudo_characters() {
        let mut real_hit = hit_with_direction("outgoing");
        real_hit.char_id = 1;
        real_hit.attack_type = Some("普攻".to_owned());
        let mut follow_up = hit_with_direction("outgoing");
        follow_up.char_id = 999_999;
        follow_up.char_known = false;
        follow_up.attack_type = Some("覆纹".to_owned());

        let hits = VecDeque::from([real_hit, follow_up]);

        assert!(is_party_member_row(
            &CharacterStats {
                char_id: 1,
                ..Default::default()
            },
            &hits
        ));
        assert!(!is_party_member_row(
            &CharacterStats {
                char_id: 999_999,
                ..Default::default()
            },
            &hits
        ));
    }

    #[test]
    fn team_dps_snapshot_uses_team_duration_and_top_party_members() {
        let stats = [
            CharacterStats {
                char_id: 1,
                name: "一".to_owned(),
                damage: 100.0,
                ..Default::default()
            },
            CharacterStats {
                char_id: 2,
                name: "二".to_owned(),
                damage: 200.0,
                ..Default::default()
            },
            CharacterStats {
                char_id: 3,
                name: "三".to_owned(),
                damage: 300.0,
                ..Default::default()
            },
            CharacterStats {
                char_id: 4,
                name: "四".to_owned(),
                damage: 400.0,
                ..Default::default()
            },
            CharacterStats {
                char_id: 5,
                name: "五".to_owned(),
                damage: 500.0,
                ..Default::default()
            },
            CharacterStats {
                char_id: 6,
                name: "零伤害".to_owned(),
                damage: 0.0,
                ..Default::default()
            },
            CharacterStats {
                char_id: 999_999,
                name: "伪角色".to_owned(),
                damage: 10_000.0,
                ..Default::default()
            },
        ];

        let team = snapshot_team_from_stats(150.0, 10.0, stats.iter()).unwrap();

        assert_eq!(team.dps, 150.0);
        assert_eq!(
            team.members
                .iter()
                .map(|member| member.id)
                .collect::<Vec<_>>(),
            vec![5, 4, 3, 2]
        );
        assert_eq!(
            team.members
                .iter()
                .map(|member| member.dps)
                .collect::<Vec<_>>(),
            vec![50.0, 40.0, 30.0, 20.0]
        );
    }

    #[test]
    fn team_dps_export_prefers_realtime_abyss_halves() {
        let mut state = CombatState::default();
        let mut upper_hit = hit_with_direction("outgoing");
        upper_hit.char_id = 10;
        upper_hit.char_name = "上行".to_owned();
        upper_hit.damage = 100.0;
        let mut lower_hit = hit_with_direction("outgoing");
        lower_hit.char_id = 20;
        lower_hit.char_name = "下行".to_owned();
        lower_hit.damage = 200.0;
        state.abyss.first_half.push_hit(upper_hit);
        state.abyss.second_half.push_hit(lower_hit);

        let overview = AbyssOverviewState {
            upper_team: Some(TeamDps {
                dps: 1.0,
                members: vec![TeamDpsMember {
                    id: 99,
                    dps: 1.0,
                    name: "旧预测".to_owned(),
                }],
            }),
            ..AbyssOverviewState::default()
        };

        let export = build_team_dps_export(&state, &overview, true).unwrap();

        assert!(export.single.is_none());
        assert_eq!(export.upper.unwrap().members[0].id, 10);
        assert_eq!(export.lower.unwrap().members[0].id, 20);
    }

    #[test]
    fn character_avatar_colors_fill_missing_table_colors() {
        let mut characters = HashMap::from([
            (
                1004,
                CharacterInfo {
                    name_zh: "安魂曲".to_owned(),
                    name_en: String::new(),
                    color: None,
                    avatar: Some("res/images/characters/player_004_256.png".to_owned()),
                    attribute: Some("暗".to_owned()),
                },
            ),
            (
                1020,
                CharacterInfo {
                    name_zh: "哈尼娅".to_owned(),
                    name_en: String::new(),
                    color: None,
                    avatar: Some("res/images/characters/player_020_256.png".to_owned()),
                    attribute: Some("魂".to_owned()),
                },
            ),
        ]);

        fill_missing_character_colors_from_avatars(&mut characters, std::path::Path::new("."));

        let first = character_color(1004, &characters, 0, false);
        let second = character_color(1020, &characters, 1, false);
        assert_ne!(first, second);
        assert!(characters.values().all(|character| {
            character
                .color
                .as_deref()
                .and_then(parse_hex_color)
                .is_some()
        }));
    }

    #[test]
    fn character_fallback_color_is_stable_across_rank_indices() {
        let characters = HashMap::new();
        assert_eq!(
            character_color(4242, &characters, 0, false),
            character_color(4242, &characters, 7, false)
        );
    }

    #[test]
    fn character_table_color_wins_over_avatar_color() {
        let mut characters = HashMap::from([(
            1010,
            CharacterInfo {
                name_zh: "娜娜莉".to_owned(),
                name_en: String::new(),
                color: Some("#123456".to_owned()),
                avatar: Some("res/images/characters/player_010_256.png".to_owned()),
                attribute: Some("灵".to_owned()),
            },
        )]);

        fill_missing_character_colors_from_avatars(&mut characters, std::path::Path::new("."));

        assert_eq!(
            characters
                .get(&1010)
                .and_then(|character| character.color.as_deref()),
            Some("#123456")
        );
        assert_eq!(
            character_color(1010, &characters, 0, false),
            egui::Color32::from_rgb(0x12, 0x34, 0x56)
        );
    }

    #[test]
    fn encrypted_ini_round_trips_with_china_key() {
        let plaintext = "[Core.System]\nGameName=NTE\nEndpoint=https://example.invalid";
        let encrypted = encrypt_encrypted_ini_text(plaintext, EncryptedIniKey::China)
            .expect("fixture should encrypt");
        assert!(encrypted.lines().all(|line| BASE64.decode(line).is_ok()));

        let (key, decrypted, encrypted_lines) =
            decrypt_encrypted_ini_text(&encrypted).expect("encrypted INI should decrypt");
        assert_eq!(key, EncryptedIniKey::China);
        assert_eq!(decrypted, plaintext);
        assert_eq!(encrypted_lines, 3);
    }

    #[test]
    fn encrypted_ini_save_preserves_unchanged_records() {
        let first = BASE64.encode(
            encrypt_aes256_ecb(
                &pkcs7_pad(b"[Setting]|SPLIT||SPLIT|Sound_MainVolumn=70|SPLIT|"),
                EncryptedIniKey::China.key(),
            )
            .expect("fixture should encrypt"),
        );
        let second = BASE64.encode(
            encrypt_aes256_ecb(
                &pkcs7_pad(b"FightDefaultArmLengthScale=0"),
                EncryptedIniKey::China.key(),
            )
            .expect("fixture should encrypt"),
        );
        let encrypted = format!("{first}\n{second}\n");
        let (key, plaintext, records, line_ending, final_newline) =
            parse_encrypted_ini_text(&encrypted).expect("fixture should decrypt");
        assert_eq!(
            plaintext,
            "[Setting]\nSound_MainVolumn=70\nFightDefaultArmLengthScale=0"
        );

        let modified = plaintext.replace(
            "FightDefaultArmLengthScale=0",
            "FightDefaultArmLengthScale=1",
        );
        let saved = encrypt_encrypted_ini_records(
            &modified,
            key,
            key,
            &records,
            &line_ending,
            final_newline,
        )
        .expect("fixture should encrypt");
        let saved_lines = saved.lines().collect::<Vec<_>>();
        assert_eq!(saved_lines[0], first);
        assert_ne!(saved_lines[1], second);

        let restored = encrypt_encrypted_ini_records(
            &plaintext,
            key,
            key,
            &records,
            &line_ending,
            final_newline,
        )
        .expect("fixture should encrypt");
        assert_eq!(restored, encrypted);
    }

    #[test]
    fn encrypted_ini_reencrypts_same_payload_to_same_ciphertext_after_reload() {
        let original_payload = "|SPLIT|FightDefaultArmLengthScale=0|SPLIT|";
        let changed_line = "FightDefaultArmLengthScale=1";
        let original = format!(
            "{}\n",
            BASE64.encode(
                encrypt_aes256_ecb(
                    &pkcs7_pad(original_payload.as_bytes()),
                    EncryptedIniKey::China.key(),
                )
                .expect("fixture should encrypt"),
            )
        );
        let (key, _, records, line_ending, final_newline) =
            parse_encrypted_ini_text(&original).expect("fixture should decrypt");

        let changed = encrypt_encrypted_ini_records(
            changed_line,
            key,
            key,
            &records,
            &line_ending,
            final_newline,
        )
        .expect("fixture should encrypt");
        assert_ne!(changed, original);
        let (reloaded_key, _, reloaded_records, reloaded_line_ending, reloaded_final_newline) =
            parse_encrypted_ini_text(&changed).expect("changed fixture should decrypt");
        let restored = encrypt_encrypted_ini_records(
            "FightDefaultArmLengthScale=0",
            reloaded_key,
            reloaded_key,
            &reloaded_records,
            &reloaded_line_ending,
            reloaded_final_newline,
        )
        .expect("fixture should encrypt");
        assert_eq!(restored, original);
    }

    #[test]
    fn encrypted_ini_save_preserves_crlf_and_missing_final_newline() {
        let first = BASE64.encode(
            encrypt_aes256_ecb(
                &pkcs7_pad(b"[Setting]|SPLIT|Sound_MainVolumn=70"),
                EncryptedIniKey::China.key(),
            )
            .expect("fixture should encrypt"),
        );
        let second = BASE64.encode(
            encrypt_aes256_ecb(
                &pkcs7_pad(b"FightDefaultArmLengthScale=0"),
                EncryptedIniKey::China.key(),
            )
            .expect("fixture should encrypt"),
        );
        let encrypted = format!("{first}\r\n{second}");
        let (key, plaintext, records, line_ending, final_newline) =
            parse_encrypted_ini_text(&encrypted).expect("fixture should decrypt");
        assert_eq!(line_ending, "\r\n");
        assert!(!final_newline);

        let saved = encrypt_encrypted_ini_records(
            &plaintext,
            key,
            key,
            &records,
            &line_ending,
            final_newline,
        )
        .expect("fixture should encrypt");
        assert_eq!(saved, encrypted);
        assert!(!saved.ends_with('\n'));
        assert!(saved.contains("\r\n"));
    }

    #[test]
    fn encrypted_ini_search_is_case_insensitive() {
        let text = "bUseHDR=True\nHDRDisplay=1\nOther=0";
        let matches = encrypted_ini_search_matches(text, "hdr");
        assert_eq!(matches.len(), 2);
        assert_eq!(&text[matches[0]..matches[0] + 3], "HDR");
    }

    #[test]
    fn encrypted_ini_text_fingerprint_tracks_same_length_edits() {
        assert_ne!(
            encrypted_ini_text_fingerprint("Alpha=1\nBeta=2"),
            encrypted_ini_text_fingerprint("Alpha=1\nBeta=3")
        );
    }

    #[test]
    fn ui_config_save_plan_debounces_first_dirty_state() {
        let now = Instant::now();
        let saved = UiConfig {
            opacity: 0.8,
            dark_mode: false,
            always_on_top: true,
            server_damage_calibration: false,
            ..UiConfig::default()
        };
        let current = UiConfig {
            opacity: 0.9,
            dark_mode: false,
            always_on_top: true,
            server_damage_calibration: false,
            ..UiConfig::default()
        };
        match DpsApp::ui_config_save_plan(&current, &saved, None, now) {
            UiConfigSavePlan::SetPending((pending, save_at)) => {
                assert_eq!(pending, current);
                assert!(save_at > now + Duration::from_millis(300));
            }
            _ => panic!("expected set-pending schedule"),
        }
    }

    #[test]
    fn ui_config_save_plan_retries_when_unmodified_pending_expires() {
        let now = Instant::now();
        let saved = UiConfig {
            opacity: 0.8,
            dark_mode: false,
            always_on_top: true,
            server_damage_calibration: false,
            ..UiConfig::default()
        };
        let current = UiConfig {
            opacity: 0.9,
            dark_mode: false,
            always_on_top: true,
            server_damage_calibration: false,
            ..UiConfig::default()
        };
        let future = now + Duration::from_millis(500);
        let pending = Some((current.clone(), future));
        match DpsApp::ui_config_save_plan(&current, &saved, pending.as_ref(), now) {
            UiConfigSavePlan::KeepPending((_, wait_until)) => {
                assert_eq!(wait_until, future);
            }
            _ => panic!("expected keep-pending state"),
        }

        let expired = now + Duration::from_millis(1000);
        match DpsApp::ui_config_save_plan(&current, &saved, pending.as_ref(), expired) {
            UiConfigSavePlan::Save(pending) => {
                assert_eq!(pending, current);
            }
            _ => panic!("expected save attempt"),
        }
    }
}
