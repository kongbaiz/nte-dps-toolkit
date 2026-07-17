//! Asynchronous client for the in-process equipment plugin's local named pipe.
//! The blocking pipe transaction stays on a worker thread; the UI only submits
//! validated session item IDs and polls completed responses.

use std::io;
use std::thread::{self, JoinHandle};

use crossbeam_channel::{Receiver, Sender, TryRecvError, unbounded};
use windows_sys::Win32::System::Pipes::CallNamedPipeW;

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

enum WorkerCommand {
    Request(EquipmentPluginRequest),
    Stop,
}

pub struct EquipmentPluginClient {
    sender: Sender<WorkerCommand>,
    receiver: Receiver<EquipmentPluginResponse>,
    thread: Option<JoinHandle<()>>,
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
        let (sender, command_receiver) = unbounded();
        let (response_sender, receiver) = unbounded();
        let thread = thread::spawn(move || {
            while let Ok(command) = command_receiver.recv() {
                match command {
                    WorkerCommand::Request(request) => {
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
                    }
                    WorkerCommand::Stop => return,
                }
            }
        });
        Self {
            sender,
            receiver,
            thread: Some(thread),
            next_request_id: 1,
        }
    }

    pub fn submit(&mut self, character: HtItemNetId, operation: EquipmentPluginOperation) -> u64 {
        let request_id = self.next_request_id;
        self.next_request_id = self.next_request_id.wrapping_add(1).max(1);
        self.submit_request(EquipmentPluginRequest {
            request_id,
            character,
            operation,
        });
        request_id
    }

    pub fn submit_request(&self, request: EquipmentPluginRequest) {
        self.sender
            .send(WorkerCommand::Request(request))
            .expect("equipment plugin worker must remain alive while its client exists");
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
        let _ = self.sender.send(WorkerCommand::Stop);
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
}
