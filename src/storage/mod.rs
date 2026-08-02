//! Persistence and resource access: UI config load/save/migration, the local
//! de-identified history library, atomic file I/O helpers, embedded/external
//! resource reads and UI localization.

pub mod ability_names;
pub mod capture_logs;
pub mod config;
pub mod history;
pub mod i18n;
pub mod io_util;
#[cfg(feature = "desktop")]
pub mod mod_scripts;
pub mod paths;
pub mod resource;
#[cfg(feature = "desktop")]
pub mod update;
