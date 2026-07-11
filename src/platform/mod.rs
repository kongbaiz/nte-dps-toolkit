//! Windows / OS integration: game process and NIC detection, window rounding /
//! transparency / topmost handling, the global passthrough hotkey and native
//! file-drop bridging. `network` and `locale` are shared with the headless CLI
//! build; the window/hotkey/drop bridges only exist for the GUI.

#[cfg(feature = "gui")]
pub mod file_drop;
#[cfg(feature = "gui")]
pub mod hotkey;
pub mod locale;
pub mod network;
#[cfg(feature = "gui")]
pub mod window_attributes;
