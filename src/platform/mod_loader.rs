use std::{
    ffi::{OsStr, c_void},
    os::windows::ffi::OsStrExt,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
};

use windows_sys::Win32::{
    Foundation::{
        CloseHandle, ERROR_CANCELLED, GetLastError, HANDLE, WAIT_FAILED, WAIT_OBJECT_0,
        WAIT_TIMEOUT,
    },
    System::Threading::{CreateEventW, GetCurrentProcessId, SetEvent, WaitForSingleObject},
    UI::{
        Shell::{SEE_MASK_NOCLOSEPROCESS, SHELLEXECUTEINFOW, ShellExecuteExW},
        WindowsAndMessaging::SW_HIDE,
    },
};

pub const MOD_LOADER_FILE_NAME: &str = "nte-mod-loader.exe";
pub const MOD_LOADER_PAYLOAD_RELATIVE_PATH: &str = "plugins/dwmapi.dll";
// The loader exits only after it has terminated every launcher process that
// received the session shim, so the owner wait must cover that cleanup.
const MOD_LOADER_STOP_TIMEOUT_MS: u32 = 15_000;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ModLoaderRuntimePhase {
    MissingLoader,
    MissingPayload,
    Stopped,
    Running,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ModLoaderRuntimeSnapshot {
    pub phase: ModLoaderRuntimePhase,
    pub loader_present: bool,
    pub payload_present: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ModLoaderRuntimeError {
    Busy,
    LayoutUnavailable,
    LoaderMissing,
    PayloadMissing,
    LaunchCancelled,
    LaunchFailed(u32),
    StopFailed(u32),
    StopTimedOut,
    ProbeFailed(u32),
    StatePoisoned,
}

#[derive(Clone)]
pub struct ModLoaderRuntimeService(Arc<ModLoaderRuntimeInner>);

struct ModLoaderRuntimeInner {
    /// Serializes finite start/stop/probe transactions. The runtime state lock is
    /// only held for in-memory transitions and never across Shell/UAC or waits.
    transaction: Mutex<()>,
    state: Mutex<RuntimeState>,
    directory: Option<PathBuf>,
}

enum RuntimeState {
    Idle,
    Starting,
    Running(RuntimeSession),
    Stopping,
}

struct RuntimeSession {
    process: isize,
    stop_event: isize,
}

impl ModLoaderRuntimeService {
    pub fn for_current_executable() -> Self {
        let directory = std::env::current_exe()
            .ok()
            .and_then(|path| path.parent().map(Path::to_path_buf));
        Self::with_directory(directory)
    }

    pub fn for_directory(directory: PathBuf) -> Self {
        Self::with_directory(Some(directory))
    }

    fn with_directory(directory: Option<PathBuf>) -> Self {
        Self(Arc::new(ModLoaderRuntimeInner {
            transaction: Mutex::new(()),
            state: Mutex::new(RuntimeState::Idle),
            directory,
        }))
    }

    pub fn application_directory(&self) -> Result<PathBuf, ModLoaderRuntimeError> {
        self.0
            .directory
            .clone()
            .ok_or(ModLoaderRuntimeError::LayoutUnavailable)
    }

    pub fn snapshot(&self) -> Result<ModLoaderRuntimeSnapshot, ModLoaderRuntimeError> {
        let _transaction = self
            .0
            .transaction
            .lock()
            .map_err(|_| ModLoaderRuntimeError::StatePoisoned)?;
        let running = self.reconcile_running_session()?;
        self.project_snapshot(running)
    }

    /// Mutation effect: only the Mod loader runtime read model changes. Combat,
    /// packet, history, settings and presentation revisions are unaffected.
    pub fn start(&self) -> Result<ModLoaderRuntimeSnapshot, ModLoaderRuntimeError> {
        let _transaction = self
            .0
            .transaction
            .lock()
            .map_err(|_| ModLoaderRuntimeError::StatePoisoned)?;
        if self.reconcile_running_session()? {
            return self.project_snapshot(true);
        }
        let snapshot = self.project_snapshot(false)?;
        if !snapshot.loader_present {
            return Err(ModLoaderRuntimeError::LoaderMissing);
        }
        if !snapshot.payload_present {
            return Err(ModLoaderRuntimeError::PayloadMissing);
        }
        self.replace_state(RuntimeState::Starting)?;

        let launch = self.launch_managed_loader();
        match launch {
            Ok(session) => {
                self.replace_state(RuntimeState::Running(session))?;
                self.project_snapshot(true)
            }
            Err(error) => {
                self.replace_state(RuntimeState::Idle)?;
                Err(error)
            }
        }
    }

    /// Mutation effect: only the Mod loader runtime read model changes. The
    /// process is asked to stop outside the runtime state lock.
    pub fn stop(&self) -> Result<ModLoaderRuntimeSnapshot, ModLoaderRuntimeError> {
        let _transaction = self
            .0
            .transaction
            .lock()
            .map_err(|_| ModLoaderRuntimeError::StatePoisoned)?;
        if !self.reconcile_running_session()? {
            return self.project_snapshot(false);
        }
        let session = {
            let mut state = self
                .0
                .state
                .lock()
                .map_err(|_| ModLoaderRuntimeError::StatePoisoned)?;
            match std::mem::replace(&mut *state, RuntimeState::Stopping) {
                RuntimeState::Running(session) => session,
                other => {
                    *state = other;
                    return Err(ModLoaderRuntimeError::Busy);
                }
            }
        };

        // SAFETY: both handles were returned by successful Win32 calls and are
        // exclusively owned by `session` for the duration of this transaction.
        let signaled = unsafe { SetEvent(as_handle(session.stop_event)) };
        if signaled == 0 {
            let code = unsafe { GetLastError() };
            self.replace_state(RuntimeState::Running(session))?;
            return Err(ModLoaderRuntimeError::StopFailed(code));
        }
        // SAFETY: the process handle remains open and exclusively owned until
        // the wait completes or the session is restored below.
        let wait =
            unsafe { WaitForSingleObject(as_handle(session.process), MOD_LOADER_STOP_TIMEOUT_MS) };
        match wait {
            WAIT_OBJECT_0 => {
                close_session(session);
                self.replace_state(RuntimeState::Idle)?;
                self.project_snapshot(false)
            }
            WAIT_TIMEOUT => {
                self.replace_state(RuntimeState::Running(session))?;
                Err(ModLoaderRuntimeError::StopTimedOut)
            }
            WAIT_FAILED => {
                let code = unsafe { GetLastError() };
                self.replace_state(RuntimeState::Running(session))?;
                Err(ModLoaderRuntimeError::StopFailed(code))
            }
            code => {
                self.replace_state(RuntimeState::Running(session))?;
                Err(ModLoaderRuntimeError::StopFailed(code))
            }
        }
    }

    fn reconcile_running_session(&self) -> Result<bool, ModLoaderRuntimeError> {
        let process = {
            let state = self
                .0
                .state
                .lock()
                .map_err(|_| ModLoaderRuntimeError::StatePoisoned)?;
            match &*state {
                RuntimeState::Idle => return Ok(false),
                RuntimeState::Starting | RuntimeState::Stopping => {
                    return Err(ModLoaderRuntimeError::Busy);
                }
                RuntimeState::Running(session) => session.process,
            }
        };
        // SAFETY: the transaction mutex prevents start/stop from closing this
        // process handle while the non-blocking liveness probe runs.
        let wait = unsafe { WaitForSingleObject(as_handle(process), 0) };
        match wait {
            WAIT_TIMEOUT => Ok(true),
            WAIT_OBJECT_0 => {
                let session = self.take_running_session()?;
                close_session(session);
                Ok(false)
            }
            WAIT_FAILED => Err(ModLoaderRuntimeError::ProbeFailed(unsafe {
                GetLastError()
            })),
            code => Err(ModLoaderRuntimeError::ProbeFailed(code)),
        }
    }

    fn take_running_session(&self) -> Result<RuntimeSession, ModLoaderRuntimeError> {
        let mut state = self
            .0
            .state
            .lock()
            .map_err(|_| ModLoaderRuntimeError::StatePoisoned)?;
        match std::mem::replace(&mut *state, RuntimeState::Idle) {
            RuntimeState::Running(session) => Ok(session),
            other => {
                *state = other;
                Err(ModLoaderRuntimeError::Busy)
            }
        }
    }

    fn replace_state(&self, replacement: RuntimeState) -> Result<(), ModLoaderRuntimeError> {
        *self
            .0
            .state
            .lock()
            .map_err(|_| ModLoaderRuntimeError::StatePoisoned)? = replacement;
        Ok(())
    }

    fn project_snapshot(
        &self,
        running: bool,
    ) -> Result<ModLoaderRuntimeSnapshot, ModLoaderRuntimeError> {
        let directory = self.application_directory()?;
        let loader_present = directory.join(MOD_LOADER_FILE_NAME).is_file();
        let payload_present = directory
            .join(Path::new(MOD_LOADER_PAYLOAD_RELATIVE_PATH))
            .is_file();
        let phase = if running {
            ModLoaderRuntimePhase::Running
        } else if !loader_present {
            ModLoaderRuntimePhase::MissingLoader
        } else if !payload_present {
            ModLoaderRuntimePhase::MissingPayload
        } else {
            ModLoaderRuntimePhase::Stopped
        };
        Ok(ModLoaderRuntimeSnapshot {
            phase,
            loader_present,
            payload_present,
        })
    }

    fn launch_managed_loader(&self) -> Result<RuntimeSession, ModLoaderRuntimeError> {
        let directory = self.application_directory()?;
        let executable = directory.join(MOD_LOADER_FILE_NAME);
        let event_name = format!(
            "Local\\NTE-DPS-TOOL-ModLoader-{:08x}{:08x}",
            unsafe { GetCurrentProcessId() },
            next_session_suffix()
        );
        let event_name_wide = wide(&event_name);
        // SAFETY: a null security descriptor requests the caller's default
        // DACL; the NUL-terminated event name remains alive for this call.
        let stop_event = unsafe { CreateEventW(std::ptr::null(), 1, 0, event_name_wide.as_ptr()) };
        if stop_event.is_null() {
            return Err(ModLoaderRuntimeError::LaunchFailed(unsafe {
                GetLastError()
            }));
        }

        let verb = wide("runas");
        let executable = wide(executable.as_os_str());
        let directory_wide = wide(directory.as_os_str());
        let parameters = wide(format!(
            "--monitor-timeout 0 --stop-event {event_name} --owner-pid {}",
            unsafe { GetCurrentProcessId() }
        ));
        let mut execution = SHELLEXECUTEINFOW {
            cbSize: std::mem::size_of::<SHELLEXECUTEINFOW>() as u32,
            fMask: SEE_MASK_NOCLOSEPROCESS,
            lpVerb: verb.as_ptr(),
            lpFile: executable.as_ptr(),
            lpParameters: parameters.as_ptr(),
            lpDirectory: directory_wide.as_ptr(),
            nShow: SW_HIDE,
            ..SHELLEXECUTEINFOW::default()
        };
        // SAFETY: all pointers in `execution` reference NUL-terminated buffers
        // that outlive the synchronous ShellExecuteExW call.
        if unsafe { ShellExecuteExW(&mut execution) } == 0 || execution.hProcess.is_null() {
            let code = unsafe { GetLastError() };
            unsafe { CloseHandle(stop_event) };
            return Err(if code == ERROR_CANCELLED {
                ModLoaderRuntimeError::LaunchCancelled
            } else {
                ModLoaderRuntimeError::LaunchFailed(code)
            });
        }
        Ok(RuntimeSession {
            process: execution.hProcess as isize,
            stop_event: stop_event as isize,
        })
    }
}

impl Default for ModLoaderRuntimeService {
    fn default() -> Self {
        Self::for_current_executable()
    }
}

impl Drop for ModLoaderRuntimeInner {
    fn drop(&mut self) {
        let Ok(state) = self.state.get_mut() else {
            return;
        };
        if let RuntimeState::Running(session) = std::mem::replace(state, RuntimeState::Idle) {
            // SAFETY: last-owner drop exclusively owns the two session handles.
            unsafe {
                SetEvent(as_handle(session.stop_event));
            }
            close_session(session);
        }
    }
}

fn close_session(session: RuntimeSession) {
    // SAFETY: session handles are non-null, uniquely owned, and closed once.
    unsafe {
        CloseHandle(as_handle(session.process));
        CloseHandle(as_handle(session.stop_event));
    }
}

fn as_handle(value: isize) -> HANDLE {
    value as *mut c_void
}

fn wide(value: impl AsRef<OsStr>) -> Vec<u16> {
    value.as_ref().encode_wide().chain(Some(0)).collect()
}

fn next_session_suffix() -> u32 {
    use std::sync::atomic::{AtomicU32, Ordering};
    static NEXT: AtomicU32 = AtomicU32::new(1);
    NEXT.fetch_add(1, Ordering::Relaxed)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn layout_contract_uses_release_sibling_paths() {
        let root = std::env::temp_dir().join(format!(
            "nte-mod-loader-layout-{}-{}",
            std::process::id(),
            next_session_suffix()
        ));
        std::fs::create_dir_all(root.join("plugins")).expect("create test layout");
        let service = ModLoaderRuntimeService::for_directory(root.clone());

        assert_eq!(
            service.snapshot().expect("missing loader snapshot").phase,
            ModLoaderRuntimePhase::MissingLoader
        );
        std::fs::write(root.join(MOD_LOADER_FILE_NAME), b"fixture").expect("write loader fixture");
        assert_eq!(
            service.snapshot().expect("missing payload snapshot").phase,
            ModLoaderRuntimePhase::MissingPayload
        );
        std::fs::write(root.join(MOD_LOADER_PAYLOAD_RELATIVE_PATH), b"fixture")
            .expect("write payload fixture");
        assert_eq!(
            service.snapshot().expect("ready snapshot").phase,
            ModLoaderRuntimePhase::Stopped
        );

        std::fs::remove_dir_all(root).expect("remove test layout");
    }
}
