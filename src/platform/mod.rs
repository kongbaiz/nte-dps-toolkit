//! Windows / OS integration: game process and NIC detection, window rounding /
//! transparency / topmost handling, the global passthrough hotkey and native
//! file-drop bridging. `equipment_plugin`, `network`, and `locale` are shared
//! with the headless CLI build; the window/hotkey/drop bridges only exist for
//! the GUI.

pub mod equipment_plugin;
#[cfg(feature = "gui")]
pub mod file_drop;
#[cfg(feature = "gui")]
pub mod hotkey;
pub mod locale;
pub mod network;
#[cfg(feature = "gui")]
pub mod update_http;
#[cfg(feature = "gui")]
pub mod update_install;
#[cfg(feature = "gui")]
pub mod window_attributes;
