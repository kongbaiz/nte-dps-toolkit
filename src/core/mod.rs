//! Non-UI core shared by the GUI and the CLI sidecar: the single
//! `EngineEvent` -> `CombatState` reducer and capture environment
//! preparation/control. This layer must stay free of egui, i18n
//! (`t()`/`tf()`), stdout and JSON-RPC concerns; frontends translate
//! `CoreError` codes at their own display boundary.

pub mod capture;
pub mod reducer;
pub mod snapshot;

/// Stable machine-readable error category shared by both frontends. The GUI
/// picks user-facing wording per code at its display boundary; the CLI maps
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
