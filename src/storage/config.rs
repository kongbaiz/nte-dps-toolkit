use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use super::i18n::Language;

/// `%LOCALAPPDATA%` folder name used by releases up to 0.3; only read for
/// migration, never written.
const LEGACY_CONFIG_DIRECTORY: &str = "NTE DPS Tool";
const CONFIG_FILENAME: &str = "config.json";
const UI_CONFIG_MAX_BYTES: u64 = 1024 * 1024;
/// Smallest inner size (logical points) each window may be dragged down to. Enforced both when
/// sanitizing a persisted size and at runtime via `with_min_inner_size`, so free resize can never
/// collapse a window below a usable layout. Roughly 0.6–0.7× of each window's base size.
pub const MAIN_WINDOW_MIN_SIZE: [f32; 2] = [420.0, 300.0];
pub const CONSOLE_WINDOW_MIN_SIZE: [f32; 2] = [640.0, 420.0];
pub const HIT_DETAIL_WINDOW_MIN_SIZE: [f32; 2] = [720.0, 480.0];
pub const TEAM_HIT_DETAIL_WINDOW_MIN_SIZE: [f32; 2] = [640.0, 440.0];
pub const ABYSS_WINDOW_MIN_SIZE: [f32; 2] = [680.0, 460.0];
/// Upper bound on a persisted window dimension, guarding against a corrupt config pushing a
/// window off every monitor.
const WINDOW_SIZE_MAX: f32 = 6000.0;
pub const TIMELINE_BUCKET_SECONDS_DEFAULT: f32 = 1.0;
pub const TIMELINE_BUCKET_SECONDS_MIN: f32 = 0.2;
pub const TIMELINE_BUCKET_SECONDS_MAX: f32 = 10.0;
pub const AUTO_ROUND_IDLE_SECONDS_DEFAULT: u32 = 30;
pub const AUTO_ROUND_IDLE_SECONDS_MIN: u32 = 5;
pub const AUTO_ROUND_IDLE_SECONDS_MAX: u32 = 600;
pub const HUD_WIDTH_DEFAULT: u16 = 380;
pub const HUD_WIDTH_MIN: u16 = 280;
/// Covers a full-width 4K workspace at 1x while preventing an invalid config
/// from creating an effectively unreachable overlay.
pub const HUD_WIDTH_MAX: u16 = 3840;
const HIT_DETAIL_COLUMN_WIDTH_MIN: u16 = 64;
const HIT_DETAIL_COLUMN_WIDTH_MAX: u16 = 600;
pub const MOD_STUDIO_GAME_DIRECTORY_MAX_BYTES: usize = 32_768;

const DPS_TIME_MODES: [DpsTimeMode; 2] = [DpsTimeMode::TimeStopAdjusted, DpsTimeMode::RealTime];
const TIMELINE_DPS_VIEW_MODES: [TimelineDpsViewMode; 2] =
    [TimelineDpsViewMode::Team, TimelineDpsViewMode::Characters];
const ACCENT_COLORS: [AccentColor; 5] = [
    AccentColor::Zinc,
    AccentColor::Blue,
    AccentColor::Violet,
    AccentColor::Orange,
    AccentColor::Green,
];
const THEME_PRESETS: [ThemePreset; 3] = [
    ThemePreset::Zinc,
    ThemePreset::Tactical,
    ThemePreset::HighContrast,
];
const UI_DENSITIES: [UiDensity; 3] = [UiDensity::Compact, UiDensity::Cozy, UiDensity::Comfortable];
const HUD_MODULES: [HudModule; 5] = [
    HudModule::Title,
    HudModule::Summary,
    HudModule::Status,
    HudModule::Characters,
    HudModule::Timeline,
];
const GLOBAL_HOTKEY_ACTIONS: [GlobalHotkeyAction; 4] = [
    GlobalHotkeyAction::ToggleCapture,
    GlobalHotkeyAction::ResetSession,
    GlobalHotkeyAction::ToggleHud,
    GlobalHotkeyAction::NewRound,
];
const HOTKEY_KEYS: [HotkeyKey; 59] = [
    HotkeyKey::A,
    HotkeyKey::B,
    HotkeyKey::C,
    HotkeyKey::D,
    HotkeyKey::E,
    HotkeyKey::F,
    HotkeyKey::G,
    HotkeyKey::H,
    HotkeyKey::I,
    HotkeyKey::J,
    HotkeyKey::K,
    HotkeyKey::L,
    HotkeyKey::M,
    HotkeyKey::N,
    HotkeyKey::O,
    HotkeyKey::P,
    HotkeyKey::Q,
    HotkeyKey::R,
    HotkeyKey::S,
    HotkeyKey::T,
    HotkeyKey::U,
    HotkeyKey::V,
    HotkeyKey::W,
    HotkeyKey::X,
    HotkeyKey::Y,
    HotkeyKey::Z,
    HotkeyKey::Digit0,
    HotkeyKey::Digit1,
    HotkeyKey::Digit2,
    HotkeyKey::Digit3,
    HotkeyKey::Digit4,
    HotkeyKey::Digit5,
    HotkeyKey::Digit6,
    HotkeyKey::Digit7,
    HotkeyKey::Digit8,
    HotkeyKey::Digit9,
    HotkeyKey::F1,
    HotkeyKey::F2,
    HotkeyKey::F3,
    HotkeyKey::F4,
    HotkeyKey::F5,
    HotkeyKey::F6,
    HotkeyKey::F7,
    HotkeyKey::F8,
    HotkeyKey::F9,
    HotkeyKey::F10,
    HotkeyKey::F11,
    HotkeyKey::F12,
    HotkeyKey::Home,
    HotkeyKey::End,
    HotkeyKey::Insert,
    HotkeyKey::Delete,
    HotkeyKey::PageUp,
    HotkeyKey::PageDown,
    HotkeyKey::ArrowUp,
    HotkeyKey::ArrowDown,
    HotkeyKey::ArrowLeft,
    HotkeyKey::ArrowRight,
    HotkeyKey::Space,
];

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DpsTimeMode {
    #[default]
    TimeStopAdjusted,
    RealTime,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ModStudioLoadingMethod {
    #[default]
    Proxy,
    Loader,
}

impl DpsTimeMode {
    pub fn all() -> &'static [Self] {
        &DPS_TIME_MODES
    }

    /// English key; wrap with [`crate::storage::i18n::t`] at the display site.
    pub fn label(self) -> &'static str {
        match self {
            Self::TimeStopAdjusted => "Exclude Time Stop",
            Self::RealTime => "Real Time",
        }
    }

    /// English key; wrap with [`crate::storage::i18n::t`] at the display site.
    pub fn description(self) -> &'static str {
        match self {
            Self::TimeStopAdjusted => {
                "Output time is not counted during the authoritative game pause"
            }
            Self::RealTime => "Output time accrues over the capture time span",
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TimelineDpsViewMode {
    #[default]
    Team,
    Characters,
}

impl TimelineDpsViewMode {
    pub fn all() -> &'static [Self] {
        &TIMELINE_DPS_VIEW_MODES
    }

    /// English key; wrap with [`crate::storage::i18n::t`] at the display site.
    pub fn label(self) -> &'static str {
        match self {
            Self::Team => "Whole Team",
            Self::Characters => "By Character",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GlobalHotkeyAction {
    ToggleCapture,
    ResetSession,
    ToggleHud,
    NewRound,
}

impl GlobalHotkeyAction {
    pub fn all() -> &'static [Self] {
        &GLOBAL_HOTKEY_ACTIONS
    }

    /// English key; wrap with [`crate::storage::i18n::t`] at the display site.
    pub fn label(self) -> &'static str {
        match self {
            Self::ToggleCapture => "Start / Stop Capture",
            Self::ResetSession => "Reset Session",
            Self::ToggleHud => "Toggle Combat HUD",
            Self::NewRound => "New Round",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HotkeyKey {
    A,
    B,
    C,
    D,
    E,
    F,
    G,
    H,
    I,
    J,
    K,
    L,
    M,
    N,
    O,
    P,
    Q,
    R,
    S,
    T,
    U,
    V,
    W,
    X,
    Y,
    Z,
    Digit0,
    Digit1,
    Digit2,
    Digit3,
    Digit4,
    Digit5,
    Digit6,
    Digit7,
    Digit8,
    Digit9,
    F1,
    F2,
    F3,
    F4,
    F5,
    F6,
    F7,
    F8,
    F9,
    F10,
    F11,
    F12,
    Home,
    End,
    Insert,
    Delete,
    PageUp,
    PageDown,
    ArrowUp,
    ArrowDown,
    ArrowLeft,
    ArrowRight,
    Space,
}

impl HotkeyKey {
    pub fn all() -> &'static [Self] {
        &HOTKEY_KEYS
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::A => "A",
            Self::B => "B",
            Self::C => "C",
            Self::D => "D",
            Self::E => "E",
            Self::F => "F",
            Self::G => "G",
            Self::H => "H",
            Self::I => "I",
            Self::J => "J",
            Self::K => "K",
            Self::L => "L",
            Self::M => "M",
            Self::N => "N",
            Self::O => "O",
            Self::P => "P",
            Self::Q => "Q",
            Self::R => "R",
            Self::S => "S",
            Self::T => "T",
            Self::U => "U",
            Self::V => "V",
            Self::W => "W",
            Self::X => "X",
            Self::Y => "Y",
            Self::Z => "Z",
            Self::Digit0 => "0",
            Self::Digit1 => "1",
            Self::Digit2 => "2",
            Self::Digit3 => "3",
            Self::Digit4 => "4",
            Self::Digit5 => "5",
            Self::Digit6 => "6",
            Self::Digit7 => "7",
            Self::Digit8 => "8",
            Self::Digit9 => "9",
            Self::F1 => "F1",
            Self::F2 => "F2",
            Self::F3 => "F3",
            Self::F4 => "F4",
            Self::F5 => "F5",
            Self::F6 => "F6",
            Self::F7 => "F7",
            Self::F8 => "F8",
            Self::F9 => "F9",
            Self::F10 => "F10",
            Self::F11 => "F11",
            Self::F12 => "F12",
            Self::Home => "Home",
            Self::End => "End",
            Self::Insert => "Insert",
            Self::Delete => "Delete",
            Self::PageUp => "PageUp",
            Self::PageDown => "PageDown",
            Self::ArrowUp => "ArrowUp",
            Self::ArrowDown => "ArrowDown",
            Self::ArrowLeft => "ArrowLeft",
            Self::ArrowRight => "ArrowRight",
            Self::Space => "Space",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct HotkeyBinding {
    pub ctrl: bool,
    pub alt: bool,
    pub shift: bool,
    pub key: HotkeyKey,
}

impl HotkeyBinding {
    pub const fn new(ctrl: bool, alt: bool, shift: bool, key: HotkeyKey) -> Self {
        Self {
            ctrl,
            alt,
            shift,
            key,
        }
    }

    pub fn label(self) -> String {
        let mut parts = Vec::with_capacity(4);
        if self.ctrl {
            parts.push("Ctrl");
        }
        if self.alt {
            parts.push("Alt");
        }
        if self.shift {
            parts.push("Shift");
        }
        parts.push(self.key.label());
        parts.join("+")
    }

    pub fn is_reserved(self) -> bool {
        self.alt && self.key == HotkeyKey::F4
    }
}

impl Default for HotkeyBinding {
    fn default() -> Self {
        Self::new(false, false, false, HotkeyKey::F1)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct GlobalHotkeys {
    pub enabled: bool,
    pub capture: Option<HotkeyBinding>,
    pub reset: Option<HotkeyBinding>,
    pub hud: Option<HotkeyBinding>,
    pub new_round: Option<HotkeyBinding>,
}

impl GlobalHotkeys {
    pub fn binding(self, action: GlobalHotkeyAction) -> Option<HotkeyBinding> {
        match action {
            GlobalHotkeyAction::ToggleCapture => self.capture,
            GlobalHotkeyAction::ResetSession => self.reset,
            GlobalHotkeyAction::ToggleHud => self.hud,
            GlobalHotkeyAction::NewRound => self.new_round,
        }
    }

    pub fn set_binding(&mut self, action: GlobalHotkeyAction, binding: Option<HotkeyBinding>) {
        match action {
            GlobalHotkeyAction::ToggleCapture => self.capture = binding,
            GlobalHotkeyAction::ResetSession => self.reset = binding,
            GlobalHotkeyAction::ToggleHud => self.hud = binding,
            GlobalHotkeyAction::NewRound => self.new_round = binding,
        }
    }

    pub fn sanitized(mut self) -> Self {
        for action in GlobalHotkeyAction::all() {
            if self
                .binding(*action)
                .is_some_and(HotkeyBinding::is_reserved)
            {
                self.set_binding(*action, None);
            }
        }
        if self.reset.is_some() && self.reset == self.capture {
            self.reset = None;
        }
        if self.hud.is_some() && (self.hud == self.capture || self.hud == self.reset) {
            self.hud = None;
        }
        if self.new_round.is_some()
            && (self.new_round == self.capture
                || self.new_round == self.reset
                || self.new_round == self.hud)
        {
            self.new_round = None;
        }
        self
    }

    fn without_binding(mut self, binding: HotkeyBinding) -> Self {
        for action in GlobalHotkeyAction::all() {
            if self.binding(*action) == Some(binding) {
                self.set_binding(*action, None);
            }
        }
        self
    }
}

impl Default for GlobalHotkeys {
    fn default() -> Self {
        Self {
            enabled: true,
            capture: Some(HotkeyBinding::new(true, false, false, HotkeyKey::F9)),
            reset: Some(HotkeyBinding::new(true, false, false, HotkeyKey::F10)),
            hud: Some(HotkeyBinding::new(true, false, false, HotkeyKey::F11)),
            new_round: None,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MainDpsMetric {
    TeamDps,
    TotalDamage,
    TotalDamageTaken,
    Duration,
}

impl MainDpsMetric {
    pub const ALL: [Self; 4] = [
        Self::TeamDps,
        Self::TotalDamage,
        Self::TotalDamageTaken,
        Self::Duration,
    ];

    pub const fn id(self) -> &'static str {
        match self {
            Self::TeamDps => "team-dps",
            Self::TotalDamage => "total-damage",
            Self::TotalDamageTaken => "total-damage-taken",
            Self::Duration => "duration",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MainDpsAttribution {
    Character,
    Reaction,
    Shared,
    Unattributed,
    MaxHpReduction,
}

impl MainDpsAttribution {
    pub const ALL: [Self; 5] = [
        Self::Character,
        Self::Reaction,
        Self::Shared,
        Self::Unattributed,
        Self::MaxHpReduction,
    ];

    pub const fn id(self) -> &'static str {
        match self {
            Self::Character => "character",
            Self::Reaction => "reaction",
            Self::Shared => "shared",
            Self::Unattributed => "unattributed",
            Self::MaxHpReduction => "max-hp-reduction",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct MainDpsDisplayConfig {
    pub metrics: Vec<MainDpsMetric>,
    pub attributions: Vec<MainDpsAttribution>,
}

impl Default for MainDpsDisplayConfig {
    fn default() -> Self {
        Self {
            metrics: MainDpsMetric::ALL.to_vec(),
            attributions: MainDpsAttribution::ALL.to_vec(),
        }
    }
}

impl MainDpsDisplayConfig {
    fn sanitized(mut self) -> Self {
        let mut metrics = Vec::with_capacity(self.metrics.len().min(MainDpsMetric::ALL.len()));
        for metric in self.metrics.drain(..) {
            if !metrics.contains(&metric) {
                metrics.push(metric);
            }
        }
        let mut attributions =
            Vec::with_capacity(self.attributions.len().min(MainDpsAttribution::ALL.len()));
        for attribution in self.attributions.drain(..) {
            if !attributions.contains(&attribution) {
                attributions.push(attribution);
            }
        }
        Self {
            metrics,
            attributions,
        }
    }
}

fn default_passthrough_hotkey() -> HotkeyBinding {
    HotkeyBinding::new(false, false, false, HotkeyKey::Home)
}

fn deserialize_passthrough_hotkey<'de, D>(deserializer: D) -> Result<HotkeyBinding, D::Error>
where
    D: serde::Deserializer<'de>,
{
    #[derive(Deserialize)]
    #[serde(rename_all = "snake_case")]
    enum LegacyPassthroughHotkey {
        Home,
        Insert,
        F8,
        F9,
    }

    #[derive(Deserialize)]
    #[serde(untagged)]
    enum PersistedPassthroughHotkey {
        Binding(HotkeyBinding),
        Legacy(LegacyPassthroughHotkey),
    }

    Ok(
        match PersistedPassthroughHotkey::deserialize(deserializer)? {
            PersistedPassthroughHotkey::Binding(binding) => binding,
            PersistedPassthroughHotkey::Legacy(value) => HotkeyBinding::new(
                false,
                false,
                false,
                match value {
                    LegacyPassthroughHotkey::Home => HotkeyKey::Home,
                    LegacyPassthroughHotkey::Insert => HotkeyKey::Insert,
                    LegacyPassthroughHotkey::F8 => HotkeyKey::F8,
                    LegacyPassthroughHotkey::F9 => HotkeyKey::F9,
                },
            ),
        },
    )
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AccentColor {
    #[default]
    Zinc,
    Blue,
    Violet,
    Orange,
    Green,
}

impl AccentColor {
    pub fn all() -> &'static [Self] {
        &ACCENT_COLORS
    }

    /// English key; wrap with [`crate::storage::i18n::t`] at the display site.
    pub fn label(self) -> &'static str {
        match self {
            Self::Zinc => "Zinc",
            Self::Blue => "Blue",
            Self::Violet => "Violet",
            Self::Orange => "Orange",
            Self::Green => "Green",
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ThemePreset {
    #[default]
    Zinc,
    Tactical,
    HighContrast,
}

impl ThemePreset {
    pub fn all() -> &'static [Self] {
        &THEME_PRESETS
    }

    /// English key; wrap with [`crate::storage::i18n::t`] at the display site.
    pub fn label(self) -> &'static str {
        match self {
            Self::Zinc => "Zinc Theme",
            Self::Tactical => "Tactical",
            Self::HighContrast => "High Contrast",
        }
    }

    /// English key; wrap with [`crate::storage::i18n::t`] at the display site.
    pub fn description(self) -> &'static str {
        match self {
            Self::Zinc => "Neutral desktop surfaces with the selected accent",
            Self::Tactical => "Near-black surfaces with a high-saturation tactical accent",
            Self::HighContrast => "Pure high-contrast surfaces with stronger borders",
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UiDensity {
    Compact,
    #[default]
    Cozy,
    Comfortable,
}

impl UiDensity {
    pub fn all() -> &'static [Self] {
        &UI_DENSITIES
    }

    /// English key; wrap with [`crate::storage::i18n::t`] at the display site.
    pub fn label(self) -> &'static str {
        match self {
            Self::Compact => "Compact",
            Self::Cozy => "Cozy",
            Self::Comfortable => "Comfortable",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HudModule {
    Title,
    Summary,
    Status,
    Characters,
    Timeline,
}

impl HudModule {
    pub fn all() -> &'static [Self] {
        &HUD_MODULES
    }

    /// English key; wrap with [`crate::storage::i18n::t`] at the display site.
    pub fn label(self) -> &'static str {
        match self {
            Self::Title => "Title",
            Self::Summary => "Summary",
            Self::Status => "Status",
            Self::Characters => "Character Ranking",
            Self::Timeline => "Curve",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HitDetailColumn {
    Time,
    Character,
    Type,
    Damage,
    TargetHp,
}

impl HitDetailColumn {
    /// English key; wrap with [`crate::storage::i18n::t`] at the display site.
    pub fn label(self) -> &'static str {
        match self {
            Self::Time => "Time",
            Self::Character => "Character",
            Self::Type => "Type",
            Self::Damage => "Damage",
            Self::TargetHp => "Target / HP",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct HitDetailColumnsConfig {
    pub show_time: bool,
    pub show_character: bool,
    pub show_type: bool,
    pub show_damage: bool,
    pub show_target_hp: bool,
    pub time_width: u16,
    pub character_width: u16,
    pub type_width: u16,
    pub damage_width: u16,
    pub target_hp_width: u16,
}

impl Default for HitDetailColumnsConfig {
    fn default() -> Self {
        Self {
            show_time: true,
            show_character: true,
            show_type: true,
            show_damage: true,
            show_target_hp: true,
            time_width: 92,
            character_width: 132,
            type_width: 250,
            damage_width: 130,
            target_hp_width: 180,
        }
    }
}

impl HitDetailColumnsConfig {
    pub fn visible(self, column: HitDetailColumn) -> bool {
        match column {
            HitDetailColumn::Time => self.show_time,
            HitDetailColumn::Character => self.show_character,
            HitDetailColumn::Type => self.show_type,
            HitDetailColumn::Damage => self.show_damage,
            HitDetailColumn::TargetHp => self.show_target_hp,
        }
    }

    pub fn set_visible(&mut self, column: HitDetailColumn, visible: bool) {
        match column {
            HitDetailColumn::Time => self.show_time = visible,
            HitDetailColumn::Character => self.show_character = visible,
            HitDetailColumn::Type => self.show_type = visible,
            HitDetailColumn::Damage => self.show_damage = visible,
            HitDetailColumn::TargetHp => self.show_target_hp = visible,
        }
    }

    pub fn width(self, column: HitDetailColumn) -> u16 {
        match column {
            HitDetailColumn::Time => self.time_width,
            HitDetailColumn::Character => self.character_width,
            HitDetailColumn::Type => self.type_width,
            HitDetailColumn::Damage => self.damage_width,
            HitDetailColumn::TargetHp => self.target_hp_width,
        }
    }

    pub fn set_width(&mut self, column: HitDetailColumn, width: u16) {
        let width = width.clamp(HIT_DETAIL_COLUMN_WIDTH_MIN, HIT_DETAIL_COLUMN_WIDTH_MAX);
        match column {
            HitDetailColumn::Time => self.time_width = width,
            HitDetailColumn::Character => self.character_width = width,
            HitDetailColumn::Type => self.type_width = width,
            HitDetailColumn::Damage => self.damage_width = width,
            HitDetailColumn::TargetHp => self.target_hp_width = width,
        }
    }

    pub fn sanitized(mut self) -> Self {
        for column in [
            HitDetailColumn::Time,
            HitDetailColumn::Character,
            HitDetailColumn::Type,
            HitDetailColumn::Damage,
            HitDetailColumn::TargetHp,
        ] {
            self.set_width(column, self.width(column));
        }
        self
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct HudConfig {
    pub width: u16,
    pub module_order: Vec<HudModule>,
    pub show_title: bool,
    pub show_team_dps: bool,
    pub show_duration: bool,
    pub show_total_damage: bool,
    pub show_character_rows: bool,
    pub show_damage_taken: bool,
    pub show_abyss_half: bool,
    pub show_passthrough_state: bool,
    pub show_mini_timeline: bool,
}

impl Default for HudConfig {
    fn default() -> Self {
        Self {
            width: HUD_WIDTH_DEFAULT,
            module_order: HUD_MODULES.to_vec(),
            show_title: false,
            show_team_dps: true,
            show_duration: true,
            show_total_damage: true,
            show_character_rows: true,
            show_damage_taken: false,
            show_abyss_half: false,
            show_passthrough_state: false,
            show_mini_timeline: false,
        }
    }
}

impl HudConfig {
    /// Pared-down overlay: just team DPS and a short character ranking. Pairs
    /// with [`Self::default`] ("标准") and [`Self::detailed`] ("详细") as the
    /// one-click HUD presets in settings.
    pub fn minimal() -> Self {
        Self {
            show_title: false,
            show_team_dps: true,
            show_duration: false,
            show_total_damage: false,
            show_character_rows: true,
            show_damage_taken: false,
            show_abyss_half: false,
            show_passthrough_state: false,
            show_mini_timeline: false,
            ..Self::default()
        }
    }

    /// Everything on, for a full diagnostic readout.
    pub fn detailed() -> Self {
        Self {
            show_title: true,
            show_team_dps: true,
            show_duration: true,
            show_total_damage: true,
            show_character_rows: true,
            show_damage_taken: true,
            show_abyss_half: true,
            show_passthrough_state: true,
            show_mini_timeline: true,
            ..Self::default()
        }
    }

    pub fn sanitized(mut self) -> Self {
        self.width = self.width.clamp(HUD_WIDTH_MIN, HUD_WIDTH_MAX);
        let mut normalized = Vec::with_capacity(HUD_MODULES.len());
        for module in self.module_order {
            if !normalized.contains(&module) {
                normalized.push(module);
            }
        }
        for module in HudModule::all().iter().copied() {
            if !normalized.contains(&module) {
                normalized.push(module);
            }
        }
        self.module_order = normalized;
        self
    }

    pub fn has_summary_row(&self) -> bool {
        self.show_team_dps || self.show_duration || self.show_total_damage || self.show_damage_taken
    }

    pub fn module_visible(&self, module: HudModule) -> bool {
        match module {
            HudModule::Title => self.show_title,
            HudModule::Summary => self.has_summary_row(),
            HudModule::Status => self.show_abyss_half || self.show_passthrough_state,
            HudModule::Characters => self.show_character_rows,
            HudModule::Timeline => self.show_mini_timeline,
        }
    }

    pub fn set_module_visible(&mut self, module: HudModule, visible: bool) {
        match module {
            HudModule::Title => self.show_title = visible,
            HudModule::Summary => {
                if visible {
                    self.show_team_dps = true;
                } else {
                    self.show_team_dps = false;
                    self.show_duration = false;
                    self.show_total_damage = false;
                    self.show_damage_taken = false;
                }
            }
            HudModule::Status => {
                if visible {
                    self.show_passthrough_state = true;
                } else {
                    self.show_abyss_half = false;
                    self.show_passthrough_state = false;
                }
            }
            HudModule::Characters => self.show_character_rows = visible,
            HudModule::Timeline => self.show_mini_timeline = visible,
        }
    }

    pub fn move_module(&mut self, dragged: HudModule, target: HudModule, insert_after: bool) {
        let from = self
            .module_order
            .iter()
            .position(|module| *module == dragged)
            .expect("dragged HUD module belongs to module_order");
        let target = self
            .module_order
            .iter()
            .position(|module| *module == target)
            .expect("drop target belongs to module_order");
        let mut insertion = target + usize::from(insert_after);
        let dragged = self.module_order.remove(from);
        if from < insertion {
            insertion -= 1;
        }
        self.module_order.insert(insertion, dragged);
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct UiConfig {
    /// Active UI language. Absent in older configs → defaults to Simplified Chinese
    /// (the historical UI language) so upgrades are not disrupted.
    pub language: Language,
    pub opacity: f32,
    pub dark_mode: bool,
    #[serde(default)]
    pub theme_preset: ThemePreset,
    #[serde(default)]
    pub accent: AccentColor,
    #[serde(default)]
    pub density: UiDensity,
    #[serde(default)]
    pub reduce_motion: bool,
    #[serde(default = "default_auto_check_updates")]
    pub auto_check_updates: bool,
    #[serde(default)]
    pub auto_download_updates: bool,
    /// Per-region manual game directories selected in Mod Studio. `None` keeps
    /// automatic installation detection for that region.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mod_studio_china_game_directory: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mod_studio_global_game_directory: Option<String>,
    #[serde(default)]
    pub mod_studio_loading_method: ModStudioLoadingMethod,
    #[serde(default)]
    pub mod_studio_risk_acknowledged: bool,
    pub always_on_top: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub main_dps_always_on_top: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hud_always_on_top: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub console_always_on_top: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub abyss_values_always_on_top: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub character_details_always_on_top: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub team_details_always_on_top: Option<bool>,
    /// Show notifications in the global "dynamic island" capsule floating
    /// above every window (its own overlay viewport). Off falls back to the
    /// legacy in-window corner toasts.
    #[serde(default = "default_island_notifications")]
    pub island_notifications: bool,
    /// Horizontal offset of the island from the screen center, in logical points.
    #[serde(default)]
    pub island_offset_x: f32,
    #[serde(default = "default_capture_filter")]
    pub capture_filter: String,
    pub server_damage_calibration: bool,
    #[serde(default)]
    pub include_max_hp_reduction_in_total_damage: bool,
    #[serde(default)]
    pub separate_reaction_damage: bool,
    #[serde(default)]
    pub auto_round_after_idle: bool,
    #[serde(default = "default_auto_round_idle_seconds")]
    pub auto_round_idle_seconds: u32,
    /// Manual capture-NIC override (the Npcap device `name`, e.g. `\Device\NPF_{GUID}`). `None`
    /// keeps automatic detection; `Some(name)` pins capture to that interface as a VPN fallback.
    pub manual_capture_device: Option<String>,
    pub dps_time_mode: DpsTimeMode,
    pub timeline_bucket_seconds: f32,
    pub timeline_dps_view_mode: TimelineDpsViewMode,
    pub hud: HudConfig,
    #[serde(default)]
    pub hit_detail_columns: HitDetailColumnsConfig,
    #[serde(
        default = "default_passthrough_hotkey",
        deserialize_with = "deserialize_passthrough_hotkey"
    )]
    pub passthrough_hotkey: HotkeyBinding,
    #[serde(default)]
    pub global_hotkeys: GlobalHotkeys,
    #[serde(default)]
    pub main_dps_display: MainDpsDisplayConfig,
    #[serde(default = "default_onboarding_done")]
    pub onboarding_done: bool,
    /// Last inner size (logical points) each window was dragged to, restored on the next launch.
    /// Absent (older configs, or the retired `*_window_scale` keys) → the window opens at its base
    /// size. Replaces the removed fixed-ratio `−／＋` scale.
    #[serde(default)]
    pub main_window_size: Option<[f32; 2]>,
    #[serde(default)]
    pub abyss_window_size: Option<[f32; 2]>,
    #[serde(default)]
    pub hit_detail_window_size: Option<[f32; 2]>,
    #[serde(default)]
    pub team_hit_detail_window_size: Option<[f32; 2]>,
    #[serde(default)]
    pub console_window_size: Option<[f32; 2]>,
    /// Last normal outer position (logical points) for each native window. Negative coordinates
    /// are valid for monitors placed left of or above the primary display.
    #[serde(default)]
    pub main_window_position: Option<[f32; 2]>,
    #[serde(default)]
    pub abyss_window_position: Option<[f32; 2]>,
    #[serde(default)]
    pub hit_detail_window_position: Option<[f32; 2]>,
    #[serde(default)]
    pub team_hit_detail_window_position: Option<[f32; 2]>,
    #[serde(default)]
    pub console_window_position: Option<[f32; 2]>,
    /// Last Tauri HUD outer position in physical virtual-desktop pixels.
    /// Physical coordinates preserve negative secondary-monitor origins without
    /// applying the startup monitor's DPI scale to another display.
    #[serde(default)]
    pub hud_window_position: Option<[i32; 2]>,
}

impl Default for UiConfig {
    fn default() -> Self {
        Self {
            language: Language::default(),
            opacity: 0.92,
            dark_mode: false,
            theme_preset: ThemePreset::default(),
            accent: AccentColor::default(),
            density: UiDensity::default(),
            reduce_motion: false,
            auto_check_updates: true,
            auto_download_updates: false,
            mod_studio_china_game_directory: None,
            mod_studio_global_game_directory: None,
            mod_studio_loading_method: ModStudioLoadingMethod::default(),
            mod_studio_risk_acknowledged: false,
            always_on_top: true,
            main_dps_always_on_top: None,
            hud_always_on_top: None,
            console_always_on_top: None,
            abyss_values_always_on_top: None,
            character_details_always_on_top: None,
            team_details_always_on_top: None,
            island_notifications: true,
            island_offset_x: 0.0,
            capture_filter: default_capture_filter(),
            server_damage_calibration: false,
            include_max_hp_reduction_in_total_damage: false,
            separate_reaction_damage: false,
            auto_round_after_idle: false,
            auto_round_idle_seconds: AUTO_ROUND_IDLE_SECONDS_DEFAULT,
            manual_capture_device: None,
            dps_time_mode: DpsTimeMode::default(),
            timeline_bucket_seconds: TIMELINE_BUCKET_SECONDS_DEFAULT,
            timeline_dps_view_mode: TimelineDpsViewMode::default(),
            hud: HudConfig::default(),
            hit_detail_columns: HitDetailColumnsConfig::default(),
            passthrough_hotkey: default_passthrough_hotkey(),
            global_hotkeys: GlobalHotkeys::default(),
            main_dps_display: MainDpsDisplayConfig::default(),
            onboarding_done: true,
            main_window_size: None,
            abyss_window_size: None,
            hit_detail_window_size: None,
            team_hit_detail_window_size: None,
            console_window_size: None,
            main_window_position: None,
            abyss_window_position: None,
            hit_detail_window_position: None,
            team_hit_detail_window_position: None,
            console_window_position: None,
            hud_window_position: None,
        }
    }
}

impl UiConfig {
    pub fn sanitized(mut self) -> Self {
        self.main_dps_always_on_top =
            Some(self.main_dps_always_on_top.unwrap_or(self.always_on_top));
        self.hud_always_on_top = Some(self.hud_always_on_top.unwrap_or(self.always_on_top));
        self.console_always_on_top = Some(self.console_always_on_top.unwrap_or(false));
        self.abyss_values_always_on_top = Some(self.abyss_values_always_on_top.unwrap_or(false));
        self.character_details_always_on_top =
            Some(self.character_details_always_on_top.unwrap_or(false));
        self.team_details_always_on_top = Some(self.team_details_always_on_top.unwrap_or(false));
        self.opacity = if self.opacity.is_finite() {
            self.opacity.clamp(0.35, 1.0)
        } else {
            Self::default().opacity
        };
        self.island_offset_x = if self.island_offset_x.is_finite() {
            self.island_offset_x.clamp(-4000.0, 4000.0)
        } else {
            0.0
        };
        self.capture_filter = sanitize_capture_filter(&self.capture_filter);
        self.main_window_size = sanitize_window_size(self.main_window_size, MAIN_WINDOW_MIN_SIZE);
        self.abyss_window_size =
            sanitize_window_size(self.abyss_window_size, ABYSS_WINDOW_MIN_SIZE);
        self.hit_detail_window_size =
            sanitize_window_size(self.hit_detail_window_size, HIT_DETAIL_WINDOW_MIN_SIZE);
        self.team_hit_detail_window_size = sanitize_window_size(
            self.team_hit_detail_window_size,
            TEAM_HIT_DETAIL_WINDOW_MIN_SIZE,
        );
        self.console_window_size =
            sanitize_window_size(self.console_window_size, CONSOLE_WINDOW_MIN_SIZE);
        self.main_window_position = sanitize_window_position(self.main_window_position);
        self.abyss_window_position = sanitize_window_position(self.abyss_window_position);
        self.hit_detail_window_position = sanitize_window_position(self.hit_detail_window_position);
        self.team_hit_detail_window_position =
            sanitize_window_position(self.team_hit_detail_window_position);
        self.console_window_position = sanitize_window_position(self.console_window_position);
        self.timeline_bucket_seconds =
            sanitize_timeline_bucket_seconds(self.timeline_bucket_seconds);
        self.auto_round_idle_seconds = self
            .auto_round_idle_seconds
            .clamp(AUTO_ROUND_IDLE_SECONDS_MIN, AUTO_ROUND_IDLE_SECONDS_MAX);
        self.manual_capture_device = self
            .manual_capture_device
            .take()
            .filter(|name| !name.trim().is_empty());
        self.mod_studio_china_game_directory =
            sanitize_mod_studio_game_directory(self.mod_studio_china_game_directory.take());
        self.mod_studio_global_game_directory =
            sanitize_mod_studio_game_directory(self.mod_studio_global_game_directory.take());
        self.hud = self.hud.sanitized();
        self.hit_detail_columns = self.hit_detail_columns.sanitized();
        self.global_hotkeys = self.global_hotkeys.sanitized();
        self.global_hotkeys = self.global_hotkeys.without_binding(self.passthrough_hotkey);
        self.main_dps_display = self.main_dps_display.sanitized();
        self
    }
}

fn sanitize_mod_studio_game_directory(directory: Option<String>) -> Option<String> {
    directory.and_then(|directory| {
        let trimmed = directory.trim();
        if trimmed.is_empty()
            || trimmed.len() > MOD_STUDIO_GAME_DIRECTORY_MAX_BYTES
            || trimmed
                .chars()
                .any(|character| matches!(character, '\0' | '\r' | '\n'))
        {
            None
        } else {
            Some(trimmed.to_owned())
        }
    })
}

const fn default_onboarding_done() -> bool {
    true
}

const fn default_island_notifications() -> bool {
    true
}

const fn default_auto_check_updates() -> bool {
    true
}

const fn default_auto_round_idle_seconds() -> u32 {
    AUTO_ROUND_IDLE_SECONDS_DEFAULT
}

fn default_capture_filter() -> String {
    "udp".to_owned()
}

pub fn sanitize_capture_filter(filter: &str) -> String {
    let filter = filter.trim();
    if filter.is_empty()
        || filter.len() > 512
        || filter
            .chars()
            .any(|character| matches!(character, '\0' | '\r' | '\n'))
    {
        default_capture_filter()
    } else {
        filter.to_owned()
    }
}

fn new_install_config() -> UiConfig {
    UiConfig {
        language: Language::system_default(),
        onboarding_done: false,
        ..UiConfig::default()
    }
}

pub fn sanitize_timeline_bucket_seconds(seconds: f32) -> f32 {
    if seconds.is_finite() {
        seconds.clamp(TIMELINE_BUCKET_SECONDS_MIN, TIMELINE_BUCKET_SECONDS_MAX)
    } else {
        TIMELINE_BUCKET_SECONDS_DEFAULT
    }
}

/// Clamps a persisted window size to `[min, WINDOW_SIZE_MAX]` per axis. A non-finite or absent
/// size becomes `None`, letting the caller fall back to the window's base size.
fn sanitize_window_size(size: Option<[f32; 2]>, min: [f32; 2]) -> Option<[f32; 2]> {
    let [width, height] = size?;
    if !width.is_finite() || !height.is_finite() {
        return None;
    }
    Some([
        width.clamp(min[0], WINDOW_SIZE_MAX),
        height.clamp(min[1], WINDOW_SIZE_MAX),
    ])
}

fn sanitize_window_position(position: Option<[f32; 2]>) -> Option<[f32; 2]> {
    let [x, y] = position?;
    (x.is_finite() && y.is_finite()).then_some([x, y])
}

pub fn config_path() -> PathBuf {
    crate::storage::paths::software_dir().join(CONFIG_FILENAME)
}

/// Where releases up to 0.3 stored the config. Read once when the
/// exe-adjacent config does not exist yet so an upgrade keeps the user's
/// settings; the legacy file itself is left untouched.
fn legacy_config_path() -> Option<PathBuf> {
    std::env::var_os("LOCALAPPDATA").map(|base| {
        PathBuf::from(base)
            .join(LEGACY_CONFIG_DIRECTORY)
            .join(CONFIG_FILENAME)
    })
}

pub fn load() -> (UiConfig, Option<String>) {
    load_with_paths(&config_path(), legacy_config_path().as_deref())
}

fn load_with_paths(path: &Path, legacy_path: Option<&Path>) -> (UiConfig, Option<String>) {
    if config_path_requires_load_attempt(path) {
        return read_config_file(path);
    }
    if let Some(legacy_path) =
        legacy_path.filter(|legacy_path| config_path_requires_load_attempt(legacy_path))
    {
        let (config, warning) = read_config_file(legacy_path);
        if warning.is_some() {
            return (config, warning);
        }
        // Adopt the legacy config into the new location so later loads and
        // saves agree on one file.
        let warning = save(path, &config).err().map(|error| {
            crate::storage::i18n::tf(
                "Failed to create default UI config ({}): {}",
                &[&path.display().to_string(), &error],
            )
        });
        return (config, warning);
    }
    // Brand-new install: pick the UI language from the system locale (if a
    // localization file matches it) instead of the historical
    // Simplified-Chinese default, which only exists to keep upgrades from
    // older (pre-i18n) configs stable — see `Language::system_default`.
    let config = new_install_config();
    let warning = save(path, &config).err().map(|error| {
        crate::storage::i18n::tf(
            "Failed to create default UI config ({}): {}",
            &[&path.display().to_string(), &error],
        )
    });
    (config, warning)
}

fn config_path_requires_load_attempt(path: &Path) -> bool {
    match fs::symlink_metadata(path) {
        Ok(_) => true,
        Err(error) => error.kind() != std::io::ErrorKind::NotFound,
    }
}

#[derive(Debug)]
enum ConfigFileLoadError {
    NotAFile,
    TooLarge,
    Io(std::io::Error),
    InvalidUtf8,
    InvalidJson(serde_json::Error),
}

impl std::fmt::Display for ConfigFileLoadError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotAFile => formatter.write_str("UI config path is not a regular file"),
            Self::TooLarge => write!(
                formatter,
                "UI config exceeds the {UI_CONFIG_MAX_BYTES}-byte limit"
            ),
            Self::Io(error) => write!(formatter, "UI config I/O failed: {error}"),
            Self::InvalidUtf8 => formatter.write_str("UI config is not valid UTF-8"),
            Self::InvalidJson(error) => write!(formatter, "UI config JSON is invalid: {error}"),
        }
    }
}

fn read_config_text(path: &Path) -> Result<String, ConfigFileLoadError> {
    // Classify directories before opening so Windows and Unix produce the same
    // typed result. The authoritative type and byte budget checks below still
    // use metadata from the opened handle, closing the path-swap gap.
    let path_metadata = fs::metadata(path).map_err(ConfigFileLoadError::Io)?;
    if !path_metadata.is_file() {
        return Err(ConfigFileLoadError::NotAFile);
    }

    let file = fs::File::open(path).map_err(ConfigFileLoadError::Io)?;
    let metadata = file.metadata().map_err(ConfigFileLoadError::Io)?;
    if !metadata.is_file() {
        return Err(ConfigFileLoadError::NotAFile);
    }
    read_config_text_from_open_file(file, metadata.len())
}

fn read_config_text_from_open_file(
    mut file: fs::File,
    opened_size: u64,
) -> Result<String, ConfigFileLoadError> {
    if opened_size > UI_CONFIG_MAX_BYTES {
        return Err(ConfigFileLoadError::TooLarge);
    }

    let mut bytes = Vec::with_capacity(opened_size as usize);
    {
        let mut bounded = (&mut file).take(UI_CONFIG_MAX_BYTES);
        bounded
            .read_to_end(&mut bytes)
            .map_err(ConfigFileLoadError::Io)?;
    }

    // `take(limit)` alone cannot distinguish an exact-limit file from a file
    // that grew after metadata was sampled. Probe the same handle once more.
    if bytes.len() as u64 == UI_CONFIG_MAX_BYTES {
        let mut growth_probe = [0_u8; 1];
        if file
            .read(&mut growth_probe)
            .map_err(ConfigFileLoadError::Io)?
            != 0
        {
            return Err(ConfigFileLoadError::TooLarge);
        }
    }

    String::from_utf8(bytes).map_err(|_| ConfigFileLoadError::InvalidUtf8)
}

fn read_config_file(path: &Path) -> (UiConfig, Option<String>) {
    match read_config_text(path).and_then(|text| {
        serde_json::from_str::<UiConfig>(&text).map_err(ConfigFileLoadError::InvalidJson)
    }) {
        Ok(config) => (config.sanitized(), None),
        Err(error) => {
            let error = error.to_string();
            (
                UiConfig::default(),
                Some(crate::storage::i18n::tf(
                    "Failed to load UI config ({}): {}",
                    &[&path.display().to_string(), &error],
                )),
            )
        }
    }
}

pub fn save(path: &Path, config: &UiConfig) -> Result<(), String> {
    let text = serde_json::to_string_pretty(&config.clone().sanitized())
        .map_err(|error| error.to_string())?;
    // Atomic write so a crash mid-write cannot leave a truncated/corrupt config.json.
    crate::storage::io_util::atomic_write_text(path, &format!("{text}\n"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_config_dir(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "nte_config_{tag}_{}_{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn write_config(path: &Path, opacity: f32) {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        let config = UiConfig {
            opacity,
            ..UiConfig::default()
        };
        fs::write(path, serde_json::to_string(&config).unwrap()).unwrap();
    }

    #[test]
    fn config_reader_accepts_the_exact_byte_limit() {
        let dir = temp_config_dir("exact_limit");
        let path = dir.join(CONFIG_FILENAME);
        fs::write(&path, vec![b' '; UI_CONFIG_MAX_BYTES as usize]).unwrap();

        let text = read_config_text(&path).expect("exact-limit config should be readable");

        assert_eq!(text.len() as u64, UI_CONFIG_MAX_BYTES);
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn config_reader_rejects_one_byte_over_the_limit() {
        let dir = temp_config_dir("over_limit");
        let path = dir.join(CONFIG_FILENAME);
        fs::write(
            &path,
            vec![b' '; UI_CONFIG_MAX_BYTES.saturating_add(1) as usize],
        )
        .unwrap();

        let error = read_config_text(&path).expect_err("oversized config must be rejected");

        assert!(matches!(&error, ConfigFileLoadError::TooLarge));
        assert_eq!(
            error.to_string(),
            format!("UI config exceeds the {UI_CONFIG_MAX_BYTES}-byte limit")
        );
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn config_reader_rejects_a_directory_with_a_typed_error() {
        let dir = temp_config_dir("directory");
        let path = dir.join(CONFIG_FILENAME);
        fs::create_dir(&path).unwrap();

        let error = read_config_text(&path).expect_err("directory must not be read as config");

        assert!(matches!(error, ConfigFileLoadError::NotAFile));
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn config_reader_detects_growth_after_open_handle_metadata() {
        use std::io::Write as _;

        let dir = temp_config_dir("growth_after_open");
        let path = dir.join(CONFIG_FILENAME);
        fs::write(&path, vec![b' '; UI_CONFIG_MAX_BYTES as usize]).unwrap();
        let file = fs::File::open(&path).unwrap();
        let opened_size = file.metadata().unwrap().len();
        let mut writer = fs::OpenOptions::new().append(true).open(&path).unwrap();
        writer.write_all(b"x").unwrap();
        writer.flush().unwrap();

        let error = read_config_text_from_open_file(file, opened_size)
            .expect_err("post-metadata growth must be rejected");

        assert!(matches!(error, ConfigFileLoadError::TooLarge));
        drop(writer);
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn load_adopts_legacy_config_when_new_path_missing() {
        let dir = temp_config_dir("legacy");
        let legacy = dir.join("legacy").join(CONFIG_FILENAME);
        write_config(&legacy, 0.5);
        let path = dir.join(CONFIG_FILENAME);

        let (loaded, warning) = load_with_paths(&path, Some(&legacy));

        assert_eq!(warning, None);
        assert_eq!(loaded.opacity, 0.5);
        assert!(path.is_file(), "legacy config should be copied to new path");
        assert!(legacy.is_file(), "legacy config must stay in place");
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn load_prefers_existing_config_over_legacy() {
        let dir = temp_config_dir("prefer_new");
        let legacy = dir.join("legacy").join(CONFIG_FILENAME);
        write_config(&legacy, 0.9);
        let path = dir.join(CONFIG_FILENAME);
        write_config(&path, 0.5);

        let (loaded, warning) = load_with_paths(&path, Some(&legacy));

        assert_eq!(warning, None);
        assert_eq!(loaded.opacity, 0.5);
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn load_without_any_config_creates_default_at_new_path() {
        let dir = temp_config_dir("fresh");
        let path = dir.join(CONFIG_FILENAME);

        let (_, warning) = load_with_paths(&path, None);

        assert_eq!(warning, None);
        assert!(path.is_file(), "default config should be created");
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn sanitizes_invalid_opacity() {
        assert_eq!(
            UiConfig {
                opacity: 2.0,
                ..UiConfig::default()
            }
            .sanitized()
            .opacity,
            1.0
        );
        assert_eq!(
            UiConfig {
                opacity: f32::NAN,
                ..UiConfig::default()
            }
            .sanitized()
            .opacity,
            UiConfig::default().opacity
        );
    }

    #[test]
    fn sanitizes_invalid_window_size() {
        // Below the per-window minimum is clamped up to it.
        assert_eq!(
            UiConfig {
                main_window_size: Some([10.0, 10.0]),
                ..UiConfig::default()
            }
            .sanitized()
            .main_window_size,
            Some(MAIN_WINDOW_MIN_SIZE)
        );
        // Absurdly large is clamped down to the ceiling.
        assert_eq!(
            UiConfig {
                console_window_size: Some([99999.0, 99999.0]),
                ..UiConfig::default()
            }
            .sanitized()
            .console_window_size,
            Some([WINDOW_SIZE_MAX, WINDOW_SIZE_MAX])
        );
        // Non-finite falls back to "use the base size".
        assert_eq!(
            UiConfig {
                console_window_size: Some([f32::NAN, 640.0]),
                ..UiConfig::default()
            }
            .sanitized()
            .console_window_size,
            None
        );
    }

    #[test]
    fn sanitizes_window_positions_without_discarding_secondary_monitor_coordinates() {
        let config = UiConfig {
            main_window_position: Some([-1920.0, 84.0]),
            console_window_position: Some([f32::NAN, 120.0]),
            hud_window_position: Some([-1920, 84]),
            ..UiConfig::default()
        }
        .sanitized();

        assert_eq!(config.main_window_position, Some([-1920.0, 84.0]));
        assert_eq!(config.console_window_position, None);
        assert_eq!(config.hud_window_position, Some([-1920, 84]));
    }

    #[test]
    fn legacy_config_defaults_the_tauri_hud_position() {
        let config: UiConfig = serde_json::from_str("{}").expect("legacy config");

        assert_eq!(config.hud_window_position, None);
    }

    #[test]
    fn legacy_always_on_top_migrates_without_coupling_desktop_windows() {
        let disabled = serde_json::from_str::<UiConfig>(r#"{"always_on_top":false}"#)
            .expect("legacy config")
            .sanitized();
        assert_eq!(disabled.main_dps_always_on_top, Some(false));
        assert_eq!(disabled.hud_always_on_top, Some(false));
        assert_eq!(disabled.console_always_on_top, Some(false));
        assert_eq!(disabled.abyss_values_always_on_top, Some(false));

        let independent = UiConfig {
            always_on_top: false,
            main_dps_always_on_top: Some(true),
            hud_always_on_top: Some(false),
            console_always_on_top: Some(true),
            abyss_values_always_on_top: Some(false),
            character_details_always_on_top: Some(true),
            team_details_always_on_top: Some(false),
            ..UiConfig::default()
        }
        .sanitized();
        assert_eq!(independent.main_dps_always_on_top, Some(true));
        assert_eq!(independent.hud_always_on_top, Some(false));
        assert_eq!(independent.console_always_on_top, Some(true));
        assert_eq!(independent.character_details_always_on_top, Some(true));
        assert_eq!(independent.team_details_always_on_top, Some(false));
    }

    #[test]
    fn tauri_hud_position_roundtrips_physical_secondary_monitor_coordinates() {
        let config = UiConfig {
            hud_window_position: Some([-1920, 84]),
            ..UiConfig::default()
        };
        let json = serde_json::to_string(&config).expect("config should serialize");
        let decoded: UiConfig = serde_json::from_str(&json).expect("config should deserialize");

        assert_eq!(decoded.hud_window_position, Some([-1920, 84]));
    }

    #[test]
    fn sanitizes_invalid_timeline_bucket_seconds() {
        assert_eq!(
            UiConfig {
                timeline_bucket_seconds: 0.05,
                ..UiConfig::default()
            }
            .sanitized()
            .timeline_bucket_seconds,
            TIMELINE_BUCKET_SECONDS_MIN
        );
        assert_eq!(
            UiConfig {
                timeline_bucket_seconds: 99.0,
                ..UiConfig::default()
            }
            .sanitized()
            .timeline_bucket_seconds,
            TIMELINE_BUCKET_SECONDS_MAX
        );
        assert_eq!(
            UiConfig {
                timeline_bucket_seconds: f32::NAN,
                ..UiConfig::default()
            }
            .sanitized()
            .timeline_bucket_seconds,
            TIMELINE_BUCKET_SECONDS_DEFAULT
        );
    }

    #[test]
    fn sanitizes_blank_manual_capture_device() {
        assert_eq!(
            UiConfig {
                manual_capture_device: Some("   ".to_owned()),
                ..UiConfig::default()
            }
            .sanitized()
            .manual_capture_device,
            None
        );
        assert_eq!(
            UiConfig {
                manual_capture_device: Some(r"\Device\NPF_{abc}".to_owned()),
                ..UiConfig::default()
            }
            .sanitized()
            .manual_capture_device,
            Some(r"\Device\NPF_{abc}".to_owned())
        );
    }

    #[test]
    fn sanitizes_mod_studio_game_directories() {
        let config = UiConfig {
            mod_studio_china_game_directory: Some("  D:\\Game  ".to_owned()),
            mod_studio_global_game_directory: Some("\0invalid".to_owned()),
            ..UiConfig::default()
        }
        .sanitized();

        assert_eq!(
            config.mod_studio_china_game_directory,
            Some("D:\\Game".to_owned())
        );
        assert_eq!(config.mod_studio_global_game_directory, None);
        assert_eq!(
            UiConfig {
                mod_studio_china_game_directory: Some(
                    "x".repeat(MOD_STUDIO_GAME_DIRECTORY_MAX_BYTES + 1),
                ),
                ..UiConfig::default()
            }
            .sanitized()
            .mod_studio_china_game_directory,
            None
        );
    }

    #[test]
    fn mod_studio_preferences_default_and_round_trip() {
        let defaulted: UiConfig = serde_json::from_str("{}").expect("default UI config");
        assert_eq!(
            defaulted.mod_studio_loading_method,
            ModStudioLoadingMethod::Proxy
        );
        assert!(!defaulted.mod_studio_risk_acknowledged);

        let encoded = serde_json::to_string(&UiConfig {
            mod_studio_loading_method: ModStudioLoadingMethod::Loader,
            mod_studio_risk_acknowledged: true,
            ..UiConfig::default()
        })
        .expect("serialize UI config");
        let decoded: UiConfig = serde_json::from_str(&encoded).expect("deserialize UI config");
        assert_eq!(
            decoded.mod_studio_loading_method,
            ModStudioLoadingMethod::Loader
        );
        assert!(decoded.mod_studio_risk_acknowledged);
    }

    #[test]
    fn hud_presets_are_distinct() {
        assert_ne!(HudConfig::minimal(), HudConfig::default());
        assert_ne!(HudConfig::detailed(), HudConfig::default());
        assert!(HudConfig::detailed().show_mini_timeline);
        assert!(!HudConfig::minimal().show_total_damage);
    }

    #[test]
    fn hud_editor_configuration_is_sanitized() {
        let config = HudConfig {
            width: u16::MAX,
            module_order: vec![HudModule::Timeline, HudModule::Timeline, HudModule::Title],
            ..HudConfig::default()
        }
        .sanitized();

        assert_eq!(config.width, HUD_WIDTH_MAX);
        assert_eq!(
            config.module_order,
            [
                HudModule::Timeline,
                HudModule::Title,
                HudModule::Summary,
                HudModule::Status,
                HudModule::Characters,
            ]
        );

        let mut visibility = HudConfig::default();
        visibility.set_module_visible(HudModule::Summary, false);
        assert!(!visibility.module_visible(HudModule::Summary));
        visibility.set_module_visible(HudModule::Summary, true);
        assert!(visibility.module_visible(HudModule::Summary));

        let minimum = HudConfig {
            width: 0,
            ..HudConfig::default()
        }
        .sanitized();
        assert_eq!(minimum.width, HUD_WIDTH_MIN);
    }

    #[test]
    fn hit_detail_columns_are_persisted_and_sanitized() {
        let mut columns = HitDetailColumnsConfig {
            time_width: 0,
            type_width: u16::MAX,
            ..HitDetailColumnsConfig::default()
        };
        columns.set_visible(HitDetailColumn::TargetHp, false);
        let config = UiConfig {
            hit_detail_columns: columns,
            ..UiConfig::default()
        }
        .sanitized();

        assert_eq!(
            config.hit_detail_columns.width(HitDetailColumn::Time),
            HIT_DETAIL_COLUMN_WIDTH_MIN
        );
        assert_eq!(
            config.hit_detail_columns.width(HitDetailColumn::Type),
            HIT_DETAIL_COLUMN_WIDTH_MAX
        );
        assert!(!config.hit_detail_columns.visible(HitDetailColumn::TargetHp));

        let json = serde_json::to_string(&config).expect("config should serialize");
        let restored: UiConfig = serde_json::from_str(&json).expect("config should deserialize");
        assert_eq!(restored.hit_detail_columns, config.hit_detail_columns);
    }

    #[test]
    fn interaction_preferences_use_stable_serialized_codes() {
        assert_eq!(
            AccentColor::all()
                .iter()
                .map(|value| serde_json::to_string(value).unwrap())
                .collect::<Vec<_>>(),
            [
                "\"zinc\"",
                "\"blue\"",
                "\"violet\"",
                "\"orange\"",
                "\"green\"",
            ]
        );
        assert_eq!(
            AccentColor::all()
                .iter()
                .map(|value| value.label())
                .collect::<Vec<_>>(),
            ["Zinc", "Blue", "Violet", "Orange", "Green"]
        );
        assert_eq!(
            UiDensity::all()
                .iter()
                .map(|value| serde_json::to_string(value).unwrap())
                .collect::<Vec<_>>(),
            ["\"compact\"", "\"cozy\"", "\"comfortable\""]
        );
        assert_eq!(
            UiDensity::all()
                .iter()
                .map(|value| value.label())
                .collect::<Vec<_>>(),
            ["Compact", "Cozy", "Comfortable"]
        );
        assert_eq!(
            ThemePreset::all()
                .iter()
                .map(|value| serde_json::to_string(value).unwrap())
                .collect::<Vec<_>>(),
            ["\"zinc\"", "\"tactical\"", "\"high_contrast\""]
        );
        assert_eq!(
            ThemePreset::all()
                .iter()
                .map(|value| value.label())
                .collect::<Vec<_>>(),
            ["Zinc Theme", "Tactical", "High Contrast"]
        );
    }

    #[test]
    fn older_config_defaults_interaction_preferences() {
        let config: UiConfig = serde_json::from_str(r#"{"opacity":0.75,"dark_mode":true}"#)
            .expect("older config should deserialize");

        assert_eq!(config.opacity, 0.75);
        assert!(config.dark_mode);
        assert_eq!(config.theme_preset, ThemePreset::Zinc);
        assert_eq!(config.accent, AccentColor::Zinc);
        assert_eq!(config.density, UiDensity::Cozy);
        assert_eq!(config.hud.width, HUD_WIDTH_DEFAULT);
        assert_eq!(config.hud.module_order, HudModule::all());
        assert_eq!(config.hit_detail_columns, HitDetailColumnsConfig::default());
        assert!(!config.reduce_motion);
        assert!(config.auto_check_updates);
        assert!(!config.auto_download_updates);
        assert_eq!(config.capture_filter, "udp");
        assert!(!config.separate_reaction_damage);
        assert!(!config.auto_round_after_idle);
        assert_eq!(
            config.auto_round_idle_seconds,
            AUTO_ROUND_IDLE_SECONDS_DEFAULT
        );
        assert_eq!(config.global_hotkeys, GlobalHotkeys::default());
        assert_eq!(config.main_dps_display, MainDpsDisplayConfig::default());
        assert!(config.onboarding_done);

        let f9_config: UiConfig = serde_json::from_str(r#"{"passthrough_hotkey":"f9"}"#)
            .expect("legacy F9 config should deserialize");
        assert_eq!(
            f9_config.passthrough_hotkey,
            HotkeyBinding::new(false, false, false, HotkeyKey::F9)
        );
        assert_eq!(
            f9_config.sanitized().global_hotkeys.capture,
            GlobalHotkeys::default().capture
        );
    }

    #[test]
    fn auto_round_idle_seconds_are_bounded() {
        let minimum = UiConfig {
            auto_round_idle_seconds: 0,
            ..UiConfig::default()
        }
        .sanitized();
        let maximum = UiConfig {
            auto_round_idle_seconds: u32::MAX,
            ..UiConfig::default()
        }
        .sanitized();

        assert_eq!(minimum.auto_round_idle_seconds, AUTO_ROUND_IDLE_SECONDS_MIN);
        assert_eq!(maximum.auto_round_idle_seconds, AUTO_ROUND_IDLE_SECONDS_MAX);
    }

    #[test]
    fn capture_filter_is_backward_compatible_trimmed_and_bounded() {
        let legacy: UiConfig = serde_json::from_str("{}").expect("legacy config");
        assert_eq!(legacy.capture_filter, "udp");

        let custom = UiConfig {
            capture_filter: "  udp port 30196  ".to_owned(),
            ..UiConfig::default()
        }
        .sanitized();
        assert_eq!(custom.capture_filter, "udp port 30196");
        assert_eq!(sanitize_capture_filter(""), "udp");
        assert_eq!(sanitize_capture_filter("tcp\nport 80"), "udp");
        assert_eq!(sanitize_capture_filter(&"x".repeat(513)), "udp");
    }

    #[test]
    fn global_hotkeys_round_trip_with_stable_codes() {
        let hotkeys = GlobalHotkeys {
            enabled: false,
            capture: Some(HotkeyBinding::new(true, true, false, HotkeyKey::F12)),
            reset: None,
            hud: Some(HotkeyBinding::new(false, false, true, HotkeyKey::F7)),
            new_round: Some(HotkeyBinding::new(true, false, true, HotkeyKey::F6)),
        };

        let json = serde_json::to_string(&hotkeys).expect("hotkeys should serialize");
        let decoded: GlobalHotkeys =
            serde_json::from_str(&json).expect("hotkeys should deserialize");

        assert_eq!(decoded, hotkeys);
        assert!(json.contains("\"f12\""));
        assert!(json.contains("\"f7\""));
        assert!(json.contains("\"f6\""));
        assert_eq!(hotkeys.capture.unwrap().label(), "Ctrl+Alt+F12");
        assert_eq!(
            GlobalHotkeyAction::ToggleCapture.label(),
            "Start / Stop Capture"
        );
    }

    #[test]
    fn sanitizes_duplicate_and_passthrough_conflicting_hotkeys() {
        let duplicate = HotkeyBinding::new(true, false, false, HotkeyKey::F9);
        let hotkeys = GlobalHotkeys {
            capture: Some(duplicate),
            reset: Some(duplicate),
            hud: Some(duplicate),
            new_round: Some(duplicate),
            ..GlobalHotkeys::default()
        }
        .sanitized();
        assert_eq!(hotkeys.capture, Some(duplicate));
        assert_eq!(hotkeys.reset, None);
        assert_eq!(hotkeys.hud, None);
        assert_eq!(hotkeys.new_round, None);

        let plain_f9 = HotkeyBinding::new(false, false, false, HotkeyKey::F9);
        let config = UiConfig {
            passthrough_hotkey: plain_f9,
            global_hotkeys: GlobalHotkeys {
                capture: Some(plain_f9),
                ..GlobalHotkeys::default()
            },
            ..UiConfig::default()
        }
        .sanitized();
        assert_eq!(config.global_hotkeys.capture, None);
    }

    #[test]
    fn preserves_unmodified_global_hotkeys() {
        let hotkeys = GlobalHotkeys {
            capture: Some(HotkeyBinding::new(false, false, false, HotkeyKey::F9)),
            ..GlobalHotkeys::default()
        }
        .sanitized();

        assert_eq!(
            hotkeys.capture,
            Some(HotkeyBinding::new(false, false, false, HotkeyKey::F9))
        );
        assert!(hotkeys.reset.is_some());
        assert!(hotkeys.hud.is_some());
    }

    #[test]
    fn sanitizes_windows_reserved_global_hotkeys() {
        let hotkeys = GlobalHotkeys {
            capture: Some(HotkeyBinding::new(false, true, false, HotkeyKey::F4)),
            ..GlobalHotkeys::default()
        };
        assert_eq!(hotkeys.sanitized().capture, None);
    }

    #[test]
    fn main_dps_display_is_persisted_and_deduplicated() {
        let config = UiConfig {
            main_dps_display: MainDpsDisplayConfig {
                metrics: vec![MainDpsMetric::Duration, MainDpsMetric::Duration],
                attributions: vec![
                    MainDpsAttribution::MaxHpReduction,
                    MainDpsAttribution::Character,
                    MainDpsAttribution::MaxHpReduction,
                ],
            },
            ..UiConfig::default()
        }
        .sanitized();

        assert_eq!(config.main_dps_display.metrics, [MainDpsMetric::Duration]);
        assert_eq!(
            config.main_dps_display.attributions,
            [
                MainDpsAttribution::MaxHpReduction,
                MainDpsAttribution::Character
            ]
        );
        let json = serde_json::to_string(&config).expect("serialize display settings");
        let restored: UiConfig = serde_json::from_str(&json).expect("restore display settings");
        assert_eq!(restored.main_dps_display, config.main_dps_display);
    }

    #[test]
    fn onboarding_only_opens_for_a_new_install() {
        assert!(UiConfig::default().onboarding_done);
        assert!(!new_install_config().onboarding_done);
    }
}
