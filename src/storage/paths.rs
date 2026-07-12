//! Filesystem roots for everything the tool persists locally. All data (UI
//! config, history, raw capture and panic logs) lives next to the executable,
//! so the tool stays portable and files cannot scatter when the process is
//! launched with an unexpected working directory (shortcut "Start in",
//! terminal, another launcher).

use std::path::{Path, PathBuf};

/// Directory containing the running executable, falling back to the working
/// directory (then `.`) when the executable path cannot be resolved.
pub fn software_dir() -> PathBuf {
    std::env::current_exe()
        .ok()
        .and_then(|path| path.parent().map(Path::to_path_buf))
        .or_else(|| std::env::current_dir().ok())
        .unwrap_or_else(|| PathBuf::from("."))
}

/// Directory the capture engine writes `nte_raw_*.pcapng` files to; panic
/// logs land here as well.
pub fn capture_log_dir() -> PathBuf {
    software_dir().join("logs")
}
