//! Client for the compiled UE Tools Toolkit v1 ABI. Never loads a DLL into
//! this process, injects code, or falls back to the retired Mod v7 protocol.
use serde::{Deserialize, Serialize};
use std::{
    ffi::OsStr,
    os::windows::ffi::OsStrExt,
    ptr,
    time::{Duration, Instant},
};
use windows_sys::Win32::{
    Foundation::{CloseHandle, FILETIME, HANDLE, WAIT_ABANDONED, WAIT_OBJECT_0, WAIT_TIMEOUT},
    System::{
        Memory::{
            FILE_MAP_ALL_ACCESS, MEMORY_MAPPED_VIEW_ADDRESS, MapViewOfFile, OpenFileMappingW,
            UnmapViewOfFile,
        },
        SystemInformation::GetTickCount64,
        Threading::{
            CreateMutexW, GetProcessTimes, OpenMutexW, OpenProcess,
            PROCESS_QUERY_LIMITED_INFORMATION, ReleaseMutex, SYNCHRONIZATION_SYNCHRONIZE,
            WaitForSingleObject,
        },
    },
};

const REGION_BYTES: usize = 262_760;
const PAGE_BYTES: usize = 262_144;
pub const MAX_BLOB_BYTES: usize = 32 * 1024 * 1024;
const REQUEST: usize = 40;
const RESPONSE: usize = 584;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ToolkitError {
    Unavailable,
    Busy,
    Timeout,
    InvalidProtocol,
    TooLarge,
    Unsupported,
    Failed,
    Cancelled,
    DataGap,
    SessionChanged,
}
impl std::fmt::Display for ToolkitError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{self:?}")
    }
}
impl std::error::Error for ToolkitError {}

struct Handle(HANDLE);
impl Handle {
    fn new(raw: HANDLE) -> Result<Self, ToolkitError> {
        if raw.is_null() {
            Err(ToolkitError::Unavailable)
        } else {
            Ok(Self(raw))
        }
    }
    fn lock(&self, deadline: Instant) -> Result<MutexGuard<'_>, ToolkitError> {
        let ms = deadline
            .saturating_duration_since(Instant::now())
            .as_millis()
            .min(100) as u32;
        // SAFETY: Owned live mutex handle; waits are bounded and never hold a Rust state lock.
        match unsafe { WaitForSingleObject(self.0, ms) } {
            WAIT_OBJECT_0 => Ok(MutexGuard(self)),
            WAIT_ABANDONED => {
                unsafe {
                    ReleaseMutex(self.0);
                }
                Err(ToolkitError::InvalidProtocol)
            }
            WAIT_TIMEOUT => Err(ToolkitError::Busy),
            _ => Err(ToolkitError::Unavailable),
        }
    }
}
impl Drop for Handle {
    fn drop(&mut self) {
        unsafe {
            CloseHandle(self.0);
        }
    }
}
struct MutexGuard<'a>(&'a Handle);
impl Drop for MutexGuard<'_> {
    fn drop(&mut self) {
        unsafe {
            ReleaseMutex(self.0.0);
        }
    }
}
struct Mapping(*mut u8);
impl Drop for Mapping {
    fn drop(&mut self) {
        unsafe {
            UnmapViewOfFile(MEMORY_MAPPED_VIEW_ADDRESS {
                Value: self.0.cast(),
            });
        }
    }
}
fn wide(s: &str) -> Vec<u16> {
    OsStr::new(s).encode_wide().chain(Some(0)).collect()
}

/// One finite request group owns all handles. `.client` serializes the entire
/// call including pagination, `.gate` only protects bounded memory copies.
/// Capacity 1 / serialized order / Busy on full / no replay after disconnect.
pub struct ToolkitClient {
    view: Mapping,
    _mapping: Handle,
    gate: Handle,
    client: Handle,
    process: Handle,
    pid: u32,
    created: u64,
}
impl ToolkitClient {
    pub fn open(pid: u32) -> Result<Self, ToolkitError> {
        let prefix = format!("Local\\NTE.Toolkit.v1.{pid}");
        // SAFETY: NUL-terminated names, no borrowed security descriptors; each
        // returned handle/view is owned by RAII even on partial initialization.
        unsafe {
            let gate = Handle::new(OpenMutexW(
                0x100001,
                0,
                wide(&format!("{prefix}.gate")).as_ptr(),
            ))?;
            let mapping = Handle::new(OpenFileMappingW(
                FILE_MAP_ALL_ACCESS,
                0,
                wide(&format!("{prefix}.memory")).as_ptr(),
            ))?;
            let client = Handle::new(CreateMutexW(
                ptr::null(),
                0,
                wide(&format!("{prefix}.client")).as_ptr(),
            ))?;
            let process = Handle::new(OpenProcess(
                PROCESS_QUERY_LIMITED_INFORMATION | SYNCHRONIZATION_SYNCHRONIZE,
                0,
                pid,
            ))?;
            let address = MapViewOfFile(mapping.0, FILE_MAP_ALL_ACCESS, 0, 0, REGION_BYTES);
            if address.Value.is_null() {
                return Err(ToolkitError::Unavailable);
            }
            let view = Mapping(address.Value.cast());
            let mut times = [FILETIME {
                dwLowDateTime: 0,
                dwHighDateTime: 0,
            }; 4];
            if GetProcessTimes(
                process.0,
                &mut times[0],
                &mut times[1],
                &mut times[2],
                &mut times[3],
            ) == 0
            {
                return Err(ToolkitError::Unavailable);
            }
            let created =
                u64::from(times[0].dwHighDateTime) << 32 | u64::from(times[0].dwLowDateTime);
            let result = Self {
                view,
                _mapping: mapping,
                gate,
                client,
                process,
                pid,
                created,
            };
            {
                let _gate = result
                    .gate
                    .lock(Instant::now() + Duration::from_millis(100))?;
                result.validate()?;
            }
            Ok(result)
        }
    }
    // SAFETY: All offsets below are fixed ABI offsets inside REGION_BYTES;
    // all accesses happen while the OS gate is held. Volatile access avoids
    // creating Rust references to concurrently mapped foreign memory.
    fn u32(&self, offset: usize) -> u32 {
        unsafe { ptr::read_volatile(self.view.0.add(offset).cast()) }
    }
    fn u64(&self, offset: usize) -> u64 {
        unsafe { ptr::read_volatile(self.view.0.add(offset).cast()) }
    }
    fn put32(&self, offset: usize, value: u32) {
        unsafe {
            ptr::write_volatile(self.view.0.add(offset).cast(), value);
        }
    }
    fn put64(&self, offset: usize, value: u64) {
        unsafe {
            ptr::write_volatile(self.view.0.add(offset).cast(), value);
        }
    }
    fn validate(&self) -> Result<(), ToolkitError> {
        if self.u32(0) != 0x314b544e
            || self.u32(4) != 1
            || self.u32(8) != REGION_BYTES as u32
            || self.u32(12) != self.pid
            || self.u64(16) != self.created
        {
            return Err(ToolkitError::InvalidProtocol);
        }
        // SAFETY: live process handle; OS clock has no pointer arguments.
        let alive = unsafe { WaitForSingleObject(self.process.0, 0) } == WAIT_TIMEOUT;
        let now = unsafe { GetTickCount64() };
        if !alive || !matches!(self.u32(36), 1 | 2) || now.saturating_sub(self.u64(24)) > 15_000 {
            return Err(ToolkitError::Unavailable);
        }
        Ok(())
    }
    fn exchange(
        &self,
        command: u32,
        argument: u32,
        text: &[u8],
        cursor: (u64, u32),
        deadline: Instant,
        cancelled: &dyn Fn() -> bool,
    ) -> Result<Page, ToolkitError> {
        if cancelled() {
            return Err(ToolkitError::Cancelled);
        }
        let id;
        {
            let _gate = self.gate.lock(deadline)?;
            self.validate()?;
            if !matches!(self.u32(32), 0 | 3) {
                return Err(ToolkitError::Busy);
            }
            id = self
                .u64(REQUEST)
                .checked_add(1)
                .ok_or(ToolkitError::InvalidProtocol)?;
            self.put64(REQUEST, id);
            self.put32(REQUEST + 8, command);
            self.put32(REQUEST + 12, argument);
            self.put64(REQUEST + 16, cursor.0);
            self.put32(REQUEST + 24, cursor.1);
            self.put32(REQUEST + 28, text.len() as u32);
            // SAFETY: text was bounded to 511 bytes by call, request text is 512 bytes.
            unsafe {
                ptr::copy_nonoverlapping(text.as_ptr(), self.view.0.add(REQUEST + 32), text.len());
                ptr::write_volatile(self.view.0.add(REQUEST + 32 + text.len()), 0);
            }
            self.put32(32, 1);
        }
        while Instant::now() < deadline {
            if cancelled() {
                return Err(ToolkitError::Cancelled);
            }
            {
                let _gate = self.gate.lock(deadline)?;
                self.validate()?;
                if self.u32(32) == 3 {
                    if self.u64(RESPONSE) != id {
                        return Err(ToolkitError::InvalidProtocol);
                    }
                    let status = self.u32(RESPONSE + 8);
                    let bytes = self.u32(RESPONSE + 12) as usize;
                    let blob = self.u64(RESPONSE + 16);
                    let total = self.u32(RESPONSE + 24) as usize;
                    let offset = self.u32(RESPONSE + 28) as usize;
                    validate_page(bytes, total, offset)?;
                    let mut payload = vec![0; bytes];
                    unsafe {
                        ptr::copy_nonoverlapping(
                            self.view.0.add(RESPONSE + 32),
                            payload.as_mut_ptr(),
                            bytes,
                        );
                    }
                    self.put32(32, 0);
                    return Ok(Page {
                        status,
                        blob,
                        total,
                        offset,
                        payload,
                    });
                }
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        // The server may still execute it. Never reset a Pending slot or retry.
        Err(ToolkitError::Timeout)
    }
    pub fn call(
        &self,
        command: u32,
        argument: u32,
        text: &str,
        cancelled: &dyn Fn() -> bool,
    ) -> Result<Vec<u8>, ToolkitError> {
        if text.len() >= 512 || text.contains('\0') {
            return Err(ToolkitError::InvalidProtocol);
        }
        let deadline = Instant::now() + Duration::from_secs(5);
        let _client = self.client.lock(deadline)?;
        let first = self.exchange(
            command,
            argument,
            text.as_bytes(),
            (0, 0),
            deadline,
            cancelled,
        )?;
        if first.offset != 0 {
            return Err(ToolkitError::InvalidProtocol);
        }
        match first.status {
            0 | 1 => (),
            3 => return Err(ToolkitError::Unsupported),
            4 | 9 => return Err(ToolkitError::Unavailable),
            5 => return Err(ToolkitError::Busy),
            7 => return Err(ToolkitError::TooLarge),
            _ => return Err(ToolkitError::Failed),
        }
        let mut bytes = first.payload;
        while bytes.len() < first.total {
            let next = self.exchange(
                200,
                0,
                &[],
                (first.blob, bytes.len() as u32),
                deadline,
                cancelled,
            )?;
            if next.status != 0
                || next.blob != first.blob
                || next.total != first.total
                || next.offset != bytes.len()
                || next.payload.is_empty()
            {
                return Err(ToolkitError::InvalidProtocol);
            }
            bytes.extend(next.payload);
        }
        Ok(bytes)
    }
    pub fn json<T: serde::de::DeserializeOwned>(
        &self,
        command: u32,
        argument: u32,
        text: &str,
        cancelled: &dyn Fn() -> bool,
    ) -> Result<T, ToolkitError> {
        serde_json::from_slice(&self.call(command, argument, text, cancelled)?)
            .map_err(|_| ToolkitError::InvalidProtocol)
    }
    pub fn capabilities(&self) -> Result<Vec<u32>, ToolkitError> {
        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase")]
        struct Description {
            protocol: u32,
            mode: String,
            // UI presence is metadata, not a different control protocol.
            #[serde(rename = "ui")]
            _ui: bool,
            legacy_mod_protocol: bool,
            commands: Vec<u32>,
        }
        let d: Description = self.json(1, 0, "", &|| false)?;
        if d.protocol != 1
            || d.mode != "toolkit"
            || d.legacy_mod_protocol
            || d.commands.len() > 128
            || ![3, 100, 101, 102, 106, 107, 200]
                .iter()
                .all(|c| d.commands.contains(c))
        {
            return Err(ToolkitError::Unsupported);
        }
        let mut commands = d.commands;
        commands.sort_unstable();
        commands.dedup();
        Ok(commands)
    }
    pub fn describe(&self) -> Result<(), ToolkitError> {
        self.capabilities().map(|_| ())
    }
    pub fn process_identity(&self) -> (u32, u64) {
        (self.pid, self.created)
    }
    pub fn identity(&self) -> String {
        format!("{}:{}", self.pid, self.created)
    }
}
struct Page {
    status: u32,
    blob: u64,
    total: usize,
    offset: usize,
    payload: Vec<u8>,
}
fn validate_page(bytes: usize, total: usize, offset: usize) -> Result<(), ToolkitError> {
    if total > MAX_BLOB_BYTES {
        return Err(ToolkitError::TooLarge);
    }
    if bytes > PAGE_BYTES || offset > total || bytes > total - offset {
        return Err(ToolkitError::InvalidProtocol);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn toolkit_page_limits() {
        assert_eq!(validate_page(0, 0, 0), Ok(()));
        assert_eq!(validate_page(PAGE_BYTES, MAX_BLOB_BYTES, 0), Ok(()));
        assert_eq!(
            validate_page(PAGE_BYTES + 1, MAX_BLOB_BYTES, 0),
            Err(ToolkitError::InvalidProtocol)
        );
        assert_eq!(
            validate_page(1, MAX_BLOB_BYTES + 1, 0),
            Err(ToolkitError::TooLarge)
        );
        assert_eq!(validate_page(4, 3, 0), Err(ToolkitError::InvalidProtocol));
        assert_eq!(validate_page(0, 3, 4), Err(ToolkitError::InvalidProtocol));
    }
    /// An opt-in test against a separate process hosting the unchanged compiled DLLs.
    #[test]
    #[ignore]
    fn toolkit_compiled_binary_contract() {
        let pid: u32 = std::env::var("NTE_TOOLKIT_TEST_PID")
            .unwrap()
            .parse()
            .unwrap();
        let client = ToolkitClient::open(pid).unwrap();
        client.describe().unwrap();
        let plugins: Vec<serde_json::Value> = client.json(3, 0, "", &|| false).unwrap();
        assert!(
            plugins
                .iter()
                .any(|p| p["file"] == "NTE_PluginCombat.dll" && p["state"] == "loaded")
        );
        for (command, expected) in [(5, "unloaded"), (4, "loaded")] {
            client
                .call(command, 0, "NTE_PluginCombat.dll", &|| false)
                .unwrap();
            let deadline = Instant::now() + Duration::from_secs(10);
            loop {
                let list: Vec<serde_json::Value> = client.json(3, 0, "", &|| false).unwrap();
                if list
                    .iter()
                    .any(|p| p["file"] == "NTE_PluginCombat.dll" && p["state"] == expected)
                {
                    break;
                }
                assert!(
                    Instant::now() < deadline,
                    "plugin operation did not complete"
                );
                std::thread::sleep(Duration::from_millis(25));
            }
        }
        client.call(8, 2, "", &|| false).unwrap();
        println!(
            "TOOLKIT_CONTROL_PASS: real compiled plugin disabled, enabled, and log level accepted"
        );
        let report: serde_json::Value = client.json(107, 0, "", &|| false).unwrap();
        assert_eq!(report["schemaVersion"], 14);
        println!(
            "TOOLKIT_BINARY_PASS: Describe v1, loaded combat plugin, report schema 14; no game data asserted"
        );
    }
}
