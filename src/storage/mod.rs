//! Persistence and resource access: UI config load/save/migration, the local
//! de-identified history library, atomic file I/O helpers and embedded/external
//! resource reads.

pub mod capture_logs;
pub mod config;
pub mod history;
pub mod io_util;
pub mod resource;
