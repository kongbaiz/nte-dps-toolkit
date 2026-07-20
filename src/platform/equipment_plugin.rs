//! Asynchronous client for the in-process equipment plugin's local named pipe.
//! The blocking pipe transaction stays on a worker thread; the UI only submits
//! validated session item IDs and polls completed responses.

use std::io;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread::{self, JoinHandle};

#[cfg(feature = "gui")]
use std::fs;
#[cfg(feature = "gui")]
use std::path::{Path, PathBuf};
#[cfg(feature = "gui")]
use std::ptr;

use crossbeam_channel::{Receiver, Sender, TryRecvError, TrySendError, bounded, unbounded};
#[cfg(feature = "gui")]
use windows_sys::Win32::Foundation::ERROR_FILE_NOT_FOUND;
use windows_sys::Win32::System::Pipes::CallNamedPipeW;
#[cfg(feature = "gui")]
use windows_sys::Win32::System::Registry::{
    HKEY_LOCAL_MACHINE, REG_SZ, RRF_RT_REG_SZ, RRF_SUBKEY_WOW6432KEY, RegGetValueW,
};

use crate::engine::model::HtItemNetId;

const PIPE_NAME: &str = r"\\.\pipe\nte-equipment-plugin-v3";
const IPC_MAGIC: u32 = 0x5145_544e;
const IPC_VERSION: u16 = 3;
const IPC_EQUIP_MODULE: u16 = 1;
const IPC_EQUIP_CORE: u16 = 2;
const IPC_UNEQUIP_MODULE: u16 = 3;
const IPC_UNEQUIP_CORE: u16 = 4;
const IPC_UNEQUIP_ALL: u16 = 5;
const IPC_EQUIP_ONE_KEY: u16 = 6;
const IPC_MOVE_MODULE_TO_CHARACTER: u16 = 7;
const IPC_MOVE_CORE_TO_CHARACTER: u16 = 8;
const IPC_SET_ITEM_DISCARDED: u16 = 9;
const IPC_SET_ITEM_LOCKED: u16 = 10;
const IPC_TIMEOUT_MS: u32 = 1_500;
const MAX_PLACEMENTS: usize = 64;
const REQUEST_HEADER_SIZE: usize = 56;
const PLACEMENT_SIZE: usize = 16;
const REQUEST_SIZE: usize = REQUEST_HEADER_SIZE + MAX_PLACEMENTS * PLACEMENT_SIZE;
const RESPONSE_SIZE: usize = 24;
const MAX_PLUGIN_STATUS: u32 = 12;

#[cfg(feature = "gui")]
const GAME_INSTALL_REGISTRY_KEYS: [(EquipmentPluginGameRegion, &str); 2] = [
    (
        EquipmentPluginGameRegion::China,
        r"SOFTWARE\Microsoft\Windows\CurrentVersion\Uninstall\YH",
    ),
    (
        EquipmentPluginGameRegion::Global,
        r"SOFTWARE\Microsoft\Windows\CurrentVersion\Uninstall\NTEGlobal",
    ),
];
#[cfg(feature = "gui")]
const GAME_BINARY_RELATIVE_PATH: &str = r"Client\WindowsNoEditor\HT\Binaries\Win64";
#[cfg(feature = "gui")]
const GAME_EXECUTABLE_NAME: &str = "HTGame.exe";
#[cfg(feature = "gui")]
const PLUGIN_FILE_NAME: &str = "dwmapi.dll";
#[cfg(feature = "gui")]
const PLUGIN_MARKER_FILE_NAME: &str = ".nte-dps-tool-equipment-plugin";
#[cfg(feature = "gui")]
const PLUGIN_MARKER_HEADER: &str = "NTE_DPS_TOOL_EQUIPMENT_PLUGIN_V1";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct EquipmentPluginPlacement {
    pub equipment: HtItemNetId,
    pub row: i32,
    pub column: i32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum EquipmentPluginOperation {
    EquipModule {
        equipment: HtItemNetId,
        row: i32,
        column: i32,
    },
    EquipCore {
        equipment: HtItemNetId,
    },
    UnequipModule {
        equipment: HtItemNetId,
    },
    UnequipCore {
        equipment: HtItemNetId,
    },
    UnequipAll,
    EquipOneKey {
        placements: Vec<EquipmentPluginPlacement>,
        core: HtItemNetId,
    },
    MoveModuleToCharacter {
        equipment: HtItemNetId,
        row: i32,
        column: i32,
    },
    MoveCoreToCharacter {
        equipment: HtItemNetId,
    },
    SetItemDiscarded {
        equipment: HtItemNetId,
        discarded: bool,
    },
    SetItemLocked {
        equipment: HtItemNetId,
        locked: bool,
    },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EquipmentPluginRequest {
    pub request_id: u64,
    pub character: HtItemNetId,
    pub operation: EquipmentPluginOperation,
}

#[derive(Debug, PartialEq, Eq)]
pub struct EquipmentPluginResponse {
    pub request_id: u64,
    pub status: Result<u32, String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EquipmentPluginSubmitError {
    Busy,
}

#[cfg(feature = "gui")]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EquipmentPluginGameRegion {
    China,
    Global,
}

#[cfg(feature = "gui")]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct EquipmentPluginGameStatus {
    pub region: EquipmentPluginGameRegion,
    pub installed: bool,
    pub current: bool,
}

#[cfg(feature = "gui")]
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct EquipmentPluginDeploymentStatus {
    pub installations: usize,
    pub installed: usize,
    pub current: usize,
    pub source_available: bool,
    pub games: Vec<EquipmentPluginGameStatus>,
}

#[cfg(feature = "gui")]
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum EquipmentPluginDeploymentError {
    GameRunning,
    GameProcessProbe(String),
    GameInstallationNotFound,
    Registry(String),
    PluginSourceNotFound,
    ConflictingDwmapi,
    InstalledPluginChanged,
    FileSystem(String),
}

enum WorkerCommand {
    Request(EquipmentPluginRequest),
    Stop,
}

pub struct EquipmentPluginClient {
    sender: Sender<WorkerCommand>,
    receiver: Receiver<EquipmentPluginResponse>,
    thread: Option<JoinHandle<()>>,
    stop: Arc<AtomicBool>,
    next_request_id: u64,
}

impl Default for EquipmentPluginClient {
    fn default() -> Self {
        Self::new()
    }
}

impl EquipmentPluginClient {
    pub fn new() -> Self {
        Self::with_call(call_plugin)
    }

    fn with_call<F>(call: F) -> Self
    where
        F: Fn(&EquipmentPluginRequest) -> Result<u32, String> + Send + 'static,
    {
        let (sender, command_receiver) = bounded(1);
        let (response_sender, receiver) = unbounded();
        let stop = Arc::new(AtomicBool::new(false));
        let worker_stop = Arc::clone(&stop);
        let thread = thread::spawn(move || {
            while let Ok(command) = command_receiver.recv() {
                match command {
                    WorkerCommand::Request(request) => {
                        if worker_stop.load(Ordering::Acquire) {
                            return;
                        }
                        let status = call(&request);
                        if response_sender
                            .send(EquipmentPluginResponse {
                                request_id: request.request_id,
                                status,
                            })
                            .is_err()
                        {
                            return;
                        }
                        if worker_stop.load(Ordering::Acquire) {
                            return;
                        }
                    }
                    WorkerCommand::Stop => return,
                }
            }
        });
        Self {
            sender,
            receiver,
            thread: Some(thread),
            stop,
            next_request_id: 1,
        }
    }

    pub fn submit(
        &mut self,
        character: HtItemNetId,
        operation: EquipmentPluginOperation,
    ) -> Result<u64, EquipmentPluginSubmitError> {
        let request_id = self.next_request_id;
        self.next_request_id = self.next_request_id.wrapping_add(1).max(1);
        self.submit_request(EquipmentPluginRequest {
            request_id,
            character,
            operation,
        })?;
        Ok(request_id)
    }

    pub fn submit_request(
        &self,
        request: EquipmentPluginRequest,
    ) -> Result<(), EquipmentPluginSubmitError> {
        match self.sender.try_send(WorkerCommand::Request(request)) {
            Ok(()) => Ok(()),
            Err(TrySendError::Full(_)) => Err(EquipmentPluginSubmitError::Busy),
            Err(TrySendError::Disconnected(_)) => {
                panic!("equipment plugin worker must remain alive while its client exists")
            }
        }
    }

    pub fn response_receiver(&self) -> Receiver<EquipmentPluginResponse> {
        self.receiver.clone()
    }

    pub fn try_recv(&self) -> Option<EquipmentPluginResponse> {
        match self.receiver.try_recv() {
            Ok(response) => Some(response),
            Err(TryRecvError::Empty) => None,
            Err(TryRecvError::Disconnected) => {
                panic!("equipment plugin worker disconnected before its client was dropped")
            }
        }
    }

    #[cfg(test)]
    pub(crate) fn with_call_for_test<F>(call: F) -> Self
    where
        F: Fn(&EquipmentPluginRequest) -> Result<u32, String> + Send + 'static,
    {
        Self::with_call(call)
    }
}

impl Drop for EquipmentPluginClient {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        let _ = self.sender.try_send(WorkerCommand::Stop);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

pub(crate) fn call_plugin(request: &EquipmentPluginRequest) -> Result<u32, String> {
    let request_bytes = encode_request(request);
    let mut response = [0_u8; RESPONSE_SIZE];
    let mut bytes_read = 0;
    let mut pipe_name = PIPE_NAME.encode_utf16().collect::<Vec<_>>();
    pipe_name.push(0);

    // SAFETY: both buffers live for the duration of the synchronous call, their
    // exact lengths are passed to Win32, and the pipe name is NUL-terminated.
    let succeeded = unsafe {
        CallNamedPipeW(
            pipe_name.as_ptr(),
            request_bytes.as_ptr().cast(),
            REQUEST_SIZE as u32,
            response.as_mut_ptr().cast(),
            RESPONSE_SIZE as u32,
            &mut bytes_read,
            IPC_TIMEOUT_MS,
        )
    };
    if succeeded == 0 {
        return Err(io::Error::last_os_error().to_string());
    }
    if bytes_read != RESPONSE_SIZE as u32 {
        return Err(format!(
            "equipment plugin returned {bytes_read} bytes; expected {RESPONSE_SIZE}"
        ));
    }
    decode_response(&response, request.request_id)
}

fn encode_request(request: &EquipmentPluginRequest) -> [u8; REQUEST_SIZE] {
    let (operation, equipment, core, row, column, state, placements) = match &request.operation {
        EquipmentPluginOperation::EquipModule {
            equipment,
            row,
            column,
        } => (
            IPC_EQUIP_MODULE,
            *equipment,
            HtItemNetId::ZERO,
            *row,
            *column,
            0,
            &[][..],
        ),
        EquipmentPluginOperation::EquipCore { equipment } => (
            IPC_EQUIP_CORE,
            *equipment,
            HtItemNetId::ZERO,
            0,
            0,
            0,
            &[][..],
        ),
        EquipmentPluginOperation::UnequipModule { equipment } => (
            IPC_UNEQUIP_MODULE,
            *equipment,
            HtItemNetId::ZERO,
            0,
            0,
            0,
            &[][..],
        ),
        EquipmentPluginOperation::UnequipCore { equipment } => (
            IPC_UNEQUIP_CORE,
            *equipment,
            HtItemNetId::ZERO,
            0,
            0,
            0,
            &[][..],
        ),
        EquipmentPluginOperation::UnequipAll => (
            IPC_UNEQUIP_ALL,
            HtItemNetId::ZERO,
            HtItemNetId::ZERO,
            0,
            0,
            0,
            &[][..],
        ),
        EquipmentPluginOperation::EquipOneKey { placements, core } => {
            assert!(
                !placements.is_empty() && placements.len() <= MAX_PLACEMENTS,
                "business-layer one-key plans must fit the plugin ABI"
            );
            (
                IPC_EQUIP_ONE_KEY,
                HtItemNetId::ZERO,
                *core,
                0,
                0,
                0,
                placements.as_slice(),
            )
        }
        EquipmentPluginOperation::MoveModuleToCharacter {
            equipment,
            row,
            column,
        } => (
            IPC_MOVE_MODULE_TO_CHARACTER,
            *equipment,
            HtItemNetId::ZERO,
            *row,
            *column,
            0,
            &[][..],
        ),
        EquipmentPluginOperation::MoveCoreToCharacter { equipment } => (
            IPC_MOVE_CORE_TO_CHARACTER,
            *equipment,
            HtItemNetId::ZERO,
            0,
            0,
            0,
            &[][..],
        ),
        EquipmentPluginOperation::SetItemDiscarded {
            equipment,
            discarded,
        } => (
            IPC_SET_ITEM_DISCARDED,
            *equipment,
            HtItemNetId::ZERO,
            0,
            0,
            u32::from(*discarded),
            &[][..],
        ),
        EquipmentPluginOperation::SetItemLocked { equipment, locked } => (
            IPC_SET_ITEM_LOCKED,
            *equipment,
            HtItemNetId::ZERO,
            0,
            0,
            u32::from(*locked),
            &[][..],
        ),
    };
    let mut bytes = [0_u8; REQUEST_SIZE];
    bytes[0..4].copy_from_slice(&IPC_MAGIC.to_le_bytes());
    bytes[4..6].copy_from_slice(&IPC_VERSION.to_le_bytes());
    bytes[6..8].copy_from_slice(&operation.to_le_bytes());
    bytes[8..16].copy_from_slice(&request.request_id.to_le_bytes());
    bytes[16..20].copy_from_slice(&request.character.solt.to_le_bytes());
    bytes[20..24].copy_from_slice(&request.character.serial.to_le_bytes());
    bytes[24..28].copy_from_slice(&equipment.solt.to_le_bytes());
    bytes[28..32].copy_from_slice(&equipment.serial.to_le_bytes());
    bytes[32..36].copy_from_slice(&core.solt.to_le_bytes());
    bytes[36..40].copy_from_slice(&core.serial.to_le_bytes());
    bytes[40..44].copy_from_slice(&row.to_le_bytes());
    bytes[44..48].copy_from_slice(&column.to_le_bytes());
    bytes[48..52].copy_from_slice(&(placements.len() as u32).to_le_bytes());
    bytes[52..56].copy_from_slice(&state.to_le_bytes());
    for (index, placement) in placements.iter().enumerate() {
        let offset = REQUEST_HEADER_SIZE + index * PLACEMENT_SIZE;
        bytes[offset..offset + 4].copy_from_slice(&placement.equipment.solt.to_le_bytes());
        bytes[offset + 4..offset + 8].copy_from_slice(&placement.equipment.serial.to_le_bytes());
        bytes[offset + 8..offset + 12].copy_from_slice(&placement.row.to_le_bytes());
        bytes[offset + 12..offset + 16].copy_from_slice(&placement.column.to_le_bytes());
    }
    bytes
}

fn decode_response(bytes: &[u8; RESPONSE_SIZE], request_id: u64) -> Result<u32, String> {
    let magic = u32::from_le_bytes(bytes[0..4].try_into().expect("fixed response magic"));
    let version = u16::from_le_bytes(bytes[4..6].try_into().expect("fixed response version"));
    let reserved = u16::from_le_bytes(bytes[6..8].try_into().expect("fixed response reserved"));
    let response_id =
        u64::from_le_bytes(bytes[8..16].try_into().expect("fixed response request id"));
    let status = u32::from_le_bytes(bytes[16..20].try_into().expect("fixed response status"));
    let reserved2 = u32::from_le_bytes(bytes[20..24].try_into().expect("fixed response reserved2"));
    if magic != IPC_MAGIC
        || version != IPC_VERSION
        || reserved != 0
        || reserved2 != 0
        || response_id != request_id
        || status > MAX_PLUGIN_STATUS
    {
        return Err("equipment plugin returned an invalid IPC response".to_owned());
    }
    Ok(status)
}

#[cfg(feature = "gui")]
pub fn inspect_plugin_deployment(
    current_plugin: Option<&[u8]>,
) -> Result<EquipmentPluginDeploymentStatus, EquipmentPluginDeploymentError> {
    let installations = game_installation_directories()?;
    inspect_game_installations(&installations, current_plugin)
}

#[cfg(feature = "gui")]
pub fn install_equipment_plugin(
    region: EquipmentPluginGameRegion,
    plugin: &[u8],
) -> Result<EquipmentPluginDeploymentStatus, EquipmentPluginDeploymentError> {
    ensure_game_is_closed()?;
    let installations = game_installation_directories()?;
    let directory = selected_game_directory(&installations, region)?;
    install_plugin_to_directories(std::slice::from_ref(directory), plugin)?;
    inspect_game_installations(&installations, Some(plugin))
}

#[cfg(feature = "gui")]
pub fn remove_equipment_plugin(
    region: EquipmentPluginGameRegion,
) -> Result<EquipmentPluginDeploymentStatus, EquipmentPluginDeploymentError> {
    ensure_game_is_closed()?;
    let installations = game_installation_directories()?;
    let directory = selected_game_directory(&installations, region)?;
    remove_plugin_from_directories(std::slice::from_ref(directory))?;
    inspect_game_installations(&installations, None)
}

#[cfg(feature = "gui")]
fn ensure_game_is_closed() -> Result<(), EquipmentPluginDeploymentError> {
    match super::network::game_process_is_running() {
        Ok(false) => Ok(()),
        Ok(true) => Err(EquipmentPluginDeploymentError::GameRunning),
        Err(error) => Err(EquipmentPluginDeploymentError::GameProcessProbe(error)),
    }
}

#[cfg(feature = "gui")]
fn game_installation_directories()
-> Result<Vec<(EquipmentPluginGameRegion, PathBuf)>, EquipmentPluginDeploymentError> {
    let mut directories = Vec::new();
    for (region, key) in GAME_INSTALL_REGISTRY_KEYS {
        let Some(root) = read_registry_string(key, "InstallLocation")? else {
            continue;
        };
        let root = PathBuf::from(root.trim().trim_matches('"'));
        for directory in [
            root.join(GAME_BINARY_RELATIVE_PATH),
            root.join("Neverness To Everness")
                .join(GAME_BINARY_RELATIVE_PATH),
        ] {
            if directory.join(GAME_EXECUTABLE_NAME).is_file()
                && !directories.iter().any(|(_, known)| known == &directory)
            {
                directories.push((region, directory));
                break;
            }
        }
    }
    if directories.is_empty() {
        return Err(EquipmentPluginDeploymentError::GameInstallationNotFound);
    }
    Ok(directories)
}

#[cfg(feature = "gui")]
fn selected_game_directory(
    installations: &[(EquipmentPluginGameRegion, PathBuf)],
    region: EquipmentPluginGameRegion,
) -> Result<&PathBuf, EquipmentPluginDeploymentError> {
    installations
        .iter()
        .find_map(|(candidate, directory)| (*candidate == region).then_some(directory))
        .ok_or(EquipmentPluginDeploymentError::GameInstallationNotFound)
}

#[cfg(feature = "gui")]
fn read_registry_string(
    subkey: &str,
    value: &str,
) -> Result<Option<String>, EquipmentPluginDeploymentError> {
    let subkey = wide_null(subkey);
    let value = wide_null(value);
    let flags = RRF_RT_REG_SZ | RRF_SUBKEY_WOW6432KEY;
    let mut value_type = 0;
    let mut byte_len = 0;
    // SAFETY: both strings are NUL-terminated, output pointers are valid, and
    // the first call requests only the required byte count.
    let first = unsafe {
        RegGetValueW(
            HKEY_LOCAL_MACHINE,
            subkey.as_ptr(),
            value.as_ptr(),
            flags,
            &mut value_type,
            ptr::null_mut(),
            &mut byte_len,
        )
    };
    if first == ERROR_FILE_NOT_FOUND {
        return Ok(None);
    }
    if first != 0 {
        return Err(EquipmentPluginDeploymentError::Registry(format!(
            "registry query failed with error code {first}"
        )));
    }
    if value_type != REG_SZ || byte_len < 2 || byte_len % 2 != 0 {
        return Err(EquipmentPluginDeploymentError::Registry(
            "game install registry value has an invalid type or length".to_owned(),
        ));
    }
    let mut buffer = vec![0_u16; byte_len as usize / 2];
    // SAFETY: the buffer has the exact byte capacity reported by the first
    // query and the same NUL-terminated key and value names remain alive.
    let second = unsafe {
        RegGetValueW(
            HKEY_LOCAL_MACHINE,
            subkey.as_ptr(),
            value.as_ptr(),
            flags,
            &mut value_type,
            buffer.as_mut_ptr().cast(),
            &mut byte_len,
        )
    };
    if second != 0 {
        return Err(EquipmentPluginDeploymentError::Registry(format!(
            "registry value read failed with error code {second}"
        )));
    }
    if value_type != REG_SZ
        || byte_len < 2
        || byte_len % 2 != 0
        || byte_len as usize > buffer.len() * 2
    {
        return Err(EquipmentPluginDeploymentError::Registry(
            "game install registry value changed to an invalid type or length".to_owned(),
        ));
    }
    buffer.truncate(byte_len as usize / 2);
    if buffer.last() != Some(&0) {
        return Err(EquipmentPluginDeploymentError::Registry(
            "game install registry value is not a terminated string".to_owned(),
        ));
    }
    buffer.pop();
    String::from_utf16(&buffer).map(Some).map_err(|_| {
        EquipmentPluginDeploymentError::Registry(
            "game install registry value is not valid UTF-16".to_owned(),
        )
    })
}

#[cfg(feature = "gui")]
fn wide_null(value: &str) -> Vec<u16> {
    value.encode_utf16().chain([0]).collect()
}

#[cfg(feature = "gui")]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct PluginMarker {
    size: u64,
    fingerprint: u64,
}

#[cfg(feature = "gui")]
fn plugin_marker(plugin: &[u8]) -> PluginMarker {
    PluginMarker {
        size: plugin.len() as u64,
        fingerprint: fnv1a64(plugin),
    }
}

#[cfg(feature = "gui")]
fn fnv1a64(bytes: &[u8]) -> u64 {
    let mut hash = 0xcbf2_9ce4_8422_2325_u64;
    for byte in bytes {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    hash
}

#[cfg(feature = "gui")]
fn encode_plugin_marker(plugin: &[u8]) -> String {
    let marker = plugin_marker(plugin);
    format!(
        "{PLUGIN_MARKER_HEADER}\nsize={}\nfnv1a64={:016x}\n",
        marker.size, marker.fingerprint
    )
}

#[cfg(feature = "gui")]
fn parse_plugin_marker(text: &str) -> Option<PluginMarker> {
    let mut lines = text.lines();
    if lines.next()? != PLUGIN_MARKER_HEADER {
        return None;
    }
    let size = lines.next()?.strip_prefix("size=")?.parse().ok()?;
    let fingerprint = u64::from_str_radix(lines.next()?.strip_prefix("fnv1a64=")?, 16).ok()?;
    if lines.next().is_some() {
        return None;
    }
    Some(PluginMarker { size, fingerprint })
}

#[cfg(feature = "gui")]
fn inspect_game_installations(
    installations: &[(EquipmentPluginGameRegion, PathBuf)],
    current_plugin: Option<&[u8]>,
) -> Result<EquipmentPluginDeploymentStatus, EquipmentPluginDeploymentError> {
    let mut status = EquipmentPluginDeploymentStatus {
        source_available: current_plugin.is_some(),
        ..Default::default()
    };
    for (region, directory) in installations {
        let game_status =
            inspect_plugin_directories(std::slice::from_ref(directory), current_plugin)?;
        status.installations += 1;
        status.installed += game_status.installed;
        status.current += game_status.current;
        status.games.push(EquipmentPluginGameStatus {
            region: *region,
            installed: game_status.installed == 1,
            current: game_status.current == 1,
        });
    }
    Ok(status)
}

#[cfg(feature = "gui")]
fn inspect_plugin_directories(
    directories: &[PathBuf],
    current_plugin: Option<&[u8]>,
) -> Result<EquipmentPluginDeploymentStatus, EquipmentPluginDeploymentError> {
    let mut status = EquipmentPluginDeploymentStatus {
        installations: directories.len(),
        ..Default::default()
    };
    for directory in directories {
        let plugin_path = directory.join(PLUGIN_FILE_NAME);
        let marker_path = directory.join(PLUGIN_MARKER_FILE_NAME);
        if !marker_path.exists() {
            continue;
        }
        let marker = read_marker(&marker_path)?;
        let plugin = fs::read(&plugin_path)
            .map_err(|_| EquipmentPluginDeploymentError::InstalledPluginChanged)?;
        if plugin_marker(&plugin) != marker {
            return Err(EquipmentPluginDeploymentError::InstalledPluginChanged);
        }
        status.installed += 1;
        if current_plugin.is_some_and(|current| current == plugin) {
            status.current += 1;
        }
    }
    Ok(status)
}

#[cfg(feature = "gui")]
fn install_plugin_to_directories(
    directories: &[PathBuf],
    plugin: &[u8],
) -> Result<(), EquipmentPluginDeploymentError> {
    if plugin.is_empty() {
        return Err(EquipmentPluginDeploymentError::FileSystem(
            "equipment plugin file is empty".to_owned(),
        ));
    }
    for directory in directories {
        let plugin_path = directory.join(PLUGIN_FILE_NAME);
        let marker_path = directory.join(PLUGIN_MARKER_FILE_NAME);
        if !plugin_path.exists() {
            continue;
        }
        if marker_path.exists() {
            let marker = read_marker(&marker_path)?;
            let existing = fs::read(&plugin_path).map_err(file_system_error)?;
            if plugin_marker(&existing) != marker {
                return Err(EquipmentPluginDeploymentError::InstalledPluginChanged);
            }
        } else {
            let existing = fs::read(&plugin_path).map_err(file_system_error)?;
            if existing != plugin {
                return Err(EquipmentPluginDeploymentError::ConflictingDwmapi);
            }
        }
    }
    let marker = encode_plugin_marker(plugin);
    for directory in directories {
        let plugin_path = directory.join(PLUGIN_FILE_NAME);
        let marker_path = directory.join(PLUGIN_MARKER_FILE_NAME);
        fs::write(&plugin_path, plugin).map_err(file_system_error)?;
        if let Err(error) = fs::write(&marker_path, marker.as_bytes()) {
            let _ = fs::remove_file(plugin_path);
            return Err(file_system_error(error));
        }
    }
    Ok(())
}

#[cfg(feature = "gui")]
fn remove_plugin_from_directories(
    directories: &[PathBuf],
) -> Result<(), EquipmentPluginDeploymentError> {
    for directory in directories {
        let plugin_path = directory.join(PLUGIN_FILE_NAME);
        let marker_path = directory.join(PLUGIN_MARKER_FILE_NAME);
        if !marker_path.exists() {
            continue;
        }
        if plugin_path.exists() {
            let marker = read_marker(&marker_path)?;
            let plugin = fs::read(&plugin_path).map_err(file_system_error)?;
            if plugin_marker(&plugin) != marker {
                return Err(EquipmentPluginDeploymentError::InstalledPluginChanged);
            }
            fs::remove_file(&plugin_path).map_err(file_system_error)?;
        }
        fs::remove_file(&marker_path).map_err(file_system_error)?;
    }
    Ok(())
}

#[cfg(feature = "gui")]
fn read_marker(path: &Path) -> Result<PluginMarker, EquipmentPluginDeploymentError> {
    let text = fs::read_to_string(path).map_err(file_system_error)?;
    parse_plugin_marker(&text).ok_or(EquipmentPluginDeploymentError::InstalledPluginChanged)
}

#[cfg(feature = "gui")]
fn file_system_error(error: io::Error) -> EquipmentPluginDeploymentError {
    EquipmentPluginDeploymentError::FileSystem(error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn module_request_uses_the_stable_little_endian_wire_layout() {
        let bytes = encode_request(&EquipmentPluginRequest {
            request_id: 9,
            character: HtItemNetId { solt: 1, serial: 2 },
            operation: EquipmentPluginOperation::EquipModule {
                equipment: HtItemNetId { solt: 3, serial: 4 },
                row: 5,
                column: 4,
            },
        });
        assert_eq!(u16::from_le_bytes(bytes[6..8].try_into().unwrap()), 1);
        assert_eq!(u64::from_le_bytes(bytes[8..16].try_into().unwrap()), 9);
        assert_eq!(u32::from_le_bytes(bytes[16..20].try_into().unwrap()), 1);
        assert_eq!(u32::from_le_bytes(bytes[28..32].try_into().unwrap()), 4);
        assert_eq!(i32::from_le_bytes(bytes[40..44].try_into().unwrap()), 5);
        assert_eq!(i32::from_le_bytes(bytes[44..48].try_into().unwrap()), 4);
        assert!(bytes[48..].iter().all(|byte| *byte == 0));
    }

    #[test]
    fn one_key_request_encodes_native_rpc_placements() {
        let bytes = encode_request(&EquipmentPluginRequest {
            request_id: 11,
            character: HtItemNetId { solt: 1, serial: 2 },
            operation: EquipmentPluginOperation::EquipOneKey {
                placements: vec![EquipmentPluginPlacement {
                    equipment: HtItemNetId { solt: 3, serial: 4 },
                    row: 2,
                    column: 3,
                }],
                core: HtItemNetId { solt: 5, serial: 6 },
            },
        });
        assert_eq!(u16::from_le_bytes(bytes[6..8].try_into().unwrap()), 6);
        assert_eq!(u32::from_le_bytes(bytes[32..36].try_into().unwrap()), 5);
        assert_eq!(u32::from_le_bytes(bytes[36..40].try_into().unwrap()), 6);
        assert_eq!(u32::from_le_bytes(bytes[48..52].try_into().unwrap()), 1);
        assert_eq!(u32::from_le_bytes(bytes[56..60].try_into().unwrap()), 3);
        assert_eq!(i32::from_le_bytes(bytes[64..68].try_into().unwrap()), 2);
        assert_eq!(i32::from_le_bytes(bytes[68..72].try_into().unwrap()), 3);
    }

    #[test]
    fn new_v3_operations_encode_state_and_move_fields() {
        let moved = encode_request(&EquipmentPluginRequest {
            request_id: 12,
            character: HtItemNetId { solt: 1, serial: 2 },
            operation: EquipmentPluginOperation::MoveModuleToCharacter {
                equipment: HtItemNetId { solt: 3, serial: 4 },
                row: 2,
                column: 5,
            },
        });
        assert_eq!(u16::from_le_bytes(moved[4..6].try_into().unwrap()), 3);
        assert_eq!(u16::from_le_bytes(moved[6..8].try_into().unwrap()), 7);
        assert_eq!(u32::from_le_bytes(moved[16..20].try_into().unwrap()), 1);
        assert_eq!(u32::from_le_bytes(moved[24..28].try_into().unwrap()), 3);
        assert_eq!(i32::from_le_bytes(moved[40..44].try_into().unwrap()), 2);
        assert_eq!(i32::from_le_bytes(moved[44..48].try_into().unwrap()), 5);

        let moved_core = encode_request(&EquipmentPluginRequest {
            request_id: 13,
            character: HtItemNetId { solt: 7, serial: 8 },
            operation: EquipmentPluginOperation::MoveCoreToCharacter {
                equipment: HtItemNetId {
                    solt: 9,
                    serial: 10,
                },
            },
        });
        assert_eq!(u16::from_le_bytes(moved_core[6..8].try_into().unwrap()), 8);
        assert_eq!(
            u32::from_le_bytes(moved_core[16..20].try_into().unwrap()),
            7
        );
        assert_eq!(
            u32::from_le_bytes(moved_core[24..28].try_into().unwrap()),
            9
        );

        let discarded = encode_request(&EquipmentPluginRequest {
            request_id: 14,
            character: HtItemNetId::ZERO,
            operation: EquipmentPluginOperation::SetItemDiscarded {
                equipment: HtItemNetId {
                    solt: 11,
                    serial: 12,
                },
                discarded: true,
            },
        });
        assert_eq!(u16::from_le_bytes(discarded[6..8].try_into().unwrap()), 9);
        assert_eq!(u32::from_le_bytes(discarded[52..56].try_into().unwrap()), 1);

        let locked = encode_request(&EquipmentPluginRequest {
            request_id: 15,
            character: HtItemNetId::ZERO,
            operation: EquipmentPluginOperation::SetItemLocked {
                equipment: HtItemNetId { solt: 5, serial: 6 },
                locked: true,
            },
        });
        assert_eq!(u16::from_le_bytes(locked[6..8].try_into().unwrap()), 10);
        assert!(locked[16..24].iter().all(|byte| *byte == 0));
        assert_eq!(u32::from_le_bytes(locked[24..28].try_into().unwrap()), 5);
        assert_eq!(u32::from_le_bytes(locked[52..56].try_into().unwrap()), 1);
        assert!(locked[56..].iter().all(|byte| *byte == 0));
    }

    #[test]
    fn response_rejects_a_mismatched_request_id() {
        let mut bytes = [0_u8; RESPONSE_SIZE];
        bytes[0..4].copy_from_slice(&IPC_MAGIC.to_le_bytes());
        bytes[4..6].copy_from_slice(&IPC_VERSION.to_le_bytes());
        bytes[8..16].copy_from_slice(&10_u64.to_le_bytes());
        assert!(decode_response(&bytes, 9).is_err());
    }

    #[test]
    fn response_accepts_the_new_boolean_validation_status() {
        let mut bytes = [0_u8; RESPONSE_SIZE];
        bytes[0..4].copy_from_slice(&IPC_MAGIC.to_le_bytes());
        bytes[4..6].copy_from_slice(&IPC_VERSION.to_le_bytes());
        bytes[8..16].copy_from_slice(&9_u64.to_le_bytes());
        bytes[16..20].copy_from_slice(&12_u32.to_le_bytes());
        assert_eq!(decode_response(&bytes, 9), Ok(12));
    }

    #[test]
    fn client_bounds_requests_behind_the_active_pipe_call() {
        let (started_tx, started_rx) = bounded(1);
        let (release_tx, release_rx) = bounded(1);
        let call_count = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let worker_call_count = Arc::clone(&call_count);
        let mut client = EquipmentPluginClient::with_call_for_test(move |_| {
            if worker_call_count.fetch_add(1, Ordering::AcqRel) == 0 {
                started_tx.send(()).unwrap();
                release_rx.recv().unwrap();
            }
            Ok(0)
        });

        assert_eq!(
            client.submit(
                HtItemNetId { solt: 1, serial: 2 },
                EquipmentPluginOperation::UnequipAll
            ),
            Ok(1)
        );
        started_rx
            .recv_timeout(std::time::Duration::from_secs(1))
            .unwrap();
        assert_eq!(
            client.submit(
                HtItemNetId { solt: 3, serial: 4 },
                EquipmentPluginOperation::UnequipAll
            ),
            Ok(2)
        );
        assert_eq!(
            client.submit(
                HtItemNetId { solt: 5, serial: 6 },
                EquipmentPluginOperation::UnequipAll
            ),
            Err(EquipmentPluginSubmitError::Busy)
        );

        release_tx.send(()).unwrap();
        assert_eq!(
            client
                .receiver
                .recv_timeout(std::time::Duration::from_secs(1))
                .unwrap()
                .request_id,
            1
        );
    }

    #[cfg(feature = "gui")]
    fn deployment_test_directory(name: &str) -> PathBuf {
        let unique = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::env::temp_dir().join(format!(
            "nte-equipment-plugin-{name}-{}-{unique}",
            std::process::id()
        ));
        fs::create_dir_all(&path).unwrap();
        path
    }

    #[test]
    #[cfg(feature = "gui")]
    fn deployment_marks_installs_and_removes_only_the_managed_plugin() {
        let directory = deployment_test_directory("lifecycle");
        let directories = vec![directory.clone()];
        let plugin = b"test equipment plugin";

        install_plugin_to_directories(&directories, plugin).unwrap();
        assert_eq!(
            inspect_plugin_directories(&directories, Some(plugin)).unwrap(),
            EquipmentPluginDeploymentStatus {
                installations: 1,
                installed: 1,
                current: 1,
                source_available: false,
                games: Vec::new(),
            }
        );
        remove_plugin_from_directories(&directories).unwrap();
        assert_eq!(
            inspect_plugin_directories(&directories, Some(plugin)).unwrap(),
            EquipmentPluginDeploymentStatus {
                installations: 1,
                installed: 0,
                current: 0,
                source_available: false,
                games: Vec::new(),
            }
        );
        assert!(!directory.join(PLUGIN_FILE_NAME).exists());
        assert!(!directory.join(PLUGIN_MARKER_FILE_NAME).exists());

        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    #[cfg(feature = "gui")]
    fn deployment_preserves_an_unmanaged_dwmapi_proxy() {
        let directory = deployment_test_directory("conflict");
        fs::write(directory.join(PLUGIN_FILE_NAME), b"another mod").unwrap();

        assert_eq!(
            install_plugin_to_directories(std::slice::from_ref(&directory), b"our plugin"),
            Err(EquipmentPluginDeploymentError::ConflictingDwmapi)
        );
        assert_eq!(
            fs::read(directory.join(PLUGIN_FILE_NAME)).unwrap(),
            b"another mod"
        );

        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    #[cfg(feature = "gui")]
    fn selected_client_install_ignores_another_clients_unmanaged_proxy() {
        let china = deployment_test_directory("selected-china");
        let global = deployment_test_directory("selected-global");
        fs::write(global.join(PLUGIN_FILE_NAME), b"another mod").unwrap();
        let installations = vec![
            (EquipmentPluginGameRegion::China, china.clone()),
            (EquipmentPluginGameRegion::Global, global.clone()),
        ];
        let selected =
            selected_game_directory(&installations, EquipmentPluginGameRegion::China).unwrap();

        install_plugin_to_directories(std::slice::from_ref(selected), b"our plugin").unwrap();
        let status = inspect_game_installations(&installations, Some(b"our plugin")).unwrap();

        assert!(status.source_available);
        assert_eq!(
            status.games,
            vec![
                EquipmentPluginGameStatus {
                    region: EquipmentPluginGameRegion::China,
                    installed: true,
                    current: true,
                },
                EquipmentPluginGameStatus {
                    region: EquipmentPluginGameRegion::Global,
                    installed: false,
                    current: false,
                },
            ]
        );
        assert_eq!(
            fs::read(global.join(PLUGIN_FILE_NAME)).unwrap(),
            b"another mod"
        );

        fs::remove_dir_all(china).unwrap();
        fs::remove_dir_all(global).unwrap();
    }

    #[test]
    #[cfg(feature = "gui")]
    fn deployment_rejects_a_managed_plugin_changed_outside_the_tool() {
        let directory = deployment_test_directory("changed");
        let directories = vec![directory.clone()];
        install_plugin_to_directories(&directories, b"original plugin").unwrap();
        fs::write(directory.join(PLUGIN_FILE_NAME), b"changed plugin").unwrap();

        assert_eq!(
            remove_plugin_from_directories(&directories),
            Err(EquipmentPluginDeploymentError::InstalledPluginChanged)
        );
        assert!(directory.join(PLUGIN_FILE_NAME).exists());

        fs::remove_dir_all(directory).unwrap();
    }
}
