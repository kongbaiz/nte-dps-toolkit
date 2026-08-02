//! Shared Rust core for the Tauri desktop application (`desktop`) and the
//! headless stdio sidecar (`cli`). UI rendering lives in `src-tauri` and
//! `frontend`; this crate owns capture, parsing, persistence, update, and native
//! integration behavior.

#[cfg(feature = "cli")]
pub mod api;
#[cfg(feature = "cli")]
pub mod cli;
pub mod core;
pub mod engine;
pub mod platform;
pub mod storage;
