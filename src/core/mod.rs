//! Non-UI core shared by the Tauri desktop and the CLI sidecar: the single
//! `EngineEvent` -> `CombatState` reducer and capture environment
//! preparation/control. This layer must stay free of UI frameworks, i18n
//! (`t()`/`tf()`), stdout and JSON-RPC concerns; frontends translate
//! `CoreError` codes at their own display boundary.

pub mod capture;
pub mod character_data;
pub mod combat_details;
#[cfg(feature = "desktop")]
pub mod diagnostics;
pub mod empty_curtain;
pub mod encrypted_ini;
pub mod history;
pub mod hud;
pub mod live_capture;
#[cfg(feature = "desktop")]
pub mod mod_market;
#[cfg(feature = "desktop")]
pub mod mod_sdk;
#[cfg(feature = "desktop")]
pub mod mod_studio;
pub mod packets;
pub mod reducer;
pub mod skills;
pub mod snapshot;
#[cfg(feature = "desktop")]
pub mod team_data;
pub mod timeline;
#[cfg(feature = "desktop")]
pub mod update;

/// Stable machine-readable error category shared by both frontends. Tauri
/// picks user-facing wording per code at its contract boundary; the CLI maps
/// codes to JSON-RPC domain codes in later phases.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CoreErrorCode {
    /// Npcap DLLs could not be loaded or device enumeration failed.
    NpcapNotFound,
    /// The game process/connection could not be located.
    GameProcessNotFound,
    /// The requested capture device does not exist (e.g. a manual NIC vanished).
    CaptureDeviceNotFound,
    /// An OS-level probe (process/TCP-table query) itself failed.
    SystemProbeFailed,
    /// A capture is already active for this controller.
    CaptureAlreadyRunning,
    /// No capture is active for this controller.
    CaptureNotRunning,
    /// The live-capture runtime detected poisoned authoritative or lifecycle
    /// state and permanently stopped the current service instance.
    CaptureStateUnavailable,
}

/// `detail` carries the underlying technical message. Today those messages
/// bubble up from `platform::network` / `engine::capture`, which still
/// localize with `tf()` at the source; the strings pass through here opaquely.
#[derive(Clone, Debug)]
pub struct CoreError {
    pub code: CoreErrorCode,
    pub detail: String,
}

impl CoreError {
    pub fn new(code: CoreErrorCode, detail: impl Into<String>) -> Self {
        Self {
            code,
            detail: detail.into(),
        }
    }
}
