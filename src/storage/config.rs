use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use super::i18n::Language;
use eframe::egui;

const CONFIG_DIRECTORY: &str = "NTE DPS Tool";
const CONFIG_FILENAME: &str = "config.json";
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

const PASSTHROUGH_HOTKEYS: [PassthroughHotkey; 4] = [
    PassthroughHotkey::Home,
    PassthroughHotkey::Insert,
    PassthroughHotkey::F8,
    PassthroughHotkey::F9,
];
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
const UI_DENSITIES: [UiDensity; 3] = [UiDensity::Compact, UiDensity::Cozy, UiDensity::Comfortable];
const GLOBAL_HOTKEY_ACTIONS: [GlobalHotkeyAction; 3] = [
    GlobalHotkeyAction::ToggleCapture,
    GlobalHotkeyAction::ResetSession,
    GlobalHotkeyAction::ToggleHud,
];
const HOTKEY_KEYS: [HotkeyKey; 12] = [
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
];

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PassthroughHotkey {
    #[default]
    Home,
    Insert,
    F8,
    F9,
}

impl PassthroughHotkey {
    pub fn all() -> &'static [Self] {
        &PASSTHROUGH_HOTKEYS
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Home => "Home",
            Self::Insert => "Insert",
            Self::F8 => "F8",
            Self::F9 => "F9",
        }
    }

    pub fn egui_key(self) -> egui::Key {
        match self {
            Self::Home => egui::Key::Home,
            Self::Insert => egui::Key::Insert,
            Self::F8 => egui::Key::F8,
            Self::F9 => egui::Key::F9,
        }
    }

    pub fn matches_egui(self, modifiers: egui::Modifiers, key: egui::Key) -> bool {
        self.egui_key() == key && !modifiers.ctrl && !modifiers.alt && !modifiers.shift
    }

    fn global_binding(self) -> Option<HotkeyBinding> {
        let key = match self {
            Self::F8 => HotkeyKey::F8,
            Self::F9 => HotkeyKey::F9,
            Self::Home | Self::Insert => return None,
        };
        Some(HotkeyBinding::new(false, false, false, key))
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DpsTimeMode {
    #[default]
    TimeStopAdjusted,
    RealTime,
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
            Self::TimeStopAdjusted => "Output time is not counted during ultimate/extra time-stop",
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
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HotkeyKey {
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
}

impl HotkeyKey {
    pub fn all() -> &'static [Self] {
        &HOTKEY_KEYS
    }

    pub fn label(self) -> &'static str {
        match self {
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
        }
    }

    pub fn egui_key(self) -> egui::Key {
        match self {
            Self::F1 => egui::Key::F1,
            Self::F2 => egui::Key::F2,
            Self::F3 => egui::Key::F3,
            Self::F4 => egui::Key::F4,
            Self::F5 => egui::Key::F5,
            Self::F6 => egui::Key::F6,
            Self::F7 => egui::Key::F7,
            Self::F8 => egui::Key::F8,
            Self::F9 => egui::Key::F9,
            Self::F10 => egui::Key::F10,
            Self::F11 => egui::Key::F11,
            Self::F12 => egui::Key::F12,
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

    pub fn matches_egui(self, modifiers: egui::Modifiers, key: egui::Key) -> bool {
        self.key.egui_key() == key
            && self.ctrl == modifiers.ctrl
            && self.alt == modifiers.alt
            && self.shift == modifiers.shift
    }

    pub fn is_reserved(self) -> bool {
        self.alt && self.key == HotkeyKey::F4
    }

    const fn has_modifier(self) -> bool {
        self.ctrl || self.alt || self.shift
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
}

impl GlobalHotkeys {
    pub fn binding(self, action: GlobalHotkeyAction) -> Option<HotkeyBinding> {
        match action {
            GlobalHotkeyAction::ToggleCapture => self.capture,
            GlobalHotkeyAction::ResetSession => self.reset,
            GlobalHotkeyAction::ToggleHud => self.hud,
        }
    }

    pub fn set_binding(&mut self, action: GlobalHotkeyAction, binding: Option<HotkeyBinding>) {
        match action {
            GlobalHotkeyAction::ToggleCapture => self.capture = binding,
            GlobalHotkeyAction::ResetSession => self.reset = binding,
            GlobalHotkeyAction::ToggleHud => self.hud = binding,
        }
    }

    pub fn sanitized(mut self) -> Self {
        for action in GlobalHotkeyAction::all() {
            if self
                .binding(*action)
                .is_some_and(|binding| !binding.has_modifier() || binding.is_reserved())
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
        }
    }
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

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct HudConfig {
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
        }
    }

    /// Retained as the config-sanitize hook even though no field currently needs
    /// clamping, so callers (and `UiConfig::sanitized`) stay stable.
    pub fn sanitized(self) -> Self {
        self
    }

    pub fn has_summary_row(&self) -> bool {
        self.show_team_dps || self.show_duration || self.show_total_damage || self.show_damage_taken
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
    pub accent: AccentColor,
    #[serde(default)]
    pub density: UiDensity,
    #[serde(default)]
    pub reduce_motion: bool,
    pub always_on_top: bool,
    pub server_damage_calibration: bool,
    /// Manual capture-NIC override (the Npcap device `name`, e.g. `\Device\NPF_{GUID}`). `None`
    /// keeps automatic detection; `Some(name)` pins capture to that interface as a VPN fallback.
    pub manual_capture_device: Option<String>,
    pub dps_time_mode: DpsTimeMode,
    pub timeline_bucket_seconds: f32,
    pub timeline_dps_view_mode: TimelineDpsViewMode,
    pub hud: HudConfig,
    pub passthrough_hotkey: PassthroughHotkey,
    #[serde(default)]
    pub global_hotkeys: GlobalHotkeys,
    #[serde(default = "default_onboarding_done")]
    pub onboarding_done: bool,
    #[serde(default)]
    pub console_sidebar_migration_seen: bool,
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
}

impl Default for UiConfig {
    fn default() -> Self {
        Self {
            language: Language::default(),
            opacity: 0.92,
            dark_mode: false,
            accent: AccentColor::default(),
            density: UiDensity::default(),
            reduce_motion: false,
            always_on_top: true,
            server_damage_calibration: false,
            manual_capture_device: None,
            dps_time_mode: DpsTimeMode::default(),
            timeline_bucket_seconds: TIMELINE_BUCKET_SECONDS_DEFAULT,
            timeline_dps_view_mode: TimelineDpsViewMode::default(),
            hud: HudConfig::default(),
            passthrough_hotkey: PassthroughHotkey::default(),
            global_hotkeys: GlobalHotkeys::default(),
            onboarding_done: true,
            console_sidebar_migration_seen: false,
            main_window_size: None,
            abyss_window_size: None,
            hit_detail_window_size: None,
            team_hit_detail_window_size: None,
            console_window_size: None,
        }
    }
}

impl UiConfig {
    pub fn sanitized(mut self) -> Self {
        self.opacity = if self.opacity.is_finite() {
            self.opacity.clamp(0.35, 1.0)
        } else {
            Self::default().opacity
        };
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
        self.timeline_bucket_seconds =
            sanitize_timeline_bucket_seconds(self.timeline_bucket_seconds);
        self.manual_capture_device = self
            .manual_capture_device
            .take()
            .filter(|name| !name.trim().is_empty());
        self.hud = self.hud.sanitized();
        self.global_hotkeys = self.global_hotkeys.sanitized();
        if let Some(binding) = self.passthrough_hotkey.global_binding() {
            self.global_hotkeys = self.global_hotkeys.without_binding(binding);
        }
        self
    }
}

const fn default_onboarding_done() -> bool {
    true
}

fn new_install_config() -> UiConfig {
    UiConfig {
        language: Language::system_default(),
        onboarding_done: false,
        console_sidebar_migration_seen: true,
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

pub fn config_path() -> PathBuf {
    std::env::var_os("LOCALAPPDATA")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."))
        .join(CONFIG_DIRECTORY)
        .join(CONFIG_FILENAME)
}

pub fn load() -> (UiConfig, Option<String>) {
    let path = config_path();
    if !path.is_file() {
        // Brand-new install: pick the UI language from the system locale (if a
        // localization file matches it) instead of the historical
        // Simplified-Chinese default, which only exists to keep upgrades from
        // older (pre-i18n) configs stable — see `Language::system_default`.
        let config = new_install_config();
        let warning = save(&path, &config).err().map(|error| {
            crate::storage::i18n::tf(
                "Failed to create default UI config ({}): {}",
                &[&path.display().to_string(), &error],
            )
        });
        return (config, warning);
    }
    match fs::read_to_string(&path)
        .map_err(|error| error.to_string())
        .and_then(|text| serde_json::from_str::<UiConfig>(&text).map_err(|error| error.to_string()))
    {
        Ok(config) => (config.sanitized(), None),
        Err(error) => (
            UiConfig::default(),
            Some(crate::storage::i18n::tf(
                "Failed to load UI config ({}): {}",
                &[&path.display().to_string(), &error],
            )),
        ),
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
    fn hud_presets_are_distinct() {
        assert_ne!(HudConfig::minimal(), HudConfig::default());
        assert_ne!(HudConfig::detailed(), HudConfig::default());
        assert!(HudConfig::detailed().show_mini_timeline);
        assert!(!HudConfig::minimal().show_total_damage);
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
    }

    #[test]
    fn older_config_defaults_interaction_preferences() {
        let config: UiConfig = serde_json::from_str(r#"{"opacity":0.75,"dark_mode":true}"#)
            .expect("older config should deserialize");

        assert_eq!(config.opacity, 0.75);
        assert!(config.dark_mode);
        assert_eq!(config.accent, AccentColor::Zinc);
        assert_eq!(config.density, UiDensity::Cozy);
        assert!(!config.reduce_motion);
        assert_eq!(config.global_hotkeys, GlobalHotkeys::default());
        assert!(config.onboarding_done);
        assert!(!config.console_sidebar_migration_seen);

        let f9_config: UiConfig = serde_json::from_str(r#"{"passthrough_hotkey":"f9"}"#)
            .expect("legacy F9 config should deserialize");
        assert_eq!(f9_config.passthrough_hotkey, PassthroughHotkey::F9);
        assert_eq!(
            f9_config.sanitized().global_hotkeys.capture,
            GlobalHotkeys::default().capture
        );
    }

    #[test]
    fn global_hotkeys_round_trip_with_stable_codes() {
        let hotkeys = GlobalHotkeys {
            enabled: false,
            capture: Some(HotkeyBinding::new(true, true, false, HotkeyKey::F12)),
            reset: None,
            hud: Some(HotkeyBinding::new(false, false, true, HotkeyKey::F7)),
        };

        let json = serde_json::to_string(&hotkeys).expect("hotkeys should serialize");
        let decoded: GlobalHotkeys =
            serde_json::from_str(&json).expect("hotkeys should deserialize");

        assert_eq!(decoded, hotkeys);
        assert!(json.contains("\"f12\""));
        assert!(json.contains("\"f7\""));
        assert_eq!(hotkeys.capture.unwrap().label(), "Ctrl+Alt+F12");
        assert_eq!(
            GlobalHotkeyAction::ToggleCapture.label(),
            "Start / Stop Capture"
        );
    }

    #[test]
    fn hotkey_binding_matches_exact_egui_modifiers() {
        let binding = HotkeyBinding::new(true, false, false, HotkeyKey::F9);
        let ctrl = egui::Modifiers {
            ctrl: true,
            ..Default::default()
        };
        let ctrl_shift = egui::Modifiers {
            ctrl: true,
            shift: true,
            ..Default::default()
        };

        assert!(binding.matches_egui(ctrl, egui::Key::F9));
        assert!(!binding.matches_egui(ctrl_shift, egui::Key::F9));
        assert!(!binding.matches_egui(ctrl, egui::Key::F10));
        assert!(
            !HotkeyBinding::new(false, false, false, HotkeyKey::F9)
                .matches_egui(ctrl, egui::Key::F9)
        );
        assert!(PassthroughHotkey::F9.matches_egui(egui::Modifiers::default(), egui::Key::F9));
        assert!(!PassthroughHotkey::F9.matches_egui(ctrl, egui::Key::F9));
    }

    #[test]
    fn sanitizes_duplicate_and_passthrough_conflicting_hotkeys() {
        let duplicate = HotkeyBinding::new(true, false, false, HotkeyKey::F9);
        let hotkeys = GlobalHotkeys {
            capture: Some(duplicate),
            reset: Some(duplicate),
            hud: Some(duplicate),
            ..GlobalHotkeys::default()
        }
        .sanitized();
        assert_eq!(hotkeys.capture, Some(duplicate));
        assert_eq!(hotkeys.reset, None);
        assert_eq!(hotkeys.hud, None);

        let plain_f9 = HotkeyBinding::new(false, false, false, HotkeyKey::F9);
        let config = UiConfig {
            passthrough_hotkey: PassthroughHotkey::F9,
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
    fn sanitizes_unmodified_global_hotkeys() {
        let hotkeys = GlobalHotkeys {
            capture: Some(HotkeyBinding::new(false, false, false, HotkeyKey::F9)),
            ..GlobalHotkeys::default()
        }
        .sanitized();

        assert_eq!(hotkeys.capture, None);
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
    fn onboarding_only_opens_for_a_new_install() {
        assert!(UiConfig::default().onboarding_done);
        assert!(!new_install_config().onboarding_done);
        assert!(new_install_config().console_sidebar_migration_seen);
    }
}
