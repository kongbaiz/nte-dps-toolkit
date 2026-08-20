//! Explicit, loader-lock-free bootstrap for the deployed in-process Mods Plugin.
//!
//! This module is intentionally synchronous. Desktop adapters must run it on a
//! bounded blocking worker, never while holding capture/reducer state locks.

use std::ffi::c_void;
use std::fmt;
use std::fs::{self, File};
use std::io::{self, Read};
use std::mem::{self, size_of};
use std::os::windows::fs::MetadataExt;
use std::path::{Path, PathBuf};
use std::ptr;
use std::sync::{Mutex, TryLockError};
use std::thread;
use std::time::{Duration, Instant};

use windows_sys::Win32::Foundation::{
    CloseHandle, ERROR_BAD_LENGTH, ERROR_NO_MORE_FILES, FILETIME, GetLastError, HANDLE,
    INVALID_HANDLE_VALUE, STILL_ACTIVE, WAIT_FAILED, WAIT_OBJECT_0, WAIT_TIMEOUT,
};
use windows_sys::Win32::System::Diagnostics::Debug::ReadProcessMemory;
use windows_sys::Win32::System::Diagnostics::ToolHelp::{
    CreateToolhelp32Snapshot, MODULEENTRY32W, Module32FirstW, Module32NextW, PROCESSENTRY32W,
    Process32FirstW, Process32NextW, TH32CS_SNAPMODULE, TH32CS_SNAPMODULE32, TH32CS_SNAPPROCESS,
};
use windows_sys::Win32::System::Threading::{
    CreateRemoteThread, GetExitCodeThread, GetProcessTimes, LPTHREAD_START_ROUTINE, OpenProcess,
    PROCESS_CREATE_THREAD, PROCESS_QUERY_INFORMATION, PROCESS_QUERY_LIMITED_INFORMATION,
    PROCESS_VM_OPERATION, PROCESS_VM_READ, PROCESS_VM_WRITE, QueryFullProcessImageNameW,
    WaitForSingleObject,
};

pub const MODS_PLUGIN_INITIALIZE_EXPORT: &str = "NteModsPluginInitialize";
pub const DEFAULT_MODS_PLUGIN_INITIALIZE_TIMEOUT: Duration = Duration::from_secs(5);

const GAME_EXECUTABLE_NAME: &str = "HTGame.exe";
const PLUGIN_FILE_NAME: &str = "dwmapi.dll";
const MAX_PLUGIN_IMAGE_BYTES: u64 = 64 * 1024 * 1024;
const MAX_PE_HEADER_BYTES: usize = 1024 * 1024;
const MAX_PROCESS_IMAGE_UTF16: usize = 32_768;
const MAX_MODULES: usize = 4_096;
const MAX_SECTIONS: usize = 96;
const MAX_EXPORTS: usize = 4_096;
const MAX_EXPORT_NAME_BYTES: usize = 128;
const MAX_SNAPSHOT_ATTEMPTS: usize = 4;
const MAX_IN_PROGRESS_ATTEMPTS: usize = 256;
const IN_PROGRESS_RETRY_DELAY: Duration = Duration::from_millis(20);
const MIN_INITIALIZE_TIMEOUT: Duration = Duration::from_millis(1);
const MAX_INITIALIZE_TIMEOUT: Duration = Duration::from_secs(10);
const IMAGE_FILE_MACHINE_AMD64: u16 = 0x8664;
const IMAGE_NT_OPTIONAL_HDR64_MAGIC: u16 = 0x20b;
const IMAGE_SCN_MEM_EXECUTE: u32 = 0x2000_0000;

static BOOTSTRAP_GATE: Mutex<()> = Mutex::new(());

/// The two idempotent-success return values of `NteModsPluginInitialize`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ModsPluginInitializeOutcome {
    Started,
    AlreadyRunning,
}

/// Stable error categories suitable for an adapter contract.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ModsPluginBootstrapErrorCode {
    InvalidRequest,
    FileSystem,
    InvalidPluginImage,
    ProcessOpenFailed,
    ProcessIdentityFailed,
    ProcessSnapshotFailed,
    ProcessEnumerationFailed,
    ProcessAmbiguous,
    TargetMismatch,
    ModuleSnapshotFailed,
    ModuleEnumerationFailed,
    ModuleNotLoaded,
    ModulePathMismatch,
    ModuleAmbiguous,
    ModuleImageMismatch,
    RemoteThreadFailed,
    RemoteThreadTimedOut,
    RemoteThreadWaitFailed,
    RemoteThreadExitFailed,
    NotGameHost,
    InitializationInProgress,
    InitializationFailed,
    UnexpectedStatus,
}

impl ModsPluginBootstrapErrorCode {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::InvalidRequest => "INVALID_REQUEST",
            Self::FileSystem => "FILE_SYSTEM",
            Self::InvalidPluginImage => "INVALID_PLUGIN_IMAGE",
            Self::ProcessOpenFailed => "PROCESS_OPEN_FAILED",
            Self::ProcessIdentityFailed => "PROCESS_IDENTITY_FAILED",
            Self::ProcessSnapshotFailed => "PROCESS_SNAPSHOT_FAILED",
            Self::ProcessEnumerationFailed => "PROCESS_ENUMERATION_FAILED",
            Self::ProcessAmbiguous => "PROCESS_AMBIGUOUS",
            Self::TargetMismatch => "TARGET_MISMATCH",
            Self::ModuleSnapshotFailed => "MODULE_SNAPSHOT_FAILED",
            Self::ModuleEnumerationFailed => "MODULE_ENUMERATION_FAILED",
            Self::ModuleNotLoaded => "MODULE_NOT_LOADED",
            Self::ModulePathMismatch => "MODULE_PATH_MISMATCH",
            Self::ModuleAmbiguous => "MODULE_AMBIGUOUS",
            Self::ModuleImageMismatch => "MODULE_IMAGE_MISMATCH",
            Self::RemoteThreadFailed => "REMOTE_THREAD_FAILED",
            Self::RemoteThreadTimedOut => "REMOTE_THREAD_TIMED_OUT",
            Self::RemoteThreadWaitFailed => "REMOTE_THREAD_WAIT_FAILED",
            Self::RemoteThreadExitFailed => "REMOTE_THREAD_EXIT_FAILED",
            Self::NotGameHost => "NOT_GAME_HOST",
            Self::InitializationInProgress => "INITIALIZATION_IN_PROGRESS",
            Self::InitializationFailed => "INITIALIZATION_FAILED",
            Self::UnexpectedStatus => "UNEXPECTED_STATUS",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ModsPluginImageError {
    Truncated,
    InvalidDosHeader,
    InvalidPeHeader,
    UnsupportedMachine,
    UnsupportedOptionalHeader,
    InvalidSectionTable,
    InvalidImageSize,
    InvalidExportDirectory,
    ExportBudgetExceeded,
    ExportNameInvalid,
    InitializeExportMissing,
    InitializeExportAmbiguous,
    InitializeExportForwarded,
    InitializeExportNotExecutable,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ModsPluginBootstrapError {
    InvalidProcessId,
    InvalidTimeout,
    LocalBootstrapInProgress,
    LocalBootstrapStatePoisoned,
    GameExecutableUnavailable(io::ErrorKind),
    PluginUnavailable(io::ErrorKind),
    PluginPathMismatch,
    PluginTooLarge { bytes: u64, limit: u64 },
    PluginReadFailed(io::ErrorKind),
    InvalidPluginImage(ModsPluginImageError),
    ProcessOpenFailed { win32: u32 },
    ProcessImageQueryFailed { win32: u32 },
    ProcessTimesQueryFailed { win32: u32 },
    ProcessIdentityChanged,
    ProcessImageInvalid,
    ProcessSnapshotFailed { win32: u32 },
    ProcessEnumerationFailed { win32: u32 },
    ProcessBudgetExceeded,
    TargetProcessAmbiguous,
    TargetExecutableMismatch,
    ModuleSnapshotFailed { win32: u32 },
    ModuleEnumerationFailed { win32: u32 },
    ModulePathInvalid,
    ModuleBudgetExceeded,
    DeployedModuleNotLoaded,
    DeployedModulePathMismatch,
    DeployedModuleAmbiguous,
    RemoteImageSizeMismatch { expected: u32, actual: u32 },
    RemoteImageReadFailed { win32: u32 },
    RemoteImageIdentityMismatch,
    RemoteExportAddressOverflow,
    RemoteThreadCreateFailed { win32: u32 },
    RemoteThreadTimedOut,
    RemoteThreadWaitFailed { win32: u32 },
    UnexpectedWaitStatus { status: u32 },
    RemoteThreadExitQueryFailed { win32: u32 },
    RemoteThreadStillActive,
    NotGameHost,
    InitializationInProgress,
    InitializationFailed,
    UnexpectedInitializeStatus { status: u32 },
}

impl ModsPluginBootstrapError {
    pub const fn code(&self) -> ModsPluginBootstrapErrorCode {
        match self {
            Self::InvalidProcessId
            | Self::InvalidTimeout
            | Self::LocalBootstrapInProgress
            | Self::LocalBootstrapStatePoisoned
            | Self::PluginPathMismatch => ModsPluginBootstrapErrorCode::InvalidRequest,
            Self::GameExecutableUnavailable(_)
            | Self::PluginUnavailable(_)
            | Self::PluginTooLarge { .. }
            | Self::PluginReadFailed(_) => ModsPluginBootstrapErrorCode::FileSystem,
            Self::InvalidPluginImage(_) => ModsPluginBootstrapErrorCode::InvalidPluginImage,
            Self::ProcessOpenFailed { .. } => ModsPluginBootstrapErrorCode::ProcessOpenFailed,
            Self::ProcessImageQueryFailed { .. }
            | Self::ProcessTimesQueryFailed { .. }
            | Self::ProcessIdentityChanged
            | Self::ProcessImageInvalid => ModsPluginBootstrapErrorCode::ProcessIdentityFailed,
            Self::ProcessSnapshotFailed { .. } => {
                ModsPluginBootstrapErrorCode::ProcessSnapshotFailed
            }
            Self::ProcessEnumerationFailed { .. } | Self::ProcessBudgetExceeded => {
                ModsPluginBootstrapErrorCode::ProcessEnumerationFailed
            }
            Self::TargetProcessAmbiguous => ModsPluginBootstrapErrorCode::ProcessAmbiguous,
            Self::TargetExecutableMismatch => ModsPluginBootstrapErrorCode::TargetMismatch,
            Self::ModuleSnapshotFailed { .. } => ModsPluginBootstrapErrorCode::ModuleSnapshotFailed,
            Self::ModuleEnumerationFailed { .. }
            | Self::ModulePathInvalid
            | Self::ModuleBudgetExceeded => ModsPluginBootstrapErrorCode::ModuleEnumerationFailed,
            Self::DeployedModuleNotLoaded => ModsPluginBootstrapErrorCode::ModuleNotLoaded,
            Self::DeployedModulePathMismatch => ModsPluginBootstrapErrorCode::ModulePathMismatch,
            Self::DeployedModuleAmbiguous => ModsPluginBootstrapErrorCode::ModuleAmbiguous,
            Self::RemoteImageSizeMismatch { .. }
            | Self::RemoteImageReadFailed { .. }
            | Self::RemoteImageIdentityMismatch
            | Self::RemoteExportAddressOverflow => {
                ModsPluginBootstrapErrorCode::ModuleImageMismatch
            }
            Self::RemoteThreadCreateFailed { .. } => {
                ModsPluginBootstrapErrorCode::RemoteThreadFailed
            }
            Self::RemoteThreadTimedOut => ModsPluginBootstrapErrorCode::RemoteThreadTimedOut,
            Self::RemoteThreadWaitFailed { .. } | Self::UnexpectedWaitStatus { .. } => {
                ModsPluginBootstrapErrorCode::RemoteThreadWaitFailed
            }
            Self::RemoteThreadExitQueryFailed { .. } | Self::RemoteThreadStillActive => {
                ModsPluginBootstrapErrorCode::RemoteThreadExitFailed
            }
            Self::NotGameHost => ModsPluginBootstrapErrorCode::NotGameHost,
            Self::InitializationInProgress => {
                ModsPluginBootstrapErrorCode::InitializationInProgress
            }
            Self::InitializationFailed => ModsPluginBootstrapErrorCode::InitializationFailed,
            Self::UnexpectedInitializeStatus { .. } => {
                ModsPluginBootstrapErrorCode::UnexpectedStatus
            }
        }
    }
}

impl fmt::Display for ModsPluginBootstrapError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "Mods Plugin bootstrap failed ({})",
            self.code().as_str()
        )?;
        match self {
            Self::ProcessOpenFailed { win32 }
            | Self::ProcessImageQueryFailed { win32 }
            | Self::ProcessTimesQueryFailed { win32 }
            | Self::ProcessSnapshotFailed { win32 }
            | Self::ProcessEnumerationFailed { win32 }
            | Self::ModuleSnapshotFailed { win32 }
            | Self::ModuleEnumerationFailed { win32 }
            | Self::RemoteImageReadFailed { win32 }
            | Self::RemoteThreadCreateFailed { win32 }
            | Self::RemoteThreadWaitFailed { win32 }
            | Self::RemoteThreadExitQueryFailed { win32 } => {
                write!(formatter, ": Win32 error {win32}")
            }
            Self::UnexpectedWaitStatus { status } | Self::UnexpectedInitializeStatus { status } => {
                write!(formatter, ": status {status}")
            }
            _ => Ok(()),
        }
    }
}

/// Stable identity used by the desktop retry gate. It deliberately contains
/// no handles: every bootstrap attempt re-opens and re-validates the process.
/// The process creation time prevents a recycled PID from inheriting a prior
/// failure latch, while the deployed-file metadata invalidates the latch after
/// an install or replacement without hashing a potentially 64 MiB DLL at 1 Hz.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RunningModsPluginBootstrapContext {
    process_id: u32,
    process_creation_time: u64,
    game_executable: PathBuf,
    deployed_plugin: PathBuf,
    deployed_plugin_identity: DeployedPluginIdentity,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum DeployedPluginIdentity {
    Missing,
    Present {
        bytes: u64,
        creation_time: u64,
        last_write_time: u64,
        attributes: u32,
    },
}

impl std::error::Error for ModsPluginBootstrapError {}

/// Explicit low-level request. Both paths must name the exact files used by the
/// target process; the plugin must be the `dwmapi.dll` sibling of `HTGame.exe`.
#[derive(Clone, Copy, Debug)]
pub struct ModsPluginBootstrapRequest<'a> {
    pub process_id: u32,
    pub game_executable: &'a Path,
    pub deployed_plugin: &'a Path,
    pub timeout: Duration,
}

impl<'a> ModsPluginBootstrapRequest<'a> {
    pub fn new(process_id: u32, game_executable: &'a Path, deployed_plugin: &'a Path) -> Self {
        Self {
            process_id,
            game_executable,
            deployed_plugin,
            timeout: DEFAULT_MODS_PLUGIN_INITIALIZE_TIMEOUT,
        }
    }
}

/// Desktop integration entry point for the proxy deployment layout.
///
/// The caller supplies the directory that contains both `HTGame.exe` and the
/// managed `dwmapi.dll`. Run this finite blocking operation outside async/UI and
/// capture locks. The complete operation, including `IN_PROGRESS` retries, is
/// bounded by five seconds.
pub fn initialize_deployed_mods_plugin(
    process_id: u32,
    game_binary_directory: &Path,
) -> Result<ModsPluginInitializeOutcome, ModsPluginBootstrapError> {
    initialize_deployed_mods_plugin_with_timeout(
        process_id,
        game_binary_directory,
        DEFAULT_MODS_PLUGIN_INITIALIZE_TIMEOUT,
    )
}

pub fn initialize_deployed_mods_plugin_with_timeout(
    process_id: u32,
    game_binary_directory: &Path,
    timeout: Duration,
) -> Result<ModsPluginInitializeOutcome, ModsPluginBootstrapError> {
    let executable = game_binary_directory.join(GAME_EXECUTABLE_NAME);
    let plugin = game_binary_directory.join(PLUGIN_FILE_NAME);
    initialize_loaded_mods_plugin(ModsPluginBootstrapRequest {
        process_id,
        game_executable: &executable,
        deployed_plugin: &plugin,
        timeout,
    })
}

/// Inspect the bounded runtime identity needed by the desktop retry gate.
///
/// This performs process discovery and metadata queries only. It never opens a
/// remote-thread-capable handle and never reads the complete plugin image.
/// `Ok(None)` is the ordinary no-game state.
pub fn inspect_running_deployed_mods_plugin_context()
-> Result<Option<RunningModsPluginBootstrapContext>, ModsPluginBootstrapError> {
    let process_id = match find_running_game_process()? {
        Some(process_id) => process_id,
        None => return Ok(None),
    };
    // The discovery handle requests query rights only. The verified bootstrap
    // opens its own finite-lifetime handle with the CreateRemoteThread rights.
    // SAFETY: the PID came from a bounded Toolhelp snapshot; inheritance is off.
    let process = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, process_id) };
    if process.is_null() {
        return Err(ModsPluginBootstrapError::ProcessOpenFailed {
            // SAFETY: this immediately follows the failed Win32 call.
            win32: unsafe { GetLastError() },
        });
    }
    let process = OwnedHandle::new(process);
    let process_creation_time = query_process_creation_time(process.raw())?;
    let executable = query_process_image(process.raw())?;
    if !file_name_is(&executable, GAME_EXECUTABLE_NAME) {
        return Err(ModsPluginBootstrapError::TargetExecutableMismatch);
    }
    let directory = executable
        .parent()
        .ok_or(ModsPluginBootstrapError::ProcessImageInvalid)?;
    let deployed_plugin = directory.join(PLUGIN_FILE_NAME);
    let deployed_plugin_identity = match fs::metadata(&deployed_plugin) {
        Ok(metadata) => DeployedPluginIdentity::Present {
            bytes: metadata.file_size(),
            creation_time: metadata.creation_time(),
            last_write_time: metadata.last_write_time(),
            attributes: metadata.file_attributes(),
        },
        Err(error) if error.kind() == io::ErrorKind::NotFound => DeployedPluginIdentity::Missing,
        Err(error) => return Err(ModsPluginBootstrapError::PluginUnavailable(error.kind())),
    };
    Ok(Some(RunningModsPluginBootstrapContext {
        process_id,
        process_creation_time,
        game_executable: executable,
        deployed_plugin,
        deployed_plugin_identity,
    }))
}

/// Bootstrap the exact process/deployment identity returned by
/// [`inspect_running_deployed_mods_plugin_context`]. The identity is verified
/// again before the finite remote operation so PID reuse cannot inherit either
/// a stale request or a stale failure latch.
pub fn initialize_running_deployed_mods_plugin_context(
    context: &RunningModsPluginBootstrapContext,
) -> Result<ModsPluginInitializeOutcome, ModsPluginBootstrapError> {
    // SAFETY: the PID came from a bounded Toolhelp snapshot; inheritance is off.
    let process = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, context.process_id) };
    if process.is_null() {
        return Err(ModsPluginBootstrapError::ProcessOpenFailed {
            // SAFETY: this immediately follows the failed Win32 call.
            win32: unsafe { GetLastError() },
        });
    }
    let process = OwnedHandle::new(process);
    if query_process_creation_time(process.raw())? != context.process_creation_time
        || !paths_equal(
            &query_process_image(process.raw())?,
            &context.game_executable,
        )
    {
        return Err(ModsPluginBootstrapError::ProcessIdentityChanged);
    }
    let game_directory = context
        .game_executable
        .parent()
        .ok_or(ModsPluginBootstrapError::ProcessImageInvalid)?;
    let plugin_directory = context
        .deployed_plugin
        .parent()
        .ok_or(ModsPluginBootstrapError::PluginPathMismatch)?;
    if !paths_equal(game_directory, plugin_directory) {
        return Err(ModsPluginBootstrapError::PluginPathMismatch);
    }
    initialize_deployed_mods_plugin(context.process_id, game_directory)
}

/// Runtime-monitor integration: locate one running `HTGame.exe`, derive the
/// deployment directory from its authenticated executable path, and bootstrap
/// the already loaded sibling proxy. `Ok(None)` is the ordinary no-game state.
pub fn initialize_running_deployed_mods_plugin()
-> Result<Option<ModsPluginInitializeOutcome>, ModsPluginBootstrapError> {
    inspect_running_deployed_mods_plugin_context()?
        .as_ref()
        .map(initialize_running_deployed_mods_plugin_context)
        .transpose()
}

/// Verify the local deployment and loaded target module, then invoke the stable
/// lifecycle export on a fresh target-process thread. No remote allocation or
/// string injection is performed.
pub fn initialize_loaded_mods_plugin(
    request: ModsPluginBootstrapRequest<'_>,
) -> Result<ModsPluginInitializeOutcome, ModsPluginBootstrapError> {
    validate_request(&request)?;
    let _gate = match BOOTSTRAP_GATE.try_lock() {
        Ok(guard) => guard,
        Err(TryLockError::WouldBlock) => {
            return Err(ModsPluginBootstrapError::LocalBootstrapInProgress);
        }
        Err(TryLockError::Poisoned(_)) => {
            return Err(ModsPluginBootstrapError::LocalBootstrapStatePoisoned);
        }
    };

    let game_executable = canonical_file(
        request.game_executable,
        ModsPluginBootstrapError::GameExecutableUnavailable,
    )?;
    let deployed_plugin = canonical_file(
        request.deployed_plugin,
        ModsPluginBootstrapError::PluginUnavailable,
    )?;
    validate_deployment_layout(&game_executable, &deployed_plugin)?;
    let local_image = read_plugin_image(&deployed_plugin)?;

    // CreateRemoteThread documents this complete access set. No inherited
    // handle and no remote memory allocation are used.
    let access = PROCESS_CREATE_THREAD
        | PROCESS_QUERY_INFORMATION
        | PROCESS_VM_OPERATION
        | PROCESS_VM_WRITE
        | PROCESS_VM_READ;
    // SAFETY: the PID is non-zero, access is a documented process-right mask,
    // and handle inheritance is disabled. A non-null result is owned below.
    let process = unsafe { OpenProcess(access, 0, request.process_id) };
    if process.is_null() {
        return Err(ModsPluginBootstrapError::ProcessOpenFailed {
            // SAFETY: this immediately follows the failed Win32 call.
            win32: unsafe { GetLastError() },
        });
    }
    let process = OwnedHandle::new(process);
    validate_process_image(process.raw(), &game_executable)?;
    let remote = find_deployed_module(request.process_id, &deployed_plugin)?;
    if remote.image_size != local_image.size_of_image {
        return Err(ModsPluginBootstrapError::RemoteImageSizeMismatch {
            expected: local_image.size_of_image,
            actual: remote.image_size,
        });
    }
    let remote_export_rva = validate_remote_plugin_image(process.raw(), remote, &local_image)?;
    let remote_export = remote
        .base
        .checked_add(remote_export_rva as usize)
        .filter(|address| *address >= remote.base)
        .filter(|address| *address < remote.base.saturating_add(remote.image_size as usize))
        .ok_or(ModsPluginBootstrapError::RemoteExportAddressOverflow)?;

    let deadline = Instant::now()
        .checked_add(request.timeout)
        .ok_or(ModsPluginBootstrapError::InvalidTimeout)?;
    for _ in 0..MAX_IN_PROGRESS_ATTEMPTS {
        let remaining = deadline
            .checked_duration_since(Instant::now())
            .ok_or(ModsPluginBootstrapError::InitializationInProgress)?;
        match invoke_initialize(process.raw(), remote_export, remaining)? {
            InitializeReturnCode::Started => return Ok(ModsPluginInitializeOutcome::Started),
            InitializeReturnCode::AlreadyRunning => {
                return Ok(ModsPluginInitializeOutcome::AlreadyRunning);
            }
            InitializeReturnCode::NotGameHost => {
                return Err(ModsPluginBootstrapError::NotGameHost);
            }
            InitializeReturnCode::Failed => {
                return Err(ModsPluginBootstrapError::InitializationFailed);
            }
            InitializeReturnCode::InProgress => {
                let remaining = deadline
                    .checked_duration_since(Instant::now())
                    .ok_or(ModsPluginBootstrapError::InitializationInProgress)?;
                thread::sleep(IN_PROGRESS_RETRY_DELAY.min(remaining));
            }
        }
    }
    Err(ModsPluginBootstrapError::InitializationInProgress)
}

fn validate_request(
    request: &ModsPluginBootstrapRequest<'_>,
) -> Result<(), ModsPluginBootstrapError> {
    if request.process_id == 0 {
        return Err(ModsPluginBootstrapError::InvalidProcessId);
    }
    if request.timeout < MIN_INITIALIZE_TIMEOUT || request.timeout > MAX_INITIALIZE_TIMEOUT {
        return Err(ModsPluginBootstrapError::InvalidTimeout);
    }
    Ok(())
}

fn canonical_file(
    path: &Path,
    error: fn(io::ErrorKind) -> ModsPluginBootstrapError,
) -> Result<PathBuf, ModsPluginBootstrapError> {
    let metadata = fs::metadata(path).map_err(|value| error(value.kind()))?;
    if !metadata.is_file() {
        return Err(error(io::ErrorKind::InvalidInput));
    }
    fs::canonicalize(path).map_err(|value| error(value.kind()))
}

fn validate_deployment_layout(
    game_executable: &Path,
    deployed_plugin: &Path,
) -> Result<(), ModsPluginBootstrapError> {
    if !file_name_is(game_executable, GAME_EXECUTABLE_NAME)
        || !file_name_is(deployed_plugin, PLUGIN_FILE_NAME)
        || game_executable.parent() != deployed_plugin.parent()
    {
        return Err(ModsPluginBootstrapError::PluginPathMismatch);
    }
    Ok(())
}

fn file_name_is(path: &Path, expected: &str) -> bool {
    path.file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| name.eq_ignore_ascii_case(expected))
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct ParsedPluginImage {
    size_of_image: u32,
    size_of_headers: u32,
    export_directory_rva: u32,
    export_directory_size: u32,
    initialize_export_rva: u32,
    header_identity: Box<[u8]>,
    sections: Vec<PeSection>,
}

fn read_plugin_image(path: &Path) -> Result<ParsedPluginImage, ModsPluginBootstrapError> {
    let file = File::open(path)
        .map_err(|error| ModsPluginBootstrapError::PluginUnavailable(error.kind()))?;
    let metadata = file
        .metadata()
        .map_err(|error| ModsPluginBootstrapError::PluginUnavailable(error.kind()))?;
    if metadata.len() > MAX_PLUGIN_IMAGE_BYTES {
        return Err(ModsPluginBootstrapError::PluginTooLarge {
            bytes: metadata.len(),
            limit: MAX_PLUGIN_IMAGE_BYTES,
        });
    }
    let mut bytes = Vec::with_capacity((metadata.len().min(MAX_PLUGIN_IMAGE_BYTES)) as usize);
    file.take(MAX_PLUGIN_IMAGE_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|error| ModsPluginBootstrapError::PluginReadFailed(error.kind()))?;
    if bytes.len() as u64 > MAX_PLUGIN_IMAGE_BYTES {
        return Err(ModsPluginBootstrapError::PluginTooLarge {
            bytes: bytes.len() as u64,
            limit: MAX_PLUGIN_IMAGE_BYTES,
        });
    }
    parse_plugin_image(&bytes).map_err(ModsPluginBootstrapError::InvalidPluginImage)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct PeSection {
    virtual_address: u32,
    virtual_size: u32,
    raw_offset: u32,
    raw_size: u32,
    characteristics: u32,
}

fn parse_plugin_image(bytes: &[u8]) -> Result<ParsedPluginImage, ModsPluginImageError> {
    if bytes.len() < 0x40 {
        return Err(ModsPluginImageError::Truncated);
    }
    if bytes.get(..2) != Some(b"MZ") {
        return Err(ModsPluginImageError::InvalidDosHeader);
    }
    let pe_offset = read_u32(bytes, 0x3c)? as usize;
    let coff = pe_offset
        .checked_add(4)
        .ok_or(ModsPluginImageError::InvalidPeHeader)?;
    if bytes.get(pe_offset..coff) != Some(b"PE\0\0") {
        return Err(ModsPluginImageError::InvalidPeHeader);
    }
    if read_u16(bytes, coff)? != IMAGE_FILE_MACHINE_AMD64 {
        return Err(ModsPluginImageError::UnsupportedMachine);
    }
    let section_count = read_u16(bytes, coff + 2)? as usize;
    if section_count == 0 || section_count > MAX_SECTIONS {
        return Err(ModsPluginImageError::InvalidSectionTable);
    }
    let optional_size = read_u16(bytes, coff + 16)? as usize;
    let optional = coff
        .checked_add(20)
        .ok_or(ModsPluginImageError::InvalidPeHeader)?;
    let optional_end = optional
        .checked_add(optional_size)
        .ok_or(ModsPluginImageError::UnsupportedOptionalHeader)?;
    if optional_size < 120 || optional_end > bytes.len() {
        return Err(ModsPluginImageError::UnsupportedOptionalHeader);
    }
    if read_u16(bytes, optional)? != IMAGE_NT_OPTIONAL_HDR64_MAGIC
        || read_u32(bytes, optional + 108)? == 0
    {
        return Err(ModsPluginImageError::UnsupportedOptionalHeader);
    }
    let size_of_image = read_u32(bytes, optional + 56)?;
    let size_of_headers = read_u32(bytes, optional + 60)?;
    if size_of_image == 0 || size_of_headers == 0 || size_of_headers as usize > bytes.len() {
        return Err(ModsPluginImageError::InvalidImageSize);
    }
    if size_of_headers as usize > MAX_PE_HEADER_BYTES {
        return Err(ModsPluginImageError::InvalidImageSize);
    }
    let export_rva = read_u32(bytes, optional + 112)?;
    let export_size = read_u32(bytes, optional + 116)?;
    let export_end = export_rva
        .checked_add(export_size)
        .filter(|end| *end <= size_of_image)
        .ok_or(ModsPluginImageError::InvalidExportDirectory)?;
    if export_rva == 0 || export_size < 40 {
        return Err(ModsPluginImageError::InvalidExportDirectory);
    }

    let section_table_size = section_count
        .checked_mul(40)
        .ok_or(ModsPluginImageError::InvalidSectionTable)?;
    let section_table_end = optional_end
        .checked_add(section_table_size)
        .ok_or(ModsPluginImageError::InvalidSectionTable)?;
    if section_table_end > bytes.len() || section_table_end > size_of_headers as usize {
        return Err(ModsPluginImageError::InvalidSectionTable);
    }
    let mut sections = Vec::with_capacity(section_count);
    for index in 0..section_count {
        let offset = optional_end + index * 40;
        sections.push(PeSection {
            virtual_size: read_u32(bytes, offset + 8)?,
            virtual_address: read_u32(bytes, offset + 12)?,
            raw_size: read_u32(bytes, offset + 16)?,
            raw_offset: read_u32(bytes, offset + 20)?,
            characteristics: read_u32(bytes, offset + 36)?,
        });
    }
    let export_offset = rva_to_offset(export_rva, 40, size_of_headers, &sections, bytes.len())?;
    let function_count = read_u32(bytes, export_offset + 20)? as usize;
    let name_count = read_u32(bytes, export_offset + 24)? as usize;
    if function_count == 0
        || function_count > MAX_EXPORTS
        || name_count == 0
        || name_count > MAX_EXPORTS
        || name_count > function_count
    {
        return Err(ModsPluginImageError::ExportBudgetExceeded);
    }
    let functions_rva = read_u32(bytes, export_offset + 28)?;
    let names_rva = read_u32(bytes, export_offset + 32)?;
    let ordinals_rva = read_u32(bytes, export_offset + 36)?;
    let functions_offset = rva_to_offset(
        functions_rva,
        function_count * size_of::<u32>(),
        size_of_headers,
        &sections,
        bytes.len(),
    )?;
    let names_offset = rva_to_offset(
        names_rva,
        name_count * size_of::<u32>(),
        size_of_headers,
        &sections,
        bytes.len(),
    )?;
    let ordinals_offset = rva_to_offset(
        ordinals_rva,
        name_count * size_of::<u16>(),
        size_of_headers,
        &sections,
        bytes.len(),
    )?;

    let mut found = None;
    for index in 0..name_count {
        let name_rva = read_u32(bytes, names_offset + index * 4)?;
        let name_offset = rva_to_offset(name_rva, 1, size_of_headers, &sections, bytes.len())?;
        let name = read_bounded_c_string(bytes, name_offset)?;
        if name != MODS_PLUGIN_INITIALIZE_EXPORT.as_bytes() {
            continue;
        }
        if found.is_some() {
            return Err(ModsPluginImageError::InitializeExportAmbiguous);
        }
        let ordinal = read_u16(bytes, ordinals_offset + index * 2)? as usize;
        if ordinal >= function_count {
            return Err(ModsPluginImageError::InvalidExportDirectory);
        }
        let function_rva = read_u32(bytes, functions_offset + ordinal * 4)?;
        if function_rva >= export_rva && function_rva < export_end {
            return Err(ModsPluginImageError::InitializeExportForwarded);
        }
        if function_rva == 0 || function_rva >= size_of_image {
            return Err(ModsPluginImageError::InitializeExportNotExecutable);
        }
        let executable = sections.iter().any(|section| {
            let span = section.virtual_size.max(section.raw_size);
            section.characteristics & IMAGE_SCN_MEM_EXECUTE != 0
                && function_rva >= section.virtual_address
                && function_rva < section.virtual_address.saturating_add(span)
        });
        if !executable {
            return Err(ModsPluginImageError::InitializeExportNotExecutable);
        }
        found = Some(function_rva);
    }
    let initialize_export_rva = found.ok_or(ModsPluginImageError::InitializeExportMissing)?;
    Ok(ParsedPluginImage {
        size_of_image,
        size_of_headers,
        export_directory_rva: export_rva,
        export_directory_size: export_size,
        initialize_export_rva,
        header_identity: normalized_header_identity(&bytes[..size_of_headers as usize])?,
        sections,
    })
}

fn read_u16(bytes: &[u8], offset: usize) -> Result<u16, ModsPluginImageError> {
    let value = bytes
        .get(offset..offset.saturating_add(2))
        .ok_or(ModsPluginImageError::Truncated)?;
    Ok(u16::from_le_bytes([value[0], value[1]]))
}

fn read_u32(bytes: &[u8], offset: usize) -> Result<u32, ModsPluginImageError> {
    let value = bytes
        .get(offset..offset.saturating_add(4))
        .ok_or(ModsPluginImageError::Truncated)?;
    Ok(u32::from_le_bytes([value[0], value[1], value[2], value[3]]))
}

fn normalized_header_identity(bytes: &[u8]) -> Result<Box<[u8]>, ModsPluginImageError> {
    let pe_offset = read_u32(bytes, 0x3c)? as usize;
    let optional = pe_offset
        .checked_add(24)
        .ok_or(ModsPluginImageError::InvalidPeHeader)?;
    let image_base = optional
        .checked_add(24)
        .ok_or(ModsPluginImageError::InvalidPeHeader)?;
    let image_base_end = image_base
        .checked_add(8)
        .ok_or(ModsPluginImageError::InvalidPeHeader)?;
    if image_base_end > bytes.len() {
        return Err(ModsPluginImageError::Truncated);
    }
    let mut identity = bytes.to_vec();
    identity[image_base..image_base_end].fill(0);
    Ok(identity.into_boxed_slice())
}

fn rva_to_offset(
    rva: u32,
    length: usize,
    size_of_headers: u32,
    sections: &[PeSection],
    file_size: usize,
) -> Result<usize, ModsPluginImageError> {
    let offset = if rva < size_of_headers {
        rva as usize
    } else {
        let section = sections
            .iter()
            .find(|section| {
                rva >= section.virtual_address
                    && rva < section.virtual_address.saturating_add(section.raw_size)
            })
            .ok_or(ModsPluginImageError::InvalidExportDirectory)?;
        let delta = rva - section.virtual_address;
        section
            .raw_offset
            .checked_add(delta)
            .ok_or(ModsPluginImageError::InvalidExportDirectory)? as usize
    };
    offset
        .checked_add(length)
        .filter(|end| *end <= file_size)
        .map(|_| offset)
        .ok_or(ModsPluginImageError::InvalidExportDirectory)
}

fn read_bounded_c_string(bytes: &[u8], offset: usize) -> Result<&[u8], ModsPluginImageError> {
    let available = bytes
        .get(offset..)
        .ok_or(ModsPluginImageError::ExportNameInvalid)?;
    let bound = available.len().min(MAX_EXPORT_NAME_BYTES + 1);
    let terminator = available[..bound]
        .iter()
        .position(|value| *value == 0)
        .ok_or(ModsPluginImageError::ExportNameInvalid)?;
    if terminator == 0 || terminator > MAX_EXPORT_NAME_BYTES {
        return Err(ModsPluginImageError::ExportNameInvalid);
    }
    Ok(&available[..terminator])
}

struct OwnedHandle(HANDLE);

impl OwnedHandle {
    fn new(handle: HANDLE) -> Self {
        Self(handle)
    }

    fn raw(&self) -> HANDLE {
        self.0
    }
}

impl Drop for OwnedHandle {
    fn drop(&mut self) {
        if !self.0.is_null() && self.0 != INVALID_HANDLE_VALUE {
            // SAFETY: this wrapper exclusively owns one checked Win32 handle.
            unsafe {
                CloseHandle(self.0);
            }
        }
    }
}

fn validate_process_image(
    process: HANDLE,
    expected: &Path,
) -> Result<(), ModsPluginBootstrapError> {
    let actual = query_process_image(process)?;
    if !paths_equal(&actual, expected) || !file_name_is(&actual, GAME_EXECUTABLE_NAME) {
        return Err(ModsPluginBootstrapError::TargetExecutableMismatch);
    }
    Ok(())
}

fn query_process_image(process: HANDLE) -> Result<PathBuf, ModsPluginBootstrapError> {
    let mut image = vec![0_u16; MAX_PROCESS_IMAGE_UTF16];
    let mut length = image.len() as u32;
    // SAFETY: process carries query rights and the UTF-16 buffer/length pair is
    // valid for the duration of this call.
    if unsafe { QueryFullProcessImageNameW(process, 0, image.as_mut_ptr(), &mut length) } == 0 {
        return Err(ModsPluginBootstrapError::ProcessImageQueryFailed {
            // SAFETY: this immediately follows the failed Win32 call.
            win32: unsafe { GetLastError() },
        });
    }
    if length == 0 || length as usize > image.len() {
        return Err(ModsPluginBootstrapError::ProcessImageInvalid);
    }
    let image = String::from_utf16(&image[..length as usize])
        .map_err(|_| ModsPluginBootstrapError::ProcessImageInvalid)?;
    let actual = fs::canonicalize(Path::new(&image))
        .map_err(|_| ModsPluginBootstrapError::ProcessImageInvalid)?;
    Ok(actual)
}

fn query_process_creation_time(process: HANDLE) -> Result<u64, ModsPluginBootstrapError> {
    let mut creation = FILETIME {
        dwLowDateTime: 0,
        dwHighDateTime: 0,
    };
    let mut exit = creation;
    let mut kernel = creation;
    let mut user = creation;
    // SAFETY: `process` carries query rights and all four FILETIME outputs are
    // valid, uniquely borrowed stack values for the duration of this call.
    if unsafe { GetProcessTimes(process, &mut creation, &mut exit, &mut kernel, &mut user) } == 0 {
        return Err(ModsPluginBootstrapError::ProcessTimesQueryFailed {
            // SAFETY: this immediately follows the failed Win32 call.
            win32: unsafe { GetLastError() },
        });
    }
    Ok((u64::from(creation.dwHighDateTime) << 32) | u64::from(creation.dwLowDateTime))
}

#[derive(Clone, Copy)]
struct RemoteModule {
    base: usize,
    image_size: u32,
}

fn validate_remote_plugin_image(
    process: HANDLE,
    remote: RemoteModule,
    local: &ParsedPluginImage,
) -> Result<u32, ModsPluginBootstrapError> {
    let headers = read_remote_bytes(process, remote.base, local.size_of_headers as usize)?;
    let remote_identity = normalized_header_identity(&headers)
        .map_err(|_| ModsPluginBootstrapError::RemoteImageIdentityMismatch)?;
    validate_header_identity(local.header_identity.as_ref(), &remote_identity)?;

    let export = read_remote_rva(process, remote, local.export_directory_rva, 40)?;
    let function_count = remote_u32(&export, 20)? as usize;
    let name_count = remote_u32(&export, 24)? as usize;
    if function_count == 0
        || function_count > MAX_EXPORTS
        || name_count == 0
        || name_count > MAX_EXPORTS
        || name_count > function_count
    {
        return Err(ModsPluginBootstrapError::RemoteImageIdentityMismatch);
    }
    let functions_rva = remote_u32(&export, 28)?;
    let names_rva = remote_u32(&export, 32)?;
    let ordinals_rva = remote_u32(&export, 36)?;
    let functions = read_remote_rva(process, remote, functions_rva, function_count * 4)?;
    let names = read_remote_rva(process, remote, names_rva, name_count * 4)?;
    let ordinals = read_remote_rva(process, remote, ordinals_rva, name_count * 2)?;

    let export_end = local
        .export_directory_rva
        .checked_add(local.export_directory_size)
        .ok_or(ModsPluginBootstrapError::RemoteImageIdentityMismatch)?;
    let mut found = None;
    for index in 0..name_count {
        let name_rva = remote_u32(&names, index * 4)?;
        let available = remote
            .image_size
            .checked_sub(name_rva)
            .ok_or(ModsPluginBootstrapError::RemoteImageIdentityMismatch)?
            as usize;
        let name = read_remote_rva(
            process,
            remote,
            name_rva,
            available.min(MAX_EXPORT_NAME_BYTES + 1),
        )?;
        let terminator = name
            .iter()
            .position(|byte| *byte == 0)
            .ok_or(ModsPluginBootstrapError::RemoteImageIdentityMismatch)?;
        if &name[..terminator] != MODS_PLUGIN_INITIALIZE_EXPORT.as_bytes() {
            continue;
        }
        if found.is_some() {
            return Err(ModsPluginBootstrapError::RemoteImageIdentityMismatch);
        }
        let ordinal_offset = index
            .checked_mul(2)
            .ok_or(ModsPluginBootstrapError::RemoteImageIdentityMismatch)?;
        let ordinal = remote_u16(&ordinals, ordinal_offset)? as usize;
        if ordinal >= function_count {
            return Err(ModsPluginBootstrapError::RemoteImageIdentityMismatch);
        }
        let function_rva = remote_u32(&functions, ordinal * 4)?;
        if function_rva >= local.export_directory_rva && function_rva < export_end {
            return Err(ModsPluginBootstrapError::RemoteImageIdentityMismatch);
        }
        let executable = local.sections.iter().any(|section| {
            let span = section.virtual_size.max(section.raw_size);
            section.characteristics & IMAGE_SCN_MEM_EXECUTE != 0
                && function_rva >= section.virtual_address
                && function_rva < section.virtual_address.saturating_add(span)
        });
        if !executable || function_rva >= remote.image_size {
            return Err(ModsPluginBootstrapError::RemoteImageIdentityMismatch);
        }
        found = Some(function_rva);
    }
    let remote_rva = found.ok_or(ModsPluginBootstrapError::RemoteImageIdentityMismatch)?;
    if remote_rva != local.initialize_export_rva {
        return Err(ModsPluginBootstrapError::RemoteImageIdentityMismatch);
    }
    Ok(remote_rva)
}

fn validate_header_identity(local: &[u8], remote: &[u8]) -> Result<(), ModsPluginBootstrapError> {
    if local == remote {
        Ok(())
    } else {
        Err(ModsPluginBootstrapError::RemoteImageIdentityMismatch)
    }
}

fn read_remote_rva(
    process: HANDLE,
    remote: RemoteModule,
    rva: u32,
    length: usize,
) -> Result<Vec<u8>, ModsPluginBootstrapError> {
    (rva as usize)
        .checked_add(length)
        .filter(|end| *end <= remote.image_size as usize)
        .ok_or(ModsPluginBootstrapError::RemoteImageIdentityMismatch)?;
    let address = remote
        .base
        .checked_add(rva as usize)
        .ok_or(ModsPluginBootstrapError::RemoteImageIdentityMismatch)?;
    read_remote_bytes(process, address, length)
}

fn read_remote_bytes(
    process: HANDLE,
    address: usize,
    length: usize,
) -> Result<Vec<u8>, ModsPluginBootstrapError> {
    if length == 0 || length > MAX_PLUGIN_IMAGE_BYTES as usize {
        return Err(ModsPluginBootstrapError::RemoteImageIdentityMismatch);
    }
    let mut bytes = vec![0_u8; length];
    let mut read = 0_usize;
    // SAFETY: process has PROCESS_VM_READ, the target range was checked against
    // the bounded Toolhelp image, and the local output buffer is fully writable.
    if unsafe {
        ReadProcessMemory(
            process,
            address as *const c_void,
            bytes.as_mut_ptr().cast(),
            length,
            &mut read,
        )
    } == 0
    {
        return Err(ModsPluginBootstrapError::RemoteImageReadFailed {
            // SAFETY: this immediately follows the failed Win32 call.
            win32: unsafe { GetLastError() },
        });
    }
    if read != length {
        return Err(ModsPluginBootstrapError::RemoteImageIdentityMismatch);
    }
    Ok(bytes)
}

fn remote_u16(bytes: &[u8], offset: usize) -> Result<u16, ModsPluginBootstrapError> {
    read_u16(bytes, offset).map_err(|_| ModsPluginBootstrapError::RemoteImageIdentityMismatch)
}

fn remote_u32(bytes: &[u8], offset: usize) -> Result<u32, ModsPluginBootstrapError> {
    read_u32(bytes, offset).map_err(|_| ModsPluginBootstrapError::RemoteImageIdentityMismatch)
}

fn find_running_game_process() -> Result<Option<u32>, ModsPluginBootstrapError> {
    // SAFETY: this requests a read-only system process snapshot. A successful
    // result is owned and closed by `OwnedHandle`.
    let snapshot = unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) };
    if snapshot == INVALID_HANDLE_VALUE {
        return Err(ModsPluginBootstrapError::ProcessSnapshotFailed {
            // SAFETY: this immediately follows the failed Win32 call.
            win32: unsafe { GetLastError() },
        });
    }
    let snapshot = OwnedHandle::new(snapshot);
    let mut entry = PROCESSENTRY32W {
        dwSize: size_of::<PROCESSENTRY32W>() as u32,
        ..Default::default()
    };
    // SAFETY: entry has the documented size and remains writable while the
    // owned snapshot is live.
    if unsafe { Process32FirstW(snapshot.raw(), &mut entry) } == 0 {
        // SAFETY: this immediately follows the failed Win32 call.
        let win32 = unsafe { GetLastError() };
        if win32 == ERROR_NO_MORE_FILES {
            return Ok(None);
        }
        return Err(ModsPluginBootstrapError::ProcessEnumerationFailed { win32 });
    }

    let mut match_id = None;
    for index in 0..MAX_MODULES {
        let name = utf16_array(&entry.szExeFile)
            .map_err(|_| ModsPluginBootstrapError::ProcessImageInvalid)?;
        if name.eq_ignore_ascii_case(GAME_EXECUTABLE_NAME)
            && match_id.replace(entry.th32ProcessID).is_some()
        {
            return Err(ModsPluginBootstrapError::TargetProcessAmbiguous);
        }
        if index + 1 == MAX_MODULES {
            // SAFETY: snapshot and entry remain valid as above.
            if unsafe { Process32NextW(snapshot.raw(), &mut entry) } != 0 {
                return Err(ModsPluginBootstrapError::ProcessBudgetExceeded);
            }
            // SAFETY: this immediately follows the failed enumeration call.
            let win32 = unsafe { GetLastError() };
            if win32 != ERROR_NO_MORE_FILES {
                return Err(ModsPluginBootstrapError::ProcessEnumerationFailed { win32 });
            }
            break;
        }
        // SAFETY: snapshot and entry remain valid as above.
        if unsafe { Process32NextW(snapshot.raw(), &mut entry) } == 0 {
            // SAFETY: this immediately follows the failed enumeration call.
            let win32 = unsafe { GetLastError() };
            if win32 != ERROR_NO_MORE_FILES {
                return Err(ModsPluginBootstrapError::ProcessEnumerationFailed { win32 });
            }
            break;
        }
    }
    Ok(match_id)
}

fn find_deployed_module(
    process_id: u32,
    expected_plugin: &Path,
) -> Result<RemoteModule, ModsPluginBootstrapError> {
    let snapshot = create_module_snapshot(process_id)?;
    let mut entry = MODULEENTRY32W {
        dwSize: size_of::<MODULEENTRY32W>() as u32,
        ..Default::default()
    };
    // SAFETY: entry has the documented size and remains writable while the
    // owned snapshot is live.
    if unsafe { Module32FirstW(snapshot.raw(), &mut entry) } == 0 {
        // SAFETY: this immediately follows the failed Win32 call.
        let win32 = unsafe { GetLastError() };
        if win32 == ERROR_NO_MORE_FILES {
            return Err(ModsPluginBootstrapError::DeployedModuleNotLoaded);
        }
        return Err(ModsPluginBootstrapError::ModuleEnumerationFailed { win32 });
    }

    let mut matched = None;
    let mut saw_other_dwmapi = false;
    for index in 0..MAX_MODULES {
        let module_name = utf16_array(&entry.szModule)?;
        if module_name.eq_ignore_ascii_case(PLUGIN_FILE_NAME) {
            let module_path = utf16_array(&entry.szExePath)?;
            let canonical = fs::canonicalize(Path::new(&module_path));
            if canonical
                .as_ref()
                .is_ok_and(|path| paths_equal(path, expected_plugin))
            {
                if matched.is_some() {
                    return Err(ModsPluginBootstrapError::DeployedModuleAmbiguous);
                }
                if entry.modBaseAddr.is_null() || entry.modBaseSize == 0 {
                    return Err(ModsPluginBootstrapError::ModulePathInvalid);
                }
                matched = Some(RemoteModule {
                    base: entry.modBaseAddr as usize,
                    image_size: entry.modBaseSize,
                });
            } else {
                saw_other_dwmapi = true;
            }
        }
        // The last permitted entry was consumed; never continue an unbounded
        // snapshot walk even if a hostile/inconsistent snapshot says more.
        if index + 1 == MAX_MODULES {
            // SAFETY: snapshot and entry remain valid as above.
            if unsafe { Module32NextW(snapshot.raw(), &mut entry) } != 0 {
                return Err(ModsPluginBootstrapError::ModuleBudgetExceeded);
            }
            // SAFETY: this immediately follows the failed enumeration call.
            let win32 = unsafe { GetLastError() };
            if win32 != ERROR_NO_MORE_FILES {
                return Err(ModsPluginBootstrapError::ModuleEnumerationFailed { win32 });
            }
            break;
        }
        // SAFETY: snapshot and entry remain valid as above.
        if unsafe { Module32NextW(snapshot.raw(), &mut entry) } == 0 {
            // SAFETY: this immediately follows the failed enumeration call.
            let win32 = unsafe { GetLastError() };
            if win32 != ERROR_NO_MORE_FILES {
                return Err(ModsPluginBootstrapError::ModuleEnumerationFailed { win32 });
            }
            break;
        }
    }
    matched.ok_or(if saw_other_dwmapi {
        ModsPluginBootstrapError::DeployedModulePathMismatch
    } else {
        ModsPluginBootstrapError::DeployedModuleNotLoaded
    })
}

fn create_module_snapshot(process_id: u32) -> Result<OwnedHandle, ModsPluginBootstrapError> {
    let mut last_error = 0;
    for _ in 0..MAX_SNAPSHOT_ATTEMPTS {
        // SAFETY: flags request a read-only module snapshot for the validated
        // PID. A successful result is exclusively owned by the wrapper.
        let snapshot = unsafe {
            CreateToolhelp32Snapshot(TH32CS_SNAPMODULE | TH32CS_SNAPMODULE32, process_id)
        };
        if snapshot != INVALID_HANDLE_VALUE {
            return Ok(OwnedHandle::new(snapshot));
        }
        // SAFETY: this immediately follows the failed Win32 call.
        last_error = unsafe { GetLastError() };
        if last_error != ERROR_BAD_LENGTH {
            break;
        }
        thread::yield_now();
    }
    Err(ModsPluginBootstrapError::ModuleSnapshotFailed { win32: last_error })
}

fn utf16_array<const N: usize>(value: &[u16; N]) -> Result<String, ModsPluginBootstrapError> {
    let length = value.iter().position(|unit| *unit == 0).unwrap_or(N);
    if length == N {
        return Err(ModsPluginBootstrapError::ModulePathInvalid);
    }
    String::from_utf16(&value[..length]).map_err(|_| ModsPluginBootstrapError::ModulePathInvalid)
}

fn paths_equal(left: &Path, right: &Path) -> bool {
    normalize_path(left) == normalize_path(right)
}

fn normalize_path(path: &Path) -> String {
    let value = path.to_string_lossy().replace('/', "\\");
    let value = value
        .strip_prefix(r"\\?\UNC\")
        .map(|rest| format!(r"\\{rest}"))
        .or_else(|| value.strip_prefix(r"\\?\").map(ToOwned::to_owned))
        .unwrap_or(value);
    value.trim_end_matches('\\').to_lowercase()
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum InitializeReturnCode {
    Started,
    AlreadyRunning,
    NotGameHost,
    InProgress,
    Failed,
}

fn classify_initialize_return(
    status: u32,
) -> Result<InitializeReturnCode, ModsPluginBootstrapError> {
    match status {
        0 => Ok(InitializeReturnCode::Started),
        1 => Ok(InitializeReturnCode::AlreadyRunning),
        2 => Ok(InitializeReturnCode::NotGameHost),
        3 => Ok(InitializeReturnCode::InProgress),
        4 => Ok(InitializeReturnCode::Failed),
        status => Err(ModsPluginBootstrapError::UnexpectedInitializeStatus { status }),
    }
}

fn invoke_initialize(
    process: HANDLE,
    remote_export: usize,
    timeout: Duration,
) -> Result<InitializeReturnCode, ModsPluginBootstrapError> {
    // SAFETY: PE validation proves an AMD64 executable, non-forwarded export at
    // this address; the deployed native ABI is `DWORD WINAPI(void*)` and null is
    // its required reserved argument.
    let entry: LPTHREAD_START_ROUTINE = Some(unsafe {
        mem::transmute::<usize, unsafe extern "system" fn(*mut c_void) -> u32>(remote_export)
    });
    // SAFETY: process has the documented CreateRemoteThread rights, entry is the
    // verified loaded export, all optional pointers are null, and the returned
    // thread handle is checked before ownership is assumed.
    let remote_thread = unsafe {
        CreateRemoteThread(
            process,
            ptr::null(),
            0,
            entry,
            ptr::null(),
            0,
            ptr::null_mut(),
        )
    };
    if remote_thread.is_null() {
        return Err(ModsPluginBootstrapError::RemoteThreadCreateFailed {
            // SAFETY: this immediately follows the failed Win32 call.
            win32: unsafe { GetLastError() },
        });
    }
    let remote_thread = OwnedHandle::new(remote_thread);
    let wait_ms = timeout.as_millis().clamp(1, u32::MAX as u128) as u32;
    // SAFETY: remote_thread is a live, synchronizable thread handle and timeout
    // is explicitly bounded by request validation/deadline accounting.
    let wait = unsafe { WaitForSingleObject(remote_thread.raw(), wait_ms) };
    match wait {
        WAIT_OBJECT_0 => {}
        WAIT_TIMEOUT => return Err(ModsPluginBootstrapError::RemoteThreadTimedOut),
        WAIT_FAILED => {
            return Err(ModsPluginBootstrapError::RemoteThreadWaitFailed {
                // SAFETY: this immediately follows the failed Win32 call.
                win32: unsafe { GetLastError() },
            });
        }
        status => return Err(ModsPluginBootstrapError::UnexpectedWaitStatus { status }),
    }
    let mut exit_code = STILL_ACTIVE as u32;
    // SAFETY: the signaled thread handle remains live and exit_code is writable.
    if unsafe { GetExitCodeThread(remote_thread.raw(), &mut exit_code) } == 0 {
        return Err(ModsPluginBootstrapError::RemoteThreadExitQueryFailed {
            // SAFETY: this immediately follows the failed Win32 call.
            win32: unsafe { GetLastError() },
        });
    }
    if exit_code == STILL_ACTIVE as u32 {
        return Err(ModsPluginBootstrapError::RemoteThreadStillActive);
    }
    classify_initialize_return(exit_code)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::{Command, Stdio};
    use std::time::SystemTime;

    fn put_u16(bytes: &mut [u8], offset: usize, value: u16) {
        bytes[offset..offset + 2].copy_from_slice(&value.to_le_bytes());
    }

    fn put_u32(bytes: &mut [u8], offset: usize, value: u32) {
        bytes[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
    }

    fn plugin_fixture() -> Vec<u8> {
        let mut bytes = vec![0_u8; 0x800];
        bytes[..2].copy_from_slice(b"MZ");
        put_u32(&mut bytes, 0x3c, 0x80);
        bytes[0x80..0x84].copy_from_slice(b"PE\0\0");
        let coff = 0x84;
        put_u16(&mut bytes, coff, IMAGE_FILE_MACHINE_AMD64);
        put_u16(&mut bytes, coff + 2, 1);
        put_u16(&mut bytes, coff + 16, 0xf0);
        let optional = coff + 20;
        put_u16(&mut bytes, optional, IMAGE_NT_OPTIONAL_HDR64_MAGIC);
        put_u32(&mut bytes, optional + 56, 0x2000);
        put_u32(&mut bytes, optional + 60, 0x200);
        put_u32(&mut bytes, optional + 108, 16);
        put_u32(&mut bytes, optional + 112, 0x1100);
        put_u32(&mut bytes, optional + 116, 0x100);
        let section = optional + 0xf0;
        bytes[section..section + 5].copy_from_slice(b".text");
        put_u32(&mut bytes, section + 8, 0x600);
        put_u32(&mut bytes, section + 12, 0x1000);
        put_u32(&mut bytes, section + 16, 0x600);
        put_u32(&mut bytes, section + 20, 0x200);
        put_u32(
            &mut bytes,
            section + 36,
            IMAGE_SCN_MEM_EXECUTE | 0x4000_0000,
        );
        let export = 0x300;
        put_u32(&mut bytes, export + 20, 1);
        put_u32(&mut bytes, export + 24, 1);
        put_u32(&mut bytes, export + 28, 0x1140);
        put_u32(&mut bytes, export + 32, 0x1144);
        put_u32(&mut bytes, export + 36, 0x1148);
        put_u32(&mut bytes, 0x340, 0x1200);
        put_u32(&mut bytes, 0x344, 0x1160);
        put_u16(&mut bytes, 0x348, 0);
        let name = MODS_PLUGIN_INITIALIZE_EXPORT.as_bytes();
        bytes[0x360..0x360 + name.len()].copy_from_slice(name);
        bytes[0x360 + name.len()] = 0;
        bytes
    }

    #[test]
    fn parses_bounded_non_forwarded_amd64_initialize_export() {
        let image = parse_plugin_image(&plugin_fixture()).expect("fixture should parse");
        assert_eq!(image.size_of_image, 0x2000);
        assert_eq!(image.initialize_export_rva, 0x1200);
    }

    #[test]
    fn rejects_forwarded_initialize_export() {
        let mut fixture = plugin_fixture();
        put_u32(&mut fixture, 0x340, 0x1180);
        assert_eq!(
            parse_plugin_image(&fixture),
            Err(ModsPluginImageError::InitializeExportForwarded)
        );
    }

    #[test]
    fn rejects_non_executable_initialize_export() {
        let mut fixture = plugin_fixture();
        put_u32(&mut fixture, 0x84 + 20 + 0xf0 + 36, 0x4000_0000);
        assert_eq!(
            parse_plugin_image(&fixture),
            Err(ModsPluginImageError::InitializeExportNotExecutable)
        );
    }

    #[test]
    fn rejects_unbounded_export_counts() {
        let mut fixture = plugin_fixture();
        put_u32(&mut fixture, 0x300 + 20, MAX_EXPORTS as u32 + 1);
        assert_eq!(
            parse_plugin_image(&fixture),
            Err(ModsPluginImageError::ExportBudgetExceeded)
        );
    }

    #[test]
    fn rejects_same_size_remote_image_with_different_headers() {
        let local = parse_plugin_image(&plugin_fixture()).expect("fixture should parse");
        let mut remote_headers = local.header_identity.to_vec();
        remote_headers[0x88] ^= 0x5a;
        assert_eq!(
            validate_header_identity(local.header_identity.as_ref(), &remote_headers),
            Err(ModsPluginBootstrapError::RemoteImageIdentityMismatch)
        );
    }

    #[test]
    fn maps_every_stable_initialize_return_code() {
        assert_eq!(
            classify_initialize_return(0),
            Ok(InitializeReturnCode::Started)
        );
        assert_eq!(
            classify_initialize_return(1),
            Ok(InitializeReturnCode::AlreadyRunning)
        );
        assert_eq!(
            classify_initialize_return(2),
            Ok(InitializeReturnCode::NotGameHost)
        );
        assert_eq!(
            classify_initialize_return(3),
            Ok(InitializeReturnCode::InProgress)
        );
        assert_eq!(
            classify_initialize_return(4),
            Ok(InitializeReturnCode::Failed)
        );
        assert_eq!(
            classify_initialize_return(5),
            Err(ModsPluginBootstrapError::UnexpectedInitializeStatus { status: 5 })
        );
    }

    #[test]
    fn rejects_zero_pid_and_out_of_budget_timeout() {
        let request = ModsPluginBootstrapRequest {
            process_id: 0,
            game_executable: Path::new("HTGame.exe"),
            deployed_plugin: Path::new("dwmapi.dll"),
            timeout: DEFAULT_MODS_PLUGIN_INITIALIZE_TIMEOUT,
        };
        assert_eq!(
            validate_request(&request),
            Err(ModsPluginBootstrapError::InvalidProcessId)
        );
        let request = ModsPluginBootstrapRequest {
            process_id: 1,
            timeout: MAX_INITIALIZE_TIMEOUT + Duration::from_millis(1),
            ..request
        };
        assert_eq!(
            validate_request(&request),
            Err(ModsPluginBootstrapError::InvalidTimeout)
        );
    }

    #[test]
    fn normalizes_extended_windows_paths_case_insensitively() {
        assert!(paths_equal(
            Path::new(r"\\?\C:\Games\NTE\HTGame.exe"),
            Path::new(r"c:\games\nte\htgame.exe")
        ));
        assert!(paths_equal(
            Path::new(r"\\?\UNC\server\share\dwmapi.dll"),
            Path::new(r"\\SERVER\SHARE\DWMAPI.DLL")
        ));
    }

    #[test]
    fn running_context_tracks_process_generation_and_deployed_image_identity() {
        let context = RunningModsPluginBootstrapContext {
            process_id: 41,
            process_creation_time: 100,
            game_executable: PathBuf::from(r"C:\Games\NTE\HTGame.exe"),
            deployed_plugin: PathBuf::from(r"C:\Games\NTE\dwmapi.dll"),
            deployed_plugin_identity: DeployedPluginIdentity::Present {
                bytes: 4096,
                creation_time: 200,
                last_write_time: 300,
                attributes: 0,
            },
        };
        let mut recycled_process = context.clone();
        recycled_process.process_creation_time += 1;
        assert_ne!(context, recycled_process);

        let mut replaced_plugin = context.clone();
        replaced_plugin.deployed_plugin_identity = DeployedPluginIdentity::Present {
            bytes: 8192,
            creation_time: 201,
            last_write_time: 301,
            attributes: 0,
        };
        assert_ne!(context, replaced_plugin);
    }

    #[test]
    #[ignore = "requires the freshly built native Release proxy"]
    fn remote_bootstrap_invokes_loaded_export() {
        const CHILD_ENV: &str = "NTE_MODS_PLUGIN_BOOTSTRAP_TEST_CHILD";
        let current_executable = std::env::current_exe().expect("test executable path");
        let source_plugin = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("native/nte-mods-plugin/x64/Release/dwmapi.dll");
        assert!(
            source_plugin.is_file(),
            "build the native Release proxy first"
        );

        if std::env::var_os(CHILD_ENV).is_some() {
            let plugin = current_executable
                .parent()
                .expect("child directory")
                .join(PLUGIN_FILE_NAME);
            // SAFETY: this fixture loads the freshly built, verified local proxy
            // and keeps its owner alive for the complete child process lifetime.
            let _plugin = unsafe { libloading::Library::new(plugin) }.expect("load proxy");
            fs::write(
                current_executable
                    .parent()
                    .expect("child directory")
                    .join("ready"),
                b"loaded",
            )
            .expect("publish child readiness");
            thread::sleep(Duration::from_secs(15));
            return;
        }

        let nonce = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .expect("system clock")
            .as_nanos();
        let directory = std::env::temp_dir().join(format!(
            "nte-mods-plugin-bootstrap-{}-{nonce}",
            std::process::id()
        ));
        fs::create_dir(&directory).expect("create fixture directory");
        let host = directory.join(GAME_EXECUTABLE_NAME);
        fs::copy(&current_executable, &host).expect("copy fixture host");
        fs::copy(&source_plugin, directory.join(PLUGIN_FILE_NAME)).expect("copy proxy");
        let mut child = Command::new(&host)
            .args([
                "--exact",
                "platform::mods_plugin_bootstrap::tests::remote_bootstrap_invokes_loaded_export",
                "--ignored",
                "--nocapture",
            ])
            .env(CHILD_ENV, "1")
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("start fixture host");
        let ready = directory.join("ready");
        let ready_deadline = Instant::now() + Duration::from_secs(5);
        while !ready.is_file() && Instant::now() < ready_deadline {
            thread::sleep(Duration::from_millis(20));
        }
        assert!(ready.is_file(), "fixture host did not load the proxy");

        let first = initialize_running_deployed_mods_plugin();
        let second = initialize_deployed_mods_plugin(child.id(), &directory);
        let _ = child.kill();
        let _ = child.wait();
        let _ = fs::remove_dir_all(&directory);
        assert_eq!(first, Ok(Some(ModsPluginInitializeOutcome::Started)));
        assert_eq!(second, Ok(ModsPluginInitializeOutcome::AlreadyRunning));
    }
}
