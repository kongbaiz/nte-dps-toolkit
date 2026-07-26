//! Asynchronous client for the in-process NTE Mods Plugin's local named pipe.
//! The blocking pipe transaction stays on a worker thread; the UI only submits
//! validated session item IDs and polls completed responses.

use std::io;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::thread::{self, JoinHandle};

use crossbeam_channel::{Receiver, Sender, TryRecvError, TrySendError, bounded, unbounded};
#[cfg(feature = "gui")]
use std::collections::BTreeMap;
#[cfg(feature = "gui")]
use std::fmt;
#[cfg(feature = "gui")]
use std::fs;
#[cfg(feature = "gui")]
use std::io::Write;
#[cfg(feature = "gui")]
use std::os::windows::ffi::OsStrExt;
#[cfg(feature = "gui")]
use std::path::{Path, PathBuf};
#[cfg(feature = "gui")]
use std::ptr;
#[cfg(feature = "gui")]
use windows_sys::Win32::Foundation::{ERROR_FILE_NOT_FOUND, ERROR_SUCCESS};
#[cfg(feature = "gui")]
use windows_sys::Win32::Storage::FileSystem::{
    MOVEFILE_REPLACE_EXISTING, MOVEFILE_WRITE_THROUGH, MoveFileExW,
};
use windows_sys::Win32::System::Pipes::CallNamedPipeW;
#[cfg(feature = "gui")]
use windows_sys::Win32::System::Registry::{
    HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE, REG_SZ, RRF_RT_REG_SZ, RRF_SUBKEY_WOW6432KEY,
    RegCloseKey, RegCreateKeyW, RegGetValueW, RegSetValueExW,
};

use crate::engine::model::HtItemNetId;
#[cfg(feature = "gui")]
use crate::storage::mod_scripts::{
    mod_script_workspace_directory, validate_enabled_mod_set, validate_mod_source,
};

const PIPE_NAME: &str = r"\\.\pipe\nte-mods-plugin-v7";
const IPC_MAGIC: u32 = 0x5145_544e;
const IPC_VERSION: u16 = 7;
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
const IPC_QUERY_COMBAT_CLOCK_TRANSITIONS: u16 = 11;
const IPC_QUERY_MOD_EVENTS: u16 = 12;
const IPC_TIMEOUT_MS: u32 = 1_500;
const MAX_PLACEMENTS: usize = 64;
const REQUEST_HEADER_SIZE: usize = 56;
const PLACEMENT_SIZE: usize = 16;
const REQUEST_SIZE: usize = REQUEST_HEADER_SIZE + MAX_PLACEMENTS * PLACEMENT_SIZE;
const RESPONSE_HEADER_SIZE: usize = 24;
const COMBAT_CLOCK_TRANSITION_SIZE: usize = 32;
const COMBAT_CLOCK_HISTORY_SIZE: usize = 64;
const MOD_EVENT_SIZE: usize = 112;
const MOD_EVENT_HISTORY_SIZE: usize = 18;
const MOD_EVENT_ID_SIZE: usize = 32;
const MOD_EVENT_NAME_SIZE: usize = 32;
const MOD_EVENT_VALUE_COUNT: usize = 3;
const RESPONSE_SIZE: usize =
    RESPONSE_HEADER_SIZE + COMBAT_CLOCK_HISTORY_SIZE * COMBAT_CLOCK_TRANSITION_SIZE;
const COMBAT_CLOCK_PAUSE_VALID: u32 = 0x1;
const MAX_PLUGIN_STATUS: u32 = 13;
const PLUGIN_STATUS_DRY_RUN_OK: u32 = 1;
const PLUGIN_STATUS_MOD_DISABLED: u32 = 13;
static COMBAT_CLOCK_QUERY_SEQUENCE: AtomicU64 = AtomicU64::new(1);
static MOD_EVENT_QUERY_SEQUENCE: AtomicU64 = AtomicU64::new(1);
#[cfg(feature = "gui")]
static PLUGIN_TEMP_SEQUENCE: AtomicU64 = AtomicU64::new(1);

#[cfg(feature = "gui")]
const GAME_INSTALL_REGISTRY_KEYS: [(ModsPluginGameRegion, &str); 2] = [
    (
        ModsPluginGameRegion::China,
        r"SOFTWARE\Microsoft\Windows\CurrentVersion\Uninstall\YH",
    ),
    (
        ModsPluginGameRegion::Global,
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
const LEGACY_PLUGIN_MARKER_FILE_NAME: &str = ".nte-dps-tool-equipment-plugin";
#[cfg(feature = "gui")]
const LEGACY_PLUGIN_MARKER_HEADER: &str = "NTE_DPS_TOOL_EQUIPMENT_PLUGIN_V1";
#[cfg(feature = "gui")]
const PLUGIN_BINARY_SIGNATURE: &[u8] = b"NTE_DPS_TOOL_MODS_PLUGIN_V1";
#[cfg(feature = "gui")]
const LEGACY_PLUGIN_BINARY_SIGNATURE: &[u8] = b"NTE_DPS_TOOL_MOD_LOADER_V1";
#[cfg(feature = "gui")]
const MOD_WORKSPACE_REGISTRY_KEY: &str = r"Software\NTE DPS Tool\Mods Plugin";
#[cfg(feature = "gui")]
const MOD_WORKSPACE_REGISTRY_VALUE: &str = "Workspace";
#[cfg(feature = "gui")]
const MAX_LEGACY_MOD_FILE_BYTES: usize = 16 * 1024;
#[cfg(feature = "gui")]
const MOD_SET_FILE_NAME: &str = "nte-mods.enabled";
#[cfg(feature = "gui")]
const MOD_DIRECTORY_NAME: &str = "nte-mods";
#[cfg(feature = "gui")]
const EQUIPMENT_MOD_FILE_NAME: &str = "equipment.nte";
#[cfg(feature = "gui")]
const COMBAT_CLOCK_MOD_FILE_NAME: &str = "combat-clock.nte";
#[cfg(feature = "gui")]
const DEFAULT_MOD_SET: &[u8] = include_bytes!("../../plugins/nte-mods.enabled");
#[cfg(feature = "gui")]
const EQUIPMENT_MOD: &[u8] = include_bytes!("../../plugins/nte-mods/equipment.nte");
#[cfg(feature = "gui")]
const COMBAT_CLOCK_MOD: &[u8] = include_bytes!("../../plugins/nte-mods/combat-clock.nte");
#[cfg(feature = "gui")]
const LEGACY_EQUIPMENT_MOD_V1: &[u8] =
    b"nte_mod 1\nmod equipment\non viewport_tick equipment.prepare\non viewport_tick ipc.pump\n";
#[cfg(feature = "gui")]
const LEGACY_COMBAT_CLOCK_MOD_V1: &[u8] =
    b"nte_mod 1\nmod combat-clock\non viewport_tick combat_clock.observe\non viewport_tick ipc.pump\n";
#[cfg(feature = "gui")]
const LEGACY_EQUIPMENT_MOD_V2: &[u8] = b"nte_mod 2\nmod equipment\ncapability equipment\n\non viewport_tick\nload r0 event.viewport\nread_ptr r1 r0 0x80\nread_tarray_first r2 r1 0x38\nread_ptr r3 r2 0x30\nread_ptr r4 r3 0x2d0\nif equipment.cache_missing\ncall equipment.prepare r4\nend\ncall ipc.pump r4 null\nend\n";
#[cfg(feature = "gui")]
const LEGACY_COMBAT_CLOCK_MOD_V2: &[u8] = b"nte_mod 2\nmod combat-clock\ncapability combat-clock\n\non viewport_tick\nload r0 event.viewport\nread_ptr r1 r0 0x80\nread_tarray_first r2 r1 0x38\nread_ptr r3 r2 0x30\ncall combat_clock.observe r3\ncall ipc.pump null r3\nend\n";
#[cfg(feature = "gui")]
const LEGACY_EQUIPMENT_MOD_V3: &[u8] = b"nte_mod(3)\nmod(\"equipment\")\ncapability(\"equipment\")\n\ndef on_viewport_tick(event):\n    viewport = event.viewport\n    game_instance = read_ptr(viewport, 0x80)\n    local_player = read_tarray_first(game_instance, 0x38)\n    player_controller = read_ptr(local_player, 0x30)\n    player_state = read_ptr(player_controller, 0x2d0)\n    if equipment.cache_missing():\n        equipment.prepare(player_state)\n    ipc.pump(player_state, None)\n";
#[cfg(feature = "gui")]
const LEGACY_COMBAT_CLOCK_MOD_V3: &[u8] = b"nte_mod(3)\nmod(\"combat-clock\")\ncapability(\"combat-clock\")\n\ndef on_viewport_tick(event):\n    viewport = event.viewport\n    game_instance = read_ptr(viewport, 0x80)\n    local_player = read_tarray_first(game_instance, 0x38)\n    player_controller = read_ptr(local_player, 0x30)\n    combat_clock.observe(player_controller)\n    ipc.pump(None, player_controller)\n";
#[cfg(feature = "gui")]
const LEGACY_EQUIPMENT_MOD_V4: &[u8] = b"nte_mod(4)\nmod(\"equipment\")\nrequires(\"viewport.tick\")\nrequires(\"memory.read\")\nrequires(\"sdk.read\")\nrequires(\"equipment\")\nrequires(\"ipc\")\n\ndef on_viewport_tick(event):\n    viewport = event.viewport\n    game_instance = memory.read_ptr(viewport, 0x80)\n    local_player = memory.tarray_first(game_instance, 0x38)\n    player_controller = memory.read_ptr(local_player, 0x30)\n    player_state = sdk.player_state(player_controller)\n    if equipment.cache_missing():\n        equipment.prepare(player_state)\n    ipc.bind(player_state, None)\n";
#[cfg(feature = "gui")]
const LEGACY_COMBAT_CLOCK_MOD_V4: &[u8] = b"nte_mod(4)\nmod(\"combat-clock\")\nrequires(\"viewport.tick\")\nrequires(\"memory.read\")\nrequires(\"combat-clock\")\nrequires(\"ipc\")\n\ndef on_viewport_tick(event):\n    viewport = event.viewport\n    game_instance = memory.read_ptr(viewport, 0x80)\n    local_player = memory.tarray_first(game_instance, 0x38)\n    player_controller = memory.read_ptr(local_player, 0x30)\n    combat_clock.observe(player_controller)\n    ipc.bind(None, player_controller)\n";
#[cfg(feature = "gui")]
const LEGACY_EQUIPMENT_MOD_V4_OFFSETS: &[u8] = br#"nte_mod(4)
mod("equipment")
requires("viewport.tick")
requires("memory.read")
requires("sdk.read")
requires("equipment")
requires("ipc")

# The script owns session discovery, cache retry, and IPC activation.
# Host APIs only perform checked reads or one bounded native operation.
state.last_player_state = 0
state.next_prepare_at = 0

def on_viewport_tick(event):
    # Resolve the current local session. A failed step returns None.
    game_instance = memory.read_ptr(event.viewport, 0x80)
    local_player = memory.tarray_first(game_instance, 0x38)
    player_controller = memory.read_ptr(local_player, 0x30)
    player_state = sdk.player_state(player_controller)
    now = time.now_ms()

    if player_state != None:
        # A new PlayerState starts a fresh cache lifecycle.
        if player_state != state.last_player_state:
            state.last_player_state = player_state
            state.next_prepare_at = 0

        cache_ready = equipment.cache_ready(player_state)
        if cache_ready == False:
            # Retry at most once per second while UE functions are unavailable.
            if now >= state.next_prepare_at:
                equipment.prepare(player_state)
                state.next_prepare_at = now + 1000
                cache_ready = equipment.cache_ready(player_state)

        # Equipment IPC becomes actionable only after this Mod prepared it.
        if cache_ready == True:
            ipc.bind(player_state, None)
"#;
#[cfg(feature = "gui")]
const LEGACY_COMBAT_CLOCK_MOD_V4_OFFSETS: &[u8] = br#"nte_mod(4)
mod("combat-clock")
requires("viewport.tick")
requires("memory.read")
requires("combat-clock")
requires("ipc")

# The script owns sampling, transition detection, and forwarding.
# sample() packs state_flags in the high 32 bits and pause_mask below.
state.initialized = 0
state.last_controller = 0
state.last_pause_mask = 0
state.last_state_flags = 0

def on_viewport_tick(event):
    # Resolve the controller independently from the equipment Mod.
    game_instance = memory.read_ptr(event.viewport, 0x80)
    local_player = memory.tarray_first(game_instance, 0x38)
    player_controller = memory.read_ptr(local_player, 0x30)

    # A zero sample represents an unavailable controller or pause API.
    sample = 0
    if player_controller != None:
        sample = combat_clock.sample(player_controller)
        ipc.bind(None, player_controller)
    pause_mask = sample & 0xFFFFFFFF
    state_flags = sample >> 32

    # Forward only the first sample or a real state transition.
    changed = state.initialized == False
    if player_controller != state.last_controller:
        changed = True
    if pause_mask != state.last_pause_mask:
        changed = True
    if state_flags != state.last_state_flags:
        changed = True

    if changed == True:
        combat_clock.forward(pause_mask, state_flags)
        state.initialized = 1
        state.last_controller = player_controller
        state.last_pause_mask = pause_mask
        state.last_state_flags = state_flags
"#;
#[cfg(feature = "gui")]
const NO_LEGACY_MOD_PROGRAMS: &[&[u8]] = &[];
#[cfg(feature = "gui")]
const LEGACY_EQUIPMENT_MOD_PROGRAMS: &[&[u8]] = &[
    LEGACY_EQUIPMENT_MOD_V1,
    LEGACY_EQUIPMENT_MOD_V2,
    LEGACY_EQUIPMENT_MOD_V3,
    LEGACY_EQUIPMENT_MOD_V4,
    LEGACY_EQUIPMENT_MOD_V4_OFFSETS,
];
#[cfg(feature = "gui")]
const LEGACY_COMBAT_CLOCK_MOD_PROGRAMS: &[&[u8]] = &[
    LEGACY_COMBAT_CLOCK_MOD_V1,
    LEGACY_COMBAT_CLOCK_MOD_V2,
    LEGACY_COMBAT_CLOCK_MOD_V3,
    LEGACY_COMBAT_CLOCK_MOD_V4,
    LEGACY_COMBAT_CLOCK_MOD_V4_OFFSETS,
];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ModsPluginPlacement {
    pub equipment: HtItemNetId,
    pub row: i32,
    pub column: i32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ModsPluginOperation {
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
        placements: Vec<ModsPluginPlacement>,
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
pub struct ModsPluginRequest {
    pub request_id: u64,
    pub character: HtItemNetId,
    pub operation: ModsPluginOperation,
}

#[derive(Debug, PartialEq, Eq)]
pub struct ModsPluginResponse {
    pub request_id: u64,
    pub status: Result<u32, String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct CombatClockTransitionSnapshot {
    pub sequence: u64,
    pub timestamp_100ns: u64,
    pub pause_type_mask: u32,
    pub reserved_value: i32,
    pub state_flags: u32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ModEventSnapshot {
    pub sequence: u64,
    pub timestamp_100ns: u64,
    pub mod_id: String,
    pub name: String,
    pub values: Vec<u64>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ModsPluginSubmitError {
    Busy,
}

#[cfg(feature = "gui")]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ModsPluginGameRegion {
    China,
    Global,
}

#[cfg(feature = "gui")]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ModsPluginGameStatus {
    pub region: ModsPluginGameRegion,
    pub installed: bool,
    pub current: bool,
}

#[cfg(feature = "gui")]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ModsPluginModTarget {
    pub region: ModsPluginGameRegion,
    pub directory: PathBuf,
    pub installed: bool,
}

#[cfg(feature = "gui")]
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ModsPluginDeploymentStatus {
    pub installations: usize,
    pub installed: usize,
    pub current: usize,
    pub source_available: bool,
    pub games: Vec<ModsPluginGameStatus>,
}

#[cfg(feature = "gui")]
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ModsPluginDeploymentError {
    GameRunning,
    GameProcessProbe(String),
    GameInstallationNotFound,
    Registry(String),
    PluginSourceNotFound,
    ConflictingDwmapi,
    InstalledPluginChanged,
    FileSystem(String),
}

#[cfg(feature = "gui")]
impl fmt::Display for ModsPluginDeploymentError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::GameRunning => formatter.write_str("HTGame.exe is running"),
            Self::GameProcessProbe(error) => {
                write!(formatter, "game process detection failed: {error}")
            }
            Self::GameInstallationNotFound => {
                formatter.write_str("game installation was not found")
            }
            Self::Registry(error) => write!(formatter, "game registry lookup failed: {error}"),
            Self::PluginSourceNotFound => formatter.write_str("Mod loader source was not found"),
            Self::ConflictingDwmapi => {
                formatter.write_str("game directory contains an unmanaged dwmapi.dll")
            }
            Self::InstalledPluginChanged => {
                formatter.write_str("installed Mod loader changed outside this tool")
            }
            Self::FileSystem(error) => write!(formatter, "Mod loader file failed: {error}"),
        }
    }
}

enum WorkerCommand {
    Request(ModsPluginRequest),
    Stop,
}

pub struct ModsPluginClient {
    sender: Sender<WorkerCommand>,
    receiver: Receiver<ModsPluginResponse>,
    thread: Option<JoinHandle<()>>,
    stop: Arc<AtomicBool>,
    next_request_id: u64,
}

impl Default for ModsPluginClient {
    fn default() -> Self {
        Self::new()
    }
}

impl ModsPluginClient {
    pub fn new() -> Self {
        Self::with_call(call_plugin)
    }

    fn with_call<F>(call: F) -> Self
    where
        F: Fn(&ModsPluginRequest) -> Result<u32, String> + Send + 'static,
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
                            .send(ModsPluginResponse {
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
        operation: ModsPluginOperation,
    ) -> Result<u64, ModsPluginSubmitError> {
        let request_id = self.next_request_id;
        self.next_request_id = self.next_request_id.wrapping_add(1).max(1);
        self.submit_request(ModsPluginRequest {
            request_id,
            character,
            operation,
        })?;
        Ok(request_id)
    }

    pub fn submit_request(&self, request: ModsPluginRequest) -> Result<(), ModsPluginSubmitError> {
        match self.sender.try_send(WorkerCommand::Request(request)) {
            Ok(()) => Ok(()),
            Err(TrySendError::Full(_)) => Err(ModsPluginSubmitError::Busy),
            Err(TrySendError::Disconnected(_)) => {
                panic!("Mod loader worker must remain alive while its client exists")
            }
        }
    }

    pub fn response_receiver(&self) -> Receiver<ModsPluginResponse> {
        self.receiver.clone()
    }

    pub fn try_recv(&self) -> Option<ModsPluginResponse> {
        match self.receiver.try_recv() {
            Ok(response) => Some(response),
            Err(TryRecvError::Empty) => None,
            Err(TryRecvError::Disconnected) => {
                panic!("Mod loader worker disconnected before its client was dropped")
            }
        }
    }

    #[cfg(test)]
    pub(crate) fn with_call_for_test<F>(call: F) -> Self
    where
        F: Fn(&ModsPluginRequest) -> Result<u32, String> + Send + 'static,
    {
        Self::with_call(call)
    }
}

impl Drop for ModsPluginClient {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        let _ = self.sender.try_send(WorkerCommand::Stop);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

pub(crate) fn call_plugin(request: &ModsPluginRequest) -> Result<u32, String> {
    let request_bytes = encode_request(request);
    let response = call_plugin_request(&request_bytes)?;
    decode_response(&response, request.request_id)
}

pub(crate) fn query_combat_clock_transitions() -> Result<Vec<CombatClockTransitionSnapshot>, String>
{
    let request_id = COMBAT_CLOCK_QUERY_SEQUENCE
        .fetch_add(1, Ordering::Relaxed)
        .max(1);
    let mut request = [0_u8; REQUEST_SIZE];
    request[0..4].copy_from_slice(&IPC_MAGIC.to_le_bytes());
    request[4..6].copy_from_slice(&IPC_VERSION.to_le_bytes());
    request[6..8].copy_from_slice(&IPC_QUERY_COMBAT_CLOCK_TRANSITIONS.to_le_bytes());
    request[8..16].copy_from_slice(&request_id.to_le_bytes());
    let response = call_plugin_request(&request)?;
    decode_combat_clock_transitions(&response, request_id)
}

pub fn query_mod_events() -> Result<Vec<ModEventSnapshot>, String> {
    let request_id = MOD_EVENT_QUERY_SEQUENCE
        .fetch_add(1, Ordering::Relaxed)
        .max(1);
    let mut request = [0_u8; REQUEST_SIZE];
    request[0..4].copy_from_slice(&IPC_MAGIC.to_le_bytes());
    request[4..6].copy_from_slice(&IPC_VERSION.to_le_bytes());
    request[6..8].copy_from_slice(&IPC_QUERY_MOD_EVENTS.to_le_bytes());
    request[8..16].copy_from_slice(&request_id.to_le_bytes());
    let response = call_plugin_request(&request)?;
    decode_mod_events(&response, request_id)
}

fn call_plugin_request(request: &[u8; REQUEST_SIZE]) -> Result<[u8; RESPONSE_SIZE], String> {
    let mut response = [0_u8; RESPONSE_SIZE];
    let mut bytes_read = 0;
    let mut pipe_name = PIPE_NAME.encode_utf16().collect::<Vec<_>>();
    pipe_name.push(0);

    // SAFETY: both buffers live for the duration of the synchronous call, their
    // exact lengths are passed to Win32, and the pipe name is NUL-terminated.
    let succeeded = unsafe {
        CallNamedPipeW(
            pipe_name.as_ptr(),
            request.as_ptr().cast(),
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
            "Mod loader returned {bytes_read} bytes; expected {RESPONSE_SIZE}"
        ));
    }
    Ok(response)
}

fn encode_request(request: &ModsPluginRequest) -> [u8; REQUEST_SIZE] {
    let (operation, equipment, core, row, column, state, placements) = match &request.operation {
        ModsPluginOperation::EquipModule {
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
        ModsPluginOperation::EquipCore { equipment } => (
            IPC_EQUIP_CORE,
            *equipment,
            HtItemNetId::ZERO,
            0,
            0,
            0,
            &[][..],
        ),
        ModsPluginOperation::UnequipModule { equipment } => (
            IPC_UNEQUIP_MODULE,
            *equipment,
            HtItemNetId::ZERO,
            0,
            0,
            0,
            &[][..],
        ),
        ModsPluginOperation::UnequipCore { equipment } => (
            IPC_UNEQUIP_CORE,
            *equipment,
            HtItemNetId::ZERO,
            0,
            0,
            0,
            &[][..],
        ),
        ModsPluginOperation::UnequipAll => (
            IPC_UNEQUIP_ALL,
            HtItemNetId::ZERO,
            HtItemNetId::ZERO,
            0,
            0,
            0,
            &[][..],
        ),
        ModsPluginOperation::EquipOneKey { placements, core } => {
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
        ModsPluginOperation::MoveModuleToCharacter {
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
        ModsPluginOperation::MoveCoreToCharacter { equipment } => (
            IPC_MOVE_CORE_TO_CHARACTER,
            *equipment,
            HtItemNetId::ZERO,
            0,
            0,
            0,
            &[][..],
        ),
        ModsPluginOperation::SetItemDiscarded {
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
        ModsPluginOperation::SetItemLocked { equipment, locked } => (
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
    let (status, record_count) = decode_response_header(bytes, request_id)?;
    if record_count != 0 {
        return Err(
            "Mod loader returned combat clock transitions for an equipment request".to_owned(),
        );
    }
    Ok(status)
}

fn decode_response_header(
    bytes: &[u8; RESPONSE_SIZE],
    request_id: u64,
) -> Result<(u32, u32), String> {
    let magic = u32::from_le_bytes(bytes[0..4].try_into().expect("fixed response magic"));
    let version = u16::from_le_bytes(bytes[4..6].try_into().expect("fixed response version"));
    let reserved = u16::from_le_bytes(bytes[6..8].try_into().expect("fixed response reserved"));
    let response_id =
        u64::from_le_bytes(bytes[8..16].try_into().expect("fixed response request id"));
    let status = u32::from_le_bytes(bytes[16..20].try_into().expect("fixed response status"));
    let record_count = u32::from_le_bytes(bytes[20..24].try_into().expect("fixed record count"));
    if magic != IPC_MAGIC
        || version != IPC_VERSION
        || reserved != 0
        || response_id != request_id
        || status > MAX_PLUGIN_STATUS
    {
        return Err("Mod loader returned an invalid IPC response".to_owned());
    }
    Ok((status, record_count))
}

fn decode_combat_clock_transitions(
    bytes: &[u8; RESPONSE_SIZE],
    request_id: u64,
) -> Result<Vec<CombatClockTransitionSnapshot>, String> {
    let (status, transition_count) = decode_response_header(bytes, request_id)?;
    if status == PLUGIN_STATUS_MOD_DISABLED {
        return Err("combat clock mod is disabled".to_owned());
    }
    if status != PLUGIN_STATUS_DRY_RUN_OK || transition_count as usize > COMBAT_CLOCK_HISTORY_SIZE {
        return Err("Mod loader returned invalid combat clock history".to_owned());
    }

    let mut transitions = Vec::with_capacity(transition_count as usize);
    for index in 0..transition_count as usize {
        let offset = RESPONSE_HEADER_SIZE + index * COMBAT_CLOCK_TRANSITION_SIZE;
        let state_flags = u32::from_le_bytes(
            bytes[offset + 24..offset + 28]
                .try_into()
                .expect("fixed combat clock flags"),
        );
        let reserved = u32::from_le_bytes(
            bytes[offset + 28..offset + 32]
                .try_into()
                .expect("fixed transition reserved"),
        );
        let pause_type_mask = u32::from_le_bytes(
            bytes[offset + 16..offset + 20]
                .try_into()
                .expect("fixed pause type mask"),
        );
        let reserved_value = i32::from_le_bytes(
            bytes[offset + 20..offset + 24]
                .try_into()
                .expect("fixed transition reserved value"),
        );
        if reserved != 0
            || reserved_value != 0
            || state_flags & !COMBAT_CLOCK_PAUSE_VALID != 0
            || pause_type_mask & !0x1c != 0
            || state_flags & COMBAT_CLOCK_PAUSE_VALID == 0 && pause_type_mask != 0
        {
            return Err("Mod loader returned invalid combat clock history".to_owned());
        }
        transitions.push(CombatClockTransitionSnapshot {
            sequence: u64::from_le_bytes(
                bytes[offset..offset + 8]
                    .try_into()
                    .expect("fixed transition sequence"),
            ),
            timestamp_100ns: u64::from_le_bytes(
                bytes[offset + 8..offset + 16]
                    .try_into()
                    .expect("fixed transition timestamp"),
            ),
            pause_type_mask,
            reserved_value,
            state_flags,
        });
    }
    Ok(transitions)
}

fn decode_fixed_ascii(bytes: &[u8], field: &str) -> Result<String, String> {
    let end = bytes
        .iter()
        .position(|value| *value == 0)
        .unwrap_or(bytes.len());
    if bytes[end..].iter().any(|value| *value != 0)
        || bytes[..end].iter().any(|value| {
            !value.is_ascii_lowercase()
                && !value.is_ascii_digit()
                && !matches!(*value, b'-' | b'_' | b'.')
        })
        || end == 0
    {
        return Err(format!("Mod loader returned an invalid mod event {field}"));
    }
    String::from_utf8(bytes[..end].to_vec())
        .map_err(|_| format!("Mod loader returned an invalid mod event {field}"))
}

fn decode_mod_events(
    bytes: &[u8; RESPONSE_SIZE],
    request_id: u64,
) -> Result<Vec<ModEventSnapshot>, String> {
    let (status, event_count) = decode_response_header(bytes, request_id)?;
    if status == PLUGIN_STATUS_MOD_DISABLED {
        return Err("mod event IPC is disabled".to_owned());
    }
    if status != PLUGIN_STATUS_DRY_RUN_OK || event_count as usize > MOD_EVENT_HISTORY_SIZE {
        return Err("Mod loader returned invalid mod event history".to_owned());
    }

    let mut events = Vec::with_capacity(event_count as usize);
    for index in 0..event_count as usize {
        let offset = RESPONSE_HEADER_SIZE + index * MOD_EVENT_SIZE;
        let value_count = u32::from_le_bytes(
            bytes[offset + 80..offset + 84]
                .try_into()
                .expect("fixed mod event value count"),
        ) as usize;
        let reserved = u32::from_le_bytes(
            bytes[offset + 84..offset + 88]
                .try_into()
                .expect("fixed mod event reserved"),
        );
        if value_count > MOD_EVENT_VALUE_COUNT || reserved != 0 {
            return Err("Mod loader returned invalid mod event history".to_owned());
        }
        let mut values = Vec::with_capacity(value_count);
        for value_index in 0..MOD_EVENT_VALUE_COUNT {
            let value_offset = offset + 88 + value_index * 8;
            let value = u64::from_le_bytes(
                bytes[value_offset..value_offset + 8]
                    .try_into()
                    .expect("fixed mod event value"),
            );
            if value_index < value_count {
                values.push(value);
            } else if value != 0 {
                return Err("Mod loader returned invalid mod event history".to_owned());
            }
        }
        events.push(ModEventSnapshot {
            sequence: u64::from_le_bytes(
                bytes[offset..offset + 8]
                    .try_into()
                    .expect("fixed mod event sequence"),
            ),
            timestamp_100ns: u64::from_le_bytes(
                bytes[offset + 8..offset + 16]
                    .try_into()
                    .expect("fixed mod event timestamp"),
            ),
            mod_id: decode_fixed_ascii(
                &bytes[offset + 16..offset + 16 + MOD_EVENT_ID_SIZE],
                "mod id",
            )?,
            name: decode_fixed_ascii(
                &bytes[offset + 48..offset + 48 + MOD_EVENT_NAME_SIZE],
                "name",
            )?,
            values,
        });
    }
    Ok(events)
}

#[cfg(feature = "gui")]
pub fn inspect_plugin_deployment(
    current_plugin: Option<&[u8]>,
) -> Result<ModsPluginDeploymentStatus, ModsPluginDeploymentError> {
    prepare_mod_workspace()?;
    let installations = game_installation_directories()?;
    inspect_game_installations(&installations, current_plugin)
}

#[cfg(feature = "gui")]
pub fn mods_plugin_mod_targets() -> Result<Vec<ModsPluginModTarget>, ModsPluginDeploymentError> {
    let workspace = prepare_mod_workspace()?;
    let installations = game_installation_directories()?;
    inspect_mod_targets(&installations, &workspace)
}

#[cfg(feature = "gui")]
pub fn install_mods_plugin(
    region: ModsPluginGameRegion,
    plugin: &[u8],
) -> Result<ModsPluginDeploymentStatus, ModsPluginDeploymentError> {
    ensure_game_is_closed()?;
    let workspace = prepare_mod_workspace()?;
    let installations = game_installation_directories()?;
    let directory = selected_game_directory(&installations, region)?;
    migrate_legacy_mod_workspace(std::slice::from_ref(directory), &workspace)?;
    install_plugin_to_directories(std::slice::from_ref(directory), plugin)?;
    inspect_game_installations(&installations, Some(plugin))
}

#[cfg(feature = "gui")]
pub fn remove_mods_plugin(
    region: ModsPluginGameRegion,
) -> Result<ModsPluginDeploymentStatus, ModsPluginDeploymentError> {
    ensure_game_is_closed()?;
    let workspace = prepare_mod_workspace()?;
    let installations = game_installation_directories()?;
    let directory = selected_game_directory(&installations, region)?;
    migrate_legacy_mod_workspace(std::slice::from_ref(directory), &workspace)?;
    remove_plugin_from_directories(std::slice::from_ref(directory))?;
    inspect_game_installations(&installations, None)
}

#[cfg(feature = "gui")]
pub fn refresh_installed_mods_plugins(
    plugin: &[u8],
) -> Result<ModsPluginDeploymentStatus, ModsPluginDeploymentError> {
    let workspace = prepare_mod_workspace()?;
    let installations = match game_installation_directories() {
        Ok(installations) => installations,
        Err(ModsPluginDeploymentError::GameInstallationNotFound) => {
            return Ok(ModsPluginDeploymentStatus {
                source_available: true,
                ..Default::default()
            });
        }
        Err(error) => return Err(error),
    };
    let mut managed_directories = Vec::new();
    for (_, directory) in &installations {
        if plugin_directory_is_managed(directory)? {
            managed_directories.push(directory.clone());
        }
    }
    if managed_directories.is_empty() {
        return inspect_game_installations(&installations, Some(plugin));
    }
    ensure_game_is_closed()?;
    migrate_legacy_mod_workspace(&managed_directories, &workspace)?;
    replace_managed_plugin_directories(&managed_directories, plugin)?;
    inspect_game_installations(&installations, Some(plugin))
}

#[cfg(feature = "gui")]
fn ensure_game_is_closed() -> Result<(), ModsPluginDeploymentError> {
    match super::network::game_process_is_running() {
        Ok(false) => Ok(()),
        Ok(true) => Err(ModsPluginDeploymentError::GameRunning),
        Err(error) => Err(ModsPluginDeploymentError::GameProcessProbe(error)),
    }
}

#[cfg(feature = "gui")]
fn prepare_mod_workspace() -> Result<PathBuf, ModsPluginDeploymentError> {
    let workspace = mod_script_workspace_directory();
    install_default_mod_files(&workspace).map_err(file_system_error)?;
    let workspace = workspace.canonicalize().map_err(file_system_error)?;
    register_mod_workspace(&workspace)?;
    Ok(workspace)
}

#[cfg(feature = "gui")]
fn register_mod_workspace(workspace: &Path) -> Result<(), ModsPluginDeploymentError> {
    let subkey = wide_null(MOD_WORKSPACE_REGISTRY_KEY);
    let value_name = wide_null(MOD_WORKSPACE_REGISTRY_VALUE);
    let workspace_value = workspace
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect::<Vec<_>>();
    let mut key = ptr::null_mut();
    // SAFETY: The key path is NUL-terminated and the output handle points to
    // writable storage for the duration of the call.
    let create_result = unsafe { RegCreateKeyW(HKEY_CURRENT_USER, subkey.as_ptr(), &mut key) };
    if create_result != ERROR_SUCCESS {
        return Err(ModsPluginDeploymentError::FileSystem(format!(
            "Mod workspace registry key creation failed with error code {create_result}"
        )));
    }
    // SAFETY: The handle is valid, the value name is NUL-terminated, and the
    // byte count covers the UTF-16 path including its terminator.
    let write_result = unsafe {
        RegSetValueExW(
            key,
            value_name.as_ptr(),
            0,
            REG_SZ,
            workspace_value.as_ptr().cast(),
            (workspace_value.len() * std::mem::size_of::<u16>()) as u32,
        )
    };
    // SAFETY: The handle was returned by RegCreateKeyW and is closed once.
    unsafe {
        RegCloseKey(key);
    }
    if write_result != ERROR_SUCCESS {
        return Err(ModsPluginDeploymentError::FileSystem(format!(
            "Mod workspace registry value write failed with error code {write_result}"
        )));
    }
    Ok(())
}

#[cfg(feature = "gui")]
fn game_installation_directories()
-> Result<Vec<(ModsPluginGameRegion, PathBuf)>, ModsPluginDeploymentError> {
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
        return Err(ModsPluginDeploymentError::GameInstallationNotFound);
    }
    Ok(directories)
}

#[cfg(feature = "gui")]
fn selected_game_directory(
    installations: &[(ModsPluginGameRegion, PathBuf)],
    region: ModsPluginGameRegion,
) -> Result<&PathBuf, ModsPluginDeploymentError> {
    installations
        .iter()
        .find_map(|(candidate, directory)| (*candidate == region).then_some(directory))
        .ok_or(ModsPluginDeploymentError::GameInstallationNotFound)
}

#[cfg(feature = "gui")]
fn read_registry_string(
    subkey: &str,
    value: &str,
) -> Result<Option<String>, ModsPluginDeploymentError> {
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
        return Err(ModsPluginDeploymentError::Registry(format!(
            "registry query failed with error code {first}"
        )));
    }
    if value_type != REG_SZ || byte_len < 2 || byte_len % 2 != 0 {
        return Err(ModsPluginDeploymentError::Registry(
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
        return Err(ModsPluginDeploymentError::Registry(format!(
            "registry value read failed with error code {second}"
        )));
    }
    if value_type != REG_SZ
        || byte_len < 2
        || byte_len % 2 != 0
        || byte_len as usize > buffer.len() * 2
    {
        return Err(ModsPluginDeploymentError::Registry(
            "game install registry value changed to an invalid type or length".to_owned(),
        ));
    }
    buffer.truncate(byte_len as usize / 2);
    if buffer.last() != Some(&0) {
        return Err(ModsPluginDeploymentError::Registry(
            "game install registry value is not a terminated string".to_owned(),
        ));
    }
    buffer.pop();
    String::from_utf16(&buffer).map(Some).map_err(|_| {
        ModsPluginDeploymentError::Registry(
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

#[cfg(all(feature = "gui", test))]
fn encode_plugin_marker(plugin: &[u8]) -> String {
    let marker = plugin_marker(plugin);
    format!(
        "{LEGACY_PLUGIN_MARKER_HEADER}\nsize={}\nfnv1a64={:016x}\n",
        marker.size, marker.fingerprint
    )
}

#[cfg(feature = "gui")]
fn parse_plugin_marker(text: &str) -> Option<PluginMarker> {
    let mut lines = text.lines();
    if lines.next()? != LEGACY_PLUGIN_MARKER_HEADER {
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
fn plugin_binary_is_managed(plugin: &[u8]) -> bool {
    [PLUGIN_BINARY_SIGNATURE, LEGACY_PLUGIN_BINARY_SIGNATURE]
        .iter()
        .any(|signature| {
            plugin
                .windows(signature.len())
                .any(|window| window == *signature)
        })
}

#[cfg(feature = "gui")]
fn plugin_directory_is_managed(directory: &Path) -> Result<bool, ModsPluginDeploymentError> {
    let plugin_path = directory.join(PLUGIN_FILE_NAME);
    let marker_path = directory.join(LEGACY_PLUGIN_MARKER_FILE_NAME);
    if !plugin_path.exists() {
        return if marker_path.exists() {
            Err(ModsPluginDeploymentError::InstalledPluginChanged)
        } else {
            Ok(false)
        };
    }
    let plugin = fs::read(&plugin_path).map_err(file_system_error)?;
    if marker_path.exists() {
        let marker = read_marker(&marker_path)?;
        if plugin_marker(&plugin) != marker {
            return Err(ModsPluginDeploymentError::InstalledPluginChanged);
        }
        return Ok(true);
    }
    Ok(plugin_binary_is_managed(&plugin))
}

#[cfg(feature = "gui")]
fn inspect_game_installations(
    installations: &[(ModsPluginGameRegion, PathBuf)],
    current_plugin: Option<&[u8]>,
) -> Result<ModsPluginDeploymentStatus, ModsPluginDeploymentError> {
    let mut status = ModsPluginDeploymentStatus {
        source_available: current_plugin.is_some(),
        ..Default::default()
    };
    for (region, directory) in installations {
        let game_status =
            inspect_plugin_directories(std::slice::from_ref(directory), current_plugin)?;
        status.installations += 1;
        status.installed += game_status.installed;
        status.current += game_status.current;
        status.games.push(ModsPluginGameStatus {
            region: *region,
            installed: game_status.installed == 1,
            current: game_status.current == 1,
        });
    }
    Ok(status)
}

#[cfg(feature = "gui")]
fn inspect_mod_targets(
    installations: &[(ModsPluginGameRegion, PathBuf)],
    workspace: &Path,
) -> Result<Vec<ModsPluginModTarget>, ModsPluginDeploymentError> {
    installations
        .iter()
        .map(|(region, directory)| {
            let status = inspect_plugin_directories(std::slice::from_ref(directory), None)?;
            Ok(ModsPluginModTarget {
                region: *region,
                directory: workspace.to_path_buf(),
                installed: status.installed == 1,
            })
        })
        .collect()
}

#[cfg(feature = "gui")]
fn inspect_plugin_directories(
    directories: &[PathBuf],
    current_plugin: Option<&[u8]>,
) -> Result<ModsPluginDeploymentStatus, ModsPluginDeploymentError> {
    let mut status = ModsPluginDeploymentStatus {
        installations: directories.len(),
        ..Default::default()
    };
    for directory in directories {
        if !plugin_directory_is_managed(directory)? {
            continue;
        }
        let plugin = fs::read(directory.join(PLUGIN_FILE_NAME)).map_err(file_system_error)?;
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
) -> Result<(), ModsPluginDeploymentError> {
    if !plugin_binary_is_managed(plugin) {
        return Err(ModsPluginDeploymentError::FileSystem(
            "Mod loader binary signature is missing".to_owned(),
        ));
    }
    for directory in directories {
        let plugin_path = directory.join(PLUGIN_FILE_NAME);
        if !plugin_path.exists() {
            continue;
        }
        if !plugin_directory_is_managed(directory)? {
            return Err(ModsPluginDeploymentError::ConflictingDwmapi);
        }
    }
    for directory in directories {
        let plugin_path = directory.join(PLUGIN_FILE_NAME);
        atomic_replace_bytes(&plugin_path, plugin).map_err(file_system_error)?;
        remove_legacy_game_mod_files(directory).map_err(file_system_error)?;
    }
    Ok(())
}

#[cfg(feature = "gui")]
fn replace_managed_plugin_directories(
    directories: &[PathBuf],
    plugin: &[u8],
) -> Result<(), ModsPluginDeploymentError> {
    if !plugin_binary_is_managed(plugin) {
        return Err(ModsPluginDeploymentError::FileSystem(
            "Mod loader binary signature is missing".to_owned(),
        ));
    }
    let mut backups = Vec::with_capacity(directories.len());
    for directory in directories {
        if !plugin_directory_is_managed(directory)? {
            return Err(ModsPluginDeploymentError::InstalledPluginChanged);
        }
        let plugin_path = directory.join(PLUGIN_FILE_NAME);
        let existing = fs::read(&plugin_path)
            .map_err(|_| ModsPluginDeploymentError::InstalledPluginChanged)?;
        backups.push((plugin_path, existing));
    }

    for (plugin_path, _) in &backups {
        let result = atomic_replace_bytes(plugin_path, plugin).and_then(|_| {
            remove_legacy_game_mod_files(
                plugin_path
                    .parent()
                    .expect("validated plugin path has a parent directory"),
            )
        });
        if let Err(error) = result {
            let rollback_failures = restore_managed_plugin_backups(&backups);
            let detail = if rollback_failures.is_empty() {
                error.to_string()
            } else {
                format!("{error}; rollback failed: {}", rollback_failures.join("; "))
            };
            return Err(ModsPluginDeploymentError::FileSystem(detail));
        }
    }
    Ok(())
}

#[cfg(feature = "gui")]
fn restore_managed_plugin_backups(backups: &[(PathBuf, Vec<u8>)]) -> Vec<String> {
    let mut failures = Vec::new();
    for (plugin_path, plugin) in backups.iter().rev() {
        if let Err(error) = atomic_replace_bytes(plugin_path, plugin) {
            failures.push(format!("{}: {error}", plugin_path.display()));
        }
    }
    failures
}

#[cfg(feature = "gui")]
fn atomic_replace_bytes(path: &Path, bytes: &[u8]) -> io::Result<()> {
    let parent = path
        .parent()
        .expect("validated plugin path has a parent directory");
    fs::create_dir_all(parent)?;
    let file_name = path
        .file_name()
        .expect("validated plugin path has a file name")
        .to_string_lossy();
    let sequence = PLUGIN_TEMP_SEQUENCE.fetch_add(1, Ordering::Relaxed);
    let temporary = parent.join(format!(
        ".{file_name}.{}.{sequence}.tmp",
        std::process::id()
    ));
    if temporary.exists() {
        fs::remove_file(&temporary)?;
    }
    let result = (|| {
        let mut file = fs::File::create(&temporary)?;
        file.write_all(bytes)?;
        file.flush()?;
        file.sync_all()?;
        drop(file);
        let temporary_wide = temporary
            .as_os_str()
            .encode_wide()
            .chain(std::iter::once(0))
            .collect::<Vec<_>>();
        let target_wide = path
            .as_os_str()
            .encode_wide()
            .chain(std::iter::once(0))
            .collect::<Vec<_>>();
        // SAFETY: Both path buffers are null-terminated and remain alive for the call.
        if unsafe {
            MoveFileExW(
                temporary_wide.as_ptr(),
                target_wide.as_ptr(),
                MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
            )
        } == 0
        {
            return Err(io::Error::last_os_error());
        }
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_file(temporary);
    }
    result
}

#[cfg(feature = "gui")]
fn remove_plugin_from_directories(
    directories: &[PathBuf],
) -> Result<(), ModsPluginDeploymentError> {
    for directory in directories {
        let plugin_path = directory.join(PLUGIN_FILE_NAME);
        if !plugin_directory_is_managed(directory)? {
            continue;
        }
        fs::remove_file(&plugin_path).map_err(file_system_error)?;
        remove_legacy_game_mod_files(directory).map_err(file_system_error)?;
    }
    Ok(())
}

#[cfg(feature = "gui")]
fn migrate_legacy_mod_workspace(
    game_directories: &[PathBuf],
    workspace: &Path,
) -> Result<(), ModsPluginDeploymentError> {
    let mut enabled_set = None;
    let mut scripts = BTreeMap::<String, Vec<u8>>::new();
    for game_directory in game_directories {
        let enabled_path = game_directory.join(MOD_SET_FILE_NAME);
        if enabled_path.is_file() {
            let bytes = read_legacy_mod_file(&enabled_path)?;
            let source = std::str::from_utf8(&bytes).map_err(|_| {
                ModsPluginDeploymentError::FileSystem(format!(
                    "{} is not UTF-8",
                    enabled_path.display()
                ))
            })?;
            validate_enabled_mod_set(source).map_err(|error| {
                ModsPluginDeploymentError::FileSystem(format!(
                    "{} is invalid: {error:?}",
                    enabled_path.display()
                ))
            })?;
            if enabled_set
                .as_ref()
                .is_some_and(|existing: &Vec<u8>| existing != &bytes)
            {
                return Err(ModsPluginDeploymentError::FileSystem(
                    "game installations contain different enabled Mod sets".to_owned(),
                ));
            }
            enabled_set = Some(bytes);
        }

        let mod_directory = game_directory.join(MOD_DIRECTORY_NAME);
        let entries = match fs::read_dir(&mod_directory) {
            Ok(entries) => entries,
            Err(error) if error.kind() == io::ErrorKind::NotFound => continue,
            Err(error) => return Err(file_system_error(error)),
        };
        for entry in entries {
            let entry = entry.map_err(file_system_error)?;
            let file_type = entry.file_type().map_err(file_system_error)?;
            let path = entry.path();
            let is_nte = path
                .extension()
                .and_then(|extension| extension.to_str())
                .is_some_and(|extension| extension.eq_ignore_ascii_case("nte"));
            if !file_type.is_file() || !is_nte {
                return Err(ModsPluginDeploymentError::FileSystem(format!(
                    "{} contains an unsupported legacy Mod entry",
                    mod_directory.display()
                )));
            }
            let file_name = entry.file_name().into_string().map_err(|_| {
                ModsPluginDeploymentError::FileSystem(format!(
                    "{} contains a non-Unicode file name",
                    mod_directory.display()
                ))
            })?;
            let id = path
                .file_stem()
                .and_then(|value| value.to_str())
                .ok_or_else(|| {
                    ModsPluginDeploymentError::FileSystem(format!(
                        "{} has an invalid Mod file name",
                        path.display()
                    ))
                })?;
            let mut bytes = read_legacy_mod_file(&path)?;
            if file_name.eq_ignore_ascii_case(EQUIPMENT_MOD_FILE_NAME)
                && LEGACY_EQUIPMENT_MOD_PROGRAMS
                    .iter()
                    .any(|legacy| bytes == *legacy)
            {
                bytes = EQUIPMENT_MOD.to_vec();
            } else if file_name.eq_ignore_ascii_case(COMBAT_CLOCK_MOD_FILE_NAME)
                && LEGACY_COMBAT_CLOCK_MOD_PROGRAMS
                    .iter()
                    .any(|legacy| bytes == *legacy)
            {
                bytes = COMBAT_CLOCK_MOD.to_vec();
            }
            let source = std::str::from_utf8(&bytes).map_err(|_| {
                ModsPluginDeploymentError::FileSystem(format!("{} is not UTF-8", path.display()))
            })?;
            validate_mod_source(id, source).map_err(|error| {
                ModsPluginDeploymentError::FileSystem(format!(
                    "{} is invalid: {error:?}",
                    path.display()
                ))
            })?;
            if scripts
                .get(&file_name)
                .is_some_and(|existing| existing != &bytes)
            {
                return Err(ModsPluginDeploymentError::FileSystem(format!(
                    "game installations contain different {file_name} programs"
                )));
            }
            scripts.insert(file_name, bytes);
        }
    }

    if let Some(bytes) = enabled_set {
        atomic_replace_bytes(&workspace.join(MOD_SET_FILE_NAME), &bytes)
            .map_err(file_system_error)?;
    }
    let mod_directory = workspace.join(MOD_DIRECTORY_NAME);
    fs::create_dir_all(&mod_directory).map_err(file_system_error)?;
    for (file_name, bytes) in scripts {
        atomic_replace_bytes(&mod_directory.join(file_name), &bytes).map_err(file_system_error)?;
    }
    Ok(())
}

#[cfg(feature = "gui")]
fn read_legacy_mod_file(path: &Path) -> Result<Vec<u8>, ModsPluginDeploymentError> {
    let bytes = fs::read(path).map_err(file_system_error)?;
    if bytes.len() > MAX_LEGACY_MOD_FILE_BYTES {
        return Err(ModsPluginDeploymentError::FileSystem(format!(
            "{} exceeds 16 KiB",
            path.display()
        )));
    }
    Ok(bytes)
}

#[cfg(feature = "gui")]
fn remove_legacy_game_mod_files(directory: &Path) -> io::Result<()> {
    let mod_directory = directory.join(MOD_DIRECTORY_NAME);
    let mut mod_files = Vec::new();
    match fs::read_dir(&mod_directory) {
        Ok(entries) => {
            for entry in entries {
                let entry = entry?;
                let file_type = entry.file_type()?;
                let path = entry.path();
                let is_nte = path
                    .extension()
                    .and_then(|extension| extension.to_str())
                    .is_some_and(|extension| extension.eq_ignore_ascii_case("nte"));
                if !file_type.is_file() || !is_nte {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidData,
                        format!(
                            "{} contains an unsupported legacy Mod entry",
                            mod_directory.display()
                        ),
                    ));
                }
                mod_files.push(path);
            }
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound => {}
        Err(error) => return Err(error),
    }
    for path in mod_files {
        fs::remove_file(path)?;
    }
    if mod_directory.exists() {
        fs::remove_dir(&mod_directory)?;
    }
    let enabled_path = directory.join(MOD_SET_FILE_NAME);
    if enabled_path.exists() {
        fs::remove_file(enabled_path)?;
    }
    let marker_path = directory.join(LEGACY_PLUGIN_MARKER_FILE_NAME);
    if marker_path.exists() {
        fs::remove_file(marker_path)?;
    }
    Ok(())
}

#[cfg(feature = "gui")]
fn rollback_default_mod_files(created: &[PathBuf], migrated: &[(PathBuf, Vec<u8>)]) {
    for created_path in created.iter().rev() {
        let _ = fs::remove_file(created_path);
    }
    for (migrated_path, previous) in migrated.iter().rev() {
        let _ = fs::write(migrated_path, previous);
    }
}

#[cfg(feature = "gui")]
fn install_default_mod_files(workspace_directory: &Path) -> io::Result<()> {
    let mod_directory = workspace_directory.join(MOD_DIRECTORY_NAME);
    fs::create_dir_all(&mod_directory)?;
    let mut created = Vec::new();
    let mut migrated = Vec::new();
    for (path, bytes, legacy) in [
        (
            workspace_directory.join(MOD_SET_FILE_NAME),
            DEFAULT_MOD_SET,
            NO_LEGACY_MOD_PROGRAMS,
        ),
        (
            mod_directory.join(EQUIPMENT_MOD_FILE_NAME),
            EQUIPMENT_MOD,
            LEGACY_EQUIPMENT_MOD_PROGRAMS,
        ),
        (
            mod_directory.join(COMBAT_CLOCK_MOD_FILE_NAME),
            COMBAT_CLOCK_MOD,
            LEGACY_COMBAT_CLOCK_MOD_PROGRAMS,
        ),
    ] {
        if !path.exists() {
            if let Err(error) = fs::write(&path, bytes) {
                rollback_default_mod_files(&created, &migrated);
                let _ = fs::remove_dir(&mod_directory);
                return Err(error);
            }
            created.push(path);
        } else {
            let existing = match fs::read(&path) {
                Ok(existing) => existing,
                Err(error) => {
                    rollback_default_mod_files(&created, &migrated);
                    return Err(error);
                }
            };
            if !legacy.iter().any(|previous| existing == *previous) {
                continue;
            }
            if let Err(error) = fs::write(&path, bytes) {
                rollback_default_mod_files(&created, &migrated);
                return Err(error);
            }
            migrated.push((path, existing));
        }
    }
    Ok(())
}

#[cfg(feature = "gui")]
fn read_marker(path: &Path) -> Result<PluginMarker, ModsPluginDeploymentError> {
    let text = fs::read_to_string(path).map_err(file_system_error)?;
    parse_plugin_marker(&text).ok_or(ModsPluginDeploymentError::InstalledPluginChanged)
}

#[cfg(feature = "gui")]
fn file_system_error(error: io::Error) -> ModsPluginDeploymentError {
    ModsPluginDeploymentError::FileSystem(error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    const NATIVE_IPC_HEADER: &str = include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/native/nte-mods-plugin/include/nte_mods_ipc.h"
    ));

    fn native_define(name: &str) -> u64 {
        let prefix = format!("#define {name} ");
        let value = NATIVE_IPC_HEADER
            .lines()
            .find_map(|line| line.trim().strip_prefix(&prefix))
            .unwrap_or_else(|| panic!("native IPC header is missing {name}"))
            .trim_end_matches(['u', 'U']);
        if let Some(hex) = value.strip_prefix("0x") {
            u64::from_str_radix(hex, 16).expect("native IPC hexadecimal define must be valid")
        } else {
            value
                .parse()
                .expect("native IPC decimal define must be valid")
        }
    }

    fn native_enum(name: &str) -> u64 {
        let prefix = format!("{name} = ");
        NATIVE_IPC_HEADER
            .lines()
            .find_map(|line| line.trim().strip_prefix(&prefix))
            .map(|value| value.trim_end_matches(','))
            .unwrap_or_else(|| panic!("native IPC header is missing {name}"))
            .parse()
            .expect("native IPC enum value must be valid")
    }

    #[test]
    #[cfg(feature = "gui")]
    fn bundled_mods_keep_distinct_control_flow_in_external_sources() {
        let equipment =
            std::str::from_utf8(EQUIPMENT_MOD).expect("bundled equipment Mod must be UTF-8");
        let combat_clock =
            std::str::from_utf8(COMBAT_CLOCK_MOD).expect("bundled combat-clock Mod must be UTF-8");

        crate::storage::mod_scripts::validate_mod_source("equipment", equipment).unwrap();
        crate::storage::mod_scripts::validate_mod_source("combat-clock", combat_clock).unwrap();
        assert!(equipment.contains("game.player_state"));
        assert!(equipment.contains("state.next_prepare_at"));
        assert!(equipment.contains("equipment.cache_ready(player_state)"));
        assert!(equipment.contains("ipc.bind(player_state, None)"));
        assert!(!equipment.contains("0x"));
        assert!(combat_clock.contains("game.player_controller"));
        assert!(combat_clock.contains("state.last_pause_mask"));
        assert!(combat_clock.contains("combat_clock.pause_mask(player_controller)"));
        assert!(combat_clock.contains("combat_clock.state_flags(player_controller)"));
        assert!(combat_clock.contains("combat_clock.forward(pause_mask, state_flags)"));
        assert!(!combat_clock.contains("combat_clock.observe("));
        assert!(!combat_clock.contains("0x"));
        assert_ne!(equipment, combat_clock);
    }

    #[test]
    fn rust_wire_constants_match_the_native_ipc_header() {
        assert_eq!(native_define("NTE_MODS_IPC_MAGIC"), IPC_MAGIC as u64);
        assert_eq!(native_define("NTE_MODS_IPC_VERSION"), IPC_VERSION as u64);
        assert_eq!(
            native_define("NTE_EQUIPMENT_MAX_PLACEMENTS"),
            MAX_PLACEMENTS as u64
        );
        assert_eq!(
            native_define("NTE_MODS_IPC_REQUEST_SIZE"),
            REQUEST_SIZE as u64
        );
        assert_eq!(
            native_define("NTE_MODS_IPC_RESPONSE_SIZE"),
            RESPONSE_SIZE as u64
        );
        assert_eq!(
            native_define("NTE_COMBAT_CLOCK_HISTORY_SIZE"),
            COMBAT_CLOCK_HISTORY_SIZE as u64
        );
        assert_eq!(
            native_define("NTE_MOD_EVENT_HISTORY_SIZE"),
            MOD_EVENT_HISTORY_SIZE as u64
        );
        assert_eq!(
            native_define("NTE_MOD_EVENT_ID_SIZE"),
            MOD_EVENT_ID_SIZE as u64
        );
        assert_eq!(
            native_define("NTE_MOD_EVENT_NAME_SIZE"),
            MOD_EVENT_NAME_SIZE as u64
        );
        assert_eq!(
            native_define("NTE_MOD_EVENT_VALUE_COUNT"),
            MOD_EVENT_VALUE_COUNT as u64
        );
        assert_eq!(
            native_enum("NTE_MODS_IPC_EQUIP_MODULE"),
            IPC_EQUIP_MODULE as u64
        );
        assert_eq!(
            native_enum("NTE_MODS_IPC_EQUIP_CORE"),
            IPC_EQUIP_CORE as u64
        );
        assert_eq!(
            native_enum("NTE_MODS_IPC_UNEQUIP_MODULE"),
            IPC_UNEQUIP_MODULE as u64
        );
        assert_eq!(
            native_enum("NTE_MODS_IPC_UNEQUIP_CORE"),
            IPC_UNEQUIP_CORE as u64
        );
        assert_eq!(
            native_enum("NTE_MODS_IPC_UNEQUIP_ALL"),
            IPC_UNEQUIP_ALL as u64
        );
        assert_eq!(
            native_enum("NTE_MODS_IPC_EQUIP_ONE_KEY"),
            IPC_EQUIP_ONE_KEY as u64
        );
        assert_eq!(
            native_enum("NTE_MODS_IPC_MOVE_MODULE_TO_CHARACTER"),
            IPC_MOVE_MODULE_TO_CHARACTER as u64
        );
        assert_eq!(
            native_enum("NTE_MODS_IPC_MOVE_CORE_TO_CHARACTER"),
            IPC_MOVE_CORE_TO_CHARACTER as u64
        );
        assert_eq!(
            native_enum("NTE_MODS_IPC_SET_ITEM_DISCARDED"),
            IPC_SET_ITEM_DISCARDED as u64
        );
        assert_eq!(
            native_enum("NTE_MODS_IPC_SET_ITEM_LOCKED"),
            IPC_SET_ITEM_LOCKED as u64
        );
        assert_eq!(
            native_enum("NTE_MODS_IPC_QUERY_COMBAT_CLOCK_TRANSITIONS"),
            IPC_QUERY_COMBAT_CLOCK_TRANSITIONS as u64
        );
        assert_eq!(
            native_enum("NTE_MODS_IPC_QUERY_MOD_EVENTS"),
            IPC_QUERY_MOD_EVENTS as u64
        );
        assert_eq!(
            native_enum("NTE_MODS_STATUS_MOD_DISABLED"),
            MAX_PLUGIN_STATUS as u64
        );
    }

    #[test]
    fn module_request_uses_the_stable_little_endian_wire_layout() {
        let bytes = encode_request(&ModsPluginRequest {
            request_id: 9,
            character: HtItemNetId { solt: 1, serial: 2 },
            operation: ModsPluginOperation::EquipModule {
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
        let bytes = encode_request(&ModsPluginRequest {
            request_id: 11,
            character: HtItemNetId { solt: 1, serial: 2 },
            operation: ModsPluginOperation::EquipOneKey {
                placements: vec![ModsPluginPlacement {
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
    fn v7_operations_encode_state_and_move_fields() {
        let moved = encode_request(&ModsPluginRequest {
            request_id: 12,
            character: HtItemNetId { solt: 1, serial: 2 },
            operation: ModsPluginOperation::MoveModuleToCharacter {
                equipment: HtItemNetId { solt: 3, serial: 4 },
                row: 2,
                column: 5,
            },
        });
        assert_eq!(
            u16::from_le_bytes(moved[4..6].try_into().unwrap()),
            IPC_VERSION
        );
        assert_eq!(u16::from_le_bytes(moved[6..8].try_into().unwrap()), 7);
        assert_eq!(u32::from_le_bytes(moved[16..20].try_into().unwrap()), 1);
        assert_eq!(u32::from_le_bytes(moved[24..28].try_into().unwrap()), 3);
        assert_eq!(i32::from_le_bytes(moved[40..44].try_into().unwrap()), 2);
        assert_eq!(i32::from_le_bytes(moved[44..48].try_into().unwrap()), 5);

        let moved_core = encode_request(&ModsPluginRequest {
            request_id: 13,
            character: HtItemNetId { solt: 7, serial: 8 },
            operation: ModsPluginOperation::MoveCoreToCharacter {
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

        let discarded = encode_request(&ModsPluginRequest {
            request_id: 14,
            character: HtItemNetId::ZERO,
            operation: ModsPluginOperation::SetItemDiscarded {
                equipment: HtItemNetId {
                    solt: 11,
                    serial: 12,
                },
                discarded: true,
            },
        });
        assert_eq!(u16::from_le_bytes(discarded[6..8].try_into().unwrap()), 9);
        assert_eq!(u32::from_le_bytes(discarded[52..56].try_into().unwrap()), 1);

        let locked = encode_request(&ModsPluginRequest {
            request_id: 15,
            character: HtItemNetId::ZERO,
            operation: ModsPluginOperation::SetItemLocked {
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
    fn response_accepts_the_mod_disabled_status() {
        let mut bytes = [0_u8; RESPONSE_SIZE];
        bytes[0..4].copy_from_slice(&IPC_MAGIC.to_le_bytes());
        bytes[4..6].copy_from_slice(&IPC_VERSION.to_le_bytes());
        bytes[8..16].copy_from_slice(&9_u64.to_le_bytes());
        bytes[16..20].copy_from_slice(&PLUGIN_STATUS_MOD_DISABLED.to_le_bytes());
        assert_eq!(decode_response(&bytes, 9), Ok(PLUGIN_STATUS_MOD_DISABLED));
    }

    #[test]
    fn combat_clock_mod_disabled_status_is_reported_explicitly() {
        let mut bytes = [0_u8; RESPONSE_SIZE];
        bytes[0..4].copy_from_slice(&IPC_MAGIC.to_le_bytes());
        bytes[4..6].copy_from_slice(&IPC_VERSION.to_le_bytes());
        bytes[8..16].copy_from_slice(&19_u64.to_le_bytes());
        bytes[16..20].copy_from_slice(&PLUGIN_STATUS_MOD_DISABLED.to_le_bytes());

        assert_eq!(
            decode_combat_clock_transitions(&bytes, 19),
            Err("combat clock mod is disabled".to_owned())
        );
    }

    #[test]
    fn combat_clock_response_uses_the_v7_transition_layout() {
        let mut bytes = [0_u8; RESPONSE_SIZE];
        bytes[0..4].copy_from_slice(&IPC_MAGIC.to_le_bytes());
        bytes[4..6].copy_from_slice(&IPC_VERSION.to_le_bytes());
        bytes[8..16].copy_from_slice(&19_u64.to_le_bytes());
        bytes[16..20].copy_from_slice(&PLUGIN_STATUS_DRY_RUN_OK.to_le_bytes());
        bytes[20..24].copy_from_slice(&1_u32.to_le_bytes());
        bytes[24..32].copy_from_slice(&7_u64.to_le_bytes());
        bytes[32..40].copy_from_slice(&133_000_000_000_000_000_u64.to_le_bytes());
        bytes[40..44].copy_from_slice(&(1_u32 << 2).to_le_bytes());
        bytes[48..52].copy_from_slice(&COMBAT_CLOCK_PAUSE_VALID.to_le_bytes());

        assert_eq!(
            decode_combat_clock_transitions(&bytes, 19),
            Ok(vec![CombatClockTransitionSnapshot {
                sequence: 7,
                timestamp_100ns: 133_000_000_000_000_000,
                pause_type_mask: 1 << 2,
                reserved_value: 0,
                state_flags: COMBAT_CLOCK_PAUSE_VALID,
            }])
        );
    }

    #[test]
    fn mod_event_response_uses_the_v7_union_layout() {
        let mut bytes = [0_u8; RESPONSE_SIZE];
        bytes[0..4].copy_from_slice(&IPC_MAGIC.to_le_bytes());
        bytes[4..6].copy_from_slice(&IPC_VERSION.to_le_bytes());
        bytes[8..16].copy_from_slice(&23_u64.to_le_bytes());
        bytes[16..20].copy_from_slice(&PLUGIN_STATUS_DRY_RUN_OK.to_le_bytes());
        bytes[20..24].copy_from_slice(&1_u32.to_le_bytes());
        bytes[24..32].copy_from_slice(&9_u64.to_le_bytes());
        bytes[32..40].copy_from_slice(&133_000_000_000_000_000_u64.to_le_bytes());
        bytes[40..49].copy_from_slice(b"telemetry");
        bytes[72..82].copy_from_slice(b"hp.changed");
        bytes[104..108].copy_from_slice(&2_u32.to_le_bytes());
        bytes[112..120].copy_from_slice(&12_500_u64.to_le_bytes());
        bytes[120..128].copy_from_slice(&11_000_u64.to_le_bytes());

        assert_eq!(
            decode_mod_events(&bytes, 23),
            Ok(vec![ModEventSnapshot {
                sequence: 9,
                timestamp_100ns: 133_000_000_000_000_000,
                mod_id: "telemetry".to_owned(),
                name: "hp.changed".to_owned(),
                values: vec![12_500, 11_000],
            }])
        );
    }

    #[test]
    fn mod_event_response_rejects_nonzero_unused_values() {
        let mut bytes = [0_u8; RESPONSE_SIZE];
        bytes[0..4].copy_from_slice(&IPC_MAGIC.to_le_bytes());
        bytes[4..6].copy_from_slice(&IPC_VERSION.to_le_bytes());
        bytes[8..16].copy_from_slice(&23_u64.to_le_bytes());
        bytes[16..20].copy_from_slice(&PLUGIN_STATUS_DRY_RUN_OK.to_le_bytes());
        bytes[20..24].copy_from_slice(&1_u32.to_le_bytes());
        bytes[40..49].copy_from_slice(b"telemetry");
        bytes[72..82].copy_from_slice(b"hp.changed");
        bytes[104..108].copy_from_slice(&1_u32.to_le_bytes());
        bytes[112..120].copy_from_slice(&12_500_u64.to_le_bytes());
        bytes[120..128].copy_from_slice(&11_000_u64.to_le_bytes());

        assert!(decode_mod_events(&bytes, 23).is_err());
    }

    #[test]
    fn combat_clock_response_rejects_legacy_timer_payloads() {
        let mut bytes = [0_u8; RESPONSE_SIZE];
        bytes[0..4].copy_from_slice(&IPC_MAGIC.to_le_bytes());
        bytes[4..6].copy_from_slice(&IPC_VERSION.to_le_bytes());
        bytes[8..16].copy_from_slice(&19_u64.to_le_bytes());
        bytes[16..20].copy_from_slice(&PLUGIN_STATUS_DRY_RUN_OK.to_le_bytes());
        bytes[20..24].copy_from_slice(&1_u32.to_le_bytes());
        bytes[24..32].copy_from_slice(&7_u64.to_le_bytes());
        bytes[32..40].copy_from_slice(&133_000_000_000_000_000_u64.to_le_bytes());
        bytes[44..48].copy_from_slice(&87_i32.to_le_bytes());
        bytes[48..52].copy_from_slice(&2_u32.to_le_bytes());

        assert!(decode_combat_clock_transitions(&bytes, 19).is_err());
    }

    #[test]
    fn combat_clock_response_rejects_non_authoritative_pause_types() {
        let mut bytes = [0_u8; RESPONSE_SIZE];
        bytes[0..4].copy_from_slice(&IPC_MAGIC.to_le_bytes());
        bytes[4..6].copy_from_slice(&IPC_VERSION.to_le_bytes());
        bytes[8..16].copy_from_slice(&19_u64.to_le_bytes());
        bytes[16..20].copy_from_slice(&PLUGIN_STATUS_DRY_RUN_OK.to_le_bytes());
        bytes[20..24].copy_from_slice(&1_u32.to_le_bytes());
        bytes[24..32].copy_from_slice(&7_u64.to_le_bytes());
        bytes[32..40].copy_from_slice(&133_000_000_000_000_000_u64.to_le_bytes());
        bytes[40..44].copy_from_slice(&(1_u32 << 1).to_le_bytes());
        bytes[48..52].copy_from_slice(&COMBAT_CLOCK_PAUSE_VALID.to_le_bytes());

        assert!(decode_combat_clock_transitions(&bytes, 19).is_err());
    }

    #[test]
    fn client_bounds_requests_behind_the_active_pipe_call() {
        let (started_tx, started_rx) = bounded(1);
        let (release_tx, release_rx) = bounded(1);
        let call_count = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let worker_call_count = Arc::clone(&call_count);
        let mut client = ModsPluginClient::with_call_for_test(move |_| {
            if worker_call_count.fetch_add(1, Ordering::AcqRel) == 0 {
                started_tx.send(()).unwrap();
                release_rx.recv().unwrap();
            }
            Ok(0)
        });

        assert_eq!(
            client.submit(
                HtItemNetId { solt: 1, serial: 2 },
                ModsPluginOperation::UnequipAll
            ),
            Ok(1)
        );
        started_rx
            .recv_timeout(std::time::Duration::from_secs(1))
            .unwrap();
        assert_eq!(
            client.submit(
                HtItemNetId { solt: 3, serial: 4 },
                ModsPluginOperation::UnequipAll
            ),
            Ok(2)
        );
        assert_eq!(
            client.submit(
                HtItemNetId { solt: 5, serial: 6 },
                ModsPluginOperation::UnequipAll
            ),
            Err(ModsPluginSubmitError::Busy)
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
            "nte-mods-plugin-{name}-{}-{unique}",
            std::process::id()
        ));
        fs::create_dir_all(&path).unwrap();
        path
    }

    #[cfg(feature = "gui")]
    fn managed_plugin(label: &str) -> Vec<u8> {
        format!(
            "{}:{label}",
            std::str::from_utf8(PLUGIN_BINARY_SIGNATURE).unwrap()
        )
        .into_bytes()
    }

    #[test]
    #[cfg(feature = "gui")]
    fn retired_loader_signature_remains_managed_for_upgrade() {
        let legacy = [LEGACY_PLUGIN_BINARY_SIGNATURE, b":legacy-runtime"].concat();

        assert!(plugin_binary_is_managed(&legacy));
    }

    #[cfg(feature = "gui")]
    fn install_legacy_managed_plugin(directory: &Path, plugin: &[u8]) {
        fs::write(directory.join(PLUGIN_FILE_NAME), plugin).unwrap();
        fs::write(
            directory.join(LEGACY_PLUGIN_MARKER_FILE_NAME),
            encode_plugin_marker(plugin),
        )
        .unwrap();
        install_default_mod_files(directory).unwrap();
    }

    #[test]
    #[cfg(feature = "gui")]
    fn mod_targets_report_only_managed_plugin_installations() {
        let china = deployment_test_directory("mod-target-china");
        let global = deployment_test_directory("mod-target-global");
        let workspace = deployment_test_directory("mod-target-workspace");
        install_plugin_to_directories(std::slice::from_ref(&china), &managed_plugin("current"))
            .unwrap();
        fs::write(global.join(PLUGIN_FILE_NAME), b"unmanaged").unwrap();
        let targets = inspect_mod_targets(
            &[
                (ModsPluginGameRegion::China, china.clone()),
                (ModsPluginGameRegion::Global, global.clone()),
            ],
            &workspace,
        )
        .unwrap();

        assert_eq!(
            targets,
            vec![
                ModsPluginModTarget {
                    region: ModsPluginGameRegion::China,
                    directory: workspace.clone(),
                    installed: true,
                },
                ModsPluginModTarget {
                    region: ModsPluginGameRegion::Global,
                    directory: workspace.clone(),
                    installed: false,
                },
            ]
        );
        fs::remove_dir_all(china).unwrap();
        fs::remove_dir_all(global).unwrap();
        fs::remove_dir_all(workspace).unwrap();
    }

    #[test]
    #[cfg(feature = "gui")]
    fn deployment_marks_installs_and_removes_only_the_managed_plugin() {
        let directory = deployment_test_directory("lifecycle");
        let directories = vec![directory.clone()];
        let plugin = managed_plugin("lifecycle");

        install_plugin_to_directories(&directories, &plugin).unwrap();
        assert_eq!(fs::read(directory.join(PLUGIN_FILE_NAME)).unwrap(), plugin);
        assert!(!directory.join(LEGACY_PLUGIN_MARKER_FILE_NAME).exists());
        assert!(!directory.join(MOD_SET_FILE_NAME).exists());
        assert!(!directory.join(MOD_DIRECTORY_NAME).exists());
        assert_eq!(
            inspect_plugin_directories(&directories, Some(&plugin)).unwrap(),
            ModsPluginDeploymentStatus {
                installations: 1,
                installed: 1,
                current: 1,
                source_available: false,
                games: Vec::new(),
            }
        );
        remove_plugin_from_directories(&directories).unwrap();
        assert_eq!(
            inspect_plugin_directories(&directories, Some(&plugin)).unwrap(),
            ModsPluginDeploymentStatus {
                installations: 1,
                installed: 0,
                current: 0,
                source_available: false,
                games: Vec::new(),
            }
        );
        assert!(!directory.join(PLUGIN_FILE_NAME).exists());
        assert!(!directory.join(LEGACY_PLUGIN_MARKER_FILE_NAME).exists());
        assert!(!directory.join(MOD_SET_FILE_NAME).exists());
        assert!(!directory.join(MOD_DIRECTORY_NAME).exists());

        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    #[cfg(feature = "gui")]
    fn legacy_game_mods_migrate_to_the_software_workspace_before_removal() {
        let directory = deployment_test_directory("mod-migration");
        let workspace = deployment_test_directory("mod-workspace");
        let plugin = managed_plugin("migration");
        install_plugin_to_directories(std::slice::from_ref(&directory), &plugin).unwrap();
        fs::create_dir_all(directory.join(MOD_DIRECTORY_NAME)).unwrap();
        fs::write(
            directory.join(MOD_SET_FILE_NAME),
            b"nte_mod_set 1\nload combat-clock\n",
        )
        .unwrap();
        let custom = b"nte_mod(4)\nmod(\"combat-clock\")\nrequires(\"viewport.tick\")\n\ndef on_viewport_tick(event):\n    value = 1\n";
        fs::write(
            directory
                .join(MOD_DIRECTORY_NAME)
                .join(COMBAT_CLOCK_MOD_FILE_NAME),
            custom,
        )
        .unwrap();

        migrate_legacy_mod_workspace(std::slice::from_ref(&directory), &workspace).unwrap();
        remove_plugin_from_directories(std::slice::from_ref(&directory)).unwrap();

        assert_eq!(
            fs::read(workspace.join(MOD_SET_FILE_NAME)).unwrap(),
            b"nte_mod_set 1\nload combat-clock\n"
        );
        assert!(
            !workspace
                .join(MOD_DIRECTORY_NAME)
                .join(EQUIPMENT_MOD_FILE_NAME)
                .exists()
        );
        assert_eq!(
            fs::read(
                workspace
                    .join(MOD_DIRECTORY_NAME)
                    .join(COMBAT_CLOCK_MOD_FILE_NAME)
            )
            .unwrap(),
            custom
        );
        assert!(!directory.join(PLUGIN_FILE_NAME).exists());
        assert!(!directory.join(MOD_SET_FILE_NAME).exists());
        assert!(!directory.join(MOD_DIRECTORY_NAME).exists());
        fs::remove_dir_all(directory).unwrap();
        fs::remove_dir_all(workspace).unwrap();
    }

    #[test]
    #[cfg(feature = "gui")]
    fn deployment_preserves_an_unmanaged_dwmapi_proxy() {
        let directory = deployment_test_directory("conflict");
        fs::write(directory.join(PLUGIN_FILE_NAME), b"another mod").unwrap();
        let plugin = managed_plugin("ours");

        assert_eq!(
            install_plugin_to_directories(std::slice::from_ref(&directory), &plugin),
            Err(ModsPluginDeploymentError::ConflictingDwmapi)
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
            (ModsPluginGameRegion::China, china.clone()),
            (ModsPluginGameRegion::Global, global.clone()),
        ];
        let selected =
            selected_game_directory(&installations, ModsPluginGameRegion::China).unwrap();
        let plugin = managed_plugin("selected");

        install_plugin_to_directories(std::slice::from_ref(selected), &plugin).unwrap();
        let status = inspect_game_installations(&installations, Some(&plugin)).unwrap();

        assert!(status.source_available);
        assert_eq!(
            status.games,
            vec![
                ModsPluginGameStatus {
                    region: ModsPluginGameRegion::China,
                    installed: true,
                    current: true,
                },
                ModsPluginGameStatus {
                    region: ModsPluginGameRegion::Global,
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
    fn deployment_preserves_a_managed_plugin_replaced_outside_the_tool() {
        let directory = deployment_test_directory("changed");
        let directories = vec![directory.clone()];
        install_plugin_to_directories(&directories, &managed_plugin("original")).unwrap();
        fs::write(directory.join(PLUGIN_FILE_NAME), b"changed plugin").unwrap();

        remove_plugin_from_directories(&directories).unwrap();
        assert!(directory.join(PLUGIN_FILE_NAME).exists());

        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    #[cfg(feature = "gui")]
    fn managed_plugin_refresh_migrates_legacy_mod_programs() {
        for (version, equipment, combat_clock) in [
            ("v1", LEGACY_EQUIPMENT_MOD_V1, LEGACY_COMBAT_CLOCK_MOD_V1),
            ("v2", LEGACY_EQUIPMENT_MOD_V2, LEGACY_COMBAT_CLOCK_MOD_V2),
            ("v3", LEGACY_EQUIPMENT_MOD_V3, LEGACY_COMBAT_CLOCK_MOD_V3),
            ("v4", LEGACY_EQUIPMENT_MOD_V4, LEGACY_COMBAT_CLOCK_MOD_V4),
            (
                "v4-offsets",
                LEGACY_EQUIPMENT_MOD_V4_OFFSETS,
                LEGACY_COMBAT_CLOCK_MOD_V4_OFFSETS,
            ),
        ] {
            let directory = deployment_test_directory(&format!("{version}-program-migration"));
            let workspace = deployment_test_directory(&format!("{version}-software-workspace"));
            install_default_mod_files(&workspace).unwrap();
            install_legacy_managed_plugin(&directory, b"old plugin");
            let mod_directory = directory.join(MOD_DIRECTORY_NAME);
            fs::write(mod_directory.join(EQUIPMENT_MOD_FILE_NAME), equipment).unwrap();
            fs::write(mod_directory.join(COMBAT_CLOCK_MOD_FILE_NAME), combat_clock).unwrap();

            migrate_legacy_mod_workspace(std::slice::from_ref(&directory), &workspace).unwrap();
            replace_managed_plugin_directories(
                std::slice::from_ref(&directory),
                &managed_plugin("new"),
            )
            .unwrap();

            assert_eq!(
                fs::read(
                    workspace
                        .join(MOD_DIRECTORY_NAME)
                        .join(EQUIPMENT_MOD_FILE_NAME)
                )
                .unwrap(),
                EQUIPMENT_MOD
            );
            assert_eq!(
                fs::read(
                    workspace
                        .join(MOD_DIRECTORY_NAME)
                        .join(COMBAT_CLOCK_MOD_FILE_NAME)
                )
                .unwrap(),
                COMBAT_CLOCK_MOD
            );
            assert!(!directory.join(LEGACY_PLUGIN_MARKER_FILE_NAME).exists());
            assert!(!directory.join(MOD_SET_FILE_NAME).exists());
            assert!(!mod_directory.exists());
            fs::remove_dir_all(directory).unwrap();
            fs::remove_dir_all(workspace).unwrap();
        }
    }

    #[test]
    #[cfg(feature = "gui")]
    fn managed_plugin_refresh_preserves_custom_mod_programs() {
        let directory = deployment_test_directory("custom-program-refresh");
        let workspace = deployment_test_directory("custom-program-workspace");
        install_default_mod_files(&workspace).unwrap();
        install_legacy_managed_plugin(&directory, b"old plugin");
        let custom = b"nte_mod(4)\nmod(\"equipment\")\nrequires(\"viewport.tick\")\n\ndef on_viewport_tick(event):\n    value = 1\n";
        let path = directory
            .join(MOD_DIRECTORY_NAME)
            .join(EQUIPMENT_MOD_FILE_NAME);
        fs::write(&path, custom).unwrap();

        migrate_legacy_mod_workspace(std::slice::from_ref(&directory), &workspace).unwrap();
        replace_managed_plugin_directories(
            std::slice::from_ref(&directory),
            &managed_plugin("new"),
        )
        .unwrap();

        assert_eq!(
            fs::read(
                workspace
                    .join(MOD_DIRECTORY_NAME)
                    .join(EQUIPMENT_MOD_FILE_NAME)
            )
            .unwrap(),
            custom
        );
        assert!(!path.exists());
        fs::remove_dir_all(directory).unwrap();
        fs::remove_dir_all(workspace).unwrap();
    }

    #[test]
    #[cfg(feature = "gui")]
    fn managed_plugin_refresh_updates_only_enabled_directories() {
        let enabled = deployment_test_directory("refresh-enabled");
        let disabled = deployment_test_directory("refresh-disabled");
        install_plugin_to_directories(std::slice::from_ref(&enabled), &managed_plugin("old"))
            .unwrap();

        let new_plugin = managed_plugin("new");
        replace_managed_plugin_directories(std::slice::from_ref(&enabled), &new_plugin).unwrap();

        assert_eq!(
            fs::read(enabled.join(PLUGIN_FILE_NAME)).unwrap(),
            new_plugin
        );
        assert!(!enabled.join(LEGACY_PLUGIN_MARKER_FILE_NAME).exists());
        assert!(!enabled.join(MOD_SET_FILE_NAME).exists());
        assert!(!enabled.join(MOD_DIRECTORY_NAME).exists());
        assert!(!disabled.join(PLUGIN_FILE_NAME).exists());
        fs::remove_dir_all(enabled).unwrap();
        fs::remove_dir_all(disabled).unwrap();
    }
}
