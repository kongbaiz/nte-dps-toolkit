use std::borrow::Cow;
use std::collections::{HashMap, HashSet, VecDeque};
use std::ffi::{CStr, CString, c_char, c_int, c_uchar, c_uint};
use std::fs::File;
use std::io::{BufWriter, Write};
use std::net::Ipv4Addr;
use std::path::{Path, PathBuf};
use std::ptr;
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, Ordering},
};
use std::thread;
use std::time::Duration;

use chrono::Local;
use crossbeam_channel::{Receiver, Sender, TrySendError, bounded};
use libloading::Library;
use pcap_file::DataLink;
use pcap_file::pcapng::blocks::enhanced_packet::EnhancedPacketBlock;
use pcap_file::pcapng::blocks::interface_description::{
    InterfaceDescriptionBlock, InterfaceDescriptionOption,
};
use pcap_file::pcapng::{Block, PcapNgReader, PcapNgWriter};
use serde::Deserialize;

use crate::engine::model::{
    AbyssEvent, AbyssHalf, CharacterInfo, EmptyCurtainCharacter, EmptyCurtainItem, EngineEvent,
    Hit, HitDamageCorrection, HitFollowUp, HtItemNetId, PacketDebug, PacketObservation,
    TimeStopEvent,
};
use crate::engine::parser::{
    ABILITY_TIPS_PATH, EQUIPMENT_CATALOG_PATH, EquipmentCatalog, GAMEPLAY_EFFECT_MAPPING_PATH,
    GameplayEffectSkill, ParsedEquipmentSlot, ParsedGameplayEffect, SKILL_DAMAGE_DATA_PATH,
    ULTRA_TIME_STOP_DATA_PATH, UltraTimeStopEntry, classify_attack_type,
    classify_attack_type_from_description, declared_character_ids_from_evidence, find_data_file,
    find_declared_character_evidence, find_final_tower_character_evidence, load_ability_tip_names,
    load_equipment_catalog, load_gameplay_effect_mapping, load_gameplay_effect_skills,
    load_ultra_time_stops, matches_shifted_bytes_at, normalize_damage_name, parse_boss_hp_updates,
    parse_current_hp_updates, parse_damage_payload, parse_empty_curtain_character_owners,
    parse_empty_curtain_items, parse_equipment_slots, parse_gameplay_effects, qte_reaction_type,
    valid_item_net_id, validate_empty_curtain_snapshot,
};
use crate::storage::i18n;

use crate::engine::protocol::{
    SequencedPacket, SingleBunch, TransportPacket, parse_inventory_bunches, parse_single_bunch,
    parse_transport_packet,
};

const PCAP_ERRBUF_SIZE: usize = 256;
const MIN_READABLE_TEXT_LEN: usize = 4;
const MAX_IGNORABLE_BINARY_PACKET_LEN: usize = 96;
const UNREADABLE_PROTOCOL_TEXT: &str = "未解析到可读协议文本";
const CAPTURE_SNAPLEN: u32 = 65_535;
const RAW_CAPTURE_FLUSH_INTERVAL: u64 = 256;
/// Bounded queue between the acquisition thread and the parser thread. Large enough that realistic
/// game traffic never fills it, so a parse latency spike no longer stalls `pcap_next_ex` (which
/// would let the Npcap kernel buffer overflow and drop frames). If it ever fills under pathological
/// load the frame is dropped for live parsing only — the raw frame is already written to the
/// PCAPNG and stays recoverable via Debug replay.
const CAPTURE_FRAME_QUEUE_CAPACITY: usize = 16_384;

struct CaptureFrame {
    data: Vec<u8>,
    timestamp: f64,
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

struct PcapHandle {
    raw: *mut PcapT,
    close: Close,
}

impl PcapHandle {
    fn new(raw: *mut PcapT, close: Close) -> Self {
        Self { raw, close }
    }

    fn as_ptr(&self) -> *mut PcapT {
        self.raw
    }
}

impl Drop for PcapHandle {
    fn drop(&mut self) {
        if !self.raw.is_null() {
            unsafe {
                (self.close)(self.raw);
            }
            self.raw = ptr::null_mut();
        }
    }
}

struct BpfProgramGuard {
    program: BpfProgram,
    free_code: FreeCode,
    active: bool,
}

impl BpfProgramGuard {
    fn new(free_code: FreeCode) -> Self {
        Self {
            program: BpfProgram {
                bf_len: 0,
                bf_insns: ptr::null_mut(),
            },
            free_code,
            active: true,
        }
    }

    fn as_mut(&mut self) -> &mut BpfProgram {
        &mut self.program
    }

    fn release(&mut self) {
        if self.active {
            unsafe {
                (self.free_code)(&mut self.program);
            }
            self.active = false;
        }
    }
}

impl Drop for BpfProgramGuard {
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
    pub sender: Sender<EngineEvent>,
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
        self.stop.store(true, Ordering::Relaxed);
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

struct RawCaptureData {
    path: Option<PathBuf>,
    writer: Option<RawCaptureWriter>,
    packet_count: u64,
    captured_bytes: u64,
    write_error: Option<String>,
}

impl RawCaptureBuffer {
    fn new(device: CaptureDevice, directory: Option<&std::path::Path>) -> Self {
        let timestamp = Local::now().format("%Y%m%d_%H%M%S_%3f");
        let path = directory.map(|directory| directory.join(format!("nte_raw_{timestamp}.pcapng")));
        let (writer, write_error) = match path.as_deref() {
            Some(path) => match RawCaptureWriter::create(path, &device) {
                Ok(writer) => (Some(writer), None),
                Err(error) => (None, Some(error)),
            },
            None => (None, None),
        };
        Self {
            inner: Arc::new(Mutex::new(RawCaptureData {
                path,
                writer,
                packet_count: 0,
                captured_bytes: 0,
                write_error,
            })),
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
    fn create(path: &std::path::Path, device: &CaptureDevice) -> Result<Self, String> {
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
        let mut interface = InterfaceDescriptionBlock::new(DataLink::ETHERNET, CAPTURE_SNAPLEN);
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

unsafe fn load_symbol<T: Copy>(library: &Library, name: &[u8]) -> Result<T, String> {
    // SAFETY: The requested names and signatures match the public libpcap API.
    unsafe {
        library
            .get::<T>(name)
            .map(|symbol| *symbol)
            .map_err(|error| error.to_string())
    }
}

fn c_string(value: *const c_char) -> String {
    if value.is_null() {
        String::new()
    } else {
        // SAFETY: libpcap returns null-terminated strings valid during this call.
        unsafe { CStr::from_ptr(value).to_string_lossy().into_owned() }
    }
}

pub fn list_devices() -> Result<Vec<CaptureDevice>, String> {
    // SAFETY: Loading a known Npcap DLL and calling its documented API.
    unsafe {
        let _packet_library = Library::new(packet_library_path())
            .map_err(|error| format!("无法加载 Npcap Packet.dll: {error}"))?;
        let library = Library::new(npcap_library_path())
            .map_err(|error| format!("无法加载 Npcap，请先安装 Npcap: {error}"))?;
        let find_all_devs: FindAllDevs = load_symbol(&library, b"pcap_findalldevs\0")?;
        let free_all_devs: FreeAllDevs = load_symbol(&library, b"pcap_freealldevs\0")?;
        let mut devices_ptr = ptr::null_mut();
        let mut error_buffer = [0_i8; PCAP_ERRBUF_SIZE];
        if find_all_devs(&mut devices_ptr, error_buffer.as_mut_ptr()) != 0 {
            return Err(c_string(error_buffer.as_ptr()));
        }
        let mut result = Vec::new();
        let mut current = devices_ptr;
        while !current.is_null() {
            let device = &*current;
            let mut ipv4 = Vec::new();
            let mut address = device.addresses;
            while !address.is_null() {
                let addr = (*address).addr;
                if !addr.is_null() && (*addr).family == 2 {
                    let bytes = &(*addr).data;
                    ipv4.push(Ipv4Addr::new(bytes[2], bytes[3], bytes[4], bytes[5]));
                }
                address = (*address).next;
            }
            result.push(CaptureDevice {
                name: c_string(device.name),
                description: c_string(device.description),
                ipv4,
            });
            current = device.next;
        }
        free_all_devs(devices_ptr);
        Ok(result)
    }
}

fn parse_udp_ipv4(packet: &[u8]) -> Option<(Ipv4Addr, u16, Ipv4Addr, u16, &[u8])> {
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
    let ip = &packet[ethernet_offset..];
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

fn infer_outgoing(
    src: Ipv4Addr,
    src_port: u16,
    dst: Ipv4Addr,
    local_ip: Option<Ipv4Addr>,
    ids: &[u32],
    client_endpoints: &HashSet<(Ipv4Addr, u16)>,
) -> bool {
    if let Some(local_ip) = local_ip {
        return src == local_ip;
    }
    match (src.is_private(), dst.is_private()) {
        (true, false) => true,
        (false, true) => false,
        _ => ids.len() == 1 || client_endpoints.contains(&(src, src_port)),
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

fn ultra_montage_object(entry: &UltraTimeStopEntry) -> Option<&str> {
    let (package, object) = entry.montage_asset.rsplit_once('.')?;
    (package.rsplit('/').next()? == object).then_some(object)
}

fn ultra_montage_character_id(package: &str) -> Option<u32> {
    let player_path = package.strip_prefix("/Game/Characters/Player/")?;
    let (number, _) = player_path.split_once('_')?;
    if number.len() != 3 {
        return None;
    }
    number.parse::<u32>().ok()?.checked_add(1000)
}

fn text_contains_ultra_montage(text: &str, char_id: u32, montage_object: &str) -> bool {
    text.lines().any(|line| {
        line.match_indices("/Game/Characters/Player/")
            .any(|(path_offset, _)| {
                let player_path = &line[path_offset..];
                if ultra_montage_character_id(player_path) != Some(char_id) {
                    return false;
                }
                player_path
                    .match_indices(montage_object)
                    .any(|(object_offset, _)| {
                        let bytes = player_path.as_bytes();
                        let prefix_is_boundary =
                            object_offset == 0 || matches!(bytes[object_offset - 1], b'/' | b'.');
                        let suffix = bytes.get(object_offset + montage_object.len());
                        prefix_is_boundary && matches!(suffix, None | Some(b'.' | b'_'))
                    })
            })
    })
}

fn ultra_montage_character_ids(
    decoded_text: &str,
    ultra_time_stops: &HashMap<u32, UltraTimeStopEntry>,
) -> Vec<u32> {
    if !decoded_text.contains("/Game/Characters/Player/") {
        return Vec::new();
    }
    let mut ids = Vec::new();
    for (char_id, entry) in ultra_time_stops {
        let Some(montage_object) = ultra_montage_object(entry) else {
            continue;
        };
        if text_contains_ultra_montage(decoded_text, *char_id, montage_object)
            && !ids.contains(char_id)
        {
            ids.push(*char_id);
        }
    }
    ids
}

fn ultra_activation_character_ids(
    decoded_text: &str,
    ultra_time_stops: &HashMap<u32, UltraTimeStopEntry>,
) -> Vec<u32> {
    let mut ids = Vec::new();
    for (char_id, entry) in ultra_time_stops {
        if entry
            .activation_cooldown_tags
            .iter()
            .any(|tag| tag != ULTRA_COOLDOWN_TAG && text_has_exact_marker(decoded_text, tag))
        {
            ids.push(*char_id);
        }
    }
    ids
}

fn decode_payload_text(data: &[u8]) -> String {
    decode_payload_text_filtered(data, |_| true).text
}

fn decode_summary_payload_text(
    data: &[u8],
    ultra_time_stops: &HashMap<u32, UltraTimeStopEntry>,
) -> DecodedPayloadText {
    decode_payload_text_filtered(data, |value| {
        value.contains("Abyss")
            || value.contains("ConditionState_Success")
            || value.contains("UltraSkill")
            || !ultra_montage_character_ids(value, ultra_time_stops).is_empty()
            || ultra_time_stops.values().any(|entry| {
                entry
                    .ignored_cooldown_tags
                    .iter()
                    .any(|tag| !tag.is_empty() && value.contains(tag))
                    || entry.extra_cooldowns.iter().any(|cooldown| {
                        !cooldown.cooldown_tag.is_empty() && value.contains(&cooldown.cooldown_tag)
                    })
            })
    })
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
    } else if !is_restart
        && decoded_text.contains("FAbyssGamePlayData")
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

fn fixed_ultra_time_stop_event(
    timestamp: f64,
    char_id: u32,
    ultra_time_stops: &HashMap<u32, UltraTimeStopEntry>,
    recent_ultra_time_stops: &mut HashMap<u32, f64>,
) -> Option<TimeStopEvent> {
    let entry = ultra_time_stops.get(&char_id)?;
    fixed_ultra_time_stop_event_with_duration(
        timestamp,
        char_id,
        &entry.ability_id,
        entry.end_ability_event_seconds,
        recent_ultra_time_stops,
    )
}

fn fixed_ultra_time_stop_event_with_duration(
    timestamp: f64,
    char_id: u32,
    ability_id: &str,
    duration: f64,
    recent_ultra_time_stops: &mut HashMap<u32, f64>,
) -> Option<TimeStopEvent> {
    if !duration.is_finite() || duration <= 0.0 {
        return None;
    }
    if recent_ultra_time_stops
        .get(&char_id)
        .is_some_and(|previous| timestamp - previous < duration.max(1.0))
    {
        return None;
    }
    recent_ultra_time_stops.insert(char_id, timestamp);
    recent_ultra_time_stops.retain(|_, previous| timestamp - *previous <= 30.0);
    Some(TimeStopEvent::UltraAnimation {
        timestamp,
        char_id,
        ability_id: ability_id.to_owned(),
        duration_seconds: duration,
    })
}

fn special_ultra_cooldown_events(
    timestamp: f64,
    decoded_text: &str,
    ultra_time_stops: &HashMap<u32, UltraTimeStopEntry>,
    recent_ultra_time_stops: &mut HashMap<u32, f64>,
) -> (Vec<TimeStopEvent>, bool) {
    let mut events = Vec::new();
    let mut handled_special_cooldown = false;
    for (char_id, entry) in ultra_time_stops {
        if entry
            .ignored_cooldown_tags
            .iter()
            .any(|tag| !tag.is_empty() && decoded_text.contains(tag))
        {
            handled_special_cooldown = true;
        }
        for cooldown in &entry.extra_cooldowns {
            if cooldown.cooldown_tag.is_empty() || !decoded_text.contains(&cooldown.cooldown_tag) {
                continue;
            }
            handled_special_cooldown = true;
            let ability_id = if cooldown.ability_id.is_empty() {
                entry.ability_id.as_str()
            } else {
                cooldown.ability_id.as_str()
            };
            if let Some(event) = fixed_ultra_time_stop_event_with_duration(
                timestamp,
                *char_id,
                ability_id,
                cooldown.duration_seconds,
                recent_ultra_time_stops,
            ) {
                events.push(event);
            }
        }
    }
    (events, handled_special_cooldown)
}

fn extra_time_stop_events_from_text(timestamp: f64, decoded_text: &str) -> Vec<TimeStopEvent> {
    let mut events = Vec::new();
    for line in decoded_text.lines() {
        if line.contains(JIN_ENTER_TIME_STOP_TAG) {
            events.push(TimeStopEvent::ExtraStart {
                timestamp,
                reason: JIN_EXTRA_TIME_STOP_REASON.to_owned(),
            });
        }
        if line.contains(JIN_CLEAR_TIME_STOP_TAG) {
            events.push(TimeStopEvent::ExtraEnd {
                timestamp,
                reason: JIN_EXTRA_TIME_STOP_REASON.to_owned(),
            });
        }
    }
    events
}

#[derive(Default)]
struct UltraTimeStopTracker {
    recent_emitted: HashMap<u32, f64>,
    pending_cooldowns: Vec<PendingUltraTimeStop>,
    recent_montages: Vec<RecentUltraMontage>,
    next_pending_id: u64,
}

struct PendingUltraTimeStop {
    timestamp: f64,
    flow: Option<UltraTimeStopFlowKey>,
    public_reason: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct UltraTimeStopFlowKey {
    source: (Ipv4Addr, u16),
    destination: (Ipv4Addr, u16),
}

struct RecentUltraMontage {
    timestamp: f64,
    char_id: u32,
    flow: Option<UltraTimeStopFlowKey>,
}

fn export_ultra_time_stop_flow_key(
    source: &str,
    destination: &str,
) -> Option<UltraTimeStopFlowKey> {
    fn endpoint(value: &str) -> Option<(Ipv4Addr, u16)> {
        let (ip, port) = value.rsplit_once(':')?;
        Some((ip.parse().ok()?, port.parse().ok()?))
    }

    Some(UltraTimeStopFlowKey {
        source: endpoint(source)?,
        destination: endpoint(destination)?,
    })
}

impl UltraTimeStopTracker {
    fn events_from_packet(
        &mut self,
        timestamp: f64,
        decoded_text: &str,
        declared_ids: &[u32],
        server_to_client: bool,
        flow: Option<UltraTimeStopFlowKey>,
        ultra_time_stops: &HashMap<u32, UltraTimeStopEntry>,
    ) -> Vec<TimeStopEvent> {
        let mut events = extra_time_stop_events_from_text(timestamp, decoded_text);
        self.resolve_finished_cooldowns(timestamp, decoded_text, ultra_time_stops, &mut events);
        self.observe_ultra_montages(
            timestamp,
            decoded_text,
            server_to_client,
            flow,
            ultra_time_stops,
        );
        let (mut special_events, handled_special_cooldown) = special_ultra_cooldown_events(
            timestamp,
            decoded_text,
            ultra_time_stops,
            &mut self.recent_emitted,
        );
        events.append(&mut special_events);

        if handled_special_cooldown || !server_to_client {
            return events;
        }

        let has_time_actor = text_has_exact_marker(decoded_text, ULTRA_TIME_ACTOR_TAG);
        let activation_char_ids = ultra_activation_character_ids(decoded_text, ultra_time_stops);
        let has_activation = text_has_exact_marker(decoded_text, ULTRA_COOLDOWN_TAG)
            || !activation_char_ids.is_empty();
        if has_time_actor && !has_activation {
            self.start_time_actor(timestamp, flow, &mut events);
            return events;
        }
        if !has_activation || is_shinku_rage_cooldown_snapshot(decoded_text) {
            return events;
        }

        let time_actor_reason = self.take_time_actor_reason(flow);
        let had_time_actor = time_actor_reason.is_some();
        if let Some(reason) = time_actor_reason {
            events.push(TimeStopEvent::ExtraEnd { timestamp, reason });
        }
        let montage_char_id = self.consume_recent_ultra_montage(flow);
        let char_id = match activation_char_ids.as_slice() {
            [activation_id] => Some(*activation_id),
            [] => match (montage_char_id, declared_ids) {
                (Some(montage_id), [declared_id]) if montage_id != *declared_id => None,
                (Some(montage_id), _) => Some(montage_id),
                (None, [declared_id]) if !has_time_actor || had_time_actor => Some(*declared_id),
                _ => None,
            },
            _ => None,
        };
        match char_id {
            Some(char_id) => {
                self.clear_internal_cooldowns(flow);
                if let Some(event) = fixed_ultra_time_stop_event(
                    timestamp,
                    char_id,
                    ultra_time_stops,
                    &mut self.recent_emitted,
                ) {
                    events.push(event);
                }
            }
            None => {
                if !has_time_actor || had_time_actor {
                    self.queue_internal_cooldown(timestamp, flow);
                }
            }
        }
        events
    }

    fn start_time_actor(
        &mut self,
        timestamp: f64,
        flow: Option<UltraTimeStopFlowKey>,
        events: &mut Vec<TimeStopEvent>,
    ) {
        if self
            .pending_cooldowns
            .iter()
            .any(|pending| pending.flow == flow && pending.public_reason.is_some())
        {
            return;
        }
        self.clear_internal_cooldowns(flow);
        let reason = format!("{PENDING_ULTRA_TIME_STOP_REASON}.{}", self.next_pending_id);
        self.next_pending_id += 1;
        self.pending_cooldowns.push(PendingUltraTimeStop {
            timestamp,
            flow,
            public_reason: Some(reason.clone()),
        });
        events.push(TimeStopEvent::ExtraStart { timestamp, reason });
    }

    fn take_time_actor_reason(&mut self, flow: Option<UltraTimeStopFlowKey>) -> Option<String> {
        let index = self
            .pending_cooldowns
            .iter()
            .rposition(|pending| pending.flow == flow && pending.public_reason.is_some())?;
        self.pending_cooldowns.swap_remove(index).public_reason
    }

    fn queue_internal_cooldown(&mut self, timestamp: f64, flow: Option<UltraTimeStopFlowKey>) {
        if self.pending_cooldowns.iter().any(|pending| {
            pending.flow == flow
                && pending.public_reason.is_none()
                && timestamp - pending.timestamp <= ULTRA_INTERNAL_COOLDOWN_DUPLICATE_WINDOW_SECONDS
        }) {
            return;
        }
        self.pending_cooldowns.push(PendingUltraTimeStop {
            timestamp,
            flow,
            public_reason: None,
        });
    }

    fn clear_internal_cooldowns(&mut self, flow: Option<UltraTimeStopFlowKey>) {
        self.pending_cooldowns
            .retain(|pending| pending.flow != flow || pending.public_reason.is_some());
    }

    fn observe_ultra_montages(
        &mut self,
        timestamp: f64,
        decoded_text: &str,
        server_to_client: bool,
        flow: Option<UltraTimeStopFlowKey>,
        ultra_time_stops: &HashMap<u32, UltraTimeStopEntry>,
    ) {
        self.recent_montages.retain(|candidate| {
            let age = timestamp - candidate.timestamp;
            (0.0..=ULTRA_MONTAGE_ASSOCIATION_WINDOW_SECONDS).contains(&age)
        });
        if !server_to_client {
            return;
        }
        for char_id in ultra_montage_character_ids(decoded_text, ultra_time_stops) {
            if !self
                .recent_montages
                .iter()
                .any(|candidate| candidate.char_id == char_id && candidate.flow == flow)
            {
                self.recent_montages.push(RecentUltraMontage {
                    timestamp,
                    char_id,
                    flow,
                });
            }
        }
    }

    fn consume_recent_ultra_montage(&mut self, flow: Option<UltraTimeStopFlowKey>) -> Option<u32> {
        let mut char_id = None;
        let mut ambiguous = false;
        for candidate in self
            .recent_montages
            .iter()
            .filter(|candidate| candidate.flow == flow)
        {
            match char_id {
                None => char_id = Some(candidate.char_id),
                Some(existing) if existing == candidate.char_id => {}
                Some(_) => ambiguous = true,
            }
        }
        self.recent_montages
            .retain(|candidate| candidate.flow != flow);
        if ambiguous { None } else { char_id }
    }

    fn events_from_hits(
        &mut self,
        hits: &[Hit],
        ultra_time_stops: &HashMap<u32, UltraTimeStopEntry>,
    ) -> Vec<TimeStopEvent> {
        let mut events = Vec::new();
        for hit in hits {
            self.resolve_cooldown_from_hit(hit, ultra_time_stops, &mut events);
        }
        events
    }

    fn resolve_cooldown_from_hit(
        &mut self,
        hit: &Hit,
        ultra_time_stops: &HashMap<u32, UltraTimeStopEntry>,
        events: &mut Vec<TimeStopEvent>,
    ) {
        let Some(char_id) = ultra_damage_time_stop_char_id(hit, ultra_time_stops) else {
            return;
        };
        let Some((index, _)) = self
            .pending_cooldowns
            .iter()
            .enumerate()
            .filter(|(_, pending)| {
                let window = if pending.public_reason.is_some() {
                    ULTRA_TIME_ACTOR_PENDING_WINDOW_SECONDS
                } else {
                    ULTRA_TIME_STOP_PENDING_WINDOW_SECONDS
                };
                hit.timestamp >= pending.timestamp && hit.timestamp - pending.timestamp <= window
            })
            .max_by(|(_, left), (_, right)| left.timestamp.total_cmp(&right.timestamp))
        else {
            return;
        };
        let pending = self.pending_cooldowns.swap_remove(index);
        let activation_timestamp = if let Some(reason) = pending.public_reason {
            events.push(TimeStopEvent::ExtraEnd {
                timestamp: hit.timestamp,
                reason,
            });
            hit.timestamp
        } else {
            pending.timestamp
        };
        if let Some(event) = fixed_ultra_time_stop_event(
            activation_timestamp,
            char_id,
            ultra_time_stops,
            &mut self.recent_emitted,
        ) {
            events.push(event);
        }
    }

    fn resolve_finished_cooldowns(
        &mut self,
        timestamp: f64,
        _decoded_text: &str,
        _ultra_time_stops: &HashMap<u32, UltraTimeStopEntry>,
        events: &mut Vec<TimeStopEvent>,
    ) {
        let mut index = 0;
        while index < self.pending_cooldowns.len() {
            let pending = &self.pending_cooldowns[index];
            let window = if pending.public_reason.is_some() {
                ULTRA_TIME_ACTOR_PENDING_WINDOW_SECONDS
            } else {
                ULTRA_TIME_STOP_PENDING_WINDOW_SECONDS
            };
            let expires_at = pending.timestamp + window;
            let expired = timestamp > expires_at;
            if !expired {
                index += 1;
                continue;
            }
            let pending = self.pending_cooldowns.swap_remove(index);
            if let Some(reason) = pending.public_reason {
                events.push(TimeStopEvent::ExtraEnd {
                    timestamp: expires_at,
                    reason,
                });
            }
        }
    }
}

fn text_has_exact_marker(decoded_text: &str, marker: &str) -> bool {
    decoded_text.lines().any(|line| line.trim() == marker)
}

fn is_shinku_rage_cooldown_snapshot(decoded_text: &str) -> bool {
    text_has_exact_marker(decoded_text, SHINKU_RAGE_ABILITY_TAG)
        || text_has_exact_marker(decoded_text, SHINKU_RAGE_GAMEPLAY_CUE_TAG)
}

fn ultra_damage_time_stop_char_id(
    hit: &Hit,
    ultra_time_stops: &HashMap<u32, UltraTimeStopEntry>,
) -> Option<u32> {
    if hit.direction == "incoming" {
        return None;
    }
    let ability_name = hit.ability_name.as_deref()?;
    let entry = ultra_time_stops.get(&hit.char_id)?;
    (ability_name == entry.ability_id && ability_name.contains("UltraSkill")).then_some(hit.char_id)
}

fn send_packet_events(sender: &Sender<EngineEvent>, packet: PacketDebug) {
    for event in abyss_events_from_text(packet.timestamp, &packet.decoded_text) {
        let _ = sender.send(EngineEvent::Abyss(event));
    }
    let _ = sender.send(EngineEvent::Packet(Box::new(packet)));
}

const MAX_PENDING_FOLLOW_UP_HITS: usize = 256;
const AMBIGUOUS_HIT_CONFIRMATION_WINDOW_SECONDS: f64 = 0.5;
/// Npcap or a capture driver can report the same outbound frame twice with only
/// tens of microseconds between copies. Keep the window short so a later, real
/// transmission of identical application data is not suppressed.
const DUPLICATE_FRAME_WINDOW_SECONDS: f64 = 0.001;
/// Bounds memory for external captures containing many frames with one timestamp.
const MAX_RECENT_CAPTURE_FRAMES: usize = 512;
const FUWEN_START_SIGNATURE_SHIFT: u8 = 3;
const FUWEN_START_SIGNATURE_OFFSET: usize = 22;
const FUWEN_START_SIGNATURE: &[u8] = &[1, 0, 0, 0, 2, 0, 0, 0];
const FUWEN_ENTERING_ID_SHIFT: u8 = 0;
const FUWEN_ENTERING_ID_OFFSET: usize = 53;
const FUWEN_PREVIOUS_ID_SHIFT: u8 = 2;
const FUWEN_PREVIOUS_ID_OFFSET: usize = 66;
const MIN_FOLLOW_UP_RESIDUAL_DAMAGE: f64 = 1.0;
/// How far back to look for the real owner of an attribute-locked reaction whose
/// damage packet carried no caster. Matches the 3s window used by the 环合 retag.
const REACTION_REATTRIBUTION_WINDOW_SECONDS: f64 = 3.0;
const RECENT_CONFIRMED_HIT_WINDOW_SECONDS: f64 = 0.75;
const UNTYPED_SHADOW_HIT_WINDOW_SECONDS: f64 = 0.05;
const BOSS_HP_SYNC_WINDOW_SECONDS: f64 = 1.0;
const SERVER_DAMAGE_CALIBRATION_WINDOW_SECONDS: f64 = 1.0;
const JIN_EXTRA_TIME_STOP_REASON: &str = "Event.Montage.Player.UltraSkill.Jin";
const JIN_ENTER_TIME_STOP_TAG: &str = "Event.Montage.Player.UltraSkill.Jin.EnterTimeStop";
const JIN_CLEAR_TIME_STOP_TAG: &str = "Event.Montage.Player.UltraSkill.Jin.ClearTimeStop";
const ULTRA_COOLDOWN_TAG: &str = "CoolDown.Player.UltraSkill.F";
const ULTRA_TIME_ACTOR_TAG: &str = "CoolDown.Player.UltraSkill.TimeActor";
const PENDING_ULTRA_TIME_STOP_REASON: &str = "CoolDown.Player.UltraSkill.Pending";
const ULTRA_TIME_STOP_PENDING_WINDOW_SECONDS: f64 = 4.5;
const ULTRA_TIME_ACTOR_PENDING_WINDOW_SECONDS: f64 = 2.5;
const ULTRA_INTERNAL_COOLDOWN_DUPLICATE_WINDOW_SECONDS: f64 = 0.1;
const ULTRA_MONTAGE_ASSOCIATION_WINDOW_SECONDS: f64 = 0.02;
const SHINKU_RAGE_ABILITY_TAG: &str = "Ability.Player.Shinku.Rage";
const SHINKU_RAGE_GAMEPLAY_CUE_TAG: &str = "GameplayCue.Display.Shinku.Rage";

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
        if hit.direction == "incoming"
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
        if hit.direction == "incoming" || hit.attack_type.as_deref() != Some("环合·覆纹") {
            return;
        }
        self.fuwen_active = true;
        self.fuwen_start_pending = false;
        self.fuwen_recorded_damage = false;
        self.pending_hits.clear();
        self.last_server_hp = None;
    }

    fn observe_server_hp(&mut self, timestamp: f64, current_hp: f64) -> Option<HitFollowUp> {
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
        if !self.fuwen_active {
            return None;
        }
        if residual_damage < MIN_FOLLOW_UP_RESIDUAL_DAMAGE {
            return None;
        }
        self.fuwen_recorded_damage = true;
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
    }
}

#[derive(Clone)]
struct ServerDamagePendingHit {
    hit: Hit,
}

#[derive(Clone, Copy)]
struct ServerHpSnapshot {
    timestamp: f64,
    hp: f64,
}

#[derive(Default)]
struct ServerDamageCalibrationTracker {
    hp_by_handle: HashMap<[u8; 16], ServerHpSnapshot>,
    pending_hits: VecDeque<ServerDamagePendingHit>,
}

impl ServerDamageCalibrationTracker {
    fn observe_hit(&mut self, hit: &Hit) {
        if hit.direction == "incoming"
            || hit.char_id == 0
            || hit.target_max_hp <= 0.0
            || hit.target_hp_before <= 0.0
        {
            return;
        }
        if self
            .pending_hits
            .back()
            .is_some_and(|pending| hit.timestamp - pending.hit.timestamp > 2.0)
        {
            self.pending_hits.clear();
        }
        self.pending_hits
            .push_back(ServerDamagePendingHit { hit: hit.clone() });
        while self.pending_hits.len() > MAX_PENDING_FOLLOW_UP_HITS {
            self.pending_hits.pop_front();
        }
    }

    fn observe_boss_hp(
        &mut self,
        timestamp: f64,
        update: &crate::engine::parser::ParsedBossHpUpdate,
    ) -> Option<HitDamageCorrection> {
        let current_hp = if update.current_hp <= 1.0 {
            0.0
        } else {
            update.current_hp as f64
        };
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
        let previous = previous?;
        if current_hp >= previous.hp {
            self.pending_hits
                .retain(|pending| pending.hit.timestamp > timestamp);
            return None;
        }
        let candidates = self
            .pending_hits
            .iter()
            .enumerate()
            .filter(|(_, pending)| {
                pending.hit.timestamp > previous.timestamp
                    && pending.hit.timestamp <= timestamp
                    && pending.hit.target_max_hp > 0.0
            })
            .map(|(index, _)| index)
            .collect::<Vec<_>>();
        if candidates.len() != 1 {
            return None;
        }
        let source_index = candidates[0];
        let source = self.pending_hits[source_index].hit.clone();
        self.pending_hits
            .retain(|pending| pending.hit.timestamp > source.timestamp);
        let damage = previous.hp - current_hp;
        if damage < MIN_FOLLOW_UP_RESIDUAL_DAMAGE {
            return None;
        }
        Some(HitDamageCorrection {
            source_timestamp: source.timestamp,
            source_char_id: source.char_id,
            source_damage: source.damage,
            source_target_hp_before: source.target_hp_before,
            source_target_hp_after: source.target_hp_after,
            source_target_max_hp: source.target_max_hp,
            source_gameplay_effect_index: source.gameplay_effect_index,
            damage,
            target_hp_before: previous.hp,
            target_hp_after: current_hp,
            target_hp_percent: if source.target_max_hp > 0.0 {
                current_hp / source.target_max_hp * 100.0
            } else {
                0.0
            },
        })
    }
}

const MAX_INVENTORY_CONNECTIONS: usize = 16;
const MAX_INVENTORY_FRAGMENTS_PER_CONNECTION: usize = 4096;
const MAX_INVENTORY_STREAM_BITS: usize = 16 * 1024 * 1024;
const MAX_INVENTORY_ITEMS: usize = 4096;

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

#[derive(Default)]
struct InventoryConnectionState {
    known_channels: HashSet<u16>,
    fragments: HashMap<(u16, u16), SingleBunch>,
    fragment_order: VecDeque<(u16, u16)>,
    character_ids: HashMap<HtItemNetId, u32>,
}

impl InventoryConnectionState {
    fn push_bunches(&mut self, bunches: Vec<SingleBunch>) -> Vec<InventoryBitPayload> {
        let mut added = false;
        for bunch in bunches {
            self.known_channels.insert(bunch.prefix);
            let key = (bunch.prefix, bunch.sequence);
            if let Some(existing) = self.fragments.get(&key) {
                if existing == &bunch {
                    continue;
                }
                self.fragment_order.retain(|stored| *stored != key);
            }
            while self.fragments.len() >= MAX_INVENTORY_FRAGMENTS_PER_CONNECTION {
                let Some(oldest) = self.fragment_order.pop_front() else {
                    break;
                };
                self.fragments.remove(&oldest);
            }
            self.fragment_order.push_back(key);
            self.fragments.insert(key, bunch);
            added = true;
        }
        if !added {
            return Vec::new();
        }
        self.take_completed_streams()
    }

    fn take_completed_streams(&mut self) -> Vec<InventoryBitPayload> {
        let mut starts = self
            .fragments
            .iter()
            .filter_map(|(key, bunch)| matches!(bunch.partial_flags, 0x09 | 0x0d).then_some(*key))
            .collect::<Vec<_>>();
        starts.sort_unstable();

        let mut completed = Vec::new();
        let mut consumed = HashSet::new();
        for start @ (channel, initial_sequence) in starts {
            let Some(initial) = self.fragments.get(&start) else {
                continue;
            };
            if initial.partial_flags == 0x0d {
                completed.push(InventoryBitPayload {
                    data: initial.data.clone(),
                    bit_len: initial.data_bit_len,
                });
                consumed.insert(start);
                continue;
            }

            let mut data = Vec::new();
            let mut bit_len = 0;
            let mut sequence = initial_sequence;
            let mut is_complete = false;
            let mut chain_keys = Vec::new();
            for index in 0..1024 {
                let key = (channel, sequence);
                let Some(fragment) = self.fragments.get(&key) else {
                    break;
                };
                let valid_flag = if index == 0 {
                    fragment.partial_flags == 0x09
                } else {
                    matches!(fragment.partial_flags, 0x08 | 0x0c)
                };
                if !valid_flag
                    || append_inventory_bits(
                        &mut data,
                        &mut bit_len,
                        &fragment.data,
                        fragment.data_bit_len,
                    )
                    .is_none()
                {
                    break;
                }
                chain_keys.push(key);
                if fragment.partial_flags == 0x0c {
                    is_complete = true;
                    break;
                }
                sequence = (sequence + 1) & 0x03ff;
            }
            if is_complete {
                completed.push(InventoryBitPayload { data, bit_len });
                consumed.extend(chain_keys);
            }
        }
        for key in &consumed {
            self.fragments.remove(key);
        }
        self.fragment_order.retain(|key| !consumed.contains(key));
        completed
    }
}

fn append_inventory_bits(
    destination: &mut Vec<u8>,
    destination_bit_len: &mut usize,
    source: &[u8],
    source_bit_len: usize,
) -> Option<()> {
    if source_bit_len > source.len().checked_mul(8)? {
        return None;
    }
    let new_bit_len = destination_bit_len.checked_add(source_bit_len)?;
    if new_bit_len > MAX_INVENTORY_STREAM_BITS {
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

    fn process_packet(
        &mut self,
        connection: InventoryConnectionKey,
        packet: &SequencedPacket,
    ) -> InventoryPacketResult {
        if !self.connections.contains_key(&connection) {
            while self.connections.len() >= MAX_INVENTORY_CONNECTIONS {
                let Some(oldest) = self.connection_order.pop_front() else {
                    break;
                };
                self.connections.remove(&oldest);
            }
            self.connection_order.push_back(connection.clone());
            self.connections
                .insert(connection.clone(), InventoryConnectionState::default());
        }

        let streams = {
            let state = self
                .connections
                .get_mut(&connection)
                .expect("new or existing inventory connection must be present");
            let known_channels = state.known_channels.iter().copied().collect::<Vec<_>>();
            let bunches = parse_inventory_bunches(packet, &known_channels);
            if bunches.is_empty() {
                return InventoryPacketResult::default();
            }
            state.push_bunches(bunches)
        };

        let mut items_changed = false;
        let mut characters_changed = false;
        for stream in streams {
            let character_ids = parse_empty_curtain_character_owners(&stream.data, stream.bit_len);
            let parsed = parse_empty_curtain_items(&stream.data, stream.bit_len, &self.catalog);
            if character_ids.is_empty() && parsed.is_empty() {
                continue;
            }
            let character_mapping_changed = {
                let state = self
                    .connections
                    .get_mut(&connection)
                    .expect("inventory connection must remain present while processing streams");
                let mut changed = false;
                for (net_id, character_id) in character_ids {
                    if state.character_ids.insert(net_id, character_id) != Some(character_id) {
                        changed = true;
                    }
                }
                changed
            };
            if !parsed.is_empty() && self.active_connection.as_ref() != Some(&connection) {
                self.active_connection = Some(connection.clone());
                items_changed |= !self.items.is_empty();
                self.items.clear();
            }
            if self.active_connection.as_ref() != Some(&connection) {
                continue;
            }
            let character_ids = &self
                .connections
                .get(&connection)
                .expect("active inventory connection must remain present")
                .character_ids;
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
            for mut item in parsed {
                item.equipped_character_id = item
                    .character_net_id
                    .and_then(|net_id| character_ids.get(&net_id).copied());
                if self.items.get(&item.id) != Some(&item) {
                    if !self.items.contains_key(&item.id) && self.items.len() >= MAX_INVENTORY_ITEMS
                    {
                        continue;
                    }
                    self.items.insert(item.id, item);
                    items_changed = true;
                }
            }
        }
        let snapshot = items_changed.then(|| {
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
            recognized: true,
            snapshot,
            characters,
        }
    }
}

struct RecentCaptureFrame {
    timestamp: f64,
    bytes: Vec<u8>,
}

#[derive(Clone, Copy)]
enum FrameTimestamp {
    Known(f64),
    Unknown,
}

/// Suppresses byte-for-byte duplicate Ethernet frames reported back-to-back by
/// the capture layer. Full-frame comparison keeps a genuine retransmission with
/// different network headers distinct, even when its UDP payload is unchanged.
#[derive(Default)]
struct FrameDedup {
    recent: VecDeque<RecentCaptureFrame>,
    last_timestamp: Option<f64>,
}

impl FrameDedup {
    fn is_duplicate(&mut self, frame: &[u8], timestamp: Option<f64>) -> bool {
        let Some(timestamp) = timestamp.filter(|timestamp| timestamp.is_finite()) else {
            self.recent.clear();
            self.last_timestamp = None;
            return false;
        };
        if self
            .last_timestamp
            .is_some_and(|previous| timestamp < previous)
        {
            self.recent.clear();
        }
        self.last_timestamp = Some(timestamp);

        while let Some(entry) = self.recent.front() {
            if timestamp - entry.timestamp <= DUPLICATE_FRAME_WINDOW_SECONDS {
                break;
            }
            self.recent.pop_front();
        }
        if self.recent.iter().any(|entry| entry.bytes == frame) {
            return true;
        }
        if self.recent.len() == MAX_RECENT_CAPTURE_FRAMES {
            self.recent.pop_front();
        }
        self.recent.push_back(RecentCaptureFrame {
            timestamp,
            bytes: frame.to_vec(),
        });
        false
    }
}

struct PacketDecoder {
    packet_emission: PacketEmissionMode,
    session_characters: HashMap<(Ipv4Addr, u16, Ipv4Addr, u16), u32>,
    client_endpoints: HashSet<(Ipv4Addr, u16)>,
    gameplay_effect_names: HashMap<u32, String>,
    gameplay_effect_skills: HashMap<String, GameplayEffectSkill>,
    ultra_time_stops: HashMap<u32, UltraTimeStopEntry>,
    ability_tip_names: HashMap<String, String>,
    follow_up_damage: FollowUpDamageTracker,
    server_damage_calibration: ServerDamageCalibrationTracker,
    use_server_damage_calibration: bool,
    character_declarations: HashMap<u32, f64>,
    ultra_time_stop: UltraTimeStopTracker,
    pending_ambiguous_hits: Vec<Hit>,
    recent_confirmed_hits: Vec<Hit>,
    empty_curtain: EmptyCurtainDecoder,
    frame_dedup: FrameDedup,
    resource_warnings: Vec<String>,
}

#[derive(Default)]
struct PreparedHits {
    emit: Vec<Hit>,
    filtered_incoming: usize,
    deferred_ambiguous: usize,
    suppressed_ambiguous: usize,
}

impl Default for PacketDecoder {
    fn default() -> Self {
        let mut resource_warnings = Vec::new();
        let gameplay_effect_names = load_resource(
            GAMEPLAY_EFFECT_MAPPING_PATH,
            &mut resource_warnings,
            load_gameplay_effect_mapping,
        );
        let gameplay_effect_skills = load_resource(
            SKILL_DAMAGE_DATA_PATH,
            &mut resource_warnings,
            load_gameplay_effect_skills,
        );
        let ultra_time_stops = load_resource(
            ULTRA_TIME_STOP_DATA_PATH,
            &mut resource_warnings,
            load_ultra_time_stops,
        );
        let ui_language = i18n::current_language();
        let ability_tip_names = load_resource(ABILITY_TIPS_PATH, &mut resource_warnings, |path| {
            load_ability_tip_names(path, ui_language)
        });
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
            gameplay_effect_skills,
            ultra_time_stops,
            ability_tip_names,
            follow_up_damage: FollowUpDamageTracker::default(),
            server_damage_calibration: ServerDamageCalibrationTracker::default(),
            use_server_damage_calibration: false,
            character_declarations: HashMap::new(),
            ultra_time_stop: UltraTimeStopTracker::default(),
            pending_ambiguous_hits: Vec::new(),
            recent_confirmed_hits: Vec::new(),
            empty_curtain: EmptyCurtainDecoder::new(equipment_catalog),
            frame_dedup: FrameDedup::default(),
            resource_warnings,
        }
    }
}

impl PacketDecoder {
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
    fn resource_warning(&self) -> Option<String> {
        (!self.resource_warnings.is_empty()).then(|| self.resource_warnings.join("; "))
    }

    fn take_expired_ambiguous_hits(&mut self, timestamp: f64) -> Vec<Hit> {
        let mut expired = Vec::new();
        let mut pending = Vec::with_capacity(self.pending_ambiguous_hits.len());
        for hit in self.pending_ambiguous_hits.drain(..) {
            if timestamp - hit.timestamp > AMBIGUOUS_HIT_CONFIRMATION_WINDOW_SECONDS {
                expired.push(hit);
            } else {
                pending.push(hit);
            }
        }
        self.pending_ambiguous_hits = pending;
        expired
    }

    fn take_all_ambiguous_hits(&mut self) -> Vec<Hit> {
        self.pending_ambiguous_hits.drain(..).collect()
    }

    fn emit_hits(
        &mut self,
        hits: impl IntoIterator<Item = Hit>,
        characters: &HashMap<u32, CharacterInfo>,
        sender: &Sender<EngineEvent>,
    ) {
        for hit in hits {
            self.follow_up_damage
                .observe_hit(&hit, hit.gameplay_effect_index, characters);
            if self.use_server_damage_calibration {
                self.server_damage_calibration.observe_hit(&hit);
            }
            let _ = sender.send(EngineEvent::Hit(Box::new(hit)));
        }
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
            if !include_incoming && hit.direction == "incoming" {
                prepared.filtered_incoming += 1;
                continue;
            }
            if gameplay_effect_confirms_session_hit(&hit, declared_ids, characters) {
                hit.direction = "outgoing".to_owned();
                hit.char_source = "gameplay_effect".to_owned();
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
            prepared.suppressed_ambiguous += self.suppress_matching_ambiguous_hits(&hit);
            if is_confirmed_packet_hit(&hit) {
                self.recent_confirmed_hits.push(hit.clone());
            }
            prepared.emit.push(hit);
        }
        prepared
    }

    fn infer_boss_hp_sync_damage(
        &mut self,
        timestamp: f64,
        current_hp: f64,
        characters: &HashMap<u32, CharacterInfo>,
    ) -> Option<HitFollowUp> {
        self.recent_confirmed_hits
            .retain(|hit| timestamp - hit.timestamp <= BOSS_HP_SYNC_WINDOW_SECONDS);
        if current_hp > 1.0 {
            return None;
        }
        let current_hp = 0.0;
        if self
            .recent_confirmed_hits
            .iter()
            .any(|hit| nearly_same(hit.target_hp_after, current_hp))
        {
            return None;
        }
        let source_index = self.recent_confirmed_hits.iter().rev().position(|hit| {
            hit.direction != "incoming"
                && hit.target_max_hp > 0.0
                && hit.target_hp_after - current_hp >= MIN_FOLLOW_UP_RESIDUAL_DAMAGE
        })?;
        let source_index = self.recent_confirmed_hits.len() - 1 - source_index;
        let source = self.recent_confirmed_hits[source_index].clone();
        self.recent_confirmed_hits[source_index].target_hp_after = current_hp;
        self.recent_confirmed_hits[source_index].target_hp_percent = if source.target_max_hp > 0.0 {
            current_hp / source.target_max_hp * 100.0
        } else {
            0.0
        };
        let damage = source.target_hp_after - current_hp;
        let damage_attribute = source.damage_attribute.clone().or_else(|| {
            characters
                .get(&source.char_id)
                .and_then(|character| character.attribute.clone())
        });
        Some(HitFollowUp {
            source_timestamp: source.timestamp,
            source_char_id: source.char_id,
            source_damage: source.damage,
            source_target_hp_before: source.target_hp_before,
            source_target_hp_after: source.target_hp_after,
            source_target_max_hp: source.target_max_hp,
            source_gameplay_effect_index: source.gameplay_effect_index,
            timestamp,
            damage,
            target_hp_after: current_hp,
            target_hp_percent: if source.target_max_hp > 0.0 {
                current_hp / source.target_max_hp * 100.0
            } else {
                0.0
            },
            damage_name: Some("HP同步伤害".to_owned()),
            attack_type: Some("HP同步伤害".to_owned()),
            damage_attribute,
        })
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
    /// most one of these mechanisms, with the reaction follow-up given first
    /// refusal — but the calibration tracker still gets to *observe* every
    /// update regardless, since it keeps its own HP snapshot/pending-hit state
    /// (`ServerDamageCalibrationTracker::hp_by_handle`); skipping the call
    /// entirely on a claimed update would leave that state stale and make its
    /// *next* correction compare against the wrong baseline.
    fn reconcile_boss_hp_updates(
        &mut self,
        timestamp: f64,
        boss_hp_updates: &[crate::engine::parser::ParsedBossHpUpdate],
        characters: &HashMap<u32, CharacterInfo>,
    ) -> (Vec<HitFollowUp>, Vec<HitFollowUp>, Vec<HitDamageCorrection>) {
        let mut inferred_follow_ups = Vec::new();
        let mut hp_sync_follow_ups = Vec::new();
        let mut server_damage_corrections = Vec::new();
        for update in boss_hp_updates {
            let follow_up = self
                .follow_up_damage
                .observe_server_hp(timestamp, update.current_hp as f64);
            let claimed = follow_up.is_some();
            inferred_follow_ups.extend(follow_up);
            if self.use_server_damage_calibration {
                let correction = self
                    .server_damage_calibration
                    .observe_boss_hp(timestamp, update);
                if !claimed {
                    server_damage_corrections.extend(correction);
                }
            } else if !claimed
                && let Some(follow_up) =
                    self.infer_boss_hp_sync_damage(timestamp, update.current_hp as f64, characters)
            {
                hp_sync_follow_ups.push(follow_up);
            }
        }
        (
            inferred_follow_ups,
            hp_sync_follow_ups,
            server_damage_corrections,
        )
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
        && hit.char_source == "session"
        && hit.direction == "unknown"
        && hit.gameplay_effect_index.is_some()
}

fn gameplay_effect_confirms_session_hit(
    hit: &Hit,
    declared_ids: &[u32],
    characters: &HashMap<u32, CharacterInfo>,
) -> bool {
    if declared_ids.len() <= 1
        || hit.char_source != "session"
        || hit.direction != "unknown"
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
    matches!(hit.char_source.as_str(), "packet" | "gameplay_effect") && hit.direction == "outgoing"
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
        hit.gameplay_effect_index.is_none()
            && confirmed.gameplay_effect_index.is_some()
            && timestamp_delta <= UNTYPED_SHADOW_HIT_WINDOW_SECONDS
            && hit.char_id == confirmed.char_id
            && nearly_same(hit.damage, confirmed.damage)
    })
}

fn nearly_same(left: f64, right: f64) -> bool {
    (left - right).abs() <= 0.5
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
) -> Option<&'a ParsedGameplayEffect> {
    let mut aligned = effects
        .iter()
        .filter(|effect| effect.bit_shift == hit.bit_shift);
    let first = aligned.next();
    if first.is_some() && aligned.next().is_none() {
        first
    } else if effects.len() == 1 {
        effects.first()
    } else {
        None
    }
}

fn enrich_hit_with_gameplay_effect(
    hit: &mut Hit,
    effects: &[ParsedGameplayEffect],
    names: &HashMap<u32, String>,
    skills: &HashMap<String, GameplayEffectSkill>,
    ability_tip_names: &HashMap<String, String>,
) {
    let Some(effect) = matching_gameplay_effect(hit, effects) else {
        return;
    };
    hit.gameplay_effect_index = Some(effect.unique_index);
    let Some(effect_name) = names.get(&effect.unique_index) else {
        return;
    };
    hit.gameplay_effect_name = Some(effect_name.clone());
    hit.damage_name = resolve_damage_name(effect_name, skills, ability_tip_names);
    let skill = skills.get(effect_name);
    if let Some(skill) = skill {
        hit.ability_name = skill.ability_name.clone();
        hit.attack_type = Some(skill.attack_type.clone());
    } else if let Some(attack_type) = hit
        .damage_name
        .as_deref()
        .and_then(classify_attack_type_from_description)
    {
        hit.attack_type = Some(attack_type);
    } else {
        hit.attack_type = Some(classify_attack_type(None, effect_name, None));
    }
    if is_known_incoming_damage_effect(effect_name) {
        hit.direction = "incoming".to_owned();
    }
    if is_known_outgoing_damage_effect(effect_name, skill) {
        hit.direction = "outgoing".to_owned();
    }
    if is_vehicle_physical_damage_effect(effect_name) {
        hit.direction = "outgoing".to_owned();
        hit.damage_attribute = Some("物理".to_owned());
        hit.attack_type = Some("载具伤害".to_owned());
    }
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
    if hit.direction == "incoming" {
        return;
    }
    if hit.char_source == "packet" && !can_override_packet_id {
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
    hit.char_source = "gameplay_effect".to_owned();
}

fn is_known_outgoing_damage_effect(effect_name: &str, skill: Option<&GameplayEffectSkill>) -> bool {
    let effect_name_lower = effect_name.to_ascii_lowercase();
    if effect_name_lower.starts_with("ge_mon_") {
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
    effect_name_lower.starts_with("ge_mon_")
        && !effect_name_lower.contains("steal")
        && (effect_name_lower.contains("damage") || effect_name_lower.contains("_dmg"))
}

fn is_vehicle_physical_damage_effect(effect_name: &str) -> bool {
    effect_name.starts_with("GE_Vehicle_HitOut")
        || effect_name.starts_with("GE_VehicleCombatDamage")
        || effect_name == "GE_Player_VehicleExplode_HitOut"
}

fn resolve_damage_name(
    effect_name: &str,
    skills: &HashMap<String, GameplayEffectSkill>,
    ability_tip_names: &HashMap<String, String>,
) -> Option<String> {
    let ability_name = skills.get(effect_name)?.ability_name.as_deref()?;
    ability_tip_names.get(ability_name).cloned()
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
    fn process_ethernet_frame(
        &mut self,
        packet: &[u8],
        frame_timestamp: FrameTimestamp,
        local_ip: Option<Ipv4Addr>,
        include_incoming: bool,
        characters: &HashMap<u32, CharacterInfo>,
        sender: &Sender<EngineEvent>,
    ) {
        let (timestamp, capture_timestamp) = match frame_timestamp {
            FrameTimestamp::Known(timestamp) => (timestamp, Some(timestamp)),
            FrameTimestamp::Unknown => (0.0, None),
        };
        let Some((src, src_port, dst, dst_port, payload)) = parse_udp_ipv4(packet) else {
            return;
        };
        if local_ip.is_some_and(|ip| src != ip && dst != ip) {
            return;
        }
        // Drop a frame already reported by the capture layer so its damage
        // records are not counted a second time. See [`FrameDedup`].
        if self.frame_dedup.is_duplicate(packet, capture_timestamp) {
            return;
        }
        let expired_hits = self.take_expired_ambiguous_hits(timestamp);
        self.emit_hits(expired_hits, characters, sender);

        let decoded_payload = match self.packet_emission {
            PacketEmissionMode::FullDebug => decode_payload_text_filtered(payload, |_| true),
            PacketEmissionMode::SummaryOnly => {
                decode_summary_payload_text(payload, &self.ultra_time_stops)
            }
        };
        let decoded_text = decoded_payload.text;
        let evidence = find_declared_character_evidence(payload);
        let final_tower_evidence = find_final_tower_character_evidence(payload);
        let character_evidence = merged_character_evidence(&evidence, &final_tower_evidence);
        let ids = character_ids_from_evidence_sources(&evidence, &final_tower_evidence);
        self.follow_up_damage
            .observe_characters(ids.iter().copied(), characters);
        let outgoing = infer_outgoing(src, src_port, dst, local_ip, &ids, &self.client_endpoints);
        if outgoing && !ids.is_empty() {
            self.client_endpoints.insert((src, src_port));
        }
        let direction = if outgoing { "C2S" } else { "S2C" };
        let ultra_time_stop_flow = Some(UltraTimeStopFlowKey {
            source: (src, src_port),
            destination: (dst, dst_port),
        });
        let gameplay_effects = parse_gameplay_effects(payload);
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
                payload,
                timestamp,
                packet_char_id,
                fallback,
                characters,
                &evidence,
            )
        } else {
            Vec::new()
        };
        for hit in &mut hits {
            enrich_hit_with_gameplay_effect(
                hit,
                &gameplay_effects,
                &self.gameplay_effect_names,
                &self.gameplay_effect_skills,
                &self.ability_tip_names,
            );
            reattribute_hit_from_ability_name(hit, !final_tower_evidence.is_empty(), characters);
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
        let mut time_stop_events = self.ultra_time_stop.events_from_packet(
            timestamp,
            &decoded_text,
            &ids,
            !outgoing,
            ultra_time_stop_flow,
            &self.ultra_time_stops,
        );
        time_stop_events.extend(
            self.ultra_time_stop
                .events_from_hits(&hits, &self.ultra_time_stops),
        );
        let prepared_hits =
            self.prepare_hits_for_emission(hits, &ids, include_incoming, characters);
        for character_id in &ids {
            self.character_declarations.insert(*character_id, timestamp);
        }
        self.character_declarations
            .retain(|_, declared_at| timestamp - *declared_at <= 10.0);
        let accepted = prepared_hits.emit.len();
        // CurrentHP 候选缺少目标 handle 校验，仅用于调试显示，不参与 follow-up 计算。
        let current_hp_updates = if outgoing {
            Vec::new()
        } else {
            parse_current_hp_updates(payload)
        };
        let boss_hp_updates = if outgoing {
            Vec::new()
        } else {
            parse_boss_hp_updates(payload)
        };
        let transport_packet = parse_transport_packet(payload);
        let single_bunch = match &transport_packet {
            Some(TransportPacket::Sequenced(packet)) => parse_single_bunch(packet),
            _ => None,
        };
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
            && boss_hp_updates.is_empty()
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
        for event in time_stop_events {
            let _ = sender.send(EngineEvent::TimeStop(event));
        }
        if current_hp_updates.is_empty()
            && boss_hp_updates.is_empty()
            && prepared_hits.deferred_ambiguous == 0
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
            let (inferred_follow_ups, hp_sync_follow_ups, server_damage_corrections) =
                self.reconcile_boss_hp_updates(timestamp, &boss_hp_updates, characters);
            for event in abyss_events_from_text(timestamp, &decoded_text) {
                let _ = sender.send(EngineEvent::Abyss(event));
            }
            let _ = sender.send(EngineEvent::PacketObservation(PacketObservation {
                parsed_hits: accepted,
            }));
            self.emit_hits(prepared_hits.emit, characters, sender);
            for follow_up in inferred_follow_ups {
                let _ = sender.send(EngineEvent::HitFollowUp(follow_up));
            }
            for follow_up in hp_sync_follow_ups {
                let _ = sender.send(EngineEvent::HitFollowUp(follow_up));
            }
            for correction in server_damage_corrections {
                let _ = sender.send(EngineEvent::HitDamageCorrection(correction));
            }
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
        append_packet_note(&mut note, equipment_slots_note(&equipment_slots));
        let (inferred_follow_ups, hp_sync_follow_ups, server_damage_corrections) =
            self.reconcile_boss_hp_updates(timestamp, &boss_hp_updates, characters);
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
                        "SingleBunch seq {}，descriptor 0x{:02x}，partial 0x{:x}，数据 {} bit",
                        bunch.sequence, bunch.descriptor, bunch.partial_flags, bunch.data_bit_len
                    )),
                );
            }
        }
        send_packet_events(
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
        self.emit_hits(prepared_hits.emit, characters, sender);
        for follow_up in inferred_follow_ups {
            let _ = sender.send(EngineEvent::HitFollowUp(follow_up));
        }
        for follow_up in hp_sync_follow_ups {
            let _ = sender.send(EngineEvent::HitFollowUp(follow_up));
        }
        for correction in server_damage_corrections {
            let _ = sender.send(EngineEvent::HitDamageCorrection(correction));
        }
    }
}

pub fn start_capture(
    device: CaptureDevice,
    local_ip: Option<Ipv4Addr>,
    filter: String,
    include_incoming: bool,
    use_server_damage_calibration: bool,
    characters: Arc<HashMap<u32, CharacterInfo>>,
    output: CaptureOutput,
) -> CaptureHandle {
    let CaptureOutput {
        raw_capture_directory,
        packet_emission,
        sender,
    } = output;
    let stop = Arc::new(AtomicBool::new(false));
    let thread_stop = stop.clone();
    let raw_capture = RawCaptureBuffer::new(device.clone(), raw_capture_directory.as_deref());
    let thread_raw_capture = raw_capture.clone();
    let thread = thread::spawn(move || {
        let result = run_capture(CaptureRunConfig {
            device: &device,
            local_ip,
            filter: &filter,
            include_incoming,
            use_server_damage_calibration,
            characters,
            sender: &sender,
            stop: &thread_stop,
            raw_capture: &thread_raw_capture,
            packet_emission,
        });
        thread_raw_capture.finish();
        let _ = sender.send(EngineEvent::CaptureStopped);
        if let Err(error) = result {
            let _ = sender.send(EngineEvent::Error(error));
        }
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
    characters: Arc<HashMap<u32, CharacterInfo>>,
    sender: &'a Sender<EngineEvent>,
    stop: &'a AtomicBool,
    raw_capture: &'a RawCaptureBuffer,
    packet_emission: PacketEmissionMode,
}

/// Parser thread body: drains decoded frames off the bounded queue and runs the stable decode
/// pipeline, fully decoupled from packet acquisition. It owns its own `PacketDecoder` and exits
/// once the acquisition thread drops the frame sender, flushing any deferred ambiguous hits.
fn run_parser(
    frames: Receiver<CaptureFrame>,
    local_ip: Option<Ipv4Addr>,
    include_incoming: bool,
    use_server_damage_calibration: bool,
    packet_emission: PacketEmissionMode,
    characters: Arc<HashMap<u32, CharacterInfo>>,
    sender: Sender<EngineEvent>,
) {
    let mut decoder = PacketDecoder::with_server_damage_calibration(use_server_damage_calibration);
    decoder.packet_emission = packet_emission;
    if let Some(warning) = decoder.resource_warning() {
        let _ = sender.send(EngineEvent::Warning(warning));
    }
    while let Ok(frame) = frames.recv() {
        decoder.process_ethernet_frame(
            &frame.data,
            FrameTimestamp::Known(frame.timestamp),
            local_ip,
            include_incoming,
            &characters,
            &sender,
        );
    }
    let pending_hits = decoder.take_all_ambiguous_hits();
    decoder.emit_hits(pending_hits, &characters, &sender);
}

fn run_capture(config: CaptureRunConfig<'_>) -> Result<(), String> {
    let CaptureRunConfig {
        device,
        local_ip,
        filter,
        include_incoming,
        use_server_damage_calibration,
        characters,
        sender,
        stop,
        raw_capture,
        packet_emission,
    } = config;
    // SAFETY: Function pointers are loaded from Npcap and used per the libpcap API.
    unsafe {
        let _packet_library =
            Library::new(packet_library_path()).map_err(|error| error.to_string())?;
        let library = Library::new(npcap_library_path()).map_err(|error| error.to_string())?;
        let open_live: OpenLive = load_symbol(&library, b"pcap_open_live\0")?;
        let next_ex: NextEx = load_symbol(&library, b"pcap_next_ex\0")?;
        let close: Close = load_symbol(&library, b"pcap_close\0")?;
        let compile: Compile = load_symbol(&library, b"pcap_compile\0")?;
        let set_filter: SetFilter = load_symbol(&library, b"pcap_setfilter\0")?;
        let free_code: FreeCode = load_symbol(&library, b"pcap_freecode\0")?;
        let get_err: GetErr = load_symbol(&library, b"pcap_geterr\0")?;

        let device_name = CString::new(device.name.as_str()).map_err(|error| error.to_string())?;
        let mut error_buffer = [0_i8; PCAP_ERRBUF_SIZE];
        let handle = open_live(
            device_name.as_ptr(),
            65_535,
            1,
            100,
            error_buffer.as_mut_ptr(),
        );
        if handle.is_null() {
            return Err(format!(
                "failed to open device: {}",
                c_string(error_buffer.as_ptr())
            ));
        }
        let handle = PcapHandle::new(handle, close);

        let capture_filter = CString::new(filter).map_err(|error| error.to_string())?;
        let mut program = BpfProgramGuard::new(free_code);
        if compile(
            handle.as_ptr(),
            program.as_mut(),
            capture_filter.as_ptr(),
            1,
            u32::MAX,
        ) != 0
            || set_filter(handle.as_ptr(), program.as_mut()) != 0
        {
            let error = c_string(get_err(handle.as_ptr()));
            return Err(format!("failed to set capture filter: {error}"));
        }
        program.release();
        let raw_capture_status = raw_capture.path().map_or_else(
            || "; raw capture unavailable".to_owned(),
            |path| format!("; writing raw capture to {}", path.display()),
        );
        let _ = sender.send(EngineEvent::Status(format!(
            "capturing: {} ({}){}",
            device.description,
            local_ip
                .map(|ip| ip.to_string())
                .unwrap_or_else(|| "local IP not filtered".to_owned()),
            raw_capture_status
        )));

        // Decode on a dedicated thread so a parse latency spike cannot stall the acquisition loop
        // below. The acquisition thread only reads frames, writes the raw PCAPNG, and forwards a
        // copy onto the bounded queue.
        let (frame_sender, frame_receiver) = bounded::<CaptureFrame>(CAPTURE_FRAME_QUEUE_CAPACITY);
        let parser_thread = {
            let characters = Arc::clone(&characters);
            let sender = sender.clone();
            thread::spawn(move || {
                run_parser(
                    frame_receiver,
                    local_ip,
                    include_incoming,
                    use_server_damage_calibration,
                    packet_emission,
                    characters,
                    sender,
                );
            })
        };

        let mut loop_result = Ok(());
        while !stop.load(Ordering::Relaxed) {
            let mut header = ptr::null();
            let mut packet_data = ptr::null();
            let result = next_ex(handle.as_ptr(), &mut header, &mut packet_data);
            if result == 0 {
                continue;
            }
            if result < 0 {
                let error = c_string(get_err(handle.as_ptr()));
                loop_result = Err(format!("failed to read packet: {error}"));
                break;
            }
            if header.is_null() || packet_data.is_null() {
                continue;
            }
            let header_ref = &*header;
            if header_ref.caplen == 0 {
                continue;
            }
            let packet = std::slice::from_raw_parts(packet_data, header_ref.caplen as usize);
            let timestamp =
                header_ref.ts.tv_sec as f64 + header_ref.ts.tv_usec as f64 / 1_000_000.0;
            let raw_timestamp = Duration::new(
                header_ref.ts.tv_sec.max(0) as u64,
                header_ref.ts.tv_usec.clamp(0, 999_999) as u32 * 1_000,
            );
            raw_capture.push(raw_timestamp, header_ref.len, packet);
            match frame_sender.try_send(CaptureFrame {
                data: packet.to_vec(),
                timestamp,
            }) {
                // Queue full: drop for live parsing only. The raw frame is already persisted above
                // and remains recoverable via Debug replay, so acquisition keeps draining Npcap.
                Ok(()) | Err(TrySendError::Full(_)) => {}
                // Parser thread ended unexpectedly; nothing left to feed.
                Err(TrySendError::Disconnected(_)) => break,
            }
        }
        drop(frame_sender);
        let _ = parser_thread.join();
        loop_result?;
    }
    Ok(())
}

pub fn import_pcapng(
    path: PathBuf,
    characters: Arc<HashMap<u32, CharacterInfo>>,
    local_ip_hint: Option<Ipv4Addr>,
    include_incoming: bool,
    use_server_damage_calibration: bool,
    sender: Sender<EngineEvent>,
    stop: Arc<AtomicBool>,
) -> thread::JoinHandle<()> {
    thread::spawn(move || {
        let direction_mode = local_ip_hint.map_or_else(
            || "heuristic direction".to_owned(),
            |ip| format!("local IP {ip}"),
        );
        let _ = sender.send(EngineEvent::Status(format!(
            "importing pcapng: {direction_mode}"
        )));
        let result = (|| -> Result<(usize, usize), String> {
            let file = File::open(&path).map_err(|error| error.to_string())?;
            let mut reader = PcapNgReader::new(file).map_err(|error| error.to_string())?;
            let mut decoder =
                PacketDecoder::with_server_damage_calibration(use_server_damage_calibration);
            if let Some(warning) = decoder.resource_warning() {
                let _ = sender.send(EngineEvent::Warning(warning));
            }
            let mut packet_count = 0;
            let mut supported_count = 0;

            while let Some(block) = reader.next_block() {
                if stop.load(Ordering::Relaxed) {
                    break;
                }
                let block = block.map_err(|error| error.to_string())?;
                let (interface_id, timestamp, data) = match block {
                    Block::EnhancedPacket(packet) => (
                        packet.interface_id as usize,
                        FrameTimestamp::Known(packet.timestamp.as_secs_f64()),
                        packet.data.into_owned(),
                    ),
                    Block::SimplePacket(packet) => {
                        (0, FrameTimestamp::Unknown, packet.data.into_owned())
                    }
                    _ => continue,
                };
                packet_count += 1;
                let Some(interface) = reader.interfaces().get(interface_id) else {
                    continue;
                };
                if interface.linktype != DataLink::ETHERNET {
                    continue;
                }
                supported_count += 1;
                decoder.process_ethernet_frame(
                    &data,
                    timestamp,
                    local_ip_hint,
                    include_incoming,
                    &characters,
                    &sender,
                );
            }
            let pending_hits = decoder.take_all_ambiguous_hits();
            decoder.emit_hits(pending_hits, &characters, &sender);
            if packet_count > 0 && supported_count == 0 {
                return Err("pcapng contains no supported Ethernet packets".to_owned());
            }
            Ok((packet_count, supported_count))
        })();

        let _ = sender.send(EngineEvent::CaptureStopped);
        match result {
            Ok((packet_count, supported_count)) => {
                let _ = sender.send(EngineEvent::Status(format!(
                    "pcapng import complete: read {packet_count} packets, parsed {supported_count} Ethernet packets; {direction_mode}"
                )));
            }
            Err(error) => {
                let _ = sender.send(EngineEvent::Error(format!("pcapng import failed: {error}")));
            }
        }
    })
}

#[derive(Deserialize)]
struct CaptureExport {
    #[serde(default)]
    hits: Vec<ExportHit>,
    #[serde(default)]
    packets: Vec<ExportPacket>,
    #[serde(default)]
    empty_curtain: Vec<EmptyCurtainItem>,
    #[serde(default)]
    empty_curtain_characters: Vec<EmptyCurtainCharacter>,
}

#[derive(Deserialize)]
struct ExportHit {
    timestamp_unix: f64,
    char_id: u32,
    char_name: String,
    damage: f64,
    #[serde(default = "default_outgoing_direction")]
    direction: String,
    #[serde(default)]
    target_hp_before: f64,
    #[serde(default)]
    target_hp_after: f64,
    #[serde(default)]
    target_max_hp: f64,
    #[serde(default)]
    target_hp_percent: f64,
    #[serde(default)]
    target_id: Option<String>,
    #[serde(default)]
    target_name: Option<String>,
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
}

fn default_outgoing_direction() -> String {
    "outgoing".to_owned()
}

#[derive(Deserialize)]
struct ExportPacket {
    timestamp_unix: f64,
    source: String,
    destination: String,
    #[serde(default)]
    direction: String,
    #[serde(default)]
    payload_len: usize,
    #[serde(default)]
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

pub fn import_capture_json(
    path: PathBuf,
    sender: Sender<EngineEvent>,
    stop: Arc<AtomicBool>,
) -> thread::JoinHandle<()> {
    thread::spawn(move || {
        let result = (|| -> Result<(usize, usize), String> {
            let text = std::fs::read_to_string(&path).map_err(|error| error.to_string())?;
            let mut document = parse_capture_export(&text)?;
            drop(text);
            let saved_empty_curtain = std::mem::take(&mut document.empty_curtain);
            let mut saved_empty_curtain_characters =
                std::mem::take(&mut document.empty_curtain_characters);
            if saved_empty_curtain_characters.is_empty() {
                saved_empty_curtain_characters = saved_empty_curtain
                    .iter()
                    .filter_map(|item| {
                        Some(EmptyCurtainCharacter {
                            net_id: item.character_net_id?,
                            character_id: item.equipped_character_id?,
                        })
                    })
                    .collect();
                saved_empty_curtain_characters.sort_by_key(|character| {
                    (
                        character.character_id,
                        character.net_id.solt,
                        character.net_id.serial,
                    )
                });
                saved_empty_curtain_characters.dedup();
            }
            let saved_empty_curtain_characters =
                validate_empty_curtain_characters(saved_empty_curtain_characters)
                    .ok_or_else(|| "invalid Console equipment snapshot".to_owned())?;
            let ultra_time_stops = find_data_file(Path::new(ULTRA_TIME_STOP_DATA_PATH))
                .and_then(|path| load_ultra_time_stops(&path).ok())
                .unwrap_or_default();
            let mut ultra_time_stop = UltraTimeStopTracker::default();
            let equipment_catalog = match find_data_file(Path::new(EQUIPMENT_CATALOG_PATH)) {
                Some(path) => match load_equipment_catalog(&path) {
                    Ok(catalog) => catalog,
                    // stderr is invisible in the windows-subsystem GUI, so the load
                    // failure detail must travel over the Warning channel instead.
                    Err(error) if saved_empty_curtain.is_empty() => {
                        let _ = sender.send(EngineEvent::Warning(format!(
                            "Failed to load Console equipment data for JSON replay: {error:#}"
                        )));
                        EquipmentCatalog::default()
                    }
                    Err(error) => {
                        let _ = sender.send(EngineEvent::Warning(format!(
                            "Failed to load Console equipment data for JSON replay: {error:#}"
                        )));
                        // humanize_engine_error matches this exact string for the
                        // localized message; keep the returned error stable.
                        return Err("Console equipment data is unavailable".to_owned());
                    }
                },
                None if saved_empty_curtain.is_empty() => EquipmentCatalog::default(),
                None => return Err("Console equipment data is unavailable".to_owned()),
            };
            if !validate_empty_curtain_snapshot(&saved_empty_curtain, &equipment_catalog) {
                return Err("invalid Console equipment snapshot".to_owned());
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
            let mut packets = document.packets.into_iter().peekable();
            let mut hits = document.hits.into_iter().peekable();

            while packets.peek().is_some() || hits.peek().is_some() {
                if stop.load(Ordering::Relaxed) {
                    break;
                }
                let take_packet = match (packets.peek(), hits.peek()) {
                    (Some(packet), Some(hit)) => packet.timestamp_unix <= hit.timestamp_unix,
                    (Some(_), None) => true,
                    (None, Some(_)) => false,
                    (None, None) => break,
                };
                if take_packet {
                    let packet = packets.next().expect("peeked packet must exist");
                    if send_export_packet(
                        packet,
                        &sender,
                        &ultra_time_stops,
                        &mut ultra_time_stop,
                        &mut empty_curtain,
                    )? {
                        packet_count += 1;
                    }
                } else {
                    let hit = hits.next().expect("peeked hit must exist");
                    let event = export_hit_event(hit);
                    if let EngineEvent::Hit(hit) = &event {
                        for time_stop in ultra_time_stop
                            .events_from_hits(std::slice::from_ref(hit.as_ref()), &ultra_time_stops)
                        {
                            sender
                                .send(EngineEvent::TimeStop(time_stop))
                                .map_err(|error| error.to_string())?;
                        }
                    }
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

        let _ = sender.send(EngineEvent::CaptureStopped);
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
    })
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
    sender: &Sender<EngineEvent>,
    ultra_time_stops: &HashMap<u32, UltraTimeStopEntry>,
    ultra_time_stop: &mut UltraTimeStopTracker,
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
    let server_to_client = !packet.direction.eq_ignore_ascii_case("C2S");
    let ultra_time_stop_flow = export_ultra_time_stop_flow_key(&packet.source, &packet.destination);
    for event in ultra_time_stop.events_from_packet(
        packet.timestamp_unix,
        &decoded_text,
        &declared_ids,
        server_to_client,
        ultra_time_stop_flow,
        ultra_time_stops,
    ) {
        sender
            .send(EngineEvent::TimeStop(event))
            .map_err(|error| error.to_string())?;
    }
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
    for event in abyss_events_from_text(packet.timestamp, &packet.decoded_text) {
        sender
            .send(EngineEvent::Abyss(event))
            .map_err(|error| error.to_string())?;
    }
    sender
        .send(EngineEvent::Packet(Box::new(packet)))
        .map_err(|error| error.to_string())?;
    Ok(true)
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
        char_source: "export_json".to_owned(),
        direction: hit.direction,
        target_hp_before: hit.target_hp_before,
        target_hp_after: hit.target_hp_after,
        target_max_hp: hit.target_max_hp,
        target_hp_percent: hit.target_hp_percent,
        target_id: hit.target_id,
        target_name: hit.target_name,
        target_context: hit.target_context,
        gameplay_effect_index: hit.gameplay_effect_index,
        gameplay_effect_name: hit.gameplay_effect_name,
        ability_name: hit.ability_name,
        damage_name: hit.damage_name.map(|name| normalize_damage_name(&name)),
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
    }))
}

fn parse_capture_export(text: &str) -> Result<CaptureExport, String> {
    serde_json::from_str(text)
        .or_else(|_| {
            let repaired = text
                .lines()
                .map(|line| {
                    if line.trim_start().starts_with(r#""payload_hex":"#) && !line.ends_with(',') {
                        format!("{line},")
                    } else {
                        line.to_owned()
                    }
                })
                .collect::<Vec<_>>()
                .join("\n");
            serde_json::from_str(&repaired)
        })
        .map_err(|error| error.to_string())
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

    use crate::engine::parser::{CHARACTER_DATA_PATH, UltraTimeStopCooldown, load_characters};

    fn inventory_bunch(sequence: u16, partial_flags: u8, data: u8) -> SingleBunch {
        SingleBunch {
            prefix: 7,
            sequence,
            descriptor: 0xcc,
            partial_flags,
            data_bit_len: 8,
            data: vec![data],
        }
    }

    fn ultra_entry(ability_id: &str, montage_asset: &str, duration: f64) -> UltraTimeStopEntry {
        UltraTimeStopEntry {
            ability_id: ability_id.to_owned(),
            montage_asset: montage_asset.to_owned(),
            end_ability_event_seconds: duration,
            source: "test".to_owned(),
            confidence: "high".to_owned(),
            ..UltraTimeStopEntry::default()
        }
    }

    fn ultra_entry_with_activation_tags(
        ability_id: &str,
        montage_asset: &str,
        duration: f64,
        activation_cooldown_tags: &[&str],
    ) -> UltraTimeStopEntry {
        UltraTimeStopEntry {
            activation_cooldown_tags: activation_cooldown_tags
                .iter()
                .map(|tag| (*tag).to_owned())
                .collect(),
            ..ultra_entry(ability_id, montage_asset, duration)
        }
    }

    fn ultra_test_flow(server_port: u16) -> UltraTimeStopFlowKey {
        UltraTimeStopFlowKey {
            source: (Ipv4Addr::new(10, 0, 0, 3), server_port),
            destination: (Ipv4Addr::new(10, 0, 0, 2), 50_000),
        }
    }

    fn ultra_test_table() -> HashMap<u32, UltraTimeStopEntry> {
        let female_montage = "/Game/Characters/Player/051_female/animation/Skill/char_f_skill_utraskill_Montage.char_f_skill_utraskill_Montage";
        HashMap::from([
            (
                1003,
                ultra_entry_with_activation_tags(
                    "GA_Sagiri_UltraSkill",
                    "/Game/Characters/Player/003_sagiri_1/animation/Skill/Sagiri_UltralSkill.Sagiri_UltralSkill",
                    3.533936,
                    &["CoolDown.Player.UltraSkill.Sagiri"],
                ),
            ),
            (
                1004,
                ultra_entry_with_activation_tags(
                    "GA_Lacrimosa_UltraSkill",
                    "/Game/Characters/Player/004_lacrimosa/animation/Skill/Lacrimosa_UltraSkill_ModB.Lacrimosa_UltraSkill_ModB",
                    4.218461,
                    &["CoolDown.Player.UltraSkill.Lacrimosa"],
                ),
            ),
            (
                1010,
                ultra_entry(
                    "GA_Nanally_UltraSkill",
                    "/Game/Characters/Player/010_nanally/animation/skill/Nanally_UltralSkill.Nanally_UltralSkill",
                    3.584608,
                ),
            ),
            (
                1025,
                ultra_entry(
                    "GA_Hathor_UltraSkill",
                    "/Game/Characters/Player/025_hathor_1/animation/Skill/Hathor_UltraSkill.Hathor_UltraSkill",
                    4.518597,
                ),
            ),
            (
                1036,
                ultra_entry("GA_Zankou_UltraSkill", female_montage, 4.161653),
            ),
            (
                1051,
                ultra_entry("GA_Female051_UltraSkill", female_montage, 4.161653),
            ),
            (
                1055,
                ultra_entry_with_activation_tags(
                    "GA_Kuhara_UltraSkill",
                    "/Game/Characters/Player/055_kuhara/animation/skill/Kuhara_UltraSkill.Kuhara_UltraSkill",
                    5.599441,
                    &["CoolDown.Player.UltraSkill.Kuhara"],
                ),
            ),
            (
                1076,
                ultra_entry(
                    "GA_Shinku_UltraSkill",
                    "/Game/Characters/Player/076_shinku/animation/skill/Shinku_UltraSkill.Shinku_UltraSkill",
                    7.300015,
                ),
            ),
        ])
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

    fn character_owner_packet(character_id: u32, net_id: HtItemNetId) -> SequencedPacket {
        let mut record = InventoryTestBitWriter::default();
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

        let mut payload = InventoryTestBitWriter::default();
        payload.push_bits(4122, 13);
        payload.push_bits(87, 10);
        payload.push_bits(0xccd, 12);
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
        }
    }

    #[test]
    fn inventory_reassembly_replaces_stale_fragments_after_sequence_wrap() {
        let mut state = InventoryConnectionState::default();
        assert!(
            state
                .push_bunches(vec![
                    inventory_bunch(1023, 0x09, 0xa1),
                    inventory_bunch(1, 0x0c, 0xa3),
                ])
                .is_empty()
        );

        let completed = state.push_bunches(vec![
            inventory_bunch(1023, 0x09, 0xb1),
            inventory_bunch(0, 0x08, 0xb2),
            inventory_bunch(1, 0x0c, 0xb3),
        ]);

        assert_eq!(completed.len(), 1);
        assert_eq!(completed[0].data, vec![0xb1, 0xb2, 0xb3]);
        assert_eq!(completed[0].bit_len, 24);
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

        let result =
            decoder.process_packet(connection, &character_owner_packet(1020, owner_net_id));

        assert!(result.recognized);
        assert!(result.snapshot.is_none());
        assert_eq!(
            result.characters,
            Some(vec![EmptyCurtainCharacter {
                net_id: owner_net_id,
                character_id: 1020,
            }])
        );
    }

    #[test]
    fn capture_export_defaults_and_preserves_empty_curtain_snapshot() {
        let legacy = parse_capture_export(r#"{"hits":[],"packets":[]}"#)
            .expect("legacy capture export should remain compatible");
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
        assert_eq!(current.empty_curtain_characters.len(), 1);
        assert_eq!(current.empty_curtain_characters[0].character_id, 1020);
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
    fn disabled_raw_capture_has_no_path_or_writer() {
        let buffer = RawCaptureBuffer::new(
            CaptureDevice {
                name: "test".to_owned(),
                description: "test".to_owned(),
                ipv4: Vec::new(),
            },
            None,
        );
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
        let mut empty_curtain = EmptyCurtainDecoder::new(EquipmentCatalog::default());
        let packet = ExportPacket {
            timestamp_unix: 1.0,
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
            send_export_packet(
                packet,
                &sender,
                &HashMap::new(),
                &mut UltraTimeStopTracker::default(),
                &mut empty_curtain,
            )
            .unwrap_err()
            .contains("payload_hex 无效")
        );
        assert!(receiver.try_recv().is_err());
    }

    #[test]
    fn send_export_packet_adds_final_tower_ids_from_payload() {
        let (sender, receiver) = unbounded();
        let mut empty_curtain = EmptyCurtainDecoder::new(EquipmentCatalog::default());
        let payload = b"FCharacterForNet....ft_character_1076".to_vec();
        let packet = ExportPacket {
            timestamp_unix: 1.0,
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

        assert!(
            send_export_packet(
                packet,
                &sender,
                &HashMap::new(),
                &mut UltraTimeStopTracker::default(),
                &mut empty_curtain,
            )
            .is_ok()
        );
        match receiver.try_recv().expect("packet event should be emitted") {
            EngineEvent::Packet(packet) => {
                assert_eq!(packet.declared_ids, [1076]);
                assert!(packet.decoded_text.contains("ft_character_1076"));
            }
            _ => panic!("expected packet event"),
        }
        assert!(receiver.try_recv().is_err());
    }

    #[test]
    fn send_export_packet_accepts_empty_payload_hex() {
        let (sender, receiver) = unbounded();
        let mut empty_curtain = EmptyCurtainDecoder::new(EquipmentCatalog::default());
        let packet = ExportPacket {
            timestamp_unix: 1.0,
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
        assert!(
            send_export_packet(
                packet,
                &sender,
                &HashMap::new(),
                &mut UltraTimeStopTracker::default(),
                &mut empty_curtain,
            )
            .is_ok()
        );
        match receiver.try_recv().expect("packet event should be emitted") {
            EngineEvent::Packet(packet) => {
                assert_eq!(packet.payload_hex, String::new());
                assert_eq!(packet.payload_len, 0);
            }
            _ => panic!("expected packet event"),
        }
        assert!(receiver.try_recv().is_err());
    }

    #[test]
    fn send_export_packet_preserves_exported_decoded_text() {
        let (sender, receiver) = unbounded();
        let mut empty_curtain = EmptyCurtainDecoder::new(EquipmentCatalog::default());
        let packet = ExportPacket {
            timestamp_unix: 1.0,
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

        assert!(
            send_export_packet(
                packet,
                &sender,
                &HashMap::new(),
                &mut UltraTimeStopTracker::default(),
                &mut empty_curtain,
            )
            .is_ok()
        );
        match receiver.try_recv().expect("packet event should be emitted") {
            EngineEvent::Packet(packet) => {
                assert_eq!(packet.decoded_text, "导出时的协议文本");
                assert_eq!(packet.payload_hex, "0000");
            }
            _ => panic!("expected packet event"),
        }
        assert!(receiver.try_recv().is_err());
    }

    #[test]
    fn send_export_packet_emits_pending_time_stop_end_before_filtering() {
        let (sender, receiver) = unbounded();
        let table = HashMap::from([(
            1010,
            UltraTimeStopEntry {
                ability_id: "GA_Nanally_UltraSkill".to_owned(),
                end_ability_event_seconds: 3.584608,
                source: "test".to_owned(),
                confidence: "high".to_owned(),
                ..UltraTimeStopEntry::default()
            },
        )]);
        let mut tracker = UltraTimeStopTracker::default();
        let mut empty_curtain = EmptyCurtainDecoder::new(EquipmentCatalog::default());
        let start_packet = ExportPacket {
            timestamp_unix: 10.0,
            source: "127.0.0.1:1234".to_owned(),
            destination: "127.0.0.1:5678".to_owned(),
            direction: "S2C".to_owned(),
            payload_len: 0,
            declared_ids: serde_json::json!([]),
            parsed_hits: 0,
            note: String::new(),
            payload_preview: String::new(),
            payload_hex: String::new(),
            decoded_text: "CoolDown.Player.UltraSkill.TimeActor".to_owned(),
        };
        assert!(
            send_export_packet(
                start_packet,
                &sender,
                &table,
                &mut tracker,
                &mut empty_curtain,
            )
            .unwrap()
        );
        assert!(matches!(
            receiver
                .try_recv()
                .expect("pending start should be emitted"),
            EngineEvent::TimeStop(TimeStopEvent::ExtraStart {
                timestamp: 10.0,
                ..
            })
        ));
        assert!(matches!(
            receiver.try_recv().expect("debug packet should be emitted"),
            EngineEvent::Packet(_)
        ));

        let ignored_packet = ExportPacket {
            timestamp_unix: 15.0,
            source: "127.0.0.1:1234".to_owned(),
            destination: "127.0.0.1:5678".to_owned(),
            direction: "S2C".to_owned(),
            payload_len: 4,
            declared_ids: serde_json::json!([]),
            parsed_hits: 0,
            note: String::new(),
            payload_preview: "00000000".to_owned(),
            payload_hex: "00000000".to_owned(),
            decoded_text: String::new(),
        };
        assert!(
            !send_export_packet(
                ignored_packet,
                &sender,
                &table,
                &mut tracker,
                &mut empty_curtain,
            )
            .unwrap()
        );
        assert!(matches!(
            receiver.try_recv().expect("pending end should be emitted"),
            EngineEvent::TimeStop(TimeStopEvent::ExtraEnd {
                timestamp: 12.5,
                ..
            })
        ));
        assert!(receiver.try_recv().is_err());
    }

    #[test]
    fn ultra_cooldown_packet_emits_fixed_time_stop_once() {
        let table = HashMap::from([(
            1052,
            UltraTimeStopEntry {
                ability_id: "GA_Jin_UltraSkill".to_owned(),
                end_ability_event_seconds: 2.183333,
                source: "test".to_owned(),
                confidence: "high".to_owned(),
                ..UltraTimeStopEntry::default()
            },
        )]);
        let mut tracker = UltraTimeStopTracker::default();
        assert!(matches!(
            tracker
                .events_from_packet(
                    9.0,
                    "CoolDown.Player.UltraSkill.TimeActor",
                    &[1052],
                    true,
                    None,
                    &table,
                )
                .as_slice(),
            [TimeStopEvent::ExtraStart { timestamp: 9.0, .. }]
        ));
        let events = tracker.events_from_packet(
            10.0,
            "CoolDown.Player.UltraSkill.F",
            &[1052],
            true,
            None,
            &table,
        );

        assert_eq!(
            events,
            vec![
                TimeStopEvent::ExtraEnd {
                    timestamp: 10.0,
                    reason: format!("{PENDING_ULTRA_TIME_STOP_REASON}.0"),
                },
                TimeStopEvent::UltraAnimation {
                    timestamp: 10.0,
                    char_id: 1052,
                    ability_id: "GA_Jin_UltraSkill".to_owned(),
                    duration_seconds: 2.183333,
                },
            ]
        );
        assert!(
            tracker
                .events_from_packet(
                    11.0,
                    "CoolDown.Player.UltraSkill.F",
                    &[1052],
                    true,
                    None,
                    &table,
                )
                .is_empty()
        );
    }

    #[test]
    fn time_actor_prelude_closes_at_montage_activation() {
        let table = ultra_test_table();
        let flow = Some(ultra_test_flow(7_777));
        let mut tracker = UltraTimeStopTracker::default();

        assert_eq!(
            tracker.events_from_packet(
                10.0,
                "CoolDown.Player.UltraSkill.TimeActor",
                &[],
                true,
                flow,
                &table,
            ),
            vec![TimeStopEvent::ExtraStart {
                timestamp: 10.0,
                reason: format!("{PENDING_ULTRA_TIME_STOP_REASON}.0"),
            }]
        );
        assert_eq!(
            tracker.events_from_packet(
                11.846596,
                "/Game/Characters/Player/025_hathor_1/animation/Skill/Hathor_UltraSkill\nCoolDown.Player.UltraSkill.F",
                &[],
                true,
                flow,
                &table,
            ),
            vec![
                TimeStopEvent::ExtraEnd {
                    timestamp: 11.846596,
                    reason: format!("{PENDING_ULTRA_TIME_STOP_REASON}.0"),
                },
                TimeStopEvent::UltraAnimation {
                    timestamp: 11.846596,
                    char_id: 1025,
                    ability_id: "GA_Hathor_UltraSkill".to_owned(),
                    duration_seconds: 4.518597,
                },
            ]
        );
    }

    #[test]
    fn time_actor_prelude_closes_at_specific_activation_tag() {
        let table = ultra_test_table();
        let flow = Some(ultra_test_flow(7_777));
        let mut tracker = UltraTimeStopTracker::default();

        assert_eq!(
            tracker.events_from_packet(
                10.0,
                "CoolDown.Player.UltraSkill.TimeActor",
                &[],
                true,
                flow,
                &table,
            ),
            vec![TimeStopEvent::ExtraStart {
                timestamp: 10.0,
                reason: format!("{PENDING_ULTRA_TIME_STOP_REASON}.0"),
            }]
        );
        assert_eq!(
            tracker.events_from_packet(
                11.877756,
                "CoolDown.Player.UltraSkill.Sagiri",
                &[1051],
                true,
                flow,
                &table,
            ),
            vec![
                TimeStopEvent::ExtraEnd {
                    timestamp: 11.877756,
                    reason: format!("{PENDING_ULTRA_TIME_STOP_REASON}.0"),
                },
                TimeStopEvent::UltraAnimation {
                    timestamp: 11.877756,
                    char_id: 1003,
                    ability_id: "GA_Sagiri_UltraSkill".to_owned(),
                    duration_seconds: 3.533936,
                },
            ]
        );
    }

    #[test]
    fn specific_activation_tags_identify_every_non_f_test_character() {
        let table = ultra_test_table();
        for (tag, char_id, ability_id, duration_seconds) in [
            (
                "CoolDown.Player.UltraSkill.Sagiri",
                1003,
                "GA_Sagiri_UltraSkill",
                3.533936,
            ),
            (
                "CoolDown.Player.UltraSkill.Lacrimosa",
                1004,
                "GA_Lacrimosa_UltraSkill",
                4.218461,
            ),
            (
                "CoolDown.Player.UltraSkill.Kuhara",
                1055,
                "GA_Kuhara_UltraSkill",
                5.599441,
            ),
        ] {
            let mut tracker = UltraTimeStopTracker::default();
            assert_eq!(
                tracker.events_from_packet(10.0, tag, &[], true, None, &table),
                vec![TimeStopEvent::UltraAnimation {
                    timestamp: 10.0,
                    char_id,
                    ability_id: ability_id.to_owned(),
                    duration_seconds,
                }]
            );
        }
    }

    #[test]
    fn specific_activation_tag_overrides_mismatched_recent_montage() {
        let table = ultra_test_table();
        let flow = Some(ultra_test_flow(7_777));
        let mut tracker = UltraTimeStopTracker::default();

        assert!(
            tracker
                .events_from_packet(
                    10.0,
                    "/Game/Characters/Player/010_nanally/animation/skill/Nanally_UltralSkill",
                    &[],
                    true,
                    flow,
                    &table,
                )
                .is_empty()
        );
        assert_eq!(
            tracker.events_from_packet(
                10.01,
                "CoolDown.Player.UltraSkill.Sagiri",
                &[],
                true,
                flow,
                &table,
            ),
            vec![TimeStopEvent::UltraAnimation {
                timestamp: 10.01,
                char_id: 1003,
                ability_id: "GA_Sagiri_UltraSkill".to_owned(),
                duration_seconds: 3.533936,
            }]
        );
    }

    #[test]
    fn generic_cooldown_accepts_skin_montage_variant() {
        let table = ultra_test_table();
        let mut tracker = UltraTimeStopTracker::default();

        assert_eq!(
            tracker.events_from_packet(
                10.0,
                "/Game/Characters/Player/004_lacrimosa_fashion2/animation/Skill/Lacrimosa_UltraSkill_ModB\nCoolDown.Player.UltraSkill.F",
                &[],
                true,
                Some(ultra_test_flow(7_777)),
                &table,
            ),
            vec![TimeStopEvent::UltraAnimation {
                timestamp: 10.0,
                char_id: 1004,
                ability_id: "GA_Lacrimosa_UltraSkill".to_owned(),
                duration_seconds: 4.218461,
            }]
        );
    }

    #[test]
    fn generic_cooldown_uses_single_declared_character_without_time_actor() {
        let table = ultra_test_table();
        let mut tracker = UltraTimeStopTracker::default();

        assert_eq!(
            tracker.events_from_packet(
                10.0,
                "CoolDown.Player.UltraSkill.F",
                &[1010],
                true,
                Some(ultra_test_flow(7_777)),
                &table,
            ),
            vec![TimeStopEvent::UltraAnimation {
                timestamp: 10.0,
                char_id: 1010,
                ability_id: "GA_Nanally_UltraSkill".to_owned(),
                duration_seconds: 3.584608,
            }]
        );
    }

    #[test]
    fn time_actor_declared_id_does_not_identify_character() {
        let table = ultra_test_table();
        let mut tracker = UltraTimeStopTracker::default();

        assert!(matches!(
            tracker
                .events_from_packet(
                    10.0,
                    "CoolDown.Player.UltraSkill.TimeActor",
                    &[1051],
                    true,
                    Some(ultra_test_flow(7_777)),
                    &table,
                )
                .as_slice(),
            [TimeStopEvent::ExtraStart {
                timestamp: 10.0,
                ..
            }]
        ));
        assert!(tracker.recent_emitted.is_empty());
    }

    #[test]
    fn expired_time_actor_ends_at_timeout() {
        let table = ultra_test_table();
        let mut tracker = UltraTimeStopTracker::default();

        assert!(matches!(
            tracker
                .events_from_packet(
                    10.0,
                    "CoolDown.Player.UltraSkill.TimeActor",
                    &[],
                    true,
                    None,
                    &table,
                )
                .as_slice(),
            [TimeStopEvent::ExtraStart {
                timestamp: 10.0,
                ..
            }]
        ));
        assert_eq!(
            tracker.events_from_packet(30.0, "", &[], true, None, &table),
            vec![TimeStopEvent::ExtraEnd {
                timestamp: 10.0 + ULTRA_TIME_ACTOR_PENDING_WINDOW_SECONDS,
                reason: format!("{PENDING_ULTRA_TIME_STOP_REASON}.0"),
            }]
        );
    }

    #[test]
    fn mixed_time_actor_snapshot_does_not_open_public_pending() {
        let table = ultra_test_table();
        let mut tracker = UltraTimeStopTracker::default();

        assert!(
            tracker
                .events_from_packet(
                    10.0,
                    "CoolDown.Player.UltraSkill.TimeActor\nCoolDown.Player.UltraSkill.F",
                    &[1051],
                    true,
                    Some(ultra_test_flow(7_777)),
                    &table,
                )
                .is_empty()
        );
        assert!(tracker.pending_cooldowns.is_empty());
    }

    #[test]
    fn shinku_rage_repeated_f_does_not_open_or_claim_pending() {
        let table = ultra_test_table();
        let mut tracker = UltraTimeStopTracker::default();

        assert!(
            tracker
                .events_from_packet(
                    10.0,
                    "GameplayCue.Display.Shinku.Rage\nCoolDown.Player.UltraSkill.F\nAbility.Player.Shinku.Rage",
                    &[],
                    true,
                    Some(ultra_test_flow(7_777)),
                    &table,
                )
                .is_empty()
        );
        let mut hit = targetless_hit();
        hit.timestamp = 10.5;
        hit.char_id = 1076;
        hit.ability_name = Some("GA_Shinku_UltraSkill".to_owned());
        assert!(tracker.events_from_hits(&[hit], &table).is_empty());
    }

    #[test]
    fn ultra_cooldown_same_packet_montage_identifies_empty_nanally_cast() {
        let table = ultra_test_table();
        let mut tracker = UltraTimeStopTracker::default();

        assert_eq!(
            tracker.events_from_packet(
                10.0,
                "/Game/Characters/Player/010_nanally/animation/skill/Nanally_UltralSkill\nCoolDown.Player.UltraSkill.F",
                &[],
                true,
                Some(ultra_test_flow(7_777)),
                &table,
            ),
            vec![TimeStopEvent::UltraAnimation {
                timestamp: 10.0,
                char_id: 1010,
                ability_id: "GA_Nanally_UltraSkill".to_owned(),
                duration_seconds: 3.584608,
            }]
        );
    }

    #[test]
    fn ultra_cooldown_uses_adjacent_montage_and_ignores_mismatched_table_key() {
        let table = ultra_test_table();
        let flow = Some(ultra_test_flow(7_777));
        let mut tracker = UltraTimeStopTracker::default();

        assert!(
            tracker
                .events_from_packet(
                    10.0,
                    "/Game/Characters/Player/051_female/animation/Skill/char_f_skill_utraskill_Montage",
                    &[],
                    true,
                    flow,
                    &table,
                )
                .is_empty()
        );
        assert_eq!(
            tracker.events_from_packet(
                10.003,
                "CoolDown.Player.UltraSkill.F",
                &[],
                true,
                flow,
                &table,
            ),
            vec![TimeStopEvent::UltraAnimation {
                timestamp: 10.003,
                char_id: 1051,
                ability_id: "GA_Female051_UltraSkill".to_owned(),
                duration_seconds: 4.161653,
            }]
        );
    }

    #[test]
    fn ultra_cooldown_accepts_character_montage_variant() {
        let table = ultra_test_table();
        let mut tracker = UltraTimeStopTracker::default();

        assert_eq!(
            tracker.events_from_packet(
                10.0,
                "/Game/Characters/Player/076_shinku/animation/skill/Shinku_UltraSkill_Boss\nCoolDown.Player.UltraSkill.F",
                &[],
                true,
                Some(ultra_test_flow(7_777)),
                &table,
            ),
            vec![TimeStopEvent::UltraAnimation {
                timestamp: 10.0,
                char_id: 1076,
                ability_id: "GA_Shinku_UltraSkill".to_owned(),
                duration_seconds: 7.300015,
            }]
        );
    }

    #[test]
    fn ultra_cooldown_ignores_c2s_and_other_flow_montages() {
        let table = ultra_test_table();
        let montage = "/Game/Characters/Player/010_nanally/animation/skill/Nanally_UltralSkill";
        let flow = Some(ultra_test_flow(7_777));
        let mut tracker = UltraTimeStopTracker::default();

        assert!(
            tracker
                .events_from_packet(10.0, montage, &[], false, flow, &table)
                .is_empty()
        );
        assert!(
            tracker
                .events_from_packet(
                    10.005,
                    montage,
                    &[],
                    true,
                    Some(ultra_test_flow(8_888)),
                    &table,
                )
                .is_empty()
        );
        assert!(
            tracker
                .events_from_packet(
                    10.01,
                    "CoolDown.Player.UltraSkill.F",
                    &[],
                    true,
                    flow,
                    &table,
                )
                .is_empty()
        );
        assert_eq!(tracker.pending_cooldowns.len(), 1);
    }

    #[test]
    fn ultra_cooldown_keeps_ambiguous_montages_pending() {
        let table = ultra_test_table();
        let mut tracker = UltraTimeStopTracker::default();

        assert!(
            tracker
                .events_from_packet(
                    10.0,
                    "/Game/Characters/Player/010_nanally/animation/skill/Nanally_UltralSkill\n/Game/Characters/Player/025_hathor_1/animation/Skill/Hathor_UltraSkill\nCoolDown.Player.UltraSkill.F",
                    &[],
                    true,
                    Some(ultra_test_flow(7_777)),
                    &table,
                )
                .is_empty()
        );
        assert_eq!(tracker.pending_cooldowns.len(), 1);
    }

    #[test]
    fn ultra_cooldown_keeps_conflicting_declared_character_pending() {
        let table = ultra_test_table();
        let mut tracker = UltraTimeStopTracker::default();

        assert!(
            tracker
                .events_from_packet(
                    10.0,
                    "/Game/Characters/Player/010_nanally/animation/skill/Nanally_UltralSkill\nCoolDown.Player.UltraSkill.F",
                    &[1025],
                    true,
                    Some(ultra_test_flow(7_777)),
                    &table,
                )
                .is_empty()
        );
        assert_eq!(tracker.pending_cooldowns.len(), 1);
    }

    #[test]
    fn shinku_special_cooldowns_emit_two_distinct_time_stop_segments() {
        let table = HashMap::from([(
            1076,
            UltraTimeStopEntry {
                ability_id: "GA_Shinku_UltraSkill".to_owned(),
                montage_asset: "/Game/Characters/Player/076_shinku/animation/skill/Shinku_UltraSkill.Shinku_UltraSkill".to_owned(),
                activation_cooldown_tags: vec![
                    "CoolDown.Player.UltraSkill.F".to_owned(),
                ],
                end_ability_event_seconds: 7.300015,
                extra_cooldowns: vec![UltraTimeStopCooldown {
                    cooldown_tag: "CoolDown.Player.UltraSkill.Shinku.UltraRage".to_owned(),
                    ability_id: "GA_Shinku_UltraSkill_Rage".to_owned(),
                    duration_seconds: 5.300056,
                }],
                ignored_cooldown_tags: vec![
                    "CoolDown.Player.UltraSkill.Shinku.UltraPre".to_owned(),
                ],
                source: "test".to_owned(),
                confidence: "high".to_owned(),
            },
        )]);
        let mut tracker = UltraTimeStopTracker::default();
        assert!(
            tracker
                .events_from_packet(
                    10.0,
                    "CoolDown.Player.UltraSkill.F",
                    &[],
                    true,
                    None,
                    &table,
                )
                .is_empty()
        );

        let mut hit = targetless_hit();
        hit.timestamp = 10.2;
        hit.char_id = 1076;
        hit.ability_name = Some("GA_Shinku_UltraSkill".to_owned());
        assert_eq!(
            tracker.events_from_hits(&[hit], &table),
            vec![TimeStopEvent::UltraAnimation {
                timestamp: 10.0,
                char_id: 1076,
                ability_id: "GA_Shinku_UltraSkill".to_owned(),
                duration_seconds: 7.300015,
            }]
        );

        assert!(
            tracker
                .events_from_packet(
                    20.0,
                    "CoolDown.Player.UltraSkill.Shinku.UltraPre",
                    &[],
                    true,
                    None,
                    &table,
                )
                .is_empty()
        );
        assert_eq!(
            tracker.events_from_packet(
                21.2,
                "CoolDown.Player.UltraSkill.Shinku.UltraRage",
                &[],
                true,
                None,
                &table,
            ),
            vec![TimeStopEvent::UltraAnimation {
                timestamp: 21.2,
                char_id: 1076,
                ability_id: "GA_Shinku_UltraSkill_Rage".to_owned(),
                duration_seconds: 5.300056,
            }]
        );
    }

    #[test]
    fn pending_ultra_cooldown_is_claimed_by_later_ultra_damage_hit() {
        let table = HashMap::from([(
            1010,
            UltraTimeStopEntry {
                ability_id: "GA_Nanally_UltraSkill".to_owned(),
                end_ability_event_seconds: 3.584608,
                source: "test".to_owned(),
                confidence: "high".to_owned(),
                ..UltraTimeStopEntry::default()
            },
        )]);
        let mut tracker = UltraTimeStopTracker::default();
        assert!(
            tracker
                .events_from_packet(
                    10.0,
                    "CoolDown.Player.UltraSkill.F",
                    &[],
                    true,
                    None,
                    &table,
                )
                .is_empty()
        );
        let mut hit = targetless_hit();
        hit.timestamp = 12.0;
        hit.char_id = 1010;
        hit.ability_name = Some("GA_Nanally_UltraSkill".to_owned());
        let events = tracker.events_from_hits(&[hit], &table);

        assert_eq!(
            events,
            vec![TimeStopEvent::UltraAnimation {
                timestamp: 10.0,
                char_id: 1010,
                ability_id: "GA_Nanally_UltraSkill".to_owned(),
                duration_seconds: 3.584608,
            }]
        );
    }

    #[test]
    fn pending_ultra_cooldown_ignores_non_ultra_damage_before_claim() {
        let table = HashMap::from([
            (
                1010,
                UltraTimeStopEntry {
                    ability_id: "GA_Nanally_UltraSkill".to_owned(),
                    end_ability_event_seconds: 3.584608,
                    source: "test".to_owned(),
                    confidence: "high".to_owned(),
                    ..UltraTimeStopEntry::default()
                },
            ),
            (
                1052,
                UltraTimeStopEntry {
                    ability_id: "GA_Jin_UltraSkill".to_owned(),
                    end_ability_event_seconds: 2.183333,
                    source: "test".to_owned(),
                    confidence: "high".to_owned(),
                    ..UltraTimeStopEntry::default()
                },
            ),
        ]);
        let mut tracker = UltraTimeStopTracker::default();
        assert!(
            tracker
                .events_from_packet(
                    10.0,
                    "CoolDown.Player.UltraSkill.F",
                    &[],
                    true,
                    None,
                    &table,
                )
                .is_empty()
        );
        let mut jin_skill_hit = targetless_hit();
        jin_skill_hit.timestamp = 10.5;
        jin_skill_hit.char_id = 1052;
        jin_skill_hit.ability_name = Some("GA_Jin_Skill".to_owned());
        assert!(
            tracker
                .events_from_hits(&[jin_skill_hit], &table)
                .is_empty()
        );

        let mut nanally_ultra_hit = targetless_hit();
        nanally_ultra_hit.timestamp = 12.0;
        nanally_ultra_hit.char_id = 1010;
        nanally_ultra_hit.ability_name = Some("GA_Nanally_UltraSkill".to_owned());
        assert_eq!(
            tracker.events_from_hits(&[nanally_ultra_hit], &table),
            vec![TimeStopEvent::UltraAnimation {
                timestamp: 10.0,
                char_id: 1010,
                ability_id: "GA_Nanally_UltraSkill".to_owned(),
                duration_seconds: 3.584608,
            }]
        );
    }

    #[test]
    fn co_axis_cooldown_with_multiple_characters_waits_for_the_ultra_hit() {
        let table = HashMap::from([
            (
                1010,
                UltraTimeStopEntry {
                    ability_id: "GA_Nanally_UltraSkill".to_owned(),
                    end_ability_event_seconds: 3.584608,
                    source: "test".to_owned(),
                    confidence: "high".to_owned(),
                    ..UltraTimeStopEntry::default()
                },
            ),
            (
                1052,
                UltraTimeStopEntry {
                    ability_id: "GA_Jin_UltraSkill".to_owned(),
                    end_ability_event_seconds: 2.183333,
                    source: "test".to_owned(),
                    confidence: "high".to_owned(),
                    ..UltraTimeStopEntry::default()
                },
            ),
        ]);
        let mut tracker = UltraTimeStopTracker::default();
        assert!(
            tracker
                .events_from_packet(
                    10.0,
                    "CoolDown.Player.UltraSkill.F",
                    &[1052, 1010],
                    true,
                    None,
                    &table,
                )
                .is_empty()
        );
        assert_eq!(tracker.pending_cooldowns.len(), 1);

        let mut previous_character_hit = targetless_hit();
        previous_character_hit.timestamp = 10.2;
        previous_character_hit.char_id = 1052;
        previous_character_hit.ability_name = Some("GA_Jin_Skill".to_owned());
        assert!(
            tracker
                .events_from_hits(&[previous_character_hit], &table)
                .is_empty()
        );

        let mut active_character_ultra_hit = targetless_hit();
        active_character_ultra_hit.timestamp = 12.0;
        active_character_ultra_hit.char_id = 1010;
        active_character_ultra_hit.ability_name = Some("GA_Nanally_UltraSkill".to_owned());
        assert_eq!(
            tracker.events_from_hits(&[active_character_ultra_hit], &table),
            vec![TimeStopEvent::UltraAnimation {
                timestamp: 10.0,
                char_id: 1010,
                ability_id: "GA_Nanally_UltraSkill".to_owned(),
                duration_seconds: 3.584608,
            }]
        );
    }

    #[test]
    fn pending_ultra_cooldown_claims_closest_prior_start() {
        let table = HashMap::from([(
            1010,
            UltraTimeStopEntry {
                ability_id: "GA_Nanally_UltraSkill".to_owned(),
                end_ability_event_seconds: 3.584608,
                source: "test".to_owned(),
                confidence: "high".to_owned(),
                ..UltraTimeStopEntry::default()
            },
        )]);
        let mut tracker = UltraTimeStopTracker::default();
        assert!(
            tracker
                .events_from_packet(
                    10.0,
                    "CoolDown.Player.UltraSkill.F",
                    &[],
                    true,
                    None,
                    &table,
                )
                .is_empty()
        );
        assert!(
            tracker
                .events_from_packet(
                    11.5,
                    "CoolDown.Player.UltraSkill.F",
                    &[],
                    true,
                    None,
                    &table,
                )
                .is_empty()
        );

        let mut hit = targetless_hit();
        hit.timestamp = 12.0;
        hit.char_id = 1010;
        hit.ability_name = Some("GA_Nanally_UltraSkill".to_owned());
        assert_eq!(
            tracker.events_from_hits(&[hit], &table),
            vec![TimeStopEvent::UltraAnimation {
                timestamp: 11.5,
                char_id: 1010,
                ability_id: "GA_Nanally_UltraSkill".to_owned(),
                duration_seconds: 3.584608,
            }]
        );

        assert!(
            tracker
                .events_from_packet(15.0, "", &[], true, None, &table)
                .is_empty()
        );
    }

    #[test]
    fn ignored_packet_still_emits_pending_ultra_time_stop_end() {
        let (sender, receiver) = unbounded();
        let mut decoder = PacketDecoder {
            ultra_time_stops: HashMap::from([(
                1010,
                UltraTimeStopEntry {
                    ability_id: "GA_Nanally_UltraSkill".to_owned(),
                    end_ability_event_seconds: 3.584608,
                    source: "test".to_owned(),
                    confidence: "high".to_owned(),
                    ..UltraTimeStopEntry::default()
                },
            )]),
            ..PacketDecoder::default()
        };
        let characters = HashMap::new();
        let local_ip = Ipv4Addr::new(10, 0, 0, 2);
        let remote_ip = Ipv4Addr::new(10, 0, 0, 3);

        decoder.process_ethernet_frame(
            &udp_ipv4_packet(
                b"CoolDown.Player.UltraSkill.TimeActor",
                remote_ip,
                7_777,
                local_ip,
                50_000,
            ),
            FrameTimestamp::Known(10.0),
            Some(local_ip),
            true,
            &characters,
            &sender,
        );
        assert!(matches!(
            receiver
                .try_recv()
                .expect("pending start should be emitted"),
            EngineEvent::TimeStop(TimeStopEvent::ExtraStart {
                timestamp: 10.0,
                ..
            })
        ));
        assert!(matches!(
            receiver.try_recv().expect("debug packet should be emitted"),
            EngineEvent::Packet(_)
        ));

        decoder.process_ethernet_frame(
            &udp_ipv4_packet(&[0, 0, 0, 0], remote_ip, 7_777, local_ip, 50_000),
            FrameTimestamp::Known(15.0),
            Some(local_ip),
            true,
            &characters,
            &sender,
        );
        assert!(matches!(
            receiver.try_recv().expect("pending end should be emitted"),
            EngineEvent::TimeStop(TimeStopEvent::ExtraEnd {
                timestamp: 12.5,
                ..
            })
        ));
        assert!(receiver.try_recv().is_err());
    }

    #[test]
    fn summary_payload_text_retains_only_runtime_markers() {
        let irrelevant =
            decode_summary_payload_text(b"Some.DebugProtocolIdentifier", &HashMap::new());
        assert!(irrelevant.has_readable_text);
        assert_eq!(irrelevant.text, UNREADABLE_PROTOCOL_TEXT);

        let abyss = decode_summary_payload_text(
            b"FAbyssGamePlayData ConditionState_Success",
            &HashMap::new(),
        );
        assert!(abyss.has_readable_text);
        assert!(abyss.text.contains("FAbyssGamePlayData"));
        assert!(abyss.text.contains("ConditionState_Success"));
    }

    #[test]
    fn summary_payload_text_retains_registered_montage_with_nonstandard_spelling() {
        let montage = "/Game/Characters/Player/010_nanally/animation/skill/Nanally_UltralSkill";
        let table = ultra_test_table();

        let decoded = decode_summary_payload_text(montage.as_bytes(), &table);

        assert!(decoded.has_readable_text);
        assert_eq!(decoded.text, montage);
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
        PacketDecoder::default().process_ethernet_frame(
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
    fn jin_packet_events_emit_extra_time_stop_interval_markers() {
        let mut tracker = UltraTimeStopTracker::default();
        let events = tracker.events_from_packet(
            12.0,
            "Event.Montage.Player.UltraSkill.Jin.EnterTimeStop\nEvent.Montage.Player.UltraSkill.Jin.ClearTimeStop",
            &[],
            true,
            None,
            &HashMap::new(),
        );

        assert_eq!(
            events,
            vec![
                TimeStopEvent::ExtraStart {
                    timestamp: 12.0,
                    reason: JIN_EXTRA_TIME_STOP_REASON.to_owned(),
                },
                TimeStopEvent::ExtraEnd {
                    timestamp: 12.0,
                    reason: JIN_EXTRA_TIME_STOP_REASON.to_owned(),
                },
            ]
        );
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
        hit.direction = "incoming".to_owned();

        enrich_hit_with_gameplay_effect(
            &mut hit,
            &effects,
            &names,
            &HashMap::new(),
            &HashMap::new(),
        );

        assert_eq!(hit.direction, "outgoing");
        assert_eq!(hit.attack_type.as_deref(), Some("载具伤害"));
        assert_eq!(hit.damage_attribute.as_deref(), Some("物理"));
    }

    #[test]
    fn player_damage_effect_is_outgoing_even_when_record_looks_incoming() {
        let effects = [ParsedGameplayEffect {
            unique_index: 2031,
            byte_offset: 0,
            bit_shift: 0,
        }];
        let names = HashMap::from([(2031, "GE_Player_Hathor_QTE1_Damage".to_owned())]);
        let mut hit = targetless_hit();
        hit.direction = "incoming".to_owned();

        enrich_hit_with_gameplay_effect(
            &mut hit,
            &effects,
            &names,
            &HashMap::new(),
            &HashMap::new(),
        );

        assert_eq!(hit.direction, "outgoing");
        assert_eq!(hit.attack_type.as_deref(), Some("环合"));
    }

    #[test]
    fn non_prefixed_player_skill_damage_effect_overrides_incoming_direction() {
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
            },
        )]);
        let mut hit = targetless_hit();
        hit.direction = "incoming".to_owned();

        enrich_hit_with_gameplay_effect(&mut hit, &effects, &names, &skills, &HashMap::new());

        assert_eq!(hit.direction, "outgoing");
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
        hit.char_source = "packet".to_owned();
        hit.ability_name = Some("GA_Mint019_Skill".to_owned());

        reattribute_hit_from_ability_name(&mut hit, true, &characters);

        assert_eq!(hit.char_id, 1019);
        assert_eq!(hit.char_name, "薄荷");
        assert_eq!(hit.char_source, "gameplay_effect");
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
        hit.char_source = "packet".to_owned();
        hit.ability_name = Some("GA_Mint019_Skill".to_owned());

        reattribute_hit_from_ability_name(&mut hit, false, &characters);

        assert_eq!(hit.char_id, 1076);
        assert_eq!(hit.char_name, "真红");
        assert_eq!(hit.char_source, "packet");
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
        hit.char_source = "session".to_owned();
        hit.ability_name = Some("GA_Mint019_QTE".to_owned());

        reattribute_hit_from_ability_name(&mut hit, false, &characters);

        assert_eq!(hit.char_id, 1019);
        assert_eq!(hit.char_name, "薄荷");
        assert_eq!(hit.char_source, "gameplay_effect");
    }

    #[test]
    fn reaction_damage_effect_overrides_incoming_direction() {
        let effects = [ParsedGameplayEffect {
            unique_index: 4010,
            byte_offset: 0,
            bit_shift: 0,
        }];
        let names = HashMap::from([(4010, "GE_ActorReaction_1_Damage".to_owned())]);
        let mut hit = targetless_hit();
        hit.direction = "incoming".to_owned();

        enrich_hit_with_gameplay_effect(
            &mut hit,
            &effects,
            &names,
            &HashMap::new(),
            &HashMap::new(),
        );

        assert_eq!(hit.direction, "outgoing");
        assert_eq!(hit.attack_type.as_deref(), Some("创生花"));
    }

    #[test]
    fn reaction_buff_effect_overrides_incoming_direction() {
        let effects = [ParsedGameplayEffect {
            unique_index: 4011,
            byte_offset: 0,
            bit_shift: 0,
        }];
        let names = HashMap::from([(4011, "Buff_Reaction_4_new".to_owned())]);
        let mut hit = targetless_hit();
        hit.direction = "incoming".to_owned();

        enrich_hit_with_gameplay_effect(
            &mut hit,
            &effects,
            &names,
            &HashMap::new(),
            &HashMap::new(),
        );

        assert_eq!(hit.direction, "outgoing");
        assert_eq!(hit.attack_type.as_deref(), Some("黯星"));
    }

    #[test]
    fn tenacity_damage_effect_overrides_incoming_direction() {
        let effects = [ParsedGameplayEffect {
            unique_index: 4012,
            byte_offset: 0,
            bit_shift: 0,
        }];
        let names = HashMap::from([(4012, "Buff_Tenacity_damage".to_owned())]);
        let mut hit = targetless_hit();
        hit.direction = "incoming".to_owned();

        enrich_hit_with_gameplay_effect(
            &mut hit,
            &effects,
            &names,
            &HashMap::new(),
            &HashMap::new(),
        );

        assert_eq!(hit.direction, "outgoing");
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
        hit.direction = "outgoing".to_owned();

        enrich_hit_with_gameplay_effect(
            &mut hit,
            &effects,
            &names,
            &HashMap::new(),
            &HashMap::new(),
        );

        assert_eq!(hit.direction, "incoming");
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
            },
        )]);
        let mut hit = targetless_hit();
        hit.direction = "outgoing".to_owned();

        enrich_hit_with_gameplay_effect(&mut hit, &effects, &names, &skills, &HashMap::new());

        assert_eq!(hit.direction, "outgoing");
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
            Some(local_ip),
            &[],
            &endpoints,
        ));
        assert!(!infer_outgoing(
            remote_ip,
            40_000,
            local_ip,
            Some(local_ip),
            &[1001],
            &endpoints,
        ));
        assert!(infer_outgoing(
            remote_ip,
            40_000,
            local_ip,
            None,
            &[1001],
            &endpoints,
        ));
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
            char_source: "test".to_owned(),
            direction: "outgoing".to_owned(),
            target_hp_before: 0.0,
            target_hp_after: 0.0,
            target_max_hp: 0.0,
            target_hp_percent: 0.0,
            target_id: None,
            target_name: None,
            target_context: Vec::new(),
            gameplay_effect_index: None,
            gameplay_effect_name: None,
            ability_name: None,
            damage_name: None,
            attack_type: None,
            damage_attribute: None,
            follow_up_damage: 0.0,
            follow_up_timestamp: None,
            follow_up_damage_name: None,
            follow_up_attack_type: None,
            follow_up_damage_attribute: None,
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

    fn duplicate_test_hit(timestamp: f64, char_source: &str, direction: &str) -> Hit {
        let mut hit = targetless_hit();
        hit.timestamp = timestamp;
        hit.char_id = if char_source == "packet" { 1051 } else { 1010 };
        hit.char_name = if char_source == "packet" {
            "零(女)".to_owned()
        } else {
            "娜娜莉".to_owned()
        };
        hit.damage = 5_829.0;
        hit.char_source = char_source.to_owned();
        hit.direction = direction.to_owned();
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
                .gameplay_effect_skills
                .get("GE_Player_Nanally_Melee1_Damage")
                .map(|skill| skill.attack_type.as_str()),
            Some("普攻")
        );
        assert_eq!(
            decoder
                .ability_tip_names
                .get("GA_Cang_Melee")
                .map(String::as_str),
            Some("言行合一")
        );
    }

    fn boss_hp_update(timestamp_hp: f32) -> crate::engine::parser::ParsedBossHpUpdate {
        crate::engine::parser::ParsedBossHpUpdate {
            target_handle: [7; 16],
            current_hp: timestamp_hp,
            byte_offset: 0,
            bit_shift: 0,
        }
    }

    #[test]
    fn server_damage_calibration_corrects_single_pending_hit() {
        let mut tracker = ServerDamageCalibrationTracker::default();
        assert!(
            tracker
                .observe_boss_hp(9.0, &boss_hp_update(10_000.0))
                .is_none()
        );
        let mut hit = duplicate_test_hit(10.0, "packet", "outgoing");
        hit.damage = 1_000.0;
        hit.target_hp_before = 10_000.0;
        hit.target_hp_after = 9_000.0;
        hit.target_max_hp = 10_000.0;
        tracker.observe_hit(&hit);

        let correction = tracker
            .observe_boss_hp(10.05, &boss_hp_update(8_750.0))
            .expect("single pending hit should use server HP delta");

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
        let mut first = duplicate_test_hit(10.0, "packet", "outgoing");
        first.target_hp_before = 10_000.0;
        first.target_hp_after = 9_000.0;
        first.target_max_hp = 10_000.0;
        let mut second = duplicate_test_hit(10.02, "packet", "outgoing");
        second.target_hp_before = 9_000.0;
        second.target_hp_after = 8_500.0;
        second.target_max_hp = 10_000.0;
        tracker.observe_hit(&first);
        tracker.observe_hit(&second);

        assert!(
            tracker
                .observe_boss_hp(10.05, &boss_hp_update(8_500.0))
                .is_none()
        );
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
        let _ = decoder.reconcile_boss_hp_updates(0.0, std::slice::from_ref(&warm_up), &characters);

        let mut hit = targetless_hit();
        hit.char_id = 1;
        hit.timestamp = 0.1;
        hit.target_max_hp = 1_000_000.0;
        hit.target_hp_before = 1_000_000.0;
        hit.damage = 1_000.0;
        hit.attack_type = Some("普攻".to_owned());
        decoder
            .follow_up_damage
            .observe_hit(&hit, None, &characters);
        decoder.server_damage_calibration.observe_hit(&hit);

        let update = boss_hp_update(998_750.0);
        let (inferred_follow_ups, hp_sync_follow_ups, server_damage_corrections) =
            decoder.reconcile_boss_hp_updates(0.2, std::slice::from_ref(&update), &characters);

        assert_eq!(inferred_follow_ups.len(), 1);
        assert_eq!(inferred_follow_ups[0].damage, 250.0);
        assert!(hp_sync_follow_ups.is_empty());
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
        let _ = decoder.reconcile_boss_hp_updates(0.0, std::slice::from_ref(&warm_up), &characters);

        // This hit is claimed by the reaction follow-up below.
        let mut hit = targetless_hit();
        hit.char_id = 1;
        hit.timestamp = 0.1;
        hit.target_max_hp = 1_000_000.0;
        hit.target_hp_before = 1_000_000.0;
        hit.damage = 1_000.0;
        hit.attack_type = Some("普攻".to_owned());
        decoder
            .follow_up_damage
            .observe_hit(&hit, None, &characters);
        decoder.server_damage_calibration.observe_hit(&hit);

        let claimed_update = boss_hp_update(998_750.0);
        let (inferred_follow_ups, _, server_damage_corrections) = decoder
            .reconcile_boss_hp_updates(0.2, std::slice::from_ref(&claimed_update), &characters);
        assert_eq!(inferred_follow_ups.len(), 1);
        assert!(server_damage_corrections.is_empty());

        // A later, fuwen-ineligible hit (its own attack_type is itself an
        // excluded reaction label) that calibration alone should evaluate.
        let mut second_hit = targetless_hit();
        second_hit.char_id = 2;
        second_hit.timestamp = 0.3;
        second_hit.target_max_hp = 1_000_000.0;
        second_hit.target_hp_before = 998_750.0;
        second_hit.damage = 700.0;
        second_hit.attack_type = Some("创生花".to_owned());
        decoder
            .follow_up_damage
            .observe_hit(&second_hit, None, &characters);
        decoder.server_damage_calibration.observe_hit(&second_hit);

        let next_update = boss_hp_update(998_000.0);
        let (_, _, server_damage_corrections) =
            decoder.reconcile_boss_hp_updates(0.4, std::slice::from_ref(&next_update), &characters);

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
        let _ = decoder.reconcile_boss_hp_updates(9.0, std::slice::from_ref(&warm_up), &characters);

        let mut hit = duplicate_test_hit(10.0, "packet", "outgoing");
        hit.damage = 1_000.0;
        hit.target_hp_before = 10_000.0;
        hit.target_hp_after = 9_000.0;
        hit.target_max_hp = 10_000.0;
        decoder
            .follow_up_damage
            .observe_hit(&hit, None, &characters);
        decoder.server_damage_calibration.observe_hit(&hit);

        let update = boss_hp_update(8_750.0);
        let (inferred_follow_ups, hp_sync_follow_ups, server_damage_corrections) =
            decoder.reconcile_boss_hp_updates(10.05, std::slice::from_ref(&update), &characters);

        assert!(inferred_follow_ups.is_empty());
        assert!(hp_sync_follow_ups.is_empty());
        assert_eq!(server_damage_corrections.len(), 1);
        assert_eq!(server_damage_corrections[0].damage, 1_250.0);
    }

    #[test]
    fn confirmed_packet_hit_suppresses_ambiguous_session_candidate() {
        let mut decoder = PacketDecoder::default();
        let candidate = duplicate_test_hit(10.0, "session", "unknown");

        let characters = duplicate_test_characters();

        let prepared =
            decoder.prepare_hits_for_emission(vec![candidate], &[1051, 1055], false, &characters);

        assert!(prepared.emit.is_empty());
        assert_eq!(prepared.deferred_ambiguous, 1);
        assert_eq!(decoder.pending_ambiguous_hits.len(), 1);

        let confirmed = duplicate_test_hit(10.1, "packet", "outgoing");
        let prepared =
            decoder.prepare_hits_for_emission(vec![confirmed], &[1051], false, &characters);

        assert_eq!(prepared.emit.len(), 1);
        assert_eq!(prepared.suppressed_ambiguous, 1);
        assert_eq!(prepared.emit[0].char_id, 1051);
        assert!(decoder.pending_ambiguous_hits.is_empty());
    }

    #[test]
    fn confirmed_packet_hit_suppresses_recent_duplicate_records() {
        let mut decoder = PacketDecoder::default();
        let characters = duplicate_test_characters();
        let confirmed = duplicate_test_hit(10.0, "packet", "outgoing");
        let mut duplicate = confirmed.clone();
        duplicate.gameplay_effect_index = None;
        duplicate.gameplay_effect_name = None;
        duplicate.attack_type = None;
        duplicate.target_hp_before += 2_000.0;
        duplicate.target_hp_after += 2_000.0;

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
    fn boss_hp_sync_damage_merges_into_recent_confirmed_hit() {
        let mut decoder = PacketDecoder::default();
        let characters = duplicate_test_characters();
        let mut source = duplicate_test_hit(10.0, "packet", "outgoing");
        source.damage = 26_185.0;
        source.target_hp_before = 29_700.0;
        source.target_hp_after = 3_515.0;
        source.target_max_hp = 1_930_389.0;

        let prepared = decoder.prepare_hits_for_emission(vec![source], &[1051], false, &characters);
        assert_eq!(prepared.emit.len(), 1);

        let follow_up = decoder
            .infer_boss_hp_sync_damage(10.04, 1.0, &characters)
            .expect("server HP sync should add the missing lethal delta");

        assert_eq!(follow_up.damage, 3_515.0);
        assert_eq!(follow_up.target_hp_after, 0.0);
        assert_eq!(follow_up.damage_name.as_deref(), Some("HP同步伤害"));
        assert_eq!(follow_up.source_char_id, 1051);
    }

    #[test]
    fn nonlethal_boss_hp_sync_does_not_merge_into_recent_confirmed_hit() {
        let mut decoder = PacketDecoder::default();
        let characters = duplicate_test_characters();
        let mut source = duplicate_test_hit(10.0, "packet", "outgoing");
        source.damage = 8_446.0;
        source.target_hp_after = 1_081_975.0;
        source.target_max_hp = 1_930_389.0;

        let prepared = decoder.prepare_hits_for_emission(vec![source], &[1051], false, &characters);
        assert_eq!(prepared.emit.len(), 1);

        assert!(
            decoder
                .infer_boss_hp_sync_damage(10.04, 1_057_660.0, &characters)
                .is_none()
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
        hit.char_source = "session".to_owned();
        hit.direction = "unknown".to_owned();
        hit.damage = 6_961.0;
        hit.gameplay_effect_index = Some(3261);
        hit.gameplay_effect_name = Some("GE_Player_Haniel_Skill1_Damage".to_owned());
        hit.attack_type = Some("E技能".to_owned());

        let prepared =
            decoder.prepare_hits_for_emission(vec![hit], &[1010, 1020], false, &characters);

        assert_eq!(prepared.emit.len(), 1);
        assert_eq!(prepared.deferred_ambiguous, 0);
        assert_eq!(prepared.emit[0].direction, "outgoing");
        assert_eq!(prepared.emit[0].char_source, "gameplay_effect");
        assert!(decoder.pending_ambiguous_hits.is_empty());
    }

    #[test]
    fn ambiguous_session_candidate_expires_when_unconfirmed() {
        let mut decoder = PacketDecoder::default();
        let candidate = duplicate_test_hit(10.0, "session", "unknown");

        let characters = duplicate_test_characters();

        decoder.prepare_hits_for_emission(vec![candidate], &[1051, 1055], false, &characters);

        assert!(decoder.take_expired_ambiguous_hits(10.25).is_empty());
        let expired = decoder.take_expired_ambiguous_hits(10.75);

        assert_eq!(expired.len(), 1);
        assert_eq!(expired[0].char_source, "session");
        assert!(decoder.pending_ambiguous_hits.is_empty());
    }

    #[test]
    fn resolves_damage_name_from_ability_tips() {
        let skills = HashMap::from([(
            "GE_Player_Cang_Melee1_Damage".to_owned(),
            GameplayEffectSkill {
                damage_source_category: Some("A".to_owned()),
                ability_name: Some("GA_Cang_Melee".to_owned()),
                attack_type: "普攻".to_owned(),
            },
        )]);
        let ability_tips = HashMap::from([("GA_Cang_Melee".to_owned(), "言行合一".to_owned())]);

        assert_eq!(
            resolve_damage_name("GE_Player_Cang_Melee1_Damage", &skills, &ability_tips).as_deref(),
            Some("言行合一")
        );
    }

    #[test]
    fn resolves_explode_damage_name_from_its_own_ability_name() {
        let skills = HashMap::from([(
            "GE_Player_Lacrimosa_B_Melee3_Explode_Damage".to_owned(),
            GameplayEffectSkill {
                damage_source_category: Some("A".to_owned()),
                ability_name: Some("GA_Lacrimosa_Melee".to_owned()),
                attack_type: "普攻".to_owned(),
            },
        )]);
        let ability_tips =
            HashMap::from([("GA_Lacrimosa_Melee".to_owned(), "酸甜口味的制裁".to_owned())]);

        assert_eq!(
            resolve_damage_name(
                "GE_Player_Lacrimosa_B_Melee3_Explode_Damage",
                &skills,
                &ability_tips
            )
            .as_deref(),
            Some("酸甜口味的制裁")
        );
    }

    #[test]
    fn returns_none_when_ability_has_no_tip() {
        let skills = HashMap::from([(
            "GE_Test_Skill1_Damage".to_owned(),
            GameplayEffectSkill {
                damage_source_category: Some("E".to_owned()),
                ability_name: Some("GA_Test_Skill".to_owned()),
                attack_type: "E技能".to_owned(),
            },
        )]);

        assert_eq!(
            resolve_damage_name("GE_Test_Skill1_Damage", &skills, &HashMap::new()),
            None
        );
    }

    #[test]
    #[ignore = "set NTE_TEST_CAPTURE to a local pcapng path for capture diagnostics"]
    fn diagnose_empty_curtain_inventory() {
        let path = std::env::var("NTE_TEST_CAPTURE").expect("NTE_TEST_CAPTURE must be set");
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
        let stop = Arc::new(AtomicBool::new(false));
        let handle = import_pcapng(
            PathBuf::from(&path),
            characters,
            None,
            true,
            true,
            sender,
            stop,
        );
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
        let (sender, receiver) = unbounded();
        let stop = Arc::new(AtomicBool::new(false));
        let handle = import_pcapng(
            PathBuf::from(path),
            characters,
            None,
            true,
            true,
            sender,
            stop,
        );
        handle.join().expect("pcapng import thread should finish");

        let mut shinku_hits = Vec::new();
        let mut time_stops = Vec::new();
        let mut relevant_packets = Vec::new();
        let mut statuses = Vec::new();
        let mut warnings = Vec::new();
        let mut errors = Vec::new();
        for event in receiver.try_iter() {
            match event {
                EngineEvent::Hit(hit) if hit.char_id == 1076 => shinku_hits.push(*hit),
                EngineEvent::Packet(packet)
                    if packet.decoded_text.contains("Shinku")
                        || packet.decoded_text.contains("UltraSkill")
                        || packet.decoded_text.contains("1076") =>
                {
                    relevant_packets.push(*packet);
                }
                EngineEvent::TimeStop(event) => time_stops.push(event),
                EngineEvent::Status(status) => statuses.push(status),
                EngineEvent::Warning(warning) => warnings.push(warning),
                EngineEvent::Error(error) => errors.push(error),
                _ => {}
            }
        }

        println!("statuses: {statuses:#?}");
        println!("warnings: {warnings:#?}");
        println!("errors: {errors:#?}");
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
        println!("time stop count: {}", time_stops.len());
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
        let stop = Arc::new(AtomicBool::new(false));
        let handle = import_capture_json(PathBuf::from(path.clone()), sender, stop);
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
        let mut last_raw_handle: Option<[u8; 16]> = None;
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
                handle: [u8; 16],
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
                EngineEvent::Hit(hit) if hit.direction == "outgoing" => {
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
        let mut last_handle: Option<[u8; 16]> = None;
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
        let mut handle_first_seen: Vec<([u8; 16], f64)> = Vec::new();
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
        let mut last_handle: Option<[u8; 16]> = None;
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
