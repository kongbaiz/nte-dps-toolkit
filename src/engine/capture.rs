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
use std::time::Duration;

use chrono::{DateTime, Local};
use crossbeam_channel::{Receiver, Sender, TrySendError, bounded};
use libloading::Library;
use pcap_file::pcapng::blocks::enhanced_packet::EnhancedPacketBlock;
use pcap_file::pcapng::blocks::interface_description::{
    InterfaceDescriptionBlock, InterfaceDescriptionOption,
};
use pcap_file::pcapng::{Block, PcapNgReader, PcapNgWriter};
use pcap_file::{DataLink, PcapError};
use serde::{
    Deserialize, Deserializer, Serialize,
    de::Error as _,
    ser::{SerializeMap, SerializeSeq},
};

use crate::core::sync::{lock_or_repair, wait_or_repair};
use crate::engine::model::{
    AbyssEvent, AbyssHalf, COMBAT_CLOCK_RELEVANT_PAUSE_MASK, CharacterInfo,
    CombatClockRuntimeHealth, CombatState, DpsTimeBasis, EmptyCurtainCharacter, EmptyCurtainItem,
    EmptyCurtainPlacement, EngineEvent, Hit, HitCharacterSource, HitDamageCorrection, HitDirection,
    HitFollowUp, HtItemNetId, ModScriptEvent, ModScriptEventPhase, PacketDebug, PacketObservation,
    PartyCombatState, TimeStopEvent, UnattributedServerDamage,
};
use crate::engine::parser::{
    AbilityCatalog, DamageDisplayType, DamageRecordEncoding, ENEMY_CATALOG_PATH,
    EQUIPMENT_CATALOG_PATH, EquipmentCatalog, EquipmentKind, GAMEPLAY_EFFECT_MAPPING_PATH,
    GAMEPLAY_EFFECT_SEMANTICS_PATH, GameplayEffectSkill, ParsedEmptyCurtainEquipmentSnapshot,
    ParsedEquipmentSlot, ParsedGameplayEffect, ParsedServerDamageSettlement,
    SKILL_DAMAGE_DATA_PATH, classify_attack_type, damage_record_encoding_at,
    declared_character_ids_from_evidence, find_data_file, find_declared_character_evidence,
    find_final_tower_character_evidence, load_enemy_catalog, load_equipment_catalog,
    load_gameplay_effect_mapping, normalize_damage_name, parse_boss_hp_updates,
    parse_client_damage_boss_update, parse_client_fight_target_updates, parse_current_hp_updates,
    parse_damage_payload, parse_empty_curtain_character_owners,
    parse_empty_curtain_compact_module_placements, parse_empty_curtain_equipment_snapshot,
    parse_empty_curtain_item_additions, parse_empty_curtain_item_removals,
    parse_empty_curtain_items, parse_equipment_slots, parse_gameplay_effects,
    parse_server_damage_settlements, valid_item_net_id, validate_empty_curtain_snapshot,
};
use crate::platform::mods_plugin::CombatClockTransitionSnapshot;
use crate::storage::io_util::atomic_write_file;

use crate::engine::protocol::{
    BunchPacket, BunchReassembler, ReassembledBunch, SequencedPacket, SingleBunch, TransportPacket,
    append_bounded_bits, parse_bunch_packet, parse_expected_bunch_continuations,
    parse_inventory_bunches, parse_single_bunch, parse_transport_packet,
    parse_verified_bunch_starts, reliable_bunch_channel,
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
            Self::InvalidFormat(detail) => write!(formatter, "invalid pcapng: {detail}"),
            Self::ReceiverDisconnected => formatter.write_str("engine event receiver disconnected"),
            Self::Io(error) => write!(formatter, "cannot read pcapng: {error}"),
        }
    }
}

impl std::error::Error for PcapngImportError {}

/// Metadata preflight that rejects a bad path synchronously. The worker's
/// `PcapngImportReader` is the authoritative byte bound.
pub fn validate_pcapng_import(path: &Path) -> Result<(), PcapngImportError> {
    let metadata = std::fs::metadata(path).map_err(PcapngImportError::Io)?;
    if !metadata.is_file() {
        return Err(PcapngImportError::NotAFile);
    }
    let size = metadata.len();
    if size > MAX_PCAPNG_IMPORT_BYTES {
        return Err(PcapngImportError::TooLarge {
            size,
            limit: MAX_PCAPNG_IMPORT_BYTES,
        });
    }
    Ok(())
}

/// Total packet bytes need no separate budget: they are a subset of the file
/// bytes that `PcapngImportReader` already bounds.
fn account_pcapng_frame(size: usize, packet_count: &mut usize) -> Result<(), PcapngImportError> {
    if size > CAPTURE_SNAPLEN as usize {
        return Err(PcapngImportError::FrameTooLarge {
            size,
            limit: CAPTURE_SNAPLEN as usize,
        });
    }
    *packet_count += 1;
    if *packet_count > MAX_PCAPNG_IMPORT_PACKETS {
        return Err(PcapngImportError::TooManyPackets {
            count: *packet_count,
            limit: MAX_PCAPNG_IMPORT_PACKETS,
        });
    }
    Ok(())
}

fn account_pcapng_interface(
    snaplen: u32,
    interface_count: &mut usize,
) -> Result<(), PcapngImportError> {
    *interface_count += 1;
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
            let recovered;
            (state, recovered) =
                wait_or_repair(&self.available, &self.state, state, disconnect_byte_budget);
            if recovered {
                return Err(self.byte_budget_unavailable());
            }
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
        let (state, recovered) = lock_or_repair(&self.state, disconnect_byte_budget);
        if recovered {
            return Err(self.byte_budget_unavailable());
        }
        Ok(state)
    }

    fn byte_budget_unavailable(&self) -> String {
        self.available.notify_all();
        "capture frame queue byte budget became unavailable".to_owned()
    }

    fn release(&self, bytes: usize) {
        let (mut state, recovered) = lock_or_repair(&self.state, |state| {
            state.reserved = 0;
            state.receiver_connected = false;
        });
        if !recovered {
            // Reservations taken before a poison reset outlive the reset to 0.
            state.reserved = state.reserved.saturating_sub(bytes);
        }
        drop(state);
        self.available.notify_all();
    }

    fn disconnect_receiver(&self) {
        disconnect_byte_budget(&mut lock_or_repair(&self.state, |_| {}).0);
        self.available.notify_all();
    }

    fn observed_high_water(&self) -> usize {
        self.state
            .lock()
            .map_or(self.high_water, |state| state.observed_high_water)
    }
}

fn disconnect_byte_budget(state: &mut CaptureFrameByteBudgetState) {
    state.receiver_connected = false;
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
        let mut options = std::fs::OpenOptions::new();
        options.write(true).create(true).truncate(true);
        #[cfg(windows)]
        {
            use std::os::windows::fs::OpenOptionsExt;
            use windows_sys::Win32::Storage::FileSystem::{FILE_SHARE_READ, FILE_SHARE_WRITE};
            // Keep log cleanup from deleting a capture while its writer is alive.
            options.share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE);
        }
        let file = options.open(path).map_err(|error| {
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

    fn finish(mut self) -> Result<(u64, u64), String> {
        self.writer
            .get_mut()
            .flush()
            .map_err(|error| error.to_string())?;
        Ok((self.packet_count, self.captured_bytes))
    }
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
        || pause_type_mask & !COMBAT_CLOCK_RELEVANT_PAUSE_MASK != 0
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

#[derive(Default)]
struct GamePauseIntervalTracker {
    pause_type_mask: u32,
}

impl GamePauseIntervalTracker {
    fn apply_transition(&mut self, timestamp: f64, pause_type_mask: u32) -> Option<TimeStopEvent> {
        let previous_pause_type_mask = self.pause_type_mask;
        let event = if previous_pause_type_mask == 0 && pause_type_mask != 0 {
            Some(TimeStopEvent::GamePauseStarted {
                timestamp,
                pause_type_mask,
            })
        } else if previous_pause_type_mask != 0 && pause_type_mask == 0 {
            Some(TimeStopEvent::GamePauseEnded {
                timestamp,
                pause_type_mask: previous_pause_type_mask,
            })
        } else if previous_pause_type_mask != pause_type_mask {
            Some(TimeStopEvent::GamePauseMaskChanged {
                timestamp,
                pause_type_mask,
            })
        } else {
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
    if let Some((_, floor, half)) = explicit_stage {
        events.push(AbyssEvent::Stage {
            timestamp,
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
const MIN_FOLLOW_UP_RESIDUAL_DAMAGE: f64 = 1.0;
/// This window is used only to associate an already enum-classified appended
/// server value with its source hit. It never classifies a damage type or
/// derives a damage amount.
const AUTHORITATIVE_DISPLAY_SOURCE_WINDOW_SECONDS: f64 = 0.1;
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

#[derive(Clone)]
struct ServerDamagePendingHit {
    hit: Hit,
    target_handle: Option<[u8; 29]>,
    use_server_damage: bool,
    max_hp_reduction_percent: u32,
}

struct ServerDamageSettlementObservation {
    corrections: Vec<HitDamageCorrection>,
    unattributed: Vec<UnattributedServerDamage>,
    sources: Vec<Option<Hit>>,
}

struct ServerDamageReconciliation {
    corrections: Vec<HitDamageCorrection>,
    unattributed: Vec<UnattributedServerDamage>,
    residual_hits: Vec<Hit>,
    sources: Vec<Option<Hit>>,
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
    /// policy evicts the oldest unresolved hit. A target death, reset, max-HP
    /// increase, or ambiguity drops that target's candidates instead of
    /// guessing attribution.
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
                .is_some_and(|snapshot| {
                    nearly_same(hit.target_hp_before, hit.target_max_hp)
                        && (snapshot.hp <= 1.0 || hit.target_hp_before > snapshot.hp + 1.0)
                });
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
            source_byte_offset: Some(source_hit.byte_offset),
            source_bit_shift: Some(source_hit.bit_shift),
            source_target_id: source_hit.target_id.clone(),
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
            damage_name: None,
            attack_type: None,
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
        if correction.damage_name.is_some() {
            pending.hit.damage_name.clone_from(&correction.damage_name);
        }
        if correction.attack_type.is_some() {
            pending.hit.attack_type.clone_from(&correction.attack_type);
        }
        if let Some(reduction) = correction.max_hp_reduction {
            pending.hit.max_hp_reduction = reduction;
        }
    }

    fn clear_max_hp_reduction_candidates(&mut self, target_handle: [u8; 29]) {
        self.pending_max_hp_reduction_hits
            .retain(|pending| pending.target_handle != Some(target_handle));
    }

    fn remove_max_hp_reduction_candidate(&mut self, source_hit: &Hit) {
        let source_target = wire_handle_from_hit(source_hit);
        self.pending_max_hp_reduction_hits.retain(|pending| {
            !(pending.target_handle == source_target
                && pending.hit.timestamp.to_bits() == source_hit.timestamp.to_bits()
                && pending.hit.char_id == source_hit.char_id
                && pending.hit.gameplay_effect_index == source_hit.gameplay_effect_index)
        });
    }

    fn scaled_max_hp_reduction_for_settlement(
        &self,
        source: &ServerDamagePendingHit,
        previous: Option<ServerHpSnapshot>,
        current_hp: f64,
        raw_damage: f64,
        target_handle: [u8; 29],
    ) -> Option<(f64, f64)> {
        let previous = previous?;
        let previous_max_hp = self.target_max_hp_by_handle.get(&target_handle).copied()?;
        if source.max_hp_reduction_percent == 0 || previous_max_hp <= 0.0 {
            return None;
        }
        let reduction = raw_damage * f64::from(source.max_hp_reduction_percent) / 100.0;
        let next_max_hp = previous_max_hp - reduction;
        if reduction <= 0.0 || next_max_hp <= 0.0 {
            return None;
        }
        let hp_after_direct_damage = (previous.hp - raw_damage).max(0.0);
        let ordinary_error = (hp_after_direct_damage - current_hp).abs();
        let expected_scaled_hp = hp_after_direct_damage * next_max_hp / previous_max_hp;
        let scaled_error = (expected_scaled_hp - current_hp).abs();
        (ordinary_error > 0.5 && scaled_error <= 0.5).then_some((reduction, next_max_hp))
    }

    fn observe_server_damage_settlements_with_sources(
        &mut self,
        timestamp: f64,
        settlements: &[ParsedServerDamageSettlement],
    ) -> ServerDamageSettlementObservation {
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

        // One authoritative settlement owns at most one decoded source
        // occurrence. Reserve exact damage and HP-continuity matches across the
        // whole batch before falling back to wire order, so an earlier fuzzy
        // settlement cannot steal a later exact source. Duplicate hits remain
        // distinct by index and are therefore consumed one at a time. Fuzzy
        // FIFO assignments still calibrate server damage. A container's role
        // declaration constrains every candidate before the existing exact,
        // unique HP-bridge and unique recent source matching rules run.
        let mut candidate_rows = Vec::new();
        let mut hp_bridge_counts = vec![0_usize; settlements.len()];
        let mut recent_counts = vec![0_usize; settlements.len()];
        for (settlement_index, settlement) in settlements.iter().enumerate() {
            let current_hp = current_hps[settlement_index];
            let additional_damage = f64::from(settlement.additional_damage.unwrap_or(0));
            for (pending_index, pending) in self.pending_hits.iter().enumerate() {
                if pending.target_handle != Some(settlement.target_handle)
                    || pending.hit.timestamp > timestamp + f64::EPSILON
                    || settlement
                        .source_character_id
                        .is_some_and(|id| id != pending.hit.char_id)
                {
                    continue;
                }
                let exact_damage = pending.hit.damage == f64::from(settlement.raw_damage);
                let hp_bridge =
                    (pending.hit.target_hp_after - current_hp - additional_damage).abs() <= 1.0;
                let recent = timestamp >= pending.hit.timestamp
                    && timestamp - pending.hit.timestamp
                        <= AUTHORITATIVE_DISPLAY_SOURCE_WINDOW_SECONDS;
                if hp_bridge {
                    hp_bridge_counts[settlement_index] += 1;
                }
                if recent {
                    recent_counts[settlement_index] += 1;
                }
                candidate_rows.push((
                    settlement_index,
                    pending_index,
                    exact_damage,
                    hp_bridge,
                    recent,
                ));
            }
        }

        let mut candidates = candidate_rows
            .into_iter()
            .map(
                |(settlement_index, pending_index, exact_damage, hp_bridge, recent)| {
                    let unique_hp_bridge = hp_bridge && hp_bridge_counts[settlement_index] == 1;
                    let unique_recent = recent && recent_counts[settlement_index] == 1;
                    let priority = if exact_damage && hp_bridge {
                        0_u8
                    } else if exact_damage {
                        1
                    } else if hp_bridge {
                        2
                    } else if unique_recent {
                        3
                    } else {
                        4
                    };
                    (
                        priority,
                        settlement_index,
                        pending_index,
                        exact_damage || unique_hp_bridge || unique_recent,
                    )
                },
            )
            .collect::<Vec<_>>();
        candidates.sort_unstable();

        let mut assigned_pending = HashSet::new();
        let mut assignments = vec![None; settlements.len()];
        let mut assignment_can_attribute_follow_up = vec![false; settlements.len()];
        for (_, settlement_index, pending_index, can_attribute_follow_up) in candidates {
            if assignments[settlement_index].is_some() || assigned_pending.contains(&pending_index)
            {
                continue;
            }
            assigned_pending.insert(pending_index);
            assignments[settlement_index] = Some(pending_index);
            assignment_can_attribute_follow_up[settlement_index] = can_attribute_follow_up;
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
                if settlement.display_type.attack_type().is_some() {
                    let target_max_hp = self
                        .target_max_hp_by_handle
                        .get(&settlement.target_handle)
                        .copied()
                        .unwrap_or(0.0);
                    self.residual_hits.push(unattributed_display_damage_hit(
                        timestamp,
                        settlement,
                        raw_damage,
                        settlement.display_type,
                        target_max_hp,
                    ));
                    continue;
                }
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
            let confirmed_max_hp_reduction = self.scaled_max_hp_reduction_for_settlement(
                source,
                previous,
                current_hp,
                raw_damage,
                settlement.target_handle,
            );
            let target_max_hp = confirmed_max_hp_reduction
                .map(|(_, next_max_hp)| next_max_hp)
                .unwrap_or(source_hit.target_max_hp);
            let correction = HitDamageCorrection {
                source_timestamp: source_hit.timestamp,
                source_byte_offset: Some(source_hit.byte_offset),
                source_bit_shift: Some(source_hit.bit_shift),
                source_target_id: source_hit.target_id.clone(),
                source_char_id: source_hit.char_id,
                source_damage: source_hit.damage,
                source_target_hp_before: source_hit.target_hp_before,
                source_target_hp_after: source_hit.target_hp_after,
                source_target_max_hp: source_hit.target_max_hp,
                source_gameplay_effect_index: source_hit.gameplay_effect_index,
                damage: raw_damage,
                target_hp_before,
                target_hp_after: current_hp,
                target_hp_percent: if target_max_hp > 0.0 {
                    current_hp / target_max_hp * 100.0
                } else {
                    0.0
                },
                damage_name: settlement.display_type.damage_name().map(str::to_owned),
                attack_type: settlement.display_type.attack_type().map(str::to_owned),
                max_hp_reduction: confirmed_max_hp_reduction.map(|(reduction, _)| reduction),
                // A structurally validated Type 0x06 settlement is already the
                // exact server result. The legacy HP-delta preference controls
                // only fallback inference and must not discard this value.
                reconciled_overkill_damage: Some(0.0),
            };
            if let Some((_, next_max_hp)) = confirmed_max_hp_reduction {
                self.target_max_hp_by_handle
                    .insert(settlement.target_handle, next_max_hp);
                self.remove_max_hp_reduction_candidate(source_hit);
            } else {
                self.update_pending_max_hp_reduction_hit(source_hit, &correction);
            }
            corrections.push(correction);
        }
        for (settlement, current_hp) in settlements.iter().zip(current_hps.iter().copied()) {
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
        for (settlement, current_hp) in settlements.iter().zip(current_hps.iter().copied()) {
            if current_hp <= 0.0 {
                self.clear_max_hp_reduction_candidates(settlement.target_handle);
            }
        }
        ServerDamageSettlementObservation {
            corrections,
            unattributed,
            sources: sources
                .into_iter()
                .zip(assignment_can_attribute_follow_up)
                .map(
                    |(source, can_attribute_follow_up)| match (source, can_attribute_follow_up) {
                        (Some(source), true) => Some(source.hit),
                        _ => None,
                    },
                )
                .collect(),
        }
    }

    fn take_residual_hits(&mut self) -> Vec<Hit> {
        std::mem::take(&mut self.residual_hits)
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
        let target_died = current_hp <= 0.0;
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
            if target_died {
                self.clear_max_hp_reduction_candidates(update.target_handle);
            }
            return (None, None);
        };
        if current_hp >= previous.hp {
            self.pending_hits.retain(|pending| {
                pending.target_handle != Some(update.target_handle)
                    || pending.hit.timestamp > timestamp
            });
            if target_died {
                self.clear_max_hp_reduction_candidates(update.target_handle);
            }
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
            if target_died {
                self.clear_max_hp_reduction_candidates(update.target_handle);
            }
            return (None, None);
        }
        if candidate_count != 1 {
            // A decoded hit is already present in team/personal totals. Report
            // only the positive portion of the authoritative HP delta that the
            // decoded candidates do not explain, regardless of candidate
            // cardinality; diagnostics must not double-count visible damage.
            let residual = (damage - decoded_damage).max(0.0);
            if target_died {
                self.clear_max_hp_reduction_candidates(update.target_handle);
            }
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
            source_byte_offset: Some(source_hit.byte_offset),
            source_bit_shift: Some(source_hit.bit_shift),
            source_target_id: source_hit.target_id.clone(),
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
            damage_name: None,
            attack_type: None,
            // A current-HP delta can include proportional compression caused by
            // a max-HP change. Only an observed max-HP drop or the validated
            // scaled settlement formula may attribute max-HP reduction.
            max_hp_reduction: None,
            reconciled_overkill_damage: source.use_server_damage.then_some(0.0),
        };
        self.update_pending_max_hp_reduction_hit(source_hit, &correction);
        if target_died {
            self.clear_max_hp_reduction_candidates(update.target_handle);
        }
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
        // Unsupported Bunch modes may still carry raw records, but their packet IDs
        // must not seed or advance the inventory fragment clock. Supported packets
        // without inventory Bunches still advance that clock and expire stale fragments.
        let (streams, bunches_recognized) = if packet.mode == 0 {
            let state = self
                .connections
                .get_mut(&connection)
                .expect("new or existing inventory connection must be present");
            let known_channels = state.bunch_reassembler.known_channels().collect::<Vec<_>>();
            let bunches = parse_inventory_bunches(packet, &known_channels);
            let recognized = !bunches.is_empty();
            (state.push_bunches(packet.packet_id, bunches), recognized)
        } else {
            (Vec::new(), false)
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
    exact_mode: bool,
    exact_runtime: Option<crate::engine::settlement::runtime::Runtime>,
    exact_auto: Option<crate::engine::settlement::automatic::Automatic>,
    packet_emission: PacketEmissionMode,
    session_characters: HashMap<(Ipv4Addr, u16, Ipv4Addr, u16), u32>,
    client_endpoints: HashSet<(Ipv4Addr, u16)>,
    gameplay_effect_names: HashMap<u32, String>,
    ability_catalog: Arc<AbilityCatalog>,
    server_damage_calibration: ServerDamageCalibrationTracker,
    use_server_damage_calibration: bool,
    character_declarations: HashMap<u32, f64>,
    pending_ambiguous_hits: Vec<Hit>,
    /// Confirmed outgoing hits without a direct wire target wait for the
    /// following server target-state response and then briefly for a matching
    /// direct-target record. FIFO, bounded to `MAX_PENDING_FOLLOW_UP_HITS`;
    /// full policy emits the oldest hit, and capture shutdown flushes the rest.
    pending_targetless_hits: VecDeque<Hit>,
    recent_confirmed_hits: Vec<Hit>,
    target_snapshots: HashMap<String, HitTargetSnapshot>,
    exact_inventory: super::inventory::InventoryDecoder,
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
    fn enable_exact_mode(&mut self, sender: &EngineEventSink) {
        self.exact_mode = true;
        let _ = sender.send(EngineEvent::PacketInventory {
            items: vec![],
            characters: vec![],
        });
        match crate::engine::settlement::runtime::Runtime::from_environment() {
            Ok(Some(runtime)) => self.exact_runtime = Some(runtime),
            Ok(None) => match crate::engine::settlement::automatic::Automatic::bundled() {
                Ok(automatic) => {
                    self.exact_auto = Some(automatic);
                    let _ = sender.send(EngineEvent::Warning(
                        crate::engine::settlement::automatic::WAITING.into(),
                    ));
                }
                Err(code) => {
                    let _ = sender.send(EngineEvent::Error(code.into()));
                }
            },
            Err(code) => {
                let _ = sender.send(EngineEvent::Error(code.into()));
            }
        }
    }

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
            exact_mode: false,
            exact_runtime: None,
            exact_auto: None,
            packet_emission: PacketEmissionMode::FullDebug,
            session_characters: HashMap::new(),
            client_endpoints: HashSet::new(),
            gameplay_effect_names,
            ability_catalog,
            server_damage_calibration: ServerDamageCalibrationTracker::default(),
            use_server_damage_calibration,
            character_declarations: HashMap::new(),
            pending_ambiguous_hits: Vec::new(),
            pending_targetless_hits: VecDeque::new(),
            recent_confirmed_hits: Vec::new(),
            target_snapshots: HashMap::new(),
            exact_inventory: super::inventory::InventoryDecoder::new(Arc::new(equipment_catalog)),
            frame_dedup: FrameDedup::default(),
            gameplay_effect_fragments: GameplayEffectFragmentTracker::default(),
            bool_enum_gameplay_effect_fragments: BoolEnumGameplayEffectFragmentTracker::default(),
            bunch_connections: BunchConnectionTracker::default(),
            resource_warnings,
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
            let packet_target = unique_packet_target_before(hits, index);
            let tracked_target =
                packet_target.or_else(|| self.unique_tracked_target_for_hit(&hits[index]));
            if let Some((target_id, snapshot)) = tracked_target {
                let replace_target = match hits[index].target_id.as_ref() {
                    None => true,
                    Some(current_target) if current_target != &target_id => self
                        .target_snapshots
                        .get(current_target)
                        .is_some_and(|current| {
                            !target_snapshot_max_matches_hit(current, &hits[index])
                        }),
                    Some(_) => false,
                };
                if replace_target {
                    apply_target_snapshot(&mut hits[index], target_id, &snapshot);
                }
            }
            self.observe_hit_target(&hits[index]);
        }
    }

    fn unique_tracked_target_for_hit(&self, hit: &Hit) -> Option<(String, HitTargetSnapshot)> {
        let mut candidate = None;
        for (target_id, snapshot) in &self.target_snapshots {
            if !target_snapshot_matches_hit(snapshot, hit) {
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
        settlements: &[crate::engine::parser::ParsedServerDamageSettlement],
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
                hit.direction = HitDirection::Outgoing;
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
            let target_id = target_id_from_wire_handle(&target_handle);
            let Some(snapshot) = self.target_snapshots.get(&target_id).cloned() else {
                continue;
            };
            let has_matching_settlement = settlements.iter().any(|settlement| {
                settlement.target_handle == target_handle
                    && f64::from(settlement.current_hp).to_bits() == snapshot.current_hp.to_bits()
            });
            if !has_matching_settlement {
                if let Some(hit) = self.pending_targetless_hits.get_mut(index) {
                    apply_target_snapshot(hit, target_id, &snapshot);
                }
                continue;
            }
            if let Some(mut hit) = self.pending_targetless_hits.remove(index) {
                apply_target_snapshot(&mut hit, target_id, &snapshot);
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
        _characters: &HashMap<u32, CharacterInfo>,
        sender: &EngineEventSink,
        observe_server_damage: bool,
    ) {
        for hit in hits {
            if is_enemy_death_settlement_hit(&hit) {
                continue;
            }
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
            semantic.map_or(self.use_server_damage_calibration, |skill| {
                skill.use_server_damage
            }),
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
            prepared.suppressed_ambiguous +=
                self.suppress_matching_hp_resolved_targetless_hit(&hit);
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

    fn reconcile_authoritative_additional_damage_settlements(
        &mut self,
        timestamp: f64,
        settlements: &[ParsedServerDamageSettlement],
        sources: &[Option<Hit>],
    ) -> (Vec<HitFollowUp>, Vec<Hit>) {
        let mut follow_ups = Vec::new();
        let mut shared_hits = Vec::new();
        for (settlement_index, settlement) in settlements.iter().enumerate() {
            let (Some(additional_damage), Some(additional_display_type)) = (
                settlement.additional_damage,
                settlement.additional_display_type,
            ) else {
                continue;
            };
            let (Some(damage_name), Some(attack_type)) = (
                additional_display_type.damage_name(),
                additional_display_type.attack_type(),
            ) else {
                continue;
            };
            let source = sources
                .get(settlement_index)
                .and_then(Option::as_ref)
                .filter(|hit| {
                    settlement
                        .source_character_id
                        .is_none_or(|id| id == hit.char_id)
                });
            let Some(source) = source else {
                let target_max_hp = self
                    .server_damage_calibration
                    .target_max_hp_by_handle
                    .get(&settlement.target_handle)
                    .copied()
                    .unwrap_or(0.0);
                shared_hits.push(unattributed_display_damage_hit(
                    timestamp,
                    settlement,
                    additional_damage as f64,
                    additional_display_type,
                    target_max_hp,
                ));
                continue;
            };
            follow_ups.push(HitFollowUp {
                source_timestamp: source.timestamp,
                source_byte_offset: Some(source.byte_offset),
                source_bit_shift: Some(source.bit_shift),
                source_target_id: source.target_id.clone(),
                source_char_id: source.char_id,
                source_damage: source.damage,
                source_target_hp_before: source.target_hp_before,
                source_target_hp_after: source.target_hp_after,
                source_target_max_hp: source.target_max_hp,
                source_gameplay_effect_index: source.gameplay_effect_index,
                timestamp,
                damage: additional_damage as f64,
                target_hp_after: settlement.current_hp as f64,
                target_hp_percent: if source.target_max_hp > 0.0 {
                    settlement.current_hp as f64 / source.target_max_hp * 100.0
                } else {
                    0.0
                },
                damage_name: Some(damage_name.to_owned()),
                attack_type: Some(attack_type.to_owned()),
                damage_attribute: None,
            });
        }
        (follow_ups, shared_hits)
    }

    fn hp_update_has_authoritative_settlement(
        settlements: &[ParsedServerDamageSettlement],
        update: &crate::engine::parser::ParsedBossHpUpdate,
    ) -> bool {
        settlements.iter().any(|settlement| {
            settlement.target_handle == update.target_handle
                && settlement.current_hp.to_bits() == update.current_hp.to_bits()
        })
    }

    /// Reconciles legacy boss-HP synchronization for server-damage calibration.
    /// HP deltas can correct already-observed damage, but they never create a
    /// display-classified reaction record. Only a concrete `EDamageDisPlayType`
    /// entry decoded by `parse_server_damage_settlements` can do that.
    fn reconcile_boss_hp_updates(
        &mut self,
        timestamp: f64,
        boss_hp_updates: &[crate::engine::parser::ParsedBossHpUpdate],
    ) -> BossHpReconciliation {
        let mut server_damage_corrections = Vec::new();
        let mut unattributed_server_damage = Vec::new();
        for update in boss_hp_updates {
            let (correction, unattributed) = self
                .server_damage_calibration
                .observe_boss_hp_detailed(timestamp, update);
            unattributed_server_damage.extend(unattributed);
            let force_server_damage = correction
                .as_ref()
                .is_some_and(|correction| correction.reconciled_overkill_damage == Some(0.0));
            if force_server_damage {
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
        (
            Vec::new(),
            server_damage_corrections,
            unattributed_server_damage,
        )
    }

    fn reconcile_server_damage_settlements_with_sources(
        &mut self,
        timestamp: f64,
        settlements: &[ParsedServerDamageSettlement],
    ) -> ServerDamageReconciliation {
        let observation = self
            .server_damage_calibration
            .observe_server_damage_settlements_with_sources(timestamp, settlements);
        let mut unattributed = observation.unattributed;
        let mut accepted_corrections = Vec::with_capacity(observation.corrections.len());
        for correction in observation.corrections {
            if correction.reconciled_overkill_damage == Some(0.0) {
                accepted_corrections.push(correction);
                continue;
            }
            let residual = (correction.damage - correction.source_damage).max(0.0);
            if residual >= MIN_FOLLOW_UP_RESIDUAL_DAMAGE {
                unattributed.push(UnattributedServerDamage {
                    timestamp,
                    damage: residual,
                    candidate_hits: 1,
                });
            }
        }
        let residual_hits = self.server_damage_calibration.take_residual_hits();
        ServerDamageReconciliation {
            corrections: accepted_corrections,
            unattributed,
            residual_hits,
            sources: observation.sources,
        }
    }

    fn reconcile_current_packet_server_damage_settlements_with_sources<'a>(
        &mut self,
        timestamp: f64,
        settlements: &[ParsedServerDamageSettlement],
        hits: impl IntoIterator<Item = &'a Hit>,
    ) -> ServerDamageReconciliation {
        // A server settlement can be decoded from the same packet as its
        // client hit. Queue those hits before reconciliation while callers
        // preserve the public Hit -> HitDamageCorrection event order.
        let mut max_hp_reduction_corrections = self.observe_server_damage_hits(hits);
        let mut reconciliation =
            self.reconcile_server_damage_settlements_with_sources(timestamp, settlements);
        max_hp_reduction_corrections.append(&mut reconciliation.corrections);
        reconciliation.corrections = max_hp_reduction_corrections;
        reconciliation
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

    fn suppress_matching_hp_resolved_targetless_hit(&mut self, confirmed_hit: &Hit) -> usize {
        if !is_confirmed_packet_hit(confirmed_hit) || !has_direct_wire_target(confirmed_hit) {
            return 0;
        }
        let Some(index) = self
            .pending_targetless_hits
            .iter()
            .position(|pending| same_hp_resolved_damage_event(pending, confirmed_hit))
        else {
            return 0;
        };
        self.pending_targetless_hits.remove(index);
        1
    }
}

// Direct packet and reassembled views may expose different amounts of the
// same source header. Preserve occurrence counts and prefer exact role matches
// before using a missing declaration as a wildcard. Queues are packet-local;
// each direct occurrence is visited a bounded number of times.
fn merge_reassembled_server_damage_settlements(
    settlements: &mut Vec<ParsedServerDamageSettlement>,
    reassembled: Vec<ParsedServerDamageSettlement>,
) {
    let key = |row: &ParsedServerDamageSettlement| {
        (
            row.target_handle,
            row.current_hp.to_bits(),
            row.dead_state,
            row.raw_damage,
            row.display_type,
            row.additional_damage,
            row.additional_display_type,
        )
    };
    let mut by_source = HashMap::new();
    let mut by_value = HashMap::new();
    for (index, row) in settlements.iter().enumerate() {
        by_source
            .entry((key(row), row.source_character_id))
            .or_insert_with(VecDeque::new)
            .push_back(index);
        by_value
            .entry(key(row))
            .or_insert_with(VecDeque::new)
            .push_back(index);
    }
    let mut used = vec![false; settlements.len()];
    let mut assignments = vec![None; reassembled.len()];
    // Reserve known-to-known matches before using a missing declaration.
    for (index, row) in reassembled.iter().enumerate() {
        if row.source_character_id.is_none() {
            continue;
        }
        if let Some(original) = by_source
            .get_mut(&(key(row), row.source_character_id))
            .and_then(VecDeque::pop_front)
        {
            used[original] = true;
            assignments[index] = Some(original);
        }
    }
    // Known reassembled roles get the remaining unknown direct occurrences
    // before an unknown reassembled row can consume those occurrences.
    for (index, row) in reassembled.iter().enumerate() {
        if row.source_character_id.is_none() || assignments[index].is_some() {
            continue;
        }
        if let Some(original) = by_source
            .get_mut(&(key(row), None))
            .and_then(VecDeque::pop_front)
        {
            used[original] = true;
            assignments[index] = Some(original);
        }
    }
    for (index, row) in reassembled.iter().enumerate() {
        if row.source_character_id.is_some() {
            continue;
        }
        if let Some(original) = by_value.get_mut(&key(row)).and_then(|indices| {
            while let Some(index) = indices.pop_front() {
                if !used[index] {
                    return Some(index);
                }
            }
            None
        }) {
            used[original] = true;
            assignments[index] = Some(original);
        }
    }
    for (row, original) in reassembled.into_iter().zip(assignments) {
        if let Some(original) = original {
            if settlements[original].source_character_id.is_none() {
                settlements[original].source_character_id = row.source_character_id;
            }
        } else {
            settlements.push(row);
        }
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
            .split(|character: char| !character.is_ascii_alphanumeric())
            .any(|token| token.eq_ignore_ascii_case(name))
}

fn is_confirmed_packet_hit(hit: &Hit) -> bool {
    matches!(
        hit.char_source,
        HitCharacterSource::Packet | HitCharacterSource::GameplayEffect
    ) && hit.direction.is_outgoing()
}

fn should_defer_partial_stream_hit(hit: &Hit) -> bool {
    hit.target_id.is_none() || hit.char_source == HitCharacterSource::Session
}

fn promote_resolved_outgoing_hits(hits: &mut [Hit]) {
    for hit in hits {
        if hit.direction.is_unknown() && hit.target_id.is_some() {
            hit.direction = HitDirection::Outgoing;
        }
    }
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

fn has_direct_wire_target(hit: &Hit) -> bool {
    let Some(encoded) = hit
        .target_id
        .as_deref()
        .and_then(|target_id| target_id.strip_prefix("enemy-wire:"))
    else {
        return false;
    };
    hit.target_context.iter().any(|context| {
        context
            .strip_prefix("enemy_target_wire=")
            .is_some_and(|wire| wire == encoded)
    })
}

fn same_hp_resolved_damage_event(pending: &Hit, confirmed: &Hit) -> bool {
    pending.target_id.is_some()
        && pending.target_id == confirmed.target_id
        && pending.gameplay_effect_index.is_some()
        && pending.gameplay_effect_index == confirmed.gameplay_effect_index
        && (pending.timestamp - confirmed.timestamp).abs()
            <= AMBIGUOUS_HIT_CONFIRMATION_WINDOW_SECONDS
        && same_exact_wire_damage_event(pending, confirmed)
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
        exact: None,
        plugin_snapshot: None,
        wire_event: None,
    }
}

fn unattributed_display_damage_hit(
    timestamp: f64,
    settlement: &ParsedServerDamageSettlement,
    damage: f64,
    display_type: DamageDisplayType,
    target_max_hp: f64,
) -> Hit {
    let target_hp_after = f64::from(settlement.current_hp);
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
        target_hp_before: target_hp_after + damage,
        target_hp_after,
        target_max_hp,
        max_hp_reduction: 0.0,
        target_hp_percent: if target_max_hp > 0.0 {
            target_hp_after / target_max_hp * 100.0
        } else {
            0.0
        },
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
        damage_name: display_type.damage_name().map(str::to_owned),
        damage_component: None,
        attack_type: display_type.attack_type().map(str::to_owned),
        damage_attribute: None,
        follow_up_damage: 0.0,
        follow_up_timestamp: None,
        follow_up_damage_name: None,
        follow_up_attack_type: None,
        follow_up_damage_attribute: None,
        reconciled_overkill_damage: Some(0.0),
        exact: None,
        plugin_snapshot: None,
        wire_event: None,
    }
}

fn is_enemy_death_settlement_hit(hit: &Hit) -> bool {
    hit.direction.is_outgoing()
        && hit.damage == 1.0
        && hit.target_hp_before == 1.0
        && hit.target_hp_after == 0.0
}

fn pending_hit_matches_target_update(
    hit: &Hit,
    update_timestamp: f64,
    update: &crate::engine::parser::ParsedBossHpUpdate,
) -> bool {
    !hit.direction.is_incoming()
        && hit.target_id.is_none()
        && update_timestamp >= hit.timestamp
        && update_timestamp - hit.timestamp <= AMBIGUOUS_HIT_CONFIRMATION_WINDOW_SECONDS
        && nearly_same(hit.target_hp_after, normalized_server_hp(update.current_hp))
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

fn target_snapshot_matches_hit(snapshot: &HitTargetSnapshot, hit: &Hit) -> bool {
    nearly_same(snapshot.current_hp, hit.target_hp_before)
        && target_snapshot_max_matches_hit(snapshot, hit)
}

fn target_snapshot_max_matches_hit(snapshot: &HitTargetSnapshot, hit: &Hit) -> bool {
    snapshot.max_hp <= 0.0
        || hit.target_max_hp <= 0.0
        || nearly_same(snapshot.max_hp, hit.target_max_hp)
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
        if is_authoritative_display_attack_type(&skill.attack_type) {
            hit.attack_type = None;
        } else {
            hit.attack_type = Some(skill.attack_type.clone());
        }
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

fn is_authoritative_display_attack_type(attack_type: &str) -> bool {
    if attack_type == "环合伤害" || attack_type.starts_with("环合·") {
        return true;
    }
    [
        DamageDisplayType::Unbal,
        DamageDisplayType::GuangLingReactionFollow,
        DamageDisplayType::LingZhouReactionFollow,
        DamageDisplayType::ZhouAnReactionFollow,
        DamageDisplayType::AnHunReactionFollow,
        DamageDisplayType::HunXiangReactionFollow,
        DamageDisplayType::XiangGuangReactionFollow,
    ]
    .into_iter()
    .filter_map(DamageDisplayType::attack_type)
    .any(|authoritative| attack_type == authoritative)
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
    if hit.direction.is_incoming() {
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

impl PacketDecoder {
    fn flush_inventory(&mut self, sender: &EngineEventSink) {
        match self.exact_inventory.finish() {
            Ok(Some(update)) => {
                let _ = sender.send(EngineEvent::PacketInventory {
                    items: update.items,
                    characters: update.characters,
                });
            }
            Err(_) => {
                let _ = sender.send(EngineEvent::Warning(
                    "exact_inventory_incomplete_capture".into(),
                ));
                let _ = sender.send(EngineEvent::PacketInventory {
                    items: vec![],
                    characters: vec![],
                });
            }
            _ => {}
        }
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
        let mut exact_components = 0;
        if let Some(automatic) = self.exact_auto.as_mut() {
            let was_ready = automatic.ready;
            let projections = automatic.datagram(
                src,
                src_port,
                dst,
                dst_port,
                local_ip,
                payload,
                capture_timestamp,
                characters,
                include_incoming,
            );
            for projection in projections {
                let _ = sender.send(EngineEvent::ExactSettlement(Box::new(projection)));
            }
            if automatic.ready && !was_ready {
                let _ = sender.send(EngineEvent::Status(
                    crate::engine::settlement::automatic::READY.into(),
                ));
            } else if was_ready && !automatic.ready {
                let _ = sender.send(EngineEvent::Warning(
                    crate::engine::settlement::automatic::WAITING.into(),
                ));
            }
            if let Some(code) = automatic.warning {
                let _ = sender.send(EngineEvent::Warning(code.into()));
            }
            exact_components = automatic.last_settlement_components;
        }
        if let Some(runtime) = self.exact_runtime.as_mut() {
            match runtime.datagram(
                src,
                src_port,
                dst,
                dst_port,
                payload,
                capture_timestamp,
                characters,
                include_incoming,
            ) {
                Ok(projections) => {
                    for p in projections {
                        let _ = sender.send(EngineEvent::ExactSettlement(Box::new(p)));
                    }
                }
                Err(code) => {
                    let _ = sender.send(EngineEvent::Error(code.into()));
                }
            }
            if let Some(code) = runtime.warning {
                let _ = sender.send(EngineEvent::Warning(code.into()));
            }
            exact_components = runtime.last_settlement_components;
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
        let mut hits = if outgoing && !self.exact_mode {
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
            promote_resolved_outgoing_hits(&mut hits);
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
                if should_defer_partial_stream_hit(&hit)
                    && self.pending_ambiguous_hits.len() < MAX_PENDING_FOLLOW_UP_HITS
                {
                    self.pending_ambiguous_hits.push(hit);
                } else {
                    retained.push(hit);
                }
            }
            hits = retained;
        }
        if outgoing && !self.exact_mode {
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
        if outgoing {
            promote_resolved_outgoing_hits(&mut hits);
        }
        let prepared_hits =
            self.prepare_hits_for_emission(hits, &ids, include_incoming, characters);
        for character_id in &ids {
            self.character_declarations.insert(*character_id, timestamp);
        }
        self.character_declarations
            .retain(|_, declared_at| timestamp - *declared_at <= 10.0);
        let mut accepted = prepared_hits.emit.len() + exact_components;
        // CurrentHP 候选缺少目标 handle 校验，仅用于调试显示，不参与 follow-up 计算。
        let current_hp_updates = if outgoing || self.exact_mode {
            Vec::new()
        } else {
            parse_current_hp_updates(payload)
        };
        let direct_client_damage_boss = if outgoing || self.exact_mode {
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
        let mut target_hp_updates = if outgoing || self.exact_mode {
            Vec::new()
        } else {
            parse_client_fight_target_updates(payload)
        };
        if !outgoing && !self.exact_mode {
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
        let mut server_damage_settlements = if outgoing || self.exact_mode {
            Vec::new()
        } else {
            parse_server_damage_settlements(payload)
        };
        if !outgoing && !self.exact_mode {
            let reassembled_settlements = reassembled_bunches
                .iter()
                .flat_map(|observation| parse_server_damage_settlements(&observation.bunch.data))
                .collect();
            merge_reassembled_server_damage_settlements(
                &mut server_damage_settlements,
                reassembled_settlements,
            );
        }
        let target_hp_keys = target_hp_updates
            .iter()
            .map(|update| (update.target_handle, update.current_hp.to_bits()))
            .collect::<HashSet<_>>();
        let mut boss_hp_updates = match direct_client_damage_boss {
            Some(update) => vec![update],
            None if !outgoing && !self.exact_mode => parse_boss_hp_updates(payload)
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
        for settlement in &server_damage_settlements {
            if target_evidence_keys
                .insert((settlement.target_handle, settlement.current_hp.to_bits()))
            {
                let update = crate::engine::parser::ParsedBossHpUpdate {
                    target_handle: settlement.target_handle,
                    current_hp: settlement.current_hp,
                    byte_offset: settlement.byte_offset,
                    bit_shift: settlement.bit_shift,
                };
                self.observe_target_hp_update(timestamp, &update);
                target_evidence.push(update);
            }
        }
        let resolved_target_hits = self.resolve_pending_hit_targets(
            timestamp,
            &target_evidence,
            &server_damage_settlements,
        );
        accepted += resolved_target_hits.len();
        // Inventory direction must not depend on byte-scanned character guesses.
        let inventory_inbound = local_ip.map_or(!src.is_private() && dst.is_private(), |local| {
            dst == local && src != local
        });
        let inventory_result = if inventory_inbound {
            match self.exact_inventory.datagram(
                format!("{src}:{src_port}"),
                format!("{dst}:{dst_port}"),
                payload,
                timestamp,
            ) {
                Ok(Some(update)) => InventoryPacketResult {
                    snapshot: Some(update.items),
                    characters: Some(update.characters),
                    recognized: true,
                },
                Ok(None) => InventoryPacketResult::default(),
                Err(_) => {
                    let _ = sender.send(EngineEvent::Warning(
                        "exact_inventory_unavailable_or_unsupported_update".into(),
                    ));
                    InventoryPacketResult {
                        snapshot: Some(vec![]),
                        characters: Some(vec![]),
                        recognized: true,
                    }
                }
            }
        } else {
            InventoryPacketResult::default()
        };
        if let (Some(items), Some(characters)) =
            (inventory_result.snapshot, inventory_result.characters)
        {
            let _ = sender.send(EngineEvent::PacketInventory { items, characters });
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
        if current_hp_updates.is_empty()
            && target_hp_updates.is_empty()
            && boss_hp_updates.is_empty()
            && server_damage_settlements.is_empty()
            && prepared_hits.deferred_ambiguous == 0
            && prepared_hits.deferred_targetless == 0
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
            let current_packet_hits = resolved_target_hits
                .iter()
                .chain(prepared_hits.emit.iter())
                .collect::<Vec<_>>();
            let reconciliation = self
                .reconcile_current_packet_server_damage_settlements_with_sources(
                    timestamp,
                    &server_damage_settlements,
                    current_packet_hits.iter().copied(),
                );
            let (inferred_follow_ups, authoritative_shared_hits) = self
                .reconcile_authoritative_additional_damage_settlements(
                    timestamp,
                    &server_damage_settlements,
                    &reconciliation.sources,
                );
            let mut server_damage_corrections = reconciliation.corrections;
            let mut unattributed_server_damage = reconciliation.unattributed;
            let mut server_residual_hits = reconciliation.residual_hits;
            server_residual_hits.extend(authoritative_shared_hits);
            let unclaimed_boss_hp_updates = boss_hp_updates
                .iter()
                .filter(|update| {
                    !Self::hp_update_has_authoritative_settlement(
                        &server_damage_settlements,
                        update,
                    )
                })
                .cloned()
                .collect::<Vec<_>>();
            let (_, legacy_server_damage_corrections, legacy_unattributed_server_damage) =
                self.reconcile_boss_hp_updates(timestamp, &unclaimed_boss_hp_updates);
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
                    "Server damage settlements: {} rows, raw={}, additional_rows={}, additional_damage={}",
                    server_damage_settlements.len(),
                    server_damage_settlements
                        .iter()
                        .map(|settlement| u64::from(settlement.raw_damage))
                        .sum::<u64>(),
                    server_damage_settlements
                        .iter()
                        .filter(|settlement| settlement.additional_damage.is_some())
                        .count(),
                    server_damage_settlements
                        .iter()
                        .filter_map(|settlement| settlement.additional_damage)
                        .map(u64::from)
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
        let current_packet_hits = resolved_target_hits
            .iter()
            .chain(prepared_hits.emit.iter())
            .collect::<Vec<_>>();
        let reconciliation = self.reconcile_current_packet_server_damage_settlements_with_sources(
            timestamp,
            &server_damage_settlements,
            current_packet_hits.iter().copied(),
        );
        let (inferred_follow_ups, authoritative_shared_hits) = self
            .reconcile_authoritative_additional_damage_settlements(
                timestamp,
                &server_damage_settlements,
                &reconciliation.sources,
            );
        let mut server_damage_corrections = reconciliation.corrections;
        let mut unattributed_server_damage = reconciliation.unattributed;
        let mut server_residual_hits = reconciliation.residual_hits;
        server_residual_hits.extend(authoritative_shared_hits);
        let unclaimed_boss_hp_updates = boss_hp_updates
            .iter()
            .filter(|update| {
                !Self::hp_update_has_authoritative_settlement(&server_damage_settlements, update)
            })
            .cloned()
            .collect::<Vec<_>>();
        let (_, legacy_server_damage_corrections, legacy_unattributed_server_damage) =
            self.reconcile_boss_hp_updates(timestamp, &unclaimed_boss_hp_updates);
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
    decoder.enable_exact_mode(&sender);
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
    decoder.flush_inventory(&sender);
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
    if parser_thread.join().is_err() && loop_result.is_ok() {
        loop_result = Err("capture parser thread stopped unexpectedly".to_owned());
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
    // replay owner or allocating parser state. If the file grows afterwards,
    // the worker's bounded reader fails closed.
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
    decoder.enable_exact_mode(&sender);
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
            let mut previous_recorded_clock_health = None;

            while let Some(block) = reader.next_block() {
                if stop.load(Ordering::Relaxed) {
                    break;
                }
                block_count += 1;
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
                        account_pcapng_frame(packet.data.len(), &mut packet_count)?;
                        (
                            packet.interface_id as usize,
                            FrameTimestamp::Known(packet.timestamp.as_secs_f64()),
                            packet.data.into_owned(),
                        )
                    }
                    Block::SimplePacket(packet) => {
                        account_pcapng_frame(packet.data.len(), &mut packet_count)?;
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
            decoder.flush_inventory(&sender);
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
    GamePauseMaskChanged {
        timestamp: f64,
        pause_type_mask: u32,
    },
    // Variants below come from exports written by releases before 2026-07-27
    // (v0.3.x). Remove them when CAPTURE_EXPORT_VERSION is bumped.
    GamePause {
        start_timestamp: f64,
        end_timestamp: f64,
        pause_type_mask: u32,
    },
    GameTimerSample(serde::de::IgnoredAny),
    UltraAnimation(serde::de::IgnoredAny),
    ExtraStart(serde::de::IgnoredAny),
    ExtraEnd(serde::de::IgnoredAny),
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
            CaptureTimeStopEvent::GamePauseMaskChanged {
                timestamp,
                pause_type_mask,
            } => {
                validate_saved_pause_state(timestamp, pause_type_mask).map_err(D::Error::custom)?;
                events.push(TimeStopEvent::GamePauseMaskChanged {
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
            CaptureTimeStopEvent::GameTimerSample(_)
            | CaptureTimeStopEvent::UltraAnimation(_)
            | CaptureTimeStopEvent::ExtraStart(_)
            | CaptureTimeStopEvent::ExtraEnd(_) => {}
        }
    }
    Ok(events)
}

fn validate_saved_pause_state(timestamp: f64, pause_type_mask: u32) -> Result<(), &'static str> {
    if !timestamp.is_finite()
        || pause_type_mask == 0
        || pause_type_mask & !COMBAT_CLOCK_RELEVANT_PAUSE_MASK != 0
    {
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
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        serialize_with = "crate::engine::model::plugin_snapshot::serialize",
        deserialize_with = "crate::engine::model::plugin_snapshot::deserialize"
    )]
    plugin_snapshot:
        Option<std::sync::Arc<crate::engine::model::plugin_snapshot::PluginHitSnapshot>>,
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
    declared_ids: Vec<u32>,
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

fn deserialize_export_declared_ids<'de, D>(deserializer: D) -> Result<Vec<u32>, D::Error>
where
    D: Deserializer<'de>,
{
    struct DeclaredIdsVisitor;

    impl<'de> serde::de::Visitor<'de> for DeclaredIdsVisitor {
        type Value = Vec<u32>;

        fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            formatter.write_str("an array of at most 64 unsigned 32-bit ids")
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
                ids.push(value);
            }
            Ok(ids)
        }
    }

    deserializer.deserialize_seq(DeclaredIdsVisitor)
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
            plugin_snapshot: hit.plugin_snapshot.clone(),
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
            declared_ids: packet.declared_ids.clone(),
            parsed_hits: packet.parsed_hits,
            note: packet.note.clone(),
            payload_preview: packet.payload_preview.clone(),
            payload_hex: packet.payload_hex.clone(),
            decoded_text: packet.decoded_text.clone(),
        }
    }
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

fn read_capture_json_import(path: &Path) -> Result<String, CaptureImportError> {
    let file = File::open(path).map_err(CaptureImportError::Io)?;
    let metadata = file.metadata().map_err(CaptureImportError::Io)?;
    if !metadata.is_file() {
        return Err(CaptureImportError::NotAFile);
    }
    read_bounded_utf8(file, metadata.len(), MAX_CAPTURE_JSON_IMPORT_BYTES)
}

fn validate_capture_export_structure(
    document: &CaptureExportDocument,
) -> Result<(), CaptureImportError> {
    if document.hits.len() > MAX_CAPTURE_JSON_IMPORT_HITS {
        return Err(CaptureImportError::TooManyHits {
            count: document.hits.len(),
            limit: MAX_CAPTURE_JSON_IMPORT_HITS,
        });
    }
    if document.packets.len() > MAX_CAPTURE_JSON_IMPORT_PACKETS {
        return Err(CaptureImportError::TooManyPackets {
            count: document.packets.len(),
            limit: MAX_CAPTURE_JSON_IMPORT_PACKETS,
        });
    }
    validate_capture_collection(
        "party",
        document.party.len(),
        MAX_CAPTURE_JSON_IMPORT_PARTY_ROWS,
    )?;
    validate_capture_collection(
        "abyss.first_half.party",
        document.abyss.first_half.party.len(),
        MAX_CAPTURE_JSON_IMPORT_PARTY_ROWS,
    )?;
    validate_capture_collection(
        "abyss.second_half.party",
        document.abyss.second_half.party.len(),
        MAX_CAPTURE_JSON_IMPORT_PARTY_ROWS,
    )?;
    validate_capture_collection(
        "empty_curtain",
        document.empty_curtain.len(),
        MAX_CAPTURE_JSON_IMPORT_EMPTY_CURTAIN_ITEMS,
    )?;
    validate_capture_collection(
        "empty_curtain_characters",
        document.empty_curtain_characters.len(),
        MAX_CAPTURE_JSON_IMPORT_EMPTY_CURTAIN_CHARACTERS,
    )?;
    validate_capture_collection(
        "time_stop_events",
        document.time_stop_events.len(),
        MAX_CAPTURE_JSON_IMPORT_TIME_STOP_EVENTS,
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
            MAX_CAPTURE_JSON_IMPORT_ITEM_STATS,
        )?;
        validate_capture_collection(
            "empty_curtain[].sub_stats",
            item.sub_stats.len(),
            MAX_CAPTURE_JSON_IMPORT_ITEM_STATS,
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
            MAX_CAPTURE_JSON_IMPORT_TARGET_CONTEXT,
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
            | TimeStopEvent::GamePauseEnded { timestamp, .. }
            | TimeStopEvent::GamePauseMaskChanged { timestamp, .. } => *timestamp,
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
    let text = read_capture_json_import(path)?;
    let mut document = parse_capture_export_typed(text)?;
    validate_capture_export_structure(&document)?;
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
        MAX_CAPTURE_JSON_IMPORT_EMPTY_CURTAIN_CHARACTERS,
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
        let (mut state, mut recovered) = lock_or_repair(&self.state, cancel_delivery);
        while *state == EngineEventDeliveryState::Pending {
            (state, recovered) = wait_or_repair(&self.ready, &self.state, state, cancel_delivery);
        }
        if recovered {
            self.ready.notify_all();
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
        let (mut state, recovered) = lock_or_repair(&gate.state, cancel_delivery);
        if !recovered {
            *state = next;
        }
        drop(state);
        gate.ready.notify_all();
    }
}

fn cancel_delivery(state: &mut EngineEventDeliveryState) {
    *state = EngineEventDeliveryState::Cancelled;
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
    let mut declared_ids = packet.declared_ids;
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
        | TimeStopEvent::GamePauseEnded { timestamp, .. }
        | TimeStopEvent::GamePauseMaskChanged { timestamp, .. } => *timestamp,
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
        exact: None,
        plugin_snapshot: hit.plugin_snapshot,
        wire_event: None,
    }))
}

fn parse_capture_export_typed(
    text: impl Into<String>,
) -> Result<CaptureExportDocument, CaptureImportError> {
    let document: CaptureExportDocument =
        serde_json::from_str(&text.into()).map_err(|_| CaptureImportError::InvalidFormat)?;
    if document.version != CAPTURE_EXPORT_VERSION {
        return Err(CaptureImportError::UnsupportedVersion {
            found: document.version,
            expected: CAPTURE_EXPORT_VERSION,
        });
    }
    Ok(document)
}

#[cfg(test)]
mod tests;
