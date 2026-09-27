//! Framed, bounded native Combat push transport. No v7 messages, DLL loading,
//! packet parsing or automatic retry of a mutation. Closing owns session cleanup.
use super::{
    mods_plugin::{overlapped_read_exact, overlapped_write_exact, pipe_io_ready},
    toolkit::{ToolkitClient, ToolkitError},
};
use std::{
    ptr,
    sync::atomic::{AtomicBool, Ordering},
    time::{Duration, Instant},
};
use windows_sys::Win32::{
    Foundation::{CloseHandle, HANDLE, INVALID_HANDLE_VALUE},
    Storage::FileSystem::{
        CreateFileW, FILE_FLAG_OVERLAPPED, FILE_READ_DATA, FILE_WRITE_DATA, OPEN_EXISTING,
    },
    System::Pipes::{GetNamedPipeServerProcessId, PeekNamedPipe},
};
pub const MAX_FRAME: usize = 262_144;
pub struct CapturePipe {
    handle: HANDLE,
}
impl Drop for CapturePipe {
    fn drop(&mut self) {
        // SAFETY: sole owned handle, every transfer completed or retained its
        // owned operation storage on cancellation failure before this close.
        unsafe {
            CloseHandle(self.handle);
        }
    }
}
impl CapturePipe {
    pub fn open(host: &ToolkitClient) -> Result<Self, ToolkitError> {
        if !pipe_io_ready() {
            return Err(ToolkitError::Unavailable);
        }
        let (pid, created) = host.process_identity();
        let name: Vec<u16> = format!("\\\\.\\pipe\\nte.capture.v1.{pid}.{created}")
            .encode_utf16()
            .chain(Some(0))
            .collect();
        // SAFETY: terminated name; the handle is exclusively owned, and all I/O uses
        // stable owned OVERLAPPED buffers with cancellation/drain in shared helpers.
        let handle = unsafe {
            CreateFileW(
                name.as_ptr(),
                FILE_READ_DATA | FILE_WRITE_DATA,
                0,
                ptr::null(),
                OPEN_EXISTING,
                FILE_FLAG_OVERLAPPED,
                ptr::null_mut(),
            )
        };
        if handle == INVALID_HANDLE_VALUE {
            return Err(ToolkitError::Unavailable);
        }
        let pipe = Self { handle };
        let mut server = 0;
        // SAFETY: live pipe handle and valid DWORD output; bind to the inspected host.
        if unsafe { GetNamedPipeServerProcessId(handle, &mut server) } == 0 || server != pid {
            return Err(ToolkitError::InvalidProtocol);
        }
        Ok(pipe)
    }
    pub fn send(
        &self,
        id: &str,
        method: &str,
        params: serde_json::Value,
    ) -> Result<(), ToolkitError> {
        if !pipe_io_ready() {
            return Err(ToolkitError::Unavailable);
        }
        let bytes = serde_json::to_vec(
            &serde_json::json!({"jsonrpc":"2.0","id":id,"method":method,"params":params}),
        )
        .map_err(|_| ToolkitError::InvalidProtocol)?;
        if bytes.len() > 16384 {
            return Err(ToolkitError::TooLarge);
        }
        let mut frame = (bytes.len() as u32).to_le_bytes().to_vec();
        frame.extend(bytes);
        overlapped_write_exact(self.handle, &frame, Instant::now() + Duration::from_secs(2))
            .map_err(|_| ToolkitError::Failed)
    }
    fn available(&self) -> Result<usize, ToolkitError> {
        let mut n = 0;
        // SAFETY: live sole-reader pipe, valid DWORD output; no payload access here.
        if unsafe {
            PeekNamedPipe(
                self.handle,
                ptr::null_mut(),
                0,
                ptr::null_mut(),
                &mut n,
                ptr::null_mut(),
            )
        } == 0
        {
            return Err(ToolkitError::Unavailable);
        }
        Ok(n as usize)
    }
    fn exact(
        &self,
        n: usize,
        deadline: Instant,
        stop: &AtomicBool,
    ) -> Result<Vec<u8>, ToolkitError> {
        let mut out = Vec::with_capacity(n);
        while out.len() < n {
            if stop.load(Ordering::Acquire) {
                return Err(ToolkitError::Cancelled);
            }
            if Instant::now() >= deadline {
                return Err(ToolkitError::Timeout);
            }
            let count = self.available()?.min(n - out.len());
            if count == 0 {
                std::thread::sleep(Duration::from_millis(2));
                continue;
            }
            let bytes = overlapped_read_exact(self.handle, count, deadline)
                .map_err(|_| ToolkitError::Failed)?;
            out.extend_from_slice(&bytes);
        }
        Ok(out)
    }
    pub fn read(&self, stop: &AtomicBool) -> Result<Option<Vec<u8>>, ToolkitError> {
        if !pipe_io_ready() {
            return Err(ToolkitError::Unavailable);
        }
        if stop.load(Ordering::Acquire) {
            return Err(ToolkitError::Cancelled);
        }
        if self.available()? == 0 {
            return Ok(None);
        }
        let deadline = Instant::now() + Duration::from_secs(2);
        let header = self.exact(4, deadline, stop)?;
        let size = u32::from_le_bytes(
            header
                .try_into()
                .map_err(|_| ToolkitError::InvalidProtocol)?,
        ) as usize;
        if size == 0 || size > MAX_FRAME {
            return Err(ToolkitError::TooLarge);
        }
        self.exact(size, deadline, stop).map(Some)
    }
}
