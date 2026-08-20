//! Windows / OS integration shared by the Rust core and Tauri adapter.

pub mod locale;
#[cfg(all(windows, feature = "desktop"))]
pub mod mod_loader;
pub mod mods_plugin;
#[cfg(all(windows, feature = "desktop"))]
pub mod mods_plugin_bootstrap;
pub mod network;
#[cfg(windows)]
pub mod passthrough_hotkey;
#[cfg(feature = "desktop")]
pub mod update_http;
#[cfg(feature = "desktop")]
pub mod update_install;
#[cfg(windows)]
pub mod window_style;
