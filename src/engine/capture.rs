use std::borrow::Cow;
use std::collections::hash_map::RandomState;
use std::collections::{HashMap, HashSet, VecDeque};
use std::ffi::{CStr, CString, c_char, c_int, c_uchar, c_uint};
use std::fs::File;
use std::hash::BuildHasher;
use std::io::{BufWriter, Read, Write};
use std::marker::PhantomData;
use std::net::Ipv4Addr;
use std::path::{Path, PathBuf};
use std::ptr::{self, NonNull};
use std::sync::{
    Arc, Condvar, Mutex,
    atomic::{AtomicBool, AtomicU64, Ordering},
};
use std::thread;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use chrono::{DateTime, Local};
use crossbeam_channel::{Receiver, Sender, TrySendError, bounded};
use libloading::Library;
use pcap_file::pcapng::blocks::enhanced_packet::EnhancedPacketBlock;
use pcap_file::pcapng::blocks::interface_description::{
    InterfaceDescriptionBlock, InterfaceDescriptionOption,
};
use pcap_file::pcapng::blocks::unknown::UnknownBlock;
use pcap_file::pcapng::{Block, PcapNgReader, PcapNgWriter};
use pcap_file::{DataLink, PcapError};
use serde::{
    Deserialize, Deserializer, Serialize,
    de::Error as _,
    ser::{SerializeMap, SerializeSeq},
};

use crate::engine::model::{
    AbyssEvent, AbyssHalf, CharacterInfo, CombatClockRuntimeHealth, CombatState, DpsTimeBasis,
    EmptyCurtainCharacter, EmptyCurtainItem, EmptyCurtainPlacement, EngineEvent, Hit,
    HitCharacterSource, HitDamageCorrection, HitDirection, HitFollowUp, HtItemNetId,
    ModScriptEvent, ModScriptEventPhase, PacketDebug, PacketObservation, PartyCombatState,
    TimeStopEvent, UnattributedServerDamage,
};
use crate::engine::parser::{
    AbilityCatalog, DamageRecordEncoding, ENEMY_CATALOG_PATH, EQUIPMENT_CATALOG_PATH,
    EquipmentCatalog, EquipmentKind, GAMEPLAY_EFFECT_MAPPING_PATH, GAMEPLAY_EFFECT_SEMANTICS_PATH,
    GameplayEffectSkill, ParsedEmptyCurtainEquipmentSnapshot, ParsedEquipmentSlot,
    ParsedGameplayEffect, ParsedServerDamageSettlement, SKILL_DAMAGE_DATA_PATH,
    classify_attack_type, damage_record_encoding_at, declared_character_ids_from_evidence,
    find_data_file, find_declared_character_evidence, find_final_tower_character_evidence,
    load_enemy_catalog, load_equipment_catalog, load_gameplay_effect_mapping,
    matches_shifted_bytes_at, normalize_damage_name, parse_boss_hp_updates,
    parse_client_damage_boss_update, parse_client_fight_target_updates, parse_current_hp_updates,
    parse_damage_payload, parse_empty_curtain_character_owners,
    parse_empty_curtain_compact_module_placements, parse_empty_curtain_equipment_snapshot,
    parse_empty_curtain_item_additions, parse_empty_curtain_item_removals,
    parse_empty_curtain_items, parse_equipment_slots, parse_gameplay_effects,
    parse_server_damage_settlements, qte_reaction_type, valid_item_net_id,
    validate_empty_curtain_snapshot,
};
use crate::platform::mods_plugin::{
    CombatClockQueryError, CombatClockTransitionSnapshot, query_combat_clock_transitions,
    query_mod_events,
};
use crate::storage::io_util::atomic_write_file;

use crate::engine::protocol::{
    BunchPacket, BunchReassembler, ReassembledBunch, SequencedPacket, SingleBunch, TransportPacket,
    parse_bunch_packet, parse_expected_bunch_continuations, parse_inventory_bunches,
    parse_single_bunch, parse_transport_packet, parse_verified_bunch_starts,
    reliable_bunch_channel,
};

const PCAP_ERRBUF_SIZE: usize = 256;
const DLT_EN10MB: c_int = 1;
const DLT_RAW: c_int = 12;
const DLT_IPV4: c_int = 228;
const IANA_DYNAMIC_PORT_START: u16 = 49_152;
const MIN_READABLE_TEXT_LEN: usize = 4;
const MAX_IGNORABLE_BINARY_PACKET_LEN: usize = 96;
const UNREADABLE_PROTOCOL_TEXT: &str = "未解析到可读协议文本";
const CAPTURE_SNAPLEN: u32 = 65_535;
const RAW_CAPTURE_FLUSH_INTERVAL: u64 = 256;
const NTE_COMBAT_CLOCK_BLOCK_TYPE: u32 = 0x4e54_4543;
const NTE_COMBAT_CLOCK_BLOCK_MAGIC: &[u8; 8] = b"NTECLK01";
const NTE_COMBAT_CLOCK_BLOCK_SIZE: usize = 40;
const NTE_MOD_SCRIPT_BLOCK_TYPE: u32 = 0x4e54_454d;
const NTE_MOD_SCRIPT_BLOCK_MAGIC: &[u8; 8] = b"NTEMOD01";
const NTE_MOD_SCRIPT_BLOCK_SIZE: usize = 120;
const NTE_MOD_SCRIPT_STRING_CAPACITY: usize = 32;
const NTE_MOD_SCRIPT_VALUE_CAPACITY: usize = 3;
const COMBAT_CLOCK_PAUSE_VALID: u32 = 0x1;
const FILETIME_UNIX_EPOCH_100NS: u64 = 116_444_736_000_000_000;
const FILETIME_TICKS_PER_SECOND: u64 = 10_000_000;
const COMBAT_CLOCK_POLL_INTERVAL: Duration = Duration::from_millis(100);
const COMBAT_CLOCK_PROVIDER_FAILURE_THRESHOLD: u8 = 3;
const MAX_GAMEPLAY_EFFECT_FRAGMENT_STREAMS: usize = 64;
const MAX_GAMEPLAY_EFFECT_FRAGMENT_BITS: usize = 256 * 1024 * 8;
const GAMEPLAY_EFFECT_FRAGMENT_TIMEOUT_SECONDS: f64 = 0.5;
const MAX_BUNCH_CONNECTIONS: usize = 16;
const MAX_BUNCH_FRAGMENTS_PER_CONNECTION: usize = 512;
const MAX_REASSEMBLED_BUNCH_BITS: usize = 1024 * 1024 * 8;
const MAX_BUNCH_FRAGMENT_PACKET_SPAN: i64 = 96;
const CAPTURE_FRAME_QUEUE_CAPACITY: usize = 16_384;
// The frame-count bound protects queue metadata; this independent high-water
// mark caps payload ownership at 32 MiB. A 1,500-byte Ethernet workload can
// still use the full count capacity, while large snaplen frames backpressure
// acquisition much earlier.
const CAPTURE_FRAME_QUEUE_BYTE_HIGH_WATER: usize = 32 * 1024 * 1024;
/// PCAPNG is an external trust boundary. Runtime combat history is complete,
/// but one imported file must fit explicit byte and structural budgets.
pub const MAX_PCAPNG_IMPORT_BYTES: u64 = 512 * 1024 * 1024;
pub const MAX_PCAPNG_IMPORT_BLOCKS: usize = 2_000_000;
pub const MAX_PCAPNG_IMPORT_PACKETS: usize = 500_000;
pub const MAX_PCAPNG_IMPORT_PACKET_BYTES: u64 = 512 * 1024 * 1024;
pub const MAX_PCAPNG_IMPORT_INTERFACES: usize = 256;

/// Keeps every actual PCAPNG read on the already-validated file handle inside
/// the import byte budget. Metadata is only a point-in-time preflight: another
/// writer can extend the file after `File::metadata`, so the parser itself must
/// also be bounded. Once the budget is exhausted, one byte is probed to
/// distinguish an exact-limit EOF from a concurrently-grown/oversized file;
/// no bytes beyond that probe can be consumed.
struct PcapngImportReader<R> {
    inner: R,
    remaining: u64,
    exceeded: Arc<AtomicBool>,
}

impl<R> PcapngImportReader<R> {
    fn new(inner: R, limit: u64, exceeded: Arc<AtomicBool>) -> Self {
        Self {
            inner,
            remaining: limit,
            exceeded,
        }
    }
}

impl<R: Read> Read for PcapngImportReader<R> {
    fn read(&mut self, buffer: &mut [u8]) -> std::io::Result<usize> {
        if buffer.is_empty() {
            return Ok(0);
        }
        if self.remaining > 0 {
            let remaining = usize::try_from(self.remaining).unwrap_or(usize::MAX);
            let allowed = buffer.len().min(remaining);
            let read = self.inner.read(&mut buffer[..allowed])?;
            self.remaining -= read as u64;
            return Ok(read);
        }

        let mut probe = [0_u8; 1];
        match self.inner.read(&mut probe) {
            Ok(0) => Ok(0),
            Ok(_) => {
                self.exceeded.store(true, Ordering::Relaxed);
                Err(std::io::Error::new(
                    std::io::ErrorKind::InvalidData,
                    "pcapng import byte budget exceeded",
                ))
            }
            Err(error) => Err(error),
        }
    }
}

fn map_pcapng_reader_error(
    error: PcapError,
    byte_budget_exceeded: &AtomicBool,
) -> PcapngImportError {
    if byte_budget_exceeded.load(Ordering::Relaxed) {
        return PcapngImportError::TooLarge {
            size: MAX_PCAPNG_IMPORT_BYTES.saturating_add(1),
            limit: MAX_PCAPNG_IMPORT_BYTES,
        };
    }
    match error {
        PcapError::IoError(error) => PcapngImportError::Io(error),
        error => PcapngImportError::InvalidFormat(error.to_string()),
    }
}

#[derive(Debug)]
pub enum PcapngImportError {
    NotAFile,
    TooLarge { size: u64, limit: u64 },
    TooManyBlocks { count: usize, limit: usize },
    TooManyPackets { count: usize, limit: usize },
    TooManyInterfaces { count: usize, limit: usize },
    SnaplenTooLarge { snaplen: u32, limit: u32 },
    FrameTooLarge { size: usize, limit: usize },
    PacketBytesExceeded { size: u64, limit: u64 },
    InvalidFormat(String),
    ReceiverDisconnected,
    Io(std::io::Error),
}

impl std::fmt::Display for PcapngImportError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotAFile => formatter.write_str("import path is not a regular file"),
            Self::TooLarge { size, limit } => write!(
                formatter,
                "pcapng file is too large ({size} bytes; limit {limit} bytes)"
            ),
            Self::TooManyBlocks { count, limit } => write!(
                formatter,
                "pcapng has too many blocks ({count}; limit {limit})"
            ),
            Self::TooManyPackets { count, limit } => write!(
                formatter,
                "pcapng has too many packets ({count}; limit {limit})"
            ),
            Self::TooManyInterfaces { count, limit } => write!(
                formatter,
                "pcapng has too many interfaces ({count}; limit {limit})"
            ),
            Self::SnaplenTooLarge { snaplen, limit } => write!(
                formatter,
                "pcapng interface snaplen exceeds budget ({snaplen}; limit {limit})"
            ),
            Self::FrameTooLarge { size, limit } => write!(
                formatter,
                "pcapng frame exceeds snaplen ({size} bytes; limit {limit} bytes)"
            ),
            Self::PacketBytesExceeded { size, limit } => write!(
                formatter,
                "pcapng packet bytes exceed budget ({size}; limit {limit})"
            ),
            Self::InvalidFormat(detail) => write!(formatter, "invalid pcapng: {detail}"),
            Self::ReceiverDisconnected => formatter.write_str("engine event receiver disconnected"),
            Self::Io(error) => write!(formatter, "cannot read pcapng: {error}"),
        }
    }
}

impl std::error::Error for PcapngImportError {}

pub fn validate_pcapng_import(path: &Path) -> Result<(), PcapngImportError> {
    validate_pcapng_import_with_limit(path, MAX_PCAPNG_IMPORT_BYTES)
}

fn validate_pcapng_import_with_limit(path: &Path, limit: u64) -> Result<(), PcapngImportError> {
    let metadata = std::fs::metadata(path).map_err(PcapngImportError::Io)?;
    if !metadata.is_file() {
        return Err(PcapngImportError::NotAFile);
    }
    let size = metadata.len();
    if size > limit {
        return Err(PcapngImportError::TooLarge { size, limit });
    }
    Ok(())
}

fn account_pcapng_frame(
    size: usize,
    packet_count: &mut usize,
    packet_bytes: &mut u64,
) -> Result<(), PcapngImportError> {
    if size > CAPTURE_SNAPLEN as usize {
        return Err(PcapngImportError::FrameTooLarge {
            size,
            limit: CAPTURE_SNAPLEN as usize,
        });
    }
    *packet_count = packet_count.saturating_add(1);
    if *packet_count > MAX_PCAPNG_IMPORT_PACKETS {
        return Err(PcapngImportError::TooManyPackets {
            count: *packet_count,
            limit: MAX_PCAPNG_IMPORT_PACKETS,
        });
    }
    *packet_bytes = packet_bytes.saturating_add(size as u64);
    if *packet_bytes > MAX_PCAPNG_IMPORT_PACKET_BYTES {
        return Err(PcapngImportError::PacketBytesExceeded {
            size: *packet_bytes,
            limit: MAX_PCAPNG_IMPORT_PACKET_BYTES,
        });
    }
    Ok(())
}

fn account_pcapng_interface(
    snaplen: u32,
    interface_count: &mut usize,
) -> Result<(), PcapngImportError> {
    *interface_count = interface_count.saturating_add(1);
    if *interface_count > MAX_PCAPNG_IMPORT_INTERFACES {
        return Err(PcapngImportError::TooManyInterfaces {
            count: *interface_count,
            limit: MAX_PCAPNG_IMPORT_INTERFACES,
        });
    }
    // PCAPNG uses zero to mean "unlimited", which is also outside the
    // importer's fixed frame/snaplen contract.
    if snaplen == 0 || snaplen > CAPTURE_SNAPLEN {
        return Err(PcapngImportError::SnaplenTooLarge {
            snaplen,
            limit: CAPTURE_SNAPLEN,
        });
    }
    Ok(())
}

struct CaptureFrame {
    data: Vec<u8>,
    timestamp: f64,
    _byte_reservation: Option<CaptureFrameByteReservation>,
}

impl CaptureFrame {
    fn new(data: Vec<u8>, timestamp: f64) -> Self {
        Self {
            data,
            timestamp,
            _byte_reservation: None,
        }
    }
}

#[derive(Clone)]
struct CaptureFrameSender {
    frames: Sender<CaptureFrame>,
    byte_budget: Arc<CaptureFrameByteBudget>,
}

struct CaptureFrameReceiver {
    frames: Receiver<CaptureFrame>,
    byte_budget: Arc<CaptureFrameByteBudget>,
}

struct CaptureFrameByteBudget {
    high_water: usize,
    state: Mutex<CaptureFrameByteBudgetState>,
    available: Condvar,
}

struct CaptureFrameByteBudgetState {
    reserved: usize,
    observed_high_water: usize,
    receiver_connected: bool,
}

struct CaptureFrameByteReservation {
    byte_budget: Arc<CaptureFrameByteBudget>,
    bytes: usize,
}

fn capture_frame_queue(
    frame_capacity: usize,
    byte_high_water: usize,
) -> (CaptureFrameSender, CaptureFrameReceiver) {
    let (frames, receiver) = bounded(frame_capacity);
    let byte_budget = Arc::new(CaptureFrameByteBudget {
        high_water: byte_high_water,
        state: Mutex::new(CaptureFrameByteBudgetState {
            reserved: 0,
            observed_high_water: 0,
            receiver_connected: true,
        }),
        available: Condvar::new(),
    });
    (
        CaptureFrameSender {
            frames,
            byte_budget: Arc::clone(&byte_budget),
        },
        CaptureFrameReceiver {
            frames: receiver,
            byte_budget,
        },
    )
}

impl CaptureFrameSender {
    fn send(&self, mut frame: CaptureFrame) -> Result<(), String> {
        let reservation = self.byte_budget.reserve(frame.data.capacity())?;
        frame._byte_reservation = Some(reservation);
        self.frames
            .send(frame)
            .map_err(|_| "capture parser thread stopped unexpectedly".to_owned())
    }

    fn byte_high_water_mark(&self) -> usize {
        self.byte_budget.observed_high_water()
    }
}

impl CaptureFrameReceiver {
    fn recv(&self) -> Result<CaptureFrame, crossbeam_channel::RecvError> {
        self.frames.recv()
    }
}

impl Drop for CaptureFrameReceiver {
    fn drop(&mut self) {
        self.byte_budget.disconnect_receiver();
    }
}

impl CaptureFrameByteBudget {
    fn reserve(self: &Arc<Self>, bytes: usize) -> Result<CaptureFrameByteReservation, String> {
        if bytes > self.high_water {
            return Err(format!(
                "capture frame size {bytes} exceeds parser queue byte budget {}",
                self.high_water
            ));
        }
        let mut state = self.lock_state()?;
        while state.receiver_connected && bytes > self.high_water - state.reserved {
            state = match self.available.wait(state) {
                Ok(state) => state,
                Err(mut error) => {
                    error.get_mut().receiver_connected = false;
                    self.state.clear_poison();
                    self.available.notify_all();
                    return Err("capture frame queue byte budget became unavailable".to_owned());
                }
            };
        }
        if !state.receiver_connected {
            return Err("capture parser thread stopped unexpectedly".to_owned());
        }
        state.reserved += bytes;
        state.observed_high_water = state.observed_high_water.max(state.reserved);
        Ok(CaptureFrameByteReservation {
            byte_budget: Arc::clone(self),
            bytes,
        })
    }

    fn lock_state(&self) -> Result<std::sync::MutexGuard<'_, CaptureFrameByteBudgetState>, String> {
        match self.state.lock() {
            Ok(state) => Ok(state),
            Err(mut error) => {
                error.get_mut().receiver_connected = false;
                self.state.clear_poison();
                self.available.notify_all();
                Err("capture frame queue byte budget became unavailable".to_owned())
            }
        }
    }

    fn release(&self, bytes: usize) {
        match self.state.lock() {
            Ok(mut state) => {
                state.reserved = state.reserved.saturating_sub(bytes);
            }
            Err(mut error) => {
                let state = error.get_mut();
                state.reserved = 0;
                state.receiver_connected = false;
                self.state.clear_poison();
            }
        }
        self.available.notify_all();
    }

    fn disconnect_receiver(&self) {
        match self.state.lock() {
            Ok(mut state) => state.receiver_connected = false,
            Err(mut error) => {
                error.get_mut().receiver_connected = false;
                self.state.clear_poison();
            }
        }
        self.available.notify_all();
    }

    fn observed_high_water(&self) -> usize {
        self.state
            .lock()
            .map_or(self.high_water, |state| state.observed_high_water)
    }
}

impl Drop for CaptureFrameByteReservation {
    fn drop(&mut self) {
        self.byte_budget.release(self.bytes);
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum CaptureLinkType {
    Ethernet,
    RawIpv4,
    Ipv4,
}

impl CaptureLinkType {
    fn from_npcap_datalink(data_link: c_int) -> Result<Self, String> {
        match data_link {
            DLT_EN10MB => Ok(Self::Ethernet),
            DLT_RAW => Ok(Self::RawIpv4),
            DLT_IPV4 => Ok(Self::Ipv4),
            unsupported => Err(format!(
                "unsupported Npcap data link type {unsupported}; supported types are DLT_EN10MB ({DLT_EN10MB}), DLT_RAW ({DLT_RAW}), and DLT_IPV4 ({DLT_IPV4})"
            )),
        }
    }

    fn from_pcapng(data_link: DataLink) -> Option<Self> {
        match data_link {
            DataLink::ETHERNET => Some(Self::Ethernet),
            // LINKTYPE_RAW can contain IPv4 or IPv6, while LINKTYPE_IPV4 is the
            // IPv4-only form emitted by some capture stacks for layer-3 TUN
            // interfaces. Both carry the IPv4 header at byte zero.
            DataLink::RAW => Some(Self::RawIpv4),
            DataLink::IPV4 => Some(Self::Ipv4),
            _ => None,
        }
    }

    fn pcapng_data_link(self) -> DataLink {
        match self {
            Self::Ethernet => DataLink::ETHERNET,
            Self::RawIpv4 => DataLink::RAW,
            Self::Ipv4 => DataLink::IPV4,
        }
    }

    fn label(self) -> &'static str {
        match self {
            Self::Ethernet => "Ethernet",
            Self::RawIpv4 => "raw IPv4",
            Self::Ipv4 => "IPv4",
        }
    }
}

#[repr(C)]
struct PcapIf {
    next: *mut PcapIf,
    name: *mut c_char,
    description: *mut c_char,
    addresses: *mut PcapAddr,
    flags: c_uint,
}

#[repr(C)]
struct PcapAddr {
    next: *mut PcapAddr,
    addr: *mut SockAddr,
    netmask: *mut SockAddr,
    broadaddr: *mut SockAddr,
    dstaddr: *mut SockAddr,
}

#[repr(C)]
struct SockAddr {
    family: u16,
    data: [u8; 14],
}

#[repr(C)]
struct TimeVal {
    tv_sec: i32,
    tv_usec: i32,
}

#[repr(C)]
struct PcapPkthdr {
    ts: TimeVal,
    caplen: c_uint,
    len: c_uint,
}

#[repr(C)]
struct BpfProgram {
    bf_len: c_uint,
    bf_insns: *mut std::ffi::c_void,
}

type PcapT = std::ffi::c_void;
type FindAllDevs = unsafe extern "C" fn(*mut *mut PcapIf, *mut c_char) -> c_int;
type FreeAllDevs = unsafe extern "C" fn(*mut PcapIf);
type OpenLive = unsafe extern "C" fn(*const c_char, c_int, c_int, c_int, *mut c_char) -> *mut PcapT;
type NextEx =
    unsafe extern "C" fn(*mut PcapT, *mut *const PcapPkthdr, *mut *const c_uchar) -> c_int;
type Close = unsafe extern "C" fn(*mut PcapT);
type Compile =
    unsafe extern "C" fn(*mut PcapT, *mut BpfProgram, *const c_char, c_int, c_uint) -> c_int;
type SetFilter = unsafe extern "C" fn(*mut PcapT, *mut BpfProgram) -> c_int;
type FreeCode = unsafe extern "C" fn(*mut BpfProgram);
type GetErr = unsafe extern "C" fn(*mut PcapT) -> *const c_char;
type PcapDataLink = unsafe extern "C" fn(*mut PcapT) -> c_int;

struct PcapHandle<'library> {
    raw: NonNull<PcapT>,
    close: Close,
    _library: PhantomData<&'library Library>,
}

impl<'library> PcapHandle<'library> {
    /// # Safety
    ///
    /// `raw` must be an exclusively owned handle returned by `pcap_open_live`,
    /// `close` must come from the same `library`, and no other owner may close
    /// the handle. The lifetime marker keeps that DLL loaded through Drop.
    unsafe fn from_raw(raw: NonNull<PcapT>, close: Close, _library: &'library Library) -> Self {
        Self {
            raw,
            close,
            _library: PhantomData,
        }
    }

    fn as_ptr(&self) -> *mut PcapT {
        self.raw.as_ptr()
    }
}

impl Drop for PcapHandle<'_> {
    fn drop(&mut self) {
        // SAFETY: `from_raw` accepts exclusive ownership of a live handle and
        // ties the matching close symbol to its loaded library until this Drop.
        unsafe {
            (self.close)(self.raw.as_ptr());
        }
    }
}

struct BpfProgramGuard<'library> {
    program: BpfProgram,
    free_code: FreeCode,
    compiled: bool,
    _library: PhantomData<&'library Library>,
}

impl<'library> BpfProgramGuard<'library> {
    fn new(free_code: FreeCode, _library: &'library Library) -> Self {
        Self {
            program: BpfProgram {
                bf_len: 0,
                bf_insns: ptr::null_mut(),
            },
            free_code,
            compiled: false,
            _library: PhantomData,
        }
    }

    fn as_mut_ptr(&mut self) -> *mut BpfProgram {
        &mut self.program
    }

    /// # Safety
    ///
    /// The preceding `pcap_compile` call must have returned success after
    /// initializing this exact program through `as_mut_ptr`.
    unsafe fn mark_compiled(&mut self) {
        self.compiled = true;
    }

    fn release(&mut self) {
        if self.compiled {
            // SAFETY: `compiled` is set only after successful pcap_compile;
            // the matching symbol's library is retained by the lifetime marker.
            unsafe {
                (self.free_code)(&mut self.program);
            }
            self.compiled = false;
        }
    }
}

impl Drop for BpfProgramGuard<'_> {
    fn drop(&mut self) {
        self.release();
    }
}

#[derive(Clone, Debug)]
pub struct CaptureDevice {
    pub name: String,
    pub description: String,
    pub ipv4: Vec<Ipv4Addr>,
}

pub struct CaptureHandle {
    stop: Arc<AtomicBool>,
    thread: Option<thread::JoinHandle<()>>,
    raw_capture: RawCaptureBuffer,
}

pub struct CaptureOutput {
    pub raw_capture_directory: Option<PathBuf>,
    pub packet_emission: PacketEmissionMode,
    pub sender: EngineEventSink,
}

#[derive(Clone)]
pub struct CaptureResources {
    pub characters: Arc<HashMap<u32, CharacterInfo>>,
    pub ability_catalog: Arc<AbilityCatalog>,
}

/// Frontend-neutral event output. Semantic events use the reliable lane; full
/// debug packets use an optional bounded lane and are discarded at the producer
/// when that lane is full, before their large payload strings can accumulate.
#[derive(Clone)]
pub struct EngineEventSink {
    reliable: Sender<EngineEvent>,
    debug: Option<Sender<EngineEvent>>,
    dropped_debug_packets: Arc<AtomicU64>,
    delivery_gate: Option<Arc<EngineEventDeliveryGate>>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum EngineEventDeliveryState {
    Pending,
    Released,
    Cancelled,
}

struct EngineEventDeliveryGate {
    state: Mutex<EngineEventDeliveryState>,
    ready: Condvar,
}

/// One-shot owner for a producer delivery barrier. The capture/replay producer
/// may be started before an authoritative session transaction commits, but no
/// event can cross the sink until this permit is explicitly released. Dropping
/// the permit cancels delivery and wakes blocked producers fail-closed.
pub struct EngineEventDeliveryPermit {
    gate: Option<Arc<EngineEventDeliveryGate>>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct EngineEventSendError;

impl std::fmt::Display for EngineEventSendError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("engine event receiver disconnected")
    }
}

impl std::error::Error for EngineEventSendError {}

impl EngineEventSink {
    pub fn reliable(sender: Sender<EngineEvent>) -> Self {
        Self {
            reliable: sender,
            debug: None,
            dropped_debug_packets: Arc::new(AtomicU64::new(0)),
            delivery_gate: None,
        }
    }

    pub fn split(reliable: Sender<EngineEvent>, debug: Sender<EngineEvent>) -> Self {
        Self {
            reliable,
            debug: Some(debug),
            dropped_debug_packets: Arc::new(AtomicU64::new(0)),
            delivery_gate: None,
        }
    }

    /// Returns a cloneable sink whose producers block before their first event
    /// until the paired permit is released or cancelled.
    pub fn pause_delivery(mut self) -> (Self, EngineEventDeliveryPermit) {
        let gate = Arc::new(EngineEventDeliveryGate {
            state: Mutex::new(EngineEventDeliveryState::Pending),
            ready: Condvar::new(),
        });
        self.delivery_gate = Some(Arc::clone(&gate));
        (self, EngineEventDeliveryPermit { gate: Some(gate) })
    }

    pub fn send(&self, event: EngineEvent) -> Result<(), EngineEventSendError> {
        if let Some(gate) = &self.delivery_gate {
            gate.wait_until_ready()?;
        }
        if !event.is_droppable_debug_packet() {
            return self.reliable.send(event).map_err(|_| EngineEventSendError);
        }
        let Some(debug) = &self.debug else {
            return self.reliable.send(event).map_err(|_| EngineEventSendError);
        };
        match debug.try_send(event) {
            Ok(()) => Ok(()),
            Err(TrySendError::Full(_)) => {
                let _ = self.dropped_debug_packets.fetch_update(
                    Ordering::Relaxed,
                    Ordering::Relaxed,
                    |count| Some(count.saturating_add(1)),
                );
                Ok(())
            }
            Err(TrySendError::Disconnected(_)) => Err(EngineEventSendError),
        }
    }

    pub fn take_dropped_debug_packets(&self) -> u64 {
        self.dropped_debug_packets.swap(0, Ordering::Relaxed)
    }

    pub fn pending_len(&self) -> usize {
        self.reliable
            .len()
            .saturating_add(self.debug.as_ref().map_or(0, Sender::len))
    }
}

impl From<Sender<EngineEvent>> for EngineEventSink {
    fn from(sender: Sender<EngineEvent>) -> Self {
        Self::reliable(sender)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PacketEmissionMode {
    FullDebug,
    SummaryOnly,
}

impl CaptureHandle {
    #[cfg(test)]
    pub(crate) fn from_test_thread(stop: Arc<AtomicBool>, thread: thread::JoinHandle<()>) -> Self {
        Self {
            stop,
            thread: Some(thread),
            raw_capture: RawCaptureBuffer::new(None),
        }
    }

    pub fn raw_capture(&self) -> RawCaptureBuffer {
        self.raw_capture.clone()
    }

    pub fn stop(&mut self) {
        self.stop_with_drain(|| {});
    }

    /// Stop while allowing a bounded event consumer to make progress. GUI
    /// callers use this to release a parser blocked on the reliable lane before
    /// joining its thread; unbounded consumers can keep using [`Self::stop`].
    pub fn stop_with_drain(&mut self, mut drain: impl FnMut()) {
        self.stop.store(true, Ordering::Relaxed);
        while self
            .thread
            .as_ref()
            .is_some_and(|thread| !thread.is_finished())
        {
            drain();
            thread::sleep(Duration::from_millis(1));
        }
        drain();
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

impl Drop for CaptureHandle {
    fn drop(&mut self) {
        self.stop();
    }
}

#[derive(Clone)]
pub struct RawCaptureBuffer {
    inner: Arc<Mutex<RawCaptureData>>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct RawCaptureSnapshot {
    pub path: Option<PathBuf>,
    pub packet_count: u64,
    pub captured_bytes: u64,
    pub write_error: bool,
    pub writing: bool,
}

struct RawCaptureData {
    path: Option<PathBuf>,
    writer: Option<RawCaptureWriter>,
    packet_count: u64,
    captured_bytes: u64,
    write_error: Option<String>,
}

impl RawCaptureBuffer {
    fn new(directory: Option<&std::path::Path>) -> Self {
        let timestamp = Local::now().format("%Y%m%d_%H%M%S_%3f");
        let path = directory.map(|directory| directory.join(format!("nte_raw_{timestamp}.pcapng")));
        Self {
            inner: Arc::new(Mutex::new(RawCaptureData {
                path,
                writer: None,
                packet_count: 0,
                captured_bytes: 0,
                write_error: None,
            })),
        }
    }

    fn initialize(&self, device: &CaptureDevice, link_type: CaptureLinkType) {
        let path = self.inner.lock().ok().and_then(|capture| {
            if capture.writer.is_none() && capture.write_error.is_none() {
                capture.path.clone()
            } else {
                None
            }
        });
        let Some(path) = path else {
            return;
        };
        let result = RawCaptureWriter::create(&path, device, link_type);
        let Ok(mut capture) = self.inner.lock() else {
            return;
        };
        match result {
            Ok(writer) => capture.writer = Some(writer),
            Err(error) => capture.write_error = Some(error),
        }
    }

    fn push(&self, timestamp: Duration, original_len: u32, packet: &[u8]) {
        if let Ok(mut capture) = self.inner.lock() {
            let result = capture
                .writer
                .as_mut()
                .map(|writer| writer.write_packet(timestamp, original_len, packet));
            match result {
                Some(Ok(())) => {
                    capture.packet_count += 1;
                    capture.captured_bytes += packet.len() as u64;
                }
                Some(Err(error)) => {
                    capture.write_error = Some(error);
                    capture.writer = None;
                }
                None => {}
            }
        }
    }

    fn push_combat_clock_transition(&self, transition: &CombatClockTransitionSnapshot) {
        if let Ok(mut capture) = self.inner.lock() {
            let result = capture
                .writer
                .as_mut()
                .map(|writer| writer.write_combat_clock_transition(transition));
            if let Some(Err(error)) = result {
                capture.write_error = Some(error);
                capture.writer = None;
            }
        }
    }

    fn push_mod_script_event(&self, event: &ModScriptEvent) {
        if let Ok(mut capture) = self.inner.lock() {
            let result = capture
                .writer
                .as_mut()
                .map(|writer| writer.write_mod_script_event(event));
            if let Some(Err(error)) = result {
                capture.write_error = Some(error);
                capture.writer = None;
            }
        }
    }

    pub fn packet_count(&self) -> usize {
        self.inner
            .lock()
            .map_or(0, |capture| capture.packet_count as usize)
    }

    pub fn snapshot(&self) -> RawCaptureSnapshot {
        self.inner.lock().map_or_else(
            |_| RawCaptureSnapshot {
                write_error: true,
                ..RawCaptureSnapshot::default()
            },
            |capture| RawCaptureSnapshot {
                path: capture.path.clone(),
                packet_count: capture.packet_count,
                captured_bytes: capture.captured_bytes,
                write_error: capture.write_error.is_some(),
                writing: capture.writer.is_some(),
            },
        )
    }

    pub fn path(&self) -> Option<PathBuf> {
        self.inner
            .lock()
            .ok()
            .filter(|capture| capture.write_error.is_none())
            .and_then(|capture| capture.path.clone())
    }

    fn finish(&self) {
        let Ok(mut capture) = self.inner.lock() else {
            return;
        };
        if let Some(writer) = capture.writer.take()
            && let Err(error) = writer.finish()
        {
            capture.write_error = Some(error);
        }
    }

    pub fn save(&self, path: &std::path::Path) -> Result<(u64, u64), String> {
        let capture = self
            .inner
            .lock()
            .map_err(|_| "raw capture lock poisoned".to_owned())?;
        if capture.writer.is_some() {
            return Err("raw capture is still being written; stop capture first".to_owned());
        }
        if let Some(error) = &capture.write_error {
            return Err(format!("raw capture write failed: {error}"));
        }
        let source = capture
            .path
            .as_ref()
            .ok_or_else(|| "raw capture is disabled".to_owned())?;
        if path != source {
            std::fs::copy(source, path).map_err(|error| {
                format!(
                    "failed to copy raw capture {} to {}: {error}",
                    source.display(),
                    path.display()
                )
            })?;
        }
        Ok((capture.packet_count, capture.captured_bytes))
    }
}

struct RawCaptureWriter {
    writer: PcapNgWriter<BufWriter<File>>,
    packet_count: u64,
    captured_bytes: u64,
}

impl RawCaptureWriter {
    fn create(
        path: &std::path::Path,
        device: &CaptureDevice,
        link_type: CaptureLinkType,
    ) -> Result<Self, String> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|error| {
                format!(
                    "failed to create raw capture directory {}: {error}",
                    parent.display()
                )
            })?;
        }
        let file = File::create(path).map_err(|error| {
            format!(
                "failed to create raw capture file {}: {error}",
                path.display()
            )
        })?;
        let mut writer =
            PcapNgWriter::new(BufWriter::new(file)).map_err(|error| error.to_string())?;
        let mut interface =
            InterfaceDescriptionBlock::new(link_type.pcapng_data_link(), CAPTURE_SNAPLEN);
        interface
            .options
            .push(InterfaceDescriptionOption::IfName(Cow::Owned(
                device.name.clone(),
            )));
        if !device.description.is_empty() {
            interface
                .options
                .push(InterfaceDescriptionOption::IfDescription(Cow::Owned(
                    device.description.clone(),
                )));
        }
        // EnhancedPacketBlock stores Duration as nanoseconds.
        interface
            .options
            .push(InterfaceDescriptionOption::IfTsResol(9));
        writer
            .write_pcapng_block(interface)
            .map_err(|error| error.to_string())?;
        Ok(Self {
            writer,
            packet_count: 0,
            captured_bytes: 0,
        })
    }

    fn write_packet(
        &mut self,
        timestamp: Duration,
        original_len: u32,
        packet: &[u8],
    ) -> Result<(), String> {
        self.writer
            .write_pcapng_block(EnhancedPacketBlock {
                interface_id: 0,
                timestamp,
                original_len,
                data: Cow::Borrowed(packet),
                options: Vec::new(),
            })
            .map_err(|error| error.to_string())?;
        self.packet_count += 1;
        self.captured_bytes += packet.len() as u64;
        if self.packet_count & (RAW_CAPTURE_FLUSH_INTERVAL - 1) == 0 {
            self.writer
                .get_mut()
                .flush()
                .map_err(|error| error.to_string())?;
        }
        Ok(())
    }

    fn write_combat_clock_transition(
        &mut self,
        transition: &CombatClockTransitionSnapshot,
    ) -> Result<(), String> {
        let payload = encode_combat_clock_block(transition);
        self.writer
            .write_pcapng_block(UnknownBlock::new(NTE_COMBAT_CLOCK_BLOCK_TYPE, 0, &payload))
            .map(|_| ())
            .map_err(|error| error.to_string())
    }

    fn write_mod_script_event(&mut self, event: &ModScriptEvent) -> Result<(), String> {
        let payload = encode_mod_script_block(event)
            .ok_or_else(|| "invalid ModScript event for raw capture".to_owned())?;
        self.writer
            .write_pcapng_block(UnknownBlock::new(NTE_MOD_SCRIPT_BLOCK_TYPE, 0, &payload))
            .map(|_| ())
            .map_err(|error| error.to_string())
    }

    fn finish(mut self) -> Result<(u64, u64), String> {
        self.writer
            .get_mut()
            .flush()
            .map_err(|error| error.to_string())?;
        Ok((self.packet_count, self.captured_bytes))
    }
}

fn encode_combat_clock_block(
    transition: &CombatClockTransitionSnapshot,
) -> [u8; NTE_COMBAT_CLOCK_BLOCK_SIZE] {
    let mut payload = [0_u8; NTE_COMBAT_CLOCK_BLOCK_SIZE];
    payload[0..8].copy_from_slice(NTE_COMBAT_CLOCK_BLOCK_MAGIC);
    payload[8..16].copy_from_slice(&transition.sequence.to_le_bytes());
    payload[16..24].copy_from_slice(&transition.timestamp_100ns.to_le_bytes());
    payload[24..28].copy_from_slice(&transition.pause_type_mask.to_le_bytes());
    payload[28..32].copy_from_slice(&transition.reserved_value.to_le_bytes());
    payload[32..36].copy_from_slice(&transition.state_flags.to_le_bytes());
    payload
}

fn decode_combat_clock_block(value: &[u8]) -> Option<CombatClockTransitionSnapshot> {
    if value.len() != NTE_COMBAT_CLOCK_BLOCK_SIZE
        || &value[0..8] != NTE_COMBAT_CLOCK_BLOCK_MAGIC
        || value[36..40] != [0; 4]
    {
        return None;
    }
    let pause_type_mask =
        u32::from_le_bytes(value[24..28].try_into().expect("fixed pause type mask"));
    let reserved_value =
        i32::from_le_bytes(value[28..32].try_into().expect("fixed reserved value"));
    let state_flags =
        u32::from_le_bytes(value[32..36].try_into().expect("fixed clock state flags"));
    if reserved_value != 0
        || state_flags & !COMBAT_CLOCK_PAUSE_VALID != 0
        || pause_type_mask & !0x1c != 0
        || state_flags & COMBAT_CLOCK_PAUSE_VALID == 0 && pause_type_mask != 0
    {
        return None;
    }
    Some(CombatClockTransitionSnapshot {
        sequence: u64::from_le_bytes(value[8..16].try_into().expect("fixed transition sequence")),
        timestamp_100ns: u64::from_le_bytes(
            value[16..24]
                .try_into()
                .expect("fixed transition timestamp"),
        ),
        pause_type_mask,
        reserved_value,
        state_flags,
    })
}

fn encode_mod_script_block(event: &ModScriptEvent) -> Option<[u8; NTE_MOD_SCRIPT_BLOCK_SIZE]> {
    if !valid_mod_script_block_identifier(&event.mod_id)
        || !valid_mod_script_block_identifier(&event.name)
        || event.values.len() > NTE_MOD_SCRIPT_VALUE_CAPACITY
        || event.timestamp_100ns < FILETIME_UNIX_EPOCH_100NS
    {
        return None;
    }
    let mut payload = [0_u8; NTE_MOD_SCRIPT_BLOCK_SIZE];
    payload[0..8].copy_from_slice(NTE_MOD_SCRIPT_BLOCK_MAGIC);
    payload[8..16].copy_from_slice(&event.sequence.to_le_bytes());
    payload[16..24].copy_from_slice(&event.timestamp_100ns.to_le_bytes());
    payload[24] = match event.phase {
        ModScriptEventPhase::Event => 0,
        ModScriptEventPhase::Preprocess => 1,
        ModScriptEventPhase::Postprocess => 2,
    };
    payload[25] = event.values.len() as u8;
    payload[26] = event.mod_id.len() as u8;
    payload[27] = event.name.len() as u8;
    for (index, value) in event.values.iter().enumerate() {
        let start = 32 + index * 8;
        payload[start..start + 8].copy_from_slice(&value.to_le_bytes());
    }
    payload[56..56 + event.mod_id.len()].copy_from_slice(event.mod_id.as_bytes());
    payload[88..88 + event.name.len()].copy_from_slice(event.name.as_bytes());
    Some(payload)
}

fn decode_mod_script_block(value: &[u8]) -> Option<ModScriptEvent> {
    if value.len() != NTE_MOD_SCRIPT_BLOCK_SIZE
        || &value[0..8] != NTE_MOD_SCRIPT_BLOCK_MAGIC
        || value[28..32] != [0; 4]
    {
        return None;
    }
    let phase = match value[24] {
        0 => ModScriptEventPhase::Event,
        1 => ModScriptEventPhase::Preprocess,
        2 => ModScriptEventPhase::Postprocess,
        _ => return None,
    };
    let value_count = usize::from(value[25]);
    let mod_id_length = usize::from(value[26]);
    let name_length = usize::from(value[27]);
    let timestamp_100ns =
        u64::from_le_bytes(value[16..24].try_into().expect("fixed ModScript timestamp"));
    if value_count > NTE_MOD_SCRIPT_VALUE_CAPACITY
        || mod_id_length == 0
        || mod_id_length >= NTE_MOD_SCRIPT_STRING_CAPACITY
        || name_length == 0
        || name_length >= NTE_MOD_SCRIPT_STRING_CAPACITY
        || timestamp_100ns < FILETIME_UNIX_EPOCH_100NS
        || value[32 + value_count * 8..56]
            .iter()
            .any(|byte| *byte != 0)
        || value[56 + mod_id_length..88].iter().any(|byte| *byte != 0)
        || value[88 + name_length..120].iter().any(|byte| *byte != 0)
    {
        return None;
    }
    let mod_id = std::str::from_utf8(&value[56..56 + mod_id_length]).ok()?;
    let name = std::str::from_utf8(&value[88..88 + name_length]).ok()?;
    if !valid_mod_script_block_identifier(mod_id) || !valid_mod_script_block_identifier(name) {
        return None;
    }
    let values = value[32..32 + value_count * 8]
        .as_chunks::<8>()
        .0
        .iter()
        .map(|bytes| u64::from_le_bytes(*bytes))
        .collect();
    Some(ModScriptEvent {
        sequence: u64::from_le_bytes(value[8..16].try_into().expect("fixed ModScript sequence")),
        timestamp_100ns,
        mod_id: mod_id.to_owned(),
        phase,
        name: name.to_owned(),
        values,
        enemy_identity: None,
    })
}

fn valid_mod_script_block_identifier(value: &str) -> bool {
    !value.is_empty()
        && value.len() < NTE_MOD_SCRIPT_STRING_CAPACITY
        && value.bytes().all(|byte| {
            byte.is_ascii_lowercase() || byte.is_ascii_digit() || matches!(byte, b'.' | b'_' | b'-')
        })
}

fn filetime_100ns_to_unix_seconds(timestamp_100ns: u64) -> Option<f64> {
    let timestamp_ticks = timestamp_100ns.checked_sub(FILETIME_UNIX_EPOCH_100NS)?;
    Some(timestamp_ticks as f64 / FILETIME_TICKS_PER_SECOND as f64)
}

fn current_filetime_100ns() -> u64 {
    let elapsed = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system time must be after the Unix epoch");
    FILETIME_UNIX_EPOCH_100NS
        + elapsed.as_secs() * FILETIME_TICKS_PER_SECOND
        + u64::from(elapsed.subsec_nanos()) / 100
}

#[derive(Default)]
struct GamePauseIntervalTracker {
    active_type_mask: u32,
    pause_type_mask: u32,
}

impl GamePauseIntervalTracker {
    fn apply_transition(&mut self, timestamp: f64, pause_type_mask: u32) -> Option<TimeStopEvent> {
        let event = if self.pause_type_mask == 0 && pause_type_mask != 0 {
            self.active_type_mask = pause_type_mask;
            Some(TimeStopEvent::GamePauseStarted {
                timestamp,
                pause_type_mask,
            })
        } else if self.pause_type_mask != 0 && pause_type_mask == 0 {
            Some(TimeStopEvent::GamePauseEnded {
                timestamp,
                pause_type_mask: self.active_type_mask,
            })
        } else {
            self.active_type_mask |= pause_type_mask;
            None
        };
        self.pause_type_mask = pause_type_mask;
        event
    }
}

fn send_game_pause_transition(
    sender: &EngineEventSink,
    event: TimeStopEvent,
) -> Result<(), EngineEventSendError> {
    sender.send(EngineEvent::TimeStop(event))
}

fn combat_clock_error_health(error: CombatClockQueryError) -> CombatClockRuntimeHealth {
    match error {
        CombatClockQueryError::ProviderUnavailable => CombatClockRuntimeHealth::ProviderUnavailable,
        CombatClockQueryError::ModDisabled => CombatClockRuntimeHealth::ModDisabled,
        CombatClockQueryError::InvalidResponse => CombatClockRuntimeHealth::InvalidResponse,
    }
}

fn combat_clock_sample_health(state_flags: u32, recorded: bool) -> CombatClockRuntimeHealth {
    if state_flags & COMBAT_CLOCK_PAUSE_VALID == 0 {
        CombatClockRuntimeHealth::DataUnavailable
    } else if recorded {
        CombatClockRuntimeHealth::Recorded
    } else {
        CombatClockRuntimeHealth::Available
    }
}

fn publish_combat_clock_health(
    sender: &EngineEventSink,
    previous: &mut Option<CombatClockRuntimeHealth>,
    health: CombatClockRuntimeHealth,
) -> Result<(), EngineEventSendError> {
    if *previous == Some(health) {
        return Ok(());
    }
    sender.send(EngineEvent::CombatClockHealth(health))?;
    *previous = Some(health);
    Ok(())
}

fn stable_combat_clock_error_health(
    error: CombatClockQueryError,
    consecutive_provider_failures: &mut u8,
) -> Option<CombatClockRuntimeHealth> {
    if error != CombatClockQueryError::ProviderUnavailable {
        *consecutive_provider_failures = 0;
        return Some(combat_clock_error_health(error));
    }
    *consecutive_provider_failures = consecutive_provider_failures.saturating_add(1);
    (*consecutive_provider_failures >= COMBAT_CLOCK_PROVIDER_FAILURE_THRESHOLD)
        .then_some(CombatClockRuntimeHealth::ProviderUnavailable)
}

fn publish_combat_clock_snapshot_health(
    sender: &EngineEventSink,
    previous: &mut Option<CombatClockRuntimeHealth>,
    transitions: &[CombatClockTransitionSnapshot],
) -> Result<(), EngineEventSendError> {
    let Some(transition) = transitions.last() else {
        return Ok(());
    };
    publish_combat_clock_health(
        sender,
        previous,
        combat_clock_sample_health(transition.state_flags, false),
    )
}

fn run_plugin_monitor(
    stop: &AtomicBool,
    capture_started_100ns: u64,
    raw_capture: &RawCaptureBuffer,
    sender: &EngineEventSink,
) {
    let mut resource_warnings = Vec::new();
    let enemy_catalog = load_resource(
        ENEMY_CATALOG_PATH,
        &mut resource_warnings,
        load_enemy_catalog,
    );
    if !resource_warnings.is_empty() {
        let _ = sender.send(EngineEvent::Warning(format!(
            "enemy telemetry catalog: {}",
            resource_warnings.join("; ")
        )));
    }
    let mut last_sequence = 0;
    let mut last_mod_event_sequence = 0;
    let mut tracker = GamePauseIntervalTracker::default();
    let capture_started = filetime_100ns_to_unix_seconds(capture_started_100ns)
        .expect("capture FILETIME must be after Unix epoch");
    let mut initialized = false;
    let mut pause_state_valid = false;
    let mut previous_pause_type_mask = 0;
    let mut previous_combat_clock_health = None;
    let mut consecutive_combat_clock_provider_failures = 0;
    while !stop.load(Ordering::Relaxed) {
        match query_combat_clock_transitions() {
            Ok(transitions) => {
                consecutive_combat_clock_provider_failures = 0;
                // Every response is a bounded authoritative history snapshot.
                // Re-publish health from its newest sample even when its
                // sequence was already consumed; otherwise one transient IPC
                // failure leaves time-stop adjustment degraded until the next
                // real pause transition.
                if publish_combat_clock_snapshot_health(
                    sender,
                    &mut previous_combat_clock_health,
                    &transitions,
                )
                .is_err()
                {
                    return;
                }
                let mut current = Vec::new();
                for transition in transitions {
                    if transition.sequence <= last_sequence {
                        continue;
                    }
                    last_sequence = transition.sequence;
                    current.push(transition);
                }
                for transition in current {
                    if transition.timestamp_100ns < capture_started_100ns {
                        pause_state_valid = transition.state_flags & COMBAT_CLOCK_PAUSE_VALID != 0;
                        previous_pause_type_mask = if pause_state_valid {
                            transition.pause_type_mask
                        } else {
                            0
                        };
                        continue;
                    }
                    if !initialized {
                        initialized = true;
                        raw_capture.push_combat_clock_transition(&CombatClockTransitionSnapshot {
                            sequence: 0,
                            timestamp_100ns: capture_started_100ns,
                            pause_type_mask: if pause_state_valid {
                                previous_pause_type_mask
                            } else {
                                0
                            },
                            reserved_value: 0,
                            state_flags: u32::from(pause_state_valid) * COMBAT_CLOCK_PAUSE_VALID,
                        });
                        if pause_state_valid
                            && let Some(event) =
                                tracker.apply_transition(capture_started, previous_pause_type_mask)
                            && send_game_pause_transition(sender, event).is_err()
                        {
                            return;
                        }
                    }
                    raw_capture.push_combat_clock_transition(&transition);
                    let Some(timestamp) =
                        filetime_100ns_to_unix_seconds(transition.timestamp_100ns)
                    else {
                        continue;
                    };
                    pause_state_valid = transition.state_flags & COMBAT_CLOCK_PAUSE_VALID != 0;
                    let pause_type_mask = if pause_state_valid {
                        transition.pause_type_mask
                    } else {
                        0
                    };
                    if let Some(event) = tracker.apply_transition(timestamp, pause_type_mask)
                        && send_game_pause_transition(sender, event).is_err()
                    {
                        return;
                    }
                }
                if !initialized {
                    initialized = true;
                    raw_capture.push_combat_clock_transition(&CombatClockTransitionSnapshot {
                        sequence: 0,
                        timestamp_100ns: capture_started_100ns,
                        pause_type_mask: if pause_state_valid {
                            previous_pause_type_mask
                        } else {
                            0
                        },
                        reserved_value: 0,
                        state_flags: u32::from(pause_state_valid) * COMBAT_CLOCK_PAUSE_VALID,
                    });
                    if pause_state_valid
                        && let Some(event) =
                            tracker.apply_transition(capture_started, previous_pause_type_mask)
                        && send_game_pause_transition(sender, event).is_err()
                    {
                        return;
                    }
                }
            }
            Err(error) => {
                if let Some(health) = stable_combat_clock_error_health(
                    error,
                    &mut consecutive_combat_clock_provider_failures,
                ) && publish_combat_clock_health(
                    sender,
                    &mut previous_combat_clock_health,
                    health,
                )
                .is_err()
                {
                    return;
                }
            }
        }
        if let Ok(events) = query_mod_events() {
            for event in events {
                if event.sequence <= last_mod_event_sequence {
                    continue;
                }
                last_mod_event_sequence = event.sequence;
                if event.mod_id == "enemy-telemetry"
                    && event.timestamp_100ns < capture_started_100ns
                {
                    continue;
                }
                let mut event = crate::engine::model::ModScriptEvent::from_bridge(
                    event.sequence,
                    event.timestamp_100ns,
                    event.mod_id,
                    event.name,
                    event.values,
                );
                if event.mod_id == "enemy-telemetry"
                    && matches!(event.name.as_str(), "enemy.identity" | "enemy.hit_target")
                    && let [_, config_hash, _] = event.values.as_slice()
                {
                    event.enemy_identity = enemy_catalog.get(*config_hash).cloned();
                }
                raw_capture.push_mod_script_event(&event);
                if sender.send(EngineEvent::ModScript(event)).is_err() {
                    return;
                }
            }
        }
        thread::sleep(COMBAT_CLOCK_POLL_INTERVAL);
    }
    if tracker.pause_type_mask != 0 {
        let ended_100ns = current_filetime_100ns();
        let transition = CombatClockTransitionSnapshot {
            sequence: last_sequence.saturating_add(1),
            timestamp_100ns: ended_100ns,
            pause_type_mask: 0,
            reserved_value: 0,
            state_flags: COMBAT_CLOCK_PAUSE_VALID,
        };
        raw_capture.push_combat_clock_transition(&transition);
        if let Some(timestamp) = filetime_100ns_to_unix_seconds(ended_100ns)
            && let Some(event) = tracker.apply_transition(timestamp, 0)
        {
            let _ = send_game_pause_transition(sender, event);
        }
    }
}

fn npcap_library_path() -> PathBuf {
    windows_system_directory().join("Npcap").join("wpcap.dll")
}

fn packet_library_path() -> PathBuf {
    windows_system_directory().join("Npcap").join("Packet.dll")
}

fn windows_system_directory() -> PathBuf {
    std::env::var_os("SystemRoot")
        .or_else(|| std::env::var_os("WINDIR"))
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(r"C:\Windows"))
        .join("System32")
}

struct NpcapLibraries {
    // Fields drop in declaration order: unload wpcap before its Packet.dll
    // dependency. Handles and compiled programs borrow `wpcap`, so Rust also
    // prevents either library owner from being dropped before their guards.
    wpcap: Library,
    _packet: Library,
}

impl NpcapLibraries {
    fn load() -> Result<Self, String> {
        // SAFETY: The absolute System32/Npcap path selects the installed Npcap
        // dependency; the returned owner remains live in this struct.
        let packet =
            unsafe { Library::new(packet_library_path()) }.map_err(|error| error.to_string())?;
        // SAFETY: The absolute System32/Npcap path selects wpcap.dll. Packet.dll
        // was loaded first and both owners are retained for the full API use.
        let wpcap =
            unsafe { Library::new(npcap_library_path()) }.map_err(|error| error.to_string())?;
        Ok(Self {
            wpcap,
            _packet: packet,
        })
    }
}

unsafe fn load_symbol<T: Copy>(library: &Library, name: &[u8]) -> Result<T, String> {
    // SAFETY: The requested names and signatures match the public libpcap API.
    unsafe {
        library
            .get::<T>(name)
            .map(|symbol| *symbol)
            .map_err(|error| error.to_string())
    }
}

/// # Safety
///
/// A non-null `value` must point to a readable NUL-terminated string for the
/// duration of this call. Null is accepted and maps to an empty string.
unsafe fn c_string(value: *const c_char) -> String {
    if value.is_null() {
        String::new()
    } else {
        // SAFETY: The caller guarantees a readable NUL-terminated buffer.
        unsafe { CStr::from_ptr(value).to_string_lossy().into_owned() }
    }
}

const MAX_NPCAP_DEVICES: usize = 1_024;
const MAX_NPCAP_ADDRESSES_PER_DEVICE: usize = 1_024;

struct NpcapDeviceList {
    raw: *mut PcapIf,
    free: FreeAllDevs,
}

impl Drop for NpcapDeviceList {
    fn drop(&mut self) {
        // SAFETY: `raw` is exactly the list returned by pcap_findalldevs and
        // `free` is the matching symbol kept alive by the caller's library
        // owner until this guard is dropped.
        unsafe { (self.free)(self.raw) };
    }
}

/// # Safety
///
/// `current` must be the head returned by a successful `pcap_findalldevs`
/// call and remain owned by a live [`NpcapDeviceList`] for this call. Npcap's
/// documented linked-list nodes and strings must remain readable until freed.
unsafe fn project_npcap_devices(mut current: *mut PcapIf) -> Result<Vec<CaptureDevice>, String> {
    let mut result = Vec::new();
    while !current.is_null() {
        if result.len() == MAX_NPCAP_DEVICES {
            return Err("Npcap returned too many capture devices".to_owned());
        }
        // SAFETY: upheld by this function's contract; current is checked
        // non-null before dereferencing the documented PcapIf node.
        let device = unsafe { &*current };
        let mut ipv4 = Vec::new();
        let mut address = device.addresses;
        let mut address_count = 0_usize;
        while !address.is_null() {
            if address_count == MAX_NPCAP_ADDRESSES_PER_DEVICE {
                return Err("Npcap returned too many addresses for a device".to_owned());
            }
            address_count += 1;
            // SAFETY: upheld by this function's contract; address is checked
            // non-null and belongs to the current Npcap address list.
            let address_ref = unsafe { &*address };
            let addr = address_ref.addr;
            if !addr.is_null() {
                // SAFETY: addr belongs to this live Npcap list and was checked
                // non-null. sockaddr's fixed family/data prefix is ABI-stable.
                let addr_ref = unsafe { &*addr };
                if addr_ref.family == 2 {
                    let bytes = &addr_ref.data;
                    ipv4.push(Ipv4Addr::new(bytes[2], bytes[3], bytes[4], bytes[5]));
                }
            }
            address = address_ref.next;
        }
        result.push(CaptureDevice {
            // SAFETY: Npcap documents name/description as nullable C strings
            // owned by the device list for its lifetime.
            name: unsafe { c_string(device.name) },
            // SAFETY: same lifetime and nullability guarantee as name.
            description: unsafe { c_string(device.description) },
            ipv4,
        });
        current = device.next;
    }
    Ok(result)
}

pub fn list_devices() -> Result<Vec<CaptureDevice>, String> {
    let libraries = NpcapLibraries::load()
        .map_err(|error| format!("无法加载 Npcap，请先安装 Npcap: {error}"))?;
    // SAFETY: symbol names and signatures are the documented libpcap ABI;
    // `libraries` stays alive through every call and device-list drop below.
    let find_all_devs: FindAllDevs =
        unsafe { load_symbol(&libraries.wpcap, b"pcap_findalldevs\0")? };
    // SAFETY: same ABI/lifetime proof as pcap_findalldevs.
    let free_all_devs: FreeAllDevs =
        unsafe { load_symbol(&libraries.wpcap, b"pcap_freealldevs\0")? };
    let mut devices_ptr = ptr::null_mut();
    let mut error_buffer = [0_i8; PCAP_ERRBUF_SIZE];
    // SAFETY: both out-pointers reference writable storage of the documented
    // size, and the loaded function remains valid while `libraries` lives.
    if unsafe { find_all_devs(&mut devices_ptr, error_buffer.as_mut_ptr()) } != 0 {
        // SAFETY: Npcap writes a NUL-terminated error string into errbuf on
        // failure; the fixed 256-byte buffer remains alive for this call.
        return Err(unsafe { c_string(error_buffer.as_ptr()) });
    }
    let devices = NpcapDeviceList {
        raw: devices_ptr,
        free: free_all_devs,
    };
    // SAFETY: `devices` owns the successful result and keeps it alive until
    // projection returns; the Npcap libraries outlive the guard's Drop.
    unsafe { project_npcap_devices(devices.raw) }
}

fn parse_udp_ipv4(
    link_type: CaptureLinkType,
    packet: &[u8],
) -> Option<(Ipv4Addr, u16, Ipv4Addr, u16, &[u8])> {
    let ip = match link_type {
        CaptureLinkType::Ethernet => {
            if packet.len() < 14 {
                return None;
            }
            let mut ethernet_offset = 14;
            let mut ether_type = u16::from_be_bytes([packet[12], packet[13]]);
            if ether_type == 0x8100 && packet.len() >= 18 {
                ether_type = u16::from_be_bytes([packet[16], packet[17]]);
                ethernet_offset = 18;
            }
            if ether_type != 0x0800 || packet.len() < ethernet_offset + 20 {
                return None;
            }
            &packet[ethernet_offset..]
        }
        CaptureLinkType::RawIpv4 | CaptureLinkType::Ipv4 if packet.len() >= 20 => packet,
        CaptureLinkType::RawIpv4 | CaptureLinkType::Ipv4 => return None,
    };
    if ip[0] >> 4 != 4 {
        return None;
    }
    let ip_header_len = ((ip[0] & 0x0f) as usize) * 4;
    let total_len = u16::from_be_bytes([ip[2], ip[3]]) as usize;
    let fragment = u16::from_be_bytes([ip[6], ip[7]]);
    if ip[0] >> 4 != 4
        || ip_header_len < 20
        || total_len < ip_header_len + 8
        || ip.len() < total_len
        || ip[9] != 17
        || fragment & 0x3fff != 0
    {
        return None;
    }
    let ip = &ip[..total_len];
    let source = Ipv4Addr::new(ip[12], ip[13], ip[14], ip[15]);
    let destination = Ipv4Addr::new(ip[16], ip[17], ip[18], ip[19]);
    let udp = &ip[ip_header_len..];
    let source_port = u16::from_be_bytes([udp[0], udp[1]]);
    let destination_port = u16::from_be_bytes([udp[2], udp[3]]);
    let udp_len = u16::from_be_bytes([udp[4], udp[5]]) as usize;
    if udp_len < 8 || udp_len > udp.len() {
        return None;
    }
    Some((
        source,
        source_port,
        destination,
        destination_port,
        &udp[8..udp_len],
    ))
}

fn replay_frame_local_ip_hint(
    link_type: CaptureLinkType,
    packet: &[u8],
    local_ip_hint: Option<Ipv4Addr>,
) -> Option<Ipv4Addr> {
    let local_ip = local_ip_hint?;
    let (source, _, destination, _, _) = parse_udp_ipv4(link_type, packet)?;
    (source == local_ip || destination == local_ip).then_some(local_ip)
}

fn infer_outgoing(
    src: Ipv4Addr,
    src_port: u16,
    dst: Ipv4Addr,
    dst_port: u16,
    local_ip: Option<Ipv4Addr>,
    ids: &[u32],
    client_endpoints: &HashSet<(Ipv4Addr, u16)>,
) -> bool {
    if let Some(local_ip) = local_ip {
        return src == local_ip;
    }
    if client_endpoints.contains(&(src, src_port)) {
        return true;
    }
    if client_endpoints.contains(&(dst, dst_port)) {
        return false;
    }
    match (src.is_private(), dst.is_private()) {
        (true, false) => true,
        (false, true) => false,
        // A replay from another machine cannot use this machine's local IP.
        // TUN captures commonly keep the client's dynamic UDP port while both
        // addresses are non-private (for example, 198.18.0.0/15 inside the
        // tunnel), so use the IANA dynamic-port boundary before payload-only
        // character evidence.
        _ => match (
            src_port >= IANA_DYNAMIC_PORT_START,
            dst_port >= IANA_DYNAMIC_PORT_START,
        ) {
            (true, false) => true,
            (false, true) => false,
            _ => ids.len() == 1,
        },
    }
}

fn merged_character_evidence(
    declared: &[(u32, u8, usize)],
    final_tower: &[(u32, u8, usize)],
) -> Vec<(u32, u8, usize)> {
    let mut merged = declared.to_vec();
    for evidence in final_tower {
        if !merged.contains(evidence) {
            merged.push(*evidence);
        }
    }
    merged
}

fn append_unique_ids(ids: &mut Vec<u32>, new_ids: impl IntoIterator<Item = u32>) {
    for id in new_ids {
        if !ids.contains(&id) {
            ids.push(id);
        }
    }
}

fn character_ids_from_evidence_sources(
    declared: &[(u32, u8, usize)],
    final_tower: &[(u32, u8, usize)],
) -> Vec<u32> {
    declared_character_ids_from_evidence(&merged_character_evidence(declared, final_tower))
}

fn decode_shifted_payload(data: &[u8], bit_shift: u8) -> Vec<u8> {
    if bit_shift == 0 {
        return data.to_vec();
    }
    data.windows(2)
        .map(|pair| (pair[0] >> bit_shift) | (pair[1] << (8 - bit_shift)))
        .collect()
}

fn protocol_text_score(value: &str) -> usize {
    let length = value.len();
    if length < MIN_READABLE_TEXT_LEN {
        return 0;
    }
    let letters = value.bytes().filter(u8::is_ascii_alphabetic).count();
    let digits = value.bytes().filter(u8::is_ascii_digit).count();
    let spaces = value.bytes().filter(|byte| *byte == b' ').count();
    let punctuation = length.saturating_sub(letters + digits + spaces);
    let protocol_markers = [
        "Abyss",
        "Ability.",
        "AbilitySystem",
        "AppearMelee",
        "BackEvade",
        "Boss",
        "CharacterForNet",
        "CityEvent",
        "CityLive",
        "CoolDown.",
        "CurrentGameplayID",
        "DataLayer",
        "DissolveMontage",
        "DropBox",
        "Event.",
        "FrontEvade",
        "Game/",
        "GameplayCue.",
        "HTClient",
        "HTRoom",
        "Monster",
        "PrivateSpawn",
        "Record",
        "SilentCheckComponent",
        "SkeletalMesh",
        "Stamina",
        "State.",
        "Teleport",
        "UnbalCurrent",
        "WorldBoss",
        "FirstHalf",
        "SecondHalf",
        "Phase",
        "Wave",
        "MaxHP",
        "ft_character_",
    ];
    if protocol_markers.iter().any(|marker| value.contains(marker)) {
        return 100 + length.min(100);
    }
    if value.starts_with("/Game/") {
        return 200 + length.min(100);
    }

    let structured_identifier = value.bytes().all(|byte| {
        byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'.' | b':' | b'/' | b'-')
    });
    let has_upper = value.bytes().any(|byte| byte.is_ascii_uppercase());
    let has_lower = value.bytes().any(|byte| byte.is_ascii_lowercase());
    let has_structure = value.contains('_') || value.contains('.') || value.contains("::");
    let bytes = value.as_bytes();
    let unreal_type_name = bytes.len() >= 2
        && matches!(bytes[0], b'A' | b'E' | b'F' | b'U')
        && bytes[1].is_ascii_uppercase()
        && has_upper
        && has_lower;
    if length >= 8
        && structured_identifier
        && (has_structure || unreal_type_name)
        && letters >= 5
        && punctuation * 4 <= length
    {
        return 20 + length.min(50);
    }
    0
}

fn length_prefixed_identifier_score(value: &str) -> usize {
    let length = value.len();
    if !(4..=96).contains(&length)
        || !value.bytes().all(|byte| {
            byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'.' | b':' | b'/' | b'-' | b' ')
        })
    {
        return 0;
    }
    let letters = value.bytes().filter(u8::is_ascii_alphabetic).count();
    let has_upper = value.bytes().any(|byte| byte.is_ascii_uppercase());
    let has_lower = value.bytes().any(|byte| byte.is_ascii_lowercase());
    if letters < 4 || !has_upper || !has_lower {
        return 0;
    }
    80 + length.min(80)
}

fn extract_length_prefixed_identifiers(data: &[u8]) -> Vec<(usize, String)> {
    let mut found = Vec::new();
    let mut seen = HashSet::new();
    for offset in 0..data.len().saturating_sub(8) {
        let Some(length_bytes) = data.get(offset..offset + 4) else {
            continue;
        };
        let length = u32::from_le_bytes(length_bytes.try_into().unwrap()) as usize;
        if !(5..=97).contains(&length) {
            continue;
        }
        let Some(raw) = data.get(offset + 4..offset + 4 + length) else {
            continue;
        };
        let Some(value_bytes) = raw.strip_suffix(&[0]) else {
            continue;
        };
        let Ok(value) = std::str::from_utf8(value_bytes) else {
            continue;
        };
        let score = length_prefixed_identifier_score(value);
        if score > 0 && seen.insert(value.to_owned()) {
            found.push((score, value.to_owned()));
        }
    }
    found
}

struct DecodedPayloadText {
    text: String,
    has_readable_text: bool,
}

fn decode_payload_text(data: &[u8]) -> String {
    decode_payload_text_filtered(data, |_| true).text
}

const SUMMARY_MARKER_ABYSS: usize = 0;
const SUMMARY_MARKER_ABYSS_GAMEPLAY: usize = 1;
const SUMMARY_MARKER_SUCCESS: usize = 2;
const SUMMARY_MARKER_FIRST_HALF: usize = 3;
const SUMMARY_MARKER_SECOND_HALF: usize = 4;
const SUMMARY_MARKER_ABYSS_CLONE: usize = 5;
const SUMMARY_MARKER_ABYSS_RESTART: usize = 6;
const SUMMARY_MARKER_ABYSS_EXIT: usize = 7;
const SUMMARY_MARKER_ULTRA_SKILL: usize = 8;
const SUMMARY_MARKER_COUNT: usize = 9;
const SUMMARY_TEXT_MAX_LEN: usize = 256;
const SUMMARY_MARKER_PATTERNS: [&[u8]; SUMMARY_MARKER_COUNT] = [
    b"Abyss",
    b"FAbyssGamePlayData",
    b"ConditionState_Success",
    b"EAbyssFightStage::FirstHalf",
    b"EAbyssFightStage::SecondHalf",
    b"AbyssClone",
    b"Abyss_Battle_Born",
    b"Abyss_Station_LeaveClone",
    b"UltraSkill",
];

#[derive(Default)]
struct SummaryPayloadMarkers {
    found: [bool; SUMMARY_MARKER_COUNT],
    explicit_stage: Option<(u32, u32, AbyssHalf)>,
}

/// Streaming parser for a printable identifier that starts with `Abyss_`.
/// It retains only the last three underscore-delimited numeric components,
/// matching [`parse_abyss_stage_id`] without allocating the complete token.
struct AbyssStageTokenScanner {
    position: usize,
    prefix_matches: bool,
    capturing_components: bool,
    component_count: usize,
    component_value: u32,
    component_has_digit: bool,
    component_is_numeric: bool,
    component_trailing_spaces: bool,
    last_components: [Option<u32>; 3],
}

impl AbyssStageTokenScanner {
    const PREFIX: &'static [u8] = b"Abyss_";

    fn new() -> Self {
        Self {
            position: 0,
            prefix_matches: true,
            capturing_components: false,
            component_count: 0,
            component_value: 0,
            component_has_digit: false,
            component_is_numeric: true,
            component_trailing_spaces: false,
            last_components: [None; 3],
        }
    }

    fn push_printable(&mut self, byte: u8) {
        // The full decoder trims printable runs before parsing identifiers.
        if self.position == 0 && byte == b' ' {
            return;
        }
        if self.position < Self::PREFIX.len() {
            self.prefix_matches &= byte == Self::PREFIX[self.position];
            self.position += 1;
            self.capturing_components = self.position == Self::PREFIX.len() && self.prefix_matches;
            return;
        }
        self.position += 1;
        if !self.capturing_components {
            return;
        }
        if byte == b' ' && self.component_has_digit && self.component_is_numeric {
            self.component_trailing_spaces = true;
            return;
        }
        if self.component_trailing_spaces {
            // Spaces are valid only when they are trimmed from the end of the
            // whole printable run. Any following byte makes them internal.
            self.component_is_numeric = false;
            self.component_trailing_spaces = false;
        }
        if byte == b'_' {
            self.finish_component();
            return;
        }
        self.component_has_digit = true;
        let Some(digit) = byte.checked_sub(b'0').filter(|digit| *digit <= 9) else {
            self.component_is_numeric = false;
            return;
        };
        if self.component_is_numeric {
            match self
                .component_value
                .checked_mul(10)
                .and_then(|value| value.checked_add(u32::from(digit)))
            {
                Some(value) => self.component_value = value,
                None => self.component_is_numeric = false,
            }
        }
    }

    fn finish_component(&mut self) {
        self.last_components.rotate_left(1);
        self.last_components[2] =
            (self.component_has_digit && self.component_is_numeric).then_some(self.component_value);
        self.component_count = self.component_count.saturating_add(1);
        self.component_value = 0;
        self.component_has_digit = false;
        self.component_is_numeric = true;
        self.component_trailing_spaces = false;
    }

    fn finish(mut self) -> Option<(u32, u32, AbyssHalf)> {
        if !self.capturing_components {
            return None;
        }
        self.finish_component();
        if self.component_count < 3 {
            return None;
        }
        let [Some(cycle), Some(floor), Some(half)] = self.last_components else {
            return None;
        };
        let half = match half {
            0 => AbyssHalf::First,
            1 => AbyssHalf::Second,
            _ => return None,
        };
        Some((cycle, floor, half))
    }
}

fn shifted_payload_len(data: &[u8], bit_shift: u8) -> usize {
    match bit_shift {
        0 => data.len(),
        1..=7 => data.len().saturating_sub(1),
        _ => 0,
    }
}

fn shifted_payload_byte(data: &[u8], bit_shift: u8, index: usize) -> Option<u8> {
    match bit_shift {
        0 => data.get(index).copied(),
        1..=7 => Some(
            (data.get(index).copied()? >> bit_shift)
                | (data.get(index.checked_add(1)?).copied()? << (8 - bit_shift)),
        ),
        _ => None,
    }
}

fn shifted_payload_ends_with(data: &[u8], bit_shift: u8, end_index: usize, pattern: &[u8]) -> bool {
    let Some(end_exclusive) = end_index.checked_add(1) else {
        return false;
    };
    let Some(start) = end_exclusive.checked_sub(pattern.len()) else {
        return false;
    };
    pattern.iter().enumerate().all(|(offset, expected)| {
        shifted_payload_byte(data, bit_shift, start + offset) == Some(*expected)
    })
}

fn scan_summary_payload_markers(data: &[u8]) -> SummaryPayloadMarkers {
    let mut markers = SummaryPayloadMarkers::default();
    for bit_shift in 0..8 {
        let mut stage_token = AbyssStageTokenScanner::new();
        for index in 0..shifted_payload_len(data, bit_shift) {
            let Some(byte) = shifted_payload_byte(data, bit_shift, index) else {
                break;
            };
            // One streaming pass per bit alignment. Only a possible terminal
            // byte triggers a bounded comparison against its fixed marker;
            // unrelated bytes do not fan out over every pattern.
            let candidates: &[usize] = match byte {
                b'a' => &[SUMMARY_MARKER_ABYSS_GAMEPLAY],
                b'e' => &[SUMMARY_MARKER_ABYSS_CLONE, SUMMARY_MARKER_ABYSS_EXIT],
                b'f' => &[SUMMARY_MARKER_FIRST_HALF, SUMMARY_MARKER_SECOND_HALF],
                b'l' => &[SUMMARY_MARKER_ULTRA_SKILL],
                b'n' => &[SUMMARY_MARKER_ABYSS_RESTART],
                b's' => &[SUMMARY_MARKER_ABYSS, SUMMARY_MARKER_SUCCESS],
                _ => &[],
            };
            for marker_index in candidates {
                if !markers.found[*marker_index]
                    && shifted_payload_ends_with(
                        data,
                        bit_shift,
                        index,
                        SUMMARY_MARKER_PATTERNS[*marker_index],
                    )
                {
                    markers.found[*marker_index] = true;
                }
            }

            if (0x20..=0x7e).contains(&byte) {
                stage_token.push_printable(byte);
            } else {
                if let Some(stage) = stage_token.finish() {
                    markers.explicit_stage = Some(stage);
                }
                stage_token = AbyssStageTokenScanner::new();
            }
        }
        if let Some(stage) = stage_token.finish() {
            markers.explicit_stage = Some(stage);
        }
    }
    markers
}

fn append_summary_marker(text: &mut String, marker: &str) {
    if !text.is_empty() {
        text.push('\n');
    }
    text.push_str(marker);
}

fn decode_summary_payload_text(data: &[u8]) -> DecodedPayloadText {
    let markers = scan_summary_payload_markers(data);
    let mut text = String::with_capacity(SUMMARY_TEXT_MAX_LEN);
    if markers.found[SUMMARY_MARKER_ABYSS_GAMEPLAY] {
        append_summary_marker(&mut text, "FAbyssGamePlayData");
    }
    if markers.found[SUMMARY_MARKER_SUCCESS] {
        append_summary_marker(&mut text, "ConditionState_Success");
    }
    if markers.found[SUMMARY_MARKER_FIRST_HALF] {
        append_summary_marker(&mut text, "EAbyssFightStage::FirstHalf");
    }
    if markers.found[SUMMARY_MARKER_SECOND_HALF] {
        append_summary_marker(&mut text, "EAbyssFightStage::SecondHalf");
    }
    if markers.found[SUMMARY_MARKER_ABYSS_CLONE] {
        append_summary_marker(&mut text, "AbyssClone");
    }
    if markers.found[SUMMARY_MARKER_ABYSS_RESTART] {
        append_summary_marker(&mut text, "Abyss_Battle_Born");
    }
    if markers.found[SUMMARY_MARKER_ABYSS_EXIT] {
        append_summary_marker(&mut text, "Abyss_Station_LeaveClone");
    }
    if let Some((cycle, floor, half)) = markers.explicit_stage {
        let half = match half {
            AbyssHalf::First => 0,
            AbyssHalf::Second => 1,
        };
        append_summary_marker(&mut text, &format!("Abyss_{cycle}_{floor}_{half}"));
    }
    let has_specific_abyss_marker = markers.found[SUMMARY_MARKER_ABYSS_GAMEPLAY]
        || markers.found[SUMMARY_MARKER_FIRST_HALF]
        || markers.found[SUMMARY_MARKER_SECOND_HALF]
        || markers.found[SUMMARY_MARKER_ABYSS_CLONE]
        || markers.found[SUMMARY_MARKER_ABYSS_RESTART]
        || markers.found[SUMMARY_MARKER_ABYSS_EXIT]
        || markers.explicit_stage.is_some();
    if markers.found[SUMMARY_MARKER_ABYSS] && !has_specific_abyss_marker {
        append_summary_marker(&mut text, "Abyss");
    }
    if markers.found[SUMMARY_MARKER_ULTRA_SKILL] {
        append_summary_marker(&mut text, "UltraSkill");
    }
    debug_assert!(text.len() <= SUMMARY_TEXT_MAX_LEN);
    let has_readable_text = !text.is_empty();
    if !has_readable_text {
        text.push_str(UNREADABLE_PROTOCOL_TEXT);
    }
    DecodedPayloadText {
        text,
        has_readable_text,
    }
}

fn decode_payload_text_filtered(data: &[u8], keep: impl Fn(&str) -> bool) -> DecodedPayloadText {
    let mut found = Vec::<(usize, String)>::new();
    let mut seen = HashSet::new();
    let mut has_readable_text = false;
    for bit_shift in 0..8 {
        let shifted = decode_shifted_payload(data, bit_shift);
        for (score, value) in extract_length_prefixed_identifiers(&shifted) {
            if seen.insert(value.clone()) {
                has_readable_text = true;
                if keep(&value) {
                    found.push((score, value));
                }
            }
        }
        for bytes in shifted.split(|byte| !(0x20..=0x7e).contains(byte)) {
            if bytes.len() < MIN_READABLE_TEXT_LEN {
                continue;
            }
            let Ok(value) = std::str::from_utf8(bytes) else {
                continue;
            };
            let value = value.trim();
            let score = protocol_text_score(value);
            if score == 0 || !seen.insert(value.to_owned()) {
                continue;
            }
            has_readable_text = true;
            if keep(value) {
                found.push((score, value.to_owned()));
            }
        }
    }
    let text = if found.is_empty() {
        UNREADABLE_PROTOCOL_TEXT.to_owned()
    } else {
        found.sort_by_key(|item| std::cmp::Reverse(item.0));
        found
            .into_iter()
            .map(|(_, value)| value)
            .collect::<Vec<_>>()
            .join("\n")
    };
    DecodedPayloadText {
        text,
        has_readable_text,
    }
}

fn is_padding_payload(data: &[u8]) -> bool {
    data.is_empty()
        || data
            .first()
            .is_some_and(|first| data.iter().all(|byte| byte == first))
}

fn should_keep_debug_packet(
    payload: &[u8],
    declared_ids: &[u32],
    parsed_hits: usize,
    parsed_equipment_slots: usize,
    inventory_bunch: bool,
    has_readable_text: bool,
) -> bool {
    if parsed_hits > 0
        || parsed_equipment_slots > 0
        || inventory_bunch
        || !declared_ids.is_empty()
        || has_readable_text
    {
        return true;
    }
    !is_padding_payload(payload) && payload.len() > MAX_IGNORABLE_BINARY_PACKET_LEN
}

fn shannon_entropy(data: &[u8]) -> f64 {
    if data.is_empty() {
        return 0.0;
    }
    let mut counts = [0_usize; 256];
    for byte in data {
        counts[*byte as usize] += 1;
    }
    let length = data.len() as f64;
    counts
        .into_iter()
        .filter(|count| *count > 0)
        .map(|count| {
            let probability = count as f64 / length;
            -probability * probability.log2()
        })
        .sum()
}

fn binary_payload_diagnostic(
    payload: &[u8],
    direction: &str,
    decoded_text: &str,
    evidence: &[(u32, u8, usize)],
) -> Option<String> {
    if direction != "S2C" || decoded_text != UNREADABLE_PROTOCOL_TEXT {
        return None;
    }
    if !evidence.is_empty() {
        let mut anchors = evidence
            .iter()
            .map(|(id, shift, offset)| format!("{id}@bit{}", offset * 8 + *shift as usize))
            .collect::<Vec<_>>();
        anchors.sort();
        anchors.dedup();
        let alignments = evidence
            .iter()
            .map(|(_, shift, _)| *shift)
            .collect::<HashSet<_>>()
            .len();
        return Some(format!(
            "detected {} character anchors across {} bit alignments: {}",
            anchors.len(),
            alignments,
            anchors.join(", ")
        ));
    }
    if payload.len() < 300 || is_padding_payload(payload) {
        return None;
    }
    let zero_ratio =
        payload.iter().filter(|byte| **byte == 0).count() as f64 / payload.len() as f64;
    let entropy = shannon_entropy(payload);
    if zero_ratio < 0.20 || entropy > 5.5 {
        return None;
    }
    Some(format!(
        "candidate packed replication payload: zero_ratio={:.1}%, entropy={:.2} bit/byte",
        zero_ratio * 100.0,
        entropy
    ))
}

fn append_packet_note(note: &mut String, diagnostic: Option<String>) {
    let Some(diagnostic) = diagnostic else {
        return;
    };
    if note.contains(&diagnostic) {
        return;
    }
    if !note.is_empty() {
        note.push_str("; ");
    }
    note.push_str(&diagnostic);
}

fn bunch_wire_shape_note(
    direction: &str,
    packet: &BunchPacket,
    reassembled_payloads: usize,
) -> String {
    const DISPLAY_BUNCH_LIMIT: usize = 8;

    let mut shape = packet
        .bunches
        .iter()
        .take(DISPLAY_BUNCH_LIMIT)
        .map(|located| {
            format!(
                "c{}/d{:02x}/f{:x}/b{}",
                reliable_bunch_channel(located.bunch.prefix),
                located.bunch.descriptor,
                located.bunch.partial_flags,
                located.bunch.data_bit_len
            )
        })
        .collect::<Vec<_>>()
        .join(",");
    if packet.bunches.len() > DISPLAY_BUNCH_LIMIT {
        shape.push_str(&format!(",+{}", packet.bunches.len() - DISPLAY_BUNCH_LIMIT));
    }
    format!(
        "Bunch 链 {} 个，packet-info {} bit，跨包完成 {} 个载荷；WireShape v2 {direction} info={} [{}]",
        packet.bunches.len(),
        packet.packet_info_bit_len,
        reassembled_payloads,
        packet.packet_info_bit_len,
        shape
    )
}

fn same_equipment_slot(left: &ParsedEquipmentSlot, right: &ParsedEquipmentSlot) -> bool {
    left.state == right.state
        && left.equipment_id == right.equipment_id
        && left.equip_net_id == right.equip_net_id
        && left.first_step == right.first_step
        && left.row == right.row
        && left.column == right.column
        && left.new_flag == right.new_flag
}

fn append_unique_equipment_slots(
    slots: &mut Vec<ParsedEquipmentSlot>,
    new_slots: impl IntoIterator<Item = ParsedEquipmentSlot>,
) {
    for slot in new_slots {
        if !slots
            .iter()
            .any(|existing| same_equipment_slot(existing, &slot))
        {
            slots.push(slot);
        }
    }
}

fn equipment_slots_note(slots: &[ParsedEquipmentSlot]) -> Option<String> {
    if slots.is_empty() {
        return None;
    }
    let occupied = slots
        .iter()
        .filter(|slot| slot.state >= 0 && slot.equipment_id != "None")
        .collect::<Vec<_>>();
    let mut note = format!(
        "EquipmentSlotInfo: {} tagged slots, {} occupied",
        slots.len(),
        occupied.len()
    );
    if !occupied.is_empty() {
        let details = occupied
            .iter()
            .take(8)
            .map(|slot| {
                format!(
                    "{}#{}:{} r{}c{}@{}:{}",
                    slot.equipment_id,
                    slot.equip_net_id.solt,
                    slot.equip_net_id.serial,
                    slot.row,
                    slot.column,
                    slot.byte_offset,
                    slot.bit_shift
                )
            })
            .collect::<Vec<_>>()
            .join(", ");
        note.push_str(": ");
        note.push_str(&details);
        if occupied.len() > 8 {
            note.push_str(&format!(", +{} more", occupied.len() - 8));
        }
    }
    Some(note)
}

fn parse_abyss_stage_id(value: &str) -> Option<(u32, u32, AbyssHalf)> {
    let parts: Vec<_> = value.split('_').collect();
    if parts.len() < 4 || parts.first().copied() != Some("Abyss") {
        return None;
    }
    let cycle = parts.get(parts.len() - 3)?.parse().ok()?;
    let floor = parts.get(parts.len() - 2)?.parse().ok()?;
    let half = match *parts.last()? {
        "0" => AbyssHalf::First,
        "1" => AbyssHalf::Second,
        _ => return None,
    };
    Some((cycle, floor, half))
}

fn abyss_events_from_text(timestamp: f64, decoded_text: &str) -> Vec<AbyssEvent> {
    let mut events = Vec::new();
    let is_restart = decoded_text.contains("Abyss_Battle_Born");
    if is_restart {
        events.push(AbyssEvent::RestartDetected { timestamp });
    }
    let is_success = decoded_text.contains("ConditionState_Success")
        && decoded_text.contains("FAbyssGamePlayData");
    let mut explicit_stage = None;
    for value in decoded_text.lines() {
        if let Some(stage) = parse_abyss_stage_id(value) {
            explicit_stage = Some(stage);
        }
    }
    if let Some((cycle, floor, half)) = explicit_stage {
        events.push(AbyssEvent::Stage {
            timestamp,
            cycle: Some(cycle),
            floor: Some(floor),
            half,
            allow_late_backfill: false,
        });
    } else if decoded_text.contains("FAbyssGamePlayData")
        && (is_success || !decoded_text.contains("AbyssClone"))
    {
        let first = decoded_text.contains("EAbyssFightStage::FirstHalf");
        let second = decoded_text.contains("EAbyssFightStage::SecondHalf");
        if first ^ second {
            events.push(AbyssEvent::Stage {
                timestamp,
                cycle: None,
                floor: None,
                half: if first {
                    AbyssHalf::First
                } else {
                    AbyssHalf::Second
                },
                allow_late_backfill: is_success,
            });
        }
    }
    if is_success {
        events.push(AbyssEvent::Success { timestamp });
    }
    if decoded_text.contains("Abyss_Station_LeaveClone") {
        events.push(AbyssEvent::Exit { timestamp });
    }
    events
}

fn send_packet_events(
    sender: &EngineEventSink,
    packet: PacketDebug,
) -> Result<(), EngineEventSendError> {
    for event in abyss_events_from_text(packet.timestamp, &packet.decoded_text) {
        sender.send(EngineEvent::Abyss(event))?;
    }
    send_packet_debug_events(sender, packet)
}

fn send_packet_debug_events(
    sender: &EngineEventSink,
    packet: PacketDebug,
) -> Result<(), EngineEventSendError> {
    sender.send(EngineEvent::PacketObservation(PacketObservation {
        parsed_hits: packet.parsed_hits,
    }))?;
    sender.send(EngineEvent::Packet(Box::new(packet)))
}

const MAX_PENDING_FOLLOW_UP_HITS: usize = 256;
const AMBIGUOUS_HIT_CONFIRMATION_WINDOW_SECONDS: f64 = 0.5;
/// Npcap or a capture driver can report the same outbound frame twice with only
/// tens of microseconds between copies. Keep the window short so a later, real
/// transmission of identical application data is not suppressed.
const DUPLICATE_FRAME_WINDOW_SECONDS: f64 = 0.001;
/// Bounds memory for external captures containing many frames with one timestamp.
const MAX_RECENT_CAPTURE_FRAMES: usize = 512;
const FRAME_DEDUP_VERIFICATION_BYTE_BUDGET: usize = 256 * 1024;
const FUWEN_START_SIGNATURE_SHIFT: u8 = 3;
const FUWEN_START_SIGNATURE_OFFSET: usize = 22;
const FUWEN_START_SIGNATURE: &[u8] = &[1, 0, 0, 0, 2, 0, 0, 0];
const FUWEN_ENTERING_ID_SHIFT: u8 = 0;
const FUWEN_ENTERING_ID_OFFSET: usize = 53;
const FUWEN_PREVIOUS_ID_SHIFT: u8 = 2;
const FUWEN_PREVIOUS_ID_OFFSET: usize = 66;
const MIN_FOLLOW_UP_RESIDUAL_DAMAGE: f64 = 1.0;
const FUWEN_DAMAGE_GAMEPLAY_EFFECT_NAME: &str = "GE_ActorReaction_2_new_Damage";
const FUWEN_DAMAGE_EFFECT_ASSOCIATION_WINDOW_SECONDS: f64 = 1.0;
/// How far back to look for the real owner of an attribute-locked reaction whose
/// damage packet carried no caster. Matches the 3s window used by the 环合 retag.
const REACTION_REATTRIBUTION_WINDOW_SECONDS: f64 = 3.0;
const RECENT_CONFIRMED_HIT_WINDOW_SECONDS: f64 = 0.75;
const UNTYPED_SHADOW_HIT_WINDOW_SECONDS: f64 = 0.05;
const MAX_SERVER_DAMAGE_TARGETS: usize = 256;
const SERVER_DAMAGE_CALIBRATION_WINDOW_SECONDS: f64 = 1.0;
/// The serialized GameplayEffect unique index precedes the first field of its
/// paired legacy damage record by this SDK-specific fixed distance.
const LEGACY_GAMEPLAY_EFFECT_TO_DAMAGE_RECORD_BITS: usize = 1330;
/// Compact FHTClientActiveGE data in the client fight-data wrapper follows its
/// paired damage record by this fixed distance.
const DAMAGE_RECORD_TO_COMPACT_GAMEPLAY_EFFECT_BITS: usize = 1255;
/// The bool/enum SDK moved the record-local damage GameplayEffect after the
/// damage field. The alternate distance is used by periodic/reaction records.
const BOOL_ENUM_DAMAGE_RECORD_TO_GAMEPLAY_EFFECT_BITS: [usize; 2] = [1508, 1524];
/// The source character selected by reaction settlement brackets its damage
/// record at schema-specific offsets. Both copies must agree before attribution.
const LEGACY_DAMAGE_RECORD_SOURCE_CHARACTER_BITS: (usize, usize) = (769, 916);
const BOOL_ENUM_DAMAGE_RECORD_SOURCE_CHARACTER_BITS: (usize, usize) = (537, 1173);

fn hit_can_trigger_fuwen_follow_up(hit: &Hit) -> bool {
    match hit.attack_type.as_deref() {
        Some("创生") | Some("创生花") | Some("覆纹") | Some("延滞") | Some("黯星")
        | Some("浊燃") | Some("浸染") | Some("盈蓄") | Some("失谐") => false,
        Some(attack_type) if attack_type.starts_with("环合·") => attack_type == "环合·覆纹",
        _ => true,
    }
}

#[derive(Clone)]
struct PendingHit {
    hit: Hit,
}

#[derive(Default)]
struct FollowUpDamageTracker {
    last_server_hp: Option<f64>,
    last_hit_timestamp: Option<f64>,
    target_max_hp: Option<f64>,
    pending_hits: VecDeque<PendingHit>,
    team_attributes: HashSet<String>,
    character_attributes: HashMap<u32, String>,
    fuwen_active: bool,
    fuwen_start_pending: bool,
    fuwen_recorded_damage: bool,
    fuwen_damage_effect_at: Option<f64>,
}

impl FollowUpDamageTracker {
    fn reset_battle(&mut self) {
        self.pending_hits.clear();
        self.team_attributes.clear();
        self.character_attributes.clear();
        self.clear_fuwen_state();
    }

    fn observe_characters(
        &mut self,
        character_ids: impl IntoIterator<Item = u32>,
        characters: &HashMap<u32, CharacterInfo>,
    ) {
        for character_id in character_ids {
            let Some(attribute) = characters
                .get(&character_id)
                .and_then(|character| character.attribute.as_deref())
            else {
                continue;
            };
            self.team_attributes.insert(attribute.to_owned());
            self.character_attributes
                .insert(character_id, attribute.to_owned());
        }
    }

    fn observe_hit(
        &mut self,
        hit: &Hit,
        _gameplay_effect_index: Option<u32>,
        characters: &HashMap<u32, CharacterInfo>,
    ) {
        if hit.direction.is_incoming()
            || hit.char_id == 0
            || hit.target_max_hp <= 500_000.0
            || hit.target_hp_before <= 0.0
        {
            return;
        }
        let new_full_health_battle = self.last_hit_timestamp.is_some_and(|last_timestamp| {
            hit.timestamp - last_timestamp > 10.0 && hit.target_hp_before >= hit.target_max_hp * 0.9
        });
        let changed_target_max_hp = self
            .target_max_hp
            .is_some_and(|maximum| (maximum - hit.target_max_hp).abs() > 1.0);
        if new_full_health_battle || changed_target_max_hp {
            self.reset_battle();
            self.last_server_hp = None;
        }
        self.last_hit_timestamp = Some(hit.timestamp);
        self.target_max_hp = Some(hit.target_max_hp);
        self.observe_characters([hit.char_id], characters);
        if self
            .pending_hits
            .back()
            .is_some_and(|previous| hit.timestamp - previous.hit.timestamp > 1.0)
        {
            self.pending_hits.clear();
        }
        self.pending_hits.push_back(PendingHit { hit: hit.clone() });
        while self.pending_hits.len() > MAX_PENDING_FOLLOW_UP_HITS {
            self.pending_hits.pop_front();
        }
    }

    fn observe_fuwen_start_candidate(
        &mut self,
        _timestamp: f64,
        entering_character_id: u32,
        previous_character_id: u32,
        characters: &HashMap<u32, CharacterInfo>,
    ) {
        self.observe_characters([entering_character_id, previous_character_id], characters);
        self.fuwen_start_pending = true;
    }

    fn observe_fuwen_trigger_hit(&mut self, hit: &Hit) {
        if hit.direction.is_incoming() || hit.attack_type.as_deref() != Some("环合·覆纹") {
            return;
        }
        self.fuwen_active = true;
        self.fuwen_start_pending = false;
        self.fuwen_recorded_damage = false;
        self.pending_hits.clear();
        self.last_server_hp = None;
    }

    fn observe_fuwen_damage_effect(&mut self, timestamp: f64) {
        self.fuwen_damage_effect_at = Some(timestamp);
    }

    fn observe_direct_fuwen_damage_hit(&mut self) {
        self.fuwen_damage_effect_at = None;
    }

    fn observe_server_hp(&mut self, timestamp: f64, current_hp: f64) -> Option<HitFollowUp> {
        let has_recent_fuwen_damage_effect =
            self.fuwen_damage_effect_at.is_some_and(|observed_at| {
                timestamp >= observed_at
                    && timestamp - observed_at <= FUWEN_DAMAGE_EFFECT_ASSOCIATION_WINDOW_SECONDS
            });
        if self.fuwen_damage_effect_at.is_some() && !has_recent_fuwen_damage_effect {
            self.fuwen_damage_effect_at = None;
        }
        self.pending_hits
            .retain(|pending| timestamp - pending.hit.timestamp <= 1.0);
        let previous_hp = self.last_server_hp.or_else(|| {
            self.pending_hits
                .front()
                .map(|pending| pending.hit.target_hp_before)
        });
        self.last_server_hp = Some(current_hp);
        let previous_hp = previous_hp?;
        if current_hp >= previous_hp || self.pending_hits.is_empty() {
            if current_hp > previous_hp {
                let reset_threshold = self.target_max_hp.unwrap_or(current_hp) * 0.25;
                if current_hp - previous_hp >= reset_threshold {
                    self.reset_battle();
                } else {
                    self.pending_hits.clear();
                }
            }
            return None;
        }

        let actual_damage = previous_hp - current_hp;
        let source = self.pending_hits.pop_front()?.hit;
        if !hit_can_trigger_fuwen_follow_up(&source) {
            return None;
        }
        let residual_damage = actual_damage - source.damage;
        let has_required_team_attributes =
            self.team_attributes.contains("灵") && self.team_attributes.contains("咒");
        let source_attribute = self.character_attributes.get(&source.char_id)?;
        if !has_required_team_attributes || !matches!(source_attribute.as_str(), "灵" | "咒") {
            return None;
        }
        if !self.fuwen_active && !has_recent_fuwen_damage_effect {
            return None;
        }
        if residual_damage < MIN_FOLLOW_UP_RESIDUAL_DAMAGE {
            return None;
        }
        self.fuwen_recorded_damage = true;
        self.fuwen_damage_effect_at = None;
        Some(HitFollowUp {
            source_timestamp: source.timestamp,
            source_char_id: source.char_id,
            source_damage: source.damage,
            source_target_hp_before: source.target_hp_before,
            source_target_hp_after: source.target_hp_after,
            source_target_max_hp: source.target_max_hp,
            source_gameplay_effect_index: source.gameplay_effect_index,
            timestamp,
            damage: residual_damage,
            target_hp_after: current_hp,
            target_hp_percent: if source.target_max_hp > 0.0 {
                current_hp / source.target_max_hp * 100.0
            } else {
                0.0
            },
            damage_name: Some("覆纹追加攻击".to_owned()),
            attack_type: Some("覆纹".to_owned()),
            damage_attribute: Some(source_attribute.clone()),
        })
    }

    fn clear_fuwen_state(&mut self) {
        self.fuwen_active = false;
        self.fuwen_start_pending = false;
        self.fuwen_recorded_damage = false;
        self.fuwen_damage_effect_at = None;
    }
}

#[derive(Clone)]
struct ServerDamagePendingHit {
    hit: Hit,
    target_handle: Option<[u8; 29]>,
    use_server_damage: bool,
    max_hp_reduction_percent: u32,
}

#[derive(Clone, Copy)]
struct ServerHpSnapshot {
    timestamp: f64,
    hp: f64,
}

#[derive(Default)]
struct ServerDamageCalibrationTracker {
    hp_by_handle: HashMap<[u8; 29], ServerHpSnapshot>,
    pending_hits: VecDeque<ServerDamagePendingHit>,
    /// FIFO by observation time; a later max-HP snapshot may claim exactly one
    /// matching semantic hit. Capacity is `MAX_PENDING_FOLLOW_UP_HITS`; full
    /// policy evicts the oldest unresolved hit, while reset or ambiguity drops
    /// that target's candidates instead of guessing attribution.
    pending_max_hp_reduction_hits: VecDeque<ServerDamagePendingHit>,
    client_damage_by_handle: HashMap<[u8; 29], f64>,
    target_max_hp_by_handle: HashMap<[u8; 29], f64>,
    residual_emitted: HashSet<[u8; 29]>,
    residual_hits: Vec<Hit>,
}

impl ServerDamageCalibrationTracker {
    fn evict_oldest_target_if_full(&mut self, target_handle: [u8; 29]) {
        if self.hp_by_handle.contains_key(&target_handle)
            || self.hp_by_handle.len() < MAX_SERVER_DAMAGE_TARGETS
        {
            return;
        }
        let oldest = self
            .hp_by_handle
            .iter()
            .min_by(|(left_handle, left), (right_handle, right)| {
                left.timestamp
                    .total_cmp(&right.timestamp)
                    .then_with(|| left_handle.cmp(right_handle))
            })
            .map(|(handle, _)| *handle);
        if let Some(oldest) = oldest {
            self.hp_by_handle.remove(&oldest);
            self.client_damage_by_handle.remove(&oldest);
            self.target_max_hp_by_handle.remove(&oldest);
            self.residual_emitted.remove(&oldest);
            self.pending_max_hp_reduction_hits
                .retain(|pending| pending.target_handle != Some(oldest));
        }
    }

    #[cfg(test)]
    fn observe_hit(&mut self, hit: &Hit) {
        let _ = self.observe_hit_with_semantics(hit, false, 0);
    }

    fn observe_hit_with_semantics(
        &mut self,
        hit: &Hit,
        use_server_damage: bool,
        max_hp_reduction_percent: u32,
    ) -> Option<HitDamageCorrection> {
        if hit.direction.is_incoming()
            || hit.char_id == 0
            || hit.target_max_hp <= 0.0
            || hit.target_hp_before <= 0.0
        {
            return None;
        }
        let target_handle = wire_handle_from_hit(hit);
        let max_hp_reduction_correction = target_handle.and_then(|target_handle| {
            self.evict_oldest_target_if_full(target_handle);
            let target_reset = self
                .hp_by_handle
                .get(&target_handle)
                .is_some_and(|snapshot| snapshot.hp <= 1.0)
                && nearly_same(hit.target_hp_before, hit.target_max_hp);
            if target_reset {
                self.hp_by_handle.remove(&target_handle);
                self.client_damage_by_handle.remove(&target_handle);
                self.target_max_hp_by_handle.remove(&target_handle);
                self.residual_emitted.remove(&target_handle);
                self.pending_hits
                    .retain(|pending| pending.target_handle != Some(target_handle));
                self.pending_max_hp_reduction_hits
                    .retain(|pending| pending.target_handle != Some(target_handle));
                None
            } else {
                self.observe_target_max_hp_change(hit, target_handle)
            }
        });
        if let Some(target_handle) = target_handle {
            if !self.hp_by_handle.contains_key(&target_handle)
                && self.hp_by_handle.len() < MAX_SERVER_DAMAGE_TARGETS
            {
                self.hp_by_handle.insert(
                    target_handle,
                    ServerHpSnapshot {
                        timestamp: hit.timestamp,
                        hp: hit.target_hp_before,
                    },
                );
            }
            self.target_max_hp_by_handle
                .insert(target_handle, hit.target_max_hp);
            *self
                .client_damage_by_handle
                .entry(target_handle)
                .or_default() += (hit.damage - hit.overkill_damage()).max(0.0);
        }
        if self
            .pending_hits
            .back()
            .is_some_and(|pending| hit.timestamp - pending.hit.timestamp > 2.0)
        {
            self.pending_hits.clear();
        }
        self.pending_hits.push_back(ServerDamagePendingHit {
            hit: hit.clone(),
            target_handle,
            use_server_damage,
            max_hp_reduction_percent,
        });
        while self.pending_hits.len() > MAX_PENDING_FOLLOW_UP_HITS {
            self.pending_hits.pop_front();
        }
        if max_hp_reduction_percent > 0 {
            self.pending_max_hp_reduction_hits
                .push_back(ServerDamagePendingHit {
                    hit: hit.clone(),
                    target_handle,
                    use_server_damage,
                    max_hp_reduction_percent,
                });
            while self.pending_max_hp_reduction_hits.len() > MAX_PENDING_FOLLOW_UP_HITS {
                self.pending_max_hp_reduction_hits.pop_front();
            }
        }
        max_hp_reduction_correction
    }

    fn observe_target_max_hp_change(
        &mut self,
        hit: &Hit,
        target_handle: [u8; 29],
    ) -> Option<HitDamageCorrection> {
        let previous_max_hp = self.target_max_hp_by_handle.get(&target_handle).copied()?;
        if hit.target_max_hp > previous_max_hp + 1.0 {
            self.pending_max_hp_reduction_hits
                .retain(|pending| pending.target_handle != Some(target_handle));
            return None;
        }
        let reduction = previous_max_hp - hit.target_max_hp;
        if reduction <= 1.0 {
            return None;
        }

        let candidates = self
            .pending_max_hp_reduction_hits
            .iter()
            .enumerate()
            .filter_map(|(index, pending)| {
                (pending.target_handle == Some(target_handle)).then_some(index)
            })
            .collect::<Vec<_>>();
        let source = (candidates.len() == 1)
            .then(|| self.pending_max_hp_reduction_hits[candidates[0]].clone());
        self.pending_max_hp_reduction_hits
            .retain(|pending| pending.target_handle != Some(target_handle));
        let source = source?;
        let source_hit = source.hit;
        Some(HitDamageCorrection {
            source_timestamp: source_hit.timestamp,
            source_char_id: source_hit.char_id,
            source_damage: source_hit.damage,
            source_target_hp_before: source_hit.target_hp_before,
            source_target_hp_after: source_hit.target_hp_after,
            source_target_max_hp: source_hit.target_max_hp,
            source_gameplay_effect_index: source_hit.gameplay_effect_index,
            damage: source_hit.damage,
            target_hp_before: source_hit.target_hp_before,
            target_hp_after: source_hit.target_hp_after,
            target_hp_percent: source_hit.target_hp_percent,
            max_hp_reduction: Some(reduction),
            reconciled_overkill_damage: None,
        })
    }

    fn update_pending_max_hp_reduction_hit(
        &mut self,
        source_hit: &Hit,
        correction: &HitDamageCorrection,
    ) {
        let source_target = wire_handle_from_hit(source_hit);
        let Some(pending) = self
            .pending_max_hp_reduction_hits
            .iter_mut()
            .find(|pending| {
                pending.target_handle == source_target
                    && pending.hit.timestamp.to_bits() == source_hit.timestamp.to_bits()
                    && pending.hit.char_id == source_hit.char_id
                    && pending.hit.gameplay_effect_index == source_hit.gameplay_effect_index
                    && pending.hit.damage.to_bits() == source_hit.damage.to_bits()
                    && pending.hit.target_hp_before.to_bits()
                        == source_hit.target_hp_before.to_bits()
                    && pending.hit.target_hp_after.to_bits() == source_hit.target_hp_after.to_bits()
                    && pending.hit.target_max_hp.to_bits() == source_hit.target_max_hp.to_bits()
            })
        else {
            return;
        };
        pending.hit.damage = correction.damage;
        pending.hit.target_hp_before = correction.target_hp_before;
        pending.hit.target_hp_after = correction.target_hp_after;
        pending.hit.target_hp_percent = correction.target_hp_percent;
        if let Some(reduction) = correction.max_hp_reduction {
            pending.hit.max_hp_reduction = reduction;
        }
    }

    #[cfg(test)]
    fn observe_server_damage_settlement(
        &mut self,
        timestamp: f64,
        settlement: &ParsedServerDamageSettlement,
    ) -> (
        Option<HitDamageCorrection>,
        Option<UnattributedServerDamage>,
    ) {
        let (mut corrections, mut unattributed) =
            self.observe_server_damage_settlements(timestamp, std::slice::from_ref(settlement));
        (corrections.pop(), unattributed.pop())
    }

    fn observe_server_damage_settlements(
        &mut self,
        timestamp: f64,
        settlements: &[ParsedServerDamageSettlement],
    ) -> (Vec<HitDamageCorrection>, Vec<UnattributedServerDamage>) {
        self.pending_hits.retain(|pending| {
            timestamp - pending.hit.timestamp <= SERVER_DAMAGE_CALIBRATION_WINDOW_SECONDS
        });

        let mut previous_snapshots = Vec::with_capacity(settlements.len());
        let mut current_hps = Vec::with_capacity(settlements.len());
        for settlement in settlements {
            let current_hp = normalized_server_hp(settlement.current_hp);
            self.evict_oldest_target_if_full(settlement.target_handle);
            previous_snapshots.push(self.hp_by_handle.insert(
                settlement.target_handle,
                ServerHpSnapshot {
                    timestamp,
                    hp: current_hp,
                },
            ));
            current_hps.push(current_hp);
        }

        let mut assigned_pending = HashSet::new();
        let mut assignments = vec![None; settlements.len()];
        for (settlement_index, settlement) in settlements.iter().enumerate() {
            let Some(pending_index) =
                self.pending_hits
                    .iter()
                    .enumerate()
                    .find_map(|(pending_index, pending)| {
                        (!assigned_pending.contains(&pending_index)
                            && pending.target_handle == Some(settlement.target_handle))
                        .then_some(pending_index)
                    })
            else {
                continue;
            };
            assigned_pending.insert(pending_index);
            assignments[settlement_index] = Some(pending_index);
        }

        let sources = assignments
            .iter()
            .map(|pending_index| {
                pending_index.and_then(|index| self.pending_hits.get(index).cloned())
            })
            .collect::<Vec<_>>();
        let mut pending_index = 0_usize;
        self.pending_hits.retain(|_| {
            let keep = !assigned_pending.contains(&pending_index);
            pending_index += 1;
            keep
        });

        let mut corrections = Vec::new();
        let mut unattributed = Vec::new();
        for (settlement_index, settlement) in settlements.iter().enumerate() {
            let current_hp = current_hps[settlement_index];
            let previous = previous_snapshots[settlement_index];
            let raw_damage = f64::from(settlement.raw_damage);
            let Some(source) = &sources[settlement_index] else {
                let effective_damage = previous
                    .map(|snapshot| raw_damage.min((snapshot.hp - current_hp).max(0.0)))
                    .unwrap_or(raw_damage);
                if effective_damage >= MIN_FOLLOW_UP_RESIDUAL_DAMAGE {
                    unattributed.push(UnattributedServerDamage {
                        timestamp,
                        damage: effective_damage,
                        candidate_hits: 0,
                    });
                }
                continue;
            };
            let source_hit = &source.hit;
            let target_hp_before = previous
                .map(|snapshot| snapshot.hp)
                .filter(|previous_hp| *previous_hp >= current_hp)
                .unwrap_or_else(|| source_hit.target_hp_before.max(current_hp));
            let effective_damage = raw_damage.min((target_hp_before - current_hp).max(0.0));
            if let Some(source_target) = wire_handle_from_hit(source_hit)
                && source_target == settlement.target_handle
                && let Some(client_damage) = self.client_damage_by_handle.get_mut(&source_target)
            {
                let source_effective = (source_hit.damage - source_hit.overkill_damage()).max(0.0);
                *client_damage = (*client_damage + effective_damage - source_effective).max(0.0);
            }
            let correction = HitDamageCorrection {
                source_timestamp: source_hit.timestamp,
                source_char_id: source_hit.char_id,
                source_damage: source_hit.damage,
                source_target_hp_before: source_hit.target_hp_before,
                source_target_hp_after: source_hit.target_hp_after,
                source_target_max_hp: source_hit.target_max_hp,
                source_gameplay_effect_index: source_hit.gameplay_effect_index,
                damage: raw_damage,
                target_hp_before,
                target_hp_after: current_hp,
                target_hp_percent: if source_hit.target_max_hp > 0.0 {
                    current_hp / source_hit.target_max_hp * 100.0
                } else {
                    0.0
                },
                max_hp_reduction: (source.max_hp_reduction_percent > 0)
                    .then_some(raw_damage * f64::from(source.max_hp_reduction_percent) / 100.0),
                reconciled_overkill_damage: Some(if source.use_server_damage {
                    0.0
                } else {
                    (raw_damage - effective_damage).max(0.0)
                }),
            };
            self.update_pending_max_hp_reduction_hit(source_hit, &correction);
            corrections.push(correction);
        }
        for (settlement, current_hp) in settlements.iter().zip(current_hps) {
            if current_hp > 0.0 || self.residual_emitted.contains(&settlement.target_handle) {
                continue;
            }
            let Some(target_max_hp) = self
                .target_max_hp_by_handle
                .get(&settlement.target_handle)
                .copied()
                .filter(|value| value.is_finite() && *value > 0.0)
            else {
                continue;
            };
            // The terminal one-point settlement is deliberately filtered from
            // damage output. The latest wire max-HP snapshot already reflects
            // max-HP reduction mechanics, so the remaining lifecycle budget is
            // exactly that current maximum minus the ignored terminal point.
            let authoritative_damage = (target_max_hp - 1.0).max(0.0);
            let represented_damage = self
                .client_damage_by_handle
                .get(&settlement.target_handle)
                .copied()
                .unwrap_or(0.0)
                .min(target_max_hp);
            let residual_damage = (authoritative_damage - represented_damage).max(0.0);
            self.residual_emitted.insert(settlement.target_handle);
            self.residual_hits.push(server_residual_hit(
                timestamp,
                settlement,
                residual_damage,
                authoritative_damage,
                target_max_hp,
            ));
        }
        (corrections, unattributed)
    }

    fn take_residual_hits(&mut self) -> Vec<Hit> {
        std::mem::take(&mut self.residual_hits)
    }

    #[cfg(test)]
    fn observe_boss_hp(
        &mut self,
        timestamp: f64,
        update: &crate::engine::parser::ParsedBossHpUpdate,
    ) -> Option<HitDamageCorrection> {
        self.observe_boss_hp_detailed(timestamp, update).0
    }

    fn observe_boss_hp_detailed(
        &mut self,
        timestamp: f64,
        update: &crate::engine::parser::ParsedBossHpUpdate,
    ) -> (
        Option<HitDamageCorrection>,
        Option<UnattributedServerDamage>,
    ) {
        let current_hp = if update.current_hp <= 1.0 {
            0.0
        } else {
            update.current_hp as f64
        };
        self.evict_oldest_target_if_full(update.target_handle);
        let previous = self.hp_by_handle.insert(
            update.target_handle,
            ServerHpSnapshot {
                timestamp,
                hp: current_hp,
            },
        );
        self.pending_hits.retain(|pending| {
            timestamp - pending.hit.timestamp <= SERVER_DAMAGE_CALIBRATION_WINDOW_SECONDS
        });
        let Some(previous) = previous else {
            return (None, None);
        };
        if current_hp >= previous.hp {
            self.pending_hits.retain(|pending| {
                pending.target_handle != Some(update.target_handle)
                    || pending.hit.timestamp > timestamp
            });
            return (None, None);
        }
        let mut candidate_count = 0_u32;
        let mut source_index = 0_usize;
        let mut decoded_damage = 0.0_f64;
        for (index, pending) in self.pending_hits.iter().enumerate() {
            if pending.hit.timestamp > previous.timestamp
                && pending.hit.timestamp <= timestamp
                && pending.hit.target_max_hp > 0.0
                && pending.target_handle == Some(update.target_handle)
            {
                candidate_count = candidate_count.saturating_add(1);
                source_index = index;
                decoded_damage += pending.hit.damage.max(0.0);
            }
        }
        let damage = previous.hp - current_hp;
        if damage < MIN_FOLLOW_UP_RESIDUAL_DAMAGE {
            return (None, None);
        }
        if candidate_count != 1 {
            // A decoded hit is already present in team/personal totals. Report
            // only the positive portion of the authoritative HP delta that the
            // decoded candidates do not explain, regardless of candidate
            // cardinality; diagnostics must not double-count visible damage.
            let residual = (damage - decoded_damage).max(0.0);
            if residual < MIN_FOLLOW_UP_RESIDUAL_DAMAGE {
                return (None, None);
            }
            return (
                None,
                Some(UnattributedServerDamage {
                    timestamp,
                    damage: residual,
                    candidate_hits: candidate_count,
                }),
            );
        }
        let source = self.pending_hits[source_index].clone();
        self.pending_hits.retain(|pending| {
            pending.target_handle != Some(update.target_handle)
                || pending.hit.timestamp > source.hit.timestamp
        });
        let source_hit = &source.hit;
        let correction = HitDamageCorrection {
            source_timestamp: source_hit.timestamp,
            source_char_id: source_hit.char_id,
            source_damage: source_hit.damage,
            source_target_hp_before: source_hit.target_hp_before,
            source_target_hp_after: source_hit.target_hp_after,
            source_target_max_hp: source_hit.target_max_hp,
            source_gameplay_effect_index: source_hit.gameplay_effect_index,
            damage,
            target_hp_before: previous.hp,
            target_hp_after: current_hp,
            target_hp_percent: if source_hit.target_max_hp > 0.0 {
                current_hp / source_hit.target_max_hp * 100.0
            } else {
                0.0
            },
            max_hp_reduction: (source.max_hp_reduction_percent > 0)
                .then_some(damage * f64::from(source.max_hp_reduction_percent) / 100.0),
            reconciled_overkill_damage: source.use_server_damage.then_some(0.0),
        };
        self.update_pending_max_hp_reduction_hit(source_hit, &correction);
        (Some(correction), None)
    }
}

const MAX_INVENTORY_CONNECTIONS: usize = 16;
const MAX_INVENTORY_FRAGMENTS_PER_CONNECTION: usize = 4096;
const MAX_INVENTORY_STREAM_BITS: usize = 16 * 1024 * 1024;
const MAX_INVENTORY_ITEMS: usize = 4096;
// Reliable retransmission can put a missing bunch behind continuations in transport order. A
// 96-packet window covers several retry intervals while remaining a small fraction of the 10-bit
// reliable-bunch sequence space, so sequence reuse cannot bridge arbitrarily old cached fragments.
const MAX_INVENTORY_FRAGMENT_PACKET_SPAN: i64 = 96;

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
struct InventoryConnectionKey {
    source: String,
    destination: String,
}

impl InventoryConnectionKey {
    fn new(source: String, destination: String) -> Self {
        Self {
            source,
            destination,
        }
    }
}

struct InventoryBitPayload {
    data: Vec<u8>,
    bit_len: usize,
}

struct InventoryConnectionState {
    bunch_reassembler: BunchReassembler,
    character_ids: HashMap<HtItemNetId, u32>,
    module_placements: HashMap<HtItemNetId, (String, EmptyCurtainPlacement)>,
}

impl Default for InventoryConnectionState {
    fn default() -> Self {
        Self {
            bunch_reassembler: BunchReassembler::new(
                MAX_INVENTORY_FRAGMENTS_PER_CONNECTION,
                MAX_INVENTORY_STREAM_BITS,
                MAX_INVENTORY_FRAGMENT_PACKET_SPAN,
            ),
            character_ids: HashMap::new(),
            module_placements: HashMap::new(),
        }
    }
}

impl InventoryConnectionState {
    fn shares_character_identity(&self, other: &Self) -> bool {
        self.character_ids
            .iter()
            .any(|(net_id, character_id)| other.character_ids.get(net_id) == Some(character_id))
    }

    fn push_bunches(
        &mut self,
        packet_id: u16,
        bunches: Vec<SingleBunch>,
    ) -> Vec<InventoryBitPayload> {
        self.bunch_reassembler
            .observe_packet(packet_id, bunches)
            .into_iter()
            .map(|payload| InventoryBitPayload {
                data: payload.data,
                bit_len: payload.data_bit_len,
            })
            .collect()
    }
}

fn append_bounded_bits(
    destination: &mut Vec<u8>,
    destination_bit_len: &mut usize,
    source: &[u8],
    source_bit_len: usize,
    max_bits: usize,
) -> Option<()> {
    if source_bit_len > source.len().checked_mul(8)? {
        return None;
    }
    let new_bit_len = destination_bit_len.checked_add(source_bit_len)?;
    if new_bit_len > max_bits {
        return None;
    }
    destination.resize(new_bit_len.div_ceil(8), 0);
    for index in 0..source_bit_len {
        let bit = (source[index / 8] >> (index % 8)) & 1;
        let target = *destination_bit_len + index;
        destination[target / 8] |= bit << (target % 8);
    }
    *destination_bit_len = new_bit_len;
    Some(())
}

#[derive(Default)]
struct InventoryPacketResult {
    recognized: bool,
    snapshot: Option<Vec<EmptyCurtainItem>>,
    characters: Option<Vec<EmptyCurtainCharacter>>,
}

struct EmptyCurtainDecoder {
    catalog: EquipmentCatalog,
    connections: HashMap<InventoryConnectionKey, InventoryConnectionState>,
    connection_order: VecDeque<InventoryConnectionKey>,
    active_connection: Option<InventoryConnectionKey>,
    items: HashMap<HtItemNetId, EmptyCurtainItem>,
}

impl EmptyCurtainDecoder {
    fn new(catalog: EquipmentCatalog) -> Self {
        Self {
            catalog,
            connections: HashMap::new(),
            connection_order: VecDeque::new(),
            active_connection: None,
            items: HashMap::new(),
        }
    }

    fn apply_equipment_snapshot(
        &mut self,
        connection: &InventoryConnectionKey,
        snapshot: ParsedEmptyCurtainEquipmentSnapshot,
    ) -> bool {
        let (character_net_id, character_id, kind, equipped, module_placements) = match snapshot {
            ParsedEmptyCurtainEquipmentSnapshot::Modules {
                character_net_id,
                character_id,
                placements,
            } => {
                let module_placements = placements
                    .iter()
                    .map(|placement| {
                        (
                            placement.equipment,
                            (
                                placement.item_id.clone(),
                                EmptyCurtainPlacement {
                                    row: placement.row,
                                    column: placement.column,
                                },
                            ),
                        )
                    })
                    .collect::<Vec<_>>();
                let equipped = placements
                    .into_iter()
                    .map(|placement| {
                        (
                            placement.equipment,
                            Some(EmptyCurtainPlacement {
                                row: placement.row,
                                column: placement.column,
                            }),
                        )
                    })
                    .collect::<HashMap<_, _>>();
                (
                    character_net_id,
                    character_id,
                    EquipmentKind::Module,
                    equipped,
                    module_placements,
                )
            }
            ParsedEmptyCurtainEquipmentSnapshot::Core {
                character_net_id,
                character_id,
                item_id,
            } => (
                character_net_id,
                character_id,
                EquipmentKind::Core,
                item_id
                    .into_iter()
                    .map(|item_id| (item_id, None))
                    .collect::<HashMap<_, _>>(),
                Vec::new(),
            ),
        };
        if !module_placements.is_empty() {
            let state = self
                .connections
                .get_mut(connection)
                .expect("inventory snapshot connection must remain present");
            for (equipment, placement) in module_placements {
                if !state.module_placements.contains_key(&equipment)
                    && state.module_placements.len() >= MAX_INVENTORY_ITEMS
                {
                    continue;
                }
                state.module_placements.insert(equipment, placement);
            }
        }
        let mut changed = false;
        for item in self.items.values_mut() {
            let Some(definition) = self.catalog.items.get(&item.item_id) else {
                continue;
            };
            if definition.kind != kind {
                continue;
            }
            if let Some(&placement) = equipped.get(&item.id) {
                if item.character_net_id != Some(character_net_id)
                    || item.equipped_character_id != Some(character_id)
                    || item.equipped_placement != placement
                {
                    item.character_net_id = Some(character_net_id);
                    item.equipped_character_id = Some(character_id);
                    item.equipped_placement = placement;
                    changed = true;
                }
            } else if item.character_net_id == Some(character_net_id) {
                item.character_net_id = None;
                item.equipped_character_id = None;
                item.equipped_placement = None;
                changed = true;
            }
        }
        changed
    }

    fn apply_item_updates(
        &mut self,
        connection: &InventoryConnectionKey,
        parsed: Vec<EmptyCurtainItem>,
    ) -> bool {
        if self.active_connection.as_ref() != Some(connection) {
            return false;
        }
        let mut changed = false;
        let state = self
            .connections
            .get(connection)
            .expect("active inventory connection must remain present");
        for mut item in parsed {
            item.equipped_character_id = item
                .character_net_id
                .and_then(|net_id| state.character_ids.get(&net_id).copied());
            if let Some(existing) = self.items.get(&item.id)
                && existing.character_net_id == item.character_net_id
            {
                item.equipped_placement = existing.equipped_placement;
            } else if !self.items.contains_key(&item.id)
                && item.character_net_id.is_some()
                && let Some((cached_item_id, placement)) = state.module_placements.get(&item.id)
                && cached_item_id == &item.item_id
            {
                item.equipped_placement = Some(*placement);
            }
            if self.items.get(&item.id) != Some(&item) {
                if !self.items.contains_key(&item.id) && self.items.len() >= MAX_INVENTORY_ITEMS {
                    continue;
                }
                self.items.insert(item.id, item);
                changed = true;
            }
        }
        changed
    }

    fn apply_item_removals(
        &mut self,
        connection: &InventoryConnectionKey,
        removals: Vec<(HtItemNetId, String)>,
    ) -> bool {
        if self.active_connection.as_ref() != Some(connection) {
            return false;
        }
        let state = self
            .connections
            .get_mut(connection)
            .expect("active inventory connection must remain present");
        let mut changed = false;
        for (id, item_id) in removals {
            if self
                .items
                .get(&id)
                .is_some_and(|item| item.item_id == item_id)
            {
                self.items.remove(&id);
                state.module_placements.remove(&id);
                changed = true;
            }
        }
        changed
    }

    fn process_packet(
        &mut self,
        connection: InventoryConnectionKey,
        packet: &SequencedPacket,
    ) -> InventoryPacketResult {
        if !self.connections.contains_key(&connection) {
            while self.connections.len() >= MAX_INVENTORY_CONNECTIONS {
                let eviction_index = self
                    .connection_order
                    .iter()
                    .position(|stored| self.active_connection.as_ref() != Some(stored))
                    .expect("a full inventory connection cache must contain a non-active entry");
                let oldest = self
                    .connection_order
                    .remove(eviction_index)
                    .expect("the selected inventory connection must remain in the order queue");
                self.connections.remove(&oldest);
            }
            self.connection_order.push_back(connection.clone());
            self.connections
                .insert(connection.clone(), InventoryConnectionState::default());
        }

        let raw_character_ids =
            parse_empty_curtain_character_owners(&packet.payload, packet.payload_bit_len);
        let raw_character_mapping_changed = {
            let state = self
                .connections
                .get_mut(&connection)
                .expect("new or existing inventory connection must be present");
            let mut changed = false;
            for (&net_id, &character_id) in &raw_character_ids {
                if state.character_ids.insert(net_id, character_id) != Some(character_id) {
                    changed = true;
                }
            }
            changed
        };
        let initial_additions = if self.active_connection.is_none() {
            parse_empty_curtain_item_additions(
                &packet.payload,
                packet.payload_bit_len,
                &self.catalog,
            )
        } else {
            Vec::new()
        };
        let (raw_items, raw_removals) = if self.active_connection.as_ref() == Some(&connection) {
            (
                parse_empty_curtain_items(&packet.payload, packet.payload_bit_len, &self.catalog),
                parse_empty_curtain_item_removals(
                    &packet.payload,
                    packet.payload_bit_len,
                    &self.catalog,
                ),
            )
        } else {
            (initial_additions, Vec::new())
        };
        let raw_records_recognized =
            !raw_character_ids.is_empty() || !raw_items.is_empty() || !raw_removals.is_empty();
        let activated_by_raw_items = self.active_connection.is_none() && !raw_items.is_empty();
        if activated_by_raw_items {
            self.active_connection = Some(connection.clone());
        }
        let (streams, bunches_recognized) = {
            let state = self
                .connections
                .get_mut(&connection)
                .expect("new or existing inventory connection must be present");
            let known_channels = state.bunch_reassembler.known_channels().collect::<Vec<_>>();
            let bunches = parse_inventory_bunches(packet, &known_channels);
            let recognized = !bunches.is_empty();
            (state.push_bunches(packet.packet_id, bunches), recognized)
        };

        let mut items_changed = false;
        let mut characters_changed = activated_by_raw_items;
        if raw_character_mapping_changed && self.active_connection.as_ref() == Some(&connection) {
            characters_changed = true;
            let character_ids = &self
                .connections
                .get(&connection)
                .expect("active inventory connection must remain present")
                .character_ids;
            for item in self.items.values_mut() {
                let character_id = item
                    .character_net_id
                    .and_then(|net_id| character_ids.get(&net_id).copied());
                if item.equipped_character_id != character_id {
                    item.equipped_character_id = character_id;
                    items_changed = true;
                }
            }
        }
        let mut completed_stream_removals = Vec::new();
        for stream in streams {
            let stream_character_ids =
                parse_empty_curtain_character_owners(&stream.data, stream.bit_len);
            let compact_module_placements = if stream_character_ids.is_empty() {
                Vec::new()
            } else {
                parse_empty_curtain_compact_module_placements(
                    &stream.data,
                    stream.bit_len,
                    &self.items,
                    &self.catalog,
                )
            };
            let removals =
                parse_empty_curtain_item_removals(&stream.data, stream.bit_len, &self.catalog);
            let parsed = if removals.is_empty() {
                parse_empty_curtain_items(&stream.data, stream.bit_len, &self.catalog)
            } else {
                Vec::new()
            };
            if stream_character_ids.is_empty()
                && compact_module_placements.is_empty()
                && parsed.is_empty()
                && removals.is_empty()
            {
                continue;
            }
            let character_mapping_changed = {
                let state = self
                    .connections
                    .get_mut(&connection)
                    .expect("inventory connection must remain present while processing streams");
                let mut changed = false;
                for (&net_id, &character_id) in &stream_character_ids {
                    if state.character_ids.insert(net_id, character_id) != Some(character_id) {
                        changed = true;
                    }
                }
                for placement in &compact_module_placements {
                    if !state.module_placements.contains_key(&placement.equipment)
                        && state.module_placements.len() >= MAX_INVENTORY_ITEMS
                    {
                        continue;
                    }
                    state.module_placements.insert(
                        placement.equipment,
                        (
                            placement.item_id.clone(),
                            EmptyCurtainPlacement {
                                row: placement.row,
                                column: placement.column,
                            },
                        ),
                    );
                }
                changed
            };
            if !parsed.is_empty() && self.active_connection.as_ref() != Some(&connection) {
                let shares_character_identity = self
                    .active_connection
                    .as_ref()
                    .and_then(|active_connection| {
                        let active_state = self.connections.get(active_connection)?;
                        let next_state = self.connections.get(&connection)?;
                        Some(active_state.shares_character_identity(next_state))
                    })
                    .unwrap_or(false);
                self.active_connection = Some(connection.clone());
                characters_changed = true;
                if shares_character_identity {
                    let character_ids = &self
                        .connections
                        .get(&connection)
                        .expect("active inventory connection must remain present")
                        .character_ids;
                    for item in self.items.values_mut() {
                        let character_id = item
                            .character_net_id
                            .and_then(|net_id| character_ids.get(&net_id).copied());
                        if item.equipped_character_id != character_id {
                            item.equipped_character_id = character_id;
                            items_changed = true;
                        }
                    }
                } else {
                    items_changed |= !self.items.is_empty();
                    self.items.clear();
                }
            }
            if self.active_connection.as_ref() != Some(&connection) {
                continue;
            }
            completed_stream_removals.extend(removals.iter().cloned());
            items_changed |= self.apply_item_removals(&connection, removals);
            let connection_state = self
                .connections
                .get(&connection)
                .expect("active inventory connection must remain present");
            let character_ids = &connection_state.character_ids;
            let module_placements = &connection_state.module_placements;
            if character_mapping_changed {
                characters_changed = true;
                for item in self.items.values_mut() {
                    let character_id = item
                        .character_net_id
                        .and_then(|net_id| character_ids.get(&net_id).copied());
                    if item.equipped_character_id != character_id {
                        item.equipped_character_id = character_id;
                        items_changed = true;
                    }
                }
            }
            for placement in &compact_module_placements {
                let Some(item) = self.items.get_mut(&placement.equipment) else {
                    continue;
                };
                let resolved = EmptyCurtainPlacement {
                    row: placement.row,
                    column: placement.column,
                };
                if item.character_net_id.is_some()
                    && item.item_id == placement.item_id
                    && item.equipped_placement != Some(resolved)
                {
                    item.equipped_placement = Some(resolved);
                    items_changed = true;
                }
            }
            for mut item in parsed {
                item.equipped_character_id = item
                    .character_net_id
                    .and_then(|net_id| character_ids.get(&net_id).copied());
                if let Some(existing) = self.items.get(&item.id)
                    && existing.character_net_id == item.character_net_id
                {
                    item.equipped_placement = existing.equipped_placement;
                } else if !self.items.contains_key(&item.id)
                    && item.character_net_id.is_some()
                    && let Some((cached_item_id, placement)) = module_placements.get(&item.id)
                    && cached_item_id == &item.item_id
                {
                    item.equipped_placement = Some(*placement);
                }
                if self.items.get(&item.id) != Some(&item) {
                    if !self.items.contains_key(&item.id) && self.items.len() >= MAX_INVENTORY_ITEMS
                    {
                        continue;
                    }
                    self.items.insert(item.id, item);
                    items_changed = true;
                }
            }
            let equipment_snapshot = parse_empty_curtain_equipment_snapshot(
                &stream.data,
                stream.bit_len,
                &stream_character_ids,
                &self.items,
                &self.catalog,
            );
            if let Some(snapshot) = equipment_snapshot {
                items_changed |= self.apply_equipment_snapshot(&connection, snapshot);
            }
        }
        let raw_items = raw_items
            .into_iter()
            .filter(|item| {
                !completed_stream_removals
                    .iter()
                    .any(|(id, item_id)| id == &item.id && item_id == &item.item_id)
            })
            .collect();
        items_changed |= self.apply_item_updates(&connection, raw_items);
        items_changed |= self.apply_item_removals(&connection, raw_removals);
        let snapshot = (items_changed || characters_changed).then(|| {
            let mut items = self.items.values().cloned().collect::<Vec<_>>();
            items.sort_by(|left, right| {
                left.item_id
                    .cmp(&right.item_id)
                    .then_with(|| left.id.solt.cmp(&right.id.solt))
                    .then_with(|| left.id.serial.cmp(&right.id.serial))
            });
            items
        });
        let characters = (items_changed || characters_changed).then(|| {
            let mut characters = self
                .active_connection
                .as_ref()
                .and_then(|connection| self.connections.get(connection))
                .expect("changed inventory must have an active connection")
                .character_ids
                .iter()
                .map(|(net_id, character_id)| EmptyCurtainCharacter {
                    net_id: *net_id,
                    character_id: *character_id,
                })
                .collect::<Vec<_>>();
            characters.sort_by_key(|character| {
                (
                    character.character_id,
                    character.net_id.solt,
                    character.net_id.serial,
                )
            });
            characters
        });
        InventoryPacketResult {
            recognized: bunches_recognized || raw_records_recognized,
            snapshot,
            characters,
        }
    }
}

struct RecentCaptureFrame {
    sequence: u64,
    timestamp: f64,
    fingerprint: u64,
    frame_len: usize,
}

struct RecentCaptureFrameBytes {
    sequence: u64,
    bytes: Box<[u8]>,
}

#[derive(Clone, Copy)]
enum FrameTimestamp {
    Known(f64),
    Unknown,
}

#[derive(Clone, Copy)]
struct CapturedPacket<'a> {
    link_type: CaptureLinkType,
    data: &'a [u8],
    timestamp: FrameTimestamp,
}

/// Suppresses byte-for-byte duplicate capture frames reported back-to-back by
/// the capture layer. Fingerprint/length metadata covers the full 512-entry
/// window, while exact comparison bodies have a separate byte budget. A hash
/// match whose body was evicted is deliberately treated as fresh, so collision
/// or memory pressure can cause only a safe false negative, never false dedup.
struct FrameDedup {
    recent: VecDeque<RecentCaptureFrame>,
    verification: VecDeque<RecentCaptureFrameBytes>,
    retained_verification_bytes: usize,
    verification_byte_budget: usize,
    next_sequence: u64,
    last_timestamp: Option<f64>,
    fingerprint_builder: RandomState,
}

impl Default for FrameDedup {
    fn default() -> Self {
        Self::with_verification_byte_budget(FRAME_DEDUP_VERIFICATION_BYTE_BUDGET)
    }
}

impl FrameDedup {
    fn is_duplicate(&mut self, frame: &[u8], timestamp: Option<f64>) -> bool {
        let fingerprint = self.fingerprint_builder.hash_one(frame);
        self.is_duplicate_with_fingerprint(frame, timestamp, fingerprint)
    }

    fn is_duplicate_with_fingerprint(
        &mut self,
        frame: &[u8],
        timestamp: Option<f64>,
        fingerprint: u64,
    ) -> bool {
        let Some(timestamp) = timestamp.filter(|timestamp| timestamp.is_finite()) else {
            self.clear_recent();
            self.last_timestamp = None;
            return false;
        };
        if self
            .last_timestamp
            .is_some_and(|previous| timestamp < previous)
        {
            self.clear_recent();
        }
        self.last_timestamp = Some(timestamp);

        while let Some(entry) = self.recent.front() {
            if timestamp - entry.timestamp <= DUPLICATE_FRAME_WINDOW_SECONDS {
                break;
            }
            self.pop_oldest_recent();
        }
        if self.recent.iter().any(|entry| {
            entry.fingerprint == fingerprint
                && entry.frame_len == frame.len()
                && self
                    .verification
                    .iter()
                    .find(|body| body.sequence == entry.sequence)
                    .is_some_and(|body| body.bytes.as_ref() == frame)
        }) {
            return true;
        }
        if self.recent.len() == MAX_RECENT_CAPTURE_FRAMES {
            self.pop_oldest_recent();
        }
        let sequence = self.next_sequence;
        self.next_sequence = self.next_sequence.wrapping_add(1);
        if self.next_sequence == 0 {
            // Sequence identity is internal to this bounded cache. Clear on
            // wrap rather than allowing an ancient verification body to alias.
            self.clear_recent();
        }
        self.recent.push_back(RecentCaptureFrame {
            sequence,
            timestamp,
            fingerprint,
            frame_len: frame.len(),
        });
        self.retain_verification_body(sequence, frame);
        false
    }

    fn with_verification_byte_budget(verification_byte_budget: usize) -> Self {
        Self {
            recent: VecDeque::new(),
            verification: VecDeque::new(),
            retained_verification_bytes: 0,
            verification_byte_budget,
            next_sequence: 0,
            last_timestamp: None,
            fingerprint_builder: RandomState::new(),
        }
    }

    fn retain_verification_body(&mut self, sequence: u64, frame: &[u8]) {
        if frame.len() > self.verification_byte_budget {
            return;
        }
        while frame.len() > self.verification_byte_budget - self.retained_verification_bytes {
            let Some(expired) = self.verification.pop_front() else {
                self.retained_verification_bytes = 0;
                break;
            };
            self.retained_verification_bytes = self
                .retained_verification_bytes
                .saturating_sub(expired.bytes.len());
        }
        self.retained_verification_bytes += frame.len();
        self.verification.push_back(RecentCaptureFrameBytes {
            sequence,
            bytes: frame.into(),
        });
    }

    fn pop_oldest_recent(&mut self) {
        let Some(expired) = self.recent.pop_front() else {
            return;
        };
        if self
            .verification
            .front()
            .is_some_and(|body| body.sequence == expired.sequence)
            && let Some(body) = self.verification.pop_front()
        {
            self.retained_verification_bytes = self
                .retained_verification_bytes
                .saturating_sub(body.bytes.len());
        }
    }

    fn clear_recent(&mut self) {
        self.recent.clear();
        self.verification.clear();
        self.retained_verification_bytes = 0;
    }

    #[cfg(test)]
    fn retained_verification_bytes(&self) -> usize {
        self.retained_verification_bytes
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
struct GameplayEffectFragmentKey {
    source: (Ipv4Addr, u16),
    destination: (Ipv4Addr, u16),
    channel: u16,
}

struct PendingGameplayEffectFragment {
    next_sequence: u16,
    effect: Option<ParsedGameplayEffect>,
}

struct PendingBoolEnumGameplayEffectFragment {
    next_sequence: u16,
    data: Vec<u8>,
    bit_len: usize,
    pending_hit: Option<Hit>,
    updated_at: f64,
}

struct CompletedBoolEnumGameplayEffectFragment {
    hit: Hit,
    payload: Vec<u8>,
}

#[derive(Default)]
struct BoolEnumGameplayEffectFragmentObservation {
    completed: Option<CompletedBoolEnumGameplayEffectFragment>,
    abandoned_hits: Vec<Hit>,
}

#[derive(Default)]
struct BoolEnumGameplayEffectFragmentTracker {
    // One producer/consumer runs inside PacketDecoder. Streams are ordered by
    // reliable bunch sequence, capped at 64 entries and 256 KiB each. A gap,
    // replacement, timeout, or capacity eviction releases the delayed hit
    // without a guessed skill instead of dropping it or joining stale bytes.
    pending: HashMap<GameplayEffectFragmentKey, PendingBoolEnumGameplayEffectFragment>,
    order: VecDeque<GameplayEffectFragmentKey>,
}

#[derive(Default)]
struct GameplayEffectFragmentTracker {
    pending: HashMap<GameplayEffectFragmentKey, PendingGameplayEffectFragment>,
    order: VecDeque<GameplayEffectFragmentKey>,
}

impl GameplayEffectFragmentTracker {
    fn observe(
        &mut self,
        source: (Ipv4Addr, u16),
        destination: (Ipv4Addr, u16),
        bunch: &SingleBunch,
        effects: &[ParsedGameplayEffect],
    ) -> Option<ParsedGameplayEffect> {
        let key = GameplayEffectFragmentKey {
            source,
            destination,
            channel: reliable_bunch_channel(bunch.prefix),
        };
        let own_effect = match effects {
            [effect] => Some(effect.clone()),
            _ => None,
        };
        match bunch.partial_flags {
            0x09 => {
                self.insert(
                    key,
                    PendingGameplayEffectFragment {
                        next_sequence: (bunch.sequence + 1) & 0x03ff,
                        effect: own_effect,
                    },
                );
                None
            }
            0x08 | 0x0c => {
                let mut pending = self.remove(key)?;
                if pending.next_sequence != bunch.sequence {
                    return None;
                }
                let inherited = effects.is_empty().then(|| pending.effect.clone()).flatten();
                pending.effect = match effects {
                    [] => pending.effect,
                    [_] => own_effect,
                    _ => None,
                };
                pending.next_sequence = (bunch.sequence + 1) & 0x03ff;
                if bunch.partial_flags == 0x08 {
                    self.insert(key, pending);
                }
                inherited
            }
            _ => {
                self.remove(key);
                None
            }
        }
    }

    fn insert(&mut self, key: GameplayEffectFragmentKey, pending: PendingGameplayEffectFragment) {
        if self.pending.contains_key(&key) {
            self.order.retain(|stored| *stored != key);
        }
        while self.pending.len() >= MAX_GAMEPLAY_EFFECT_FRAGMENT_STREAMS {
            let Some(oldest) = self.order.pop_front() else {
                break;
            };
            self.pending.remove(&oldest);
        }
        self.order.push_back(key);
        self.pending.insert(key, pending);
    }

    fn remove(&mut self, key: GameplayEffectFragmentKey) -> Option<PendingGameplayEffectFragment> {
        let pending = self.pending.remove(&key)?;
        self.order.retain(|stored| *stored != key);
        Some(pending)
    }
}

impl BoolEnumGameplayEffectFragmentTracker {
    fn observe(
        &mut self,
        timestamp: f64,
        source: (Ipv4Addr, u16),
        destination: (Ipv4Addr, u16),
        bunch: &SingleBunch,
    ) -> BoolEnumGameplayEffectFragmentObservation {
        let mut observation = BoolEnumGameplayEffectFragmentObservation {
            abandoned_hits: self.take_expired(timestamp),
            ..Default::default()
        };
        let key = GameplayEffectFragmentKey {
            source,
            destination,
            channel: reliable_bunch_channel(bunch.prefix),
        };
        match bunch.partial_flags {
            0x09 => {
                if let Some(replaced) = self.remove(key)
                    && let Some(hit) = replaced.pending_hit
                {
                    observation.abandoned_hits.push(hit);
                }
                let mut data = Vec::new();
                let mut bit_len = 0;
                if append_bounded_bits(
                    &mut data,
                    &mut bit_len,
                    &bunch.data,
                    bunch.data_bit_len,
                    MAX_GAMEPLAY_EFFECT_FRAGMENT_BITS,
                )
                .is_some()
                {
                    observation.abandoned_hits.extend(self.insert(
                        key,
                        PendingBoolEnumGameplayEffectFragment {
                            next_sequence: (bunch.sequence + 1) & 0x03ff,
                            data,
                            bit_len,
                            pending_hit: None,
                            updated_at: timestamp,
                        },
                    ));
                }
            }
            0x08 | 0x0c => {
                let Some(mut pending) = self.remove(key) else {
                    return observation;
                };
                if pending.next_sequence != bunch.sequence
                    || append_bounded_bits(
                        &mut pending.data,
                        &mut pending.bit_len,
                        &bunch.data,
                        bunch.data_bit_len,
                        MAX_GAMEPLAY_EFFECT_FRAGMENT_BITS,
                    )
                    .is_none()
                {
                    observation
                        .abandoned_hits
                        .extend(pending.pending_hit.take());
                    return observation;
                }
                pending.next_sequence = (bunch.sequence + 1) & 0x03ff;
                pending.updated_at = timestamp;
                if bunch.partial_flags == 0x08 {
                    observation.abandoned_hits.extend(self.insert(key, pending));
                } else if let Some(hit) = pending.pending_hit {
                    observation.completed = Some(CompletedBoolEnumGameplayEffectFragment {
                        hit,
                        payload: pending.data,
                    });
                }
            }
            _ => {
                if let Some(mut removed) = self.remove(key) {
                    observation
                        .abandoned_hits
                        .extend(removed.pending_hit.take());
                }
            }
        }
        observation
    }

    fn attach_hit(
        &mut self,
        source: (Ipv4Addr, u16),
        destination: (Ipv4Addr, u16),
        bunch: &SingleBunch,
        hit: Hit,
    ) -> Option<Hit> {
        let key = GameplayEffectFragmentKey {
            source,
            destination,
            channel: reliable_bunch_channel(bunch.prefix),
        };
        let Some(pending) = self.pending.get_mut(&key) else {
            return Some(hit);
        };
        if bunch.partial_flags != 0x09
            || pending.next_sequence != (bunch.sequence + 1) & 0x03ff
            || pending.pending_hit.is_some()
        {
            return Some(hit);
        }
        pending.pending_hit = Some(hit);
        None
    }

    fn take_expired(&mut self, timestamp: f64) -> Vec<Hit> {
        let expired = self
            .order
            .iter()
            .copied()
            .filter(|key| {
                self.pending.get(key).is_some_and(|pending| {
                    timestamp - pending.updated_at > GAMEPLAY_EFFECT_FRAGMENT_TIMEOUT_SECONDS
                })
            })
            .collect::<Vec<_>>();
        expired
            .into_iter()
            .filter_map(|key| self.remove(key)?.pending_hit)
            .collect()
    }

    fn insert(
        &mut self,
        key: GameplayEffectFragmentKey,
        pending: PendingBoolEnumGameplayEffectFragment,
    ) -> Vec<Hit> {
        let mut abandoned_hits = Vec::new();
        if let Some(mut replaced) = self.remove(key) {
            abandoned_hits.extend(replaced.pending_hit.take());
        }
        while self.pending.len() >= MAX_GAMEPLAY_EFFECT_FRAGMENT_STREAMS {
            let Some(oldest) = self.order.pop_front() else {
                break;
            };
            if let Some(mut evicted) = self.pending.remove(&oldest) {
                abandoned_hits.extend(evicted.pending_hit.take());
            }
        }
        self.order.push_back(key);
        self.pending.insert(key, pending);
        abandoned_hits
    }

    fn remove(
        &mut self,
        key: GameplayEffectFragmentKey,
    ) -> Option<PendingBoolEnumGameplayEffectFragment> {
        let pending = self.pending.remove(&key)?;
        self.order.retain(|stored| *stored != key);
        Some(pending)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
struct BunchConnectionKey {
    source: (Ipv4Addr, u16),
    destination: (Ipv4Addr, u16),
}

struct BunchConnectionState {
    reassembler: BunchReassembler,
    fragment_started_at: HashMap<(u16, u16), f64>,
    fragment_start_order: VecDeque<(u16, u16)>,
}

impl BunchConnectionState {
    fn new() -> Self {
        Self {
            reassembler: BunchReassembler::new(
                MAX_BUNCH_FRAGMENTS_PER_CONNECTION,
                MAX_REASSEMBLED_BUNCH_BITS,
                MAX_BUNCH_FRAGMENT_PACKET_SPAN,
            ),
            fragment_started_at: HashMap::new(),
            fragment_start_order: VecDeque::new(),
        }
    }

    fn record_fragment_starts(&mut self, timestamp: f64, bunches: &[SingleBunch]) {
        for bunch in bunches.iter().filter(|bunch| bunch.partial_flags == 0x09) {
            let key = (reliable_bunch_channel(bunch.prefix), bunch.sequence);
            self.fragment_start_order.retain(|stored| *stored != key);
            while self.fragment_started_at.len() >= MAX_BUNCH_FRAGMENTS_PER_CONNECTION {
                let Some(oldest) = self.fragment_start_order.pop_front() else {
                    break;
                };
                self.fragment_started_at.remove(&oldest);
            }
            self.fragment_start_order.push_back(key);
            self.fragment_started_at.insert(key, timestamp);
        }
    }
}

struct ReassembledBunchObservation {
    bunch: ReassembledBunch,
    started_at: f64,
}

#[derive(Default)]
struct BunchConnectionTracker {
    states: HashMap<BunchConnectionKey, BunchConnectionState>,
    order: VecDeque<BunchConnectionKey>,
}

impl BunchConnectionTracker {
    fn observe(
        &mut self,
        source: (Ipv4Addr, u16),
        destination: (Ipv4Addr, u16),
        timestamp: f64,
        packet_id: u16,
        packet: &BunchPacket,
    ) -> Vec<ReassembledBunchObservation> {
        let key = BunchConnectionKey {
            source,
            destination,
        };
        if self.states.contains_key(&key) {
            self.order.retain(|stored| *stored != key);
        } else {
            while self.states.len() >= MAX_BUNCH_CONNECTIONS {
                let Some(oldest) = self.order.pop_front() else {
                    break;
                };
                self.states.remove(&oldest);
            }
            self.states.insert(key, BunchConnectionState::new());
        }
        self.order.push_back(key);
        let Some(state) = self.states.get_mut(&key) else {
            return Vec::new();
        };
        let bunches = packet
            .bunches
            .iter()
            .map(|located| located.bunch.clone())
            .collect::<Vec<_>>();
        state.record_fragment_starts(timestamp, &bunches);
        state
            .reassembler
            .observe_packet(packet_id, bunches)
            .into_iter()
            .map(|bunch| {
                let key = (bunch.channel, bunch.first_sequence);
                let started_at = state.fragment_started_at.remove(&key).unwrap_or(timestamp);
                state.fragment_start_order.retain(|stored| *stored != key);
                ReassembledBunchObservation { bunch, started_at }
            })
            .collect()
    }

    fn observe_sequenced(
        &mut self,
        source: (Ipv4Addr, u16),
        destination: (Ipv4Addr, u16),
        timestamp: f64,
        packet: &SequencedPacket,
        parsed: Option<&BunchPacket>,
    ) -> Vec<ReassembledBunchObservation> {
        let key = BunchConnectionKey {
            source,
            destination,
        };
        let expected = self
            .states
            .get(&key)
            .map(|state| state.reassembler.expected_continuations())
            .unwrap_or_default();
        let verified_profiles = self
            .states
            .get(&key)
            .map(|state| state.reassembler.verified_partial_profiles())
            .unwrap_or_default();
        let mut bunches = parsed
            .into_iter()
            .flat_map(|packet| packet.bunches.iter().map(|located| located.bunch.clone()))
            .collect::<Vec<_>>();
        let recovered = parse_expected_bunch_continuations(packet, &expected)
            .into_iter()
            .chain(parse_verified_bunch_starts(packet, &verified_profiles));
        for recovered in recovered {
            let identity = (
                reliable_bunch_channel(recovered.prefix),
                recovered.sequence,
                recovered.descriptor,
            );
            if let Some(existing) = bunches.iter_mut().find(|candidate| {
                (
                    reliable_bunch_channel(candidate.prefix),
                    candidate.sequence,
                    candidate.descriptor,
                ) == identity
            }) {
                *existing = recovered;
            } else {
                bunches.push(recovered);
            }
        }
        let aggregate = BunchPacket {
            packet_info_bit_len: parsed.map_or(0, |packet| packet.packet_info_bit_len),
            bunches: bunches
                .into_iter()
                .map(|bunch| crate::engine::protocol::LocatedBunch {
                    bit_offset: 0,
                    bunch,
                })
                .collect(),
        };
        self.observe(source, destination, timestamp, packet.packet_id, &aggregate)
    }
}

#[derive(Clone)]
struct HitTargetSnapshot {
    observed_at: f64,
    current_hp: f64,
    max_hp: f64,
    target_name: Option<String>,
    target_name_en: Option<String>,
    target_name_ja: Option<String>,
    target_monster_id: Option<String>,
    target_context: Vec<String>,
}

struct PacketDecoder {
    packet_emission: PacketEmissionMode,
    session_characters: HashMap<(Ipv4Addr, u16, Ipv4Addr, u16), u32>,
    client_endpoints: HashSet<(Ipv4Addr, u16)>,
    gameplay_effect_names: HashMap<u32, String>,
    ability_catalog: Arc<AbilityCatalog>,
    follow_up_damage: FollowUpDamageTracker,
    server_damage_calibration: ServerDamageCalibrationTracker,
    use_server_damage_calibration: bool,
    character_declarations: HashMap<u32, f64>,
    pending_ambiguous_hits: Vec<Hit>,
    /// Confirmed outgoing hits without a wire target wait for the following
    /// server target-state response. FIFO, bounded to
    /// `MAX_PENDING_FOLLOW_UP_HITS`; full policy emits the oldest hit without
    /// inventing a target, and capture shutdown flushes the remaining hits.
    pending_targetless_hits: VecDeque<Hit>,
    recent_confirmed_hits: Vec<Hit>,
    target_snapshots: HashMap<String, HitTargetSnapshot>,
    empty_curtain: EmptyCurtainDecoder,
    frame_dedup: FrameDedup,
    gameplay_effect_fragments: GameplayEffectFragmentTracker,
    bool_enum_gameplay_effect_fragments: BoolEnumGameplayEffectFragmentTracker,
    bunch_connections: BunchConnectionTracker,
    resource_warnings: Vec<String>,
}

#[derive(Default)]
struct PreparedHits {
    emit: Vec<Hit>,
    filtered_incoming: usize,
    deferred_ambiguous: usize,
    deferred_targetless: usize,
    suppressed_ambiguous: usize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
enum PendingTargetLocation {
    Ambiguous(usize),
    Targetless(usize),
}

type BossHpReconciliation = (
    Vec<HitFollowUp>,
    Vec<HitFollowUp>,
    Vec<HitDamageCorrection>,
    Vec<UnattributedServerDamage>,
);

impl Default for PacketDecoder {
    fn default() -> Self {
        let mut resource_warnings = Vec::new();
        let mut ability_catalog = load_resource(
            SKILL_DAMAGE_DATA_PATH,
            &mut resource_warnings,
            AbilityCatalog::load,
        );
        if let Some(path) = find_data_file(Path::new(GAMEPLAY_EFFECT_SEMANTICS_PATH)) {
            if let Err(error) = ability_catalog.apply_semantics(&path) {
                resource_warnings.push(format!("{}: {error}", path.display()));
            }
        } else {
            resource_warnings.push(format!("missing resource {GAMEPLAY_EFFECT_SEMANTICS_PATH}"));
        }
        Self::with_ability_catalog_and_warnings(Arc::new(ability_catalog), false, resource_warnings)
    }
}

impl PacketDecoder {
    fn with_ability_catalog(
        ability_catalog: Arc<AbilityCatalog>,
        use_server_damage_calibration: bool,
    ) -> Self {
        Self::with_ability_catalog_and_warnings(
            ability_catalog,
            use_server_damage_calibration,
            Vec::new(),
        )
    }

    fn with_ability_catalog_and_warnings(
        ability_catalog: Arc<AbilityCatalog>,
        use_server_damage_calibration: bool,
        mut resource_warnings: Vec<String>,
    ) -> Self {
        let gameplay_effect_names = load_resource(
            GAMEPLAY_EFFECT_MAPPING_PATH,
            &mut resource_warnings,
            load_gameplay_effect_mapping,
        );
        let equipment_catalog = load_resource(
            EQUIPMENT_CATALOG_PATH,
            &mut resource_warnings,
            load_equipment_catalog,
        );

        Self {
            packet_emission: PacketEmissionMode::FullDebug,
            session_characters: HashMap::new(),
            client_endpoints: HashSet::new(),
            gameplay_effect_names,
            ability_catalog,
            follow_up_damage: FollowUpDamageTracker::default(),
            server_damage_calibration: ServerDamageCalibrationTracker::default(),
            use_server_damage_calibration,
            character_declarations: HashMap::new(),
            pending_ambiguous_hits: Vec::new(),
            pending_targetless_hits: VecDeque::new(),
            recent_confirmed_hits: Vec::new(),
            target_snapshots: HashMap::new(),
            empty_curtain: EmptyCurtainDecoder::new(equipment_catalog),
            frame_dedup: FrameDedup::default(),
            gameplay_effect_fragments: GameplayEffectFragmentTracker::default(),
            bool_enum_gameplay_effect_fragments: BoolEnumGameplayEffectFragmentTracker::default(),
            bunch_connections: BunchConnectionTracker::default(),
            resource_warnings,
        }
    }
    #[cfg(test)]
    fn with_server_damage_calibration(use_server_damage_calibration: bool) -> Self {
        Self {
            use_server_damage_calibration,
            ..Self::default()
        }
    }
}

fn load_resource<T>(
    relative_path: &str,
    warnings: &mut Vec<String>,
    loader: impl FnOnce(&Path) -> anyhow::Result<T>,
) -> T
where
    T: Default,
{
    let path = Path::new(relative_path);
    let Some(path) = find_data_file(path) else {
        warnings.push(format!("missing resource {relative_path}"));
        return T::default();
    };
    match loader(&path) {
        Ok(value) => value,
        Err(error) => {
            warnings.push(format!("{}: {error}", path.display()));
            T::default()
        }
    }
}

impl PacketDecoder {
    fn resolve_and_observe_hit_targets(&mut self, hits: &mut [Hit]) {
        for index in 0..hits.len() {
            if hits[index].target_id.is_none() {
                let packet_target = unique_packet_target_before(hits, index);
                let tracked_target =
                    packet_target.or_else(|| self.unique_tracked_target_for_hit(&hits[index]));
                if let Some((target_id, snapshot)) = tracked_target {
                    apply_target_snapshot(&mut hits[index], target_id, &snapshot);
                }
            }
            self.observe_hit_target(&hits[index]);
        }
    }

    fn unique_tracked_target_for_hit(&self, hit: &Hit) -> Option<(String, HitTargetSnapshot)> {
        let mut candidate = None;
        for (target_id, snapshot) in &self.target_snapshots {
            if !nearly_same(snapshot.current_hp, hit.target_hp_before)
                || (snapshot.max_hp > 0.0
                    && hit.target_max_hp > 0.0
                    && !nearly_same(snapshot.max_hp, hit.target_max_hp))
            {
                continue;
            }
            if candidate.is_some() {
                return None;
            }
            candidate = Some((target_id.clone(), snapshot.clone()));
        }
        candidate
    }

    fn observe_hit_target(&mut self, hit: &Hit) {
        let Some(target_id) = hit.target_id.as_ref() else {
            return;
        };
        self.make_room_for_target(target_id);
        self.target_snapshots.insert(
            target_id.clone(),
            HitTargetSnapshot {
                observed_at: hit.timestamp,
                current_hp: hit.target_hp_after,
                max_hp: hit.target_max_hp,
                target_name: hit.target_name.clone(),
                target_name_en: hit.target_name_en.clone(),
                target_name_ja: hit.target_name_ja.clone(),
                target_monster_id: hit.target_monster_id.clone(),
                target_context: hit.target_context.clone(),
            },
        );
    }

    fn observe_target_hp_update(
        &mut self,
        timestamp: f64,
        update: &crate::engine::parser::ParsedBossHpUpdate,
    ) {
        let target_id = target_id_from_wire_handle(&update.target_handle);
        self.make_room_for_target(&target_id);
        let snapshot =
            self.target_snapshots
                .entry(target_id)
                .or_insert_with(|| HitTargetSnapshot {
                    observed_at: timestamp,
                    current_hp: f64::from(update.current_hp),
                    max_hp: 0.0,
                    target_name: None,
                    target_name_en: None,
                    target_name_ja: None,
                    target_monster_id: None,
                    target_context: Vec::new(),
                });
        snapshot.observed_at = timestamp;
        snapshot.current_hp = f64::from(update.current_hp);
    }

    fn make_room_for_target(&mut self, target_id: &str) {
        if self.target_snapshots.contains_key(target_id)
            || self.target_snapshots.len() < MAX_SERVER_DAMAGE_TARGETS
        {
            return;
        }
        if let Some(oldest) = self
            .target_snapshots
            .iter()
            .min_by(|(left_id, left), (right_id, right)| {
                left.observed_at
                    .total_cmp(&right.observed_at)
                    .then_with(|| left_id.cmp(right_id))
            })
            .map(|(target_id, _)| target_id.clone())
        {
            self.target_snapshots.remove(&oldest);
        }
    }

    fn resource_warning(&self) -> Option<String> {
        (!self.resource_warnings.is_empty()).then(|| self.resource_warnings.join("; "))
    }

    fn take_expired_ambiguous_hits(&mut self, timestamp: f64) -> Vec<Hit> {
        let mut expired = Vec::new();
        let mut pending = Vec::with_capacity(self.pending_ambiguous_hits.len());
        for hit in self.pending_ambiguous_hits.drain(..) {
            let window = if hit.direction.is_outgoing() && hit.target_id.is_none() {
                AMBIGUOUS_HIT_CONFIRMATION_WINDOW_SECONDS
            } else if is_untyped_skill_candidate(&hit) {
                UNTYPED_SHADOW_HIT_WINDOW_SECONDS
            } else {
                AMBIGUOUS_HIT_CONFIRMATION_WINDOW_SECONDS
            };
            if timestamp - hit.timestamp > window {
                expired.push(hit);
            } else {
                pending.push(hit);
            }
        }
        self.pending_ambiguous_hits = pending;
        expired
    }

    fn take_all_ambiguous_hits(&mut self) -> Vec<Hit> {
        std::mem::take(&mut self.pending_ambiguous_hits)
    }

    fn take_expired_targetless_hits(&mut self, timestamp: f64) -> Vec<Hit> {
        let mut expired = Vec::new();
        while self.pending_targetless_hits.front().is_some_and(|hit| {
            timestamp - hit.timestamp > AMBIGUOUS_HIT_CONFIRMATION_WINDOW_SECONDS
        }) {
            if let Some(hit) = self.pending_targetless_hits.pop_front() {
                expired.push(hit);
            }
        }
        expired
    }

    fn take_all_targetless_hits(&mut self) -> Vec<Hit> {
        self.pending_targetless_hits.drain(..).collect()
    }

    /// Applies a wire target only when the target-state responses and pending
    /// hits form a one-to-one exact CurrentHP match. Equal HP on multiple
    /// enemies or multiple pending hits is deliberately left unresolved.
    fn resolve_pending_hit_targets(
        &mut self,
        timestamp: f64,
        updates: &[crate::engine::parser::ParsedBossHpUpdate],
    ) -> Vec<Hit> {
        let mut update_candidates = Vec::new();
        let mut candidate_handles = HashMap::<PendingTargetLocation, HashSet<[u8; 29]>>::new();
        for update in updates {
            let mut candidates = Vec::new();
            for (index, hit) in self.pending_ambiguous_hits.iter().enumerate() {
                if pending_hit_matches_target_update(hit, timestamp, update) {
                    let location = PendingTargetLocation::Ambiguous(index);
                    candidates.push(location);
                    candidate_handles
                        .entry(location)
                        .or_default()
                        .insert(update.target_handle);
                }
            }
            for (index, hit) in self.pending_targetless_hits.iter().enumerate() {
                if pending_hit_matches_target_update(hit, timestamp, update) {
                    let location = PendingTargetLocation::Targetless(index);
                    candidates.push(location);
                    candidate_handles
                        .entry(location)
                        .or_default()
                        .insert(update.target_handle);
                }
            }
            update_candidates.push((update.target_handle, candidates));
        }

        let mut resolved_locations = HashMap::<PendingTargetLocation, [u8; 29]>::new();
        for (target_handle, candidates) in update_candidates {
            let [location] = candidates.as_slice() else {
                continue;
            };
            if candidate_handles
                .get(location)
                .is_some_and(|handles| handles.len() == 1)
            {
                resolved_locations.insert(*location, target_handle);
            }
        }

        for (index, hit) in self.pending_ambiguous_hits.iter_mut().enumerate() {
            let location = PendingTargetLocation::Ambiguous(index);
            let Some(target_handle) = resolved_locations.get(&location) else {
                continue;
            };
            let target_id = target_id_from_wire_handle(target_handle);
            if let Some(snapshot) = self.target_snapshots.get(&target_id) {
                apply_target_snapshot(hit, target_id, snapshot);
            }
        }

        let mut targetless = resolved_locations
            .into_iter()
            .filter_map(|(location, handle)| match location {
                PendingTargetLocation::Targetless(index) => Some((index, handle)),
                PendingTargetLocation::Ambiguous(_) => None,
            })
            .collect::<Vec<_>>();
        targetless.sort_unstable_by_key(|(index, _)| std::cmp::Reverse(*index));
        let mut resolved = Vec::with_capacity(targetless.len());
        for (index, target_handle) in targetless {
            let Some(mut hit) = self.pending_targetless_hits.remove(index) else {
                continue;
            };
            let target_id = target_id_from_wire_handle(&target_handle);
            if let Some(snapshot) = self.target_snapshots.get(&target_id) {
                apply_target_snapshot(&mut hit, target_id, snapshot);
                resolved.push(hit);
            }
        }
        resolved.sort_by(|left, right| left.timestamp.total_cmp(&right.timestamp));
        resolved
    }

    fn emit_hits(
        &mut self,
        hits: impl IntoIterator<Item = Hit>,
        characters: &HashMap<u32, CharacterInfo>,
        sender: &EngineEventSink,
    ) {
        self.emit_hits_inner(hits, characters, sender, true);
    }

    fn emit_preobserved_hits(
        &mut self,
        hits: impl IntoIterator<Item = Hit>,
        characters: &HashMap<u32, CharacterInfo>,
        sender: &EngineEventSink,
    ) {
        self.emit_hits_inner(hits, characters, sender, false);
    }

    fn emit_hits_inner(
        &mut self,
        hits: impl IntoIterator<Item = Hit>,
        characters: &HashMap<u32, CharacterInfo>,
        sender: &EngineEventSink,
        observe_server_damage: bool,
    ) {
        for hit in hits {
            if is_enemy_death_settlement_hit(&hit) {
                continue;
            }
            self.follow_up_damage
                .observe_hit(&hit, hit.gameplay_effect_index, characters);
            let max_hp_reduction_correction = observe_server_damage
                .then(|| self.observe_server_damage_hit(&hit))
                .flatten();
            let _ = sender.send(EngineEvent::Hit(Box::new(hit)));
            if let Some(correction) = max_hp_reduction_correction {
                let _ = sender.send(EngineEvent::HitDamageCorrection(correction));
            }
        }
    }

    fn observe_server_damage_hits<'a>(
        &mut self,
        hits: impl IntoIterator<Item = &'a Hit>,
    ) -> Vec<HitDamageCorrection> {
        hits.into_iter()
            .filter(|hit| !is_enemy_death_settlement_hit(hit))
            .filter_map(|hit| self.observe_server_damage_hit(hit))
            .collect()
    }

    fn observe_server_damage_hit(&mut self, hit: &Hit) -> Option<HitDamageCorrection> {
        // HP-delta evidence remains observable even when automatic correction
        // is disabled. Same-packet settlements must see the hit before event
        // emission, otherwise their authoritative value cannot be matched.
        let semantic = hit
            .gameplay_effect_name
            .as_deref()
            .and_then(|effect_name| self.ability_catalog.skill(effect_name));
        self.server_damage_calibration.observe_hit_with_semantics(
            hit,
            semantic.is_some_and(|skill| skill.use_server_damage),
            semantic.map_or(0, |skill| skill.max_hp_reduction_percent),
        )
    }

    fn prepare_hits_for_emission(
        &mut self,
        hits: Vec<Hit>,
        declared_ids: &[u32],
        include_incoming: bool,
        characters: &HashMap<u32, CharacterInfo>,
    ) -> PreparedHits {
        let mut prepared = PreparedHits::default();
        for mut hit in hits {
            self.recent_confirmed_hits.retain(|confirmed| {
                hit.timestamp - confirmed.timestamp <= RECENT_CONFIRMED_HIT_WINDOW_SECONDS
            });
            if !include_incoming && hit.direction.is_incoming() {
                prepared.filtered_incoming += 1;
                continue;
            }
            if gameplay_effect_confirms_session_hit(&hit, declared_ids, characters) {
                hit.direction = HitDirection::Outgoing;
                hit.char_source = HitCharacterSource::GameplayEffect;
            }
            if is_recent_confirmed_duplicate(&hit, &self.recent_confirmed_hits) {
                prepared.suppressed_ambiguous += 1;
                continue;
            }
            if is_ambiguous_session_hit(&hit, declared_ids) {
                self.pending_ambiguous_hits.push(hit);
                prepared.deferred_ambiguous += 1;
                continue;
            }
            if is_untyped_skill_candidate(&hit)
                && self.pending_ambiguous_hits.len() < MAX_PENDING_FOLLOW_UP_HITS
            {
                self.pending_ambiguous_hits.push(hit);
                prepared.deferred_ambiguous += 1;
                continue;
            }
            prepared.suppressed_ambiguous += self.suppress_matching_ambiguous_hits(&hit);
            if is_confirmed_packet_hit(&hit) {
                self.recent_confirmed_hits.push(hit.clone());
            }
            if hit.direction.is_outgoing() && hit.target_id.is_none() && hit.target_max_hp > 0.0 {
                if self.pending_targetless_hits.len() >= MAX_PENDING_FOLLOW_UP_HITS
                    && let Some(oldest) = self.pending_targetless_hits.pop_front()
                {
                    prepared.emit.push(oldest);
                }
                self.pending_targetless_hits.push_back(hit);
                prepared.deferred_targetless += 1;
                continue;
            }
            prepared.emit.push(hit);
        }
        prepared
    }

    fn resolve_pending_untyped_skills(&mut self, confirmed_hits: &[Hit]) -> Vec<Hit> {
        let mut resolved = Vec::new();
        let mut pending = Vec::with_capacity(self.pending_ambiguous_hits.len());
        let mut matched_confirmed = vec![false; confirmed_hits.len()];
        for mut candidate in self.pending_ambiguous_hits.drain(..) {
            if let Some((index, confirmed_hit)) =
                confirmed_hits
                    .iter()
                    .enumerate()
                    .find(|(index, confirmed)| {
                        !matched_confirmed[*index]
                            && exact_untyped_skill_match(&candidate, confirmed)
                    })
            {
                matched_confirmed[index] = true;
                copy_skill_attribution(&mut candidate, confirmed_hit);
                resolved.push(candidate);
            } else {
                pending.push(candidate);
            }
        }
        self.pending_ambiguous_hits = pending;
        resolved
    }

    fn take_new_reassembled_hits(
        &mut self,
        mut confirmed_hits: Vec<Hit>,
        resolved_pending: &[Hit],
        fragment_started_at: f64,
    ) -> Vec<Hit> {
        let mut unique_confirmed = Vec::with_capacity(confirmed_hits.len());
        for hit in confirmed_hits.drain(..) {
            if !unique_confirmed
                .iter()
                .any(|stored| same_exact_wire_damage_event(stored, &hit))
            {
                unique_confirmed.push(hit);
            }
        }
        let mut matched_resolved = vec![false; resolved_pending.len()];
        let mut matched_recent = vec![false; self.recent_confirmed_hits.len()];
        let mut recovered = Vec::new();
        for confirmed in unique_confirmed {
            if let Some((index, _)) =
                resolved_pending
                    .iter()
                    .enumerate()
                    .find(|(index, resolved)| {
                        !matched_resolved[*index]
                            && same_exact_wire_damage_event(resolved, &confirmed)
                    })
            {
                matched_resolved[index] = true;
                continue;
            }
            if let Some((index, _)) =
                self.recent_confirmed_hits
                    .iter()
                    .enumerate()
                    .find(|(index, recent)| {
                        !matched_recent[*index]
                            && recent.timestamp + f64::EPSILON >= fragment_started_at
                            && (confirmed.timestamp - recent.timestamp).abs()
                                <= UNTYPED_SHADOW_HIT_WINDOW_SECONDS
                            && same_exact_wire_damage_event(recent, &confirmed)
                    })
            {
                matched_recent[index] = true;
                continue;
            }
            recovered.push(confirmed);
        }
        recovered
    }

    /// Reconciles this packet's boss-HP-sync candidates against the pending
    /// hits queued in each of the three damage-reconciliation mechanisms.
    ///
    /// A boss-HP delta already explained by a reaction follow-up (e.g. 覆纹) is
    /// fully accounted for: source damage + residual == the observed delta by
    /// construction. Handing that same delta to the legacy kill-merge or the
    /// server-damage-calibration pass as well would make them treat the whole
    /// delta as an undiscovered correction to the base hit, silently
    /// overwriting a damage value that was already correct and erasing the
    /// follow-up attribution in the process. So each update is *claimed* by at
    /// most one attribution mechanism, with the reaction follow-up given first
    /// refusal. Conservative mode never turns an HP-sync residual into guessed
    /// character damage; aggressive calibration remains an explicit opt-in.
    /// The calibration tracker still gets to *observe* every update regardless,
    /// since it keeps its own HP snapshot/pending-hit state
    /// (`ServerDamageCalibrationTracker::hp_by_handle`); skipping the call
    /// entirely on a claimed update would leave that state stale and make its
    /// *next* correction compare against the wrong baseline.
    fn reconcile_boss_hp_updates(
        &mut self,
        timestamp: f64,
        boss_hp_updates: &[crate::engine::parser::ParsedBossHpUpdate],
    ) -> BossHpReconciliation {
        let mut inferred_follow_ups = Vec::new();
        let hp_sync_follow_ups = Vec::new();
        let mut server_damage_corrections = Vec::new();
        let mut unattributed_server_damage = Vec::new();
        for update in boss_hp_updates {
            let follow_up = self
                .follow_up_damage
                .observe_server_hp(timestamp, update.current_hp as f64);
            let claimed = follow_up.is_some();
            inferred_follow_ups.extend(follow_up);
            let (correction, unattributed) = self
                .server_damage_calibration
                .observe_boss_hp_detailed(timestamp, update);
            if !claimed {
                unattributed_server_damage.extend(unattributed);
                let force_server_damage = correction
                    .as_ref()
                    .is_some_and(|correction| correction.reconciled_overkill_damage == Some(0.0));
                if self.use_server_damage_calibration || force_server_damage {
                    server_damage_corrections.extend(correction);
                } else if let Some(correction) = correction {
                    // The decoded hit already contributes source_damage to
                    // team/personal totals. Conservative mode publishes only
                    // the positive residual as unassigned evidence and never
                    // fabricates a follow-up for a likely character.
                    let residual = correction.damage - correction.source_damage;
                    if residual >= MIN_FOLLOW_UP_RESIDUAL_DAMAGE {
                        unattributed_server_damage.push(UnattributedServerDamage {
                            timestamp,
                            damage: residual,
                            candidate_hits: 1,
                        });
                    }
                }
            }
        }
        (
            inferred_follow_ups,
            hp_sync_follow_ups,
            server_damage_corrections,
            unattributed_server_damage,
        )
    }

    fn reconcile_server_damage_settlements(
        &mut self,
        timestamp: f64,
        settlements: &[ParsedServerDamageSettlement],
    ) -> (
        Vec<HitDamageCorrection>,
        Vec<UnattributedServerDamage>,
        Vec<Hit>,
    ) {
        let (corrections, unattributed) = self
            .server_damage_calibration
            .observe_server_damage_settlements(timestamp, settlements);
        let residual_hits = self.server_damage_calibration.take_residual_hits();
        (corrections, unattributed, residual_hits)
    }

    fn reconcile_current_packet_server_damage_settlements<'a>(
        &mut self,
        timestamp: f64,
        settlements: &[ParsedServerDamageSettlement],
        hits: impl IntoIterator<Item = &'a Hit>,
    ) -> (
        Vec<HitDamageCorrection>,
        Vec<UnattributedServerDamage>,
        Vec<Hit>,
    ) {
        // A server settlement can be decoded from the same packet as its
        // client hit. Queue those hits before reconciliation while callers
        // preserve the public Hit -> HitDamageCorrection event order.
        let mut max_hp_reduction_corrections = self.observe_server_damage_hits(hits);
        let (mut corrections, unattributed, residual_hits) =
            self.reconcile_server_damage_settlements(timestamp, settlements);
        max_hp_reduction_corrections.append(&mut corrections);
        (max_hp_reduction_corrections, unattributed, residual_hits)
    }

    fn suppress_matching_ambiguous_hits(&mut self, confirmed_hit: &Hit) -> usize {
        if !is_confirmed_packet_hit(confirmed_hit) {
            return 0;
        }
        let before = self.pending_ambiguous_hits.len();
        self.pending_ambiguous_hits
            .retain(|pending| !same_damage_event(pending, confirmed_hit));
        before - self.pending_ambiguous_hits.len()
    }
}

fn is_ambiguous_session_hit(hit: &Hit, declared_ids: &[u32]) -> bool {
    declared_ids.len() > 1
        && hit.char_source == HitCharacterSource::Session
        && hit.direction.is_unknown()
        && hit.gameplay_effect_index.is_some()
}

fn gameplay_effect_confirms_session_hit(
    hit: &Hit,
    declared_ids: &[u32],
    characters: &HashMap<u32, CharacterInfo>,
) -> bool {
    if declared_ids.len() <= 1
        || hit.char_source != HitCharacterSource::Session
        || !hit.direction.is_unknown()
        || !declared_ids.contains(&hit.char_id)
    {
        return false;
    }
    let Some(effect_name) = hit.gameplay_effect_name.as_deref() else {
        return false;
    };
    let Some(character) = characters.get(&hit.char_id) else {
        return false;
    };
    let name = character.name_en.trim();
    !name.is_empty()
        && effect_name
            .to_ascii_lowercase()
            .contains(&name.to_ascii_lowercase())
}

fn is_confirmed_packet_hit(hit: &Hit) -> bool {
    matches!(
        hit.char_source,
        HitCharacterSource::Packet | HitCharacterSource::GameplayEffect
    ) && hit.direction.is_outgoing()
}

fn same_damage_event(left: &Hit, right: &Hit) -> bool {
    (left.timestamp - right.timestamp).abs() <= AMBIGUOUS_HIT_CONFIRMATION_WINDOW_SECONDS
        && left.gameplay_effect_index.is_some()
        && left.gameplay_effect_index == right.gameplay_effect_index
        && nearly_same(left.damage, right.damage)
        && nearly_same(left.target_hp_before, right.target_hp_before)
        && nearly_same(left.target_hp_after, right.target_hp_after)
        && nearly_same(left.target_max_hp, right.target_max_hp)
}

fn is_recent_confirmed_duplicate(hit: &Hit, confirmed_hits: &[Hit]) -> bool {
    confirmed_hits.iter().any(|confirmed| {
        let timestamp_delta = (hit.timestamp - confirmed.timestamp).abs();
        timestamp_delta <= UNTYPED_SHADOW_HIT_WINDOW_SECONDS
            && exact_untyped_skill_match(hit, confirmed)
    })
}

fn is_untyped_skill_candidate(hit: &Hit) -> bool {
    hit.direction.is_outgoing()
        && hit.char_source == HitCharacterSource::Packet
        && hit.gameplay_effect_index.is_none()
}

fn exact_untyped_skill_match(candidate: &Hit, confirmed: &Hit) -> bool {
    (is_untyped_skill_candidate(candidate) || candidate.target_id.is_none())
        && confirmed.gameplay_effect_index.is_some()
        && (candidate.timestamp - confirmed.timestamp).abs() <= UNTYPED_SHADOW_HIT_WINDOW_SECONDS
        && same_wire_damage_snapshot(candidate, confirmed)
        && match (candidate.wire_event, confirmed.wire_event) {
            (Some(candidate), Some(confirmed)) => candidate == confirmed,
            _ => true,
        }
        && candidate
            .target_id
            .as_ref()
            .is_none_or(|target| Some(target) == confirmed.target_id.as_ref())
}

fn same_exact_wire_damage_event(left: &Hit, right: &Hit) -> bool {
    if !same_wire_damage_snapshot(left, right) {
        return false;
    }
    match (left.wire_event, right.wire_event) {
        (Some(left_event), Some(right_event)) => left_event == right_event,
        _ => left.target_id.is_some() && left.target_id == right.target_id,
    }
}

fn extend_unique_exact_wire_hits(
    hits: &mut Vec<Hit>,
    additional_hits: impl IntoIterator<Item = Hit>,
) {
    for hit in additional_hits {
        if !hits
            .iter()
            .any(|existing| same_exact_wire_damage_event(existing, &hit))
        {
            hits.push(hit);
        }
    }
}

fn same_wire_damage_snapshot(left: &Hit, right: &Hit) -> bool {
    left.char_id == right.char_id
        && nearly_same(left.damage, right.damage)
        && nearly_same(left.target_hp_before, right.target_hp_before)
        && nearly_same(left.target_hp_after, right.target_hp_after)
        && nearly_same(left.target_max_hp, right.target_max_hp)
}

fn copy_skill_attribution(target: &mut Hit, source: &Hit) {
    target.gameplay_effect_index = source.gameplay_effect_index;
    target
        .gameplay_effect_name
        .clone_from(&source.gameplay_effect_name);
    target.ability_name.clone_from(&source.ability_name);
    target.attack_type.clone_from(&source.attack_type);
    target.damage_component.clone_from(&source.damage_component);
    if target.damage_attribute.is_none() {
        target.damage_attribute.clone_from(&source.damage_attribute);
    }
    if target.target_id.is_none() {
        target.target_id.clone_from(&source.target_id);
        target.target_name.clone_from(&source.target_name);
        target.target_name_en.clone_from(&source.target_name_en);
        target.target_name_ja.clone_from(&source.target_name_ja);
        target
            .target_monster_id
            .clone_from(&source.target_monster_id);
        target.target_context.clone_from(&source.target_context);
    }
}

fn propagate_unanimous_trailing_skill(hits: &mut [Hit]) {
    let Some((last, previous)) = hits.split_last_mut() else {
        return;
    };
    if last.gameplay_effect_index.is_some() {
        return;
    }
    if previous.iter().any(|candidate| {
        candidate.char_id == last.char_id
            && candidate.gameplay_effect_index.is_some()
            && nearly_same(candidate.damage, last.damage)
    }) {
        return;
    }
    let mut source = None::<&Hit>;
    let mut mapped_count = 0_usize;
    for candidate in previous
        .iter()
        .filter(|candidate| candidate.char_id == last.char_id)
        .filter(|candidate| candidate.gameplay_effect_index.is_some())
    {
        mapped_count += 1;
        if source
            .is_some_and(|stored| stored.gameplay_effect_index != candidate.gameplay_effect_index)
        {
            return;
        }
        source = Some(candidate);
    }
    if mapped_count >= 2
        && let Some(source) = source
    {
        copy_skill_attribution(last, source);
    }
}

fn nearly_same(left: f64, right: f64) -> bool {
    (left - right).abs() <= 0.5
}

fn normalized_server_hp(current_hp: f32) -> f64 {
    if current_hp <= 1.0 {
        0.0
    } else {
        f64::from(current_hp)
    }
}

fn server_residual_hit(
    timestamp: f64,
    settlement: &ParsedServerDamageSettlement,
    damage: f64,
    authoritative_damage: f64,
    target_max_hp: f64,
) -> Hit {
    Hit {
        timestamp,
        char_id: 0,
        char_name: "Unattributed".to_owned(),
        char_known: false,
        damage,
        byte_offset: settlement.byte_offset,
        bit_shift: settlement.bit_shift,
        char_source: HitCharacterSource::Unknown,
        direction: HitDirection::Outgoing,
        target_hp_before: authoritative_damage,
        target_hp_after: 0.0,
        target_max_hp,
        max_hp_reduction: 0.0,
        target_hp_percent: 0.0,
        target_id: Some(target_id_from_wire_handle(&settlement.target_handle)),
        target_name: None,
        target_name_en: None,
        target_name_ja: None,
        target_monster_id: None,
        target_context: vec![format!(
            "enemy_target_wire={}",
            hex::encode(settlement.target_handle)
        )],
        gameplay_effect_index: None,
        gameplay_effect_name: None,
        ability_name: None,
        damage_name: Some("Server settlement residual".to_owned()),
        damage_component: None,
        attack_type: None,
        damage_attribute: None,
        follow_up_damage: 0.0,
        follow_up_timestamp: None,
        follow_up_damage_name: None,
        follow_up_attack_type: None,
        follow_up_damage_attribute: None,
        reconciled_overkill_damage: Some(0.0),
        wire_event: None,
    }
}

fn is_enemy_death_settlement_hit(hit: &Hit) -> bool {
    hit.direction.is_outgoing()
        && hit.target_id.is_some()
        && hit.damage == 1.0
        && hit.target_hp_before == 1.0
        && hit.target_hp_after == 0.0
}

fn pending_hit_matches_target_update(
    hit: &Hit,
    update_timestamp: f64,
    update: &crate::engine::parser::ParsedBossHpUpdate,
) -> bool {
    hit.direction.is_outgoing()
        && hit.target_id.is_none()
        && update_timestamp >= hit.timestamp
        && update_timestamp - hit.timestamp <= AMBIGUOUS_HIT_CONFIRMATION_WINDOW_SECONDS
        && nearly_same(hit.target_hp_after, f64::from(update.current_hp))
}

fn target_id_from_wire_handle(handle: &[u8; 29]) -> String {
    format!("enemy-wire:{}", hex::encode(handle))
}

fn wire_handle_from_hit(hit: &Hit) -> Option<[u8; 29]> {
    let encoded = hit.target_id.as_deref()?.strip_prefix("enemy-wire:")?;
    let mut handle = [0_u8; 29];
    hex::decode_to_slice(encoded, &mut handle).ok()?;
    Some(handle)
}

fn target_snapshot_from_hit(hit: &Hit) -> HitTargetSnapshot {
    HitTargetSnapshot {
        observed_at: hit.timestamp,
        current_hp: hit.target_hp_after,
        max_hp: hit.target_max_hp,
        target_name: hit.target_name.clone(),
        target_name_en: hit.target_name_en.clone(),
        target_name_ja: hit.target_name_ja.clone(),
        target_monster_id: hit.target_monster_id.clone(),
        target_context: hit.target_context.clone(),
    }
}

fn unique_packet_target_before(hits: &[Hit], index: usize) -> Option<(String, HitTargetSnapshot)> {
    let hit = hits.get(index)?;
    let mut candidate = None::<(String, HitTargetSnapshot)>;
    for previous in &hits[..index] {
        if previous.char_id != hit.char_id
            || !nearly_same(previous.target_hp_before, hit.target_hp_before)
            || !nearly_same(previous.target_max_hp, hit.target_max_hp)
        {
            continue;
        }
        let Some(target_id) = previous.target_id.as_ref() else {
            continue;
        };
        if candidate
            .as_ref()
            .is_some_and(|(stored_id, _)| stored_id != target_id)
        {
            return None;
        }
        candidate = Some((target_id.clone(), target_snapshot_from_hit(previous)));
    }
    candidate
}

fn apply_target_snapshot(hit: &mut Hit, target_id: String, snapshot: &HitTargetSnapshot) {
    hit.target_id = Some(target_id);
    hit.target_name.clone_from(&snapshot.target_name);
    hit.target_name_en.clone_from(&snapshot.target_name_en);
    hit.target_name_ja.clone_from(&snapshot.target_name_ja);
    hit.target_monster_id
        .clone_from(&snapshot.target_monster_id);
    hit.target_context.clone_from(&snapshot.target_context);
}

fn fuwen_start_pair(
    payload: &[u8],
    evidence: &[(u32, u8, usize)],
    characters: &HashMap<u32, CharacterInfo>,
) -> Option<(u32, u32)> {
    if !matches_shifted_bytes_at(
        payload,
        FUWEN_START_SIGNATURE_SHIFT,
        FUWEN_START_SIGNATURE_OFFSET,
        FUWEN_START_SIGNATURE,
    ) {
        return None;
    }
    let entering_character_id = character_id_at_evidence_location(
        evidence,
        FUWEN_ENTERING_ID_SHIFT,
        FUWEN_ENTERING_ID_OFFSET,
    )?;
    let previous_character_id = character_id_at_evidence_location(
        evidence,
        FUWEN_PREVIOUS_ID_SHIFT,
        FUWEN_PREVIOUS_ID_OFFSET,
    )?;
    if entering_character_id == previous_character_id {
        return None;
    }
    let entering_attribute = characters
        .get(&entering_character_id)
        .and_then(|character| character.attribute.as_deref())?;
    let previous_attribute = characters
        .get(&previous_character_id)
        .and_then(|character| character.attribute.as_deref())?;
    let has_fuwen_pair = (entering_attribute == "灵" && previous_attribute == "咒")
        || (entering_attribute == "咒" && previous_attribute == "灵");
    has_fuwen_pair.then_some((entering_character_id, previous_character_id))
}

fn character_id_at_evidence_location(
    evidence: &[(u32, u8, usize)],
    bit_shift: u8,
    byte_offset: usize,
) -> Option<u32> {
    evidence
        .iter()
        .find(|(_, shift, offset)| *shift == bit_shift && *offset == byte_offset)
        .map(|(character_id, _, _)| *character_id)
}

fn damage_record_source_character(
    hit: &Hit,
    evidence: &[(u32, u8, usize)],
    encoding: DamageRecordEncoding,
) -> Option<u32> {
    let hit_bit_offset = hit.byte_offset * 8 + usize::from(hit.bit_shift);
    let (before_distance, after_distance) = match encoding {
        DamageRecordEncoding::LegacyInt32 => LEGACY_DAMAGE_RECORD_SOURCE_CHARACTER_BITS,
        DamageRecordEncoding::BoolAndEnums => BOOL_ENUM_DAMAGE_RECORD_SOURCE_CHARACTER_BITS,
    };
    let before_bit_offset = hit_bit_offset.checked_sub(before_distance)?;
    let after_bit_offset = hit_bit_offset.checked_add(after_distance)?;
    let character_at = |bit_offset: usize| {
        evidence
            .iter()
            .find(|(_, shift, offset)| offset * 8 + usize::from(*shift) == bit_offset)
            .map(|(character_id, _, _)| *character_id)
    };
    let before = character_at(before_bit_offset)?;
    let after = character_at(after_bit_offset)?;
    (before == after).then_some(before)
}

/// Uses the two character anchors that bracket a damage record to recover its
/// exact outgoing owner. This record-local evidence is stronger than a
/// packet-wide bit alignment or the connection's last single-character owner.
/// It also repairs a packet-wide false `Incoming` only when the record has an
/// exact enemy wire target and a known outgoing effect naming that same owner.
fn reattribute_hit_from_damage_record_owner(
    hit: &mut Hit,
    evidence: &[(u32, u8, usize)],
    encoding: DamageRecordEncoding,
    characters: &HashMap<u32, CharacterInfo>,
) {
    let Some(character_id) = damage_record_source_character(hit, evidence, encoding) else {
        return;
    };
    let recover_false_incoming = hit.direction.is_incoming()
        && character_id != hit.char_id
        && wire_handle_from_hit(hit).is_some()
        && hit
            .gameplay_effect_name
            .as_deref()
            .is_some_and(|effect_name| {
                is_known_outgoing_damage_effect(effect_name, None)
                    && characters.get(&character_id).is_some_and(|character| {
                        let owner_name = character.name_en.trim().as_bytes();
                        !owner_name.is_empty()
                            && effect_name
                                .as_bytes()
                                .windows(owner_name.len())
                                .any(|window| window.eq_ignore_ascii_case(owner_name))
                    })
            });
    if !hit.direction.is_outgoing() && !recover_false_incoming {
        return;
    }
    if recover_false_incoming {
        hit.direction = HitDirection::Outgoing;
    }
    if character_id != hit.char_id {
        set_hit_character(hit, character_id, characters);
    }
    hit.char_source = HitCharacterSource::Packet;
}

fn character_debug_label(character_id: u32, characters: &HashMap<u32, CharacterInfo>) -> String {
    characters.get(&character_id).map_or_else(
        || character_id.to_string(),
        |character| {
            let name = if character.name_zh.is_empty() {
                character.name_en.as_str()
            } else {
                character.name_zh.as_str()
            };
            match character.attribute.as_deref() {
                Some(attribute) if !name.is_empty() => {
                    format!("{name}({character_id}/{attribute})")
                }
                Some(attribute) => format!("{character_id}/{attribute}"),
                None if !name.is_empty() => format!("{name}({character_id})"),
                None => character_id.to_string(),
            }
        },
    )
}

fn matching_gameplay_effect<'a>(
    hit: &Hit,
    effects: &'a [ParsedGameplayEffect],
    previous_hit_bit_offset: Option<usize>,
) -> Option<&'a ParsedGameplayEffect> {
    let hit_bit_offset = hit.byte_offset * 8 + usize::from(hit.bit_shift);
    let belongs_to_current_record = |effect: &ParsedGameplayEffect| {
        let effect_bit_offset = effect.byte_offset * 8 + usize::from(effect.bit_shift);
        effect_bit_offset < hit_bit_offset
            && previous_hit_bit_offset.is_none_or(|previous| effect_bit_offset > previous)
    };
    effects
        .iter()
        .find(|effect| {
            effect.byte_offset * 8 + usize::from(effect.bit_shift)
                == hit_bit_offset + DAMAGE_RECORD_TO_COMPACT_GAMEPLAY_EFFECT_BITS
        })
        .or_else(|| {
            effects.iter().find(|effect| {
                belongs_to_current_record(effect)
                    && effect.byte_offset * 8
                        + usize::from(effect.bit_shift)
                        + LEGACY_GAMEPLAY_EFFECT_TO_DAMAGE_RECORD_BITS
                        == hit_bit_offset
            })
        })
        .or_else(|| {
            // A fragmented bunch can carry the GE in its head and the sole
            // damage record in its tail, so their packet-local offsets differ.
            (previous_hit_bit_offset.is_none() && effects.len() == 1).then(|| &effects[0])
        })
}

fn read_u32_at_bit_offset(data: &[u8], bit_offset: usize) -> Option<u32> {
    let byte_offset = bit_offset / 8;
    let bit_shift = bit_offset % 8;
    let mut decoded = [0_u8; 4];
    for (index, byte) in decoded.iter_mut().enumerate() {
        let source_offset = byte_offset.checked_add(index)?;
        let current = *data.get(source_offset)?;
        let mut value = u16::from(current) >> bit_shift;
        if bit_shift != 0 {
            value |= u16::from(*data.get(source_offset.checked_add(1)?)?) << (8 - bit_shift);
        }
        *byte = value as u8;
    }
    Some(u32::from_le_bytes(decoded))
}

fn matching_bool_enum_gameplay_effect(
    payload: &[u8],
    hit: &Hit,
    encoding: DamageRecordEncoding,
    names: &HashMap<u32, String>,
    ability_catalog: &AbilityCatalog,
) -> Option<ParsedGameplayEffect> {
    if encoding != DamageRecordEncoding::BoolAndEnums {
        return None;
    }
    let hit_bit_offset = hit.byte_offset * 8 + usize::from(hit.bit_shift);
    BOOL_ENUM_DAMAGE_RECORD_TO_GAMEPLAY_EFFECT_BITS
        .into_iter()
        .find_map(|distance| {
            let effect_bit_offset = hit_bit_offset.checked_add(distance)?;
            let unique_index = read_u32_at_bit_offset(payload, effect_bit_offset)?;
            let effect_name = names.get(&unique_index)?;
            is_damage_gameplay_effect(effect_name, ability_catalog.skill(effect_name)).then_some(
                ParsedGameplayEffect {
                    unique_index,
                    byte_offset: effect_bit_offset / 8,
                    bit_shift: (effect_bit_offset % 8) as u8,
                },
            )
        })
}

fn enrich_hit_with_gameplay_effect(
    hit: &mut Hit,
    effects: &[ParsedGameplayEffect],
    names: &HashMap<u32, String>,
    ability_catalog: &AbilityCatalog,
    previous_hit_bit_offset: Option<usize>,
) {
    let Some(effect) = matching_gameplay_effect(hit, effects, previous_hit_bit_offset) else {
        return;
    };
    apply_gameplay_effect(hit, effect, names, ability_catalog);
}

fn apply_gameplay_effect(
    hit: &mut Hit,
    effect: &ParsedGameplayEffect,
    names: &HashMap<u32, String>,
    ability_catalog: &AbilityCatalog,
) {
    let Some(effect_name) = names.get(&effect.unique_index) else {
        return;
    };
    let skill = ability_catalog.skill(effect_name);
    if !is_damage_gameplay_effect(effect_name, skill) {
        return;
    }
    hit.gameplay_effect_index = Some(effect.unique_index);
    hit.gameplay_effect_name = Some(effect_name.clone());
    if let Some(skill) = skill {
        hit.ability_name = skill.ability_name.clone();
        hit.attack_type = Some(skill.attack_type.clone());
        hit.damage_component = skill.damage_component.clone();
    } else {
        hit.attack_type = Some(classify_attack_type(None, effect_name, None));
    }
    // A GameplayEffect name is only a fallback direction signal. When the
    // damage record already carries a target snapshot, its packet-local
    // direction is stronger evidence: the latest capture contains monster
    // `Hitout` effects on the boss HP stream, which must remain outgoing.
    if is_known_incoming_damage_effect(effect_name)
        && (hit.direction.is_unknown() || hit.target_max_hp <= 0.0)
    {
        hit.direction = HitDirection::Incoming;
    }
    if is_known_outgoing_damage_effect(effect_name, skill) && !hit.direction.is_incoming() {
        hit.direction = HitDirection::Outgoing;
    }
    if is_vehicle_physical_damage_effect(effect_name) {
        hit.direction = HitDirection::Outgoing;
        hit.damage_attribute = Some("物理".to_owned());
        hit.attack_type = Some("载具伤害".to_owned());
    }
}

fn enrich_packet_hits(
    payload: &[u8],
    hits: &mut [Hit],
    effects: &[ParsedGameplayEffect],
    names: &HashMap<u32, String>,
    ability_catalog: &AbilityCatalog,
    evidence: &[(u32, u8, usize)],
    characters: &HashMap<u32, CharacterInfo>,
) {
    let mut previous_hit_bit_offset = None;
    for hit in hits {
        let hit_bit_offset = hit.byte_offset * 8 + usize::from(hit.bit_shift);
        let record_encoding = damage_record_encoding_at(payload, hit.byte_offset, hit.bit_shift);
        let bool_enum_effect = record_encoding.and_then(|encoding| {
            matching_bool_enum_gameplay_effect(payload, hit, encoding, names, ability_catalog)
        });
        if let Some(effect) = bool_enum_effect.as_ref() {
            apply_gameplay_effect(hit, effect, names, ability_catalog);
        } else {
            enrich_hit_with_gameplay_effect(
                hit,
                effects,
                names,
                ability_catalog,
                previous_hit_bit_offset,
            );
        }
        previous_hit_bit_offset = Some(hit_bit_offset);
        if let Some(encoding) = record_encoding {
            reattribute_hit_from_damage_record_owner(hit, evidence, encoding, characters);
        }
        reattribute_hit_from_gameplay_effect_semantics(hit, ability_catalog, characters);
    }
}

fn is_damage_gameplay_effect(effect_name: &str, skill: Option<&GameplayEffectSkill>) -> bool {
    skill.is_some()
        || is_known_incoming_damage_effect(effect_name)
        || is_known_outgoing_damage_effect(effect_name, skill)
        || is_vehicle_physical_damage_effect(effect_name)
}

fn reattribute_hit_from_gameplay_effect_semantics(
    hit: &mut Hit,
    ability_catalog: &AbilityCatalog,
    characters: &HashMap<u32, CharacterInfo>,
) {
    if hit.direction.is_incoming() {
        return;
    }
    let Some(effect_name) = hit.gameplay_effect_name.as_deref() else {
        return;
    };
    let Some(character_id) = ability_catalog
        .skill(effect_name)
        .and_then(|skill| skill.owner_character_id)
    else {
        return;
    };
    if character_id != hit.char_id {
        set_hit_character(hit, character_id, characters);
    }
    hit.char_source = HitCharacterSource::GameplayEffect;
}

fn character_id_from_ability_name(
    ability_name: &str,
    characters: &HashMap<u32, CharacterInfo>,
) -> Option<u32> {
    for token in ability_name.split('_') {
        let bytes = token.as_bytes();
        if bytes.len() < 4 {
            continue;
        }
        let suffix = &bytes[bytes.len() - 3..];
        if !suffix.iter().all(u8::is_ascii_digit) {
            continue;
        }
        let prefix = &bytes[..bytes.len() - 3];
        if prefix.is_empty() || !prefix.iter().any(u8::is_ascii_alphabetic) {
            continue;
        }
        let id = 1000
            + suffix
                .iter()
                .fold(0_u32, |value, digit| value * 10 + (digit - b'0') as u32);
        if characters.contains_key(&id) {
            return Some(id);
        }
    }
    None
}

fn reattribute_hit_from_ability_name(
    hit: &mut Hit,
    can_override_packet_id: bool,
    characters: &HashMap<u32, CharacterInfo>,
) {
    if hit.direction.is_incoming() || hit.attack_type.as_deref() == Some("创生花") {
        return;
    }
    if hit.char_source == HitCharacterSource::Packet && !can_override_packet_id {
        return;
    }
    let Some(ability_name) = hit.ability_name.as_deref() else {
        return;
    };
    let Some(character_id) = character_id_from_ability_name(ability_name, characters) else {
        return;
    };
    if character_id == hit.char_id {
        return;
    }
    set_hit_character(hit, character_id, characters);
    hit.char_source = HitCharacterSource::GameplayEffect;
}

fn is_known_outgoing_damage_effect(effect_name: &str, skill: Option<&GameplayEffectSkill>) -> bool {
    let effect_name_lower = effect_name.to_ascii_lowercase();
    if effect_name_lower.starts_with("ge_mon_") || effect_name_lower.starts_with("ge_boss_") {
        return false;
    }
    if effect_name.starts_with("GE_Player_") && effect_name.contains("_Damage") {
        return true;
    }
    if effect_name.starts_with("GE_ActorReaction_")
        || effect_name.starts_with("GE_Reaction")
        || effect_name.starts_with("Buff_Reaction_")
    {
        return true;
    }
    if effect_name_lower.contains("tenacity") && effect_name_lower.contains("damage") {
        return true;
    }
    let damage_like_effect = effect_name_lower.contains("damage")
        || effect_name_lower.contains("_dmg")
        || effect_name_lower.ends_with("_dmg");
    damage_like_effect
        && skill.is_some_and(|skill| {
            skill
                .ability_name
                .as_deref()
                .is_some_and(|ability_name| ability_name.starts_with("GA_"))
        })
}

fn is_known_incoming_damage_effect(effect_name: &str) -> bool {
    let effect_name_lower = effect_name.to_ascii_lowercase();
    (effect_name_lower.starts_with("ge_mon_") || effect_name_lower.starts_with("ge_boss_"))
        && !effect_name_lower.contains("steal")
        && (effect_name_lower.contains("damage")
            || effect_name_lower.contains("_dmg")
            || effect_name_lower.contains("hitout"))
}

fn is_vehicle_physical_damage_effect(effect_name: &str) -> bool {
    effect_name.starts_with("GE_Vehicle_HitOut")
        || effect_name.starts_with("GE_VehicleCombatDamage")
        || effect_name == "GE_Player_VehicleExplode_HitOut"
}

fn set_hit_character(hit: &mut Hit, new_char_id: u32, characters: &HashMap<u32, CharacterInfo>) {
    let character = characters.get(&new_char_id);
    hit.char_id = new_char_id;
    hit.char_known = character.is_some();
    hit.char_name = character
        .map(|row| {
            if row.name_zh.is_empty() {
                row.name_en.clone()
            } else {
                row.name_zh.clone()
            }
        })
        .unwrap_or_else(|| format!("未知角色({new_char_id})"));
}

/// The two character attributes whose 环合 produces a given reaction *burst*
/// (`Buff_Reaction_*`, classified by [`classify_attack_type`]). Only reactions
/// that are attribute-locked and whose damage packets carry no caster need this;
/// returns `None` for everything else. Mirrors the pairings in `qte_reaction_type`.
fn reaction_owner_attributes(attack_type: &str) -> Option<[&'static str; 2]> {
    match attack_type {
        // 黯星 = 暗 + 魂. See `qte_reaction_type("暗", "魂")`.
        "黯星" => Some(["暗", "魂"]),
        _ => None,
    }
}

/// Re-home a reaction burst that was credited to a character who can't produce it.
///
/// Reactions like `黯星` carry no caster in their damage record, so
/// [`parse_damage_payload`] attributes them to whatever single character the
/// packet happened to declare. When the game bundles such a tick into an
/// unrelated character's replication packet (e.g. a 咒 character who is merely
/// on-field), the credit lands on someone whose attribute can't generate the
/// reaction. Detect that and move it to the most recently declared character
/// whose attribute *can* — i.e. the on-field reaction participant.
///
/// No-op when the reaction isn't attribute-locked, when the current owner is
/// already plausible, or when no recent valid owner is on record.
fn reattribute_orphan_reaction(
    hit: &mut Hit,
    character_declarations: &HashMap<u32, f64>,
    timestamp: f64,
    characters: &HashMap<u32, CharacterInfo>,
) {
    let Some(valid_attributes) = hit
        .attack_type
        .as_deref()
        .and_then(reaction_owner_attributes)
    else {
        return;
    };
    let attribute_of = |character_id: &u32| {
        characters
            .get(character_id)
            .and_then(|character| character.attribute.as_deref())
    };
    if attribute_of(&hit.char_id).is_some_and(|attribute| valid_attributes.contains(&attribute)) {
        return; // already credited to a character that can produce this reaction
    }
    let Some(new_char_id) = character_declarations
        .iter()
        .filter(|(character_id, declared_at)| {
            timestamp - **declared_at <= REACTION_REATTRIBUTION_WINDOW_SECONDS
                && attribute_of(character_id)
                    .is_some_and(|attribute| valid_attributes.contains(&attribute))
        })
        .max_by(|left, right| left.1.total_cmp(right.1))
        .map(|(character_id, _)| *character_id)
    else {
        return; // no on-field 暗/魂 character to credit — leave attribution as-is
    };
    if new_char_id == hit.char_id {
        return;
    }
    set_hit_character(hit, new_char_id, characters);
}

impl PacketDecoder {
    fn finalize_contextual_hit_attribution(
        &mut self,
        hit: &mut Hit,
        timestamp: f64,
        characters: &HashMap<u32, CharacterInfo>,
    ) {
        if hit
            .attack_type
            .as_deref()
            .is_some_and(|attack_type| attack_type.starts_with("环合"))
        {
            let previous_declared_character = self
                .character_declarations
                .iter()
                .filter(|(character_id, declared_at)| {
                    **character_id != hit.char_id && timestamp - **declared_at <= 3.0
                })
                .max_by(|left, right| left.1.total_cmp(right.1))
                .map(|(character_id, _)| *character_id);
            let previous_attribute = previous_declared_character
                .and_then(|character_id| characters.get(&character_id))
                .and_then(|character| character.attribute.as_deref());
            let entering_attribute = characters
                .get(&hit.char_id)
                .and_then(|character| character.attribute.as_deref());
            if let (Some(previous_attribute), Some(entering_attribute)) =
                (previous_attribute, entering_attribute)
                && let Some(reaction_type) =
                    qte_reaction_type(previous_attribute, entering_attribute)
            {
                hit.attack_type = Some(format!("环合·{reaction_type}"));
            }
        }
        reattribute_orphan_reaction(hit, &self.character_declarations, timestamp, characters);
        self.follow_up_damage.observe_fuwen_trigger_hit(hit);
    }

    #[cfg(test)]
    fn process_ethernet_frame(
        &mut self,
        packet: &[u8],
        frame_timestamp: FrameTimestamp,
        local_ip: Option<Ipv4Addr>,
        include_incoming: bool,
        characters: &HashMap<u32, CharacterInfo>,
        sender: &EngineEventSink,
    ) {
        self.process_capture_frame(
            CapturedPacket {
                link_type: CaptureLinkType::Ethernet,
                data: packet,
                timestamp: frame_timestamp,
            },
            local_ip,
            include_incoming,
            characters,
            sender,
        );
    }

    fn process_capture_frame(
        &mut self,
        packet: CapturedPacket<'_>,
        local_ip: Option<Ipv4Addr>,
        include_incoming: bool,
        characters: &HashMap<u32, CharacterInfo>,
        sender: &EngineEventSink,
    ) {
        let (timestamp, capture_timestamp) = match packet.timestamp {
            FrameTimestamp::Known(timestamp) => (timestamp, Some(timestamp)),
            FrameTimestamp::Unknown => (0.0, None),
        };
        let Some((src, src_port, dst, dst_port, payload)) =
            parse_udp_ipv4(packet.link_type, packet.data)
        else {
            return;
        };
        if local_ip.is_some_and(|ip| src != ip && dst != ip) {
            return;
        }
        // Drop a frame already reported by the capture layer so its damage
        // records are not counted a second time. See [`FrameDedup`].
        if self
            .frame_dedup
            .is_duplicate(packet.data, capture_timestamp)
        {
            return;
        }
        let expired_hits = self.take_expired_ambiguous_hits(timestamp);
        self.emit_hits(expired_hits, characters, sender);
        let expired_targetless_hits = self.take_expired_targetless_hits(timestamp);
        self.emit_hits(expired_targetless_hits, characters, sender);

        let decoded_payload = match self.packet_emission {
            PacketEmissionMode::FullDebug => decode_payload_text_filtered(payload, |_| true),
            PacketEmissionMode::SummaryOnly => decode_summary_payload_text(payload),
        };
        let decoded_text = decoded_payload.text;
        let abyss_events = abyss_events_from_text(timestamp, &decoded_text);
        let transport_packet = parse_transport_packet(payload);
        let bunch_packet = match &transport_packet {
            Some(TransportPacket::Sequenced(packet)) => parse_bunch_packet(packet).ok(),
            _ => None,
        };
        let single_bunch = match &transport_packet {
            Some(TransportPacket::Sequenced(packet)) => parse_single_bunch(packet),
            _ => None,
        };
        let fallback_bunch_packet = single_bunch.as_ref().map(|bunch| BunchPacket {
            packet_info_bit_len: 0,
            bunches: vec![crate::engine::protocol::LocatedBunch {
                bit_offset: 0,
                bunch: bunch.clone(),
            }],
        });
        let reassembled_bunches = match &transport_packet {
            Some(TransportPacket::Sequenced(packet)) => self.bunch_connections.observe_sequenced(
                (src, src_port),
                (dst, dst_port),
                timestamp,
                packet,
                bunch_packet.as_ref().or(fallback_bunch_packet.as_ref()),
            ),
            _ => Vec::new(),
        };
        let mut bool_enum_fragment_observation = match single_bunch.as_ref() {
            Some(bunch) => self.bool_enum_gameplay_effect_fragments.observe(
                timestamp,
                (src, src_port),
                (dst, dst_port),
                bunch,
            ),
            None => BoolEnumGameplayEffectFragmentObservation {
                abandoned_hits: self
                    .bool_enum_gameplay_effect_fragments
                    .take_expired(timestamp),
                ..Default::default()
            },
        };
        let combat_payload = single_bunch
            .as_ref()
            .filter(|bunch| bunch.partial_flags == 0x09)
            .map_or(payload, |bunch| bunch.data.as_slice());
        let evidence = find_declared_character_evidence(combat_payload);
        let final_tower_evidence = find_final_tower_character_evidence(combat_payload);
        let character_evidence = merged_character_evidence(&evidence, &final_tower_evidence);
        let ids = character_ids_from_evidence_sources(&evidence, &final_tower_evidence);
        self.follow_up_damage
            .observe_characters(ids.iter().copied(), characters);
        let outgoing = infer_outgoing(
            src,
            src_port,
            dst,
            dst_port,
            local_ip,
            &ids,
            &self.client_endpoints,
        );
        if outgoing && !ids.is_empty() {
            self.client_endpoints.insert((src, src_port));
        }
        let direction = if outgoing { "C2S" } else { "S2C" };
        let gameplay_effects = parse_gameplay_effects(combat_payload);
        let inherited_gameplay_effect = single_bunch.as_ref().and_then(|bunch| {
            self.gameplay_effect_fragments.observe(
                (src, src_port),
                (dst, dst_port),
                bunch,
                &gameplay_effects,
            )
        });
        let mut hits = if outgoing {
            let packet_char_id = if ids.len() == 1 {
                ids.first().copied()
            } else {
                None
            };
            let session_key = (src, src_port, dst, dst_port);
            if let Some(id) = packet_char_id {
                self.session_characters.insert(session_key, id);
            }
            let fallback = self.session_characters.get(&session_key).copied();
            parse_damage_payload(
                combat_payload,
                timestamp,
                packet_char_id,
                fallback,
                characters,
                &evidence,
            )
        } else {
            Vec::new()
        };
        let effective_gameplay_effects = inherited_gameplay_effect
            .as_ref()
            .map_or(gameplay_effects.as_slice(), std::slice::from_ref);
        // SDK/CN exposes the server damage application through
        // FHandleDamageInfo_Net.GameplayEffect and FPlayGamePlayEffect_Net.GameplayEffect.
        // Prefer that exact GE identity over any damage-ratio inference.
        let packet_has_fuwen_damage_effect = effective_gameplay_effects.iter().any(|effect| {
            self.gameplay_effect_names
                .get(&effect.unique_index)
                .is_some_and(|name| name == FUWEN_DAMAGE_GAMEPLAY_EFFECT_NAME)
        });
        enrich_packet_hits(
            combat_payload,
            &mut hits,
            effective_gameplay_effects,
            &self.gameplay_effect_names,
            &self.ability_catalog,
            &evidence,
            characters,
        );
        for hit in &mut hits {
            reattribute_hit_from_ability_name(hit, !final_tower_evidence.is_empty(), characters);
        }
        if outgoing {
            for hit in &mut hits {
                if hit.direction.is_unknown() && hit.target_id.is_some() {
                    hit.direction = HitDirection::Outgoing;
                }
            }
        }
        let starts_partial_stream = single_bunch
            .as_ref()
            .is_some_and(|bunch| bunch.partial_flags == 0x09)
            || bunch_packet.as_ref().is_some_and(|packet| {
                packet
                    .bunches
                    .iter()
                    .any(|located| located.bunch.partial_flags == 0x09)
            });
        if outgoing && starts_partial_stream {
            let mut retained = Vec::with_capacity(hits.len());
            for hit in hits {
                if hit.target_id.is_none()
                    && self.pending_ambiguous_hits.len() < MAX_PENDING_FOLLOW_UP_HITS
                {
                    self.pending_ambiguous_hits.push(hit);
                } else {
                    retained.push(hit);
                }
            }
            hits = retained;
        }
        if outgoing {
            for reassembled in reassembled_bunches
                .iter()
                .filter(|observation| observation.bunch.fragment_count > 1)
            {
                let reassembled_payload = &reassembled.bunch;
                let reassembled_evidence =
                    find_declared_character_evidence(&reassembled_payload.data);
                let reassembled_final_tower =
                    find_final_tower_character_evidence(&reassembled_payload.data);
                let reassembled_ids = character_ids_from_evidence_sources(
                    &reassembled_evidence,
                    &reassembled_final_tower,
                );
                let reassembled_packet_char_id = if reassembled_ids.len() == 1 {
                    reassembled_ids.first().copied()
                } else {
                    None
                };
                let session_key = (src, src_port, dst, dst_port);
                let fallback = self.session_characters.get(&session_key).copied();
                let mut confirmed_hits = parse_damage_payload(
                    &reassembled_payload.data,
                    timestamp,
                    reassembled_packet_char_id,
                    fallback,
                    characters,
                    &reassembled_evidence,
                );
                let reassembled_effects = parse_gameplay_effects(&reassembled_payload.data);
                enrich_packet_hits(
                    &reassembled_payload.data,
                    &mut confirmed_hits,
                    &reassembled_effects,
                    &self.gameplay_effect_names,
                    &self.ability_catalog,
                    &reassembled_evidence,
                    characters,
                );
                for hit in &mut confirmed_hits {
                    reattribute_hit_from_ability_name(
                        hit,
                        !reassembled_final_tower.is_empty(),
                        characters,
                    );
                }
                let mut resolved = self.resolve_pending_untyped_skills(&confirmed_hits);
                let recovered = self.take_new_reassembled_hits(
                    confirmed_hits,
                    &resolved,
                    reassembled.started_at,
                );
                resolved.extend(recovered);
                extend_unique_exact_wire_hits(&mut hits, resolved);
            }
        }

        if let Some(bunch) = single_bunch
            .as_ref()
            .filter(|bunch| bunch.partial_flags == 0x09)
            && hits.last().is_some_and(|hit| {
                hit.gameplay_effect_name.is_none()
                    && damage_record_encoding_at(combat_payload, hit.byte_offset, hit.bit_shift)
                        == Some(DamageRecordEncoding::BoolAndEnums)
                    && hit.byte_offset * 8
                        + usize::from(hit.bit_shift)
                        + BOOL_ENUM_DAMAGE_RECORD_TO_GAMEPLAY_EFFECT_BITS[0]
                        + 32
                        > bunch.data_bit_len
            })
            && let Some(pending_hit) = hits.pop()
            && let Some(hit) = self.bool_enum_gameplay_effect_fragments.attach_hit(
                (src, src_port),
                (dst, dst_port),
                bunch,
                pending_hit,
            )
        {
            hits.push(hit);
        }

        if let Some(mut completed) = bool_enum_fragment_observation.completed.take() {
            let encoding = damage_record_encoding_at(
                &completed.payload,
                completed.hit.byte_offset,
                completed.hit.bit_shift,
            );
            let effect = encoding.and_then(|encoding| {
                matching_bool_enum_gameplay_effect(
                    &completed.payload,
                    &completed.hit,
                    encoding,
                    &self.gameplay_effect_names,
                    &self.ability_catalog,
                )
            });
            if let Some(effect) = effect.as_ref() {
                apply_gameplay_effect(
                    &mut completed.hit,
                    effect,
                    &self.gameplay_effect_names,
                    &self.ability_catalog,
                );
                reattribute_hit_from_gameplay_effect_semantics(
                    &mut completed.hit,
                    &self.ability_catalog,
                    characters,
                );
                reattribute_hit_from_ability_name(&mut completed.hit, false, characters);
            }
            hits.push(completed.hit);
        }
        hits.append(&mut bool_enum_fragment_observation.abandoned_hits);
        propagate_unanimous_trailing_skill(&mut hits);
        self.resolve_and_observe_hit_targets(&mut hits);
        let has_direct_fuwen_damage_hit = hits.iter().any(|hit| {
            hit.gameplay_effect_name.as_deref() == Some(FUWEN_DAMAGE_GAMEPLAY_EFFECT_NAME)
        });
        if has_direct_fuwen_damage_hit {
            // A decoded damage record already carries the authoritative GE and
            // will be counted directly as 覆纹; do not infer the same damage
            // again from the following boss-HP synchronization.
            self.follow_up_damage.observe_direct_fuwen_damage_hit();
        } else if packet_has_fuwen_damage_effect {
            self.follow_up_damage.observe_fuwen_damage_effect(timestamp);
        }
        for hit in &mut hits {
            let hit_timestamp = hit.timestamp;
            self.finalize_contextual_hit_attribution(hit, hit_timestamp, characters);
        }
        let prepared_hits =
            self.prepare_hits_for_emission(hits, &ids, include_incoming, characters);
        for character_id in &ids {
            self.character_declarations.insert(*character_id, timestamp);
        }
        self.character_declarations
            .retain(|_, declared_at| timestamp - *declared_at <= 10.0);
        let mut accepted = prepared_hits.emit.len();
        // CurrentHP 候选缺少目标 handle 校验，仅用于调试显示，不参与 follow-up 计算。
        let current_hp_updates = if outgoing {
            Vec::new()
        } else {
            parse_current_hp_updates(payload)
        };
        let direct_client_damage_boss = if outgoing {
            None
        } else {
            match (&transport_packet, &bunch_packet) {
                (Some(TransportPacket::Sequenced(packet)), Some(bunches)) => {
                    parse_client_damage_boss_update(packet, bunches)
                }
                _ => None,
            }
        };
        let direct_boss_target = direct_client_damage_boss
            .as_ref()
            .map(|update| update.target_handle);
        let used_direct_client_damage_boss = direct_boss_target.is_some();
        let mut target_hp_updates = if outgoing {
            Vec::new()
        } else {
            parse_client_fight_target_updates(payload)
        };
        if !outgoing {
            let mut seen = target_hp_updates
                .iter()
                .map(|update| (update.target_handle, update.current_hp.to_bits()))
                .collect::<HashSet<_>>();
            for observation in &reassembled_bunches {
                for update in parse_client_fight_target_updates(&observation.bunch.data) {
                    if seen.insert((update.target_handle, update.current_hp.to_bits())) {
                        target_hp_updates.push(update);
                    }
                }
            }
        }
        let mut server_damage_settlements = if outgoing {
            Vec::new()
        } else {
            parse_server_damage_settlements(payload)
        };
        if !outgoing {
            let settlement_key = |settlement: &ParsedServerDamageSettlement| {
                (
                    settlement.target_handle,
                    settlement.current_hp.to_bits(),
                    settlement.dead_state,
                    settlement.raw_damage,
                    settlement.secondary_value,
                )
            };
            let mut packet_occurrences = HashMap::new();
            for settlement in &server_damage_settlements {
                *packet_occurrences
                    .entry(settlement_key(settlement))
                    .or_insert(0_usize) += 1;
            }
            for observation in &reassembled_bunches {
                for settlement in parse_server_damage_settlements(&observation.bunch.data) {
                    let key = settlement_key(&settlement);
                    if packet_occurrences.get_mut(&key).is_some_and(|remaining| {
                        if *remaining == 0 {
                            false
                        } else {
                            *remaining -= 1;
                            true
                        }
                    }) {
                        continue;
                    }
                    server_damage_settlements.push(settlement);
                }
            }
        }
        let target_hp_keys = target_hp_updates
            .iter()
            .map(|update| (update.target_handle, update.current_hp.to_bits()))
            .collect::<HashSet<_>>();
        let mut boss_hp_updates = match direct_client_damage_boss {
            Some(update) => vec![update],
            None if !outgoing => parse_boss_hp_updates(payload)
                .into_iter()
                .filter(|update| {
                    !target_hp_keys.contains(&(update.target_handle, update.current_hp.to_bits()))
                })
                .collect(),
            None => Vec::new(),
        };
        if !outgoing && direct_boss_target.is_none() {
            let mut seen = boss_hp_updates
                .iter()
                .map(|update| (update.target_handle, update.current_hp.to_bits()))
                .collect::<HashSet<_>>();
            for observation in &reassembled_bunches {
                let generic = parse_client_fight_target_updates(&observation.bunch.data)
                    .into_iter()
                    .map(|update| (update.target_handle, update.current_hp.to_bits()))
                    .collect::<HashSet<_>>();
                for update in parse_boss_hp_updates(&observation.bunch.data) {
                    let key = (update.target_handle, update.current_hp.to_bits());
                    if !generic.contains(&key) && seen.insert(key) {
                        boss_hp_updates.push(update);
                    }
                }
            }
        }
        for update in &target_hp_updates {
            self.observe_target_hp_update(timestamp, update);
        }
        for update in &boss_hp_updates {
            self.observe_target_hp_update(timestamp, update);
        }
        let mut target_evidence = target_hp_updates.clone();
        let mut target_evidence_keys = target_evidence
            .iter()
            .map(|update| (update.target_handle, update.current_hp.to_bits()))
            .collect::<HashSet<_>>();
        for update in &boss_hp_updates {
            if target_evidence_keys.insert((update.target_handle, update.current_hp.to_bits())) {
                target_evidence.push(update.clone());
            }
        }
        let resolved_target_hits = self.resolve_pending_hit_targets(timestamp, &target_evidence);
        accepted += resolved_target_hits.len();
        let inventory_result = if !outgoing {
            match &transport_packet {
                Some(TransportPacket::Sequenced(packet)) => self.empty_curtain.process_packet(
                    InventoryConnectionKey::new(
                        format!("{src}:{src_port}"),
                        format!("{dst}:{dst_port}"),
                    ),
                    packet,
                ),
                _ => InventoryPacketResult::default(),
            }
        } else {
            InventoryPacketResult::default()
        };
        if let Some(characters) = inventory_result.characters {
            let _ = sender.send(EngineEvent::EmptyCurtainCharacters(characters));
        }
        if let Some(snapshot) = inventory_result.snapshot {
            let _ = sender.send(EngineEvent::EmptyCurtain(snapshot));
        }
        let mut equipment_slots = Vec::new();
        if !outgoing {
            append_unique_equipment_slots(&mut equipment_slots, parse_equipment_slots(payload));
            if let Some(bunch) = &single_bunch {
                append_unique_equipment_slots(
                    &mut equipment_slots,
                    parse_equipment_slots(&bunch.data),
                );
            }
        }
        let fuwen_start = if !outgoing
            && gameplay_effects.is_empty()
            && current_hp_updates.is_empty()
            && target_hp_updates.is_empty()
            && boss_hp_updates.is_empty()
            && server_damage_settlements.is_empty()
            && equipment_slots.is_empty()
        {
            fuwen_start_pair(payload, &evidence, characters)
        } else {
            None
        };
        if let Some((entering_character_id, previous_character_id)) = fuwen_start {
            self.follow_up_damage.observe_fuwen_start_candidate(
                timestamp,
                entering_character_id,
                previous_character_id,
                characters,
            );
        }
        if current_hp_updates.is_empty()
            && target_hp_updates.is_empty()
            && boss_hp_updates.is_empty()
            && server_damage_settlements.is_empty()
            && prepared_hits.deferred_ambiguous == 0
            && prepared_hits.deferred_targetless == 0
            && fuwen_start.is_none()
            && equipment_slots.is_empty()
            && !should_keep_debug_packet(
                payload,
                &ids,
                accepted,
                equipment_slots.len(),
                inventory_result.recognized,
                decoded_payload.has_readable_text,
            )
        {
            return;
        }
        if matches!(self.packet_emission, PacketEmissionMode::SummaryOnly) {
            let (
                mut server_damage_corrections,
                mut unattributed_server_damage,
                server_residual_hits,
            ) = self.reconcile_current_packet_server_damage_settlements(
                timestamp,
                &server_damage_settlements,
                resolved_target_hits.iter().chain(prepared_hits.emit.iter()),
            );
            let (
                inferred_follow_ups,
                hp_sync_follow_ups,
                legacy_server_damage_corrections,
                legacy_unattributed_server_damage,
            ) = self.reconcile_boss_hp_updates(timestamp, &boss_hp_updates);
            server_damage_corrections.extend(legacy_server_damage_corrections);
            unattributed_server_damage.extend(legacy_unattributed_server_damage);
            let _ = sender.send(EngineEvent::PacketObservation(PacketObservation {
                parsed_hits: accepted,
            }));
            self.emit_preobserved_hits(resolved_target_hits, characters, sender);
            self.emit_preobserved_hits(prepared_hits.emit, characters, sender);
            for event in abyss_events {
                let _ = sender.send(EngineEvent::Abyss(event));
            }
            for follow_up in inferred_follow_ups {
                let _ = sender.send(EngineEvent::HitFollowUp(follow_up));
            }
            for follow_up in hp_sync_follow_ups {
                let _ = sender.send(EngineEvent::HitFollowUp(follow_up));
            }
            for correction in server_damage_corrections {
                let _ = sender.send(EngineEvent::HitDamageCorrection(correction));
            }
            for observation in unattributed_server_damage {
                let _ = sender.send(EngineEvent::UnattributedServerDamage(observation));
            }
            self.emit_hits(server_residual_hits, characters, sender);
            return;
        }
        let mut note = String::new();
        if prepared_hits.filtered_incoming > 0 {
            append_packet_note(
                &mut note,
                Some(format!(
                    "过滤 {} 条 incoming 记录",
                    prepared_hits.filtered_incoming
                )),
            );
        }
        if prepared_hits.deferred_ambiguous > 0 {
            append_packet_note(
                &mut note,
                Some(format!(
                    "暂存 {} 条多角色候选伤害等待确认",
                    prepared_hits.deferred_ambiguous
                )),
            );
        }
        if prepared_hits.deferred_targetless > 0 {
            append_packet_note(
                &mut note,
                Some(format!(
                    "暂存 {} 条缺少目标的输出等待服务器目标回执",
                    prepared_hits.deferred_targetless
                )),
            );
        }
        if prepared_hits.suppressed_ambiguous > 0 {
            append_packet_note(
                &mut note,
                Some(format!(
                    "丢弃 {} 条已确认重复候选伤害",
                    prepared_hits.suppressed_ambiguous
                )),
            );
        }
        if let Some((entering_character_id, previous_character_id)) = fuwen_start {
            append_packet_note(
                &mut note,
                Some(format!(
                    "覆纹启动：{} + {}",
                    character_debug_label(entering_character_id, characters),
                    character_debug_label(previous_character_id, characters)
                )),
            );
        }
        append_packet_note(
            &mut note,
            binary_payload_diagnostic(payload, direction, &decoded_text, &character_evidence),
        );
        if !gameplay_effects.is_empty() {
            append_packet_note(
                &mut note,
                Some(format!(
                    "GameplayEffect: {}",
                    gameplay_effects
                        .iter()
                        .map(|effect| {
                            let location = format!("@{}:{}", effect.byte_offset, effect.bit_shift);
                            self.gameplay_effect_names
                                .get(&effect.unique_index)
                                .map_or_else(
                                    || format!("{}{}", effect.unique_index, location),
                                    |name| format!("{} {}{}", effect.unique_index, name, location),
                                )
                        })
                        .collect::<Vec<_>>()
                        .join(", ")
                )),
            );
        }
        if !current_hp_updates.is_empty() {
            append_packet_note(
                &mut note,
                Some(format!(
                    "CurrentHP 更新候选：{}",
                    current_hp_updates
                        .iter()
                        .map(|update| format!(
                            "{:.0}@{}:{}",
                            update.current_hp, update.byte_offset, update.bit_shift
                        ))
                        .collect::<Vec<_>>()
                        .join(", ")
                )),
            );
        }
        if !target_hp_updates.is_empty() {
            append_packet_note(
                &mut note,
                Some(format!(
                    "ClientFight target updates: {}",
                    target_hp_updates
                        .iter()
                        .map(|update| format!(
                            "{}={:.0}@{}:{}",
                            hex::encode(update.target_handle),
                            update.current_hp,
                            update.byte_offset,
                            update.bit_shift
                        ))
                        .collect::<Vec<_>>()
                        .join(", ")
                )),
            );
        }
        if !boss_hp_updates.is_empty() {
            append_packet_note(
                &mut note,
                Some(format!(
                    "Boss HP updates: {}",
                    boss_hp_updates
                        .iter()
                        .map(|update| format!(
                            "{}={:.0}@{}:{}",
                            hex::encode(update.target_handle),
                            update.current_hp,
                            update.byte_offset,
                            update.bit_shift
                        ))
                        .collect::<Vec<_>>()
                        .join(", ")
                )),
            );
        }
        if !server_damage_settlements.is_empty() {
            append_packet_note(
                &mut note,
                Some(format!(
                    "Server damage settlements: {} rows, raw={}",
                    server_damage_settlements.len(),
                    server_damage_settlements
                        .iter()
                        .map(|settlement| u64::from(settlement.raw_damage))
                        .sum::<u64>()
                )),
            );
        }
        if used_direct_client_damage_boss {
            append_packet_note(
                &mut note,
                Some(
                    "ValidatedCombatWire v1: ClientDamageBoss remaining HP @ packet bit 594"
                        .to_owned(),
                ),
            );
        }
        append_packet_note(&mut note, equipment_slots_note(&equipment_slots));
        let (mut server_damage_corrections, mut unattributed_server_damage, server_residual_hits) =
            self.reconcile_current_packet_server_damage_settlements(
                timestamp,
                &server_damage_settlements,
                resolved_target_hits.iter().chain(prepared_hits.emit.iter()),
            );
        let (
            inferred_follow_ups,
            hp_sync_follow_ups,
            legacy_server_damage_corrections,
            legacy_unattributed_server_damage,
        ) = self.reconcile_boss_hp_updates(timestamp, &boss_hp_updates);
        server_damage_corrections.extend(legacy_server_damage_corrections);
        unattributed_server_damage.extend(legacy_unattributed_server_damage);
        if let Some(TransportPacket::Sequenced(packet)) = &transport_packet {
            if packet.mode != 0 {
                append_packet_note(
                    &mut note,
                    Some(format!(
                        "传输模式 {}，PacketId {}，Ack {}，应用载荷 {} bit",
                        packet.mode,
                        packet.packet_id,
                        packet.acknowledged_packet_id,
                        packet.payload_bit_len
                    )),
                );
            } else if let Some(bunch) = &single_bunch {
                append_packet_note(
                    &mut note,
                    Some(format!(
                        "SingleBunch channel {}，seq {}，descriptor 0x{:02x}，partial 0x{:x}，数据 {} bit",
                        reliable_bunch_channel(bunch.prefix),
                        bunch.sequence,
                        bunch.descriptor,
                        bunch.partial_flags,
                        bunch.data_bit_len
                    )),
                );
            }
            if let Some(bunches) = &bunch_packet {
                append_packet_note(
                    &mut note,
                    Some(bunch_wire_shape_note(
                        direction,
                        bunches,
                        reassembled_bunches.len(),
                    )),
                );
            }
        }
        let _ = send_packet_debug_events(
            sender,
            PacketDebug {
                timestamp,
                source: format!("{src}:{src_port}"),
                destination: format!("{dst}:{dst_port}"),
                direction: direction.to_owned(),
                payload_len: payload.len(),
                declared_ids: ids,
                parsed_hits: accepted,
                note,
                payload_preview: {
                    let preview_len = payload.len().min(96);
                    hex::encode(&payload[..preview_len])
                },
                payload_hex: hex::encode(payload),
                decoded_text,
            },
        );
        self.emit_preobserved_hits(resolved_target_hits, characters, sender);
        self.emit_preobserved_hits(prepared_hits.emit, characters, sender);
        for event in abyss_events {
            let _ = sender.send(EngineEvent::Abyss(event));
        }
        for follow_up in inferred_follow_ups {
            let _ = sender.send(EngineEvent::HitFollowUp(follow_up));
        }
        for follow_up in hp_sync_follow_ups {
            let _ = sender.send(EngineEvent::HitFollowUp(follow_up));
        }
        for correction in server_damage_corrections {
            let _ = sender.send(EngineEvent::HitDamageCorrection(correction));
        }
        for observation in unattributed_server_damage {
            let _ = sender.send(EngineEvent::UnattributedServerDamage(observation));
        }
        self.emit_hits(server_residual_hits, characters, sender);
    }
}

pub fn start_capture(
    device: CaptureDevice,
    local_ip: Option<Ipv4Addr>,
    filter: String,
    include_incoming: bool,
    use_server_damage_calibration: bool,
    resources: CaptureResources,
    output: CaptureOutput,
) -> CaptureHandle {
    let CaptureOutput {
        raw_capture_directory,
        packet_emission,
        sender,
    } = output;
    let stop = Arc::new(AtomicBool::new(false));
    let thread_stop = stop.clone();
    let raw_capture = RawCaptureBuffer::new(raw_capture_directory.as_deref());
    let thread_raw_capture = raw_capture.clone();
    let thread = thread::spawn(move || {
        let result = run_capture(CaptureRunConfig {
            device: &device,
            local_ip,
            filter: &filter,
            include_incoming,
            use_server_damage_calibration,
            resources,
            sender: &sender,
            stop: &thread_stop,
            raw_capture: &thread_raw_capture,
            packet_emission,
        });
        thread_raw_capture.finish();
        if let Err(error) = result {
            let _ = sender.send(EngineEvent::Error(error));
        }
        // Lifecycle completion is the final reliable event. Consumers may
        // safely join the producer after observing it without blocking on a
        // later reliable send into the bounded queue.
        let _ = sender.send(EngineEvent::CaptureStopped);
    });
    CaptureHandle {
        stop,
        thread: Some(thread),
        raw_capture,
    }
}

struct CaptureRunConfig<'a> {
    device: &'a CaptureDevice,
    local_ip: Option<Ipv4Addr>,
    filter: &'a str,
    include_incoming: bool,
    use_server_damage_calibration: bool,
    resources: CaptureResources,
    sender: &'a EngineEventSink,
    stop: &'a AtomicBool,
    raw_capture: &'a RawCaptureBuffer,
    packet_emission: PacketEmissionMode,
}

struct ParserRunConfig {
    link_type: CaptureLinkType,
    local_ip: Option<Ipv4Addr>,
    include_incoming: bool,
    use_server_damage_calibration: bool,
    packet_emission: PacketEmissionMode,
    resources: CaptureResources,
    sender: EngineEventSink,
}

/// Parser thread body: drains decoded frames off the bounded queue and runs the stable decode
/// pipeline, fully decoupled from packet acquisition. It owns its own `PacketDecoder` and exits
/// once the acquisition thread drops the frame sender, flushing any deferred ambiguous hits.
fn run_parser(frames: CaptureFrameReceiver, config: ParserRunConfig) {
    let ParserRunConfig {
        link_type,
        local_ip,
        include_incoming,
        use_server_damage_calibration,
        packet_emission,
        resources,
        sender,
    } = config;
    let CaptureResources {
        characters,
        ability_catalog,
    } = resources;
    let mut decoder =
        PacketDecoder::with_ability_catalog(ability_catalog, use_server_damage_calibration);
    decoder.packet_emission = packet_emission;
    if let Some(warning) = decoder.resource_warning() {
        let _ = sender.send(EngineEvent::Warning(warning));
    }
    while let Ok(frame) = frames.recv() {
        decoder.process_capture_frame(
            CapturedPacket {
                link_type,
                data: &frame.data,
                timestamp: FrameTimestamp::Known(frame.timestamp),
            },
            local_ip,
            include_incoming,
            &characters,
            &sender,
        );
    }
    let pending_hits = decoder.take_all_ambiguous_hits();
    decoder.emit_hits(pending_hits, &characters, &sender);
    let pending_targetless_hits = decoder.take_all_targetless_hits();
    decoder.emit_hits(pending_targetless_hits, &characters, &sender);
}

fn forward_capture_frame(sender: &CaptureFrameSender, frame: CaptureFrame) -> Result<(), String> {
    sender.send(frame)
}

fn run_capture(config: CaptureRunConfig<'_>) -> Result<(), String> {
    let CaptureRunConfig {
        device,
        local_ip,
        filter,
        include_incoming,
        use_server_damage_calibration,
        resources,
        sender,
        stop,
        raw_capture,
        packet_emission,
    } = config;
    let libraries = NpcapLibraries::load()?;
    let library = &libraries.wpcap;
    // SAFETY: This exact export has the documented OpenLive signature.
    let open_live: OpenLive = unsafe { load_symbol(library, b"pcap_open_live\0")? };
    // SAFETY: This exact export has the documented NextEx signature.
    let next_ex: NextEx = unsafe { load_symbol(library, b"pcap_next_ex\0")? };
    // SAFETY: This exact export has the documented Close signature.
    let close: Close = unsafe { load_symbol(library, b"pcap_close\0")? };
    // SAFETY: This exact export has the documented Compile signature.
    let compile: Compile = unsafe { load_symbol(library, b"pcap_compile\0")? };
    // SAFETY: This exact export has the documented SetFilter signature.
    let set_filter: SetFilter = unsafe { load_symbol(library, b"pcap_setfilter\0")? };
    // SAFETY: This exact export has the documented FreeCode signature.
    let free_code: FreeCode = unsafe { load_symbol(library, b"pcap_freecode\0")? };
    // SAFETY: This exact export has the documented GetErr signature.
    let get_err: GetErr = unsafe { load_symbol(library, b"pcap_geterr\0")? };
    // SAFETY: This exact export has the documented PcapDataLink signature.
    let pcap_datalink: PcapDataLink = unsafe { load_symbol(library, b"pcap_datalink\0")? };

    let device_name = CString::new(device.name.as_str()).map_err(|error| error.to_string())?;
    let mut error_buffer = [0_i8; PCAP_ERRBUF_SIZE];
    // SAFETY: All pointers reference live writable/readable buffers for the
    // duration of this documented pcap_open_live call.
    let raw_handle = unsafe {
        open_live(
            device_name.as_ptr(),
            CAPTURE_SNAPLEN as c_int,
            1,
            100,
            error_buffer.as_mut_ptr(),
        )
    };
    let Some(raw_handle) = NonNull::new(raw_handle) else {
        // SAFETY: Npcap writes a NUL-terminated PCAP_ERRBUF_SIZE error buffer
        // when pcap_open_live reports failure.
        let error = unsafe { c_string(error_buffer.as_ptr()) };
        return Err(format!("failed to open device: {error}"));
    };
    // SAFETY: The non-null pointer was just returned by this library's
    // pcap_open_live and this guard becomes its sole close owner.
    let handle = unsafe { PcapHandle::from_raw(raw_handle, close, library) };
    // SAFETY: `handle` is live and owned, and the typed symbol's DLL is retained.
    let data_link = unsafe { pcap_datalink(handle.as_ptr()) };
    let link_type = CaptureLinkType::from_npcap_datalink(data_link)?;
    raw_capture.initialize(device, link_type);

    let capture_filter = CString::new(filter).map_err(|error| error.to_string())?;
    let mut program = BpfProgramGuard::new(free_code, library);
    // SAFETY: The handle and filter C string are live; `program` exposes its
    // exact writable BpfProgram storage for initialization by pcap_compile.
    let compile_result = unsafe {
        compile(
            handle.as_ptr(),
            program.as_mut_ptr(),
            capture_filter.as_ptr(),
            1,
            u32::MAX,
        )
    };
    if compile_result != 0 {
        // SAFETY: The handle remains live and pcap_geterr returns a borrowed
        // string valid until the next operation on this handle.
        let error_ptr = unsafe { get_err(handle.as_ptr()) };
        // SAFETY: The returned Npcap error pointer is NUL-terminated and is
        // consumed before another operation can invalidate it.
        let error = unsafe { c_string(error_ptr) };
        return Err(format!("failed to set capture filter: {error}"));
    }
    // SAFETY: A zero compile result initialized this exact program, making it
    // eligible for pcap_freecode on every subsequent exit path.
    unsafe { program.mark_compiled() };
    // SAFETY: The handle is live and `program` was successfully compiled by the
    // same loaded Npcap library.
    let set_filter_result = unsafe { set_filter(handle.as_ptr(), program.as_mut_ptr()) };
    if set_filter_result != 0 {
        // SAFETY: The handle remains live and pcap_geterr returns a borrowed
        // string valid until the next operation on this handle.
        let error_ptr = unsafe { get_err(handle.as_ptr()) };
        // SAFETY: The returned Npcap error pointer is NUL-terminated and is
        // consumed before another operation can invalidate it.
        let error = unsafe { c_string(error_ptr) };
        return Err(format!("failed to set capture filter: {error}"));
    }
    program.release();
    let raw_capture_status = raw_capture.path().map_or_else(
        || "; raw capture unavailable".to_owned(),
        |path| format!("; writing raw capture to {}", path.display()),
    );
    let _ = sender.send(EngineEvent::Status(format!(
        "capturing: {} ({}, {}){}",
        device.description,
        local_ip
            .map(|ip| ip.to_string())
            .unwrap_or_else(|| "local IP not filtered".to_owned()),
        link_type.label(),
        raw_capture_status
    )));

    // The plugin monitor starts only after the capture link type has been frozen and the raw
    // writer has emitted its matching interface block, so no custom block can precede it.
    let monitor_stop = Arc::new(AtomicBool::new(false));
    let monitor_thread = {
        let stop = Arc::clone(&monitor_stop);
        let raw_capture = raw_capture.clone();
        let sender = sender.clone();
        let capture_started_100ns = current_filetime_100ns();
        thread::spawn(move || {
            run_plugin_monitor(&stop, capture_started_100ns, &raw_capture, &sender);
        })
    };

    // Decode on a dedicated thread. Acquisition writes every raw frame before forwarding it to
    // the FIFO parser queue. Both the frame count and payload-byte high-water are reliable
    // backpressure bounds: full blocks the acquisition producer, no frame is dropped, and a
    // disconnected parser fails the capture session.
    let (frame_sender, frame_receiver) = capture_frame_queue(
        CAPTURE_FRAME_QUEUE_CAPACITY,
        CAPTURE_FRAME_QUEUE_BYTE_HIGH_WATER,
    );
    let parser_thread = {
        let resources = resources.clone();
        let sender = sender.clone();
        thread::spawn(move || {
            run_parser(
                frame_receiver,
                ParserRunConfig {
                    link_type,
                    local_ip,
                    include_incoming,
                    use_server_damage_calibration,
                    packet_emission,
                    resources,
                    sender,
                },
            );
        })
    };

    let mut loop_result = Ok(());
    while !stop.load(Ordering::Relaxed) {
        let mut header = ptr::null();
        let mut packet_data = ptr::null();
        // SAFETY: The live handle and output-pointer storage meet pcap_next_ex's
        // contract; returned packet pointers are consumed before the next call.
        let result = unsafe { next_ex(handle.as_ptr(), &mut header, &mut packet_data) };
        if result == 0 {
            continue;
        }
        if result < 0 {
            // SAFETY: The handle remains live and pcap_geterr returns a borrowed
            // string valid until the next operation on this handle.
            let error_ptr = unsafe { get_err(handle.as_ptr()) };
            // SAFETY: The returned pointer is NUL-terminated and copied now.
            let error = unsafe { c_string(error_ptr) };
            loop_result = Err(format!("failed to read packet: {error}"));
            break;
        }
        if header.is_null() || packet_data.is_null() {
            continue;
        }
        // SAFETY: A positive pcap_next_ex result guarantees a readable header
        // valid until the next call; null was rejected above.
        let header_ref = unsafe { &*header };
        if header_ref.caplen == 0 {
            continue;
        }
        if header_ref.caplen > CAPTURE_SNAPLEN {
            loop_result = Err(format!(
                "Npcap frame length {} exceeds configured snaplen {CAPTURE_SNAPLEN}",
                header_ref.caplen
            ));
            break;
        }
        // SAFETY: A positive pcap_next_ex result guarantees at least `caplen`
        // readable bytes until the next call, and caplen is snaplen-bounded.
        let packet = unsafe { std::slice::from_raw_parts(packet_data, header_ref.caplen as usize) };
        let timestamp = header_ref.ts.tv_sec as f64 + header_ref.ts.tv_usec as f64 / 1_000_000.0;
        let raw_timestamp = Duration::new(
            header_ref.ts.tv_sec.max(0) as u64,
            header_ref.ts.tv_usec.clamp(0, 999_999) as u32 * 1_000,
        );
        raw_capture.push(raw_timestamp, header_ref.len, packet);
        if let Err(error) =
            forward_capture_frame(&frame_sender, CaptureFrame::new(packet.to_vec(), timestamp))
        {
            loop_result = Err(error);
            break;
        }
    }
    let frame_queue_high_water = frame_sender.byte_high_water_mark();
    drop(frame_sender);
    monitor_stop.store(true, Ordering::Relaxed);
    let monitor_panicked = monitor_thread.join().is_err();
    if parser_thread.join().is_err() && loop_result.is_ok() {
        loop_result = Err("capture parser thread stopped unexpectedly".to_owned());
    }
    if monitor_panicked && loop_result.is_ok() {
        loop_result = Err("capture plugin monitor stopped unexpectedly".to_owned());
    }
    if frame_queue_high_water > CAPTURE_FRAME_QUEUE_BYTE_HIGH_WATER && loop_result.is_ok() {
        loop_result = Err("capture parser queue exceeded its byte budget".to_owned());
    }
    loop_result?;
    Ok(())
}

pub fn import_pcapng(
    path: PathBuf,
    resources: CaptureResources,
    local_ip_hint: Option<Ipv4Addr>,
    include_incoming: bool,
    use_server_damage_calibration: bool,
    sender: impl Into<EngineEventSink>,
    stop: Arc<AtomicBool>,
) -> std::io::Result<thread::JoinHandle<()>> {
    // Reject an invalid path/oversized file synchronously, before creating a
    // replay owner or allocating parser state. The worker repeats this check
    // through the opened file to fail closed if the path changes meanwhile.
    validate_pcapng_import(&path).map_err(std::io::Error::other)?;
    let sender = sender.into();
    thread::Builder::new()
        .name("nte-pcapng-replay".to_owned())
        .spawn(move || {
        let CaptureResources {
            characters,
            ability_catalog,
        } = resources;
        let direction_mode = local_ip_hint.map_or_else(
            || "heuristic direction".to_owned(),
            |ip| format!("local IP {ip}"),
        );
        let _ = sender.send(EngineEvent::Status(format!(
            "importing pcapng: {direction_mode}"
        )));
        let result = (|| -> Result<(usize, usize), PcapngImportError> {
            let file = File::open(&path).map_err(PcapngImportError::Io)?;
            let metadata = file.metadata().map_err(PcapngImportError::Io)?;
            if !metadata.is_file() {
                return Err(PcapngImportError::NotAFile);
            }
            if metadata.len() > MAX_PCAPNG_IMPORT_BYTES {
                return Err(PcapngImportError::TooLarge {
                    size: metadata.len(),
                    limit: MAX_PCAPNG_IMPORT_BYTES,
                });
            }
            let byte_budget_exceeded = Arc::new(AtomicBool::new(false));
            let bounded_file = PcapngImportReader::new(
                file,
                MAX_PCAPNG_IMPORT_BYTES,
                byte_budget_exceeded.clone(),
            );
            let mut reader = PcapNgReader::new(bounded_file)
                .map_err(|error| map_pcapng_reader_error(error, &byte_budget_exceeded))?;
            let mut decoder =
                PacketDecoder::with_ability_catalog(ability_catalog, use_server_damage_calibration);
            // PCAP replay is an explicit diagnostics/import operation and
            // retains the legacy full packet projection for export fidelity.
            decoder.packet_emission = PacketEmissionMode::FullDebug;
            let mut game_pause = GamePauseIntervalTracker::default();
            let mut resource_warnings = Vec::new();
            let enemy_catalog = load_resource(
                ENEMY_CATALOG_PATH,
                &mut resource_warnings,
                load_enemy_catalog,
            );
            if let Some(warning) = decoder.resource_warning() {
                let _ = sender.send(EngineEvent::Warning(warning));
            }
            if !resource_warnings.is_empty() {
                let _ = sender.send(EngineEvent::Warning(format!(
                    "enemy telemetry catalog: {}",
                    resource_warnings.join("; ")
                )));
            }
            let mut packet_count = 0;
            let mut supported_count = 0;
            let mut block_count = 0_usize;
            let mut interface_count = 0_usize;
            let mut packet_bytes = 0_u64;
            let mut previous_recorded_clock_health = None;

            while let Some(block) = reader.next_block() {
                if stop.load(Ordering::Relaxed) {
                    break;
                }
                block_count = block_count.saturating_add(1);
                if block_count > MAX_PCAPNG_IMPORT_BLOCKS {
                    return Err(PcapngImportError::TooManyBlocks {
                        count: block_count,
                        limit: MAX_PCAPNG_IMPORT_BLOCKS,
                    });
                }
                let block = block
                    .map_err(|error| map_pcapng_reader_error(error, &byte_budget_exceeded))?;
                let (interface_id, timestamp, data) = match block {
                    Block::InterfaceDescription(interface) => {
                        account_pcapng_interface(interface.snaplen, &mut interface_count)?;
                        continue;
                    }
                    Block::Unknown(block) if block.type_ == NTE_COMBAT_CLOCK_BLOCK_TYPE => {
                        if let Some(transition) = decode_combat_clock_block(block.value.as_ref()) {
                            publish_combat_clock_health(
                                &sender,
                                &mut previous_recorded_clock_health,
                                combat_clock_sample_health(transition.state_flags, true),
                            )
                            .map_err(|_| PcapngImportError::ReceiverDisconnected)?;
                            if let Some(timestamp) =
                                filetime_100ns_to_unix_seconds(transition.timestamp_100ns)
                                && let Some(event) = game_pause.apply_transition(
                                    timestamp,
                                    if transition.state_flags & COMBAT_CLOCK_PAUSE_VALID != 0 {
                                        transition.pause_type_mask
                                    } else {
                                        0
                                    },
                                )
                            {
                                send_game_pause_transition(&sender, event)
                                    .map_err(|_| PcapngImportError::ReceiverDisconnected)?;
                            }
                        }
                        continue;
                    }
                    Block::Unknown(block) if block.type_ == NTE_MOD_SCRIPT_BLOCK_TYPE => {
                        if let Some(mut event) = decode_mod_script_block(block.value.as_ref()) {
                            if event.mod_id == "enemy-telemetry"
                                && matches!(
                                    event.name.as_str(),
                                    "enemy.identity" | "enemy.hit_target"
                                )
                                && let [_, config_hash, _] = event.values.as_slice()
                            {
                                event.enemy_identity = enemy_catalog.get(*config_hash).cloned();
                            }
                            sender
                                .send(EngineEvent::ModScript(event))
                                .map_err(|_| PcapngImportError::ReceiverDisconnected)?;
                        }
                        continue;
                    }
                    Block::EnhancedPacket(packet) => {
                        account_pcapng_frame(
                            packet.data.len(),
                            &mut packet_count,
                            &mut packet_bytes,
                        )?;
                        (
                            packet.interface_id as usize,
                            FrameTimestamp::Known(packet.timestamp.as_secs_f64()),
                            packet.data.into_owned(),
                        )
                    }
                    Block::SimplePacket(packet) => {
                        account_pcapng_frame(
                            packet.data.len(),
                            &mut packet_count,
                            &mut packet_bytes,
                        )?;
                        (0, FrameTimestamp::Unknown, packet.data.into_owned())
                    }
                    _ => continue,
                };
                let Some(interface) = reader.interfaces().get(interface_id) else {
                    continue;
                };
                let Some(link_type) = CaptureLinkType::from_pcapng(interface.linktype) else {
                    continue;
                };
                supported_count += 1;
                let frame_local_ip_hint =
                    replay_frame_local_ip_hint(link_type, &data, local_ip_hint);
                decoder.process_capture_frame(
                    CapturedPacket {
                        link_type,
                        data: &data,
                        timestamp,
                    },
                    frame_local_ip_hint,
                    include_incoming,
                    &characters,
                    &sender,
                );
            }
            let pending_hits = decoder.take_all_ambiguous_hits();
            decoder.emit_hits(pending_hits, &characters, &sender);
            let pending_targetless_hits = decoder.take_all_targetless_hits();
            decoder.emit_hits(pending_targetless_hits, &characters, &sender);
            if packet_count > 0 && supported_count == 0 {
                return Err(PcapngImportError::InvalidFormat(
                    "pcapng contains no supported Ethernet or raw IPv4 packets".to_owned(),
                ));
            }
            Ok((packet_count, supported_count))
        })();

        match result {
            Ok((packet_count, supported_count)) => {
                let _ = sender.send(EngineEvent::Status(format!(
                    "pcapng import complete: read {packet_count} packets, parsed {supported_count} supported packets; {direction_mode}"
                )));
            }
            Err(error) => {
                let _ = sender.send(EngineEvent::Error(format!("pcapng import failed: {error}")));
            }
        }
        let _ = sender.send(EngineEvent::CaptureStopped);
    })
}

pub const CAPTURE_EXPORT_VERSION: u32 = 1;
/// A capture export never retains a second complete in-memory hit document.
/// The authoritative state is copied in short, generation-checked pages and
/// serialized after the state lock has been released.
pub const CAPTURE_EXPORT_HIT_PAGE_SIZE: usize = 1_024;

fn capture_export_version() -> u32 {
    CAPTURE_EXPORT_VERSION
}

#[derive(Clone, Debug)]
pub struct CaptureExportOptions {
    pub filter: String,
    pub include_incoming: bool,
    pub game_network: Option<CaptureExportNetwork>,
    pub dps_time_mode: DpsTimeBasis,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct CaptureExportNetwork {
    pub pid: u32,
    pub local_ip: String,
    pub remote_ip: String,
    pub remote_port: u16,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CaptureExportDocument {
    #[serde(default = "capture_export_version")]
    version: u32,
    #[serde(default)]
    exported_at: String,
    #[serde(default)]
    filter: String,
    #[serde(default)]
    include_incoming: bool,
    #[serde(default)]
    game_network: Option<CaptureExportNetwork>,
    #[serde(default)]
    summary: CaptureExportSummary,
    #[serde(default)]
    party: Vec<CaptureExportPartyRow>,
    #[serde(default)]
    abyss: CaptureExportAbyss,
    #[serde(default)]
    empty_curtain: Vec<EmptyCurtainItem>,
    #[serde(default)]
    empty_curtain_characters: Vec<EmptyCurtainCharacter>,
    #[serde(default)]
    hits: Vec<ExportHit>,
    #[serde(default)]
    packets: Vec<ExportPacket>,
    #[serde(default, deserialize_with = "deserialize_time_stop_events")]
    time_stop_events: Vec<TimeStopEvent>,
}

/// Immutable metadata for a generation-checked, page-streamed capture export.
/// `document.hits` is always empty: the hit body is supplied to the writer one
/// bounded page at a time so an arbitrarily long combat is never cloned while
/// the authoritative state lock is held.
#[derive(Clone, Debug)]
pub struct CaptureExportPlan {
    document: CaptureExportDocument,
    hits_generation: u64,
    hit_count: usize,
}

impl CaptureExportPlan {
    pub fn hits_generation(&self) -> u64 {
        self.hits_generation
    }

    pub fn hit_count(&self) -> usize {
        self.hit_count
    }
}

#[allow(dead_code)]
#[derive(Deserialize)]
enum CaptureTimeStopEvent {
    GamePauseStarted {
        timestamp: f64,
        pause_type_mask: u32,
    },
    GamePauseEnded {
        timestamp: f64,
        pause_type_mask: u32,
    },
    GamePause {
        start_timestamp: f64,
        end_timestamp: f64,
        pause_type_mask: u32,
    },
    GameTimerSample {
        timestamp: f64,
        remaining_seconds: i32,
    },
    UltraAnimation {
        timestamp: f64,
        char_id: u32,
        ability_id: serde::de::IgnoredAny,
        duration_seconds: f64,
    },
    ExtraStart {
        timestamp: f64,
        reason: serde::de::IgnoredAny,
    },
    ExtraEnd {
        timestamp: f64,
        reason: serde::de::IgnoredAny,
    },
}

fn deserialize_time_stop_events<'de, D>(deserializer: D) -> Result<Vec<TimeStopEvent>, D::Error>
where
    D: Deserializer<'de>,
{
    let saved_events = Vec::<CaptureTimeStopEvent>::deserialize(deserializer)?;
    let mut events = Vec::with_capacity(saved_events.len());
    for event in saved_events {
        match event {
            CaptureTimeStopEvent::GamePauseStarted {
                timestamp,
                pause_type_mask,
            } => {
                validate_saved_pause_state(timestamp, pause_type_mask).map_err(D::Error::custom)?;
                events.push(TimeStopEvent::GamePauseStarted {
                    timestamp,
                    pause_type_mask,
                });
            }
            CaptureTimeStopEvent::GamePauseEnded {
                timestamp,
                pause_type_mask,
            } => {
                validate_saved_pause_state(timestamp, pause_type_mask).map_err(D::Error::custom)?;
                events.push(TimeStopEvent::GamePauseEnded {
                    timestamp,
                    pause_type_mask,
                });
            }
            CaptureTimeStopEvent::GamePause {
                start_timestamp,
                end_timestamp,
                pause_type_mask,
            } => {
                validate_saved_pause_state(start_timestamp, pause_type_mask)
                    .map_err(D::Error::custom)?;
                validate_saved_pause_state(end_timestamp, pause_type_mask)
                    .map_err(D::Error::custom)?;
                if end_timestamp <= start_timestamp {
                    return Err(D::Error::custom("invalid saved game pause interval"));
                }
                events.push(TimeStopEvent::GamePauseStarted {
                    timestamp: start_timestamp,
                    pause_type_mask,
                });
                events.push(TimeStopEvent::GamePauseEnded {
                    timestamp: end_timestamp,
                    pause_type_mask,
                });
            }
            CaptureTimeStopEvent::GameTimerSample { .. }
            | CaptureTimeStopEvent::UltraAnimation { .. }
            | CaptureTimeStopEvent::ExtraStart { .. }
            | CaptureTimeStopEvent::ExtraEnd { .. } => {}
        }
    }
    Ok(events)
}

fn validate_saved_pause_state(timestamp: f64, pause_type_mask: u32) -> Result<(), &'static str> {
    if !timestamp.is_finite() || pause_type_mask == 0 || pause_type_mask & !0x1c != 0 {
        return Err("invalid saved game pause state");
    }
    Ok(())
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
struct CaptureExportSummary {
    hits: usize,
    packets: usize,
    total_damage: f64,
    dps: f64,
    duration_seconds: f64,
    dps_time_mode: String,
    #[serde(default)]
    started_at_unix: Option<f64>,
    #[serde(default)]
    started_at_local: Option<String>,
    #[serde(default)]
    ended_at_unix: Option<f64>,
    #[serde(default)]
    ended_at_local: Option<String>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
struct CaptureExportPartyRow {
    char_id: u32,
    name: String,
    hits: u64,
    damage: f64,
    dps: f64,
    duration_seconds: f64,
    share_percent: f64,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
struct CaptureExportAbyss {
    detected: bool,
    #[serde(default)]
    floor: Option<u32>,
    #[serde(default)]
    active_half: Option<String>,
    #[serde(default)]
    success_at_unix: Option<f64>,
    #[serde(default)]
    first_half_at_unix: Option<f64>,
    #[serde(default)]
    second_half_at_unix: Option<f64>,
    #[serde(default)]
    exited_at_unix: Option<f64>,
    #[serde(default)]
    first_half: CaptureExportAbyssHalf,
    #[serde(default)]
    second_half: CaptureExportAbyssHalf,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
struct CaptureExportAbyssHalf {
    hits: usize,
    total_damage: f64,
    total_damage_taken: f64,
    dps: f64,
    duration_seconds: f64,
    #[serde(default)]
    started_at_unix: Option<f64>,
    #[serde(default)]
    ended_at_unix: Option<f64>,
    #[serde(default)]
    party: Vec<CaptureExportAbyssPartyRow>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
struct CaptureExportAbyssPartyRow {
    char_id: u32,
    name: String,
    hits: u64,
    damage: f64,
    hits_taken: u64,
    damage_taken: f64,
    dps: f64,
    duration_seconds: f64,
    share_percent: f64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct ExportHit {
    timestamp_unix: f64,
    #[serde(default)]
    time_local: String,
    char_id: u32,
    char_name: String,
    damage: f64,
    #[serde(default = "default_outgoing_direction")]
    direction: HitDirection,
    #[serde(default)]
    target_hp_before: f64,
    #[serde(default)]
    target_hp_after: f64,
    #[serde(default)]
    target_max_hp: f64,
    #[serde(default)]
    max_hp_reduction: f64,
    #[serde(default)]
    target_hp_percent: f64,
    #[serde(default)]
    target_id: Option<String>,
    #[serde(default)]
    target_name: Option<String>,
    #[serde(default)]
    target_name_en: Option<String>,
    #[serde(default)]
    target_name_ja: Option<String>,
    #[serde(default)]
    target_monster_id: Option<String>,
    #[serde(default)]
    target_context: Vec<String>,
    #[serde(default)]
    gameplay_effect_index: Option<u32>,
    #[serde(default)]
    gameplay_effect_name: Option<String>,
    #[serde(default)]
    ability_name: Option<String>,
    #[serde(default)]
    damage_name: Option<String>,
    #[serde(default)]
    damage_component: Option<String>,
    #[serde(default)]
    attack_type: Option<String>,
    #[serde(default)]
    damage_attribute: Option<String>,
    #[serde(default)]
    follow_up_damage: f64,
    #[serde(default)]
    follow_up_timestamp: Option<f64>,
    #[serde(default)]
    follow_up_damage_name: Option<String>,
    #[serde(default)]
    follow_up_attack_type: Option<String>,
    #[serde(default)]
    follow_up_damage_attribute: Option<String>,
    #[serde(default)]
    reconciled_overkill_damage: Option<f64>,
}

fn default_outgoing_direction() -> HitDirection {
    HitDirection::Outgoing
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct ExportPacket {
    timestamp_unix: f64,
    #[serde(default)]
    time_local: String,
    source: String,
    destination: String,
    #[serde(default)]
    direction: String,
    #[serde(default)]
    payload_len: usize,
    #[serde(default, deserialize_with = "deserialize_export_declared_ids")]
    declared_ids: serde_json::Value,
    #[serde(default)]
    parsed_hits: usize,
    #[serde(default)]
    note: String,
    #[serde(default)]
    payload_preview: String,
    #[serde(default)]
    payload_hex: String,
    #[serde(default)]
    decoded_text: String,
}

fn deserialize_export_declared_ids<'de, D>(deserializer: D) -> Result<serde_json::Value, D::Error>
where
    D: Deserializer<'de>,
{
    struct DeclaredIdsVisitor;

    impl<'de> serde::de::Visitor<'de> for DeclaredIdsVisitor {
        type Value = serde_json::Value;

        fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            formatter
                .write_str("an array of at most 64 unsigned 32-bit ids or a bounded legacy string")
        }

        fn visit_seq<A>(self, mut sequence: A) -> Result<Self::Value, A::Error>
        where
            A: serde::de::SeqAccess<'de>,
        {
            let mut ids = Vec::with_capacity(
                sequence
                    .size_hint()
                    .unwrap_or_default()
                    .min(MAX_CAPTURE_JSON_IMPORT_DECLARED_IDS),
            );
            while let Some(value) = sequence.next_element::<u64>()? {
                if ids.len() >= MAX_CAPTURE_JSON_IMPORT_DECLARED_IDS {
                    return Err(serde::de::Error::custom("declared_ids exceeds item budget"));
                }
                let value = u32::try_from(value).map_err(|_| {
                    serde::de::Error::custom("declared_ids contains a non-u32 value")
                })?;
                ids.push(serde_json::Value::from(value));
            }
            Ok(serde_json::Value::Array(ids))
        }

        fn visit_borrowed_str<E>(self, value: &'de str) -> Result<Self::Value, E>
        where
            E: serde::de::Error,
        {
            parse_legacy_declared_ids(value).map_err(E::custom)
        }

        fn visit_str<E>(self, value: &str) -> Result<Self::Value, E>
        where
            E: serde::de::Error,
        {
            parse_legacy_declared_ids(value).map_err(E::custom)
        }
    }

    deserializer.deserialize_any(DeclaredIdsVisitor)
}

fn parse_legacy_declared_ids(value: &str) -> Result<serde_json::Value, &'static str> {
    if value.len() > MAX_CAPTURE_JSON_IMPORT_FILTER_BYTES {
        return Err("declared_ids legacy string exceeds byte budget");
    }
    let mut ids = Vec::new();
    for part in value
        .trim()
        .trim_matches(['[', ']'])
        .split(',')
        .map(str::trim)
        .filter(|part| !part.is_empty())
    {
        if ids.len() >= MAX_CAPTURE_JSON_IMPORT_DECLARED_IDS {
            return Err("declared_ids exceeds item budget");
        }
        let parsed = part
            .parse::<u32>()
            .map_err(|_| "declared_ids legacy string contains a non-u32 value")?;
        ids.push(serde_json::Value::from(parsed));
    }
    Ok(serde_json::Value::Array(ids))
}

impl CaptureExportDocument {
    pub fn prepare(state: &CombatState, options: CaptureExportOptions) -> CaptureExportPlan {
        let subtract_time_stop = options.dps_time_mode.subtracts_time_stop();
        let duration = state.duration_with_time_stop(subtract_time_stop).max(0.001);
        let ended_at = state
            .ended_at
            .into_iter()
            .chain(state.packets.iter().map(|packet| packet.timestamp))
            .max_by(|left, right| left.total_cmp(right));
        let party = capture_export_party(state, subtract_time_stop);
        let abyss = CaptureExportAbyss {
            detected: state.abyss.is_active(),
            floor: state.abyss.floor,
            active_half: state.abyss.active_half.map(|half| half.label().to_owned()),
            success_at_unix: state.abyss.success_at,
            first_half_at_unix: state.abyss.first_half_at,
            second_half_at_unix: state.abyss.second_half_at,
            exited_at_unix: state.abyss.exited_at,
            first_half: capture_export_abyss_half(&state.abyss.first_half, subtract_time_stop),
            second_half: capture_export_abyss_half(&state.abyss.second_half, subtract_time_stop),
        };

        let hit_count = state.hits.len();
        CaptureExportPlan {
            hits_generation: state.hits_generation,
            hit_count,
            document: Self {
                version: CAPTURE_EXPORT_VERSION,
                exported_at: Local::now().format("%Y-%m-%d %H:%M:%S").to_string(),
                filter: options.filter,
                include_incoming: options.include_incoming,
                game_network: options.game_network,
                summary: CaptureExportSummary {
                    hits: hit_count,
                    packets: state.packets.len(),
                    total_damage: state.total_damage,
                    dps: state.dps_with_time_stop(subtract_time_stop),
                    duration_seconds: duration,
                    dps_time_mode: options.dps_time_mode.label().to_owned(),
                    started_at_unix: state.started_at,
                    started_at_local: state.started_at.map(format_capture_time),
                    ended_at_unix: ended_at,
                    ended_at_local: ended_at.map(format_capture_time),
                },
                party,
                abyss,
                empty_curtain: state.empty_curtain.clone(),
                empty_curtain_characters: state.empty_curtain_characters.clone(),
                hits: Vec::new(),
                packets: state.packets.iter().map(ExportPacket::from).collect(),
                time_stop_events: state.time_stop_events.clone(),
            },
        }
    }

    /// Compatibility helper for low-frequency tests and callers that already
    /// own an isolated state. Production desktop export uses `prepare` plus
    /// `write_capture_export_streaming` and never calls this under a live lock.
    pub fn snapshot(state: &CombatState, options: CaptureExportOptions) -> Self {
        let mut plan = Self::prepare(state, options);
        plan.document.hits = state.hits.iter().map(ExportHit::from).collect();
        let ended_at = state
            .hits
            .iter()
            .map(|hit| hit.timestamp)
            .chain(state.packets.iter().map(|packet| packet.timestamp))
            .max_by(f64::total_cmp);
        plan.document.summary.ended_at_unix = ended_at;
        plan.document.summary.ended_at_local = ended_at.map(format_capture_time);
        plan.document
    }
}

impl From<&Hit> for ExportHit {
    fn from(hit: &Hit) -> Self {
        Self {
            timestamp_unix: hit.timestamp,
            time_local: format_capture_time(hit.timestamp),
            char_id: hit.char_id,
            char_name: hit.char_name.clone(),
            damage: hit.damage,
            direction: hit.direction,
            target_hp_before: hit.target_hp_before,
            target_hp_after: hit.target_hp_after,
            target_max_hp: hit.target_max_hp,
            max_hp_reduction: hit.max_hp_reduction,
            target_hp_percent: hit.target_hp_percent,
            target_id: hit.target_id.clone(),
            target_name: hit.target_name.clone(),
            target_name_en: hit.target_name_en.clone(),
            target_name_ja: hit.target_name_ja.clone(),
            target_monster_id: hit.target_monster_id.clone(),
            target_context: hit.target_context.clone(),
            gameplay_effect_index: hit.gameplay_effect_index,
            gameplay_effect_name: hit.gameplay_effect_name.clone(),
            ability_name: hit.ability_name.clone(),
            damage_name: hit.damage_name.clone(),
            damage_component: hit.damage_component.clone(),
            attack_type: hit.attack_type.clone(),
            damage_attribute: hit.damage_attribute.clone(),
            follow_up_damage: hit.follow_up_damage,
            follow_up_timestamp: hit.follow_up_timestamp,
            follow_up_damage_name: hit.follow_up_damage_name.clone(),
            follow_up_attack_type: hit.follow_up_attack_type.clone(),
            follow_up_damage_attribute: hit.follow_up_damage_attribute.clone(),
            reconciled_overkill_damage: hit.reconciled_overkill_damage,
        }
    }
}

impl From<&PacketDebug> for ExportPacket {
    fn from(packet: &PacketDebug) -> Self {
        Self {
            timestamp_unix: packet.timestamp,
            time_local: format_capture_time(packet.timestamp),
            source: packet.source.clone(),
            destination: packet.destination.clone(),
            direction: packet.direction.clone(),
            payload_len: packet.payload_len,
            declared_ids: serde_json::json!(packet.declared_ids),
            parsed_hits: packet.parsed_hits,
            note: packet.note.clone(),
            payload_preview: packet.payload_preview.clone(),
            payload_hex: packet.payload_hex.clone(),
            decoded_text: packet.decoded_text.clone(),
        }
    }
}

pub fn write_capture_export(path: &Path, document: &CaptureExportDocument) -> Result<(), String> {
    atomic_write_file(path, |writer| {
        serde_json::to_writer_pretty(writer, document).map_err(|error| error.to_string())
    })
}

struct StreamingCaptureExport<'a, F> {
    plan: &'a CaptureExportPlan,
    load_page: std::cell::RefCell<F>,
}

impl<F> Serialize for StreamingCaptureExport<'_, F>
where
    F: FnMut(usize, usize) -> Result<Vec<Hit>, String>,
{
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        let document = &self.plan.document;
        let ended_at = std::cell::RefCell::new(document.summary.ended_at_unix);
        let mut map = serializer.serialize_map(Some(13))?;
        map.serialize_entry("version", &document.version)?;
        map.serialize_entry("exported_at", &document.exported_at)?;
        map.serialize_entry("filter", &document.filter)?;
        map.serialize_entry("include_incoming", &document.include_incoming)?;
        map.serialize_entry("game_network", &document.game_network)?;
        map.serialize_entry("party", &document.party)?;
        map.serialize_entry("abyss", &document.abyss)?;
        map.serialize_entry("empty_curtain", &document.empty_curtain)?;
        map.serialize_entry(
            "empty_curtain_characters",
            &document.empty_curtain_characters,
        )?;
        map.serialize_entry(
            "hits",
            &StreamingCaptureHits {
                hit_count: self.plan.hit_count,
                load_page: &self.load_page,
                ended_at: &ended_at,
            },
        )?;
        let mut summary = document.summary.clone();
        summary.ended_at_unix = *ended_at.borrow();
        summary.ended_at_local = summary.ended_at_unix.map(format_capture_time);
        map.serialize_entry("summary", &summary)?;
        map.serialize_entry("packets", &document.packets)?;
        map.serialize_entry("time_stop_events", &document.time_stop_events)?;
        map.end()
    }
}

struct StreamingCaptureHits<'a, F> {
    hit_count: usize,
    load_page: &'a std::cell::RefCell<F>,
    ended_at: &'a std::cell::RefCell<Option<f64>>,
}

impl<F> Serialize for StreamingCaptureHits<'_, F>
where
    F: FnMut(usize, usize) -> Result<Vec<Hit>, String>,
{
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        let mut sequence = serializer.serialize_seq(Some(self.hit_count))?;
        let mut start = 0usize;
        while start < self.hit_count {
            let requested = self
                .hit_count
                .saturating_sub(start)
                .min(CAPTURE_EXPORT_HIT_PAGE_SIZE);
            let page = (self.load_page.borrow_mut())(start, requested)
                .map_err(<S::Error as serde::ser::Error>::custom)?;
            if page.len() != requested {
                return Err(<S::Error as serde::ser::Error>::custom(format!(
                    "capture export hit page length changed: expected {requested}, got {}",
                    page.len()
                )));
            }
            for hit in &page {
                if hit.timestamp.is_finite() {
                    let previous = *self.ended_at.borrow();
                    self.ended_at.replace(Some(
                        previous.map_or(hit.timestamp, |value| value.max(hit.timestamp)),
                    ));
                }
                sequence.serialize_element(&ExportHit::from(hit))?;
            }
            start = start.saturating_add(page.len());
        }
        sequence.end()
    }
}

/// Atomically writes a self-contained capture document while retaining at
/// most one bounded hit page outside the authoritative state. If the source
/// generation changes, the loader error aborts the temporary file and the
/// destination remains untouched.
pub fn write_capture_export_streaming<F>(
    path: &Path,
    plan: &CaptureExportPlan,
    load_page: F,
) -> Result<(), String>
where
    F: FnMut(usize, usize) -> Result<Vec<Hit>, String>,
{
    atomic_write_file(path, |writer| {
        serde_json::to_writer_pretty(
            writer,
            &StreamingCaptureExport {
                plan,
                load_page: std::cell::RefCell::new(load_page),
            },
        )
        .map_err(|error| error.to_string())
    })
}

fn capture_export_party(
    state: &CombatState,
    subtract_time_stop: bool,
) -> Vec<CaptureExportPartyRow> {
    let mut rows = state.stats.values().collect::<Vec<_>>();
    rows.sort_by(|left, right| right.damage.total_cmp(&left.damage));
    rows.into_iter()
        .map(|row| CaptureExportPartyRow {
            char_id: row.char_id,
            name: row.name.clone(),
            hits: row.hits,
            damage: row.damage,
            dps: state.character_dps_with_time_stop(row, subtract_time_stop),
            duration_seconds: state.character_duration_with_time_stop(row, subtract_time_stop),
            share_percent: if state.total_damage > 0.0 {
                row.damage / state.total_damage * 100.0
            } else {
                0.0
            },
        })
        .collect()
}

fn capture_export_abyss_half(
    party: &PartyCombatState,
    subtract_time_stop: bool,
) -> CaptureExportAbyssHalf {
    let mut rows = party.stats.values().collect::<Vec<_>>();
    rows.sort_by(|left, right| right.damage.total_cmp(&left.damage));
    let rows = rows
        .into_iter()
        .map(|row| CaptureExportAbyssPartyRow {
            char_id: row.char_id,
            name: row.name.clone(),
            hits: row.hits,
            damage: row.damage,
            hits_taken: row.hits_taken,
            damage_taken: row.damage_taken,
            dps: party.character_dps_with_time_stop(row, subtract_time_stop),
            duration_seconds: party.character_duration_with_time_stop(row, subtract_time_stop),
            share_percent: if party.total_damage > 0.0 {
                row.damage / party.total_damage * 100.0
            } else {
                0.0
            },
        })
        .collect();
    CaptureExportAbyssHalf {
        hits: party.hits.len(),
        total_damage: party.total_damage,
        total_damage_taken: party.total_damage_taken,
        dps: party.dps_with_time_stop(subtract_time_stop),
        duration_seconds: party.duration_with_time_stop(subtract_time_stop),
        started_at_unix: party.started_at,
        ended_at_unix: party.ended_at,
        party: rows,
    }
}

fn format_capture_time(timestamp: f64) -> String {
    DateTime::<Local>::from(std::time::UNIX_EPOCH + Duration::from_secs_f64(timestamp.max(0.0)))
        .format("%H:%M:%S%.3f")
        .to_string()
}

/// Byte budget enforced before a capture JSON export is read into memory.
/// The check runs on the shared Rust import boundary, never only in a Tauri
/// command or frontend drag-drop handler.
pub const MAX_CAPTURE_JSON_IMPORT_BYTES: u64 = 256 * 1024 * 1024;
/// Legacy comma repair is intentionally lower than the normal import budget:
/// a current document parses without a second text buffer, while a malformed
/// legacy document needs one bounded replacement allocation.
pub const MAX_CAPTURE_JSON_LEGACY_REPAIR_BYTES: usize = 64 * 1024 * 1024;
/// Structural budget for parsed hits. Runtime capture retains the complete
/// combat, while this external-file boundary still needs a finite allocation
/// budget before an untrusted document is accepted.
pub const MAX_CAPTURE_JSON_IMPORT_HITS: usize = 500_000;
/// Structural budget for parsed packet records in a capture export.
pub const MAX_CAPTURE_JSON_IMPORT_PACKETS: usize = 500_000;
/// Structural budgets for lower-volume capture metadata collections.
pub const MAX_CAPTURE_JSON_IMPORT_PARTY_ROWS: usize = 64;
pub const MAX_CAPTURE_JSON_IMPORT_EMPTY_CURTAIN_ITEMS: usize = 4_096;
pub const MAX_CAPTURE_JSON_IMPORT_EMPTY_CURTAIN_CHARACTERS: usize = 64;
pub const MAX_CAPTURE_JSON_IMPORT_TIME_STOP_EVENTS: usize = 8_192;
pub const MAX_CAPTURE_JSON_IMPORT_ITEM_STATS: usize = 32;
pub const MAX_CAPTURE_JSON_IMPORT_TARGET_CONTEXT: usize = 64;
pub const MAX_CAPTURE_JSON_IMPORT_DECLARED_IDS: usize = 64;
/// UTF-8 budgets for untrusted strings that can later cross a desktop stream
/// boundary. These are checked after deserialization but before any imported
/// record is published to the reducer.
pub const MAX_CAPTURE_JSON_IMPORT_CHARACTER_NAME_BYTES: usize = 128;
pub const MAX_CAPTURE_JSON_IMPORT_LABEL_BYTES: usize = 256;
pub const MAX_CAPTURE_JSON_IMPORT_FILTER_BYTES: usize = 4 * 1024;
pub const MAX_CAPTURE_JSON_IMPORT_PACKET_NOTE_BYTES: usize = 4 * 1024;
pub const MAX_CAPTURE_JSON_IMPORT_PACKET_PREVIEW_BYTES: usize = 16 * 1024;
pub const MAX_CAPTURE_JSON_IMPORT_PACKET_PAYLOAD_BYTES: usize = 256 * 1024;
pub const MAX_CAPTURE_JSON_IMPORT_TIMESTAMP: f64 = 253_402_300_799.0;
pub const MAX_CAPTURE_JSON_IMPORT_DAMAGE: f64 = 1.0e18;
pub const MAX_CAPTURE_JSON_IMPORT_TOTAL_DAMAGE: f64 = 1.0e24;
pub const MAX_CAPTURE_JSON_IMPORT_SCALAR: f64 = 1.0e24;

#[derive(Clone, Copy)]
struct CaptureImportStructureLimits {
    hits: usize,
    packets: usize,
    party_rows: usize,
    abyss_party_rows: usize,
    empty_curtain_items: usize,
    empty_curtain_characters: usize,
    time_stop_events: usize,
    item_stats: usize,
    target_context: usize,
    declared_ids: usize,
}

const CAPTURE_IMPORT_STRUCTURE_LIMITS: CaptureImportStructureLimits =
    CaptureImportStructureLimits {
        hits: MAX_CAPTURE_JSON_IMPORT_HITS,
        packets: MAX_CAPTURE_JSON_IMPORT_PACKETS,
        party_rows: MAX_CAPTURE_JSON_IMPORT_PARTY_ROWS,
        abyss_party_rows: MAX_CAPTURE_JSON_IMPORT_PARTY_ROWS,
        empty_curtain_items: MAX_CAPTURE_JSON_IMPORT_EMPTY_CURTAIN_ITEMS,
        empty_curtain_characters: MAX_CAPTURE_JSON_IMPORT_EMPTY_CURTAIN_CHARACTERS,
        time_stop_events: MAX_CAPTURE_JSON_IMPORT_TIME_STOP_EVENTS,
        item_stats: MAX_CAPTURE_JSON_IMPORT_ITEM_STATS,
        target_context: MAX_CAPTURE_JSON_IMPORT_TARGET_CONTEXT,
        declared_ids: MAX_CAPTURE_JSON_IMPORT_DECLARED_IDS,
    };

/// Typed rejection reasons for capture JSON imports. Messages intentionally
/// never embed the full local path of the rejected file.
#[derive(Debug)]
pub enum CaptureImportError {
    NotAFile,
    InvalidUtf8,
    InvalidFormat,
    UnsupportedVersion {
        found: u32,
        expected: u32,
    },
    TooLarge {
        size: u64,
        limit: u64,
    },
    TooManyHits {
        count: usize,
        limit: usize,
    },
    TooManyPackets {
        count: usize,
        limit: usize,
    },
    TooManyRecords {
        field: &'static str,
        count: usize,
        limit: usize,
    },
    FieldTooLarge {
        field: &'static str,
        size: usize,
        limit: usize,
    },
    InvalidNumber {
        field: &'static str,
    },
    InvalidEquipmentSnapshot,
    EquipmentDataUnavailable,
    Io(std::io::Error),
}

impl std::fmt::Display for CaptureImportError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotAFile => write!(formatter, "import path is not a regular file"),
            Self::InvalidUtf8 => formatter.write_str("capture export is not valid UTF-8"),
            Self::InvalidFormat => formatter.write_str("capture export is not valid JSON"),
            Self::UnsupportedVersion { found, expected } => write!(
                formatter,
                "unsupported capture export version {found}; expected {expected}"
            ),
            Self::TooLarge { size, limit } => write!(
                formatter,
                "import file is too large ({size} bytes; limit {limit} bytes)"
            ),
            Self::TooManyHits { count, limit } => write!(
                formatter,
                "capture export has too many hits ({count}; limit {limit})"
            ),
            Self::TooManyPackets { count, limit } => write!(
                formatter,
                "capture export has too many packets ({count}; limit {limit})"
            ),
            Self::TooManyRecords {
                field,
                count,
                limit,
            } => write!(
                formatter,
                "capture export field {field} has too many records ({count}; limit {limit})"
            ),
            Self::FieldTooLarge { field, size, limit } => write!(
                formatter,
                "capture export field {field} is too large ({size} UTF-8 bytes; limit {limit})"
            ),
            Self::InvalidNumber { field } => {
                write!(
                    formatter,
                    "capture export field {field} is outside its numeric budget"
                )
            }
            Self::InvalidEquipmentSnapshot => {
                formatter.write_str("capture export contains an invalid Console equipment snapshot")
            }
            Self::EquipmentDataUnavailable => {
                formatter.write_str("Console equipment data is unavailable")
            }
            Self::Io(error) => write!(formatter, "cannot read import file: {error}"),
        }
    }
}

impl std::error::Error for CaptureImportError {}

impl CaptureImportError {
    /// Stable adapter code for desktop command boundaries. The technical error
    /// remains typed here; frontends choose localized wording independently.
    pub const fn stable_code(&self) -> &'static str {
        match self {
            Self::NotAFile | Self::Io(_) => "replay_file_unavailable",
            Self::TooLarge { .. } => "replay_file_too_large",
            Self::InvalidUtf8 | Self::InvalidFormat => "replay_file_invalid",
            Self::UnsupportedVersion { .. } => "replay_version_unsupported",
            Self::TooManyHits { .. }
            | Self::TooManyPackets { .. }
            | Self::TooManyRecords { .. }
            | Self::FieldTooLarge { .. }
            | Self::InvalidNumber { .. }
            | Self::InvalidEquipmentSnapshot
            | Self::EquipmentDataUnavailable => "replay_validation_failed",
        }
    }
}

/// Checks a capture JSON import path before any bytes are read:
/// - the path must be a regular file;
/// - its metadata size must fit the import budget.
///
/// Every replay/import entry point must call this before `read_to_string`.
pub fn validate_capture_json_import(path: &Path) -> Result<(), CaptureImportError> {
    validate_capture_json_import_with_limit(path, MAX_CAPTURE_JSON_IMPORT_BYTES)
}

fn validate_capture_json_import_with_limit(
    path: &Path,
    limit: u64,
) -> Result<(), CaptureImportError> {
    let metadata = std::fs::metadata(path).map_err(CaptureImportError::Io)?;
    if !metadata.is_file() {
        return Err(CaptureImportError::NotAFile);
    }
    let size = metadata.len();
    if size > limit {
        return Err(CaptureImportError::TooLarge { size, limit });
    }
    Ok(())
}

fn read_bounded_utf8(
    reader: impl Read,
    checked_size: u64,
    limit: u64,
) -> Result<String, CaptureImportError> {
    if checked_size > limit {
        return Err(CaptureImportError::TooLarge {
            size: checked_size,
            limit,
        });
    }
    let mut reader = reader.take(limit.saturating_add(1));
    let mut bytes = Vec::new();
    reader
        .read_to_end(&mut bytes)
        .map_err(CaptureImportError::Io)?;
    let actual_size = bytes.len() as u64;
    if actual_size > limit {
        return Err(CaptureImportError::TooLarge {
            size: actual_size,
            limit,
        });
    }
    String::from_utf8(bytes).map_err(|_| CaptureImportError::InvalidUtf8)
}

fn read_capture_json_import_with_limit(
    path: &Path,
    limit: u64,
) -> Result<String, CaptureImportError> {
    let file = File::open(path).map_err(CaptureImportError::Io)?;
    let metadata = file.metadata().map_err(CaptureImportError::Io)?;
    if !metadata.is_file() {
        return Err(CaptureImportError::NotAFile);
    }
    read_bounded_utf8(file, metadata.len(), limit)
}

fn validate_capture_export_structure_with_limits(
    document: &CaptureExportDocument,
    limits: CaptureImportStructureLimits,
) -> Result<(), CaptureImportError> {
    if document.hits.len() > limits.hits {
        return Err(CaptureImportError::TooManyHits {
            count: document.hits.len(),
            limit: limits.hits,
        });
    }
    if document.packets.len() > limits.packets {
        return Err(CaptureImportError::TooManyPackets {
            count: document.packets.len(),
            limit: limits.packets,
        });
    }
    validate_capture_collection("party", document.party.len(), limits.party_rows)?;
    validate_capture_collection(
        "abyss.first_half.party",
        document.abyss.first_half.party.len(),
        limits.abyss_party_rows,
    )?;
    validate_capture_collection(
        "abyss.second_half.party",
        document.abyss.second_half.party.len(),
        limits.abyss_party_rows,
    )?;
    validate_capture_collection(
        "empty_curtain",
        document.empty_curtain.len(),
        limits.empty_curtain_items,
    )?;
    validate_capture_collection(
        "empty_curtain_characters",
        document.empty_curtain_characters.len(),
        limits.empty_curtain_characters,
    )?;
    validate_capture_collection(
        "time_stop_events",
        document.time_stop_events.len(),
        limits.time_stop_events,
    )?;
    validate_capture_text(
        "exported_at",
        &document.exported_at,
        MAX_CAPTURE_JSON_IMPORT_LABEL_BYTES,
    )?;
    for (field, value) in [
        ("summary.total_damage", document.summary.total_damage),
        ("summary.dps", document.summary.dps),
        (
            "summary.duration_seconds",
            document.summary.duration_seconds,
        ),
    ] {
        validate_capture_number(field, value, 0.0, MAX_CAPTURE_JSON_IMPORT_SCALAR)?;
    }
    for (field, value) in [
        ("summary.started_at_unix", document.summary.started_at_unix),
        ("summary.ended_at_unix", document.summary.ended_at_unix),
        ("abyss.success_at_unix", document.abyss.success_at_unix),
        (
            "abyss.first_half_at_unix",
            document.abyss.first_half_at_unix,
        ),
        (
            "abyss.second_half_at_unix",
            document.abyss.second_half_at_unix,
        ),
        ("abyss.exited_at_unix", document.abyss.exited_at_unix),
    ] {
        if let Some(value) = value {
            validate_capture_number(field, value, 0.0, MAX_CAPTURE_JSON_IMPORT_TIMESTAMP)?;
        }
    }
    validate_capture_text(
        "filter",
        &document.filter,
        MAX_CAPTURE_JSON_IMPORT_FILTER_BYTES,
    )?;
    validate_capture_text(
        "summary.dps_time_mode",
        &document.summary.dps_time_mode,
        MAX_CAPTURE_JSON_IMPORT_LABEL_BYTES,
    )?;
    for value in [
        document.summary.started_at_local.as_deref(),
        document.summary.ended_at_local.as_deref(),
        document.abyss.active_half.as_deref(),
    ]
    .into_iter()
    .flatten()
    {
        validate_capture_text(
            "capture metadata label",
            value,
            MAX_CAPTURE_JSON_IMPORT_LABEL_BYTES,
        )?;
    }
    if let Some(network) = &document.game_network {
        validate_capture_text(
            "game_network.local_ip",
            &network.local_ip,
            MAX_CAPTURE_JSON_IMPORT_LABEL_BYTES,
        )?;
        validate_capture_text(
            "game_network.remote_ip",
            &network.remote_ip,
            MAX_CAPTURE_JSON_IMPORT_LABEL_BYTES,
        )?;
    }
    for row in &document.party {
        validate_capture_text(
            "party[].name",
            &row.name,
            MAX_CAPTURE_JSON_IMPORT_CHARACTER_NAME_BYTES,
        )?;
        validate_capture_summary_row(row.damage, row.dps, row.duration_seconds, row.share_percent)?;
    }
    for row in document
        .abyss
        .first_half
        .party
        .iter()
        .chain(document.abyss.second_half.party.iter())
    {
        validate_capture_text(
            "abyss.*.party[].name",
            &row.name,
            MAX_CAPTURE_JSON_IMPORT_CHARACTER_NAME_BYTES,
        )?;
        validate_capture_summary_row(row.damage, row.dps, row.duration_seconds, row.share_percent)?;
        validate_capture_number(
            "abyss.*.party[].damage_taken",
            row.damage_taken,
            0.0,
            MAX_CAPTURE_JSON_IMPORT_SCALAR,
        )?;
    }
    for (field, half) in [
        ("abyss.first_half", &document.abyss.first_half),
        ("abyss.second_half", &document.abyss.second_half),
    ] {
        for value in [
            half.total_damage,
            half.total_damage_taken,
            half.dps,
            half.duration_seconds,
        ] {
            validate_capture_number(field, value, 0.0, MAX_CAPTURE_JSON_IMPORT_SCALAR)?;
        }
        for timestamp in [half.started_at_unix, half.ended_at_unix]
            .into_iter()
            .flatten()
        {
            validate_capture_number(field, timestamp, 0.0, MAX_CAPTURE_JSON_IMPORT_TIMESTAMP)?;
        }
    }
    for item in &document.empty_curtain {
        validate_capture_collection(
            "empty_curtain[].main_stats",
            item.main_stats.len(),
            limits.item_stats,
        )?;
        validate_capture_collection(
            "empty_curtain[].sub_stats",
            item.sub_stats.len(),
            limits.item_stats,
        )?;
        validate_capture_text(
            "empty_curtain[].item_id",
            &item.item_id,
            MAX_CAPTURE_JSON_IMPORT_LABEL_BYTES,
        )?;
        for stat in item.main_stats.iter().chain(item.sub_stats.iter()) {
            validate_capture_text(
                "empty_curtain[].stats[].property",
                &stat.property,
                MAX_CAPTURE_JSON_IMPORT_LABEL_BYTES,
            )?;
            validate_capture_number(
                "empty_curtain[].stats[].value",
                f64::from(stat.value),
                -MAX_CAPTURE_JSON_IMPORT_SCALAR,
                MAX_CAPTURE_JSON_IMPORT_SCALAR,
            )?;
        }
    }
    let mut aggregate_damage = 0.0_f64;
    for hit in &document.hits {
        validate_capture_collection(
            "hits[].target_context",
            hit.target_context.len(),
            limits.target_context,
        )?;
        validate_capture_text(
            "hits[].char_name",
            &hit.char_name,
            MAX_CAPTURE_JSON_IMPORT_CHARACTER_NAME_BYTES,
        )?;
        validate_capture_text(
            "hits[].time_local",
            &hit.time_local,
            MAX_CAPTURE_JSON_IMPORT_LABEL_BYTES,
        )?;
        for (field, value) in [
            ("hits[].target_id", hit.target_id.as_deref()),
            ("hits[].target_name", hit.target_name.as_deref()),
            ("hits[].target_name_en", hit.target_name_en.as_deref()),
            ("hits[].target_name_ja", hit.target_name_ja.as_deref()),
            ("hits[].target_monster_id", hit.target_monster_id.as_deref()),
            (
                "hits[].gameplay_effect_name",
                hit.gameplay_effect_name.as_deref(),
            ),
            ("hits[].ability_name", hit.ability_name.as_deref()),
            ("hits[].damage_name", hit.damage_name.as_deref()),
            ("hits[].damage_component", hit.damage_component.as_deref()),
            ("hits[].attack_type", hit.attack_type.as_deref()),
            ("hits[].damage_attribute", hit.damage_attribute.as_deref()),
            (
                "hits[].follow_up_damage_name",
                hit.follow_up_damage_name.as_deref(),
            ),
            (
                "hits[].follow_up_attack_type",
                hit.follow_up_attack_type.as_deref(),
            ),
            (
                "hits[].follow_up_damage_attribute",
                hit.follow_up_damage_attribute.as_deref(),
            ),
        ] {
            if let Some(value) = value {
                validate_capture_text(field, value, MAX_CAPTURE_JSON_IMPORT_LABEL_BYTES)?;
            }
        }
        for value in &hit.target_context {
            validate_capture_text(
                "hits[].target_context[]",
                value,
                MAX_CAPTURE_JSON_IMPORT_LABEL_BYTES,
            )?;
        }
        validate_capture_number(
            "hits[].timestamp_unix",
            hit.timestamp_unix,
            0.0,
            MAX_CAPTURE_JSON_IMPORT_TIMESTAMP,
        )?;
        validate_capture_number(
            "hits[].damage",
            hit.damage,
            0.0,
            MAX_CAPTURE_JSON_IMPORT_DAMAGE,
        )?;
        validate_capture_number(
            "hits[].follow_up_damage",
            hit.follow_up_damage,
            0.0,
            MAX_CAPTURE_JSON_IMPORT_DAMAGE,
        )?;
        validate_capture_number(
            "hits[].max_hp_reduction",
            hit.max_hp_reduction,
            0.0,
            MAX_CAPTURE_JSON_IMPORT_DAMAGE,
        )?;
        if let Some(overkill) = hit.reconciled_overkill_damage {
            validate_capture_number(
                "hits[].reconciled_overkill_damage",
                overkill,
                0.0,
                hit.damage,
            )?;
        }
        if let Some(timestamp) = hit.follow_up_timestamp {
            validate_capture_number(
                "hits[].follow_up_timestamp",
                timestamp,
                0.0,
                MAX_CAPTURE_JSON_IMPORT_TIMESTAMP,
            )?;
        }
        for (field, value) in [
            ("hits[].target_hp_before", hit.target_hp_before),
            ("hits[].target_hp_after", hit.target_hp_after),
            ("hits[].target_max_hp", hit.target_max_hp),
            ("hits[].target_hp_percent", hit.target_hp_percent),
        ] {
            validate_capture_number(field, value, 0.0, MAX_CAPTURE_JSON_IMPORT_SCALAR)?;
        }
        let next_aggregate_damage = aggregate_damage + hit.damage + hit.follow_up_damage;
        if !next_aggregate_damage.is_finite()
            || next_aggregate_damage > MAX_CAPTURE_JSON_IMPORT_TOTAL_DAMAGE
        {
            return Err(CaptureImportError::InvalidNumber {
                field: "hits[].aggregate_damage",
            });
        }
        aggregate_damage = next_aggregate_damage;
    }
    for packet in &document.packets {
        let declared_id_count = match &packet.declared_ids {
            serde_json::Value::Array(values) => values.len(),
            serde_json::Value::String(value) => value
                .trim_matches(['[', ']'])
                .split(',')
                .filter(|part| !part.trim().is_empty())
                .take(limits.declared_ids.saturating_add(1))
                .count(),
            _ => 0,
        };
        validate_capture_collection(
            "packets[].declared_ids",
            declared_id_count,
            limits.declared_ids,
        )?;
        for (field, value, limit) in [
            (
                "packets[].time_local",
                packet.time_local.as_str(),
                MAX_CAPTURE_JSON_IMPORT_LABEL_BYTES,
            ),
            (
                "packets[].source",
                packet.source.as_str(),
                MAX_CAPTURE_JSON_IMPORT_LABEL_BYTES,
            ),
            (
                "packets[].destination",
                packet.destination.as_str(),
                MAX_CAPTURE_JSON_IMPORT_LABEL_BYTES,
            ),
            (
                "packets[].direction",
                packet.direction.as_str(),
                MAX_CAPTURE_JSON_IMPORT_LABEL_BYTES,
            ),
            (
                "packets[].note",
                packet.note.as_str(),
                MAX_CAPTURE_JSON_IMPORT_PACKET_NOTE_BYTES,
            ),
            (
                "packets[].payload_preview",
                packet.payload_preview.as_str(),
                MAX_CAPTURE_JSON_IMPORT_PACKET_PREVIEW_BYTES,
            ),
            (
                "packets[].payload_hex",
                packet.payload_hex.as_str(),
                MAX_CAPTURE_JSON_IMPORT_PACKET_PAYLOAD_BYTES,
            ),
            (
                "packets[].decoded_text",
                packet.decoded_text.as_str(),
                MAX_CAPTURE_JSON_IMPORT_PACKET_PAYLOAD_BYTES,
            ),
        ] {
            validate_capture_text(field, value, limit)?;
        }
        if let serde_json::Value::String(value) = &packet.declared_ids {
            validate_capture_text(
                "packets[].declared_ids",
                value,
                MAX_CAPTURE_JSON_IMPORT_FILTER_BYTES,
            )?;
        }
        validate_capture_number(
            "packets[].timestamp_unix",
            packet.timestamp_unix,
            0.0,
            MAX_CAPTURE_JSON_IMPORT_TIMESTAMP,
        )?;
        if packet.payload_len > CAPTURE_SNAPLEN as usize {
            return Err(CaptureImportError::InvalidNumber {
                field: "packets[].payload_len",
            });
        }
    }
    for event in &document.time_stop_events {
        let timestamp = match event {
            TimeStopEvent::GamePauseStarted { timestamp, .. }
            | TimeStopEvent::GamePauseEnded { timestamp, .. } => *timestamp,
        };
        validate_capture_number(
            "time_stop_events[].timestamp",
            timestamp,
            0.0,
            MAX_CAPTURE_JSON_IMPORT_TIMESTAMP,
        )?;
    }
    Ok(())
}

fn validate_capture_summary_row(
    damage: f64,
    dps: f64,
    duration_seconds: f64,
    share_percent: f64,
) -> Result<(), CaptureImportError> {
    for (field, value, max) in [
        ("party[].damage", damage, MAX_CAPTURE_JSON_IMPORT_SCALAR),
        ("party[].dps", dps, MAX_CAPTURE_JSON_IMPORT_SCALAR),
        (
            "party[].duration_seconds",
            duration_seconds,
            MAX_CAPTURE_JSON_IMPORT_TIMESTAMP,
        ),
        ("party[].share_percent", share_percent, 100.0),
    ] {
        validate_capture_number(field, value, 0.0, max)?;
    }
    Ok(())
}

fn validate_capture_number(
    field: &'static str,
    value: f64,
    minimum: f64,
    maximum: f64,
) -> Result<(), CaptureImportError> {
    if !value.is_finite() || value < minimum || value > maximum {
        return Err(CaptureImportError::InvalidNumber { field });
    }
    Ok(())
}

fn validate_capture_text(
    field: &'static str,
    value: &str,
    limit: usize,
) -> Result<(), CaptureImportError> {
    let size = value.len();
    if size > limit {
        return Err(CaptureImportError::FieldTooLarge { field, size, limit });
    }
    Ok(())
}

fn validate_capture_collection(
    field: &'static str,
    count: usize,
    limit: usize,
) -> Result<(), CaptureImportError> {
    if count > limit {
        return Err(CaptureImportError::TooManyRecords {
            field,
            count,
            limit,
        });
    }
    Ok(())
}

/// Fully validated JSON replay input. The opened file is read once through a
/// bounded reader, and every fallible document/resource validation completes
/// before the live-capture service is allowed to replace authoritative state.
#[derive(Debug)]
pub struct PreparedCaptureJsonReplay {
    document: CaptureExportDocument,
    empty_curtain: Vec<EmptyCurtainItem>,
    empty_curtain_characters: Vec<EmptyCurtainCharacter>,
    time_stop_events: Vec<TimeStopEvent>,
    equipment_catalog: EquipmentCatalog,
    equipment_warning: Option<String>,
}

pub fn prepare_capture_json_replay(
    path: &Path,
) -> Result<PreparedCaptureJsonReplay, CaptureImportError> {
    prepare_capture_json_replay_with_limits(
        path,
        MAX_CAPTURE_JSON_IMPORT_BYTES,
        CAPTURE_IMPORT_STRUCTURE_LIMITS,
    )
}

fn prepare_capture_json_replay_with_limits(
    path: &Path,
    byte_limit: u64,
    structure_limits: CaptureImportStructureLimits,
) -> Result<PreparedCaptureJsonReplay, CaptureImportError> {
    let text = read_capture_json_import_with_limit(path, byte_limit)?;
    let mut document = parse_capture_export_typed(text)?;
    validate_capture_export_structure_with_limits(&document, structure_limits)?;
    if document.packets.iter().any(|packet| {
        !packet.payload_hex.trim().is_empty()
            && (packet.payload_hex.len() % 2 != 0
                || !packet
                    .payload_hex
                    .bytes()
                    .all(|byte| byte.is_ascii_hexdigit()))
    }) {
        return Err(CaptureImportError::InvalidFormat);
    }

    let empty_curtain = std::mem::take(&mut document.empty_curtain);
    let time_stop_events = std::mem::take(&mut document.time_stop_events);
    let mut empty_curtain_characters = std::mem::take(&mut document.empty_curtain_characters);
    if empty_curtain_characters.is_empty() {
        empty_curtain_characters = empty_curtain
            .iter()
            .filter_map(|item| {
                Some(EmptyCurtainCharacter {
                    net_id: item.character_net_id?,
                    character_id: item.equipped_character_id?,
                })
            })
            .collect();
        empty_curtain_characters.sort_by_key(|character| {
            (
                character.character_id,
                character.net_id.solt,
                character.net_id.serial,
            )
        });
        empty_curtain_characters.dedup();
    }
    validate_capture_collection(
        "empty_curtain_characters",
        empty_curtain_characters.len(),
        structure_limits.empty_curtain_characters,
    )?;
    let empty_curtain_characters = validate_empty_curtain_characters(empty_curtain_characters)
        .ok_or(CaptureImportError::InvalidEquipmentSnapshot)?;

    let (equipment_catalog, equipment_warning) =
        match find_data_file(Path::new(EQUIPMENT_CATALOG_PATH)) {
            Some(path) => match load_equipment_catalog(&path) {
                Ok(catalog) => (catalog, None),
                Err(error) if empty_curtain.is_empty() => (
                    EquipmentCatalog::default(),
                    Some(format!(
                        "Failed to load Console equipment data for JSON replay: {error:#}"
                    )),
                ),
                Err(_) => return Err(CaptureImportError::EquipmentDataUnavailable),
            },
            None if empty_curtain.is_empty() => (EquipmentCatalog::default(), None),
            None => return Err(CaptureImportError::EquipmentDataUnavailable),
        };
    if !validate_empty_curtain_snapshot(&empty_curtain, &equipment_catalog) {
        return Err(CaptureImportError::InvalidEquipmentSnapshot);
    }

    Ok(PreparedCaptureJsonReplay {
        document,
        empty_curtain,
        empty_curtain_characters,
        time_stop_events,
        equipment_catalog,
        equipment_warning,
    })
}

pub fn import_capture_json(
    path: PathBuf,
    sender: impl Into<EngineEventSink>,
    stop: Arc<AtomicBool>,
) -> Result<thread::JoinHandle<()>, CaptureImportError> {
    let prepared = prepare_capture_json_replay(&path)?;
    import_prepared_capture_json(prepared, sender, stop).map_err(CaptureImportError::Io)
}

pub fn import_prepared_capture_json(
    prepared: PreparedCaptureJsonReplay,
    sender: impl Into<EngineEventSink>,
    stop: Arc<AtomicBool>,
) -> std::io::Result<thread::JoinHandle<()>> {
    let sender = sender.into();
    thread::Builder::new()
        .name("nte-json-replay".to_owned())
        .spawn(move || {
            let result = (|| -> Result<(usize, usize), String> {
                let PreparedCaptureJsonReplay {
                    mut document,
                    empty_curtain: saved_empty_curtain,
                    empty_curtain_characters: saved_empty_curtain_characters,
                    time_stop_events: mut saved_time_stop_events,
                    equipment_catalog,
                    equipment_warning,
                } = prepared;
                if let Some(warning) = equipment_warning {
                    sender
                        .send(EngineEvent::Warning(warning))
                        .map_err(|error| error.to_string())?;
                }
                if !saved_time_stop_events.is_empty() {
                    sender
                        .send(EngineEvent::CombatClockHealth(
                            CombatClockRuntimeHealth::Recorded,
                        ))
                        .map_err(|error| error.to_string())?;
                }
                let mut empty_curtain = EmptyCurtainDecoder::new(equipment_catalog);
                let hit_count = document.hits.len();
                let mut packet_count = 0;
                document
                    .packets
                    .sort_by(|left, right| left.timestamp_unix.total_cmp(&right.timestamp_unix));
                document
                    .hits
                    .sort_by(|left, right| left.timestamp_unix.total_cmp(&right.timestamp_unix));
                saved_time_stop_events.sort_by(|left, right| {
                    time_stop_event_timestamp(left).total_cmp(&time_stop_event_timestamp(right))
                });
                let mut packets = document.packets.into_iter().peekable();
                let mut hits = document.hits.into_iter().peekable();
                let mut time_stop_events = saved_time_stop_events.into_iter().peekable();

                while packets.peek().is_some()
                    || hits.peek().is_some()
                    || time_stop_events.peek().is_some()
                {
                    if stop.load(Ordering::Relaxed) {
                        break;
                    }
                    let packet_timestamp = packets
                        .peek()
                        .map_or(f64::INFINITY, |packet| packet.timestamp_unix);
                    let hit_timestamp = hits.peek().map_or(f64::INFINITY, |hit| hit.timestamp_unix);
                    let time_stop_timestamp = time_stop_events
                        .peek()
                        .map_or(f64::INFINITY, time_stop_event_timestamp);
                    if time_stop_timestamp <= packet_timestamp
                        && time_stop_timestamp <= hit_timestamp
                    {
                        let Some(event) = time_stop_events.next() else {
                            return Err(
                                "capture replay time-stop ordering became invalid".to_owned()
                            );
                        };
                        sender
                            .send(EngineEvent::TimeStop(event))
                            .map_err(|error| error.to_string())?;
                        continue;
                    }
                    let take_packet = match (packets.peek(), hits.peek()) {
                        (Some(packet), Some(hit)) => packet.timestamp_unix <= hit.timestamp_unix,
                        (Some(_), None) => true,
                        (None, Some(_)) => false,
                        (None, None) => break,
                    };
                    if take_packet {
                        let Some(packet) = packets.next() else {
                            return Err("capture replay packet ordering became invalid".to_owned());
                        };
                        if send_export_packet(packet, &sender, &mut empty_curtain)? {
                            packet_count += 1;
                        }
                    } else {
                        let Some(hit) = hits.next() else {
                            return Err("capture replay hit ordering became invalid".to_owned());
                        };
                        let event = export_hit_event(hit);
                        sender.send(event).map_err(|error| error.to_string())?;
                    }
                }
                if !stop.load(Ordering::Relaxed) && !saved_empty_curtain_characters.is_empty() {
                    sender
                        .send(EngineEvent::EmptyCurtainCharacters(
                            saved_empty_curtain_characters,
                        ))
                        .map_err(|error| error.to_string())?;
                }
                if !stop.load(Ordering::Relaxed) && !saved_empty_curtain.is_empty() {
                    sender
                        .send(EngineEvent::EmptyCurtain(saved_empty_curtain))
                        .map_err(|error| error.to_string())?;
                }
                Ok((hit_count, packet_count))
            })();

            match result {
                Ok((hit_count, packet_count)) => {
                    let _ = sender.send(EngineEvent::Status(format!(
                        "JSON import complete: {packet_count} packets, {hit_count} hits"
                    )));
                }
                Err(error) => {
                    let _ = sender.send(EngineEvent::Error(format!("JSON import failed: {error}")));
                }
            }
            let _ = sender.send(EngineEvent::CaptureStopped);
        })
}

impl EngineEventDeliveryGate {
    fn wait_until_ready(&self) -> Result<(), EngineEventSendError> {
        let mut state = match self.state.lock() {
            Ok(state) => state,
            Err(mut error) => {
                **error.get_mut() = EngineEventDeliveryState::Cancelled;
                self.state.clear_poison();
                self.ready.notify_all();
                return Err(EngineEventSendError);
            }
        };
        while *state == EngineEventDeliveryState::Pending {
            state = match self.ready.wait(state) {
                Ok(state) => state,
                Err(mut error) => {
                    **error.get_mut() = EngineEventDeliveryState::Cancelled;
                    self.state.clear_poison();
                    self.ready.notify_all();
                    return Err(EngineEventSendError);
                }
            };
        }
        match *state {
            EngineEventDeliveryState::Released => Ok(()),
            EngineEventDeliveryState::Cancelled | EngineEventDeliveryState::Pending => {
                Err(EngineEventSendError)
            }
        }
    }
}

impl EngineEventDeliveryPermit {
    pub fn release(mut self) {
        self.finish(EngineEventDeliveryState::Released);
    }

    pub fn cancel(mut self) {
        self.finish(EngineEventDeliveryState::Cancelled);
    }

    fn finish(&mut self, next: EngineEventDeliveryState) {
        let Some(gate) = self.gate.take() else {
            return;
        };
        match gate.state.lock() {
            Ok(mut state) => *state = next,
            Err(mut error) => {
                **error.get_mut() = EngineEventDeliveryState::Cancelled;
                gate.state.clear_poison();
            }
        }
        gate.ready.notify_all();
    }
}

impl Drop for EngineEventDeliveryPermit {
    fn drop(&mut self) {
        self.finish(EngineEventDeliveryState::Cancelled);
    }
}

fn validate_empty_curtain_characters(
    characters: Vec<EmptyCurtainCharacter>,
) -> Option<Vec<EmptyCurtainCharacter>> {
    let mut character_ids = HashMap::with_capacity(characters.len());
    let mut validated = Vec::with_capacity(characters.len());
    for character in characters {
        if !valid_item_net_id(character.net_id) || character.character_id == 0 {
            return None;
        }
        match character_ids.insert(character.net_id, character.character_id) {
            None => validated.push(character),
            Some(existing) if existing == character.character_id => {}
            Some(_) => return None,
        }
    }
    Some(validated)
}

fn send_export_packet(
    packet: ExportPacket,
    sender: &EngineEventSink,
    empty_curtain: &mut EmptyCurtainDecoder,
) -> Result<bool, String> {
    let mut declared_ids = parse_export_ids(&packet.declared_ids);
    let payload = if packet.payload_hex.trim().is_empty() {
        Vec::new()
    } else {
        hex::decode(&packet.payload_hex).map_err(|error| format!("payload_hex 无效: {error}"))?
    };
    let evidence = find_declared_character_evidence(&payload);
    let final_tower_evidence = find_final_tower_character_evidence(&payload);
    let character_evidence = merged_character_evidence(&evidence, &final_tower_evidence);
    append_unique_ids(
        &mut declared_ids,
        character_ids_from_evidence_sources(&evidence, &final_tower_evidence),
    );
    let decoded_text = if packet.decoded_text.trim().is_empty() && !payload.is_empty() {
        decode_payload_text(&payload)
    } else {
        packet.decoded_text
    };
    let equipment_slots = parse_equipment_slots(&payload);
    let inventory_result = if !packet.direction.eq_ignore_ascii_case("C2S") {
        match parse_transport_packet(&payload) {
            Some(TransportPacket::Sequenced(transport)) => empty_curtain.process_packet(
                InventoryConnectionKey::new(packet.source.clone(), packet.destination.clone()),
                &transport,
            ),
            _ => InventoryPacketResult::default(),
        }
    } else {
        InventoryPacketResult::default()
    };
    if let Some(characters) = inventory_result.characters {
        sender
            .send(EngineEvent::EmptyCurtainCharacters(characters))
            .map_err(|error| error.to_string())?;
    }
    if let Some(snapshot) = inventory_result.snapshot {
        sender
            .send(EngineEvent::EmptyCurtain(snapshot))
            .map_err(|error| error.to_string())?;
    }
    if !should_keep_debug_packet(
        &payload,
        &declared_ids,
        packet.parsed_hits,
        equipment_slots.len(),
        inventory_result.recognized,
        decoded_text != UNREADABLE_PROTOCOL_TEXT,
    ) {
        return Ok(false);
    }
    let mut note = packet.note;
    append_packet_note(
        &mut note,
        binary_payload_diagnostic(
            &payload,
            &packet.direction,
            &decoded_text,
            &character_evidence,
        ),
    );
    append_packet_note(&mut note, equipment_slots_note(&equipment_slots));
    let packet = PacketDebug {
        timestamp: packet.timestamp_unix,
        source: packet.source,
        destination: packet.destination,
        direction: packet.direction,
        payload_len: packet.payload_len,
        declared_ids,
        parsed_hits: packet.parsed_hits,
        note,
        payload_preview: packet.payload_preview,
        payload_hex: packet.payload_hex,
        decoded_text,
    };
    send_packet_events(sender, packet).map_err(|error| error.to_string())?;
    Ok(true)
}

fn time_stop_event_timestamp(event: &TimeStopEvent) -> f64 {
    match event {
        TimeStopEvent::GamePauseStarted { timestamp, .. }
        | TimeStopEvent::GamePauseEnded { timestamp, .. } => *timestamp,
    }
}

fn export_hit_event(hit: ExportHit) -> EngineEvent {
    EngineEvent::Hit(Box::new(Hit {
        timestamp: hit.timestamp_unix,
        char_id: hit.char_id,
        char_name: hit.char_name,
        char_known: true,
        damage: hit.damage,
        byte_offset: 0,
        bit_shift: 0,
        char_source: HitCharacterSource::ExportJson,
        direction: hit.direction,
        target_hp_before: hit.target_hp_before,
        target_hp_after: hit.target_hp_after,
        target_max_hp: hit.target_max_hp,
        max_hp_reduction: hit.max_hp_reduction,
        target_hp_percent: hit.target_hp_percent,
        target_id: hit.target_id,
        target_name: hit.target_name,
        target_name_en: hit.target_name_en,
        target_name_ja: hit.target_name_ja,
        target_monster_id: hit.target_monster_id,
        target_context: hit.target_context,
        gameplay_effect_index: hit.gameplay_effect_index,
        gameplay_effect_name: hit.gameplay_effect_name,
        ability_name: hit.ability_name,
        damage_name: hit.damage_name.map(|name| normalize_damage_name(&name)),
        damage_component: hit.damage_component,
        attack_type: hit.attack_type.map(|attack_type| {
            if attack_type == "QTE" {
                "环合".to_owned()
            } else if let Some(reaction_type) = attack_type.strip_prefix("QTE·") {
                format!("环合·{reaction_type}")
            } else {
                attack_type
            }
        }),
        damage_attribute: hit.damage_attribute,
        follow_up_damage: hit.follow_up_damage,
        follow_up_timestamp: hit.follow_up_timestamp,
        follow_up_damage_name: hit.follow_up_damage_name,
        follow_up_attack_type: hit.follow_up_attack_type,
        follow_up_damage_attribute: hit.follow_up_damage_attribute,
        reconciled_overkill_damage: hit.reconciled_overkill_damage,
        wire_event: None,
    }))
}

fn parse_capture_export_typed(
    text: impl Into<String>,
) -> Result<CaptureExportDocument, CaptureImportError> {
    let text = text.into();
    let document: CaptureExportDocument = match serde_json::from_str(&text) {
        Ok(document) => document,
        Err(_) => {
            // Legacy v1 writers omitted a comma after pretty-printed payload_hex
            // lines. Build one bounded replacement buffer, release the original,
            // and only then deserialize. The old Vec<String> + join path retained
            // the original, every line allocation, and the repaired document at
            // the same time for inputs up to the full import limit.
            let repaired = repair_legacy_capture_export(&text)
                .map_err(|_| CaptureImportError::InvalidFormat)?;
            drop(text);
            serde_json::from_str(&repaired).map_err(|_| CaptureImportError::InvalidFormat)?
        }
    };
    if document.version != CAPTURE_EXPORT_VERSION {
        return Err(CaptureImportError::UnsupportedVersion {
            found: document.version,
            expected: CAPTURE_EXPORT_VERSION,
        });
    }
    Ok(document)
}

#[cfg(test)]
fn parse_capture_export(text: impl Into<String>) -> Result<CaptureExportDocument, String> {
    parse_capture_export_typed(text).map_err(|error| error.to_string())
}

fn repair_legacy_capture_export(text: &str) -> Result<String, String> {
    repair_legacy_capture_export_with_limit(text, MAX_CAPTURE_JSON_LEGACY_REPAIR_BYTES)
}

fn repair_legacy_capture_export_with_limit(text: &str, limit: usize) -> Result<String, String> {
    if text.len() > limit {
        return Err(format!("legacy capture repair input exceeds {limit} bytes"));
    }
    let inserted_commas = text
        .split_inclusive('\n')
        .filter(|line| legacy_payload_hex_line_needs_comma(line))
        .count();
    let repaired_len = text
        .len()
        .checked_add(inserted_commas)
        .filter(|size| *size <= limit)
        .ok_or_else(|| format!("legacy capture repair exceeds {limit} bytes"))?;
    let mut repaired = String::with_capacity(repaired_len);
    for line in text.split_inclusive('\n') {
        let line_without_newline = line.strip_suffix('\n').unwrap_or(line);
        let content = line_without_newline
            .strip_suffix('\r')
            .unwrap_or(line_without_newline);
        repaired.push_str(content);
        if legacy_payload_hex_line_needs_comma(content) {
            repaired.push(',');
        }
        if line_without_newline.len() != line.len() {
            if line_without_newline.ends_with('\r') {
                repaired.push('\r');
            }
            repaired.push('\n');
        }
    }
    debug_assert_eq!(repaired.len(), repaired_len);
    Ok(repaired)
}

fn legacy_payload_hex_line_needs_comma(line: &str) -> bool {
    let content = line.trim_end_matches(['\r', '\n']);
    if content.trim_end().ends_with(',') {
        return false;
    }
    let Some(after_key) = content.trim_start().strip_prefix(r#""payload_hex""#) else {
        return false;
    };
    after_key.trim_start().starts_with(':')
}

fn parse_export_ids(value: &serde_json::Value) -> Vec<u32> {
    match value {
        serde_json::Value::Array(values) => values
            .iter()
            .filter_map(serde_json::Value::as_u64)
            .filter_map(|value| u32::try_from(value).ok())
            .collect(),
        serde_json::Value::String(value) => value
            .trim_matches(['[', ']'])
            .split(',')
            .filter_map(|part| part.trim().parse().ok())
            .collect(),
        _ => Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use crossbeam_channel::unbounded;

    use crate::engine::parser::{
        CHARACTER_DATA_PATH, ParsedEmptyCurtainModulePlacement, load_characters,
    };

    #[test]
    fn staged_event_sink_releases_delivery_only_after_commit() {
        let (sender, receiver) = bounded(1);
        let (sink, permit) = EngineEventSink::reliable(sender).pause_delivery();
        let producer = thread::spawn(move || sink.send(EngineEvent::CaptureStopped));

        assert!(receiver.recv_timeout(Duration::from_millis(20)).is_err());
        permit.release();
        assert!(matches!(
            receiver.recv_timeout(Duration::from_secs(1)),
            Ok(EngineEvent::CaptureStopped)
        ));
        assert!(producer.join().expect("producer should finish").is_ok());
    }

    #[test]
    fn cancelled_staged_event_sink_wakes_the_blocked_producer() {
        let (sender, receiver) = bounded(1);
        let (sink, permit) = EngineEventSink::reliable(sender).pause_delivery();
        let producer = thread::spawn(move || sink.send(EngineEvent::CaptureStopped));

        assert!(receiver.recv_timeout(Duration::from_millis(20)).is_err());
        permit.cancel();
        assert!(producer.join().expect("producer should finish").is_err());
        assert!(receiver.try_recv().is_err());
    }

    #[test]
    fn capture_stop_drains_full_reliable_lane_before_join() {
        let (sender, receiver) = bounded(1);
        sender
            .send(EngineEvent::Status("queued".to_owned()))
            .expect("prefill reliable lane");
        let stop = Arc::new(AtomicBool::new(false));
        let producer_stop = Arc::clone(&stop);
        let sink = EngineEventSink::reliable(sender);
        let producer = thread::spawn(move || {
            while !producer_stop.load(Ordering::Relaxed) {
                thread::yield_now();
            }
            let _ = sink.send(EngineEvent::CaptureStopped);
        });
        let mut capture = CaptureHandle::from_test_thread(stop, producer);
        let (completed_sender, completed_receiver) = bounded(1);
        thread::spawn(move || {
            let mut drained = Vec::new();
            capture.stop_with_drain(|| {
                drained.extend(receiver.try_iter());
            });
            let _ = completed_sender.send(drained);
        });

        let drained = completed_receiver
            .recv_timeout(Duration::from_secs(2))
            .expect("full reliable lane stop must finish without waiting forever");
        assert!(matches!(drained.first(), Some(EngineEvent::Status(value)) if value == "queued"));
        assert!(matches!(drained.last(), Some(EngineEvent::CaptureStopped)));
    }

    #[test]
    fn bounded_utf8_reader_rejects_growth_past_checked_size() {
        let limit = 8_u64;
        let reader = std::io::Cursor::new(b"123456789".to_vec());
        assert!(matches!(
            read_bounded_utf8(reader, limit, limit),
            Err(CaptureImportError::TooLarge { size: 9, limit: 8 })
        ));
    }

    #[test]
    fn combat_clock_health_is_typed_and_published_only_on_change() {
        assert_eq!(
            combat_clock_error_health(CombatClockQueryError::ProviderUnavailable),
            CombatClockRuntimeHealth::ProviderUnavailable
        );
        assert_eq!(
            combat_clock_error_health(CombatClockQueryError::ModDisabled),
            CombatClockRuntimeHealth::ModDisabled
        );
        assert_eq!(
            combat_clock_error_health(CombatClockQueryError::InvalidResponse),
            CombatClockRuntimeHealth::InvalidResponse
        );
        assert_eq!(
            combat_clock_sample_health(0, false),
            CombatClockRuntimeHealth::DataUnavailable
        );
        assert_eq!(
            combat_clock_sample_health(COMBAT_CLOCK_PAUSE_VALID, false),
            CombatClockRuntimeHealth::Available
        );
        assert_eq!(
            combat_clock_sample_health(COMBAT_CLOCK_PAUSE_VALID, true),
            CombatClockRuntimeHealth::Recorded
        );

        let (sender, receiver) = bounded(2);
        let sink = EngineEventSink::reliable(sender);
        let mut previous = None;
        publish_combat_clock_health(
            &sink,
            &mut previous,
            CombatClockRuntimeHealth::ProviderUnavailable,
        )
        .expect("first degradation is published");
        publish_combat_clock_health(
            &sink,
            &mut previous,
            CombatClockRuntimeHealth::ProviderUnavailable,
        )
        .expect("duplicate degradation is a no-op");
        assert!(matches!(
            receiver.try_recv(),
            Ok(EngineEvent::CombatClockHealth(
                CombatClockRuntimeHealth::ProviderUnavailable
            ))
        ));
        assert!(receiver.try_recv().is_err());
    }

    #[test]
    fn combat_clock_valid_to_invalid_transition_publishes_typed_degradation_once() {
        let (sender, receiver) = bounded(4);
        let sink = EngineEventSink::reliable(sender);
        let mut previous = None;
        for flags in [COMBAT_CLOCK_PAUSE_VALID, 0, 0] {
            publish_combat_clock_health(
                &sink,
                &mut previous,
                combat_clock_sample_health(flags, false),
            )
            .expect("health transition should publish");
        }

        let health = receiver
            .try_iter()
            .filter_map(|event| match event {
                EngineEvent::CombatClockHealth(health) => Some(health),
                _ => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(
            health,
            vec![
                CombatClockRuntimeHealth::Available,
                CombatClockRuntimeHealth::DataUnavailable,
            ]
        );
    }

    #[test]
    fn successful_repeated_combat_clock_snapshot_recovers_transient_degradation() {
        let (sender, receiver) = bounded(2);
        let sink = EngineEventSink::reliable(sender);
        let mut previous = None;
        publish_combat_clock_health(
            &sink,
            &mut previous,
            CombatClockRuntimeHealth::ProviderUnavailable,
        )
        .expect("publish transient degradation");

        publish_combat_clock_snapshot_health(
            &sink,
            &mut previous,
            &[CombatClockTransitionSnapshot {
                sequence: 7,
                timestamp_100ns: 133_000_000_000_000_000,
                pause_type_mask: 0,
                reserved_value: 0,
                state_flags: COMBAT_CLOCK_PAUSE_VALID,
            }],
        )
        .expect("repeated authoritative snapshot restores health");

        let health = receiver
            .try_iter()
            .filter_map(|event| match event {
                EngineEvent::CombatClockHealth(health) => Some(health),
                _ => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(
            health,
            vec![
                CombatClockRuntimeHealth::ProviderUnavailable,
                CombatClockRuntimeHealth::Available,
            ]
        );
    }

    #[test]
    fn transient_provider_failure_requires_a_stable_threshold() {
        let mut consecutive = 0;
        for _ in 1..COMBAT_CLOCK_PROVIDER_FAILURE_THRESHOLD {
            assert_eq!(
                stable_combat_clock_error_health(
                    CombatClockQueryError::ProviderUnavailable,
                    &mut consecutive,
                ),
                None
            );
        }
        assert_eq!(
            stable_combat_clock_error_health(
                CombatClockQueryError::ProviderUnavailable,
                &mut consecutive,
            ),
            Some(CombatClockRuntimeHealth::ProviderUnavailable)
        );
        assert_eq!(
            stable_combat_clock_error_health(CombatClockQueryError::ModDisabled, &mut consecutive),
            Some(CombatClockRuntimeHealth::ModDisabled)
        );
        assert_eq!(consecutive, 0);
    }

    #[test]
    fn pcapng_import_validates_file_and_frame_budgets_before_allocation() {
        let directory = std::env::temp_dir().join(format!(
            "nte-pcapng-budget-test-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("clock")
                .as_nanos()
        ));
        std::fs::create_dir_all(&directory).expect("create pcapng budget fixture directory");
        struct Cleanup(PathBuf);
        impl Drop for Cleanup {
            fn drop(&mut self) {
                let _ = std::fs::remove_dir_all(&self.0);
            }
        }
        let _cleanup = Cleanup(directory.clone());

        let file = directory.join("capture.pcapng");
        std::fs::write(&file, b"12345678").expect("write pcapng fixture");
        assert!(validate_pcapng_import_with_limit(&file, 8).is_ok());
        assert!(matches!(
            validate_pcapng_import_with_limit(&file, 7),
            Err(PcapngImportError::TooLarge { size: 8, limit: 7 })
        ));
        assert!(matches!(
            validate_pcapng_import_with_limit(&directory, 8),
            Err(PcapngImportError::NotAFile)
        ));

        let mut packet_count = 0;
        let mut packet_bytes = 0;
        assert!(matches!(
            account_pcapng_frame(
                CAPTURE_SNAPLEN as usize + 1,
                &mut packet_count,
                &mut packet_bytes
            ),
            Err(PcapngImportError::FrameTooLarge { .. })
        ));

        packet_count = MAX_PCAPNG_IMPORT_PACKETS;
        assert!(matches!(
            account_pcapng_frame(0, &mut packet_count, &mut packet_bytes),
            Err(PcapngImportError::TooManyPackets { .. })
        ));

        packet_count = 0;
        packet_bytes = MAX_PCAPNG_IMPORT_PACKET_BYTES;
        assert!(matches!(
            account_pcapng_frame(1, &mut packet_count, &mut packet_bytes),
            Err(PcapngImportError::PacketBytesExceeded { .. })
        ));

        let mut interface_count = 0;
        assert!(account_pcapng_interface(CAPTURE_SNAPLEN, &mut interface_count).is_ok());
        assert!(matches!(
            account_pcapng_interface(0, &mut interface_count),
            Err(PcapngImportError::SnaplenTooLarge { snaplen: 0, .. })
        ));
        interface_count = MAX_PCAPNG_IMPORT_INTERFACES;
        assert!(matches!(
            account_pcapng_interface(CAPTURE_SNAPLEN, &mut interface_count),
            Err(PcapngImportError::TooManyInterfaces { .. })
        ));

        let exact_limit_flag = Arc::new(AtomicBool::new(false));
        let mut exact_limit = PcapngImportReader::new(
            std::io::Cursor::new(b"12345678"),
            8,
            exact_limit_flag.clone(),
        );
        let mut bytes = Vec::new();
        exact_limit
            .read_to_end(&mut bytes)
            .expect("a file exactly at the limit remains readable");
        assert_eq!(bytes, b"12345678");
        assert!(!exact_limit_flag.load(Ordering::Relaxed));

        let grown_file_flag = Arc::new(AtomicBool::new(false));
        let mut grown_file = PcapngImportReader::new(
            std::io::Cursor::new(b"123456789"),
            8,
            grown_file_flag.clone(),
        );
        let error = grown_file
            .read_to_end(&mut Vec::new())
            .expect_err("growth beyond the validated budget must stop the reader");
        assert_eq!(error.kind(), std::io::ErrorKind::InvalidData);
        assert!(grown_file_flag.load(Ordering::Relaxed));
        assert!(matches!(
            map_pcapng_reader_error(PcapError::IoError(error), &grown_file_flag),
            PcapngImportError::TooLarge {
                size,
                limit: MAX_PCAPNG_IMPORT_BYTES
            } if size == MAX_PCAPNG_IMPORT_BYTES + 1
        ));
    }

    #[test]
    fn capture_json_import_validates_size_and_structure_before_use() {
        let directory =
            std::env::temp_dir().join(format!("nte-capture-import-test-{}", std::process::id()));
        std::fs::create_dir_all(&directory).expect("create test directory");
        struct Cleanup(std::path::PathBuf);
        impl Drop for Cleanup {
            fn drop(&mut self) {
                let _ = std::fs::remove_dir_all(&self.0);
            }
        }
        let _cleanup = Cleanup(directory.clone());

        let within = directory.join("within.json");
        std::fs::write(&within, b"{\"version\":1,\"hits\":[],\"packets\":[]}")
            .expect("write small fixture");

        assert!(matches!(
            validate_capture_json_import_with_limit(&directory, 1 << 20),
            Err(CaptureImportError::NotAFile)
        ));
        assert!(matches!(
            validate_capture_json_import_with_limit(&directory.join("missing.json"), 1 << 20),
            Err(CaptureImportError::Io(_))
        ));

        let size = std::fs::metadata(&within).expect("fixture metadata").len();
        assert!(
            validate_capture_json_import_with_limit(&within, size).is_ok(),
            "a file exactly at the limit is accepted"
        );
        assert!(matches!(
            validate_capture_json_import_with_limit(&within, size - 1),
            Err(CaptureImportError::TooLarge { .. })
        ));
        assert!(
            validate_capture_json_import(&within).is_ok(),
            "production entry point accepts small fixtures"
        );

        let document: CaptureExportDocument = serde_json::from_value(serde_json::json!({
            "version": 1,
            "party": [{}],
            "abyss": {
                "first_half": { "party": [{}] },
                "second_half": { "party": [{}] }
            },
            "empty_curtain": [{
                "id": { "solt": 1, "serial": 2 },
                "item_id": "item",
                "level": 1,
                "main_stats": [{ "property": "attack", "value": 1.0 }],
                "sub_stats": [],
                "locked": false
            }],
            "empty_curtain_characters": [{
                "net_id": { "solt": 3, "serial": 4 },
                "character_id": 1
            }],
            "hits": [{
                "timestamp_unix": 1,
                "char_id": 1,
                "char_name": "a",
                "damage": 1.0,
                "target_context": ["boss"]
            }],
            "packets": [{
                "timestamp_unix": 1,
                "source": "a",
                "destination": "b",
                "declared_ids": [1]
            }],
            "time_stop_events": [{
                "GamePauseStarted": { "timestamp": 1.0, "pause_type_mask": 4 }
            }]
        }))
        .expect("bounded export parses");
        let at_limit = CaptureImportStructureLimits {
            hits: 1,
            packets: 1,
            party_rows: 1,
            abyss_party_rows: 1,
            empty_curtain_items: 1,
            empty_curtain_characters: 1,
            time_stop_events: 1,
            item_stats: 1,
            target_context: 1,
            declared_ids: 1,
        };
        assert!(
            validate_capture_export_structure_with_limits(&document, at_limit).is_ok(),
            "structure at the limit is accepted"
        );
        let mut exact_text_limit = document.clone();
        exact_text_limit.hits[0].char_name =
            "a".repeat(MAX_CAPTURE_JSON_IMPORT_CHARACTER_NAME_BYTES);
        assert!(
            validate_capture_export_structure_with_limits(&exact_text_limit, at_limit).is_ok(),
            "a string exactly at its UTF-8 byte limit is accepted"
        );
        let mut oversized_text = document.clone();
        oversized_text.hits[0].char_name = "界".repeat(43);
        assert!(matches!(
            validate_capture_export_structure_with_limits(&oversized_text, at_limit),
            Err(CaptureImportError::FieldTooLarge {
                field: "hits[].char_name",
                size: 129,
                limit: MAX_CAPTURE_JSON_IMPORT_CHARACTER_NAME_BYTES,
            })
        ));
        let mut oversized_item = document.clone();
        oversized_item.empty_curtain[0].item_id =
            "i".repeat(MAX_CAPTURE_JSON_IMPORT_LABEL_BYTES + 1);
        assert!(matches!(
            validate_capture_export_structure_with_limits(&oversized_item, at_limit),
            Err(CaptureImportError::FieldTooLarge {
                field: "empty_curtain[].item_id",
                ..
            })
        ));
        let mut invalid_timestamp = document.clone();
        invalid_timestamp.hits[0].timestamp_unix = MAX_CAPTURE_JSON_IMPORT_TIMESTAMP + 1.0;
        assert!(matches!(
            validate_capture_export_structure_with_limits(&invalid_timestamp, at_limit),
            Err(CaptureImportError::InvalidNumber {
                field: "hits[].timestamp_unix"
            })
        ));
        let mut invalid_damage = document.clone();
        invalid_damage.hits[0].damage = -1.0;
        assert!(matches!(
            validate_capture_export_structure_with_limits(&invalid_damage, at_limit),
            Err(CaptureImportError::InvalidNumber {
                field: "hits[].damage"
            })
        ));
        let mut limits = at_limit;
        limits.hits = 0;
        assert!(matches!(
            validate_capture_export_structure_with_limits(&document, limits),
            Err(CaptureImportError::TooManyHits { .. })
        ));
        let mut limits = at_limit;
        limits.packets = 0;
        assert!(matches!(
            validate_capture_export_structure_with_limits(&document, limits),
            Err(CaptureImportError::TooManyPackets { .. })
        ));
        for (field, limits) in [
            (
                "party",
                CaptureImportStructureLimits {
                    party_rows: 0,
                    ..at_limit
                },
            ),
            (
                "abyss.first_half.party",
                CaptureImportStructureLimits {
                    abyss_party_rows: 0,
                    ..at_limit
                },
            ),
            (
                "empty_curtain",
                CaptureImportStructureLimits {
                    empty_curtain_items: 0,
                    ..at_limit
                },
            ),
            (
                "empty_curtain_characters",
                CaptureImportStructureLimits {
                    empty_curtain_characters: 0,
                    ..at_limit
                },
            ),
            (
                "time_stop_events",
                CaptureImportStructureLimits {
                    time_stop_events: 0,
                    ..at_limit
                },
            ),
            (
                "empty_curtain[].main_stats",
                CaptureImportStructureLimits {
                    item_stats: 0,
                    ..at_limit
                },
            ),
            (
                "hits[].target_context",
                CaptureImportStructureLimits {
                    target_context: 0,
                    ..at_limit
                },
            ),
            (
                "packets[].declared_ids",
                CaptureImportStructureLimits {
                    declared_ids: 0,
                    ..at_limit
                },
            ),
        ] {
            assert!(matches!(
                validate_capture_export_structure_with_limits(&document, limits),
                Err(CaptureImportError::TooManyRecords {
                    field: rejected,
                    ..
                }) if rejected == field
            ));
        }
    }

    #[test]
    fn capture_json_preflight_returns_typed_boundary_errors() {
        let directory = std::env::temp_dir().join(format!(
            "nte-capture-preflight-test-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("clock")
                .as_nanos()
        ));
        std::fs::create_dir_all(&directory).expect("create preflight fixture directory");
        struct Cleanup(PathBuf);
        impl Drop for Cleanup {
            fn drop(&mut self) {
                let _ = std::fs::remove_dir_all(&self.0);
            }
        }
        let _cleanup = Cleanup(directory.clone());

        let exact = directory.join("exact.json");
        std::fs::write(&exact, b"{}").expect("write exact-limit fixture");
        assert!(
            prepare_capture_json_replay_with_limits(&exact, 2, CAPTURE_IMPORT_STRUCTURE_LIMITS)
                .is_ok(),
            "a valid document exactly at the byte limit must be prepared"
        );

        let limit_plus_one = directory.join("limit-plus-one.json");
        std::fs::write(&limit_plus_one, b"{}\n").expect("write oversized fixture");
        assert!(matches!(
            prepare_capture_json_replay_with_limits(
                &limit_plus_one,
                2,
                CAPTURE_IMPORT_STRUCTURE_LIMITS
            ),
            Err(CaptureImportError::TooLarge { size: 3, limit: 2 })
        ));

        let malformed = directory.join("malformed.json");
        std::fs::write(&malformed, b"{").expect("write malformed fixture");
        assert!(matches!(
            prepare_capture_json_replay(&malformed),
            Err(CaptureImportError::InvalidFormat)
        ));

        let invalid_utf8 = directory.join("invalid-utf8.json");
        std::fs::write(&invalid_utf8, [0xff]).expect("write invalid UTF-8 fixture");
        assert!(matches!(
            prepare_capture_json_replay(&invalid_utf8),
            Err(CaptureImportError::InvalidUtf8)
        ));

        let unsupported = directory.join("unsupported.json");
        std::fs::write(&unsupported, br#"{"version":2}"#)
            .expect("write unsupported-version fixture");
        assert!(matches!(
            prepare_capture_json_replay(&unsupported),
            Err(CaptureImportError::UnsupportedVersion {
                found: 2,
                expected: CAPTURE_EXPORT_VERSION
            })
        ));

        let too_many = directory.join("too-many.json");
        let hit = serde_json::json!({
            "timestamp_unix": 1.0,
            "char_id": 1,
            "char_name": "Fixture",
            "damage": 1.0
        });
        std::fs::write(
            &too_many,
            serde_json::to_vec(&serde_json::json!({
                "version": CAPTURE_EXPORT_VERSION,
                "hits": [hit.clone(), hit],
                "packets": []
            }))
            .expect("serialize too-many fixture"),
        )
        .expect("write too-many fixture");
        let one_hit = CaptureImportStructureLimits {
            hits: 1,
            ..CAPTURE_IMPORT_STRUCTURE_LIMITS
        };
        assert!(matches!(
            prepare_capture_json_replay_with_limits(&too_many, 1 << 20, one_hit),
            Err(CaptureImportError::TooManyHits { count: 2, limit: 1 })
        ));

        let invalid_payload = directory.join("invalid-payload.json");
        std::fs::write(
            &invalid_payload,
            br#"{"version":1,"hits":[],"packets":[{"timestamp_unix":1,"source":"a","destination":"b","payload_hex":"xyz"}]}"#,
        )
        .expect("write invalid payload fixture");
        assert!(matches!(
            prepare_capture_json_replay(&invalid_payload),
            Err(CaptureImportError::InvalidFormat)
        ));
    }

    #[test]
    fn capture_frame_queue_applies_backpressure_without_dropping_frames() {
        let (sender, receiver) = capture_frame_queue(1, 2);
        forward_capture_frame(&sender, CaptureFrame::new(vec![1], 1.0)).unwrap();
        let (completed_sender, completed_receiver) = bounded(1);
        let blocked_sender = sender.clone();
        let blocked = thread::spawn(move || {
            forward_capture_frame(&blocked_sender, CaptureFrame::new(vec![2], 2.0)).unwrap();
            completed_sender.send(()).unwrap();
        });

        assert!(
            completed_receiver
                .recv_timeout(Duration::from_millis(20))
                .is_err()
        );
        assert_eq!(receiver.recv().unwrap().timestamp, 1.0);
        completed_receiver
            .recv_timeout(Duration::from_secs(1))
            .unwrap();
        assert_eq!(receiver.recv().unwrap().timestamp, 2.0);
        blocked.join().unwrap();
    }

    #[test]
    fn capture_frame_queue_applies_byte_high_water_to_in_flight_frames() {
        let (sender, receiver) = capture_frame_queue(4, 5);
        forward_capture_frame(&sender, CaptureFrame::new(vec![1; 3], 1.0)).unwrap();
        forward_capture_frame(&sender, CaptureFrame::new(vec![2; 2], 2.0)).unwrap();
        assert_eq!(sender.byte_high_water_mark(), 5);

        let (completed_sender, completed_receiver) = bounded(1);
        let blocked_sender = sender.clone();
        let blocked = thread::spawn(move || {
            let result = forward_capture_frame(&blocked_sender, CaptureFrame::new(vec![3], 3.0));
            completed_sender.send(result).unwrap();
        });

        let first = receiver.recv().unwrap();
        assert_eq!(first.timestamp, 1.0);
        assert!(
            completed_receiver
                .recv_timeout(Duration::from_millis(20))
                .is_err(),
            "a frame being parsed must still count against the byte budget"
        );
        drop(first);
        completed_receiver
            .recv_timeout(Duration::from_secs(1))
            .unwrap()
            .unwrap();
        assert_eq!(receiver.recv().unwrap().timestamp, 2.0);
        assert_eq!(receiver.recv().unwrap().timestamp, 3.0);
        assert_eq!(sender.byte_high_water_mark(), 5);
        blocked.join().unwrap();
    }

    #[test]
    fn capture_frame_queue_rejects_oversize_and_wakes_on_disconnect() {
        let (sender, receiver) = capture_frame_queue(2, 2);
        let oversize =
            forward_capture_frame(&sender, CaptureFrame::new(vec![0; 3], 1.0)).unwrap_err();
        assert!(oversize.contains("exceeds parser queue byte budget"));

        forward_capture_frame(&sender, CaptureFrame::new(vec![1; 2], 2.0)).unwrap();
        let (completed_sender, completed_receiver) = bounded(1);
        let blocked_sender = sender.clone();
        let blocked = thread::spawn(move || {
            completed_sender
                .send(forward_capture_frame(
                    &blocked_sender,
                    CaptureFrame::new(vec![2], 3.0),
                ))
                .unwrap();
        });
        drop(receiver);
        let error = completed_receiver
            .recv_timeout(Duration::from_secs(1))
            .unwrap()
            .unwrap_err();
        assert!(error.contains("capture parser thread stopped unexpectedly"));
        blocked.join().unwrap();
    }

    #[test]
    fn combat_clock_block_round_trips_authoritative_pause_state() {
        let transition = CombatClockTransitionSnapshot {
            sequence: 9,
            timestamp_100ns: FILETIME_UNIX_EPOCH_100NS + 87 * FILETIME_TICKS_PER_SECOND,
            pause_type_mask: 1 << 3,
            reserved_value: 0,
            state_flags: COMBAT_CLOCK_PAUSE_VALID,
        };
        let payload = encode_combat_clock_block(&transition);

        assert_eq!(decode_combat_clock_block(&payload), Some(transition));
        assert!(decode_combat_clock_block(&payload[..39]).is_none());

        let mut invalid_reserved = payload;
        invalid_reserved[28..32].copy_from_slice(&1_i32.to_le_bytes());
        assert!(decode_combat_clock_block(&invalid_reserved).is_none());

        let mut invalid_flags = payload;
        invalid_flags[32..36].copy_from_slice(&2_u32.to_le_bytes());
        assert!(decode_combat_clock_block(&invalid_flags).is_none());
    }

    #[test]
    fn pcapng_replay_with_only_invalid_clock_sample_is_not_marked_recorded() {
        let path = std::env::temp_dir().join(format!(
            "nte-invalid-clock-replay-{}-{}.pcapng",
            std::process::id(),
            current_filetime_100ns()
        ));
        let transition = CombatClockTransitionSnapshot {
            sequence: 1,
            timestamp_100ns: FILETIME_UNIX_EPOCH_100NS + FILETIME_TICKS_PER_SECOND,
            pause_type_mask: 0,
            reserved_value: 0,
            state_flags: 0,
        };
        let payload = encode_combat_clock_block(&transition);
        let file = File::create(&path).expect("pcapng fixture should be created");
        let mut writer =
            PcapNgWriter::new(BufWriter::new(file)).expect("pcapng writer should initialize");
        writer
            .write_pcapng_block(UnknownBlock::new(NTE_COMBAT_CLOCK_BLOCK_TYPE, 0, &payload))
            .expect("clock block should be written");
        writer
            .get_mut()
            .flush()
            .expect("clock fixture should flush");
        drop(writer);

        let (sender, receiver) = unbounded();
        let handle = import_pcapng(
            path.clone(),
            CaptureResources {
                characters: Arc::new(HashMap::new()),
                ability_catalog: Arc::new(AbilityCatalog::default()),
            },
            None,
            true,
            false,
            sender,
            Arc::new(AtomicBool::new(false)),
        )
        .expect("pcapng import thread should spawn");
        handle.join().expect("pcapng import thread should finish");
        std::fs::remove_file(path).expect("pcapng fixture should be removable");

        let health = receiver
            .try_iter()
            .filter_map(|event| match event {
                EngineEvent::CombatClockHealth(health) => Some(health),
                _ => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(health, vec![CombatClockRuntimeHealth::DataUnavailable]);
        assert!(!health.contains(&CombatClockRuntimeHealth::Recorded));
    }

    #[test]
    fn mod_script_block_round_trips_bounded_event_data() {
        let event = ModScriptEvent {
            sequence: 17,
            timestamp_100ns: FILETIME_UNIX_EPOCH_100NS + 42 * FILETIME_TICKS_PER_SECOND,
            mod_id: "enemy-telemetry".to_owned(),
            phase: ModScriptEventPhase::Postprocess,
            name: "enemy.hit_target".to_owned(),
            values: vec![0x1234, 0x4d88_7b49_05d5_dbaf, 80],
            enemy_identity: None,
        };
        let payload =
            encode_mod_script_block(&event).expect("bounded ModScript event should encode");

        assert_eq!(decode_mod_script_block(&payload), Some(event));
        assert!(decode_mod_script_block(&payload[..119]).is_none());

        let mut invalid_phase = payload;
        invalid_phase[24] = 3;
        assert!(decode_mod_script_block(&invalid_phase).is_none());

        let mut invalid_timestamp = payload;
        invalid_timestamp[16..24].copy_from_slice(&(FILETIME_UNIX_EPOCH_100NS - 1).to_le_bytes());
        assert!(decode_mod_script_block(&invalid_timestamp).is_none());

        let mut invalid_padding = payload;
        invalid_padding[119] = 1;
        assert!(decode_mod_script_block(&invalid_padding).is_none());
    }

    #[test]
    fn pcapng_import_restores_mod_script_enemy_identity() {
        let path = std::env::temp_dir().join(format!(
            "nte-mod-script-replay-{}-{}.pcapng",
            std::process::id(),
            Local::now()
                .timestamp_nanos_opt()
                .expect("current local time must fit in nanoseconds")
        ));
        let event = ModScriptEvent {
            sequence: 23,
            timestamp_100ns: FILETIME_UNIX_EPOCH_100NS + 84 * FILETIME_TICKS_PER_SECOND,
            mod_id: "enemy-telemetry".to_owned(),
            phase: ModScriptEventPhase::Postprocess,
            name: "enemy.hit_target".to_owned(),
            values: vec![0x5678, 0x4d88_7b49_05d5_dbaf, 0],
            enemy_identity: None,
        };
        let payload = encode_mod_script_block(&event).expect("ModScript event should encode");
        let file = File::create(&path).expect("pcapng fixture should be created");
        let mut writer =
            PcapNgWriter::new(BufWriter::new(file)).expect("pcapng writer should initialize");
        writer
            .write_pcapng_block(UnknownBlock::new(NTE_MOD_SCRIPT_BLOCK_TYPE, 0, &payload))
            .expect("ModScript block should be written");
        writer
            .get_mut()
            .flush()
            .expect("pcapng fixture should flush");
        drop(writer);

        let (sender, receiver) = unbounded();
        let handle = import_pcapng(
            path.clone(),
            CaptureResources {
                characters: Arc::new(HashMap::new()),
                ability_catalog: Arc::new(AbilityCatalog::default()),
            },
            None,
            true,
            true,
            sender,
            Arc::new(AtomicBool::new(false)),
        )
        .expect("pcapng import thread should spawn");
        handle.join().expect("pcapng import thread should finish");
        std::fs::remove_file(path).expect("pcapng fixture should be removable");

        let restored = receiver
            .try_iter()
            .find_map(|event| match event {
                EngineEvent::ModScript(event) => Some(event),
                _ => None,
            })
            .expect("pcapng import should restore the ModScript event");
        assert_eq!(restored.sequence, 23);
        assert_eq!(restored.name, "enemy.hit_target");
        assert_eq!(
            restored
                .enemy_identity
                .as_ref()
                .map(|identity| identity.name_zh.as_str()),
            Some("随心泥")
        );
    }

    #[test]
    fn capture_export_migrates_only_legacy_authoritative_pause_intervals() {
        let document: CaptureExportDocument = serde_json::from_str(
            r#"{
                "time_stop_events": [
                    {"GamePause":{"start_timestamp":10.0,"end_timestamp":13.5,"pause_type_mask":4}},
                    {"GameTimerSample":{"timestamp":11.0,"remaining_seconds":87}},
                    {"UltraAnimation":{"timestamp":11.0,"char_id":1010,"ability_id":"GA_Test","duration_seconds":2.0}},
                    {"ExtraStart":{"timestamp":11.0,"reason":"legacy"}},
                    {"ExtraEnd":{"timestamp":12.0,"reason":"legacy"}}
                ]
            }"#,
        )
        .expect("legacy capture export should migrate at the JSON boundary");

        assert_eq!(
            document.time_stop_events,
            vec![
                TimeStopEvent::GamePauseStarted {
                    timestamp: 10.0,
                    pause_type_mask: 1 << 2,
                },
                TimeStopEvent::GamePauseEnded {
                    timestamp: 13.5,
                    pause_type_mask: 1 << 2,
                },
            ]
        );
    }

    #[test]
    fn capture_export_rejects_invalid_saved_pause_state() {
        let invalid_mask = r#"{
            "time_stop_events": [
                {"GamePauseStarted":{"timestamp":10.0,"pause_type_mask":2}}
            ]
        }"#;

        assert!(serde_json::from_str::<CaptureExportDocument>(invalid_mask).is_err());
    }

    #[test]
    fn game_pause_tracker_unions_nested_authoritative_pause_types() {
        let mut tracker = GamePauseIntervalTracker::default();
        assert_eq!(
            tracker.apply_transition(10.0, 1 << 2),
            Some(TimeStopEvent::GamePauseStarted {
                timestamp: 10.0,
                pause_type_mask: 1 << 2,
            })
        );
        assert_eq!(tracker.apply_transition(11.0, (1 << 2) | (1 << 4)), None);
        assert_eq!(tracker.apply_transition(12.0, 1 << 4), None);
        assert_eq!(
            tracker.apply_transition(13.0, 0),
            Some(TimeStopEvent::GamePauseEnded {
                timestamp: 13.0,
                pause_type_mask: (1 << 2) | (1 << 4),
            })
        );
        assert_eq!(tracker.apply_transition(14.0, 0), None);
    }

    fn debug_packet(note: &str) -> PacketDebug {
        PacketDebug {
            timestamp: 1.0,
            source: "127.0.0.1:1".to_owned(),
            destination: "127.0.0.1:2".to_owned(),
            direction: "C2S".to_owned(),
            payload_len: 0,
            declared_ids: Vec::new(),
            parsed_hits: 0,
            note: note.to_owned(),
            payload_preview: String::new(),
            payload_hex: String::new(),
            decoded_text: String::new(),
        }
    }

    #[test]
    fn wire_shape_note_preserves_exact_channel_but_excludes_sequence_and_payload() {
        let first = BunchPacket {
            packet_info_bit_len: 13,
            bunches: vec![crate::engine::protocol::LocatedBunch {
                bit_offset: 13,
                bunch: SingleBunch {
                    prefix: 17,
                    sequence: 5,
                    descriptor: 0x22,
                    partial_flags: 0x09,
                    data_bit_len: 77,
                    data: vec![0; 10],
                },
            }],
        };
        let second = BunchPacket {
            packet_info_bit_len: 13,
            bunches: vec![crate::engine::protocol::LocatedBunch {
                bit_offset: 13,
                bunch: SingleBunch {
                    prefix: 999,
                    sequence: 711,
                    descriptor: 0x22,
                    partial_flags: 0x09,
                    data_bit_len: 77,
                    data: vec![0xff; 10],
                },
            }],
        };

        let first_note = bunch_wire_shape_note("S2C", &first, 1);
        let second_note = bunch_wire_shape_note("S2C", &second, 1);

        assert_ne!(first_note, second_note);
        assert_eq!(
            first_note,
            "Bunch 链 1 个，packet-info 13 bit，跨包完成 1 个载荷；WireShape v2 S2C info=13 [c17/d22/f9/b77]"
        );
        assert_eq!(
            second_note,
            "Bunch 链 1 个，packet-info 13 bit，跨包完成 1 个载荷；WireShape v2 S2C info=13 [c999/d22/f9/b77]"
        );
    }

    fn receive_observed_debug_packet(
        receiver: &Receiver<EngineEvent>,
        expected_hits: usize,
    ) -> Box<PacketDebug> {
        assert!(matches!(
            receiver
                .try_recv()
                .expect("packet observation should be emitted"),
            EngineEvent::PacketObservation(PacketObservation { parsed_hits })
                if parsed_hits == expected_hits
        ));
        let EngineEvent::Packet(packet) = receiver
            .try_recv()
            .expect("debug packet payload should be emitted")
        else {
            panic!("expected debug packet payload");
        };
        packet
    }

    #[test]
    fn split_event_sink_bounds_debug_without_dropping_reliable_events() {
        let (reliable_sender, reliable_receiver) = bounded(2);
        let (debug_sender, debug_receiver) = bounded(1);
        let sink = EngineEventSink::split(reliable_sender, debug_sender);

        sink.send(EngineEvent::EmptyCurtain(Vec::new())).unwrap();
        sink.send(EngineEvent::PacketObservation(PacketObservation {
            parsed_hits: 3,
        }))
        .unwrap();
        sink.send(EngineEvent::Packet(Box::new(debug_packet("kept"))))
            .unwrap();
        sink.send(EngineEvent::Packet(Box::new(debug_packet("dropped"))))
            .unwrap();

        assert!(matches!(
            reliable_receiver.try_recv().unwrap(),
            EngineEvent::EmptyCurtain(_)
        ));
        assert!(matches!(
            reliable_receiver.try_recv().unwrap(),
            EngineEvent::PacketObservation(PacketObservation { parsed_hits: 3 })
        ));
        let EngineEvent::Packet(packet) = debug_receiver.try_recv().unwrap() else {
            panic!("debug lane must contain the retained packet");
        };
        assert_eq!(packet.note, "kept");
        assert_eq!(sink.take_dropped_debug_packets(), 1);
        assert_eq!(sink.take_dropped_debug_packets(), 0);
    }

    #[test]
    fn full_debug_emits_observation_when_debug_payload_is_dropped() {
        let (reliable_sender, reliable_receiver) = bounded(1);
        let (debug_sender, debug_receiver) = bounded(1);
        let sink = EngineEventSink::split(reliable_sender, debug_sender);
        sink.send(EngineEvent::Packet(Box::new(debug_packet("retained"))))
            .unwrap();
        let mut dropped = debug_packet("dropped");
        dropped.parsed_hits = 3;

        send_packet_events(&sink, dropped).unwrap();

        assert!(matches!(
            reliable_receiver.try_recv().unwrap(),
            EngineEvent::PacketObservation(PacketObservation { parsed_hits: 3 })
        ));
        let EngineEvent::Packet(packet) = debug_receiver.try_recv().unwrap() else {
            panic!("debug lane must retain its existing packet");
        };
        assert_eq!(packet.note, "retained");
        assert_eq!(sink.take_dropped_debug_packets(), 1);
    }

    #[test]
    fn reliable_only_event_sink_preserves_debug_events_for_cli_consumers() {
        let (sender, receiver) = unbounded();
        let sink = EngineEventSink::reliable(sender);

        sink.send(EngineEvent::Packet(Box::new(debug_packet("cli"))))
            .unwrap();

        assert!(matches!(
            receiver.try_recv().unwrap(),
            EngineEvent::Packet(_)
        ));
        assert_eq!(sink.take_dropped_debug_packets(), 0);
    }

    #[test]
    fn stop_with_drain_releases_a_blocked_reliable_sender() {
        let (sender, receiver) = bounded(1);
        let sink = EngineEventSink::reliable(sender);
        sink.send(EngineEvent::Status("first".to_owned())).unwrap();
        let worker_sink = sink.clone();
        let worker = thread::spawn(move || {
            worker_sink
                .send(EngineEvent::Status("second".to_owned()))
                .unwrap();
        });
        let mut handle = CaptureHandle {
            stop: Arc::new(AtomicBool::new(false)),
            thread: Some(worker),
            raw_capture: RawCaptureBuffer::new(None),
        };
        let mut statuses = Vec::new();

        handle.stop_with_drain(|| {
            while let Ok(EngineEvent::Status(status)) = receiver.try_recv() {
                statuses.push(status);
            }
        });

        assert_eq!(statuses, ["first", "second"]);
    }

    fn inventory_bunch_on(prefix: u16, sequence: u16, partial_flags: u8, data: u8) -> SingleBunch {
        SingleBunch {
            prefix,
            sequence,
            descriptor: 0xcc,
            partial_flags,
            data_bit_len: 8,
            data: vec![data],
        }
    }

    fn inventory_bunch(sequence: u16, partial_flags: u8, data: u8) -> SingleBunch {
        inventory_bunch_on(7, sequence, partial_flags, data)
    }

    #[derive(Default)]
    struct InventoryTestBitWriter {
        data: Vec<u8>,
        bit_len: usize,
    }

    impl InventoryTestBitWriter {
        fn push_bits(&mut self, value: u64, count: usize) {
            let new_bit_len = self.bit_len + count;
            self.data.resize(new_bit_len.div_ceil(8), 0);
            for index in 0..count {
                let target = self.bit_len + index;
                self.data[target / 8] |= (((value >> index) & 1) as u8) << (target % 8);
            }
            self.bit_len = new_bit_len;
        }

        fn push_bool(&mut self, value: bool) {
            self.push_bits(u64::from(value), 1);
        }

        fn push_u16(&mut self, value: u16) {
            self.push_bits(u64::from(value), 16);
        }

        fn push_u32(&mut self, value: u32) {
            self.push_bits(u64::from(value), 32);
        }

        fn push_i32(&mut self, value: i32) {
            self.push_u32(value as u32);
        }

        fn push_i64(&mut self, value: i64) {
            self.push_bits(value as u64, 64);
        }

        fn push_f32(&mut self, value: f32) {
            self.push_u32(value.to_bits());
        }

        fn push_dynamic_name(&mut self, value: &str) {
            self.push_bool(false);
            self.push_i32((value.len() + 1) as i32);
            for byte in value.bytes() {
                self.push_bits(u64::from(byte), 8);
            }
            self.push_bits(0, 8);
            self.push_u32(0);
        }
    }

    fn push_character_owner_record(
        record: &mut InventoryTestBitWriter,
        character_id: u32,
        net_id: HtItemNetId,
    ) {
        record.push_dynamic_name(&character_id.to_string());
        record.push_u32(net_id.solt);
        record.push_u32(net_id.serial);
        record.push_i64(1);
        record.push_i32(0);
        record.push_i64(1);
        record.push_u16(1);
        record.push_i32(80);
        record.push_i32(6);
        record.push_u32(100);
        record.push_u32(200);
        record.push_i32(6);
        for value in [1.0, 20_000.0, 3.0, 120.0, 80.0, 1_000.0, 100.0] {
            record.push_f32(value);
        }
        record.push_bool(false);
        record.push_i32(0);
        record.push_u16(5);
    }

    fn inventory_fragment_packet(
        record: InventoryTestBitWriter,
        sequence: u16,
        partial_flags: u8,
    ) -> SequencedPacket {
        let mut payload = InventoryTestBitWriter::default();
        payload.push_bits(4122, 13);
        payload.push_bits(u64::from(sequence), 10);
        payload.push_bits(u64::from(0xcc0_u16 | u16::from(partial_flags)), 12);
        payload.push_bits(record.bit_len as u64, 13);
        for index in 0..record.bit_len {
            payload.push_bits(u64::from((record.data[index / 8] >> (index % 8)) & 1), 1);
        }
        payload.push_bool(true);
        SequencedPacket {
            handler_prefix: 0,
            mode: 0,
            header_flags: 0,
            acknowledged_packet_id: 0,
            packet_id: 0,
            acknowledgment_history: 0,
            packet_flags: 0,
            payload_bit_len: payload.bit_len,
            payload: payload.data,
        }
    }

    fn inventory_stream_packet(record: InventoryTestBitWriter) -> SequencedPacket {
        inventory_fragment_packet(record, 87, 0x0d)
    }

    fn character_owner_packet(character_id: u32, net_id: HtItemNetId) -> SequencedPacket {
        let mut record = InventoryTestBitWriter::default();
        push_character_owner_record(&mut record, character_id, net_id);
        inventory_stream_packet(record)
    }

    fn raw_character_owner_packet(character_id: u32, net_id: HtItemNetId) -> SequencedPacket {
        let mut payload = InventoryTestBitWriter::default();
        push_character_owner_record(&mut payload, character_id, net_id);
        SequencedPacket {
            handler_prefix: 0,
            mode: 1,
            header_flags: 0,
            acknowledged_packet_id: 0,
            packet_id: 0,
            acknowledgment_history: 0,
            packet_flags: 0,
            payload_bit_len: payload.bit_len,
            payload: payload.data,
        }
    }

    fn compact_module_placement_packet(
        character_id: u32,
        character_net_id: HtItemNetId,
        item_id: &str,
        id: HtItemNetId,
        row: i32,
        column: i32,
    ) -> SequencedPacket {
        let mut record = InventoryTestBitWriter::default();
        push_character_owner_record(&mut record, character_id, character_net_id);
        for (first_step, cell_row) in [(true, row), (false, row + 1)] {
            record.push_i32(1);
            record.push_dynamic_name(item_id);
            record.push_u32(id.solt);
            record.push_u32(id.serial);
            record.push_bool(first_step);
            record.push_i32(cell_row);
            record.push_i32(column);
            record.push_i32(0);
        }
        inventory_stream_packet(record)
    }

    fn raw_inventory_item_packet(
        catalog: &EquipmentCatalog,
        id: HtItemNetId,
        item_id: &str,
        character_net_id: Option<HtItemNetId>,
        locked: bool,
        discarded: bool,
    ) -> SequencedPacket {
        raw_inventory_item_packet_at_level(
            catalog,
            id,
            item_id,
            character_net_id,
            0,
            locked,
            discarded,
        )
    }

    fn raw_inventory_item_packet_at_level(
        catalog: &EquipmentCatalog,
        id: HtItemNetId,
        item_id: &str,
        character_net_id: Option<HtItemNetId>,
        level: u32,
        locked: bool,
        discarded: bool,
    ) -> SequencedPacket {
        let definition = catalog
            .items
            .get(item_id)
            .expect("test equipment must exist in the catalog");
        let main_property = catalog
            .attributes
            .keys()
            .find(|property| {
                catalog
                    .main_stat_value(definition, property.as_str(), 0)
                    .is_some()
            })
            .expect("test equipment must have a valid main property");
        let sub_property = catalog
            .attributes
            .keys()
            .next()
            .expect("test catalog must have an equipment attribute");

        let mut payload = InventoryTestBitWriter::default();
        payload.push_dynamic_name(item_id);
        payload.push_u32(id.solt);
        payload.push_u32(id.serial);
        payload.push_i64(1);
        payload.push_i32(0);
        payload.push_i64(1);
        payload.push_u16(0);
        payload.push_u16(0);
        payload.push_u16(1);
        payload.push_i32(level as i32);
        let character_net_id = character_net_id.unwrap_or(HtItemNetId { solt: 0, serial: 0 });
        payload.push_u32(character_net_id.solt);
        payload.push_u32(character_net_id.serial);
        payload.push_i32(0);
        payload.push_bool(locked);
        payload.push_bool(discarded);
        payload.push_u16(0);
        payload.push_u16(definition.main_count as u16);
        for _ in 0..definition.main_count {
            payload.push_dynamic_name(main_property);
        }
        payload.push_u16(definition.sub_count as u16);
        for _ in 0..definition.sub_count {
            payload.push_dynamic_name(sub_property);
            payload.push_f32(1.0);
        }
        payload.push_f32(0.0);
        payload.push_bool(false);
        payload.push_i32(0);
        payload.push_i32(0);

        SequencedPacket {
            handler_prefix: 0,
            mode: 1,
            header_flags: 0,
            acknowledged_packet_id: 0,
            packet_id: 0,
            acknowledgment_history: 0,
            packet_flags: 0,
            payload_bit_len: payload.bit_len,
            payload: payload.data,
        }
    }

    fn inventory_item_stream_packet(
        catalog: &EquipmentCatalog,
        id: HtItemNetId,
        item_id: &str,
        character_net_id: HtItemNetId,
    ) -> SequencedPacket {
        let raw =
            raw_inventory_item_packet(catalog, id, item_id, Some(character_net_id), false, false);
        inventory_stream_packet(InventoryTestBitWriter {
            data: raw.payload,
            bit_len: raw.payload_bit_len,
        })
    }

    fn inventory_test_item(character_net_id: Option<HtItemNetId>) -> EmptyCurtainItem {
        EmptyCurtainItem {
            id: HtItemNetId {
                solt: 10,
                serial: 20,
            },
            item_id: "existing-item".to_owned(),
            level: 0,
            main_stats: Vec::new(),
            sub_stats: Vec::new(),
            locked: false,
            discarded: false,
            character_net_id,
            equipped_character_id: None,
            equipped_placement: None,
        }
    }

    fn inventory_equipment_item(
        id: HtItemNetId,
        item_id: &str,
        character_net_id: Option<HtItemNetId>,
        equipped_character_id: Option<u32>,
    ) -> EmptyCurtainItem {
        EmptyCurtainItem {
            id,
            item_id: item_id.to_owned(),
            level: 0,
            main_stats: Vec::new(),
            sub_stats: Vec::new(),
            locked: false,
            discarded: false,
            character_net_id,
            equipped_character_id,
            equipped_placement: None,
        }
    }

    #[test]
    fn inventory_reassembly_replaces_stale_fragments_after_sequence_wrap() {
        let mut state = InventoryConnectionState::default();
        assert!(
            state
                .push_bunches(
                    100,
                    vec![
                        inventory_bunch(1023, 0x09, 0xa1),
                        inventory_bunch(1, 0x0c, 0xa3),
                    ]
                )
                .is_empty()
        );

        let completed = state.push_bunches(
            200,
            vec![
                inventory_bunch(1023, 0x09, 0xb1),
                inventory_bunch(0, 0x08, 0xb2),
                inventory_bunch(1, 0x0c, 0xb3),
            ],
        );

        assert_eq!(completed.len(), 1);
        assert_eq!(completed[0].data, vec![0xb1, 0xb2, 0xb3]);
        assert_eq!(completed[0].bit_len, 24);
    }

    #[test]
    fn inventory_reassembly_does_not_join_a_new_start_to_stale_future_fragments() {
        let mut state = InventoryConnectionState::default();
        assert!(
            state
                .push_bunches(
                    100,
                    vec![
                        inventory_bunch(12, 0x08, 0xa3),
                        inventory_bunch(13, 0x0c, 0xa4),
                    ]
                )
                .is_empty()
        );
        assert!(
            state
                .push_bunches(
                    200,
                    vec![
                        inventory_bunch(10, 0x09, 0xb1),
                        inventory_bunch(11, 0x08, 0xb2),
                    ]
                )
                .is_empty()
        );

        let completed = state.push_bunches(
            201,
            vec![
                inventory_bunch(12, 0x08, 0xb3),
                inventory_bunch(13, 0x0c, 0xb4),
            ],
        );

        assert_eq!(completed.len(), 1);
        assert_eq!(completed[0].data, vec![0xb1, 0xb2, 0xb3, 0xb4]);
        assert_eq!(completed[0].bit_len, 32);
    }

    #[test]
    fn inventory_reassembly_retries_after_a_fragment_is_retransmitted_in_a_newer_packet() {
        let mut state = InventoryConnectionState::default();
        assert!(
            state
                .push_bunches(100, vec![inventory_bunch(12, 0x0c, 0xa3)])
                .is_empty()
        );
        assert!(
            state
                .push_bunches(
                    200,
                    vec![
                        inventory_bunch(10, 0x09, 0xa1),
                        inventory_bunch(11, 0x08, 0xa2),
                    ],
                )
                .is_empty()
        );

        let completed = state.push_bunches(201, vec![inventory_bunch(12, 0x0c, 0xa3)]);

        assert_eq!(completed.len(), 1);
        assert_eq!(completed[0].data, vec![0xa1, 0xa2, 0xa3]);
        assert_eq!(completed[0].bit_len, 24);
    }

    #[test]
    fn inventory_reassembly_keeps_logically_newer_fragments_that_arrive_before_their_start() {
        let mut state = InventoryConnectionState::default();
        assert!(
            state
                .push_bunches(1_867, vec![inventory_bunch(810, 0x08, 0xa2)])
                .is_empty()
        );
        assert!(
            state
                .push_bunches(1_868, vec![inventory_bunch(811, 0x08, 0xa3)])
                .is_empty()
        );
        assert!(
            state
                .push_bunches(1_866, vec![inventory_bunch(809, 0x09, 0xa1)])
                .is_empty()
        );

        let completed = state.push_bunches(1_869, vec![inventory_bunch(812, 0x0c, 0xa4)]);

        assert_eq!(completed.len(), 1);
        assert_eq!(completed[0].data, vec![0xa1, 0xa2, 0xa3, 0xa4]);
        assert_eq!(completed[0].bit_len, 32);
    }

    #[test]
    fn inventory_reassembly_accepts_a_late_retransmitted_start_fragment() {
        let mut state = InventoryConnectionState::default();
        assert!(
            state
                .push_bunches(4_684, vec![inventory_bunch(992, 0x08, 0xa2)])
                .is_empty()
        );
        assert!(
            state
                .push_bunches(4_685, vec![inventory_bunch(993, 0x08, 0xa3)])
                .is_empty()
        );
        assert!(
            state
                .push_bunches(4_686, vec![inventory_bunch(994, 0x0c, 0xa4)])
                .is_empty()
        );

        let completed = state.push_bunches(4_763, vec![inventory_bunch(991, 0x09, 0xa1)]);

        assert_eq!(completed.len(), 1);
        assert_eq!(completed[0].data, vec![0xa1, 0xa2, 0xa3, 0xa4]);
        assert_eq!(completed[0].bit_len, 32);
    }

    #[test]
    fn inventory_reassembly_accepts_transport_packet_id_wrap() {
        let mut state = InventoryConnectionState::default();
        assert!(
            state
                .push_bunches((1 << 14) - 1, vec![inventory_bunch(100, 0x09, 0xa1)],)
                .is_empty()
        );

        let completed = state.push_bunches(0, vec![inventory_bunch(101, 0x0c, 0xa2)]);

        assert_eq!(completed.len(), 1);
        assert_eq!(completed[0].data, vec![0xa1, 0xa2]);
        assert_eq!(completed[0].bit_len, 16);
    }

    #[test]
    fn inventory_reassembly_finishes_a_stream_before_the_next_start_in_the_same_packet() {
        let mut state = InventoryConnectionState::default();
        assert!(
            state
                .push_bunches(
                    1_912,
                    vec![
                        inventory_bunch(821, 0x09, 0xa1),
                        inventory_bunch(822, 0x08, 0xa2),
                        inventory_bunch(823, 0x08, 0xa3),
                    ],
                )
                .is_empty()
        );

        let completed = state.push_bunches(
            1_915,
            vec![
                inventory_bunch(824, 0x0c, 0xa4),
                inventory_bunch(825, 0x09, 0xb1),
            ],
        );

        assert_eq!(completed.len(), 1);
        assert_eq!(completed[0].data, vec![0xa1, 0xa2, 0xa3, 0xa4]);
        assert_eq!(completed[0].bit_len, 32);
        let next = state.push_bunches(1_916, vec![inventory_bunch(826, 0x0c, 0xb2)]);
        assert_eq!(next.len(), 1);
        assert_eq!(next[0].data, vec![0xb1, 0xb2]);
    }

    #[test]
    fn inventory_reassembly_uses_channel_index_across_prefix_flag_changes() {
        let mut state = InventoryConnectionState::default();
        assert!(
            state
                .push_bunches(100, vec![inventory_bunch_on(5146, 483, 0x09, 0xa1)],)
                .is_empty()
        );
        assert!(
            state
                .push_bunches(
                    101,
                    vec![
                        inventory_bunch_on(4122, 484, 0x08, 0xa2),
                        inventory_bunch_on(4122, 485, 0x08, 0xa3),
                        inventory_bunch_on(4122, 486, 0x08, 0xa4),
                    ],
                )
                .is_empty()
        );

        let completed = state.push_bunches(102, vec![inventory_bunch_on(4122, 487, 0x0c, 0xa5)]);

        assert_eq!(
            state
                .bunch_reassembler
                .known_channels()
                .collect::<HashSet<_>>(),
            HashSet::from([26])
        );
        assert_eq!(completed.len(), 1);
        assert_eq!(completed[0].data, vec![0xa1, 0xa2, 0xa3, 0xa4, 0xa5]);
        assert_eq!(completed[0].bit_len, 40);
    }

    #[test]
    fn gameplay_effect_fragment_context_reaches_contiguous_tail() {
        let source = (Ipv4Addr::new(10, 0, 0, 2), 50_000);
        let destination = (Ipv4Addr::new(10, 0, 0, 3), 7_777);
        let effect = ParsedGameplayEffect {
            unique_index: 4741,
            byte_offset: 62,
            bit_shift: 3,
        };
        let mut tracker = GameplayEffectFragmentTracker::default();

        assert_eq!(
            tracker.observe(
                source,
                destination,
                &inventory_bunch_on(5146, 647, 0x09, 1),
                std::slice::from_ref(&effect),
            ),
            None
        );
        assert_eq!(
            tracker.observe(
                source,
                destination,
                &inventory_bunch_on(4122, 648, 0x08, 2),
                &[],
            ),
            Some(effect.clone())
        );
        assert_eq!(
            tracker.observe(
                source,
                destination,
                &inventory_bunch_on(4122, 649, 0x0c, 3),
                &[],
            ),
            Some(effect)
        );
        assert!(tracker.pending.is_empty());
    }

    #[test]
    fn gameplay_effect_fragment_context_rejects_sequence_gap() {
        let source = (Ipv4Addr::new(10, 0, 0, 2), 50_000);
        let destination = (Ipv4Addr::new(10, 0, 0, 3), 7_777);
        let effect = ParsedGameplayEffect {
            unique_index: 4741,
            byte_offset: 62,
            bit_shift: 3,
        };
        let mut tracker = GameplayEffectFragmentTracker::default();

        tracker.observe(
            source,
            destination,
            &inventory_bunch(647, 0x09, 1),
            &[effect],
        );
        assert_eq!(
            tracker.observe(source, destination, &inventory_bunch(649, 0x0c, 2), &[],),
            None
        );
        assert!(tracker.pending.is_empty());
    }

    #[test]
    fn gameplay_effect_fragment_context_rejects_ambiguous_start() {
        let source = (Ipv4Addr::new(10, 0, 0, 2), 50_000);
        let destination = (Ipv4Addr::new(10, 0, 0, 3), 7_777);
        let mut tracker = GameplayEffectFragmentTracker::default();
        let effects = [
            ParsedGameplayEffect {
                unique_index: 4740,
                byte_offset: 32,
                bit_shift: 3,
            },
            ParsedGameplayEffect {
                unique_index: 4741,
                byte_offset: 62,
                bit_shift: 3,
            },
        ];

        tracker.observe(
            source,
            destination,
            &inventory_bunch(647, 0x09, 1),
            &effects,
        );
        assert_eq!(
            tracker.observe(source, destination, &inventory_bunch(648, 0x0c, 2), &[],),
            None
        );
        assert!(tracker.pending.is_empty());
    }

    #[test]
    fn bool_enum_gameplay_effect_fragment_reassembles_pending_hit() {
        let source = (Ipv4Addr::new(10, 0, 0, 2), 50_000);
        let destination = (Ipv4Addr::new(10, 0, 0, 3), 7_777);
        let start = inventory_bunch_on(5146, 647, 0x09, 0xa1);
        let tail = inventory_bunch_on(4122, 648, 0x0c, 0xa2);
        let mut hit = targetless_hit();
        hit.damage = 163_027.0;
        let mut tracker = BoolEnumGameplayEffectFragmentTracker::default();

        let start_observation = tracker.observe(10.0, source, destination, &start);
        assert!(start_observation.completed.is_none());
        assert!(start_observation.abandoned_hits.is_empty());
        assert!(
            tracker
                .attach_hit(source, destination, &start, hit)
                .is_none()
        );

        let completed = tracker.observe(10.01, source, destination, &tail);
        let completed = completed
            .completed
            .expect("contiguous tail should complete");
        assert_eq!(completed.hit.damage, 163_027.0);
        assert_eq!(completed.payload, vec![0xa1, 0xa2]);
        assert!(tracker.pending.is_empty());
    }

    #[test]
    fn bool_enum_gameplay_effect_fragment_gap_releases_pending_hit() {
        let source = (Ipv4Addr::new(10, 0, 0, 2), 50_000);
        let destination = (Ipv4Addr::new(10, 0, 0, 3), 7_777);
        let start = inventory_bunch(647, 0x09, 0xa1);
        let mut hit = targetless_hit();
        hit.damage = 42.0;
        let mut tracker = BoolEnumGameplayEffectFragmentTracker::default();
        tracker.observe(10.0, source, destination, &start);
        assert!(
            tracker
                .attach_hit(source, destination, &start, hit)
                .is_none()
        );

        let observation = tracker.observe(
            10.01,
            source,
            destination,
            &inventory_bunch(649, 0x0c, 0xa2),
        );

        assert!(observation.completed.is_none());
        assert_eq!(observation.abandoned_hits.len(), 1);
        assert_eq!(observation.abandoned_hits[0].damage, 42.0);
        assert!(tracker.pending.is_empty());
    }

    #[test]
    fn bool_enum_gameplay_effect_fragment_timeout_releases_pending_hit() {
        let source = (Ipv4Addr::new(10, 0, 0, 2), 50_000);
        let destination = (Ipv4Addr::new(10, 0, 0, 3), 7_777);
        let start = inventory_bunch(647, 0x09, 0xa1);
        let mut hit = targetless_hit();
        hit.damage = 84.0;
        let mut tracker = BoolEnumGameplayEffectFragmentTracker::default();
        tracker.observe(10.0, source, destination, &start);
        assert!(
            tracker
                .attach_hit(source, destination, &start, hit)
                .is_none()
        );

        let expired = tracker.take_expired(10.0 + GAMEPLAY_EFFECT_FRAGMENT_TIMEOUT_SECONDS + 0.01);

        assert_eq!(expired.len(), 1);
        assert_eq!(expired[0].damage, 84.0);
        assert!(tracker.pending.is_empty());
    }

    #[test]
    fn bool_enum_gameplay_effect_fragment_replacement_releases_pending_hit() {
        let source = (Ipv4Addr::new(10, 0, 0, 2), 50_000);
        let destination = (Ipv4Addr::new(10, 0, 0, 3), 7_777);
        let first = inventory_bunch(647, 0x09, 0xa1);
        let replacement = inventory_bunch(700, 0x09, 0xb1);
        let mut hit = targetless_hit();
        hit.damage = 126.0;
        let mut tracker = BoolEnumGameplayEffectFragmentTracker::default();
        tracker.observe(10.0, source, destination, &first);
        assert!(
            tracker
                .attach_hit(source, destination, &first, hit)
                .is_none()
        );

        let observation = tracker.observe(10.01, source, destination, &replacement);

        assert_eq!(observation.abandoned_hits.len(), 1);
        assert_eq!(observation.abandoned_hits[0].damage, 126.0);
        assert!(observation.completed.is_none());
        assert_eq!(tracker.pending.len(), 1);
    }

    #[test]
    fn owner_only_connection_does_not_replace_active_inventory() {
        let active = InventoryConnectionKey::new("old:1".to_owned(), "client:1".to_owned());
        let owner_only = InventoryConnectionKey::new("new:1".to_owned(), "client:1".to_owned());
        let owner_net_id = HtItemNetId {
            solt: 30,
            serial: 40,
        };
        let item = inventory_test_item(None);
        let mut decoder = EmptyCurtainDecoder::new(EquipmentCatalog::default());
        decoder.active_connection = Some(active.clone());
        decoder
            .connections
            .insert(active.clone(), InventoryConnectionState::default());
        decoder.items.insert(item.id, item.clone());

        let result = decoder.process_packet(
            owner_only.clone(),
            &character_owner_packet(1020, owner_net_id),
        );

        assert!(result.recognized);
        assert!(result.snapshot.is_none());
        assert!(result.characters.is_none());
        assert_eq!(decoder.active_connection, Some(active));
        assert_eq!(decoder.items, HashMap::from([(item.id, item)]));
        assert_eq!(
            decoder.connections[&owner_only]
                .character_ids
                .get(&owner_net_id),
            Some(&1020)
        );
    }

    #[test]
    fn raw_owner_record_is_cached_without_a_completed_inventory_stream() {
        let active = InventoryConnectionKey::new("old:1".to_owned(), "client:1".to_owned());
        let owner_only = InventoryConnectionKey::new("new:1".to_owned(), "client:1".to_owned());
        let owner_net_id = HtItemNetId {
            solt: 30,
            serial: 40,
        };
        let item = inventory_test_item(None);
        let mut decoder = EmptyCurtainDecoder::new(EquipmentCatalog::default());
        decoder.active_connection = Some(active.clone());
        decoder
            .connections
            .insert(active.clone(), InventoryConnectionState::default());
        decoder.items.insert(item.id, item.clone());

        let result = decoder.process_packet(
            owner_only.clone(),
            &raw_character_owner_packet(1020, owner_net_id),
        );

        assert!(result.recognized);
        assert!(result.snapshot.is_none());
        assert!(result.characters.is_none());
        assert_eq!(decoder.active_connection, Some(active));
        assert_eq!(decoder.items, HashMap::from([(item.id, item)]));
        assert_eq!(
            decoder.connections[&owner_only]
                .character_ids
                .get(&owner_net_id),
            Some(&1020)
        );
    }

    #[test]
    fn same_account_connection_switch_preserves_existing_inventory() {
        let active = InventoryConnectionKey::new("old:1".to_owned(), "client:1".to_owned());
        let reconnect = InventoryConnectionKey::new("new:1".to_owned(), "client:2".to_owned());
        let owner_net_id = HtItemNetId {
            solt: 30,
            serial: 40,
        };
        let existing_id = HtItemNetId {
            solt: 500,
            serial: 600,
        };
        let new_id = HtItemNetId {
            solt: 501,
            serial: 601,
        };
        let item_id = "GetEfficiency_orange";
        let catalog = load_equipment_catalog(Path::new(EQUIPMENT_CATALOG_PATH))
            .expect("bundled equipment catalog should load");
        let mut decoder = EmptyCurtainDecoder::new(catalog);
        decoder.active_connection = Some(active.clone());
        decoder.connections.insert(
            active,
            InventoryConnectionState {
                character_ids: HashMap::from([(owner_net_id, 1020)]),
                ..InventoryConnectionState::default()
            },
        );
        decoder.items.insert(
            existing_id,
            inventory_equipment_item(existing_id, item_id, Some(owner_net_id), Some(1020)),
        );

        decoder.process_packet(
            reconnect.clone(),
            &raw_character_owner_packet(1020, owner_net_id),
        );
        let result = decoder.process_packet(
            reconnect.clone(),
            &inventory_item_stream_packet(&decoder.catalog, new_id, item_id, owner_net_id),
        );

        let snapshot = result
            .snapshot
            .expect("same-account reconnect should publish the merged inventory");
        assert_eq!(decoder.active_connection, Some(reconnect));
        assert_eq!(snapshot.len(), 2);
        assert!(snapshot.iter().any(|item| item.id == existing_id));
        assert!(snapshot.iter().any(|item| item.id == new_id));
    }

    #[test]
    fn different_account_connection_switch_replaces_existing_inventory() {
        let active = InventoryConnectionKey::new("old:1".to_owned(), "client:1".to_owned());
        let reconnect = InventoryConnectionKey::new("new:1".to_owned(), "client:2".to_owned());
        let old_owner_net_id = HtItemNetId {
            solt: 30,
            serial: 40,
        };
        let new_owner_net_id = HtItemNetId {
            solt: 31,
            serial: 41,
        };
        let existing_id = HtItemNetId {
            solt: 500,
            serial: 600,
        };
        let new_id = HtItemNetId {
            solt: 501,
            serial: 601,
        };
        let item_id = "GetEfficiency_orange";
        let catalog = load_equipment_catalog(Path::new(EQUIPMENT_CATALOG_PATH))
            .expect("bundled equipment catalog should load");
        let mut decoder = EmptyCurtainDecoder::new(catalog);
        decoder.active_connection = Some(active.clone());
        decoder.connections.insert(
            active,
            InventoryConnectionState {
                character_ids: HashMap::from([(old_owner_net_id, 1020)]),
                ..InventoryConnectionState::default()
            },
        );
        decoder.items.insert(
            existing_id,
            inventory_equipment_item(existing_id, item_id, Some(old_owner_net_id), Some(1020)),
        );

        decoder.process_packet(
            reconnect.clone(),
            &raw_character_owner_packet(1023, new_owner_net_id),
        );
        let result = decoder.process_packet(
            reconnect.clone(),
            &inventory_item_stream_packet(&decoder.catalog, new_id, item_id, new_owner_net_id),
        );

        let snapshot = result
            .snapshot
            .expect("different-account reconnect should publish the replacement inventory");
        assert_eq!(decoder.active_connection, Some(reconnect));
        assert_eq!(snapshot.len(), 1);
        assert_eq!(snapshot[0].id, new_id);
    }

    #[test]
    fn unrelated_flows_do_not_evict_the_active_inventory_connection() {
        let active = InventoryConnectionKey::new("server:1".to_owned(), "client:1".to_owned());
        let owner_net_id = HtItemNetId {
            solt: 30,
            serial: 40,
        };
        let mut active_state = InventoryConnectionState::default();
        active_state.character_ids.insert(owner_net_id, 1020);
        let mut decoder = EmptyCurtainDecoder::new(EquipmentCatalog::default());
        decoder.active_connection = Some(active.clone());
        decoder.connection_order.push_back(active.clone());
        decoder.connections.insert(active.clone(), active_state);

        for index in 0..MAX_INVENTORY_CONNECTIONS {
            let connection =
                InventoryConnectionKey::new(format!("unrelated:{index}"), "client:1".to_owned());
            decoder.process_packet(
                connection,
                &character_owner_packet(
                    1023,
                    HtItemNetId {
                        solt: 100 + index as u32,
                        serial: 200 + index as u32,
                    },
                ),
            );
        }

        assert_eq!(decoder.active_connection, Some(active.clone()));
        assert_eq!(
            decoder.connections[&active].character_ids,
            HashMap::from([(owner_net_id, 1020)])
        );
    }

    #[test]
    fn owner_only_active_connection_enriches_existing_inventory() {
        let connection = InventoryConnectionKey::new("server:1".to_owned(), "client:1".to_owned());
        let owner_net_id = HtItemNetId {
            solt: 30,
            serial: 40,
        };
        let item = inventory_test_item(Some(owner_net_id));
        let mut decoder = EmptyCurtainDecoder::new(EquipmentCatalog::default());
        decoder.active_connection = Some(connection.clone());
        decoder
            .connections
            .insert(connection.clone(), InventoryConnectionState::default());
        decoder.items.insert(item.id, item);

        let result =
            decoder.process_packet(connection, &character_owner_packet(1020, owner_net_id));

        assert!(result.recognized);
        let snapshot = result
            .snapshot
            .expect("owner mapping should enrich inventory");
        assert_eq!(snapshot.len(), 1);
        assert_eq!(snapshot[0].equipped_character_id, Some(1020));
        assert_eq!(
            result.characters,
            Some(vec![EmptyCurtainCharacter {
                net_id: owner_net_id,
                character_id: 1020,
            }])
        );
    }

    #[test]
    fn active_connection_publishes_a_character_without_equipped_items() {
        let connection = InventoryConnectionKey::new("server:1".to_owned(), "client:1".to_owned());
        let owner_net_id = HtItemNetId {
            solt: 30,
            serial: 40,
        };
        let mut decoder = EmptyCurtainDecoder::new(EquipmentCatalog::default());
        decoder.active_connection = Some(connection.clone());
        decoder
            .connections
            .insert(connection.clone(), InventoryConnectionState::default());
        let item = inventory_test_item(None);
        decoder.items.insert(item.id, item.clone());

        let result =
            decoder.process_packet(connection, &character_owner_packet(1020, owner_net_id));

        assert!(result.recognized);
        assert_eq!(result.snapshot, Some(vec![item]));
        assert_eq!(
            result.characters,
            Some(vec![EmptyCurtainCharacter {
                net_id: owner_net_id,
                character_id: 1020,
            }])
        );
    }

    #[test]
    fn compact_login_placement_is_applied_when_item_details_arrive_later() {
        let connection = InventoryConnectionKey::new("server:1".to_owned(), "client:1".to_owned());
        let character_net_id = HtItemNetId {
            solt: 30,
            serial: 40,
        };
        let item_id = "cell2_style2_1_Orange";
        let id = HtItemNetId {
            solt: 1_296_894_783,
            serial: 538_340_435,
        };
        let catalog = load_equipment_catalog(Path::new(EQUIPMENT_CATALOG_PATH))
            .expect("bundled equipment catalog should load");
        let mut decoder = EmptyCurtainDecoder::new(catalog);
        decoder.active_connection = Some(connection.clone());
        decoder
            .connections
            .insert(connection.clone(), InventoryConnectionState::default());

        let placement_result = decoder.process_packet(
            connection.clone(),
            &compact_module_placement_packet(1023, character_net_id, item_id, id, 1, 5),
        );
        assert!(placement_result.recognized);
        assert_eq!(
            decoder.connections[&connection].module_placements.get(&id),
            Some(&(
                item_id.to_owned(),
                EmptyCurtainPlacement { row: 1, column: 5 }
            ))
        );

        let item_result = decoder.process_packet(
            connection,
            &inventory_item_stream_packet(&decoder.catalog, id, item_id, character_net_id),
        );
        let item = item_result
            .snapshot
            .expect("later item details should publish the cached placement")
            .into_iter()
            .find(|item| item.id == id)
            .expect("the equipped module should be present");
        assert_eq!(item.equipped_character_id, Some(1023));
        assert_eq!(
            item.equipped_placement,
            Some(EmptyCurtainPlacement { row: 1, column: 5 })
        );
    }

    #[test]
    fn full_grid_placement_is_applied_when_item_details_arrive_later() {
        let connection = InventoryConnectionKey::new("server:1".to_owned(), "client:1".to_owned());
        let character_net_id = HtItemNetId {
            solt: 30,
            serial: 40,
        };
        let item_id = "cell3_style6_1_Orange";
        let id = HtItemNetId {
            solt: 1_296_894_783,
            serial: 538_340_435,
        };
        let catalog = load_equipment_catalog(Path::new(EQUIPMENT_CATALOG_PATH))
            .expect("bundled equipment catalog should load");
        let mut decoder = EmptyCurtainDecoder::new(catalog);
        decoder.active_connection = Some(connection.clone());
        decoder.connections.insert(
            connection.clone(),
            InventoryConnectionState {
                character_ids: HashMap::from([(character_net_id, 1023)]),
                ..InventoryConnectionState::default()
            },
        );

        assert!(!decoder.apply_equipment_snapshot(
            &connection,
            ParsedEmptyCurtainEquipmentSnapshot::Modules {
                character_net_id,
                character_id: 1023,
                placements: vec![ParsedEmptyCurtainModulePlacement {
                    equipment: id,
                    item_id: item_id.to_owned(),
                    row: 1,
                    column: 2,
                }],
            },
        ));
        assert_eq!(
            decoder.connections[&connection].module_placements.get(&id),
            Some(&(
                item_id.to_owned(),
                EmptyCurtainPlacement { row: 1, column: 2 }
            ))
        );

        let item_result = decoder.process_packet(
            connection,
            &inventory_item_stream_packet(&decoder.catalog, id, item_id, character_net_id),
        );
        let item = item_result
            .snapshot
            .expect("later item details should publish the cached full-grid placement")
            .into_iter()
            .find(|item| item.id == id)
            .expect("the equipped module should be present");
        assert_eq!(
            item.equipped_placement,
            Some(EmptyCurtainPlacement { row: 1, column: 2 })
        );
    }

    #[test]
    fn raw_item_records_apply_complete_updates_and_insert_new_items() {
        let connection = InventoryConnectionKey::new("server:1".to_owned(), "client:1".to_owned());
        let id = HtItemNetId {
            solt: 500,
            serial: 600,
        };
        let item_id = "GetEfficiency_orange";
        let catalog = load_equipment_catalog(Path::new(EQUIPMENT_CATALOG_PATH))
            .expect("bundled equipment catalog should load");
        let packets = [(true, false), (false, false), (false, true), (false, false)].map(
            |(locked, discarded)| {
                raw_inventory_item_packet(&catalog, id, item_id, None, locked, discarded)
            },
        );
        let mut decoder = EmptyCurtainDecoder::new(catalog);
        decoder.active_connection = Some(connection.clone());
        decoder
            .connections
            .insert(connection.clone(), InventoryConnectionState::default());
        decoder
            .items
            .insert(id, inventory_equipment_item(id, item_id, None, None));

        for ((locked, discarded), packet) in
            [(true, false), (false, false), (false, true), (false, false)]
                .into_iter()
                .zip(packets)
        {
            let result = decoder.process_packet(connection.clone(), &packet);
            assert!(result.recognized);
            let snapshot = result
                .snapshot
                .expect("changed raw item flags should publish an inventory snapshot");
            assert_eq!(
                (snapshot[0].locked, snapshot[0].discarded),
                (locked, discarded)
            );
        }

        let duplicate =
            raw_inventory_item_packet(&decoder.catalog, id, item_id, None, false, false);
        let result = decoder.process_packet(connection.clone(), &duplicate);
        assert!(result.recognized);
        assert!(result.snapshot.is_none());

        let upgrade = raw_inventory_item_packet_at_level(
            &decoder.catalog,
            id,
            item_id,
            None,
            20,
            false,
            false,
        );
        let result = decoder.process_packet(connection.clone(), &upgrade);
        assert!(result.recognized);
        let upgraded = result
            .snapshot
            .expect("a raw level change should publish an inventory snapshot")
            .into_iter()
            .find(|item| item.id == id)
            .expect("the upgraded item should remain in the inventory");
        assert_eq!(upgraded.level, 20);
        assert!(!upgraded.main_stats.is_empty());

        let unknown_id = HtItemNetId {
            solt: id.solt + 1,
            serial: id.serial + 1,
        };
        let unknown =
            raw_inventory_item_packet(&decoder.catalog, unknown_id, item_id, None, true, false);
        let result = decoder.process_packet(connection.clone(), &unknown);
        assert!(result.recognized);
        assert!(result.snapshot.is_some());
        assert_eq!(decoder.items[&unknown_id].item_id, item_id);
        assert!(decoder.items[&unknown_id].locked);

        let other_connection =
            InventoryConnectionKey::new("other:1".to_owned(), "client:1".to_owned());
        let inactive = raw_inventory_item_packet(&decoder.catalog, id, item_id, None, true, false);
        let result = decoder.process_packet(other_connection, &inactive);
        assert!(!result.recognized);
        assert!(result.snapshot.is_none());
        assert_eq!(
            (decoder.items[&id].locked, decoder.items[&id].discarded),
            (false, false)
        );
    }

    #[test]
    fn first_item_add_notification_activates_the_initial_inventory_connection() {
        let connection = InventoryConnectionKey::new("server:1".to_owned(), "client:1".to_owned());
        let id = HtItemNetId {
            solt: 500,
            serial: 600,
        };
        let item_id = "GetEfficiency_orange";
        let catalog = load_equipment_catalog(Path::new(EQUIPMENT_CATALOG_PATH))
            .expect("bundled equipment catalog should load");
        let raw = raw_inventory_item_packet(&catalog, id, item_id, None, false, false);
        let mut notification = InventoryTestBitWriter::default();
        notification.push_bits(3, 7);
        notification.push_bits(2, 17);
        for index in 0..raw.payload_bit_len {
            notification.push_bits(u64::from((raw.payload[index / 8] >> (index % 8)) & 1), 1);
        }
        let packet = SequencedPacket {
            payload_bit_len: notification.bit_len,
            payload: notification.data,
            ..raw
        };
        let mut decoder = EmptyCurtainDecoder::new(catalog);

        let result = decoder.process_packet(connection.clone(), &packet);

        assert!(result.recognized);
        assert_eq!(decoder.active_connection, Some(connection));
        assert_eq!(
            result
                .snapshot
                .expect("the first complete raw item should publish an inventory snapshot")
                .len(),
            1
        );
        assert_eq!(decoder.items[&id].item_id, item_id);
    }

    #[test]
    fn unscoped_raw_item_does_not_activate_an_inventory_connection() {
        let connection = InventoryConnectionKey::new("server:1".to_owned(), "client:1".to_owned());
        let id = HtItemNetId {
            solt: 500,
            serial: 600,
        };
        let item_id = "GetEfficiency_orange";
        let catalog = load_equipment_catalog(Path::new(EQUIPMENT_CATALOG_PATH))
            .expect("bundled equipment catalog should load");
        let packet = raw_inventory_item_packet(&catalog, id, item_id, None, false, false);
        let mut decoder = EmptyCurtainDecoder::new(catalog);

        let result = decoder.process_packet(connection, &packet);

        assert!(!result.recognized);
        assert!(result.snapshot.is_none());
        assert!(decoder.active_connection.is_none());
        assert!(decoder.items.is_empty());
    }

    #[test]
    fn raw_remove_notification_deletes_matching_items_without_a_completed_stream() {
        let connection = InventoryConnectionKey::new("server:1".to_owned(), "client:1".to_owned());
        let removed = HtItemNetId {
            solt: 500,
            serial: 600,
        };
        let retained = HtItemNetId {
            solt: 501,
            serial: 601,
        };
        let item_id = "GetEfficiency_orange";
        let catalog = load_equipment_catalog(Path::new(EQUIPMENT_CATALOG_PATH))
            .expect("bundled equipment catalog should load");
        let raw = raw_inventory_item_packet(&catalog, removed, item_id, None, false, false);
        let mut notification = InventoryTestBitWriter::default();
        notification.push_bits(2, 7);
        notification.push_bits(1, 17);
        for index in 0..raw.payload_bit_len {
            notification.push_bits(u64::from((raw.payload[index / 8] >> (index % 8)) & 1), 1);
        }
        let packet = SequencedPacket {
            payload_bit_len: notification.bit_len,
            payload: notification.data,
            ..raw
        };
        let mut decoder = EmptyCurtainDecoder::new(catalog);
        decoder.active_connection = Some(connection.clone());
        decoder
            .connections
            .insert(connection.clone(), InventoryConnectionState::default());
        decoder.items = HashMap::from([
            (
                removed,
                inventory_equipment_item(removed, item_id, None, None),
            ),
            (
                retained,
                inventory_equipment_item(retained, item_id, None, None),
            ),
        ]);

        let result = decoder.process_packet(connection, &packet);

        assert!(result.recognized);
        assert!(result.snapshot.is_some());
        assert!(!decoder.items.contains_key(&removed));
        assert!(decoder.items.contains_key(&retained));
    }

    #[test]
    fn completed_remove_stream_wins_over_raw_tail_item_update() {
        let connection = InventoryConnectionKey::new("server:1".to_owned(), "client:1".to_owned());
        let first_removed = HtItemNetId {
            solt: 500,
            serial: 600,
        };
        let tail_removed = HtItemNetId {
            solt: 501,
            serial: 601,
        };
        let retained = HtItemNetId {
            solt: 502,
            serial: 602,
        };
        let item_id = "Cosmos_purple";
        let catalog = load_equipment_catalog(Path::new(EQUIPMENT_CATALOG_PATH))
            .expect("bundled equipment catalog should load");
        let first_record =
            raw_inventory_item_packet(&catalog, first_removed, item_id, None, false, false);
        let tail_record =
            raw_inventory_item_packet(&catalog, tail_removed, item_id, None, false, false);
        let mut start_data = InventoryTestBitWriter::default();
        start_data.push_bits(2, 7);
        start_data.push_bits(2, 17);
        for index in 0..first_record.payload_bit_len {
            start_data.push_bits(
                u64::from((first_record.payload[index / 8] >> (index % 8)) & 1),
                1,
            );
        }
        let start = inventory_fragment_packet(start_data, 472, 0x09);
        let tail = inventory_fragment_packet(
            InventoryTestBitWriter {
                data: tail_record.payload,
                bit_len: tail_record.payload_bit_len,
            },
            473,
            0x0c,
        );
        let mut decoder = EmptyCurtainDecoder::new(catalog);
        decoder.active_connection = Some(connection.clone());
        decoder
            .connections
            .insert(connection.clone(), InventoryConnectionState::default());
        decoder.items = HashMap::from([
            (
                first_removed,
                inventory_equipment_item(first_removed, item_id, None, None),
            ),
            (
                tail_removed,
                inventory_equipment_item(tail_removed, item_id, None, None),
            ),
            (
                retained,
                inventory_equipment_item(retained, item_id, None, None),
            ),
        ]);

        let start_result = decoder.process_packet(connection.clone(), &start);
        let start_snapshot = start_result
            .snapshot
            .expect("the raw item detail update should publish");
        assert_eq!(start_snapshot.len(), 3);
        assert!(decoder.items.contains_key(&first_removed));

        let tail_result = decoder.process_packet(connection, &tail);
        let snapshot = tail_result
            .snapshot
            .expect("the completed removal stream should publish");
        assert_eq!(snapshot.len(), 1);
        assert_eq!(snapshot[0].id, retained);
        assert!(!decoder.items.contains_key(&first_removed));
        assert!(!decoder.items.contains_key(&tail_removed));
    }

    #[test]
    fn module_snapshot_assigns_present_items_and_clears_removed_items() {
        let connection = InventoryConnectionKey::new("server:1".to_owned(), "client:1".to_owned());
        let character_net_id = HtItemNetId {
            solt: 30,
            serial: 40,
        };
        let other_character_net_id = HtItemNetId {
            solt: 31,
            serial: 41,
        };
        let equipped = HtItemNetId {
            solt: 50,
            serial: 60,
        };
        let removed = HtItemNetId {
            solt: 51,
            serial: 61,
        };
        let other = HtItemNetId {
            solt: 52,
            serial: 62,
        };
        let core = HtItemNetId {
            solt: 53,
            serial: 63,
        };
        let catalog = load_equipment_catalog(Path::new(EQUIPMENT_CATALOG_PATH))
            .expect("bundled equipment catalog should load");
        let mut decoder = EmptyCurtainDecoder::new(catalog);
        decoder
            .connections
            .insert(connection.clone(), InventoryConnectionState::default());
        decoder.items = HashMap::from([
            (
                equipped,
                inventory_equipment_item(equipped, "cell3_style6_1_Orange", None, None),
            ),
            (
                removed,
                inventory_equipment_item(
                    removed,
                    "cell2_style2_1_Orange",
                    Some(character_net_id),
                    Some(1023),
                ),
            ),
            (
                other,
                inventory_equipment_item(
                    other,
                    "cell2_style2_1_Orange",
                    Some(other_character_net_id),
                    Some(1020),
                ),
            ),
            (
                core,
                inventory_equipment_item(
                    core,
                    "Incantation_orange",
                    Some(character_net_id),
                    Some(1023),
                ),
            ),
        ]);
        decoder
            .items
            .get_mut(&removed)
            .expect("removed module must exist")
            .equipped_placement = Some(EmptyCurtainPlacement { row: 2, column: 3 });
        decoder
            .items
            .get_mut(&other)
            .expect("other character module must exist")
            .equipped_placement = Some(EmptyCurtainPlacement { row: 4, column: 5 });
        let snapshot = ParsedEmptyCurtainEquipmentSnapshot::Modules {
            character_net_id,
            character_id: 1023,
            placements: vec![ParsedEmptyCurtainModulePlacement {
                equipment: equipped,
                item_id: "cell3_style6_1_Orange".to_owned(),
                row: 1,
                column: 2,
            }],
        };

        assert!(decoder.apply_equipment_snapshot(&connection, snapshot.clone()));
        assert_eq!(
            decoder.items[&equipped].character_net_id,
            Some(character_net_id)
        );
        assert_eq!(decoder.items[&equipped].equipped_character_id, Some(1023));
        assert_eq!(
            decoder.items[&equipped].equipped_placement,
            Some(EmptyCurtainPlacement { row: 1, column: 2 })
        );
        assert_eq!(decoder.items[&removed].character_net_id, None);
        assert_eq!(decoder.items[&removed].equipped_character_id, None);
        assert_eq!(decoder.items[&removed].equipped_placement, None);
        assert_eq!(
            decoder.items[&other].character_net_id,
            Some(other_character_net_id)
        );
        assert_eq!(
            decoder.items[&other].equipped_placement,
            Some(EmptyCurtainPlacement { row: 4, column: 5 })
        );
        assert_eq!(
            decoder.items[&core].character_net_id,
            Some(character_net_id)
        );
        assert!(!decoder.apply_equipment_snapshot(&connection, snapshot));
    }

    #[test]
    fn core_snapshot_replaces_and_clears_only_the_character_core() {
        let connection = InventoryConnectionKey::new("server:1".to_owned(), "client:1".to_owned());
        let character_net_id = HtItemNetId {
            solt: 30,
            serial: 40,
        };
        let equipped = HtItemNetId {
            solt: 50,
            serial: 60,
        };
        let removed = HtItemNetId {
            solt: 51,
            serial: 61,
        };
        let module = HtItemNetId {
            solt: 52,
            serial: 62,
        };
        let catalog = load_equipment_catalog(Path::new(EQUIPMENT_CATALOG_PATH))
            .expect("bundled equipment catalog should load");
        let mut decoder = EmptyCurtainDecoder::new(catalog);
        decoder.items = HashMap::from([
            (
                equipped,
                inventory_equipment_item(equipped, "Incantation_orange", None, None),
            ),
            (
                removed,
                inventory_equipment_item(
                    removed,
                    "Nature_orange",
                    Some(character_net_id),
                    Some(1023),
                ),
            ),
            (
                module,
                inventory_equipment_item(
                    module,
                    "cell2_style2_1_Orange",
                    Some(character_net_id),
                    Some(1023),
                ),
            ),
        ]);

        assert!(decoder.apply_equipment_snapshot(
            &connection,
            ParsedEmptyCurtainEquipmentSnapshot::Core {
                character_net_id,
                character_id: 1023,
                item_id: Some(equipped),
            }
        ));
        assert_eq!(
            decoder.items[&equipped].character_net_id,
            Some(character_net_id)
        );
        assert_eq!(decoder.items[&removed].character_net_id, None);
        assert_eq!(
            decoder.items[&module].character_net_id,
            Some(character_net_id)
        );
        assert!(decoder.apply_equipment_snapshot(
            &connection,
            ParsedEmptyCurtainEquipmentSnapshot::Core {
                character_net_id,
                character_id: 1023,
                item_id: None,
            }
        ));
        assert_eq!(decoder.items[&equipped].character_net_id, None);
        assert_eq!(
            decoder.items[&module].character_net_id,
            Some(character_net_id)
        );
    }

    #[test]
    fn capture_export_defaults_and_preserves_empty_curtain_snapshot() {
        let legacy = parse_capture_export(r#"{"hits":[],"packets":[]}"#)
            .expect("legacy capture export should remain compatible");
        assert_eq!(legacy.version, CAPTURE_EXPORT_VERSION);
        assert!(legacy.empty_curtain.is_empty());
        assert!(legacy.empty_curtain_characters.is_empty());

        let current = parse_capture_export(
            r#"{"hits":[],"packets":[],"empty_curtain":[{"id":{"solt":1,"serial":2},"item_id":"cell2_style1_1_Orange","level":20,"main_stats":[],"sub_stats":[],"locked":true,"character_net_id":{"solt":3,"serial":4},"equipped_character_id":1020}],"empty_curtain_characters":[{"net_id":{"solt":3,"serial":4},"character_id":1020}]}"#,
        )
        .expect("current capture export should preserve Console equipment");
        assert_eq!(current.empty_curtain.len(), 1);
        assert_eq!(current.empty_curtain[0].id.solt, 1);
        assert!(current.empty_curtain[0].locked);
        assert!(!current.empty_curtain[0].discarded);
        assert_eq!(current.empty_curtain[0].equipped_character_id, Some(1020));
        assert_eq!(current.empty_curtain[0].equipped_placement, None);
        assert_eq!(current.empty_curtain_characters.len(), 1);
        assert_eq!(current.empty_curtain_characters[0].character_id, 1020);

        let mut positioned = current.empty_curtain[0].clone();
        positioned.equipped_placement = Some(EmptyCurtainPlacement { row: 2, column: 3 });
        let json = serde_json::to_string(&positioned).expect("positioned item should serialize");
        let restored: EmptyCurtainItem =
            serde_json::from_str(&json).expect("positioned item should deserialize");
        assert_eq!(restored.equipped_placement, positioned.equipped_placement);
    }

    #[test]
    fn versioned_capture_export_snapshot_round_trips_replay_fields() {
        let mut state = CombatState::default();
        let mut hit = targetless_hit();
        hit.timestamp = 1.25;
        hit.char_id = 1076;
        hit.char_name = "Shinku".to_owned();
        hit.damage = 1_234.5;
        hit.target_id = Some("TARGET".to_owned());
        hit.target_context = vec!["context-a".to_owned(), "context-b".to_owned()];
        hit.ability_name = Some("GA_Test".to_owned());
        hit.follow_up_damage = 12.5;
        hit.follow_up_timestamp = Some(1.5);
        state.push_hit(hit);
        state.push_packet(debug_packet("round trip"));
        state.apply_time_stop_event(TimeStopEvent::GamePauseStarted {
            timestamp: 1.0,
            pause_type_mask: 1 << 2,
        });
        state.apply_time_stop_event(TimeStopEvent::GamePauseEnded {
            timestamp: 1.2,
            pause_type_mask: 1 << 2,
        });

        let document = CaptureExportDocument::snapshot(
            &state,
            CaptureExportOptions {
                filter: "udp and host TARGET".to_owned(),
                include_incoming: true,
                game_network: Some(CaptureExportNetwork {
                    pid: 123,
                    local_ip: "127.0.0.1".to_owned(),
                    remote_ip: "127.0.0.2".to_owned(),
                    remote_port: 30_031,
                }),
                dps_time_mode: DpsTimeBasis::SubtractTimeStop,
            },
        );
        let json = serde_json::to_string_pretty(&document)
            .expect("capture export snapshot should serialize");
        let restored = parse_capture_export(&json)
            .expect("serialized capture export snapshot should remain replayable");

        assert_eq!(restored.version, CAPTURE_EXPORT_VERSION);
        assert_eq!(restored.filter, "udp and host TARGET");
        assert!(restored.include_incoming);
        assert_eq!(restored.summary.hits, 1);
        assert_eq!(restored.summary.packets, 1);
        assert_eq!(restored.summary.dps_time_mode, "Exclude Time Stop");
        assert_eq!(restored.party.len(), 1);
        assert_eq!(restored.hits[0].ability_name.as_deref(), Some("GA_Test"));
        assert_eq!(restored.hits[0].target_context, ["context-a", "context-b"]);
        assert_eq!(restored.packets[0].note, "round trip");
        assert_eq!(restored.packets[0].declared_ids, serde_json::json!([]));
        assert_eq!(restored.time_stop_events, state.time_stop_events);
    }

    #[test]
    fn capture_export_rejects_unknown_versions() {
        let error = parse_capture_export(r#"{"version":2,"hits":[],"packets":[]}"#)
            .expect_err("unknown capture export versions must be rejected");
        assert!(error.contains("unsupported capture export version 2"));
    }

    #[test]
    fn legacy_capture_repair_uses_one_bounded_buffer_and_preserves_crlf() {
        let legacy = concat!(
            "{\r\n",
            "  \"version\":1,\r\n",
            "  \"hits\":[],\r\n",
            "  \"packets\":[{\r\n",
            "    \"timestamp_unix\":1,\r\n",
            "    \"source\":\"a\",\r\n",
            "    \"destination\":\"b\",\r\n",
            "    \"payload_hex\" : \"00\"\r\n",
            "    \"decoded_text\":\"ok\"\r\n",
            "  }]\r\n",
            "}"
        );
        let repaired = repair_legacy_capture_export(legacy)
            .expect("bounded legacy capture should be repairable");
        assert!(repaired.contains("\"payload_hex\" : \"00\",\r\n"));
        let parsed = parse_capture_export(legacy)
            .expect("legacy missing payload comma should remain replayable");
        assert_eq!(parsed.packets.len(), 1);
        assert_eq!(parsed.packets[0].decoded_text, "ok");
        assert!(repair_legacy_capture_export_with_limit(legacy, legacy.len() - 1).is_err());
        let exact_limit_missing_comma = "  \"payload_hex\": \"00\"";
        assert!(
            repair_legacy_capture_export_with_limit(
                exact_limit_missing_comma,
                exact_limit_missing_comma.len(),
            )
            .is_err(),
            "the extra comma must be budgeted before allocating the output buffer"
        );
        let unrelated = "  \"decoded_text\": \"payload_hex\"";
        assert_eq!(
            repair_legacy_capture_export_with_limit(unrelated, unrelated.len())
                .expect("unrelated lines need no allocation growth"),
            unrelated
        );
    }

    #[test]
    fn capture_export_declared_ids_rejects_invalid_shapes_before_state_use() {
        let packet_document = |declared_ids: serde_json::Value| {
            serde_json::json!({
                "version": 1,
                "hits": [],
                "packets": [{
                    "timestamp_unix": 1,
                    "source": "a",
                    "destination": "b",
                    "declared_ids": declared_ids
                }]
            })
            .to_string()
        };

        assert!(parse_capture_export(packet_document(serde_json::json!([1, 2]))).is_ok());
        assert!(parse_capture_export(packet_document(serde_json::json!([1, "2"]))).is_err());
        assert!(parse_capture_export(packet_document(serde_json::json!({ "id": 1 }))).is_err());
        assert!(
            parse_capture_export(packet_document(serde_json::json!([[1]]))).is_err(),
            "nested arrays must not be retained as arbitrary JSON"
        );
        assert!(
            parse_capture_export(packet_document(serde_json::json!(
                (0..=MAX_CAPTURE_JSON_IMPORT_DECLARED_IDS).collect::<Vec<_>>()
            )))
            .is_err()
        );
        let legacy = format!(
            "[{}]",
            (0..MAX_CAPTURE_JSON_IMPORT_DECLARED_IDS)
                .map(|id| id.to_string())
                .collect::<Vec<_>>()
                .join(",")
        );
        assert!(parse_capture_export(packet_document(serde_json::json!(legacy))).is_ok());
    }

    #[test]
    fn capture_export_writer_persists_a_replayable_document() {
        let path = std::env::temp_dir().join(format!(
            "nte-capture-export-{}-{}.json",
            std::process::id(),
            Local::now()
                .timestamp_nanos_opt()
                .expect("current local time must fit in nanoseconds")
        ));
        let document = CaptureExportDocument::snapshot(
            &CombatState::default(),
            CaptureExportOptions {
                filter: "udp".to_owned(),
                include_incoming: false,
                game_network: None,
                dps_time_mode: DpsTimeBasis::WallClock,
            },
        );

        write_capture_export(&path, &document)
            .expect("capture export should be written atomically");
        let text = std::fs::read_to_string(&path).expect("capture export should be readable");
        let restored = parse_capture_export(&text).expect("written capture export should replay");
        std::fs::remove_file(&path).expect("capture export fixture should be removable");

        assert_eq!(restored.version, CAPTURE_EXPORT_VERSION);
        assert_eq!(restored.filter, "udp");
        assert_eq!(restored.summary.dps_time_mode, "Real Time");
    }

    #[test]
    fn capture_export_streams_generation_checked_bounded_hit_pages() {
        let path = std::env::temp_dir().join(format!(
            "nte-capture-stream-export-{}-{}.json",
            std::process::id(),
            Local::now()
                .timestamp_nanos_opt()
                .expect("current local time must fit in nanoseconds")
        ));
        let mut state = CombatState::default();
        let hit_count = CAPTURE_EXPORT_HIT_PAGE_SIZE * 2 + 17;
        for index in 0..hit_count {
            let mut hit = targetless_hit();
            hit.timestamp = index as f64;
            hit.damage = index as f64 + 1.0;
            if index + 1 == hit_count {
                hit.direction = HitDirection::Incoming;
            }
            state.push_hit(hit);
        }
        let plan = CaptureExportDocument::prepare(
            &state,
            CaptureExportOptions {
                filter: "udp".to_owned(),
                include_incoming: false,
                game_network: None,
                dps_time_mode: DpsTimeBasis::WallClock,
            },
        );
        let mut page_sizes = Vec::new();

        write_capture_export_streaming(&path, &plan, |start, limit| {
            page_sizes.push(limit);
            Ok(state.hits.range(start..start + limit).cloned().collect())
        })
        .expect("bounded capture export should stream atomically");
        let text = std::fs::read_to_string(&path).expect("streamed export should be readable");
        let restored = parse_capture_export(text).expect("streamed export should replay");
        std::fs::remove_file(&path).expect("capture export fixture should be removable");

        assert_eq!(restored.hits.len(), hit_count);
        assert_eq!(restored.summary.hits, hit_count);
        assert_eq!(restored.summary.ended_at_unix, Some((hit_count - 1) as f64));
        assert_eq!(restored.hits[hit_count - 1].damage, hit_count as f64);
        assert_eq!(
            page_sizes,
            [
                CAPTURE_EXPORT_HIT_PAGE_SIZE,
                CAPTURE_EXPORT_HIT_PAGE_SIZE,
                17
            ]
        );
    }

    #[test]
    fn capture_export_source_change_preserves_existing_destination() {
        let path = std::env::temp_dir().join(format!(
            "nte-capture-stream-conflict-{}-{}.json",
            std::process::id(),
            Local::now()
                .timestamp_nanos_opt()
                .expect("current local time must fit in nanoseconds")
        ));
        std::fs::write(&path, b"existing export")
            .expect("existing export fixture should be writable");
        let mut state = CombatState::default();
        for index in 0..=CAPTURE_EXPORT_HIT_PAGE_SIZE {
            let mut hit = targetless_hit();
            hit.timestamp = index as f64;
            state.push_hit(hit);
        }
        let plan = CaptureExportDocument::prepare(
            &state,
            CaptureExportOptions {
                filter: String::new(),
                include_incoming: false,
                game_network: None,
                dps_time_mode: DpsTimeBasis::WallClock,
            },
        );
        let result = write_capture_export_streaming(&path, &plan, |start, limit| {
            if start != 0 {
                return Err("capture export source changed during streaming".to_owned());
            }
            Ok(state.hits.range(start..start + limit).cloned().collect())
        });
        let contents = std::fs::read(&path).expect("existing destination should remain readable");
        std::fs::remove_file(&path).expect("capture export fixture should be removable");

        assert!(result.is_err());
        assert_eq!(contents, b"existing export");
    }

    #[test]
    fn current_capture_hit_schema_replays_every_supported_field() {
        let document = parse_capture_export(
            r#"{
                "hits":[{
                    "timestamp_unix":1.25,
                    "char_id":1076,
                    "char_name":"Shinku",
                    "damage":1234.5,
                    "direction":"outgoing",
                    "target_hp_before":10000.0,
                    "target_hp_after":8765.5,
                    "target_max_hp":10000.0,
                    "target_hp_percent":87.655,
                    "target_id":"TARGET",
                    "target_name":"Target Name",
                    "target_name_en":"Target Name EN",
                    "target_name_ja":"ターゲット名",
                    "target_monster_id":"Boss_16",
                    "target_context":["context-a","context-b"],
                    "gameplay_effect_index":77,
                    "gameplay_effect_name":"GE_Test_Damage",
                    "ability_name":"GA_Test",
                    "damage_name":"Test Move",
                    "damage_component":"Exact Component",
                    "attack_type":"Q技能",
                    "damage_attribute":"灵",
                    "follow_up_damage":12.5,
                    "follow_up_timestamp":1.5,
                    "follow_up_damage_name":"Follow Up",
                    "follow_up_attack_type":"覆纹",
                    "follow_up_damage_attribute":"灵",
                    "reconciled_overkill_damage":234.5
                }],
                "packets":[],
                "empty_curtain":[],
                "empty_curtain_characters":[]
            }"#,
        )
        .expect("current capture schema should parse");

        let event = export_hit_event(
            document
                .hits
                .into_iter()
                .next()
                .expect("fixture contains one hit"),
        );
        let EngineEvent::Hit(hit) = event else {
            panic!("capture hit must replay as a hit event");
        };

        assert_eq!(hit.timestamp, 1.25);
        assert_eq!(hit.char_id, 1076);
        assert_eq!(hit.char_name, "Shinku");
        assert_eq!(hit.damage, 1234.5);
        assert_eq!(hit.direction, HitDirection::Outgoing);
        assert_eq!(hit.target_hp_before, 10_000.0);
        assert_eq!(hit.target_hp_after, 8_765.5);
        assert_eq!(hit.target_max_hp, 10_000.0);
        assert_eq!(hit.target_hp_percent, 87.655);
        assert_eq!(hit.target_id.as_deref(), Some("TARGET"));
        assert_eq!(hit.target_name.as_deref(), Some("Target Name"));
        assert_eq!(hit.target_name_en.as_deref(), Some("Target Name EN"));
        assert_eq!(hit.target_name_ja.as_deref(), Some("ターゲット名"));
        assert_eq!(hit.target_monster_id.as_deref(), Some("Boss_16"));
        assert_eq!(hit.target_context, ["context-a", "context-b"]);
        assert_eq!(hit.gameplay_effect_index, Some(77));
        assert_eq!(hit.gameplay_effect_name.as_deref(), Some("GE_Test_Damage"));
        assert_eq!(hit.ability_name.as_deref(), Some("GA_Test"));
        assert_eq!(hit.damage_name.as_deref(), Some("Test Move"));
        assert_eq!(hit.damage_component.as_deref(), Some("Exact Component"));
        assert_eq!(hit.attack_type.as_deref(), Some("Q技能"));
        assert_eq!(hit.damage_attribute.as_deref(), Some("灵"));
        assert_eq!(hit.follow_up_damage, 12.5);
        assert_eq!(hit.follow_up_timestamp, Some(1.5));
        assert_eq!(hit.follow_up_damage_name.as_deref(), Some("Follow Up"));
        assert_eq!(hit.follow_up_attack_type.as_deref(), Some("覆纹"));
        assert_eq!(hit.follow_up_damage_attribute.as_deref(), Some("灵"));
        assert_eq!(hit.reconciled_overkill_damage, Some(234.5));
        assert_eq!(hit.overkill_damage(), 0.0);
        assert_eq!(hit.char_source, HitCharacterSource::ExportJson);
    }

    #[test]
    fn replay_character_mappings_validate_ids_and_deduplicate() {
        let character = EmptyCurtainCharacter {
            net_id: HtItemNetId { solt: 1, serial: 2 },
            character_id: 1020,
        };
        assert_eq!(
            validate_empty_curtain_characters(vec![character, character]),
            Some(vec![character])
        );
        for net_id in [
            HtItemNetId { solt: 0, serial: 0 },
            HtItemNetId { solt: 0, serial: 2 },
            HtItemNetId { solt: 1, serial: 0 },
            HtItemNetId {
                solt: u32::MAX,
                serial: 2,
            },
            HtItemNetId {
                solt: 1,
                serial: u32::MAX,
            },
            HtItemNetId {
                solt: u32::MAX,
                serial: u32::MAX,
            },
        ] {
            assert!(
                validate_empty_curtain_characters(vec![EmptyCurtainCharacter {
                    net_id,
                    character_id: 1020,
                }])
                .is_none()
            );
        }
        assert!(
            validate_empty_curtain_characters(vec![EmptyCurtainCharacter {
                character_id: 0,
                ..character
            }])
            .is_none()
        );
        assert!(
            validate_empty_curtain_characters(vec![
                character,
                EmptyCurtainCharacter {
                    character_id: 1032,
                    ..character
                },
            ])
            .is_none()
        );
    }

    #[test]
    fn capture_link_type_maps_supported_npcap_and_pcapng_values() {
        assert_eq!(
            CaptureLinkType::from_npcap_datalink(DLT_EN10MB).unwrap(),
            CaptureLinkType::Ethernet
        );
        assert_eq!(
            CaptureLinkType::from_npcap_datalink(DLT_RAW).unwrap(),
            CaptureLinkType::RawIpv4
        );
        assert_eq!(
            CaptureLinkType::from_npcap_datalink(DLT_IPV4).unwrap(),
            CaptureLinkType::Ipv4
        );
        assert_eq!(
            CaptureLinkType::from_pcapng(DataLink::ETHERNET),
            Some(CaptureLinkType::Ethernet)
        );
        assert_eq!(
            CaptureLinkType::from_pcapng(DataLink::RAW),
            Some(CaptureLinkType::RawIpv4)
        );
        assert_eq!(
            CaptureLinkType::from_pcapng(DataLink::IPV4),
            Some(CaptureLinkType::Ipv4)
        );
        assert_eq!(CaptureLinkType::RawIpv4.pcapng_data_link(), DataLink::RAW);
        assert_eq!(CaptureLinkType::Ipv4.pcapng_data_link(), DataLink::IPV4);
        assert_eq!(CaptureLinkType::from_pcapng(DataLink::NULL), None);

        let error = CaptureLinkType::from_npcap_datalink(0).unwrap_err();
        assert!(error.contains("unsupported Npcap data link type 0"));
    }

    #[test]
    fn raw_ipv4_packet_uses_the_same_udp_decoder_as_ethernet() {
        let source = Ipv4Addr::new(10, 0, 0, 2);
        let destination = Ipv4Addr::new(10, 0, 0, 3);
        let ethernet = udp_ipv4_packet(b"raw-ipv4", source, 50_000, destination, 7_777);
        let raw_ipv4 = &ethernet[14..];

        assert_eq!(
            parse_udp_ipv4(CaptureLinkType::Ethernet, &ethernet),
            parse_udp_ipv4(CaptureLinkType::RawIpv4, raw_ipv4)
        );
        assert_eq!(
            parse_udp_ipv4(CaptureLinkType::RawIpv4, raw_ipv4),
            parse_udp_ipv4(CaptureLinkType::Ipv4, raw_ipv4)
        );

        let mut ipv6 = raw_ipv4.to_vec();
        ipv6[0] = 0x60;
        assert_eq!(parse_udp_ipv4(CaptureLinkType::RawIpv4, &ipv6), None);
    }

    #[test]
    fn pcapng_import_accepts_foreign_tun_linktype_ipv4() {
        let directory = std::env::temp_dir().join(format!(
            "nte-foreign-tun-import-test-{}-{}",
            std::process::id(),
            current_filetime_100ns()
        ));
        std::fs::create_dir_all(&directory).expect("create test directory");
        struct Cleanup(std::path::PathBuf);
        impl Drop for Cleanup {
            fn drop(&mut self) {
                let _ = std::fs::remove_dir_all(&self.0);
            }
        }
        let _cleanup = Cleanup(directory.clone());
        let path = directory.join("foreign-tun.pcapng");
        let capture_local_ip = Ipv4Addr::new(198, 18, 0, 2);
        let remote_ip = Ipv4Addr::new(203, 0, 113, 9);
        let ethernet = udp_ipv4_packet(b"Foreign_Tun", capture_local_ip, 50_000, remote_ip, 30_196);
        let raw_ipv4 = &ethernet[14..];
        {
            let file = File::create(&path).expect("create pcapng fixture");
            let mut writer =
                PcapNgWriter::new(BufWriter::new(file)).expect("initialize pcapng fixture");
            writer
                .write_pcapng_block(InterfaceDescriptionBlock::new(
                    DataLink::IPV4,
                    CAPTURE_SNAPLEN,
                ))
                .expect("write IPv4 interface");
            writer
                .write_pcapng_block(EnhancedPacketBlock {
                    interface_id: 0,
                    timestamp: Duration::from_secs(1),
                    original_len: raw_ipv4.len() as u32,
                    data: Cow::Borrowed(raw_ipv4),
                    options: Vec::new(),
                })
                .expect("write raw IPv4 packet");
            writer.get_mut().flush().expect("flush pcapng fixture");
        }

        let (sender, receiver) = unbounded();
        let handle = import_pcapng(
            path,
            CaptureResources {
                characters: Arc::new(HashMap::new()),
                ability_catalog: Arc::new(AbilityCatalog::default()),
            },
            Some(Ipv4Addr::new(192, 0, 2, 99)),
            true,
            false,
            EngineEventSink::reliable(sender),
            Arc::new(AtomicBool::new(false)),
        )
        .expect("pcapng import thread should spawn");
        handle.join().expect("pcapng import thread should finish");

        let events = receiver.try_iter().collect::<Vec<_>>();
        assert!(
            events.iter().any(|event| matches!(
                event,
                EngineEvent::Packet(packet)
                    if packet.decoded_text.contains("Foreign_Tun")
                        && packet.direction == "C2S"
            )),
            "foreign TUN packet should reach the shared decoder: {events:#?}"
        );
        assert!(
            events
                .iter()
                .all(|event| !matches!(event, EngineEvent::Error(_))),
            "foreign TUN import should not emit an error: {events:#?}"
        );
    }

    #[test]
    fn raw_capture_writer_records_the_actual_raw_linktype() {
        let directory = std::env::temp_dir().join(format!(
            "nte-raw-linktype-test-{}-{}",
            std::process::id(),
            current_filetime_100ns()
        ));
        std::fs::create_dir_all(&directory).expect("create test directory");
        struct Cleanup(std::path::PathBuf);
        impl Drop for Cleanup {
            fn drop(&mut self) {
                let _ = std::fs::remove_dir_all(&self.0);
            }
        }
        let _cleanup = Cleanup(directory.clone());
        let path = directory.join("raw.pcapng");
        let device = CaptureDevice {
            name: "test-tun".to_owned(),
            description: "test TUN".to_owned(),
            ipv4: Vec::new(),
        };
        let ethernet = udp_ipv4_packet(
            b"raw-ipv4",
            Ipv4Addr::new(10, 0, 0, 2),
            50_000,
            Ipv4Addr::new(10, 0, 0, 3),
            7_777,
        );
        let raw_ipv4 = &ethernet[14..];
        let mut writer =
            RawCaptureWriter::create(&path, &device, CaptureLinkType::RawIpv4).unwrap();
        writer
            .write_packet(Duration::from_secs(1), raw_ipv4.len() as u32, raw_ipv4)
            .unwrap();
        writer.finish().unwrap();

        let file = File::open(&path).unwrap();
        let mut reader = PcapNgReader::new(file).unwrap();
        while reader.interfaces().is_empty() {
            assert!(reader.next_block().is_some());
        }
        assert_eq!(reader.interfaces()[0].linktype, DataLink::RAW);
    }

    #[test]
    fn disabled_raw_capture_has_no_path_or_writer() {
        let buffer = RawCaptureBuffer::new(None);
        assert_eq!(buffer.path(), None);
        assert_eq!(buffer.packet_count(), 0);
        assert_eq!(
            buffer.save(Path::new("unused.pcapng")).unwrap_err(),
            "raw capture is disabled"
        );
    }

    #[test]
    fn follow_up_pending_hits_are_bounded_and_recent_hits_still_resolve() {
        let characters = HashMap::from([
            (
                1,
                CharacterInfo {
                    name_zh: "character1".to_owned(),
                    name_en: String::new(),
                    color: None,
                    avatar: None,
                    attribute: Some("灵".to_owned()),
                },
            ),
            (
                2,
                CharacterInfo {
                    name_zh: "character2".to_owned(),
                    name_en: String::new(),
                    color: None,
                    avatar: None,
                    attribute: Some("咒".to_owned()),
                },
            ),
        ]);
        let mut tracker = FollowUpDamageTracker::default();
        tracker.observe_characters([1, 2], &characters);
        tracker.observe_fuwen_start_candidate(0.0, 1, 2, &characters);
        observe_visible_fuwen_trigger(&mut tracker, 1, 0.0);
        let mut hit = targetless_hit();
        hit.char_id = 1;
        hit.char_name = "character1".to_owned();
        hit.target_max_hp = 1_000_000.0;
        hit.target_hp_before = 1_000_000.0;
        hit.damage = 3_177.0;
        for index in 0..MAX_PENDING_FOLLOW_UP_HITS + 20 {
            hit.timestamp = index as f64 / 1_000.0;
            tracker.observe_hit(&hit, Some(241), &characters);
        }
        assert_eq!(tracker.pending_hits.len(), MAX_PENDING_FOLLOW_UP_HITS);

        hit.timestamp = 2.0;
        tracker.observe_hit(&hit, Some(241), &characters);
        assert_eq!(tracker.pending_hits.len(), 1);
        let follow_up = tracker
            .observe_server_hp(2.1, 996_150.0)
            .expect("recent pending hit should still resolve follow-up damage");
        assert_eq!(follow_up.damage, 673.0);
        assert_eq!(follow_up.source_char_id, 1);
        assert_eq!(follow_up.source_damage, 3_177.0);
        assert_eq!(follow_up.damage_name.as_deref(), Some("覆纹追加攻击"));
        assert_eq!(follow_up.attack_type.as_deref(), Some("覆纹"));
        assert_eq!(follow_up.damage_attribute.as_deref(), Some("灵"));
    }

    #[test]
    fn follow_up_requires_visible_fuwen_trigger() {
        let characters = follow_up_test_characters();
        let mut tracker = FollowUpDamageTracker::default();
        tracker.observe_characters([1, 2], &characters);
        let mut hit = targetless_hit();
        hit.char_id = 1;
        hit.target_max_hp = 1_000_000.0;
        hit.target_hp_before = 1_000_000.0;
        hit.damage = 1_000.0;

        tracker.observe_hit(&hit, None, &characters);

        assert!(tracker.observe_server_hp(0.1, 998_750.0).is_none());
    }

    #[test]
    fn exact_fuwen_damage_effect_claims_server_residual_without_ratio_guessing() {
        let characters = follow_up_test_characters();
        let mut tracker = FollowUpDamageTracker::default();
        tracker.observe_characters([1, 2], &characters);
        let mut hit = targetless_hit();
        hit.char_id = 1;
        hit.target_max_hp = 1_000_000.0;
        hit.target_hp_before = 1_000_000.0;
        hit.damage = 1_000.0;
        tracker.observe_hit(&hit, None, &characters);
        tracker.observe_fuwen_damage_effect(0.05);

        let follow_up = tracker
            .observe_server_hp(0.1, 997_500.0)
            .expect("the exact reaction damage GE should claim the server residual");

        assert_eq!(follow_up.damage, 1_500.0);
        assert_eq!(follow_up.attack_type.as_deref(), Some("覆纹"));
        assert_eq!(follow_up.damage_attribute.as_deref(), Some("灵"));
        assert_eq!(tracker.fuwen_damage_effect_at, None);
    }

    #[test]
    fn stale_fuwen_damage_effect_does_not_claim_later_server_residual() {
        let characters = follow_up_test_characters();
        let mut tracker = FollowUpDamageTracker::default();
        tracker.observe_characters([1, 2], &characters);
        tracker.observe_fuwen_damage_effect(0.0);
        let mut hit = targetless_hit();
        hit.timestamp = 1.5;
        hit.char_id = 1;
        hit.target_max_hp = 1_000_000.0;
        hit.target_hp_before = 1_000_000.0;
        hit.damage = 1_000.0;
        tracker.observe_hit(&hit, None, &characters);

        assert!(tracker.observe_server_hp(2.0, 997_500.0).is_none());
        assert_eq!(tracker.fuwen_damage_effect_at, None);
    }

    #[test]
    fn exact_fuwen_damage_effect_is_attached_to_direct_damage_record() {
        let names = HashMap::from([(555, FUWEN_DAMAGE_GAMEPLAY_EFFECT_NAME.to_owned())]);
        let effects = [ParsedGameplayEffect {
            unique_index: 555,
            byte_offset: 0,
            bit_shift: 0,
        }];
        let mut hit = targetless_hit();

        enrich_hit_with_gameplay_effect(
            &mut hit,
            &effects,
            &names,
            &AbilityCatalog::default(),
            None,
        );

        assert_eq!(hit.gameplay_effect_index, Some(555));
        assert_eq!(
            hit.gameplay_effect_name.as_deref(),
            Some(FUWEN_DAMAGE_GAMEPLAY_EFFECT_NAME)
        );
        assert_eq!(hit.attack_type.as_deref(), Some("覆纹"));
    }

    #[test]
    fn visible_fuwen_trigger_without_start_packet_records_follow_up() {
        let characters = follow_up_test_characters();
        let mut tracker = FollowUpDamageTracker::default();
        tracker.observe_characters([1, 2], &characters);
        observe_visible_fuwen_trigger(&mut tracker, 1, 0.0);
        let mut hit = targetless_hit();
        hit.char_id = 2;
        hit.target_max_hp = 1_000_000.0;
        hit.target_hp_before = 800_000.0;
        hit.damage = 1_000.0;

        tracker.observe_hit(&hit, None, &characters);
        let follow_up = tracker
            .observe_server_hp(0.1, 798_750.0)
            .expect("visible fuwen trigger should be enough to open follow-up tracking");
        assert_eq!(follow_up.damage, 250.0);
        assert_eq!(follow_up.damage_attribute.as_deref(), Some("咒"));
    }

    fn character_with_attribute(name_zh: &str, attribute: &str) -> CharacterInfo {
        CharacterInfo {
            name_zh: name_zh.to_owned(),
            name_en: String::new(),
            color: None,
            avatar: None,
            attribute: Some(attribute.to_owned()),
        }
    }

    #[test]
    fn orphan_dark_star_reaction_is_rehomed_to_dark_or_soul_owner() {
        let characters = HashMap::from([
            (1003, character_with_attribute("早雾", "咒")),
            (1004, character_with_attribute("安魂曲", "暗")),
            (1020, character_with_attribute("哈尼娅", "魂")),
        ]);
        // 暗 (安魂曲) is the most recently declared reaction participant.
        let declarations = HashMap::from([(1004_u32, 10.0_f64), (1020_u32, 8.0_f64)]);

        // 黯星 burst mis-credited to 早雾 (咒) because the packet declared 早雾.
        let mut orphan = targetless_hit();
        orphan.char_id = 1003;
        orphan.char_name = "早雾".to_owned();
        orphan.attack_type = Some("黯星".to_owned());
        reattribute_orphan_reaction(&mut orphan, &declarations, 10.001, &characters);
        assert_eq!(orphan.char_id, 1004);
        assert_eq!(orphan.char_name, "安魂曲");
        assert!(orphan.char_known);

        // Already on a 暗 character: untouched.
        let mut already_ok = targetless_hit();
        already_ok.char_id = 1004;
        already_ok.attack_type = Some("黯星".to_owned());
        reattribute_orphan_reaction(&mut already_ok, &declarations, 10.001, &characters);
        assert_eq!(already_ok.char_id, 1004);

        // No recent 暗/魂 declaration in window: left as-is rather than guessed.
        let mut stale = targetless_hit();
        stale.char_id = 1003;
        stale.attack_type = Some("黯星".to_owned());
        reattribute_orphan_reaction(&mut stale, &declarations, 99.0, &characters);
        assert_eq!(stale.char_id, 1003);

        // A non-attribute-locked reaction is never rehomed.
        let mut other = targetless_hit();
        other.char_id = 1003;
        other.attack_type = Some("普攻".to_owned());
        reattribute_orphan_reaction(&mut other, &declarations, 10.001, &characters);
        assert_eq!(other.char_id, 1003);
    }

    #[test]
    fn creation_flower_keeps_record_owner_across_different_reaction_partners() {
        let characters = HashMap::from([
            (1010, character_with_attribute("娜娜莉", "灵")),
            (1051, character_with_attribute("「零」", "光")),
            (1055, character_with_attribute("九原", "灵")),
        ]);
        let mut nanally_pair_flower = targetless_hit();
        nanally_pair_flower.char_id = 1010;
        nanally_pair_flower.char_name = "娜娜莉".to_owned();
        nanally_pair_flower.char_source = HitCharacterSource::Session;
        nanally_pair_flower.direction = HitDirection::Outgoing;
        nanally_pair_flower.attack_type = Some("创生花".to_owned());
        nanally_pair_flower.byte_offset = 253;
        nanally_pair_flower.bit_shift = 6;
        let nanally_pair_evidence = [(1051, 5, 157), (1051, 2, 368), (1010, 7, 640)];

        reattribute_hit_from_damage_record_owner(
            &mut nanally_pair_flower,
            &nanally_pair_evidence,
            DamageRecordEncoding::LegacyInt32,
            &characters,
        );

        let mut kuhara_pair_flower = targetless_hit();
        kuhara_pair_flower.char_id = 1055;
        kuhara_pair_flower.char_name = "九原".to_owned();
        kuhara_pair_flower.char_source = HitCharacterSource::Session;
        kuhara_pair_flower.direction = HitDirection::Outgoing;
        kuhara_pair_flower.attack_type = Some("创生花".to_owned());
        kuhara_pair_flower.byte_offset = 244;
        kuhara_pair_flower.bit_shift = 5;
        let kuhara_pair_evidence = [(1051, 4, 148), (1051, 1, 359), (1055, 5, 529)];

        reattribute_hit_from_damage_record_owner(
            &mut kuhara_pair_flower,
            &kuhara_pair_evidence,
            DamageRecordEncoding::LegacyInt32,
            &characters,
        );

        for hit in [nanally_pair_flower, kuhara_pair_flower] {
            assert_eq!(hit.char_id, 1051);
            assert_eq!(hit.char_name, "「零」");
            assert_eq!(hit.char_source, HitCharacterSource::Packet);
        }
    }

    #[test]
    fn damage_record_owner_requires_matching_anchors_and_outgoing_hit() {
        let characters = HashMap::from([
            (1010, character_with_attribute("娜娜莉", "灵")),
            (1051, character_with_attribute("「零」", "光")),
            (1055, character_with_attribute("九原", "灵")),
        ]);
        let mut flower = targetless_hit();
        flower.char_id = 1010;
        flower.char_name = "娜娜莉".to_owned();
        flower.char_source = HitCharacterSource::Session;
        flower.direction = HitDirection::Outgoing;
        flower.attack_type = Some("创生花".to_owned());
        flower.byte_offset = 253;
        flower.bit_shift = 6;

        reattribute_hit_from_damage_record_owner(
            &mut flower,
            &[(1051, 5, 157)],
            DamageRecordEncoding::LegacyInt32,
            &characters,
        );
        assert_eq!(flower.char_id, 1010);
        assert_eq!(flower.char_source, HitCharacterSource::Session);

        reattribute_hit_from_damage_record_owner(
            &mut flower,
            &[(1051, 5, 157), (1055, 2, 368)],
            DamageRecordEncoding::LegacyInt32,
            &characters,
        );
        assert_eq!(flower.char_id, 1010);
        assert_eq!(flower.char_source, HitCharacterSource::Session);

        flower.direction = HitDirection::Incoming;
        flower.attack_type = Some("Passive Damage".to_owned());
        reattribute_hit_from_damage_record_owner(
            &mut flower,
            &[(1051, 5, 157), (1051, 2, 368)],
            DamageRecordEncoding::LegacyInt32,
            &characters,
        );
        assert_eq!(flower.char_id, 1010);
        assert_eq!(flower.char_source, HitCharacterSource::Session);
    }

    #[test]
    fn damage_record_owner_separates_mixed_oneiroi_and_nanally_hits() {
        let characters = HashMap::from([
            (1010, character_with_attribute("娜娜莉", "灵")),
            (1075, character_with_attribute("伊洛伊", "灵")),
        ]);
        let evidence = [
            (1075, 4, 148),
            (1075, 1, 359),
            (1010, 6, 642),
            (1010, 3, 853),
        ];
        let mut oneiroi = targetless_hit();
        oneiroi.char_id = 1010;
        oneiroi.char_name = "娜娜莉".to_owned();
        oneiroi.char_source = HitCharacterSource::Session;
        oneiroi.direction = HitDirection::Outgoing;
        oneiroi.byte_offset = 244;
        oneiroi.bit_shift = 5;
        let mut nanally = targetless_hit();
        nanally.char_id = 1010;
        nanally.char_name = "娜娜莉".to_owned();
        nanally.char_source = HitCharacterSource::Session;
        nanally.direction = HitDirection::Outgoing;
        nanally.byte_offset = 738;
        nanally.bit_shift = 7;

        reattribute_hit_from_damage_record_owner(
            &mut oneiroi,
            &evidence,
            DamageRecordEncoding::LegacyInt32,
            &characters,
        );
        reattribute_hit_from_damage_record_owner(
            &mut nanally,
            &evidence,
            DamageRecordEncoding::LegacyInt32,
            &characters,
        );

        assert_eq!(oneiroi.char_id, 1075);
        assert_eq!(oneiroi.char_name, "伊洛伊");
        assert_eq!(oneiroi.char_source, HitCharacterSource::Packet);
        assert_eq!(nanally.char_id, 1010);
        assert_eq!(nanally.char_name, "娜娜莉");
        assert_eq!(nanally.char_source, HitCharacterSource::Packet);
    }

    #[test]
    fn damage_record_owner_overrides_unrelated_same_shift_packet_id() {
        let characters = HashMap::from([
            (1055, character_with_attribute("九原", "灵")),
            (1075, character_with_attribute("伊洛伊", "灵")),
        ]);
        let mut hit = targetless_hit();
        hit.char_id = 1075;
        hit.char_name = "伊洛伊".to_owned();
        hit.char_source = HitCharacterSource::Packet;
        hit.direction = HitDirection::Outgoing;
        hit.byte_offset = 244;
        hit.bit_shift = 5;
        let evidence = [(1055, 4, 148), (1055, 1, 359), (1075, 5, 650)];

        reattribute_hit_from_damage_record_owner(
            &mut hit,
            &evidence,
            DamageRecordEncoding::LegacyInt32,
            &characters,
        );

        assert_eq!(hit.char_id, 1055);
        assert_eq!(hit.char_name, "九原");
        assert_eq!(hit.char_source, HitCharacterSource::Packet);
    }

    #[test]
    fn bool_enum_damage_record_uses_updated_owner_anchors() {
        let characters = HashMap::from([
            (1004, character_with_attribute("安魂曲", "暗")),
            (1036, character_with_attribute("残虹", "热")),
        ]);
        let mut hit = targetless_hit();
        hit.char_id = 1036;
        hit.char_name = "残虹".to_owned();
        hit.char_source = HitCharacterSource::Packet;
        hit.direction = HitDirection::Outgoing;
        hit.byte_offset = 200;
        hit.bit_shift = 5;
        let evidence = [(1004, 4, 133), (1004, 2, 347)];

        reattribute_hit_from_damage_record_owner(
            &mut hit,
            &evidence,
            DamageRecordEncoding::BoolAndEnums,
            &characters,
        );

        assert_eq!(hit.char_id, 1004);
        assert_eq!(hit.char_name, "安魂曲");
        assert_eq!(hit.char_source, HitCharacterSource::Packet);
    }

    #[test]
    fn aoe_regression_targeted_false_incoming_uses_record_owner() {
        let characters = HashMap::from([
            (
                1020,
                CharacterInfo {
                    name_zh: "哈尼娅".to_owned(),
                    name_en: "Haniel".to_owned(),
                    color: None,
                    avatar: None,
                    attribute: Some("魂".to_owned()),
                },
            ),
            (1036, character_with_attribute("残虹", "热")),
        ]);
        let mut hit = targetless_hit();
        hit.char_id = 1036;
        hit.char_name = "残虹".to_owned();
        hit.char_source = HitCharacterSource::Packet;
        hit.direction = HitDirection::Incoming;
        hit.byte_offset = 172;
        hit.bit_shift = 4;
        hit.target_hp_before = 1_227_015.0;
        hit.target_hp_after = 1_224_057.0;
        hit.target_max_hp = 1_752_612.0;
        hit.gameplay_effect_name = Some("GE_Player_Haniel_UltraSkill_Extra_Damage".to_owned());
        set_wire_target(&mut hit, [0x17; 29]);
        let evidence = [(1020, 3, 105), (1020, 1, 319), (1036, 4, 714)];

        let mut mismatched_effect = hit.clone();
        mismatched_effect.gameplay_effect_name = Some("GE_Player_Zankou_Skill_Damage".to_owned());
        reattribute_hit_from_damage_record_owner(
            &mut mismatched_effect,
            &evidence,
            DamageRecordEncoding::BoolAndEnums,
            &characters,
        );
        assert_eq!(mismatched_effect.char_id, 1036);
        assert_eq!(mismatched_effect.direction, HitDirection::Incoming);

        reattribute_hit_from_damage_record_owner(
            &mut hit,
            &evidence,
            DamageRecordEncoding::BoolAndEnums,
            &characters,
        );

        assert_eq!(hit.char_id, 1020);
        assert_eq!(hit.char_name, "哈尼娅");
        assert_eq!(hit.char_source, HitCharacterSource::Packet);
        assert_eq!(hit.direction, HitDirection::Outgoing);
    }

    #[test]
    fn creation_flower_does_not_trigger_fuwen_follow_up() {
        let characters = follow_up_test_characters();
        let mut tracker = FollowUpDamageTracker::default();
        tracker.observe_characters([1, 2], &characters);
        observe_visible_fuwen_trigger(&mut tracker, 1, 0.0);
        let mut hit = targetless_hit();
        hit.char_id = 2;
        hit.target_max_hp = 1_000_000.0;
        hit.target_hp_before = 800_000.0;
        hit.damage = 1_000.0;
        hit.attack_type = Some("创生花".to_owned());

        tracker.observe_hit(&hit, None, &characters);

        assert!(tracker.observe_server_hp(0.1, 798_750.0).is_none());
        assert!(tracker.fuwen_active);
    }

    #[test]
    fn non_ling_zhou_hit_does_not_end_active_fuwen() {
        let characters = follow_up_test_characters();
        let mut tracker = FollowUpDamageTracker::default();
        tracker.observe_fuwen_start_candidate(0.0, 1, 2, &characters);
        observe_visible_fuwen_trigger(&mut tracker, 1, 0.0);
        let mut hit = targetless_hit();
        hit.target_max_hp = 1_000_000.0;

        hit.char_id = 3;
        hit.target_hp_before = 1_000_000.0;
        hit.damage = 500.0;
        tracker.observe_hit(&hit, None, &characters);
        assert!(tracker.observe_server_hp(0.1, 999_500.0).is_none());
        assert!(tracker.fuwen_active);

        hit.char_id = 1;
        hit.timestamp = 0.2;
        hit.target_hp_before = 999_500.0;
        hit.damage = 1_000.0;
        tracker.observe_hit(&hit, None, &characters);
        let follow_up = tracker
            .observe_server_hp(0.3, 998_250.0)
            .expect("ling/zhou residual should still be recorded after other-attribute hit");
        assert_eq!(follow_up.damage, 250.0);
    }

    #[test]
    fn zero_residual_does_not_end_active_fuwen_after_recorded_residual() {
        let characters = follow_up_test_characters();
        let mut tracker = FollowUpDamageTracker::default();
        tracker.observe_fuwen_start_candidate(0.0, 1, 2, &characters);
        observe_visible_fuwen_trigger(&mut tracker, 1, 0.0);
        let mut hit = targetless_hit();
        hit.char_id = 1;
        hit.target_max_hp = 1_000_000.0;
        hit.target_hp_before = 1_000_000.0;
        hit.damage = 1_000.0;

        tracker.observe_hit(&hit, None, &characters);

        assert!(tracker.observe_server_hp(0.1, 999_000.0).is_none());
        assert!(tracker.fuwen_active);

        hit.timestamp = 0.2;
        hit.target_hp_before = 999_000.0;
        hit.damage = 1_000.0;
        hit.attack_type = Some("普攻".to_owned());
        tracker.observe_hit(&hit, None, &characters);
        let follow_up = tracker
            .observe_server_hp(0.3, 997_750.0)
            .expect("first residual after fuwen start should be recorded");
        assert_eq!(follow_up.damage, 250.0);
        assert!(tracker.fuwen_active);

        hit.timestamp = 0.4;
        hit.target_hp_before = 997_750.0;
        hit.damage = 1_000.0;
        tracker.observe_hit(&hit, None, &characters);
        assert!(tracker.observe_server_hp(0.5, 996_750.0).is_none());
        assert!(tracker.fuwen_active);
    }

    #[test]
    fn fuwen_stays_active_across_long_gaps_until_battle_reset() {
        let characters = follow_up_test_characters();
        let mut tracker = FollowUpDamageTracker::default();
        tracker.observe_fuwen_start_candidate(0.0, 1, 2, &characters);
        observe_visible_fuwen_trigger(&mut tracker, 1, 0.0);
        let mut hit = targetless_hit();
        hit.char_id = 1;
        hit.target_max_hp = 1_000_000.0;
        hit.target_hp_before = 800_000.0;
        hit.damage = 1_000.0;

        hit.timestamp = 60.0;
        tracker.observe_hit(&hit, None, &characters);
        let follow_up = tracker
            .observe_server_hp(60.1, 798_750.0)
            .expect("fuwen follow-up should not expire just because of a long idle gap");
        assert_eq!(follow_up.damage, 250.0);
        assert!(tracker.fuwen_active);
    }

    #[test]
    fn hidden_fuwen_candidate_does_not_record_without_visible_trigger() {
        let characters = follow_up_test_characters();
        let mut tracker = FollowUpDamageTracker::default();
        tracker.observe_fuwen_start_candidate(0.0, 1, 2, &characters);
        let mut hit = targetless_hit();
        hit.timestamp = 5.0;
        hit.char_id = 2;
        hit.target_max_hp = 1_000_000.0;
        hit.target_hp_before = 1_000_000.0;
        hit.damage = 1_000.0;
        hit.attack_type = Some("普攻".to_owned());

        tracker.observe_hit(&hit, None, &characters);
        assert!(tracker.observe_server_hp(5.1, 999_000.0).is_none());
        assert!(!tracker.fuwen_active);
        assert!(tracker.fuwen_start_pending);

        hit.char_id = 1;
        hit.timestamp = 5.2;
        hit.target_hp_before = 999_000.0;
        hit.attack_type = Some("Q技能".to_owned());
        tracker.observe_hit(&hit, None, &characters);
        assert!(tracker.observe_server_hp(5.3, 998_000.0).is_none());
        assert!(!tracker.fuwen_active);
        assert!(tracker.fuwen_start_pending);
    }

    #[test]
    fn visible_fuwen_trigger_activates_follow_up_window() {
        let characters = follow_up_test_characters();
        let mut tracker = FollowUpDamageTracker::default();
        tracker.observe_characters([1, 2], &characters);
        observe_visible_fuwen_trigger(&mut tracker, 1, 5.0);

        assert!(tracker.fuwen_active);
        assert!(!tracker.fuwen_start_pending);
    }

    #[test]
    fn fuwen_start_pair_uses_shifted_signature_and_fixed_role_positions() {
        let mut payload = vec![0_u8; 90];
        write_shifted_bytes(
            &mut payload,
            FUWEN_START_SIGNATURE_SHIFT,
            FUWEN_START_SIGNATURE_OFFSET,
            FUWEN_START_SIGNATURE,
        );
        write_shifted_bytes(
            &mut payload,
            FUWEN_ENTERING_ID_SHIFT,
            FUWEN_ENTERING_ID_OFFSET,
            &character_evidence_row(1001),
        );
        write_shifted_bytes(
            &mut payload,
            FUWEN_PREVIOUS_ID_SHIFT,
            FUWEN_PREVIOUS_ID_OFFSET,
            &character_evidence_row(1002),
        );
        let evidence = find_declared_character_evidence(&payload);
        let characters = HashMap::from([
            (
                1001,
                CharacterInfo {
                    name_zh: "entering".to_owned(),
                    name_en: String::new(),
                    color: None,
                    avatar: None,
                    attribute: Some("灵".to_owned()),
                },
            ),
            (
                1002,
                CharacterInfo {
                    name_zh: "previous".to_owned(),
                    name_en: String::new(),
                    color: None,
                    avatar: None,
                    attribute: Some("咒".to_owned()),
                },
            ),
        ]);

        assert_eq!(
            fuwen_start_pair(&payload, &evidence, &characters),
            Some((1001, 1002))
        );
    }

    #[test]
    fn parses_export_ids_from_array_and_legacy_string() {
        assert_eq!(
            parse_export_ids(&serde_json::json!([1001, 1002])),
            [1001, 1002]
        );
        assert_eq!(
            parse_export_ids(&serde_json::json!("[1001, 1002]")),
            [1001, 1002]
        );
        assert!(parse_export_ids(&serde_json::json!([4294967296_u64])).is_empty());
    }

    #[test]
    fn final_tower_ids_merge_with_declared_ids() {
        let declared = [(1001, 0, 4)];
        let final_tower = [(1076, 5, 30), (1001, 5, 52)];

        assert_eq!(
            character_ids_from_evidence_sources(&declared, &final_tower),
            [1001, 1076]
        );
    }

    #[test]
    fn send_export_packet_rejects_invalid_hex_payload() {
        let (sender, receiver) = unbounded();
        let sender = EngineEventSink::reliable(sender);
        let mut empty_curtain = EmptyCurtainDecoder::new(EquipmentCatalog::default());
        let packet = ExportPacket {
            timestamp_unix: 1.0,
            time_local: String::new(),
            source: "127.0.0.1:1234".to_owned(),
            destination: "127.0.0.1:5678".to_owned(),
            direction: "S2C".to_owned(),
            payload_len: 0,
            declared_ids: serde_json::json!([1]),
            parsed_hits: 0,
            note: String::new(),
            payload_preview: String::new(),
            payload_hex: "ZZ".to_owned(),
            decoded_text: String::new(),
        };
        assert!(
            send_export_packet(packet, &sender, &mut empty_curtain)
                .unwrap_err()
                .contains("payload_hex 无效")
        );
        assert!(receiver.try_recv().is_err());
    }

    #[test]
    fn send_export_packet_adds_final_tower_ids_from_payload() {
        let (sender, receiver) = unbounded();
        let sender = EngineEventSink::reliable(sender);
        let mut empty_curtain = EmptyCurtainDecoder::new(EquipmentCatalog::default());
        let payload = b"FCharacterForNet....ft_character_1076".to_vec();
        let packet = ExportPacket {
            timestamp_unix: 1.0,
            time_local: String::new(),
            source: "127.0.0.1:1234".to_owned(),
            destination: "127.0.0.1:5678".to_owned(),
            direction: "C2S".to_owned(),
            payload_len: payload.len(),
            declared_ids: serde_json::json!([]),
            parsed_hits: 0,
            note: String::new(),
            payload_preview: hex::encode(&payload[..payload.len().min(48)]),
            payload_hex: hex::encode(&payload),
            decoded_text: String::new(),
        };

        assert!(send_export_packet(packet, &sender, &mut empty_curtain).is_ok());
        let packet = receive_observed_debug_packet(&receiver, 0);
        assert_eq!(packet.declared_ids, [1076]);
        assert!(packet.decoded_text.contains("ft_character_1076"));
        assert!(receiver.try_recv().is_err());
    }

    #[test]
    fn send_export_packet_accepts_empty_payload_hex() {
        let (sender, receiver) = unbounded();
        let sender = EngineEventSink::reliable(sender);
        let mut empty_curtain = EmptyCurtainDecoder::new(EquipmentCatalog::default());
        let packet = ExportPacket {
            timestamp_unix: 1.0,
            time_local: String::new(),
            source: "127.0.0.1:1234".to_owned(),
            destination: "127.0.0.1:5678".to_owned(),
            direction: "S2C".to_owned(),
            payload_len: 0,
            declared_ids: serde_json::json!([1]),
            parsed_hits: 0,
            note: String::new(),
            payload_preview: String::new(),
            payload_hex: String::new(),
            decoded_text: "测试解码".to_owned(),
        };
        assert!(send_export_packet(packet, &sender, &mut empty_curtain).is_ok());
        let packet = receive_observed_debug_packet(&receiver, 0);
        assert_eq!(packet.payload_hex, String::new());
        assert_eq!(packet.payload_len, 0);
        assert!(receiver.try_recv().is_err());
    }

    #[test]
    fn send_export_packet_preserves_exported_decoded_text() {
        let (sender, receiver) = unbounded();
        let sender = EngineEventSink::reliable(sender);
        let mut empty_curtain = EmptyCurtainDecoder::new(EquipmentCatalog::default());
        let packet = ExportPacket {
            timestamp_unix: 1.0,
            time_local: String::new(),
            source: "127.0.0.1:1234".to_owned(),
            destination: "127.0.0.1:5678".to_owned(),
            direction: "S2C".to_owned(),
            payload_len: 2,
            declared_ids: serde_json::json!([]),
            parsed_hits: 1,
            note: String::new(),
            payload_preview: "0000".to_owned(),
            payload_hex: "0000".to_owned(),
            decoded_text: "导出时的协议文本".to_owned(),
        };

        assert!(send_export_packet(packet, &sender, &mut empty_curtain).is_ok());
        let packet = receive_observed_debug_packet(&receiver, 1);
        assert_eq!(packet.decoded_text, "导出时的协议文本");
        assert_eq!(packet.payload_hex, "0000");
        assert!(receiver.try_recv().is_err());
    }

    #[test]
    fn summary_payload_text_retains_only_runtime_markers() {
        let irrelevant = decode_summary_payload_text(b"Some.DebugProtocolIdentifier");
        assert!(!irrelevant.has_readable_text);
        assert_eq!(irrelevant.text, UNREADABLE_PROTOCOL_TEXT);

        let abyss = decode_summary_payload_text(b"FAbyssGamePlayData ConditionState_Success");
        assert!(abyss.has_readable_text);
        assert!(abyss.text.contains("FAbyssGamePlayData"));
        assert!(abyss.text.contains("ConditionState_Success"));

        let ultra = decode_summary_payload_text(b"Event.Montage.Player.UltraSkillB");
        assert!(ultra.has_readable_text);
        assert_eq!(ultra.text, "UltraSkill");
    }

    #[test]
    fn summary_payload_text_is_bounded_to_canonical_markers() {
        let mut payload = vec![b'X'; CAPTURE_SNAPLEN as usize];
        payload[32_000..32_010].copy_from_slice(b"UltraSkill");

        let summary = decode_summary_payload_text(&payload);

        assert_eq!(summary.text, "UltraSkill");
        assert!(summary.text.len() < 256);
    }

    fn summary_marker_presence(text: &str) -> [bool; 8] {
        [
            text.contains("FAbyssGamePlayData"),
            text.contains("ConditionState_Success"),
            text.contains("EAbyssFightStage::FirstHalf"),
            text.contains("EAbyssFightStage::SecondHalf"),
            text.contains("AbyssClone"),
            text.contains("Abyss_Battle_Born"),
            text.contains("Abyss_Station_LeaveClone"),
            text.contains("UltraSkill"),
        ]
    }

    #[test]
    fn summary_payload_markers_match_full_debug_for_every_bit_shift() {
        let text = b"FAbyssGamePlayData ConditionState_Success EAbyssFightStage::SecondHalf AbyssCloneCharacterItemData Event.Montage.Player.UltraSkillB";

        for bit_shift in 0..8 {
            let mut payload = vec![0_u8; text.len() + 4];
            write_shifted_bytes(&mut payload, bit_shift, 1, text);

            let full = decode_payload_text_filtered(&payload, |value| {
                value.contains("Abyss")
                    || value.contains("ConditionState_Success")
                    || value.contains("UltraSkill")
            });
            let summary = decode_summary_payload_text(&payload);

            assert_eq!(
                summary_marker_presence(&summary.text),
                summary_marker_presence(&full.text),
                "marker parity failed at bit shift {bit_shift}"
            );
            assert_eq!(
                format!("{:?}", abyss_events_from_text(30.0, &summary.text)),
                format!("{:?}", abyss_events_from_text(30.0, &full.text)),
                "Abyss event parity failed at bit shift {bit_shift}"
            );
        }
    }

    #[test]
    fn summary_payload_scanner_handles_stream_boundaries_and_no_match() {
        for (bit_shift, marker, expected) in [
            (0, b"Abyss_Battle_Born".as_slice(), "Abyss_Battle_Born"),
            (
                3,
                b"ConditionState_Success".as_slice(),
                "ConditionState_Success",
            ),
            (7, b"UltraSkill".as_slice(), "UltraSkill"),
        ] {
            let mut payload = vec![0_u8; marker.len() + usize::from(bit_shift > 0)];
            write_shifted_bytes(&mut payload, bit_shift, 0, marker);
            let summary = decode_summary_payload_text(&payload);
            assert!(summary.text.contains(expected));
        }

        for payload in [
            Vec::new(),
            b"Abys".to_vec(),
            b"ConditionState_Succes".to_vec(),
            b"Event.Montage.Player.UltraSkil".to_vec(),
            vec![0xff; 257],
        ] {
            let summary = decode_summary_payload_text(&payload);
            assert_eq!(summary.text, UNREADABLE_PROTOCOL_TEXT);
            assert!(!summary.has_readable_text);
        }
    }

    #[test]
    fn summary_payload_explicit_stage_matches_full_debug() {
        let text = b"  Abyss_12_6_1  ";
        for bit_shift in 0..8 {
            let mut payload = vec![0_u8; text.len() + 3];
            write_shifted_bytes(&mut payload, bit_shift, 1, text);
            let full = decode_payload_text_filtered(&payload, |value| value.contains("Abyss"));
            let summary = decode_summary_payload_text(&payload);
            assert_eq!(
                format!("{:?}", abyss_events_from_text(40.0, &summary.text)),
                format!("{:?}", abyss_events_from_text(40.0, &full.text)),
                "explicit stage parity failed at bit shift {bit_shift}"
            );
        }
    }

    #[test]
    fn summary_payload_length_prefixed_stage_matches_full_debug() {
        let identifier = b"Abyss_12_6_1";
        let mut logical_payload = Vec::with_capacity(identifier.len() + 5);
        logical_payload.extend_from_slice(&((identifier.len() + 1) as u32).to_le_bytes());
        logical_payload.extend_from_slice(identifier);
        logical_payload.push(0);

        for bit_shift in 0..8 {
            let mut payload = vec![0_u8; logical_payload.len() + 3];
            write_shifted_bytes(&mut payload, bit_shift, 1, &logical_payload);
            let full = decode_payload_text_filtered(&payload, |value| value.contains("Abyss"));
            let summary = decode_summary_payload_text(&payload);
            assert_eq!(
                format!("{:?}", abyss_events_from_text(50.0, &summary.text)),
                format!("{:?}", abyss_events_from_text(50.0, &full.text)),
                "length-prefixed stage parity failed at bit shift {bit_shift}"
            );
        }
    }

    #[test]
    fn summary_payload_scanner_never_panics_on_untrusted_payloads() {
        for length in 0..=CAPTURE_SNAPLEN as usize {
            if length > 512 && length != CAPTURE_SNAPLEN as usize {
                continue;
            }
            let payload = (0..length)
                .map(|index| (index as u8).wrapping_mul(37).wrapping_add(length as u8))
                .collect::<Vec<_>>();
            let _ = decode_summary_payload_text(&payload);
        }
    }

    #[test]
    fn summary_only_emits_observation_without_debug_payload() {
        let payload = (0..128)
            .map(|index| if index % 2 == 0 { 0x80 } else { 0x01 })
            .collect::<Vec<_>>();
        let local_ip = Ipv4Addr::new(10, 0, 0, 2);
        let remote_ip = Ipv4Addr::new(10, 0, 0, 3);
        let packet = udp_ipv4_packet(&payload, local_ip, 50_000, remote_ip, 7_777);
        let characters = HashMap::new();

        let (full_sender, full_receiver) = unbounded();
        let full_sender = EngineEventSink::reliable(full_sender);
        PacketDecoder {
            packet_emission: PacketEmissionMode::FullDebug,
            ..PacketDecoder::default()
        }
        .process_ethernet_frame(
            &packet,
            FrameTimestamp::Known(10.0),
            Some(local_ip),
            true,
            &characters,
            &full_sender,
        );
        let full_events = full_receiver.try_iter().collect::<Vec<_>>();
        let full_packet = full_events.iter().find_map(|event| match event {
            EngineEvent::Packet(packet) => Some(packet),
            _ => None,
        });
        let full_packet = full_packet.expect("full mode should retain debug packet fields");
        assert_eq!(full_packet.payload_hex.len(), payload.len() * 2);
        assert!(!full_packet.payload_preview.is_empty());

        let (summary_sender, summary_receiver) = unbounded();
        let summary_sender = EngineEventSink::reliable(summary_sender);
        PacketDecoder {
            packet_emission: PacketEmissionMode::SummaryOnly,
            ..PacketDecoder::default()
        }
        .process_ethernet_frame(
            &packet,
            FrameTimestamp::Known(10.0),
            Some(local_ip),
            true,
            &characters,
            &summary_sender,
        );
        let summary_events = summary_receiver.try_iter().collect::<Vec<_>>();
        assert!(
            summary_events
                .iter()
                .any(|event| matches!(event, EngineEvent::PacketObservation(_)))
        );
        assert!(
            !summary_events
                .iter()
                .any(|event| matches!(event, EngineEvent::Packet(_)))
        );
    }

    #[test]
    fn frame_dedup_suppresses_only_identical_frames_within_window() {
        let mut dedup = FrameDedup::default();
        let frame = [0xde, 0xad, 0xbe, 0xef, 0x01, 0x02];

        assert!(!dedup.is_duplicate(&frame, Some(10.0)));
        assert!(dedup.is_duplicate(&frame, Some(10.000_05)));

        let mut different_frame = frame;
        different_frame[0] ^= 1;
        assert!(!dedup.is_duplicate(&different_frame, Some(10.000_06)));
        assert!(!dedup.is_duplicate(&frame, Some(10.002)));
    }

    #[test]
    fn frame_dedup_skips_unknown_timestamps_and_resets_on_regression() {
        let mut dedup = FrameDedup::default();
        let frame = [0xde, 0xad, 0xbe, 0xef];

        assert!(!dedup.is_duplicate(&frame, Some(10.0)));
        assert!(!dedup.is_duplicate(&frame, None));
        assert!(dedup.recent.is_empty());
        assert!(!dedup.is_duplicate(&frame, Some(10.000_05)));
        assert!(!dedup.is_duplicate(&frame, Some(f64::NAN)));
        assert!(dedup.recent.is_empty());

        assert!(!dedup.is_duplicate(&frame, Some(10.0)));
        assert!(!dedup.is_duplicate(&frame, Some(9.0)));
        assert!(dedup.is_duplicate(&frame, Some(9.000_05)));
    }

    #[test]
    fn frame_dedup_bounds_same_timestamp_cache() {
        let mut dedup = FrameDedup::default();

        for index in 0..=MAX_RECENT_CAPTURE_FRAMES {
            assert!(!dedup.is_duplicate(&index.to_le_bytes(), Some(10.0)));
        }

        assert_eq!(dedup.recent.len(), MAX_RECENT_CAPTURE_FRAMES);
    }

    #[test]
    fn frame_dedup_hash_collision_never_drops_a_distinct_frame() {
        let mut dedup = FrameDedup::default();
        let forced_fingerprint = 7;

        assert!(!dedup.is_duplicate_with_fingerprint(
            b"first frame",
            Some(10.0),
            forced_fingerprint,
        ));
        assert!(!dedup.is_duplicate_with_fingerprint(
            b"other frame",
            Some(10.000_01),
            forced_fingerprint,
        ));
        assert!(dedup.is_duplicate_with_fingerprint(
            b"first frame",
            Some(10.000_02),
            forced_fingerprint,
        ));
    }

    #[test]
    fn frame_dedup_bounds_collision_verification_bytes() {
        let mut dedup = FrameDedup::with_verification_byte_budget(8);

        assert!(!dedup.is_duplicate_with_fingerprint(b"123456", Some(10.0), 1));
        assert!(!dedup.is_duplicate_with_fingerprint(b"abcdef", Some(10.000_01), 2,));
        assert!(dedup.retained_verification_bytes() <= 8);
        assert_eq!(dedup.recent.len(), 2);
        assert!(
            !dedup.is_duplicate_with_fingerprint(b"123456", Some(10.000_02), 1),
            "an evicted verification body must cause a safe false negative, not hash-only dedup"
        );
        assert!(dedup.retained_verification_bytes() <= 8);
    }

    #[test]
    fn duplicate_capture_frame_is_not_decoded_twice() {
        let payload = (0..128)
            .map(|index| if index % 2 == 0 { 0x80 } else { 0x01 })
            .collect::<Vec<_>>();
        let local_ip = Ipv4Addr::new(10, 0, 0, 2);
        let remote_ip = Ipv4Addr::new(10, 0, 0, 3);
        let packet = udp_ipv4_packet(&payload, local_ip, 50_000, remote_ip, 7_777);
        let characters = HashMap::new();
        let mut decoder = PacketDecoder::default();
        let (sender, receiver) = unbounded();
        let sender = EngineEventSink::reliable(sender);

        decoder.process_ethernet_frame(
            &packet,
            FrameTimestamp::Known(10.0),
            Some(local_ip),
            true,
            &characters,
            &sender,
        );
        assert!(
            receiver.try_iter().count() > 0,
            "the original datagram should be decoded"
        );

        decoder.process_ethernet_frame(
            &packet,
            FrameTimestamp::Known(10.000_05),
            Some(local_ip),
            true,
            &characters,
            &sender,
        );
        assert_eq!(
            receiver.try_iter().count(),
            0,
            "a redundant duplicate must be dropped before it is decoded"
        );

        let mut retransmission = packet.clone();
        retransmission[18] ^= 1;
        decoder.process_ethernet_frame(
            &retransmission,
            FrameTimestamp::Known(10.000_1),
            Some(local_ip),
            true,
            &characters,
            &sender,
        );
        assert!(
            receiver.try_iter().count() > 0,
            "the same payload in a distinct network frame must be decoded"
        );

        decoder.process_ethernet_frame(
            &packet,
            FrameTimestamp::Known(10.002),
            Some(local_ip),
            true,
            &characters,
            &sender,
        );
        assert!(
            receiver.try_iter().count() > 0,
            "an identical frame past the window is a fresh transmission"
        );
    }

    #[test]
    fn abyss_success_packet_can_also_identify_second_half_stage() {
        let events = abyss_events_from_text(
            30.0,
            "FAbyssGamePlayData\nConditionState_Success\nEAbyssFightStage::SecondHalf\nAbyssCloneCharacterItemData",
        );
        assert_eq!(events.len(), 2);
        assert!(matches!(
            events[0],
            AbyssEvent::Stage {
                timestamp: 30.0,
                cycle: None,
                floor: None,
                half: AbyssHalf::Second,
                allow_late_backfill: true,
            }
        ));
        assert!(matches!(events[1], AbyssEvent::Success { timestamp: 30.0 }));
    }

    #[test]
    fn abyss_restart_packet_also_identifies_new_first_half_stage() {
        let events = abyss_events_from_text(
            40.0,
            "EAbyssFightStage::FirstHalf\nEAbyssFightStage::None\nFAbyssGamePlayData\nAbyss_Battle_Born\nAbyss_6",
        );

        assert_eq!(events.len(), 2);
        assert!(matches!(
            events[0],
            AbyssEvent::RestartDetected { timestamp: 40.0 }
        ));
        assert!(matches!(
            events[1],
            AbyssEvent::Stage {
                timestamp: 40.0,
                cycle: None,
                floor: None,
                half: AbyssHalf::First,
                allow_late_backfill: false,
            }
        ));
    }

    #[test]
    fn vehicle_hitout_is_outgoing_physical_damage() {
        let effects = [ParsedGameplayEffect {
            unique_index: 1845,
            byte_offset: 0,
            bit_shift: 0,
        }];
        let names = HashMap::from([(1845, "GE_Vehicle_HitOut2".to_owned())]);
        let mut hit = targetless_hit();
        hit.direction = HitDirection::Incoming;

        enrich_hit_with_gameplay_effect(
            &mut hit,
            &effects,
            &names,
            &AbilityCatalog::default(),
            None,
        );

        assert_eq!(hit.direction, HitDirection::Outgoing);
        assert_eq!(hit.attack_type.as_deref(), Some("载具伤害"));
        assert_eq!(hit.damage_attribute.as_deref(), Some("物理"));
    }

    #[test]
    fn exact_incoming_record_is_not_reversed_by_adjacent_player_effect() {
        let effects = [ParsedGameplayEffect {
            unique_index: 2031,
            byte_offset: 0,
            bit_shift: 0,
        }];
        let names = HashMap::from([(2031, "GE_Player_Hathor_QTE1_Damage".to_owned())]);
        let mut hit = targetless_hit();
        hit.direction = HitDirection::Incoming;

        enrich_hit_with_gameplay_effect(
            &mut hit,
            &effects,
            &names,
            &AbilityCatalog::default(),
            None,
        );

        assert_eq!(hit.direction, HitDirection::Incoming);
        assert_eq!(hit.attack_type.as_deref(), Some("环合"));
    }

    #[test]
    fn non_prefixed_player_skill_damage_effect_keeps_exact_incoming_direction() {
        let effects = [ParsedGameplayEffect {
            unique_index: 3010,
            byte_offset: 0,
            bit_shift: 0,
        }];
        let names = HashMap::from([(3010, "GE_Nanally010_Lv3_Damage".to_owned())]);
        let skills = HashMap::from([(
            "GE_Nanally010_Lv3_Damage".to_owned(),
            GameplayEffectSkill {
                damage_source_category: Some("A".to_owned()),
                ability_name: Some("GA_Nanally_Melee".to_owned()),
                attack_type: "普攻".to_owned(),
                damage_component: None,
                owner_character_id: None,
                use_server_damage: false,
                max_hp_reduction_percent: 0,
            },
        )]);
        let mut hit = targetless_hit();
        hit.direction = HitDirection::Incoming;

        enrich_hit_with_gameplay_effect(
            &mut hit,
            &effects,
            &names,
            &AbilityCatalog::from(skills),
            None,
        );

        assert_eq!(hit.direction, HitDirection::Incoming);
        assert_eq!(hit.attack_type.as_deref(), Some("普攻"));
    }

    #[test]
    fn numeric_ability_owner_overrides_final_tower_packet_id() {
        let characters = HashMap::from([
            (1019, character_with_attribute("薄荷", "灵")),
            (1076, character_with_attribute("真红", "光")),
        ]);
        let mut hit = targetless_hit();
        hit.char_id = 1076;
        hit.char_name = "真红".to_owned();
        hit.char_source = HitCharacterSource::Packet;
        hit.ability_name = Some("GA_Mint019_Skill".to_owned());

        reattribute_hit_from_ability_name(&mut hit, true, &characters);

        assert_eq!(hit.char_id, 1019);
        assert_eq!(hit.char_name, "薄荷");
        assert_eq!(hit.char_source, HitCharacterSource::GameplayEffect);
    }

    #[test]
    fn numeric_ability_owner_keeps_regular_packet_id() {
        let characters = HashMap::from([
            (1019, character_with_attribute("薄荷", "灵")),
            (1076, character_with_attribute("真红", "光")),
        ]);
        let mut hit = targetless_hit();
        hit.char_id = 1076;
        hit.char_name = "真红".to_owned();
        hit.char_source = HitCharacterSource::Packet;
        hit.ability_name = Some("GA_Mint019_Skill".to_owned());

        reattribute_hit_from_ability_name(&mut hit, false, &characters);

        assert_eq!(hit.char_id, 1076);
        assert_eq!(hit.char_name, "真红");
        assert_eq!(hit.char_source, HitCharacterSource::Packet);
    }

    #[test]
    fn exact_gameplay_effect_owner_overrides_regular_packet_id() {
        let characters = HashMap::from([
            (1010, character_with_attribute("娜娜莉", "咒")),
            (1051, character_with_attribute("零(女)", "光")),
        ]);
        let catalog = AbilityCatalog::from(HashMap::from([(
            "GE_Player_Nanally_UltraSkill3_Damage".to_owned(),
            GameplayEffectSkill {
                damage_source_category: Some("Q".to_owned()),
                ability_name: Some("GA_Nanally_UltraSkill".to_owned()),
                attack_type: "Q技能".to_owned(),
                damage_component: None,
                owner_character_id: Some(1010),
                use_server_damage: false,
                max_hp_reduction_percent: 0,
            },
        )]));
        let mut hit = targetless_hit();
        hit.char_id = 1051;
        hit.char_name = "零(女)".to_owned();
        hit.char_source = HitCharacterSource::Packet;
        hit.direction = HitDirection::Outgoing;
        hit.gameplay_effect_name = Some("GE_Player_Nanally_UltraSkill3_Damage".to_owned());

        reattribute_hit_from_gameplay_effect_semantics(&mut hit, &catalog, &characters);

        assert_eq!(hit.char_id, 1010);
        assert_eq!(hit.char_name, "娜娜莉");
        assert_eq!(hit.char_source, HitCharacterSource::GameplayEffect);
    }

    #[test]
    fn numeric_ability_owner_overrides_session_id() {
        let characters = HashMap::from([
            (1019, character_with_attribute("薄荷", "灵")),
            (1076, character_with_attribute("真红", "光")),
        ]);
        let mut hit = targetless_hit();
        hit.char_id = 1076;
        hit.char_name = "真红".to_owned();
        hit.char_source = HitCharacterSource::Session;
        hit.ability_name = Some("GA_Mint019_QTE".to_owned());

        reattribute_hit_from_ability_name(&mut hit, false, &characters);

        assert_eq!(hit.char_id, 1019);
        assert_eq!(hit.char_name, "薄荷");
        assert_eq!(hit.char_source, HitCharacterSource::GameplayEffect);
    }

    #[test]
    fn creation_flower_passive_keeps_reaction_settlement_owner() {
        let characters = HashMap::from([
            (1051, character_with_attribute("「零」", "光")),
            (1075, character_with_attribute("伊洛伊", "灵")),
        ]);
        let mut hit = targetless_hit();
        hit.char_id = 1051;
        hit.char_name = "「零」".to_owned();
        hit.char_source = HitCharacterSource::Session;
        hit.ability_name = Some("GA_Oneiroi_Passive_1".to_owned());
        hit.attack_type = Some("创生花".to_owned());

        reattribute_hit_from_ability_name(&mut hit, false, &characters);

        assert_eq!(hit.char_id, 1051);
        assert_eq!(hit.char_name, "「零」");
        assert_eq!(hit.char_source, HitCharacterSource::Session);
    }

    #[test]
    fn reaction_damage_effect_keeps_exact_incoming_direction() {
        let effects = [ParsedGameplayEffect {
            unique_index: 4010,
            byte_offset: 0,
            bit_shift: 0,
        }];
        let names = HashMap::from([(4010, "GE_ActorReaction_1_Damage".to_owned())]);
        let mut hit = targetless_hit();
        hit.direction = HitDirection::Incoming;

        enrich_hit_with_gameplay_effect(
            &mut hit,
            &effects,
            &names,
            &AbilityCatalog::default(),
            None,
        );

        assert_eq!(hit.direction, HitDirection::Incoming);
        assert_eq!(hit.attack_type.as_deref(), Some("创生花"));
    }

    #[test]
    fn multi_effect_packet_matches_each_damage_record_by_position() {
        let effects = [
            ParsedGameplayEffect {
                unique_index: 52,
                byte_offset: 78,
                bit_shift: 3,
            },
            ParsedGameplayEffect {
                unique_index: 3529,
                byte_offset: 561,
                bit_shift: 5,
            },
        ];
        let names = HashMap::from([
            (52, "GE_ActorReaction_1_Damage".to_owned()),
            (3529, "GE_Player_Kuhara_SeedReaction_Damage".to_owned()),
        ]);
        let catalog = AbilityCatalog::from(HashMap::from([(
            "GE_Player_Kuhara_SeedReaction_Damage".to_owned(),
            GameplayEffectSkill {
                damage_source_category: Some("A".to_owned()),
                ability_name: Some("GA_Kuhara_Passive_2".to_owned()),
                attack_type: "Passive Damage".to_owned(),
                damage_component: Some("Additional Settlement".to_owned()),
                owner_character_id: Some(1055),
                use_server_damage: false,
                max_hp_reduction_percent: 0,
            },
        )]));
        let mut creation_flower = targetless_hit();
        creation_flower.byte_offset = 244;
        creation_flower.bit_shift = 5;
        let mut seed_reaction = targetless_hit();
        seed_reaction.byte_offset = 727;
        seed_reaction.bit_shift = 7;

        enrich_hit_with_gameplay_effect(&mut creation_flower, &effects, &names, &catalog, None);
        enrich_hit_with_gameplay_effect(
            &mut seed_reaction,
            &effects,
            &names,
            &catalog,
            Some(creation_flower.byte_offset * 8 + usize::from(creation_flower.bit_shift)),
        );
        let characters = HashMap::from([(1055, character_with_attribute("无主幽灵", "暗"))]);
        reattribute_hit_from_gameplay_effect_semantics(&mut seed_reaction, &catalog, &characters);

        assert_eq!(creation_flower.gameplay_effect_index, Some(52));
        assert_eq!(
            creation_flower.gameplay_effect_name.as_deref(),
            Some("GE_ActorReaction_1_Damage")
        );
        assert_eq!(creation_flower.attack_type.as_deref(), Some("创生花"));
        assert_eq!(creation_flower.ability_name, None);
        assert_eq!(seed_reaction.gameplay_effect_index, Some(3529));
        assert_eq!(
            seed_reaction.gameplay_effect_name.as_deref(),
            Some("GE_Player_Kuhara_SeedReaction_Damage")
        );
        assert_eq!(
            seed_reaction.ability_name.as_deref(),
            Some("GA_Kuhara_Passive_2")
        );
        assert_eq!(
            seed_reaction.damage_component.as_deref(),
            Some("Additional Settlement")
        );
        assert_eq!(seed_reaction.attack_type.as_deref(), Some("Passive Damage"));
        assert_eq!(seed_reaction.char_id, 1055);
        assert_eq!(seed_reaction.char_name, "无主幽灵");
        assert_eq!(
            seed_reaction.char_source,
            HitCharacterSource::GameplayEffect
        );
    }

    #[test]
    fn compact_gameplay_effect_records_classify_the_preceding_damage() {
        let effects = [
            ParsedGameplayEffect {
                unique_index: 3983,
                byte_offset: 78,
                bit_shift: 6,
            },
            ParsedGameplayEffect {
                unique_index: 3983,
                byte_offset: 401,
                bit_shift: 7,
            },
            ParsedGameplayEffect {
                unique_index: 4579,
                byte_offset: 711,
                bit_shift: 6,
            },
        ];
        let names = HashMap::from([
            (3983, "GE_Player_Shinku_Skill1_2_Damage".to_owned()),
            (4579, "GE_Player_Shinku_WatchEx_Damage".to_owned()),
        ]);
        let catalog = AbilityCatalog::from(HashMap::from([
            (
                "GE_Player_Shinku_Skill1_2_Damage".to_owned(),
                GameplayEffectSkill {
                    damage_source_category: Some("E".to_owned()),
                    ability_name: Some("GA_Shinku_Skill".to_owned()),
                    attack_type: "E技能".to_owned(),
                    damage_component: None,
                    owner_character_id: None,
                    use_server_damage: false,
                    max_hp_reduction_percent: 0,
                },
            ),
            (
                "GE_Player_Shinku_WatchEx_Damage".to_owned(),
                GameplayEffectSkill {
                    damage_source_category: Some("A".to_owned()),
                    ability_name: Some("GA_Shinku_Passive_3".to_owned()),
                    attack_type: "Passive Damage".to_owned(),
                    damage_component: Some("Instant Strike Bonus".to_owned()),
                    owner_character_id: Some(1076),
                    use_server_damage: false,
                    max_hp_reduction_percent: 0,
                },
            ),
        ]));
        let mut first = targetless_hit();
        first.byte_offset = 245;
        let mut second = targetless_hit();
        second.byte_offset = 554;
        second.bit_shift = 7;
        let mut third = targetless_hit();
        third.byte_offset = 869;
        third.bit_shift = 6;

        enrich_hit_with_gameplay_effect(&mut first, &effects, &names, &catalog, None);
        enrich_hit_with_gameplay_effect(
            &mut second,
            &effects,
            &names,
            &catalog,
            Some(first.byte_offset * 8 + usize::from(first.bit_shift)),
        );
        enrich_hit_with_gameplay_effect(
            &mut third,
            &effects,
            &names,
            &catalog,
            Some(second.byte_offset * 8 + usize::from(second.bit_shift)),
        );

        assert_eq!(first.gameplay_effect_index, Some(3983));
        assert_eq!(second.gameplay_effect_index, Some(4579));
        assert_eq!(third.gameplay_effect_index, None);
        assert_eq!(
            second.gameplay_effect_name.as_deref(),
            Some("GE_Player_Shinku_WatchEx_Damage")
        );
        assert_eq!(second.ability_name.as_deref(), Some("GA_Shinku_Passive_3"));
        assert_eq!(second.attack_type.as_deref(), Some("Passive Damage"));
        assert_eq!(
            second.damage_component.as_deref(),
            Some("Instant Strike Bonus")
        );
    }

    #[test]
    fn repeated_watch_packets_match_each_trailing_compact_effect() {
        let cases = [
            (4072, (78, 6), (401, 7), 4579, (781, 6), (245, 0), (624, 7)),
            (3981, (78, 3), (401, 4), 4083, (733, 3), (244, 5), (576, 4)),
            (3917, (78, 3), (401, 4), 4083, (733, 3), (244, 5), (576, 4)),
            (3917, (78, 3), (401, 4), 4083, (725, 3), (244, 5), (568, 4)),
        ];

        for (
            main_index,
            main_full,
            main_compact,
            watch_index,
            watch_compact,
            first_position,
            second_position,
        ) in cases
        {
            let effects = [
                ParsedGameplayEffect {
                    unique_index: main_index,
                    byte_offset: main_full.0,
                    bit_shift: main_full.1,
                },
                ParsedGameplayEffect {
                    unique_index: main_index,
                    byte_offset: main_compact.0,
                    bit_shift: main_compact.1,
                },
                ParsedGameplayEffect {
                    unique_index: watch_index,
                    byte_offset: watch_compact.0,
                    bit_shift: watch_compact.1,
                },
            ];
            let mut first = targetless_hit();
            first.byte_offset = first_position.0;
            first.bit_shift = first_position.1;
            let mut second = targetless_hit();
            second.byte_offset = second_position.0;
            second.bit_shift = second_position.1;

            assert_eq!(
                matching_gameplay_effect(&first, &effects, None).map(|effect| effect.unique_index),
                Some(main_index)
            );
            assert_eq!(
                matching_gameplay_effect(
                    &second,
                    &effects,
                    Some(first.byte_offset * 8 + usize::from(first.bit_shift)),
                )
                .map(|effect| effect.unique_index),
                Some(watch_index)
            );
        }
    }

    #[test]
    fn bool_enum_active_gameplay_effect_is_not_matched_as_damage_record_effect() {
        let mut hit = targetless_hit();
        hit.byte_offset = 245;
        let effects = [ParsedGameplayEffect {
            unique_index: 3983,
            byte_offset: 92,
            bit_shift: 4,
        }];

        assert_eq!(
            matching_gameplay_effect(&hit, &effects, Some(100)).map(|effect| effect.unique_index),
            None
        );
    }

    #[test]
    fn bool_enum_damage_record_reads_trailing_gameplay_effect_at_both_layouts() {
        let names = HashMap::from([
            (3983, "GE_Player_Shinku_Skill1_2_Damage".to_owned()),
            (599, "GE_Player_Lacrimosa_Blood_Damage_LV6".to_owned()),
        ]);
        let catalog = AbilityCatalog::from(HashMap::from([
            (
                "GE_Player_Shinku_Skill1_2_Damage".to_owned(),
                GameplayEffectSkill {
                    damage_source_category: Some("E".to_owned()),
                    ability_name: Some("GA_Shinku_Skill".to_owned()),
                    attack_type: "E技能".to_owned(),
                    damage_component: None,
                    owner_character_id: None,
                    use_server_damage: false,
                    max_hp_reduction_percent: 0,
                },
            ),
            (
                "GE_Player_Lacrimosa_Blood_Damage_LV6".to_owned(),
                GameplayEffectSkill {
                    damage_source_category: Some("A".to_owned()),
                    ability_name: Some("GA_Lacrimosa_Passive".to_owned()),
                    attack_type: "被动伤害".to_owned(),
                    damage_component: None,
                    owner_character_id: Some(1004),
                    use_server_damage: false,
                    max_hp_reduction_percent: 0,
                },
            ),
        ]));
        let mut hit = targetless_hit();
        hit.byte_offset = 100;
        hit.bit_shift = 3;
        let hit_bit_offset = hit.byte_offset * 8 + usize::from(hit.bit_shift);

        let primary_bit_offset =
            hit_bit_offset + BOOL_ENUM_DAMAGE_RECORD_TO_GAMEPLAY_EFFECT_BITS[0];
        let mut primary_payload = vec![0; primary_bit_offset.div_ceil(8) + 5];
        write_shifted_bytes(
            &mut primary_payload,
            (primary_bit_offset % 8) as u8,
            primary_bit_offset / 8,
            &3983_u32.to_le_bytes(),
        );
        assert_eq!(
            matching_bool_enum_gameplay_effect(
                &primary_payload,
                &hit,
                DamageRecordEncoding::BoolAndEnums,
                &names,
                &catalog,
            )
            .map(|effect| effect.unique_index),
            Some(3983)
        );

        let alternate_bit_offset =
            hit_bit_offset + BOOL_ENUM_DAMAGE_RECORD_TO_GAMEPLAY_EFFECT_BITS[1];
        let mut alternate_payload = vec![0; alternate_bit_offset.div_ceil(8) + 5];
        write_shifted_bytes(
            &mut alternate_payload,
            (alternate_bit_offset % 8) as u8,
            alternate_bit_offset / 8,
            &599_u32.to_le_bytes(),
        );
        assert_eq!(
            matching_bool_enum_gameplay_effect(
                &alternate_payload,
                &hit,
                DamageRecordEncoding::BoolAndEnums,
                &names,
                &catalog,
            )
            .map(|effect| effect.unique_index),
            Some(599)
        );
        assert!(
            matching_bool_enum_gameplay_effect(
                &primary_payload,
                &hit,
                DamageRecordEncoding::LegacyInt32,
                &names,
                &catalog,
            )
            .is_none()
        );
    }

    #[test]
    fn damage_record_without_own_effect_does_not_reuse_previous_record_effect() {
        let effects = [ParsedGameplayEffect {
            unique_index: 3983,
            byte_offset: 78,
            bit_shift: 3,
        }];
        let names = HashMap::from([(3983, "GE_Player_Shinku_Skill1_2_Damage".to_owned())]);
        let mut first = targetless_hit();
        first.byte_offset = 244;
        first.bit_shift = 5;
        let mut second = targetless_hit();
        second.byte_offset = 500;

        enrich_hit_with_gameplay_effect(
            &mut first,
            &effects,
            &names,
            &AbilityCatalog::default(),
            None,
        );
        enrich_hit_with_gameplay_effect(
            &mut second,
            &effects,
            &names,
            &AbilityCatalog::default(),
            Some(first.byte_offset * 8 + usize::from(first.bit_shift)),
        );

        assert_eq!(first.gameplay_effect_index, Some(3983));
        assert_eq!(second.gameplay_effect_index, None);
    }

    #[test]
    fn reaction_buff_effect_keeps_exact_incoming_direction() {
        let effects = [ParsedGameplayEffect {
            unique_index: 4011,
            byte_offset: 0,
            bit_shift: 0,
        }];
        let names = HashMap::from([(4011, "Buff_Reaction_4_new".to_owned())]);
        let mut hit = targetless_hit();
        hit.direction = HitDirection::Incoming;

        enrich_hit_with_gameplay_effect(
            &mut hit,
            &effects,
            &names,
            &AbilityCatalog::default(),
            None,
        );

        assert_eq!(hit.direction, HitDirection::Incoming);
        assert_eq!(hit.attack_type.as_deref(), Some("黯星"));
    }

    #[test]
    fn tenacity_damage_effect_keeps_exact_incoming_direction() {
        let effects = [ParsedGameplayEffect {
            unique_index: 4012,
            byte_offset: 0,
            bit_shift: 0,
        }];
        let names = HashMap::from([(4012, "Buff_Tenacity_damage".to_owned())]);
        let mut hit = targetless_hit();
        hit.direction = HitDirection::Incoming;

        enrich_hit_with_gameplay_effect(
            &mut hit,
            &effects,
            &names,
            &AbilityCatalog::default(),
            None,
        );

        assert_eq!(hit.direction, HitDirection::Incoming);
        assert_eq!(hit.attack_type.as_deref(), Some("倾陷伤害"));
    }

    #[test]
    fn monster_damage_effect_overrides_outgoing_direction_to_incoming() {
        let effects = [ParsedGameplayEffect {
            unique_index: 5010,
            byte_offset: 0,
            bit_shift: 0,
        }];
        let names = HashMap::from([(5010, "GE_mon_25_act05_Dmg02_BP".to_owned())]);
        let mut hit = targetless_hit();
        hit.direction = HitDirection::Outgoing;

        enrich_hit_with_gameplay_effect(
            &mut hit,
            &effects,
            &names,
            &AbilityCatalog::default(),
            None,
        );

        assert_eq!(hit.direction, HitDirection::Incoming);
        assert_eq!(hit.attack_type.as_deref(), Some("其他"));
    }

    #[test]
    fn monster_hitout_effect_overrides_outgoing_direction_to_incoming() {
        let effects = [ParsedGameplayEffect {
            unique_index: 4724,
            byte_offset: 0,
            bit_shift: 0,
        }];
        let names = HashMap::from([(4724, "GE_mon_63_Hitout_600cm".to_owned())]);
        let mut hit = targetless_hit();
        hit.direction = HitDirection::Outgoing;

        enrich_hit_with_gameplay_effect(
            &mut hit,
            &effects,
            &names,
            &AbilityCatalog::default(),
            None,
        );

        assert_eq!(hit.direction, HitDirection::Incoming);
        assert_eq!(hit.attack_type.as_deref(), Some("其他"));
    }

    #[test]
    fn monster_hitout_effect_keeps_outgoing_direction_for_target_snapshot() {
        let effects = [ParsedGameplayEffect {
            unique_index: 4724,
            byte_offset: 0,
            bit_shift: 0,
        }];
        let names = HashMap::from([(4724, "GE_mon_63_Hitout_600cm".to_owned())]);
        let mut hit = targetless_hit();
        hit.direction = HitDirection::Outgoing;
        hit.target_hp_before = 3_268_491.0;
        hit.target_hp_after = 3_265_602.0;
        hit.target_max_hp = 3_514_714.0;

        enrich_hit_with_gameplay_effect(
            &mut hit,
            &effects,
            &names,
            &AbilityCatalog::default(),
            None,
        );

        assert_eq!(hit.direction, HitDirection::Outgoing);
        assert_eq!(
            hit.gameplay_effect_name.as_deref(),
            Some("GE_mon_63_Hitout_600cm")
        );
        assert_eq!(hit.attack_type.as_deref(), Some("其他"));
    }

    #[test]
    fn non_damage_buff_effect_does_not_become_hit_type() {
        let effects = [ParsedGameplayEffect {
            unique_index: 4832,
            byte_offset: 0,
            bit_shift: 0,
        }];
        let names = HashMap::from([(4832, "Buff_41_Remove".to_owned())]);
        let mut hit = targetless_hit();

        enrich_hit_with_gameplay_effect(
            &mut hit,
            &effects,
            &names,
            &AbilityCatalog::default(),
            None,
        );

        assert_eq!(hit.direction, HitDirection::Outgoing);
        assert_eq!(hit.gameplay_effect_index, None);
        assert_eq!(hit.gameplay_effect_name, None);
        assert_eq!(hit.attack_type, None);
    }

    #[test]
    fn boss_damage_effect_overrides_outgoing_direction_to_incoming() {
        let effects = [ParsedGameplayEffect {
            unique_index: 4116,
            byte_offset: 0,
            bit_shift: 0,
        }];
        let names = HashMap::from([(4116, "GE_boss_26_act16_Dmg02_BP".to_owned())]);
        let mut hit = targetless_hit();
        hit.direction = HitDirection::Outgoing;

        enrich_hit_with_gameplay_effect(
            &mut hit,
            &effects,
            &names,
            &AbilityCatalog::default(),
            None,
        );

        assert_eq!(hit.direction, HitDirection::Incoming);
        assert_eq!(hit.attack_type.as_deref(), Some("其他"));
    }

    #[test]
    fn monster_steal_damage_effect_does_not_force_incoming_direction() {
        let effects = [ParsedGameplayEffect {
            unique_index: 5010,
            byte_offset: 0,
            bit_shift: 0,
        }];
        let names = HashMap::from([(5010, "GE_mon_14_act05_Dmg01_Steal_BP".to_owned())]);
        let skills = HashMap::from([(
            "GE_mon_14_act05_Dmg01_Steal_BP".to_owned(),
            GameplayEffectSkill {
                damage_source_category: Some("E".to_owned()),
                ability_name: Some("GA_Lacrimosa_Skill".to_owned()),
                attack_type: "E技能".to_owned(),
                damage_component: None,
                owner_character_id: None,
                use_server_damage: false,
                max_hp_reduction_percent: 0,
            },
        )]);
        let mut hit = targetless_hit();
        hit.direction = HitDirection::Outgoing;

        enrich_hit_with_gameplay_effect(
            &mut hit,
            &effects,
            &names,
            &AbilityCatalog::from(skills),
            None,
        );

        assert_eq!(hit.direction, HitDirection::Outgoing);
        assert_eq!(hit.attack_type.as_deref(), Some("E技能"));
    }

    #[test]
    fn local_ip_hint_controls_import_direction_inference() {
        let local_ip = Ipv4Addr::new(10, 0, 0, 2);
        let remote_ip = Ipv4Addr::new(10, 0, 0, 3);
        let endpoints = HashSet::new();

        assert!(infer_outgoing(
            local_ip,
            50_000,
            remote_ip,
            40_000,
            Some(local_ip),
            &[],
            &endpoints,
        ));
        assert!(!infer_outgoing(
            remote_ip,
            40_000,
            local_ip,
            50_000,
            Some(local_ip),
            &[1001],
            &endpoints,
        ));
        assert!(!infer_outgoing(
            remote_ip,
            40_000,
            local_ip,
            50_000,
            None,
            &[1001],
            &endpoints,
        ));
        assert!(infer_outgoing(
            local_ip,
            50_000,
            remote_ip,
            40_000,
            None,
            &[],
            &endpoints,
        ));
    }

    #[test]
    fn replay_ignores_a_live_local_ip_hint_from_another_machine() {
        let capture_local_ip = Ipv4Addr::new(192, 168, 1, 77);
        let live_local_ip = Ipv4Addr::new(192, 168, 1, 99);
        let remote_ip = Ipv4Addr::new(1, 1, 1, 1);
        let packet = udp_ipv4_packet(&[], remote_ip, 7_777, capture_local_ip, 50_000);

        assert_eq!(
            replay_frame_local_ip_hint(CaptureLinkType::Ethernet, &packet, Some(live_local_ip)),
            None
        );
        assert_eq!(
            replay_frame_local_ip_hint(CaptureLinkType::Ethernet, &packet, Some(capture_local_ip)),
            Some(capture_local_ip)
        );
    }

    fn targetless_hit() -> Hit {
        Hit {
            timestamp: 0.0,
            char_id: 1,
            char_name: "test character".to_owned(),
            char_known: true,
            damage: 100.0,
            byte_offset: 0,
            bit_shift: 0,
            char_source: HitCharacterSource::Unknown,
            direction: HitDirection::Outgoing,
            target_hp_before: 0.0,
            target_hp_after: 0.0,
            target_max_hp: 0.0,
            max_hp_reduction: 0.0,
            target_hp_percent: 0.0,
            target_id: None,
            target_name: None,
            target_name_en: None,
            target_name_ja: None,
            target_monster_id: None,
            target_context: Vec::new(),
            gameplay_effect_index: None,
            gameplay_effect_name: None,
            ability_name: None,
            damage_name: None,
            damage_component: None,
            attack_type: None,
            damage_attribute: None,
            follow_up_damage: 0.0,
            follow_up_timestamp: None,
            follow_up_damage_name: None,
            follow_up_attack_type: None,
            follow_up_damage_attribute: None,
            reconciled_overkill_damage: None,
            wire_event: None,
        }
    }

    fn udp_ipv4_packet(
        payload: &[u8],
        src: Ipv4Addr,
        src_port: u16,
        dst: Ipv4Addr,
        dst_port: u16,
    ) -> Vec<u8> {
        let ip_len = 20 + 8 + payload.len();
        let udp_len = 8 + payload.len();
        let mut packet = Vec::with_capacity(14 + ip_len);
        packet.extend_from_slice(&[0, 1, 2, 3, 4, 5]);
        packet.extend_from_slice(&[6, 7, 8, 9, 10, 11]);
        packet.extend_from_slice(&0x0800_u16.to_be_bytes());
        packet.push(0x45);
        packet.push(0);
        packet.extend_from_slice(&(ip_len as u16).to_be_bytes());
        packet.extend_from_slice(&0_u16.to_be_bytes());
        packet.extend_from_slice(&0_u16.to_be_bytes());
        packet.push(64);
        packet.push(17);
        packet.extend_from_slice(&0_u16.to_be_bytes());
        packet.extend_from_slice(&src.octets());
        packet.extend_from_slice(&dst.octets());
        packet.extend_from_slice(&src_port.to_be_bytes());
        packet.extend_from_slice(&dst_port.to_be_bytes());
        packet.extend_from_slice(&(udp_len as u16).to_be_bytes());
        packet.extend_from_slice(&0_u16.to_be_bytes());
        packet.extend_from_slice(payload);
        packet
    }

    fn follow_up_test_characters() -> HashMap<u32, CharacterInfo> {
        HashMap::from([
            (
                1,
                CharacterInfo {
                    name_zh: "ling".to_owned(),
                    name_en: String::new(),
                    color: None,
                    avatar: None,
                    attribute: Some("灵".to_owned()),
                },
            ),
            (
                2,
                CharacterInfo {
                    name_zh: "zhou".to_owned(),
                    name_en: String::new(),
                    color: None,
                    avatar: None,
                    attribute: Some("咒".to_owned()),
                },
            ),
            (
                3,
                CharacterInfo {
                    name_zh: "other".to_owned(),
                    name_en: String::new(),
                    color: None,
                    avatar: None,
                    attribute: Some("光".to_owned()),
                },
            ),
        ])
    }

    fn observe_visible_fuwen_trigger(
        tracker: &mut FollowUpDamageTracker,
        character_id: u32,
        timestamp: f64,
    ) {
        let mut hit = targetless_hit();
        hit.char_id = character_id;
        hit.timestamp = timestamp;
        hit.attack_type = Some("环合·覆纹".to_owned());
        tracker.observe_fuwen_trigger_hit(&hit);
    }

    fn character_evidence_row(character_id: u32) -> [u8; 9] {
        let digits = format!("{character_id:04}");
        let mut row = [0_u8; 9];
        row[..4].copy_from_slice(&[5, 0, 0, 0]);
        row[4..8].copy_from_slice(digits.as_bytes());
        row
    }

    fn write_shifted_bytes(payload: &mut [u8], bit_shift: u8, byte_offset: usize, bytes: &[u8]) {
        for (index, byte) in bytes.iter().enumerate() {
            for bit in 0..8 {
                let bit_value = (byte >> bit) & 1;
                let target_bit = bit_shift as usize + (byte_offset + index) * 8 + bit;
                let target_byte = target_bit / 8;
                let target_bit_offset = target_bit % 8;
                if bit_value == 1 {
                    payload[target_byte] |= 1 << target_bit_offset;
                } else {
                    payload[target_byte] &= !(1 << target_bit_offset);
                }
            }
        }
    }

    fn duplicate_test_hit(timestamp: f64, char_source: HitCharacterSource, direction: &str) -> Hit {
        let mut hit = targetless_hit();
        hit.timestamp = timestamp;
        hit.char_id = if char_source == HitCharacterSource::Packet {
            1051
        } else {
            1010
        };
        hit.char_name = if char_source == HitCharacterSource::Packet {
            "零(女)".to_owned()
        } else {
            "娜娜莉".to_owned()
        };
        hit.damage = 5_829.0;
        hit.char_source = char_source;
        hit.direction = HitDirection::try_from(direction).expect("test direction must be valid");
        hit.target_hp_before = 1_389_577.0;
        hit.target_hp_after = 1_383_748.0;
        hit.target_max_hp = 1_930_389.0;
        hit.gameplay_effect_index = Some(52);
        hit.gameplay_effect_name = Some("GE_ActorReaction_1_Damage".to_owned());
        hit.attack_type = Some("创生花".to_owned());
        hit
    }

    fn duplicate_test_characters() -> HashMap<u32, CharacterInfo> {
        HashMap::from([
            (
                1010,
                CharacterInfo {
                    name_zh: "娜娜莉".to_owned(),
                    name_en: "Nanally".to_owned(),
                    color: None,
                    avatar: None,
                    attribute: Some("咒".to_owned()),
                },
            ),
            (
                1020,
                CharacterInfo {
                    name_zh: "哈尼娅".to_owned(),
                    name_en: "Haniel".to_owned(),
                    color: None,
                    avatar: None,
                    attribute: Some("光".to_owned()),
                },
            ),
            (
                1051,
                CharacterInfo {
                    name_zh: "零(女)".to_owned(),
                    name_en: "Rei".to_owned(),
                    color: None,
                    avatar: None,
                    attribute: Some("灵".to_owned()),
                },
            ),
            (
                1055,
                CharacterInfo {
                    name_zh: "测试角色".to_owned(),
                    name_en: "Test".to_owned(),
                    color: None,
                    avatar: None,
                    attribute: None,
                },
            ),
        ])
    }

    #[test]
    fn packet_decoder_loads_attack_resources_outside_project_cwd() {
        let decoder = PacketDecoder::default();

        assert!(
            decoder.resource_warnings.is_empty(),
            "{}",
            decoder.resource_warnings.join("; ")
        );
        assert_eq!(
            decoder.gameplay_effect_names.get(&241).map(String::as_str),
            Some("GE_Player_Nanally_Melee1_Damage")
        );
        assert_eq!(
            decoder
                .ability_catalog
                .skill("GE_Player_Nanally_Melee1_Damage")
                .map(|skill| skill.attack_type.as_str()),
            Some("普攻")
        );
        assert_eq!(
            decoder
                .ability_catalog
                .ability_name("GE_Player_Cang_Melee1_Damage"),
            Some("GA_Cang_Melee")
        );
    }

    fn boss_hp_update(timestamp_hp: f32) -> crate::engine::parser::ParsedBossHpUpdate {
        boss_hp_update_for([7; 29], timestamp_hp)
    }

    fn boss_hp_update_for(
        target_handle: [u8; 29],
        timestamp_hp: f32,
    ) -> crate::engine::parser::ParsedBossHpUpdate {
        crate::engine::parser::ParsedBossHpUpdate {
            target_handle,
            current_hp: timestamp_hp,
            byte_offset: 0,
            bit_shift: 0,
        }
    }

    fn server_damage_settlement_for(
        target_handle: [u8; 29],
        current_hp: f32,
        dead_state: u32,
        raw_damage: u32,
    ) -> ParsedServerDamageSettlement {
        ParsedServerDamageSettlement {
            target_handle,
            current_hp,
            dead_state,
            raw_damage,
            secondary_value: 0,
            byte_offset: 0,
            bit_shift: 0,
        }
    }

    fn set_wire_target(hit: &mut Hit, target_handle: [u8; 29]) {
        let target_id = target_id_from_wire_handle(&target_handle);
        hit.target_id = Some(target_id);
        hit.target_context = vec![format!("enemy_target_wire={}", hex::encode(target_handle))];
    }

    #[test]
    fn packet_hp_anchor_recovers_trailing_target_without_guessing_between_instances() {
        let mut decoder = PacketDecoder::default();
        let handle = [1_u8; 29];
        let mut first = targetless_hit();
        first.timestamp = 1.0;
        first.char_id = 1076;
        first.target_hp_before = 808_898.0;
        first.target_hp_after = 807_616.0;
        first.target_max_hp = 808_898.0;
        set_wire_target(&mut first, handle);
        let mut second = first.clone();
        second.target_hp_after = 799_816.0;
        let mut trailing = first.clone();
        trailing.damage = 126_190.0;
        trailing.target_hp_after = 682_708.0;
        trailing.target_id = None;
        trailing.target_context.clear();
        let mut hits = vec![first, second, trailing];

        decoder.resolve_and_observe_hit_targets(&mut hits);

        assert_eq!(hits[2].target_id, hits[0].target_id);
        assert_eq!(wire_handle_from_hit(&hits[2]), Some(handle));
    }

    #[test]
    fn exact_previous_hp_recovers_target_from_another_multi_target_packet() {
        let mut decoder = PacketDecoder::default();
        let boss = [7_u8; 29];
        let mut prior = targetless_hit();
        prior.timestamp = 1.0;
        prior.target_hp_before = 2_500_000.0;
        prior.target_hp_after = 2_446_555.0;
        prior.target_max_hp = 2_628_918.0;
        set_wire_target(&mut prior, boss);
        decoder.resolve_and_observe_hit_targets(std::slice::from_mut(&mut prior));

        let mut missing = targetless_hit();
        missing.timestamp = 2.0;
        missing.target_hp_before = 2_446_555.0;
        missing.target_hp_after = 2_442_521.0;
        missing.target_max_hp = 2_628_918.0;
        decoder.resolve_and_observe_hit_targets(std::slice::from_mut(&mut missing));

        assert_eq!(wire_handle_from_hit(&missing), Some(boss));
    }

    #[test]
    fn unanimous_multi_target_batch_skill_reaches_trailing_damage() {
        let mut first = targetless_hit();
        first.char_id = 1075;
        first.damage = 9_037.0;
        first.gameplay_effect_index = Some(100);
        first.gameplay_effect_name = Some("GE_Player_Oneiroi_Skill_Trumpet_Damage".to_owned());
        first.ability_name = Some("GA_Oneiroi_Skill".to_owned());
        first.attack_type = Some("E技能".to_owned());
        let second = first.clone();
        let mut trailing = first.clone();
        trailing.damage = 4_034.0;
        trailing.gameplay_effect_index = None;
        trailing.gameplay_effect_name = None;
        trailing.ability_name = None;
        trailing.attack_type = None;
        let mut hits = vec![first, second, trailing];

        propagate_unanimous_trailing_skill(&mut hits);

        assert_eq!(hits[2].gameplay_effect_index, Some(100));
        assert_eq!(hits[2].attack_type.as_deref(), Some("E技能"));
    }

    #[test]
    fn reassembled_typed_hit_resolves_exact_earlier_untyped_record() {
        let mut decoder = PacketDecoder::default();
        let characters = duplicate_test_characters();
        let mut untyped = duplicate_test_hit(10.0, HitCharacterSource::Packet, "outgoing");
        untyped.char_id = 1051;
        untyped.gameplay_effect_index = None;
        untyped.gameplay_effect_name = None;
        untyped.ability_name = None;
        untyped.attack_type = None;
        set_wire_target(&mut untyped, [1; 29]);
        let first = decoder.prepare_hits_for_emission(vec![untyped], &[1051], false, &characters);
        assert!(first.emit.is_empty());
        assert_eq!(first.deferred_ambiguous, 1);

        let mut confirmed = duplicate_test_hit(10.01, HitCharacterSource::Packet, "outgoing");
        confirmed.char_id = 1051;
        set_wire_target(&mut confirmed, [1; 29]);
        let resolved = decoder.resolve_pending_untyped_skills(&[confirmed]);

        assert_eq!(resolved.len(), 1);
        assert_eq!(resolved[0].gameplay_effect_index, Some(52));
        assert_eq!(wire_handle_from_hit(&resolved[0]), Some([1; 29]));
    }

    #[test]
    fn aoe_regression_reassembled_pending_uses_exact_wire_event_target() {
        let mut decoder = PacketDecoder::default();
        let first_event = crate::engine::model::DamageWireEvent {
            damage: 5_910.0,
            target_hp_before: 654_462.0,
            target_max_hp: 808_898.0,
            damage_time: 16_977_752.810_147_6,
            world_time: 0.0,
            repeated_damage: 5_910.0,
            state_flags: [0, 1, 0],
            trailing_value: 0.0,
        };
        let third_event = crate::engine::model::DamageWireEvent {
            damage_time: 16_977_752.810_758_2,
            ..first_event
        };
        let mut pending = duplicate_test_hit(10.0, HitCharacterSource::Packet, "outgoing");
        pending.damage = 5_910.0;
        pending.target_hp_before = 654_462.0;
        pending.target_hp_after = 648_552.0;
        pending.target_max_hp = 808_898.0;
        pending.gameplay_effect_index = None;
        pending.gameplay_effect_name = None;
        pending.ability_name = None;
        pending.attack_type = None;
        pending.wire_event = Some(third_event);
        decoder.pending_ambiguous_hits.push(pending);

        let mut first = duplicate_test_hit(10.01, HitCharacterSource::Packet, "outgoing");
        first.damage = 5_910.0;
        first.target_hp_before = 654_462.0;
        first.target_hp_after = 648_552.0;
        first.target_max_hp = 808_898.0;
        first.wire_event = Some(first_event);
        set_wire_target(&mut first, [1; 29]);
        let mut third = first.clone();
        third.wire_event = Some(third_event);
        set_wire_target(&mut third, [3; 29]);

        let resolved = decoder.resolve_pending_untyped_skills(&[first, third]);

        assert_eq!(resolved.len(), 1);
        assert_eq!(wire_handle_from_hit(&resolved[0]), Some([3; 29]));
    }

    #[test]
    fn reassembled_batch_emits_only_target_not_seen_in_fragments() {
        let mut decoder = PacketDecoder::default();
        let mut first = duplicate_test_hit(10.0, HitCharacterSource::Packet, "outgoing");
        set_wire_target(&mut first, [1; 29]);
        decoder.recent_confirmed_hits.push(first.clone());
        let mut second = first.clone();
        set_wire_target(&mut second, [2; 29]);
        let mut third = first.clone();
        set_wire_target(&mut third, [3; 29]);

        let recovered = decoder.take_new_reassembled_hits(
            vec![first, second.clone(), third.clone()],
            &[second],
            10.0,
        );

        assert_eq!(recovered.len(), 1);
        assert_eq!(wire_handle_from_hit(&recovered[0]), Some([3; 29]));
    }

    #[test]
    fn multiple_reassembled_batches_merge_without_overwriting_packet_hits() {
        let mut packet_hit = duplicate_test_hit(10.0, HitCharacterSource::Packet, "outgoing");
        set_wire_target(&mut packet_hit, [1; 29]);
        let mut duplicate = packet_hit.clone();
        duplicate.timestamp = 10.01;
        let mut first_reassembled = packet_hit.clone();
        set_wire_target(&mut first_reassembled, [2; 29]);
        let mut second_reassembled = packet_hit.clone();
        set_wire_target(&mut second_reassembled, [3; 29]);
        let mut hits = vec![packet_hit];

        extend_unique_exact_wire_hits(&mut hits, [duplicate, first_reassembled]);
        extend_unique_exact_wire_hits(&mut hits, [second_reassembled]);

        assert_eq!(hits.len(), 3);
        assert_eq!(wire_handle_from_hit(&hits[0]), Some([1; 29]));
        assert_eq!(wire_handle_from_hit(&hits[1]), Some([2; 29]));
        assert_eq!(wire_handle_from_hit(&hits[2]), Some([3; 29]));
    }

    #[test]
    fn exact_wire_event_deduplicates_across_conflicting_target_handles() {
        let mut packet_hit = duplicate_test_hit(10.0, HitCharacterSource::Packet, "outgoing");
        packet_hit.wire_event = Some(crate::engine::model::DamageWireEvent {
            damage: packet_hit.damage as f32,
            target_hp_before: packet_hit.target_hp_before as f32,
            target_max_hp: packet_hit.target_max_hp as f32,
            damage_time: 16_933_585.899_492_1,
            world_time: 151.193_88,
            repeated_damage: packet_hit.damage as f32,
            state_flags: [0, 1, 0],
            trailing_value: 0.0,
        });
        set_wire_target(&mut packet_hit, [1; 29]);
        let mut attributed = packet_hit.clone();
        attributed.timestamp = 10.001;
        attributed.char_source = HitCharacterSource::GameplayEffect;
        set_wire_target(&mut attributed, [2; 29]);
        let mut later_overlap = attributed.clone();
        later_overlap.timestamp = 10.05;
        later_overlap.wire_event.as_mut().unwrap().damage_time += 0.05;
        let mut hits = vec![packet_hit];

        extend_unique_exact_wire_hits(&mut hits, [attributed, later_overlap]);

        assert_eq!(hits.len(), 2);
        assert_eq!(wire_handle_from_hit(&hits[0]), Some([1; 29]));
        assert_eq!(wire_handle_from_hit(&hits[1]), Some([2; 29]));
    }

    #[test]
    fn reassembled_batch_does_not_consume_same_hit_from_previous_chain() {
        let mut decoder = PacketDecoder::default();
        let mut previous = duplicate_test_hit(9.99, HitCharacterSource::Packet, "outgoing");
        set_wire_target(&mut previous, [1; 29]);
        decoder.recent_confirmed_hits.push(previous);
        let mut confirmed = duplicate_test_hit(10.01, HitCharacterSource::Packet, "outgoing");
        set_wire_target(&mut confirmed, [1; 29]);

        let recovered = decoder.take_new_reassembled_hits(vec![confirmed], &[], 10.0);

        assert_eq!(recovered.len(), 1);
        assert_eq!(wire_handle_from_hit(&recovered[0]), Some([1; 29]));
    }

    #[test]
    fn reassembled_batch_deduplicates_rebound_target_for_same_wire_event() {
        let mut decoder = PacketDecoder::default();
        let mut emitted = duplicate_test_hit(10.0, HitCharacterSource::Packet, "outgoing");
        emitted.wire_event = Some(crate::engine::model::DamageWireEvent {
            damage: emitted.damage as f32,
            target_hp_before: emitted.target_hp_before as f32,
            target_max_hp: emitted.target_max_hp as f32,
            damage_time: 16_933_585.899_492_1,
            world_time: 151.193_88,
            repeated_damage: emitted.damage as f32,
            state_flags: [0, 1, 0],
            trailing_value: 0.0,
        });
        set_wire_target(&mut emitted, [1; 29]);
        decoder.recent_confirmed_hits.push(emitted.clone());
        let mut reassembled = emitted;
        reassembled.timestamp = 10.001;
        reassembled.char_source = HitCharacterSource::GameplayEffect;
        set_wire_target(&mut reassembled, [2; 29]);

        let recovered = decoder.take_new_reassembled_hits(vec![reassembled], &[], 10.0);

        assert!(recovered.is_empty());
    }

    #[test]
    fn reassembled_batch_fills_exact_target_on_deferred_targetless_hit() {
        let mut decoder = PacketDecoder::default();
        let candidate = duplicate_test_hit(10.0, HitCharacterSource::Packet, "outgoing");
        assert!(candidate.target_id.is_none());
        decoder.pending_ambiguous_hits.push(candidate);
        let mut confirmed = duplicate_test_hit(10.01, HitCharacterSource::Packet, "outgoing");
        set_wire_target(&mut confirmed, [7; 29]);

        let resolved = decoder.resolve_pending_untyped_skills(&[confirmed]);

        assert_eq!(resolved.len(), 1);
        assert_eq!(wire_handle_from_hit(&resolved[0]), Some([7; 29]));
    }

    #[test]
    fn client_damage_boss_semantics_does_not_change_generic_enemy_kind() {
        let mut decoder = PacketDecoder::default();
        let boss = [7_u8; 29];
        decoder.observe_target_hp_update(1.0, &boss_hp_update_for(boss, 2_000.0));
        let mut hit = targetless_hit();
        set_wire_target(&mut hit, boss);

        decoder.resolve_and_observe_hit_targets(std::slice::from_mut(&mut hit));

        assert!(
            !hit.target_context
                .iter()
                .any(|value| value.starts_with("target_kind="))
        );
    }

    #[test]
    fn client_fight_target_update_bootstraps_exact_target_snapshot() {
        let mut decoder = PacketDecoder::default();
        let target = [3_u8; 29];
        decoder.observe_target_hp_update(1.0, &boss_hp_update_for(target, 2_000.0));
        let mut hit = targetless_hit();
        hit.timestamp = 1.01;
        hit.target_hp_before = 2_000.0;
        hit.target_hp_after = 1_900.0;
        hit.target_max_hp = 2_500.0;

        decoder.resolve_and_observe_hit_targets(std::slice::from_mut(&mut hit));

        assert_eq!(wire_handle_from_hit(&hit), Some(target));
        assert!(hit.target_context.is_empty());
    }

    #[test]
    fn server_target_response_resolves_one_exact_pending_hit() {
        let mut decoder = PacketDecoder::default();
        let target = [4_u8; 29];
        let mut hit = targetless_hit();
        hit.timestamp = 10.0;
        hit.target_hp_before = 2_000.0;
        hit.target_hp_after = 1_900.0;
        hit.target_max_hp = 2_500.0;
        decoder.pending_targetless_hits.push_back(hit);
        let update = boss_hp_update_for(target, 1_900.0);
        decoder.observe_target_hp_update(10.1, &update);

        let resolved = decoder.resolve_pending_hit_targets(10.1, &[update]);

        assert_eq!(resolved.len(), 1);
        assert_eq!(wire_handle_from_hit(&resolved[0]), Some(target));
        assert!(decoder.pending_targetless_hits.is_empty());
    }

    #[test]
    fn equal_hp_on_multiple_targets_or_hits_remains_unresolved() {
        let mut multiple_targets = PacketDecoder::default();
        let mut hit = targetless_hit();
        hit.timestamp = 10.0;
        hit.target_hp_after = 1_900.0;
        multiple_targets.pending_targetless_hits.push_back(hit);
        let first = boss_hp_update_for([4_u8; 29], 1_900.0);
        let second = boss_hp_update_for([5_u8; 29], 1_900.0);
        multiple_targets.observe_target_hp_update(10.1, &first);
        multiple_targets.observe_target_hp_update(10.1, &second);

        assert!(
            multiple_targets
                .resolve_pending_hit_targets(10.1, &[first, second])
                .is_empty()
        );
        assert_eq!(multiple_targets.pending_targetless_hits.len(), 1);

        let mut multiple_hits = PacketDecoder::default();
        for timestamp in [10.0, 10.01] {
            let mut hit = targetless_hit();
            hit.timestamp = timestamp;
            hit.target_hp_after = 1_900.0;
            multiple_hits.pending_targetless_hits.push_back(hit);
        }
        let update = boss_hp_update_for([4_u8; 29], 1_900.0);
        multiple_hits.observe_target_hp_update(10.1, &update);

        assert!(
            multiple_hits
                .resolve_pending_hit_targets(10.1, &[update])
                .is_empty()
        );
        assert_eq!(multiple_hits.pending_targetless_hits.len(), 2);
    }

    #[test]
    fn unresolved_targetless_hit_expires_without_fabricated_target() {
        let mut decoder = PacketDecoder::default();
        let mut hit = targetless_hit();
        hit.timestamp = 10.0;
        hit.target_hp_after = 1_900.0;
        decoder.pending_targetless_hits.push_back(hit);

        assert!(decoder.take_expired_targetless_hits(10.5).is_empty());
        let expired = decoder.take_expired_targetless_hits(10.51);

        assert_eq!(expired.len(), 1);
        assert!(expired[0].target_id.is_none());
        assert!(decoder.pending_targetless_hits.is_empty());
    }

    #[test]
    fn emission_drops_only_targeted_outgoing_death_settlement_marker() {
        let mut decoder = PacketDecoder::default();
        let characters = HashMap::new();
        let (sender, receiver) = bounded(4);
        let sink = EngineEventSink::reliable(sender);
        let mut marker = targetless_hit();
        marker.damage = 1.0;
        marker.target_hp_before = 1.0;
        marker.target_hp_after = 0.0;
        marker.target_max_hp = 100.0;
        set_wire_target(&mut marker, [6_u8; 29]);

        decoder.emit_hits(std::iter::once(marker.clone()), &characters, &sink);
        assert!(receiver.try_recv().is_err());

        marker.direction = HitDirection::Incoming;
        marker.target_id = None;
        marker.target_context.clear();
        decoder.emit_hits(std::iter::once(marker), &characters, &sink);
        assert!(matches!(receiver.try_recv(), Ok(EngineEvent::Hit(_))));
    }

    #[test]
    fn server_damage_settlement_marks_partial_and_post_terminal_overkill() {
        let target = [7_u8; 29];
        let mut tracker = ServerDamageCalibrationTracker::default();
        let mut lethal = duplicate_test_hit(10.0, HitCharacterSource::Packet, "outgoing");
        lethal.damage = 11_662.0;
        lethal.target_hp_before = 7_086.0;
        lethal.target_hp_after = 0.0;
        lethal.target_max_hp = 808_898.0;
        set_wire_target(&mut lethal, target);
        tracker.observe_hit(&lethal);

        let (correction, unattributed) = tracker.observe_server_damage_settlement(
            10.05,
            &server_damage_settlement_for(target, 1.0, 1, 11_662),
        );
        let correction = correction.expect("lethal settlement should match the outgoing hit");
        assert!(unattributed.is_none());
        assert_eq!(correction.damage, 11_662.0);
        assert_eq!(correction.target_hp_before, 7_086.0);
        assert_eq!(correction.target_hp_after, 0.0);
        assert_eq!(correction.reconciled_overkill_damage, Some(4_576.0));

        let mut post_terminal = lethal;
        post_terminal.timestamp = 10.06;
        post_terminal.damage = 4_256.0;
        post_terminal.target_hp_before = 1.0;
        tracker.observe_hit(&post_terminal);
        let (correction, unattributed) = tracker.observe_server_damage_settlement(
            10.10,
            &server_damage_settlement_for(target, 1.0, 1, 4_256),
        );
        assert!(unattributed.is_none());
        assert_eq!(
            correction.and_then(|row| row.reconciled_overkill_damage),
            Some(4_256.0)
        );
    }

    #[test]
    fn server_authoritative_special_damage_keeps_raw_value_and_max_hp_reduction() {
        let target = [8_u8; 29];
        let mut tracker = ServerDamageCalibrationTracker::default();
        let mut nightmare = duplicate_test_hit(10.0, HitCharacterSource::Packet, "outgoing");
        nightmare.damage = 800.0;
        nightmare.target_hp_before = 800.0;
        nightmare.target_hp_after = 0.0;
        nightmare.target_max_hp = 10_000.0;
        set_wire_target(&mut nightmare, target);
        let _ = tracker.observe_hit_with_semantics(&nightmare, true, 200);

        let (correction, unattributed) = tracker.observe_server_damage_settlement(
            10.05,
            &server_damage_settlement_for(target, 0.0, 1, 1_000),
        );

        let correction = correction.expect("server-authoritative damage should be corrected");
        assert!(unattributed.is_none());
        assert_eq!(correction.damage, 1_000.0);
        assert_eq!(correction.reconciled_overkill_damage, Some(0.0));
        assert_eq!(correction.max_hp_reduction, Some(2_000.0));
    }

    #[test]
    fn delayed_target_max_hp_drop_updates_the_pending_special_damage_hit() {
        let target = [8_u8; 29];
        let mut tracker = ServerDamageCalibrationTracker::default();

        let mut baseline = duplicate_test_hit(9.0, HitCharacterSource::Packet, "outgoing");
        baseline.target_max_hp = 10_000.0;
        set_wire_target(&mut baseline, target);
        tracker.observe_hit(&baseline);

        let mut nightmare = duplicate_test_hit(10.0, HitCharacterSource::Packet, "outgoing");
        nightmare.damage = 800.0;
        nightmare.target_hp_before = 8_000.0;
        nightmare.target_hp_after = 7_200.0;
        nightmare.target_max_hp = 10_000.0;
        set_wire_target(&mut nightmare, target);
        let _ = tracker.observe_hit_with_semantics(&nightmare, true, 200);

        let mut later_hit = duplicate_test_hit(15.0, HitCharacterSource::Packet, "outgoing");
        later_hit.target_max_hp = 8_250.0;
        set_wire_target(&mut later_hit, target);
        let correction = tracker
            .observe_hit_with_semantics(&later_hit, false, 0)
            .expect("the next maximum-HP snapshot should correct the special hit");

        assert_eq!(correction.source_timestamp, 10.0);
        assert_eq!(correction.damage, 800.0);
        assert_eq!(correction.max_hp_reduction, Some(1_750.0));
        assert_eq!(correction.reconciled_overkill_damage, None);
    }

    #[test]
    fn delayed_max_hp_drop_preserves_previous_server_damage_correction() {
        let target = [8_u8; 29];
        let mut tracker = ServerDamageCalibrationTracker::default();
        tracker.hp_by_handle.insert(
            target,
            ServerHpSnapshot {
                timestamp: 9.0,
                hp: 8_000.0,
            },
        );
        tracker.target_max_hp_by_handle.insert(target, 10_000.0);

        let mut nightmare = duplicate_test_hit(10.0, HitCharacterSource::Packet, "outgoing");
        nightmare.damage = 800.0;
        nightmare.target_hp_before = 8_000.0;
        nightmare.target_hp_after = 7_200.0;
        nightmare.target_max_hp = 10_000.0;
        set_wire_target(&mut nightmare, target);
        let _ = tracker.observe_hit_with_semantics(&nightmare, true, 200);

        let (server_correction, unattributed) = tracker.observe_server_damage_settlement(
            10.05,
            &server_damage_settlement_for(target, 7_000.0, 0, 1_000),
        );
        let server_correction =
            server_correction.expect("the server settlement should correct the semantic hit");
        assert!(unattributed.is_none());

        let mut state = CombatState::default();
        state.push_hit(nightmare);
        assert!(state.apply_damage_correction(server_correction));

        let mut later_hit = duplicate_test_hit(15.0, HitCharacterSource::Packet, "outgoing");
        later_hit.target_hp_before = 7_000.0;
        later_hit.target_hp_after = 6_900.0;
        later_hit.target_max_hp = 8_250.0;
        set_wire_target(&mut later_hit, target);
        let delayed_correction = tracker
            .observe_hit_with_semantics(&later_hit, false, 0)
            .expect("the delayed maximum-HP snapshot should update the semantic hit");

        assert!(state.apply_damage_correction(delayed_correction));
        assert_eq!(state.hits[0].damage, 1_000.0);
        assert_eq!(state.hits[0].target_hp_before, 8_000.0);
        assert_eq!(state.hits[0].target_hp_after, 7_000.0);
        assert_eq!(state.hits[0].max_hp_reduction, 1_750.0);
    }

    #[test]
    fn reused_terminal_target_handle_discards_previous_max_hp_candidate() {
        let target = [8_u8; 29];
        let mut tracker = ServerDamageCalibrationTracker::default();
        tracker.hp_by_handle.insert(
            target,
            ServerHpSnapshot {
                timestamp: 9.0,
                hp: 8_000.0,
            },
        );
        tracker.target_max_hp_by_handle.insert(target, 10_000.0);

        let mut nightmare = duplicate_test_hit(10.0, HitCharacterSource::Packet, "outgoing");
        nightmare.damage = 800.0;
        nightmare.target_hp_before = 8_000.0;
        nightmare.target_hp_after = 7_200.0;
        nightmare.target_max_hp = 10_000.0;
        set_wire_target(&mut nightmare, target);
        let _ = tracker.observe_hit_with_semantics(&nightmare, true, 200);
        tracker.hp_by_handle.insert(
            target,
            ServerHpSnapshot {
                timestamp: 10.05,
                hp: 0.0,
            },
        );

        let mut reused_target = duplicate_test_hit(15.0, HitCharacterSource::Packet, "outgoing");
        reused_target.target_hp_before = 8_000.0;
        reused_target.target_hp_after = 7_900.0;
        reused_target.target_max_hp = 8_000.0;
        set_wire_target(&mut reused_target, target);

        assert!(
            tracker
                .observe_hit_with_semantics(&reused_target, false, 0)
                .is_none()
        );
        assert!(tracker.pending_max_hp_reduction_hits.is_empty());
        assert_eq!(tracker.hp_by_handle[&target].hp, 8_000.0);
        assert_eq!(tracker.target_max_hp_by_handle[&target], 8_000.0);
    }

    #[test]
    fn target_max_hp_drop_does_not_guess_between_multiple_special_hits() {
        let target = [8_u8; 29];
        let mut tracker = ServerDamageCalibrationTracker::default();

        let mut baseline = duplicate_test_hit(9.0, HitCharacterSource::Packet, "outgoing");
        baseline.target_max_hp = 10_000.0;
        set_wire_target(&mut baseline, target);
        tracker.observe_hit(&baseline);

        for timestamp in [10.0, 11.0] {
            let mut nightmare =
                duplicate_test_hit(timestamp, HitCharacterSource::Packet, "outgoing");
            nightmare.target_max_hp = 10_000.0;
            set_wire_target(&mut nightmare, target);
            let _ = tracker.observe_hit_with_semantics(&nightmare, true, 200);
        }

        let mut later_hit = duplicate_test_hit(15.0, HitCharacterSource::Packet, "outgoing");
        later_hit.target_max_hp = 8_250.0;
        set_wire_target(&mut later_hit, target);
        let correction = tracker.observe_hit_with_semantics(&later_hit, false, 0);

        assert!(correction.is_none());
        assert!(tracker.pending_max_hp_reduction_hits.is_empty());
    }

    #[test]
    fn server_damage_settlements_follow_exact_same_target_fifo_order() {
        let target = [8_u8; 29];
        let catalog = AbilityCatalog::from(HashMap::from([(
            "GE_Player_Lacrimosa_Blood_Damage_LV6".to_owned(),
            GameplayEffectSkill {
                damage_source_category: Some("A".to_owned()),
                ability_name: Some("GA_Lacrimosa_Melee".to_owned()),
                attack_type: "Special Damage".to_owned(),
                damage_component: Some("「噩梦」".to_owned()),
                owner_character_id: Some(1004),
                use_server_damage: true,
                max_hp_reduction_percent: 200,
            },
        )]));
        let mut decoder = PacketDecoder::with_ability_catalog(catalog.into(), false);
        let mut hits = Vec::new();
        for (timestamp, damage) in [(10.0, 43_242.0), (10.001, 43_242.0), (10.01, 7_290.0)] {
            let mut hit = duplicate_test_hit(timestamp, HitCharacterSource::Packet, "outgoing");
            hit.damage = damage;
            hit.target_hp_before = 2_149_015.0;
            hit.target_hp_after = 2_149_015.0 - damage;
            hit.target_max_hp = 2_706_982.0;
            set_wire_target(&mut hit, target);
            hits.push(hit);
        }
        let mut nightmare = duplicate_test_hit(10.02, HitCharacterSource::Packet, "outgoing");
        nightmare.damage = 2_953.0;
        nightmare.target_hp_before = 2_149_015.0;
        nightmare.target_hp_after = 2_146_062.0;
        nightmare.target_max_hp = 2_706_982.0;
        nightmare.gameplay_effect_name = Some("GE_Player_Lacrimosa_Blood_Damage_LV6".to_owned());
        set_wire_target(&mut nightmare, target);
        hits.push(nightmare);

        let (first_corrections, first_unattributed, first_residual_hits) = decoder
            .reconcile_current_packet_server_damage_settlements(
                10.05,
                &[
                    server_damage_settlement_for(target, 2_102_875.0, 0, 46_140),
                    server_damage_settlement_for(target, 2_056_735.0, 0, 46_140),
                ],
                hits.iter(),
            );
        assert!(first_unattributed.is_empty());
        assert!(first_residual_hits.is_empty());
        assert_eq!(first_corrections.len(), 2);
        assert_eq!(first_corrections[0].source_timestamp, 10.0);
        assert_eq!(first_corrections[1].source_timestamp, 10.001);
        assert_eq!(first_corrections[0].damage, 46_140.0);
        assert_eq!(first_corrections[1].damage, 46_140.0);

        let (corrections, unattributed, residual_hits) = decoder
            .reconcile_server_damage_settlements(
                10.09,
                &[
                    server_damage_settlement_for(target, 2_048_957.0, 0, 7_778),
                    server_damage_settlement_for(target, 2_041_043.0, 0, 3_151),
                ],
            );

        assert!(unattributed.is_empty());
        assert!(residual_hits.is_empty());
        assert_eq!(corrections.len(), 2);
        assert_eq!(corrections[0].source_damage, 7_290.0);
        assert_eq!(corrections[0].damage, 7_778.0);
        assert_eq!(corrections[1].source_damage, 2_953.0);
        assert_eq!(corrections[1].damage, 3_151.0);
        assert_eq!(corrections[1].max_hp_reduction, Some(6_302.0));
        assert_eq!(corrections[1].reconciled_overkill_damage, Some(0.0));
    }

    #[test]
    fn server_damage_settlement_consumes_oldest_identical_pending_hit() {
        let target = [7_u8; 29];
        let mut tracker = ServerDamageCalibrationTracker::default();
        for timestamp in [10.0, 10.01] {
            let mut hit = duplicate_test_hit(timestamp, HitCharacterSource::Packet, "outgoing");
            hit.damage = 892.0;
            hit.target_hp_before = 10_000.0;
            hit.target_hp_after = 9_108.0;
            hit.target_max_hp = 10_000.0;
            set_wire_target(&mut hit, target);
            tracker.observe_hit(&hit);
        }

        let (correction, unattributed) = tracker.observe_server_damage_settlement(
            10.05,
            &server_damage_settlement_for(target, 9_108.0, 0, 892),
        );

        assert_eq!(correction.map(|row| row.source_timestamp), Some(10.0));
        assert!(unattributed.is_none());
        assert_eq!(tracker.pending_hits.len(), 1);
        assert_eq!(tracker.pending_hits[0].hit.timestamp, 10.01);
    }

    #[test]
    fn server_damage_batch_does_not_consume_a_conflicting_target_by_hp() {
        let first_target = [7_u8; 29];
        let second_target = [8_u8; 29];
        let mut tracker = ServerDamageCalibrationTracker::default();
        let mut hit = duplicate_test_hit(10.0, HitCharacterSource::Packet, "outgoing");
        hit.damage = 126_190.0;
        hit.target_hp_before = 782_367.0;
        hit.target_hp_after = 656_177.0;
        hit.target_max_hp = 808_898.0;
        set_wire_target(&mut hit, second_target);
        tracker.observe_hit(&hit);

        let (corrections, unattributed) = tracker.observe_server_damage_settlements(
            10.05,
            &[
                server_damage_settlement_for(first_target, 656_177.0, 0, 126_190),
                server_damage_settlement_for(second_target, 656_177.0, 0, 126_190),
            ],
        );

        assert_eq!(corrections.len(), 1);
        assert_eq!(corrections[0].source_timestamp, 10.0);
        assert_eq!(unattributed.len(), 1);
        assert_eq!(unattributed[0].damage, 126_190.0);
        assert_eq!(unattributed[0].candidate_hits, 0);
    }

    #[test]
    fn server_damage_accepts_one_point_rounding_only_with_target_and_hp_evidence() {
        let target = [7_u8; 29];
        let mut tracker = ServerDamageCalibrationTracker::default();
        let mut hit = duplicate_test_hit(10.0, HitCharacterSource::Packet, "outgoing");
        hit.damage = 19_986.0;
        hit.target_hp_before = 1_715_144.0;
        hit.target_hp_after = 1_695_158.0;
        hit.target_max_hp = 1_752_612.0;
        set_wire_target(&mut hit, target);
        tracker.observe_hit(&hit);

        let (correction, unattributed) = tracker.observe_server_damage_settlement(
            10.05,
            &server_damage_settlement_for(target, 1_695_157.0, 0, 19_987),
        );

        assert!(unattributed.is_none());
        assert_eq!(correction.map(|row| row.damage), Some(19_987.0));

        let mut targetless = ServerDamageCalibrationTracker::default();
        hit.target_id = None;
        hit.target_context.clear();
        targetless.observe_hit(&hit);
        let (correction, unattributed) = targetless.observe_server_damage_settlement(
            10.05,
            &server_damage_settlement_for(target, 1_695_157.0, 0, 19_987),
        );
        assert!(correction.is_none());
        assert_eq!(unattributed.map(|row| row.candidate_hits), Some(0));
    }

    #[test]
    fn server_damage_uses_previous_target_hp_to_correct_a_different_client_value() {
        let target = [7_u8; 29];
        let mut tracker = ServerDamageCalibrationTracker::default();
        tracker.hp_by_handle.insert(
            target,
            ServerHpSnapshot {
                timestamp: 10.0,
                hp: 1_662_647.0,
            },
        );
        let mut hit = duplicate_test_hit(10.01, HitCharacterSource::Packet, "outgoing");
        hit.damage = 8_492.0;
        hit.target_hp_before = 1_662_647.0;
        hit.target_hp_after = 1_654_155.0;
        hit.target_max_hp = 1_752_612.0;
        set_wire_target(&mut hit, target);
        tracker.observe_hit(&hit);

        let (correction, unattributed) = tracker.observe_server_damage_settlement(
            10.05,
            &server_damage_settlement_for(target, 1_651_786.0, 0, 10_861),
        );

        assert!(unattributed.is_none());
        let correction = correction.expect("previous HP should anchor the target-specific hit");
        assert_eq!(correction.damage, 10_861.0);
        assert_eq!(correction.target_hp_before, 1_662_647.0);
        assert_eq!(correction.target_hp_after, 1_651_786.0);
    }

    #[test]
    fn server_damage_emits_one_unattributed_residual_when_a_target_dies() {
        let target = [7_u8; 29];
        let mut tracker = ServerDamageCalibrationTracker::default();
        let mut hit = duplicate_test_hit(10.0, HitCharacterSource::Packet, "outgoing");
        hit.damage = 800.0;
        hit.target_hp_before = 1_000.0;
        hit.target_hp_after = 200.0;
        hit.target_max_hp = 1_000.0;
        set_wire_target(&mut hit, target);
        tracker.observe_hit(&hit);

        let (corrections, unattributed) = tracker.observe_server_damage_settlements(
            10.05,
            &[
                server_damage_settlement_for(target, 200.0, 0, 800),
                server_damage_settlement_for(target, 1.0, 1, 300),
            ],
        );

        assert_eq!(corrections.len(), 1);
        assert_eq!(unattributed.len(), 1);
        let residual_hits = tracker.take_residual_hits();
        assert_eq!(residual_hits.len(), 1);
        assert_eq!(residual_hits[0].damage, 199.0);
        assert_eq!(residual_hits[0].target_hp_before, 999.0);
        assert_eq!(residual_hits[0].char_id, 0);
        assert!(!residual_hits[0].char_known);
        assert_eq!(
            residual_hits[0].target_id,
            Some(target_id_from_wire_handle(&target))
        );

        let _ = tracker.observe_server_damage_settlement(
            10.06,
            &server_damage_settlement_for(target, 1.0, 1, 2),
        );
        assert!(tracker.take_residual_hits().is_empty());
    }

    #[test]
    fn server_damage_calibration_keeps_same_prefix_instances_separate() {
        let mut tracker = ServerDamageCalibrationTracker::default();
        let first = [7; 29];
        let mut second = first;
        second[20] = 8;
        let _ = tracker.observe_boss_hp_detailed(1.0, &boss_hp_update_for(first, 1_000.0));
        let _ = tracker.observe_boss_hp_detailed(1.0, &boss_hp_update_for(second, 2_000.0));

        let (_, observation) =
            tracker.observe_boss_hp_detailed(2.0, &boss_hp_update_for(first, 900.0));

        assert_eq!(
            observation,
            Some(UnattributedServerDamage {
                timestamp: 2.0,
                damage: 100.0,
                candidate_hits: 0,
            })
        );
    }

    #[test]
    fn server_damage_calibration_corrects_single_pending_hit() {
        let mut tracker = ServerDamageCalibrationTracker::default();
        assert!(
            tracker
                .observe_boss_hp(9.0, &boss_hp_update(10_000.0))
                .is_none()
        );
        let mut hit = duplicate_test_hit(10.0, HitCharacterSource::Packet, "outgoing");
        hit.damage = 1_000.0;
        hit.target_hp_before = 10_000.0;
        hit.target_hp_after = 9_000.0;
        hit.target_max_hp = 10_000.0;
        set_wire_target(&mut hit, [7; 29]);
        tracker.observe_hit(&hit);

        let (correction, unattributed) =
            tracker.observe_boss_hp_detailed(10.05, &boss_hp_update(8_750.0));
        let correction = correction.expect("single pending hit should use server HP delta");

        assert!(unattributed.is_none());
        assert_eq!(correction.source_damage, 1_000.0);
        assert_eq!(correction.damage, 1_250.0);
        assert_eq!(correction.target_hp_before, 10_000.0);
        assert_eq!(correction.target_hp_after, 8_750.0);
    }

    #[test]
    fn server_damage_calibration_does_not_split_multiple_pending_hits() {
        let mut tracker = ServerDamageCalibrationTracker::default();
        assert!(
            tracker
                .observe_boss_hp(9.0, &boss_hp_update(10_000.0))
                .is_none()
        );
        let mut first = duplicate_test_hit(10.0, HitCharacterSource::Packet, "outgoing");
        first.target_hp_before = 10_000.0;
        first.target_hp_after = 9_000.0;
        first.target_max_hp = 10_000.0;
        set_wire_target(&mut first, [7; 29]);
        let mut second = duplicate_test_hit(10.02, HitCharacterSource::Packet, "outgoing");
        second.target_hp_before = 9_000.0;
        second.target_hp_after = 8_500.0;
        second.target_max_hp = 10_000.0;
        set_wire_target(&mut second, [7; 29]);
        tracker.observe_hit(&first);
        tracker.observe_hit(&second);

        assert!(
            tracker
                .observe_boss_hp(10.05, &boss_hp_update(8_500.0))
                .is_none()
        );
    }

    #[test]
    fn server_damage_calibration_counts_only_matching_wire_target() {
        let mut tracker = ServerDamageCalibrationTracker::default();
        let boss = [7_u8; 29];
        let minion = [9_u8; 29];
        let _ = tracker.observe_boss_hp(9.0, &boss_hp_update_for(boss, 10_000.0));
        let mut boss_hit = duplicate_test_hit(10.0, HitCharacterSource::Packet, "outgoing");
        boss_hit.damage = 1_000.0;
        boss_hit.target_hp_before = 10_000.0;
        boss_hit.target_hp_after = 9_000.0;
        boss_hit.target_max_hp = 10_000.0;
        set_wire_target(&mut boss_hit, boss);
        let mut minion_hit = boss_hit.clone();
        set_wire_target(&mut minion_hit, minion);
        tracker.observe_hit(&boss_hit);
        tracker.observe_hit(&minion_hit);

        let (correction, unattributed) =
            tracker.observe_boss_hp_detailed(10.05, &boss_hp_update_for(boss, 8_750.0));

        assert!(unattributed.is_none());
        assert_eq!(correction.map(|row| row.source_damage), Some(1_000.0));
    }

    #[test]
    fn server_damage_calibration_reports_unattributed_hp_delta_without_guessing_a_role() {
        let mut no_candidate = ServerDamageCalibrationTracker::default();
        assert!(
            no_candidate
                .observe_boss_hp(9.0, &boss_hp_update(10_000.0))
                .is_none()
        );
        let (correction, observation) =
            no_candidate.observe_boss_hp_detailed(10.0, &boss_hp_update(9_500.0));
        assert!(correction.is_none());
        assert_eq!(
            observation,
            Some(UnattributedServerDamage {
                timestamp: 10.0,
                damage: 500.0,
                candidate_hits: 0,
            })
        );

        let mut ambiguous = ServerDamageCalibrationTracker::default();
        let _ = ambiguous.observe_boss_hp(9.0, &boss_hp_update(10_000.0));
        for (timestamp, damage) in [(10.0, 600.0), (10.02, 400.0)] {
            let mut hit = duplicate_test_hit(timestamp, HitCharacterSource::Packet, "outgoing");
            hit.damage = damage;
            hit.target_hp_before = 10_000.0;
            hit.target_hp_after = 10_000.0 - damage;
            hit.target_max_hp = 10_000.0;
            set_wire_target(&mut hit, [7; 29]);
            ambiguous.observe_hit(&hit);
        }
        let (correction, observation) =
            ambiguous.observe_boss_hp_detailed(10.05, &boss_hp_update(8_500.0));
        assert!(correction.is_none());
        assert_eq!(observation.map(|row| row.candidate_hits), Some(2));
        assert_eq!(
            observation.map(|row| row.damage),
            Some(500.0),
            "only the HP delta unexplained by both decoded hits is unassigned"
        );
    }

    #[test]
    fn server_damage_calibration_bounds_untrusted_target_handles() {
        let mut tracker = ServerDamageCalibrationTracker::default();
        for index in 0..MAX_SERVER_DAMAGE_TARGETS + 20 {
            let mut update = boss_hp_update(10_000.0);
            update.target_handle[..8].copy_from_slice(&(index as u64).to_le_bytes());
            let _ = tracker.observe_boss_hp_detailed(index as f64, &update);
        }

        assert_eq!(tracker.hp_by_handle.len(), MAX_SERVER_DAMAGE_TARGETS);
        let mut newest = [7_u8; 29];
        newest[..8].copy_from_slice(&((MAX_SERVER_DAMAGE_TARGETS + 19) as u64).to_le_bytes());
        assert!(tracker.hp_by_handle.contains_key(&newest));
        let mut oldest = [7_u8; 29];
        oldest[..8].copy_from_slice(&0_u64.to_le_bytes());
        assert!(!tracker.hp_by_handle.contains_key(&oldest));
    }

    #[test]
    fn server_damage_calibration_bounds_all_per_target_state() {
        let mut tracker = ServerDamageCalibrationTracker::default();
        for index in 0..MAX_SERVER_DAMAGE_TARGETS + 20 {
            let mut target = [7_u8; 29];
            target[..8].copy_from_slice(&(index as u64).to_le_bytes());
            let timestamp = index as f64;
            let mut hit = duplicate_test_hit(timestamp, HitCharacterSource::Packet, "outgoing");
            hit.damage = 100.0;
            hit.target_hp_before = 1_000.0;
            hit.target_hp_after = 900.0;
            hit.target_max_hp = 1_000.0;
            set_wire_target(&mut hit, target);
            tracker.observe_hit(&hit);
            tracker.observe_server_damage_settlement(
                timestamp + 0.05,
                &server_damage_settlement_for(target, 0.0, 1, 100),
            );
            tracker.take_residual_hits();
        }

        assert_eq!(tracker.hp_by_handle.len(), MAX_SERVER_DAMAGE_TARGETS);
        assert_eq!(
            tracker.client_damage_by_handle.len(),
            MAX_SERVER_DAMAGE_TARGETS
        );
        assert_eq!(
            tracker.target_max_hp_by_handle.len(),
            MAX_SERVER_DAMAGE_TARGETS
        );
        assert_eq!(tracker.residual_emitted.len(), MAX_SERVER_DAMAGE_TARGETS);
        let mut oldest = [7_u8; 29];
        oldest[..8].copy_from_slice(&0_u64.to_le_bytes());
        assert!(!tracker.hp_by_handle.contains_key(&oldest));
        assert!(!tracker.client_damage_by_handle.contains_key(&oldest));
        assert!(!tracker.target_max_hp_by_handle.contains_key(&oldest));
        assert!(!tracker.residual_emitted.contains(&oldest));
    }

    #[test]
    fn reconcile_boss_hp_updates_lets_follow_up_claim_before_calibration() {
        let characters = follow_up_test_characters();
        let mut decoder = PacketDecoder::with_server_damage_calibration(true);
        decoder
            .follow_up_damage
            .observe_fuwen_start_candidate(0.0, 1, 2, &characters);
        observe_visible_fuwen_trigger(&mut decoder.follow_up_damage, 1, 0.0);

        let warm_up = boss_hp_update(1_000_000.0);
        let _ = decoder.reconcile_boss_hp_updates(0.0, std::slice::from_ref(&warm_up));

        let mut hit = targetless_hit();
        hit.char_id = 1;
        hit.timestamp = 0.1;
        hit.target_max_hp = 1_000_000.0;
        hit.target_hp_before = 1_000_000.0;
        hit.target_hp_after = 999_000.0;
        hit.damage = 1_000.0;
        hit.attack_type = Some("普攻".to_owned());
        set_wire_target(&mut hit, [7; 29]);
        decoder
            .follow_up_damage
            .observe_hit(&hit, None, &characters);
        decoder.server_damage_calibration.observe_hit(&hit);

        let update = boss_hp_update(998_750.0);
        let (inferred_follow_ups, hp_sync_follow_ups, server_damage_corrections, unattributed) =
            decoder.reconcile_boss_hp_updates(0.2, std::slice::from_ref(&update));

        assert_eq!(inferred_follow_ups.len(), 1);
        assert_eq!(inferred_follow_ups[0].damage, 250.0);
        assert!(hp_sync_follow_ups.is_empty());
        assert!(unattributed.is_empty());
        assert!(
            server_damage_corrections.is_empty(),
            "calibration must not also overwrite a hit the reaction follow-up already fully explained"
        );
    }

    #[test]
    fn reconcile_boss_hp_updates_keeps_calibration_baseline_fresh_after_a_claimed_update() {
        let characters = follow_up_test_characters();
        let mut decoder = PacketDecoder::with_server_damage_calibration(true);
        decoder
            .follow_up_damage
            .observe_fuwen_start_candidate(0.0, 1, 2, &characters);
        observe_visible_fuwen_trigger(&mut decoder.follow_up_damage, 1, 0.0);

        let warm_up = boss_hp_update(1_000_000.0);
        let _ = decoder.reconcile_boss_hp_updates(0.0, std::slice::from_ref(&warm_up));

        // This hit is claimed by the reaction follow-up below.
        let mut hit = targetless_hit();
        hit.char_id = 1;
        hit.timestamp = 0.1;
        hit.target_max_hp = 1_000_000.0;
        hit.target_hp_before = 1_000_000.0;
        hit.target_hp_after = 999_000.0;
        hit.damage = 1_000.0;
        hit.attack_type = Some("普攻".to_owned());
        set_wire_target(&mut hit, [7; 29]);
        decoder
            .follow_up_damage
            .observe_hit(&hit, None, &characters);
        decoder.server_damage_calibration.observe_hit(&hit);

        let claimed_update = boss_hp_update(998_750.0);
        let (inferred_follow_ups, _, server_damage_corrections, unattributed) =
            decoder.reconcile_boss_hp_updates(0.2, std::slice::from_ref(&claimed_update));
        assert_eq!(inferred_follow_ups.len(), 1);
        assert!(server_damage_corrections.is_empty());
        assert!(unattributed.is_empty());

        // A later, fuwen-ineligible hit (its own attack_type is itself an
        // excluded reaction label) that calibration alone should evaluate.
        let mut second_hit = targetless_hit();
        second_hit.char_id = 2;
        second_hit.timestamp = 0.3;
        second_hit.target_max_hp = 1_000_000.0;
        second_hit.target_hp_before = 998_750.0;
        second_hit.target_hp_after = 998_050.0;
        second_hit.damage = 700.0;
        second_hit.attack_type = Some("创生花".to_owned());
        set_wire_target(&mut second_hit, [7; 29]);
        decoder
            .follow_up_damage
            .observe_hit(&second_hit, None, &characters);
        decoder.server_damage_calibration.observe_hit(&second_hit);

        let next_update = boss_hp_update(998_000.0);
        let (_, _, server_damage_corrections, unattributed) =
            decoder.reconcile_boss_hp_updates(0.4, std::slice::from_ref(&next_update));
        assert!(unattributed.is_empty());

        // If the claimed update above hadn't also advanced calibration's own
        // HP snapshot and pending queue, this would compare against the stale
        // pre-claim baseline (1,000,000) with the first hit still queued
        // alongside this one, so `candidates.len() != 1` and no correction
        // would come out at all.
        let correction = server_damage_corrections
            .first()
            .expect("calibration's baseline must have advanced past the claimed update");
        assert_eq!(correction.damage, 750.0);
        assert_eq!(correction.source_damage, 700.0);
    }

    #[test]
    fn reconcile_boss_hp_updates_still_calibrates_when_no_follow_up_applies() {
        let characters = duplicate_test_characters();
        let mut decoder = PacketDecoder::with_server_damage_calibration(true);

        let warm_up = boss_hp_update(10_000.0);
        let _ = decoder.reconcile_boss_hp_updates(9.0, std::slice::from_ref(&warm_up));

        let mut hit = duplicate_test_hit(10.0, HitCharacterSource::Packet, "outgoing");
        hit.damage = 1_000.0;
        hit.target_hp_before = 10_000.0;
        hit.target_hp_after = 9_000.0;
        hit.target_max_hp = 10_000.0;
        set_wire_target(&mut hit, [7; 29]);
        decoder
            .follow_up_damage
            .observe_hit(&hit, None, &characters);
        decoder.server_damage_calibration.observe_hit(&hit);

        let update = boss_hp_update(8_750.0);
        let (inferred_follow_ups, hp_sync_follow_ups, server_damage_corrections, unattributed) =
            decoder.reconcile_boss_hp_updates(10.05, std::slice::from_ref(&update));

        assert!(inferred_follow_ups.is_empty());
        assert!(hp_sync_follow_ups.is_empty());
        assert!(unattributed.is_empty());
        assert_eq!(server_damage_corrections.len(), 1);
        assert_eq!(server_damage_corrections[0].damage, 1_250.0);
    }

    #[test]
    fn disabled_calibration_still_reports_server_only_damage_without_polluting_totals() {
        let mut decoder = PacketDecoder::with_server_damage_calibration(false);
        let warm_up = boss_hp_update(10_000.0);
        let _ = decoder.reconcile_boss_hp_updates(9.0, std::slice::from_ref(&warm_up));

        let update = boss_hp_update(9_500.0);
        let (inferred_follow_ups, hp_sync_follow_ups, corrections, unattributed) =
            decoder.reconcile_boss_hp_updates(10.0, std::slice::from_ref(&update));

        assert!(inferred_follow_ups.is_empty());
        assert!(hp_sync_follow_ups.is_empty());
        assert!(corrections.is_empty());
        assert_eq!(
            unattributed,
            vec![UnattributedServerDamage {
                timestamp: 10.0,
                damage: 500.0,
                candidate_hits: 0,
            }]
        );

        let mut state = CombatState::default();
        for observation in unattributed {
            assert!(state.observe_unattributed_server_damage(observation));
        }
        assert_eq!(state.total_damage, 0.0);
        assert!(state.stats.is_empty());
        assert_eq!(state.unattributed_server_damage, 500.0);
    }

    #[test]
    fn disabled_calibration_observes_hits_but_exposes_only_the_unassigned_residual() {
        let characters = duplicate_test_characters();
        let mut decoder = PacketDecoder::with_server_damage_calibration(false);
        let warm_up = boss_hp_update(10_000.0);
        let _ = decoder.reconcile_boss_hp_updates(9.0, std::slice::from_ref(&warm_up));

        let mut hit = duplicate_test_hit(10.0, HitCharacterSource::Packet, "outgoing");
        hit.damage = 1_000.0;
        hit.target_hp_before = 10_000.0;
        hit.target_hp_after = 9_000.0;
        hit.target_max_hp = 10_000.0;
        set_wire_target(&mut hit, [7; 29]);
        let (sender, _receiver) = bounded(4);
        decoder.emit_hits(
            std::iter::once(hit),
            &characters,
            &EngineEventSink::reliable(sender),
        );

        let update = boss_hp_update(8_750.0);
        let (inferred_follow_ups, hp_sync_follow_ups, corrections, unattributed) =
            decoder.reconcile_boss_hp_updates(10.05, std::slice::from_ref(&update));

        assert!(inferred_follow_ups.is_empty());
        assert!(hp_sync_follow_ups.is_empty());
        assert!(corrections.is_empty());
        assert_eq!(
            unattributed,
            vec![UnattributedServerDamage {
                timestamp: 10.05,
                damage: 250.0,
                candidate_hits: 1,
            }]
        );
    }

    #[test]
    fn server_authoritative_semantic_forces_legacy_hp_correction_when_calibration_is_disabled() {
        let characters = duplicate_test_characters();
        let catalog = AbilityCatalog::from(HashMap::from([(
            "GE_Player_Lacrimosa_Blood_Damage_LV6".to_owned(),
            GameplayEffectSkill {
                damage_source_category: Some("A".to_owned()),
                ability_name: Some("GA_Lacrimosa_Melee".to_owned()),
                attack_type: "Special Damage".to_owned(),
                damage_component: Some("「噩梦」".to_owned()),
                owner_character_id: Some(1004),
                use_server_damage: true,
                max_hp_reduction_percent: 200,
            },
        )]));
        let mut decoder = PacketDecoder::with_ability_catalog(catalog.into(), false);
        let warm_up = boss_hp_update(10_000.0);
        let _ = decoder.reconcile_boss_hp_updates(9.0, std::slice::from_ref(&warm_up));

        let mut hit = duplicate_test_hit(10.0, HitCharacterSource::Packet, "outgoing");
        hit.damage = 1_000.0;
        hit.target_hp_before = 10_000.0;
        hit.target_hp_after = 9_000.0;
        hit.target_max_hp = 10_000.0;
        hit.gameplay_effect_name = Some("GE_Player_Lacrimosa_Blood_Damage_LV6".to_owned());
        set_wire_target(&mut hit, [7; 29]);
        let (sender, _receiver) = bounded(4);
        decoder.emit_hits(
            std::iter::once(hit),
            &characters,
            &EngineEventSink::reliable(sender),
        );

        let update = boss_hp_update(8_750.0);
        let (_, _, corrections, unattributed) =
            decoder.reconcile_boss_hp_updates(10.05, std::slice::from_ref(&update));

        assert!(unattributed.is_empty());
        assert_eq!(corrections.len(), 1);
        assert_eq!(corrections[0].damage, 1_250.0);
        assert_eq!(corrections[0].reconciled_overkill_damage, Some(0.0));
        assert_eq!(corrections[0].max_hp_reduction, Some(2_500.0));
    }

    #[test]
    fn disabled_calibration_reports_only_multi_candidate_residual_without_mutating_totals() {
        let mut decoder = PacketDecoder::with_server_damage_calibration(false);
        let warm_up = boss_hp_update(10_000.0);
        let _ = decoder.reconcile_boss_hp_updates(9.0, std::slice::from_ref(&warm_up));

        let mut hits = Vec::new();
        for (timestamp, damage) in [(10.0, 600.0), (10.02, 400.0)] {
            let mut hit = duplicate_test_hit(timestamp, HitCharacterSource::Packet, "outgoing");
            hit.damage = damage;
            hit.target_hp_before = 10_000.0;
            hit.target_hp_after = 10_000.0 - damage;
            hit.target_max_hp = 10_000.0;
            set_wire_target(&mut hit, [7; 29]);
            decoder.server_damage_calibration.observe_hit(&hit);
            hits.push(hit);
        }

        let update = boss_hp_update(8_500.0);
        let (_, hp_sync_follow_ups, corrections, unattributed) =
            decoder.reconcile_boss_hp_updates(10.05, std::slice::from_ref(&update));
        assert!(hp_sync_follow_ups.is_empty());
        assert!(corrections.is_empty());
        assert_eq!(
            unattributed,
            vec![UnattributedServerDamage {
                timestamp: 10.05,
                damage: 500.0,
                candidate_hits: 2,
            }]
        );

        let mut state = CombatState::default();
        for hit in hits {
            state.push_hit(hit);
        }
        let decoded_total = state.total_damage;
        for observation in unattributed {
            assert!(state.observe_unattributed_server_damage(observation));
        }
        assert_eq!(decoded_total, 1_000.0);
        assert_eq!(state.total_damage, decoded_total);
        assert_eq!(state.unattributed_server_damage, 500.0);
    }

    #[test]
    fn disabled_calibration_keeps_lethal_residual_unattributed() {
        let characters = duplicate_test_characters();
        let mut decoder = PacketDecoder::with_server_damage_calibration(false);
        let warm_up = boss_hp_update(29_700.0);
        let _ = decoder.reconcile_boss_hp_updates(9.0, std::slice::from_ref(&warm_up));

        let mut source = duplicate_test_hit(10.0, HitCharacterSource::Packet, "outgoing");
        source.damage = 26_185.0;
        source.target_hp_before = 29_700.0;
        source.target_hp_after = 3_515.0;
        source.target_max_hp = 1_930_389.0;
        set_wire_target(&mut source, [7; 29]);
        let prepared = decoder.prepare_hits_for_emission(vec![source], &[1051], false, &characters);
        let (sender, _receiver) = bounded(4);
        decoder.emit_hits(
            prepared.emit,
            &characters,
            &EngineEventSink::reliable(sender),
        );

        let lethal = boss_hp_update(1.0);
        let (inferred_follow_ups, hp_sync_follow_ups, corrections, unattributed) =
            decoder.reconcile_boss_hp_updates(10.04, std::slice::from_ref(&lethal));

        assert!(inferred_follow_ups.is_empty());
        assert!(corrections.is_empty());
        assert!(
            hp_sync_follow_ups.is_empty(),
            "conservative mode must not guess that a lethal environmental residual belongs to the recent role"
        );
        assert_eq!(
            unattributed,
            vec![UnattributedServerDamage {
                timestamp: 10.04,
                damage: 3_515.0,
                candidate_hits: 1,
            }]
        );
    }

    #[test]
    fn confirmed_packet_hit_suppresses_ambiguous_session_candidate() {
        let mut decoder = PacketDecoder::default();
        let candidate = duplicate_test_hit(10.0, HitCharacterSource::Session, "unknown");

        let characters = duplicate_test_characters();

        let prepared =
            decoder.prepare_hits_for_emission(vec![candidate], &[1051, 1055], false, &characters);

        assert!(prepared.emit.is_empty());
        assert_eq!(prepared.deferred_ambiguous, 1);
        assert_eq!(decoder.pending_ambiguous_hits.len(), 1);

        let confirmed = duplicate_test_hit(10.1, HitCharacterSource::Packet, "outgoing");
        let prepared =
            decoder.prepare_hits_for_emission(vec![confirmed], &[1051], false, &characters);

        assert!(prepared.emit.is_empty());
        assert_eq!(prepared.suppressed_ambiguous, 1);
        assert_eq!(prepared.deferred_targetless, 1);
        assert!(decoder.pending_ambiguous_hits.is_empty());
        assert_eq!(decoder.pending_targetless_hits.len(), 1);
        assert_eq!(decoder.pending_targetless_hits[0].char_id, 1051);
    }

    #[test]
    fn confirmed_packet_hit_suppresses_exact_recent_duplicate_records() {
        let mut decoder = PacketDecoder::default();
        let characters = duplicate_test_characters();
        let mut confirmed = duplicate_test_hit(10.0, HitCharacterSource::Packet, "outgoing");
        set_wire_target(&mut confirmed, [1; 29]);
        let mut duplicate = confirmed.clone();
        duplicate.gameplay_effect_index = None;
        duplicate.gameplay_effect_name = None;
        duplicate.attack_type = None;

        let prepared = decoder.prepare_hits_for_emission(
            vec![confirmed, duplicate],
            &[1051],
            true,
            &characters,
        );

        assert_eq!(prepared.emit.len(), 1);
        assert_eq!(prepared.suppressed_ambiguous, 1);
        assert_eq!(prepared.emit[0].attack_type.as_deref(), Some("创生花"));
    }

    #[test]
    fn same_damage_on_distinct_wire_target_is_not_suppressed_as_duplicate() {
        let mut decoder = PacketDecoder::default();
        let characters = duplicate_test_characters();
        let mut confirmed = duplicate_test_hit(10.0, HitCharacterSource::Packet, "outgoing");
        set_wire_target(&mut confirmed, [1; 29]);
        let mut distinct_target = confirmed.clone();
        distinct_target.gameplay_effect_index = None;
        distinct_target.gameplay_effect_name = None;
        distinct_target.attack_type = None;
        set_wire_target(&mut distinct_target, [2; 29]);

        let prepared = decoder.prepare_hits_for_emission(
            vec![confirmed, distinct_target],
            &[1051],
            true,
            &characters,
        );

        assert_eq!(prepared.emit.len(), 1);
        assert_eq!(prepared.suppressed_ambiguous, 0);
        assert_eq!(prepared.deferred_ambiguous, 1);
        assert_eq!(decoder.pending_ambiguous_hits.len(), 1);
        assert_eq!(
            wire_handle_from_hit(&decoder.pending_ambiguous_hits[0]),
            Some([2; 29])
        );
    }

    #[test]
    #[ignore = "set NTE_TEST_CAPTURE to a local large or concatenated pcapng path"]
    fn stress_large_pcapng_import_with_bounded_event_lanes() {
        let path =
            PathBuf::from(std::env::var("NTE_TEST_CAPTURE").expect("NTE_TEST_CAPTURE must be set"));
        let characters = Arc::new(
            load_characters(Path::new(CHARACTER_DATA_PATH))
                .expect("character resource table should load"),
        );
        let mut ability_catalog = AbilityCatalog::load(Path::new(SKILL_DAMAGE_DATA_PATH))
            .expect("skill table should load");
        ability_catalog
            .apply_semantics(Path::new(GAMEPLAY_EFFECT_SEMANTICS_PATH))
            .expect("effect semantics should load");
        let (reliable_sender, reliable_receiver) = bounded(16_384);
        let (debug_sender, debug_receiver) = bounded(2_048);
        let sink = EngineEventSink::split(reliable_sender, debug_sender);
        let dropped_debug_probe = sink.clone();
        let handle = import_pcapng(
            path,
            CaptureResources {
                characters,
                ability_catalog: Arc::new(ability_catalog),
            },
            None,
            true,
            false,
            sink,
            Arc::new(AtomicBool::new(false)),
        )
        .expect("pcapng import thread should spawn");

        let mut semantic_events = 0;
        let mut debug_packets = 0;
        let mut capture_stopped = false;
        let mut errors = Vec::new();
        while !handle.is_finished() || !reliable_receiver.is_empty() || !debug_receiver.is_empty() {
            while let Ok(event) = reliable_receiver.try_recv() {
                semantic_events += 1;
                match event {
                    EngineEvent::CaptureStopped => capture_stopped = true,
                    EngineEvent::Error(error) => errors.push(error),
                    _ => {}
                }
            }
            while debug_receiver.try_recv().is_ok() {
                debug_packets += 1;
            }
            thread::sleep(Duration::from_millis(1));
        }
        handle.join().expect("pcapng import thread should finish");

        assert!(capture_stopped);
        assert!(errors.is_empty(), "{errors:#?}");
        assert!(semantic_events > 1);
        assert!(debug_packets > 0);
        println!(
            "large pcapng import completed: semantic_events={semantic_events}, debug_packets={debug_packets}, dropped_debug_packets={}",
            dropped_debug_probe.take_dropped_debug_packets()
        );
    }

    #[test]
    fn gameplay_effect_name_confirms_session_hit_in_multi_character_packet() {
        let mut decoder = PacketDecoder::default();
        let characters = duplicate_test_characters();
        let mut hit = targetless_hit();
        hit.timestamp = 10.0;
        hit.char_id = 1020;
        hit.char_name = "哈尼娅".to_owned();
        hit.char_source = HitCharacterSource::Session;
        hit.direction = HitDirection::Unknown;
        hit.damage = 6_961.0;
        hit.gameplay_effect_index = Some(3261);
        hit.gameplay_effect_name = Some("GE_Player_Haniel_Skill1_Damage".to_owned());
        hit.attack_type = Some("E技能".to_owned());

        let prepared =
            decoder.prepare_hits_for_emission(vec![hit], &[1010, 1020], false, &characters);

        assert_eq!(prepared.emit.len(), 1);
        assert_eq!(prepared.deferred_ambiguous, 0);
        assert_eq!(prepared.emit[0].direction, HitDirection::Outgoing);
        assert_eq!(
            prepared.emit[0].char_source,
            HitCharacterSource::GameplayEffect
        );
        assert!(decoder.pending_ambiguous_hits.is_empty());
    }

    #[test]
    fn ambiguous_session_candidate_expires_when_unconfirmed() {
        let mut decoder = PacketDecoder::default();
        let candidate = duplicate_test_hit(10.0, HitCharacterSource::Session, "unknown");

        let characters = duplicate_test_characters();

        decoder.prepare_hits_for_emission(vec![candidate], &[1051, 1055], false, &characters);

        assert!(decoder.take_expired_ambiguous_hits(10.25).is_empty());
        let expired = decoder.take_expired_ambiguous_hits(10.75);

        assert_eq!(expired.len(), 1);
        assert_eq!(expired[0].char_source, HitCharacterSource::Session);
        assert!(decoder.pending_ambiguous_hits.is_empty());
    }

    #[test]
    fn gameplay_effect_enrichment_keeps_localized_name_out_of_hit() {
        let catalog = AbilityCatalog::from(HashMap::from([(
            "GE_Test_Skill1_Damage".to_owned(),
            GameplayEffectSkill {
                damage_source_category: Some("E".to_owned()),
                ability_name: Some("GA_Test_Skill".to_owned()),
                attack_type: "E技能".to_owned(),
                damage_component: None,
                owner_character_id: None,
                use_server_damage: false,
                max_hp_reduction_percent: 0,
            },
        )]));
        let effects = [ParsedGameplayEffect {
            unique_index: 42,
            byte_offset: 0,
            bit_shift: 0,
        }];
        let names = HashMap::from([(42, "GE_Test_Skill1_Damage".to_owned())]);
        let mut hit = targetless_hit();

        enrich_hit_with_gameplay_effect(&mut hit, &effects, &names, &catalog, None);

        assert_eq!(hit.ability_name.as_deref(), Some("GA_Test_Skill"));
        assert_eq!(hit.attack_type.as_deref(), Some("E技能"));
        assert_eq!(
            hit.gameplay_effect_name.as_deref(),
            Some("GE_Test_Skill1_Damage")
        );
        assert_eq!(hit.damage_name, None);
    }

    #[test]
    #[ignore = "set NTE_TEST_CAPTURE to a local pcapng path for target-instance diagnostics"]
    fn diagnose_capture_enemy_instance_resolution() {
        let path = std::env::var("NTE_TEST_CAPTURE").expect("NTE_TEST_CAPTURE must be set");
        let characters = Arc::new(
            load_characters(Path::new(CHARACTER_DATA_PATH))
                .expect("character resource table should load"),
        );
        let (sender, receiver) = unbounded();
        let sender = EngineEventSink::reliable(sender);
        let stop = Arc::new(AtomicBool::new(false));
        let handle = import_pcapng(
            PathBuf::from(&path),
            CaptureResources {
                characters,
                ability_catalog: Arc::new(AbilityCatalog::default()),
            },
            None,
            true,
            true,
            sender,
            stop,
        )
        .expect("pcapng import thread should spawn");
        handle.join().expect("pcapng import thread should finish");

        let mut state = CombatState::default();
        let mut identity_events = 0;
        let mut hit_target_events = 0;
        let mut enemy_events = Vec::new();
        for event in receiver.try_iter() {
            if let EngineEvent::ModScript(script) = &event
                && script.mod_id == "enemy-telemetry"
            {
                enemy_events.push((
                    filetime_100ns_to_unix_seconds(script.timestamp_100ns)
                        .expect("enemy event timestamp should be valid"),
                    script.name.clone(),
                    script.values.clone(),
                ));
                match script.name.as_str() {
                    "enemy.identity" => identity_events += 1,
                    "enemy.hit_target" => hit_target_events += 1,
                    _ => {}
                }
            }
            crate::core::reducer::apply_engine_event(&mut state, event);
        }

        let outgoing = state
            .hits
            .iter()
            .filter(|hit| hit.direction.is_outgoing())
            .collect::<Vec<_>>();
        let unidentified = outgoing
            .iter()
            .filter(|hit| hit.target_id.is_none())
            .collect::<Vec<_>>();
        println!(
            "{}: {identity_events} identity events, {hit_target_events} hit-target events, {}/{} outgoing hits unidentified",
            path,
            unidentified.len(),
            outgoing.len()
        );
        let identity_instances = enemy_events
            .iter()
            .filter(|(_, name, values)| name == "enemy.identity" && values.len() == 3)
            .map(|(_, _, values)| values[0])
            .collect::<HashSet<_>>();
        let projected_instances = outgoing
            .iter()
            .flat_map(|hit| hit.target_context.iter())
            .filter_map(|context| context.strip_prefix("enemy_target_instance="))
            .collect::<HashSet<_>>();
        let wire_instances = outgoing
            .iter()
            .filter_map(|hit| hit.target_id.as_deref())
            .collect::<HashSet<_>>();
        let mut simultaneous = HashMap::<u64, Vec<&Hit>>::new();
        for hit in &outgoing {
            simultaneous
                .entry(hit.timestamp.to_bits())
                .or_default()
                .push(hit);
        }
        let simultaneous_groups = simultaneous.values().filter(|hits| hits.len() > 1).count();
        println!(
            "  unique identity instances={} projected instances={} wire instances={} simultaneous hit groups={simultaneous_groups}",
            identity_instances.len(),
            projected_instances.len(),
            wire_instances.len()
        );
        for hits in simultaneous.values().filter(|hits| hits.len() > 1).take(8) {
            println!(
                "  simultaneous t={:.6} hits={}",
                hits[0].timestamp,
                hits.len()
            );
            for hit in hits {
                println!(
                    "    char={} damage={:.1} hp={:.1}->{:.1} target={:?}",
                    hit.char_name,
                    hit.total_damage(),
                    hit.target_hp_before,
                    hit.target_hp_after,
                    hit.target_context
                        .iter()
                        .find(|context| context.starts_with("enemy_target_instance="))
                );
            }
        }
        for hit in unidentified {
            println!(
                "t={:.3} char={} damage={:.1} max_hp={:.1}",
                hit.timestamp,
                hit.char_name,
                hit.total_damage(),
                hit.target_max_hp
            );
            let mut nearby = enemy_events.iter().collect::<Vec<_>>();
            nearby.sort_by(|left, right| {
                (left.0 - hit.timestamp)
                    .abs()
                    .total_cmp(&(right.0 - hit.timestamp).abs())
            });
            for (timestamp, name, values) in nearby.into_iter().take(6) {
                println!(
                    "  enemy_event delta={:+.6} t={timestamp:.6} name={name} values={values:?}",
                    timestamp - hit.timestamp
                );
            }
        }
    }

    #[test]
    #[ignore = "set NTE_TEST_CAPTURE to a local pcapng path for capture diagnostics"]
    fn diagnose_empty_curtain_inventory() {
        let path = std::env::var("NTE_TEST_CAPTURE").expect("NTE_TEST_CAPTURE must be set");
        let local_ip_hint = std::env::var("NTE_TEST_LOCAL_IP").ok().map(|value| {
            value
                .parse::<Ipv4Addr>()
                .expect("NTE_TEST_LOCAL_IP must be an IPv4 address")
        });
        let expected = std::env::var("NTE_EXPECT_EQUIPMENT").ok().map(|value| {
            value
                .parse::<usize>()
                .expect("NTE_EXPECT_EQUIPMENT must be a number")
        });
        let characters = Arc::new(
            load_characters(Path::new(CHARACTER_DATA_PATH))
                .expect("character resource table should load"),
        );
        let (sender, receiver) = unbounded();
        let sender = EngineEventSink::reliable(sender);
        let stop = Arc::new(AtomicBool::new(false));
        let handle = import_pcapng(
            PathBuf::from(&path),
            CaptureResources {
                characters,
                ability_catalog: Arc::new(AbilityCatalog::default()),
            },
            local_ip_hint,
            true,
            true,
            sender,
            stop,
        )
        .expect("pcapng import thread should spawn");
        handle.join().expect("pcapng import thread should finish");

        let mut latest = Vec::new();
        let mut latest_characters = Vec::new();
        let mut errors = Vec::new();
        for event in receiver.try_iter() {
            match event {
                EngineEvent::EmptyCurtain(items) => latest = items,
                EngineEvent::EmptyCurtainCharacters(characters) => {
                    latest_characters = characters;
                }
                EngineEvent::Error(error) => errors.push(error),
                _ => {}
            }
        }
        assert!(errors.is_empty(), "capture import errors: {errors:?}");
        assert!(
            !latest.is_empty(),
            "no Console equipment parsed from {path}"
        );
        if let Some(expected) = expected {
            assert_eq!(latest.len(), expected);
        }
        let equipped = latest.iter().filter(|item| item.is_equipped()).count();
        let identified = latest
            .iter()
            .filter(|item| item.equipped_character_id.is_some())
            .count();
        if std::env::var_os("NTE_DIAG_INVENTORY_IDS").is_some() {
            for item in &latest {
                println!(
                    "inventory item {} {}:{}",
                    item.item_id, item.id.solt, item.id.serial
                );
            }
        }
        println!(
            "parsed {} Console equipment items and {} character session IDs from {path}; identified {identified}/{equipped} equipped owners",
            latest.len(),
            latest_characters.len(),
        );
    }

    #[test]
    #[ignore = "set NTE_TEST_CAPTURE to a local pcapng path for capture diagnostics"]
    fn diagnose_capture_time_stop_events() {
        let path = std::env::var("NTE_TEST_CAPTURE").expect("NTE_TEST_CAPTURE must be set");
        let characters = Arc::new(
            load_characters(Path::new(CHARACTER_DATA_PATH))
                .expect("character resource table should load"),
        );
        let mut ability_catalog = AbilityCatalog::load(Path::new(SKILL_DAMAGE_DATA_PATH))
            .expect("skill table should load");
        ability_catalog
            .apply_semantics(Path::new(GAMEPLAY_EFFECT_SEMANTICS_PATH))
            .expect("effect semantics should load");
        let (sender, receiver) = unbounded();
        let sender = EngineEventSink::reliable(sender);
        let stop = Arc::new(AtomicBool::new(false));
        let handle = import_pcapng(
            PathBuf::from(path),
            CaptureResources {
                characters,
                ability_catalog: Arc::new(ability_catalog),
            },
            None,
            true,
            true,
            sender,
            stop,
        )
        .expect("pcapng import thread should spawn");
        handle.join().expect("pcapng import thread should finish");

        let mut shinku_hits = Vec::new();
        let mut skill_audit_hits = Vec::new();
        let mut outgoing_hit_timestamps = Vec::new();
        let mut abyss_events = Vec::new();
        let mut time_stops = Vec::new();
        let mut relevant_packets = Vec::new();
        let mut statuses = Vec::new();
        let mut warnings = Vec::new();
        let mut errors = Vec::new();
        let mut state = CombatState::default();
        for event in receiver.try_iter() {
            match event {
                EngineEvent::Hit(hit) => {
                    if hit.direction == HitDirection::Outgoing {
                        outgoing_hit_timestamps.push(hit.timestamp);
                    }
                    if hit.char_id == 1076 {
                        shinku_hits.push((*hit).clone());
                    }
                    skill_audit_hits.push((*hit).clone());
                    state.push_hit(*hit);
                }
                EngineEvent::Abyss(event) => {
                    abyss_events.push(event.clone());
                    state.apply_abyss_event(event);
                }
                EngineEvent::TimeStop(event) => {
                    time_stops.push(event.clone());
                    state.apply_time_stop_event(event);
                }
                EngineEvent::Packet(packet)
                    if packet.decoded_text.contains("Shinku")
                        || packet.decoded_text.contains("UltraSkill")
                        || packet.decoded_text.contains("1076") =>
                {
                    relevant_packets.push(*packet);
                }
                EngineEvent::Status(status) => statuses.push(status),
                EngineEvent::Warning(warning) => warnings.push(warning),
                EngineEvent::Error(error) => errors.push(error),
                _ => {}
            }
        }

        println!("statuses: {statuses:#?}");
        println!("warnings: {warnings:#?}");
        println!("errors: {errors:#?}");
        println!(
            "outgoing hit count: {}, window: {:?}..{:?}, wall duration: {:.6}",
            outgoing_hit_timestamps.len(),
            outgoing_hit_timestamps.first(),
            outgoing_hit_timestamps.last(),
            outgoing_hit_timestamps
                .first()
                .zip(outgoing_hit_timestamps.last())
                .map(|(start, end)| end - start)
                .unwrap_or(0.0)
        );
        println!("abyss events: {abyss_events:#?}");
        println!(
            "abyss first damage={:.1}, duration: wall={:.6}, adjusted={:.6}; second damage={:.1}, duration: wall={:.6}, adjusted={:.6}",
            state.abyss.first_half.total_damage,
            state.abyss.first_half.duration_with_time_stop(false),
            state.abyss.first_half.duration_with_time_stop(true),
            state.abyss.second_half.total_damage,
            state.abyss.second_half.duration_with_time_stop(false),
            state.abyss.second_half.duration_with_time_stop(true),
        );
        println!("shinku hit count: {}", shinku_hits.len());
        for hit in &shinku_hits {
            println!(
                "shinku hit t={:.6} damage={:.1} ability={:?} effect={:?} attack={:?}",
                hit.timestamp,
                hit.damage,
                hit.ability_name,
                hit.gameplay_effect_name,
                hit.attack_type
            );
        }
        let unmapped = skill_audit_hits
            .iter()
            .filter(|hit| {
                hit.gameplay_effect_name.is_none()
                    && hit.ability_name.is_none()
                    && hit.damage_component.is_none()
                    && hit.damage_name.is_none()
            })
            .collect::<Vec<_>>();
        let non_outgoing = skill_audit_hits
            .iter()
            .filter(|hit| hit.direction != HitDirection::Outgoing)
            .collect::<Vec<_>>();
        println!(
            "skill audit: total={}, mapped={}, unmapped={}, non_outgoing={}",
            skill_audit_hits.len(),
            skill_audit_hits.len() - unmapped.len(),
            unmapped.len(),
            non_outgoing.len()
        );
        let outgoing_targetless = skill_audit_hits
            .iter()
            .filter(|hit| hit.direction.is_outgoing() && hit.target_id.is_none())
            .count();
        let unique_targets = skill_audit_hits
            .iter()
            .filter(|hit| hit.direction.is_outgoing())
            .filter_map(|hit| hit.target_id.as_deref())
            .collect::<HashSet<_>>();
        println!(
            "target audit: unique={}, targetless_outgoing={outgoing_targetless}",
            unique_targets.len()
        );
        println!(
            "damage audit: total={:.1}, unattributed_events={}, unattributed_damage={:.1}",
            state.total_damage,
            state.unattributed_server_damage_events,
            state.unattributed_server_damage
        );
        if std::env::var_os("NTE_DIAG_SKILL_AUDIT_ROWS").is_some() {
            for hit in &skill_audit_hits {
                println!(
                    "skill row t={:.6} damage={:.1} hp={:.1}->{:.1}/{:.1} char={} source={:?} direction={:?} target={:?} effect={:?} ability={:?} attack={:?}",
                    hit.timestamp,
                    hit.damage,
                    hit.target_hp_before,
                    hit.target_hp_after,
                    hit.target_max_hp,
                    hit.char_id,
                    hit.char_source,
                    hit.direction,
                    hit.target_id,
                    hit.gameplay_effect_name,
                    hit.ability_name,
                    hit.attack_type,
                );
            }
        }
        for hit in unmapped {
            println!(
                "unmapped hit t={:.6} damage={:.1} char={} direction={:?}",
                hit.timestamp, hit.damage, hit.char_id, hit.direction
            );
        }
        for hit in non_outgoing {
            println!(
                "non-outgoing hit t={:.6} damage={:.1} char={} effect={:?} direction={:?}",
                hit.timestamp, hit.damage, hit.char_id, hit.gameplay_effect_name, hit.direction
            );
        }
        let completed_time_stop_count = time_stops
            .iter()
            .filter(|event| matches!(event, TimeStopEvent::GamePauseEnded { .. }))
            .count();
        println!("time stop count: {completed_time_stop_count}");
        for event in &time_stops {
            println!("time stop: {event:?}");
        }
        println!("relevant packet count: {}", relevant_packets.len());
        for packet in &relevant_packets {
            let text = packet
                .decoded_text
                .lines()
                .filter(|line| {
                    line.contains("Shinku")
                        || line.contains("UltraSkill")
                        || line.contains("TimeStop")
                        || line.contains("EndAbility")
                        || line.contains("CoolDown")
                })
                .take(12)
                .collect::<Vec<_>>()
                .join(" | ");
            println!(
                "packet t={:.6} dir={} ids={:?} hits={} text={}",
                packet.timestamp, packet.direction, packet.declared_ids, packet.parsed_hits, text
            );
        }
    }

    #[test]
    #[ignore = "set NTE_TEST_CAPTURE_JSON to a local nte_capture_*.json export for target-handle diagnostics"]
    fn diagnose_target_handle_stability_across_encounters() {
        let path =
            std::env::var("NTE_TEST_CAPTURE_JSON").expect("NTE_TEST_CAPTURE_JSON must be set");
        let (sender, receiver) = unbounded();
        let sender = EngineEventSink::reliable(sender);
        let stop = Arc::new(AtomicBool::new(false));
        let handle = import_capture_json(PathBuf::from(path.clone()), sender, stop)
            .expect("JSON import thread should spawn");
        handle.join().expect("json import thread should finish");

        // `import_capture_json` drops packets through `should_keep_debug_packet`,
        // which only looks at the *originally recorded* parsed_hits/declared_ids —
        // a packet that carries nothing but a boss-HP sync can get filtered out
        // there even though `parse_boss_hp_updates` would still find something in
        // it. Sweep every packet's raw payload directly so this diagnostic doesn't
        // silently miss encounters late in the file.
        let raw_text = std::fs::read_to_string(&path).expect("capture json should be readable");
        let raw_document: serde_json::Value =
            serde_json::from_str(&raw_text).expect("capture json should parse");
        let mut raw_boss_hp = Vec::new();
        if let Some(packets) = raw_document.get("packets").and_then(|v| v.as_array()) {
            for packet in packets {
                let Some(timestamp) = packet.get("timestamp_unix").and_then(|v| v.as_f64()) else {
                    continue;
                };
                let Some(payload_hex) = packet.get("payload_hex").and_then(|v| v.as_str()) else {
                    continue;
                };
                let Ok(payload) = hex::decode(payload_hex) else {
                    continue;
                };
                for update in parse_boss_hp_updates(&payload) {
                    raw_boss_hp.push((timestamp, update.target_handle, update.current_hp));
                }
            }
        }
        println!(
            "raw sweep (unfiltered): {} boss-hp updates across {} distinct handles",
            raw_boss_hp.len(),
            raw_boss_hp
                .iter()
                .map(|(_, handle, _)| *handle)
                .collect::<std::collections::HashSet<_>>()
                .len()
        );
        let mut last_raw_handle: Option<[u8; 29]> = None;
        for (timestamp, handle, hp) in &raw_boss_hp {
            let changed = last_raw_handle != Some(*handle);
            last_raw_handle = Some(*handle);
            if changed {
                println!(
                    "t={:.3} RAW_BOSS_HP handle={} hp={:.1}  <- handle changed",
                    timestamp,
                    hex::encode(handle),
                    hp
                );
            }
        }

        #[derive(Debug)]
        enum Event {
            Abyss(String, f64),
            BossHp {
                timestamp: f64,
                handle: [u8; 29],
                hp: f32,
            },
            Hit {
                timestamp: f64,
                char_id: u32,
                char_name: String,
                damage: f64,
                target_hp_after: f64,
                target_max_hp: f64,
            },
        }

        let mut events = Vec::new();
        for event in receiver.try_iter() {
            match event {
                EngineEvent::Abyss(abyss_event) => {
                    let (label, timestamp) = match abyss_event {
                        crate::engine::model::AbyssEvent::RestartDetected { timestamp } => {
                            ("RestartDetected".to_owned(), timestamp)
                        }
                        crate::engine::model::AbyssEvent::Stage {
                            timestamp,
                            floor,
                            half,
                            ..
                        } => (format!("Stage floor={floor:?} half={half:?}"), timestamp),
                        crate::engine::model::AbyssEvent::Success { timestamp } => {
                            ("Success".to_owned(), timestamp)
                        }
                        crate::engine::model::AbyssEvent::Exit { timestamp } => {
                            ("Exit".to_owned(), timestamp)
                        }
                    };
                    events.push(Event::Abyss(label, timestamp));
                }
                EngineEvent::Packet(packet) => {
                    if let Ok(payload) = hex::decode(&packet.payload_hex) {
                        for update in parse_boss_hp_updates(&payload) {
                            events.push(Event::BossHp {
                                timestamp: packet.timestamp,
                                handle: update.target_handle,
                                hp: update.current_hp,
                            });
                        }
                    }
                }
                EngineEvent::Hit(hit) if hit.direction.is_outgoing() => {
                    events.push(Event::Hit {
                        timestamp: hit.timestamp,
                        char_id: hit.char_id,
                        char_name: hit.char_name.clone(),
                        damage: hit.damage,
                        target_hp_after: hit.target_hp_after,
                        target_max_hp: hit.target_max_hp,
                    });
                }
                _ => {}
            }
        }

        events.sort_by(|left, right| {
            let left_ts = match left {
                Event::Abyss(_, ts)
                | Event::BossHp { timestamp: ts, .. }
                | Event::Hit { timestamp: ts, .. } => *ts,
            };
            let right_ts = match right {
                Event::Abyss(_, ts)
                | Event::BossHp { timestamp: ts, .. }
                | Event::Hit { timestamp: ts, .. } => *ts,
            };
            left_ts.total_cmp(&right_ts)
        });

        println!("total events: {}", events.len());
        let mut last_handle: Option<[u8; 29]> = None;
        let mut last_timestamp: Option<f64> = None;
        let mut last_target_max_hp: Option<f64> = None;
        let mut seen_char_ids: std::collections::HashSet<u32> = std::collections::HashSet::new();
        for event in &events {
            let timestamp = match event {
                Event::Abyss(_, ts)
                | Event::BossHp { timestamp: ts, .. }
                | Event::Hit { timestamp: ts, .. } => *ts,
            };
            if let Some(previous) = last_timestamp
                && timestamp - previous > 3.0
            {
                println!("   ...gap of {:.1}s...", timestamp - previous);
            }
            last_timestamp = Some(timestamp);
            match event {
                Event::Abyss(label, timestamp) => {
                    println!("t={timestamp:.3} ABYSS {label}");
                }
                Event::BossHp {
                    timestamp,
                    handle,
                    hp,
                } => {
                    let changed = last_handle != Some(*handle);
                    last_handle = Some(*handle);
                    println!(
                        "t={:.3} BOSS_HP handle={} hp={:.1}{}",
                        timestamp,
                        hex::encode(handle),
                        hp,
                        if changed { "  <- handle changed" } else { "" }
                    );
                }
                Event::Hit {
                    timestamp,
                    char_id,
                    char_name,
                    damage,
                    target_hp_after,
                    target_max_hp,
                } => {
                    let new_char = seen_char_ids.insert(*char_id);
                    let hp_reset = last_target_max_hp
                        .is_some_and(|previous| (*target_max_hp - previous).abs() > 0.5);
                    last_target_max_hp = Some(*target_max_hp);
                    println!(
                        "t={:.3} HIT char={}({}) damage={:.1} target_hp_after={:.1} target_max_hp={:.1}{}{}",
                        timestamp,
                        char_id,
                        char_name,
                        damage,
                        target_hp_after,
                        target_max_hp,
                        if new_char { "  <- new char_id" } else { "" },
                        if hp_reset {
                            "  <- target_max_hp changed"
                        } else {
                            ""
                        },
                    );
                }
            }
        }
        println!("distinct char_ids: {seen_char_ids:?}");
    }

    #[test]
    #[ignore = "set NTE_TEST_CAPTURE_JSON to a local nte_capture_*.json export for spawn-identity diagnostics"]
    fn diagnose_monster_spawn_identity_evidence() {
        // Experiment A of the target-handle investigation: sweep every raw packet
        // (bypassing `should_keep_debug_packet`, same rationale as the raw sweep
        // in `diagnose_target_handle_stability_across_encounters`) for
        //   1. printable monster/stage identifiers (mon_/boss_/Abyss_/...),
        //   2. every 16-byte boss-HP target handle seen in this capture,
        //      looking for occurrences *outside* the boss-HP anchor,
        // then print a merged timeline so spawn evidence can be correlated with
        // handle changes.
        const BOSS_HP_HEAD: [u8; 8] = [0x06, 0x00, 0x00, 0x00, 0x00, 0x20, 0x00, 0x00];
        const NAME_NEEDLES: [&str; 10] = [
            "mon_", "Mon_", "boss_", "Boss_", "Abyss_", "abyss_", "Weekly", "weekly", "Wave",
            "Monster",
        ];

        fn shift_stream(data: &[u8], bit_shift: u8) -> Vec<u8> {
            if bit_shift == 0 {
                return data.to_vec();
            }
            if data.len() < 2 {
                return Vec::new();
            }
            (0..data.len() - 1)
                .map(|index| (data[index] >> bit_shift) | (data[index + 1] << (8 - bit_shift)))
                .collect()
        }

        fn hex_context(stream: &[u8], start: usize, end: usize) -> String {
            let from = start.saturating_sub(64);
            let to = (end + 64).min(stream.len());
            let end = end.min(stream.len());
            format!(
                "[-{}] {} |{}| {} [+{}]",
                start - from,
                hex::encode(&stream[from..start]),
                hex::encode(&stream[start..end]),
                hex::encode(&stream[end..to]),
                to - end,
            )
        }

        fn is_identifier_byte(byte: u8) -> bool {
            byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'.' | b'/' | b'-')
        }

        let path =
            std::env::var("NTE_TEST_CAPTURE_JSON").expect("NTE_TEST_CAPTURE_JSON must be set");
        let raw_text = std::fs::read_to_string(&path).expect("capture json should be readable");
        let raw_document: serde_json::Value =
            serde_json::from_str(&raw_text).expect("capture json should parse");
        let mut packets = Vec::new();
        if let Some(entries) = raw_document
            .get("packets")
            .and_then(|value| value.as_array())
        {
            for entry in entries {
                let Some(timestamp) = entry.get("timestamp_unix").and_then(|value| value.as_f64())
                else {
                    continue;
                };
                let direction = entry
                    .get("direction")
                    .and_then(|value| value.as_str())
                    .unwrap_or("?")
                    .to_owned();
                let Some(payload) = entry
                    .get("payload_hex")
                    .and_then(|value| value.as_str())
                    .and_then(|value| hex::decode(value).ok())
                else {
                    continue;
                };
                packets.push((timestamp, direction, payload));
            }
        }
        packets.sort_by(|left, right| left.0.total_cmp(&right.0));
        println!("packets: {}", packets.len());

        // Pass 1: collect every target handle this capture ever reports through
        // the known boss-HP anchor, so pass 2 can hunt for the same bytes
        // anywhere else.
        let mut handle_first_seen: Vec<([u8; 29], f64)> = Vec::new();
        let mut boss_hp_events = Vec::new();
        for (timestamp, _, payload) in &packets {
            for update in parse_boss_hp_updates(payload) {
                if !handle_first_seen
                    .iter()
                    .any(|(handle, _)| *handle == update.target_handle)
                {
                    handle_first_seen.push((update.target_handle, *timestamp));
                }
                boss_hp_events.push((*timestamp, update.target_handle));
            }
        }
        println!("--- handles seen through boss-HP anchor ---");
        for (handle, first_seen) in &handle_first_seen {
            println!(
                "handle {} first seen t={first_seen:.3}",
                hex::encode(handle)
            );
        }

        // Pass 2: single sweep over every packet at every bit shift, extracting
        // identifier strings and off-anchor handle occurrences.
        struct NameHit {
            timestamp: f64,
            direction: String,
            bit_shift: u8,
            offset: usize,
            handles_in_same_packet: Vec<String>,
            context: String,
        }
        let mut names: std::collections::BTreeMap<String, Vec<NameHit>> =
            std::collections::BTreeMap::new();
        let mut outside_handle_hits = Vec::new();
        let mut anchor_handle_hits = 0_usize;
        for (timestamp, direction, payload) in &packets {
            let mut packet_names: Vec<(String, u8, usize, String)> = Vec::new();
            let mut packet_handles: Vec<String> = Vec::new();
            for bit_shift in 0..8_u8 {
                let stream = shift_stream(payload, bit_shift);

                // Printable identifier runs containing one of the needles.
                let mut run_start = None;
                for index in 0..=stream.len() {
                    let printable = index < stream.len() && (0x20..=0x7e).contains(&stream[index]);
                    match (printable, run_start) {
                        (true, None) => run_start = Some(index),
                        (false, Some(start)) => {
                            run_start = None;
                            if index - start < 6 {
                                continue;
                            }
                            let run = &stream[start..index];
                            let text = String::from_utf8_lossy(run).into_owned();
                            for needle in NAME_NEEDLES {
                                for (needle_at, _) in text.match_indices(needle) {
                                    // Expand the needle match to the full
                                    // identifier around it so occurrences
                                    // dedup on the identifier, not on the
                                    // whole (noisy) printable run.
                                    let mut ident_start = needle_at;
                                    while ident_start > 0
                                        && is_identifier_byte(run[ident_start - 1])
                                    {
                                        ident_start -= 1;
                                    }
                                    let mut ident_end = needle_at + needle.len();
                                    while ident_end < run.len()
                                        && is_identifier_byte(run[ident_end])
                                    {
                                        ident_end += 1;
                                    }
                                    let identifier =
                                        String::from_utf8_lossy(&run[ident_start..ident_end])
                                            .into_owned();
                                    let already_seen =
                                        packet_names.iter().any(|(existing, shift, offset, _)| {
                                            *existing == identifier
                                                && (*shift != bit_shift
                                                    || *offset == start + ident_start)
                                        });
                                    if !already_seen {
                                        packet_names.push((
                                            identifier,
                                            bit_shift,
                                            start + ident_start,
                                            hex_context(
                                                &stream,
                                                start + ident_start,
                                                start + ident_end,
                                            ),
                                        ));
                                    }
                                }
                            }
                        }
                        _ => {}
                    }
                }

                // Handle bytes anywhere in the stream, raw and byte-reversed.
                for (handle, _) in &handle_first_seen {
                    let reversed: Vec<u8> = handle.iter().rev().copied().collect();
                    for (variant, needle) in
                        [("raw", handle.as_slice()), ("rev", reversed.as_slice())]
                    {
                        if stream.len() < needle.len() {
                            continue;
                        }
                        for offset in 0..=stream.len() - needle.len() {
                            if &stream[offset..offset + needle.len()] != needle {
                                continue;
                            }
                            let in_anchor = variant == "raw"
                                && offset >= 8
                                && stream[offset - 8..offset] == BOSS_HP_HEAD;
                            let label = hex::encode(handle);
                            if !packet_handles.contains(&label) {
                                packet_handles.push(label.clone());
                            }
                            if in_anchor {
                                anchor_handle_hits += 1;
                            } else {
                                outside_handle_hits.push((
                                    *timestamp,
                                    direction.clone(),
                                    bit_shift,
                                    offset,
                                    variant,
                                    label,
                                    hex_context(&stream, offset, offset + needle.len()),
                                ));
                            }
                        }
                    }
                }
            }
            for (identifier, bit_shift, offset, context) in packet_names {
                names.entry(identifier).or_default().push(NameHit {
                    timestamp: *timestamp,
                    direction: direction.clone(),
                    bit_shift,
                    offset,
                    handles_in_same_packet: packet_handles.clone(),
                    context,
                });
            }
        }

        println!("--- identifier summary ({} unique) ---", names.len());
        for (identifier, hits) in &names {
            let c2s = hits.iter().filter(|hit| hit.direction == "C2S").count();
            let with_handle = hits
                .iter()
                .filter(|hit| !hit.handles_in_same_packet.is_empty())
                .count();
            println!(
                "{identifier:?}: {} hits ({} C2S / {} other), {} in a packet that also carries a known handle, first t={:.3}",
                hits.len(),
                c2s,
                hits.len() - c2s,
                with_handle,
                hits[0].timestamp,
            );
            for hit in hits.iter().take(2) {
                println!(
                    "    t={:.3} dir={} shift={} off={} handles_in_pkt={:?}",
                    hit.timestamp,
                    hit.direction,
                    hit.bit_shift,
                    hit.offset,
                    hit.handles_in_same_packet,
                );
                println!("      ctx {}", hit.context);
            }
        }

        println!(
            "--- handle occurrences: {} inside boss-HP anchor, {} OUTSIDE ---",
            anchor_handle_hits,
            outside_handle_hits.len()
        );
        for (timestamp, direction, bit_shift, offset, variant, label, context) in
            outside_handle_hits.iter().take(40)
        {
            println!(
                "t={timestamp:.3} dir={direction} shift={bit_shift} off={offset} variant={variant} handle={label}"
            );
            println!("      ctx {context}");
        }
        if outside_handle_hits.len() > 40 {
            println!("...({} more outside hits)", outside_handle_hits.len() - 40);
        }

        // Pass 3: merged timeline of handle changes and identifier sightings
        // (per-identifier sightings collapsed when closer than 2s).
        let mut timeline: Vec<(f64, String)> = Vec::new();
        let mut last_handle: Option<[u8; 29]> = None;
        for (timestamp, handle) in &boss_hp_events {
            if last_handle != Some(*handle) {
                last_handle = Some(*handle);
                timeline.push((
                    *timestamp,
                    format!("BOSS_HP handle -> {}", hex::encode(handle)),
                ));
            }
        }
        for (identifier, hits) in &names {
            let mut last = f64::NEG_INFINITY;
            for hit in hits {
                if hit.timestamp - last >= 2.0 {
                    timeline.push((
                        hit.timestamp,
                        format!(
                            "NAME {identifier} dir={}{}",
                            hit.direction,
                            if hit.handles_in_same_packet.is_empty() {
                                String::new()
                            } else {
                                format!("  <- same packet as {:?}", hit.handles_in_same_packet)
                            }
                        ),
                    ));
                }
                last = hit.timestamp;
            }
        }
        timeline.sort_by(|left, right| left.0.total_cmp(&right.0));
        println!("--- timeline ({} events) ---", timeline.len());
        for (timestamp, line) in &timeline {
            println!("t={timestamp:.3} {line}");
        }
    }
}
