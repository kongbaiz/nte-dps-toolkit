//! Windows / OS integration: game process and NIC detection, window rounding /
//! transparency / topmost handling, the global passthrough hotkey and native
//! file-drop bridging. `mods_plugin`, `network`, and `locale` are shared
//! with the headless CLI build; the window/hotkey/drop bridges only exist for
//! the GUI.

#[cfg(all(windows, any(feature = "desktop", feature = "gui")))]
pub mod file_dialog;
#[cfg(feature = "gui")]
pub mod file_drop;
#[cfg(feature = "gui")]
pub mod hotkey;
pub mod locale;
pub mod mods_plugin;
pub mod network;
#[cfg(windows)]
pub mod passthrough_hotkey;
#[cfg(any(feature = "desktop", feature = "gui"))]
pub mod update_http;
#[cfg(any(feature = "desktop", feature = "gui"))]
pub mod update_install;
#[cfg(feature = "gui")]
pub mod window_attributes;
#[cfg(windows)]
pub mod window_style;
