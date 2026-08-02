//! Windows / OS integration shared by the Rust core and Tauri adapter.

#[cfg(all(windows, feature = "desktop"))]
pub mod file_dialog;
pub mod locale;
pub mod mods_plugin;
pub mod network;
#[cfg(windows)]
pub mod passthrough_hotkey;
#[cfg(feature = "desktop")]
pub mod update_http;
#[cfg(feature = "desktop")]
pub mod update_install;
#[cfg(windows)]
pub mod window_style;
